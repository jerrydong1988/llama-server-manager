import { mockConvertFileSrc, mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { emit } from '@tauri-apps/api/event'
import { defaultInstanceConfig } from '../src/store/defaults'
import type { GlobalConfigShape } from '../src/store/bootstrap'
import type {
  ConfigRevisionHistory,
  ConfigRevisionRollbackResponse,
  EngineInfo,
  GeneratedServerCommand,
  InstanceConfig,
  MsFileEntry,
  ModelInfo,
  MonitoringFrame,
  ResourcePlan,
  ResidencyAuditEvent,
  ResidencyInspection,
  ResidencyPolicy,
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
  'canary-rollout',
  'operational-metrics',
  'docs-screenshots',
].includes(BROWSER_SCENARIO ?? '')
const INSTANCE_ID = 'browser-test-instance'
const STOPPED_INSTANCE_ID = 'browser-stopped-instance'
const EMBEDDING_INSTANCE_ID = 'browser-embedding-instance'
const ENGINE_ID = 'browser-test-engine'
const VULKAN_ENGINE_ID = 'browser-vulkan-engine'
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
      '--model', '--host', '--port', '--temp', '--top-k', '--top-p', '--threads', '--kv-unified',
      '--mmap', '--no-mmap', '--perf', '--no-perf',
      '--models-autoload', '--no-models-autoload', '--image-min-tokens', '--mmproj',
    ],
    reportedDefaults: {
      '--temp': '0.8',
      '--threads': 'automatic',
      '--mmap': 'enabled',
    },
    reportedDefaultsVersion: 1,
    helpHash: 'browser-test-help',
    executableFingerprint: 'browser-test-engine-fingerprint',
    probedAt: IS_DOCS_SCENARIO ? Date.UTC(2026, 6, 31, 12, 0, 0) / 1000 : 1,
    qualification: BROWSER_SCENARIO === 'engine-qualification'
      ? {
          schemaVersion: 2,
          profileVersion: 1,
          status: 'unqualified',
          executableFingerprint: '',
          engineArtifactId: '',
          engineVersion: '',
          helpHash: '',
          modelId: '',
          modelArtifactId: '',
          modelName: '',
          modelSize: 0,
          checks: [],
          evidenceId: '',
        }
      : {
          schemaVersion: 2,
          profileVersion: 1,
          status: 'passed',
          executableFingerprint: 'browser-test-engine-fingerprint',
          engineArtifactId: 'urn:lsm:engine:v1:sha256:browser-engine',
          engineVersion: IS_DOCS_SCENARIO ? 'version: 10042 (6d2f8e1)' : 'version: 10042 (browser-test)',
          helpHash: 'browser-test-help',
          modelId: models[0].id,
          modelArtifactId: 'urn:lsm:model:v1:sha256:browser-model',
          modelName: models[0].name,
          modelSize: models[0].size,
          startedAt: 1,
          completedAt: 2,
          evidenceId: 'urn:lsm:qualification:v2:sha256:browser-evidence',
          checks: [
            { name: 'version', status: 'passed', durationMs: 10 },
            { name: 'capabilities', status: 'passed', durationMs: 10 },
            { name: 'startup', status: 'passed', durationMs: 20 },
            { name: 'health', status: 'passed', durationMs: 30 },
            { name: 'inference', status: 'passed', durationMs: 40 },
          ],
        },
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

const engines = IS_DOCS_SCENARIO ? [engine, vulkanEngine] : [engine]

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
  active_routes: IS_DOCS_SCENARIO ? 3 : BROWSER_SCENARIO === 'canary-rollout' ? 2 : ['proxy-route-health', 'proxy-route-legacy-ids'].includes(BROWSER_SCENARIO ?? '') ? 2 : ['proxy-routing', 'operational-metrics'].includes(BROWSER_SCENARIO ?? '') ? 1 : 0,
  healthy_routes: BROWSER_SCENARIO === 'canary-rollout' ? 2 : HAS_PROXY_DATA ? 1 : 0,
  unhealthy_routes: IS_DOCS_SCENARIO ? 2 : ['proxy-route-health', 'proxy-route-legacy-ids'].includes(BROWSER_SCENARIO ?? '') ? 1 : 0,
  in_flight_requests: 0,
  total_requests: IS_DOCS_SCENARIO ? 42 : 0,
  operational: BROWSER_SCENARIO === 'operational-metrics'
    ? {
        window_seconds: 300,
        request_count: 20,
        failed_request_count: 3,
        error_rate_percent: 15,
        queue_depth: 2,
        queued_requests_total: 8,
        queue_timeouts_total: 1,
        queue_wait_p95_ms: 420,
        ttft_sample_count: 18,
        ttft_p50_ms: 620,
        ttft_p95_ms: 3500,
        prompt_tokens_observed: 1000,
        cached_prompt_tokens: 400,
        cache_reuse_percent: 40,
        in_flight_requests: 58,
        max_concurrent_requests: 64,
        saturation_percent: 90.625,
        alerts: [
          { id: 'error_rate', severity: 'warning', observed: 15, threshold: 10 },
          { id: 'ttft_p95', severity: 'warning', observed: 3500, threshold: 3000 },
          { id: 'queue_wait_p95', severity: 'warning', observed: 420, threshold: 250 },
          { id: 'queue_timeouts', severity: 'warning', observed: 1, threshold: 1 },
          { id: 'saturation', severity: 'warning', observed: 90.625, threshold: 85 },
        ],
      }
    : {
        window_seconds: 300,
        request_count: 0,
        failed_request_count: 0,
        error_rate_percent: null,
        queue_depth: 0,
        queued_requests_total: 0,
        queue_timeouts_total: 0,
        queue_wait_p95_ms: null,
        ttft_sample_count: 0,
        ttft_p50_ms: null,
        ttft_p95_ms: null,
        prompt_tokens_observed: 0,
        cached_prompt_tokens: 0,
        cache_reuse_percent: null,
        in_flight_requests: 0,
        max_concurrent_requests: 64,
        saturation_percent: 0,
        alerts: [],
      },
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
  : BROWSER_SCENARIO === 'canary-rollout'
  ? [runningProxyTarget, {
      instance_id: STOPPED_INSTANCE_ID,
      name: 'Candidate Engine Revision',
      alias: 'browser-candidate',
      host: '127.0.0.1',
      port: 18082,
      running: true,
    }]
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

type BrowserCanaryRollout = Record<string, unknown> & {
  id: string
  state: 'active' | 'promoted' | 'aborted' | 'rolled_back'
  candidateWeight: number
  updatedAt: number
  events: Array<Record<string, unknown>>
}
let canaryRollouts: BrowserCanaryRollout[] = []
let canaryObservationRound = 0

function canaryHealth(instanceId: string) {
  return { instanceId, status: 'ready', ready: true }
}

function canaryEvidence(total: number, failed: number) {
  return {
    total,
    succeeded: total - failed,
    failed,
    latestCompletedAt: Date.now(),
    ttftP95Ms: 1200,
    queueWaitP95Ms: 80,
    cacheReuseBasisPoints: 3750,
  }
}

function appendCanaryEvent(rollout: BrowserCanaryRollout, kind: string, summary: string) {
  rollout.updatedAt = Date.now()
  rollout.events.unshift({
    sequence: rollout.events.length + 1,
    occurredAt: rollout.updatedAt,
    kind,
    summary,
    stableEvidence: rollout.stableEvidence ?? null,
    candidateEvidence: rollout.candidateEvidence ?? null,
    integrityValid: true,
  })
}
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

const revisionHistories = new Map<string, ConfigRevisionHistory>()
const revisionSnapshots = new Map<string, InstanceConfig>()

const ensureRevisionHistory = (instanceId: string) => {
  const existing = revisionHistories.get(instanceId)
  if (existing) return existing
  const currentConfig = state.instances[instanceId]
  if (!currentConfig) throw new Error(`browser test revision instance not found: ${instanceId}`)
  const baselineConfig = {
    ...clone(currentConfig),
    port: Math.max(1, currentConfig.port - 1),
    temp: 0.5,
    api_key: 'historical-browser-secret',
    custom_args: ['--historical-secret', 'must-not-render'],
  }
  const baselineId = `revision-baseline-${instanceId}`
  const currentId = `revision-current-${instanceId}`
  revisionSnapshots.set(baselineId, baselineConfig)
  revisionSnapshots.set(currentId, clone(currentConfig))
  const history: ConfigRevisionHistory = {
    instanceId,
    currentFingerprint: `sha256:current-${instanceId}`,
    currentRevisionId: currentId,
    currentConfigurationId: `urn:lsm:configuration:v1:sha256:current-${instanceId}`,
    knownGoodRevisionId: null,
    revisions: [
      {
        id: currentId,
        fingerprint: `sha256:current-${instanceId}`,
        identitySchemaVersion: 1,
        configurationId: `urn:lsm:configuration:v1:sha256:current-${instanceId}`,
        parentRevisionId: baselineId,
        createdAt: 1_787_000_100,
        reason: 'save',
        rollbackOf: null,
        current: true,
        knownGood: false,
        integrityValid: true,
        diffTruncated: false,
        changes: [
          {
            field: 'port',
            before: { state: 'value', value: String(baselineConfig.port) },
            after: { state: 'value', value: String(currentConfig.port) },
            redacted: false,
          },
          {
            field: 'api_key',
            before: { state: 'set' },
            after: { state: currentConfig.api_key ? 'set' : 'empty' },
            redacted: true,
          },
          {
            field: 'custom_args',
            before: { state: 'item_count', itemCount: 2 },
            after: { state: 'item_count', itemCount: currentConfig.custom_args.length },
            redacted: true,
          },
        ],
      },
      {
        id: baselineId,
        fingerprint: `sha256:baseline-${instanceId}`,
        identitySchemaVersion: 1,
        configurationId: `urn:lsm:configuration:v1:sha256:baseline-${instanceId}`,
        parentRevisionId: null,
        createdAt: 1_787_000_000,
        reason: 'migration',
        rollbackOf: null,
        current: false,
        knownGood: false,
        integrityValid: true,
        diffTruncated: false,
        changes: [],
      },
    ],
    audit: [],
  }
  revisionHistories.set(instanceId, history)
  return history
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
let residencyPolicy: ResidencyPolicy = {
  enabled: false,
  ramBudgetBytes: 0,
  vramBudgetBytes: 0,
  drainTimeoutSeconds: 120,
  intents: [],
}
const residencyAudit: ResidencyAuditEvent[] = []

const residencyInspection = (): ResidencyInspection => {
  const decisions = residencyPolicy.intents.map(intent => {
    const running = Boolean(state.running[intent.instanceId])
    return {
      instanceId: intent.instanceId,
      instanceName: state.instances[intent.instanceId]?.name || intent.instanceId,
      priority: intent.priority,
      intentEnabled: intent.enabled,
      selected: residencyPolicy.enabled && intent.enabled,
      deploymentId: `urn:lsm:managed-deployment:${intent.instanceId}`,
      revisionId: `urn:lsm:deployment-revision:${intent.instanceId}`,
      runningRevisionId: running ? `urn:lsm:deployment-revision:${intent.instanceId}` : null,
      resourceStatus: 'feasible',
      ramBytes: 2 * 1024 ** 3,
      vramBytes: 0,
      reasons: residencyPolicy.enabled && intent.enabled ? ['selected_within_budget'] : ['intent_disabled'],
    }
  })
  const operations = decisions
    .filter(decision => decision.selected && !decision.runningRevisionId)
    .map((decision, index) => ({
      sequence: index + 1,
      kind: 'warm' as const,
      instanceId: decision.instanceId,
      deploymentId: decision.deploymentId || '',
      revisionId: decision.revisionId || '',
      reason: 'selected_revision_not_running',
    }))
  return {
    policy: clone(residencyPolicy),
    plan: {
      schemaVersion: 1,
      planId: 'sha256:browser-residency-plan',
      generatedAt: Math.floor(Date.now() / 1000),
      ramBudgetBytes: residencyPolicy.ramBudgetBytes,
      ramUsedBytes: decisions.filter(decision => decision.selected).length * 2 * 1024 ** 3,
      vramBudgetBytes: residencyPolicy.vramBudgetBytes,
      vramUsedBytes: 0,
      decisions,
      operations,
    },
    placements: decisions
      .filter(decision => decision.selected && decision.runningRevisionId)
      .map(decision => ({
        instanceId: decision.instanceId,
        deploymentId: decision.deploymentId || '',
        revisionId: decision.revisionId || '',
        phase: 'resident' as const,
        planId: 'sha256:browser-residency-plan',
        updatedAt: Math.floor(Date.now() / 1000),
        routingDrained: false,
      })),
    audit: clone(residencyAudit).reverse(),
    registeredRpcWorkers: clusterWorkers.filter(worker => !worker.auto_discovered).length,
    workerAgentAvailable: false,
  }
}

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
  const speculativeActive = Boolean(config.spec_type && config.spec_type !== 'none')
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
    else if (field === 'perf') command.push(config.perf ? '--perf' : '--no-perf')
    else if (field === 'kv_unified_mode') command.push(config.kv_unified_mode === 'off' ? '--no-kv-unified' : '--kv-unified')
    else if (field === 'models_autoload') command.push(config.models_autoload ? '--models-autoload' : '--no-models-autoload')
    else if (field === 'image_min_tokens') command.push('--image-min-tokens', String(config.image_min_tokens))
    else if (field === 'mmproj_path' && config.mmproj_path) command.push('--mmproj', config.mmproj_path)
  }
  return { command, unsupportedFlags: [], emittedOverrideKeys: emitted }
}

const resourcePlan = (): ResourcePlan => {
  const status = BROWSER_SCENARIO === 'resource-plan-infeasible' ? 'infeasible' : 'feasible'
  const range = { minBytes: 5_368_709_120, expectedBytes: 6_442_450_944, maxBytes: 7_516_192_768 }
  const vramRange = { minBytes: 3_221_225_472, expectedBytes: 4_294_967_296, maxBytes: 5_368_709_120 }
  return {
    schemaVersion: 1,
    status,
    confidence: 'medium',
    ram: {
      required: range,
      totalBytes: 34_359_738_368,
      availableBytes: status === 'infeasible' ? 1_073_741_824 : 25_769_803_776,
      reservedBytes: 1_717_986_918,
      expectedHeadroomBytes: status === 'infeasible' ? -7_086_695_936 : 17_609_365_914,
    },
    vram: {
      required: vramRange,
      totalBytes: 8_589_934_592,
      availableBytes: status === 'infeasible' ? 536_870_912 : 8_321_499_136,
      reservedBytes: 536_870_912,
      expectedHeadroomBytes: status === 'infeasible' ? -4_294_967_296 : 3_489_660_928,
    },
    components: [],
    facts: {
      contextTokens: 131_072,
      parallelSlots: 1,
      modelShardsFound: 1,
      modelShardsExpected: 1,
      gpuOffloadPercent: 72,
    },
    reasons: status === 'infeasible' ? ['insufficient_available_ram', 'insufficient_available_vram'] : ['llama_fit_may_reduce_unset_parameters'],
    assumptions: ['automatic_gpu_layers_follow_current_free_vram', 'prompt_cache_is_demand_driven_up_to_configured_limit'],
  }
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
    case 'list_config_revisions': {
      const instanceId = String(args.instanceId ?? '')
      return clone(ensureRevisionHistory(instanceId))
    }
    case 'inspect_deployment_identity': {
      const instanceId = String(args.instanceId ?? '')
      if (BROWSER_SCENARIO === 'deployment-identity-incomplete') {
        return {
          ready: false,
          errorCode: 'ENGINE_QUALIFICATION_INCOMPLETE',
          message: 'browser mock detail must not be required for localized rendering',
        }
      }
      if (BROWSER_SCENARIO === 'deployment-identity-stale') {
        return {
          ready: false,
          errorCode: 'DEPLOYMENT_MODEL_IDENTITY_STALE',
          message: 'stale browser mock detail must not be rendered',
        }
      }
      if (BROWSER_SCENARIO === 'deployment-identity-legacy') {
        return {
          ready: false,
          errorCode: 'DEPLOYMENT_IDENTITY_INVALID',
          message: 'legacy browser mock detail must not be rendered',
        }
      }
      return {
        ready: true,
        identity: {
          schemaVersion: 1,
          deploymentId: `urn:lsm:deployment:v1:sha256:browser-${instanceId}`,
          engineArtifactId: 'urn:lsm:engine:v1:sha256:browser-engine',
          modelArtifactId: 'urn:lsm:model:v1:sha256:browser-model',
          configRevisionId: `revision-current-${instanceId}`,
          configurationId: `urn:lsm:configuration:v1:sha256:current-${instanceId}`,
          qualificationEvidenceId: 'urn:lsm:qualification:v2:sha256:browser-evidence',
        },
      }
    }
    case 'inspect_deployment': {
      const instanceId = String(args.instanceId ?? '')
      const deploymentId = `urn:lsm:managed-deployment:v1:sha256:browser-${instanceId}`
      if (BROWSER_SCENARIO === 'deployment-unmaterialized') {
        return {
          instanceId,
          deploymentId,
          state: 'unmaterialized',
          message: 'browser mock unmaterialized detail',
          currentRevisionId: null,
          rollbackTargetRevisionId: null,
          runningRevisionId: null,
          revisions: [],
        }
      }
      const state = BROWSER_SCENARIO === 'deployment-stale'
        ? 'stale'
        : BROWSER_SCENARIO === 'deployment-invalid' ? 'invalid' : 'ready'
      const revisionId = `urn:lsm:deployment-revision:v1:sha256:browser-${instanceId}`
      return {
        instanceId,
        deploymentId,
        state,
        message: state === 'ready' ? null : 'browser mock state detail',
        currentRevisionId: revisionId,
        rollbackTargetRevisionId: state === 'ready' ? `urn:lsm:deployment-revision:v1:sha256:previous-${instanceId}` : null,
        runningRevisionId: control.state.running[instanceId] ? revisionId : null,
        revisions: [{
          id: revisionId,
          deploymentIdentity: {
            schemaVersion: 1,
            deploymentId: `urn:lsm:deployment:v1:sha256:browser-${instanceId}`,
            engineArtifactId: 'urn:lsm:engine:v1:sha256:browser-engine',
            modelArtifactId: 'urn:lsm:model:v1:sha256:browser-model',
            configRevisionId: `revision-current-${instanceId}`,
            configurationId: `urn:lsm:configuration:v1:sha256:current-${instanceId}`,
            qualificationEvidenceId: 'urn:lsm:qualification:v2:sha256:browser-evidence',
          },
          runtimePolicy: { autoStart: false, restartPolicy: 'never' },
          routing: {
            proxyEnabled: false,
            defaultTarget: false,
            routingStrategy: 'priorityFailover',
            routes: [],
          },
          createdAt: 1_787_059_200,
          current: true,
          rollbackTarget: false,
          integrityValid: state !== 'invalid',
        }],
      }
    }
    case 'mark_config_revision_known_good': {
      const instanceId = String(args.instanceId ?? '')
      const revisionId = String(args.revisionId ?? '')
      const expectedFingerprint = String(args.expectedCurrentFingerprint ?? '')
      const history = ensureRevisionHistory(instanceId)
      if (history.currentFingerprint !== expectedFingerprint) {
        throw new Error('CONFIG_REVISION_STALE: browser mock fingerprint changed')
      }
      const target = history.revisions.find(revision => revision.id === revisionId)
      if (!target) throw new Error('CONFIG_REVISION_NOT_FOUND: browser mock revision missing')
      const previousRevisionId = history.knownGoodRevisionId
      history.knownGoodRevisionId = revisionId
      history.revisions = history.revisions.map(revision => ({
        ...revision,
        knownGood: revision.id === revisionId,
      }))
      history.audit.unshift({
        id: `audit-${history.audit.length + 1}`,
        createdAt: 1_787_000_200 + history.audit.length,
        action: 'known_good_set',
        revisionId,
        previousRevisionId,
      })
      return clone(history)
    }
    case 'rollback_config_revision': {
      const instanceId = String(args.instanceId ?? '')
      const revisionId = String(args.revisionId ?? '')
      const expectedFingerprint = String(args.expectedCurrentFingerprint ?? '')
      const history = ensureRevisionHistory(instanceId)
      if (BROWSER_SCENARIO === 'config-revision-stale' || history.currentFingerprint !== expectedFingerprint) {
        throw new Error('CONFIG_REVISION_STALE: browser mock fingerprint changed')
      }
      const target = history.revisions.find(revision => revision.id === revisionId)
      const snapshot = revisionSnapshots.get(revisionId)
      const current = state.instances[instanceId]
      if (!target || !snapshot || !current) {
        throw new Error('CONFIG_REVISION_NOT_FOUND: browser mock rollback target missing')
      }
      const restored = {
        ...clone(snapshot),
        id: current.id,
        name: current.name,
      }
      const previousPort = current.port
      state.instances[instanceId] = clone(restored)
      const rollbackId = `revision-rollback-${instanceId}-${history.revisions.length}`
      history.revisions = history.revisions.map(revision => ({ ...revision, current: false }))
      history.revisions.unshift({
        id: rollbackId,
        fingerprint: target.fingerprint,
        identitySchemaVersion: 1,
        configurationId: target.configurationId,
        parentRevisionId: history.currentRevisionId,
        createdAt: 1_787_000_300,
        reason: 'rollback',
        rollbackOf: revisionId,
        current: true,
        knownGood: false,
        integrityValid: true,
        diffTruncated: false,
        changes: [{
          field: 'port',
          before: { state: 'value', value: String(previousPort) },
          after: { state: 'value', value: String(restored.port) },
          redacted: false,
        }],
      })
      history.currentRevisionId = rollbackId
      history.currentFingerprint = target.fingerprint
      history.currentConfigurationId = target.configurationId
      revisionSnapshots.set(rollbackId, clone(restored))
      const response: ConfigRevisionRollbackResponse = {
        config: clone(restored),
        history: clone(history),
      }
      return response
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
    case 'get_models':
      return BROWSER_SCENARIO === 'delayed-inventory-cache' && !delayedInventoryCacheLoaded
        ? []
        : clone(models)
    case 'scan_engines':
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
    case 'probe_engine_capabilities': return clone(engine)
    case 'qualify_engine': {
      const engineId = String(args.engineId ?? '')
      const modelId = String(args.modelId ?? '')
      const target = engines.find(candidate => candidate.id === engineId)
      const qualificationModel = models.find(candidate => candidate.id === modelId)
      if (!target || !qualificationModel) throw new Error('browser test qualification target not found')
      target.capabilities = {
        ...target.capabilities!,
        qualification: {
          schemaVersion: 2,
          profileVersion: 1,
          status: 'passed',
          executableFingerprint: target.capabilities!.executableFingerprint,
          engineArtifactId: 'urn:lsm:engine:v1:sha256:browser-engine',
          engineVersion: target.version,
          helpHash: target.capabilities!.helpHash,
          modelId: qualificationModel.id,
          modelArtifactId: 'urn:lsm:model:v1:sha256:browser-model',
          modelName: qualificationModel.name,
          modelSize: qualificationModel.size,
          startedAt: 10,
          completedAt: 11,
          evidenceId: 'urn:lsm:qualification:v2:sha256:browser-evidence',
          checks: [
            { name: 'version', status: 'passed', durationMs: 10, detail: 'engine version was detected' },
            { name: 'capabilities', status: 'passed', durationMs: 10, detail: 'required flags were confirmed' },
            { name: 'startup', status: 'passed', durationMs: 20, detail: 'qualification server remained running' },
            { name: 'health', status: 'passed', durationMs: 30, detail: 'GET /health returned HTTP 200 OK' },
            { name: 'inference', status: 'passed', durationMs: 40, detail: 'POST /completion returned predicted output' },
          ],
        },
      }
      return clone(target)
    }
    case 'cancel_engine_qualification': return true
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
    case 'enroll_worker_agent': {
      const enrollment = (args.enrollment ?? {}) as Record<string, unknown>
      const id = 'agent-browser-secure-worker'
      const existing = clusterWorkers.find(worker => worker.id === id)
      const worker: WorkerInfo = {
        id,
        host: '127.0.0.1',
        port: Number(enrollment.localPort ?? 50152) || 50152,
        name: String(enrollment.name ?? '') || 'Secure Browser Agent',
        origin: 'agent',
        devices: [],
        status: 'Offline',
        auto_discovered: false,
        agent: {
          agent_id: 'browser-secure-worker',
          control_host: String(enrollment.controlHost ?? 'worker.example.net'),
          control_port: Number(enrollment.controlPort ?? 7443),
          tunnel_host: String(enrollment.tunnelHost ?? 'worker.example.net'),
          tunnel_port: Number(enrollment.tunnelPort ?? 7444),
          tls_server_name: String(enrollment.tlsServerName ?? 'worker.example.net'),
          tls_cert_path: String(enrollment.tlsCertPath ?? 'C:\\secure\\worker-agent.crt'),
          token_path: String(enrollment.tokenPath ?? 'C:\\secure\\worker-agent.token'),
          certificate_sha256: 'a'.repeat(64),
        },
      }
      if (existing) Object.assign(existing, worker)
      else clusterWorkers.push(worker)
      return clone(worker)
    }
    case 'test_worker_agent':
      return { rpc_running: clusterWorkers.find(worker => worker.id === args.id)?.status === 'Online' }
    case 'start_worker_agent': {
      const worker = clusterWorkers.find(worker => worker.id === args.id)
      if (worker) worker.status = 'Online'
      return { rpc_running: true }
    }
    case 'stop_worker_agent': {
      const worker = clusterWorkers.find(worker => worker.id === args.id)
      if (worker) worker.status = 'Offline'
      return { rpc_running: false }
    }
    case 'list_worker_agent_audit':
      return [{
        sequence: 1,
        timestamp: new Date().toISOString(),
        event: 'status',
        outcome: 'allowed',
        detail: 'request completed',
        hash: 'b'.repeat(64),
      }]
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
    case 'inspect_model_residency': return residencyInspection()
    case 'save_model_residency_policy': {
      residencyPolicy = clone(args.policy as ResidencyPolicy)
      residencyAudit.push({
        id: `browser-residency-audit-${residencyAudit.length + 1}`,
        recordedAt: Math.floor(Date.now() / 1000),
        action: 'policy_updated',
        outcome: 'success',
      })
      return residencyInspection()
    }
    case 'begin_model_residency_drain': return {
      instanceId: String(args.instanceId ?? ''),
      routingDrained: true,
      activeRequests: 0,
    }
    case 'get_model_residency_drain_status': return {
      instanceId: String(args.instanceId ?? ''),
      routingDrained: true,
      activeRequests: 0,
    }
    case 'complete_model_residency_operation': {
      residencyAudit.push({
        id: `browser-residency-audit-${residencyAudit.length + 1}`,
        recordedAt: Math.floor(Date.now() / 1000),
        action: String(args.action ?? ''),
        outcome: args.success ? 'success' : 'failed',
        instanceId: String(args.instanceId ?? ''),
        deploymentId: String(args.deploymentId ?? ''),
        revisionId: String(args.revisionId ?? ''),
        planId: String(args.planId ?? ''),
        message: args.error ? String(args.error) : undefined,
      })
      return residencyInspection()
    }
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
    case 'list_canary_rollouts': return clone(canaryRollouts)
    case 'create_canary_rollout': {
      const stableInstanceId = String(args.stableInstanceId ?? '')
      const candidateInstanceId = String(args.candidateInstanceId ?? '')
      const candidateWeight = Number(args.candidateWeight ?? 10)
      const now = Date.now()
      const rollout: BrowserCanaryRollout = {
        id: 'browser-canary-rollout',
        modelAlias: String(args.modelAlias ?? ''),
        state: 'active',
        stableInstanceId,
        candidateInstanceId,
        stableRevisionId: 'urn:lsm:deployment-revision:v1:sha256:stable-browser-revision',
        candidateRevisionId: 'urn:lsm:deployment-revision:v1:sha256:candidate-browser-revision',
        stableWeight: 100 - candidateWeight,
        candidateWeight,
        createdAt: now,
        updatedAt: now,
        integrityValid: true,
        drift: [],
        canChangeTraffic: true,
        canPromote: true,
        canAbort: true,
        canRollback: false,
        stableHealth: canaryHealth(stableInstanceId),
        candidateHealth: canaryHealth(candidateInstanceId),
        stableEvidence: null,
        candidateEvidence: null,
        events: [],
      }
      appendCanaryEvent(rollout, 'created', `canary activated at ${candidateWeight}% candidate traffic`)
      canaryRollouts = [rollout]
      return clone(rollout)
    }
    case 'observe_canary_rollout': {
      const rollout = canaryRollouts.find(item => item.id === String(args.rolloutId ?? ''))
      if (!rollout) throw new Error('browser canary rollout not found')
      canaryObservationRound += 1
      rollout.stableEvidence = canaryEvidence(20 + canaryObservationRound, 1)
      rollout.candidateEvidence = canaryEvidence(4 + canaryObservationRound, 0)
      appendCanaryEvent(rollout, 'observed', 'browser observation captured')
      return clone(rollout)
    }
    case 'set_canary_weight': {
      const rollout = canaryRollouts.find(item => item.id === String(args.rolloutId ?? ''))
      if (!rollout) throw new Error('browser canary rollout not found')
      const candidateWeight = Number(args.candidateWeight ?? rollout.candidateWeight)
      rollout.candidateWeight = candidateWeight
      rollout.stableWeight = 100 - candidateWeight
      appendCanaryEvent(rollout, 'traffic_changed', `candidate traffic changed to ${candidateWeight}%`)
      return clone(rollout)
    }
    case 'promote_canary_rollout': {
      const rollout = canaryRollouts.find(item => item.id === String(args.rolloutId ?? ''))
      if (!rollout) throw new Error('browser canary rollout not found')
      rollout.state = 'promoted'
      rollout.stableWeight = 0
      rollout.candidateWeight = 100
      rollout.canChangeTraffic = false
      rollout.canPromote = false
      rollout.canAbort = false
      rollout.canRollback = true
      appendCanaryEvent(rollout, 'promoted', 'candidate promoted to 100% traffic')
      return clone(rollout)
    }
    case 'abort_canary_rollout':
    case 'rollback_canary_rollout': {
      const rollout = canaryRollouts.find(item => item.id === String(args.rolloutId ?? ''))
      if (!rollout) throw new Error('browser canary rollout not found')
      const rollback = command === 'rollback_canary_rollout'
      rollout.state = rollback ? 'rolled_back' : 'aborted'
      rollout.stableWeight = 0
      rollout.candidateWeight = 0
      rollout.canChangeTraffic = false
      rollout.canPromote = false
      rollout.canAbort = false
      rollout.canRollback = false
      appendCanaryEvent(rollout, rollback ? 'rolled_back' : 'aborted', 'base routing restored')
      return clone(rollout)
    }
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
    case 'resolve_path': return args.path === 'models' ? 'C:\\browser-test\\models' : String(args.path ?? '')
    case 'generate_server_command': {
      if (BROWSER_SCENARIO === 'command-error') throw new Error('Browser test command generation failed')
      const config = args.config as InstanceConfig
      const generated = generatedCommand(config)
      control.lastGenerated = clone(generated)
      syncAutomationProbe()
      return generated
    }
    case 'plan_instance_resources': return resourcePlan()
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
    case 'stop_server': {
      const instanceId = String(args.instanceId ?? '')
      delete control.state.running[instanceId]
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
