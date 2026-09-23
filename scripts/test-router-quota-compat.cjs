// HTTP contract coverage without model downloads or access to the user's runtime.
const assert = require('node:assert/strict')
const fs = require('node:fs')
const http = require('node:http')
const os = require('node:os')
const path = require('node:path')
const { spawnRuntime, readToken, runtimeEndpoint, request, reserveLoopbackPort, waitForExit } = require('./test-runtime-service.cjs')
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const outputFields = ['max_tokens', 'max_completion_tokens', 'max_output_tokens', 'n_predict', 'max_new_tokens']

async function main() {
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'lsm-quota-compat-'))
  const received = [], counted = []
  let countsAvailable = true, service, token, endpoint, heartbeat
  const backend = http.createServer(async (req, res) => {
    const chunks = []
    for await (const chunk of req) chunks.push(chunk)
    const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString()) : {}
    res.setHeader('content-type', 'application/json')
    if (req.url === '/health') return res.end('{"status":"ok"}')
    if (req.url === '/props') return res.end('{"default_generation_settings":{"n_ctx":128}}')
    if (req.url === '/slots') return res.end('[{"id":0,"n_ctx":128,"is_processing":false}]')
    if (req.url === '/metrics') return res.end('')
    if (req.url === '/v1/models') return res.end('{"data":[{"id":"private-model"}]}')
    if (req.url.endsWith('/input_tokens') || req.url.endsWith('/count_tokens') || req.url === '/tokenize') {
      counted.push({ path: req.url, body })
      if (!countsAvailable) { res.statusCode = 404; return res.end('{}') }
      return res.end(JSON.stringify(req.url === '/tokenize' ? { tokens: Array(12).fill(1) } : { input_tokens: 12 }))
    }
    received.push({ path: req.url, body })
    const output = Math.min(2, Math.max(1, body.max_tokens ?? body.max_output_tokens ?? 2))
    const common = { id: 'fixture', model: 'private-model' }
    if (req.url === '/v1/messages') return res.end(JSON.stringify({ ...common, type: 'message', role: 'assistant', content: [{ type: 'text', text: 'OK' }], stop_reason: 'end_turn', usage: { input_tokens: 12, output_tokens: output } }))
    if (req.url === '/v1/responses') return res.end(JSON.stringify({ ...common, object: 'response', status: 'completed', output: [], usage: { input_tokens: 12, output_tokens: output, total_tokens: 12 + output } }))
    res.end(JSON.stringify({ ...common, choices: [{ index: 0, message: { role: 'assistant', content: 'OK' }, text: 'OK', finish_reason: 'stop' }], usage: { prompt_tokens: 12, completion_tokens: output, total_tokens: 12 + output } }))
  })
  try {
    await new Promise(resolve => backend.listen(0, '127.0.0.1', resolve))
    const port = backend.address().port, proxyPort = await reserveLoopbackPort()
    const executable = path.resolve(__dirname, '../src-tauri/target/debug', process.platform === 'win32' ? 'llama-server-manager.exe' : 'llama-server-manager')
    fs.mkdirSync(path.join(dataDir, 'configs'), { recursive: true })
    fs.writeFileSync(path.join(dataDir, 'configs', 'authorized-paths.json'), JSON.stringify({ engine_roots: [path.dirname(fs.realpathSync(process.execPath))], model_roots: [] }))
    service = spawnRuntime(executable, dataDir)
    token = await readToken(dataDir); endpoint = runtimeEndpoint(dataDir, token)
    const command = async value => {
      const reply = await request(endpoint, token, value, require('node:crypto').randomUUID())
      assert.ok(reply.reply && reply.reply.result !== 'error', JSON.stringify(reply))
      return reply.reply.payload
    }
    heartbeat = setInterval(() => { void command({ command: 'heartbeat', payload: { gui_pid: process.pid } }).catch(() => {}) }, 5000)
    const instance = { id: 'quota-fixture', name: 'Quota fixture', alias: 'private-model', host: '127.0.0.1', port, ctx_size: 128, ctx_size_auto: false }
    const key = 'isolated-quota-compat-test-key'
    const config = { enabled: true, host: '127.0.0.1', port: proxyPort, strict_model_routing: true, api_keys: [], routes: [{ id: 'route', enabled: true, model_alias: 'localmodel', target_instance_id: instance.id }], runtime_service_enabled: true }
    let revision = Date.now(), caseIndex = 0
    const configure = async (daily, fallback = 32) => {
      config.api_keys = [{ id: `case-${++caseIndex}`, name: 'Compatibility fixture', key, enabled: true, daily_token_budget: 1, monthly_token_budget: 1, daily_token_limit: daily, monthly_token_limit: daily, ...(fallback === null ? {} : { quota_default_output_tokens: fallback }) }]
      await command({ command: 'sync_config', payload: { revision: ++revision, proxy_config: config, instances: { [instance.id]: instance } } })
    }
    await configure(1000)
    const engineCommand = [process.execPath, '-e', 'setInterval(() => {}, 1000)']
    const launchSpec = { instance_id: instance.id, config: instance, engine_backend: 'test', executable_sha256: require('node:crypto').createHash('sha256').update(fs.readFileSync(process.execPath)).digest('hex'), command: engineCommand, command_display: 'isolated quota fixture', workload: 'inference', working_directory: dataDir }
    let deniedLaunch = await request(endpoint, token, { command: 'start_instance', payload: { spec: { ...launchSpec, executable_sha256: '0'.repeat(64) } } }, 'changed-engine')
    assert.ok(deniedLaunch.error?.includes('引擎文件已更改'), JSON.stringify(deniedLaunch))
    const grantFile = path.join(dataDir, 'configs', 'authorized-paths.json')
    const grants = fs.readFileSync(grantFile)
    fs.writeFileSync(grantFile, JSON.stringify({ engine_roots: [] }))
    deniedLaunch = await request(endpoint, token, { command: 'start_instance', payload: { spec: launchSpec } }, 'revoked-engine')
    assert.ok(deniedLaunch.error?.includes('未获授权'), JSON.stringify(deniedLaunch))
    fs.writeFileSync(grantFile, grants)
    await command({ command: 'start_instance', payload: { spec: launchSpec } })
    await command({ command: 'start_proxy' })
    for (let i = 0; ; i++) {
      if ((await command({ command: 'get_status' })).proxy.healthy_routes === 1) break
      assert.ok(i < 100, 'Fixture route never became ready'); await sleep(100)
    }
    const post = (route, body) => fetch(`http://127.0.0.1:${proxyPort}${route}`, { method: 'POST', headers: { authorization: `Bearer ${key}`, 'content-type': 'application/json', 'anthropic-version': '2023-06-01' }, body: JSON.stringify({ model: 'localmodel', ...body }), signal: AbortSignal.timeout(15000) })
    const prompt = { messages: [{ role: 'user', content: 'Hello' }] }
    const cases = [
      ['/v1/chat/completions', prompt, 32], // Octop / LangChain default request
      ['/v1/chat/completions', { ...prompt, max_tokens: null }, 32],
      ['/v1/chat/completions', { ...prompt, max_tokens: 0 }, 0],
      ['/v1/chat/completions', { ...prompt, max_tokens: -1 }, 32],
      ['/v1/chat/completions', { ...prompt, n_predict: -2 }, 32],
      ['/v1/chat/completions', { ...prompt, max_completion_tokens: '48' }, 48],
      ['/v1/chat/completions', { ...prompt, max_tokens: 64, max_completion_tokens: 48, max_new_tokens: 80, n_predict: -1, n: '1', best_of: 1 }, 48],
      ['/v1/responses', { input: 'Hello', max_output_tokens: null }, 32],
      ['/v1/responses', { input: 'Hello', max_tokens: 48 }, 48],
      ['/v1/messages', { ...prompt, max_tokens: null }, 32],
      ['/v1/messages', { ...prompt, max_new_tokens: 48 }, 48],
      ['/v1/completions', { prompt: 'Hello' }, 32],
      ['/v1/completions', { prompt: ['Hello'], n_predict: 48, best_of: 1 }, 48],
    ]
    for (const [route, body, output] of cases) {
      await configure(12 + Math.max(1, output))
      const before = received.length
      const response = await post(route, body)
      assert.equal(response.status, 200, await response.text())
      assert.equal(received.length, before + 1)
      const upstream = received.at(-1).body
      const canonical = route === '/v1/responses' ? 'max_output_tokens' : 'max_tokens'
      assert.equal(upstream[canonical], output)
      assert.deepEqual(outputFields.filter(field => field in upstream), [canonical])
      assert.equal(upstream.model, 'private-model')
      for (const field of ['n', 'best_of', 'num_return_sequences']) assert.equal(field in upstream, false)
      const blocked = await post(route, body)
      assert.equal(blocked.status, 429, await (blocked.status === 429 ? Promise.resolve('') : blocked.text()))
      assert.equal((await blocked.json()).error.code, 'token_quota_exceeded')
      assert.equal(received.length, before + 1, 'Quota denied a request after forwarding')
    }
    // Migrated config without the new field defaults to 32768, then fits 128 - 12.
    await configure(128, null)
    let response = await post('/v1/chat/completions', prompt)
    assert.equal(response.status, 200, await response.text())
    assert.equal(received.at(-1).body.max_tokens, 116)
    await configure(1000)
    let before = received.length
    response = await post('/v1/chat/completions', { ...prompt, max_tokens: 128 })
    assert.equal(response.status, 400); assert.equal((await response.json()).error.code, 'context_length_exceeded')
    assert.equal(received.length, before)
    for (const [body, code, param] of [
      [{ ...prompt, max_tokens: 1.5 }, 'token_quota_invalid_limit', 'max_tokens'],
      [{ ...prompt, n: 2 }, 'token_quota_multiple_generations', 'n'],
      [{ ...prompt, best_of: 2 }, 'token_quota_multiple_generations', 'best_of'],
      [{ ...prompt, num_return_sequences: 2 }, 'token_quota_multiple_generations', 'num_return_sequences'],
    ]) {
      response = await post('/v1/chat/completions', body)
      assert.equal(response.status, 400)
      const error = (await response.json()).error
      assert.equal(error.code, code); assert.equal(error.param, param)
      assert.equal(received.length, before)
    }
    response = await post('/v1/completions', { prompt: ['one', 'two'] })
    assert.equal(response.status, 400); assert.equal((await response.json()).error.param, 'prompt')
    countsAvailable = false
    response = await post('/v1/chat/completions', prompt)
    assert.equal(response.status, 400); assert.equal((await response.json()).error.code, 'token_quota_unmetered')
    assert.equal(received.length, before)
    // Soft-only keys retain their parameters; the ordinary context check still fails open.
    await configure(0)
    const countsBefore = counted.length
    const legacy = { ...prompt, max_tokens: null, n_predict: -1, best_of: 2 }
    response = await post('/v1/chat/completions', legacy)
    assert.equal(response.status, 200, await response.text())
    assert.deepEqual(received.at(-1).body, { ...legacy, model: 'private-model' })
    assert.equal(counted.length, countsBefore + 1)
    countsAvailable = true
    // Unsupported fields must not destroy tools, images, sampling or caller metadata.
    await configure(1000)
    const rich = { messages: [{ role: 'user', content: [{ type: 'image_url', image_url: { url: 'data:image/png;base64,dGVzdA==' } }] }], tools: [{ type: 'function', function: { name: 'example', parameters: { type: 'object' } } }], tool_choice: 'auto', temperature: 0.6, metadata: { user: 'fixture' } }
    response = await post('/v1/chat/completions', rich)
    assert.equal(response.status, 200, await response.text())
    const forwarded = { ...received.at(-1).body }; delete forwarded.model; delete forwarded.max_tokens
    assert.deepEqual(forwarded, rich)
    await configure(1)
    before = received.length
    response = await post('/v1/chat/completions/input_tokens', prompt)
    assert.equal(response.status, 200); assert.equal((await response.json()).input_tokens, 12)
    assert.equal(received.length, before)
    // Hot configuration changes must never turn disabled keys into anonymous access.
    const syncKeys = async keys => {
      config.api_keys = keys
      await command({ command: 'sync_config', payload: { revision: ++revision, proxy_config: config, instances: { [instance.id]: instance } } })
    }
    const authRequest = (route, headers = {}, method = 'POST') => fetch(`http://127.0.0.1:${proxyPort}${route}`, {
      method, headers: { 'content-type': 'application/json', 'anthropic-version': '2023-06-01', ...headers },
      ...(method === 'POST' ? { body: JSON.stringify({ model: 'localmodel', ...prompt }) } : {}),
      signal: AbortSignal.timeout(15000),
    })
    const credentials = [{}, { authorization: `Bearer ${key}` }, { 'x-api-key': key }, { authorization: 'Bearer wrong-key' }, { 'x-api-key': 'wrong-key' }]
    const limitedKey = { ...config.api_keys[0] }
    for (const headers of [credentials[0], credentials[3], credentials[4]]) {
      assert.equal((await authRequest('/v1/chat/completions', headers)).status, 401)
    }
    assert.equal((await authRequest('/v1/chat/completions', credentials[1])).status, 429)
    const deniedRoutes = ['/v1/chat/completions', '/v1/completions', '/v1/responses', '/v1/messages', '/v1/messages/count_tokens', '/v1/chat/completions/input_tokens', '/v1/responses/input_tokens', '/v1/embeddings', '/embeddings', '/embedding', '/v1/rerank', '/rerank', '/reranking', '/v1/reranking']
    before = received.length
    for (const keys of [
      [{ ...limitedKey, enabled: false }],
      [{ ...limitedKey, enabled: false }, { ...limitedKey, id: 'disabled-2', enabled: false, key: 'second-disabled-fixture-key' }],
      [{ ...limitedKey, enabled: false, key: '' }],
    ]) {
      await syncKeys(keys)
      for (const headers of credentials) {
        for (const route of deniedRoutes) assert.equal((await authRequest(route, headers)).status, 401, route)
        for (const route of ['/v1/models', '/v1/models/localmodel', '/slots', '/health', '/live', '/props', '/ready', '/metrics', '/']) {
          assert.equal((await authRequest(route, headers, 'GET')).status, 401, route)
        }
      }
    }
    assert.equal(received.length, before, 'Disabled credentials reached inference')
    const activeKey = { ...limitedKey, id: 'active-peer', key: 'active-peer-fixture-key', daily_token_limit: 0, monthly_token_limit: 0 }
    await syncKeys([{ ...limitedKey, enabled: false }, activeKey])
    assert.equal((await authRequest('/v1/chat/completions', credentials[1])).status, 401)
    assert.equal((await authRequest('/v1/chat/completions', { 'x-api-key': activeKey.key })).status, 200)
    // Re-enabling restores the original quota, not a fresh anonymous identity.
    await syncKeys([limitedKey])
    const limited = await authRequest('/v1/chat/completions', credentials[2])
    assert.equal(limited.status, 429); assert.equal((await limited.json()).error.code, 'token_quota_exceeded')
    await syncKeys([])
    for (const headers of credentials) assert.equal((await authRequest('/v1/chat/completions', headers)).status, 200)
    console.log('Authentication passed: disabled/blank/all/mixed keys; Bearer and x-api-key; inference/discovery aliases; hot reload; quota preservation; explicit anonymous mode.')
    console.log(`Quota compatibility passed: ${cases.length} client shapes; default/context fit; exact reservation boundary; actionable errors; soft-only pass-through; count exemption; tools/images preserved.`)
  } finally {
    clearInterval(heartbeat)
    if (service && endpoint && token && service.exitCode === null) await request(endpoint, token, { command: 'shutdown', payload: { stop_instances: true } }, 'cleanup').catch(() => {})
    if (service) { await waitForExit(service, 15000); if (service.exitCode === null) { service.kill(); await waitForExit(service, 5000) } }
    backend.closeAllConnections()
    await new Promise(resolve => backend.close(resolve))
    const resolved = path.resolve(dataDir)
    assert.ok(resolved.startsWith(path.resolve(os.tmpdir()) + path.sep) && path.basename(resolved).startsWith('lsm-quota-compat-'))
    if (!service || service.exitCode !== null) fs.rmSync(resolved, { recursive: true, force: true })
  }
}
main().catch(error => { console.error(error); process.exitCode = 1 })
