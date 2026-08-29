const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const checkpointSource = fs.readFileSync(
  path.join(process.cwd(), 'src-tauri', 'src', 'checkpoint.rs'),
  'utf8',
)
assert.match(checkpointSource, /CheckpointReasonCode/, 'backend must expose stable checkpoint reason codes')
assert.match(checkpointSource, /ModelArchitectureUnknown/, 'unknown model state must remain conservative')
assert.match(checkpointSource, /EngineCapabilityMissing/, 'engine capability must be part of eligibility')

const entry = `
  import assert from 'node:assert/strict'
  import { defaultInstanceConfig } from './src/store/defaults'
  import { migrateParameterIntent } from './src/parameterIntent'
  import { normalizeInstanceConfig } from './src/modelPolicy'
  import { getConfigChanges } from './src/components/ConfigPage/configWorkspace'

  const defaults = defaultInstanceConfig()
  assert.deepEqual(defaults.kv_checkpoint, {
    enabled: false,
    auto_save: true,
    auto_restore: true,
    storage_limit_gib: 8,
    minimum_prompt_tokens: 256,
  })

  const legacy = { ...defaults, explicit_overrides: null }
  delete legacy.kv_checkpoint
  const migrated = migrateParameterIntent(legacy)
  assert.deepEqual(migrated.kv_checkpoint, defaults.kv_checkpoint)
  assert.equal(migrated.explicit_overrides.includes('kv_checkpoint'), false)

  const vector = normalizeInstanceConfig({
    ...defaults,
    embedding: true,
    kv_checkpoint: { ...defaults.kv_checkpoint, enabled: true },
  }, null)
  assert.deepEqual(vector.config.kv_checkpoint, defaults.kv_checkpoint)

  const translations = { configPage: {} }
  const labels = { emptyValue: 'empty', on: 'on', off: 'off' }
  const equivalent = {
    ...defaults,
    kv_checkpoint: { ...defaults.kv_checkpoint },
  }
  assert.deepEqual(getConfigChanges(equivalent, defaults, translations, labels), [])

  console.log('kv checkpoint contract regression tests passed')
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
    sourcefile: 'kv-checkpoint.test.ts',
    loader: 'ts',
  },
})

const testModule = new Module(path.join(process.cwd(), 'kv-checkpoint.test.cjs'))
testModule.filename = path.join(process.cwd(), 'kv-checkpoint.test.cjs')
testModule.paths = Module._nodeModulePaths(process.cwd())
testModule._compile(bundled.outputFiles[0].text, testModule.filename)
