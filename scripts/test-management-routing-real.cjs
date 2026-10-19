// Opt-in real engines, protocol/accounting and background recovery across CLI/MCP.
// --bin DIR --root DIR --engine FILE --model FILE --fallback FILE [--gpu-layers 99]
const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')
const { managementClient, sleep } = require('./lib/management-client.cjs')
const args = Object.fromEntries(Array.from({ length: (process.argv.length - 2) / 2 }, (_, i) => [process.argv[2 + i * 2].slice(2), process.argv[3 + i * 2]]))
for (const k of ['bin', 'root', 'engine', 'model', 'fallback']) assert.ok(path.isAbsolute(args[k]), `absolute --${k} required`)
fs.mkdirSync(args.root, { recursive: true })
const ext = process.platform === 'win32' ? '.exe' : ''
const client = managementClient({ cli: path.join(args.bin, `lsm${ext}`), app: path.join(args.bin, `llama-server-manager${ext}`), data: path.join(args.root, 'data'), timeout: 600000 })
const report = { started_at: new Date().toISOString(), checks: [], requests: [] }
const reportPath = path.join(args.root, 'routing-cli-mcp.json')
const record = (name, data = {}) => { report.checks.push({ name, ...data }); fs.writeFileSync(reportPath, JSON.stringify(report, null, 2)); console.log(JSON.stringify({ name, ...data })) }
const key = crypto.randomUUID(), limited = crypto.randomUUID(), discovery = crypto.randomUUID()
const primary = 'cross-primary', fallback = 'cross-fallback'
const day = Math.floor(Date.now() / 86400000) * 86400000
const query = { from: day, to: day + 86400000 }
let mcp
async function update(patch) { return client.mutate('routes.update', { revision: (await client.call('routes.get')).revision, patch }) }
async function ready(id) {
  for (let i = 0; i < 360; i++) {
    const value = await client.call('instances.health', { id })
    if (value.health === 'ok') return value
    await sleep(500)
  }
  throw Error(`${id} did not become ready`)
}
async function receipt(requestId) {
  for (let i = 0; i < 100; i++) {
    const page = await client.call('usage.requests', { query: { ...query, requestId } })
    if (page.records.length) return page.records[0]
    await sleep(100)
  }
  throw Error(`Usage missing for ${requestId}`)
}
async function call(endpoint, payload, { apiKey = key, expected = 200 } = {}) {
  const response = await fetch(`http://127.0.0.1:18192${endpoint}`, { method: 'POST', headers: { authorization: `Bearer ${apiKey}`, 'content-type': 'application/json', 'anthropic-version': '2023-06-01' }, body: JSON.stringify({ model: 'cross-model', ...payload }), signal: AbortSignal.timeout(180000) })
  const raw = await response.text()
  if (Array.isArray(expected)) { assert.ok(expected.includes(response.status), raw.slice(0, 1200)); expected = response.status }
  else assert.equal(response.status, expected, raw.slice(0, 1200))
  const id = response.headers.get('x-lsm-request-id')
  assert.ok(id, 'correlation header')
  const events = payload.stream ? raw.split('\n').filter(l => l.startsWith('data:') && !l.includes('[DONE]')).flatMap(l => { try { return [JSON.parse(l.slice(5))] } catch { return [] } }) : [JSON.parse(raw)]
  let input, output, cached, cacheWrite = 0
  for (const event of events) {
    const u = event.usage || event.response?.usage || event.message?.usage
    if (!u) continue
    input = u.prompt_tokens ?? u.input_tokens ?? input
    output = u.completion_tokens ?? u.output_tokens ?? output
    cached = u.prompt_tokens_details?.cached_tokens ?? u.input_tokens_details?.cached_tokens ?? u.cache_read_input_tokens ?? cached
    cacheWrite = u.cache_creation_input_tokens ?? cacheWrite
  }
  if (endpoint === '/v1/messages' && input != null) input += (cached || 0) + cacheWrite
  const stored = await receipt(id)
  if (expected === 200) {
    assert.equal(stored.outcome, 'success', JSON.stringify(stored))
    assert.equal(stored.tokens.input, input, `input accounting ${endpoint}`)
    assert.equal(stored.tokens.output, output, `output accounting ${endpoint}`)
  } else assert.equal(stored.outcome, 'rejected')
  report.requests.push({ id, endpoint, stream: !!payload.stream, status: expected, input, output, cached, stored })
  return stored
}
async function main() {
  await client.call('service.start')
  await client.mutate('usage.clear', { from: query.from, to: query.to })
  for (const [purpose, directory] of [['engine', path.dirname(args.engine)], ['model', path.dirname(args.model)], ['model', path.dirname(args.fallback)]]) {
    await client.mutate('paths.authorize', { purpose, path: directory })
    await client.mutate('directories.add', { purpose, path: directory, revision: (await client.call('service.status')).revision })
  }
  await client.mutate('models.refresh'); await client.mutate('engines.refresh')
  const engine = (await client.call('engines.list')).engines.find(e => path.basename(e.exe).toLowerCase() === path.basename(args.engine).toLowerCase())
  await client.mutate('engines.probe', { id: engine.id })
  for (const [id, model, port, gpu] of [[primary, args.model, 18190, Number(args['gpu-layers'] ?? 99)], [fallback, args.fallback, 18191, 0]]) {
    const existing = await client.call('instances.get', { id }, { allowFailure: true })
    const config = { name: id, engine_id: engine.id, model_path: model, host: '127.0.0.1', port, launch_mode: 'managed', ctx_size: 2048, ctx_size_auto: false, gpu_layers: gpu, gpu_layers_auto: false, fit_mode: 'off', warmup: false, parallel: 1, slots_enabled: true, metrics: true, props: true, mmproj_auto: false, chat_template: 'chatml' }
    await client.mutate(existing.ok ? 'instances.update' : 'instances.create', { id, revision: (await client.call('service.status')).revision, [existing.ok ? 'patch' : 'config']: config })
    await client.mutate('instances.start', { id }); await ready(id)
  }
  mcp = await client.mcp()
  const routes = [{ id: 'primary-route', enabled: true, model_alias: 'cross-model', target_instance_id: primary, priority: 0, weight: 1 }, { id: 'fallback-route', enabled: true, model_alias: 'cross-model', target_instance_id: fallback, priority: 1, weight: 2 }]
  await update({ host: '127.0.0.1', port: 18192, default_instance_id: primary, routes, strict_model_routing: true, routing_strategy: 'priorityFailover', max_concurrent_requests: 2, fair_queue_enabled: true, queue_timeout_ms: 1000, health_check_interval_ms: 500, unhealthy_threshold: 1, recovery_cooldown_ms: 0,
    api_keys: [{ id: 'cross-key', name: 'Cross test', key, enabled: true, scopes: ['inference', 'discovery'], max_concurrent_requests: 1, daily_token_budget: 1, monthly_token_budget: 1, daily_token_limit: 100000, monthly_token_limit: 200000, quota_default_output_tokens: 16 },
      { id: 'limited-key', name: 'Quota denial test', key: limited, enabled: true, daily_token_limit: 1, monthly_token_limit: 1 },
      { id: 'discovery-key', name: 'Discovery only test', key: discovery, enabled: true, scopes: ['discovery'] }] })
  await mcp.mutate('routes.start'); await sleep(1500)
  assert.equal(JSON.stringify(await client.call('routes.get')).includes(key), false)
  assert.equal(JSON.stringify(await mcp.tool('routes.get')).includes(key), false)
  record('two-real-engines-ready-and-keys-redacted')
  for (const stream of [false, true]) {
    await call('/v1/chat/completions', { messages: [{ role: 'user', content: 'Say hello.' }], max_tokens: 12, temperature: 0, stream, ...(stream ? { stream_options: { include_usage: true } } : {}) })
    await call('/v1/responses', { input: 'Say hello.', max_output_tokens: 12, stream })
    await call('/v1/messages', { messages: [{ role: 'user', content: 'Say hello.' }], max_tokens: 12, stream })
    await call('/v1/completions', { prompt: 'Once upon a time', max_tokens: 12, stream, ...(stream ? { stream_options: { include_usage: true } } : {}) })
  }
  record('four-protocols-stream-and-nonstream-accounted', { verified: report.requests.length })
  await call('/v1/chat/completions', { messages: [{ role: 'user', content: 'Hi' }], max_tokens: 8 }, { apiKey: 'invalid', expected: 401 })
  await call('/v1/chat/completions', { messages: [{ role: 'user', content: 'Hi' }], max_tokens: 8 }, { apiKey: discovery, expected: 403 })
  await call('/v1/chat/completions', { messages: [{ role: 'user', content: 'Hi' }], max_tokens: 8 }, { apiKey: limited, expected: 429 })
  await call('/v1/completions', { model: 'undeclared-model', prompt: 'Hello', max_tokens: 4 }, { expected: 404 })
  await call('/v1/completions', { prompt: 'word '.repeat(3000), max_tokens: 4 }, { expected: 400 })
  record('auth-scope-quota-selector-and-context-rejections')
  await mcp.mutate('instances.stop', { id: primary }); await sleep(1000)
  const failover = await call('/v1/completions', { prompt: 'Once upon a time', max_tokens: 8 })
  assert.equal(failover.instanceId, fallback)
  await client.mutate('instances.start', { id: primary }); await ready(primary); await sleep(1000)
  const recovered = await call('/v1/completions', { prompt: 'Once upon a time', max_tokens: 8 })
  assert.equal(recovered.instanceId, primary)
  record('real-stop-failover-and-primary-recovery')
  routes[1].priority = 0
  await update({ routes, routing_strategy: 'weighted' })
  const counts = {}
  for (let i = 0; i < 12; i++) { const r = await call('/v1/completions', { prompt: 'Once upon a time', max_tokens: 4 }); counts[r.instanceId] = (counts[r.instanceId] || 0) + 1 }
  assert.ok(counts[primary] >= 3 && counts[fallback] >= 6, JSON.stringify(counts))
  await update({ routing_strategy: 'leastBusy' })
  const concurrent = await Promise.all(Array.from({ length: 3 }, () => call('/v1/completions', { prompt: 'Once upon a time', max_tokens: 8 }, { expected: [200, 429] })))
  assert.ok(concurrent.some(r => r.outcome === 'success' && r.queueEntered))
  record('weighted-distribution-and-bounded-fair-queue', { counts, admitted: concurrent.filter(r => r.outcome === 'success').length, rejected: concurrent.filter(r => r.outcome === 'rejected').length })
  // Keep a real stream open so admission rejection is deterministic, independent
  // of how quickly a tiny fallback model completes concurrent requests.
  await update({ routes: [routes[0]], routing_strategy: 'priorityFailover', queue_timeout_ms: 1 })
  const abort = new AbortController()
  const long = await fetch('http://127.0.0.1:18192/v1/completions', { method: 'POST', headers: { authorization: `Bearer ${key}`, 'content-type': 'application/json' }, body: JSON.stringify({ model: 'cross-model', prompt: 'Once upon a time', max_tokens: 512, ignore_eos: true, stream: true }), signal: abort.signal })
  assert.equal(long.status, 200)
  const reader = long.body.getReader(); await reader.read()
  try { await call('/v1/completions', { prompt: 'Hello', max_tokens: 4 }, { expected: 429 }) }
  finally { abort.abort(); await reader.cancel().catch(() => {}) }
  const cancelled = await receipt(long.headers.get('x-lsm-request-id'))
  assert.equal(cancelled.outcome, 'cancelled')
  report.requests.push({ id: cancelled.requestId, endpoint: '/v1/completions', stream: true, status: 200, stored: cancelled })
  for (let i = 0; i < 50 && (await mcp.tool('routes.status')).admission?.active > 0; i++) await sleep(100)
  assert.equal((await mcp.tool('routes.status')).admission?.active, 0)
  await update({ queue_timeout_ms: 1000 })
  record('real-stream-queue-timeout-cancellation-and-permit-release', { observation: cancelled })
  const cli = await client.call('usage.summary', { query }), viaMcp = await mcp.tool('usage.summary', { query })
  assert.deepEqual(cli.summary, viaMcp.summary)
  assert.equal(cli.summary.input, report.requests.reduce((s,r)=>s+(r.stored.tokens.input ?? 0),0))
  assert.equal(cli.summary.output, report.requests.reduce((s,r)=>s+(r.stored.tokens.output ?? 0),0))
  assert.equal(cli.summary.requests, report.requests.length)
  assert.equal((await client.call('usage.export', { query, group: 'keys' })).content, (await mcp.tool('usage.export', { query, group: 'keys' })).content)
  const budgets = await mcp.tool('usage.budgets')
  const ownBudget = budgets.keys.find(k => k.id === 'cross-key')
  assert.equal(ownBudget.day.used, cli.summary.input + cli.summary.output)
  record('response-cli-mcp-summary-export-and-budget-reconciled', { summary: cli.summary, budget: ownBudget })
  const before = await client.call('service.status')
  const pidBefore = (await client.call('instances.health', { id: primary })).running.pid
  await mcp.mutate('runtime.detach', { enable: true }); await mcp.close(); mcp = null
  await sleep(4000)
  const detachedResponse = await fetch('http://127.0.0.1:18192/v1/completions', { method: 'POST', headers: { authorization: `Bearer ${key}`, 'content-type': 'application/json' }, body: JSON.stringify({ model: 'cross-model', prompt: 'Once upon a time', max_tokens: 4 }), signal: AbortSignal.timeout(180000) })
  const detachedBody = await detachedResponse.json()
  assert.equal(detachedResponse.status, 200, JSON.stringify(detachedBody))
  const after = await client.call('service.start')
  assert.notEqual(before.pid, after.pid)
  assert.equal((await client.call('instances.health', { id: primary })).running.pid, pidBefore)
  mcp = await client.mcp()
  assert.equal((await mcp.tool('routes.status')).running, true)
  const detachedUsage = await receipt(detachedResponse.headers.get('x-lsm-request-id'))
  assert.equal(detachedUsage.tokens.input, detachedBody.usage.prompt_tokens)
  assert.equal(detachedUsage.tokens.output, detachedBody.usage.completion_tokens)
  record('gui-host-replacement-keeps-real-engine-router-and-accounting', { oldHost: before.pid, newHost: after.pid, enginePid: pidBefore })
  await mcp.mutate('runtime.stop')
  const quotaBeforeClear = await mcp.tool('usage.budgets')
  await client.mutate('usage.clear', { from: query.from, to: query.to })
  const quotaAfterClear = await mcp.tool('usage.budgets')
  assert.deepEqual(quotaAfterClear.keys.map(k => k.quota), quotaBeforeClear.keys.map(k => k.quota))
  assert.equal((await client.call('usage.summary', { query })).summary.requests, 0)
  record('history-clear-preserves-hard-quota-ledger', { quota: quotaAfterClear.keys.map(k => ({ id: k.id, quota: k.quota })) })
  report.status = 'passed'
}
main().catch(e => { report.status = 'failed'; report.error = e.stack; console.error(e); process.exitCode = 1 })
  .finally(async () => {
    try { await client.mutate('routes.stop'); for (const id of [primary, fallback]) await client.mutate('instances.stop', { id }); await client.mutate('runtime.stop') } catch {}
    if (mcp) await mcp.close()
    report.finished_at = new Date().toISOString(); fs.writeFileSync(reportPath, JSON.stringify(report, null, 2))
  })
