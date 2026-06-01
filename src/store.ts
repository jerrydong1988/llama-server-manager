import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

// Re-exports from split modules
export type { ModelInfo, EngineInfo, InstanceConfig, Instance, LogEntry, MsFileEntry, DownloadProgress, AppState } from './store/types'
export { defaultInstanceConfig } from './store/defaults'
import type { AppState, ModelInfo, EngineInfo, InstanceConfig, Instance, LogEntry, MsFileEntry } from './store/types'
import { defaultInstanceConfig } from './store/defaults'

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

listen<{ instanceId: string; pid: number; port: number; command: string }>('server-started', (event) => {
  const state = useAppStore.getState()
  state.updateInstance(event.payload.instanceId, {
    status: 'running',
    healthCheck: 'pending',
    startTime: Date.now(),
  })
  state.addLog({
    instanceId: event.payload.instanceId,
    text: `\u250C\u2500\u2500 \u542F\u52A8\u547D\u4EE4\n\u2502 ${event.payload.command}\n\u2514\u2500\u2500 PID: ${event.payload.pid} | \u7AEF\u53E3: ${event.payload.port}`,
    timestamp: Date.now(),
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
