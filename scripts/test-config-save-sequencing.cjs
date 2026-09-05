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
assert.match(configPageSource, /const warningTone = warningCounts\.high > 0 \? 'red' : warningCounts\.medium > 0 \? 'amber' : warningCounts\.low > 0 \? 'sky' : 'emerald'/)
assert.match(configPageSource, /label: labels\.warnings[\s\S]*tone: warningTone === 'red'[\s\S]*warningTone === 'sky'/)
assert.match(configPageSource, /message\.tone === 'amber'[\s\S]*bg-sky-50/)
assert.match(configPageSource, /saving, saveStage, setSaveStage } = useConfigDraft/)
assert.match(
  configPageSource,
  /useEffect\(\(\) => \{\s*mountedRef\.current = true\s*return \(\) => \{\s*mountedRef\.current = false/,
  'StrictMode effect replay must restore the mounted flag',
)
assert.match(configPageSource, /saveInFlightRef\.current = true[\s\S]*await saveConfig\(\)[\s\S]*finally[\s\S]*saveInFlightRef\.current = false/)
assert.match(
  configPageSource,
  /clearTimeout\(saveFeedbackTimerRef\.current\)[\s\S]*saveFeedbackTimerRef\.current = setTimeout/,
  'a newer save must replace the previous feedback timer',
)
assert.match(
  configPageSource,
  /const targetIsActive = \(\) => mountedRef\.current[\s\S]*if \(!targetIsActive\(\)\) return[\s\S]*setSaved\(true\)/,
  'a completed save must not update feedback for another instance',
)
const saveDisabledLine = configPageSource
  .split(/\r?\n/)
  .find(line => line.includes('const saveDisabled ='))
assert.ok(saveDisabledLine, 'the parameter page must define one shared save-disabled state')
for (const guard of [
  '!inst',
  'saving',
  '!manualMode',
  'probingEngineCompatibility',
  'capabilityProbeRequired',
  'unsupportedEngineFlags.length > 0',
]) {
  assert.ok(saveDisabledLine.includes(guard), `the shared save-disabled state must retain ${guard}`)
}
assert.equal(
  (configPageSource.match(/disabled=\{saveDisabled\}/g) ?? []).length,
  2,
  'the top and floating save actions must share the same disabled state',
)
assert.match(configPageSource, /saving \? savingLabel :/)
assert.match(configPageSource, /preflight-reused/)
assert.match(configPageSource, /setSaveStage\('validating'\)[\s\S]*setSaveStage\('persisting'\)/)
assert.match(configPageSource, /editRevisionRef, saveInFlightRef[\s\S]*useConfigDraft\(configInstanceId\)/)
assert.match(
  configPageSource,
  /const saveRevision = editRevisionRef\.current[\s\S]*await runRevisionGuarded[\s\S]*if \(editRevisionRef\.current === saveRevision\)/,
  'a completed save must not replace local fields edited while persistence was in flight',
)
assert.ok(
  configPageSource.indexOf('saveInFlightRef.current = true') < configPageSource.indexOf('await runRevisionGuarded'),
  'the save button must lock before asynchronous compatibility validation starts',
)
assert.ok(
  configPageSource.indexOf('const saveRevision = editRevisionRef.current')
    < configPageSource.indexOf('await runRevisionGuarded'),
  'the save revision must be captured before asynchronous compatibility validation starts',
)

for (const locale of ['zh-CN.ts', 'en-US.ts']) {
  const localeSource = fs.readFileSync(path.join(__dirname, '..', 'src', 'i18n', locale), 'utf8')
  assert.match(localeSource, /saving:/, `${locale} must label the in-progress save state`)
  assert.match(localeSource, /validating:/, `${locale} must label compatibility validation`)
  assert.match(localeSource, /persisting:/, `${locale} must label durable persistence`)
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

assert.match(saveConfigBackend, /mark_config_sync_pending\(\)/)
assert.doesNotMatch(
  saveConfigBackend,
  /sync_app_config\(&state\)\s*\.await/,
  'ordinary parameter saves must not wait for runtime service discovery or startup',
)

const runtimeEventsSource = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'store', 'runtimeEvents.ts'),
  'utf8',
)
assert.doesNotMatch(
  runtimeEventsSource,
  /state\.saveConfig\(\)/,
  'runtime events must not trigger redundant frontend configuration writes',
)

assert.match(
  configPageSource,
  /await saveConfig\(\)[\s\S]*useAppStore\.getState\(\)\.instances[\s\S]*setBaseline\(persistedConfig, normalized\.config\)/,
  'save feedback must use the backend-normalized configuration',
)
assert.match(
  configPageSource,
  /const previousSave =[\s\S]*catch \(error\)[\s\S]*updateInstance\(targetInstanceId, \{ config: previousSave\.config \}\)/,
  'a failed config persistence must restore the previously persisted store value',
)

const entry = `
  import assert from 'node:assert/strict'
  import { createLatestSaveCoordinator } from './src/store/configSaveCoordinator'
  import { runRevisionGuarded } from './src/components/ConfigPage/configSaveGuard'
  import { canReuseConfigPreflight, createConfigPreflightKey } from './src/components/ConfigPage/configPreflight'

  async function run() {

  const cachedConfig = { name: 'cached', port: 8080 } as any
  const cachedKey = createConfigPreflightKey(cachedConfig, 'llama-server')
  assert.equal(canReuseConfigPreflight(cachedKey, createConfigPreflightKey(cachedConfig, 'llama-server')), true)
  assert.equal(canReuseConfigPreflight(cachedKey, createConfigPreflightKey({ ...cachedConfig, port: 8081 }, 'llama-server')), false)

  const deferred = () => {
    let resolve
    let reject
    const promise = new Promise((resolvePromise, rejectPromise) => {
      resolve = resolvePromise
      reject = rejectPromise
    })
    return { promise, resolve, reject }
  }

  let editRevision = 7
  const preflightGate = deferred()
  const preflight = runRevisionGuarded(
    editRevision,
    () => editRevision,
    async () => {
      await preflightGate.promise
      return ['llama-server']
    },
  )
  editRevision += 1
  preflightGate.resolve()
  assert.deepEqual(
    await preflight,
    { stale: true },
    'an edit made while compatibility validation is pending must cancel the stale save',
  )

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
