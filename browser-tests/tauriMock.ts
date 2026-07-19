import { mockConvertFileSrc, mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { defaultInstanceConfig } from '../src/store/defaults'
import type { GlobalConfigShape } from '../src/store/bootstrap'
import type { EngineInfo, GeneratedServerCommand, InstanceConfig, ModelInfo, SystemMetrics } from '../src/store/types'

const BROWSER_TEST_MARKER = '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__'
const INSTANCE_ID = 'browser-test-instance'
const ENGINE_ID = 'browser-test-engine'
const MODEL_PATH = 'C:\\browser-test\\models\\Qwen-Browser-Test-Q8_0.gguf'

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T

const model: ModelInfo = {
  id: 'browser-test-model',
  name: 'Qwen Browser Test Q8_0.gguf',
  path: MODEL_PATH,
  size: 4_294_967_296,
  architecture: 'qwen3',
  context_length: 131_072,
  quant_type: 'Q8_0',
  capabilities: { metadata_complete: true },
  file_type: 'model',
}

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
    supportedFlags: ['--temp', '--top-k', '--top-p', '--kv-unified'],
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
  explicit_overrides: ['temp', 'top_k', 'kv_unified', 'kv_unified_mode'],
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

type BrowserTestControl = {
  marker: string
  calls: Array<{ command: string; payload: unknown }>
  unhandled: string[]
  saveCount: number
  lastGenerated: GeneratedServerCommand | null
  state: GlobalConfigShape
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
  state,
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
  if (field === 'kv_unified') return 'kv_unified_mode'
  if (field === 'fit') return 'fit_mode'
  if (field === 'numa') return 'numa_mode'
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
    if (field === 'temp') command.push('--temp', String(config.temp))
    else if (field === 'top_k') command.push('--top-k', String(config.top_k))
    else if (field === 'top_p') command.push('--top-p', String(config.top_p))
    else if (field === 'kv_unified_mode') command.push(config.kv_unified_mode === 'off' ? '--no-kv-unified' : '--kv-unified')
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
    case 'get_cached_scan': return [[clone(model)], [clone(engine)]]
    case 'load_config': return clone(control.state)
    case 'scan_models':
    case 'get_models': return [clone(model)]
    case 'scan_engines':
    case 'get_engines': return [clone(engine)]
    case 'probe_engine_capabilities': return clone(engine)
    case 'get_download_manager_snapshot':
      return { queue: [], active_count: 0, max_concurrent: 3, resume_policy: 'manual', bandwidth_limit_bytes_per_sec: 0, low_priority_throttle: false }
    case 'restore_download_queue': return []
    case 'get_monitoring_series': return []
    case 'get_system_health': return clone(systemMetrics)
    case 'get_workers': return []
    case 'is_autostart_enabled': return false
    case 'generate_server_command': {
      const config = args.config as InstanceConfig
      const generated = generatedCommand(config)
      control.lastGenerated = clone(generated)
      syncAutomationProbe()
      return generated
    }
    case 'save_config': {
      const instances = clone(args.instances as Record<string, InstanceConfig>)
      control.state.instances = instances
      control.saveCount += 1
      syncAutomationProbe()
      return instances
    }
    case 'enable_autostart':
    case 'disable_autostart':
    case 'show_window':
    case 'hide_window':
    case 'open_browser': return null
    default:
      control.unhandled.push(command)
      syncAutomationProbe()
      throw new Error(`Unhandled browser-test Tauri command: ${command}`)
  }
}, { shouldMockEvents: true })

window.__TAURI_BROWSER_TEST__ = control
window.__INITIAL_CONFIG__ = clone(state)
syncAutomationProbe()
