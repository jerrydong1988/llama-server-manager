const assert = require('node:assert/strict')
const fs = require('node:fs')
const http = require('node:http')
const os = require('node:os')
const path = require('node:path')
const { spawn } = require('node:child_process')

const root = path.resolve(__dirname, '..')
const checker = path.join(root, 'scripts', 'check-npm-supply-chain.cjs')
const temporaryRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'llama-npm-supply-chain-'))
let responseMode = 'safe'

function lockfile(packages) {
  return {
    name: 'supply-chain-test',
    lockfileVersion: 3,
    requires: true,
    packages,
  }
}

function writeFixture(name, contents) {
  const file = path.join(temporaryRoot, `${name}.json`)
  fs.writeFileSync(file, typeof contents === 'string' ? contents : `${JSON.stringify(contents, null, 2)}\n`, 'utf8')
  return file
}

function runChecker(lockPath, endpoint, extraArguments = []) {
  return new Promise(resolve => {
    const child = spawn(process.execPath, [checker, '--lockfile', lockPath, ...extraArguments], {
      cwd: root,
      env: {
        ...process.env,
        NPM_SUPPLY_CHAIN_OSV_URL: endpoint,
        NPM_SUPPLY_CHAIN_TIMEOUT_MS: '2000',
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.setEncoding('utf8')
    child.stderr.setEncoding('utf8')
    child.stdout.on('data', chunk => { stdout += chunk })
    child.stderr.on('data', chunk => { stderr += chunk })
    child.on('close', status => resolve({ status, stdout, stderr }))
  })
}

async function main() {
  const server = http.createServer((request, response) => {
    let requestBody = ''
    request.setEncoding('utf8')
    request.on('data', chunk => { requestBody += chunk })
    request.on('end', () => {
      if (responseMode === 'outage') {
        response.writeHead(503).end('unavailable')
        return
      }
      const parsed = JSON.parse(requestBody)
      const results = parsed.queries.map((query, index) => (
        responseMode === 'malware' && index === 0
          ? { vulns: [{ id: 'MAL-2099-00001' }] }
          : {}
      ))
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end(JSON.stringify({ results }))
    })
  })

  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  const endpoint = `http://127.0.0.1:${address.port}/v1/querybatch`

  try {
    const safeLock = writeFixture('safe', lockfile({
      '': { name: 'supply-chain-test' },
      'node_modules/esbuild': { version: '0.28.1', hasInstallScript: true },
      'node_modules/keyv': { version: '4.5.4' },
    }))
    responseMode = 'safe'
    const safe = await runChecker(safeLock, endpoint)
    assert.equal(safe.status, 0, safe.stderr)
    assert.match(safe.stdout, /2 exact package version\(s\)/)

    responseMode = 'malware'
    const liveMalware = await runChecker(safeLock, endpoint)
    assert.notEqual(liveMalware.status, 0)
    assert.match(liveMalware.stderr, /MAL-2099-00001/)

    const knownMalwareLock = writeFixture('known-malware', lockfile({
      '': { name: 'supply-chain-test' },
      'node_modules/keyv': { version: '6.0.0' },
    }))
    responseMode = 'safe'
    const knownMalware = await runChecker(knownMalwareLock, endpoint)
    assert.notEqual(knownMalware.status, 0)
    assert.match(knownMalware.stderr, /known malicious npm package version/)

    const unreviewedScriptLock = writeFixture('unreviewed-script', lockfile({
      '': { name: 'supply-chain-test' },
      'node_modules/example-package': { version: '1.0.0', hasInstallScript: true },
    }))
    const unreviewedScript = await runChecker(unreviewedScriptLock, endpoint)
    assert.notEqual(unreviewedScript.status, 0)
    assert.match(unreviewedScript.stderr, /unreviewed npm lifecycle script/)

    responseMode = 'outage'
    const outage = await runChecker(safeLock, endpoint)
    assert.notEqual(outage.status, 0)
    assert.match(outage.stderr, /failed closed: HTTP 503/)

    const malformed = writeFixture('malformed', '{not-json')
    const malformedResult = await runChecker(malformed, endpoint)
    assert.notEqual(malformedResult.status, 0)
    assert.match(malformedResult.stderr, /cannot read npm lockfile/)

    const safeNodeModules = path.join(temporaryRoot, 'safe-node-modules')
    fs.mkdirSync(path.join(safeNodeModules, 'example-package'), { recursive: true })
    fs.writeFileSync(path.join(safeNodeModules, 'example-package', 'index.js'), 'module.exports = true\n', 'utf8')
    const installedSafe = await runChecker(safeLock, endpoint, [
      '--installed-only',
      '--node-modules', safeNodeModules,
    ])
    assert.equal(installedSafe.status, 0, installedSafe.stderr)
    assert.match(installedSafe.stdout, /0 ChainDrop indicators/)

    const suspiciousNodeModules = path.join(temporaryRoot, 'suspicious-node-modules')
    fs.mkdirSync(path.join(suspiciousNodeModules, 'example-package'), { recursive: true })
    fs.writeFileSync(path.join(suspiciousNodeModules, 'example-package', 'math_init.js'), 'module.exports = true\n', 'utf8')
    const installedSuspicious = await runChecker(safeLock, endpoint, [
      '--installed-only',
      '--node-modules', suspiciousNodeModules,
    ])
    assert.notEqual(installedSuspicious.status, 0)
    assert.match(installedSuspicious.stderr, /ChainDrop file indicator/)
  } finally {
    await new Promise(resolve => server.close(resolve))
    fs.rmSync(temporaryRoot, { recursive: true, force: true })
  }

  console.log('npm supply-chain regression passed')
}

main().catch(error => {
  fs.rmSync(temporaryRoot, { recursive: true, force: true })
  console.error(error)
  process.exitCode = 1
})
