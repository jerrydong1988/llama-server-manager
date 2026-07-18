import { useState, useMemo, useEffect, useRef } from 'react'
import { Trash2, FolderOpen, X, Download, ChevronDown, ChevronUp, Pause, Play, RotateCcw, RefreshCw, Square, AlertTriangle, Database } from 'lucide-react'
import { useAppStore, type MsFileEntry } from '../store'
import type { DownloadProgress } from '../store/types'
import { formatMessage, useI18n } from '../i18n'
import { invokeApp as invoke } from '../lib/ipc'
import { open } from '@tauri-apps/plugin-dialog'
import { pathJoin } from '../utils/path'
import { forEachConcurrent } from '../utils/async'
import { formatSize, formatSpeed, formatETA } from '../utils/format'
import { PathText, surfaceClassName } from './ui'
import { MetricTile, SectionPanel, StatusBadge } from './DownloadManager/DownloadPrimitives'
import { DEFAULT_BANDWIDTH_LIMIT, DEFAULT_BANDWIDTH_UNIT, DEFAULT_SAVE_DIR, LOCAL_FILE_CHECK_CONCURRENCY, bandwidthToBytes, bytesToBandwidth, clampConcurrency, downloadFileKey, normalizeResumePolicy, preferredBandwidthDisplay } from './DownloadManager/downloadPolicy'
import {
  DownloadSettingsPanel,
  type DownloadBandwidthUnit as BandwidthUnit,
  type DownloadResumePolicy as ResumePolicy,
} from './DownloadManager/DownloadSettingsPanel'
type DownloadSource = 'modelscope' | 'huggingface'
type Section = 'queue' | 'active' | 'paused' | 'failed' | 'completed'
export default function DownloadManager() {
  const { t } = useI18n()
  const downloadTasks = useAppStore(s => s.downloadTasks)
  const downloadQueue = useAppStore(s => s.downloadQueue)
  const setDownloadTasks = useAppStore(s => s.setDownloadTasks)
  const cancelFileDownload = useAppStore(s => s.cancelFileDownload)
  const cancelAndCleanupDownload = useAppStore(s => s.cancelAndCleanupDownload)
  const removeFromDownloadQueue = useAppStore(s => s.removeFromDownloadQueue)
  const addToDownloadQueue = useAppStore(s => s.addToDownloadQueue)
  const browseModelscope = useAppStore(s => s.browseModelscope)
  const browseHuggingface = useAppStore(s => s.browseHuggingface)
  const pauseFileDownload = useAppStore(s => s.pauseFileDownload)
  const resumeDownloadTask = useAppStore(s => s.resumeDownloadTask)
  const resumeAllDownloads = useAppStore(s => s.resumeAllDownloads)
  const pauseAllDownloads = useAppStore(s => s.pauseAllDownloads)
  const cancelAllDownloads = useAppStore(s => s.cancelAllDownloads)
  const clearCompletedDownloadTasks = useAppStore(s => s.clearCompletedDownloadTasks)
  const clearFailedDownloadTasks = useAppStore(s => s.clearFailedDownloadTasks)
  const retryFailedDownload = useAppStore(s => s.retryFailedDownload)
  const redownloadFile = useAppStore(s => s.redownloadFile)
  const moveQueueEntry = useAppStore(s => s.moveQueueEntry)
  const scanModels = useAppStore(s => s.scanModels)
  const openModelFolder = useAppStore(s => s.openModelFolder)
  const [source, setSource] = useState<DownloadSource>('modelscope')
  const [repoId, setRepoId] = useState('')
  const [files, setFiles] = useState<MsFileEntry[]>([])
  const [status, setStatus] = useState('')
  const [browsing, setBrowsing] = useState(false)
  const [browsedRepoId, setBrowsedRepoId] = useState('')
  const [collapsed, setCollapsed] = useState<Record<Section, boolean>>({
    queue: false,
    active: false,
    paused: false,
    failed: false,
    completed: false,
  })
  const [saveDir, setSaveDir] = useState(() => {
    try {
      return localStorage.getItem('downloadSaveDir') || DEFAULT_SAVE_DIR
    } catch {
      return DEFAULT_SAVE_DIR
    }
  })
  const [initialLoading, setInitialLoading] = useState(true)
  const [dlSettingsOpen, setDlSettingsOpen] = useState(true)
  const [resumePolicy, setResumePolicy] = useState<ResumePolicy>('manual')
  const [concurrency, setConcurrency] = useState(1)
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null)
  const [modelImportStatuses, setModelImportStatuses] = useState<Record<string, string>>({})
  const [bandwidthLimit, setBandwidthLimit] = useState(DEFAULT_BANDWIDTH_LIMIT)
  const [bandwidthUnit, setBandwidthUnit] = useState<BandwidthUnit>(() => {
    try {
      const saved = localStorage.getItem('downloadBandwidthUnit')
      return saved === 'KiB/s' ? 'KiB/s' : DEFAULT_BANDWIDTH_UNIT
    } catch {
      return DEFAULT_BANDWIDTH_UNIT
    }
  })
  const [lowPriorityThrottle, setLowPriorityThrottle] = useState(false)
  const [settingsError, setSettingsError] = useState('')
  const ui = t.downloadWorkspace
  const initialBandwidthUnitRef = useRef(bandwidthUnit)
  const settingsLoadFailedRef = useRef(ui.settingsLoadFailed)
  const settingsFailureText = (prefix: string, error: unknown) => {
    const message = error instanceof Error ? error.message : String(error)
    return `${prefix}: ${message}`
  }
  useEffect(() => {
    const meta = (window as any).__downloadSnapshotMeta
    if (meta) {
      setResumePolicy(normalizeResumePolicy(meta.resume_policy || 'manual'))
      setConcurrency(clampConcurrency(meta.max_concurrent || 1))
      const bandwidth = preferredBandwidthDisplay(meta.bandwidth_limit_bytes_per_sec || 0, initialBandwidthUnitRef.current)
      setBandwidthLimit(bandwidth.limit)
      setBandwidthUnit(bandwidth.unit)
      setLowPriorityThrottle(!!meta.low_priority_throttle)
    } else {
      void Promise.all([
        invoke<string>('get_download_resume_policy'),
        invoke<number>('get_download_concurrency'),
        invoke<number>('get_download_bandwidth_limit'),
        invoke<boolean>('get_download_low_priority_throttle'),
      ])
        .then(([policy, concurrent, bytes, throttle]) => {
          setResumePolicy(normalizeResumePolicy(policy || 'manual'))
          setConcurrency(clampConcurrency(concurrent || 1))
          const bandwidth = preferredBandwidthDisplay(bytes || 0, initialBandwidthUnitRef.current)
          setBandwidthLimit(bandwidth.limit)
          setBandwidthUnit(bandwidth.unit)
          setLowPriorityThrottle(!!throttle)
          setSettingsError('')
        })
        .catch((error) => {
          setSettingsError(settingsFailureText(settingsLoadFailedRef.current, error))
        })
    }
    setTimeout(() => setInitialLoading(false), 300)
  }, [])
  useEffect(() => {
    try {
      localStorage.setItem('downloadBandwidthUnit', bandwidthUnit)
    } catch {
      // ignore
    }
  }, [bandwidthUnit])
  const handleResumePolicyChange = async (policy: ResumePolicy) => {
    const previous = resumePolicy
    setResumePolicy(policy)
    try {
      await invoke('set_download_resume_policy', { policy })
      setSettingsError('')
    } catch (error) {
      setResumePolicy(previous)
      setSettingsError(settingsFailureText(ui.settingsSaveFailed, error))
    }
  }
  const handleConcurrencyChange = async (n: number) => {
    const previous = concurrency
    const next = clampConcurrency(n)
    setConcurrency(next)
    try {
      await invoke('set_download_concurrency', { n: next })
      setSettingsError('')
    } catch (error) {
      setConcurrency(previous)
      setSettingsError(settingsFailureText(ui.settingsSaveFailed, error))
    }
  }
  const handleBandwidthLimitChange = async (value: number) => {
    const previous = bandwidthLimit
    const next = Math.max(0, Number.isFinite(value) ? value : 0)
    setBandwidthLimit(next)
    try {
      await invoke('set_download_bandwidth_limit', { bytesPerSec: bandwidthToBytes(next, bandwidthUnit) })
      setSettingsError('')
    } catch (error) {
      setBandwidthLimit(previous)
      setSettingsError(settingsFailureText(ui.settingsSaveFailed, error))
    }
  }
  const handleBandwidthUnitChange = (unit: BandwidthUnit) => {
    const bytes = bandwidthToBytes(bandwidthLimit, bandwidthUnit)
    setBandwidthUnit(unit)
    setBandwidthLimit(bytesToBandwidth(bytes, unit))
  }
  const handleLowPriorityThrottleChange = async (enabled: boolean) => {
    const previous = lowPriorityThrottle
    setLowPriorityThrottle(enabled)
    try {
      await invoke('set_download_low_priority_throttle', { enabled })
      setSettingsError('')
    } catch (error) {
      setLowPriorityThrottle(previous)
      setSettingsError(settingsFailureText(ui.settingsSaveFailed, error))
    }
  }
  const resetStrategyDefaults = async () => {
    await handleResumePolicyChange('manual')
    await handleConcurrencyChange(1)
    setBandwidthUnit(DEFAULT_BANDWIDTH_UNIT)
    await handleBandwidthLimitChange(DEFAULT_BANDWIDTH_LIMIT)
    await handleLowPriorityThrottleChange(false)
  }
  const toggleSection = (section: Section) => setCollapsed(prev => ({ ...prev, [section]: !prev[section] }))
  const sourceName = (value: string) => value === 'modelscope' ? 'ModelScope' : 'HuggingFace'
  const taskStatusLabel = (task: DownloadProgress) => ({
    active: t.downloadPage.active,
    paused: t.downloadPage.paused,
    pausing: ui.pausing,
    queued: t.downloadPage.queued,
    completed: t.modelRepo.done,
    cancelled: t.modelRepo.cancelled,
    error: t.downloadPage.failed,
  }[task.status] || task.status)
  const tasksByFile = useMemo(() => new Map(
    Object.values(downloadTasks).map(task => [
      downloadFileKey(task.source, task.repoId, task.remotePath, task.saveDir),
      task,
    ]),
  ), [downloadTasks])
  const taskForFile = (file: MsFileEntry) => tasksByFile.get(
    downloadFileKey(source, browsedRepoId, file.path || file.name, saveDir),
  )

  const modelTypeLabel = (fileType: string) => {
    switch (fileType) {
      case 'mmproj':
        return t.modelRepo.typeMmproj
      case 'imatrix':
        return t.modelRepo.typeImatrix
      default:
        return t.modelRepo.typeModel
    }
  }
  const modelTypeColor = (fileType: string) => {
    switch (fileType) {
      case 'mmproj':
        return 'bg-purple-100 text-purple-700 dark:bg-purple-950/40 dark:text-purple-300'
      case 'imatrix':
        return 'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300'
      default:
        return 'bg-blue-100 text-blue-700 dark:bg-blue-950/40 dark:text-blue-300'
    }
  }

  const handleBrowse = async () => {
    if (!repoId.trim()) {
      setStatus(t.modelRepo.inputRepoId)
      return
    }

    const browseStartedAt = Date.now()
    setBrowsing(true)
    setStatus(t.modelRepo.querying)

    try {
      const trimmedRepoId = repoId.trim()
      const result = source === 'modelscope'
        ? await browseModelscope(trimmedRepoId)
        : await browseHuggingface(trimmedRepoId)

      setFiles(result)
      setBrowsedRepoId(trimmedRepoId)
      setStatus(result.length === 0 ? t.modelRepo.notFound : `${t.modelRepo.found} ${result.length} ${t.modelRepo.files}`)

      const allTasks = useAppStore.getState().downloadTasks
      const completedTasks: DownloadProgress[] = []

      const resolvedDir = await invoke<string>('resolve_path', { path: saveDir })
      await forEachConcurrent(result, LOCAL_FILE_CHECK_CONCURRENCY, async file => {
        const localPath = pathJoin(resolvedDir, trimmedRepoId, file.path || file.name)
        try {
          const actualSize = await invoke<number | null>('check_local_file', { path: localPath })
          if (file.size > 0 && actualSize === file.size) {
            const existing = Object.values(allTasks).find(task =>
              task.source === source &&
              task.repoId === trimmedRepoId &&
              task.remotePath === (file.path || file.name) &&
              task.saveDir === saveDir,
            )
            const id = existing?.id || crypto.randomUUID()
            const completedAt = Date.now()
            completedTasks.push({
              id,
              fileName: file.name,
              remotePath: file.path || file.name,
              fileType: file.file_type,
              saveDir,
              repoId: trimmedRepoId,
              source,
              downloaded: actualSize,
              total: file.size,
              speed: 0,
              status: 'completed',
              path: localPath,
              version: existing?.version ?? 0,
              createdAt: existing?.createdAt ?? completedAt,
              updatedAt: completedAt,
              completedAt: existing?.completedAt ?? completedAt,
            })
          }
        } catch {
          // file not found
        }
      })

      useAppStore.setState(state => {
        const tasks = { ...state.downloadTasks }
        for (const completed of completedTasks) {
          const latest = Object.values(tasks).find(task =>
            task.source === completed.source
            && task.repoId === completed.repoId
            && task.remotePath === completed.remotePath
            && task.saveDir === completed.saveDir,
          )
          if ((latest?.version ?? 0) > (completed.version ?? 0)) continue
          if (
            latest
            && (latest.updatedAt ?? 0) > browseStartedAt
            && latest.status !== 'completed'
          ) continue
          if (latest && ['active', 'queued', 'pausing'].includes(latest.status)) continue
          const id = latest?.id || completed.id
          tasks[id] = {
            ...latest,
            ...completed,
            id,
            version: Math.max(latest?.version ?? 0, completed.version ?? 0),
          }
        }
        return { downloadTasks: tasks }
      })
    } catch (error: any) {
      setStatus(`${t.modelRepo.queryFailed}${typeof error === 'string' ? error : t.modelRepo.networkError}`)
    } finally {
      setBrowsing(false)
    }
  }

  const saveDirPersist = (dir: string) => {
    setSaveDir(dir)
    try {
      localStorage.setItem('downloadSaveDir', dir)
    } catch {
      // ignore
    }
  }

  const handleBrowseSaveDir = async () => {
    try {
      const dir = await open({ directory: true, title: t.modelRepo.saveDir })
      if (dir) saveDirPersist(dir as string)
    } catch {
      // ignore
    }
  }

  const handleDownloadFile = (file: MsFileEntry) => {
    addToDownloadQueue({ repoId: browsedRepoId, source, files: [file], saveDir })
  }

  const handleDownloadAll = () => {
    const pending = files.filter(file => taskForFile(file)?.status !== 'completed')
    if (pending.length > 0) {
      addToDownloadQueue({ repoId: browsedRepoId, source, files: pending, saveDir })
    }
  }

  const handlePause = (file: MsFileEntry) => {
    const task = taskForFile(file)
    if (!task) return
    const state = useAppStore.getState()
    setDownloadTasks({ ...state.downloadTasks, [task.id]: { ...task, status: 'pausing' } })
    pauseFileDownload(task.id, task.runId)
  }

  const handleCancel = (file: MsFileEntry) => {
    const task = taskForFile(file)
    if (!task) return
    cancelAndCleanupDownload(task.id, file.name, pathJoin(task.saveDir, browsedRepoId, file.path || file.name), task.runId, task.version)
  }

  const handleResumePersisted = (task: DownloadProgress) => {
    resumeDownloadTask(task.id)
  }

  const handleCancelPersisted = (task: DownloadProgress) => {
    void cancelAndCleanupDownload(task.id, task.fileName, pathJoin(task.saveDir, task.repoId, task.remotePath || task.fileName), task.runId, task.version)
  }

  const handleDeleteCompleted = async (task: DownloadProgress) => {
    try {
      await invoke('delete_managed_local_file', {
        filePath: task.path!,
        saveDir: task.saveDir,
        repoId: task.repoId,
        remotePath: task.remotePath || task.fileName,
      })
      useAppStore.setState(state => {
        const tasks = { ...state.downloadTasks }
        delete tasks[task.id]
        return { downloadTasks: tasks }
      })
    } catch (error) {
      useAppStore.getState().addRuntimeWarning(`local model cleanup failed: ${String(error)}`)
    }
  }

  const taskLocalPath = (task: DownloadProgress) => task.path || pathJoin(task.saveDir, task.repoId, task.remotePath || task.fileName)

  const taskEtaText = (task: DownloadProgress) => {
    const eta = formatETA(task.downloaded, task.total, task.speed || 0)
    return eta || ui.etaPending
  }

  const retrySuggestion = (task: DownloadProgress) => {
    const message = (task.error || '').toLowerCase()
    if (task.remoteChanged || message.includes('corrupt') || message.includes('does not support resume') || message.includes('larger than expected')) {
      return ui.retryAdviceRedownload
    }
    if (message.includes('403') || message.includes('404') || message.includes('access denied') || message.includes('not found')) {
      return ui.retryAdvicePermission
    }
    if (message.includes('429') || message.includes('timeout') || message.includes('network') || message.includes('connection') || message.includes('http 5')) {
      return ui.retryAdviceNetwork
    }
    return ui.retryAdviceGeneric
  }

  const handleOpenTaskLocation = async (task: DownloadProgress) => {
    try {
      await openModelFolder(taskLocalPath(task))
      setModelImportStatuses(statuses => ({ ...statuses, [task.id]: '' }))
    } catch (error: any) {
      const message = typeof error === 'string' ? error : String(error || ui.importFailed)
      setModelImportStatuses(statuses => ({ ...statuses, [task.id]: message }))
    }
  }

  const handleImportCompletedTask = async (task: DownloadProgress) => {
    const error = await scanModels([task.saveDir])
    const message = error ? `${ui.importFailed}: ${error}` : ui.importedToRepo
    setModelImportStatuses(statuses => ({ ...statuses, [task.id]: message }))
  }

  const allTasks = useMemo(() => Object.values(downloadTasks), [downloadTasks])
  const activeTasks = useMemo(() => allTasks.filter(task => task.status === 'active'), [allTasks])
  const pausedTasks = useMemo(() => allTasks.filter(task => task.status === 'paused' || task.status === 'pausing'), [allTasks])
  const failedTasks = useMemo(() => allTasks.filter(task => task.status === 'error'), [allTasks])
  const completedTasks = useMemo(() => allTasks.filter(task => task.status === 'completed' || task.status === 'cancelled'), [allTasks])
  const selectedTask = useMemo(() => {
    if (selectedTaskId && downloadTasks[selectedTaskId]) return downloadTasks[selectedTaskId]
    return activeTasks[0] || failedTasks[0] || completedTasks.find(task => task.status === 'completed') || pausedTasks[0] || null
  }, [activeTasks, completedTasks, downloadTasks, failedTasks, pausedTasks, selectedTaskId])

  const hasActive = activeTasks.length > 0
  const hasPaused = pausedTasks.length > 0
  const queueFileCount = downloadQueue.reduce((sum, entry) => sum + entry.files.length, 0)
  const hasAnyActionable = hasActive || hasPaused || queueFileCount > 0
  const activeSpeed = activeTasks.reduce((sum, task) => sum + (task.speed || 0), 0)
  const actionableCount = activeTasks.length + pausedTasks.length + queueFileCount
  const resumePolicyLabel = resumePolicy === 'auto_on_launch' ? ui.autoShort : ui.manualShort
  const bandwidthSummary = bandwidthLimit > 0 ? `${bandwidthLimit} ${bandwidthUnit}` : ui.unlimited
  const completedBytes = completedTasks
    .filter(task => task.status === 'completed')
    .reduce((sum, task) => sum + task.total, 0)

  const queueGroups = useMemo(() => {
    const map = new Map<string, { entry: typeof downloadQueue[number]; files: MsFileEntry[] }>()
    for (const entry of downloadQueue) {
      const key = `${entry.source}:${entry.repoId}`
      if (!map.has(key)) map.set(key, { entry, files: [] })
      map.get(key)!.files.push(...entry.files)
    }
    return Array.from(map.entries()).map(([key, group]) => ({ key, ...group }))
  }, [downloadQueue])

  const queuedTaskIds = useMemo(() => downloadQueue.flatMap(entry => entry.files.map(file => file.task_id).filter(Boolean) as string[]), [downloadQueue])

  const storagePreview = useMemo(() => {
    const repo = browsedRepoId || repoId.trim() || '<repo>'
    return pathJoin(saveDir || DEFAULT_SAVE_DIR, repo, '<file>')
  }, [browsedRepoId, repoId, saveDir])

  const queueStateDetail = (task: DownloadProgress) => {
    const index = queuedTaskIds.indexOf(task.id)
    if (index >= 0) return `${ui.queuePosition} ${index + 1} / ${queuedTaskIds.length}`
    if (task.status === 'active') return ui.runningNow
    if (task.status === 'queued') return t.downloadPage.queued
    return ui.notQueued
  }

  const renderTaskCard = (task: DownloadProgress) => {
    const pct = task.total > 0 ? Math.max(0, Math.min(100, (task.downloaded / task.total) * 100)) : 0
    const isSelected = selectedTask?.id === task.id
    const modelImportStatus = modelImportStatuses[task.id] || ''
    const barColor = task.status === 'active'
      ? 'bg-blue-500'
      : (task.status === 'paused' || task.status === 'pausing')
        ? 'bg-amber-500'
        : task.status === 'error'
          ? 'bg-rose-500'
          : 'bg-emerald-500'

    return (
      <div
        key={task.id}
        role="button"
        tabIndex={0}
        onClick={() => setSelectedTaskId(task.id)}
        onKeyDown={event => {
          if (event.key === 'Enter' || event.key === ' ') setSelectedTaskId(task.id)
        }}
        className={`min-w-0 border-b px-4 py-4 text-left outline-none transition last:border-b-0 ${isSelected ? 'border-blue-200 bg-blue-50/60 dark:border-blue-900/50 dark:bg-blue-950/20' : 'border-slate-200 hover:bg-slate-50 dark:border-slate-800 dark:hover:bg-slate-900/45'}`}
      >
        <div className="flex min-w-0 flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <StatusBadge status={task.status} label={taskStatusLabel(task)} />
              {task.remoteChanged && (
                <span title={t.downloadPage.remoteChangedTip}>
                  <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-amber-500" />
                </span>
              )}
            </div>
            <div className="mt-2 truncate text-sm font-medium text-slate-900 dark:text-slate-100" title={task.fileName}>
              {task.fileName}
            </div>
            <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs text-slate-500 dark:text-slate-400">
              <span className="shrink-0">{sourceName(task.source)}</span>
              <span className="min-w-0 max-w-full truncate" title={task.repoId}>{task.repoId}</span>
            </div>
            {task.remotePath && task.remotePath !== task.fileName && (
              <PathText value={task.remotePath} maxLength={88} className="mt-1 max-w-full text-slate-400 dark:text-slate-500" />
            )}
          </div>

          <div className="flex shrink-0 flex-wrap items-center gap-1 sm:justify-end">
            {task.status === 'active' && (
              <>
                <button
                  onClick={() => {
                    const state = useAppStore.getState()
                    setDownloadTasks({ ...state.downloadTasks, [task.id]: { ...task, status: 'pausing' } })
                    pauseFileDownload(task.id, task.runId)
                  }}
                  className="rounded-lg p-2 text-amber-600 transition hover:bg-amber-50 dark:hover:bg-amber-950/30"
                  title={t.modelRepo.pause}
                >
                  <Pause className="h-4 w-4" />
                </button>
                <button
                  onClick={() => cancelFileDownload(task.id, task.runId)}
                  className="rounded-lg p-2 text-rose-500 transition hover:bg-rose-50 dark:hover:bg-rose-950/30"
                  title={t.modelRepo.cancel}
                >
                  <Square className="h-4 w-4" />
                </button>
              </>
            )}

            {task.status === 'paused' && (
              <>
                <button
                  onClick={() => handleResumePersisted(task)}
                  className="rounded-lg p-2 text-emerald-600 transition hover:bg-emerald-50 dark:hover:bg-emerald-950/30"
                  title={t.modelRepo.resume}
                >
                  <Play className="h-4 w-4" />
                </button>
                <button
                  onClick={() => handleCancelPersisted(task)}
                  className="rounded-lg p-2 text-rose-500 transition hover:bg-rose-50 dark:hover:bg-rose-950/30"
                  title={t.modelRepo.cancel}
                >
                  <X className="h-4 w-4" />
                </button>
              </>
            )}

            {task.status === 'completed' && (
              <>
                <button
                  onClick={() => handleImportCompletedTask(task)}
                  className="inline-flex items-center gap-1 rounded-lg bg-emerald-50 px-2.5 py-2 text-xs font-medium text-emerald-700 transition hover:bg-emerald-100 dark:bg-emerald-950/40 dark:text-emerald-300 dark:hover:bg-emerald-950/70"
                  title={ui.addToModelRepo}
                >
                  <Database className="h-3.5 w-3.5" />
                  <span>{ui.addToModelRepo}</span>
                </button>
                <button
                  onClick={() => handleOpenTaskLocation(task)}
                  className="inline-flex items-center gap-1 rounded-lg bg-slate-100 px-2.5 py-2 text-xs font-medium text-slate-700 transition hover:bg-slate-200 dark:bg-slate-800 dark:text-slate-300 dark:hover:bg-slate-700"
                  title={ui.openLocation}
                >
                  <FolderOpen className="h-3.5 w-3.5" />
                  <span>{ui.openLocation}</span>
                </button>
              </>
            )}

            {(task.status === 'completed' || task.status === 'cancelled') && task.path && (
              <button
                onClick={() => void handleDeleteCompleted(task)}
                className="rounded-lg p-2 text-slate-500 transition hover:bg-slate-100 hover:text-rose-500 dark:hover:bg-slate-800"
                title={t.common.delete}
              >
                <Trash2 className="h-4 w-4" />
              </button>
            )}

            {task.status === 'error' && (
              <>
                <button
                  onClick={() => retryFailedDownload(task.id)}
                  className="inline-flex items-center gap-1 rounded-lg bg-blue-50 px-2.5 py-2 text-xs font-medium text-blue-700 transition hover:bg-blue-100 dark:bg-blue-950/40 dark:text-blue-300 dark:hover:bg-blue-950/70"
                  title={t.downloadPage.retry}
                >
                  <RotateCcw className="h-3.5 w-3.5" />
                  <span>{t.downloadPage.retry}</span>
                </button>
                <button
                  onClick={() => redownloadFile(task.id)}
                  className="inline-flex items-center gap-1 rounded-lg bg-amber-50 px-2.5 py-2 text-xs font-medium text-amber-700 transition hover:bg-amber-100 dark:bg-amber-950/40 dark:text-amber-300 dark:hover:bg-amber-950/70"
                  title={t.downloadPage.redownload}
                >
                  <RefreshCw className="h-3.5 w-3.5" />
                  <span>{t.downloadPage.redownload}</span>
                </button>
              </>
            )}
          </div>
        </div>

        <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-800">
          <div className={`h-full rounded-full ${barColor} transition-all duration-300`} style={{ width: `${pct}%` }} />
        </div>

        <div className="mt-3 grid min-w-0 grid-cols-2 gap-2 text-xs sm:grid-cols-4">
          <div className="min-w-0">
            <div className="text-slate-400 dark:text-slate-500">{ui.progress}</div>
            <div className="mt-0.5 truncate text-slate-600 dark:text-slate-300">{pct.toFixed(1)}%</div>
          </div>
          <div className="min-w-0">
            <div className="text-slate-400 dark:text-slate-500">{ui.downloaded}</div>
            <div className="mt-0.5 truncate text-slate-600 dark:text-slate-300">{formatSize(task.downloaded)} / {formatSize(task.total)}</div>
          </div>
          <div className="min-w-0">
            <div className="text-slate-400 dark:text-slate-500">{ui.speed}</div>
            <div className="mt-0.5 truncate text-slate-600 dark:text-slate-300">{task.status === 'active' ? formatSpeed(task.speed || 0) : '-'}</div>
          </div>
          <div className="min-w-0">
            <div className="text-slate-400 dark:text-slate-500">{ui.eta}</div>
            <div className="mt-0.5 truncate text-slate-600 dark:text-slate-300">{task.status === 'active' ? taskEtaText(task) : queueStateDetail(task)}</div>
          </div>
        </div>

        {task.status === 'error' && (
          <div className="mt-3 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-xs leading-5 text-rose-700 dark:border-rose-900/60 dark:bg-rose-950/25 dark:text-rose-300">
            <div className="truncate font-medium" title={task.error || t.modelRepo.failed}>{task.error || t.modelRepo.failed}</div>
            <div className="mt-1 text-rose-600 dark:text-rose-300">{retrySuggestion(task)}</div>
          </div>
        )}

        {task.status === 'completed' && (
          <PathText value={taskLocalPath(task)} maxLength={110} className="mt-3 max-w-full text-xs text-slate-400 dark:text-slate-500" />
        )}

        {modelImportStatus && isSelected && (
          <div className="mt-3 truncate rounded-lg border border-slate-200 bg-white px-3 py-2 text-xs text-slate-500 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-400" title={modelImportStatus}>
            {modelImportStatus}
          </div>
        )}
      </div>
    )
  }

  const renderFileRow = (file: MsFileEntry) => {
    const task = taskForFile(file)
    const pending = task?.status === 'active' || task?.status === 'pausing' || task?.status === 'queued'
    const pct = task && task.total > 0
      ? Math.max(0, Math.min(100, (task.downloaded / task.total) * 100))
      : 0

    return (
      <div key={file.path} className="min-w-0 border-b border-slate-200 px-4 py-4 last:border-b-0 dark:border-slate-800">
        <div className="flex min-w-0 flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-medium text-slate-900 dark:text-slate-100" title={file.name}>{file.name}</div>
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-slate-500 dark:text-slate-400">
              <span className={`rounded-full px-2 py-1 ${modelTypeColor(file.file_type)}`}>{modelTypeLabel(file.file_type)}</span>
              <span>{formatSize(file.size)}</span>
            </div>
            {file.path && file.path !== file.name && (
              <PathText value={file.path} maxLength={88} className="mt-1 max-w-full text-slate-400 dark:text-slate-500" />
            )}
          </div>

          <div className="flex shrink-0 flex-wrap items-center gap-2 sm:justify-end">
            {!task && (
              <button
                onClick={() => handleDownloadFile(file)}
                className="inline-flex items-center gap-1 rounded-lg bg-blue-600 px-3 py-2 text-xs font-medium text-white transition hover:bg-blue-700"
              >
                <Download className="h-3.5 w-3.5" />
                <span>{t.modelRepo.downloadBtn}</span>
              </button>
            )}

            {(task?.status === 'active' || task?.status === 'pausing') && (
              <>
                {task.status === 'active' ? (
                  <button
                    onClick={() => handlePause(file)}
                    className="rounded-lg p-2 text-amber-600 transition hover:bg-amber-50 dark:hover:bg-amber-950/30"
                    title={t.modelRepo.pause}
                  >
                    <Pause className="h-4 w-4" />
                  </button>
                ) : (
                  <span className="rounded-lg bg-amber-50 px-3 py-2 text-xs text-amber-700 dark:bg-amber-950/40 dark:text-amber-300">
                    {ui.pausing}
                  </span>
                )}
                <button
                  onClick={() => handleCancel(file)}
                  className="rounded-lg p-2 text-rose-500 transition hover:bg-rose-50 dark:hover:bg-rose-950/30"
                  title={t.modelRepo.cancel}
                >
                  <Square className="h-4 w-4" />
                </button>
              </>
            )}

            {task?.status === 'paused' && (
              <>
                <button
                  onClick={() => handleResumePersisted(task)}
                  className="rounded-lg p-2 text-emerald-600 transition hover:bg-emerald-50 dark:hover:bg-emerald-950/30"
                  title={t.modelRepo.resume}
                >
                  <Play className="h-4 w-4" />
                </button>
                <button
                  onClick={() => handleCancel(file)}
                  className="rounded-lg p-2 text-rose-500 transition hover:bg-rose-50 dark:hover:bg-rose-950/30"
                  title={t.modelRepo.cancel}
                >
                  <X className="h-4 w-4" />
                </button>
              </>
            )}

            {task?.status === 'error' && (
              <button
                onClick={() => handleDownloadFile(file)}
                className="inline-flex items-center gap-1 rounded-lg bg-blue-50 px-3 py-2 text-xs font-medium text-blue-700 transition hover:bg-blue-100 dark:bg-blue-950/40 dark:text-blue-300 dark:hover:bg-blue-950/70"
              >
                <RotateCcw className="h-3.5 w-3.5" />
                <span>{t.modelRepo.retry}</span>
              </button>
            )}
          </div>
        </div>

        {pending && (
          <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-800">
            <div className="h-full rounded-full bg-blue-500" style={{ width: `${pct}%` }} />
          </div>
        )}

        {task && (
          <div className="mt-3 flex min-w-0 flex-wrap items-center justify-between gap-2 text-xs text-slate-500 dark:text-slate-400">
            {task.status === 'active' || task.status === 'pausing' ? (
              <span className="truncate">{formatSize(task.downloaded)} / {formatSize(task.total)} &middot; {formatSpeed(task.speed || 0)} &middot; {formatETA(task.downloaded, task.total, task.speed || 0)}</span>
            ) : task.status === 'paused' ? (
              <span>{formatSize(task.downloaded)} / {formatSize(task.total)}</span>
            ) : task.status === 'completed' ? (
              <span className="text-emerald-600 dark:text-emerald-400">{t.modelRepo.done}</span>
            ) : task.status === 'queued' ? (
              <span>{t.downloadPage.queued}</span>
            ) : task.status === 'error' ? (
              <span className="max-w-[280px] truncate text-rose-500" title={task.error}>{task.error || t.modelRepo.failed}</span>
            ) : null}
          </div>
        )}
      </div>
    )
  }

  const renderSelectedTaskSummary = () => {
    if (!selectedTask) {
      return (
        <section className={`${surfaceClassName} min-w-0 overflow-hidden`}>
          <div className="border-b border-slate-200 px-4 py-4 dark:border-slate-800">
            <div className="text-sm font-semibold text-slate-900 dark:text-slate-100">{ui.taskSummary}</div>
          </div>
          <div className="px-4 py-8 text-sm text-slate-400 dark:text-slate-500">{ui.noSelectedTask}</div>
        </section>
      )
    }

    const pct = selectedTask.total > 0
      ? Math.max(0, Math.min(100, (selectedTask.downloaded / selectedTask.total) * 100))
      : 0
    const isCompleted = selectedTask.status === 'completed'
    const isFailed = selectedTask.status === 'error'
    const modelImportStatus = modelImportStatuses[selectedTask.id] || ''

    return (
      <section className={`${surfaceClassName} min-w-0 overflow-hidden`}>
        <div className="border-b border-slate-200 px-4 py-4 dark:border-slate-800">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="text-sm font-semibold text-slate-900 dark:text-slate-100">{ui.taskSummary}</div>
              <div className="mt-1 truncate text-xs text-slate-500 dark:text-slate-400" title={selectedTask.fileName}>{selectedTask.fileName}</div>
            </div>
            <StatusBadge status={selectedTask.status} label={taskStatusLabel(selectedTask)} />
          </div>
        </div>

        <div className="space-y-4 px-4 py-4">
          <div>
            <div className="mb-2 flex items-center justify-between gap-2 text-xs">
              <span className="font-medium text-slate-500 dark:text-slate-400">{ui.progress}</span>
              <span className="text-slate-600 dark:text-slate-300">{pct.toFixed(1)}%</span>
            </div>
            <div className="h-2 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-800">
              <div className="h-full rounded-full bg-blue-500 transition-all duration-300" style={{ width: `${pct}%` }} />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-3 text-xs">
            <div className="min-w-0 rounded-lg border border-slate-200 px-3 py-2.5 dark:border-slate-800">
              <div className="text-slate-400 dark:text-slate-500">{ui.downloaded}</div>
              <div className="mt-1 truncate font-medium text-slate-700 dark:text-slate-200">{formatSize(selectedTask.downloaded)} / {formatSize(selectedTask.total)}</div>
            </div>
            <div className="min-w-0 rounded-lg border border-slate-200 px-3 py-2.5 dark:border-slate-800">
              <div className="text-slate-400 dark:text-slate-500">{ui.speed}</div>
              <div className="mt-1 truncate font-medium text-slate-700 dark:text-slate-200">{selectedTask.status === 'active' ? formatSpeed(selectedTask.speed || 0) : '-'}</div>
            </div>
            <div className="min-w-0 rounded-lg border border-slate-200 px-3 py-2.5 dark:border-slate-800">
              <div className="text-slate-400 dark:text-slate-500">{ui.eta}</div>
              <div className="mt-1 truncate font-medium text-slate-700 dark:text-slate-200">{selectedTask.status === 'active' ? taskEtaText(selectedTask) : '-'}</div>
            </div>
            <div className="min-w-0 rounded-lg border border-slate-200 px-3 py-2.5 dark:border-slate-800">
              <div className="text-slate-400 dark:text-slate-500">{ui.queueState}</div>
              <div className="mt-1 truncate font-medium text-slate-700 dark:text-slate-200">{queueStateDetail(selectedTask)}</div>
            </div>
          </div>

          <div className="space-y-2 text-xs">
            <div className="text-slate-400 dark:text-slate-500">{ui.localPath}</div>
            <PathText value={taskLocalPath(selectedTask)} maxLength={120} className="max-w-full text-slate-600 dark:text-slate-300" />
          </div>

          {isFailed && (
            <div className="rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-xs leading-5 text-rose-700 dark:border-rose-900/60 dark:bg-rose-950/25 dark:text-rose-300">
              <div className="font-medium">{ui.failedReason}</div>
              <div className="mt-1 break-words">{selectedTask.error || t.modelRepo.failed}</div>
              <div className="mt-2 font-medium">{ui.retryAdvice}</div>
              <div className="mt-1">{retrySuggestion(selectedTask)}</div>
            </div>
          )}

          {isCompleted && (
            <div className="grid grid-cols-2 gap-2">
              <button
                type="button"
                onClick={() => handleImportCompletedTask(selectedTask)}
                className="inline-flex items-center justify-center gap-1 rounded-lg bg-emerald-50 px-3 py-2 text-xs font-medium text-emerald-700 transition hover:bg-emerald-100 dark:bg-emerald-950/40 dark:text-emerald-300 dark:hover:bg-emerald-950/70"
              >
                <Database className="h-3.5 w-3.5" />
                <span>{ui.addToModelRepo}</span>
              </button>
              <button
                type="button"
                onClick={() => handleOpenTaskLocation(selectedTask)}
                className="inline-flex items-center justify-center gap-1 rounded-lg bg-slate-100 px-3 py-2 text-xs font-medium text-slate-700 transition hover:bg-slate-200 dark:bg-slate-800 dark:text-slate-300 dark:hover:bg-slate-700"
              >
                <FolderOpen className="h-3.5 w-3.5" />
                <span>{ui.openLocation}</span>
              </button>
            </div>
          )}

          {modelImportStatus && (
            <div className="truncate rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-xs text-slate-500 dark:border-slate-800 dark:bg-slate-950/50 dark:text-slate-400" title={modelImportStatus}>
              {modelImportStatus}
            </div>
          )}
        </div>
      </section>
    )
  }

  if (initialLoading) {
    return (
      <div className="space-y-5">
        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          {[1, 2, 3, 4].map(i => (
            <div key={i} className={`${surfaceClassName} animate-pulse px-4 py-4`}>
              <div className="h-6 w-24 rounded bg-slate-200 dark:bg-slate-800" />
              <div className="mt-4 h-8 w-16 rounded bg-slate-200 dark:bg-slate-800" />
              <div className="mt-2 h-4 w-28 rounded bg-slate-200 dark:bg-slate-800" />
            </div>
          ))}
        </div>
        <div className="grid gap-5 2xl:grid-cols-[minmax(0,1.65fr)_380px]">
          <div className={`${surfaceClassName} h-[520px] animate-pulse bg-slate-100 dark:bg-slate-900`} />
          <div className="space-y-5">
            <div className={`${surfaceClassName} h-[220px] animate-pulse bg-slate-100 dark:bg-slate-900`} />
            <div className={`${surfaceClassName} h-[275px] animate-pulse bg-slate-100 dark:bg-slate-900`} />
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-5">
      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <MetricTile label={t.downloadPage.queue} value={queueFileCount} detail={downloadQueue.length > 0 ? `${downloadQueue.length} ${ui.batches}` : t.downloadPage.noQueue} tone="slate" />
        <MetricTile label={t.downloadPage.active} value={activeTasks.length} detail={hasActive ? formatSpeed(activeTasks.reduce((sum, task) => sum + task.speed, 0)) : t.downloadPage.noActive} tone="blue" />
        <MetricTile label={t.downloadPage.paused} value={pausedTasks.length} detail={hasPaused ? `${pausedTasks.filter(task => task.status === 'paused').length} ${ui.resumable}` : t.downloadPage.noPaused} tone="amber" />
        <MetricTile label={t.downloadPage.completed} value={completedTasks.length} detail={completedBytes > 0 ? formatSize(completedBytes) : t.downloadPage.noCompleted} tone="emerald" />
      </div>

      <div className={`${surfaceClassName} min-w-0 overflow-hidden`}>
        <div className="flex flex-col gap-3 px-4 py-3 2xl:flex-row 2xl:items-center 2xl:justify-between">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-sm font-semibold text-slate-900 dark:text-slate-100">{ui.batchTools}</span>
              <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[11px] text-slate-500 dark:bg-slate-800 dark:text-slate-400">
                {hasAnyActionable ? `${actionableCount} ${ui.controllable}` : ui.noActionable}
              </span>
            </div>
            <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-xs text-slate-500 dark:text-slate-400">
              <span>{ui.totalSpeed}: {formatSpeed(activeSpeed)}</span>
              <span>{ui.strategy}: {resumePolicyLabel}</span>
              <span>{ui.concurrencyShort}: {concurrency}</span>
              <span>{ui.bandwidth}: {bandwidthSummary}</span>
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2 2xl:justify-end">
            <button
              onClick={() => pauseAllDownloads()}
              disabled={!hasActive}
              className="inline-flex items-center gap-1 rounded-lg bg-amber-50 px-3 py-2 text-xs font-medium text-amber-700 transition hover:bg-amber-100 disabled:cursor-not-allowed disabled:opacity-40 dark:bg-amber-950/40 dark:text-amber-300 dark:hover:bg-amber-950/70"
            >
              <Pause className="h-3.5 w-3.5" />
              <span>{t.downloadPage.pauseAll}</span>
            </button>
            <button
              onClick={() => resumeAllDownloads()}
              disabled={!hasPaused}
              className="inline-flex items-center gap-1 rounded-lg bg-emerald-50 px-3 py-2 text-xs font-medium text-emerald-700 transition hover:bg-emerald-100 disabled:cursor-not-allowed disabled:opacity-40 dark:bg-emerald-950/40 dark:text-emerald-300 dark:hover:bg-emerald-950/70"
            >
              <Play className="h-3.5 w-3.5" />
              <span>{t.downloadPage.resumeAll}</span>
            </button>
            <button
              onClick={() => cancelAllDownloads()}
              disabled={!hasActive && !hasPaused && queueFileCount === 0}
              className="inline-flex items-center gap-1 rounded-lg bg-rose-50 px-3 py-2 text-xs font-medium text-rose-700 transition hover:bg-rose-100 disabled:cursor-not-allowed disabled:opacity-40 dark:bg-rose-950/40 dark:text-rose-300 dark:hover:bg-rose-950/70"
            >
              <Square className="h-3.5 w-3.5" />
              <span>{t.downloadPage.cancelAll}</span>
            </button>
          </div>
        </div>
      </div>

      <div className="grid min-w-0 gap-5 2xl:grid-cols-[minmax(0,1.65fr)_380px]">
        <section className={`${surfaceClassName} min-w-0 overflow-hidden`}>
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-200 px-4 py-4 dark:border-slate-800">
            <div className="min-w-0">
              <div className="text-sm font-semibold text-slate-900 dark:text-slate-100">{ui.addTaskTitle}</div>
              <div className="mt-1 truncate text-xs text-slate-500 dark:text-slate-400" title={browsedRepoId || status || t.downloadPage.repoLabel}>
                {browsedRepoId ? `${sourceName(source)} · ${browsedRepoId}` : ui.addTaskSub}
              </div>
            </div>
            {files.length > 0 && (
              <button
                onClick={handleDownloadAll}
                className="inline-flex items-center gap-1 rounded-lg bg-blue-600 px-3 py-2 text-xs font-medium text-white transition hover:bg-blue-700"
              >
                <Download className="h-3.5 w-3.5" />
                <span>{t.downloadPage.downloadAll} ({files.filter(file => taskForFile(file)?.status !== 'completed').length})</span>
              </button>
            )}
          </div>

          <div className="space-y-4 px-4 py-4">
            <div className="grid min-w-0 gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(320px,0.62fr)]">
              <div className="min-w-0 rounded-lg border border-slate-200 bg-slate-50/70 p-3 dark:border-slate-800 dark:bg-slate-950/35">
                <div className="mb-3 flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="text-xs font-semibold text-slate-900 dark:text-slate-100">{ui.sourceSection}</div>
                    <div className="mt-1 text-[11px] leading-5 text-slate-500 dark:text-slate-400">{ui.sourceHelp}</div>
                  </div>
                  <div className="inline-flex max-w-full shrink-0 rounded-lg bg-slate-200/70 p-1 dark:bg-slate-800" data-guide="download-source">
                    <button
                      onClick={() => { setSource('modelscope'); setFiles([]); setStatus('') }}
                      className={`rounded-md px-3 py-1.5 text-xs font-medium transition ${source === 'modelscope' ? 'bg-white text-slate-900 shadow-sm dark:bg-slate-900 dark:text-white' : 'text-slate-500 hover:text-slate-900 dark:text-slate-400 dark:hover:text-white'}`}
                    >
                      ModelScope
                    </button>
                    <button
                      onClick={() => { setSource('huggingface'); setFiles([]); setStatus('') }}
                      className={`rounded-md px-3 py-1.5 text-xs font-medium transition ${source === 'huggingface' ? 'bg-white text-slate-900 shadow-sm dark:bg-slate-900 dark:text-white' : 'text-slate-500 hover:text-slate-900 dark:text-slate-400 dark:hover:text-white'}`}
                    >
                      HuggingFace
                    </button>
                  </div>
                </div>

                <div className="flex min-w-0 flex-col gap-2 sm:flex-row">
                  <input
                    type="text"
                    value={repoId}
                    onChange={e => setRepoId(e.target.value)}
                    onKeyDown={e => e.key === 'Enter' && handleBrowse()}
                    placeholder={source === 'modelscope' ? t.modelRepo.repoIdPlaceholder : t.modelRepo.hfRepoIdPlaceholder}
                    className="min-w-0 flex-1 rounded-lg border border-slate-200 bg-white px-3 py-2.5 text-sm text-slate-900 outline-none transition placeholder:text-slate-400 focus:border-blue-500 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-100"
                  />
                  <button
                    onClick={handleBrowse}
                    disabled={browsing}
                    className="inline-flex shrink-0 items-center gap-1 rounded-lg bg-blue-600 px-4 py-2.5 text-sm font-medium text-white transition hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    <Download className="h-4 w-4" />
                    <span>{t.modelRepo.browseFiles}</span>
                  </button>
                </div>
              </div>

              <div className="min-w-0 rounded-lg border border-slate-200 bg-slate-50/70 p-3 dark:border-slate-800 dark:bg-slate-950/35">
                <div className="mb-3 min-w-0">
                  <div className="text-xs font-semibold text-slate-900 dark:text-slate-100">{ui.storageSection}</div>
                  <div className="mt-1 text-[11px] leading-5 text-slate-500 dark:text-slate-400">{ui.storageHelp}</div>
                </div>

                <div className="flex min-w-0 gap-2" data-guide="download-save-dir">
                  <input
                    type="text"
                    value={saveDir}
                    onChange={e => saveDirPersist(e.target.value)}
                    placeholder={ui.storageSection}
                    aria-label={ui.storageSection}
                    className="min-w-0 flex-1 rounded-lg border border-slate-200 bg-white px-3 py-2.5 text-sm text-slate-900 outline-none transition placeholder:text-slate-400 focus:border-blue-500 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-100"
                  />
                  <button
                    onClick={handleBrowseSaveDir}
                    title={ui.chooseFolder}
                    aria-label={ui.chooseFolder}
                    className="inline-flex h-[42px] w-[42px] shrink-0 items-center justify-center rounded-lg border border-slate-200 bg-white text-slate-600 transition hover:bg-slate-50 hover:text-slate-900 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-300 dark:hover:bg-slate-900 dark:hover:text-white"
                  >
                    <FolderOpen className="h-4 w-4" />
                  </button>
                </div>

                <div className="mt-3 rounded-lg border border-slate-200 bg-white px-3 py-2 dark:border-slate-800 dark:bg-slate-950">
                  <div className="text-[11px] font-medium text-slate-500 dark:text-slate-400">{ui.storagePreview}</div>
                  <PathText value={storagePreview} maxLength={96} className="mt-1 max-w-full text-xs text-slate-600 dark:text-slate-300" />
                </div>

                {status && (
                  <div className="mt-3 truncate rounded-lg border border-slate-200 bg-white px-3 py-2 text-xs text-slate-500 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-400" title={status}>
                    {status}
                  </div>
                )}
              </div>
            </div>
          </div>

          <div className="max-h-[560px] overflow-y-auto border-t border-slate-200 dark:border-slate-800">
            {files.length === 0 ? (
              <div className="px-4 py-10 text-center text-sm text-slate-400 dark:text-slate-500">
                {browsing ? t.downloadPage.loading : t.modelRepo.browseFiles}
              </div>
            ) : (
              files.map(renderFileRow)
            )}
          </div>
        </section>

        <div className="min-w-0 space-y-5">
          {renderSelectedTaskSummary()}

          <DownloadSettingsPanel
            open={dlSettingsOpen}
            onToggle={() => setDlSettingsOpen(!dlSettingsOpen)}
            copy={ui}
            labels={{
              resumePolicy: t.downloadPage.resumePolicy,
              resumeManual: t.downloadPage.resumeManual,
              resumeAuto: t.downloadPage.resumeAuto,
              maxConcurrent: t.downloadPage.maxConcurrent,
            }}
            settingsError={settingsError}
            resumePolicy={resumePolicy}
            resumePolicyLabel={resumePolicyLabel}
            bandwidthSummary={bandwidthSummary}
            concurrency={concurrency}
            bandwidthLimit={bandwidthLimit}
            bandwidthUnit={bandwidthUnit}
            lowPriorityThrottle={lowPriorityThrottle}
            onResumePolicyChange={policy => void handleResumePolicyChange(policy)}
            onConcurrencyChange={value => void handleConcurrencyChange(value)}
            onBandwidthLimitChange={value => void handleBandwidthLimitChange(value)}
            onBandwidthUnitChange={handleBandwidthUnitChange}
            onLowPriorityThrottleChange={enabled => void handleLowPriorityThrottleChange(enabled)}
            onResetDefaults={() => void resetStrategyDefaults()}
          />

          <SectionPanel
            title={t.downloadPage.queue}
            count={queueFileCount}
            collapsed={collapsed.queue}
            onToggle={() => toggleSection('queue')}
          >
            <div className="max-h-[440px] overflow-y-auto">
              {queueGroups.length === 0 ? (
                <div className="px-4 py-8 text-center text-sm text-slate-400 dark:text-slate-500">{t.downloadPage.noQueue}</div>
              ) : (
                queueGroups.map((group, index) => (
                  <div key={group.key} className="border-b border-slate-200 px-4 py-4 last:border-b-0 dark:border-slate-800">
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0 flex-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="rounded-full bg-slate-100 px-2 py-1 text-[11px] text-slate-600 dark:bg-slate-800 dark:text-slate-300">
                            {sourceName(group.entry.source)}
                          </span>
                          <span className="min-w-0 max-w-full truncate text-sm font-medium text-slate-900 dark:text-slate-100" title={group.entry.repoId}>{group.entry.repoId}</span>
                        </div>
                        <div className="mt-2 text-xs text-slate-500 dark:text-slate-400">
                          {group.files.length} {t.downloadPage.files} &middot; {formatSize(group.files.reduce((sum, file) => sum + file.size, 0))}
                        </div>
                      </div>
                      <div className="flex shrink-0 items-center gap-1">
                        <button
                          onClick={() => moveQueueEntry(group.entry.id, 'up')}
                          disabled={index === 0}
                          className="rounded-lg p-2 text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 disabled:cursor-not-allowed disabled:opacity-30 dark:hover:bg-slate-800 dark:hover:text-white"
                        >
                          <ChevronUp className="h-4 w-4" />
                        </button>
                        <button
                          onClick={() => moveQueueEntry(group.entry.id, 'down')}
                          disabled={index === queueGroups.length - 1}
                          className="rounded-lg p-2 text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 disabled:cursor-not-allowed disabled:opacity-30 dark:hover:bg-slate-800 dark:hover:text-white"
                        >
                          <ChevronDown className="h-4 w-4" />
                        </button>
                        <button
                          onClick={() => removeFromDownloadQueue(group.entry.id)}
                          className="rounded-lg p-2 text-slate-500 transition hover:bg-slate-100 hover:text-rose-500 dark:hover:bg-slate-800"
                        >
                          <Trash2 className="h-4 w-4" />
                        </button>
                      </div>
                    </div>
                    <div className="mt-3 space-y-1 text-xs text-slate-500 dark:text-slate-400">
                      {group.files.slice(0, 4).map(file => (
                        <div key={file.task_id || file.name} className="truncate" title={file.path || file.name}>{file.name}</div>
                      ))}
                      {group.files.length > 4 && (
                        <div>{formatMessage(ui.moreFiles, { count: group.files.length - 4 })}</div>
                      )}
                    </div>
                  </div>
                ))
              )}
            </div>
          </SectionPanel>
        </div>
      </div>

      <div className="grid min-w-0 gap-5 2xl:grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)]">
        <SectionPanel
          title={t.downloadPage.active}
          count={activeTasks.length}
          collapsed={collapsed.active}
          onToggle={() => toggleSection('active')}
        >
          <div className="max-h-[520px] overflow-y-auto">
            {activeTasks.length === 0 ? (
              <div className="px-4 py-8 text-center text-sm text-slate-400 dark:text-slate-500">{t.downloadPage.noActive}</div>
            ) : (
              activeTasks.map(renderTaskCard)
            )}
          </div>
        </SectionPanel>

        <SectionPanel
          title={t.downloadPage.paused}
          count={pausedTasks.length}
          collapsed={collapsed.paused}
          onToggle={() => toggleSection('paused')}
        >
          <div className="max-h-[520px] overflow-y-auto">
            {pausedTasks.length === 0 ? (
              <div className="px-4 py-8 text-center text-sm text-slate-400 dark:text-slate-500">{t.downloadPage.noPaused}</div>
            ) : (
              pausedTasks.map(renderTaskCard)
            )}
          </div>
        </SectionPanel>
      </div>

      <div className="grid min-w-0 gap-5 2xl:grid-cols-2">
        <SectionPanel
          title={t.downloadPage.failed}
          count={failedTasks.length}
          collapsed={collapsed.failed}
          onToggle={() => toggleSection('failed')}
          extra={failedTasks.length > 0 ? (
            <button
              onClick={() => clearFailedDownloadTasks()}
              className="rounded-lg px-2.5 py-1 text-[11px] text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 dark:hover:bg-slate-800 dark:hover:text-white"
            >
              {t.downloadPage.clearFailed}
            </button>
          ) : undefined}
        >
          <div className="max-h-[420px] overflow-y-auto">
            {failedTasks.length === 0 ? (
              <div className="px-4 py-8 text-center text-sm text-slate-400 dark:text-slate-500">{t.downloadPage.noFailed}</div>
            ) : (
              failedTasks.map(renderTaskCard)
            )}
          </div>
        </SectionPanel>

        <SectionPanel
          title={t.downloadPage.completed}
          count={completedTasks.length}
          collapsed={collapsed.completed}
          onToggle={() => toggleSection('completed')}
          extra={completedTasks.length > 0 ? (
            <button
              onClick={() => clearCompletedDownloadTasks()}
              className="rounded-lg px-2.5 py-1 text-[11px] text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 dark:hover:bg-slate-800 dark:hover:text-white"
            >
              {t.downloadPage.clearCompleted}
            </button>
          ) : undefined}
        >
          <div className="max-h-[420px] overflow-y-auto">
            {completedTasks.length === 0 ? (
              <div className="px-4 py-8 text-center text-sm text-slate-400 dark:text-slate-500">{t.downloadPage.noCompleted}</div>
            ) : (
              completedTasks.map(renderTaskCard)
            )}
          </div>
        </SectionPanel>
      </div>
    </div>
  )
}
