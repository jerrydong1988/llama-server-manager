const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { spawnSync } = require('node:child_process')

const root = path.resolve(__dirname, '..')
const version = require('../package.json').version
const temporaryRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'llama-updater-test-'))
const inputRoot = path.join(temporaryRoot, 'input')
const outputRoot = path.join(temporaryRoot, 'output')

try {
  const fixtures = [
    ['windows', 'fixture-unsigned.exe'],
    ['windows', 'fixture-unsigned.msi'],
    ['macos', 'fixture.app.tar.gz'],
    ['linux-x86_64', 'fixture.AppImage'],
    ['linux-aarch64', 'fixture.AppImage'],
  ]
  for (const [directory, fileName] of fixtures) {
    const target = path.join(inputRoot, directory)
    fs.mkdirSync(target, { recursive: true })
    fs.writeFileSync(path.join(target, fileName), `${directory}\n`)
  }
  fs.writeFileSync(path.join(inputRoot, 'macos', 'platform.txt'), 'darwin-aarch64\n')

  const environment = {
    ...process.env,
    GITHUB_REF_NAME: `v${version}`,
    UPDATER_INPUT_DIR: inputRoot,
    UPDATER_OUTPUT_DIR: outputRoot,
    R2_PUBLIC_BASE_URL: 'https://updates.example.test',
  }
  const stage = spawnSync(process.execPath, ['scripts/prepare-updater-release.mjs', '--stage'], {
    cwd: root,
    env: environment,
    encoding: 'utf8',
  })
  assert.equal(stage.status, 0, stage.stderr || stage.stdout)

  const releaseRoot = path.join(outputRoot, 'releases', `v${version}`)
  const payloads = fs.readdirSync(releaseRoot)
  assert.equal(payloads.length, 5)
  for (const payload of payloads) {
    fs.writeFileSync(path.join(releaseRoot, `${payload}.sig`), `signature-${payload}\n`)
  }

  const manifestRun = spawnSync(process.execPath, ['scripts/prepare-updater-release.mjs', '--manifest'], {
    cwd: root,
    env: environment,
    encoding: 'utf8',
  })
  assert.equal(manifestRun.status, 0, manifestRun.stderr || manifestRun.stdout)

  const manifest = JSON.parse(fs.readFileSync(path.join(outputRoot, 'latest.json'), 'utf8'))
  assert.equal(manifest.version, version)
  assert.deepEqual(
    Object.keys(manifest.platforms).sort(),
    ['darwin-aarch64', 'linux-aarch64', 'linux-x86_64', 'windows-x86_64-msi', 'windows-x86_64-nsis'],
  )
  assert.match(manifest.platforms['windows-x86_64-nsis'].url, /-unsigned-setup\.exe$/)
  assert.match(manifest.platforms['windows-x86_64-msi'].url, /-unsigned\.msi$/)
  for (const entry of Object.values(manifest.platforms)) {
    assert.ok(entry.url.startsWith(`https://updates.example.test/releases/v${version}/`))
    assert.ok(entry.signature.startsWith('signature-'))
  }
} finally {
  fs.rmSync(temporaryRoot, { recursive: true, force: true })
}

console.log('updater release manifest regression passed')
