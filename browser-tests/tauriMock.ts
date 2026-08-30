import { mockConvertFileSrc, mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { emit } from '@tauri-apps/api/event'
import { defaultInstanceConfig } from '../src/store/defaults'
import { normalizeSpeculativeTypes } from '../src/speculativeTypes'
import type { GlobalConfigShape } from '../src/store/bootstrap'
import type {
  EngineInfo,
  GeneratedServerCommand,
  InstanceConfig,
  MsFileEntry,
  ModelInfo,
  MonitoringFrame,
  SystemMetrics,
  WorkerInfo,
} from '../src/store/types'

const BROWSER_TEST_MARKER = '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__'
const BROWSER_SCENARIO = new URLSearchParams(window.location.search).get('scenario')
const IS_DOCS_SCENARIO = BROWSER_SCENARIO === 'docs-screenshots'
const HAS_MONITORING_DATA = BROWSER_SCENARIO === 'monitoring' || IS_DOCS_SCENARIO
const HAS_PROXY_DATA = [
  'background-runtime-active',
  'proxy-routing',
  'proxy-route-health',
  'proxy-route-legacy-ids',
  'docs-screenshots',
].includes(BROWSER_SCENARIO ?? '')
const INSTANCE_ID = 'browser-test-instance'
const STOPPED_INSTANCE_ID = 'browser-stopped-instance'
const EMBEDDING_INSTANCE_ID = 'browser-embedding-instance'
const ENGINE_ID = 'browser-test-engine'
const VULKAN_ENGINE_ID = 'browser-vulkan-engine'
const ROCM_ENGINE_ID = 'browser-rocm-engine'
const MODEL_PATH = 'C:\\browser-test\\models\\Qwen-Browser-Test-Q8_0.gguf'
const AMBIGUOUS_MODEL_PATH = 'C:\\browser-test\\models\\Vision-Ambiguous-Q8_0.gguf'
const QWEN_PROJECTOR_PATH = 'C:\\browser-test\\models\\mmproj-Qwen-BF16.gguf'
const LLAVA_PROJECTOR_PATH = 'C:\\browser-test\\models\\mmproj-Llava-BF16.gguf'
const DOCS_MODEL_ROOT = 'C:\\AI\\Models'
const DOCS_ENGINE_ROOT = 'C:\\AI\\Engines'
const DOCS_CHAT_MODEL_PATH = `${DOCS_MODEL_ROOT}\\Qwen3-8B-Q4_K_M.gguf`
const DOCS_VISION_MODEL_PATH = `${DOCS_MODEL_ROOT}\\Qwen3-VL-7B-Q5_K_M.gguf`
const DOCS_EMBEDDING_MODEL_PATH = `${DOCS_MODEL_ROOT}\\Embedding-Mini-Q8_0.gguf`
const DOCS_PROJECTOR_PATH = `${DOCS_MODEL_ROOT}\\mmproj-Qwen3-VL-F16.gguf`

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T

const model: ModelInfo = {
  id: 'browser-test-model',
  name: 'Qwen Browser Test Q8_0.gguf',
  path: MODEL_PATH,
  size: 4_294_967_296,
  architecture: 'qwen3',
  context_length: 131_072,
  quant_type: 'Q8_0',
  capabilities: {
    metadata_complete: true,
    is_vision_model: true,
    vision_status: 'confirmed',
    vision_family: 'qwen-vl',
    model_name: 'Qwen Browser Test',
    base_model_repo: 'https://huggingface.co/browser-tests/Qwen-Browser-Test',
    tags: ['image-text-to-text'],
  },
  file_type: 'model',
}

const ambiguousModel: ModelInfo = {
  ...model,
  id: 'browser-test-ambiguous-model',
  name: 'Vision Ambiguous Q8_0.gguf',
  path: AMBIGUOUS_MODEL_PATH,
  capabilities: { metadata_complete: true, is_vision_model: true, vision_family: 'browser-unknown' },
}

const qwenProjector: ModelInfo = {
  id: 'browser-test-qwen-projector',
  name: 'mmproj-Qwen-BF16.gguf',
  path: QWEN_PROJECTOR_PATH,
  size: 536_870_912,
  capabilities: {
    metadata_complete: true,
    is_mmproj: true,
    projector_family: 'qwen-vl',
    projector_type: 'qwen3vl_merger',
    model_name: 'Qwen Browser Test',
    base_model_repo: 'https://huggingface.co/browser-tests/Qwen-Browser-Test',
    tags: ['image-text-to-text'],
  },
  file_type: 'mmproj',
}

const llavaProjector: ModelInfo = {
  ...qwenProjector,
  id: 'browser-test-llava-projector',
  name: 'mmproj-Llava-BF16.gguf',
  path: LLAVA_PROJECTOR_PATH,
  capabilities: { metadata_complete: true, is_mmproj: true, projector_family: 'llava' },
}

const docsModels: ModelInfo[] = [
  {
    ...model,
    id: 'docs-qwen3-chat',
    name: 'Qwen3 8B Chat · Q4_K_M',
    path: DOCS_CHAT_MODEL_PATH,
    size: 5_032_755_200,
    quant_type: 'Q4_K_M',
    capabilities: {
      metadata_complete: true,
      vision_status: 'text-only',
      model_name: 'Qwen3 8B Chat',
      base_model_repo: 'https://huggingface.co/Qwen/Qwen3-8B',
      tags: ['text-generation'],
    },
  },
  {
    ...model,
    id: 'docs-qwen3-vision',
    name: 'Qwen3 VL 7B · Q5_K_M',
    path: DOCS_VISION_MODEL_PATH,
    size: 6_442_450_944,
    quant_type: 'Q5_K_M',
    capabilities: {
      metadata_complete: true,
      is_vision_model: true,
      vision_status: 'confirmed',
      vision_family: 'qwen-vl',
      model_name: 'Qwen3 VL 7B',
      base_model_repo: 'https://huggingface.co/Qwen/Qwen3-VL-7B-Instruct',
      tags: ['image-text-to-text'],
    },
  },
  {
    ...model,
    id: 'docs-embedding-mini',
    name: 'Embedding Mini · Q8_0',
    path: DOCS_EMBEDDING_MODEL_PATH,
    size: 1_073_741_824,
    architecture: 'bert',
    context_length: 8_192,
    capabilities: {
      metadata_complete: true,
      is_embedding_model: true,
      vision_status: 'text-only',
      model_name: 'Embedding Mini',
      tags: ['feature-extraction'],
    },
  },
  {
    ...qwenProjector,
    id: 'docs-qwen3-projector',
    name: 'mmproj Qwen3 VL · F16',
    path: DOCS_PROJECTOR_PATH,
    capabilities: {
      ...qwenProjector.capabilities,
      model_name: 'Qwen3 VL 7B',
      base_model_repo: 'https://huggingface.co/Qwen/Qwen3-VL-7B-Instruct',
    },
  },
]

const models = IS_DOCS_SCENARIO ? docsModels : [model, ambiguousModel, qwenProjector, llavaProjector]

const engine: EngineInfo = {
  id: ENGINE_ID,
  name: IS_DOCS_SCENARIO ? 'CUDA 12.4' : 'Browser Test Engine',
  dir: IS_DOCS_SCENARIO ? `${DOCS_ENGINE_ROOT}\\cuda-b10042` : 'C:\\browser-test\\engine',
  exe: IS_DOCS_SCENARIO ? `${DOCS_ENGINE_ROOT}\\cuda-b10042\\llama-server.exe` : 'C:\\browser-test\\engine\\llama-server.exe',
  version: IS_DOCS_SCENARIO ? 'version: 10042 (6d2f8e1)' : 'version: 10042 (browser-test)',
  backend: IS_DOCS_SCENARIO ? 'CUDA' : 'Vulkan',
  capabilities: {
    status: 'detected',
    versionStatus: 'detected',
    supportedFlags: [
      '--temp', '--top-k', '--top-p', '--threads', '--kv-unified',
      '--mmap', '--no-mmap', '--perf', '--no-perf',
      '--models-autoload', '--no-models-autoload', '--image-min-tokens', '--mmproj',
      '--spec-type',
    ],
    reportedDefaults: {
      '--temp': '0.8',
      '--threads': 'automatic',
      '--mmap': 'enabled',
    },
    reportedDefaultsVersion: 2,
    speculativeTypes: [
      'none', 'draft-simple', 'draft-eagle3', 'draft-mtp', 'draft-dflash', 'draft-dspark',
      'ngram-simple', 'ngram-map-k', 'ngram-map-k4v', 'ngram-mod', 'ngram-cache',
    ],
    helpHash: 'browser-test-help',
    executableFingerprint: 'browser-test-engine-fingerprint',
    probedAt: IS_DOCS_SCENARIO ? Date.UTC(2026, 6, 31, 12, 0, 0) / 1000 : 1,
  },
}

const vulkanEngine: EngineInfo = {
  ...clone(engine),
  id: VULKAN_ENGINE_ID,
  name: 'Vulkan',
  dir: `${DOCS_ENGINE_ROOT}\\vulkan-b10042`,
  exe: `${DOCS_ENGINE_ROOT}\\vulkan-b10042\\llama-server.exe`,
  version: 'version: 10042 (6d2f8e1)',
  backend: 'Vulkan',
  capabilities: {
    ...clone(engine.capabilities!),
    helpHash: 'docs-vulkan-help',
    executableFingerprint: 'docs-vulkan-engine-fingerprint',
  },
}

const rocmEngine: EngineInfo = {
  ...clone(engine),
  id: ROCM_ENGINE_ID,
  name: 'Browser ROCm Engine',
  dir: 'C:\\browser-test\\rocm-engine',
  exe: 'C:\\browser-test\\rocm-engine\\llama-server.exe',
  backend: 'ROCm',
  capabilities: {
    ...clone(engine.capabilities!),
    helpHash: 'browser-rocm-help',
    executableFingerprint: 'browser-rocm-engine-fingerprint',
  },
}

const batchProbeEngines: EngineInfo[] = [
  engine,
  {
    ...clone(vulkanEngine),
    name: 'Browser Vulkan Engine',
    dir: 'C:\\browser-test\\vulkan-engine',
    exe: 'C:\\browser-test\\vulkan-engine\\llama-server.exe',
  },
  rocmEngine,
]

const engines = IS_DOCS_SCENARIO
  ? [engine, vulkanEngine]
  : BROWSER_SCENARIO?.startsWith('engine-probe-batch')
    ? batchProbeEngines
    : [engine]

const instanceConfig: InstanceConfig = {
  ...defaultInstanceConfig(),
  id: INSTANCE_ID,
  name: IS_DOCS_SCENARIO ? 'Qwen3 8B Chat' : 'Browser Parameter Regression',
  engine_id: ENGINE_ID,
  model_path: IS_DOCS_SCENARIO ? DOCS_CHAT_MODEL_PATH : MODEL_PATH,
  alias: IS_DOCS_SCENARIO ? 'qwen3-chat' : 'browser-parameter-regression',
  port: 18081,
  temp: 0.6,
  top_k: 20,
  kv_unified: true,
  kv_unified_mode: 'on',
  models_autoload: false,
  image_min_tokens: 1_024,
  explicit_overrides: ['temp', 'top_k', 'kv_unified', 'kv_unified_mode', 'models_autoload', 'image_min_tokens'],
}

const docsVisionConfig: InstanceConfig = {
  ...clone(instanceConfig),
  id: STOPPED_INSTANCE_ID,
  name: 'Qwen3 VL 7B',
  model_path: DOCS_VISION_MODEL_PATH,
  mmproj_path: DOCS_PROJECTOR_PATH,
  alias: 'qwen3-vl',
  port: 18082,
  explicit_overrides: [...(instanceConfig.explicit_overrides ?? []), 'mmproj_path'],
}

const docsEmbeddingConfig: InstanceConfig = {
  ...clone(instanceConfig),
  id: EMBEDDING_INSTANCE_ID,
  name: 'Embedding Mini',
  model_path: DOCS_EMBEDDING_MODEL_PATH,
  alias: 'embedding-mini',
  port: 18083,
  embedding: true,
  pooling: 'mean',
}

const state: GlobalConfigShape = {
  instances: { [INSTANCE_ID]: clone(instanceConfig) },
  model_dirs: ['C:\\browser-test\\models'],
  engine_dirs: ['C:\\browser-test\\engine'],
  default_engine_id: ENGINE_ID,
  running: {},
  instance_order: [INSTANCE_ID],
  last_tab: 'instances',
  dark_mode: true,
}

if (IS_DOCS_SCENARIO) {
  state.instances = {
    [INSTANCE_ID]: clone(instanceConfig),
    [STOPPED_INSTANCE_ID]: clone(docsVisionConfig),
    [EMBEDDING_INSTANCE_ID]: clone(docsEmbeddingConfig),
  }
  state.model_dirs = [DOCS_MODEL_ROOT]
  state.engine_dirs = [DOCS_ENGINE_ROOT]
  state.default_engine_id = ENGINE_ID
  state.running = {
    [INSTANCE_ID]: {
      instance_id: INSTANCE_ID,
      pid: 4243,
      port: instanceConfig.port,
      host: instanceConfig.host,
      start_time: Math.floor(Date.now() / 1000) - 1_260,
    },
  }
  state.instance_order = [INSTANCE_ID, STOPPED_INSTANCE_ID, EMBEDDING_INSTANCE_ID]
  state.last_tab = 'dashboard'
}
if (BROWSER_SCENARIO === 'checkpoint-requirements') {
  Object.assign(state.instances[INSTANCE_ID], {
    parallel: 2,
    cache_prompt: false,
    cache_idle_slots: false,
    cache_ram: 0,
    slots_enabled: false,
    swa_full: false,
    kv_checkpoint: {
      ...state.instances[INSTANCE_ID].kv_checkpoint,
      enabled: true,
    },
  })
}

type BrowserProxyRoute = {
  id: string
  enabled: boolean
  priority: number
  weight?: number
  max_concurrent_requests?: number
  model_alias: string
  target_instance_id: string
}

type BrowserProxyApiKey = {
  id: string
  name: string
  key: string
  enabled: boolean
  scopes: string[]
  requests_per_minute: number
}

type BrowserProxyConfig = {
  enabled: boolean
  host: string
  port: number
  public_api_key: string
  default_instance_id: string
  routing_strategy: string
  strict_model_routing: boolean
  connect_timeout_ms: number
  timeout_ms: number
  streaming_idle_timeout_ms: number
  health_check_interval_ms: number
  health_check_timeout_ms: number
  unhealthy_threshold: number
  recovery_cooldown_ms: number
  max_concurrent_requests: number
  queue_timeout_ms: number
  requests_per_minute: number
  cors_allowed_origins: string[]
  api_keys: BrowserProxyApiKey[]
  background_service_mode: boolean
  runtime_service_enabled: boolean
  routes: BrowserProxyRoute[]
}

const proxyConfig: BrowserProxyConfig = {
  enabled: HAS_PROXY_DATA,
  host: '127.0.0.1',
  port: 11435,
  public_api_key: '',
  default_instance_id: IS_DOCS_SCENARIO ? INSTANCE_ID : '',
  routing_strategy: 'priorityFailover',
  strict_model_routing: true,
  connect_timeout_ms: 5_000,
  timeout_ms: 600_000,
  streaming_idle_timeout_ms: 300_000,
  health_check_interval_ms: 5_000,
  health_check_timeout_ms: 2_000,
  unhealthy_threshold: 3,
  recovery_cooldown_ms: 15_000,
  max_concurrent_requests: 64,
  queue_timeout_ms: 1_000,
  requests_per_minute: 0,
  cors_allowed_origins: [],
  api_keys: [],
  background_service_mode: false,
  runtime_service_enabled: BROWSER_SCENARIO === 'background-runtime-active',
  routes: IS_DOCS_SCENARIO
    ? [
        {
          id: 'docs-chat-route',
          enabled: true,
          priority: 1,
          model_alias: 'qwen3-chat',
          target_instance_id: INSTANCE_ID,
        },
        {
          id: 'docs-vision-route',
          enabled: true,
          priority: 1,
          model_alias: 'qwen3-vl',
          target_instance_id: STOPPED_INSTANCE_ID,
        },
        {
          id: 'docs-embedding-route',
          enabled: true,
          priority: 1,
          model_alias: 'embedding-mini',
          target_instance_id: EMBEDDING_INSTANCE_ID,
        },
      ]
    : BROWSER_SCENARIO === 'proxy-route-health'
    ? [
        {
          id: 'primary-stopped-route',
          enabled: true,
          priority: 1,
          model_alias: 'public-browser-model',
          target_instance_id: STOPPED_INSTANCE_ID,
        },
        {
          id: 'backup-running-route',
          enabled: true,
          priority: 2,
          model_alias: 'public-browser-model',
          target_instance_id: INSTANCE_ID,
        },
      ]
    : BROWSER_SCENARIO === 'proxy-route-legacy-ids'
      ? [
          {
            id: '',
            enabled: true,
            priority: 1,
            model_alias: 'legacy-primary-model',
            target_instance_id: STOPPED_INSTANCE_ID,
          },
          {
            id: '',
            enabled: true,
            priority: 2,
            model_alias: 'legacy-backup-model',
            target_instance_id: INSTANCE_ID,
          },
        ]
    : [],
}
const proxyStatus = {
  running: HAS_PROXY_DATA,
  bound_addr: '127.0.0.1:11435',
  active_routes: IS_DOCS_SCENARIO ? 3 : ['proxy-route-health', 'proxy-route-legacy-ids'].includes(BROWSER_SCENARIO ?? '') ? 2 : BROWSER_SCENARIO === 'proxy-routing' ? 1 : 0,
  healthy_routes: HAS_PROXY_DATA ? 1 : 0,
  unhealthy_routes: IS_DOCS_SCENARIO ? 2 : ['proxy-route-health', 'proxy-route-legacy-ids'].includes(BROWSER_SCENARIO ?? '') ? 1 : 0,
  in_flight_requests: 0,
  total_requests: IS_DOCS_SCENARIO ? 42 : 0,
  last_error: null,
}
const runningProxyTarget = {
  instance_id: INSTANCE_ID,
  name: IS_DOCS_SCENARIO ? 'Qwen3 8B Chat' : 'Browser Parameter Regression',
  alias: IS_DOCS_SCENARIO ? 'qwen3-chat' : 'browser-parameter-regression',
  host: '127.0.0.1',
  port: 18081,
  running: true,
}
const proxyTargets = IS_DOCS_SCENARIO
  ? [
      runningProxyTarget,
      {
        instance_id: STOPPED_INSTANCE_ID,
        name: 'Qwen3 VL 7B',
        alias: 'qwen3-vl',
        host: '127.0.0.1',
        port: 18082,
        running: false,
      },
      {
        instance_id: EMBEDDING_INSTANCE_ID,
        name: 'Embedding Mini',
        alias: 'embedding-mini',
        host: '127.0.0.1',
        port: 18083,
        running: false,
      },
    ]
  : BROWSER_SCENARIO === 'proxy-routing'
  ? [runningProxyTarget]
  : ['proxy-route-health', 'proxy-route-legacy-ids'].includes(BROWSER_SCENARIO ?? '')
  ? [{
      instance_id: STOPPED_INSTANCE_ID,
      name: 'Stopped Primary',
      alias: 'stopped-primary',
      host: '127.0.0.1',
      port: 18082,
      running: false,
    }, runningProxyTarget]
    : []
const runtimeStatus = {
  servicePid: 4242,
  serviceVersion: IS_DOCS_SCENARIO ? '2.9.37' : '2.9.30-browser-test',
  backgroundEnabled: BROWSER_SCENARIO === 'background-runtime-active',
  registeredForLogin: BROWSER_SCENARIO === 'background-runtime-active',
  running: BROWSER_SCENARIO === 'background-runtime-active'
    ? { 'browser-background-instance': { pid: 4243 } }
    : {},
  lastError: null,
}

if (BROWSER_SCENARIO === 'missing-engine') {
  state.instances[INSTANCE_ID].engine_id = 'removed-browser-test-engine'
}
if (BROWSER_SCENARIO === 'multimodal-match') {
  state.instances[INSTANCE_ID].mmproj_path = QWEN_PROJECTOR_PATH
  state.instances[INSTANCE_ID].explicit_overrides = [
    ...(state.instances[INSTANCE_ID].explicit_overrides ?? []),
    'mmproj_path',
  ]
}
if (BROWSER_SCENARIO === 'multimodal-mismatch') {
  state.instances[INSTANCE_ID].mmproj_path = LLAVA_PROJECTOR_PATH
  state.instances[INSTANCE_ID].explicit_overrides = [
    ...(state.instances[INSTANCE_ID].explicit_overrides ?? []),
    'mmproj_path',
  ]
}
if (BROWSER_SCENARIO === 'empty-model-roots') {
  state.model_dirs = []
}
if (BROWSER_SCENARIO === 'windows-path-aliases') {
  engine.dir = '\\\\?\\c:\\browser-test\\engine\\build'
  engine.exe = `${engine.dir}\\llama-server.exe`
  state.model_dirs = ['\\\\?\\c:\\browser-test\\models']
  model.path = '\\\\?\\c:\\browser-test\\models\\Qwen-Browser-Test-Q8_0.gguf'
}
if (BROWSER_SCENARIO === 'empty-alias') {
  state.instances[INSTANCE_ID].alias = ''
  state.instances[INSTANCE_ID].explicit_overrides = (
    state.instances[INSTANCE_ID].explicit_overrides ?? []
  ).filter(field => field !== 'alias')
}
if (BROWSER_SCENARIO === 'auto-start-stagger') {
  const secondInstanceId = 'browser-auto-start-second'
  state.instances[INSTANCE_ID].auto_start = true
  state.instances[secondInstanceId] = {
    ...clone(instanceConfig),
    id: secondInstanceId,
    name: 'Browser Auto Start Second',
    alias: 'browser-auto-start-second',
    port: 18083,
    auto_start: true,
  }
  state.instance_order = [INSTANCE_ID, secondInstanceId]
}
if (BROWSER_SCENARIO === 'monitoring') {
  state.instances[STOPPED_INSTANCE_ID] = {
    ...clone(instanceConfig),
    id: STOPPED_INSTANCE_ID,
    name: 'Stopped Monitoring Instance',
    alias: 'stopped-monitoring-instance',
    port: 18082,
  }
  state.instance_order = [INSTANCE_ID, STOPPED_INSTANCE_ID]
  state.running[INSTANCE_ID] = {
    instance_id: INSTANCE_ID,
    pid: 4243,
    port: instanceConfig.port,
    host: instanceConfig.host,
    start_time: Math.floor(Date.now() / 1000) - 120,
  }
}
if (BROWSER_SCENARIO === 'instance-order-filter') {
  const hiddenId = 'browser-hidden-instance'
  const betaId = 'browser-group-beta'
  state.instances[INSTANCE_ID].name = 'Group Alpha'
  state.instances[hiddenId] = {
    ...clone(instanceConfig),
    id: hiddenId,
    name: 'Unrelated Hidden Instance',
    alias: 'unrelated-hidden-instance',
    port: 18082,
  }
  state.instances[betaId] = {
    ...clone(instanceConfig),
    id: betaId,
    name: 'Group Beta',
    alias: 'group-beta',
    port: 18083,
  }
  state.instance_order = [INSTANCE_ID, hiddenId, betaId]
}
if (BROWSER_SCENARIO === 'instance-connection') {
  state.running[INSTANCE_ID] = {
    instance_id: INSTANCE_ID,
    pid: 4243,
    port: instanceConfig.port,
    host: instanceConfig.host,
    start_time: Math.floor(Date.now() / 1000) - 120,
  }
}

type BrowserTestControl = {
  marker: string
  calls: Array<{ command: string; payload: unknown; at: number }>
  unhandled: string[]
  saveCount: number
  lastGenerated: GeneratedServerCommand | null
  failProxyStatus: boolean
  failProxyTargets: boolean
  failRuntimeStatus: boolean
  updaterCheckCount: number
  activeProbeCount: number
  peakProbeCount: number
  state: GlobalConfigShape
  emitEvent: (event: string, payload?: unknown) => Promise<void>
  releaseBrowse: (repoId: string, files: MsFileEntry[]) => void
  releasePortCheck: (port: number, available: boolean) => void
  releaseStart: () => void
  releaseWorkerScan: (workers: WorkerInfo[]) => void
}

declare global {
  interface Window {
    __TAURI_BROWSER_TEST__: BrowserTestControl
  }
}

let releasePendingStart: (() => void) | null = null
let delayedInventoryCacheLoaded = false
let mdnsDiscoveryActive = false
let storageWebviewScheduled = false
const storageAuthorizedDirectories = [
  { purpose: 'model', root: 'C:\\browser-test\\models' },
  { purpose: 'engine', root: 'C:\\browser-test\\engines' },
]
const storageGroups = [
  { id: 'private-scratch', ownership: 'manager', action: 'confirm', automatic: true, itemCount: 1, eligibleCount: 1, totalBytes: 4096, eligibleBytes: 4096, items: [{ path: 'C:\\browser-test\\app-data\\.config.deadbeef.tmp', bytes: 4096, modifiedAt: Date.now() - 172_800_000, eligible: true, safe: true, reason: 'older-than-24-hours' }], warnings: [] },
  { id: 'private-quarantine', ownership: 'manager', action: 'confirm', automatic: true, itemCount: 1, eligibleCount: 0, totalBytes: 512, eligibleBytes: 0, items: [], warnings: [] },
  { id: 'updater-staging', ownership: 'platform', action: 'confirm', automatic: true, itemCount: 1, eligibleCount: 1, totalBytes: 15_937_536, eligibleBytes: 15_937_536, items: [], warnings: [] },
  { id: 'developer-temp', ownership: 'platform', action: 'confirm', automatic: false, itemCount: 2, eligibleCount: 2, totalBytes: 39_800_000, eligibleBytes: 39_800_000, items: [], warnings: [] },
  { id: 'crash-manager', ownership: 'platform', action: 'confirm', automatic: false, itemCount: 1, eligibleCount: 1, totalBytes: 3_216_380, eligibleBytes: 3_216_380, items: [], warnings: [] },
  { id: 'crash-engine', ownership: 'platform', action: 'confirm', automatic: false, itemCount: 2, eligibleCount: 2, totalBytes: 19_200_000, eligibleBytes: 19_200_000, items: [], warnings: [] },
  { id: 'crash-webview', ownership: 'platform', action: 'confirm', automatic: false, itemCount: 1, eligibleCount: 1, totalBytes: 10_616_846, eligibleBytes: 10_616_846, items: [], warnings: [] },
  { id: 'webview-cache', ownership: 'platform', action: 'restart', automatic: false, itemCount: 3, eligibleCount: 3, totalBytes: 116_917_925, eligibleBytes: 116_917_925, items: [], warnings: [] },
]
const pendingBrowses: Array<{ repoId: string; resolve: (files: MsFileEntry[]) => void }> = []
const pendingPortChecks: Array<{ port: number; resolve: (available: boolean) => void }> = []
const pendingWorkerScans: Array<(workers: WorkerInfo[]) => void> = []
const clusterWorkers: WorkerInfo[] = BROWSER_SCENARIO === 'cluster-worker'
  ? [{
      id: 'browser-cluster-worker',
      host: '192.168.50.10',
      port: 50052,
      name: 'Browser Cluster Worker',
      origin: 'manual',
      devices: [{ device_type: 'Vulkan', name: 'Browser GPU', vram_mb: 16_384, free_mb: 12_288 }],
      status: 'Offline',
      auto_discovered: false,
    }]
  : []

const control: BrowserTestControl = {
  marker: BROWSER_TEST_MARKER,
  calls: [],
  unhandled: [],
  saveCount: 0,
  lastGenerated: null,
  failProxyStatus: false,
  failProxyTargets: false,
  failRuntimeStatus: false,
  updaterCheckCount: 0,
  activeProbeCount: 0,
  peakProbeCount: 0,
  state,
  emitEvent: (event, payload) => emit(event, payload),
  releaseBrowse: (repoId, files) => {
    const index = pendingBrowses.findIndex(item => item.repoId === repoId)
    if (index < 0) throw new Error(`No pending browser-test browse for ${repoId}`)
    const [{ resolve }] = pendingBrowses.splice(index, 1)
    resolve(clone(files))
  },
  releasePortCheck: (port, available) => {
    const index = pendingPortChecks.findIndex(item => item.port === port)
    if (index < 0) throw new Error(`No pending browser-test port check for ${port}`)
    const [{ resolve }] = pendingPortChecks.splice(index, 1)
    resolve(available)
  },
  releaseStart: () => releasePendingStart?.(),
  releaseWorkerScan: (workers) => {
    const resolve = pendingWorkerScans.shift()
    if (!resolve) throw new Error('No pending browser-test worker scan')
    resolve(clone(workers))
  },
}

const syncAutomationProbe = () => {
  const root = document.documentElement
  root.dataset.tauriBrowserTest = control.marker
  root.dataset.tauriMockCallCount = String(control.calls.length)
  root.dataset.tauriMockUnhandled = JSON.stringify(control.unhandled)
  root.dataset.tauriMockSaveCount = String(control.saveCount)
  root.dataset.tauriMockEmitted = JSON.stringify(control.lastGenerated?.emittedOverrideKeys ?? [])
}

const canonicalField = (field: string) => {
  if (field === 'gpu_layers_auto') return 'gpu_layers'
  if (field === 'ctx_size_auto') return 'ctx_size'
  if (field === 'kv_unified') return 'kv_unified_mode'
  if (field === 'fit') return 'fit_mode'
  if (field === 'numa') return 'numa_mode'
  if (field === 'mlock' || field === 'no_mmap' || field === 'direct_io') return 'load_mode'
  if (field === 'mmproj_auto' || field === 'no_mmproj') return 'mmproj_mode'
  return field
}

const emittedOverrideKeys = (config: InstanceConfig): Array<keyof InstanceConfig> => {
  const systemManaged = new Set([
    'model_path', 'host', 'port', 'api_key', 'api_key_file',
    'metrics', 'props', 'slots_enabled', 'embedding', 'pooling', 'reranking',
  ])
  const speculativeActive = Boolean(
    normalizeSpeculativeTypes(config.spec_type)
      && normalizeSpeculativeTypes(config.spec_type) !== 'none',
  )
  const speculativeChildren = new Set([
    'draft_tokens', 'spec_draft_n_min', 'spec_draft_p_min', 'spec_draft_p_split',
    'draft_gpu_layers', 'spec_draft_device', 'spec_default',
  ])
  const unique = new Set<string>()
  for (const rawField of config.explicit_overrides ?? []) {
    const field = canonicalField(rawField)
    if (systemManaged.has(field)) continue
    if (!speculativeActive && speculativeChildren.has(field)) continue
    unique.add(field)
  }
  return [...unique] as Array<keyof InstanceConfig>
}

const generatedCommand = (config: InstanceConfig): GeneratedServerCommand => {
  const emitted = emittedOverrideKeys(config)
  const command = [
    '-m', config.model_path,
    '--host', config.host,
    '--port', String(config.port),
    '--metrics', '--props', '--slots',
  ]
  for (const field of emitted) {
    if (field === 'alias') command.push('-a', config.alias)
    else if (field === 'gpu_layers' && !config.gpu_layers_auto) command.push('-ngl', String(config.gpu_layers))
    else if (field === 'ctx_size' && !config.ctx_size_auto) command.push('-c', String(config.ctx_size))
    else if (field === 'temp') command.push('--temp', String(config.temp))
    else if (field === 'top_k') command.push('--top-k', String(config.top_k))
    else if (field === 'top_p') command.push('--top-p', String(config.top_p))
    else if (field === 'threads') command.push('--threads', String(config.threads))
    else if (field === 'load_mode' && config.load_mode) command.push('--load-mode', config.load_mode)
    else if (field === 'lazy_mode' && config.lazy_mode) command.push('--lazy-mode', config.lazy_mode)
    else if (field === 'perf') command.push(config.perf ? '--perf' : '--no-perf')
    else if (field === 'kv_unified_mode') command.push(config.kv_unified_mode === 'off' ? '--no-kv-unified' : '--kv-unified')
    else if (field === 'models_autoload') command.push(config.models_autoload ? '--models-autoload' : '--no-models-autoload')
    else if (field === 'image_min_tokens') command.push('--image-min-tokens', String(config.image_min_tokens))
    else if (field === 'mmproj_path' && config.mmproj_path) command.push('--mmproj', config.mmproj_path)
    else if (field === 'spec_type') {
      const value = normalizeSpeculativeTypes(config.spec_type)
      if (value && value !== 'none') command.push('--spec-type', value)
    }
  }
  return { command, unsupportedFlags: [], emittedOverrideKeys: emitted }
}

const systemMetrics: SystemMetrics = {
  cpu_percent: HAS_MONITORING_DATA ? (IS_DOCS_SCENARIO ? 24 : 8) : 0,
  memory_mb: HAS_MONITORING_DATA ? (IS_DOCS_SCENARIO ? 11 * 1024 : 14 * 1024) : 128,
  uptime_secs: 30,
  gpu_percent: HAS_MONITORING_DATA ? (IS_DOCS_SCENARIO ? 68 : 12) : 0,
  vram_used_mb: HAS_MONITORING_DATA ? (IS_DOCS_SCENARIO ? 9_420 : 24 * 1024) : 256,
  vram_total_mb: HAS_MONITORING_DATA ? (IS_DOCS_SCENARIO ? 16_384 : 32 * 1024) : 8_192,
  system_cpu_percent: HAS_MONITORING_DATA ? (IS_DOCS_SCENARIO ? 31 : 9) : 0,
  system_memory_total_mb: IS_DOCS_SCENARIO ? 65_536 : 32_768,
  system_memory_used_mb: HAS_MONITORING_DATA ? (IS_DOCS_SCENARIO ? 27 * 1024 : 14 * 1024) : 8_192,
  gpu_vendor: IS_DOCS_SCENARIO ? 'NVIDIA' : 'Mock',
  gpu_name: IS_DOCS_SCENARIO ? 'NVIDIA GeForce RTX 5080' : 'Browser Test GPU',
}

const monitoringNow = Date.now()
const monitoringSessionId = 'browser-monitoring-session'
const stoppedMonitoringSessionId = 'browser-stopped-monitoring-session'
const monitoringFrame: MonitoringFrame = {
  instanceId: INSTANCE_ID,
  sessionId: monitoringSessionId,
  sessionStartedAt: monitoringNow - 120_000,
  ts: monitoringNow,
  workload: 'inference',
  state: 'active',
  throughput: IS_DOCS_SCENARIO ? 47.6 : 25.8,
  throughputUnit: 'tok/s',
  outputTokensPerSecond: IS_DOCS_SCENARIO ? 47.6 : 25.8,
  inputTokensPerSecond: 0,
  itemsPerSecond: null,
  activeRequests: 1,
  queuedRequests: 0,
  slotCapacity: 1,
  busySlots: 1,
  averageLatencyMs: null,
  successRate: null,
  source: 'task',
  dataAgeMs: 0,
  system: {
    ...systemMetrics,
    system_cpu_percent: IS_DOCS_SCENARIO ? 31 : 99,
    gpu_percent: IS_DOCS_SCENARIO ? 68 : 98,
    system_memory_used_mb: IS_DOCS_SCENARIO ? 27 * 1024 : 31 * 1024,
    vram_used_mb: IS_DOCS_SCENARIO ? 9_420 : 31 * 1024,
  },
}
const monitoringSessions = [
  {
    id: monitoringSessionId,
    instance_id: INSTANCE_ID,
    instance_name: IS_DOCS_SCENARIO ? 'Qwen3 8B Chat' : 'Browser Parameter Regression',
    model_name: IS_DOCS_SCENARIO ? 'Qwen3 8B Chat · Q4_K_M' : 'Qwen Browser Test Q8_0.gguf',
    model_path: IS_DOCS_SCENARIO ? DOCS_CHAT_MODEL_PATH : MODEL_PATH,
    config_hash: 'browser-active-config',
    engine_id: ENGINE_ID,
    backend: IS_DOCS_SCENARIO ? 'CUDA' : 'Vulkan',
    workload: 'inference',
    started_at: monitoringNow - 120_000,
    stopped_at: null,
    duration_secs: null,
    avg_tokens_per_second: IS_DOCS_SCENARIO ? 47.6 : 25.8,
    avg_tokens_per_sec: IS_DOCS_SCENARIO ? 47.6 : 25.8,
    peak_vram_mb: IS_DOCS_SCENARIO ? 9_420 : 31 * 1024,
    sample_count: 2,
    stop_reason: null,
  },
  {
    id: stoppedMonitoringSessionId,
    instance_id: STOPPED_INSTANCE_ID,
    instance_name: IS_DOCS_SCENARIO ? 'Qwen3 VL 7B' : 'Stopped Monitoring Instance',
    model_name: IS_DOCS_SCENARIO ? 'Qwen3 VL 7B · Q5_K_M' : 'Historical Browser Model',
    model_path: IS_DOCS_SCENARIO ? DOCS_VISION_MODEL_PATH : MODEL_PATH,
    config_hash: 'browser-stopped-config',
    engine_id: ENGINE_ID,
    backend: IS_DOCS_SCENARIO ? 'CUDA' : 'Vulkan',
    workload: 'inference',
    started_at: monitoringNow - 600_000,
    stopped_at: monitoringNow - 300_000,
    duration_secs: 300,
    avg_tokens_per_second: IS_DOCS_SCENARIO ? 32.4 : 18,
    avg_tokens_per_sec: IS_DOCS_SCENARIO ? 32.4 : 18,
    peak_vram_mb: IS_DOCS_SCENARIO ? 11_240 : 31 * 1024,
    sample_count: 1,
    stop_reason: 'stopped',
  },
]
const monitoringSamples = [
  {
    session_id: monitoringSessionId,
    instance_id: INSTANCE_ID,
    ts: monitoringNow - 5_000,
    cpu_percent: 99,
    memory_mb: 31 * 1024,
    gpu_percent: 98,
    vram_used_mb: 31 * 1024,
    vram_total_mb: 32 * 1024,
    system_cpu_percent: 99,
    system_memory_used_mb: 31 * 1024,
    system_memory_total_mb: 32 * 1024,
    gpu_vendor: 'Session Mock',
    gpu_name: 'Session-bound GPU',
    tokens_per_sec: 0,
    prompt_tokens_per_sec: 0,
    prompt_tokens_total: 0,
    generated_tokens_total: 0,
    requests_total: 0,
    decode_calls_total: 100,
    max_tokens_observed: 100,
    requests_processing: 1,
    requests_deferred: 0,
    busy_slots_per_decode: 1,
  },
]
const monitoringOverview = {
  active_sessions: 1,
  sessions_24h: 2,
  avg_tokens_per_sec_24h: IS_DOCS_SCENARIO ? 41.3 : 21.9,
  peak_vram_mb_24h: IS_DOCS_SCENARIO ? 11_240 : 31 * 1024,
  dropped_writes: 0,
  last_write_error: null,
  last_write_error_at: null,
  latest_samples: monitoringSamples,
}
const monitoringSeries = Array.from({ length: IS_DOCS_SCENARIO ? 18 : 1 }, (_, index) => {
  const offset = IS_DOCS_SCENARIO ? 17 - index : 0
  const throughput = IS_DOCS_SCENARIO ? 39.8 + (index % 6) * 1.7 : monitoringFrame.throughput
  return {
    ...clone(monitoringFrame),
    ts: monitoringNow - offset * 20_000,
    throughput,
    outputTokensPerSecond: throughput,
    system: {
      ...clone(monitoringFrame.system ?? systemMetrics),
      system_cpu_percent: IS_DOCS_SCENARIO ? 27 + (index % 5) : monitoringFrame.system?.system_cpu_percent ?? 0,
      gpu_percent: IS_DOCS_SCENARIO ? 61 + (index % 8) : monitoringFrame.system?.gpu_percent ?? 0,
    },
  }
})
const emptyMonitoringAnalysis = {
  request_count: 0,
  avg_prompt_tokens: 0,
  avg_generated_tokens: 0,
  avg_total_tokens: 0,
  avg_prompt_tps: 0,
  avg_generation_tps: 0,
  avg_total_time_ms: 0,
  max_total_tokens: 0,
  avg_busy_slots: 0,
  max_busy_slots: 0,
  avg_cached_slots: 0,
  max_context_tokens: 0,
  slot_sample_count: 0,
  speculative_analysis: null,
  vector_analysis: null,
  vector_baseline: null,
}

mockWindows('main')
mockConvertFileSrc('windows')
mockIPC((command, payload) => {
  control.calls.push({ command, payload: clone(payload ?? null), at: Date.now() })
  syncAutomationProbe()
  const args = (payload ?? {}) as Record<string, unknown>

  switch (command) {
    case 'plugin:app|bundle_type': return 'nsis'
    case 'plugin:updater|check':
      control.updaterCheckCount += 1
      if (BROWSER_SCENARIO === 'updater-retry' && control.updaterCheckCount === 1) {
        throw new Error('browser test updater endpoint temporarily unavailable')
      }
      if (['updater-retry', 'updater-install', 'updater-install-failure'].includes(BROWSER_SCENARIO ?? '')) {
        return {
          rid: 42,
          currentVersion: '2.9.36',
          version: '2.9.37',
          date: '2026-07-31T00:00:00Z',
          body: 'Browser updater test',
          rawJson: {},
        }
      }
      return null
    case 'plugin:updater|download_and_install':
      if (BROWSER_SCENARIO === 'updater-install-failure') {
        throw new Error('browser test updater download failed')
      }
      return null
    case 'plugin:resources|close': return null
    case 'plugin:process|restart': return null
    case 'plugin:dialog|message': {
      const buttons = args.buttons as {
        OkCancelCustom?: [string, string]
        OkCustom?: string
      } | undefined
      return buttons?.OkCancelCustom?.[0] ?? buttons?.OkCustom ?? 'Ok'
    }
    case 'get_startup_elapsed': return 1
    case 'get_cached_scan':
      if (BROWSER_SCENARIO === 'delayed-inventory-cache') {
        return new Promise((resolve) => {
          window.setTimeout(() => {
            delayedInventoryCacheLoaded = true
            resolve([clone(models), clone(engines)])
          }, 250)
        })
      }
      return [clone(models), clone(engines)]
    case 'load_config': return clone(control.state)
    case 'list_checkpoint_statuses': return {}
    case 'get_checkpoint_status': return null
    case 'get_checkpoint_eligibility': {
      const config = args.config as InstanceConfig
      const reasons: string[] = []
      if (config.parallel !== 1) reasons.push('parallel_must_be_one')
      if (!config.cache_prompt) reasons.push('prompt_cache_required')
      if (!config.cache_idle_slots || (config.cache_ram !== -1 && config.cache_ram <= 0)) {
        reasons.push('prompt_cache_retention_required')
      }
      if (!config.slots_enabled) reasons.push('slots_required')
      if (BROWSER_SCENARIO === 'checkpoint-requirements' && !config.swa_full) {
        reasons.push('sliding_window_requires_full_cache')
      }
      return {
        eligible: reasons.length === 0,
        reason_code: reasons[0] ?? 'none',
        reasons,
      }
    }
    case 'clear_checkpoint':
      return {
        instance_id: String(args.instanceId ?? ''),
        phase: 'stopped',
        routable: false,
        last_operation: 'clear',
        last_outcome: 'success',
        reason_code: 'none',
        message: '',
        updated_at: Date.now(),
      }
    case 'scan_models':
      if (BROWSER_SCENARIO === 'delayed-inventory-cache') {
        return new Promise((resolve) => window.setTimeout(() => resolve(clone(models)), 3_000))
      }
      return clone(models)
    case 'browse_modelscope':
    case 'browse_huggingface': {
      const repoId = String(args.repoId ?? '')
      if (BROWSER_SCENARIO === 'download-browse-race') {
        return new Promise<MsFileEntry[]>((resolve) => pendingBrowses.push({ repoId, resolve }))
      }
      return []
    }
    case 'check_local_file': return null
    case 'check_local_files': return Array.isArray(args?.paths) ? args.paths.map(() => null) : []
    case 'get_models':
      return BROWSER_SCENARIO === 'delayed-inventory-cache' && !delayedInventoryCacheLoaded
        ? []
        : clone(models)
    case 'preview_model_deletion': {
      const logicalPath = String(args.path ?? '')
      const artifactPaths = logicalPath === MODEL_PATH
        ? [
            MODEL_PATH,
            'C:\\browser-test\\models\\Qwen-Browser-Test-Q8_0-00002-of-00003.gguf',
            'C:\\browser-test\\models\\Qwen-Browser-Test-Q8_0-00003-of-00003.gguf',
          ]
        : [logicalPath]
      return {
        logicalPath,
        artifactPaths,
        artifactCount: artifactPaths.length,
        totalBytes: logicalPath === MODEL_PATH ? 12_884_901_888 : 536_870_912,
        isSharded: artifactPaths.length > 1,
        referencedBy: [],
      }
    }
    case 'delete_model_file': {
      const logicalPath = String(args.path ?? '')
      const artifactPaths = logicalPath === MODEL_PATH
        ? [
            MODEL_PATH,
            'C:\\browser-test\\models\\Qwen-Browser-Test-Q8_0-00002-of-00003.gguf',
            'C:\\browser-test\\models\\Qwen-Browser-Test-Q8_0-00003-of-00003.gguf',
          ]
        : [logicalPath]
      for (let index = models.length - 1; index >= 0; index -= 1) {
        if (artifactPaths.includes(models[index].path)) models.splice(index, 1)
      }
      return {
        artifactPaths,
        artifactCount: artifactPaths.length,
        removedBytes: logicalPath === MODEL_PATH ? 12_884_901_888 : 536_870_912,
      }
    }
    case 'scan_engines':
      if (BROWSER_SCENARIO === 'engine-scan-error') {
        throw new Error('engine directory unavailable')
      }
      if (BROWSER_SCENARIO === 'delayed-inventory-cache') {
        return new Promise((resolve) => window.setTimeout(() => resolve(clone(engines)), 3_000))
      }
      return clone(engines)
    case 'get_engines':
      return BROWSER_SCENARIO === 'delayed-inventory-cache' && !delayedInventoryCacheLoaded
        ? []
        : clone(engines)
    case 'rename_engine': {
      const engineId = String(args.id ?? '')
      const name = String(args.name ?? '')
      const target = engines.find(candidate => candidate.id === engineId)
      if (!target) throw new Error(`browser test engine not found: ${engineId}`)
      target.name = name
      target.custom_name = name
      return null
    }
    case 'probe_engine_capabilities': {
      const engineId = String(args.engineId ?? '')
      const target = engines.find(candidate => candidate.id === engineId)
      if (!target) throw new Error(`browser test engine not found: ${engineId}`)
      if (!BROWSER_SCENARIO?.startsWith('engine-probe-batch')) return clone(target)

      control.activeProbeCount += 1
      control.peakProbeCount = Math.max(control.peakProbeCount, control.activeProbeCount)
      return new Promise<EngineInfo>((resolve, reject) => {
        window.setTimeout(() => {
          control.activeProbeCount -= 1
          if (BROWSER_SCENARIO === 'engine-probe-batch-failure' && engineId === VULKAN_ENGINE_ID) {
            reject(new Error('browser test capability probe failed'))
            return
          }
          resolve(clone(target))
        }, 100)
      })
    }
    case 'get_download_manager_snapshot':
      if (IS_DOCS_SCENARIO) {
        return {
          queue: [
            {
              id: 'docs-download-active',
              repo_id: 'Qwen/Qwen3-14B-GGUF',
              source: 'huggingface',
              files: [{
                name: 'Qwen3-14B-Q4_K_M.gguf',
                path: 'Qwen3-14B-Q4_K_M.gguf',
                size: 9_124_839_936,
                file_type: 'model',
                task_id: 'docs-download-task-active',
                run_id: 'docs-download-run-active',
                downloaded: 5_368_709_120,
                version: 3,
                status: 'active',
              }],
              save_dir: DOCS_MODEL_ROOT,
              added_at: Date.now() - 420_000,
              status: 'active',
            },
            {
              id: 'docs-download-paused',
              repo_id: 'Qwen/Qwen3-VL-30B-A3B-GGUF',
              source: 'modelscope',
              files: [{
                name: 'Qwen3-VL-30B-A3B-Q4_K_M.gguf',
                path: 'Qwen3-VL-30B-A3B-Q4_K_M.gguf',
                size: 19_327_352_832,
                file_type: 'model',
                task_id: 'docs-download-task-paused',
                run_id: 'docs-download-run-paused',
                downloaded: 4_294_967_296,
                version: 2,
                status: 'paused',
              }],
              save_dir: DOCS_MODEL_ROOT,
              added_at: Date.now() - 1_200_000,
              status: 'paused',
            },
          ],
          active_count: 1,
          max_concurrent: 3,
          resume_policy: 'auto_on_launch',
          bandwidth_limit_bytes_per_sec: 0,
          low_priority_throttle: false,
        }
      }
      if (BROWSER_SCENARIO === 'download-resume') {
        return {
          queue: [{
            id: 'browser-download-entry',
            repo_id: 'browser/model',
            source: 'huggingface',
            files: [{
              name: 'browser-model.gguf',
              path: 'browser-model.gguf',
              size: 1_024,
              file_type: 'model',
              task_id: 'browser-download-task',
              run_id: 'browser-download-run',
              downloaded: 512,
              version: 1,
              status: 'paused',
            }],
            save_dir: 'C:\\browser-test\\downloads',
            added_at: Date.now(),
            status: 'paused',
          }],
          active_count: 0,
          max_concurrent: 3,
          resume_policy: 'manual',
          bandwidth_limit_bytes_per_sec: 0,
          low_priority_throttle: false,
        }
      }
      return { queue: [], active_count: 0, max_concurrent: 3, resume_policy: 'manual', bandwidth_limit_bytes_per_sec: 0, low_priority_throttle: false }
    case 'get_storage_maintenance_inventory':
      return {
        generatedAt: Date.now(),
        appDataRoot: 'C:\\browser-test\\app-data',
        tempRoot: 'C:\\browser-test\\temp',
        webviewRoot: 'C:\\browser-test\\webview',
        scheduledWebviewCleanup: storageWebviewScheduled,
        runningInstanceCount: 0,
        groups: clone(storageGroups),
        authorizedDirectories: clone(storageAuthorizedDirectories),
        externalArtifacts: {
          references: [{ instanceId: INSTANCE_ID, instanceName: 'Browser Test Instance', source: 'custom-argument', flag: '--slot-save-path', artifactKind: 'slot-state', ownership: 'operator', value: 'C:\\operator\\slots', locationKind: 'absolute-existing', exists: true, sizeBytes: 2048 }],
          warnings: [],
        },
        telemetry: { databaseBytes: 13_762_560, walBytes: 4_173_592, sharedMemoryBytes: 32_768, totalBytes: 17_968_920 },
      }
    case 'cleanup_storage_group': {
      const groupId = String(args.groupId ?? '')
      const group = storageGroups.find(candidate => candidate.id === groupId)
      if (!group) throw new Error(`unknown browser storage group: ${groupId}`)
      if (groupId === 'updater-staging') {
        group.eligibleCount = 0
        group.eligibleBytes = 0
        return { groupId, removedItems: 0, removedBytes: 0, skippedItems: 1, failures: ['Refusing linked updater staging directory'] }
      }
      const report = { groupId, removedItems: group.eligibleCount, removedBytes: group.eligibleBytes, skippedItems: group.itemCount - group.eligibleCount, failures: [] }
      group.itemCount -= group.eligibleCount
      group.totalBytes -= group.eligibleBytes
      group.eligibleCount = 0
      group.eligibleBytes = 0
      group.items = []
      return report
    }
    case 'schedule_webview_cache_cleanup':
      storageWebviewScheduled = Boolean(args.enabled)
      return storageWebviewScheduled
    case 'optimize_telemetry_storage':
      return { before: { totalBytes: 17_968_920 }, after: { totalBytes: 13_762_560 }, reclaimedBytes: 4_206_360 }
    case 'revoke_authorized_directory': {
      const purpose = String(args.purpose ?? '')
      const root = String(args.root ?? '')
      const index = storageAuthorizedDirectories.findIndex(candidate => candidate.purpose === purpose && candidate.root === root)
      if (index >= 0) storageAuthorizedDirectories.splice(index, 1)
      return index >= 0
    }
    case 'restore_download_queue': return []
    case 'get_monitoring_series':
      return HAS_MONITORING_DATA ? clone(monitoringSeries) : []
    case 'get_system_health': return clone(systemMetrics)
    case 'get_telemetry_overview':
      return HAS_MONITORING_DATA
        ? clone(monitoringOverview)
        : {
            active_sessions: 0,
            sessions_24h: 0,
            avg_tokens_per_sec_24h: 0,
            peak_vram_mb_24h: 0,
            dropped_writes: 0,
            last_write_error: null,
            last_write_error_at: null,
            latest_samples: [],
          }
    case 'list_telemetry_sessions':
      return HAS_MONITORING_DATA ? clone(monitoringSessions) : []
    case 'get_telemetry_session_detail':
      return {
        samples: HAS_MONITORING_DATA ? clone(monitoringSamples) : [],
        requests: [],
        analysis: clone(emptyMonitoringAnalysis),
        diagnostics: [],
      }
    case 'list_inference_requests': return []
    case 'scan_workers_tcp':
      if (BROWSER_SCENARIO === 'cluster-scan-race') {
        return new Promise<WorkerInfo[]>((resolve) => pendingWorkerScans.push(resolve))
      }
      return clone(clusterWorkers)
    case 'add_worker': {
      const host = String(args.host ?? '')
      const port = Number(args.port ?? 50052)
      const name = String(args.name ?? '') || host
      const existing = clusterWorkers.find(worker => worker.host === host && worker.port === port)
      if (!existing) {
        clusterWorkers.push({
          id: `browser-worker-${clusterWorkers.length + 1}`,
          host,
          port,
          name,
          origin: 'manual',
          devices: [],
          status: 'Offline',
          auto_discovered: false,
        })
      }
      return null
    }
    case 'get_workers':
      return IS_DOCS_SCENARIO
        ? [
            {
              id: 'docs-local-worker',
              host: '127.0.0.1',
              port: 50052,
              name: 'Local GPU Worker',
              origin: 'local',
              devices: [{ device_type: 'CUDA', name: 'NVIDIA GeForce RTX 5080', vram_mb: 16_384, free_mb: 12_288 }],
              status: 'Online',
              last_seen: new Date().toISOString(),
              auto_discovered: true,
            },
            {
              id: 'docs-nas-worker',
              host: '192.168.1.20',
              port: 50052,
              name: 'NAS Compute Node',
              origin: 'manual',
              devices: [{ device_type: 'CPU', name: 'AMD Ryzen Embedded', vram_mb: 0, free_mb: 0 }],
              status: 'Offline',
              auto_discovered: false,
            },
          ]
        : clone(clusterWorkers)
    case 'is_local_host': return false
    case 'test_worker': return { ok: true, latency_ms: 12, devices: [] }
    case 'check_port':
      if (BROWSER_SCENARIO === 'port-check-race') {
        const port = Number(args.port ?? 0)
        return new Promise<boolean>((resolve) => pendingPortChecks.push({ port, resolve }))
      }
      return true
    case 'test_connection': return 'HTTP 200'
    case 'process_download_queue': return null
    case 'get_proxy_config': return clone(proxyConfig)
    case 'get_proxy_status':
      if (control.failProxyStatus) throw new Error('browser test proxy status unavailable')
      return clone(proxyStatus)
    case 'list_proxy_targets':
      if (control.failProxyTargets) throw new Error('browser test proxy target status unavailable')
      return clone(proxyTargets)
    case 'test_proxy_route': {
      const modelAlias = String(args.model ?? '').trim()
      const candidates = proxyConfig.routes
        .filter(route => route.enabled && route.model_alias.trim() === modelAlias)
        .sort((left, right) => left.priority - right.priority)
      for (const route of candidates) {
        const target = proxyTargets.find(candidate => candidate.instance_id === route.target_instance_id)
        if (target?.running) return clone(target)
      }
      throw new Error('no running instance matches the requested model')
    }
    case 'save_proxy_config': {
      const next = clone(args.config as BrowserProxyConfig)
      next.api_keys = next.api_keys.map(apiKey => ({
        ...apiKey,
        key: apiKey.key.startsWith('sha256:') ? apiKey.key : `sha256:${'a'.repeat(43)}`,
      }))
      Object.assign(proxyConfig, next)
      proxyConfig.routes = next.routes
      const enabledRoutes = next.routes.filter(route => route.enabled)
      proxyStatus.active_routes = enabledRoutes.length || proxyTargets.filter(target => target.running).length
      proxyStatus.healthy_routes = enabledRoutes.length
        ? enabledRoutes.filter(route => proxyTargets.some(target => target.instance_id === route.target_instance_id && target.running)).length
        : proxyTargets.filter(target => target.running).length
      proxyStatus.unhealthy_routes = Math.max(0, proxyStatus.active_routes - proxyStatus.healthy_routes)
      return clone(proxyConfig)
    }
    case 'start_proxy':
      proxyConfig.enabled = true
      proxyStatus.running = true
      return clone(proxyStatus)
    case 'stop_proxy':
      proxyConfig.enabled = false
      proxyStatus.running = false
      return clone(proxyStatus)
    case 'get_runtime_service_status':
      if (control.failRuntimeStatus) throw new Error('browser test runtime status unavailable')
      return clone(runtimeStatus)
    case 'clear_runtime_service_error':
      runtimeStatus.lastError = null
      return null
    case 'is_autostart_enabled': return false
    case 'is_mdns_discovery_active': return mdnsDiscoveryActive
    case 'start_mdns_discovery':
      mdnsDiscoveryActive = true
      return 'started'
    case 'stop_mdns_discovery':
      mdnsDiscoveryActive = false
      return 'stopped'
    case 'resolve_path': return args.path === 'models' ? 'C:\\browser-test\\models' : String(args.path ?? '')
    case 'generate_server_command': {
      if (BROWSER_SCENARIO === 'command-error') throw new Error('Browser test command generation failed')
      const config = args.config as InstanceConfig
      const generated = generatedCommand(config)
      control.lastGenerated = clone(generated)
      syncAutomationProbe()
      return generated
    }
    case 'save_config': {
      const instances = clone(args.instances as Record<string, InstanceConfig>)
      control.state.instances = instances
      if (Array.isArray(args.modelDirs)) control.state.model_dirs = clone(args.modelDirs as string[])
      control.saveCount += 1
      syncAutomationProbe()
      return instances
    }
    case 'start_server': {
      const instanceId = String(args.instanceId ?? '')
      const config = args.config as InstanceConfig
      const completeStart = () => {
        control.state.running[instanceId] = {
          instance_id: instanceId,
          pid: 5000 + Object.keys(control.state.running).length,
          port: config.port,
          host: config.host,
          start_time: Math.floor(Date.now() / 1000),
        }
      }
      if (BROWSER_SCENARIO === 'delayed-instance-start') {
        return new Promise(resolve => {
          releasePendingStart = () => {
            releasePendingStart = null
            completeStart()
            resolve(null)
          }
        })
      }
      completeStart()
      return null
    }
    case 'get_download_resume_policy': return IS_DOCS_SCENARIO ? 'auto_on_launch' : 'manual'
    case 'get_download_concurrency': return 3
    case 'get_download_bandwidth_limit': return 0
    case 'get_download_low_priority_throttle': return false
    case 'resume_download_task': return {
      taskId: String(args.taskId ?? ''),
      runId: 'browser-download-resumed-run',
      version: 2,
    }
    case 'enable_autostart':
    case 'disable_autostart':
    case 'show_window':
    case 'hide_window':
    case 'open_browser': return null
    case 'enable_background_and_quit':
    case 'quit_keep_runtime':
      if (BROWSER_SCENARIO === 'background-detach-error') {
        throw new Error('Browser test background handoff failed')
      }
      return null
    case 'quit_app': return null
    case 'stop_background_runtime':
      proxyConfig.enabled = false
      proxyConfig.runtime_service_enabled = false
      proxyStatus.running = false
      runtimeStatus.backgroundEnabled = false
      runtimeStatus.registeredForLogin = false
      runtimeStatus.running = {}
      return null
    default:
      control.unhandled.push(command)
      syncAutomationProbe()
      throw new Error(`Unhandled browser-test Tauri command: ${command}`)
  }
}, { shouldMockEvents: true })

window.__TAURI_BROWSER_TEST__ = control
window.__INITIAL_CONFIG__ = clone(state)
syncAutomationProbe()
