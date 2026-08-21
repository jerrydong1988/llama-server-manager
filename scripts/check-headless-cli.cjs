const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const read = relative => fs.readFileSync(path.join(root, relative), 'utf8')
const errors = []

const packageJson = JSON.parse(read('package.json'))
const tauri = JSON.parse(read('src-tauri/tauri.conf.json'))
const workflow = read('.github/workflows/build.yml')
const cli = read('src-tauri/src/headless_cli.rs')
const binary = read('src-tauri/src/bin/lsm.rs')
const docs = read('docs/HEADLESS_CLI.md')
const cargo = read('src-tauri/Cargo.toml')

function requireValue(condition, message) {
  if (!condition) errors.push(message)
}

requireValue(
  !tauri.bundle?.externalBin?.length,
  'Tauri must package lsm only as a Cargo binary, not duplicate it as externalBin',
)
requireValue(
  tauri.build?.beforeBuildCommand === 'npm run build',
  'Tauri beforeBuildCommand must not create a duplicate CLI sidecar',
)
requireValue(binary.includes('headless_cli::run'), 'lsm binary must delegate to the CLI contract')
requireValue(binary.includes('is_runtime_service_invocation'), 'lsm binary must host the runtime service')
requireValue(
  cargo.includes('default-run = "llama-server-manager"'),
  'Cargo must keep the desktop application as the default Tauri binary',
)
requireValue(!cli.match(/--(?:api-key|token|credential)/i), 'CLI help must not accept credential arguments')
requireValue(
  (workflow.match(/npm run test:headless-cli/g) || []).length === 4,
  'all four platform build jobs must run the Headless CLI lifecycle test',
)
for (const marker of [
  'Verify Headless CLI in Windows package',
  'Verify Headless CLI in macOS package',
  'Verify Headless CLI in Linux x86_64 package',
  'Verify Headless CLI in Linux ARM64 package',
]) {
  requireValue(workflow.includes(marker), `workflow must contain: ${marker}`)
}
for (let code = 0; code <= 6; code += 1) {
  requireValue(docs.includes(`| ${code} |`), `Headless CLI docs must define exit code ${code}`)
}
for (const marker of ['schemaVersion', 'cross-process file lock', 'There is no token command-line option']) {
  requireValue(docs.includes(marker), `Headless CLI docs must contain: ${marker}`)
}
requireValue(
  packageJson.scripts?.['test:headless-cli'] === 'node scripts/test-headless-cli.cjs',
  'package.json must expose test:headless-cli',
)

if (errors.length > 0) {
  console.error(`Headless CLI contract check failed with ${errors.length} issue(s):`)
  for (const error of errors) console.error(`- ${error}`)
  process.exit(1)
}

console.log('Headless CLI contract check passed')
