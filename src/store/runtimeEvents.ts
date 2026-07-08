import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { StoreApi } from 'zustand'
import { formatStartupCommand } from './commandFormatting'
import type { AppState, DownloadProgress, SystemMetrics } from './types'

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
    __lsm_listeners_registered?: boolean
  }
}

const lastProgressUpdate: Record<string, number> = {}
let scanModelDebounce: ReturnType<typeof setTimeout> | null = null
let sysMetricsTimer: ReturnType<typeof setInterval> | null = null

const runMatches = (payload: { runId?: string }, task: DownloadProgress) => (
  payload.runId ? task.runId === payload.runId : !task.runId
)

const versionMatches = (payload: { version?: number }, task: DownloadProgress) => (
  payload.version === undefined || task.version === undefined || task.version === payload.version
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

  const fetchSysMetrics = () => {
    invoke<SystemMetrics>('get_system_health')
      .then((metrics) => {
        store.getState().setSysMetrics(metrics)
      })
      .catch(() => {})
  }

  fetchSysMetrics()
  sysMetricsTimer = setInterval(fetchSysMetrics, 5000)
}

export function registerGlobalStoreListeners(
  store: StoreLike,
  startupTimings: { name: string; ms: number }[],
) {
  const globalWindow = window as Window & typeof globalThis
  if (globalWindow.__lsm_listeners_registered) {
    startSysMetricsPolling(store)
    return
  }

  globalWindow.__lsm_listeners_registered = true

  listen<{ name: string; ms: number }>('startup-timing', (event) => {
    startupTimings.push({ name: event.payload.name, ms: event.payload.ms })
  }).catch(() => {})

  listen<{ instanceId: string; text: string }>('server-log', (event) => {
    store.getState().addLog({
      instanceId: event.payload.instanceId,
      text: event.payload.text,
      timestamp: Date.now(),
    })
  }).catch(() => {})

  listen<{ instanceId: string; pid: number; port: number; command: string }>('server-started', (event) => {
    const state = store.getState()
    state.updateInstance(event.payload.instanceId, {
      status: 'running',
      healthCheck: 'pending',
      startTime: Date.now(),
    })
    state.addLog({
      instanceId: event.payload.instanceId,
      text: `${formatStartupCommand(event.payload.command)}\n-- PID: ${event.payload.pid} | Port: ${event.payload.port}`,
      timestamp: Date.now(),
    })
    state.saveConfig()
  }).catch(() => {})

  listen<{ instanceId: string }>('server-stopped', (event) => {
    const state = store.getState()
    const instance = state.instances.find((item) => item.id === event.payload.instanceId)
    if (instance) {
      const isError = instance.status === 'running' && instance.healthCheck !== 'ok'
      state.updateInstance(event.payload.instanceId, {
        status: isError ? 'error' : 'stopped',
        healthCheck: isError ? 'fail' : 'pending',
      })
    }
    state.saveConfig()
  }).catch(() => {})

  listen<{ instanceId: string; error: string }>('server-error', (event) => {
    const state = store.getState()
    state.updateInstance(event.payload.instanceId, {
      status: 'error',
      healthCheck: 'fail',
    })
    state.saveConfig()
  }).catch(() => {})

  listen<{ instanceId: string; status: string }>('health-status', (event) => {
    store.getState().updateInstance(event.payload.instanceId, {
      healthCheck: event.payload.status === 'ok' ? 'ok' : 'fail',
    })
  }).catch(() => {})

  listen<DownloadEventPayload>('download-started', (event) => {
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
  }).catch(() => {})

  listen<DownloadEventPayload>('download-progress', (event) => {
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
  }).catch(() => {})

  listen<DownloadEventPayload>('download-complete', (event) => {
    const applied = applyDownloadPatch(store, event.payload, {
      status: 'completed',
      path: event.payload.path,
      speed: 0,
    })
    if (!applied) return

    const state = store.getState()
    state.processDownloadQueue()
    scanModelDebounce ||= setTimeout(() => {
      scanModelDebounce = null
      const nextState = store.getState()
      nextState.scanModels(nextState.modelDirs)
    }, 2000)
  }).catch(() => {})

  listen<DownloadEventPayload>('download-cancelled', (event) => {
    const applied = applyDownloadPatch(store, event.payload, { status: 'cancelled', speed: 0 })
    if (!applied) return

    store.getState().processDownloadQueue()
  }).catch(() => {})

  listen<DownloadEventPayload>('download-paused', (event) => {
    applyDownloadPatch(store, event.payload, {
      status: 'paused',
      downloaded: event.payload.downloaded ?? 0,
      total: event.payload.total ?? 0,
      speed: 0,
    })
  }).catch(() => {})

  listen<DownloadEventPayload>('download-error', (event) => {
    const applied = applyDownloadPatch(store, event.payload, {
      status: 'error',
      error: event.payload.error,
      speed: 0,
    })
    if (!applied) return

    store.getState().processDownloadQueue()
  }).catch(() => {})

  listen<DownloadEventPayload>('download-restarted', (event) => {
    applyDownloadPatch(store, event.payload, {
      downloaded: 0,
      speed: 0,
      remoteChanged: false,
    })
  }).catch(() => {})

  listen<DownloadEventPayload>('download-remote-changed', (event) => {
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
  }).catch(() => {})

  listen<{ taskId?: string; fileName: string; version?: number }>('download-removed', (event) => {
    const state = store.getState()
    const taskId = event.payload.taskId || Object.values(state.downloadTasks).find((task) => (
      task.fileName === event.payload.fileName
      && versionMatches(event.payload, task)
    ))?.id
    if (!taskId) return

    const tasks = { ...state.downloadTasks }
    delete tasks[taskId]
    state.setDownloadTasks(tasks)
  }).catch(() => {})

  startSysMetricsPolling(store)
}
