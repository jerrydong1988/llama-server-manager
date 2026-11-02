// Opt-in real direct/proxy contracts, invoked by test-gufo-real.cjs --extended true.
const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const { sleep } = require('./management-client.cjs')

async function gufoApiAcceptance({ client, mcp, id, model, port, routerPort, root, record, context, authKey = '' }) {
  const publicModel = `managed-${id}`, key = `isolated-${id}-contract-key`
  await client.mutate('routes.update', { revision: (await client.call('routes.get')).revision, patch: {
    host: '127.0.0.1', port: routerPort, strict_model_routing: true, health_check_interval_ms: 1000,
    recovery_cooldown_ms: 1000, timeout_ms: 180000, streaming_idle_timeout_ms: 180000,
    api_keys: [{ id: `key-${id}`, name: 'Isolated acceptance', key, enabled: true }],
    routes: [{ id: `route-${id}`, enabled: true, model_alias: publicModel, target_instance_id: id, max_concurrent_requests: 1 }],
  } })
  await client.mutate('routes.start')
  for (let i = 0; ; i++) {
    if ((await client.call('routes.status')).healthy_routes === 1) break
    assert.ok(i < 60, 'Gufo route did not become ready'); await sleep(1000)
  }
  const events = text => text.split(/\r?\n/).filter(line => line.startsWith('data: ') && line !== 'data: [DONE]').map(line => JSON.parse(line.slice(6)))
  const captures = []
  const send = async (proxy, route, body, options = {}) => {
    const res = await fetch(`http://127.0.0.1:${proxy ? routerPort : port}${route}`, {
      method: body ? 'POST' : 'GET', headers: { 'content-type': 'application/json',
        'anthropic-version': '2023-06-01', ...(proxy ? { authorization: `Bearer ${options.key || key}` } : authKey ? { authorization: `Bearer ${authKey}` } : {}) },
      ...(body ? { body: JSON.stringify({ ...body, model: proxy ? publicModel : model }) } : {}), signal: AbortSignal.timeout(180000),
    })
    const text = await res.text()
    let value; try { value = JSON.parse(text) } catch { value = text }
    captures.push({ proxy, route, status: res.status, body: value })
    fs.writeFileSync(path.join(root, `${id}-api.json`), JSON.stringify(captures, null, 2))
    if (!options.anyStatus) assert.equal(res.status, 200, `${route}: ${text.slice(0, 4000)}`)
    return { status: res.status, value }
  }
  const prompt = { messages: [{ role: 'user', content: 'Compute 6 times 7. Reply only with the number.' }], reasoning_effort: 'none', max_tokens: 32, temperature: 0, seed: 42 }
  const count = async (route, body) => {
    const direct = (await send(false, route, body)).value
    const proxy = (await send(true, route, body)).value
    assert.ok(direct.input_tokens > 0); assert.equal(proxy.input_tokens, direct.input_tokens)
    return direct.input_tokens
  }
  assert.equal((await send(true, '/v1/models')).value.data[0].id, publicModel)
  assert.equal((await send(true, '/v1/chat/completions', prompt, { key: 'wrong', anyStatus: true })).status, 401)
  const input = await count('/v1/chat/completions/input_tokens', prompt)
  const direct = (await send(false, '/v1/chat/completions', prompt)).value
  const proxied = (await send(true, '/v1/chat/completions', prompt)).value
  assert.equal(proxied.model, publicModel)
  assert.equal(proxied.choices[0].message.content, direct.choices[0].message.content)
  assert.match(proxied.choices[0].message.content, /42/)
  assert.equal(proxied.usage.prompt_tokens, input)
  const stream = (await send(true, '/v1/chat/completions', { ...prompt, stream: true, stream_options: { include_usage: true } })).value
  assert.match(stream, /\[DONE\]/); assert.match(events(stream).map(e => e.choices?.[0]?.delta?.content || '').join(''), /42/); assert.match(stream, /completion_tokens/)
  assert.ok(!stream.includes(`"model":"${model}"`))
  record('gufo_chat_count_stream_auth', { id, input, usage: proxied.usage })

  const responses = { input: 'Compute 6 times 7. Reply only with the number.', reasoning: { effort: 'none' }, max_output_tokens: 32, temperature: 0 }
  const responseCount = await count('/v1/responses/input_tokens', responses)
  const response = (await send(true, '/v1/responses', responses)).value
  assert.equal(response.usage.input_tokens, responseCount)
  assert.match(JSON.stringify(response.output), /42/)
  const responseStream = (await send(true, '/v1/responses', { ...responses, stream: true })).value
  assert.match(responseStream, /response.completed/); assert.match(events(responseStream).filter(e => e.type === 'response.output_text.delta').map(e => e.delta).join(''), /42/)
  const anthropic = { messages: prompt.messages, max_tokens: 32, temperature: 0, thinking: { type: 'disabled' } }
  await count('/v1/messages/count_tokens', anthropic)
  const message = (await send(true, '/v1/messages', anthropic)).value
  assert.match(JSON.stringify(message.content), /42/)
  const messageStream = (await send(true, '/v1/messages', { ...anthropic, stream: true })).value
  assert.match(messageStream, /message_stop/)
  record('gufo_responses_and_anthropic', { id, responseCount, responseUsage: response.usage, anthropicUsage: message.usage })

  const tools = { ...prompt, max_tokens: 128, messages: [{ role: 'user', content: 'Call get_weather for Paris. Do not answer in prose.' }],
    tools: [{ type: 'function', function: { name: 'get_weather', description: 'Get weather for a city', parameters: { type: 'object', properties: { city: { type: 'string' } }, required: ['city'] } } }],
    tool_choice: { type: 'function', function: { name: 'get_weather' } } }
  await count('/v1/chat/completions/input_tokens', tools)
  for (const proxy of [false, true]) {
    const response = (await send(proxy, '/v1/chat/completions', tools)).value
    assert.equal(response.choices[0].message.tool_calls[0].function.name, 'get_weather')
    assert.match(JSON.parse(response.choices[0].message.tool_calls[0].function.arguments).city, /paris/i)
  }
  const toolStream = (await send(true, '/v1/chat/completions', { ...tools, stream: true })).value
  assert.match(toolStream, /tool_calls/); assert.match(toolStream, /get_weather/); assert.match(toolStream, /\[DONE\]/)
  const image = fs.readFileSync(path.resolve('src-tauri/icons/icon.png')).toString('base64')
  const vision = { ...prompt, max_tokens: 96, messages: [{ role: 'user', content: [
    { type: 'text', text: 'Describe the main colors and shapes in this image in one sentence.' },
    { type: 'image_url', image_url: { url: `data:image/png;base64,${image}` } },
  ] }] }
  const visionCount = await count('/v1/chat/completions/input_tokens', vision)
  for (const proxy of [false, true]) {
    const answer = (await send(proxy, '/v1/chat/completions', vision)).value
    assert.ok(answer.choices[0].message.content.trim().length > 5)
    assert.equal(answer.usage.prompt_tokens, visionCount)
  }
  record('gufo_tools_and_vision', { id, visionCount })
  const overflow = await send(true, '/v1/chat/completions', { ...prompt, max_tokens: context + 1 }, { anyStatus: true })
  assert.equal(overflow.status, 400); assert.match(JSON.stringify(overflow.value), /context/i)
  const exactOverflow = await send(true, '/v1/chat/completions', { ...prompt, max_tokens: context - 10 }, { anyStatus: true })
  assert.equal(exactOverflow.status, 400); assert.match(JSON.stringify(exactOverflow.value), /context/i)
  const invalid = { ...prompt, messages: [{ role: 'invalid-role', content: 'bad' }] }
  const directError = await send(false, '/v1/chat/completions', invalid, { anyStatus: true })
  const proxyError = await send(true, '/v1/chat/completions', invalid, { anyStatus: true })
  assert.equal(proxyError.status, directError.status); assert.ok(proxyError.status >= 400)

  const abort = new AbortController()
  const res = await fetch(`http://127.0.0.1:${routerPort}/v1/chat/completions`, { method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${key}` },
    body: JSON.stringify({ ...prompt, model: publicModel, messages: [{ role: 'user', content: 'Write a long story of at least 2000 words about a forest.' }], max_tokens: 2048, stream: true }), signal: abort.signal })
  assert.equal(res.status, 200)
  const reader = res.body.getReader(); await reader.read(); await sleep(1000); abort.abort(); await reader.cancel().catch(() => {})
  for (let i = 0; ; i++) {
    const slots = (await send(false, '/slots')).value
    if (slots.every(slot => !(slot.processing ?? slot.is_processing))) break
    assert.ok(i < 30, 'cancelled request still occupies Gufo'); await sleep(1000)
  }
  await send(true, '/v1/chat/completions', prompt)
  const metrics = await mcp.tool('instances.metrics', { id })
  assert.ok(metrics && metrics.gen_tokens > 0, JSON.stringify(metrics))
  assert.equal(metrics.decode_calls_total, null); assert.equal(metrics.busy_slots_per_decode, null)
  record('gufo_errors_cancellation_monitoring', { id, metrics })

  await require('./router-audit-acceptance.cjs').routerAuditAcceptance({ routerPort, port, model: publicModel,
    upstreamModel: model, key, authKey, gufo: true, root, id, record })

  const quotaKey = `quota-${id}`
  await client.mutate('routes.update', { revision: (await client.call('routes.get')).revision, patch: { api_keys: [
    { id: quotaKey, name: 'Gufo exact quota', key: quotaKey, enabled: true, daily_token_limit: input + 32, monthly_token_limit: input + 32, quota_default_output_tokens: 32 },
  ] } })
  await send(true, '/v1/chat/completions', prompt, { key: quotaKey })
  const denied = await send(true, '/v1/chat/completions', prompt, { key: quotaKey, anyStatus: true })
  assert.equal(denied.status, 429); assert.equal(denied.value.error.code, 'token_quota_exceeded')
  record('gufo_exact_hard_quota', { id, budgets: await client.call('usage.budgets') })
  await client.mutate('routes.stop')
}
module.exports = { gufoApiAcceptance }
