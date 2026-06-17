import { useState, useMemo } from 'react'
import { Trash2, FolderOpen, X, Download } from 'lucide-react'
import { useAppStore, type MsFileEntry } from '../store'
import type { DownloadProgress } from '../store/types'
import { useI18n } from '../i18n'
import { invoke } from '@tauri-apps/api/core'
import { pathJoin } from '../utils/path'

type DownloadSource = 'modelscope' | 'huggingface'
const DEFAULT_SAVE_DIR = 'models'

export default function DownloadManager() {
  const { t } = useI18n()
  const { downloadTasks, downloadQueue, setDownloadTasks, cancelFileDownload, cancelAndCleanupDownload, removeFromDownloadQueue, addToDownloadQueue, browseModelscope, browseHuggingface, pauseFileDownload } = useAppStore()
  const [source, setSource] = useState<DownloadSource>('modelscope')
  const [repoId, setRepoId] = useState('')
  const [files, setFiles] = useState<MsFileEntry[]>([])
  const [status, setStatus] = useState('')
  const [browsing, setBrowsing] = useState(false)
  const [saveDir, setSaveDir] = useState(DEFAULT_SAVE_DIR)

  const fmtSize = (n: number) => {
    if (n < 1024) return n + ' B'
    if (n < 1024 * 1024) return (n / 1024).toFixed(2) + ' KB'
    if (n < 1024 * 1024 * 1024) return (n / (1024 * 1024)).toFixed(2) + ' MB'
    return (n / (1024 * 1024 * 1024)).toFixed(2) + ' GB'
  }

  const fmtSpeed = (n: number) => {
    if (n < 1024) return n.toFixed(0) + ' B/s'
    if (n < 1024 * 1024) return (n / 1024).toFixed(1) + ' KB/s'
    return (n / 1024 / 1024).toFixed(1) + ' MB/s'
  }

  const fmtETA = (downloaded: number, total: number, speed: number) => {
    if (speed <= 0 || total <= 0) return ''
    const secs = Math.ceil((total - downloaded) / speed)
    if (secs < 60) return `${secs}s`
    if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`
    return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`
  }

  const modelTypeLabel = (fileType: string) => {
    switch (fileType) { case 'mmproj': return t.modelRepo.typeMmproj; case 'imatrix': return t.modelRepo.typeImatrix; default: return t.modelRepo.typeModel }
  }

  const handleBrowse = async () => {
    if (!repoId.trim()) { setStatus(t.modelRepo.inputRepoId); return }
    setBrowsing(true); setStatus(t.modelRepo.querying)
    try {
      const result = source === 'modelscope' ? await browseModelscope(repoId.trim()) : await browseHuggingface(repoId.trim())
      setFiles(result)
      setStatus(result.length === 0 ? t.modelRepo.notFound : `${t.modelRepo.found} ${result.length} ${t.modelRepo.files}`)

      // Detect already-downloaded files on disk
      const tasks = { ...useAppStore.getState().downloadTasks }
      for (const f of result) {
        const resolvedDir = await invoke<string>('resolve_path', { path: saveDir })
        const localPath = pathJoin(resolvedDir, repoId, f.name)
        try {
          const actualSize = await invoke<number | null>('check_local_file', { path: localPath })
          if (actualSize != null && actualSize >= f.size * 0.99) {
            tasks[f.name] = { fileName: f.name, repoId, source, downloaded: actualSize, total: f.size, speed: 0, status: 'completed', path: localPath }
          }
        } catch { /* file not found */ }
      }
      setDownloadTasks(tasks)
    } catch (e: any) {
      setStatus(`${t.modelRepo.queryFailed}${typeof e === 'string' ? e : t.modelRepo.networkError}`)
    } finally { setBrowsing(false) }
  }

  const handleBrowseSaveDir = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const dir = await open({ directory: true, title: t.modelRepo.saveDir })
      if (dir) setSaveDir(dir as string)
    } catch (_) {}
  }

  const handleDownloadFile = (f: MsFileEntry) => addToDownloadQueue({ repoId, source, files: [f], saveDir })
  const handleDownloadAll = () => { if (files.length > 0) addToDownloadQueue({ repoId, source, files, saveDir }) }

  const handlePause = (f: MsFileEntry) => {
    const task = downloadTasks[f.name]
    if (task) { setDownloadTasks({ ...downloadTasks, [f.name]: { ...task, status: 'paused' } }); pauseFileDownload(f.name) }
  }

  const handleCancel = (f: MsFileEntry) => {
    const task = downloadTasks[f.name]
    if (task) setDownloadTasks({ ...downloadTasks, [f.name]: { ...task, status: 'cancelled' } })
    cancelAndCleanupDownload(f.name, pathJoin(saveDir, repoId, f.name))
  }

  const clearCompleted = () => {
    const tasks = { ...downloadTasks }
    for (const k of Object.keys(tasks)) {
      if (tasks[k].status === 'completed' || tasks[k].status === 'cancelled') delete tasks[k]
    }
    setDownloadTasks(tasks)
  }

  // ── 进行中/已完成分组 ──
  const tasks = Object.values(downloadTasks)

  const activeGroups = useMemo(() => {
    const map = new Map<string, DownloadProgress[]>()
    for (const t of tasks.filter(t => t.status === 'active')) {
      const key = `${t.source || ''}:${t.repoId || 'unknown'}`
      if (!map.has(key)) map.set(key, [])
      map.get(key)!.push(t)
    }
    return Array.from(map.entries()).map(([key, files]) => ({ key, files }))
  }, [tasks])

  const completedGroups = useMemo(() => {
    const map = new Map<string, DownloadProgress[]>()
    for (const t of tasks.filter(t => t.status === 'completed' || t.status === 'cancelled' || t.status === 'error')) {
      const key = `${t.source || ''}:${t.repoId || 'unknown'}`
      if (!map.has(key)) map.set(key, [])
      map.get(key)!.push(t)
    }
    return Array.from(map.entries()).map(([key, files]) => ({ key, files }))
  }, [tasks])

  return (
    <div className="flex-1 p-6 overflow-y-auto space-y-6">
      <h2 className="text-2xl font-bold flex items-center gap-2"><Download className="w-6 h-6" /> Downloads</h2>

      {/* ═══ 下载源 ═══ */}
      <div className="space-y-3">
        <div className="flex gap-2">
          <button onClick={() => { setSource('modelscope'); setFiles([]); setStatus('') }}
            className={`px-3 py-1.5 rounded-lg text-sm font-medium ${source === 'modelscope' ? 'bg-blue-600 text-white' : 'bg-gray-100 dark:bg-gray-700'}`}>ModelScope</button>
          <button onClick={() => { setSource('huggingface'); setFiles([]); setStatus('') }}
            className={`px-3 py-1.5 rounded-lg text-sm font-medium ${source === 'huggingface' ? 'bg-blue-600 text-white' : 'bg-gray-100 dark:bg-gray-700'}`}>HuggingFace</button>
        </div>
        <div className="flex gap-2">
          <input type="text" value={repoId} onChange={e => setRepoId(e.target.value)} onKeyDown={e => e.key === 'Enter' && handleBrowse()}
            placeholder={source === 'modelscope' ? t.modelRepo.repoIdPlaceholder : t.modelRepo.hfRepoIdPlaceholder}
            className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 text-sm" />
          <button onClick={handleBrowse} disabled={browsing}
            className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-400 text-white rounded-lg text-sm">{t.modelRepo.browseFiles}</button>
        </div>
        <div className="flex gap-2">
          <input type="text" value={saveDir} onChange={e => setSaveDir(e.target.value)}
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
            <div className="text-xs text-gray-500">{status} · {fmtSize(files.reduce((s,f)=>s+f.size,0))}</div>
            <button onClick={handleDownloadAll} className="px-3 py-1.5 bg-green-600 hover:bg-green-700 text-white rounded-lg text-xs">
              ⬇ {t.downloadPage.downloadAll} ({files.length} {t.downloadPage.files})
            </button>
          </div>
          <div className="border dark:border-gray-700 rounded-lg max-h-80 overflow-y-auto divide-y dark:divide-gray-700">
            {files.map((f) => {
              const task = downloadTasks[f.name]
              return (
                <div key={f.path} className="px-4 py-2.5 space-y-1.5">
                  <div className="flex items-center justify-between text-xs">
                    <span className="text-sm truncate flex-1 mr-2">{f.name}</span>
                    <span className="text-gray-400 shrink-0 ml-2">{modelTypeLabel(f.file_type)} · {fmtSize(f.size)}</span>
                  </div>
                  {task && (task.status === 'active' || task.status === 'queued') && (
                    <div className="h-1.5 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
                      <div className="h-full rounded-full bg-blue-500" style={{width:`${task.total>0?(task.downloaded/task.total)*100:0}%`}} />
                    </div>
                  )}
                  <div className="flex items-center gap-2">
                    {task?.status === 'active' ? (
                      <>
                        <span className="text-xs text-blue-500">{fmtSize(task.downloaded)}/{fmtSize(task.total)} · {fmtSpeed(task.speed||0)} · {fmtETA(task.downloaded,task.total,task.speed||0)}</span>
                        <button onClick={() => handlePause(f)} className="text-xs text-yellow-500 hover:text-yellow-700">{t.modelRepo.pause}</button>
                        <button onClick={() => handleCancel(f)} className="text-xs text-red-500 hover:text-red-700 ml-auto">{t.modelRepo.cancel}</button>
                      </>
                    ) : task?.status === 'paused' ? (
                      <>
                        <span className="text-xs text-yellow-500">{fmtSize(task.downloaded)}/{fmtSize(task.total)}</span>
                        <button onClick={() => handleDownloadFile(f)} className="text-xs text-green-500 hover:text-green-700">{t.modelRepo.resume}</button>
                        <button onClick={() => handleCancel(f)} className="text-xs text-red-500 hover:text-red-700 ml-auto">{t.modelRepo.cancel}</button>
                      </>
                    ) : task?.status === 'completed' ? (
                      <span className="text-xs text-green-500">✓ {t.modelRepo.done}</span>
                    ) : task?.status === 'queued' ? (
                      <span className="text-xs text-gray-400">{t.downloadPage.queued}</span>
                    ) : task?.status === 'error' ? (
                      <span className="text-xs text-red-500">{task.error || t.modelRepo.failed}</span>
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

      {/* ⏳ Queue */}
      {downloadQueue.length > 0 && (
        <div>
          <div className="text-xs text-gray-500 mb-2">⏳ {t.downloadPage.queue} ({downloadQueue.length})</div>
          <div className="space-y-1">
            {downloadQueue.map(entry => (
              <div key={entry.id} className="flex items-center justify-between px-3 py-1.5 bg-gray-50 dark:bg-gray-800 rounded text-xs">
                <span className="truncate flex-1">{entry.source}:{entry.repoId} · {entry.files.length} file{entry.files.length>1?'s':''}</span>
                <span className="text-gray-400 mx-2">{fmtSize(entry.files.reduce((s,f)=>s+f.size,0))}</span>
                <button onClick={() => removeFromDownloadQueue(entry.id)} className="text-red-400 hover:text-red-600 ml-1">✕</button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* ═══ 进行中 ═══ */}
      {activeGroups.length > 0 && (
        <div>
          <div className="text-xs text-gray-500 mb-2">{t.downloadPage.active} ({activeGroups.reduce((s,g)=>s+g.files.length,0)})</div>
          <div className="space-y-3">
            {activeGroups.map(g => (
              <div key={g.key} className="border dark:border-gray-700 rounded-lg overflow-hidden">
                <div className="px-4 py-2 bg-gray-100 dark:bg-gray-800 text-sm font-medium">{g.key} · {g.files.length} files · {fmtSize(g.files.reduce((s,f)=>s+f.total,0))}</div>
                <div className="divide-y dark:divide-gray-700">
                  {g.files.map(f => (
                    <div key={f.fileName} className="px-4 py-2.5 space-y-1">
                      <div className="flex items-center justify-between text-xs">
                        <span className="truncate flex-1">{f.fileName}</span>
                        <button onClick={() => cancelFileDownload(f.fileName)} className="text-red-400 hover:text-red-600 ml-2"><X className="w-3 h-3" /></button>
                      </div>
                      <div className="h-1.5 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
                        <div className="h-full rounded-full bg-blue-500" style={{width:`${f.total>0?Math.min(100,(f.downloaded/f.total)*100):0}%`}} />
                      </div>
                      <div className="flex justify-between text-xs text-gray-400">
                        <span>{fmtSize(f.downloaded)}/{fmtSize(f.total)}</span>
                        <span>{fmtSpeed(f.speed||0)} · {fmtETA(f.downloaded,f.total,f.speed||0)}</span>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* ═══ 已完成 ═══ */}
      {completedGroups.length > 0 && (
        <div>
          <div className="flex items-center justify-between mb-2">
            <div className="text-xs text-gray-500">{t.downloadPage.completed} ({completedGroups.reduce((s,g)=>s+g.files.length,0)})</div>
            <button onClick={clearCompleted} className="text-xs text-gray-400 hover:text-gray-600">{t.downloadPage.clearCompleted}</button>
          </div>
          <div className="space-y-3">
            {completedGroups.map(g => (
              <div key={g.key} className="border dark:border-gray-700 rounded-lg overflow-hidden">
                <div className="px-4 py-2 bg-gray-100 dark:bg-gray-800 text-sm font-medium">{g.key} · {g.files.length} files</div>
                <div className="divide-y dark:divide-gray-700">
                  {g.files.map(f => (
                    <div key={f.fileName} className="px-4 py-2.5 flex items-center justify-between text-xs">
                      <div>
                        <span>{f.fileName}</span>
                        <span className={`ml-2 ${f.status==='completed'?'text-green-500':'text-gray-400'}`}>
                          {f.status==='completed'?`✓ ${fmtSize(f.total)}`:`${t.modelRepo.cancelled} · ${fmtSize(f.downloaded)}/${fmtSize(f.total)}`}
                        </span>
                      </div>
                      {f.path && (
                        <button onClick={() => cancelAndCleanupDownload(f.fileName, f.path!)} className="text-red-400 hover:text-red-600"><Trash2 className="w-3 h-3" /></button>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}
