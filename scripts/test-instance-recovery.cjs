const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const root = path.join(__dirname, '..')
const read = relative => fs.readFileSync(path.join(root, relative), 'utf8')

const appSource = read('src/App.tsx')
assert.match(
  appSource,
  /startInstance:\s*id\s*=>\s*startInstance\(id, false\)/,
  'application auto-start must not claim an operator recovery override',
)

const instanceSliceSource = read('src/store/instanceSlice.ts')
assert.match(
  instanceSliceSource,
  /startInstance:\s*\(id, manualRecovery = true\)/,
  'interactive starts must default to an explicit operator recovery override',
)
assert.match(
  instanceSliceSource,
  /manualRecovery,/,
  'the operator recovery intent must cross the Tauri command boundary',
)

const protocolSource = read('src-tauri/src/runtime_service/protocol.rs')
assert.match(protocolSource, /INSTANCE_RECOVERY_BACKOFF_SECS: \[u64; 3\] = \[2, 10, 30\]/)
assert.match(protocolSource, /INSTANCE_RECOVERY_MAX_ATTEMPTS: u32 = 3/)
assert.match(protocolSource, /RUNTIME_STATE_SCHEMA_VERSION: u32 = 4/)
assert.match(protocolSource, /default_manual_recovery\(\) -> bool \{\s*true/)
assert.match(protocolSource, /pub launch_config_stale: bool/)
assert.match(protocolSource, /pub deployment_identity: crate::deployment_identity::DeploymentIdentity/)
assert.match(protocolSource, /pub deployment_revision: crate::deployment::DeploymentRevision/)
assert.match(protocolSource, /DEPLOYMENT_REVISION_CAPABILITY: &str = "deployment_revision_v1"/)
assert.match(protocolSource, /LEGACY_QUALIFICATION_WIRE_PROFILE_VERSION: u8 = 2/)

const serverSource = read('src-tauri/src/commands/server.rs')
assert.match(
  serverSource,
  /engine_qualification_profile_version:\s*crate::runtime_service::protocol::LEGACY_QUALIFICATION_WIRE_PROFILE_VERSION/,
  'an updated GUI must remain compatible with an already-running profile-2 daemon',
)
assert.match(
  serverSource,
  /resolve_scanned_model_identity\([\s\S]*?launch_command\.is_some\(\)/,
  'deployment inspection must reuse verified model identity instead of hashing the whole model',
)
assert.match(
  serverSource,
  /if inventory_identity\.is_verified\(\) \{\s*return Ok\(inventory_identity\);\s*\}[\s\S]*?artifact_identity_for_path\(\s*"engine"/,
  'deployment inspection must reuse a verified engine bundle identity before considering a full bundle hash',
)

const recoveryPanelSource = read('src/components/InstanceManager/InstanceRecoveryPanel.tsx')
assert.match(recoveryPanelSource, /instance\.config\.restart_policy === 'on-failure'/)
assert.match(
  recoveryPanelSource,
  /automaticRecoveryEnabled \? labels\.recoveryStatus : labels\.runtimeFailureStatus/,
  'a failure with automatic recovery disabled must not be labelled as a recovery incident',
)
assert.match(
  recoveryPanelSource,
  /automaticRecoveryEnabled \? labels\.cancelRecovery : labels\.dismissFailure/,
  'the terminal incident action must describe clearing a plain failure when recovery is disabled',
)

const supervisorSource = read('src-tauri/src/runtime_service/supervisor.rs')
assert.match(supervisorSource, /InstanceRecoveryPhase::CrashLoop/)
assert.match(supervisorSource, /origin_failure/)
assert.match(supervisorSource, /recovery_budget_is_stable/)
assert.match(supervisorSource, /clear_stable_instance_recovery/)
assert.match(supervisorSource, /automatic start skipped/)
assert.match(
  supervisorSource,
  /bind_launch_artifacts\([\s\S]*?validate_runtime_engine_qualification\(&spec, &artifact_leases\)\?[\s\S]*?state\.instances\.get\(&spec\.instance_id\)[\s\S]*?validate_runtime_deployment_identity\(&spec, &artifact_leases, &persisted_config\)\?/,
  'runtime recovery must bind verified artifacts and the synchronized persisted configuration before deployment checks',
)
assert.match(supervisorSource, /validate_runtime_deployment_revision\(&spec, &proxy_config\)\?/)
assert.match(
  supervisorSource,
  /spec\.launch_config_stale =\s*!runtime_deployment_config_matches/,
  'saving a deployment-identity-affecting edit must invalidate the old recovery command snapshot',
)
assert.match(
  supervisorSource,
  /!spec\.launch_config_stale\s*&&/,
  'automatic recovery must reject a stale launch snapshot',
)

const entry = `
  import assert from 'node:assert/strict'
  import { instanceStatusFromRecovery, isAutoStartEligible } from './src/store/instanceRecovery'

  const failure = {
    kind: 'unexpected_exit', message: 'boom', exit_code: 1, occurred_at: 100,
  }
  const recovery = phase => ({
    phase, restart_attempts: 0, max_restart_attempts: 3,
    next_retry_at: null, origin_failure: failure, last_failure: failure,
  })
  assert.equal(instanceStatusFromRecovery(recovery('waiting')), 'recovering')
  assert.equal(instanceStatusFromRecovery(recovery('monitoring')), 'recovering')
  assert.equal(instanceStatusFromRecovery(recovery('restoring')), 'recovering')
  assert.equal(instanceStatusFromRecovery(recovery('failed')), 'error')
  assert.equal(instanceStatusFromRecovery(recovery('crash_loop')), 'crash_loop')

  const instance = status => ({ status, config: { auto_start: true } })
  assert.equal(isAutoStartEligible(instance('stopped')), true)
  assert.equal(isAutoStartEligible(instance('recovering')), false)
  assert.equal(isAutoStartEligible(instance('crash_loop')), false)
  assert.equal(isAutoStartEligible(instance('error')), false)
`

const bundled = esbuild.buildSync({
  stdin: {
    contents: entry,
    resolveDir: root,
    sourcefile: 'instance-recovery-test.ts',
    loader: 'ts',
  },
  bundle: true,
  platform: 'node',
  format: 'cjs',
  target: 'node18',
  write: false,
})

const testModule = new Module(path.join(__dirname, 'instance-recovery-test.cjs'))
testModule.filename = path.join(__dirname, 'instance-recovery-test.cjs')
testModule.paths = Module._nodeModulePaths(root)
testModule._compile(bundled.outputFiles[0].text, testModule.filename)

console.log('instance recovery regression passed')
