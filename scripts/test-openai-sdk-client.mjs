import assert from 'node:assert/strict'
import OpenAI from 'openai'

const [baseURL, model = 'local-openai'] = process.argv.slice(2)
assert.ok(baseURL, 'usage: node test-openai-sdk-client.mjs <base-url> [model]')

const client = new OpenAI({
  apiKey: 'public-sdk-key',
  baseURL: `${baseURL.replace(/\/$/, '')}/v1`,
  maxRetries: 0,
  timeout: 10_000,
})

const models = await client.models.list()
assert.equal(models.data[0]?.id, model)
const retrieved = await client.models.retrieve(model)
assert.equal(retrieved.id, model)

const chat = await client.chat.completions.create({
  model,
  messages: [{ role: 'user', content: 'Hello' }],
  tools: [{
    type: 'function',
    function: {
      name: 'get_weather',
      description: 'Return a fixture',
      parameters: { type: 'object', properties: { city: { type: 'string' } } },
    },
  }],
})
assert.equal(chat.model, model)
assert.equal(chat.choices[0]?.message.tool_calls?.[0]?.function.name, 'get_weather')

const chatChunks = []
const chatStream = await client.chat.completions.create({
  model,
  messages: [{ role: 'user', content: 'Hello' }],
  stream: true,
})
for await (const chunk of chatStream) chatChunks.push(chunk)
assert.equal(chatChunks[0]?.model, model)

const response = await client.responses.create({
  model,
  input: [{ role: 'user', content: 'Hello' }],
})
assert.equal(response.model, model)
assert.equal(response.output_text, 'Hello from Responses')

const responseEvents = []
const responseStream = await client.responses.create({
  model,
  input: 'Hello',
  stream: true,
})
for await (const event of responseStream) responseEvents.push(event)
assert.equal(responseEvents[0]?.type, 'response.created')
assert.equal(responseEvents[0]?.response?.model, model)
assert.equal(responseEvents.at(-1)?.type, 'response.completed')

const inputTokens = await client.responses.inputTokens.count({
  model,
  input: 'Hello',
})
assert.equal(inputTokens.object, 'response.input_tokens')
assert.equal(inputTokens.input_tokens, 5)

process.stdout.write(JSON.stringify({
  model,
  chatChunks: chatChunks.length,
  responseEvents: responseEvents.length,
  inputTokens: inputTokens.input_tokens,
}))
