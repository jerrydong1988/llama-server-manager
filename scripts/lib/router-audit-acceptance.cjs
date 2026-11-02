// Real-server audit regressions. Invoked only inside the isolated model harness.
const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const { sleep } = require('./management-client.cjs')

async function routerAuditAcceptance({ client, routerPort, port, model, upstreamModel, key, authKey = '', gufo, root, id, record }) {
  const evidence = []
  async function request(route, body, { direct = false, method = body ? 'POST' : 'GET', origin } = {}) {
    const res = await fetch(`http://127.0.0.1:${direct ? port : routerPort}${route}`, {
      method, headers: { 'content-type': 'application/json', 'anthropic-version': '2023-06-01',
        ...((direct ? authKey : key) ? { authorization: `Bearer ${direct ? authKey : key}` } : {}), ...(origin ? { origin } : {}) },
      ...(body ? { body: JSON.stringify(body) } : {}), signal: AbortSignal.timeout(180000),
    })
    const text = await res.text()
    let value; try { value = JSON.parse(text) } catch { value = text }
    const result = { route, direct, status: res.status, headers: Object.fromEntries(res.headers), value }
    evidence.push(result)
    fs.writeFileSync(path.join(root, `${id}-audit.json`), JSON.stringify(evidence, null, 2))
    return result
  }
  const metrics = async () => {
    const text = (await request('/metrics')).value
    return Object.fromEntries(['requests_total', 'requests_rejected_total', 'in_flight_requests'].map(name =>
      [name, Number(text.match(new RegExp(`^lsm_router_${name} (\\d+)$`, 'm'))?.[1])]))
  }
  let baseline = await metrics()
  for (const [route, body, status, code] of [
    ['/v1/chat/completions', {}, 400, 'model_required'],
    ['/v1/chat/completions', { model: null }, 400, 'model_required'],
    ['/v1/chat/completions', { model: 42 }, 400, null],
    ['/v1/responses', { model: 'audit-unknown' }, 404, 'model_not_found'],
    ['/v1/messages', {}, 400, 'model_required'],
    ['/v1/embeddings', { model, input: 'hello' }, 400, 'model_endpoint_mismatch'],
  ]) {
    const response = await request(route, body)
    assert.equal(response.status, status, JSON.stringify(response))
    if (code) assert.equal(response.value.error.code, code)
    const after = await metrics()
    assert.equal(after.requests_rejected_total - baseline.requests_rejected_total, 1)
    baseline = after
  }
  for (const [route, status, code] of [['/audit-unknown-endpoint', 404, 'endpoint_not_found'], ['/v1/chat/completions', 405, 'method_not_allowed']]) {
    const response = await request(route)
    assert.equal(response.status, status); assert.equal(response.value.error.code, code)
  }
  const prompt = { model, messages: [{ role: 'user', content: 'Reply with exactly: OK' }], max_tokens: 32, temperature: 0, seed: 42, reasoning_effort: 'none' }
  const rejectedOrigin = await request('/v1/chat/completions', prompt, { origin: 'https://audit-denied.invalid' })
  assert.equal(rejectedOrigin.status, 403)
  const answer = await request('/v1/chat/completions', prompt)
  assert.equal(answer.status, 200)
  assert.equal(answer.headers['access-control-allow-origin'], undefined)
  assert.ok(answer.headers['x-lsm-request-id']); assert.ok(answer.headers['x-request-id'])
  await require('./tool-history-acceptance.cjs').toolHistoryAcceptance({ client, request, metrics, model, upstreamModel, root, id, record })
  if (gufo) {
    for (const enabled of [true, false]) {
      const body = { model, messages: prompt.messages, max_tokens: 128, temperature: 0,
        thinking: enabled ? { type: 'enabled', budget_tokens: 64 } : { type: 'disabled' } }
      const counted = await request('/v1/messages/count_tokens', body)
      const generated = await request('/v1/messages', body)
      assert.equal(counted.status, 200); assert.equal(generated.status, 200)
      const usage = generated.value.usage
      assert.equal(counted.value.input_tokens, usage.input_tokens + (usage.cache_read_input_tokens || 0) + (usage.cache_creation_input_tokens || 0))
    }
    for (const settings of [{ max_tokens: 16 }, { max_tokens: 128, stop: ['\n\n'] }, { max_tokens: 128, stop: ['\n\n'], reasoning_effort: 'none' }]) {
      const body = { model, messages: [{ role: 'user', content: 'List three fruits, one per line.' }], temperature: 0, seed: 42, ...settings }
      const direct = await request('/v1/chat/completions', { ...body, model: upstreamModel }, { direct: true })
      const proxy = await request('/v1/chat/completions', body)
      assert.equal(direct.status, 200); assert.equal(proxy.status, 200)
      assert.equal(proxy.value.choices[0].message.content, direct.value.choices[0].message.content)
      assert.equal(proxy.value.choices[0].finish_reason, direct.value.choices[0].finish_reason)
    }
  }
  const abort = new AbortController()
  let reader
  try {
    const stream = await fetch(`http://127.0.0.1:${routerPort}/v1/chat/completions`, {
      method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${key}` },
      body: JSON.stringify({ ...prompt, stream: true, max_tokens: 2048, messages: [{ role: 'user', content: 'Write a long story of at least 2000 words about a forest.' }] }), signal: abort.signal,
    })
    assert.equal(stream.status, 200)
    reader = stream.body.getReader(); await reader.read()
    baseline = await metrics()
    for (let i = 0; i < 3; i++) {
      const response = await request('/v1/chat/completions', prompt)
      assert.equal(response.status, 503); assert.equal(response.value.error.code, 'route_capacity')
      assert.equal(response.headers['retry-after'], '1')
    }
    const after = await metrics()
    assert.equal(after.requests_rejected_total - baseline.requests_rejected_total, 3)
  } finally { abort.abort(); await reader?.cancel().catch(() => {}) }
  for (let i = 0; ; i++) {
    if ((await metrics()).in_flight_requests === 0) break
    assert.ok(i < 50, 'cancel did not release router capacity'); await sleep(100)
  }
  // An engine may need a brief cancellation acknowledgement after the router releases.
  await sleep(300)
  assert.equal((await request('/v1/chat/completions', prompt)).status, 200)
  record('router_manual_audit_regressions', { id, gufo, passed: true, evidence: `${id}-audit.json` })
}
module.exports = { routerAuditAcceptance }
