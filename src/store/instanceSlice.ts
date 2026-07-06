import { invoke } from '@tauri-apps/api/core'
import { message } from '@tauri-apps/plugin-dialog'
import { loadAppBootstrap } from './bootstrap'
import type { AppStoreGet, AppStoreSet } from './helpers'
import type { AppState, InstanceConfig, LogEntry } from './types'

const MAX_LOG_ENTRIES = 1000

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
      get().saveConfig()
    },
    updateInstance: (id, partial) => set((state) => ({
      instances: state.instances.map((instance) => (
        instance.id === id ? { ...instance, ...partial } : instance
      )),
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
      get().saveConfig()
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
      get().saveConfig()
    },
    addLog: (entry: LogEntry) => set((state) => {
      const existing = state.logs[entry.instanceId] || []
      const updated = [...existing.slice(-(MAX_LOG_ENTRIES - 1)), { ...entry, timestamp: Date.now() }]
      return { logs: { ...state.logs, [entry.instanceId]: updated } }
    }),
    clearLogs: (instanceId) => set((state) => ({
      logs: { ...state.logs, [instanceId]: [] },
    })),
    generateCommand: async (config: InstanceConfig, engineExe: string) => (
      invoke<string[]>('generate_server_command', { config, engineExe })
    ),
    startInstance: async (id) => {
      try {
        const { instances, engines, defaultEngineId } = get()
        const instance = instances.find((item) => item.id === id)
        if (!instance) {
          message('Instance not found.', { title: 'Error', kind: 'error' })
          return
        }

        const engine = engines.find((item) => item.id === instance.config.engine_id)
          || engines.find((item) => item.id === defaultEngineId)
          || engines[0]

        if (!engine) {
          message('No llama-server engine available.\n\nPlease scan engines first.', { title: 'Error', kind: 'error' })
          return
        }

        await invoke('start_server', {
          instanceId: id,
          config: instance.config,
          engineExe: engine.exe,
          engineBackend: engine.backend,
        })
        get().updateInstance(id, { status: 'running', healthCheck: 'pending' })
      } catch (error) {
        console.error('start_server error:', error)
      }
    },
    stopInstance: async (id) => {
      try {
        await invoke('stop_server', { instanceId: id })
        get().updateInstance(id, { status: 'stopped', healthCheck: 'pending' })
      } catch (error) {
        console.error('stop_server error:', error)
      }
    },
    openBrowser: async (host, port) => {
      await invoke('open_browser', { host, port })
    },
    saveConfig: async () => {
      const { instances, modelDirs, engineDirs, defaultEngineId, activeTab, darkMode } = get()
      const instancesById: Record<string, InstanceConfig> = {}
      const order: string[] = []

      instances.forEach((instance) => {
        instancesById[instance.id] = instance.config
        order.push(instance.id)
      })

      await invoke('save_config', {
        instances: instancesById,
        modelDirs,
        engineDirs,
        defaultEngineId: defaultEngineId || '',
        instanceOrder: order,
        lastTab: activeTab,
        darkMode,
      })
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
