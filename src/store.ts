import { create } from 'zustand'
import { createClusterSlice } from './store/clusterSlice'
import { createCoreSlice } from './store/coreSlice'
import { createDownloadSlice } from './store/downloadSlice'
import { createInstanceSlice } from './store/instanceSlice'
import { registerGlobalStoreListeners } from './store/runtimeEvents'
import type { AppState, SystemMetrics } from './store/types'

export const _startupTimings: { name: string; ms: number }[] = []

export type {
  ModelInfo,
  EngineInfo,
  InstanceConfig,
  Instance,
  LogEntry,
  MsFileEntry,
  DownloadProgress,
  AppState,
  SystemMetrics,
  WorkerInfo,
  WorkerDevice,
  WorkerStatus,
  Usb4Adapter,
} from './store/types'
export { defaultInstanceConfig } from './store/defaults'
export { formatStartupCommand } from './store/commandFormatting'

export const useAppStore = create<AppState>((set, get) => ({
  models: [],
  engines: [],
  instances: [],
  logs: {},
  isLoading: false,
  runtimeWarnings: [],
  darkMode: true,
  defaultEngineId: null,
  modelDirs: [],
  engineDirs: [],
  activeConfigInstanceId: null,
  activeTab: (() => {
    try {
      return localStorage.getItem('lastTab') || 'dashboard'
    } catch {
      return 'dashboard'
    }
  })(),
  workers: [],
  clusterScanning: false,
  downloadTasks: {},
  downloadQueue: [],
  sysMetrics: null as SystemMetrics | null,
  ...createCoreSlice(set, get),
  ...createInstanceSlice(set, get, _startupTimings),
  ...createDownloadSlice(set, get),
  ...createClusterSlice(set),
}))

registerGlobalStoreListeners(useAppStore, _startupTimings)
