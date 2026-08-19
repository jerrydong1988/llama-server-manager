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
assert.match(protocolSource, /RUNTIME_STATE_SCHEMA_VERSION: u32 = 3/)
assert.match(protocolSource, /default_manual_recovery\(\) -> bool \{\s*true/)
assert.match(protocolSource, /pub launch_config_stale: bool/)
assert.match(protocolSource, /pub deployment_identity: crate::deployment_identity::DeploymentIdentity/)

const supervisorSource = read('src-tauri/src/runtime_service/supervisor.rs')
assert.match(supervisorSource, /InstanceRecoveryPhase::CrashLoop/)
assert.match(supervisorSource, /origin_failure/)
assert.match(supervisorSource, /recovery_budget_is_stable/)
assert.match(supervisorSource, /clear_stable_instance_recovery/)
assert.match(supervisorSource, /automatic start skipped/)
assert.match(supervisorSource, /validate_runtime_deployment_identity\(&spec\)\?/)
assert.match(
  supervisorSource,
  /spec\.launch_config_stale = !runtime_launch_config_matches/,
  'saving a launch-affecting edit must invalidate the old recovery command snapshot',
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
