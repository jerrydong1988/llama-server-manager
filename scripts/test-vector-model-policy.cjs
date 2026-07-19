const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const entry = `
  import assert from 'node:assert/strict'
  import { defaultInstanceConfig } from './src/store/defaults'
  import { getActiveParams } from './src/components/ConfigPage/activeParams'
  import { validateConfig } from './src/validators'
  import {
    detectModelWorkload,
    getResettableFields,
    isModelWorkloadLocked,
    normalizeConfigForSelectedModel,
    normalizeInstanceConfig,
    VECTOR_ALLOWED_FIELDS,
    VECTOR_CLASSIFIED_FIELDS,
  } from './src/modelPolicy'
  import {
    applyEngineInventory,
    applyModelInventory,
    beginEngineInventoryRequest,
    beginModelInventoryRequest,
    currentModelInventoryRequest,
    isCurrentEngineInventoryRequest,
    isCurrentModelInventoryRequest,
    normalizeModelPath,
    normalizeStoredConfig,
    reconcileInstancesWithModels,
  } from './src/store/bootstrap'
  import { synchronizeInstanceSummary } from './src/store/instanceSummary'
  import { findMatchingProjector } from './src/modelProjector'
  import { isConfiguredEngineMissing, resolveEffectiveEngine } from './src/store/engineResolution'

  const model = (overrides = {}) => ({
    id: 'model', name: 'model.gguf', path: 'C:/models/model.gguf', size: 1, file_type: 'gguf',
    ...overrides,
  })

  const engines = [{ id: 'default', name: 'Default' }, { id: 'selected', name: 'Selected' }]
  assert.equal(resolveEffectiveEngine({ engine_id: 'missing' }, engines, 'default'), null)
  assert.equal(resolveEffectiveEngine({ engine_id: 'selected' }, engines, 'default')?.id, 'selected')
  assert.equal(resolveEffectiveEngine({ engine_id: '' }, engines, 'default')?.id, 'default')
  assert.equal(isConfiguredEngineMissing({ engine_id: 'missing' }, engines), true)
  assert.equal(isConfiguredEngineMissing({ engine_id: 'selected' }, engines), false)
  assert.equal(isConfiguredEngineMissing({ engine_id: '' }, engines), false)

  const visionModel = model({
    path: 'C:/models/vision.gguf',
    capabilities: { metadata_complete: true, vision_family: 'qwen2-vl' },
  })
  const projectors = [
    model({ id: 'wrong', path: 'C:/models/mmproj-first.gguf', file_type: 'mmproj', capabilities: { projector_family: 'llava' } }),
    model({ id: 'right', path: 'C:/models/mmproj-qwen.gguf', file_type: 'mmproj', capabilities: { projector_family: 'qwen2-vl' } }),
  ]
  assert.equal(findMatchingProjector(visionModel, projectors)?.id, 'right')
  assert.equal(findMatchingProjector(visionModel, [projectors[0]]), null)
  assert.equal(findMatchingProjector(visionModel, [
    model({ id: 'unknown-family', path: 'C:/models/mmproj-unknown.gguf', file_type: 'mmproj' }),
  ])?.id, 'unknown-family')

  assert.equal(detectModelWorkload(model({ capabilities: { metadata_complete: true, is_embedding_model: true } })), 'embedding')
  assert.equal(detectModelWorkload(model({ capabilities: { metadata_complete: true, is_embedding_model: true, is_reranker_model: true } })), 'reranker')
  assert.equal(detectModelWorkload(model({ capabilities: { metadata_complete: true }, name: 'nomic-embed-text.gguf', path: 'C:/models/Qwen3-Instruct.gguf' })), 'embedding')
  assert.equal(detectModelWorkload(model({ capabilities: { metadata_complete: true }, architecture: 'sentence-bert' })), 'embedding')
  assert.equal(detectModelWorkload(null, 'C:\\\\models\\\\embedding\\\\Qwen3-Instruct.gguf'), 'inference')
  assert.equal(detectModelWorkload(
    model({ capabilities: { metadata_complete: true, is_embedding_model: false, is_reranker_model: false }, name: 'nomic-embed-text.gguf' }),
    '',
    { embedding: true, reranking: false },
  ), 'embedding')
  assert.equal(detectModelWorkload(
    model({ capabilities: { metadata_complete: true, is_embedding_model: false, is_reranker_model: false }, name: 'custom-model.gguf' }),
    '',
    { embedding: true, reranking: false },
  ), 'embedding')
  assert.equal(detectModelWorkload(
    model({ capabilities: { metadata_complete: true, is_embedding_model: false, is_reranker_model: false }, name: 'custom-model.gguf' }),
    '',
    { embedding: true, reranking: true },
  ), 'reranker')
  assert.equal(detectModelWorkload(null, 'C:/models/bge-reranker-v2-m3.gguf'), 'reranker')
  assert.equal(detectModelWorkload(null, 'C:/models/nomic-embed-text-v1.5.gguf'), 'embedding')
  assert.equal(detectModelWorkload(model({ architecture: 'sentence-bert' })), 'embedding')
  assert.equal(detectModelWorkload(null, 'C:/models/Qwen3-8B-Instruct.gguf'), 'inference')
  assert.equal(isModelWorkloadLocked(model({ capabilities: { metadata_complete: true, is_embedding_model: false, is_reranker_model: false } })), false)
  assert.equal(isModelWorkloadLocked(model({ name: 'nomic-embed-text.gguf' })), true)
  assert.equal(isModelWorkloadLocked(model({ name: 'custom-model.gguf', capabilities: { metadata_complete: false } })), false)
  assert.deepEqual(getResettableFields(['embd_normalize', 'reranking'], true, true), ['embd_normalize'])
  assert.deepEqual(getResettableFields(['embd_normalize', 'reranking'], true, false), ['embd_normalize', 'reranking'])

  const reranker = normalizeInstanceConfig(defaultInstanceConfig(), model({ capabilities: { metadata_complete: true, is_reranker_model: true } }))
  assert.equal(reranker.workload, 'reranker')
  assert.equal(reranker.config.embedding, true)
  assert.equal(reranker.config.reranking, true)
  assert.equal(reranker.config.pooling, 'rank')
  const cleanVectorWarnings = validateConfig(
    reranker.config,
    model({ capabilities: { metadata_complete: true, is_reranker_model: true } }),
    null,
  )
  assert.deepEqual(cleanVectorWarnings, [])

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

  const manuallyMarkedVector = normalizeInstanceConfig(
    { ...defaultInstanceConfig(), embedding: true, reranking: true, pooling: 'rank' },
    model({ capabilities: { metadata_complete: true, is_embedding_model: false, is_reranker_model: false } }),
  )
  assert.equal(manuallyMarkedVector.workload, 'reranker')
  assert.equal(manuallyMarkedVector.vectorMode, true)
  assert.equal(manuallyMarkedVector.config.embedding, true)
  assert.equal(manuallyMarkedVector.config.reranking, true)
  assert.equal(manuallyMarkedVector.config.pooling, 'rank')

  const switchedToInference = normalizeConfigForSelectedModel(
    { ...defaultInstanceConfig(), embedding: true, reranking: true, pooling: 'rank' },
    model({ capabilities: { metadata_complete: true, is_embedding_model: false, is_reranker_model: false } }),
  )
  assert.equal(switchedToInference.workload, 'inference')
  assert.equal(switchedToInference.vectorMode, false)
  assert.equal(switchedToInference.config.embedding, false)
  assert.equal(switchedToInference.config.reranking, false)
  assert.equal(switchedToInference.config.pooling, '')

  const switchedToUnknownInference = normalizeConfigForSelectedModel(
    { ...defaultInstanceConfig(), embedding: true, reranking: true, pooling: 'rank' },
    model({
      name: 'custom-model.gguf',
      path: 'C:/models/custom-model.gguf',
      capabilities: { metadata_complete: false },
    }),
  )
  assert.equal(switchedToUnknownInference.workload, 'inference')
  assert.equal(switchedToUnknownInference.config.embedding, false)
  assert.equal(switchedToUnknownInference.config.reranking, false)
  assert.equal(switchedToUnknownInference.config.pooling, '')

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
    explicit_overrides: ['custom_args'],
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
  assert.deepEqual(result.config.custom_args, polluted.custom_args)
  assert.equal(result.config.batch_size, 512)
  assert.deepEqual(new Set(Object.keys(defaultInstanceConfig())), VECTOR_CLASSIFIED_FIELDS)
  for (const key of VECTOR_ALLOWED_FIELDS) assert.ok(key in defaultInstanceConfig(), 'unknown allowed key: ' + key)
  assert.ok(result.changes.some(change => change.key === 'spec_type' && change.group === 'speculative'))
  assert.ok(result.changes.some(change => change.key === 'temp' && change.group === 'generation'))
  assert.equal(result.changes.some(change => change.key === 'custom_args'), false)

  const created = normalizeInstanceConfig(polluted, null, { context: 'create' })
  assert.equal(created.config.spec_type, '')
  assert.deepEqual(created.changes, [])

  const activeVectorParams = getActiveParams(polluted, true)
  for (const key of activeVectorParams) {
    assert.ok(VECTOR_ALLOWED_FIELDS.has(key), 'vector active params exposed an incompatible field: ' + key)
  }
  assert.equal(activeVectorParams.has('spec_type'), false)
  assert.equal(activeVectorParams.has('custom_args'), true)
  assert.equal(activeVectorParams.has('prefill_assistant'), false)

  assert.equal(normalizeModelPath('C:\\\\Models\\\\Qwen3-Instruct.gguf'), 'c:/models/qwen3-instruct.gguf')
  assert.equal(normalizeModelPath('/Models/Qwen3-Instruct.gguf'), '/Models/Qwen3-Instruct.gguf')
  assert.equal(normalizeModelPath('\\\\\\\\Server\\\\Share\\\\Model.GGUF'), '//server/share/model.gguf')
  assert.equal(normalizeModelPath('\\\\\\\\?\\\\C:\\\\Models\\\\Model.GGUF'), 'c:/models/model.gguf')
  assert.equal(normalizeModelPath('\\\\\\\\?\\\\UNC\\\\Server\\\\Share\\\\Model.GGUF'), '//server/share/model.gguf')
  assert.equal(normalizeModelPath('//Server/Share/Model.GGUF'), '//server/share/model.gguf')
  assert.equal(
    normalizeModelPath('//Server/Share/Model.GGUF'),
    normalizeModelPath('\\\\\\\\server\\\\share\\\\model.gguf'),
  )

  const staleInventoryRequest = beginModelInventoryRequest()
  const currentInventoryRequest = beginModelInventoryRequest()
  assert.equal(currentModelInventoryRequest(), currentInventoryRequest, 'passive reads must not supersede an active scan')
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

  const staleEngineRequest = beginEngineInventoryRequest()
  const currentEngineRequest = beginEngineInventoryRequest()
  assert.equal(isCurrentEngineInventoryRequest(staleEngineRequest), false)
  assert.equal(isCurrentEngineInventoryRequest(currentEngineRequest), true)
  const engineState = { engines: [] }
  assert.equal(applyEngineInventory([{ id: 'stale' }], partial => Object.assign(engineState, partial), {}, staleEngineRequest), false)
  assert.deepEqual(engineState.engines, [])

  const indexedVectorModel = model({
    name: 'Qwen3-Instruct.gguf',
    path: 'C:\\\\Models\\\\Qwen3-Instruct.gguf',
    capabilities: { metadata_complete: true, is_embedding_model: true, is_reranker_model: false },
  })
  const storedVectorConfig = {
    ...defaultInstanceConfig(),
    id: 'vector',
    name: 'Vector instance',
    port: 18080,
    model_path: 'c:/models/Qwen3-Instruct.gguf',
    embedding: false,
    temp: 1.5,
    custom_args: ['--temp', '1.5'],
    explicit_overrides: null,
  }
  const normalizedStored = normalizeStoredConfig(storedVectorConfig, [indexedVectorModel])
  assert.equal(normalizedStored.workload, 'embedding')
  assert.equal(normalizedStored.config.embedding, true)
  assert.equal(normalizedStored.config.temp, defaultInstanceConfig().temp)
  assert.deepEqual(normalizedStored.config.custom_args, storedVectorConfig.custom_args)
  assert.equal(normalizedStored.config.explicit_overrides?.includes('custom_args'), true)

  const missingInferenceConfig = {
    ...defaultInstanceConfig(),
    id: 'inference',
    name: 'Inference instance',
    port: 18081,
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

  const changedModelConfig = {
    ...missingInferenceConfig,
    name: 'Qwen3-Reranker-8B',
    model_path: 'C:/Models/Qwen3-Reranker-8B-Q8_0.gguf',
    port: 18082,
  }
  const staleSummary = {
    ...inferenceInstance,
    name: 'Qwen3-Reranker-8B',
    model: 'Qwen3.6-27B-Q6_K.gguf',
    port: 18081,
    config: changedModelConfig,
  }
  const synchronized = synchronizeInstanceSummary(staleSummary)
  assert.equal(synchronized.name, 'Qwen3-Reranker-8B')
  assert.equal(synchronized.model, 'Qwen3-Reranker-8B-Q8_0.gguf')
  assert.equal(synchronized.port, 18082)
  const summaryReconciled = reconcileInstancesWithModels([staleSummary], [])
  assert.equal(summaryReconciled.changed, true)
  assert.equal(summaryReconciled.instances[0].model, 'Qwen3-Reranker-8B-Q8_0.gguf')
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
assert.match(startInstanceSource, /await configSaveCoordinator\.waitForIdle\(\)/, 'start must await an in-flight inventory migration save')
assert.match(saveConfigSource, /reconcileInstancesWithModels\(/, 'save must normalize every instance')

const bootstrapSource = readSource('src', 'store', 'bootstrap.ts')
const reconcileSource = section(bootstrapSource, 'export function reconcileInstancesWithModels', 'export function applyModelInventory')
assert.match(reconcileSource, /new Map<string, ModelInfo>\(\)/, 'instance reconciliation must index models once per batch')
assert.doesNotMatch(reconcileSource, /models\.find\(/, 'instance reconciliation must not scan the full model list per instance')
const cachedScanSource = section(bootstrapSource, 'const cachedModelScanRequest', 'const injected')
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
assert.match(loadInitialDataSource, /currentModelInventoryRequest\(/, 'passive get_models must join the active request generation')
assert.doesNotMatch(loadInitialDataSource, /beginModelInventoryRequest\(/, 'passive get_models must not supersede an in-flight scan')
assert.match(loadInitialDataSource, /applyEngineInventory\(/, 'get_engines results must use the engine request generation')
assert.match(scanModelsSource, /applyModelInventory\(/, 'manual model scans must reconcile instances')
assert.match(scanModelsSource, /beginModelInventoryRequest\(/, 'manual scans must supersede older inventory requests')

const processConfigSource = section(bootstrapSource, 'async function processConfig', "invoke<DownloadManagerSnapshot>('get_download_manager_snapshot')")
assert.doesNotMatch(processConfigSource, /models:\s*\[\]/, 'hydration must retain the last good inventory until a fresh scan completes')

const instanceManagerSource = readSource('src', 'components', 'InstanceManager.tsx')
const createInstanceSource = section(instanceManagerSource, 'const handleCreate', 'const handleDelete')
const showCommandSource = section(instanceManagerSource, 'const handleShowCommand', 'const handleTestConnection')
const commandFeedbackSource = readSource('src', 'components', 'InstanceManager', 'CommandFeedbackModal.tsx')
const browserMockSource = readSource('browser-tests', 'tauriMock.ts')
assert.match(instanceManagerSource, /resolveEffectiveEngine\(inst\.config, engines, defaultEngineId\)\?\.name/, 'instance rows must display the engine that will actually be used')
assert.match(instanceManagerSource, /labels\.configuredEngineMissing/, 'missing configured engines must be shown explicitly')
assert.match(instanceManagerSource, /MissingEngineBanner/, 'missing configured engines must have a proactive recovery banner')
assert.match(showCommandSource, /setCommandError\(/, 'command generation failures must be visible to the user')
assert.match(showCommandSource, /labels\.missingEngineCommandError/, 'removed engine bindings must explain why command generation is blocked')
assert.match(showCommandSource, /labels\.noEngineCommandError/, 'empty engine inventories must explain why command generation is blocked')
assert.match(showCommandSource, /labels\.commandGenerationFailed/, 'backend command generation errors must be surfaced')
assert.doesNotMatch(showCommandSource, /if\s*\(!engine\)\s*return/, 'missing engines must not make the generate-command action silently return')
assert.match(commandFeedbackSource, /role="alertdialog"/, 'command generation errors must use an accessible alert dialog')
assert.match(commandFeedbackSource, /onRecover/, 'missing engine errors must provide a direct recovery action')
assert.match(browserMockSource, /BROWSER_SCENARIO === 'missing-engine'/, 'browser tests must cover stale engine bindings')
assert.match(browserMockSource, /BROWSER_SCENARIO === 'command-error'/, 'browser tests must cover backend command failures')
assert.match(createInstanceSource, /normalizeInstanceConfig\(/, 'new instances must use the vector policy')
assert.match(createInstanceSource, /context:\s*'create'/, 'new instance cleanup must be silent')
assert.match(createInstanceSource, /config:\s*normalized\.config/, 'new instances must store the normalized config')
assert.match(createInstanceSource, /markExplicitOverride\(config, 'mmproj_path'/, 'auto-selected projectors must remain explicit launch arguments')
assert.ok(
  createInstanceSource.indexOf('config.engine_id') < createInstanceSource.indexOf('normalizeInstanceConfig('),
  'new instance identity and launch fields must be assigned before normalization',
)

const configPageSource = readSource('src', 'components', 'ConfigPage.tsx')
const configWorkspaceSource = readSource('src', 'components', 'ConfigPage', 'configWorkspace.ts')
const modelAssetPickerSource = readSource('src', 'components', 'ConfigPage', 'ModelAssetPicker.tsx')
const applyPrimaryModelSource = section(configPageSource, 'const applyPrimaryModelPath', 'const pickModel')
assert.match(applyPrimaryModelSource, /normalizeConfigForSelectedModel\(/, 'all primary model path commits must use atomic workload selection')
assert.match(applyPrimaryModelSource, /setVectorCleanupChanges\(/, 'manual primary model paths must retain cleanup feedback')
assert.match(applyPrimaryModelSource, /committedModelPathRef\.current/, 'repeated blur events must not reclassify an unchanged manual model path')
assert.match(applyPrimaryModelSource, /findMatchingProjector\(selectedModel, models\)/, 'primary model changes must safely auto-match a same-directory projector')
assert.match(applyPrimaryModelSource, /mmproj\?\.path \?\? ''/, 'failed automatic projector matching must clear the stale projector path')
const pickModelSource = section(configPageSource, 'const pickModel', 'const save =')
assert.match(pickModelSource, /applyPrimaryModelPath\(modelPath\)/, 'the model picker must reuse the primary model path commit policy')
assert.doesNotMatch(pickModelSource, /set\('model_path'/, 'primary model switching must not apply sequential partial updates')
assert.match(pickModelSource, /set\('mmproj_path', modelPath\)/, 'manual projector selection must update the managed mmproj path')
assert.match(configPageSource, /setPickerTarget\('mmproj'\)/, 'the parameter page must expose a dedicated projector picker target')
assert.match(modelAssetPickerSource, /target === 'mmproj'[\s\S]*model\.file_type === 'mmproj'/, 'projector picker mode must filter the inventory to projector files')
assert.match(modelAssetPickerSource, /target === 'mmproj'[\s\S]*onPick\(model\.path\)/, 'projector rows must be selectable only in projector picker mode')

const configDiffSource = section(configPageSource, 'const savedBaseline', 'const liveWarnings')
assert.match(configDiffSource, /vectorCleanupChanges/, 'cleanup-only keys must be filtered from the ordinary config diff')
assert.match(configDiffSource, /isEqualValue\(local\[change\.key\],\s*change\.after\)/, 'manual edits after cleanup must remain visible in the ordinary diff')
assert.match(configWorkspaceSource, /key === 'custom_args'/, 'custom argument diffs must render counts instead of values')
const saveSource = section(configPageSource, 'const save =', 'const sectionProps')
assert.match(saveSource, /const normalized = manualMode[\s\S]*: modelPathChanged/, 'managed saves must select normalization based on whether the model path changed')
assert.match(saveSource, /modelPathChanged[\s\S]*normalizeConfigForSelectedModel/, 'save must treat a manually edited model path as an explicit model switch')
assert.match(saveSource, /validateConfig\(persistedConfig, currentModel, engine\)/, 'save validation must inspect the backend-normalized configuration')
assert.match(saveSource, /config:\s*normalized\.config/, 'save must persist the same normalized configuration that was validated')
assert.ok(
  saveSource.indexOf('await saveConfig()') < saveSource.indexOf('validateConfig(persistedConfig'),
  'save validation must happen after the persisted configuration is accepted',
)
assert.match(saveSource, /setVectorCleanupChanges\(\[\]\)/, 'cleanup summary must clear only after a successful save')
assert.ok(
  saveSource.indexOf('await saveConfig()') < saveSource.indexOf('setVectorCleanupChanges([])'),
  'cleanup summary must clear after persistence succeeds',
)
assert.match(configPageSource, /\{!isEmbedding && <ReasoningSection/, 'reasoning section must be absent in vector mode')
assert.match(configPageSource, /\{!manualMode && !isEmbedding && \(\s*<Surface[^>]*data-guide="config-presets"/s, 'inference presets must be absent in vector and manual modes')
assert.match(configPageSource, /showPresetAssistant && !isEmbedding/, 'preset assistant must not render in vector mode')

const sectionsSource = readSource('src', 'components', 'ConfigPage', 'sections.tsx')
const basicSectionSource = section(sectionsSource, 'export function BasicSection', 'export function ReasoningSection')
assert.match(basicSectionSource, /onBlur=\{e => onCommitModelPath\?\.\(e\.target\.value\)\}/, 'manual model path edits must commit through the model policy')
assert.match(basicSectionSource, /disabled=\{modelWorkloadLocked\}/, 'classified model workloads must lock the embedding switch')
const advancedSectionSource = section(sectionsSource, 'export function AdvancedSection', '\n}\n')
for (const id of ['reasoning', 'model', 'sampling', 'sampling-ext', 'spec', 'multi']) {
  const marker = `config-advanced-${id}`
  const markerIndex = advancedSectionSource.indexOf(marker)
  assert.ok(markerIndex >= 0, `missing advanced group marker: ${marker}`)
  assert.match(advancedSectionSource.slice(Math.max(0, markerIndex - 180), markerIndex), /!isEmbedding/, `${marker} must be hidden in vector mode`)
}
const customMarkerIndex = advancedSectionSource.indexOf('config-advanced-custom')
assert.ok(customMarkerIndex >= 0, 'missing advanced custom argument group')
assert.doesNotMatch(advancedSectionSource.slice(Math.max(0, customMarkerIndex - 180), customMarkerIndex), /!isEmbedding/, 'custom arguments must remain visible in vector mode')
assert.match(advancedSectionSource, /config-advanced-vector/, 'vector mode must expose a dedicated vector group')
const vectorGroupSource = advancedSectionSource.slice(advancedSectionSource.indexOf('config-advanced-vector'))
assert.match(vectorGroupSource, /reranking[^\n]*disabled=\{modelWorkloadLocked\}/, 'manually configured vector workloads must retain a reranking control')
assert.match(vectorGroupSource, /set\('pooling', v \? 'rank' : ''\)/, 'manual reranking changes must update pooling atomically')
assert.match(advancedSectionSource, /ADVANCED_CONFIG_KEYS/, 'reset all must cover every advanced field exposed by the UI')
assert.match(advancedSectionSource, /ADVANCED_GROUP_CONFIG_KEYS\[id\]/, 'group reset must cover fields added beyond RESET_MAP')
assert.match(advancedSectionSource, /getResettableFields/, 'reset handlers must preserve locked workload identity fields')
assert.match(advancedSectionSource, /direct_io/, 'shared direct I/O configuration must have a visible control')
assert.match(advancedSectionSource, /onShowMmprojPicker/, 'the mmproj field must expose its independent repository picker')
assert.match(advancedSectionSource, /mmprojPathBtn/, 'the mmproj picker button must have a localized accessible label')

assert.doesNotMatch(configPageSource, /\[activeConfigInstanceId, inst\?\.config\]/, 'inventory reconciliation must not overwrite an active local draft')
assert.match(configPageSource, /const \[baseline, setBaseline\]/, 'the editor must retain a stable local baseline')
assert.match(configPageSource, /const committedModelPathRef = useRef/, 'the editor must distinguish typed paths from committed model selections')
assert.match(configPageSource, /const savedBaseline = baseline \?\?/, 'unsaved diffs must compare against the stable local baseline')

for (const locale of ['zh-CN.ts', 'en-US.ts']) {
  const source = readSource('src', 'i18n', locale)
  assert.match(source, /vectorCleanupTitle:/, `${locale} must include the vector cleanup title`)
  assert.match(source, /vectorCleanupSpeculative:/, `${locale} must label speculative cleanup`)
  assert.match(source, /vectorCleanupCustom:/, `${locale} must label custom cleanup without exposing values`)
}

console.log('vector model policy tests passed')
