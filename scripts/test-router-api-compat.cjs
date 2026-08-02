const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const read = (...parts) => fs.readFileSync(path.join(root, ...parts), 'utf8')
const proxy = read('src-tauri', 'src', 'commands', 'proxy.rs')
const protocol = read('src-tauri', 'src', 'commands', 'proxy_protocol.rs')
const runtime = read('src-tauri', 'src', 'commands', 'proxy_runtime.rs')
const models = read('src-tauri', 'src', 'models.rs')
const docs = read('docs', 'ROUTER_API_COMPATIBILITY.md')
const packageJson = JSON.parse(read('package.json'))

for (const route of [
  '/v1/models',
  '/v1/chat/completions',
  '/v1/chat/completions/input_tokens',
  '/v1/completions',
  '/v1/responses',
  '/v1/responses/input_tokens',
  '/v1/embeddings',
  '/v1/messages',
  '/v1/messages/count_tokens',
  '/ready',
  '/metrics',
  '/props',
  '/slots',
]) {
  assert.ok(proxy.includes(`"${route}"`), `missing production router route ${route}`)
}

for (const field of [
  'strict_model_routing',
  'connect_timeout_ms',
  'streaming_idle_timeout_ms',
  'health_check_interval_ms',
  'unhealthy_threshold',
  'recovery_cooldown_ms',
  'max_concurrent_requests',
  'queue_timeout_ms',
  'requests_per_minute',
  'cors_allowed_origins',
  'api_keys',
]) {
  assert.match(models, new RegExp(`pub ${field}:`), `missing router configuration field ${field}`)
}

for (const strategy of ['priorityFailover', 'roundRobin', 'leastBusy', 'weighted']) {
  assert.ok(proxy.includes(`"${strategy}"`) || runtime.includes(`"${strategy}"`), `missing scheduler ${strategy}`)
  assert.ok(docs.includes(strategy), `router documentation omits scheduler ${strategy}`)
}

assert.match(proxy, /PROXY_API_KEY_HASH_PREFIX/)
assert.match(proxy, /cors_allowed_origins/)
assert.match(proxy, /HashSet::from\(\["inference", "discovery"\]\)/)
assert.match(proxy, /for key in \["id", "n_ctx", "speculative", "is_processing", "n_past"\]/)
assert.doesNotMatch(proxy.match(/fn sanitized_slots[\s\S]*?\n}/)?.[0] ?? '', /prompt|tokens/)
assert.match(protocol, /"param": Value::Null/)
assert.match(protocol, /"code": Value::Null/)
assert.match(protocol, /"context_length_exceeded"/)
assert.match(protocol, /x-llama-server-manager-context-window/)
assert.match(protocol, /response.*get_mut/s)
for (const field of ['"context_length": context_window', '"context_window": context_window', '"max_model_len": context_window']) {
  assert.ok(proxy.includes(field), `model discovery omits ${field}`)
}
for (const counter of ['/v1/chat/completions/input_tokens', '/v1/responses/input_tokens', '/v1/messages/count_tokens', '/tokenize']) {
  assert.ok(proxy.includes(`"${counter}"`), `context preflight omits ${counter}`)
}
assert.match(proxy, /safe_context_window/)
assert.match(proxy, /context_limit_violation/)
assert.match(runtime, /lsm_router_requests_total/)
assert.match(runtime, /circuit_open_until_ms/)
assert.match(docs, /`GET \/slots\?model=/)
assert.match(docs, /131072/)
assert.match(docs, /max_model_len/)
assert.match(docs, /context_length_exceeded/)
assert.match(docs, /x-request-id/)
assert.match(docs, /request-id/)
assert.equal(packageJson.devDependencies.openai, '7.3.0')
assert.equal(packageJson.devDependencies['@anthropic-ai/sdk'], '0.115.0')

console.log('Production OpenAI / Anthropic router compatibility checks passed')
