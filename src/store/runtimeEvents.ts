import { invokeApp as invoke } from '../lib/ipc'
import { listen, type Event } from '@tauri-apps/api/event'
import type { StoreApi } from 'zustand'
import { formatStartupCommand } from './commandFormatting'
import type {
  AppState,
  DownloadProgress,
  InstanceConfig,
  MonitoringFrame,
  PerfUpdateEvent,
  SystemMetrics,
} from './types'

type StoreLike = Pick<StoreApi<AppState>, 'getState' | 'setState'>

type DownloadEventPayload = {
  queueId?: string
  taskId?: string
  runId?: string
  version?: number
  fileName: string
  repoId: string
  source: string
  remotePath?: string
  downloaded?: number
  total?: number
  speed?: number
  path?: string
  error?: string
}

declare global {
  interface Window {
    __lsm_listener_registry?: Record<string, 'pending' | 'registered'>
    __lsm_listener_retries?: Record<string, ReturnType<typeof setTimeout>>
  }
}

const lastProgressUpdate: Record<string, number> = {}
let scanModelDebounce: ReturnType<typeof setTimeout> | null = null
let sysMetricsTimer: ReturnType<typeof setTimeout> | null = null
let sysMetricsInFlight = false
let runtimeManagedIds = new Set<string>()
let lastReportedRuntimeStatusError: string | null = null

const runMatches = (payload: { runId?: string }, task: DownloadProgress) => (
  payload.runId ? task.runId === payload.runId : !task.runId
)

const versionMatches = (payload: { version?: number }, task: DownloadProgress) => (
  task.version === undefined
  || (payload.version !== undefined && task.version === payload.version)
)

function taskIdFromEvent(
  payload: DownloadEventPayload,
  tasks: Record<string, DownloadProgress>,
) {
  if (payload.taskId) {
    const task = tasks[payload.taskId]
    if (!task) return undefined
    if (!versionMatches(payload, task)) return undefined
    if (!task.runId || runMatches(payload, task)) return payload.taskId
    return undefined
  }

  return Object.values(tasks).find((task) => (
    task.fileName === payload.fileName
    && task.repoId === payload.repoId
    && task.source === payload.source
    && (!payload.remotePath || task.remotePath === payload.remotePath)
    && versionMatches(payload, task)
    && (!task.runId || runMatches(payload, task))
  ))?.id
}

function applyDownloadPatch(
  store: StoreLike,
  payload: DownloadEventPayload,
  patch: Partial<DownloadProgress>,
) {
  const state = store.getState()
  const taskId = taskIdFromEvent(payload, state.downloadTasks)
  if (!taskId) return undefined

  const prev = state.downloadTasks[taskId]
  const now = Date.now()
  const next: DownloadProgress = {
    ...(prev || {}),
    id: taskId,
    runId: payload.runId || prev?.runId,
    fileName: payload.fileName,
    remotePath: payload.remotePath || prev?.remotePath || payload.fileName,
    fileType: prev?.fileType || 'model',
    saveDir: prev?.saveDir || 'models',
    repoId: payload.repoId,
    source: payload.source,
    downloaded: payload.downloaded ?? prev?.downloaded ?? 0,
    total: payload.total ?? prev?.total ?? 0,
    createdAt: prev?.createdAt ?? now,
    updatedAt: now,
    completedAt: patch.status === 'completed' ? prev?.completedAt ?? now : prev?.completedAt,
    speed: payload.speed ?? prev?.speed ?? 0,
    status: prev?.status || 'queued',
    version: payload.version ?? prev?.version ?? 0,
    ...patch,
  }

  if (next.status === 'completed' && next.total > 0) {
    next.downloaded = next.total
  }

  state.setDownloadTasks({ ...state.downloadTasks, [taskId]: next })
  return next
}

function startSysMetricsPolling(store: StoreLike) {
  if (sysMetricsTimer) return

  const fetchSysMetrics = async () => {
    if (sysMetricsInFlight) return
    sysMetricsInFlight = true
    try {
      const metrics = await invoke<SystemMetrics>('get_system_health')
      store.getState().setSysMetrics(metrics)
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      store.getState().addRuntimeWarning(`system metrics polling failed: ${message}`)
    } finally {
      sysMetricsInFlight = false
      const delay = document.visibilityState === 'hidden' ? 15_000 : 5_000
      sysMetricsTimer = setTimeout(fetchSysMetrics, delay)
    }
  }

  sysMetricsTimer = setTimeout(fetchSysMetrics, 0)
}

function warnListenerFailure(store: StoreLike, eventName: string, error: unknown) {
  const message = error instanceof Error ? error.message : String(error)
  store.getState().addRuntimeWarning(`listener "${eventName}" failed to register: ${message}`)
}

function warnAsyncFailure(store: StoreLike, operation: string, error: unknown) {
  const message = error instanceof Error ? error.message : String(error)
  store.getState().addRuntimeWarning(`${operation} failed: ${message}`)
}

const LISTENER_RETRY_DELAYS = [250, 1_000, 3_000, 10_000, 30_000]

function registerListener<T>(
  store: StoreLike,
  eventName: string,
  handler: (event: Event<T>) => void,
  attempt = 0,
) {
  const globalWindow = window as Window & typeof globalThis
  const registry = globalWindow.__lsm_listener_registry ||= {}
  globalWindow.__lsm_listener_retries ||= {}
  if (registry[eventName]) return

  registry[eventName] = 'pending'
  void listen<T>(eventName, handler)
    .then(() => {
      registry[eventName] = 'registered'
      const retry = globalWindow.__lsm_listener_retries?.[eventName]
      if (retry) clearTimeout(retry)
      delete globalWindow.__lsm_listener_retries?.[eventName]
    })
    .catch((error) => {
      delete registry[eventName]
      warnListenerFailure(store, eventName, error)
      if (attempt >= LISTENER_RETRY_DELAYS.length) return
      globalWindow.__lsm_listener_retries![eventName] = setTimeout(
        () => registerListener(store, eventName, handler, attempt + 1),
        LISTENER_RETRY_DELAYS[attempt],
      )
    })
}

export function registerGlobalStoreListeners(
  store: StoreLike,
  startupTimings: { name: string; ms: number }[],
) {
  const classifyLogLevel = (text: string): 'info' | 'warning' | 'error' => {
    const normalized = text.trim().toLowerCase()
    if (/\b(?:no errors?|errors?\s*[=:]\s*0|failed\s*[=:]\s*0)\b/.test(normalized)
      || /(?:无错误|错误\s*[=:]\s*0|失败\s*[=:]\s*0)/.test(normalized)) return 'info'
    if (/(?:^|[\s[({:])(fatal|panic|exception|error|failed)(?:[\s\])}:]|$)/.test(normalized)
      || /(?:错误|失败|异常)/.test(normalized)) return 'error'
    if (/(?:^|[\s[({:])(warn|warning)(?:[\s\])}:]|$)/.test(normalized)
      || /(?:警告|提醒)/.test(normalized)) return 'warning'
    return 'info'
  }
  registerListener<{ name: string; ms: number }>(store, 'startup-timing', (event) => {
    startupTimings.push({ name: event.payload.name, ms: event.payload.ms })
  })

  registerListener<{
    running: Record<string, { pid: number; host: string; port: number; startTime: number }>
    previouslyManaged?: string[]
    lastError?: string | null
  }>(store, 'runtime-service-status', (event) => {
    const running = event.payload.running || {}
    const nextManagedIds = new Set(Object.keys(running))
    const previousManagedIds = new Set([
      ...runtimeManagedIds,
      ...(event.payload.previouslyManaged || []),
    ])
    runtimeManagedIds = nextManagedIds
    store.setState(state => ({
      instances: state.instances.map(instance => {
        const runtime = running[instance.id]
        if (runtime) {
          return {
            ...instance,
            status: 'running',
            startTime: runtime.startTime > 0 ? runtime.startTime * 1000 : instance.startTime,
          }
        }
        if (previousManagedIds.has(instance.id)) {
          return {
            ...instance,
            status: 'stopped',
            healthCheck: 'pending',
          }
        }
        return instance
      }),
    }))
    const nextRuntimeError = event.payload.lastError?.trim() || null
    if (nextRuntimeError && nextRuntimeError !== lastReportedRuntimeStatusError) {
      store.getState().addRuntimeWarning(`background runtime: ${nextRuntimeError}`)
    }
    lastReportedRuntimeStatusError = nextRuntimeError
  })

  registerListener<{ error: string }>(store, 'runtime-service-error', (event) => {
    store.getState().addRuntimeWarning(`background runtime: ${event.payload.error}`)
  })

  registerListener<{ instanceId: string; text: string }>(store, 'server-log', (event) => {
    store.getState().addLog({
      instanceId: event.payload.instanceId,
      text: event.payload.text,
      timestamp: Date.now(),
      level: classifyLogLevel(event.payload.text),
    })
  })

  registerListener<{ instanceId: string; lines: string[] }>(store, 'server-log-batch', (event) => {
    const timestamp = Date.now()
    store.getState().addLogs(event.payload.lines.map((text, index) => ({
      instanceId: event.payload.instanceId,
      text,
      timestamp: timestamp + index,
      level: classifyLogLevel(text),
    })))
  })

  registerListener<{
    instanceId: string
    pid: number
    port: number
    host: string
    command: string
    effectiveConfig?: Partial<InstanceConfig>
  }>(store, 'server-started', (event) => {
    lastReportedRuntimeStatusError = null
    const state = store.getState()
    const instance = state.instances.find(item => item.id === event.payload.instanceId)
    store.setState(current => ({
      runningTasksByInstance: {
        ...current.runningTasksByInstance,
        [event.payload.instanceId]: [],
      },
      lastCompletedTaskByInstance: {
        ...current.lastCompletedTaskByInstance,
        [event.payload.instanceId]: null,
      },
    }))
    state.updateInstance(event.payload.instanceId, {
      status: 'running',
      healthCheck: 'pending',
      startTime: Date.now(),
      ...(instance && event.payload.effectiveConfig
        ? { config: { ...instance.config, ...event.payload.effectiveConfig } }
        : {}),
    })
    state.addLog({
      instanceId: event.payload.instanceId,
      text: `${formatStartupCommand(event.payload.command)}\n-- PID: ${event.payload.pid} | Port: ${event.payload.port}`,
      timestamp: Date.now(),
    })
    void state.saveConfig().catch(error => warnAsyncFailure(store, 'runtime config save', error))
  })

  registerListener<{ instanceId: string; expected?: boolean; reason?: string; exitCode?: number | null }>(store, 'server-stopped', (event) => {
    const state = store.getState()
    const instance = state.instances.find((item) => item.id === event.payload.instanceId)
    if (instance) {
      const isError = event.payload.expected !== true
      state.updateInstance(event.payload.instanceId, {
        status: isError ? 'error' : 'stopped',
        healthCheck: isError ? 'fail' : 'pending',
      })
    }
    if (event.payload.expected !== true) {
      state.addRuntimeWarning(`Instance exited unexpectedly (${event.payload.reason || 'process-exited'}, code ${event.payload.exitCode ?? 'unknown'})`)
    }
    void state.saveConfig().catch(error => warnAsyncFailure(store, 'runtime config save', error))
  })

  registerListener<{ instanceId: string; error: string }>(store, 'server-error', (event) => {
    const state = store.getState()
    state.updateInstance(event.payload.instanceId, {
      status: 'error',
      healthCheck: 'fail',
    })
    void state.saveConfig().catch(error => warnAsyncFailure(store, 'runtime config save', error))
  })

  registerListener<{ instanceId: string; status: string }>(store, 'health-status', (event) => {
    store.getState().updateInstance(event.payload.instanceId, {
      healthCheck: event.payload.status === 'ok'
        ? 'ok'
        : event.payload.status === 'fail'
          ? 'fail'
          : 'pending',
    })
  })

  registerListener<MonitoringFrame>(store, 'monitoring-frame', (event) => {
    store.getState().ingestMonitoringFrame(event.payload)
  })

  registerListener<PerfUpdateEvent>(store, 'perf-update', (event) => {
    store.getState().applyPerfUpdate(event.payload)
  })

  invoke<MonitoringFrame[]>('get_monitoring_series', { rangeMs: 3_600_000 })
    .then((frames) => store.getState().hydrateMonitoringFrames(frames))
    .catch((error) => {
      store.getState().addRuntimeWarning(
        `monitoring timeline hydration failed: ${error?.message || String(error)}`,
      )
    })

  registerListener<DownloadEventPayload>(store, 'download-started', (event) => {
    const applied = applyDownloadPatch(store, event.payload, {
      status: 'active',
      downloaded: event.payload.downloaded ?? 0,
      total: event.payload.total ?? 0,
      speed: 0,
      remoteChanged: false,
    })
    if (!applied) return

    const state = store.getState()
    const queue = event.payload.queueId
      ? state.downloadQueue.filter((entry) => entry.id !== event.payload.queueId)
      : state.downloadQueue.filter((entry) => !entry.files.some((file) => file.task_id === event.payload.taskId))
    store.setState({ downloadQueue: queue })
  })

  registerListener<DownloadEventPayload>(store, 'download-progress', (event) => {
    const state = store.getState()
    const taskId = taskIdFromEvent(event.payload, state.downloadTasks)
    if (!taskId) return

    const now = Date.now()
    if (now - (lastProgressUpdate[taskId] || 0) < 200) return
    lastProgressUpdate[taskId] = now

    applyDownloadPatch(store, event.payload, {
      status: 'active',
      downloaded: event.payload.downloaded ?? 0,
      total: event.payload.total ?? 0,
      speed: event.payload.speed ?? 0,
    })
  })

  registerListener<DownloadEventPayload>(store, 'download-complete', (event) => {
    const taskId = taskIdFromEvent(event.payload, store.getState().downloadTasks)
    const applied = applyDownloadPatch(store, event.payload, {
      status: 'completed',
      path: event.payload.path,
      speed: 0,
    })
    if (!applied) return
    if (taskId) delete lastProgressUpdate[taskId]

    const state = store.getState()
    void state.processDownloadQueue()
    scanModelDebounce ||= setTimeout(() => {
      scanModelDebounce = null
      const nextState = store.getState()
      nextState.scanModels(nextState.modelDirs)
    }, 2000)
  })

  registerListener<DownloadEventPayload>(store, 'download-cancelled', (event) => {
    const taskId = taskIdFromEvent(event.payload, store.getState().downloadTasks)
    const applied = applyDownloadPatch(store, event.payload, { status: 'cancelled', speed: 0 })
    if (!applied) return
    if (taskId) delete lastProgressUpdate[taskId]

    void store.getState().processDownloadQueue()
  })

  registerListener<DownloadEventPayload>(store, 'download-paused', (event) => {
    applyDownloadPatch(store, event.payload, {
      status: 'paused',
      downloaded: event.payload.downloaded ?? 0,
      total: event.payload.total ?? 0,
      speed: 0,
    })
  })

  registerListener<DownloadEventPayload>(store, 'download-error', (event) => {
    const taskId = taskIdFromEvent(event.payload, store.getState().downloadTasks)
    const applied = applyDownloadPatch(store, event.payload, {
      status: 'error',
      error: event.payload.error,
      speed: 0,
    })
    if (!applied) return
    if (taskId) delete lastProgressUpdate[taskId]

    void store.getState().processDownloadQueue()
  })

  registerListener<DownloadEventPayload>(store, 'download-restarted', (event) => {
    applyDownloadPatch(store, event.payload, {
      downloaded: 0,
      speed: 0,
      remoteChanged: false,
    })
  })

  registerListener<DownloadEventPayload>(store, 'download-remote-changed', (event) => {
    const state = store.getState()
    const taskId = event.payload.taskId || Object.values(state.downloadTasks).find((task) => (
      task.fileName === event.payload.fileName
      && task.repoId === event.payload.repoId
      && task.source === event.payload.source
      && versionMatches(event.payload, task)
    ))?.id
    if (!taskId) return

    const prev = state.downloadTasks[taskId]
    if (!prev) return
    if (!versionMatches(event.payload, prev)) return
    if (prev.runId && !runMatches(event.payload, prev)) return

    state.setDownloadTasks({
      ...state.downloadTasks,
      [taskId]: {
        ...prev,
        downloaded: 0,
        speed: 0,
        remoteChanged: true,
      },
    })
  })

  registerListener<{ taskId?: string; fileName: string; version?: number }>(store, 'download-removed', (event) => {
    const state = store.getState()
    const taskId = event.payload.taskId || Object.values(state.downloadTasks).find((task) => (
      task.fileName === event.payload.fileName
      && versionMatches(event.payload, task)
    ))?.id
    if (!taskId) return

    const tasks = { ...state.downloadTasks }
    delete tasks[taskId]
    delete lastProgressUpdate[taskId]
    state.setDownloadTasks(tasks)
  })

  startSysMetricsPolling(store)
}
