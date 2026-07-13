const assert = require('node:assert/strict')
const fs = require('node:fs')
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
  import {
    applyModelInventory,
    beginModelInventoryRequest,
    isCurrentModelInventoryRequest,
    normalizeModelPath,
    normalizeStoredConfig,
    reconcileInstancesWithModels,
  } from './src/store/bootstrap'

  const model = (overrides = {}) => ({
    id: 'model', name: 'model.gguf', path: 'C:/models/model.gguf', size: 1, file_type: 'gguf',
    ...overrides,
  })

  assert.equal(detectModelWorkload(model({ capabilities: { metadata_complete: true, is_embedding_model: true } })), 'embedding')
  assert.equal(detectModelWorkload(model({ capabilities: { metadata_complete: true, is_embedding_model: true, is_reranker_model: true } })), 'reranker')
  assert.equal(detectModelWorkload(model({ capabilities: { metadata_complete: true }, name: 'nomic-embed-text.gguf', path: 'C:/models/Qwen3-Instruct.gguf' })), 'embedding')
  assert.equal(detectModelWorkload(model({ capabilities: { metadata_complete: true }, architecture: 'sentence-bert' })), 'embedding')
  assert.equal(detectModelWorkload(null, 'C:\\\\models\\\\embedding\\\\Qwen3-Instruct.gguf'), 'inference')
  assert.equal(detectModelWorkload(
    model({ capabilities: { metadata_complete: true, is_embedding_model: false, is_reranker_model: false }, name: 'nomic-embed-text.gguf' }),
    '',
    { embedding: true, reranking: false },
  ), 'inference')
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

  const embedding = normalizeInstanceConfig(
    { ...defaultInstanceConfig(), embedding: true, reranking: true, pooling: 'rank' },
    model({ capabilities: { metadata_complete: true, is_embedding_model: true, is_reranker_model: false } }),
  )
  assert.equal(embedding.workload, 'embedding')
  assert.equal(embedding.config.reranking, false)
  assert.equal(embedding.config.pooling, '')

  const switchedToInference = normalizeInstanceConfig(
    { ...defaultInstanceConfig(), embedding: true, reranking: true, pooling: 'rank' },
    model({ capabilities: { metadata_complete: true, is_embedding_model: false, is_reranker_model: false } }),
  )
  assert.equal(switchedToInference.workload, 'inference')
  assert.equal(switchedToInference.vectorMode, false)
  assert.equal(switchedToInference.config.embedding, false)
  assert.equal(switchedToInference.config.reranking, false)
  assert.equal(switchedToInference.config.pooling, '')

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

  const created = normalizeInstanceConfig(polluted, null, { context: 'create' })
  assert.equal(created.config.spec_type, '')
  assert.deepEqual(created.changes, [])

  assert.equal(normalizeModelPath('C:\\\\Models\\\\Qwen3-Instruct.gguf'), 'c:/models/qwen3-instruct.gguf')
  assert.equal(normalizeModelPath('/Models/Qwen3-Instruct.gguf'), '/Models/Qwen3-Instruct.gguf')
  assert.equal(normalizeModelPath('\\\\\\\\Server\\\\Share\\\\Model.GGUF'), '//server/share/model.gguf')
  assert.equal(normalizeModelPath('\\\\\\\\?\\\\C:\\\\Models\\\\Model.GGUF'), 'c:/models/model.gguf')
  assert.equal(normalizeModelPath('\\\\\\\\?\\\\UNC\\\\Server\\\\Share\\\\Model.GGUF'), '//server/share/model.gguf')
  assert.equal(normalizeModelPath('//Models/Model.GGUF'), '//Models/Model.GGUF')

  const staleInventoryRequest = beginModelInventoryRequest()
  const currentInventoryRequest = beginModelInventoryRequest()
  assert.equal(isCurrentModelInventoryRequest(staleInventoryRequest), false)
  assert.equal(isCurrentModelInventoryRequest(currentInventoryRequest), true)
  const inventoryState = { instances: [], models: [] }
  const staleApplied = applyModelInventory(
    [model({ capabilities: { metadata_complete: true, is_embedding_model: true } })],
    () => inventoryState as any,
    (partial) => Object.assign(inventoryState, partial),
    {},
    staleInventoryRequest,
  )
  assert.equal(staleApplied, false)
  assert.deepEqual(inventoryState.models, [])

  const indexedVectorModel = model({
    name: 'Qwen3-Instruct.gguf',
    path: 'C:\\\\Models\\\\Qwen3-Instruct.gguf',
    capabilities: { metadata_complete: true, is_embedding_model: true, is_reranker_model: false },
  })
  const storedVectorConfig = {
    ...defaultInstanceConfig(),
    id: 'vector',
    name: 'Vector instance',
    model_path: 'c:/models/Qwen3-Instruct.gguf',
    embedding: false,
    temp: 1.5,
    custom_args: ['--temp', '1.5'],
  }
  const normalizedStored = normalizeStoredConfig(storedVectorConfig, [indexedVectorModel])
  assert.equal(normalizedStored.workload, 'embedding')
  assert.equal(normalizedStored.config.embedding, true)
  assert.equal(normalizedStored.config.temp, defaultInstanceConfig().temp)
  assert.deepEqual(normalizedStored.config.custom_args, [])

  const missingInferenceConfig = {
    ...defaultInstanceConfig(),
    id: 'inference',
    name: 'Inference instance',
    model_path: 'C:\\\\missing\\\\Qwen3-8B-Instruct.gguf',
    temp: 1.25,
  }
  const missingInference = normalizeStoredConfig(missingInferenceConfig, [])
  assert.equal(missingInference.workload, 'inference')
  assert.equal(missingInference.changes.length, 0)
  assert.equal(missingInference.config.temp, 1.25)

  const vectorInstance = {
    id: 'vector', name: 'Vector instance', status: 'running', model: 'Qwen3-Instruct.gguf',
    port: 18080, healthCheck: 'ok', startTime: 123456, config: storedVectorConfig,
  }
  const inferenceInstance = {
    id: 'inference', name: 'Inference instance', status: 'stopped', model: 'Qwen3-8B-Instruct.gguf',
    port: 18081, healthCheck: 'pending', config: missingInferenceConfig,
  }
  const originalInstances = [vectorInstance, inferenceInstance]
  const reconciled = reconcileInstancesWithModels(originalInstances, [indexedVectorModel])
  assert.equal(reconciled.changed, true)
  assert.notStrictEqual(reconciled.instances, originalInstances)
  assert.notStrictEqual(reconciled.instances[0], vectorInstance)
  assert.strictEqual(reconciled.instances[1], inferenceInstance)
  assert.equal(reconciled.instances[0].status, 'running')
  assert.equal(reconciled.instances[0].healthCheck, 'ok')
  assert.equal(reconciled.instances[0].startTime, 123456)

  const unchanged = reconcileInstancesWithModels([inferenceInstance], [])
  assert.equal(unchanged.changed, false)
  assert.strictEqual(unchanged.instances[0], inferenceInstance)
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

function readSource(...segments) {
  return fs.readFileSync(path.join(__dirname, '..', ...segments), 'utf8')
}

function section(source, start, end) {
  const startIndex = source.indexOf(start)
  const endIndex = source.indexOf(end, startIndex + start.length)
  assert.ok(startIndex >= 0 && endIndex > startIndex, `missing source section: ${start}`)
  return source.slice(startIndex, endIndex)
}

const instanceSliceSource = readSource('src', 'store', 'instanceSlice.ts')
const generateCommandSource = section(instanceSliceSource, 'generateCommand:', 'startInstance:')
const startInstanceSource = section(instanceSliceSource, 'startInstance:', 'stopInstance:')
const saveConfigSource = section(instanceSliceSource, 'saveConfig:', 'loadConfig:')

assert.match(generateCommandSource, /normalizeStoredConfig\(/, 'command preview must normalize its config')
assert.match(generateCommandSource, /config:\s*normalized\.config/, 'command preview must invoke with normalized config')
assert.match(startInstanceSource, /normalizeStoredConfig\(/, 'start must normalize the stored config')
assert.match(startInstanceSource, /config:\s*normalized\.config/, 'start must invoke with normalized config')
assert.match(startInstanceSource, /set\(/, 'start cleanup must update Zustand state')
assert.match(startInstanceSource, /await get\(\)\.saveConfig\(\)/, 'start cleanup must be persisted before launch')
assert.match(startInstanceSource, /await configSaveQueue/, 'start must await an in-flight inventory migration save')
assert.match(saveConfigSource, /reconcileInstancesWithModels\(/, 'save must normalize every instance')

const bootstrapSource = readSource('src', 'store', 'bootstrap.ts')
const cachedScanSource = section(bootstrapSource, 'const cachedScanRequest', 'const injected')
const initialScanSource = section(bootstrapSource, 'const modelScanRequest', "invoke<EngineInfo[]>('scan_engines'")
assert.match(cachedScanSource, /applyModelInventory\(/, 'cached model inventory must reconcile instances')
assert.match(cachedScanSource, /beginModelInventoryRequest\(/, 'cached inventory must claim a request generation')
assert.match(initialScanSource, /applyModelInventory\(/, 'initial async model scan must reconcile instances')
assert.match(initialScanSource, /beginModelInventoryRequest\(/, 'initial scan must supersede older inventory requests')

const coreSliceSource = readSource('src', 'store', 'coreSlice.ts')
const setModelsSource = section(coreSliceSource, 'setModels:', 'setEngines:')
const loadInitialDataSource = section(coreSliceSource, 'loadInitialData:', 'scanModels:')
const scanModelsSource = section(coreSliceSource, 'scanModels:', 'deleteModelFile:')
assert.match(setModelsSource, /isLoading:\s*false/, 'synchronous inventory replacement must end superseded loading')
assert.match(loadInitialDataSource, /applyModelInventory\(/, 'get_models results must reconcile instances')
assert.match(loadInitialDataSource, /beginModelInventoryRequest\(/, 'get_models must claim a request generation')
assert.match(scanModelsSource, /applyModelInventory\(/, 'manual model scans must reconcile instances')
assert.match(scanModelsSource, /beginModelInventoryRequest\(/, 'manual scans must supersede older inventory requests')

const processConfigSource = section(bootstrapSource, 'async function processConfig', "invoke<DownloadManagerSnapshot>('get_download_manager_snapshot')")
assert.match(processConfigSource, /models:\s*\[\]/, 'hydration must not expose stale cached capabilities before fresh scan')

console.log('vector model policy tests passed')
