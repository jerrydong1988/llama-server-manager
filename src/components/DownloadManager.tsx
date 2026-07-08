import { useState, useMemo, useEffect, type ReactNode } from 'react'
import { Trash2, FolderOpen, X, Download, ChevronDown, ChevronUp, Pause, Play, RotateCcw, RefreshCw, Square, Settings2, AlertTriangle, Database } from 'lucide-react'
import { useAppStore, type MsFileEntry } from '../store'
import type { DownloadProgress } from '../store/types'
import { useI18n } from '../i18n'
import { invoke } from '@tauri-apps/api/core'
import { pathJoin } from '../utils/path'
import { formatSize, formatSpeed, formatETA } from '../utils/format'
import { PathText, surfaceClassName } from './ui'

type DownloadSource = 'modelscope' | 'huggingface'
type ResumePolicy = 'manual' | 'auto_on_launch'
type BandwidthUnit = 'MiB/s' | 'KiB/s'
const DEFAULT_SAVE_DIR = 'models'
const DEFAULT_BANDWIDTH_LIMIT = 0
const DEFAULT_BANDWIDTH_UNIT: BandwidthUnit = 'MiB/s'
type Section = 'queue' | 'active' | 'paused' | 'failed' | 'completed'

const clampConcurrency = (value: number) => Math.max(1, Math.min(8, Number.isFinite(value) ? value : 1))
const normalizeResumePolicy = (policy: string): ResumePolicy => policy === 'auto_on_launch' ? 'auto_on_launch' : 'manual'
const bandwidthUnitMultiplier = (unit: BandwidthUnit) => unit === 'MiB/s' ? 1024 * 1024 : 1024
const bandwidthToBytes = (value: number, unit: BandwidthUnit) => Math.max(0, Math.round((Number.isFinite(value) ? value : 0) * bandwidthUnitMultiplier(unit)))
const bytesToBandwidth = (bytes: number, unit: BandwidthUnit) => {
  if (!Number.isFinite(bytes) || bytes <= 0) return 0
  const value = bytes / bandwidthUnitMultiplier(unit)
  return Number.isInteger(value) ? value : Number(value.toFixed(unit === 'MiB/s' ? 2 : 0))
}
const preferredBandwidthDisplay = (bytes: number, fallbackUnit: BandwidthUnit) => {
  if (!Number.isFinite(bytes) || bytes <= 0) return { limit: 0, unit: fallbackUnit }
  if (bytes % (1024 * 1024) === 0) return { limit: bytes / (1024 * 1024), unit: 'MiB/s' as BandwidthUnit }
  return { limit: Math.round(bytes / 1024), unit: 'KiB/s' as BandwidthUnit }
}

function MetricTile({
  label,
  value,
  detail,
  tone,
}: {
  label: string
  value: string | number
  detail?: string
  tone: 'blue' | 'emerald' | 'amber' | 'slate'
}) {
  const tones = {
    blue: 'bg-blue-50 text-blue-700 dark:bg-blue-950/40 dark:text-blue-300',
    emerald: 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300',
    amber: 'bg-amber-50 text-amber-700 dark:bg-amber-950/40 dark:text-amber-300',
    slate: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300',
  }

  return (
    <div className={`${surfaceClassName} min-w-0 px-4 py-4`}>
      <div className={`inline-flex rounded-full px-2.5 py-1 text-[11px] font-medium ${tones[tone]}`}>{label}</div>
      <div className="mt-3 truncate text-2xl font-semibold text-slate-900 dark:text-slate-100" title={String(value)}>{value}</div>
      {detail && <div className="mt-1 truncate text-xs text-slate-500 dark:text-slate-400" title={detail}>{detail}</div>}
    </div>
  )
}

function StatusBadge({ status, label }: { status: DownloadProgress['status']; label: string }) {
  const colors: Record<string, string> = {
    active: 'bg-blue-100 text-blue-700 dark:bg-blue-950/50 dark:text-blue-300',
    paused: 'bg-amber-100 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300',
    pausing: 'bg-amber-100 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300',
    queued: 'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300',
    completed: 'bg-emerald-100 text-emerald-700 dark:bg-emerald-950/50 dark:text-emerald-300',
    cancelled: 'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300',
    error: 'bg-rose-100 text-rose-700 dark:bg-rose-950/50 dark:text-rose-300',
  }

  return (
    <span className={`inline-flex shrink-0 rounded-full px-2 py-1 text-[11px] font-medium ${colors[status] || colors.queued}`}>
      {label}
    </span>
  )
}

function SectionPanel({
  title,
  count,
  collapsed,
  onToggle,
  extra,
  children,
}: {
  title: string
  count: number
  collapsed: boolean
  onToggle: () => void
  extra?: ReactNode
  children: ReactNode
}) {
  return (
    <section className={`${surfaceClassName} min-w-0 overflow-hidden`}>
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-center justify-between gap-3 border-b border-slate-200 px-4 py-3 text-left dark:border-slate-800"
      >
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-sm font-semibold text-slate-900 dark:text-slate-100">{title}</span>
          <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[11px] text-slate-500 dark:bg-slate-800 dark:text-slate-400">
            {count}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-2" onClick={e => e.stopPropagation()}>
          {extra}
          {collapsed ? <ChevronDown className="h-4 w-4 text-slate-400" /> : <ChevronUp className="h-4 w-4 text-slate-400" />}
        </div>
      </button>
      {!collapsed && children}
    </section>
  )
}

export default function DownloadManager() {
  const { t, lang } = useI18n()
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
  const [modelImportStatus, setModelImportStatus] = useState('')
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
  const ui = useMemo(() => lang === 'zh-CN' ? {
    strategyTitle: '\u4E0B\u8F7D\u7B56\u7565',
    strategySub: '\u6062\u590D\u7B56\u7565\u3001\u5E76\u53D1\u6570\u3001\u5E26\u5BBD\u9650\u5236\u4E0E\u4F4E\u4F18\u5148\u7EA7\u6A21\u5F0F\u5747\u5DF2\u63A5\u5165\u540E\u7AEF',
    addTaskTitle: '\u6DFB\u52A0\u4E0B\u8F7D\u4EFB\u52A1',
    addTaskSub: '\u9009\u62E9\u6A21\u578B\u6765\u6E90\u5E76\u6D4F\u89C8\u4ED3\u5E93\u6587\u4EF6',
    sourceSection: '\u6A21\u578B\u6765\u6E90',
    sourceHelp: '\u8F93\u5165\u4ED3\u5E93 ID \u540E\u6D4F\u89C8\u53EF\u4E0B\u8F7D\u6587\u4EF6',
    storageSection: '\u4E0B\u8F7D\u5B58\u50A8\u8DEF\u5F84',
    storageHelp: '\u8F93\u5165\u7EDD\u5BF9\u8DEF\u5F84\uFF0C\u6216\u4F7F\u7528 models \u8FD9\u7C7B\u76F8\u5BF9\u8DEF\u5F84\u4FDD\u5B58\u5230\u5E94\u7528\u6570\u636E\u76EE\u5F55',
    storagePreview: '\u4FDD\u5B58\u843D\u70B9',
    chooseFolder: '\u9009\u62E9\u76EE\u5F55',
    batchTools: '\u6279\u91CF\u5DE5\u5177',
    totalSpeed: '\u5F53\u524D\u901F\u5EA6',
    controllable: '\u53EF\u64CD\u4F5C',
    strategy: '\u7B56\u7565',
    backendSaved: '\u5DF2\u63A5\u5165\u540E\u7AEF',
    localOnly: '\u672C\u5730',
    manualShort: '\u624B\u52A8',
    autoShort: '\u542F\u52A8\u81EA\u52A8',
    concurrencyShort: '\u5E76\u53D1',
    bandwidth: '\u5168\u5C40\u5E26\u5BBD\u9650\u5236',
    displayUnit: '\u663E\u793A\u5355\u4F4D',
    unlimited: '\u4E0D\u9650',
    limitHelp: '0 \u8868\u793A\u4E0D\u9650\u901F',
    lowPriorityThrottle: '\u4F4E\u4F18\u5148\u7EA7\u8282\u6D41',
    throttleHelp: '\u5F00\u542F\u540E\u540E\u7AEF\u4F1A\u5C06\u4E0B\u8F7D\u6536\u655B\u4E3A\u5355\u5E76\u53D1\uFF1B\u82E5\u672A\u8BBE\u7F6E\u9650\u901F\uFF0C\u5219\u81EA\u52A8\u4F7F\u7528 2 MiB/s \u540E\u53F0\u9650\u901F',
    resetDefaults: '\u91CD\u7F6E\u9ED8\u8BA4',
    noActionable: '\u6682\u65E0\u53EF\u64CD\u4F5C\u4E0B\u8F7D',
    localPreset: '\u672C\u5730\u9884\u8BBE',
    taskSummary: '\u4EFB\u52A1\u8BE6\u60C5',
    noSelectedTask: '\u6682\u65E0\u9009\u4E2D\u4EFB\u52A1',
    selectedTaskHint: '\u5355\u51FB\u4EFB\u52A1\u5361\u7247\u53EF\u5207\u6362\u8BE6\u60C5',
    progress: '\u8FDB\u5EA6',
    downloaded: '\u5DF2\u4E0B\u8F7D',
    speed: '\u901F\u5EA6',
    eta: '\u5269\u4F59',
    etaPending: '\u5F85\u4F30\u7B97',
    queueState: '\u961F\u5217\u72B6\u6001',
    queuePosition: '\u961F\u5217\u7B2C',
    runningNow: '\u6B63\u5728\u4E0B\u8F7D',
    notQueued: '\u4E0D\u5728\u7B49\u5F85\u961F\u5217',
    failedReason: '\u5931\u8D25\u539F\u56E0',
    retryAdvice: '\u91CD\u8BD5\u5EFA\u8BAE',
    retryAdviceNetwork: '\u4FDD\u7559\u5DF2\u4E0B\u8F7D\u5206\u7247\u5E76\u91CD\u8BD5\uFF1B\u5982\u679C\u7EE7\u7EED\u5931\u8D25\uFF0C\u68C0\u67E5\u7F51\u7EDC\u6216\u9650\u901F\u8BBE\u7F6E',
    retryAdviceRedownload: '\u670D\u52A1\u7AEF\u6216\u5206\u7247\u72B6\u6001\u4E0D\u9002\u5408\u7EED\u4F20\uFF0C\u5EFA\u8BAE\u4F7F\u7528\u91CD\u65B0\u4E0B\u8F7D',
    retryAdvicePermission: '\u68C0\u67E5\u4ED3\u5E93\u6743\u9650\u3001\u6587\u4EF6\u662F\u5426\u5B58\u5728\uFF0C\u7136\u540E\u91CD\u8BD5',
    retryAdviceGeneric: '\u5148\u91CD\u8BD5\u4EE5\u7EED\u4F20\uFF1B\u82E5\u9519\u8BEF\u91CD\u590D\u51FA\u73B0\uFF0C\u518D\u6267\u884C\u91CD\u65B0\u4E0B\u8F7D',
    openLocation: '\u6253\u5F00\u4F4D\u7F6E',
    addToModelRepo: '\u5165\u5E93\u626B\u63CF',
    importedToRepo: '\u5DF2\u626B\u63CF\u4FDD\u5B58\u76EE\u5F55\u5E76\u5237\u65B0\u6A21\u578B\u5E93',
    importFailed: '\u5165\u5E93\u626B\u63CF\u5931\u8D25',
    localPath: '\u672C\u5730\u6587\u4EF6',
  } : {
    strategyTitle: 'Download strategy',
    strategySub: 'Resume policy, concurrency, bandwidth limit, and low-priority mode are backend-backed',
    addTaskTitle: 'Add download task',
    addTaskSub: 'Choose a model source and browse repository files',
    sourceSection: 'Model source',
    sourceHelp: 'Enter a repository ID, then browse downloadable files',
    storageSection: 'Download storage path',
    storageHelp: 'Use an absolute folder, or a relative folder such as models under the app data directory',
    storagePreview: 'Save target',
    chooseFolder: 'Choose folder',
    batchTools: 'Bulk tools',
    totalSpeed: 'Current speed',
    controllable: 'Controllable',
    strategy: 'Strategy',
    backendSaved: 'Backend connected',
    localOnly: 'Local',
    manualShort: 'Manual',
    autoShort: 'Auto on launch',
    concurrencyShort: 'Concurrency',
    bandwidth: 'Global bandwidth limit',
    displayUnit: 'Display unit',
    unlimited: 'Unlimited',
    limitHelp: '0 means unlimited',
    lowPriorityThrottle: 'Low-priority throttling',
    throttleHelp: 'Backend switches downloads to single concurrency; without a custom limit it applies a 2 MiB/s background cap',
    resetDefaults: 'Reset defaults',
    noActionable: 'No actionable downloads',
    localPreset: 'Local preset',
    taskSummary: 'Task details',
    noSelectedTask: 'No task selected',
    selectedTaskHint: 'Click any task card to switch this panel',
    progress: 'Progress',
    downloaded: 'Downloaded',
    speed: 'Speed',
    eta: 'ETA',
    etaPending: 'Pending',
    queueState: 'Queue state',
    queuePosition: 'Queue position',
    runningNow: 'Downloading now',
    notQueued: 'Not waiting in queue',
    failedReason: 'Failure reason',
    retryAdvice: 'Retry advice',
    retryAdviceNetwork: 'Keep the partial file and retry; if it fails again, check network or bandwidth settings',
    retryAdviceRedownload: 'The server or partial file state is not suitable for resume; use redownload',
    retryAdvicePermission: 'Check repository access and whether the file still exists, then retry',
    retryAdviceGeneric: 'Retry first to resume from the partial file; redownload if the error repeats',
    openLocation: 'Open location',
    addToModelRepo: 'Scan into model repo',
    importedToRepo: 'Scanned the save directory and refreshed the model repository',
    importFailed: 'Model repo scan failed',
    localPath: 'Local file',
  }, [lang])

  useEffect(() => {
    const meta = (window as any).__downloadSnapshotMeta
    if (meta) {
      setResumePolicy(normalizeResumePolicy(meta.resume_policy || 'manual'))
      setConcurrency(clampConcurrency(meta.max_concurrent || 1))
      const bandwidth = preferredBandwidthDisplay(meta.bandwidth_limit_bytes_per_sec || 0, bandwidthUnit)
      setBandwidthLimit(bandwidth.limit)
      setBandwidthUnit(bandwidth.unit)
      setLowPriorityThrottle(!!meta.low_priority_throttle)
    } else {
      invoke<string>('get_download_resume_policy').then(p => setResumePolicy(normalizeResumePolicy(p || 'manual'))).catch(() => {})
      invoke<number>('get_download_concurrency').then(n => setConcurrency(clampConcurrency(n || 1))).catch(() => {})
      invoke<number>('get_download_bandwidth_limit').then(bytes => {
        const bandwidth = preferredBandwidthDisplay(bytes || 0, bandwidthUnit)
        setBandwidthLimit(bandwidth.limit)
        setBandwidthUnit(bandwidth.unit)
      }).catch(() => {})
      invoke<boolean>('get_download_low_priority_throttle').then(enabled => setLowPriorityThrottle(!!enabled)).catch(() => {})
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

  const handleResumePolicyChange = (policy: ResumePolicy) => {
    setResumePolicy(policy)
    invoke('set_download_resume_policy', { policy }).catch(() => {})
  }

  const handleConcurrencyChange = (n: number) => {
    const next = clampConcurrency(n)
    setConcurrency(next)
    invoke('set_download_concurrency', { n: next }).catch(() => {})
  }

  const handleBandwidthLimitChange = (value: number) => {
    const next = Math.max(0, Number.isFinite(value) ? value : 0)
    setBandwidthLimit(next)
    invoke('set_download_bandwidth_limit', { bytesPerSec: bandwidthToBytes(next, bandwidthUnit) }).catch(() => {})
  }

  const handleBandwidthUnitChange = (unit: BandwidthUnit) => {
    const bytes = bandwidthToBytes(bandwidthLimit, bandwidthUnit)
    setBandwidthUnit(unit)
    setBandwidthLimit(bytesToBandwidth(bytes, unit))
  }

  const handleLowPriorityThrottleChange = (enabled: boolean) => {
    setLowPriorityThrottle(enabled)
    invoke('set_download_low_priority_throttle', { enabled }).catch(() => {})
  }

  const resetStrategyDefaults = () => {
    handleResumePolicyChange('manual')
    handleConcurrencyChange(1)
    handleBandwidthLimitChange(DEFAULT_BANDWIDTH_LIMIT)
    setBandwidthUnit(DEFAULT_BANDWIDTH_UNIT)
    handleLowPriorityThrottleChange(false)
  }

  const toggleSection = (section: Section) => setCollapsed(prev => ({ ...prev, [section]: !prev[section] }))

  const sourceName = (value: string) => value === 'modelscope' ? 'ModelScope' : 'HuggingFace'

  const taskStatusLabel = (task: DownloadProgress) => ({
    active: t.downloadPage.active,
    paused: t.downloadPage.paused,
    pausing: lang === 'zh-CN' ? '\u6682\u505C\u4E2D' : 'Pausing',
    queued: t.downloadPage.queued,
    completed: t.modelRepo.done,
    cancelled: t.modelRepo.cancelled,
    error: t.downloadPage.failed,
  }[task.status] || task.status)

  const taskForFile = (file: MsFileEntry) => Object.values(downloadTasks).find(task =>
    task.source === source &&
    task.repoId === browsedRepoId &&
    task.remotePath === (file.path || file.name) &&
    task.saveDir === saveDir,
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
      const tasks: Record<string, DownloadProgress> = {}
      for (const key of Object.keys(allTasks)) {
        if (allTasks[key].status !== 'error' && allTasks[key].status !== 'cancelled') {
          tasks[key] = allTasks[key]
        }
      }

      const resolvedDir = await invoke<string>('resolve_path', { path: saveDir })
      await Promise.all(result.map(async file => {
        const localPath = pathJoin(resolvedDir, trimmedRepoId, file.name)
        try {
          const actualSize = await invoke<number | null>('check_local_file', { path: localPath })
          if (actualSize != null && actualSize >= file.size * 0.99) {
            const existing = Object.values(tasks).find(task =>
              task.source === source &&
              task.repoId === trimmedRepoId &&
              task.remotePath === (file.path || file.name) &&
              task.saveDir === saveDir,
            )
            const id = existing?.id || crypto.randomUUID()
            tasks[id] = {
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
            }
          }
        } catch {
          // file not found
        }
      }))

      setDownloadTasks(tasks)
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
      const { open } = await import('@tauri-apps/plugin-dialog')
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
    cancelAndCleanupDownload(task.id, file.name, pathJoin(task.saveDir, browsedRepoId, file.name), task.runId, task.version)
  }

  const handleResumePersisted = (task: DownloadProgress) => {
    resumeDownloadTask(task.id)
  }

  const handleCancelPersisted = (task: DownloadProgress) => {
    const current = downloadTasks[task.id]
    if (current) setDownloadTasks({ ...downloadTasks, [task.id]: { ...current, status: 'cancelled' } })
    cancelAndCleanupDownload(task.id, task.fileName, pathJoin(task.saveDir, task.repoId, task.fileName), task.runId, task.version)
  }

  const taskLocalPath = (task: DownloadProgress) => task.path || pathJoin(task.saveDir, task.repoId, task.fileName)

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
    } catch (error: any) {
      setModelImportStatus(typeof error === 'string' ? error : String(error || ui.importFailed))
    }
  }

  const handleImportCompletedTask = async (task: DownloadProgress) => {
    const error = await scanModels([task.saveDir])
    setModelImportStatus(error ? `${ui.importFailed}: ${error}` : ui.importedToRepo)
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
    const pct = task.total > 0 ? Math.min(100, (task.downloaded / task.total) * 100) : 0
    const isSelected = selectedTask?.id === task.id
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
                onClick={() => cancelAndCleanupDownload(task.id, task.fileName, task.path!, task.runId, task.version)}
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
    const pct = task && task.total > 0 ? (task.downloaded / task.total) * 100 : 0

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
                    {lang === 'zh-CN' ? '\u6682\u505C\u4E2D' : 'Pausing'}
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

    const pct = selectedTask.total > 0 ? Math.min(100, (selectedTask.downloaded / selectedTask.total) * 100) : 0
    const isCompleted = selectedTask.status === 'completed'
    const isFailed = selectedTask.status === 'error'

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
        <MetricTile label={t.downloadPage.queue} value={queueFileCount} detail={downloadQueue.length > 0 ? `${downloadQueue.length} ${lang === 'zh-CN' ? '\u6279\u6B21' : 'batches'}` : t.downloadPage.noQueue} tone="slate" />
        <MetricTile label={t.downloadPage.active} value={activeTasks.length} detail={hasActive ? formatSpeed(activeTasks.reduce((sum, task) => sum + task.speed, 0)) : t.downloadPage.noActive} tone="blue" />
        <MetricTile label={t.downloadPage.paused} value={pausedTasks.length} detail={hasPaused ? `${pausedTasks.filter(task => task.status === 'paused').length} ${lang === 'zh-CN' ? '\u53EF\u6062\u590D' : 'resumable'}` : t.downloadPage.noPaused} tone="amber" />
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
                    className="inline-flex shrink-0 items-center gap-1 rounded-lg bg-slate-900 px-4 py-2.5 text-sm font-medium text-white transition hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-slate-100 dark:text-slate-900 dark:hover:bg-white"
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

          <section className={`${surfaceClassName} min-w-0 overflow-hidden`}>
            <button
              type="button"
              onClick={() => setDlSettingsOpen(!dlSettingsOpen)}
              className="flex w-full items-start justify-between gap-3 border-b border-slate-200 px-4 py-4 text-left dark:border-slate-800"
            >
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <Settings2 className="h-4 w-4 text-slate-500" />
                  <span className="text-sm font-semibold text-slate-900 dark:text-slate-100">{ui.strategyTitle}</span>
                </div>
                <div className="mt-1 text-xs leading-5 text-slate-500 dark:text-slate-400">{ui.strategySub}</div>
              </div>
              {dlSettingsOpen ? <ChevronUp className="mt-0.5 h-4 w-4 shrink-0 text-slate-400" /> : <ChevronDown className="mt-0.5 h-4 w-4 shrink-0 text-slate-400" />}
            </button>

            {dlSettingsOpen && (
              <div className="space-y-5 px-4 py-4">
                <div className="grid grid-cols-2 gap-2">
                  <div className="rounded-lg border border-slate-200 px-3 py-3 dark:border-slate-800">
                    <div className="text-[11px] font-medium text-slate-500 dark:text-slate-400">{ui.strategy}</div>
                    <div className="mt-1 text-sm font-semibold text-slate-900 dark:text-slate-100">{resumePolicyLabel}</div>
                  </div>
                  <div className="rounded-lg border border-slate-200 px-3 py-3 dark:border-slate-800">
                    <div className="text-[11px] font-medium text-slate-500 dark:text-slate-400">{ui.bandwidth}</div>
                    <div className="mt-1 text-sm font-semibold text-slate-900 dark:text-slate-100">{bandwidthSummary}</div>
                  </div>
                </div>

                <div className="space-y-3">
                  <div className="flex items-center justify-between gap-3">
                    <label className="text-xs font-medium text-slate-500 dark:text-slate-400">{t.downloadPage.resumePolicy}</label>
                    <span className="rounded-full bg-emerald-50 px-2 py-0.5 text-[11px] font-medium text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300">
                      {ui.backendSaved}
                    </span>
                  </div>
                  <div className="grid grid-cols-2 gap-2 rounded-lg bg-slate-100 p-1 dark:bg-slate-800">
                    <button
                      type="button"
                      onClick={() => handleResumePolicyChange('manual')}
                      className={`rounded-md px-3 py-2 text-xs font-medium transition ${resumePolicy === 'manual' ? 'bg-white text-slate-900 shadow-sm dark:bg-slate-950 dark:text-white' : 'text-slate-500 hover:text-slate-900 dark:text-slate-400 dark:hover:text-white'}`}
                    >
                      {t.downloadPage.resumeManual}
                    </button>
                    <button
                      type="button"
                      onClick={() => handleResumePolicyChange('auto_on_launch')}
                      className={`rounded-md px-3 py-2 text-xs font-medium transition ${resumePolicy === 'auto_on_launch' ? 'bg-white text-slate-900 shadow-sm dark:bg-slate-950 dark:text-white' : 'text-slate-500 hover:text-slate-900 dark:text-slate-400 dark:hover:text-white'}`}
                    >
                      {t.downloadPage.resumeAuto}
                    </button>
                  </div>
                </div>

                <div className="space-y-3">
                  <div className="flex items-center justify-between gap-3">
                    <label className="text-xs font-medium text-slate-500 dark:text-slate-400">{t.downloadPage.maxConcurrent}</label>
                    <span className="rounded-full bg-emerald-50 px-2 py-0.5 text-[11px] font-medium text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300">
                      {ui.backendSaved}
                    </span>
                  </div>
                  <div className="flex items-center gap-3">
                    <input
                      type="range"
                      min={1}
                      max={8}
                      value={concurrency}
                      onChange={e => handleConcurrencyChange(parseInt(e.target.value, 10))}
                      className="min-w-0 flex-1 accent-blue-600"
                    />
                    <input
                      type="number"
                      min={1}
                      max={8}
                      value={concurrency}
                      onChange={e => handleConcurrencyChange(parseInt(e.target.value, 10))}
                      className="w-24 rounded-lg border border-slate-200 bg-white px-3 py-2.5 text-center text-sm text-slate-900 outline-none transition focus:border-blue-500 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-100"
                    />
                  </div>
                  <div className="text-[11px] text-slate-400">1-8</div>
                </div>

                <div className="space-y-3 border-t border-slate-200 pt-4 dark:border-slate-800">
                  <div className="flex items-center justify-between gap-3">
                    <label className="text-xs font-medium text-slate-500 dark:text-slate-400">{ui.bandwidth}</label>
                    <span className="rounded-full bg-emerald-50 px-2 py-0.5 text-[11px] font-medium text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300">
                      {ui.backendSaved}
                    </span>
                  </div>
                  <div className="grid grid-cols-[minmax(0,1fr)_96px] gap-2">
                    <input
                      type="number"
                      min={0}
                      step={bandwidthUnit === 'MiB/s' ? 1 : 64}
                      value={bandwidthLimit}
                      onChange={e => handleBandwidthLimitChange(Number(e.target.value))}
                      className="min-w-0 rounded-lg border border-slate-200 bg-white px-3 py-2.5 text-sm text-slate-900 outline-none transition focus:border-blue-500 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-100"
                    />
                    <select
                      value={bandwidthUnit}
                      onChange={e => handleBandwidthUnitChange(e.target.value as BandwidthUnit)}
                      aria-label={ui.displayUnit}
                      className="rounded-lg border border-slate-200 bg-white px-2 py-2.5 text-sm text-slate-900 outline-none transition focus:border-blue-500 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-100"
                    >
                      <option value="MiB/s">MiB/s</option>
                      <option value="KiB/s">KiB/s</option>
                    </select>
                  </div>
                  <div className="text-[11px] leading-5 text-slate-500 dark:text-slate-400">{ui.limitHelp}</div>
                </div>

                <div className="space-y-3 border-t border-slate-200 pt-4 dark:border-slate-800">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <div className="text-xs font-medium text-slate-600 dark:text-slate-300">{ui.lowPriorityThrottle}</div>
                        <span className="rounded-full bg-emerald-50 px-2 py-0.5 text-[11px] font-medium text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300">
                          {ui.backendSaved}
                        </span>
                      </div>
                      <div className="mt-1 text-[11px] leading-5 text-slate-400">{ui.throttleHelp}</div>
                    </div>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={lowPriorityThrottle}
                      onClick={() => handleLowPriorityThrottleChange(!lowPriorityThrottle)}
                      className={`relative mt-0.5 h-6 w-11 shrink-0 rounded-full transition ${lowPriorityThrottle ? 'bg-blue-600' : 'bg-slate-300 dark:bg-slate-700'}`}
                    >
                      <span className={`absolute top-1 h-4 w-4 rounded-full bg-white shadow-sm transition ${lowPriorityThrottle ? 'left-6' : 'left-1'}`} />
                    </button>
                  </div>
                </div>

                <button
                  type="button"
                  onClick={resetStrategyDefaults}
                  className="inline-flex w-full items-center justify-center gap-2 rounded-lg border border-slate-200 bg-white px-3 py-2.5 text-xs font-medium text-slate-600 transition hover:bg-slate-50 hover:text-slate-900 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-300 dark:hover:bg-slate-900 dark:hover:text-white"
                >
                  <RotateCcw className="h-3.5 w-3.5" />
                  <span>{ui.resetDefaults}</span>
                </button>
              </div>
            )}
          </section>

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
                        <div>{lang === 'zh-CN' ? `\u8FD8\u6709 ${group.files.length - 4} \u4E2A\u6587\u4EF6` : `${group.files.length - 4} more files`}</div>
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
