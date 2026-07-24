import { invokeApp as invoke } from '../lib/ipc'
import { mergeRestoredDownloadTask } from './downloadMerge'
import type { AppStoreGet, AppStoreSet } from './helpers'
import type { AppState, DownloadProgress, MsFileEntry, PersistedQueueEntry } from './types'

type ResumeDownloadTaskResult = {
  taskId: string
  runId: string
  version: number
}

const taskRemotePath = (file: MsFileEntry) => file.path || file.name

const errorMessage = (error: unknown) => error instanceof Error ? error.message : String(error)

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
    addToDownloadQueue: async (entry) => {
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
          createdAt: existing?.createdAt ?? Date.now(),
          updatedAt: Date.now(),
          completedAt: undefined,
        }
      }

      if (files.length === 0) return false

      const queuedEntry = { ...entry, files, id: queueId, addedAt: Date.now() }
      const taskUpdates = Object.fromEntries(
        files
          .map(file => file.task_id)
          .filter((taskId): taskId is string => Boolean(taskId))
          .map(taskId => [taskId, tasks[taskId]]),
      )
      try {
        await invoke('enqueue_download_queue', { entry: toPersistedQueueEntry(queuedEntry) })
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error)
        get().addRuntimeWarning(`download enqueue failed: ${message}`)
        return false
      }
      set((current) => ({
        downloadTasks: { ...current.downloadTasks, ...taskUpdates },
        downloadQueue: [...current.downloadQueue, queuedEntry],
      }))
      return true
    },
    removeFromDownloadQueue: async (id) => {
      try {
        await invoke('remove_download_queue_entry', { id })
        set((state) => ({ downloadQueue: state.downloadQueue.filter((entry) => entry.id !== id) }))
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error)
        get().addRuntimeWarning(`download queue removal failed: ${message}`)
      }
    },
    processDownloadQueue: async () => {
      try {
        await invoke('process_download_queue')
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error)
        get().addRuntimeWarning(`download queue processing failed: ${message}`)
      }
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
        get().addRuntimeWarning(`download cancellation failed: ${errorMessage(error)}`)
      }
    },
    pauseFileDownload: async (taskId, runId) => {
      try {
        await invoke('pause_file_download', { taskId, runId })
      } catch (error) {
        console.error(error)
        get().addRuntimeWarning(`download pause failed: ${errorMessage(error)}`)
        set((state) => {
          const task = state.downloadTasks[taskId]
          if (!task || task.status !== 'pausing') return {}
          return {
            downloadTasks: {
              ...state.downloadTasks,
              [taskId]: { ...task, status: 'active' },
            },
          }
        })
      }
    },
    cancelAndCleanupDownload: async (taskId, fileName, filePath, runId, version) => {
      try {
        await invoke('cancel_and_cleanup_download', { taskId, fileName, filePath, runId, version })
      } catch (error) {
        console.error(error)
        get().addRuntimeWarning(`download cancellation persistence failed: ${String(error)}`)
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
              updatedAt: Date.now(),
              completedAt: undefined,
            },
          },
        })
      } catch (error) {
        console.error(error)
        get().addRuntimeWarning(`download resume failed: ${errorMessage(error)}`)
      }
    },
    restoreDownloadQueue: (entries) => {
      const tasks = { ...get().downloadTasks }
      const restoredQueue: AppState['downloadQueue'] = []
      for (const entry of entries) {
        const source = entry.source as 'modelscope' | 'huggingface'
        const saveDir = entry.save_dir || 'models'
        for (const file of entry.files) {
          const taskId = file.task_id || crypto.randomUUID()
          if (tasks[taskId]?.status === 'completed') continue

          tasks[taskId] = mergeRestoredDownloadTask(tasks[taskId], file, {
            repoId: entry.repo_id,
            source,
            saveDir,
            entryStatus: entry.status,
            addedAt: entry.added_at,
          })
        }
        if (['queued', 'active'].includes(entry.status)) {
          const pendingFiles = entry.files.filter(file => !['completed', 'cancelled'].includes(file.status || ''))
          if (pendingFiles.length > 0) {
            restoredQueue.push({
              id: entry.id,
              repoId: entry.repo_id,
              source,
              files: pendingFiles,
              saveDir,
              addedAt: entry.added_at,
            })
          }
        }
      }
      set({ downloadTasks: tasks, downloadQueue: restoredQueue })
    },
    persistQueue: async () => {
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

      try {
        await invoke('persist_download_queue', { queue })
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error)
        get().addRuntimeWarning(`download queue persistence failed: ${message}`)
      }
    },
    resumeAllDownloads: async () => {
      let identities: Array<{ taskId: string; runId: string; version: number }> = []
      try {
        identities = await invoke<Array<{ taskId: string; runId: string; version: number }>>('resume_all_downloads')
      } catch (error) {
        console.error(error)
        get().addRuntimeWarning(`resume all downloads failed: ${errorMessage(error)}`)
        return
      }

      const tasks = { ...get().downloadTasks }
      const identityByTaskId = new Map(identities.map(identity => [identity.taskId, identity]))
      for (const [id, identity] of identityByTaskId) {
        if (tasks[id] && (tasks[id].status === 'paused' || tasks[id].status === 'pausing')) {
          tasks[id] = {
            ...tasks[id],
            status: 'queued',
            speed: 0,
            error: undefined,
            completedAt: undefined,
            updatedAt: Date.now(),
            runId: identity.runId,
            version: identity.version,
          }
        }
      }
      set({ downloadTasks: tasks })
    },
    pauseAllDownloads: async () => {
      let affected: string[] = []
      try {
        affected = await invoke<string[]>('pause_all_downloads')
      } catch (error) {
        console.error(error)
        get().addRuntimeWarning(`pause all downloads failed: ${errorMessage(error)}`)
        return
      }

      const tasks = { ...get().downloadTasks }
      for (const id of affected) {
        if (tasks[id] && ['active', 'queued'].includes(tasks[id].status)) {
          tasks[id] = { ...tasks[id], status: tasks[id].status === 'active' ? 'pausing' : 'paused' }
        }
      }
      set({ downloadTasks: tasks })
    },
    cancelAllDownloads: async () => {
      try {
        await invoke('cancel_all_downloads')
      } catch (error) {
        console.error(error)
        get().addRuntimeWarning(`cancel all downloads failed: ${errorMessage(error)}`)
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
    clearCompletedDownloadTasks: async () => {
      try {
        await invoke('clear_download_tasks_by_status', { statuses: ['completed'] })
        const tasks = { ...get().downloadTasks }
        for (const key of Object.keys(tasks)) {
          if (tasks[key].status === 'completed') delete tasks[key]
        }
        set({ downloadTasks: tasks })
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error)
        get().addRuntimeWarning(`completed download cleanup failed: ${message}`)
      }
    },
    clearFailedDownloadTasks: async () => {
      try {
        await invoke('clear_download_tasks_by_status', { statuses: ['error'] })
        const tasks = { ...get().downloadTasks }
        for (const key of Object.keys(tasks)) {
          if (tasks[key].status === 'error') delete tasks[key]
        }
        set({ downloadTasks: tasks })
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error)
        get().addRuntimeWarning(`failed download cleanup failed: ${message}`)
      }
    },
    retryFailedDownload: (taskId) => {
      const task = get().downloadTasks[taskId]
      if (!task || task.status !== 'error') return

      void get().addToDownloadQueue({
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
    redownloadFile: async (taskId) => {
      const task = get().downloadTasks[taskId]
      if (!task || task.status !== 'error') return

      try {
        await invoke('reset_download_for_redownload', {
          taskId,
        })
      } catch (error) {
        get().addRuntimeWarning(`重新下载准备失败：${String(error)}`)
        return
      }

      await get().addToDownloadQueue({
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
