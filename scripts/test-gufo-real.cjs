// Explicit opt-in, real Manager CLI/MCP + Gufo test. All writable state is under --root.
// node scripts/test-gufo-real.cjs --root ABS --launcher ABS [--context 32768] [--profile debug]
const fs = require('node:fs')
const path = require('node:path')
const net = require('node:net')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')
const { managementClient, sleep } = require('./lib/management-client.cjs')
const args = Object.fromEntries(Array.from({ length: (process.argv.length - 2) / 2 }, (_, i) => [process.argv[2 + i * 2].slice(2), process.argv[3 + i * 2]]))
for (const key of ['root', 'launcher']) assert.ok(args[key] && path.isAbsolute(args[key]), `--${key} must be absolute`)
assert.ok(!fs.existsSync(args.root), 'use a new validation root to preserve previous evidence')
fs.mkdirSync(args.root, { recursive: true })
const data = path.join(args.root, 'data')
const authKey = args.extended ? 'isolated-gufo-engine-acceptance-key' : ''
const source = JSON.parse(fs.readFileSync(args.launcher, 'utf8'))
const enginePath = source.launcher.executable
const bin = args.bin || path.resolve('src-tauri/target', args.profile || 'debug')
const client = managementClient({ cli: path.join(bin, 'lsm.exe'), app: path.join(bin, 'llama-server-manager.exe'), data, timeout: 900000 })
const report = { started_at: new Date().toISOString(), engine: enginePath, context: Number(args.context || 32768), data, checks: [] }
const reportPath = path.join(args.root, 'result.json')
const write = () => fs.writeFileSync(reportPath, JSON.stringify(report, null, 2), 'utf8')
const record = (name, result) => { report.checks.push({ name, result }); write(); console.log(JSON.stringify({ name, result })) }
const hash = p => fs.existsSync(p) ? crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex') : null
function inventory(root) {
  if (!fs.existsSync(root)) return []
  return fs.readdirSync(root, { withFileTypes: true }).flatMap(entry => {
    const p = path.join(root, entry.name)
    if (entry.isSymbolicLink()) return []
    if (entry.isDirectory()) return inventory(p)
    const stat = fs.statSync(p)
    return [{ path: p, size: stat.size, mtime: stat.mtimeMs }]
  })
}
const protectedPaths = [args.launcher, enginePath, path.join(process.env.LOCALAPPDATA, 'LlamaServerManager/configs/instances.json')]
const before = Object.fromEntries(protectedPaths.map(p => [p, hash(p)]))
const cacheRoots = [...new Set(source.presets.map(p => p.settings.cache_disk_dir).filter(Boolean))]
const cacheBefore = cacheRoots.map(inventory)
let mcp, started = false
const ids = []
const owned = new Set()
const ports = []
async function freePort() {
  const server = net.createServer()
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const port = server.address().port
  await new Promise(resolve => server.close(resolve))
  return port
}
async function request(port, route, body) {
  const response = await fetch(`http://127.0.0.1:${port}${route}`, { method: body ? 'POST' : 'GET', headers: { 'Content-Type': 'application/json', ...(authKey ? { authorization: `Bearer ${authKey}` } : {}) },
    ...(body ? { body: JSON.stringify(body) } : {}), signal: AbortSignal.timeout(180000) })
  const result = await response.json()
  assert.equal(response.status, 200, JSON.stringify(result))
  return result
}
function alive(pid) { try { process.kill(pid, 0); return true } catch { return false } }
async function main() {
  record('protection_before', before)
  await client.call('service.start'); started = true
  let status = await client.call('service.status')
  owned.add(status.pid)
  if (status.runtime) owned.add(status.runtime.service_pid)
  const directories = new Map([[path.dirname(enginePath), 'engine']])
  if (args['llama-engine']) directories.set(path.dirname(args['llama-engine']), 'engine')
  for (const preset of source.presets) for (const field of ['model', 'mmproj', 'mtp_model', 'dflash_model']) {
    if (preset.settings[field]) directories.set(path.dirname(preset.settings[field]), 'model')
  }
  for (const [directory, purpose] of directories) {
    await client.mutate('paths.authorize', { purpose, path: directory })
    await client.mutate('directories.add', { purpose, path: directory, revision: (await client.call('service.status')).revision })
  }
  await client.mutate('engines.refresh')
  const inventory = (await client.call('engines.list')).engines
  record('engine_inventory', inventory.map(e => ({ id: e.id, exe: e.exe, kind: e.kind })))
  const engine = inventory.find(e => fs.realpathSync.native(e.exe).toLowerCase() === fs.realpathSync.native(enginePath).toLowerCase())
  assert.equal(engine?.kind, 'gufo')
  await client.mutate('engines.probe', { id: engine.id })
  const probed = (await client.call('engines.list')).engines.find(e => e.id === engine.id)
  assert.equal(probed.capabilities.status, 'detected')
  assert.match(probed.version, /0\.28\.1/)
  record('engine_discovery_and_probe', { id: engine.id, version: probed.version, kind: probed.kind })
  mcp = await client.mcp()
  for (const [index, preset] of (args['only-llama'] ? [] : source.presets).entries()) {
    const s = preset.settings, id = `gufo-real-${index}`, port = await freePort()
    ids.push(id); ports.push(port)
    const config = { engine_kind: 'gufo', engine_id: engine.id, name: preset.name, model_path: s.model, mmproj_path: s.mmproj,
      alias: s.served_model_name, api_key: authKey, host: '127.0.0.1', port, ctx_size: report.context, n_predict: s.max_tokens,
      temp: s.temperature, top_k: s.top_k, top_p: s.top_p, min_p: s.min_p, seed: s.seed, repeat_penalty: s.repeat_penalty,
      repeat_last_n: s.repeat_last_n, presence_penalty: s.presence_penalty, frequency_penalty: s.frequency_penalty,
      gufo: { model_family: s.model_family, speculative: s.speculative, mtp_model: s.mtp_model, dflash_model: s.dflash_model,
        draft_tokens: s.draft_tokens, sessions: 1, think: s.think, reasoning_effort: s.reasoning_effort, preserve_thinking: s.preserve_thinking,
        cache_ram_mode: 'auto', cache_ram_gib: 4, cache_disk: true, cache_disk_gib: s.cache_disk_gib, startup_timeout_seconds: 600 } }
    await client.mutate('instances.create', { id, config, revision: (await client.call('service.status')).revision })
    const preview = await mcp.tool('instances.validate', { id })
    assert.ok(preview.command.includes('--speculative'))
    const cache = preview.command[preview.command.indexOf('--cache-disk') + 1]
    const relativeCache = path.relative(path.toNamespacedPath(data), path.toNamespacedPath(cache))
    assert.ok(relativeCache && !relativeCache.startsWith('..') && !path.isAbsolute(relativeCache), cache)
    assert.ok(!preview.command.includes('--slot-save-path'))
    const eligibility = await mcp.tool('checkpoints.eligibility', { id })
    assert.equal(eligibility.eligible, false)
    const at = Date.now()
    await client.mutate('instances.start', { id })
    let loadingObserved = false, health
    while (Date.now() - at < 660000) {
      health = await mcp.tool('instances.health', { id })
      if (health.running?.pid) owned.add(health.running.pid)
      if (health.health === 'pending') loadingObserved = true
      if (health.health === 'ok') break
      assert.notEqual(health.health, 'fail', JSON.stringify(health))
      assert.ok(health.running, JSON.stringify(health))
      await sleep(1000)
    }
    assert.equal(health.health, 'ok', 'Manager readiness timeout')
    const props = await request(port, '/props')
    assert.match(props.build_info, /408a043/)
    assert.equal(props.default_generation_settings.n_ctx, report.context)
    const body = { model: s.served_model_name, messages: [{ role: 'user', content: 'Compute 6 times 7. Reply only with the number.' }], reasoning_effort: 'none', max_tokens: 32, temperature: 0, seed: 42 }
    const answer = await request(port, '/v1/chat/completions', body)
    assert.match(answer.choices[0].message.content, /42/)
    assert.ok(answer.usage.completion_tokens > 0)
    fs.writeFileSync(path.join(args.root, `${id}-response.json`), JSON.stringify(answer, null, 2))
    const logs = await mcp.tool('instances.logs', { id, lines: 100 })
    assert.ok(JSON.stringify(logs).length > 100)
    record('model_inference', { id, model: s.served_model_name, loadingObserved, ready_seconds: (Date.now() - at) / 1000, port, props, usage: answer.usage })
    if (args.extended) {
      const routerPort = await freePort(); ports.push(routerPort)
      await require('./lib/gufo-api-acceptance.cjs').gufoApiAcceptance({ client, mcp, id, model: s.served_model_name, port, routerPort, root: args.root, record, context: report.context, authKey })
    }
    if (args.lifecycle) await require('./lib/gufo-lifecycle-acceptance.cjs').gufoLifecycleAcceptance({ client, mcp, id, port, model: s.served_model_name, data, record, owned, authKey, crash: index === 0 })
    await mcp.mutate('instances.stop', { id })
    assert.equal((await mcp.tool('instances.health', { id })).running, null)
    const probe = await fetch(`http://127.0.0.1:${port}/ready`, { signal: AbortSignal.timeout(2000) }).catch(() => null)
    assert.equal(probe, null, 'stopped instance port remains open')
    record('model_stopped', { id, port })
  }
  if (args['llama-engine']) {
    const llama = (await client.call('engines.list')).engines.find(e => fs.realpathSync.native(e.exe).toLowerCase() === fs.realpathSync.native(args['llama-engine']).toLowerCase())
    assert.equal(llama?.kind, 'llama_cpp')
    await client.mutate('engines.probe', { id: llama.id })
    const s = source.presets.find(p => p.settings.model_family === 'qwen38_27b').settings
    const id = 'llama-regression', port = await freePort()
    ids.push(id); ports.push(port)
    // Deliberately omit engine_kind: existing instance files must retain llama.cpp semantics.
    const config = { engine_id: llama.id, name: 'llama.cpp regression', model_path: s.model,
      alias: 'llama-regression', host: '127.0.0.1', port, ctx_size: 4096, gpu_layers: 99,
      gpu_layers_auto: false, parallel: 1, flash_attn: 'on', warmup: false, fit_mode: 'off' }
    await client.mutate('instances.create', { id, config, revision: (await client.call('service.status')).revision })
    const preview = await mcp.tool('instances.validate', { id })
    assert.ok(!preview.command.includes('serve'))
    await mcp.mutate('instances.start', { id })
    const deadline = Date.now() + 300000
    let health
    while (Date.now() < deadline) {
      health = await mcp.tool('instances.health', { id })
      if (health.running?.pid) owned.add(health.running.pid)
      if (health.health === 'ok') break
      assert.ok(health.running)
      await sleep(1000)
    }
    assert.equal(health.health, 'ok')
    const answer = await request(port, '/v1/chat/completions', { model: 'llama-regression',
      messages: [{ role: 'user', content: 'Compute 6 times 7. Reply only with the number.' }],
      max_tokens: 64, temperature: 0, reasoning_effort: 'none', chat_template_kwargs: { enable_thinking: false } })
    assert.match(answer.choices[0].message.content, /42/)
    if (args.extended) {
      const routerPort = await freePort(); ports.push(routerPort)
      const key = 'isolated-llama-audit-key'
      await client.mutate('routes.update', { revision: (await client.call('routes.get')).revision, patch: {
        host: '127.0.0.1', port: routerPort, strict_model_routing: true, default_instance_id: '',
        health_check_interval_ms: 1000, recovery_cooldown_ms: 1000,
        api_keys: [{ id: 'llama-audit', name: 'llama.cpp regression', key, enabled: true }],
        routes: [{ id: 'llama-audit', enabled: true, model_alias: id, target_instance_id: id, max_concurrent_requests: 1 }],
      } })
      await client.mutate('routes.start')
      await require('./lib/router-audit-acceptance.cjs').routerAuditAcceptance({ routerPort, port, model: id,
        upstreamModel: id, key, gufo: false, root: args.root, id, record })
      await client.mutate('routes.stop')
    }
    await client.mutate('instances.stop', { id })
    assert.equal((await mcp.tool('instances.health', { id })).running, null)
    record('llama_cpp_real_regression', { engine: llama.exe, model: s.model, answer: answer.choices[0].message.content, usage: answer.usage, port })
  }
  report.passed = true
}
main().catch(error => { report.passed = false; report.error = error.stack; console.error(error); process.exitCode = 1 }).finally(async () => {
  if (started) {
    for (const id of ids) await client.call('instances.stop', { id }, { mutate: true, allowFailure: true }).catch(() => {})
    await mcp?.close()
    const status = await client.call('service.status', {}, { allowFailure: true }).catch(() => null)
    if (status?.data?.runtime) owned.add(status.data.runtime.service_pid)
    await client.call('service.stop', {}, { mutate: true, allowFailure: true }).catch(() => {})
    await sleep(5000)
  }
  report.protection_after = Object.fromEntries(protectedPaths.map(p => [p, hash(p)]))
  report.protection_unchanged = JSON.stringify(report.protection_after) === JSON.stringify(before)
  report.original_cache_unchanged = JSON.stringify(cacheRoots.map(inventory)) === JSON.stringify(cacheBefore)
  report.remaining_owned_pids = [...owned].filter(alive)
  report.ports = ports
  report.finished_at = new Date().toISOString()
  if (!report.protection_unchanged || !report.original_cache_unchanged || report.remaining_owned_pids.length) { report.passed = false; process.exitCode = 1 }
  write(); console.log(JSON.stringify({ passed: report.passed, report: reportPath, remaining_owned_pids: report.remaining_owned_pids }))
})
