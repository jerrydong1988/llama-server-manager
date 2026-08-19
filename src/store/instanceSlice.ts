import { invokeApp as invoke } from '../lib/ipc'
import { message } from '@tauri-apps/plugin-dialog'
import {
  loadAppBootstrap,
  normalizeStoredConfig,
  reconcileInstancesWithModels,
} from './bootstrap'
import { createLatestSaveCoordinator } from './configSaveCoordinator'
import type { AppStoreGet, AppStoreSet } from './helpers'
import { runInstanceStart, runInstanceStop } from './instanceLifecycleCoordinator'
import { synchronizeInstanceSummary } from './instanceSummary'
import type {
  AppState,
  ConfigRevisionHistory,
  ConfigRevisionRollbackResponse,
  GeneratedServerCommand,
  InstanceConfig,
  LogEntry,
} from './types'
import { resolveEffectiveEngine } from './engineResolution'
import { pathsEqual } from '../utils/path'
import { beginOperationTiming, type OperationOutcome } from '../operationTiming'

const MAX_LOG_ENTRIES = 1000
const MAX_RECENT_LOG_ENTRIES = 2000

const mergeRecentLogs = (existing: LogEntry[], incoming: LogEntry[]) => {
  const sortedIncoming = incoming.length > 1
    ? [...incoming].sort((left, right) => left.timestamp - right.timestamp)
    : incoming
  const merged: LogEntry[] = []
  let existingIndex = 0
  let incomingIndex = 0
  while (existingIndex < existing.length && incomingIndex < sortedIncoming.length) {
    if (existing[existingIndex].timestamp <= sortedIncoming[incomingIndex].timestamp) {
      merged.push(existing[existingIndex++])
    } else {
      merged.push(sortedIncoming[incomingIndex++])
    }
  }
  merged.push(...existing.slice(existingIndex), ...sortedIncoming.slice(incomingIndex))
  return merged.slice(-MAX_RECENT_LOG_ENTRIES)
}

const isStaleEngineCapabilityError = (error: unknown) => (
  Boolean(error && typeof error === 'object' && [
    'ENGINE_CAPABILITIES_STALE',
    'ENGINE_QUALIFICATION_STALE',
  ].includes((error as { code?: string }).code || ''))
)

const invalidateEngineCapabilities = (set: AppStoreSet, engineExe: string) => {
  set(state => ({
    engines: state.engines.map(engine => {
      if (!pathsEqual(engine.exe, engineExe)) return engine
      const qualification = engine.capabilities?.qualification
      return {
        ...engine,
        version: '',
        capabilities: {
          status: 'unprobed',
          versionStatus: 'unprobed',
          supportedFlags: [],
          helpHash: '',
          executableFingerprint: '',
          error: 'Engine executable changed; compatibility probe and qualification required.',
          qualification: qualification && qualification.status !== 'unqualified'
            ? {
                ...qualification,
                status: 'stale',
                invalidatedAt: Math.floor(Date.now() / 1000),
                diagnostic: 'Engine executable changed; qualification required.',
              }
            : qualification,
        },
      }
    }),
  }))
}
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
  | 'listConfigRevisions'
  | 'markConfigRevisionKnownGood'
  | 'rollbackConfigRevision'
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
    moveInstance: (id, direction, orderedIds) => {
      const state = get()
      const index = state.instances.findIndex((instance) => instance.id === id)
      if (index < 0) return

      const order = orderedIds?.filter(candidateId => state.instances.some(instance => instance.id === candidateId))
        ?? state.instances.map(instance => instance.id)
      const visibleIndex = order.indexOf(id)
      if (visibleIndex < 0) return
      const targetId = order[direction === 'up' ? visibleIndex - 1 : visibleIndex + 1]
      const target = state.instances.findIndex(instance => instance.id === targetId)
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
      const normalizedEntries = entries.map(entry => ({
        ...entry,
        timestamp: entry.timestamp || Date.now(),
      }))
      const grouped = new Map<string, LogEntry[]>()
      for (const entry of normalizedEntries) {
        const group = grouped.get(entry.instanceId) || []
        group.push(entry)
        grouped.set(entry.instanceId, group)
      }
      const logs = { ...state.logs }
      for (const [instanceId, group] of grouped) {
        const existing = logs[instanceId] || []
        logs[instanceId] = [...existing, ...group].slice(-MAX_LOG_ENTRIES)
      }
      return {
        logs,
        recentLogs: mergeRecentLogs(state.recentLogs, normalizedEntries),
      }
    }),
    clearLogs: (instanceId) => set((state) => ({
      logs: { ...state.logs, [instanceId]: [] },
      recentLogs: state.recentLogs.filter(entry => entry.instanceId !== instanceId),
    })),
    generateCommand: async (config: InstanceConfig, engineExe: string) => {
      const normalized = normalizeStoredConfig(config, get().models)
      const matchingEngine = get().engines.find(engine => pathsEqual(engine.exe, engineExe))
      if (!normalized.config.engine_id && matchingEngine) {
        normalized.config = { ...normalized.config, engine_id: matchingEngine.id }
      }
      try {
        return await invoke<GeneratedServerCommand>('generate_server_command', { config: normalized.config, engineExe })
      } catch (error) {
        if (isStaleEngineCapabilityError(error)) invalidateEngineCapabilities(set, engineExe)
        throw error
      }
    },
    startInstance: (id, manualRecovery = true) => runInstanceStart(id, async () => {
      const timing = beginOperationTiming('instance.start')
      let outcome: OperationOutcome = 'failure'
      set(state => ({
        instanceLifecycle: { ...state.instanceLifecycle, [id]: 'starting' },
      }))
      try {
        const { instances, models, engines, defaultEngineId } = get()
        const instance = instances.find((item) => item.id === id)
        if (!instance) {
          message('Instance not found.', { title: 'Error', kind: 'error' })
          outcome = 'cancelled'
          return
        }

        const normalized = normalizeStoredConfig(instance.config, models)
        const engine = resolveEffectiveEngine(normalized.config, engines, defaultEngineId)
        timing.mark('prepare')

        if (!engine) {
          message(normalized.config.engine_id
            ? 'The configured llama-server engine is no longer available. Select another engine before starting.'
            : 'No llama-server engine available.\n\nPlease scan engines first.', { title: 'Error', kind: 'error' })
          outcome = 'cancelled'
          return
        }
        if (!normalized.config.engine_id) {
          normalized.config = { ...normalized.config, engine_id: engine.id }
        }
        if (normalized.changes.length > 0 || !pathsEqual(normalized.config.engine_id, instance.config.engine_id)) {
          set((state) => ({
            instances: state.instances.map((item) => (
              item.id === id ? { ...item, config: normalized.config } : item
            )),
          }))
          await get().saveConfig()
        }
        timing.mark('normalize-and-save')
        await configSaveCoordinator.waitForIdle()
        timing.mark('save-queue')

        await invoke('start_server', {
          instanceId: id,
          config: normalized.config,
          engineExe: engine.exe,
          engineBackend: engine.backend,
          manualRecovery,
        })
        timing.mark('backend-start')
        get().updateInstance(id, { status: 'running', healthCheck: 'pending' })
        outcome = 'success'
      } catch (error) {
        if (isStaleEngineCapabilityError(error)) {
          const state = get()
          const instance = state.instances.find(item => item.id === id)
          const engine = instance
            ? resolveEffectiveEngine(instance.config, state.engines, state.defaultEngineId)
            : null
          if (engine) invalidateEngineCapabilities(set, engine.exe)
        }
        console.error('start_server error:', error)
        get().addRuntimeWarning(`\u5b9e\u4f8b\u542f\u52a8\u5931\u8d25\uff1a${String(error)}`)
        throw error
      } finally {
        set(state => {
          if (state.instanceLifecycle[id] !== 'starting') return state
          const instanceLifecycle = { ...state.instanceLifecycle }
          delete instanceLifecycle[id]
          return { instanceLifecycle }
        })
        timing.finish(outcome)
      }
    }),
    stopInstance: (id) => runInstanceStop(id, async () => {
      const timing = beginOperationTiming('instance.stop')
      let outcome: OperationOutcome = 'failure'
      set(state => ({
        instanceLifecycle: { ...state.instanceLifecycle, [id]: 'stopping' },
      }))
      try {
        await invoke('stop_server', { instanceId: id })
        timing.mark('backend-stop')
        get().updateInstance(id, { status: 'stopped', healthCheck: 'pending', recovery: undefined })
        outcome = 'success'
      } catch (error) {
        console.error('stop_server error:', error)
        get().addRuntimeWarning(`\u5b9e\u4f8b\u505c\u6b62\u5931\u8d25\uff1a${String(error)}`)
        throw error
      } finally {
        set(state => {
          if (state.instanceLifecycle[id] !== 'stopping') return state
          const instanceLifecycle = { ...state.instanceLifecycle }
          delete instanceLifecycle[id]
          return { instanceLifecycle }
        })
        timing.finish(outcome)
      }
    }),
    openBrowser: async (instanceId, host, port, useTls = false, apiPrefix = '') => {
      await invoke('open_browser', { instanceId, host, port, useTls, apiPrefix })
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
    listConfigRevisions: async (instanceId) => {
      await configSaveCoordinator.waitForIdle()
      return invoke<ConfigRevisionHistory>('list_config_revisions', { instanceId })
    },
    markConfigRevisionKnownGood: async (instanceId, revisionId, expectedCurrentFingerprint) => {
      await configSaveCoordinator.waitForIdle()
      return invoke<ConfigRevisionHistory>('mark_config_revision_known_good', {
        instanceId,
        revisionId,
        expectedCurrentFingerprint,
      })
    },
    rollbackConfigRevision: async (instanceId, revisionId, expectedCurrentFingerprint) => {
      await configSaveCoordinator.waitForIdle()
      set(state => ({
        instanceLifecycle: { ...state.instanceLifecycle, [instanceId]: 'rolling_back' },
      }))
      try {
        const result = await invoke<ConfigRevisionRollbackResponse>('rollback_config_revision', {
          instanceId,
          revisionId,
          expectedCurrentFingerprint,
        })
        set(state => ({
          instances: state.instances.map(instance => (
            instance.id === instanceId
              ? synchronizeInstanceSummary({ ...instance, config: result.config })
              : instance
          )),
        }))
        return result
      } catch (error) {
        get().addRuntimeWarning(`配置回滚失败：${String(error)}`)
        throw error
      } finally {
        set(state => {
          if (state.instanceLifecycle[instanceId] !== 'rolling_back') return state
          const instanceLifecycle = { ...state.instanceLifecycle }
          delete instanceLifecycle[instanceId]
          return { instanceLifecycle }
        })
      }
    },
  }
}
