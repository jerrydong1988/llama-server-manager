// Exercises actual console, MCP, authenticated IPC and management host processes.
// Uses an isolated data directory and a synthetic engine; not real model inference.
const assert = require('node:assert/strict')
const cp = require('node:child_process')
const crypto = require('node:crypto')
const fs = require('node:fs')
const net = require('node:net')
const os = require('node:os')
const path = require('node:path')
const { managementClient } = require('./lib/management-client.cjs')

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
let lifecycleMcp
let checks = 0

function execute(file, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = cp.spawn(file, args, { windowsHide: true, ...options })
    let stdout = '', stderr = ''
    child.stdout?.on('data', chunk => { stdout += chunk })
    child.stderr?.on('data', chunk => { stderr += chunk })
    const timer = setTimeout(() => { child.kill(); reject(new Error(`process timed out: ${path.basename(file)}`)) }, 90000)
    child.on('error', error => { clearTimeout(timer); reject(error) })
    child.on('close', (code, signal) => { clearTimeout(timer); resolve({ code, signal, stdout, stderr }) })
    if (options.input) child.stdin.end(options.input)
  })
}

async function call(operation, argumentsValue = {}, extra = [], success = true) {
  const result = await execute(cli, ['--app', executable, '--data-dir', data, ...operation.split('.'), '--args', JSON.stringify(argumentsValue), ...extra])
  let response
  try { response = JSON.parse(result.stdout || result.stderr) } catch (cause) {
    throw new Error(`${operation}: invalid CLI JSON; exit=${result.code}, signal=${result.signal}; stdout=${JSON.stringify(result.stdout.slice(0, 4096))}; stderr=${JSON.stringify(result.stderr.slice(0, 4096))}`, { cause })
  }
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
  // A new MCP session can discover tools without a host, executable launch,
  // credentials, runtime, or configuration writes (including write-enabled sessions).
  lifecycleMcp = await managementClient({ cli, app: executable, data }).mcp()
  async function assertDisconnected(client) {
    assert.deepEqual((await client.request('ping')).result, {})
    assert.ok((await client.request('tools/list')).result.tools.length > 20)
    for (const operation of ['service.status', 'runtime.clear_error']) {
      const result = await client.tool(operation, {}, true)
      assert.equal(result.result.isError, true)
      assert.equal(result.result.structuredContent.error.code, 'MANAGEMENT_CONNECTION_FAILED')
      assert.match(result.result.structuredContent.error.message, /explicitly run lsm service start/)
    }
    checks += 4
  }
  await assertDisconnected(lifecycleMcp)
  assert.deepEqual(fs.readdirSync(data), [], 'MCP bootstrap and unavailable calls must not initialize host state')

  fs.mkdirSync(engineDir); fs.mkdirSync(modelDir)
  const engine = path.join(engineDir, `llama-server${extension}`)
  const compiled = await execute('rustc', ['--edition=2021', path.join(__dirname, 'fixtures', 'management-engine.rs'), '-o', engine])
  assert.equal(compiled.code, 0, compiled.stderr)
  fs.copyFileSync(engine, path.join(engineDir, `ggml-rpc-server${extension}`))
  const model = path.join(modelDir, 'fixture.gguf')
  const header = Buffer.alloc(24); header.write('GGUF'); header.writeUInt32LE(3, 4); fs.writeFileSync(model, header)

  // Legacy GUI presets can share an endpoint while remaining stopped. Seed
  // their persisted shape before starting the host, without bypassing its CAS.
  const sharedPort = await freePort()
  fs.mkdirSync(path.join(data, 'configs'))
  fs.writeFileSync(path.join(data, 'configs', 'instances.json'), JSON.stringify({
    instances: Object.fromEntries(['shared-managed', 'shared-manual'].map(id => [id, {
      id, name: id, model_path: model, host: '127.0.0.1', port: sharedPort,
      launch_mode: id === 'shared-manual' ? 'manual' : 'managed',
      manual_command: `--model "${model}" --host 127.0.0.1`, explicit_overrides: [],
    }])), instance_order: ['shared-managed', 'shared-manual'],
    model_dirs: ['models', 'legacy-models'], engine_dirs: ['fixture-engine', 'missing-engine'], default_engine_id: '', running: {},
  }), 'utf8')

  // Seed the bounded journal before host startup, then fail an enqueue commit.
  // No running task or production data is involved in this fault injection.
  const journal = path.join(data, 'management', 'tasks.json')
  fs.mkdirSync(path.dirname(journal))
  const seededHistory = Array.from({ length: 200 }, (_, i) => ({
    id: `seed-${i}`, operation: 'models.refresh',
    request_digest: crypto.createHash('sha256').update(JSON.stringify({ arguments: {}, operation: 'models.refresh' })).digest('hex'),
    idempotency_key: `seed-key-${i}`, status: 'succeeded', created_at: i, updated_at: i, result: {}, error: null,
  }))
  fs.writeFileSync(journal, JSON.stringify(seededHistory), 'utf8')

  const started = await call('service.start')
  hostPid = started.data.pid
  assert.equal(started.data.protocol, 1)
  assert.deepEqual(started.data.desktop, { window_created: false, tray_created: false })
  assert.equal((await lifecycleMcp.tool('service.status')).pid, hostPid, 'existing MCP connection must attach after explicit start')
  const journalBackup = journal + '.test-backup'
  fs.renameSync(journal, journalBackup); fs.mkdirSync(journal)
  try {
    assert.equal((await call('engines.rename', { id: 'missing', name: 'not executed' }, [], false)).ok, false)
    assert.equal((await call('tasks.list')).data.length, 200)
    assert.equal((await call('tasks.get', { id: 'seed-0' })).data.idempotency_key, 'seed-key-0')
  } finally { fs.rmdirSync(journal); fs.renameSync(journalBackup, journal) }
  const retainedReplay = await call('models.refresh', {}, ['--idempotency-key', 'seed-key-0'])
  assert.equal(retainedReplay.data.replayed, true)
  assert.equal(retainedReplay.data.task_id, 'seed-0')
  await call('tasks.delete', { id: 'seed-199' })
  const retainedHistory = JSON.parse(fs.readFileSync(journal, 'utf8'))
  assert.equal(retainedHistory.length, 199)
  assert.equal(retainedHistory[0].id, 'seed-0')
  await call('tasks.clear')

  // Removal uses the same data-directory base as reads, including missing
  // directories, which cannot be normalized with filesystem canonicalization.
  for (const directory of (await call('engines.list')).data.directories) {
    const revision = (await call('service.status')).data.revision
    await mutate('directories.remove', { purpose: 'engine', path: directory, revision })
    assert.ok(!(await call('engines.list')).data.directories.includes(directory))
  }
  let directoryRevision = (await call('service.status')).data.revision
  await mutate('directories.remove', { purpose: 'model', path: path.join(data, 'legacy-models'), revision: directoryRevision })
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(data, 'configs', 'instances.json'), 'utf8')).model_dirs, ['models'],
    `relative model directory removal: requested=${path.join(data, 'legacy-models')} host=${(await call('service.status')).data.data_directory}`)
  directoryRevision = (await call('service.status')).data.revision
  await mutate('paths.authorize', { purpose: 'model', path: modelDir })
  await mutate('directories.add', { purpose: 'model', path: modelDir, revision: directoryRevision })
  assert.equal(JSON.parse(fs.readFileSync(path.join(data, 'configs', 'instances.json'), 'utf8')).model_dirs.length, 1, 'adding an absolute alias must not duplicate the relative root')
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

  const sharedMcp = await managementClient({ cli, app: executable, data }).mcp()
  try {
    const before = (await call('instances.get', { id: 'shared-managed' })).data
    await mutate('instances.update', { id: 'shared-managed', revision: before.revision,
      patch: { name: 'Renamed shared preset', engine_id: engineId, temp: 0.72, host: '127.0.0.1', port: sharedPort } })
    const fromCli = await sharedMcp.tool('instances.get', { id: 'shared-managed' })
    assert.equal(fromCli.config.temp, 0.72)
    assert.equal(fromCli.config.port, sharedPort)
    assert.equal((await mutate('instances.update', { id: 'shared-managed', revision: before.revision,
      patch: { temp: 0.9 } }, [], false)).error.code, 'CONFIG_CONFLICT')
    await sharedMcp.mutate('instances.update', { id: 'shared-manual', revision: fromCli.revision,
      patch: { engine_id: engineId, temp: 0.73 } })
    const fromMcp = (await call('instances.get', { id: 'shared-manual' })).data
    assert.equal(fromMcp.config.temp, 0.73)
    assert.equal(fromMcp.config.port, sharedPort)
    for (const [operation, args] of [
      ['instances.create', { id: 'conflict-create', config: { engine_id: engineId,
        model_path: model, host: '127.0.0.1', port: sharedPort } }],
      ['instances.clone', { id: 'shared-manual', new_id: 'conflict-clone', patch: {} }],
    ]) {
      const refused = await mutate(operation, { ...args, revision: fromMcp.revision }, [], false)
      assert.equal(refused.ok, false)
      assert.match(refused.error.message, /host\/port conflicts/)
      assert.equal((await call('service.status')).data.revision, fromMcp.revision)
    }
    const moved = await mutate('instances.update', { id: 'shared-managed', revision: fromMcp.revision,
      patch: { port: await freePort() } })
    const refused = await sharedMcp.mutate('instances.update', { id: 'shared-managed',
      revision: moved.data.result.revision, patch: { port: sharedPort } }, true)
    assert.equal(refused.status, 'failed')
    assert.match(refused.error.message, /host\/port conflicts/)
    for (const id of ['shared-managed', 'shared-manual']) {
      const revision = (await call('service.status')).data.revision
      await mutate('instances.delete', { id, revision })
    }
    checks += 3 // real MCP get/update/rejected update
  } finally { await sharedMcp.close() }

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
  assert.equal((await mutate('instances.clear_log', { id: 'agent-fixture' }, [], false)).ok, false)
  assert.equal((await mutate('checkpoints.clear', { id: 'agent-fixture' }, [], false)).ok, false)
  assert.equal((await call('instances.slots', { id: 'agent-fixture' })).data.length, 1)
  await call('instances.metrics', { id: 'agent-fixture' })
  await call('instances.test_connection', { id: 'agent-fixture' })
  await call('instances.resources', { id: 'agent-fixture' })
  await call('monitoring.series', { id: 'agent-fixture', range_ms: 10000 })
  assert.equal((await call('system.port', { port })).data, false)
  await mutate('routes.stop', {})
  // Real host scheduling: a slow engine probe must not block instance controls,
  // while a queued same-domain mutation can be cancelled durably via MCP.
  fs.writeFileSync(path.join(engineDir, 'probe-delay-ms'), '3000', 'utf8')
  const slowProbe = await call('engines.probe_all')
  for (let i = 0; i < 30; i++) {
    if ((await call('tasks.get', { id: slowProbe.data.task_id })).data.status === 'running') break
    await wait(50)
  }
  const queuedRename = await call('engines.rename', { id: engineId, name: 'Cancelled rename' })
  const schedulingMcp = await managementClient({ cli, app: executable, data }).mcp()
  try {
    assert.equal((await call('tasks.delete', { id: queuedRename.data.task_id }, [], false)).error.code, 'TASK_ACTIVE')
    const runningDelete = await schedulingMcp.tool('tasks.delete', { id: slowProbe.data.task_id }, true)
    assert.equal(runningDelete.result.structuredContent.error.code, 'TASK_ACTIVE')
    const invalidCleanup = await schedulingMcp.tool('tasks.clear', { statuses: ['failed', 'running'] }, true)
    assert.equal(invalidCleanup.result.structuredContent.error.code, 'VALIDATION')
    await call('tasks.clear', { statuses: ['failed'] })
    assert.equal((await schedulingMcp.tool('tasks.get', { id: queuedRename.data.task_id })).status, 'queued')
    assert.equal((await schedulingMcp.tool('tasks.get', { id: slowProbe.data.task_id })).status, 'running')
    assert.equal((await schedulingMcp.tool('tasks.cancel', { id: queuedRename.data.task_id })).status, 'cancelled')
    assert.equal((await schedulingMcp.tool('tasks.get', { id: queuedRename.data.task_id })).status, 'cancelled')
    const runningCancel = await schedulingMcp.tool('tasks.cancel', { id: slowProbe.data.task_id }, true)
    assert.equal(runningCancel.result.structuredContent.error.code, 'TASK_RUNNING')
    // Cleanup is synchronous and must not create another task history row.
    const beforeDelete = (await call('tasks.list')).data.length
    const deletion = await call('tasks.delete', { id: queuedRename.data.task_id }, ['--wait'])
    assert.deepEqual(deletion.data.deleted_ids, [queuedRename.data.task_id])
    assert.equal(deletion.data.task_id, undefined)
    assert.equal((await schedulingMcp.tool('tasks.list')).length, beforeDelete - 1)
    assert.equal((await schedulingMcp.tool('tasks.get', { id: queuedRename.data.task_id }, true)).result.structuredContent.error.code, 'NOT_FOUND')
    assert.equal((await call('tasks.delete', { id: queuedRename.data.task_id }, [], false)).error.code, 'NOT_FOUND')
    checks += 10
  } finally { await schedulingMcp.close() }
  const stopStarted = Date.now()
  await mutate('instances.stop', { id: 'agent-fixture' })
  assert.ok(Date.now() - stopStarted < 3000, 'instance stop must not wait for the unrelated 6s probe')
  for (let i = 0; i < 100; i++) {
    const status = (await call('tasks.get', { id: slowProbe.data.task_id })).data.status
    if (status === 'succeeded') break
    assert.notEqual(status, 'failed')
    await wait(100)
    if (i === 99) throw new Error('slow probe did not complete')
  }
  fs.unlinkSync(path.join(engineDir, 'probe-delay-ms'))
  assert.equal((await call('engines.list')).data.engines[0].name, 'Agent test engine')
  assert.equal((await call('system.port', { port })).data, true)
  // Fail queued -> running after admission. The operation must never execute,
  // must report a terminal error, and must not strand active queue capacity.
  fs.writeFileSync(path.join(engineDir, 'probe-delay-ms'), '3000', 'utf8')
  const faultProbe = await call('engines.probe_all')
  for (let i = 0; i < 100; i++) {
    if ((await call('tasks.get', { id: faultProbe.data.task_id })).data.status === 'running') break
    await wait(20)
  }
  assert.equal((await call('tasks.get', { id: faultProbe.data.task_id })).data.status, 'running')
  const faultRename = await call('engines.rename', { id: engineId, name: 'Must never be applied' }, ['--idempotency-key', 'fault-rename'])
  assert.equal((await call('tasks.get', { id: faultRename.data.task_id })).data.status, 'queued')
  fs.renameSync(journal, journalBackup); fs.mkdirSync(journal)
  let failedStart
  try {
    for (let i = 0; i < 200; i++) {
      failedStart = (await call('tasks.get', { id: faultRename.data.task_id })).data
      if (failedStart.status === 'failed') break
      await wait(50)
    }
    assert.equal(failedStart.status, 'failed')
    assert.equal(failedStart.error.code, 'TASK_START_PERSIST_FAILED')
    assert.equal((await call('engines.list')).data.engines[0].name, 'Agent test engine')
  } finally { fs.rmdirSync(journal); fs.renameSync(journalBackup, journal); fs.unlinkSync(path.join(engineDir, 'probe-delay-ms')) }
  assert.equal((await call('tasks.cancel', { id: faultRename.data.task_id })).data.status, 'failed')
  assert.equal((await call('engines.rename', { id: engineId, name: 'Must never be applied' }, ['--idempotency-key', 'fault-rename'])).data.replayed, true)
  await mutate('engines.rename', { id: engineId, name: 'Agent test engine' })
  assert.equal(JSON.parse(fs.readFileSync(journal, 'utf8')).find(task => task.id === faultRename.data.task_id).status, 'failed')
  await call('tasks.delete', { id: faultRename.data.task_id })
  await call('runtime.status')
  await mutate('runtime.clear_error', {})
  await call('system.autostart')
  await call('checkpoints.list')
  await call('checkpoints.status', { id: 'agent-fixture' })
  assert.equal((await call('checkpoints.eligibility', { id: 'agent-fixture' })).data.eligible, false)
  await mutate('checkpoints.clear', { id: 'agent-fixture' })
  await call('instances.logs', { id: 'agent-fixture', lines: 10 })
  await mutate('instances.clear_log', { id: 'agent-fixture' })
  assert.equal((await call('instances.logs', { id: 'agent-fixture' }, [], false)).ok, false)
  const presets = (await call('presets.list')).data.presets
  assert.equal(Object.keys(presets).length, 6)
  assert.equal((await call('presets.preview', { id: 'agent-fixture', preset: 'safe-start' }, [], false)).ok, false, 'manual mode cannot apply managed presets')
  revision = (await call('instances.get', { id: 'agent-fixture' })).data.revision
  await mutate('instances.update', { id: 'agent-fixture', revision, patch: { launch_mode: 'managed' } })
  const preview = (await call('presets.preview', { id: 'agent-fixture', preset: 'safe-start' })).data
  assert.equal(preview.saved, false)
  assert.equal(preview.patch.parallel, 1)
  revision = (await call('instances.get', { id: 'agent-fixture' })).data.revision
  await mutate('presets.apply', { id: 'agent-fixture', preset: 'safe-start', revision })
  const presetConfig = (await call('instances.get', { id: 'agent-fixture' })).data
  assert.equal(presetConfig.config.ctx_size, 4096)
  assert.equal(presetConfig.config.api_key, '[REDACTED]')
  assert.equal((await mutate('presets.apply', { id: 'agent-fixture', preset: 'low-vram', revision }, [], false)).error.code, 'CONFIG_CONFLICT')
  await mutate('instances.update', { id: 'agent-fixture', revision: presetConfig.revision, patch: { launch_mode: 'manual' } })
  const day = Math.floor(Date.now() / 86400000) * 86400000
  const query = { from: day, to: day + 86400000 }
  let summary
  for (let i = 0; i < 30; i++) {
    summary = (await call('usage.summary', { query })).data
    if (summary.summary.requests) break
    await wait(100)
  }
  assert.equal(summary.summary.input, routed.usage.prompt_tokens)
  assert.equal(summary.summary.output, routed.usage.completion_tokens)
  assert.equal(summary.summary.requests, 1)
  const requestPage = (await call('usage.requests', { query })).data
  assert.equal(requestPage.records.length, 1)
  assert.equal(requestPage.records[0].outcome, 'success')
  await call('usage.performance', { query })
  await call('usage.budgets')
  const csv = (await call('usage.export', { query, group: 'models' })).data.content
  assert.ok(csv.includes('fixture'))
  const mcpClient = await managementClient({ cli, app: executable, data }).mcp()
  try {
    const tools = (await mcpClient.request('tools/list')).result.tools
    const expected = schemas.filter(op => ['read', 'write', 'delete'].includes(op.permission))
    assert.deepEqual(tools.map(t => t.name).sort(), expected.map(op => op.name.replaceAll('.', '_')).sort())
    for (const [op, args] of [
      ['usage.summary', { query }], ['usage.requests', { query }], ['usage.performance', { query }],
      ['usage.budgets', {}], ['usage.export', { query, group: 'models' }], ['presets.list', {}],
      ['checkpoints.list', {}], ['checkpoints.status', { id: 'agent-fixture' }], ['checkpoints.eligibility', { id: 'agent-fixture' }],
      ['runtime.status', {}], ['telemetry.overview', {}], ['telemetry.sessions', { limit: 2 }],
      ['storage.inventory', {}], ['storage.external', {}], ['system.autostart', {}], ['system.port', { port }],
      ['downloads.local_files', { paths: [model] }],
    ]) {
      const result = await mcpClient.tool(op, args)
      if (op === 'usage.summary') assert.deepEqual(result.summary, summary.summary)
      if (op === 'usage.export') assert.equal(result.content, csv)
      checks++
    }
    await mcpClient.mutate('runtime.clear_error')
    await mcpClient.mutate('checkpoints.clear', { id: 'agent-fixture' })
    const sessions = await mcpClient.tool('telemetry.sessions', { limit: 10 })
    assert.ok(sessions.length)
    for (const op of ['telemetry.samples', 'telemetry.detail', 'telemetry.analysis', 'telemetry.diagnostics', 'telemetry.requests']) {
      await mcpClient.tool(op, { id: sessions[0].id }); checks++
    }
    await mcpClient.mutate('telemetry.prune', { retention_days: 365 })
    await mcpClient.mutate('telemetry.optimize')
    await mcpClient.mutate('storage.cleanup', { group: 'private-scratch' })
    checks += 3
    if (process.platform === 'win32') {
      await mcpClient.mutate('storage.webview_cleanup', { enabled: true })
      assert.equal((await call('storage.inventory')).data.scheduledWebviewCleanup, true)
      await mutate('storage.webview_cleanup', { enabled: false })
      assert.equal((await mcpClient.tool('storage.inventory')).scheduledWebviewCleanup, false)
      checks += 2
    }
    const denied = await mcpClient.mutate('models.delete', { path: model }, true)
    assert.equal(denied.status, 'failed')
    assert.ok(fs.existsSync(model))
    checks += 3
  } finally { await mcpClient.close() }
  assert.equal((await call('usage.summary', { query: { ...query, keyid: 'typo' } }, [], false)).error.code, 'VALIDATION')
  assert.equal((await call('usage.requests', { query: { ...query, cursor: { ...requestPage.firstCursor, typo: 1 } } }, [], false)).error.code, 'VALIDATION')
  const telemetry = (await call('telemetry.sessions', { limit: 10 })).data
  assert.ok(telemetry.length)
  const sessionId = telemetry[0].id
  for (const op of ['telemetry.samples', 'telemetry.detail', 'telemetry.analysis', 'telemetry.diagnostics', 'telemetry.requests']) await call(op, { id: sessionId })
  await call('telemetry.overview')
  assert.equal((await call('telemetry.sessions', { limit: 4294967297 }, [], false)).error.code, 'VALIDATION')
  const storage = (await call('storage.inventory')).data
  assert.ok(storage.groups.some(group => group.id === 'private-scratch'))
  await call('storage.external')
  assert.equal((await mutate('storage.cleanup', { group: '../configs' }, [], false)).ok, false)
  await mutate('storage.cleanup', { group: 'private-scratch' })
  await mutate('telemetry.prune', { retention_days: 365 })
  await mutate('telemetry.optimize', {})
  assert.deepEqual((await call('downloads.local_files', { paths: [model, path.join(modelDir, 'missing.gguf')] })).data, [24, null])
  assert.equal((await call('models.delete_preview', { path: model })).data.referencedBy.length, 1)
  assert.equal((await mutate('models.delete', { path: model }, [], false)).ok, false)
  assert.ok(fs.existsSync(model))
  await mutate('usage.clear', query)
  assert.equal((await call('usage.summary', { query })).data.summary.requests, 0)
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
    { jsonrpc: '2.0', id: 7, method: 'tools/call', params: { name: 'tasks_clear', arguments: {} } },
  ].map(message => JSON.stringify(message)).join('\n') + '\n'
  const mcp = await execute(cli, ['--app', executable, '--data-dir', data, 'mcp'], { input })
  assert.equal(mcp.code, 0, mcp.stderr)
  const messages = mcp.stdout.trim().split('\n').map(line => JSON.parse(line))
  assert.equal(messages.length, 7)
  assert.ok(messages[1].result.tools.every(tool => tool.annotations.readOnlyHint))
  assert.equal(messages[2].result.structuredContent.ok, true)
  assert.equal(messages[3].error.code, -32602)
  assertDecimals(messages[4].result.structuredContent.data.config, decimalDefaults)
  assertDecimals(messages[5].result.structuredContent.data.config, { ...decimalDefaults, ...decimalPatch })
  assert.equal(messages[6].error.code, -32602)
  checks += 7
  assert.equal((await call('service.status')).data.pid, hostPid, 'MCP EOF must not terminate host')
  assert.ok(!fs.readFileSync(path.join(data, 'management', 'tasks.json'), 'utf8').includes(secret))

  await mutate('service.stop', {})
  await wait(3000)
  await assertDisconnected(lifecycleMcp)
  const reconnectedMcp = await managementClient({ cli, app: executable, data }).mcp()
  try { await assertDisconnected(reconnectedMcp) } finally { await reconnectedMcp.close() }
  assert.equal((await call('service.status', {}, [], false)).ok, false, 'MCP reconnect and tools must not resurrect a stopped host')
  // A crash can leave queued/running records on disk. Recovery must preserve
  // terminal history, mark unfinished work interrupted, and advance its timestamp.
  const journalPath = path.join(data, 'management', 'tasks.json')
  const recoveredJournal = JSON.parse(fs.readFileSync(journalPath, 'utf8'))
  for (const status of ['queued', 'running']) recoveredJournal.push({
    id: `journal-${status}`, operation: 'models.refresh', request_digest: `journal-${status}`,
    idempotency_key: null, status, created_at: 1, updated_at: 1, result: null, error: null,
  })
  fs.writeFileSync(journalPath, JSON.stringify(recoveredJournal), 'utf8')
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
  assert.deepEqual(restarted.data.desktop, { window_created: false, tray_created: false })
  assert.equal((await lifecycleMcp.tool('service.status')).pid, hostPid, 'same MCP connection must recover after explicit restart')
  await lifecycleMcp.close(); lifecycleMcp = undefined
  assert.equal((await call('tasks.get', { id: queuedRename.data.task_id }, [], false)).error.code, 'NOT_FOUND', 'deleted history must not reappear after restart')
  for (const status of ['queued', 'running']) {
    const task = (await call('tasks.get', { id: `journal-${status}` })).data
    assert.equal(task.status, 'interrupted')
    assert.equal(task.error.code, 'INTERRUPTED')
    assert.ok(task.updated_at > 1)
    assert.equal((await call('tasks.cancel', { id: task.id })).data.status, 'interrupted')
  }
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

  const cleanupMcp = await managementClient({ cli, app: executable, data }).mcp()
  const writeOnlyMcp = await managementClient({ cli, app: executable, data }).mcp(['--allow-write'])
  try {
    const available = (await cleanupMcp.request('tools/list')).result.tools
    for (const name of ['tasks_delete', 'tasks_clear']) {
      assert.equal(available.find(tool => tool.name === name).annotations.destructiveHint, true)
      assert.equal((await writeOnlyMcp.tool(name.replace('_', '.'), name === 'tasks_delete' ? { id: 'missing' } : {}, true)).error.code, -32602)
    }
    for (const statuses of [[], ['failed', 'failed'], ['unknown'], ['queued']]) {
      assert.equal((await call('tasks.clear', { statuses }, [], false)).error.code, 'VALIDATION')
    }
    assert.equal((await call('tasks.clear', {}, ['--idempotency-key', 'not-supported'], false)).error.code, 'VALIDATION')
    const before = (await call('tasks.list')).data
    const configBytes = fs.readFileSync(path.join(data, 'configs', 'instances.json'), 'utf8')
    const filtered = await cleanupMcp.tool('tasks.clear', { statuses: ['failed', 'interrupted'] })
    assert.equal(filtered.deleted_count, before.filter(task => ['failed', 'interrupted'].includes(task.status)).length)
    assert.ok((await call('tasks.list')).data.every(task => !['failed', 'interrupted'].includes(task.status)))
    const remaining = (await call('tasks.list')).data
    const cleared = await cleanupMcp.tool('tasks.clear')
    assert.equal(cleared.deleted_count, remaining.length)
    assert.deepEqual((await call('tasks.list')).data, [])
    assert.equal((await call('tasks.clear', {}, ['--wait'])).data.deleted_count, 0)
    assert.equal(fs.readFileSync(path.join(data, 'configs', 'instances.json'), 'utf8'), configBytes)
    checks += 9
  } finally { await writeOnlyMcp.close(); await cleanupMcp.close() }
  // Explicit desktop promotion remains available, and repeated show is idempotent.
  for (let i = 0; i < 2; i++) {
    await mutate('service.show', {})
    assert.deepEqual((await call('service.status')).data.desktop, { window_created: true, tray_created: true })
  }
  console.log(`Management integration passed (${checks} CLI/IPC/MCP checks; synthetic inference fixture).`)
}

run().catch(error => { console.error(error); process.exitCode = 1 }).finally(async () => {
  if (lifecycleMcp) await lifecycleMcp.close()
  try { await mutate('service.stop', {}, [], false); await wait(3000) } catch { /* fallback below */ }
  if (hostPid) { try { process.kill(hostPid) } catch { /* already exited */ } }
  if (process.env.LSM_KEEP_TEST_DATA === '1') console.log(`Retained test data: ${data}`)
  else {
    assert.ok(path.resolve(data).startsWith(path.resolve(os.tmpdir()) + path.sep + 'lsm-management-'))
    fs.rmSync(data, { recursive: true, force: true, maxRetries: 20, retryDelay: 300 })
  }
})
