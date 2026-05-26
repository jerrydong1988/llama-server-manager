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
  id: string; name: string; engine_id: string; model_path: string; alias: string;
  lora_path: string; mmproj_path: string; chat_template: string;
  reasoning_format: string; reasoning_effort: string; reasoning: string;
  jinja: boolean; reasoning_budget: string; grammar_file: string;
  ctx_size: number; ctx_size_auto: boolean; gpu_layers: number;
  threads: number; batch_size: number; ubatch_size: number; parallel: number;
  cont_batching: boolean; cache_prompt: boolean; threads_batch: number;
  flash_attn: string; moe_cpu_layers: number; mlock: boolean;
  no_mmap: boolean; numa: boolean; cache_type_k: string; cache_type_v: string;
  draft_model_path: string; draft_gpu_layers: number; draft_tokens: number;
  spec_draft_n_min: number; spec_type: string;
  host: string; port: number; api_key: string;
  ssl_key_file: string; ssl_cert_file: string;
  no_ui: boolean; embedding: boolean; pooling: string; reranking: boolean;
  n_predict: number; ignore_eos: boolean; json_schema: string;
  temp: number; top_k: number; top_p: number; repeat_penalty: number;
  seed: number; min_p: number; presence_penalty: number;
  frequency_penalty: number; repeat_last_n: number;
  mirostat: number; mirostat_lr: number; mirostat_ent: number;
  xtc_probability: number; xtc_threshold: number;
  dynatemp_range: number; dynatemp_exp: number; typical_p: number;
  dry_multiplier: number; dry_base: number;
  dry_allowed_length: number; dry_penalty_last_n: number;
  dry_sequence_breaker: string;
  timeout: number; sleep_idle: number; context_shift: boolean;
  verbose: boolean; custom_args: string[];
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
    lora_path: '', mmproj_path: '', chat_template: '',
    reasoning_format: '', reasoning_effort: '', reasoning: '',
    jinja: false, reasoning_budget: '', grammar_file: '',
    ctx_size: 4096, ctx_size_auto: false, gpu_layers: 99,
    threads: 0, batch_size: 2048, ubatch_size: 512, parallel: 1,
    cont_batching: false, cache_prompt: true, threads_batch: 0,
    flash_attn: 'auto', moe_cpu_layers: 0, mlock: false,
    no_mmap: false, numa: false, cache_type_k: '', cache_type_v: '',
    draft_model_path: '', draft_gpu_layers: 99, draft_tokens: 16,
    spec_draft_n_min: 0, spec_type: '',
    host: '127.0.0.1', port: 8080, api_key: '',
    ssl_key_file: '', ssl_cert_file: '',
    no_ui: false, embedding: false, pooling: '', reranking: false,
    n_predict: -1, ignore_eos: false, json_schema: '',
    temp: 0.8, top_k: 40, top_p: 0.9, repeat_penalty: 1.1,
    seed: -1, min_p: 0.05, presence_penalty: 0,
    frequency_penalty: 0, repeat_last_n: 64,
    mirostat: 0, mirostat_lr: 0, mirostat_ent: 0,
    xtc_probability: 0, xtc_threshold: 0,
    dynatemp_range: 0, dynatemp_exp: 0, typical_p: 1,
    dry_multiplier: 0, dry_base: 0,
    dry_allowed_length: 0, dry_penalty_last_n: 0,
    dry_sequence_breaker: '',
    timeout: 600, sleep_idle: -1, context_shift: false,
    verbose: false, custom_args: [],
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
        running: Record<string, { instance_id: string; pid: number; port: number; host: string }>
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
      if ((global.model_dirs || []).length > 0) {
        invoke('scan_models', { paths: global.model_dirs }).then((models) => set({ models: models as any })).catch(() => {})
      }
      if ((global.engine_dirs || []).length > 0) {
        invoke('scan_engines', { paths: global.engine_dirs }).then((engines) => set({ engines: engines as any })).catch(() => {})
      }
      // 为恢复的运行中实例启动健康检查
      for (const id of runningIds) {
        const ri = global.running[id]
        if (ri) {
          // 通过调用空操作触发后端健康检查，或直接在前端模拟
          setTimeout(() => {
            get().updateInstance(id, { startTime: Date.now() })
          }, 100)
        }
      }
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
