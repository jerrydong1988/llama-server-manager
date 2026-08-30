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
assert.match(checkpointSource, /ModelArtifactsIncomplete/, 'incomplete GGUF shard sets must disable checkpoint restore')
assert.match(checkpointSource, /CHECKPOINT_FINGERPRINT_SCHEMA_VERSION: u32 = 3/, 'draft-aware aggregate artifact fingerprints must use schema v3')
assert.match(checkpointSource, /spec_type: normalize_speculative_types/, 'speculative combinations must be canonical fingerprint material')
assert.match(checkpointSource, /draft_model_sha256/, 'external draft model contents must be fingerprint material')
assert.match(checkpointSource, /spec_draft_backend_sampling/, 'draft context and speculative settings must be fingerprint material')
assert.match(checkpointSource, /engine_artifact_sha256/, 'checkpoint compatibility must cover adjacent engine runtime libraries')
assert.match(checkpointSource, /"qwen4exp"/, 'hybrid recurrent qwen4exp models must fail open to a cold start')
assert.match(checkpointSource, /fingerprints-v1\.json/, 'full content fingerprints must use a versioned cache')
assert.match(checkpointSource, /HASH_CACHE_SCHEMA_VERSION: u32 = 2/, 'persistent hashes must bind to stronger file identity metadata')
assert.match(checkpointSource, /\.pending-/, 'generation commits must stage through a pending directory')
assert.match(checkpointSource, /CHECKPOINT_SLOT_OPERATION_TIMEOUT[^\n]*30 \* 60/, 'large slot operations must receive a long-running timeout')
assert.doesNotMatch(checkpointSource, /\.min\(Duration::from_secs\(30\)\)/, 'slot operation timeouts must not be clamped to 30 seconds')
assert.match(checkpointSource, /fs::rename\(scratch_payload, &destination\)/, 'generation commits must move scratch payloads without duplicating them')
assert.match(checkpointSource, /ensure_scratch_capacity/, 'known-size checkpoint staging must preflight free disk space')
assert.match(checkpointSource, /AfterManifestWrite/, 'storage tests must inject a manifest-last failure')
for (const faultPoint of [
  'AfterPayloadMove',
  'AfterPayloadSync',
  'AfterManifestWrite',
  'BeforeGenerationRename',
  'BeforeLatestUpdate',
]) {
  assert.match(checkpointSource, new RegExp(`StoreFaultPoint::${faultPoint}`), `missing fault injection: ${faultPoint}`)
}
assert.match(checkpointSource, /FILE_ATTRIBUTE_REPARSE_POINT/, 'Windows reparse points must be rejected')
assert.match(checkpointSource, /trait SlotBackend/, 'slot lifecycle tests must use an injectable backend')
assert.match(checkpointSource, /gate_allows_routing/, 'checkpoint restore must own a routing gate')
assert.match(checkpointSource, /verified_sha256 != slot\.sha256/, 'restore must verify a save round trip')
assert.match(checkpointSource, /StaleProcessEvent/, 'checkpoint events must be bound to the active process')

const rustEnumValues = (name) => {
  const body = checkpointSource.match(new RegExp(`pub enum ${name} \\{([\\s\\S]*?)\\n\\}`))?.[1] || ''
  return [...body.matchAll(/^\s+([A-Z][A-Za-z0-9]+),\s*$/gm)].map(match => (
    match[1].replace(/([a-z0-9])([A-Z])/g, '$1_$2').toLowerCase()
  ))
}
const backendReasonCodes = rustEnumValues('CheckpointReasonCode')
const backendPhases = rustEnumValues('CheckpointPhase')
assert.ok(backendReasonCodes.length > 20, 'backend reason-code contract must be discoverable')
assert.ok(backendPhases.length > 5, 'backend phase contract must be discoverable')

const runtimeProtocolSource = fs.readFileSync(
  path.join(process.cwd(), 'src-tauri', 'src', 'runtime_service', 'protocol.rs'),
  'utf8',
)
assert.match(runtimeProtocolSource, /KV_CHECKPOINT_CAPABILITY/, 'runtime protocol must advertise checkpoint ownership')
assert.match(runtimeProtocolSource, /RuntimeCheckpointLaunchSpec/, 'runtime launches must carry checkpoint eligibility and fingerprint state')
assert.match(runtimeProtocolSource, /ClearCheckpoint/, 'runtime protocol must expose exact-instance checkpoint clear')

const runtimeSupervisorSource = fs.readFileSync(
  path.join(process.cwd(), 'src-tauri', 'src', 'runtime_service', 'supervisor.rs'),
  'utf8',
)
assert.match(runtimeSupervisorSource, /prepare_runtime_checkpoint_launch/, 'runtime must validate private checkpoint launch metadata')
assert.match(runtimeSupervisorSource, /resolve_checkpoint_startup/, 'runtime must resolve restore before routing')
assert.match(runtimeSupervisorSource, /checkpoint_before_termination/, 'runtime must save at the controlled stop boundary')
assert.match(runtimeSupervisorSource, /gate_allows_routing/, 'runtime proxy paths must enforce the checkpoint gate')
assert.match(runtimeSupervisorSource, /retry_failed_restore_cleanup/, 'runtime must retry erase before cold routing')

const serverSource = fs.readFileSync(
  path.join(process.cwd(), 'src-tauri', 'src', 'commands', 'server.rs'),
  'utf8',
)
assert.match(serverSource, /retry_failed_restore_cleanup/, 'direct lifecycle must retry erase before cold routing')

const functionSlice = (source, start, end) => {
  const startIndex = source.indexOf(start)
  assert.notEqual(startIndex, -1, `missing lifecycle boundary: ${start}`)
  const endIndex = source.indexOf(end, startIndex + start.length)
  assert.notEqual(endIndex, -1, `missing lifecycle boundary: ${end}`)
  return source.slice(startIndex, endIndex)
}
const assertOrdered = (source, markers, label) => {
  let cursor = -1
  for (const marker of markers) {
    const next = source.indexOf(marker, cursor + 1)
    assert.ok(next > cursor, `${label} must order ${markers.join(' -> ')}`)
    cursor = next
  }
}
assertOrdered(
  functionSlice(serverSource, 'fn checkpoint_before_termination_blocking(', 'async fn checkpoint_before_termination('),
  ['begin_draining', 'target_snapshot', 'save_before_stop'],
  'direct checkpoint stop',
)
assertOrdered(
  functionSlice(serverSource, 'pub async fn stop_server(', 'pub async fn test_connection('),
  ['checkpoint_before_termination', 'terminate_running_instance'],
  'direct process stop',
)
assertOrdered(
  functionSlice(runtimeSupervisorSource, 'fn checkpoint_before_termination(', 'fn stop_instance_with_mode('),
  ['begin_draining', 'target_snapshot', 'save_before_stop'],
  'runtime checkpoint stop',
)
assertOrdered(
  functionSlice(runtimeSupervisorSource, 'fn stop_instance_locked(', 'pub fn stop_all_instances('),
  ['checkpoint_before_termination', 'terminate_running_instance'],
  'runtime process stop',
)
assertOrdered(
  functionSlice(checkpointSource, 'pub fn restore_or_cold', 'pub fn save_before_stop'),
  ['backend.restore', 'remove_scratch_payload', 'ensure_scratch_capacity', 'backend.save'],
  'restore scratch lifecycle',
)
const runtimeCommandHandler = functionSlice(
  runtimeSupervisorSource,
  'pub async fn handle_command(',
  'impl ProxyDataSource for RuntimeSupervisor',
)
assert.match(
  functionSlice(runtimeCommandHandler, 'RuntimeCommand::StopInstance', 'RuntimeCommand::ClearCheckpoint'),
  /spawn_blocking/,
  'runtime checkpoint stop must not construct or drop a blocking HTTP client on the async control task',
)
assert.match(
  functionSlice(runtimeCommandHandler, 'RuntimeCommand::Shutdown', '\n            }\n        }'),
  /spawn_blocking/,
  'runtime shutdown must isolate checkpoint stop work from the async control task',
)

const proxySource = fs.readFileSync(
  path.join(process.cwd(), 'src-tauri', 'src', 'commands', 'proxy.rs'),
  'utf8',
)
assert.match(proxySource, /checkpoint_unavailable_response/, 'gated routes must return a checkpoint-aware retry response')
assert.match(proxySource, /retry-after/, 'checkpoint readiness must be explicitly retryable')
assert.match(proxySource, /checkpoint_gate_keeps_another_matching_ready_target_routable/, 'checkpoint gating must retain healthy failover targets')

const persistenceSource = fs.readFileSync(
  path.join(process.cwd(), 'src-tauri', 'src', 'persistence.rs'),
  'utf8',
)
assert.match(persistenceSource, /windows_wide_path/, 'Windows atomic writes must support deep checkpoint paths')

const readDoc = relative => fs.readFileSync(path.join(process.cwd(), relative), 'utf8')
const readme = readDoc('README.md')
const guide = readDoc('GUIDE.md')
for (const [name, document] of [['README', readme], ['GUIDE', guide]]) {
  assert.doesNotMatch(document, /纯 n-gram|ngram-only speculation|limited to engine-reported `ngram-\*` types/, `${name} must not claim checkpointing is ngram-only`)
  assert.match(document, /target\/draft|target\/draft context/, `${name} must describe the explicit draft-context capability gate`)
}

const checkpointCommandSource = fs.readFileSync(
  path.join(process.cwd(), 'src-tauri', 'src', 'commands', 'checkpoint.rs'),
  'utf8',
)
assert.match(checkpointCommandSource, /list_checkpoint_statuses/, 'GUI must hydrate checkpoint status')
assert.match(checkpointCommandSource, /get_checkpoint_status/, 'GUI must support exact checkpoint status lookup')
assert.match(checkpointCommandSource, /reserve_checkpoint_clear/, 'direct clear must serialize against instance start')

const entry = `
  import assert from 'node:assert/strict'
  import { defaultInstanceConfig } from './src/store/defaults'
  import { migrateParameterIntent } from './src/parameterIntent'
  import { normalizeInstanceConfig } from './src/modelPolicy'
  import { getConfigChanges } from './src/components/ConfigPage/configWorkspace'
  import {
    CHECKPOINT_PHASES,
    canClearCheckpoint,
    checkpointPhaseLabel,
    checkpointReasonLabel,
    formatCheckpointBytes,
  } from './src/checkpointView'
  import { enUS } from './src/i18n/en-US'
  import { zhCN } from './src/i18n/zh-CN'

  const backendReasonCodes = ${JSON.stringify(backendReasonCodes)}
  const backendPhases = ${JSON.stringify(backendPhases)}

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

  for (const phase of CHECKPOINT_PHASES) {
    assert.ok(checkpointPhaseLabel(phase, enUS.checkpoint))
    assert.ok(checkpointPhaseLabel(phase, zhCN.checkpoint))
  }
  assert.deepEqual([...CHECKPOINT_PHASES], backendPhases)
  for (const reason of backendReasonCodes) {
    assert.ok(enUS.checkpoint.reasons[reason], \`missing en-US checkpoint reason: \${reason}\`)
    assert.ok(zhCN.checkpoint.reasons[reason], \`missing zh-CN checkpoint reason: \${reason}\`)
  }
  const checkpoint = {
    instance_id: 'instance-1', phase: 'stopped', routable: false,
    last_operation: 'restore', last_outcome: 'failed', reason_code: 'checksum_mismatch',
    message: '', updated_at: 1,
  }
  assert.equal(checkpointReasonLabel(checkpoint, enUS.checkpoint), enUS.checkpoint.reasons.checksum_mismatch)
  assert.equal(formatCheckpointBytes(1024 * 1024), '1.00 MiB')
  assert.equal(canClearCheckpoint('stopped', undefined, checkpoint), true)
  assert.equal(canClearCheckpoint('running', undefined, checkpoint), false)
  assert.equal(canClearCheckpoint('stopped', 'stopping', checkpoint), false)
  assert.equal(canClearCheckpoint('stopped', undefined, { ...checkpoint, phase: 'saving' }), false)

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
