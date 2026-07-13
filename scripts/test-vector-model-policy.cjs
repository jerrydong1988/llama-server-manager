const assert = require('node:assert/strict')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const entry = `
  import assert from 'node:assert/strict'
  import { defaultInstanceConfig } from './src/store/defaults'
  import {
    detectModelWorkload,
    normalizeInstanceConfig,
    VECTOR_ALLOWED_FIELDS,
    VECTOR_CLASSIFIED_FIELDS,
  } from './src/modelPolicy'

  const model = (overrides = {}) => ({
    id: 'model', name: 'model.gguf', path: 'C:/models/model.gguf', size: 1, file_type: 'gguf',
    ...overrides,
  })

  assert.equal(detectModelWorkload(model({ capabilities: { metadata_complete: true, is_embedding_model: true } })), 'embedding')
  assert.equal(detectModelWorkload(model({ capabilities: { metadata_complete: true, is_embedding_model: true, is_reranker_model: true } })), 'reranker')
  assert.equal(detectModelWorkload(model({ capabilities: { metadata_complete: true }, path: 'C:/models/nomic-embed-text.gguf' })), 'inference')
  assert.equal(detectModelWorkload(null, 'C:/models/bge-reranker-v2-m3.gguf'), 'reranker')
  assert.equal(detectModelWorkload(null, 'C:/models/nomic-embed-text-v1.5.gguf'), 'embedding')
  assert.equal(detectModelWorkload(model({ architecture: 'sentence-bert' })), 'embedding')
  assert.equal(detectModelWorkload(null, 'C:/models/Qwen3-8B-Instruct.gguf'), 'inference')

  const reranker = normalizeInstanceConfig(defaultInstanceConfig(), model({ capabilities: { metadata_complete: true, is_reranker_model: true } }))
  assert.equal(reranker.workload, 'reranker')
  assert.equal(reranker.config.embedding, true)
  assert.equal(reranker.config.reranking, true)
  assert.equal(reranker.config.pooling, 'rank')

  const pooled = normalizeInstanceConfig({ ...defaultInstanceConfig(), embedding: true, pooling: 'cls' }, null)
  assert.equal(pooled.config.pooling, 'cls')
  assert.equal(pooled.config.reranking, false)

  const polluted = {
    ...defaultInstanceConfig(),
    embedding: true,
    spec_type: 'draft-mtp',
    draft_model_path: 'C:/models/draft.gguf',
    cache_type_draft_k: 'q8_0',
    chat_template: 'chatml',
    temp: 1.5,
    mmproj_path: 'C:/models/mmproj.gguf',
    custom_args: ['--spec-type', 'draft-mtp'],
    batch_size: 2048,
    ubatch_size: 512,
  }
  const result = normalizeInstanceConfig(polluted, null)
  assert.equal(result.workload, 'embedding')
  assert.equal(result.vectorMode, true)
  assert.equal(result.config.spec_type, '')
  assert.equal(result.config.draft_model_path, '')
  assert.equal(result.config.cache_type_draft_k, '')
  assert.equal(result.config.chat_template, '')
  assert.equal(result.config.temp, defaultInstanceConfig().temp)
  assert.equal(result.config.mmproj_path, '')
  assert.deepEqual(result.config.custom_args, [])
  assert.equal(result.config.batch_size, 512)
  assert.deepEqual(new Set(Object.keys(defaultInstanceConfig())), VECTOR_CLASSIFIED_FIELDS)
  for (const key of VECTOR_ALLOWED_FIELDS) assert.ok(key in defaultInstanceConfig(), 'unknown allowed key: ' + key)
  assert.ok(result.changes.some(change => change.key === 'spec_type' && change.group === 'speculative'))
  assert.ok(result.changes.some(change => change.key === 'temp' && change.group === 'generation'))
  assert.ok(result.changes.some(change => change.key === 'custom_args' && change.group === 'custom'))
`

const bundled = esbuild.buildSync({
  bundle: true,
  format: 'cjs',
  platform: 'node',
  packages: 'external',
  write: false,
  stdin: {
    contents: entry,
    resolveDir: process.cwd(),
    sourcefile: 'vector-model-policy.test.ts',
    loader: 'ts',
  },
})

const testModule = new Module(path.join(process.cwd(), 'vector-model-policy.test.cjs'))
testModule.filename = path.join(process.cwd(), 'vector-model-policy.test.cjs')
testModule.paths = Module._nodeModulePaths(process.cwd())
testModule._compile(bundled.outputFiles[0].text, testModule.filename)

assert.ok(true)
console.log('vector model policy tests passed')
