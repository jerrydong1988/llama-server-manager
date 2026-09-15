// Opt-in local integration test. Uses an isolated runtime directory and caller-supplied GGUFs.
// node scripts/test-router-real-models.cjs --engine PATH --generation PATH --embedding PATH --report PATH
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
  const service = spawnRuntime(executable, dataDir)
  let control, token, heartbeat
  const observations = []
  const tokensById = new Map()
  const report = { engine: args.engine, generation: args.generation, embedding: args.embedding, observations, storage: null }
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
      strict_model_routing: true, api_keys: [{ id: 'test-key', name: 'Real model test', key, enabled: true, max_concurrent_requests: 1, daily_token_budget: 1, monthly_token_budget: 1 },
        { id: 'other-key', name: 'Other caller', key: otherKey, enabled: true, max_concurrent_requests: 1 }], routes: [], runtime_service_enabled: true }
    await command({ command: 'sync_config', payload: { revision: ++revision, proxy_config: config, instances: {} } })
    await command({ command: 'start_proxy' })
    for (const workload of ['generation', 'embedding']) {
      const port = await reserveLoopbackPort()
      const id = `real-${workload}`
      const engineArgs = [args.engine, '-m', args[workload], '--host', '127.0.0.1', '--port', String(port), '-ngl', '0', '-t', '4', '-c', '4096', '-np', '2', '--metrics', '--slots']
      if (workload === 'embedding') engineArgs.push('--embedding', '--pooling', 'last', '-b', '4096', '-ub', '4096')
      const instance = { name: id, alias: id, host: '127.0.0.1', port, model_path: args[workload], context_size: 4096 }
      config.routes = [{ id, enabled: true, model_alias: id, target_instance_id: id }]
      await command({ command: 'sync_config', payload: { revision: ++revision, proxy_config: config, instances: { [id]: instance } } })
      await command({ command: 'start_instance', payload: { spec: { instance_id: id, config: instance, engine_backend: 'cpu', command: engineArgs, command_display: engineArgs.join(' '), workload: workload === 'generation' ? 'inference' : 'embedding', working_directory: path.dirname(args.engine) } } })
      const deadline = Date.now() + 120000
      for (;;) {
        try { if ((await fetch(`http://127.0.0.1:${port}/health`, { signal: AbortSignal.timeout(2000) })).ok) break } catch { /* loading */ }
        if (Date.now() > deadline) throw Error(`${workload} engine readiness timed out`)
        await sleep(500)
      }
      // Probe readiness also needs to propagate into the router health snapshot.
      await sleep(5500)
      const call = async (endpoint, body, apiKey = key) => {
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
        assert.ok(Number.isFinite(input), `${endpoint} did not report input tokens`)
        if (workload === 'generation') assert.ok(Number.isFinite(output), `${endpoint} did not report output tokens`)
        tokensById.set(requestId, { input, output, stream: !!body.stream, workload })
        observations.push({ endpoint, stream: !!body.stream, requestId, input, output, elapsedMs: Date.now() - started })
      }
      if (workload === 'generation') {
        for (const stream of [false, true]) {
          await call('/v1/chat/completions', { messages: [{ role: 'user', content: 'Reply with one short greeting.' }], max_tokens: 32, stream, ...(stream ? { stream_options: { include_usage: true } } : {}) })
          await call('/v1/responses', { input: 'Reply with one short greeting.', max_output_tokens: 32, stream })
          await call('/v1/messages', { messages: [{ role: 'user', content: 'Reply with one short greeting.' }], max_tokens: 32, stream })
        }
        // Run distinct callers concurrently and cancel a long response after headers.
        await Promise.all([call('/v1/chat/completions', { messages: [{ role: 'user', content: 'Say hello.' }], max_tokens: 16 }), call('/v1/chat/completions', { messages: [{ role: 'user', content: 'Say hello.' }], max_tokens: 16 }, otherKey)])
        const abort = new AbortController()
        const response = await fetch(`http://127.0.0.1:${proxyPort}/v1/chat/completions`, { method: 'POST', headers: { authorization: `Bearer ${key}`, 'content-type': 'application/json' }, body: JSON.stringify({ model: id, messages: [{ role: 'user', content: 'Count every integer from 1 to 1000.' }], max_tokens: 512, stream: true }), signal: abort.signal })
        assert.equal(response.status, 200); const reader = response.body.getReader(); await reader.read(); abort.abort(); await reader.cancel().catch(() => {})
      } else {
        await call('/v1/embeddings', { input: ['A short sentence.', 'Another sentence.'] })
        await call('/v1/embeddings', { input: 'Token accounting verification.' }, otherKey)
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
        assert.equal(record.tokens.input, expected.input, `input usage for ${id}: ${JSON.stringify(expected)}`)
        if (expected.workload === 'generation') assert.equal(record.tokens.output, expected.output)
        assert.equal(record.outcome, 'success'); assert.equal(record.queueEntered, true)
        if (!expected.stream) assert.equal(record.firstOutputMs, null)
      }
      const performance = db.prepare("SELECT SUM(json_extract(summary,'$.duration.count')) AS duration, SUM(json_extract(summary,'$.queue.count')) AS queue, SUM(json_extract(summary,'$.firstOutput.count')) AS firstOutput FROM usage_performance_daily").get()
      assert.equal(performance.duration, tokensById.size)
      assert.ok(performance.queue >= tokensById.size)
      assert.ok(performance.firstOutput > 0 && performance.firstOutput <= 3)
      assert.ok(records.some(r => r.outcome === 'cancelled'))
      report.storage = { verifiedRequests: tokensById.size, totalRecords: records.length, performance, budgetsAreAdvisory: true, permitsReleased: true, hotUpdateVerified: true }
    } finally { db.close() }
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
