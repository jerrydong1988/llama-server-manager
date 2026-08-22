const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const read = relative => fs.readFileSync(path.join(root, relative), 'utf8')
const errors = []

const packageJson = JSON.parse(read('package.json'))
const cargo = read('src-tauri/Cargo.toml')
const library = read('src-tauri/src/lib.rs')
const binary = read('src-tauri/src/bin/lsm.rs')
const agent = read('src-tauri/src/worker_agent.rs')
const commands = read('src-tauri/src/commands/worker_agent.rs')
const clusterCommands = read('src-tauri/src/commands/cluster.rs')
const models = read('src-tauri/src/models.rs')
const main = read('src-tauri/src/main.rs')
const persistence = read('src-tauri/src/persistence.rs')
const ui = read('src/components/ClusterPage/ClusterPage.tsx')
const workflow = read('.github/workflows/build.yml')
const docs = read('docs/WORKER_AGENT.md')

function requireValue(condition, message) {
  if (!condition) errors.push(message)
}

requireValue(library.includes('pub mod worker_agent;'), 'Rust library must export the Worker Agent')
requireValue(binary.includes('argument == "worker-agent"'), 'packaged lsm binary must route worker-agent commands')
requireValue(binary.includes('worker_agent::run_cli'), 'packaged lsm binary must invoke the Worker Agent CLI')
requireValue(cargo.includes('tokio-rustls'), 'Worker Agent must use TLS for its custom protocol')
requireValue(agent.includes('AGENT_PROTOCOL_VERSION: u32 = 1'), 'Agent protocol must be explicitly versioned')
for (const action of ['Status', 'RpcStart', 'RpcStop', 'Audit']) {
  requireValue(agent.includes(action), `Agent allow-list must contain ${action}`)
}
requireValue(agent.includes('#[serde(deny_unknown_fields)]'), 'Agent protocol must reject unknown fields')
requireValue(agent.includes('certificate_sha256'), 'Agent must pin a certificate fingerprint')
requireValue(agent.includes('protect_private_token'), 'Agent must protect file-backed credentials')
requireValue(
  agent.includes('crate::persistence::atomic_write(&token_path'),
  'Agent token creation must use private atomic persistence',
)
requireValue(
  persistence.includes('with_private_security_descriptor') &&
    persistence.includes('PROTECTED_DACL_SECURITY_INFORMATION') &&
    persistence.includes('lpSecurityDescriptor: descriptor'),
  'Windows token creation must apply a protected DACL before writing contents',
)
requireValue(agent.includes('copy_bidirectional'), 'Agent must expose an encrypted RPC data tunnel')
requireValue(agent.includes('previous_hash'), 'Agent audit records must be integrity-linked')
requireValue(agent.includes('MAX_CONTROL_FRAME_BYTES + 1'), 'Agent must bound frames while reading')
for (const limit of [
  'MAX_PREAUTH_CONTROL_CONNECTIONS',
  'MAX_PREAUTH_TUNNEL_CONNECTIONS',
  'MAX_AUTHENTICATED_CONTROL_CONNECTIONS',
  'MAX_AUTHENTICATED_TUNNELS',
  'MAX_PREAUTH_CONNECTIONS_PER_SOURCE',
]) {
  requireValue(agent.includes(limit), `Agent connection policy must contain ${limit}`)
}
requireValue(agent.includes('MAX_AUDIT_FILE_BYTES'), 'Agent must bound its audit log')
requireValue(agent.includes('AUTH_FAILURE_AUDIT_INTERVAL'), 'Agent must aggregate unauthenticated audit failures')
requireValue(agent.includes('AUDIT_UNAVAILABLE'), 'Agent actions must fail closed when audit persistence is unavailable')
requireValue(agent.includes('pinned_root_store'), 'Agent TLS must trust only the pinned leaf certificate')
requireValue(agent.includes('token.len() != 64'), 'Agent tokens must have an exact 256-bit representation')
requireValue(!agent.match(/--token(?:\s|=)/), 'Agent CLI must not accept token contents')
requireValue(models.includes('Agent,'), 'Worker origin must model secure Agent workers')
requireValue(models.includes('Option<crate::worker_agent::WorkerAgentConnection>'), 'Worker persistence must store only Agent metadata')
for (const command of ['enroll_worker_agent', 'test_worker_agent', 'start_worker_agent', 'stop_worker_agent', 'list_worker_agent_audit']) {
  requireValue(commands.includes(command), `Agent IPC implementation must contain ${command}`)
  requireValue(main.includes(command), `Tauri command registry must expose ${command}`)
}
for (const command of ['enroll_worker_agent', 'test_worker_agent', 'stop_worker_agent', 'list_worker_agent_audit']) {
  requireValue(ui.includes(`'${command}'`), `Cluster UI must invoke ${command}`)
}
requireValue(!ui.includes("'start_worker_agent'"), 'Cluster UI must not request fail-closed compute startup')
requireValue(ui.includes('disabled variant="success"'), 'Cluster UI must disable Secure Agent compute startup')
requireValue(
  commands.includes('compute startup is disabled because current upstream rpc-server'),
  'native Agent startup IPC must fail closed before contacting a remote Agent',
)
requireValue(commands.includes('confirm_agent_enrollment'), 'Agent enrollment must require native approval')
requireValue(commands.includes('validate_private_token'), 'Agent enrollment must validate the token before changing its ACL')
requireValue(main.includes('restore_worker_agent_bridges'), 'manager startup must restore persisted Agent bridges')
requireValue(
  clusterCommands.includes('worker.origin == WorkerOrigin::Agent && worker.agent.is_some()'),
  'Cluster IPC must expose only enrolled Agent workers',
)
requireValue(ui.includes('{t.clusterPage.agentBadge}'), 'Cluster UI must identify Secure Agent workers')
requireValue(ui.includes('{t.clusterPage.agentSecurityNote}'), 'Cluster UI must explain the Agent security boundary')
requireValue(ui.includes('data-guide="cluster-agent"'), 'Cluster UI must expose the secure enrollment action')
requireValue(
  (workflow.match(/worker-agent help/g) || []).length === 4,
  'all four platform packages must execute lsm worker-agent help',
)
for (const marker of [
  'Remote requests cannot provide a program',
  'Credential rotation and recovery',
  'Single-node installations do not need an Agent',
  'No unauthenticated loopback child is created',
  'rotates into immutable, hash-linked',
]) {
  requireValue(docs.includes(marker), `Worker Agent documentation must contain: ${marker}`)
}
requireValue(
  packageJson.scripts?.['check:worker-agent'] === 'node scripts/check-worker-agent.cjs',
  'package.json must expose check:worker-agent',
)

if (errors.length > 0) {
  console.error(`Worker Agent contract check failed with ${errors.length} issue(s):`)
  for (const error of errors) console.error(`- ${error}`)
  process.exit(1)
}

console.log('Worker Agent contract check passed')
