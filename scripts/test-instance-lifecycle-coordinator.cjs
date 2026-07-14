const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const instanceSliceSource = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'store', 'instanceSlice.ts'),
  'utf8',
)
const startSection = instanceSliceSource.slice(
  instanceSliceSource.indexOf('startInstance:'),
  instanceSliceSource.indexOf('stopInstance:'),
)
const stopSection = instanceSliceSource.slice(
  instanceSliceSource.indexOf('stopInstance:'),
  instanceSliceSource.indexOf('openBrowser:'),
)
for (const [label, source] of [['start', startSection], ['stop', stopSection]]) {
  assert.match(source, /addRuntimeWarning\(/, `${label} failures must be visible to the user`)
  assert.match(source, /throw error/, `${label} failures must reject their caller`)
}

const serverSource = fs.readFileSync(
  path.join(__dirname, '..', 'src-tauri', 'src', 'commands', 'server.rs'),
  'utf8',
)
const startServerSource = serverSource.slice(
  serverSource.indexOf('pub async fn start_server'),
  serverSource.indexOf('fn monitor_loop'),
)
assert.ok(
  startServerSource.indexOf('reserve_instance_start') < startServerSource.indexOf('CappedLogWriter::new'),
  'backend start reservation must happen before the log is opened or a process is spawned',
)

const entry = `
  import assert from 'node:assert/strict'
  import { runInstanceStart } from './src/store/instanceLifecycleCoordinator'

  async function run() {
    let calls = 0
    let release
    const operation = () => {
      calls += 1
      return new Promise(resolve => { release = resolve })
    }

    const first = runInstanceStart('instance-a', operation)
    const second = runInstanceStart('instance-a', operation)
    assert.strictEqual(second, first)
    assert.equal(calls, 1)
    release()
    await Promise.all([first, second])

    let attempts = 0
    await assert.rejects(
      runInstanceStart('instance-b', async () => {
        attempts += 1
        throw new Error('expected start failure')
      }),
      /expected start failure/,
    )
    await runInstanceStart('instance-b', async () => { attempts += 1 })
    assert.equal(attempts, 2, 'a failed start must release its single-flight slot')
  }

  module.exports = run()
`

const bundled = esbuild.buildSync({
  stdin: {
    contents: entry,
    resolveDir: path.join(__dirname, '..'),
    sourcefile: 'instance-lifecycle-coordinator-test.ts',
    loader: 'ts',
  },
  bundle: true,
  platform: 'node',
  format: 'cjs',
  target: 'node18',
  write: false,
})

const testModule = new Module(path.join(__dirname, 'instance-lifecycle-coordinator-test.cjs'))
testModule.filename = path.join(__dirname, 'instance-lifecycle-coordinator-test.cjs')
testModule.paths = Module._nodeModulePaths(path.join(__dirname, '..'))
testModule._compile(bundled.outputFiles[0].text, testModule.filename)

Promise.resolve(testModule.exports)
  .then(() => console.log('instance lifecycle coordinator regression passed'))
  .catch((error) => {
    console.error(error)
    process.exitCode = 1
  })
