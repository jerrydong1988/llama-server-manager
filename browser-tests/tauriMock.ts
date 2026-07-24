import { mockConvertFileSrc, mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { emit } from '@tauri-apps/api/event'
import { defaultInstanceConfig } from '../src/store/defaults'
import type { GlobalConfigShape } from '../src/store/bootstrap'
import type { EngineInfo, GeneratedServerCommand, InstanceConfig, ModelInfo, SystemMetrics } from '../src/store/types'

const BROWSER_TEST_MARKER = '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__'
const BROWSER_SCENARIO = new URLSearchParams(window.location.search).get('scenario')
const INSTANCE_ID = 'browser-test-instance'
const STOPPED_INSTANCE_ID = 'browser-stopped-instance'
const ENGINE_ID = 'browser-test-engine'
const MODEL_PATH = 'C:\\browser-test\\models\\Qwen-Browser-Test-Q8_0.gguf'
const AMBIGUOUS_MODEL_PATH = 'C:\\browser-test\\models\\Vision-Ambiguous-Q8_0.gguf'
const QWEN_PROJECTOR_PATH = 'C:\\browser-test\\models\\mmproj-Qwen-BF16.gguf'
const LLAVA_PROJECTOR_PATH = 'C:\\browser-test\\models\\mmproj-Llava-BF16.gguf'

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

const models = [model, ambiguousModel, qwenProjector, llavaProjector]

const engine: EngineInfo = {
  id: ENGINE_ID,
  name: 'Browser Test Engine',
  dir: 'C:\\browser-test\\engine',
  exe: 'C:\\browser-test\\engine\\llama-server.exe',
  version: 'version: 10042 (browser-test)',
  backend: 'Vulkan',
  capabilities: {
    status: 'detected',
    versionStatus: 'detected',
    supportedFlags: [
      '--temp', '--top-k', '--top-p', '--threads', '--kv-unified',
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
    probedAt: 1,
  },
}

const instanceConfig: InstanceConfig = {
  ...defaultInstanceConfig(),
  id: INSTANCE_ID,
  name: 'Browser Parameter Regression',
  engine_id: ENGINE_ID,
  model_path: MODEL_PATH,
  alias: 'browser-parameter-regression',
  port: 18081,
  temp: 0.6,
  top_k: 20,
  kv_unified: true,
  kv_unified_mode: 'on',
  models_autoload: false,
  image_min_tokens: 1_024,
  explicit_overrides: ['temp', 'top_k', 'kv_unified', 'kv_unified_mode', 'models_autoload', 'image_min_tokens'],
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

type BrowserProxyRoute = {
  id: string
  enabled: boolean
  priority: number
  model_alias: string
  target_instance_id: string
}

type BrowserProxyConfig = {
  enabled: boolean
  host: string
  port: number
  public_api_key: string
  default_instance_id: string
  routing_strategy: string
  timeout_ms: number
  background_service_mode: boolean
  runtime_service_enabled: boolean
  routes: BrowserProxyRoute[]
}

const proxyConfig: BrowserProxyConfig = {
  enabled: ['background-runtime-active', 'proxy-routing', 'proxy-route-health', 'proxy-route-legacy-ids'].includes(BROWSER_SCENARIO ?? ''),
  host: '127.0.0.1',
  port: 11435,
  public_api_key: '',
  default_instance_id: '',
  routing_strategy: 'firstHealthy',
  timeout_ms: 600_000,
  background_service_mode: false,
  runtime_service_enabled: BROWSER_SCENARIO === 'background-runtime-active',
  routes: BROWSER_SCENARIO === 'proxy-route-health'
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
  running: ['background-runtime-active', 'proxy-routing', 'proxy-route-health', 'proxy-route-legacy-ids'].includes(BROWSER_SCENARIO ?? ''),
  bound_addr: '127.0.0.1:11435',
  active_routes: ['proxy-route-health', 'proxy-route-legacy-ids'].includes(BROWSER_SCENARIO ?? '') ? 2 : 0,
  last_error: null,
}
const runningProxyTarget = {
  instance_id: INSTANCE_ID,
  name: 'Browser Parameter Regression',
  alias: 'browser-parameter-regression',
  host: '127.0.0.1',
  port: 18081,
  running: true,
}
const proxyTargets = BROWSER_SCENARIO === 'proxy-routing'
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
  serviceVersion: '2.9.30-browser-test',
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
if (BROWSER_SCENARIO === 'empty-alias') {
  state.instances[INSTANCE_ID].alias = ''
  state.instances[INSTANCE_ID].explicit_overrides = (
    state.instances[INSTANCE_ID].explicit_overrides ?? []
  ).filter(field => field !== 'alias')
}

type BrowserTestControl = {
  marker: string
  calls: Array<{ command: string; payload: unknown }>
  unhandled: string[]
  saveCount: number
  lastGenerated: GeneratedServerCommand | null
  failProxyStatus: boolean
  failProxyTargets: boolean
  failRuntimeStatus: boolean
  state: GlobalConfigShape
  emitEvent: (event: string, payload?: unknown) => Promise<void>
}

declare global {
  interface Window {
    __TAURI_BROWSER_TEST__: BrowserTestControl
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
  state,
  emitEvent: (event, payload) => emit(event, payload),
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

const systemMetrics: SystemMetrics = {
  cpu_percent: 0,
  memory_mb: 128,
  uptime_secs: 30,
  gpu_percent: 0,
  vram_used_mb: 256,
  vram_total_mb: 8_192,
  system_cpu_percent: 0,
  system_memory_total_mb: 32_768,
  system_memory_used_mb: 8_192,
  gpu_vendor: 'Mock',
  gpu_name: 'Browser Test GPU',
}

mockWindows('main')
mockConvertFileSrc('windows')
mockIPC((command, payload) => {
  control.calls.push({ command, payload: clone(payload ?? null) })
  syncAutomationProbe()
  const args = (payload ?? {}) as Record<string, unknown>

  switch (command) {
    case 'get_startup_elapsed': return 1
    case 'get_cached_scan': return [clone(models), [clone(engine)]]
    case 'load_config': return clone(control.state)
    case 'scan_models':
    case 'get_models': return clone(models)
    case 'scan_engines':
    case 'get_engines': return [clone(engine)]
    case 'probe_engine_capabilities': return clone(engine)
    case 'get_download_manager_snapshot':
      return { queue: [], active_count: 0, max_concurrent: 3, resume_policy: 'manual', bandwidth_limit_bytes_per_sec: 0, low_priority_throttle: false }
    case 'restore_download_queue': return []
    case 'get_monitoring_series': return []
    case 'get_system_health': return clone(systemMetrics)
    case 'get_workers': return []
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
      Object.assign(proxyConfig, next)
      proxyConfig.routes = next.routes
      proxyStatus.active_routes = next.routes.filter(route => route.enabled).length
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
    case 'save_config': {
      const instances = clone(args.instances as Record<string, InstanceConfig>)
      control.state.instances = instances
      if (Array.isArray(args.modelDirs)) control.state.model_dirs = clone(args.modelDirs as string[])
      control.saveCount += 1
      syncAutomationProbe()
      return instances
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
