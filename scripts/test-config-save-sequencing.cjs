const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const backendConfigSource = fs.readFileSync(
  path.join(__dirname, '..', 'src-tauri', 'src', 'commands', 'config.rs'),
  'utf8',
)
const saveConfigBackend = backendConfigSource.slice(
  backendConfigSource.indexOf('pub async fn save_config('),
  backendConfigSource.indexOf('pub async fn load_config('),
)
assert.ok(
  saveConfigBackend.indexOf('CONFIG_WRITE_LOCK.lock()')
    < saveConfigBackend.indexOf('state.running.lock()'),
  'runtime state must be sampled only after save_config owns the config write lock',
)

const configPageSource = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'components', 'ConfigPage.tsx'),
  'utf8',
)
assert.match(configPageSource, /const \[saving, setSaving\] = useState\(false\)/)
assert.match(
  configPageSource,
  /useEffect\(\(\) => \{\s*mountedRef\.current = true\s*return \(\) => \{\s*mountedRef\.current = false/,
  'StrictMode effect replay must restore the mounted flag',
)
assert.match(configPageSource, /setSaving\(true\)[\s\S]*await saveConfig\(\)[\s\S]*finally[\s\S]*setSaving\(false\)/)
assert.match(
  configPageSource,
  /clearTimeout\(saveFeedbackTimerRef\.current\)[\s\S]*saveFeedbackTimerRef\.current = setTimeout/,
  'a newer save must replace the previous feedback timer',
)
assert.match(
  configPageSource,
  /if \(!mountedRef\.current \|\| useAppStore\.getState\(\)\.activeConfigInstanceId !== inst\.id\) return[\s\S]*setSaved\(true\)/,
  'a completed save must not update feedback for another instance',
)
assert.match(configPageSource, /disabled=\{!inst \|\| saving\}/)
assert.match(configPageSource, /saving \? t\.configPage\.saving :/)

for (const locale of ['zh-CN.ts', 'en-US.ts']) {
  const localeSource = fs.readFileSync(path.join(__dirname, '..', 'src', 'i18n', locale), 'utf8')
  assert.match(localeSource, /saving:/, `${locale} must label the in-progress save state`)
}

const instanceSliceSource = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'store', 'instanceSlice.ts'),
  'utf8',
)
assert.match(instanceSliceSource, /createLatestSaveCoordinator<ConfigSaveSnapshot,\s*PersistedConfigResult>/)
assert.match(instanceSliceSource, /configSaveCoordinator\.save\(\{/)
assert.match(instanceSliceSource, /configSaveCoordinator\.waitForIdle\(\)/)
assert.match(instanceSliceSource, /result\.revision === latestConfigSaveRevision/)
assert.match(instanceSliceSource, /result\.revision > latestAppliedConfigSaveRevision/)
assert.match(instanceSliceSource, /Object\.keys\(result\.instances\)\.length > 0/)
assert.match(
  instanceSliceSource,
  /if \(revision === latestConfigSaveRevision\)[\s\S]*addRuntimeWarning/,
  'merged failures must report only the latest failed save',
)
assert.match(instanceSliceSource, /synchronizeInstanceSummary/)
assert.doesNotMatch(instanceSliceSource, /configSaveQueue/)

assert.match(
  configPageSource,
  /await saveConfig\(\)[\s\S]*useAppStore\.getState\(\)\.instances[\s\S]*setBaseline\(persistedConfig\)/,
  'save feedback must use the backend-normalized configuration',
)

const entry = `
  import assert from 'node:assert/strict'
  import { createLatestSaveCoordinator } from './src/store/configSaveCoordinator'

  async function run() {

  const deferred = () => {
    let resolve
    let reject
    const promise = new Promise((resolvePromise, rejectPromise) => {
      resolve = resolvePromise
      reject = rejectPromise
    })
    return { promise, resolve, reject }
  }

  const writes = []
  const gates = []
  const coordinator = createLatestSaveCoordinator(async (snapshot) => {
    writes.push(snapshot)
    const gate = deferred()
    gates.push(gate)
    await gate.promise
    return snapshot.revision
  })

  const first = coordinator.save({ revision: 1 })
  await Promise.resolve()
  assert.deepEqual(writes, [{ revision: 1 }])

  const second = coordinator.save({ revision: 2 })
  const third = coordinator.save({ revision: 3 })
  await Promise.resolve()
  assert.deepEqual(writes, [{ revision: 1 }], 'pending saves must not start concurrently')

  gates[0].resolve()
  assert.equal(await first, 1)
  await Promise.resolve()
  assert.deepEqual(
    writes,
    [{ revision: 1 }, { revision: 3 }],
    'bursts must persist only the latest pending snapshot',
  )

  gates[1].resolve()
  const [secondResult, thirdResult] = await Promise.all([second, third, coordinator.waitForIdle()])
  assert.equal(secondResult, 3)
  assert.equal(thirdResult, 3)

  const recoveryWrites = []
  const recovery = createLatestSaveCoordinator(async (snapshot) => {
    recoveryWrites.push(snapshot)
    if (snapshot.revision === 1) throw new Error('expected failure')
  })
  const failed = recovery.save({ revision: 1 }).then(
    () => null,
    (error) => error,
  )
  await Promise.resolve()
  const recovered = recovery.save({ revision: 2 })
  assert.match(String(await failed), /expected failure/)
  await recovered
  await recovery.waitForIdle()
  assert.deepEqual(recoveryWrites, [{ revision: 1 }, { revision: 2 }])

  const idleRecoveryWrites = []
  const idleRecovery = createLatestSaveCoordinator(async (snapshot) => {
    idleRecoveryWrites.push(snapshot)
    if (snapshot.revision === 1) throw new Error('idle failure')
    return snapshot.revision
  })
  const idleFailureSave = idleRecovery.save({ revision: 1 })
  const activeDrain = idleRecovery.waitForIdle()
  await assert.rejects(idleFailureSave, /idle failure/)
  await assert.rejects(activeDrain, /idle failure/)
  await idleRecovery.waitForIdle()
  assert.equal(await idleRecovery.save({ revision: 2 }), 2)
  await idleRecovery.waitForIdle()
  assert.deepEqual(idleRecoveryWrites, [{ revision: 1 }, { revision: 2 }])

  const mergedWrites = []
  const mergedFailure = createLatestSaveCoordinator(async (snapshot) => {
    mergedWrites.push(snapshot)
    throw new Error('merged failure')
  })
  const mergedFirst = mergedFailure.save({ revision: 1 })
  const mergedLatest = mergedFailure.save({ revision: 2 })
  const mergedResults = await Promise.allSettled([mergedFirst, mergedLatest])
  assert.deepEqual(mergedWrites, [{ revision: 2 }])
  assert.equal(mergedResults[0].status, 'rejected')
  assert.equal(mergedResults[1].status, 'rejected')
  assert.match(String(mergedResults[0].reason), /merged failure/)
  assert.match(String(mergedResults[1].reason), /merged failure/)

  }

  module.exports = run()
`

const bundled = esbuild.buildSync({
  stdin: {
    contents: entry,
    resolveDir: path.join(__dirname, '..'),
    sourcefile: 'config-save-sequencing-test.ts',
    loader: 'ts',
  },
  bundle: true,
  platform: 'node',
  format: 'cjs',
  target: 'node18',
  write: false,
})

const testModule = new Module(path.join(__dirname, 'config-save-sequencing-test.cjs'))
testModule.filename = path.join(__dirname, 'config-save-sequencing-test.cjs')
testModule.paths = Module._nodeModulePaths(path.join(__dirname, '..'))
testModule._compile(bundled.outputFiles[0].text, testModule.filename)

Promise.resolve(testModule.exports)
  .then(() => console.log('config save sequencing regression passed'))
  .catch((error) => {
    console.error(error)
    process.exitCode = 1
  })
