const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const read = (...segments) => fs.readFileSync(path.join(process.cwd(), ...segments), 'utf8')
const capabilityBackend = read('src-tauri', 'src', 'commands', 'engine_capabilities.rs')
const scannerBackend = read('src-tauri', 'src', 'commands', 'scanner.rs')

assert.match(
  capabilityBackend,
  /let mut engines = state\.engines\.lock\(\)\.unwrap\(\);[\s\S]*model_inventory::update_engine_probe\(&probed\)[\s\S]*drop\(engines\)/,
  'a capability probe must persist its result before releasing the engine state lock',
)
assert.match(
  scannerBackend,
  /let mut state_engines = state\.engines\.lock\(\)\.unwrap\(\);[\s\S]*for engine in state_engines\.iter\(\)[\s\S]*model_inventory::update_engine_probe\(engine\)[\s\S]*state_engines\.clone\(\)/,
  'an engine scan must persist merged capabilities before releasing the engine state lock',
)

const entry = `
  import assert from 'node:assert/strict'
  import {
    getEngineCompatibilityMode,
    localizeEngineCapabilityError,
    normalizeEngineCapabilityStatus,
    normalizeEngineVersionStatus,
  } from './src/engineCapabilities'
  import { probeEngineBatch } from './src/engineProbeBatch'
  import { getEngineLabels } from './src/i18n/pageLabels'

  async function run() {
  const detected = {
    status: 'detected',
    supportedFlags: ['-m', '--temp'],
    helpHash: 'abc',
    executableFingerprint: '1:2',
  }
  assert.equal(normalizeEngineCapabilityStatus(undefined), 'unprobed')
  assert.equal(normalizeEngineVersionStatus(undefined), 'unprobed')
  assert.equal(normalizeEngineVersionStatus({ ...detected, versionStatus: 'unknown' }), 'unknown')
  assert.equal(getEngineCompatibilityMode(detected), 'full')
  assert.equal(getEngineCompatibilityMode({ ...detected, status: 'partial' }), 'recognized')
  assert.equal(getEngineCompatibilityMode({ ...detected, status: 'failed' }), 'minimal')
  const zhLabels = getEngineLabels('zh-CN')
  const enLabels = getEngineLabels('en-US')
  assert.equal(
    localizeEngineCapabilityError('engine executable changed; compatibility probe required', zhLabels),
    zhLabels.executableChanged,
  )
  assert.equal(
    localizeEngineCapabilityError('Engine executable changed; compatibility probe required.', zhLabels),
    zhLabels.executableChanged,
  )
  assert.equal(
    localizeEngineCapabilityError('engine executable changed; compatibility probe required', enLabels),
    enLabels.executableChanged,
  )
  assert.equal(
    localizeEngineCapabilityError(
      'engine executable changed while compatibility probing was in progress; probe again',
      zhLabels,
    ),
    zhLabels.executableChangedDuringProbe,
  )
  assert.equal(localizeEngineCapabilityError('access denied', zhLabels), 'access denied')

  let active = 0
  let peak = 0
  const processed = []
  const activeIds = new Set()
  const progress = []
  const batchResult = await probeEngineBatch(
    [
      { id: 'engine-a', name: 'Engine A' },
      { id: 'engine-b', name: 'Engine B' },
      { id: 'engine-c', name: 'Engine C' },
      { id: 'engine-d', name: 'Engine D' },
    ],
    async id => {
      active += 1
      peak = Math.max(peak, active)
      await new Promise(resolve => setTimeout(resolve, 5))
      processed.push(id)
      active -= 1
      if (id === 'engine-b') throw new Error('probe rejected')
    },
    {
      concurrency: 2,
      onProgress: value => progress.push(value),
      onActiveChange: (id, isActive) => {
        if (isActive) activeIds.add(id)
        else activeIds.delete(id)
      },
    },
  )
  assert.equal(peak, 2)
  assert.deepEqual(processed.sort(), ['engine-a', 'engine-b', 'engine-c', 'engine-d'])
  assert.deepEqual(batchResult, {
    total: 4,
    succeeded: 3,
    failed: 1,
    failures: [{ id: 'engine-b', name: 'Engine B', error: 'probe rejected' }],
  })
  assert.deepEqual(progress[0], { completed: 0, total: 4 })
  assert.deepEqual(progress.at(-1), { completed: 4, total: 4 })
  assert.equal(activeIds.size, 0)
  }

  run()
    .then(() => console.log('engine capability frontend regression tests passed'))
    .catch(error => {
      console.error(error)
      process.exitCode = 1
    })
`

const bundled = esbuild.buildSync({
  bundle: true,
  format: 'cjs',
  platform: 'node',
  packages: 'external',
  write: false,
  stdin: {
    contents: entry,
    resolveDir: process.cwd(),
    sourcefile: 'engine-capabilities.test.ts',
    loader: 'ts',
  },
})

const testModule = new Module(path.join(process.cwd(), 'engine-capabilities.test.cjs'))
testModule.filename = path.join(process.cwd(), 'engine-capabilities.test.cjs')
testModule.paths = Module._nodeModulePaths(process.cwd())
testModule._compile(bundled.outputFiles[0].text, testModule.filename)
