// Opt-in local integration test. Uses an isolated runtime directory and caller-supplied GGUFs.
// node scripts/test-router-real-models.cjs --engine PATH --generation PATH --embedding PATH [--reranking PATH] --report PATH
const fs = require('node:fs')
const path = require('node:path')
const os = require('node:os')
const assert = require('node:assert/strict')
const { DatabaseSync } = require('node:sqlite')
const { spawnRuntime, readToken, runtimeEndpoint, request, reserveLoopbackPort, waitForExit } = require('./test-runtime-service.cjs')
const args = Object.fromEntries(Array.from({ length: Math.floor((process.argv.length - 2) / 2) }, (_, i) => [process.argv[2 + i * 2].replace(/^--/, ''), process.argv[3 + i * 2]]))
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))

async function main() {
  for (const field of ['engine', 'generation', 'embedding']) assert.ok(args[field] && fs.existsSync(args[field]), `missing --${field}`)
  const executable = path.resolve(__dirname, '../src-tauri/target/debug', process.platform === 'win32' ? 'llama-server-manager.exe' : 'llama-server-manager')
  assert.ok(fs.existsSync(executable), 'Build the current runtime first')
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'lsm-real-models-'))
  let service = spawnRuntime(executable, dataDir)
  let control, token, heartbeat
  const observations = []
  const tokensById = new Map()
  const report = { engine: args.engine, generation: args.generation, embedding: args.embedding, reranking: args.reranking, observations, storage: null, quota: {} }
  const ledger = () => {
    const db = new DatabaseSync(path.join(dataDir, 'router-quota.db'), { readOnly: true })
    db.exec('PRAGMA busy_timeout=5000')
    try { return { requests: db.prepare('SELECT * FROM quota_requests ORDER BY id').all(), periods: db.prepare('SELECT * FROM quota_periods ORDER BY key_id,kind,start').all() } } finally { db.close() }
  }
  try {
    token = await readToken(dataDir); control = runtimeEndpoint(dataDir, token)
    const command = async (value) => {
      const response = await request(control, token, value, require('node:crypto').randomUUID())
      assert.ok(response.reply && response.reply.result !== 'error', JSON.stringify(response))
      return response.reply.payload
    }
    heartbeat = setInterval(() => { void command({ command: 'heartbeat', payload: { gui_pid: process.pid } }).catch(() => {}) }, 5000)
    const proxyPort = await reserveLoopbackPort()
    const key = 'isolated-router-real-model-key'
    const otherKey = 'isolated-router-other-key'
    let revision = Date.now()
    const config = { enabled: true, host: '127.0.0.1', port: proxyPort, max_concurrent_requests: 3, fair_queue_enabled: true, queue_timeout_ms: 5000,
      strict_model_routing: true, api_keys: [{ id: 'test-key', name: 'Real model test', key, enabled: true, max_concurrent_requests: 1, daily_token_budget: 1, monthly_token_budget: 1, daily_token_limit: 100000, monthly_token_limit: 200000, quota_default_output_tokens: 32 },
        { id: 'other-key', name: 'Other caller', key: otherKey, enabled: true, max_concurrent_requests: 1, daily_token_limit: 100000, monthly_token_limit: 200000 }], routes: [], runtime_service_enabled: true }
    await command({ command: 'sync_config', payload: { revision: ++revision, proxy_config: config, instances: {} } })
    await command({ command: 'start_proxy' })
    for (const workload of ['generation', 'embedding', ...(args.reranking ? ['reranking'] : [])]) {
      const port = await reserveLoopbackPort()
      const id = `real-${workload}`
      const engineArgs = [args.engine, '-m', args[workload], '--host', '127.0.0.1', '--port', String(port), '-ngl', '0', '-t', '4', '-c', '4096', '-np', '2', '--metrics', '--slots']
      if (workload === 'embedding') engineArgs.push('--embedding', '--pooling', 'last', '-b', '4096', '-ub', '4096')
      if (workload === 'reranking') engineArgs.push('--embedding', '--pooling', 'rank', '-b', '4096', '-ub', '4096')
      const instance = { name: id, alias: id, host: '127.0.0.1', port, model_path: args[workload], context_size: 4096 }
      config.routes = [{ id, enabled: true, model_alias: id, target_instance_id: id }]
      await command({ command: 'sync_config', payload: { revision: ++revision, proxy_config: config, instances: { [id]: instance } } })
      await command({ command: 'start_instance', payload: { spec: { instance_id: id, config: instance, engine_backend: 'cpu', command: engineArgs, command_display: engineArgs.join(' '), workload: workload === 'generation' ? 'inference' : workload === 'reranking' ? 'reranker' : 'embedding', working_directory: path.dirname(args.engine) } } })
      const deadline = Date.now() + 120000
      for (;;) {
        try { if ((await fetch(`http://127.0.0.1:${port}/health`, { signal: AbortSignal.timeout(2000) })).ok) break } catch { /* loading */ }
        if (Date.now() > deadline) throw Error(`${workload} engine readiness timed out`)
        await sleep(500)
      }
      // Probe readiness also needs to propagate into the router health snapshot.
      await sleep(5500)
      const call = async (endpoint, body, apiKey = key, known = true) => {
        const started = Date.now()
        const response = await fetch(`http://127.0.0.1:${proxyPort}${endpoint}`, { method: 'POST', headers: { authorization: `Bearer ${apiKey}`, 'content-type': 'application/json', 'anthropic-version': '2023-06-01' }, body: JSON.stringify({ model: id, ...body }), signal: AbortSignal.timeout(120000) })
        const text = await response.text()
        assert.equal(response.status, 200, text.slice(0, 600))
        const requestId = response.headers.get('x-lsm-request-id'); assert.ok(requestId)
        let input, output, cacheRead = 0, cacheWrite = 0
        const events = body.stream ? text.split('\n').filter(l => l.startsWith('data:') && !l.includes('[DONE]')).flatMap(l => { try { return [JSON.parse(l.slice(5))] } catch { return [] } }) : [JSON.parse(text)]
        for (const event of events) {
          const usage = event.usage || event.response?.usage || event.message?.usage
          if (!usage) continue
          input = usage.prompt_tokens ?? usage.input_tokens ?? input
          output = usage.completion_tokens ?? usage.output_tokens ?? output
          cacheRead = usage.cache_read_input_tokens ?? cacheRead
          cacheWrite = usage.cache_creation_input_tokens ?? cacheWrite
        }
        if (endpoint === '/v1/messages' && input != null) input += cacheRead + cacheWrite
        const clientUsage = endpoint !== '/v1/chat/completions' || !body.stream || body.stream_options?.include_usage === true
        if (known && clientUsage) assert.ok(Number.isFinite(input), `${endpoint} did not report input tokens`)
        else assert.equal(input, undefined, `${endpoint} unexpectedly reported usage; update this verification`)
        if (workload === 'generation' && clientUsage) assert.ok(Number.isFinite(output), `${endpoint} did not report output tokens`)
        tokensById.set(requestId, { input, output, stream: !!body.stream, workload, known, clientUsage })
        observations.push({ endpoint, stream: !!body.stream, requestId, input, output, clientUsage, elapsedMs: Date.now() - started })
      }
      if (workload === 'generation') {
        for (const stream of [false, true]) {
          await call('/v1/chat/completions', { messages: [{ role: 'user', content: 'Reply with one short greeting.' }], max_tokens: 32, stream, ...(stream ? { stream_options: { include_usage: true } } : {}) })
          await call('/v1/responses', { input: 'Reply with one short greeting.', max_output_tokens: 32, stream })
          await call('/v1/messages', { messages: [{ role: 'user', content: 'Reply with one short greeting.' }], max_tokens: 32, stream })
          await call('/v1/completions', { prompt: 'Hello, my name is', max_tokens: 16, stream, ...(stream ? { stream_options: { include_usage: true } } : {}) })
        }
        // SDKs such as Octop omit the output cap; all protocol/stream variants must work.
        for (const stream of [false, true]) {
          await call('/v1/chat/completions', { messages: [{ role: 'user', content: 'Say hello.' }], stream })
          await call('/v1/responses', { input: 'Say hello.', max_output_tokens: null, stream })
          await call('/v1/messages', { messages: [{ role: 'user', content: 'Say hello.' }], stream })
          await call('/v1/completions', { prompt: ['Hello'], n_predict: -1, stream })
        }
        await call('/v1/chat/completions', { messages: [{ role: 'user', content: 'Say hello.' }], max_tokens: 64, max_completion_tokens: '32', n_predict: -1, best_of: 1 })
        await call('/v1/chat/completions', { messages: [{ role: 'user', content: 'Prefill only.' }], max_tokens: 0 })
        // Run distinct callers concurrently and cancel a long response after headers.
        await Promise.all([call('/v1/chat/completions', { messages: [{ role: 'user', content: 'Say hello.' }], max_tokens: 16 }), call('/v1/chat/completions', { messages: [{ role: 'user', content: 'Say hello.' }], max_tokens: 16 }, otherKey)])
        const abort = new AbortController()
        const response = await fetch(`http://127.0.0.1:${proxyPort}/v1/chat/completions`, { method: 'POST', headers: { authorization: `Bearer ${key}`, 'content-type': 'application/json' }, body: JSON.stringify({ model: id, messages: [{ role: 'user', content: 'Count every integer from 1 to 1000.' }], max_tokens: 512, stream: true }), signal: abort.signal })
        assert.equal(response.status, 200); const reader = response.body.getReader(); await reader.read(); abort.abort(); await reader.cancel().catch(() => {})
      } else if (workload === 'embedding') {
        await call('/v1/embeddings', { input: ['A short sentence.', 'Another sentence.'] })
        await call('/v1/embeddings', { input: 'Token accounting verification.' }, otherKey)
        await call('/embedding', { content: 'Native embedding accounting.' }, key, false)
        await call('/embeddings', { content: ['One.', 'Two.'] }, otherKey, false)
      } else {
        const body = { query: 'What is the capital of France?', documents: ['Paris is the capital of France.', 'Berlin is the capital of Germany.', 'Bananas are yellow.'], top_n: 1 }
        for (const endpoint of ['/rerank', '/reranking', '/v1/rerank', '/v1/reranking']) await call(endpoint, body)
        await call('/rerank', { query: body.query, texts: body.documents, top_n: 1 }, otherKey, false)
      }
      for (let attempt = 0; ; attempt++) {
        const state = await command({ command: 'get_status' })
        if (state.proxy.admission.active === 0 && state.proxy.admission.queued === 0) break
        if (attempt >= 100) throw Error('Admission permit leaked after completion/cancellation')
        await sleep(100)
      }
      config.api_keys[0].max_concurrent_requests = 2
      await command({ command: 'sync_config', payload: { revision: ++revision, proxy_config: config, instances: { [id]: instance } } })
      assert.equal((await command({ command: 'get_status' })).proxy.admission.keys.find(k => k.id === 'test-key').limit, 2)
      if (workload === 'generation') {
        const post = (endpoint, body, signal) => fetch(`http://127.0.0.1:${proxyPort}${endpoint}`, { method: 'POST', headers: { authorization: `Bearer ${key}`, 'content-type': 'application/json', 'anthropic-version': '2023-06-01' }, body: JSON.stringify({ model: id, ...body }), signal: signal || AbortSignal.timeout(30000) })
        for (const [body, code, param] of [[{ max_tokens: 1.5 }, 'token_quota_invalid_limit', 'max_tokens'], [{ max_tokens: 10, n: 2 }, 'token_quota_multiple_generations', 'n']]) {
          const rejected = await post('/v1/chat/completions', { messages: [{ role: 'user', content: 'Hello' }], ...body })
          assert.equal(rejected.status, 400)
          const error = (await rejected.json()).error
          assert.equal(error.code, code); assert.equal(error.param, param)
        }
        await sleep(250)
        const countBefore = ledger().requests.length
        const probe = await post('/v1/chat/completions/input_tokens', { messages: [{ role: 'user', content: 'Count every integer from 1 to 1000.' }] })
        assert.equal(probe.status, 200)
        const input = (await probe.json()).input_tokens
        assert.equal(ledger().requests.length, countBefore, 'Count-only calls must not reserve quota')
        const day = ledger().periods.find(p => p.key_id === 'test-key' && p.kind === 'day')
        config.api_keys[0].daily_token_limit = day.settled + day.held + input + 512
        await command({ command: 'sync_config', payload: { revision: ++revision, proxy_config: config, instances: { [id]: instance } } })
        const abort = new AbortController()
        const long = await post('/v1/chat/completions', { messages: [{ role: 'user', content: 'Count every integer from 1 to 1000.' }], max_tokens: 512, ignore_eos: true, stream: true }, abort.signal)
        assert.equal(long.status, 200, await (long.status !== 200 ? long.text() : Promise.resolve('')))
        const reader = long.body.getReader(); await reader.read()
        const blocked = await post('/v1/messages', { messages: [{ role: 'user', content: 'Hello' }], max_tokens: 16 })
        const rejection = await blocked.json()
        assert.equal(blocked.status, 429, JSON.stringify(rejection))
        assert.equal(rejection.type, 'error'); assert.equal(rejection.error.code, 'token_quota_exceeded')
        assert.equal(rejection.request_id, blocked.headers.get('request-id'))
        abort.abort(); await reader.cancel().catch(() => {})
        await sleep(500)
        assert.equal(ledger().requests.length, countBefore + 1, 'Denied calls must not create reservations')
        assert.ok(ledger().requests.some(r => r.id === long.headers.get('x-lsm-request-id') && r.state === 'held'))
        config.api_keys[0].daily_token_limit = 100000
        await command({ command: 'sync_config', payload: { revision: ++revision, proxy_config: config, instances: { [id]: instance } } })
        report.quota.concurrentDenial = true
        report.quota.outputAliasesNormalized = true
        report.quota.missingLimitsCompatible = true
        report.quota.countEndpointsExempt = true
      }
      config.api_keys[0].max_concurrent_requests = 1
      await command({ command: 'stop_instance', payload: { instance_id: id } })
    }
    await command({ command: 'shutdown', payload: { stop_instances: true } })
    assert.equal(await waitForExit(service, 15000), 0)
    const db = new DatabaseSync(path.join(dataDir, 'router-usage.db'), { readOnly: true })
    try {
      const records = db.prepare('SELECT record FROM usage_events').all().map(r => JSON.parse(r.record))
      for (const [id, expected] of tokensById) {
        const record = records.find(r => r.requestId === id); assert.ok(record, id)
        if (expected.clientUsage) {
          assert.equal(record.tokens.input, expected.input ?? null, `input usage for ${id}: ${JSON.stringify(expected)}`)
          if (expected.workload === 'generation') assert.equal(record.tokens.output, expected.output)
        } else {
          assert.ok(Number.isFinite(record.tokens.input) && Number.isFinite(record.tokens.output), 'Hidden usage must still be accounted')
          Object.assign(observations.find(r => r.requestId === id), { storedInput: record.tokens.input, storedOutput: record.tokens.output })
        }
        assert.equal(record.outcome, 'success'); assert.equal(record.queueEntered, true)
        assert.equal(record.quality, expected.known ? 'complete' : 'unknown')
        const quota = ledger().requests.find(r => r.id === id); assert.ok(quota, id)
        if (expected.known) {
          assert.equal(quota.state, 'settled'); assert.equal(quota.actual, record.tokens.input + (record.tokens.output || 0))
          assert.ok(quota.reserved >= quota.actual, `Preflight under-reserved ${id}: ${JSON.stringify({ quota, record, expected })}`)
        } else { assert.equal(quota.state, 'held'); assert.equal(quota.actual, null) }
        if (!expected.stream) assert.equal(record.firstOutputMs, null)
      }
      const performance = db.prepare("SELECT SUM(json_extract(summary,'$.duration.count')) AS duration, SUM(json_extract(summary,'$.queue.count')) AS queue, SUM(json_extract(summary,'$.firstOutput.count')) AS firstOutput FROM usage_performance_daily").get()
      assert.equal(performance.duration, tokensById.size)
      assert.ok(performance.queue >= tokensById.size)
      assert.ok(performance.firstOutput > 0 && performance.firstOutput <= [...tokensById.values()].filter(v => v.stream).length)
      assert.ok(records.some(r => r.outcome === 'cancelled'))
      report.storage = { verifiedRequests: tokensById.size, totalRecords: records.length, performance, softBudgetsAreAdvisory: true, permitsReleased: true, hotUpdateVerified: true }
    } finally { db.close() }
    const before = ledger()
    service = spawnRuntime(executable, dataDir)
    token = await readToken(dataDir); control = runtimeEndpoint(dataDir, token)
    for (let attempt = 0; ; attempt++) {
      try { await command({ command: 'get_status' }); break } catch (error) { if (attempt > 100) throw error; await sleep(100) }
    }
    assert.deepEqual(ledger(), before, 'Runtime restart changed quota reservations')
    await command({ command: 'shutdown', payload: { stop_instances: true } })
    assert.equal(await waitForExit(service, 15000), 0)
    report.quota.restartPreservesLedger = true
    const history = new DatabaseSync(path.join(dataDir, 'router-usage.db'))
    try { history.exec('DELETE FROM usage_events; DELETE FROM usage_daily; DELETE FROM usage_performance_daily;') } finally { history.close() }
    assert.deepEqual(ledger(), before, 'Deleting usage history changed the quota ledger')
    report.quota.historyDeletionIndependent = true
    report.quota.ledger = before
    if (args.report) fs.writeFileSync(path.resolve(args.report), JSON.stringify(report, null, 2), 'utf8')
    console.log(JSON.stringify(report, null, 2))
  } finally {
    clearInterval(heartbeat)
    if (control && token && service.exitCode === null) await request(control, token, { command: 'shutdown', payload: { stop_instances: true } }, 'cleanup').catch(() => {})
    await waitForExit(service, 15000)
    if (service.exitCode === null) { service.kill(); await waitForExit(service, 5000) }
    const resolved = path.resolve(dataDir)
    assert.ok(resolved.startsWith(path.resolve(os.tmpdir()) + path.sep) && path.basename(resolved).startsWith('lsm-real-models-'))
    if (service.exitCode !== null) fs.rmSync(resolved, { recursive: true, force: true })
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
