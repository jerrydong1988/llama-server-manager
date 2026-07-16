import { invokeApp as invoke } from '../lib/ipc'
import { message } from '@tauri-apps/plugin-dialog'
import {
  loadAppBootstrap,
  normalizeStoredConfig,
  reconcileInstancesWithModels,
} from './bootstrap'
import { createLatestSaveCoordinator } from './configSaveCoordinator'
import type { AppStoreGet, AppStoreSet } from './helpers'
import { runInstanceStart } from './instanceLifecycleCoordinator'
import { synchronizeInstanceSummary } from './instanceSummary'
import type { AppState, InstanceConfig, LogEntry } from './types'

const MAX_LOG_ENTRIES = 1000
type ConfigSaveSnapshot = {
  revision: number
  instances: Record<string, InstanceConfig>
  modelDirs: string[]
  engineDirs: string[]
  defaultEngineId: string
  instanceOrder: string[]
  lastTab: string
  darkMode: boolean
}

type PersistedConfigResult = {
  revision: number
  instances: Record<string, InstanceConfig>
}

let latestConfigSaveRevision = 0
let latestAppliedConfigSaveRevision = 0

const configSaveCoordinator = createLatestSaveCoordinator<ConfigSaveSnapshot, PersistedConfigResult>(
  async ({ revision, ...snapshot }) => ({
    revision,
    instances: await invoke<Record<string, InstanceConfig>>('save_config', snapshot),
  }),
)

export function createInstanceSlice(
  set: AppStoreSet,
  get: AppStoreGet,
  startupTimings: { name: string; ms: number }[],
): Pick<
  AppState,
  | 'addInstance'
  | 'updateInstance'
  | 'deleteInstance'
  | 'moveInstance'
  | 'renameInstance'
  | 'addLog'
  | 'addLogs'
  | 'clearLogs'
  | 'generateCommand'
  | 'startInstance'
  | 'stopInstance'
  | 'openBrowser'
  | 'saveConfig'
  | 'loadConfig'
> {
  return {
    addInstance: (instance) => {
      set((state) => ({ instances: [...state.instances, instance] }))
      void get().saveConfig().catch(() => {})
    },
    updateInstance: (id, partial) => set((state) => ({
      instances: state.instances.map((instance) => {
        if (instance.id !== id) return instance
        return synchronizeInstanceSummary({ ...instance, ...partial })
      }),
    })),
    deleteInstance: (id) => set((state) => ({
      instances: state.instances.filter((instance) => instance.id !== id),
    })),
    moveInstance: (id, direction) => {
      const state = get()
      const index = state.instances.findIndex((instance) => instance.id === id)
      if (index < 0) return

      const target = direction === 'up' ? index - 1 : index + 1
      if (target < 0 || target >= state.instances.length) return

      const next = [...state.instances]
      ;[next[index], next[target]] = [next[target], next[index]]
      set({ instances: next })
      void get().saveConfig().catch(() => {})
    },
    renameInstance: (id, name) => {
      const state = get()
      const instance = state.instances.find((item) => item.id === id)
      if (!instance) return

      const config = { ...instance.config, name }
      set({
        instances: state.instances.map((item) => (
          item.id === id ? { ...item, name, config } : item
        )),
      })
      void get().saveConfig().catch(() => {})
    },
    addLog: (entry: LogEntry) => get().addLogs([entry]),
    addLogs: (entries: LogEntry[]) => set((state) => {
      if (entries.length === 0) return state
      const grouped = new Map<string, LogEntry[]>()
      for (const entry of entries) {
        const group = grouped.get(entry.instanceId) || []
        group.push({ ...entry, timestamp: entry.timestamp || Date.now() })
        grouped.set(entry.instanceId, group)
      }
      const logs = { ...state.logs }
      for (const [instanceId, group] of grouped) {
        const existing = logs[instanceId] || []
        logs[instanceId] = [...existing, ...group].slice(-MAX_LOG_ENTRIES)
      }
      return { logs }
    }),
    clearLogs: (instanceId) => set((state) => ({
      logs: { ...state.logs, [instanceId]: [] },
    })),
    generateCommand: async (config: InstanceConfig, engineExe: string) => {
      const normalized = normalizeStoredConfig(config, get().models)
      return invoke<string[]>('generate_server_command', { config: normalized.config, engineExe })
    },
    startInstance: (id) => runInstanceStart(id, async () => {
      try {
        const { instances, models, engines, defaultEngineId } = get()
        const instance = instances.find((item) => item.id === id)
        if (!instance) {
          message('Instance not found.', { title: 'Error', kind: 'error' })
          return
        }

        const normalized = normalizeStoredConfig(instance.config, models)
        if (normalized.changes.length > 0) {
          set((state) => ({
            instances: state.instances.map((item) => (
              item.id === id ? { ...item, config: normalized.config } : item
            )),
          }))
          await get().saveConfig()
        }
        await configSaveCoordinator.waitForIdle()

        const engine = engines.find((item) => item.id === normalized.config.engine_id)
          || engines.find((item) => item.id === defaultEngineId)
          || engines[0]

        if (!engine) {
          message('No llama-server engine available.\n\nPlease scan engines first.', { title: 'Error', kind: 'error' })
          return
        }

        await invoke('start_server', {
          instanceId: id,
          config: normalized.config,
          engineExe: engine.exe,
          engineBackend: engine.backend,
        })
        get().updateInstance(id, { status: 'running', healthCheck: 'pending' })
      } catch (error) {
        console.error('start_server error:', error)
        get().addRuntimeWarning(`\u5b9e\u4f8b\u542f\u52a8\u5931\u8d25\uff1a${String(error)}`)
        throw error
      }
    }),
    stopInstance: async (id) => {
      try {
        await invoke('stop_server', { instanceId: id })
        get().updateInstance(id, { status: 'stopped', healthCheck: 'pending' })
      } catch (error) {
        console.error('stop_server error:', error)
        get().addRuntimeWarning(`\u5b9e\u4f8b\u505c\u6b62\u5931\u8d25\uff1a${String(error)}`)
        throw error
      }
    },
    openBrowser: async (host, port) => {
      await invoke('open_browser', { host, port })
    },
    saveConfig: async () => {
      const state = get()
      const reconciled = reconcileInstancesWithModels(state.instances, state.models)
      if (reconciled.changed) set({ instances: reconciled.instances })

      const instancesById: Record<string, InstanceConfig> = {}
      const order: string[] = []

      reconciled.instances.forEach((instance) => {
        instancesById[instance.id] = instance.config
        order.push(instance.id)
      })

      const revision = ++latestConfigSaveRevision
      const operation = configSaveCoordinator.save({
        revision,
        instances: instancesById,
        modelDirs: state.modelDirs,
        engineDirs: state.engineDirs,
        defaultEngineId: state.defaultEngineId || '',
        instanceOrder: order,
        lastTab: state.activeTab,
        darkMode: state.darkMode,
      }).then((result) => {
        if (
          result.revision === latestConfigSaveRevision
          && result.revision > latestAppliedConfigSaveRevision
          && Object.keys(result.instances).length > 0
        ) {
          latestAppliedConfigSaveRevision = result.revision
          set((current) => ({
            instances: current.instances.map((instance) => {
              const persistedConfig = result.instances[instance.id]
              return persistedConfig
                ? synchronizeInstanceSummary({ ...instance, config: persistedConfig })
                : instance
            }),
          }))
        }
      }).catch((error) => {
        if (revision === latestConfigSaveRevision) {
          get().addRuntimeWarning(`配置保存失败：${String(error)}`)
        }
        throw error
      })
      return operation
    },
    loadConfig: async () => {
      await loadAppBootstrap(
        (partial) => set(partial),
        () => get(),
        startupTimings,
      )
    },
  }
}
