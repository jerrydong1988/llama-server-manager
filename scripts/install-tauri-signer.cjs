const crypto = require('node:crypto')
const fs = require('node:fs')
const path = require('node:path')
const process = require('node:process')
const { spawnSync } = require('node:child_process')

const root = path.resolve(__dirname, '..')
const lockfile = JSON.parse(fs.readFileSync(path.join(root, 'package-lock.json'), 'utf8'))
const targetPackages = new Map([
  ['darwin:arm64', '@tauri-apps/cli-darwin-arm64'],
  ['darwin:x64', '@tauri-apps/cli-darwin-x64'],
  ['linux:arm64', '@tauri-apps/cli-linux-arm64-gnu'],
  ['linux:x64', '@tauri-apps/cli-linux-x64-gnu'],
  ['win32:arm64', '@tauri-apps/cli-win32-arm64-msvc'],
  ['win32:ia32', '@tauri-apps/cli-win32-ia32-msvc'],
  ['win32:x64', '@tauri-apps/cli-win32-x64-msvc'],
])

function fail(message) {
  throw new Error(message)
}

function metadataFor(name) {
  const lockPath = `node_modules/${name}`
  const metadata = lockfile.packages?.[lockPath]
  if (!metadata) fail(`package-lock.json does not contain ${lockPath}`)
  if (typeof metadata.version !== 'string' || typeof metadata.resolved !== 'string' || typeof metadata.integrity !== 'string') {
    fail(`package-lock.json has incomplete download metadata for ${name}`)
  }
  const resolved = new URL(metadata.resolved)
  if (resolved.protocol !== 'https:' || resolved.hostname !== 'registry.npmjs.org') {
    fail(`refusing non-registry tarball for ${name}: ${metadata.resolved}`)
  }
  if (!metadata.integrity.startsWith('sha512-')) {
    fail(`refusing non-sha512 integrity for ${name}`)
  }
  return { ...metadata, name }
}

async function downloadAndVerify(metadata, archivePath) {
  let response
  try {
    response = await fetch(metadata.resolved, { signal: AbortSignal.timeout(30000) })
  } catch (error) {
    fail(`failed to download ${metadata.name}: ${error.message}`)
  }
  if (!response.ok) fail(`failed to download ${metadata.name}: HTTP ${response.status}`)

  const contents = Buffer.from(await response.arrayBuffer())
  const actual = crypto.createHash('sha512').update(contents).digest('base64')
  const expected = metadata.integrity.slice('sha512-'.length)
  if (!crypto.timingSafeEqual(Buffer.from(actual), Buffer.from(expected))) {
    fail(`sha512 integrity mismatch for ${metadata.name}@${metadata.version}`)
  }
  fs.writeFileSync(archivePath, contents)
}

function extract(archivePath, destination) {
  fs.mkdirSync(destination, { recursive: true })
  const result = spawnSync('tar', ['-xzf', archivePath, '--strip-components=1', '-C', destination], {
    encoding: 'utf8',
  })
  if (result.status !== 0) {
    fail(`failed to extract ${archivePath}: ${result.stderr || result.stdout}`)
  }
}

async function main() {
  const destination = process.argv[2] ? path.resolve(process.argv[2]) : ''
  if (!destination) fail('usage: node scripts/install-tauri-signer.cjs <empty-destination>')
  if (fs.existsSync(destination) && fs.readdirSync(destination).length > 0) {
    fail(`destination must be empty: ${destination}`)
  }

  const platformPackage = targetPackages.get(`${process.platform}:${process.arch}`)
  if (!platformPackage) fail(`unsupported signer platform: ${process.platform}/${process.arch}`)

  const packages = [metadataFor('@tauri-apps/cli'), metadataFor(platformPackage)]
  if (packages[0].version !== packages[1].version) {
    fail(`Tauri CLI wrapper ${packages[0].version} does not match platform binary ${packages[1].version}`)
  }

  fs.mkdirSync(destination, { recursive: true })
  for (const metadata of packages) {
    const archivePath = path.join(destination, `${metadata.name.replaceAll('/', '-')}-${metadata.version}.tgz`)
    const packageDirectory = path.join(destination, 'node_modules', ...metadata.name.split('/'))
    await downloadAndVerify(metadata, archivePath)
    extract(archivePath, packageDirectory)
    fs.rmSync(archivePath)
  }

  const cliPath = path.join(destination, 'node_modules', '@tauri-apps', 'cli', 'tauri.js')
  if (!fs.existsSync(cliPath)) fail(`Tauri signer entrypoint was not extracted: ${cliPath}`)
  console.log(`Installed integrity-verified Tauri CLI ${packages[0].version} at ${cliPath}`)
}

main().catch(error => {
  console.error(`Tauri signer installation failed: ${error.message}`)
  process.exitCode = 1
})
