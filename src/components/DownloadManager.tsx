import { useState, useMemo, useEffect } from 'react'
import { Trash2, FolderOpen, X, Download, ChevronDown, ChevronUp, Pause, Play, RotateCcw, RefreshCw, Square, Settings2, AlertTriangle } from 'lucide-react'
import { useAppStore, type MsFileEntry } from '../store'
import type { DownloadProgress } from '../store/types'
import { useI18n } from '../i18n'
import { invoke } from '@tauri-apps/api/core'
import { pathJoin } from '../utils/path'
import { formatSize, formatSpeed, formatETA } from '../utils/format'

type DownloadSource = 'modelscope' | 'huggingface'
const DEFAULT_SAVE_DIR = 'models'

type Section = 'queue' | 'active' | 'paused' | 'failed' | 'completed'

function StatusBadge({ status }: { status: DownloadProgress['status'] }) {
  const colors: Record<string, string> = {
    active: 'bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-300',
    paused: 'bg-yellow-100 dark:bg-yellow-900/30 text-yellow-700 dark:text-yellow-300',
    pausing: 'bg-yellow-100 dark:bg-yellow-900/30 text-yellow-700 dark:text-yellow-300',
    queued: 'bg-gray-100 dark:bg-gray-700 text-gray-500 dark:text-gray-400',
    completed: 'bg-green-100 dark:bg-green-900/30 text-green-700 dark:text-green-300',
    cancelled: 'bg-gray-100 dark:bg-gray-700 text-gray-500 dark:text-gray-400',
    error: 'bg-red-100 dark:bg-red-900/30 text-red-700 dark:text-red-300',
  }
  const labels: Record<string, string> = {
    active: 'Active', paused: 'Paused', pausing: 'Pausing', queued: 'Queued',
    completed: 'Done', cancelled: 'Cancelled', error: 'Failed',
  }
  return (
    <span className={`text-[10px] px-1.5 py-0.5 rounded font-medium shrink-0 ${colors[status] || ''}`}>
      {labels[status] || status}
    </span>
  )
}

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
  const resumeAllDownloads = useAppStore(s => s.resumeAllDownloads)
  const pauseAllDownloads = useAppStore(s => s.pauseAllDownloads)
  const cancelAllDownloads = useAppStore(s => s.cancelAllDownloads)
  const clearCompletedDownloadTasks = useAppStore(s => s.clearCompletedDownloadTasks)
  const clearFailedDownloadTasks = useAppStore(s => s.clearFailedDownloadTasks)
  const retryFailedDownload = useAppStore(s => s.retryFailedDownload)
  const redownloadFile = useAppStore(s => s.redownloadFile)
  const moveQueueEntry = useAppStore(s => s.moveQueueEntry)
  const [source, setSource] = useState<DownloadSource>('modelscope')
  const [repoId, setRepoId] = useState('')
  const [files, setFiles] = useState<MsFileEntry[]>([])
  const [status, setStatus] = useState('')
  const [browsing, setBrowsing] = useState(false)
  const [browsedRepoId, setBrowsedRepoId] = useState('')
  const [collapsed, setCollapsed] = useState<Record<Section, boolean>>({ queue: false, active: false, paused: false, failed: false, completed: false })
  const [saveDir, setSaveDir] = useState(() => {
    try { return localStorage.getItem('downloadSaveDir') || DEFAULT_SAVE_DIR }
    catch { return DEFAULT_SAVE_DIR }
  })
  const [initialLoading, setInitialLoading] = useState(true)
  const [dlSettingsOpen, setDlSettingsOpen] = useState(false)
  const [resumePolicy, setResumePolicy] = useState('manual')
  const [concurrency, setConcurrency] = useState(1)

  useEffect(() => {
    const meta = (window as any).__downloadSnapshotMeta
    if (meta) {
      setResumePolicy(meta.resume_policy || 'manual')
      setConcurrency(meta.max_concurrent || 1)
    } else {
      invoke<string>('get_download_resume_policy').then(p => setResumePolicy(p || 'manual')).catch(() => {})
      invoke<number>('get_download_concurrency').then(n => setConcurrency(n || 1)).catch(() => {})
    }
    setTimeout(() => setInitialLoading(false), 300)
  }, [])

  const handleResumePolicyChange = (policy: string) => {
    setResumePolicy(policy)
    invoke('set_download_resume_policy', { policy }).catch(() => {})
  }
  const handleConcurrencyChange = (n: number) => {
    setConcurrency(n)
    invoke('set_download_concurrency', { n }).catch(() => {})
  }

  const toggleSection = (s: Section) => setCollapsed(prev => ({ ...prev, [s]: !prev[s] }))

  const taskForFile = (f: MsFileEntry) => Object.values(downloadTasks).find(t =>
    t.source === source &&
    t.repoId === browsedRepoId &&
    t.remotePath === (f.path || f.name) &&
    t.saveDir === saveDir
  )

  const modelTypeLabel = (fileType: string) => {
    switch (fileType) { case 'mmproj': return t.modelRepo.typeMmproj; case 'imatrix': return t.modelRepo.typeImatrix; default: return t.modelRepo.typeModel }
  }
  const modelTypeColor = (fileType: string) => {
    switch (fileType) { case 'mmproj': return 'bg-purple-100 dark:bg-purple-900/20 text-purple-700 dark:text-purple-300'; case 'imatrix': return 'bg-slate-100 dark:bg-slate-700 text-slate-600 dark:text-slate-400'; default: return 'bg-blue-100 dark:bg-blue-900/20 text-blue-700 dark:text-blue-300' }
  }

  const handleBrowse = async () => {
    if (!repoId.trim()) { setStatus(t.modelRepo.inputRepoId); return }
    setBrowsing(true); setStatus(t.modelRepo.querying)
    try {
      const result = source === 'modelscope' ? await browseModelscope(repoId.trim()) : await browseHuggingface(repoId.trim())
      setFiles(result)
      const rid = repoId.trim()
      setBrowsedRepoId(rid)
      setStatus(result.length === 0 ? t.modelRepo.notFound : `${t.modelRepo.found} ${result.length} ${t.modelRepo.files}`)

      const allTasks = useAppStore.getState().downloadTasks
      const tasks: Record<string, DownloadProgress> = {}
      for (const k of Object.keys(allTasks)) {
        if (allTasks[k].status !== 'error' && allTasks[k].status !== 'cancelled') {
          tasks[k] = allTasks[k]
        }
      }
      const resolvedDir = await invoke<string>('resolve_path', { path: saveDir })
      await Promise.all(result.map(async (f) => {
        const localPath = pathJoin(resolvedDir, rid, f.name)
        try {
          const actualSize = await invoke<number | null>('check_local_file', { path: localPath })
          if (actualSize != null && actualSize >= f.size * 0.99) {
            const existing = Object.values(tasks).find(t => t.source === source && t.repoId === rid && t.remotePath === (f.path || f.name) && t.saveDir === saveDir)
            const id = existing?.id || crypto.randomUUID()
            tasks[id] = {
              id,
              fileName: f.name,
              remotePath: f.path || f.name,
              fileType: f.file_type,
              saveDir,
              repoId: rid,
              source,
              downloaded: actualSize,
              total: f.size,
              speed: 0,
              status: 'completed',
              path: localPath,
              version: existing?.version ?? 0,
            }
          }
        } catch { /* file not found */ }
      }))
      setDownloadTasks(tasks)
    } catch (e: any) {
      setStatus(`${t.modelRepo.queryFailed}${typeof e === 'string' ? e : t.modelRepo.networkError}`)
    } finally { setBrowsing(false) }
  }

  const saveDirPersist = (dir: string) => { setSaveDir(dir); try { localStorage.setItem('downloadSaveDir', dir) } catch {} }
  const handleBrowseSaveDir = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const dir = await open({ directory: true, title: t.modelRepo.saveDir })
      if (dir) saveDirPersist(dir as string)
    } catch (_) {}
  }

  const handleDownloadFile = (f: MsFileEntry) => addToDownloadQueue({ repoId: browsedRepoId, source, files: [f], saveDir })
  const handleDownloadAll = () => {
    const pending = files.filter(f => taskForFile(f)?.status !== 'completed')
    if (pending.length > 0) addToDownloadQueue({ repoId: browsedRepoId, source, files: pending, saveDir })
  }

  const handlePause = (f: MsFileEntry) => {
    const task = taskForFile(f)
    if (task) { const s = useAppStore.getState(); setDownloadTasks({ ...s.downloadTasks, [task.id]: { ...task, status: 'pausing' } }); pauseFileDownload(task.id, task.runId) }
  }

  const handleCancel = (f: MsFileEntry) => {
    const task = taskForFile(f)
    if (task) cancelAndCleanupDownload(task.id, f.name, pathJoin(task.saveDir, browsedRepoId, f.name), task.runId)
  }

  const handleResumePersisted = (f: DownloadProgress) => {
    addToDownloadQueue({
      repoId: f.repoId,
      source: f.source as DownloadSource,
      files: [{ name: f.fileName, path: f.remotePath, size: f.total, file_type: f.fileType || 'model', task_id: f.id, downloaded: f.downloaded }],
      saveDir: f.saveDir,
    })
  }
  const handleCancelPersisted = (f: DownloadProgress) => {
    const task = downloadTasks[f.id]
    if (task) setDownloadTasks({ ...downloadTasks, [f.id]: { ...task, status: 'cancelled' } })
    cancelAndCleanupDownload(f.id, f.fileName, pathJoin(f.saveDir, f.repoId, f.fileName), f.runId)
  }

  // ── Derived task lists ──
  const allTasks = useMemo(() => Object.values(downloadTasks), [downloadTasks])
  const activeTasks = useMemo(() => allTasks.filter(t => t.status === 'active'), [allTasks])
  const pausedTasks = useMemo(() => allTasks.filter(t => t.status === 'paused' || t.status === 'pausing'), [allTasks])
  const failedTasks = useMemo(() => allTasks.filter(t => t.status === 'error'), [allTasks])
  const completedTasks = useMemo(() => allTasks.filter(t => t.status === 'completed' || t.status === 'cancelled'), [allTasks])

  const hasActive = activeTasks.length > 0
  const hasPaused = pausedTasks.length > 0
  const hasAnyActionable = hasActive || hasPaused || downloadQueue.length > 0

  // ── Queue groups (grouped by repo) ──
  const queueGroups = useMemo(() => {
    const map = new Map<string, { entry: typeof downloadQueue[0]; files: MsFileEntry[] }>()
    for (const entry of downloadQueue) {
      const key = `${entry.source}:${entry.repoId}`
      if (!map.has(key)) map.set(key, { entry, files: [] })
      map.get(key)!.files.push(...entry.files)
    }
    return Array.from(map.entries()).map(([key, g]) => ({ key, ...g }))
  }, [downloadQueue])

  // ── Task card renderers ──
  const renderProgressBar = (task: DownloadProgress) => {
    const pct = task.total > 0 ? Math.min(100, (task.downloaded / task.total) * 100) : 0
    const color = task.status === 'active' ? 'bg-blue-500'
      : (task.status === 'paused' || task.status === 'pausing') ? 'bg-yellow-500'
      : task.status === 'error' ? 'bg-red-500'
      : 'bg-green-500'
    return (
      <div className="h-1.5 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
        <div className={`h-full rounded-full ${color} transition-all duration-300`} style={{ width: `${pct}%` }} />
      </div>
    )
  }

  const renderTaskCard = (task: DownloadProgress, _idx: number) => (
    <div key={task.id} className="px-4 py-2.5 space-y-1.5">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <StatusBadge status={task.status} />
          {task.remoteChanged && (
            <AlertTriangle className="w-3.5 h-3.5 text-amber-500 shrink-0" />
          )}
          <span className="text-xs truncate" title={task.fileName}>{task.fileName}</span>
        </div>
        <div className="flex items-center gap-1 shrink-0">
          {task.status === 'active' && (
            <>
              <button onClick={() => {
                const s = useAppStore.getState(); setDownloadTasks({ ...s.downloadTasks, [task.id]: { ...task, status: 'pausing' } })
                pauseFileDownload(task.id, task.runId)
              }} className="p-1 rounded hover:bg-yellow-100 dark:hover:bg-yellow-900/30 text-yellow-600" title={t.modelRepo.pause}>
                <Pause className="w-3 h-3" />
              </button>
              <button onClick={() => cancelFileDownload(task.id, task.runId)} className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-900/30 text-red-500" title={t.modelRepo.cancel}>
                <Square className="w-3 h-3" />
              </button>
            </>
          )}
          {task.status === 'paused' && (
            <>
              <button onClick={() => handleResumePersisted(task)} className="p-1 rounded hover:bg-green-100 dark:hover:bg-green-900/30 text-green-600" title={t.modelRepo.resume}>
                <Play className="w-3 h-3" />
              </button>
              <button onClick={() => handleCancelPersisted(task)} className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-900/30 text-red-500" title={t.modelRepo.cancel}>
                <X className="w-3 h-3" />
              </button>
            </>
          )}
          {(task.status === 'completed' || task.status === 'cancelled') && task.path && (
            <button onClick={() => cancelAndCleanupDownload(task.id, task.fileName, task.path!, task.runId)} className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-900/30 text-red-400" title={t.common.delete}>
              <Trash2 className="w-3 h-3" />
            </button>
          )}
          {task.status === 'error' && (
            <>
              <button onClick={() => retryFailedDownload(task.id)} className="px-2 py-0.5 rounded text-[10px] font-medium bg-blue-100 dark:bg-blue-900/20 text-blue-600 hover:bg-blue-200" title={t.downloadPage.retry}>
                <RotateCcw className="w-3 h-3 inline mr-0.5" />{t.downloadPage.retry}
              </button>
              <button onClick={() => redownloadFile(task.id)} className="px-2 py-0.5 rounded text-[10px] font-medium bg-orange-100 dark:bg-orange-900/20 text-orange-600 hover:bg-orange-200" title={t.downloadPage.redownload}>
                <RefreshCw className="w-3 h-3 inline mr-0.5" />{t.downloadPage.redownload}
              </button>
            </>
          )}
        </div>
      </div>
      {renderProgressBar(task)}
      <div className="flex justify-between text-[10px] text-gray-400">
        <span>{formatSize(task.downloaded)}/{formatSize(task.total)}</span>
        {task.status === 'active' ? (
          <span>{formatSpeed(task.speed || 0)} &middot; {formatETA(task.downloaded, task.total, task.speed || 0)}</span>
        ) : task.status === 'paused' || task.status === 'pausing' ? (
          <span className="text-yellow-500">{t.downloadPage.paused}</span>
        ) : task.status === 'error' && task.error ? (
          <span className="text-red-400 truncate max-w-[200px]" title={task.error}>{task.error}</span>
        ) : task.status === 'completed' ? (
          <span className="text-green-500">{t.modelRepo.done}</span>
        ) : task.status === 'cancelled' ? (
          <span className="text-gray-400">{t.modelRepo.cancelled}</span>
        ) : null}
      </div>
    </div>
  )

  // ── Section header ──
  const SectionHeader = ({ section, label, count, extra }: { section: Section; label: string; count: number; extra?: React.ReactNode }) => (
    <div className="flex items-center justify-between px-4 py-2 bg-gray-100 dark:bg-gray-800 rounded-t-lg cursor-pointer select-none" onClick={() => toggleSection(section)}>
      <div className="flex items-center gap-2">
        <span className="text-sm font-medium">{label}</span>
        <span className="text-xs text-gray-400">({count})</span>
      </div>
      <div className="flex items-center gap-2" onClick={e => e.stopPropagation()}>
        {extra}
        {collapsed[section] ? <ChevronDown className="w-4 h-4 text-gray-400" /> : <ChevronUp className="w-4 h-4 text-gray-400" />}
      </div>
    </div>
  )

  if (initialLoading) {
    return (
      <div className="flex-1 p-6 overflow-y-auto space-y-6">
        <h2 className="text-2xl font-bold flex items-center gap-2"><Download className="w-6 h-6" /> {t.downloadPage.title}</h2>
        <div className="space-y-4">
          {[1,2,3].map(i => (
            <div key={i} className="border dark:border-gray-700 rounded-lg animate-pulse">
              <div className="h-10 bg-gray-100 dark:bg-gray-800 rounded-t-lg" />
              <div className="px-4 py-6 space-y-2">
                <div className="h-3 bg-gray-200 dark:bg-gray-700 rounded w-3/4" />
                <div className="h-2 bg-gray-200 dark:bg-gray-700 rounded w-1/2" />
              </div>
            </div>
          ))}
        </div>
      </div>
    )
  }

  return (
    <div className="flex-1 p-6 overflow-y-auto space-y-6">
      <h2 className="text-2xl font-bold flex items-center gap-2"><Download className="w-6 h-6" /> {t.downloadPage.title}</h2>

      {/* ═══ 批量操作工具栏 ═══ */}
      {hasAnyActionable && (
        <div className="flex items-center gap-2">
          <span className="text-xs text-gray-400 mr-1">{t.downloadPage.batchActions}</span>
          <button onClick={() => pauseAllDownloads()} disabled={!hasActive}
            className="px-3 py-1 text-xs rounded bg-yellow-100 dark:bg-yellow-900/20 text-yellow-700 hover:bg-yellow-200 dark:hover:bg-yellow-900/40 disabled:opacity-40 disabled:cursor-not-allowed">
            <Pause className="w-3 h-3 inline mr-1" />{t.downloadPage.pauseAll}
          </button>
          <button onClick={() => resumeAllDownloads()} disabled={!hasPaused}
            className="px-3 py-1 text-xs rounded bg-green-100 dark:bg-green-900/20 text-green-700 hover:bg-green-200 dark:hover:bg-green-900/40 disabled:opacity-40 disabled:cursor-not-allowed">
            <Play className="w-3 h-3 inline mr-1" />{t.downloadPage.resumeAll}
          </button>
          <button onClick={() => cancelAllDownloads()} disabled={!hasActive && !hasPaused && downloadQueue.length === 0}
            className="px-3 py-1 text-xs rounded bg-red-100 dark:bg-red-900/20 text-red-700 hover:bg-red-200 dark:hover:bg-red-900/40 disabled:opacity-40 disabled:cursor-not-allowed">
            <Square className="w-3 h-3 inline mr-1" />{t.downloadPage.cancelAll}
          </button>
        </div>
      )}

      {/* ═══ 下载设置 ═══ */}
      <div className="border dark:border-gray-700 rounded-lg">
        <div className="flex items-center justify-between px-4 py-2 bg-gray-100 dark:bg-gray-800 rounded-t-lg cursor-pointer select-none" onClick={() => setDlSettingsOpen(!dlSettingsOpen)}>
          <div className="flex items-center gap-2">
            <Settings2 className="w-4 h-4 text-gray-500" />
            <span className="text-sm font-medium">{t.downloadPage.dlSettings}</span>
          </div>
          {dlSettingsOpen ? <ChevronUp className="w-4 h-4 text-gray-400" /> : <ChevronDown className="w-4 h-4 text-gray-400" />}
        </div>
        {dlSettingsOpen && (
          <div className="px-4 py-3 space-y-3">
            <div className="flex items-center gap-3">
              <label className="text-xs text-gray-500 w-28 shrink-0">{t.downloadPage.resumePolicy}</label>
              <select value={resumePolicy} onChange={e => handleResumePolicyChange(e.target.value)}
                className="flex-1 px-2 py-1.5 border dark:border-gray-700 rounded bg-white dark:bg-gray-900 text-sm">
                <option value="manual">{t.downloadPage.resumeManual}</option>
                <option value="auto_on_launch">{t.downloadPage.resumeAuto}</option>
              </select>
            </div>
            <div className="flex items-center gap-3">
              <label className="text-xs text-gray-500 w-28 shrink-0">{t.downloadPage.maxConcurrent}</label>
              <input type="number" min={1} max={8} value={concurrency} onChange={e => handleConcurrencyChange(Math.max(1, Math.min(8, parseInt(e.target.value) || 1)))}
                className="w-20 px-2 py-1.5 border dark:border-gray-700 rounded bg-white dark:bg-gray-900 text-sm text-center" />
              <span className="text-xs text-gray-400">1-8</span>
            </div>
          </div>
        )}
      </div>

      {/* ═══ 下载源浏览 ═══ */}
      <div className="space-y-3">
        <div className="flex gap-2">
          <button onClick={() => { setSource('modelscope'); setFiles([]); setStatus('') }}
            className={`px-3 py-1.5 rounded-lg text-sm font-medium ${source === 'modelscope' ? 'bg-blue-600 text-white' : 'bg-gray-100 dark:bg-gray-700'}`}>ModelScope</button>
          <button onClick={() => { setSource('huggingface'); setFiles([]); setStatus('') }}
            className={`px-3 py-1.5 rounded-lg text-sm font-medium ${source === 'huggingface' ? 'bg-blue-600 text-white' : 'bg-gray-100 dark:bg-gray-700'}`}>HuggingFace</button>
        </div>
        <div className="flex gap-2" data-guide="download-source">
          <input type="text" value={repoId} onChange={e => setRepoId(e.target.value)} onKeyDown={e => e.key === 'Enter' && handleBrowse()}
            placeholder={source === 'modelscope' ? t.modelRepo.repoIdPlaceholder : t.modelRepo.hfRepoIdPlaceholder}
            className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 text-sm" />
          <button onClick={handleBrowse} disabled={browsing}
            className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-400 text-white rounded-lg text-sm">{t.modelRepo.browseFiles}</button>
        </div>
        <div className="flex gap-2">
          <input type="text" value={saveDir} onChange={e => saveDirPersist(e.target.value)}
            placeholder={t.downloadPage.saveDirLabel}
            className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 text-sm" />
          <button onClick={handleBrowseSaveDir} className="px-3 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg">
            <FolderOpen className="w-5 h-5" />
          </button>
        </div>
        {status && <p className="text-sm text-gray-500">{status}</p>}
      </div>

      {/* ═══ 仓库文件 ═══ */}
      {files.length > 0 && (
        <div>
          <div className="flex items-center justify-between mb-2">
            <div className="text-xs text-gray-500">{status} &middot; {formatSize(files.reduce((s, f) => s + f.size, 0))}</div>
            <button onClick={handleDownloadAll} className="px-3 py-1.5 bg-green-600 hover:bg-green-700 text-white rounded-lg text-xs">
              {t.downloadPage.downloadAll} ({files.filter(f => taskForFile(f)?.status !== 'completed').length} {t.downloadPage.files})
            </button>
          </div>
          <div className="border dark:border-gray-700 rounded-lg max-h-80 overflow-y-auto divide-y dark:divide-gray-700">
            {files.map((f) => {
              const task = taskForFile(f)
              return (
                <div key={f.path} className="px-4 py-2.5 space-y-1.5">
                  <div className="flex items-center justify-between text-xs">
                    <span className="text-sm truncate flex-1 mr-2">{f.name}</span>
                    <span className={`text-xs px-1.5 py-0.5 rounded shrink-0 ml-2 ${modelTypeColor(f.file_type)}`}>{modelTypeLabel(f.file_type)}</span> &middot; {formatSize(f.size)}
                  </div>
                  {task && (task.status === 'active' || task.status === 'pausing' || task.status === 'queued') && (
                    <div className="h-1.5 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
                      <div className="h-full rounded-full bg-blue-500" style={{ width: `${task.total > 0 ? (task.downloaded / task.total) * 100 : 0}%` }} />
                    </div>
                  )}

                  <div className="flex items-center gap-2">
                    {task?.status === 'active' || task?.status === 'pausing' ? (
                      <>
                        <span className="text-xs text-blue-500">{formatSize(task.downloaded)}/{formatSize(task.total)} &middot; {formatSpeed(task.speed || 0)} &middot; {formatETA(task.downloaded, task.total, task.speed || 0)}</span>
                        {task.status === 'active'
                          ? <button onClick={() => handlePause(f)} className="text-xs text-yellow-500 hover:text-yellow-700">{t.modelRepo.pause}</button>
                          : <span className="text-xs text-yellow-500">{t.modelRepo.pause}</span>}
                        <button onClick={() => handleCancel(f)} className="text-xs text-red-500 hover:text-red-700 ml-auto">{t.modelRepo.cancel}</button>
                      </>
                    ) : task?.status === 'paused' ? (
                      <>
                        <span className="text-xs text-yellow-500">{formatSize(task.downloaded)}/{formatSize(task.total)}</span>
                        <button onClick={() => handleDownloadFile(f)} className="text-xs text-green-500 hover:text-green-700">{t.modelRepo.resume}</button>
                        <button onClick={() => handleCancel(f)} className="text-xs text-red-500 hover:text-red-700 ml-auto">{t.modelRepo.cancel}</button>
                      </>
                    ) : task?.status === 'completed' ? (
                      <span className="text-xs text-green-500">&#x2713; {t.modelRepo.done}</span>
                    ) : task?.status === 'queued' ? (
                      <span className="text-xs text-gray-400">{t.downloadPage.queued}</span>
                    ) : task?.status === 'error' ? (
                      <>
                        <span className="text-xs text-red-400 truncate max-w-[200px]" title={task.error}>{task.error || t.modelRepo.failed}</span>
                        <button onClick={() => handleDownloadFile(f)} className="text-xs text-blue-500 hover:text-blue-700 ml-auto shrink-0">{t.modelRepo.retry}</button>
                      </>
                    ) : (
                      <button onClick={() => handleDownloadFile(f)} className="text-xs text-blue-500 hover:text-blue-700">{t.modelRepo.downloadBtn}</button>
                    )}
                  </div>
                </div>
              )
            })}
          </div>
        </div>
      )}

      {/* ═══ 1. Queue 排队 ═══ */}
      <div className="border dark:border-gray-700 rounded-lg">
        <SectionHeader section="queue" label={t.downloadPage.queue} count={downloadQueue.reduce((s, e) => s + e.files.length, 0)} />
        {!collapsed.queue && (
          <div className="divide-y dark:divide-gray-700">
            {downloadQueue.length === 0 ? (
              <div className="px-4 py-6 text-center text-xs text-gray-400">{t.downloadPage.noQueue}</div>
            ) : (
              queueGroups.map((g, gi) => (
                <div key={g.key}>
                  <div className="px-4 py-1.5 bg-gray-50 dark:bg-gray-800/50 text-[11px] text-gray-500 flex items-center justify-between">
                    <span>{g.entry.source}:{g.entry.repoId} &middot; {g.files.length} {t.downloadPage.files}</span>
                    <span>{formatSize(g.files.reduce((s, f) => s + f.size, 0))}</span>
                  </div>
                  {g.files.map((f, fi) => (
                    <div key={f.task_id || f.name} className="px-4 py-1.5 flex items-center justify-between text-xs">
                      <span className="truncate flex-1 mr-2">{f.name} &middot; <span className="text-gray-400">{formatSize(f.size)}</span></span>
                      <div className="flex items-center gap-0.5 shrink-0">
                        <button onClick={() => moveQueueEntry(g.entry.id, 'up')} disabled={gi === 0 && fi === 0}
                          className="p-0.5 rounded hover:bg-gray-200 dark:hover:bg-gray-600 disabled:opacity-30" title="&#x25B2;">
                          <ChevronUp className="w-3 h-3" />
                        </button>
                        <button onClick={() => moveQueueEntry(g.entry.id, 'down')} disabled={gi === queueGroups.length - 1 && fi === g.files.length - 1}
                          className="p-0.5 rounded hover:bg-gray-200 dark:hover:bg-gray-600 disabled:opacity-30" title="&#x25BC;">
                          <ChevronDown className="w-3 h-3" />
                        </button>
                        <button onClick={() => removeFromDownloadQueue(g.entry.id)} className="px-1 py-0.5 text-red-400 hover:text-red-600 ml-1">&#x2715;</button>
                      </div>
                    </div>
                  ))}
                </div>
              ))
            )}
          </div>
        )}
      </div>

      {/* ═══ 2. Active 进行中 ═══ */}
      <div className="border dark:border-gray-700 rounded-lg">
        <SectionHeader section="active" label={t.downloadPage.active} count={activeTasks.length} />
        {!collapsed.active && (
          <div className="divide-y dark:divide-gray-700">
            {activeTasks.length === 0 ? (
              <div className="px-4 py-6 text-center text-xs text-gray-400">{t.downloadPage.noActive}</div>
            ) : (
              activeTasks.map((task, idx) => renderTaskCard(task, idx))
            )}
          </div>
        )}
      </div>

      {/* ═══ 3. Paused 已暂停 ═══ */}
      <div className="border dark:border-gray-700 rounded-lg">
        <SectionHeader section="paused" label={t.downloadPage.paused} count={pausedTasks.length} />
        {!collapsed.paused && (
          <div className="divide-y dark:divide-gray-700">
            {pausedTasks.length === 0 ? (
              <div className="px-4 py-6 text-center text-xs text-gray-400">{t.downloadPage.noPaused}</div>
            ) : (
              pausedTasks.map((task, idx) => renderTaskCard(task, idx))
            )}
          </div>
        )}
      </div>

      {/* ═══ 4. Failed 失败 ═══ */}
      <div className="border dark:border-gray-700 rounded-lg">
        <SectionHeader section="failed" label={t.downloadPage.failed} count={failedTasks.length}
          extra={failedTasks.length > 0 ? (
            <button onClick={() => clearFailedDownloadTasks()} className="text-[10px] text-gray-400 hover:text-gray-600 px-2 py-0.5 rounded hover:bg-gray-200 dark:hover:bg-gray-700">
              {t.downloadPage.clearFailed}
            </button>
          ) : undefined}
        />
        {!collapsed.failed && (
          <div className="divide-y dark:divide-gray-700">
            {failedTasks.length === 0 ? (
              <div className="px-4 py-6 text-center text-xs text-gray-400">{t.downloadPage.noFailed}</div>
            ) : (
              failedTasks.map((task, idx) => renderTaskCard(task, idx))
            )}
          </div>
        )}
      </div>

      {/* ═══ 5. Completed 已完成 ═══ */}
      <div className="border dark:border-gray-700 rounded-lg">
        <SectionHeader section="completed" label={t.downloadPage.completed} count={completedTasks.length}
          extra={completedTasks.length > 0 ? (
            <button onClick={() => clearCompletedDownloadTasks()} className="text-[10px] text-gray-400 hover:text-gray-600 px-2 py-0.5 rounded hover:bg-gray-200 dark:hover:bg-gray-700">
              {t.downloadPage.clearCompleted}
            </button>
          ) : undefined}
        />
        {!collapsed.completed && (
          <div className="divide-y dark:divide-gray-700">
            {completedTasks.length === 0 ? (
              <div className="px-4 py-6 text-center text-xs text-gray-400">{t.downloadPage.noCompleted}</div>
            ) : (
              completedTasks.map((task, idx) => renderTaskCard(task, idx))
            )}
          </div>
        )}
      </div>

    </div>
  )
}
