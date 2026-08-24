const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const read = (...parts) => fs.readFileSync(path.join(root, ...parts), 'utf8')
const baseline = JSON.parse(read('scripts', 'llama-parameter-baseline.json'))
const proxy = read('src-tauri', 'src', 'commands', 'proxy.rs')
const protocol = read('src-tauri', 'src', 'commands', 'proxy_protocol.rs')
const telemetry = read('src-tauri', 'src', 'commands', 'telemetry.rs')
const docs = read('docs', 'ANTHROPIC_API_COMPATIBILITY.md')
const packageJson = JSON.parse(read('package.json'))

const release = String(baseline.upstreamRelease)
const legacyBuild = release.match(/^b(\d+)$/)
const semanticRelease = release.match(/^v(\d+)\.(\d+)\.(\d+)$/)
const supportsAnthropicProtocol = legacyBuild
  ? Number(legacyBuild[1]) >= 10199
  : semanticRelease
    ? Number(semanticRelease[1]) > 0 || Number(semanticRelease[2]) >= 2
    : false
assert.ok(
  supportsAnthropicProtocol,
  'Anthropic passthrough requires llama.cpp b10199+ or v0.2.0+',
)
for (const route of ['/v1/messages', '/v1/messages/count_tokens', '/v1/models/:model_id']) {
  assert.match(proxy, new RegExp(route.replace(/[/:]/g, '\\$&')), `missing proxy route ${route}`)
}
assert.match(proxy, /MAX_ANTHROPIC_REQUEST_BODY_BYTES:\s*usize\s*=\s*32\s*\*\s*1024\s*\*\s*1024/)
assert.match(protocol, /"authentication_error"/)
assert.match(protocol, /"invalid_request_error"/)
assert.match(protocol, /object\.get_mut\("message"\)/, 'Anthropic message_start model privacy rewrite is required')
assert.match(telemetry, /api_format TEXT/)
assert.match(telemetry, /record\.api_format/)
assert.match(docs, /ANTHROPIC_BASE_URL/)
assert.match(docs, /ANTHROPIC_DEFAULT_HAIKU_MODEL/)
assert.match(docs, /ANTHROPIC_DEFAULT_SONNET_MODEL/)
assert.match(docs, /ANTHROPIC_DEFAULT_OPUS_MODEL/)
assert.match(docs, /CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY/)
assert.equal(packageJson.devDependencies['@anthropic-ai/sdk'], '0.115.0')

console.log(`Anthropic API compatibility checks passed against llama.cpp ${baseline.upstreamRelease}`)
