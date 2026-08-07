const fs = require('node:fs')
const path = require('node:path')
const process = require('node:process')

const root = path.resolve(__dirname, '..')
const defaultLockfile = path.join(root, 'package-lock.json')
const defaultNodeModules = path.join(root, 'node_modules')
const defaultOsvEndpoint = 'https://api.osv.dev/v1/querybatch'
const batchSize = 500
const maximumScannedFileSize = 5 * 1024 * 1024

// These exact versions were part of the initial ChainDrop disclosure. The
// local denylist remains effective if the advisory service is unavailable;
// the live OSV query below covers the wider and still-growing MAL advisory set.
const knownMaliciousPackages = new Set([
  '@cacheable/memory@2.2.1',
  '@cacheable/net@2.1.1',
  '@cacheable/node-cache@3.1.2',
  '@cacheable/utils@2.5.1',
  'cache-manager@7.2.10',
  'cacheable-request@13.0.20',
  'cacheable@2.5.1',
  'ecto@5.0.1',
  'file-entry-cache@11.1.6',
  'flat-cache@6.1.24',
  'keyv@6.0.0',
])

// npm lifecycle scripts are disabled in CI. This list documents every package
// currently marked hasInstallScript in the reviewed lockfile. Only esbuild is
// rebuilt explicitly; fsevents is optional watcher support and remains skipped.
const reviewedInstallScripts = new Set([
  'esbuild@0.28.1',
  'fsevents@2.3.2',
  'fsevents@2.3.3',
])

const chainDropArtifactNames = new Set(['Math_Symbol.js', 'math_init.js'])
const chainDropContentIndicators = ['npm-cache.com', 'Math_Symbol.js', 'math_init.js']
const scannedSourceExtensions = new Set(['.cjs', '.js', '.json', '.mjs'])

function fail(message) {
  throw new Error(message)
}

function parseArguments(argv) {
  let lockfile = defaultLockfile
  let nodeModules = defaultNodeModules
  let installedOnly = false
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]
    if (argument === '--lockfile') {
      const value = argv[index + 1]
      if (!value) fail('--lockfile requires a path')
      lockfile = path.resolve(value)
      index += 1
      continue
    }
    if (argument === '--node-modules') {
      const value = argv[index + 1]
      if (!value) fail('--node-modules requires a path')
      nodeModules = path.resolve(value)
      index += 1
      continue
    }
    if (argument === '--installed-only') {
      installedOnly = true
      continue
    }
    fail(`unknown argument: ${argument}`)
  }
  return { installedOnly, lockfile, nodeModules }
}

function packageNameFromLockPath(lockPath) {
  const marker = 'node_modules/'
  const normalized = lockPath.replaceAll('\\', '/')
  const index = normalized.lastIndexOf(marker)
  return index < 0 ? '' : normalized.slice(index + marker.length)
}

function readLockedPackages(lockfile) {
  let lock
  try {
    lock = JSON.parse(fs.readFileSync(lockfile, 'utf8'))
  } catch (error) {
    fail(`cannot read npm lockfile ${lockfile}: ${error.message}`)
  }

  if (!Number.isInteger(lock.lockfileVersion) || lock.lockfileVersion < 2) {
    fail(`unsupported npm lockfileVersion in ${lockfile}`)
  }
  if (!lock.packages || typeof lock.packages !== 'object' || Array.isArray(lock.packages)) {
    fail(`npm lockfile ${lockfile} does not contain a packages map`)
  }

  const packages = []
  const installScripts = []
  const seen = new Set()

  for (const [lockPath, metadata] of Object.entries(lock.packages)) {
    if (!metadata || typeof metadata !== 'object') continue
    const name = packageNameFromLockPath(lockPath)
    const version = metadata.version
    if (!name || typeof version !== 'string' || !version) continue

    const identity = `${name}@${version}`
    if (!seen.has(identity)) {
      seen.add(identity)
      packages.push({ name, version })
    }
    if (metadata.hasInstallScript === true) {
      installScripts.push({ identity, lockPath })
    }
  }

  if (packages.length === 0) fail(`npm lockfile ${lockfile} contains no resolved packages`)
  return { packages, installScripts }
}

function validateLocalPolicy(packages, installScripts) {
  const blocked = packages
    .map(({ name, version }) => `${name}@${version}`)
    .filter(identity => knownMaliciousPackages.has(identity))
  if (blocked.length > 0) {
    fail(`known malicious npm package version(s) in lockfile: ${blocked.join(', ')}`)
  }

  const unreviewed = installScripts.filter(({ identity }) => !reviewedInstallScripts.has(identity))
  if (unreviewed.length > 0) {
    const details = unreviewed.map(({ identity, lockPath }) => `${identity} (${lockPath})`)
    fail(`unreviewed npm lifecycle script(s) in lockfile: ${details.join(', ')}`)
  }
}

function scanInstalledContent(nodeModules) {
  if (!fs.statSync(nodeModules, { throwIfNoEntry: false })?.isDirectory()) {
    fail(`installed npm directory does not exist: ${nodeModules}`)
  }

  const matches = []
  const directories = [nodeModules]
  let scannedFiles = 0
  while (directories.length > 0) {
    const directory = directories.pop()
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const entryPath = path.join(directory, entry.name)
      if (entry.isDirectory()) {
        directories.push(entryPath)
        continue
      }
      if (!entry.isFile()) continue

      const relative = path.relative(nodeModules, entryPath)
      if (chainDropArtifactNames.has(entry.name)) {
        matches.push(`${relative} [artifact name]`)
        continue
      }
      if (!scannedSourceExtensions.has(path.extname(entry.name))) continue
      const size = fs.statSync(entryPath).size
      if (size > maximumScannedFileSize) continue

      scannedFiles += 1
      const contents = fs.readFileSync(entryPath)
      const indicator = chainDropContentIndicators.find(value => contents.includes(Buffer.from(value)))
      if (indicator) matches.push(`${relative} [contains ${indicator}]`)
    }
  }

  if (matches.length > 0) {
    fail(`ChainDrop file indicator match(es) in node_modules: ${matches.join('; ')}`)
  }
  return scannedFiles
}

function isMalwareAdvisory(vulnerability) {
  const identifiers = [vulnerability?.id, ...(vulnerability?.aliases || [])]
  return identifiers.some(identifier => typeof identifier === 'string' && /^MAL-/i.test(identifier))
}

async function queryOsv(packages) {
  const endpoint = process.env.NPM_SUPPLY_CHAIN_OSV_URL || defaultOsvEndpoint
  const timeout = Number.parseInt(process.env.NPM_SUPPLY_CHAIN_TIMEOUT_MS || '30000', 10)
  if (!Number.isFinite(timeout) || timeout <= 0) fail('NPM_SUPPLY_CHAIN_TIMEOUT_MS must be a positive integer')

  const malware = []
  for (let offset = 0; offset < packages.length; offset += batchSize) {
    const batch = packages.slice(offset, offset + batchSize)
    let response
    try {
      response = await fetch(endpoint, {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          'user-agent': 'llama-server-manager-supply-chain-gate',
        },
        body: JSON.stringify({
          queries: batch.map(({ name, version }) => ({
            package: { ecosystem: 'npm', name },
            version,
          })),
        }),
        signal: AbortSignal.timeout(timeout),
      })
    } catch (error) {
      fail(`OSV malware query failed closed: ${error.message}`)
    }

    if (!response.ok) {
      fail(`OSV malware query failed closed: HTTP ${response.status}`)
    }

    let body
    try {
      body = await response.json()
    } catch (error) {
      fail(`OSV malware query returned invalid JSON: ${error.message}`)
    }
    if (!Array.isArray(body.results) || body.results.length !== batch.length) {
      fail(`OSV malware query returned ${body.results?.length ?? 'no'} result(s) for ${batch.length} package(s)`)
    }

    body.results.forEach((result, index) => {
      const advisories = Array.isArray(result?.vulns) ? result.vulns.filter(isMalwareAdvisory) : []
      if (advisories.length === 0) return
      const npmPackage = batch[index]
      malware.push({
        identity: `${npmPackage.name}@${npmPackage.version}`,
        advisories: advisories.map(advisory => advisory.id).join(', '),
      })
    })
  }
  return malware
}

async function main() {
  const { installedOnly, lockfile, nodeModules } = parseArguments(process.argv.slice(2))
  const { packages, installScripts } = readLockedPackages(lockfile)
  validateLocalPolicy(packages, installScripts)

  if (installedOnly) {
    const scannedFiles = scanInstalledContent(nodeModules)
    console.log(`npm installed-content gate passed: ${scannedFiles} source file(s), 0 ChainDrop indicators.`)
  } else {
    const malware = await queryOsv(packages)
    if (malware.length > 0) {
      const details = malware.map(result => `${result.identity} [${result.advisories}]`)
      fail(`OSV malware advisory match(es): ${details.join('; ')}`)
    }

    console.log(
      `npm supply-chain gate passed: ${packages.length} exact package version(s), `
      + `${installScripts.length} reviewed lifecycle-script entr${installScripts.length === 1 ? 'y' : 'ies'}, 0 malware advisories.`,
    )
  }
}

main().catch(error => {
  console.error(`npm supply-chain gate failed: ${error.message}`)
  process.exitCode = 1
})
