const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const { sleep } = require('./management-client.cjs')

async function toolHistoryAcceptance({ client, request, metrics, model, upstreamModel, root, id, record }) {
  const started = Date.now()
  const tool = (name, property) => ({ type: 'function', function: { name, description: `Get ${name}`,
    strict: true, parameters: { type: 'object', properties: { [property]: { type: 'string' } }, required: [property], additionalProperties: false } } })
  const tools = [tool('get_weather', 'city'), tool('get_time', 'timezone')]
  const call = (callId, name, args) => ({ id: callId, type: 'function', function: { name, arguments: JSON.stringify(args) } })
  const weather = call('history-weather', 'get_weather', { city: 'Paris' })
  const assistant = { role: 'assistant', content: null, tool_calls: [weather] }
  const result = { role: 'tool', tool_call_id: weather.id, content: '{"temperature_c":22,"conditions":"sunny"}' }
  const user = { role: 'user', content: 'What is the weather in Paris?' }
  const follow = { role: 'user', content: 'Summarize the tool results you received.' }
  const base = { model, tools, max_tokens: 128, reasoning_effort: 'none', temperature: 0, seed: 42 }
  const requestIds = []
  for (const [messages, stream] of [
    [[user, assistant], false],
    [[user, assistant, follow], true],
    [[user, assistant, { ...result, tool_call_id: 'wrong' }, follow], false],
    [[user, assistant, result, result], false],
    [[user, result], false],
    [[user, { ...assistant, tool_calls: [weather, weather] }], false],
    [[user, { ...assistant, tool_calls: [{ ...weather, id: '' }] }], false],
    [[user, assistant, { role: 'tool', content: 'missing id' }], false],
  ]) {
    const before = await metrics()
    const response = await request('/v1/chat/completions', { ...base, messages, stream })
    assert.equal(response.status, 400, JSON.stringify(response))
    assert.equal(response.value.error.code, 'invalid_tool_history')
    assert.equal(response.value.error.type, 'invalid_request_error')
    assert.match(response.value.error.param, /^messages\[\d+\]/)
    requestIds.push(response.headers['x-lsm-request-id'])
    const after = await metrics()
    assert.equal(after.requests_rejected_total - before.requests_rejected_total, 1)
  }
  // Counting keeps the engine's native contract, including a possible native
  // rejection of incomplete history. The gateway must not impose its new guard.
  const directCount = await request('/v1/chat/completions/input_tokens', { ...base, model: upstreamModel, messages: [user, assistant] }, { direct: true })
  const beforeCount = await metrics()
  const count = await request('/v1/chat/completions/input_tokens', { ...base, messages: [user, assistant] })
  assert.equal(count.status, directCount.status)
  assert.notEqual(count.value.error?.code, 'invalid_tool_history')
  if (count.status === 200) { assert.ok(count.value.input_tokens > 0); assert.equal(count.value.input_tokens, directCount.value.input_tokens) }
  assert.equal((await metrics()).requests_rejected_total, beforeCount.requests_rejected_total)

  // Exercise an actual generated call, not only manually constructed history.
  const first = await request('/v1/chat/completions', { ...base, messages: [user], tool_choice: { type: 'function', function: { name: 'get_weather' } } })
  assert.equal(first.status, 200)
  const generated = first.value.choices[0]
  assert.equal(generated.finish_reason, 'tool_calls')
  assert.equal(generated.message.tool_calls.length, 1)
  assert.equal(generated.message.tool_calls[0].function.name, 'get_weather')
  const generatedId = generated.message.tool_calls[0].id
  const final = await request('/v1/chat/completions', { ...base, tool_choice: 'none',
    messages: [user, generated.message, { ...result, tool_call_id: generatedId }, follow] })
  assert.equal(final.status, 200); assert.match(final.value.choices[0].message.content, /22/)

  const parallel = { ...assistant, tool_calls: [weather, call('history-time', 'get_time', { timezone: 'UTC' })] }
  const reversed = await request('/v1/chat/completions', { ...base, tool_choice: 'none', messages: [user, parallel,
    { role: 'tool', tool_call_id: 'history-time', content: '{"time":"15:22"}' }, result,
    { role: 'user', content: 'Report temperature_c and time exactly as written in the two tool results.' }] })
  assert.equal(reversed.status, 200)
  assert.match(reversed.value.choices[0].message.content, /22/); assert.match(reversed.value.choices[0].message.content, /15:22/)
  const failedTool = await request('/v1/chat/completions', { ...base, tool_choice: 'none', messages: [user, assistant,
    { ...result, content: '{"error":"CANCELLED"}' }, { role: 'user', content: 'What happened to the tool request?' }] })
  assert.equal(failedTool.status, 200); assert.match(failedTool.value.choices[0].message.content, /cancel/i)

  let ledger
  for (let attempt = 0; ; attempt++) {
    ledger = await client.call('usage.requests', { query: { from: started - 1000, to: Date.now() + 1000, failureCode: 'invalid_tool_history' } })
    if (requestIds.every(requestId => ledger.records.some(row => row.requestId === requestId))) break
    assert.ok(attempt < 30, 'tool-history rejections missing from usage ledger'); await sleep(100)
  }
  const rows = ledger.records.filter(row => requestIds.includes(row.requestId))
  assert.equal(rows.length, requestIds.length)
  for (const row of rows) {
    assert.equal(row.forwarded, false); assert.equal(row.outcome, 'rejected'); assert.equal(row.httpStatus, 400)
    assert.equal(row.failure.stage, 'validation'); assert.equal(row.failure.code, 'invalid_tool_history')
  }
  fs.writeFileSync(path.join(root, `${id}-tool-history-ledger.json`), JSON.stringify(rows, null, 2))
  record('tool_history_validation', { id, rejected: requestIds.length, generated_call_roundtrip: true,
    reversed_parallel_results: true, failed_tool_result: true, pending_count_status: count.status, pending_count: count.value.input_tokens, ledger_rows: rows.length })
}
module.exports = { toolHistoryAcceptance }
