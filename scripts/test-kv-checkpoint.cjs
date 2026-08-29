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
assert.match(checkpointSource, /fingerprints-v1\.json/, 'full content fingerprints must use a versioned cache')
assert.match(checkpointSource, /\.pending-/, 'generation commits must stage through a pending directory')
assert.match(checkpointSource, /AfterManifestWrite/, 'storage tests must inject a manifest-last failure')
assert.match(checkpointSource, /FILE_ATTRIBUTE_REPARSE_POINT/, 'Windows reparse points must be rejected')
assert.match(checkpointSource, /trait SlotBackend/, 'slot lifecycle tests must use an injectable backend')
assert.match(checkpointSource, /gate_allows_routing/, 'checkpoint restore must own a routing gate')
assert.match(checkpointSource, /verified_sha256 != slot\.sha256/, 'restore must verify a save round trip')
assert.match(checkpointSource, /StaleProcessEvent/, 'checkpoint events must be bound to the active process')

const runtimeProtocolSource = fs.readFileSync(
  path.join(process.cwd(), 'src-tauri', 'src', 'runtime_service', 'protocol.rs'),
  'utf8',
)
assert.match(runtimeProtocolSource, /KV_CHECKPOINT_CAPABILITY/, 'runtime protocol must advertise checkpoint ownership')
assert.match(runtimeProtocolSource, /RuntimeCheckpointLaunchSpec/, 'runtime launches must carry checkpoint eligibility and fingerprint state')

const runtimeSupervisorSource = fs.readFileSync(
  path.join(process.cwd(), 'src-tauri', 'src', 'runtime_service', 'supervisor.rs'),
  'utf8',
)
assert.match(runtimeSupervisorSource, /prepare_runtime_checkpoint_launch/, 'runtime must validate private checkpoint launch metadata')
assert.match(runtimeSupervisorSource, /resolve_checkpoint_startup/, 'runtime must resolve restore before routing')
assert.match(runtimeSupervisorSource, /checkpoint_before_termination/, 'runtime must save at the controlled stop boundary')
assert.match(runtimeSupervisorSource, /gate_allows_routing/, 'runtime proxy paths must enforce the checkpoint gate')

const persistenceSource = fs.readFileSync(
  path.join(process.cwd(), 'src-tauri', 'src', 'persistence.rs'),
  'utf8',
)
assert.match(persistenceSource, /windows_wide_path/, 'Windows atomic writes must support deep checkpoint paths')

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
