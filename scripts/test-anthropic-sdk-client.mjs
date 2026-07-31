import assert from 'node:assert/strict'
import Anthropic from '@anthropic-ai/sdk'

const [baseURL, model = 'local-claude'] = process.argv.slice(2)
assert.ok(baseURL, 'usage: node test-anthropic-sdk-client.mjs <base-url> [model]')

const client = new Anthropic({
  apiKey: 'public-sdk-key',
  baseURL,
  maxRetries: 0,
  timeout: 10_000,
  defaultHeaders: { 'anthropic-beta': 'prompt-caching-2024-07-31' },
})

const models = await client.models.list()
assert.equal(models.data[0]?.id, model)
assert.equal(models.data[0]?.display_name, model)

const retrieved = await client.models.retrieve(model)
assert.equal(retrieved.id, model)

const image = {
  type: 'image',
  source: {
    type: 'base64',
    media_type: 'image/png',
    data: 'iVBORw0KGgo=',
  },
}
const tool = {
  name: 'get_weather',
  description: 'Return a local weather fixture',
  input_schema: {
    type: 'object',
    properties: { city: { type: 'string' } },
    required: ['city'],
  },
}
const request = {
  model,
  max_tokens: 2048,
  system: [{ type: 'text', text: 'Use tools when needed.', cache_control: { type: 'ephemeral' } }],
  messages: [{
    role: 'user',
    content: [image, { type: 'text', text: 'Weather in Shanghai?' }],
  }],
  tools: [tool],
  tool_choice: { type: 'auto' },
  thinking: { type: 'enabled', budget_tokens: 1024 },
  metadata: { user_id: 'claude-code-shaped-smoke' },
}

const message = await client.messages.create(request)
assert.equal(message.model, model)
assert.equal(message.type, 'message')
assert.equal(message.content[0]?.type, 'tool_use')
assert.equal(message.usage.input_tokens, 23)
assert.equal(message.usage.output_tokens, 7)

const toolResultMessage = await client.messages.create({
  ...request,
  messages: [
    request.messages[0],
    { role: 'assistant', content: [{ type: 'tool_use', id: 'toolu_local', name: 'get_weather', input: { city: 'Shanghai' } }] },
    { role: 'user', content: [{ type: 'tool_result', tool_use_id: 'toolu_local', content: 'Sunny' }] },
  ],
})
assert.equal(toolResultMessage.model, model)

const tokenCount = await client.messages.countTokens(request)
assert.equal(tokenCount.input_tokens, 23)

const events = []
const stream = await client.messages.create({ ...request, stream: true })
for await (const event of stream) events.push(event)
assert.deepEqual(
  events.map(event => event.type),
  ['message_start', 'content_block_start', 'content_block_delta', 'content_block_stop', 'message_delta', 'message_stop'],
)
assert.equal(events[0]?.message?.model, model)
assert.equal(events[2]?.delta?.type, 'input_json_delta')

process.stdout.write(JSON.stringify({
  model: message.model,
  inputTokens: tokenCount.input_tokens,
  streamEvents: events.length,
}))
