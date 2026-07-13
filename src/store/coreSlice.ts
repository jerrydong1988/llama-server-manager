import { invoke } from '@tauri-apps/api/core'
import { applyModelInventory, beginModelInventoryRequest } from './bootstrap'
import type { AppStoreGet, AppStoreSet } from './helpers'
import type { AppState, EngineInfo, GgufMetadataSummary, ModelInfo } from './types'

export function createCoreSlice(set: AppStoreSet, get: AppStoreGet): Pick<
  AppState,
  | 'setSysMetrics'
  | 'addRuntimeWarning'
  | 'clearRuntimeWarnings'
  | 'setModels'
  | 'setEngines'
  | 'setModelDirs'
  | 'setEngineDirs'
  | 'setDefaultEngineId'
  | 'setActiveConfigInstanceId'
  | 'setActiveTab'
  | 'setDarkMode'
  | 'loadInitialData'
  | 'scanModels'
  | 'deleteModelFile'
  | 'openModelFolder'
  | 'readGgufMetadata'
  | 'scanEngines'
  | 'deleteEngine'
  | 'renameEngine'
  | 'openEngineFolder'
> {
  return {
    setSysMetrics: (metrics) => set({ sysMetrics: metrics }),
    addRuntimeWarning: (message) => {
      const trimmed = String(message || '').trim()
      if (!trimmed) return
      set((state) => ({
        runtimeWarnings: [trimmed, ...state.runtimeWarnings.filter(item => item !== trimmed)].slice(0, 8),
      }))
    },
    clearRuntimeWarnings: () => set({ runtimeWarnings: [] }),
    setModels: (models) => {
      const requestGeneration = beginModelInventoryRequest()
      applyModelInventory(models, get, set, { isLoading: false }, requestGeneration)
    },
    setEngines: (engines) => set({ engines }),
    setModelDirs: (dirs) => {
      set({ modelDirs: dirs })
      void get().saveConfig().catch(() => {})
    },
    setEngineDirs: (dirs) => {
      set({ engineDirs: dirs })
      void get().saveConfig().catch(() => {})
    },
    setDefaultEngineId: (id) => {
      set({ defaultEngineId: id })
      void get().saveConfig().catch(() => {})
    },
    setActiveConfigInstanceId: (id) => set({ activeConfigInstanceId: id }),
    setActiveTab: (tab) => {
      set({ activeTab: tab })
      try {
        localStorage.setItem('lastTab', tab)
      } catch {}
    },
    setDarkMode: (darkMode) => {
      set({ darkMode })
      document.documentElement.classList.toggle('dark', darkMode)
      void get().saveConfig().catch(() => {})
    },
    loadInitialData: async () => {
      set({ isLoading: true })
      const requestGeneration = beginModelInventoryRequest()
      try {
        const [models, engines] = await Promise.all([
          invoke<ModelInfo[]>('get_models').catch(() => [] as ModelInfo[]),
          invoke<EngineInfo[]>('get_engines').catch(() => [] as EngineInfo[]),
        ])
        applyModelInventory(models, get, set, { engines, isLoading: false }, requestGeneration)
      } catch {
        set({ isLoading: false })
      }
    },
    scanModels: async (paths) => {
      set({ isLoading: true })
      const requestGeneration = beginModelInventoryRequest()
      try {
        const models = await invoke<ModelInfo[]>('scan_models', { paths })
        applyModelInventory(models, get, set, { modelDirs: paths, isLoading: false }, requestGeneration)
        return null
      } catch (error: any) {
        console.error('scan_models error:', error)
        set({ isLoading: false })
        return error?.message || error?.toString() || 'Scan failed'
      }
    },
    deleteModelFile: async (path) => {
      await invoke('delete_model_file', { path })
      set((state) => ({ models: state.models.filter((model) => model.path !== path) }))
    },
    openModelFolder: async (path) => {
      await invoke('open_model_folder', { path })
    },
    readGgufMetadata: async (path) => invoke<GgufMetadataSummary>('read_gguf_metadata', { path }),
    scanEngines: async (paths) => {
      set({ isLoading: true })
      try {
        const engines = await invoke<EngineInfo[]>('scan_engines', { paths })
        set({ engines, engineDirs: paths, isLoading: false })
      } catch (error) {
        console.error('scan_engines error:', error)
        set({ isLoading: false })
      }
    },
    deleteEngine: async (id) => {
      await invoke('delete_engine', { id })
      set((state) => ({ engines: state.engines.filter((engine) => engine.id !== id) }))
    },
    renameEngine: (id, name) => {
      invoke('rename_engine', { id, name }).catch(() => {})
      set((state) => ({
        engines: state.engines.map((engine) => (
          engine.id === id ? { ...engine, name, custom_name: name } : engine
        )),
      }))
    },
    openEngineFolder: async (dir) => {
      await invoke('open_engine_folder', { dir })
    },
  }
}
