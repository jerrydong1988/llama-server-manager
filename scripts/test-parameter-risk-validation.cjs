const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const root = path.resolve(__dirname, '..')
const read = relative => fs.readFileSync(path.join(root, relative), 'utf8')

const entry = `
  import assert from 'node:assert/strict'
  import { defaultInstanceConfig } from './src/store/defaults'
  import { getActiveParams } from './src/components/ConfigPage/activeParams'
  import { PARAMETER_CATALOG, parameterDependencyActive } from './src/parameterCatalog'
  import { KNOWN_FLAGS, tokenizeCustomArgs, validateConfig } from './src/validators'

  const config = (overrides = {}) => ({
    ...defaultInstanceConfig(),
    model_path: 'C:/models/model.gguf',
    ...overrides,
  })
  const model = (overrides = {}) => ({
    id: 'model', name: 'model.gguf', path: 'C:/models/model.gguf', size: 1,
    file_type: 'gguf', capabilities: { metadata_complete: true }, ...overrides,
  })
  const warnings = (overrides = {}, selectedModel = null, engine = null, projector = null) =>
    validateConfig(config(overrides), selectedModel, engine, projector)
  const keys = (...args) => warnings(...args).map(warning => warning.key)
  const has = (key, ...args) => keys(...args).includes(key)

  assert.deepEqual(warnings(), [], 'default managed inference config must not report risks')

  // Removed false positives must stay removed.
  const rocmBackend = warnings({ backend_sampling: true }, null, { backend: 'ROCm' })
  assert.deepEqual(rocmBackend.map(warning => warning.key), ['warnBackendSamplingExperimental'])
  assert.equal(rocmBackend[0].severity, 'low')
  assert.equal(has('warnA6', { ctx_size_auto: true, rope_scaling: 'yarn', yarn_ext_factor: 1 }), false)
  assert.equal(has('warnA10', { flash_attn: 'on' }, null, { backend: 'CPU' }), false)
  assert.equal(has('warnA13', { api_key: 'one', api_key_file: 'keys.txt' }), false)
  assert.equal(has('warnB8', { cache_ram: 0, cache_idle_slots: false, ctx_checkpoints: 8 }), false)
  assert.equal(has('warnB2', { cache_prompt: false, cache_ram: 1024 }), false)
  assert.equal(has('warnB3', { slots_enabled: false, slot_prompt_similarity: 0.75 }), false)
  assert.equal(has('warnB5', { samplers: 'temperature;top_k', top_k: 20 }), false)

  // Rewritten rules use effective runtime semantics and calibrated severities.
  assert.equal(has('warnA1', { reasoning: 'off', reasoning_format: 'deepseek' }), false)
  assert.equal(has('warnA1', { reasoning: 'off', reasoning_budget: '1024' }), true)
  assert.equal(warnings({ reasoning: 'off', reasoning_budget: '1024' }).find(w => w.key === 'warnA1').severity, 'low')
  assert.equal(has('warnA5', { spec_type: '', draft_tokens: 8 }), true)
  assert.equal(has('warnA5', { spec_type: '', draft_model_path: 'draft.gguf' }), true)
  assert.equal(has('warnA5', { spec_type: '', spec_draft_backend_sampling: false }), true)
  assert.equal(has('warnA5', { spec_type: '', cache_type_draft_k: 'q8_0' }), true)
  assert.equal(has('warnA7', { ctx_size: 262144, parallel: 2 }, model({ context_length: 131072 })), false)
  assert.equal(has('warnA7', { ctx_size: 262144, parallel: 1 }, model({ context_length: 131072 })), true)
  assert.equal(has('warnA8', { image_min_tokens: 128, mmproj_path: 'mmproj.gguf', mmproj_mode: 'off' }), true)
  assert.equal(has('warnA8', { image_min_tokens: 128, mmproj_path: 'mmproj.gguf' }), false)
  assert.equal(has('warnA9', { swa_full: true }, model({ capabilities: { metadata_complete: true } })), false)
  assert.equal(has('warnA9', { swa_full: true }, model({ capabilities: { metadata_complete: true, has_swa: false } })), true)
  assert.equal(has('warnB2', { cache_prompt: false, cache_reuse: 64 }), true)
  assert.equal(has('warnB3', { slots_enabled: false, slot_save_path: 'C:/slots' }), true)
  assert.equal(has('warnB4', { mirostat: 2, temp: 0.6 }), false, 'temperature remains active with Mirostat')
  assert.equal(has('warnB4', { mirostat: 2, top_k: 20 }), true)
  assert.equal(has('warnB5', { samplers: 'temperature', top_k: 20 }), true)
  assert.equal(has('warnB5', { sampler_seq: 't', top_p: 0.8 }), true)
  assert.equal(has('warnB9', { spec_type: 'ngram-simple', lookup_cache_static: 'cache.bin' }), true)
  assert.equal(has('warnB9', { spec_type: 'ngram-cache', lookup_cache_static: 'cache.bin' }), false)
  assert.equal(has('warnB9', { spec_type: '', lookup_cache_static: 'cache.bin' }), false)

  // External draft sources supplied through custom args satisfy draft modes.
  assert.equal(has('warnA3', { spec_type: 'draft-simple' }), true)
  assert.equal(has('warnA3', { spec_type: 'draft-simple', custom_args: ['--spec-draft-hf owner/model'] }), false)

  // Newly covered upstream incompatibilities.
  assert.equal(has('warnBackendSamplingGrammar', { backend_sampling: true, json_schema: '{}' }), true)
  assert.equal(has('warnBackendSamplingReasoning', { backend_sampling: true, reasoning_budget: '1024' }), true)
  assert.equal(has(
    'warnBackendSamplingSpeculative',
    { backend_sampling: true, spec_type: 'draft-mtp' },
    model({ capabilities: { metadata_complete: true, has_builtin_mtp: true } }),
  ), true)
  assert.equal(has('warnCacheIdleWithoutRam', { cache_idle_slots: true, cache_ram: 0 }), true)
  assert.equal(has('warnMultimodalContextShift', { mmproj_path: 'mmproj.gguf', context_shift: true }), true)
  assert.equal(has('warnMultimodalCacheReuse', { mmproj_path: 'mmproj.gguf', cache_reuse: 64 }), true)
  assert.equal(has('warnToolsRequireJinja', { tools: 'all', jinja: false }), true)
  assert.equal(has('warnToolsRequireJinja', { agent: true, jinja: false }), true)
  assert.equal(has('warnToolsRequireJinja', { mcp_servers_config: 'mcp.json', jinja: false }), true)
  assert.equal(has('warnMcpServersJsonInvalid', { mcp_servers_json: '{bad json' }), true)
  assert.equal(has('warnMcpServersJsonInvalid', { mcp_servers_json: '{"mcpServers":{}}' }), false)
  assert.equal(has('warnToolsRuntimeInvalid', { tools_runtime: 'process:host' }), true)
  assert.equal(has('warnToolsRuntimeInvalid', { tools_runtime: 'docker:ubuntu:24.04' }), false)
  assert.equal(has('warnPrivilegedToolsExposure', { tools: 'all', host: '0.0.0.0' }), true)
  assert.equal(has('warnPrivilegedToolsExposure', { agent: true, cors_origins: '*' }), true)
  assert.equal(has('warnPrivilegedToolsExposure', { mcp_servers_config: 'mcp.json' }), false)
  assert.equal(has('warnSamplerDefinitionsConflict', { samplers: 'top_k', sampler_seq: 'k' }), true)

  // Retained precedence, termination, TLS, workload and projector checks.
  assert.equal(has('warnA11', { grammar: 'root ::= "x"', grammar_file: 'grammar.gbnf' }), true)
  assert.equal(has('warnA12', { chat_template: 'chatml', chat_template_file: 'chat.jinja' }), true)
  assert.equal(has('warnA14', { ssl_key_file: 'server.key' }), true)
  assert.equal(has('warnB6', { pooling: 'mean', embedding: false }), true)
  assert.equal(has('warnB7', { gpu_layers_auto: false, gpu_layers: 0, device: 'ROCm0' }), true)
  assert.equal(has('warnB11', { json_schema: '{}', json_schema_file: 'schema.json' }), true)
  assert.equal(has('warnC3', { n_predict: -1, ignore_eos: true }), true)
  assert.equal(has(
    'warnC4',
    { spec_type: 'draft-mtp', draft_model_path: 'draft.gguf' },
    model({ capabilities: { metadata_complete: true, has_builtin_mtp: true } }),
  ), true)
  assert.equal(has(
    'warnC4',
    { spec_type: 'future-draft-mtp-mode', draft_model_path: 'draft.gguf' },
    model({ capabilities: { metadata_complete: true, has_builtin_mtp: true } }),
  ), false)
  assert.equal(has('warnC5', {}, model({ file_type: 'mmproj', capabilities: { metadata_complete: true, is_mmproj: true } })), true)

  // Custom-argument detection covers packed rows, aliases, and --flag=value without
  // treating a quoted flag-looking value as another option.
  assert.equal(has('warnD1', { custom_args: ['--temp=0.5'] }), true)
  assert.equal(has('warnD1', { custom_args: ['--temperature 0.5'] }), true)
  assert.equal(has('warnD1', { custom_args: ['--top-p 0.9 --seed 7'] }), true)
  assert.equal(has('warnD1', { custom_args: ['--prompt "--temp"'] }), false)
  assert.equal(has('warnD1', { custom_args: ['"--temp" 0.5'] }), true)
  assert.deepEqual(
    tokenizeCustomArgs(['--model "C:\\\\Models\\\\chat.gguf" --temp=0.5']),
    ['--model', 'C:\\\\Models\\\\chat.gguf', '--temp=0.5'],
  )

  // UI dependency and emitted-parameter state share the same semantics.
  assert.equal(parameterDependencyActive('image_min_tokens', config(), false), false)
  assert.equal(parameterDependencyActive('image_min_tokens', config({ mmproj_path: 'mmproj.gguf' }), false), true)
  assert.equal(parameterDependencyActive('image_min_tokens', config({ mmproj_path: 'mmproj.gguf', mmproj_mode: 'off' }), false), false)
  assert.equal(parameterDependencyActive('backend_sampling', config({ backend_sampling: true, grammar: 'root ::= "x"' }), false), false)
  assert.equal(parameterDependencyActive('backend_sampling', config({ backend_sampling: true }), false), true)
  assert.equal(parameterDependencyActive('cache_idle_slots', config({ cache_ram: 0 }), false), false)
  assert.equal(parameterDependencyActive('lookup_cache_static', config({ spec_type: 'ngram-simple' }), false), false)
  assert.equal(parameterDependencyActive('lookup_cache_static', config({ spec_type: 'ngram-cache' }), false), true)
  assert.equal(parameterDependencyActive('tools', config({ tools: 'all', jinja: false }), false), false)
  assert.equal(parameterDependencyActive('tools_runtime', config({ tools: 'all', tools_runtime: 'docker:ubuntu', jinja: true }), false), true)
  assert.equal(parameterDependencyActive('tools_runtime', config({ tools_runtime: 'docker:ubuntu', jinja: true }), false), false)
  assert.equal(parameterDependencyActive('mcp_servers_config', config({ mcp_servers_config: 'mcp.json', jinja: true }), false), true)
  assert.equal(parameterDependencyActive('mcp_servers_json', config({ mcp_servers_json: '{}', jinja: false }), false), false)

  const projectorOff = config({
    mmproj_path: 'mmproj.gguf', mmproj_mode: 'off', no_mmproj_offload: true,
    explicit_overrides: ['mmproj_path', 'mmproj_mode', 'no_mmproj_offload'],
  })
  assert.equal(getActiveParams(projectorOff, false).has('mmproj_path'), false)
  assert.equal(getActiveParams(projectorOff, false).has('no_mmproj_offload'), false)
  const projectorOn = config({
    mmproj_path: 'mmproj.gguf', mmproj_mode: '', no_mmproj_offload: true,
    explicit_overrides: ['mmproj_path', 'no_mmproj_offload'],
  })
  assert.equal(getActiveParams(projectorOn, false).has('mmproj_path'), true)
  assert.equal(getActiveParams(projectorOn, false).has('no_mmproj_offload'), true)
  const inheritedProjector = config({
    mmproj_path: 'mmproj.gguf', no_mmproj_offload: true,
    explicit_overrides: ['no_mmproj_offload'],
  })
  assert.equal(getActiveParams(inheritedProjector, false).has('no_mmproj_offload'), false)

  for (const definition of Object.values(PARAMETER_CATALOG)) {
    for (const flag of definition?.flags ?? []) {
      assert.equal(KNOWN_FLAGS.has(flag), true, 'catalogued flag missing from duplicate detection: ' + flag)
    }
  }
`

const bundled = esbuild.buildSync({
  bundle: true,
  format: 'cjs',
  platform: 'node',
  packages: 'external',
  sourcemap: 'inline',
  write: false,
  stdin: {
    contents: entry,
    resolveDir: root,
    sourcefile: 'parameter-risk-validation.test.ts',
  },
})
const testModule = new Module(path.join(root, 'parameter-risk-validation.test.cjs'), module)
testModule.filename = path.join(root, 'parameter-risk-validation.test.cjs')
testModule.paths = module.paths
testModule._compile(bundled.outputFiles[0].text, testModule.filename)

const validators = read('src/validators.ts')
const zh = read('src/i18n/zh-CN.ts')
const en = read('src/i18n/en-US.ts')
assert.doesNotMatch(en, /Only one of key\/key_file takes effect/, 'API key tooltip still describes additive sources as mutually exclusive')
const usedKeys = new Set([...validators.matchAll(/key:\s*'(?<key>warn[^']+)'/g)].map(match => match.groups.key))
for (const key of usedKeys) {
  assert.match(zh, new RegExp(`\\b${key}:`), `Chinese warning translation is missing: ${key}`)
  assert.match(en, new RegExp(`\\b${key}:`), `English warning translation is missing: ${key}`)
}
for (const staleKey of ['warnA2', 'warnA4', 'warnA6', 'warnA10', 'warnA13', 'warnB1', 'warnB8', 'warnB10']) {
  assert.doesNotMatch(validators, new RegExp(`key:\\s*'${staleKey}'`), `stale validator rule remains: ${staleKey}`)
  assert.doesNotMatch(zh, new RegExp(`\\b${staleKey}:`), `stale Chinese warning copy remains: ${staleKey}`)
  assert.doesNotMatch(en, new RegExp(`\\b${staleKey}:`), `stale English warning copy remains: ${staleKey}`)
}

console.log('Parameter-risk validation semantics and regression checks passed.')
