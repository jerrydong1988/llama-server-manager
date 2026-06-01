import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

// ── 类型定义 ───────────────────────────────────────────────────
export interface ModelInfo {
  id: string
  name: string
  path: string
  size: number
  architecture?: string
  context_length?: number
  quant_type?: string
  file_type: string
}

export interface EngineInfo {
  id: string
  name: string
  dir: string
  exe: string
  version: string
  backend: string
}

export interface InstanceConfig {
  // Basic
  id: string; name: string; engine_id: string; model_path: string; alias: string;
  lora_path: string; mmproj_path: string; lora_init_without_apply: boolean;
  chat_template: string; chat_template_file: string; skip_chat_parsing: boolean;
  reasoning_format: string; reasoning_effort: string; reasoning: string;
  jinja: boolean; reasoning_budget: string; reasoning_budget_message: string;
  grammar_file: string; grammar: string;
  // Performance & Context
  ctx_size: number; ctx_size_auto: boolean; gpu_layers: number;
  threads: number; batch_size: number; ubatch_size: number; parallel: number;
  cont_batching: boolean; cache_prompt: boolean; threads_batch: number;
  threads_http: number; keep: number; cache_reuse: number; cache_ram: number;
  warmup: boolean; ctx_checkpoints: number; checkpoint_min_step: number;
  swa_full: boolean;
  // RoPE / YaRN
  rope_scaling: string; rope_scale: number; rope_freq_base: number; rope_freq_scale: number;
  yarn_ext_factor: number; yarn_attn_factor: number; yarn_beta_slow: number; yarn_beta_fast: number;
  // Memory & KV Cache
  flash_attn: string; moe_cpu_layers: number; mlock: boolean;
  no_mmap: boolean; numa: boolean; context_shift: boolean;
  check_tensors: boolean; fit: boolean; kv_unified: boolean; cache_idle_slots: boolean;
  cache_type_k: string; cache_type_v: string;
  cache_type_draft_k: string; cache_type_draft_v: string;
  // Speculative Decoding
  draft_model_path: string; draft_gpu_layers: number; draft_tokens: number;
  spec_draft_n_min: number; spec_type: string;
  spec_draft_p_min: number; spec_draft_p_split: number; spec_draft_device: string;
  lookup_cache_static: string; lookup_cache_dynamic: string;
  // GPU & Device
  device: string; split_mode: string; tensor_split: string; main_gpu: number;
  override_kv: string;
  // Server & Network
  host: string; port: number; api_key: string; api_key_file: string;
  ssl_key_file: string; ssl_cert_file: string; path_prefix: string; api_prefix: string;
  no_ui: boolean; ui_config_file: string;
  embedding: boolean; pooling: string; embd_normalize: number; reranking: boolean;
  metrics: boolean; props: boolean; slots_enabled: boolean;
  slot_save_path: string; slot_prompt_similarity: number; prefill_assistant: string;
  // Multi-Model & Media
  models_dir: string; models_preset: string; models_max: number; models_autoload: boolean;
  mmproj_url: string; mmproj_auto: boolean; image_min_tokens: number; image_max_tokens: number;
  tags: string; media_path: string; tools: string;
  // Generation
  n_predict: number; ignore_eos: boolean; json_schema: string;
  temp: number; top_k: number; top_p: number; repeat_penalty: number;
  seed: number; min_p: number; presence_penalty: number;
  frequency_penalty: number; repeat_last_n: number;
  reverse_prompt: string; special: boolean; spm_infill: boolean; backend_sampling: boolean;
  // Advanced Sampling
  mirostat: number; mirostat_lr: number; mirostat_ent: number;
  xtc_probability: number; xtc_threshold: number;
  dynatemp_range: number; dynatemp_exp: number; typical_p: number;
  dry_multiplier: number; dry_base: number;
  dry_allowed_length: number; dry_penalty_last_n: number;
  dry_sequence_breaker: string;
  adaptive_target: number; adaptive_decay: number; top_n_sigma: number;
  logit_bias: string; samplers: string; sampler_seq: string;
  // Misc
  timeout: number; sleep_idle: number; verbose: boolean; custom_args: string[];
}

export interface Instance {
  id: string
  name: string
  status: 'running' | 'stopped' | 'error'
  model: string
  port: number
  healthCheck: 'ok' | 'fail' | 'pending'
  startTime?: number
  config: InstanceConfig
}

export interface LogEntry {
  instanceId: string
  text: string
  timestamp: number
}

export interface MsFileEntry {

  name: string
  path: string
  size: number
  file_type: string
}

interface DownloadProgress {
  fileName: string
  downloaded: number
  total: number
  index: number
  totalFiles: number
}

// ── 默认配置 ───────────────────────────────────────────────────
export function defaultInstanceConfig(): InstanceConfig {
  return {
    id: '', name: '', engine_id: '', model_path: '', alias: '',
    lora_path: '', mmproj_path: '', lora_init_without_apply: false,
    chat_template: '', chat_template_file: '', skip_chat_parsing: false,
    reasoning_format: '', reasoning_effort: '', reasoning: '',
    jinja: false, reasoning_budget: '', reasoning_budget_message: '',
    grammar_file: '', grammar: '',
    ctx_size: 4096, ctx_size_auto: false, gpu_layers: 99,
    threads: 0, batch_size: 2048, ubatch_size: 512, parallel: -1,
    cont_batching: false, cache_prompt: true, threads_batch: 0,
    threads_http: -1, keep: 0, cache_reuse: 0, cache_ram: 0,
    warmup: false, ctx_checkpoints: 32, checkpoint_min_step: 0,
    swa_full: false,
    rope_scaling: '', rope_scale: 0, rope_freq_base: 0, rope_freq_scale: 0,
    yarn_ext_factor: -1, yarn_attn_factor: 1, yarn_beta_slow: 0, yarn_beta_fast: 32,
    flash_attn: 'auto', moe_cpu_layers: 0, mlock: false,
    no_mmap: false, numa: false, context_shift: false,
    check_tensors: false, fit: false, kv_unified: false, cache_idle_slots: true,
    cache_type_k: '', cache_type_v: '',
    cache_type_draft_k: '', cache_type_draft_v: '',
    draft_model_path: '', draft_gpu_layers: 99, draft_tokens: 16,
    spec_draft_n_min: 0, spec_type: '',
    spec_draft_p_min: 0, spec_draft_p_split: 0.1, spec_draft_device: '',
    lookup_cache_static: '', lookup_cache_dynamic: '',
    device: '', split_mode: '', tensor_split: '', main_gpu: 0,
    override_kv: '',
    host: '127.0.0.1', port: 8080, api_key: '', api_key_file: '',
    ssl_key_file: '', ssl_cert_file: '', path_prefix: '', api_prefix: '',
    no_ui: false, ui_config_file: '',
    embedding: false, pooling: '', embd_normalize: 2, reranking: false,
    metrics: false, props: false, slots_enabled: true,
    slot_save_path: '', slot_prompt_similarity: 0.1, prefill_assistant: '',
    models_dir: '', models_preset: '', models_max: 4, models_autoload: false,
    mmproj_url: '', mmproj_auto: false, image_min_tokens: 0, image_max_tokens: 0,
    tags: '', media_path: '', tools: '',
    n_predict: -1, ignore_eos: false, json_schema: '',
    temp: 0.8, top_k: 40, top_p: 0.9, repeat_penalty: 1.1,
    seed: -1, min_p: 0.05, presence_penalty: 0,
    frequency_penalty: 0, repeat_last_n: 64,
    reverse_prompt: '', special: false, spm_infill: false, backend_sampling: false,
    mirostat: 0,     mirostat_lr: 0, mirostat_ent: 0,
    xtc_probability: 0, xtc_threshold: 0,
    dynatemp_range: 0, dynatemp_exp: 0, typical_p: 1.0,
    dry_multiplier: 0, dry_base: 0,
    dry_allowed_length: 0, dry_penalty_last_n: 0,
    dry_sequence_breaker: '',
    adaptive_target: 0, adaptive_decay: 0, top_n_sigma: -1,
    logit_bias: '', samplers: '', sampler_seq: '',
    timeout: 600, sleep_idle: -1, verbose: false, custom_args: [],
  }
}

// ── Store 状态 ─────────────────────────────────────────────────
interface AppState {
  models: ModelInfo[]
  engines: EngineInfo[]
  instances: Instance[]
  logs: Record<string, LogEntry[]>
  isLoading: boolean
  defaultEngineId: string | null
  modelDirs: string[]
  engineDirs: string[]
  activeConfigInstanceId: string | null
  activeTab: string
  darkMode: boolean
  setActiveTab: (tab: string) => void
  setDarkMode: (dm: boolean) => void
  setActiveConfigInstanceId: (id: string | null) => void

  setModels: (models: ModelInfo[]) => void
  setEngines: (engines: EngineInfo[]) => void
  setModelDirs: (dirs: string[]) => void
  setEngineDirs: (dirs: string[]) => void
  setDefaultEngineId: (id: string | null) => void
  addInstance: (instance: Instance) => void
  updateInstance: (id: string, partial: Partial<Instance>) => void
  deleteInstance: (id: string) => void
  moveInstance: (id: string, direction: 'up' | 'down') => void
  renameInstance: (id: string, name: string) => void
  addLog: (entry: LogEntry) => void
  clearLogs: (instanceId: string) => void

  loadInitialData: () => Promise<void>
  scanModels: (paths: string[]) => Promise<string | null>
  deleteModelFile: (path: string) => Promise<void>
  openModelFolder: (path: string) => Promise<void>
  readGgufMetadata: (path: string) => Promise<[string | null, number | null, string | null]>

  scanEngines: (paths: string[]) => Promise<void>
  deleteEngine: (id: string) => Promise<void>
  openEngineFolder: (dir: string) => Promise<void>

  generateCommand: (config: InstanceConfig, engineExe: string) => Promise<string[]>
  startInstance: (id: string) => Promise<void>
  stopInstance: (id: string) => Promise<void>
  openBrowser: (host: string, port: number) => Promise<void>

  saveConfig: () => Promise<void>
  loadConfig: () => Promise<void>

  browseModelscope: (repoId: string) => Promise<MsFileEntry[]>
  downloadModelscopeFiles: (repoId: string, files: MsFileEntry[], saveDir: string) => Promise<void>
  cancelFileDownload: (fileName: string) => Promise<void>
  pauseFileDownload: (fileName: string) => Promise<void>
  cancelAndCleanupDownload: (fileName: string, filePath: string) => Promise<void>
}

export const useAppStore = create<AppState>((set, get) => ({
  models: [],
  engines: [],
  instances: [],
  logs: {},
  isLoading: false,
  defaultEngineId: null,
  modelDirs: [],
  engineDirs: [],
  activeConfigInstanceId: null,
  activeTab: 'model-repo',
  downloadProgress: {},
  setModels: (models) => set({ models }),
  setEngines: (engines) => set({ engines }),
  setModelDirs: (dirs) => { set({ modelDirs: dirs }); get().saveConfig() },
  setEngineDirs: (dirs) => { set({ engineDirs: dirs }); get().saveConfig() },
  setDefaultEngineId: (id) => { set({ defaultEngineId: id }); get().saveConfig() },
  setActiveConfigInstanceId: (id) => set({ activeConfigInstanceId: id }),
  setActiveTab: (tab) => { set({ activeTab: tab }); get().saveConfig() },
  setDarkMode: (dm) => { set({ darkMode: dm }); document.documentElement.classList.toggle('dark', dm); get().saveConfig() },

  addInstance: (instance) => set((s) => ({ instances: [...s.instances, instance] })),
  updateInstance: (id, partial) => set((s) => ({
    instances: s.instances.map((i) => (i.id === id ? { ...i, ...partial } : i)),
  })),
  deleteInstance: (id) => set((s) => ({
    instances: s.instances.filter((i) => i.id !== id),
  })),

  moveInstance: (id, direction) => set((s) => {
    const idx = s.instances.findIndex(i => i.id === id)
    if (idx < 0) return s
    const target = direction === 'up' ? idx - 1 : idx + 1
    if (target < 0 || target >= s.instances.length) return s
    const arr = [...s.instances];
    [arr[idx], arr[target]] = [arr[target], arr[idx]]
    get().saveConfig()
    return { instances: arr }
  }),

  renameInstance: (id, name) => set((s) => {
    const inst = s.instances.find(i => i.id === id)
    if (!inst) return s
    const newConfig = { ...inst.config, name }
    get().saveConfig()
    return { instances: s.instances.map(i => i.id === id ? { ...i, name, config: newConfig } : i) }
  }),

  addLog: (entry) => set((s) => {
    const existing = s.logs[entry.instanceId] || []
    const updated = [...existing.slice(-499), { ...entry, timestamp: Date.now() }]
    return { logs: { ...s.logs, [entry.instanceId]: updated } }
  }),
  clearLogs: (instanceId) => set((s) => ({
    logs: { ...s.logs, [instanceId]: [] },
  })),

  // ── 初始化 ──────────────────────────────────────────────────
  loadInitialData: async () => {
    set({ isLoading: true })
    try {
      const [models, engines] = await Promise.all([
        invoke<ModelInfo[]>('get_models').catch(() => [] as ModelInfo[]),
        invoke<EngineInfo[]>('get_engines').catch(() => [] as EngineInfo[]),
      ])
      set({ models, engines, isLoading: false })
    } catch { set({ isLoading: false }) }
  },

  // ── 模型仓库 ──────────────────────────────────────────────────
  scanModels: async (paths: string[]) => {
    set({ isLoading: true })
    try {
      const models = await invoke<ModelInfo[]>('scan_models', { paths })
      set({ models, modelDirs: paths, isLoading: false })
      return null
    } catch (e: any) {
      console.error('scan_models error:', e)
      set({ isLoading: false })
      return e?.message || e?.toString() || '扫描失败'
    }
  },

  deleteModelFile: async (path: string) => {
    await invoke('delete_model_file', { path })
    set((s) => ({ models: s.models.filter((m) => m.path !== path) }))
  },

  openModelFolder: async (path: string) => {
    await invoke('open_model_folder', { path })
  },

  readGgufMetadata: async (path: string) => {
    return await invoke<[string | null, number | null, string | null]>('read_gguf_metadata', { path })
  },

  // ── 引擎管理 ──────────────────────────────────────────────────
  scanEngines: async (paths: string[]) => {
    set({ isLoading: true })
    try {
      const engines = await invoke<EngineInfo[]>('scan_engines', { paths })
      set({ engines, engineDirs: paths, isLoading: false })
    } catch (e) { console.error('scan_engines error:', e); set({ isLoading: false }) }
  },

  deleteEngine: async (id: string) => {
    await invoke('delete_engine', { id })
    set((s) => ({ engines: s.engines.filter((e) => e.id !== id) }))
  },

  openEngineFolder: async (dir: string) => {
    await invoke('open_engine_folder', { dir })
  },

  // ── 服务器控制 ────────────────────────────────────────────────
  generateCommand: async (config: InstanceConfig, engineExe: string) => {
    return await invoke<string[]>('generate_server_command', { config, engineExe })
  },

  startInstance: async (id: string) => {
    try {
      const { instances, engines, defaultEngineId } = get()
      const inst = instances.find(i => i.id === id)
      if (!inst) return
      // 实例指定引擎 > 默认引擎 > 第一个引擎
      const engine = engines.find(e => e.id === inst.config.engine_id)
        || engines.find(e => e.id === defaultEngineId)
        || engines[0]
      if (!engine) return

      await invoke('start_server', { instanceId: id, config: inst.config, engineExe: engine.exe })
      get().updateInstance(id, { status: 'running', healthCheck: 'pending' })
    } catch (e) {
      console.error('start_server error:', e)
    }
  },

  stopInstance: async (id: string) => {
    try {
      await invoke('stop_server', { instanceId: id })
      get().updateInstance(id, { status: 'stopped', healthCheck: 'pending' })
    } catch (e) {
      console.error('stop_server error:', e)
    }
  },

  openBrowser: async (host: string, port: number) => {
    await invoke('open_browser', { host, port })
  },

  // ── 配置持久化 ────────────────────────────────────────────────
  saveConfig: async () => {
    const { instances, modelDirs, engineDirs, defaultEngineId, activeTab, darkMode } = get()
    const map: Record<string, InstanceConfig> = {}
    const order: string[] = []
    instances.forEach((i) => { map[i.id] = i.config; order.push(i.id) })
    await invoke('save_config', { instances: map, modelDirs, engineDirs, defaultEngineId: defaultEngineId || '', instanceOrder: order, lastTab: activeTab, darkMode })
  },

  loadConfig: async () => {
    try {
      const global = await invoke<{
        instances: Record<string, InstanceConfig>
        model_dirs: string[]
        engine_dirs: string[]
        default_engine_id: string
        running: Record<string, { instance_id: string; pid: number; port: number; host: string; start_time?: number }>
        instance_order: string[]
        last_tab: string
        dark_mode: boolean
      }>('load_config')

      const runningIds = new Set(Object.keys(global.running || {}))
      const order = global.instance_order || Object.keys(global.instances)
      let list: Instance[] = Object.entries(global.instances).map(([id, config]) => ({
        id, name: config.name || '未命名实例', status: runningIds.has(id) ? 'running' as const : 'stopped' as const,
        model: config.model_path.split('\\').pop() || config.model_path,
        port: config.port, healthCheck: runningIds.has(id) ? 'pending' as const : 'pending' as const, config,
        startTime: (global.running?.[id]?.start_time ?? 0) > 0 ? global.running[id].start_time * 1000 : undefined,
      }))
      // 按保存的顺序排列
      list.sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id))
        set({
          instances: list,
          modelDirs: global.model_dirs || [],
          engineDirs: global.engine_dirs || [],
          defaultEngineId: global.default_engine_id || null,
          activeTab: global.last_tab || 'model-repo',
        })
        if (global.dark_mode !== undefined) {
          document.documentElement.classList.toggle('dark', global.dark_mode)
          set({ darkMode: !!global.dark_mode })
        }
      // 加载后扫描模型和引擎
      invoke('scan_models', { paths: global.model_dirs || [] }).then((models) => set({ models: models as any })).catch(() => {})
      invoke('scan_engines', { paths: global.engine_dirs || [] }).then((engines) => set({ engines: engines as any })).catch(() => {})
    } catch (e) { console.error('load_config error:', e) }
  },

  // ── ModelScope ──────────────────────────────────────────────────
  browseModelscope: async (repoId: string) => {
    return await invoke<MsFileEntry[]>('browse_modelscope', { repoId })
  },

  downloadModelscopeFiles: async (repoId: string, files: MsFileEntry[], saveDir: string) => {
    await invoke('download_modelscope_files', { repoId, files, saveDir })
  },

  cancelFileDownload: async (fileName: string) => {
    try { await invoke('cancel_file_download', { fileName }) } catch (e) { console.error(e) }
  },

  pauseFileDownload: async (fileName: string) => {
    try { await invoke('pause_file_download', { fileName }) } catch (e) { console.error(e) }
  },

  cancelAndCleanupDownload: async (fileName: string, filePath: string) => {
    try { await invoke('cancel_and_cleanup_download', { fileName, filePath }) } catch (e) { console.error(e) }
  },
}))

// ── 事件监听 ─────────────────────────────────────────────────
listen<{ instanceId: string; text: string }>('server-log', (event) => {
  useAppStore.getState().addLog({
    instanceId: event.payload.instanceId,
    text: event.payload.text,
    timestamp: Date.now(),
  })
}).catch(() => {})

listen<{ instanceId: string }>('server-started', (event) => {
  const state = useAppStore.getState()
  state.updateInstance(event.payload.instanceId, {
    status: 'running',
    healthCheck: 'pending',
    startTime: Date.now(),
  })
  state.saveConfig()
}).catch(() => {})

listen<{ instanceId: string }>('server-stopped', (event) => {
  const state = useAppStore.getState()
  const inst = state.instances.find(i => i.id === event.payload.instanceId)
  if (inst) {
    const isError = inst.status === 'running' && inst.healthCheck !== 'ok'
    state.updateInstance(event.payload.instanceId, {
      status: isError ? 'error' : 'stopped',
      healthCheck: isError ? 'fail' : 'pending',
    })
  }
  state.saveConfig()
}).catch(() => {})

listen<{ instanceId: string; error: string }>('server-error', (event) => {
  const state = useAppStore.getState()
  state.updateInstance(event.payload.instanceId, {
    status: 'error',
    healthCheck: 'fail',
  })
  state.saveConfig()
}).catch(() => {})

listen<{ instanceId: string; status: string }>('health-status', (event) => {
  useAppStore.getState().updateInstance(event.payload.instanceId, {
    healthCheck: event.payload.status === 'ok' ? 'ok' : 'fail',
  })
}).catch(() => {})
