import type { DownloadProgress, MsFileEntry } from './types'

type RestoreContext = {
  repoId: string
  source: 'modelscope' | 'huggingface'
  saveDir: string
  entryStatus: string
}

const taskRemotePath = (file: MsFileEntry) => file.path || file.name

const restoredStatus = (
  fileStatus: string | undefined,
  entryStatus: string,
): DownloadProgress['status'] => (
  (fileStatus as DownloadProgress['status'])
  || (entryStatus === 'active' ? 'active'
    : entryStatus === 'queued' ? 'queued'
    : entryStatus === 'pausing' ? 'paused'
    : (entryStatus as DownloadProgress['status']) || 'queued')
)

function preferExistingStatus(
  existing: DownloadProgress['status'],
  restored: DownloadProgress['status'],
): DownloadProgress['status'] {
  if (existing === 'completed') return 'completed'
  if (existing === 'active' && restored === 'queued') return 'active'
  if (existing === 'pausing' && (restored === 'queued' || restored === 'paused')) return 'pausing'
  if (existing === 'paused' && restored === 'queued') return 'paused'
  if (existing === 'error' && restored === 'queued') return 'error'
  return restored
}

export function mergeRestoredDownloadTask(
  existing: DownloadProgress | undefined,
  file: MsFileEntry,
  context: RestoreContext,
): DownloadProgress {
  const restoredVersion = file.version ?? 0
  const existingIsNewer = existing?.version !== undefined && existing.version >= restoredVersion
  const restored: DownloadProgress = {
    id: file.task_id || existing?.id || crypto.randomUUID(),
    fileName: file.name,
    remotePath: taskRemotePath(file),
    fileType: file.file_type,
    saveDir: context.saveDir,
    repoId: context.repoId,
    source: context.source,
    runId: file.run_id,
    downloaded: file.downloaded ?? 0,
    total: file.size,
    speed: 0,
    status: restoredStatus(file.status, context.entryStatus),
    version: restoredVersion,
    error: file.error,
  }

  if (!existing) return restored

  return {
    ...restored,
    id: existing.id,
    runId: existingIsNewer ? existing.runId : restored.runId,
    downloaded: Math.max(existing.downloaded ?? 0, restored.downloaded ?? 0),
    total: existing.total || restored.total,
    speed: existing.status === 'active' ? existing.speed : restored.speed,
    status: existingIsNewer
      ? preferExistingStatus(existing.status, restored.status)
      : restored.status,
    version: Math.max(existing.version ?? 0, restored.version ?? 0),
    error: existingIsNewer ? existing.error : restored.error,
    remoteChanged: existing.remoteChanged,
  }
}
