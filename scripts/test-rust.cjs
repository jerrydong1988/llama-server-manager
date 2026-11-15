// Use Cargo's supported artifact messages: its intermediate layout is not stable.
const assert = require('node:assert/strict')
const path = require('node:path')
const fs = require('node:fs')
const { spawn } = require('node:child_process')
const readline = require('node:readline')

const root = path.resolve(__dirname, '..')
const expectedManifests = {
  glib_macros: path.join(root, 'src-tauri/vendor/glib-macros-0.18.5/Cargo.toml'),
  gtk3_macros: path.join(root, 'src-tauri/vendor/gtk3-macros-0.18.2/Cargo.toml'),
}
const identity = value => {
  const absolute = path.resolve(value)
  return process.platform === 'win32' ? absolute.toLowerCase() : absolute
}

function collectMacroArtifact(message, artifacts) {
  const name = message.target?.name
  const expected = expectedManifests[name]
  if (message.reason !== 'compiler-artifact' || !expected
    || !message.target.kind?.includes('proc-macro')
    || typeof message.manifest_path !== 'string'
    || identity(message.manifest_path) !== identity(expected)) return
  const library = message.filenames?.find(file => /\.(dll|so|dylib)$/.test(file))
  if (library) artifacts[name] = library
}

function runCargo(args, env, artifacts) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.env.CARGO || 'cargo', args, {
      cwd: root, env, windowsHide: true,
      stdio: ['inherit', artifacts ? 'pipe' : 'inherit', 'inherit'],
    })
    child.on('error', reject)
    child.on('close', code => resolve(code ?? 1))
    if (artifacts) {
      readline.createInterface({ input: child.stdout }).on('line', line => {
        let message
        try { message = JSON.parse(line) } catch { return }
        collectMacroArtifact(message, artifacts)
        if (message.reason === 'compiler-message' && message.message?.rendered) {
          process.stderr.write(message.message.rendered)
        }
      })
    }
  })
}

async function main() {
  const args = process.argv.slice(2)
  const separator = args.indexOf('--')
  const compileArgs = separator < 0 ? args : args.slice(0, separator)
  assert.ok(!compileArgs.some(arg => /^(--no-run|--message-format|--manifest-path)(=|$)/.test(arg)), 'The test runner controls compilation and always executes tests')
  const common = ['test', '--manifest-path', path.join(root, 'src-tauri/Cargo.toml'), '--locked']
  const artifacts = {}
  const compiled = await runCargo([...common, ...compileArgs, '--no-run', '--message-format=json'], process.env, artifacts)
  if (compiled !== 0) return compiled
  for (const name of Object.keys(expectedManifests)) {
    assert.ok(artifacts[name] && fs.existsSync(artifacts[name]), `Cargo did not report the reviewed ${name} library`)
  }
  return runCargo([...common, ...args], {
    ...process.env, LSM_TEST_PROC_MACRO_ARTIFACTS: JSON.stringify(artifacts),
  })
}

module.exports = { collectMacroArtifact, expectedManifests }
if (require.main === module) main().then(code => { process.exitCode = code }).catch(error => {
  console.error(error.message)
  process.exitCode = 1
})
