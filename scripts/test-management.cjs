// Exercises actual console, MCP, authenticated IPC and management host processes.
// Uses an isolated data directory and a synthetic engine; not real model inference.
const assert = require('node:assert/strict')
const cp = require('node:child_process')
const crypto = require('node:crypto')
const fs = require('node:fs')
const net = require('node:net')
const os = require('node:os')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const extension = process.platform === 'win32' ? '.exe' : ''
const profile = process.env.LSM_TEST_PROFILE || 'debug'
const executable = path.join(root, 'src-tauri', 'target', profile, `llama-server-manager${extension}`)
const cli = path.join(root, 'src-tauri', 'target', profile, `lsm${extension}`)
const data = fs.mkdtempSync(path.join(os.tmpdir(), 'lsm-management-'))
const engineDir = path.join(data, 'fixture-engine')
const modelDir = path.join(data, 'models')
const wait = ms => new Promise(resolve => setTimeout(resolve, ms))
let hostPid
let checks = 0

function execute(file, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = cp.spawn(file, args, { windowsHide: true, ...options })
    let stdout = '', stderr = ''
    child.stdout?.on('data', chunk => { stdout += chunk })
    child.stderr?.on('data', chunk => { stderr += chunk })
    const timer = setTimeout(() => { child.kill(); reject(new Error(`process timed out: ${path.basename(file)}`)) }, 90000)
    child.on('error', error => { clearTimeout(timer); reject(error) })
    child.on('close', code => { clearTimeout(timer); resolve({ code, stdout, stderr }) })
    if (options.input) child.stdin.end(options.input)
  })
}

async function call(operation, argumentsValue = {}, extra = [], success = true) {
  const result = await execute(cli, ['--app', executable, '--data-dir', data, ...operation.split('.'), '--args', JSON.stringify(argumentsValue), ...extra])
  const response = JSON.parse(result.stdout || result.stderr)
  if (success) assert.equal(result.code, 0, `${operation}: ${JSON.stringify(response)}`)
  checks++
  return response
}

async function mutate(operation, args, extra = [], success = true) {
  const response = await call(operation, args, ['--wait', '--timeout', '60', ...extra], success)
  if (success) assert.equal(response.data.status, 'succeeded', JSON.stringify(response))
  return response
}

async function freePort() {
  const server = net.createServer()
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const port = server.address().port
  await new Promise(resolve => server.close(resolve))
  return port
}

async function rawRequest(envelope) {
  const token = fs.readFileSync(path.join(data, 'management', 'control-token'), 'utf8')
  const canonical = fs.realpathSync.native(data)
  const identity = process.platform === 'win32' ? canonical.replace(/^\\\\\?\\/, '').toLowerCase() : canonical
  const suffix = crypto.createHash('sha256').update(`${identity}:${token}`).digest('hex').slice(0, 32)
  const endpoint = process.platform === 'win32' ? `\\\\.\\pipe\\llama-server-manager-management-${suffix}` : `/tmp/lsm-mgmt-${suffix}/control.sock`
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(endpoint)
    const chunks = []
    socket.setTimeout(10000, () => socket.destroy(new Error('IPC timeout')))
    socket.on('error', reject)
    socket.on('connect', () => {
      const payload = Buffer.from(JSON.stringify({ version: 1, id: 'raw-test', token, operation: 'service.status', arguments: {}, ...envelope }))
      const header = Buffer.alloc(4); header.writeUInt32LE(payload.length)
      socket.write(Buffer.concat([header, payload]))
    })
    socket.on('data', chunk => chunks.push(chunk))
    socket.on('end', () => { const bytes = Buffer.concat(chunks); resolve(JSON.parse(bytes.subarray(4).toString())) })
  })
}

async function run() {
  fs.mkdirSync(engineDir); fs.mkdirSync(modelDir)
  const engine = path.join(engineDir, `llama-server${extension}`)
  const compiled = await execute('rustc', ['--edition=2021', path.join(__dirname, 'fixtures', 'management-engine.rs'), '-o', engine])
  assert.equal(compiled.code, 0, compiled.stderr)
  fs.copyFileSync(engine, path.join(engineDir, `rpc-server${extension}`))
  const model = path.join(modelDir, 'fixture.gguf')
  const header = Buffer.alloc(24); header.write('GGUF'); header.writeUInt32LE(3, 4); fs.writeFileSync(model, header)

  const started = await call('service.start')
  hostPid = started.data.pid
  assert.equal(started.data.protocol, 1)
  assert.equal((await rawRequest({ token: 'wrong-token' })).error.code, 'UNAUTHORIZED')
  assert.equal((await rawRequest({ version: 999 })).error.code, 'VERSION_MISMATCH')
  assert.equal((await rawRequest({ operation: 'no.such.operation' })).error.code, 'NOT_FOUND')
  assert.equal((await rawRequest({ operation: 'engines.rename', arguments: { id: 'x', name: 'x', ignored: true } })).error.code, 'VALIDATION')
  checks += 4

  const schemas = (await call('service.capabilities')).data.operations
  assert.ok(schemas.length >= 65)
  assert.ok(schemas.every(op => op.schema.additionalProperties === false))
  const decimalDefaults = {
    rope_scale: 0, rope_freq_base: 0, rope_freq_scale: 0,
    yarn_ext_factor: -1, yarn_attn_factor: -1, yarn_beta_slow: -1, yarn_beta_fast: -1,
    spec_draft_p_min: 0, spec_draft_p_split: 0.1, slot_prompt_similarity: 0.1,
    temp: 0.8, top_p: 0.95, repeat_penalty: 1, min_p: 0.05,
    presence_penalty: 0, frequency_penalty: 0, mirostat_lr: 0.1, mirostat_ent: 5,
    xtc_probability: 0, xtc_threshold: 0.1, dynatemp_range: 0, dynatemp_exp: 1,
    typical_p: 1, dry_multiplier: 0, dry_base: 1.75, adaptive_target: -1,
    adaptive_decay: 0.9, top_n_sigma: -1,
  }
  const assertDecimals = (config, expected) => {
    for (const [field, value] of Object.entries(expected)) assert.equal(config[field], value, field)
  }
  assertDecimals((await call('instances.defaults')).data.config, decimalDefaults)
  for (const [purpose, directory] of [['engine', engineDir], ['model', modelDir], ['download', modelDir]]) {
    await mutate('paths.authorize', { purpose, path: directory })
    if (purpose !== 'download') {
      const revision = (await call('service.status')).data.revision
      await mutate('directories.add', { purpose, path: directory, revision })
    }
  }
  await mutate('engines.refresh', {})
  await mutate('models.refresh', {})
  let engines = (await call('engines.list')).data
  assert.equal(engines.engines.length, 1)
  const engineId = engines.engines[0].id
  await mutate('engines.probe', { id: engineId })
  const renamed = await mutate('engines.rename', { id: engineId, name: 'Agent test engine' }, ['--idempotency-key', 'rename-once'])
  const replay = await call('engines.rename', { id: engineId, name: 'Agent test engine' }, ['--idempotency-key', 'rename-once'])
  assert.equal(replay.data.task_id, renamed.data.id)
  assert.equal((await call('engines.rename', { id: engineId, name: 'Different' }, ['--idempotency-key', 'rename-once'], false)).error.code, 'IDEMPOTENCY_CONFLICT')
  engines = (await call('engines.list')).data
  await mutate('engines.default', { id: engineId, revision: engines.revision })

  const port = await freePort()
  const secret = 'management-fixture-secret-1234567890'
  let revision = (await call('service.status')).data.revision
  await mutate('instances.create', { id: 'agent-fixture', revision, config: { name: 'Agent fixture', engine_id: engineId, model_path: model, host: '127.0.0.1', port, launch_mode: 'manual', manual_command: `"${engine}" --model "${model}" --host 127.0.0.1 --api-key ${secret}`, api_key: secret } })
  const configured = await call('instances.get', { id: 'agent-fixture' })
  assert.equal(configured.data.config.api_key, '[REDACTED]')
  assert.ok(!JSON.stringify(await call('instances.validate', { id: 'agent-fixture' })).includes(secret))
  const refused = await mutate('engines.delete', { id: engineId }, [], false)
  assert.equal(refused.ok, false)
  revision = configured.data.revision
  const races = await Promise.all(['First', 'Second'].map(name => mutate('instances.update', { id: 'agent-fixture', revision, patch: { name } }, [], false)))
  assert.equal(races.filter(result => result.ok).length, 1)
  assert.equal(races.find(result => !result.ok).error.code, 'CONFIG_CONFLICT')
  const decimalPatch = { temp: 0.12345679, top_p: 0.95, mirostat_lr: 0.1, xtc_threshold: 0.1, adaptive_decay: 0.9 }
  revision = (await call('instances.get', { id: 'agent-fixture' })).data.revision
  await mutate('instances.update', { id: 'agent-fixture', revision, patch: decimalPatch })
  const decimalSaved = (await call('instances.get', { id: 'agent-fixture' })).data
  assertDecimals(decimalSaved.config, { ...decimalDefaults, ...decimalPatch })
  assertDecimals(JSON.parse(fs.readFileSync(path.join(data, 'configs', 'instances.json'))).instances['agent-fixture'], decimalPatch)
  assert.ok(Object.keys(decimalPatch).every(field => decimalSaved.config.explicit_overrides.includes(field)))
  const cloned = await mutate('instances.clone', { id: 'agent-fixture', new_id: 'decimal-clone', revision: decimalSaved.revision, patch: { name: 'Decimal clone', port: await freePort(), launch_mode: 'managed' } })
  const command = cloned.data.result.validation.command
  assert.equal(command[command.indexOf('--temp') + 1], '0.12345679')
  assert.equal(command[command.indexOf('--top-p') + 1], '0.95')
  await mutate('instances.delete', { id: 'decimal-clone', revision: cloned.data.result.revision })
  await mutate('instances.start', { id: 'agent-fixture' })
  for (let attempt = 0; attempt < 30; attempt++) {
    const response = await fetch(`http://127.0.0.1:${port}/health`).catch(() => null)
    if (response?.ok) break
    if (attempt === 29) throw new Error('fixture did not start on the manual fallback port')
    await wait(200)
  }
  const inference = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, { method: 'POST', headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${secret}` }, body: JSON.stringify({ messages: [{ role: 'user', content: 'fixture' }] }) }).then(response => response.json())
  assert.equal(inference.choices[0].message.content, 'fixture-ok')
  const health = await call('instances.health', { id: 'agent-fixture' })
  assert.ok(health.data.running.pid > 0)

  const routes = (await call('routes.get')).data
  const routerPort = await freePort()
  await mutate('routes.update', { revision: routes.revision, patch: { host: '127.0.0.1', port: routerPort, default_instance_id: 'agent-fixture', routes: [{ id: 'fixture-route', enabled: true, model_alias: 'fixture', target_instance_id: 'agent-fixture', priority: 0, weight: 1, max_concurrent_requests: 1 }] } })
  assert.equal((await mutate('routes.update', { revision: routes.revision, patch: { port: routerPort } }, [], false)).error.code, 'CONFIG_CONFLICT')
  await mutate('routes.start', {})
  const routed = await fetch(`http://127.0.0.1:${routerPort}/v1/chat/completions`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ model: 'fixture', messages: [{ role: 'user', content: 'fixture' }] }) }).then(response => response.json())
  assert.equal(routed.choices[0].message.content, 'fixture-ok')
  await mutate('routes.stop', {})
  await mutate('instances.stop', { id: 'agent-fixture' })
  await mutate('downloads.settings', { concurrency: 2, bytes_per_second: 1048576, resume_policy: 'manual' })
  assert.equal((await call('downloads.list')).data.max_concurrent, 2)
  assert.equal((await call('cluster.list')).data.length, 0)
  const rpcPort = await freePort()
  assert.equal((await mutate('cluster.add', { host: '127.0.0.1', port: rpcPort, trust: false }, [], false)).error.code, 'TRUST_REQUIRED')
  const worker = (await mutate('cluster.start_local', { port: rpcPort, engine_id: engineId })).data.result.worker
  assert.equal(worker.origin, 'local')
  assert.equal((await call('cluster.list')).data.length, 1)
  await call('cluster.test', { host: '127.0.0.1', port: rpcPort })
  await mutate('cluster.stop', { id: worker.id })
  await mutate('cluster.remove', { id: worker.id })
  assert.equal((await call('cluster.mdns_status')).data.active, false)

  const input = [
    { jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'smoke', version: '1' } } },
    { jsonrpc: '2.0', method: 'notifications/initialized' },
    { jsonrpc: '2.0', id: 2, method: 'tools/list' },
    { jsonrpc: '2.0', id: 3, method: 'tools/call', params: { name: 'engines_list', arguments: {} } },
    { jsonrpc: '2.0', id: 4, method: 'tools/call', params: { name: 'engines_delete', arguments: { id: engineId } } },
    { jsonrpc: '2.0', id: 5, method: 'tools/call', params: { name: 'instances_defaults', arguments: {} } },
    { jsonrpc: '2.0', id: 6, method: 'tools/call', params: { name: 'instances_get', arguments: { id: 'agent-fixture' } } },
  ].map(message => JSON.stringify(message)).join('\n') + '\n'
  const mcp = await execute(cli, ['--app', executable, '--data-dir', data, 'mcp'], { input })
  assert.equal(mcp.code, 0, mcp.stderr)
  const messages = mcp.stdout.trim().split('\n').map(line => JSON.parse(line))
  assert.equal(messages.length, 6)
  assert.ok(messages[1].result.tools.every(tool => tool.annotations.readOnlyHint))
  assert.equal(messages[2].result.structuredContent.ok, true)
  assert.equal(messages[3].error.code, -32602)
  assertDecimals(messages[4].result.structuredContent.data.config, decimalDefaults)
  assertDecimals(messages[5].result.structuredContent.data.config, { ...decimalDefaults, ...decimalPatch })
  checks += 6
  assert.equal((await call('service.status')).data.pid, hostPid, 'MCP EOF must not terminate host')
  assert.ok(!fs.readFileSync(path.join(data, 'management', 'tasks.json'), 'utf8').includes(secret))

  await mutate('service.stop', {})
  await wait(3000)
  // Terminal and paused tasks live on disk, outside the active scheduler queue.
  const history = ['paused', 'completed', 'error'].map((status, index) => ({
    id: `history-${status}`, repo_id: 'fixture/history', source: 'huggingface',
    save_dir: modelDir, added_at: index, status, retries: 0, max_retries: 3,
    files: [{ name: `${status}.gguf`, path: `${status}.gguf`, size: 24,
      file_type: 'model', task_id: `task-${status}`, run_id: `run-${status}`,
      downloaded: status === 'completed' ? 24 : 0, version: 0, status }],
  }))
  fs.writeFileSync(path.join(data, 'configs', 'downloads.json'), JSON.stringify({ queue: history }))
  const restarted = await call('service.start', {}, ['--data-dir', data.replaceAll('\\', '/')])
  assert.notEqual(restarted.data.pid, hostPid)
  hostPid = restarted.data.pid
  assertDecimals((await call('instances.get', { id: 'agent-fixture' })).data.config, { ...decimalDefaults, ...decimalPatch })
  assert.equal((await call('downloads.list')).data.max_concurrent, 2)
  const restoredFiles = (await call('downloads.list')).data.queue.flatMap(entry => entry.files)
  for (const entry of history) assert.ok(restoredFiles.some(file => file.task_id === entry.files[0].task_id && file.status === entry.status))
  assert.equal((await mutate('downloads.clear', { statuses: ['active', 'paused'] }, [], false)).error.code, 'VALIDATION')
  await mutate('downloads.clear', { statuses: ['completed', 'error'] })
  assert.deepEqual((await call('downloads.list')).data.queue.flatMap(entry => entry.files.map(file => file.status)), ['paused'])
  assert.equal((await call('tasks.get', { id: renamed.data.id })).data.status, 'succeeded')
  assert.equal((await call('engines.list')).data.engines[0].name, 'Agent test engine')
  const referencedRevision = (await call('instances.get', { id: 'agent-fixture' })).data.revision
  assert.equal((await mutate('instances.delete', { id: 'agent-fixture', revision: referencedRevision }, [], false)).ok, false)
  const finalRoutes = (await call('routes.get')).data
  await mutate('routes.update', { revision: finalRoutes.revision, patch: { default_instance_id: '', routes: [] } })
  revision = (await call('instances.get', { id: 'agent-fixture' })).data.revision
  await mutate('instances.delete', { id: 'agent-fixture', revision })
  engines = (await call('engines.list')).data
  await mutate('engines.default', { clear: true, revision: engines.revision })
  const removed = await mutate('engines.delete', { id: engineId })
  assert.equal(removed.data.result.files_deleted, false)
  assert.ok(fs.existsSync(engine))
  console.log(`Management integration passed (${checks} CLI/IPC/MCP checks; synthetic inference fixture).`)
}

run().catch(error => { console.error(error); process.exitCode = 1 }).finally(async () => {
  try { await mutate('service.stop', {}, [], false); await wait(3000) } catch { /* fallback below */ }
  if (hostPid) { try { process.kill(hostPid) } catch { /* already exited */ } }
  if (process.env.LSM_KEEP_TEST_DATA === '1') console.log(`Retained test data: ${data}`)
  else {
    assert.ok(path.resolve(data).startsWith(path.resolve(os.tmpdir()) + path.sep + 'lsm-management-'))
    fs.rmSync(data, { recursive: true, force: true, maxRetries: 20, retryDelay: 300 })
  }
})
