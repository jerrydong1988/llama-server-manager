import { invoke } from '@tauri-apps/api/core'
import type { AppStoreGet, AppStoreSet } from './helpers'
import type { AppState, DownloadProgress, MsFileEntry, PersistedQueueEntry } from './types'

type ResumeDownloadTaskResult = {
  taskId: string
  runId: string
  version: number
}

const taskRemotePath = (file: MsFileEntry) => file.path || file.name

const findTaskForFile = (
  tasks: Record<string, DownloadProgress>,
  source: 'modelscope' | 'huggingface',
  repoId: string,
  file: MsFileEntry,
  saveDir: string,
) => Object.values(tasks).find((task) => (
  task.source === source
  && task.repoId === repoId
  && task.remotePath === taskRemotePath(file)
  && task.saveDir === saveDir
))

const queueHasTask = (
  queue: AppState['downloadQueue'],
  source: 'modelscope' | 'huggingface',
  repoId: string,
  file: MsFileEntry,
  saveDir: string,
) => queue.some((entry) => (
  entry.source === source
  && entry.repoId === repoId
  && entry.saveDir === saveDir
  && entry.files.some((queuedFile) => (
    (queuedFile.task_id && queuedFile.task_id === file.task_id)
    || taskRemotePath(queuedFile) === taskRemotePath(file)
  ))
))

const toPersistedQueueEntry = (
  entry: {
    id: string
    repoId: string
    source: 'modelscope' | 'huggingface'
    files: MsFileEntry[]
    saveDir: string
    addedAt: number
  },
  status = 'queued',
): PersistedQueueEntry => ({
  id: entry.id,
  repo_id: entry.repoId,
  source: entry.source,
  files: entry.files,
  save_dir: entry.saveDir,
  added_at: entry.addedAt,
  status,
})

export function createDownloadSlice(set: AppStoreSet, get: AppStoreGet): Pick<
  AppState,
  | 'setDownloadTasks'
  | 'addToDownloadQueue'
  | 'removeFromDownloadQueue'
  | 'processDownloadQueue'
  | 'browseModelscope'
  | 'downloadModelscopeFiles'
  | 'browseHuggingface'
  | 'downloadHuggingfaceFiles'
  | 'cancelFileDownload'
  | 'pauseFileDownload'
  | 'cancelAndCleanupDownload'
  | 'resumeDownloadTask'
  | 'restoreDownloadQueue'
  | 'persistQueue'
  | 'resumeAllDownloads'
  | 'pauseAllDownloads'
  | 'cancelAllDownloads'
  | 'clearCompletedDownloadTasks'
  | 'clearFailedDownloadTasks'
  | 'retryFailedDownload'
  | 'redownloadFile'
  | 'moveQueueEntry'
> {
  return {
    setDownloadTasks: (tasks) => set({ downloadTasks: tasks }),
    addToDownloadQueue: (entry) => {
      const state = get()
      const queueId = crypto.randomUUID()
      const tasks = { ...state.downloadTasks }
      const files: MsFileEntry[] = []

      for (const file of entry.files) {
        const existing = findTaskForFile(tasks, entry.source, entry.repoId, file, entry.saveDir)
        if (existing && existing.status !== 'cancelled' && existing.status !== 'error' && existing.status !== 'paused') {
          continue
        }
        if (queueHasTask(state.downloadQueue, entry.source, entry.repoId, file, entry.saveDir)) {
          continue
        }

        const taskId = file.task_id || existing?.id || crypto.randomUUID()
        const runId = crypto.randomUUID()
        const version = (existing?.version ?? file.version ?? 0) + 1
        const hydrated: MsFileEntry = {
          ...file,
          path: taskRemotePath(file),
          task_id: taskId,
          run_id: runId,
          downloaded: existing?.downloaded ?? file.downloaded ?? 0,
          version,
          status: 'queued',
          error: undefined,
        }

        files.push(hydrated)
        tasks[taskId] = {
          ...(existing || {}),
          id: taskId,
          runId,
          fileName: file.name,
          remotePath: hydrated.path,
          fileType: file.file_type,
          saveDir: entry.saveDir,
          repoId: entry.repoId,
          source: entry.source,
          downloaded: existing?.downloaded ?? file.downloaded ?? 0,
          total: file.size,
          speed: 0,
          status: 'queued',
          version,
        }
      }

      if (files.length === 0) return

      const queuedEntry = { ...entry, files, id: queueId, addedAt: Date.now() }
      set({
        downloadTasks: tasks,
        downloadQueue: [...state.downloadQueue, queuedEntry],
      })
      invoke('enqueue_download_queue', { entry: toPersistedQueueEntry(queuedEntry) }).catch(() => {})
    },
    removeFromDownloadQueue: (id) => {
      set((state) => ({ downloadQueue: state.downloadQueue.filter((entry) => entry.id !== id) }))
      invoke('remove_download_queue_entry', { id }).catch(() => {})
    },
    processDownloadQueue: () => {
      invoke('process_download_queue').catch(() => {})
    },
    browseModelscope: async (repoId) => invoke<MsFileEntry[]>('browse_modelscope', { repoId }),
    downloadModelscopeFiles: async (repoId, files, saveDir) => {
      await invoke('download_modelscope_files', { repoId, files, saveDir })
    },
    browseHuggingface: async (repoId) => invoke<MsFileEntry[]>('browse_huggingface', { repoId }),
    downloadHuggingfaceFiles: async (repoId, files, saveDir) => {
      await invoke('download_huggingface_files', { repoId, files, saveDir })
    },
    cancelFileDownload: async (taskId, runId) => {
      try {
        await invoke('cancel_file_download', { taskId, runId })
      } catch (error) {
        console.error(error)
      }
    },
    pauseFileDownload: async (taskId, runId) => {
      try {
        await invoke('pause_file_download', { taskId, runId })
      } catch (error) {
        console.error(error)
      }
    },
    cancelAndCleanupDownload: async (taskId, fileName, filePath, runId, version) => {
      try {
        await invoke('cancel_and_cleanup_download', { taskId, fileName, filePath, runId, version })
      } catch (error) {
        console.error(error)
      }
    },
    resumeDownloadTask: async (taskId) => {
      try {
        const resumed = await invoke<ResumeDownloadTaskResult>('resume_download_task', { taskId })
        const task = get().downloadTasks[taskId]
        if (!task) return

        set({
          downloadTasks: {
            ...get().downloadTasks,
            [taskId]: {
              ...task,
              runId: resumed.runId,
              version: resumed.version,
              status: 'queued',
              speed: 0,
              error: undefined,
            },
          },
        })
      } catch (error) {
        console.error(error)
      }
    },
    restoreDownloadQueue: (entries) => {
      const tasks = { ...get().downloadTasks }
      for (const entry of entries) {
        const source = entry.source as 'modelscope' | 'huggingface'
        const saveDir = entry.save_dir || 'models'
        for (const file of entry.files) {
          const taskId = file.task_id || crypto.randomUUID()
          if (tasks[taskId]?.status === 'completed') continue

          tasks[taskId] = {
            id: taskId,
            fileName: file.name,
            remotePath: taskRemotePath(file),
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
      set({ downloadTasks: tasks })
    },
    persistQueue: () => {
      const { downloadQueue, downloadTasks } = get()
      const queue = downloadQueue.map((entry) => {
        const files = entry.files.map((file) => {
          const taskId = file.task_id
          const task = taskId ? downloadTasks[taskId] : undefined
          return {
            ...file,
            path: task?.remotePath || taskRemotePath(file),
            file_type: task?.fileType || file.file_type,
            task_id: taskId,
            run_id: task?.runId || file.run_id,
            downloaded: task?.downloaded ?? file.downloaded ?? 0,
            version: task?.version ?? file.version ?? 0,
            status: task?.status ?? file.status ?? 'queued',
            error: task?.error ?? file.error,
          }
        })
        const statuses = files.map((file) => (file.task_id ? downloadTasks[file.task_id]?.status : undefined))
        const status = statuses.includes('active')
          ? 'active'
          : statuses.includes('error')
            ? 'error'
            : statuses.includes('paused')
              ? 'paused'
              : statuses.every((item) => item === 'completed')
                ? 'completed'
                : 'queued'

        return {
          id: entry.id,
          repo_id: entry.repoId,
          source: entry.source,
          files,
          save_dir: entry.saveDir,
          added_at: entry.addedAt,
          status,
        }
      })

      invoke('persist_download_queue', { queue }).catch(() => {})
    },
    resumeAllDownloads: async () => {
      let identities: Array<{ taskId: string; runId: string; version: number }> = []
      try {
        identities = await invoke<Array<{ taskId: string; runId: string; version: number }>>('resume_all_downloads')
      } catch (error) {
        console.error(error)
        return
      }

      const tasks = { ...get().downloadTasks }
      const identityByTaskId = new Map(identities.map(identity => [identity.taskId, identity]))
      for (const id of Object.keys(tasks)) {
        if (tasks[id].status === 'paused' || tasks[id].status === 'pausing') {
          const identity = identityByTaskId.get(id)
          tasks[id] = {
            ...tasks[id],
            status: 'queued',
            speed: 0,
            ...(identity ? { runId: identity.runId, version: identity.version } : {}),
          }
        }
      }
      set({ downloadTasks: tasks })
    },
    pauseAllDownloads: async () => {
      try {
        await invoke('pause_all_downloads')
      } catch (error) {
        console.error(error)
        return
      }

      const tasks = { ...get().downloadTasks }
      for (const id of Object.keys(tasks)) {
        if (tasks[id].status === 'active') {
          tasks[id] = { ...tasks[id], status: 'pausing' }
        }
      }
      set({ downloadTasks: tasks })
    },
    cancelAllDownloads: async () => {
      try {
        await invoke('cancel_all_downloads')
      } catch (error) {
        console.error(error)
        return
      }

      const tasks = { ...get().downloadTasks }
      for (const id of Object.keys(tasks)) {
        if (['active', 'paused', 'pausing', 'queued'].includes(tasks[id].status)) {
          tasks[id] = { ...tasks[id], status: 'cancelled', speed: 0 }
        }
      }
      set({ downloadTasks: tasks, downloadQueue: [] })
    },
    clearCompletedDownloadTasks: () => {
      const tasks = { ...get().downloadTasks }
      for (const key of Object.keys(tasks)) {
        if (tasks[key].status === 'completed' || tasks[key].status === 'cancelled') {
          delete tasks[key]
        }
      }
      set({ downloadTasks: tasks })
      invoke('clear_download_tasks_by_status', { statuses: ['completed', 'cancelled'] }).catch(() => {})
    },
    clearFailedDownloadTasks: () => {
      const tasks = { ...get().downloadTasks }
      for (const key of Object.keys(tasks)) {
        if (tasks[key].status === 'error') {
          delete tasks[key]
        }
      }
      set({ downloadTasks: tasks })
      invoke('clear_download_tasks_by_status', { statuses: ['error'] }).catch(() => {})
    },
    retryFailedDownload: (taskId) => {
      const task = get().downloadTasks[taskId]
      if (!task || task.status !== 'error') return

      get().addToDownloadQueue({
        repoId: task.repoId,
        source: task.source as 'modelscope' | 'huggingface',
        files: [{
          name: task.fileName,
          path: task.remotePath,
          size: task.total,
          file_type: task.fileType || 'model',
          task_id: taskId,
          downloaded: task.downloaded,
          version: task.version,
          status: 'queued',
        }],
        saveDir: task.saveDir,
      })
    },
    redownloadFile: (taskId) => {
      const task = get().downloadTasks[taskId]
      if (!task || task.status !== 'error') return

      invoke('reset_download_for_redownload', {
        taskId,
        fileName: task.fileName,
        saveDir: task.saveDir,
      }).catch(() => {})

      const tasks = { ...get().downloadTasks }
      delete tasks[taskId]
      set({ downloadTasks: tasks })

      get().addToDownloadQueue({
        repoId: task.repoId,
        source: task.source as 'modelscope' | 'huggingface',
        files: [{
          name: task.fileName,
          path: task.remotePath,
          size: task.total,
          file_type: task.fileType || 'model',
          downloaded: 0,
          status: 'queued',
        }],
        saveDir: task.saveDir,
      })
    },
    moveQueueEntry: (id, direction) => {
      const state = get()
      const index = state.downloadQueue.findIndex((entry) => entry.id === id)
      if (index < 0) return

      const target = direction === 'up' ? index - 1 : index + 1
      if (target < 0 || target >= state.downloadQueue.length) return

      const next = [...state.downloadQueue]
      ;[next[index], next[target]] = [next[target], next[index]]
      set({ downloadQueue: next })
      get().persistQueue()
    },
  }
}
