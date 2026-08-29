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
const instanceManagerSource = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'components', 'InstanceManager.tsx'),
  'utf8',
)
const deleteSection = instanceManagerSource.slice(
  instanceManagerSource.indexOf('const handleDelete'),
  instanceManagerSource.indexOf('const [copyFeedback'),
)
assert.ok(
  deleteSection.indexOf('await stopInstance(id)') < deleteSection.indexOf('deleteInstance(id)'),
  'deleting an instance must stop its backend process before removing the config',
)
assert.match(
  serverSource,
  /let ri = state\.running[\s\S]*if ri\.is_none\(\) \{\s*return Ok\(\(\)\)/,
  'stop_server must be idempotent so deletion can always coordinate with the backend',
)
const startServerSource = serverSource.slice(
  serverSource.indexOf('pub async fn start_server'),
  serverSource.indexOf('fn monitor_loop'),
)
assert.ok(
  startServerSource.indexOf('reserve_instance_start') < startServerSource.indexOf('CappedLogWriter::new'),
  'backend start reservation must happen before the log is opened or a process is spawned',
)
assert.ok(
  startServerSource.indexOf('apply_managed_checkpoint_arguments') < startServerSource.indexOf('Command::new(&cmd[0])'),
  'managed checkpoint arguments must be finalized before the process is spawned',
)
assert.ok(
  startServerSource.indexOf('register_start_with_context') < startServerSource.indexOf('running.insert('),
  'checkpoint routing must be gated before a running instance becomes visible',
)
const directStopSection = serverSource.slice(
  serverSource.indexOf('pub async fn stop_server('),
  serverSource.indexOf('fn is_recorded_process_alive'),
)
assert.ok(
  directStopSection.indexOf('checkpoint_before_termination') < directStopSection.indexOf('terminate_running_instance'),
  'direct stop must drain and save before terminating the process',
)
assert.match(serverSource, /resolve_checkpoint_startup[\s\S]*restore_or_cold/)
const proxySource = fs.readFileSync(
  path.join(__dirname, '..', 'src-tauri', 'src', 'commands', 'proxy.rs'),
  'utf8',
)
assert.match(
  proxySource,
  /checkpoint_coordinator[\s\S]*gate_allows_routing/,
  'proxy snapshots and request resolution must honor the checkpoint routing gate',
)
const runtimeServiceSource = fs.readFileSync(
  path.join(__dirname, '..', 'src-tauri', 'src', 'runtime_service', 'mod.rs'),
  'utf8',
)
const runtimeStartSection = runtimeServiceSource.slice(
  runtimeServiceSource.indexOf('pub async fn start_instance'),
  runtimeServiceSource.indexOf('pub async fn stop_instance'),
)
assert.match(runtimeStartSection, /call_recovering\(RuntimeCommand::StartInstance/)
assert.doesNotMatch(
  runtimeStartSection,
  /ensure_runtime_service\(\)\.await/,
  'warm starts must issue the requested runtime command without a redundant status preflight',
)
assert.doesNotMatch(
  stopSection,
  /is_instance_managed/,
  'stops must use the locally reconciled ownership set instead of an extra daemon status round trip',
)

const entry = `
  import assert from 'node:assert/strict'
  import { runInstanceStart, runInstanceStop } from './src/store/instanceLifecycleCoordinator'

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

    const order = []
    let releaseStart
    const start = runInstanceStart('instance-c', async () => {
      order.push('start-begin')
      await new Promise(resolve => { releaseStart = resolve })
      order.push('start-end')
    })
    const stop = runInstanceStop('instance-c', async () => { order.push('stop') })
    await Promise.resolve()
    assert.deepEqual(order, ['start-begin'], 'opposite lifecycle operations must not overlap')
    releaseStart()
    await Promise.all([start, stop])
    assert.deepEqual(order, ['start-begin', 'start-end', 'stop'])
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
