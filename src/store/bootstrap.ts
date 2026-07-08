import { startTransition } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { pathBasename } from '../utils/path'
import type { AppStoreGet, AppStoreSet } from './helpers'
import type {
  DownloadManagerSnapshot,
  DownloadProgress,
  EngineInfo,
  Instance,
  InstanceConfig,
  ModelInfo,
  PersistedQueueEntry,
} from './types'

export type GlobalConfigShape = {
  instances: Record<string, InstanceConfig>
  model_dirs: string[]
  engine_dirs: string[]
  default_engine_id: string
  running: Record<string, { instance_id: string; pid: number; port: number; host: string; start_time?: number }>
  instance_order: string[]
  last_tab: string
  dark_mode: boolean
  download_bandwidth_limit_bytes_per_sec?: number
  download_low_priority_throttle?: boolean
}

declare global {
  interface Window {
    __INITIAL_CONFIG__?: GlobalConfigShape | null
    __downloadSnapshotMeta?: {
      active_count: number
      max_concurrent: number
      resume_policy: string
      bandwidth_limit_bytes_per_sec: number
      low_priority_throttle: boolean
    }
  }
}

function hydrateDownloadTasksFromSnapshot(
  snapshot: DownloadManagerSnapshot,
  get: AppStoreGet,
  set: AppStoreSet,
) {
  if (snapshot.queue.length > 0) {
    const existing = { ...get().downloadTasks }
    for (const entry of snapshot.queue) {
      const source = entry.source as 'modelscope' | 'huggingface'
      const saveDir = entry.save_dir || 'models'
      for (const file of entry.files) {
        const taskId = file.task_id || crypto.randomUUID()
        if (existing[taskId]) continue

        existing[taskId] = {
          id: taskId,
          fileName: file.name,
          remotePath: file.path || file.name,
          fileType: file.file_type,
          saveDir,
          repoId: entry.repo_id,
          source,
          runId: file.run_id,
          downloaded: file.downloaded ?? 0,
          total: file.size,
          speed: 0,
          status: (file.status as DownloadProgress['status'])
            || (entry.status === 'active' ? 'active'
              : entry.status === 'queued' ? 'queued'
              : entry.status === 'pausing' ? 'paused'
              : (entry.status as DownloadProgress['status']) || 'queued'),
          version: file.version ?? 0,
          error: file.error,
        }
      }
    }

    startTransition(() => {
      set({ downloadTasks: existing })
    })
  }

  window.__downloadSnapshotMeta = {
    active_count: snapshot.active_count,
    max_concurrent: snapshot.max_concurrent,
    resume_policy: snapshot.resume_policy,
    bandwidth_limit_bytes_per_sec: snapshot.bandwidth_limit_bytes_per_sec,
    low_priority_throttle: snapshot.low_priority_throttle,
  }
}

async function processConfig(
  global: GlobalConfigShape,
  get: AppStoreGet,
  set: AppStoreSet,
) {
  const runningIds = new Set(Object.keys(global.running || {}))
  const order = global.instance_order || Object.keys(global.instances)
  const orderIndex = new Map(order.map((id, index) => [id, index]))

  const instances: Instance[] = Object.entries(global.instances).map(([id, config]) => {
    const startedAt = global.running?.[id]?.start_time ?? 0
    return {
      id,
      name: config.name || 'Unnamed instance',
      status: runningIds.has(id) ? 'running' : 'stopped',
      model: pathBasename(config.model_path),
      port: config.port,
      healthCheck: 'pending',
      config,
      startTime: startedAt > 0 ? startedAt * 1000 : undefined,
    }
  })

  instances.sort((left, right) => (
    (orderIndex.get(left.id) ?? Number.MAX_SAFE_INTEGER)
    - (orderIndex.get(right.id) ?? Number.MAX_SAFE_INTEGER)
  ))

  startTransition(() => {
    set({
      instances,
      modelDirs: global.model_dirs || [],
      engineDirs: global.engine_dirs || [],
      defaultEngineId: global.default_engine_id || null,
    })
  })

  if (global.dark_mode !== undefined) {
    document.documentElement.classList.toggle('dark', global.dark_mode)
    startTransition(() => {
      set({ darkMode: !!global.dark_mode })
    })
  }

  await invoke<[ModelInfo[], EngineInfo[], PersistedQueueEntry[]]>('load_app_data', {
    paths: global.model_dirs || [],
    enginePaths: global.engine_dirs || [],
  })
    .then(([models, engines, queue]) => {
      startTransition(() => {
        set({ models, engines })
      })
      if (queue?.length > 0) {
        startTransition(() => {
          get().restoreDownloadQueue(queue)
        })
      }
    })
    .catch(() => {
      invoke<EngineInfo[]>('scan_engines', { paths: global.engine_dirs || [] })
        .then((engines) => startTransition(() => {
          set({ engines })
        }))
        .catch(() => {})
    })

  invoke<DownloadManagerSnapshot>('get_download_manager_snapshot')
    .then((snapshot) => {
      if (snapshot) {
        hydrateDownloadTasksFromSnapshot(snapshot, get, set)
      }
    })
    .catch(() => {})
}

export async function loadAppBootstrap(
  set: AppStoreSet,
  get: AppStoreGet,
  startupTimings: { name: string; ms: number }[],
) {
  const startedAt = performance.now()
  set({ isLoading: true })

  try {
    invoke<number>('get_startup_elapsed')
      .then((ms) => {
        startupTimings.push({ name: 'native-init', ms })
      })
      .catch(() => {})

    invoke<[ModelInfo[], EngineInfo[]] | null>('get_cached_scan')
      .then((data) => {
        if (data) {
          startTransition(() => {
            set({ models: data[0], engines: data[1] })
          })
        }
      })
      .catch(() => {})

    const injected = window.__INITIAL_CONFIG__
    if (injected) {
      window.__INITIAL_CONFIG__ = null
      startupTimings.push({ name: 'config-source', ms: 0 })
      await processConfig(injected, get, set)
    } else {
      const ipcStartedAt = performance.now()
      const global = await invoke<GlobalConfigShape>('load_config')
      startupTimings.push({ name: 'config-source', ms: Math.round(performance.now() - ipcStartedAt) })
      await processConfig(global, get, set)
    }
  } catch (error) {
    console.error('load_config error:', error)
  }

  set({ isLoading: false })
  startupTimings.push({ name: 'loadConfig', ms: Math.round(performance.now() - startedAt) })
}
