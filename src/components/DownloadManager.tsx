import { useState, useMemo } from 'react'
import { Trash2, FolderOpen, X, Download } from 'lucide-react'
import { useAppStore, type MsFileEntry } from '../store'
import type { DownloadProgress } from '../store/types'
import { useI18n } from '../i18n'
import { pathJoin } from '../utils/path'

type Tab = 'browse' | 'active' | 'completed'
type DownloadSource = 'modelscope' | 'huggingface'

const DEFAULT_SAVE_DIR = 'models'

export default function DownloadManager() {
  const { t } = useI18n()
  const { downloadTasks, downloadQueue, cancelFileDownload, cancelAndCleanupDownload, removeFromDownloadQueue, addToDownloadQueue, browseModelscope, browseHuggingface, pauseFileDownload } = useAppStore()
  const [tab, setTab] = useState<Tab>('browse')
  const [source, setSource] = useState<DownloadSource>('modelscope')
  const [repoId, setRepoId] = useState('')
  const [files, setFiles] = useState<MsFileEntry[]>([])
  const [status, setStatus] = useState('')
  const [browsing, setBrowsing] = useState(false)
  const [saveDir, setSaveDir] = useState(DEFAULT_SAVE_DIR)

  // ── Helpers ──
  const fmtSize = (bytes: number) => {
    if (bytes < 1024) return bytes + ' B'
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(2) + ' KB'
    if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(2) + ' MB'
    return (bytes / (1024 * 1024 * 1024)).toFixed(2) + ' GB'
  }

  const fmtSpeed = (bytesPerSec: number) => {
    if (bytesPerSec < 1024) return bytesPerSec.toFixed(0) + ' B/s'
    if (bytesPerSec < 1024 * 1024) return (bytesPerSec / 1024).toFixed(1) + ' KB/s'
    return (bytesPerSec / 1024 / 1024).toFixed(1) + ' MB/s'
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

  // ── Browse handlers ──
  const handleBrowse = async () => {
    if (!repoId.trim()) { setStatus(t.modelRepo.inputRepoId); return }
    setBrowsing(true); setStatus(t.modelRepo.querying)
    try {
      const result = source === 'modelscope'
        ? await browseModelscope(repoId.trim())
        : await browseHuggingface(repoId.trim())
      setFiles(result)
      setStatus(result.length === 0 ? t.modelRepo.notFound : `${t.modelRepo.found} ${result.length} ${t.modelRepo.files}`)
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

  const handleDownloadFile = (f: MsFileEntry) => {
    addToDownloadQueue({ repoId, source, files: [f], saveDir })
  }

  const handleDownloadAll = () => {
    if (files.length === 0) return
    addToDownloadQueue({ repoId, source, files, saveDir })
  }

  const handlePause = (f: MsFileEntry) => {
    const task = downloadTasks[f.name]
    if (task) {
      useAppStore.getState().setDownloadTasks({ ...downloadTasks, [f.name]: { ...task, status: 'paused' } })
      pauseFileDownload(f.name)
    }
  }

  const handleCancelFile = (f: MsFileEntry) => {
    const task = downloadTasks[f.name]
    if (task) {
      useAppStore.getState().setDownloadTasks({ ...downloadTasks, [f.name]: { ...task, status: 'cancelled' } })
    }
    cancelAndCleanupDownload(f.name, pathJoin(saveDir, repoId, f.name))
  }

  // ── Active/Completed tab helpers ──
  const tasks = Object.values(downloadTasks)

  const groups = useMemo(() => {
    const map = new Map<string, DownloadProgress[]>()
    for (const t of tasks) {
      const key = `${t.source || ''}:${t.repoId || 'unknown'}`
      if (!map.has(key)) map.set(key, [])
      map.get(key)!.push(t)
    }
    return Array.from(map.entries()).map(([key, files]) => ({ key, files }))
  }, [tasks])

  const activeGroups = groups.filter(g => g.files.some(f => f.status === 'active' || f.status === 'queued' || f.status === 'error'))
  const completedGroups = groups.filter(g => g.files.every(f => f.status === 'completed' || f.status === 'cancelled'))
  const totalActive = tasks.filter(t => t.status === 'active').length

  return (
    <div className="flex-1 p-6 overflow-y-auto">
      <div className="flex items-center justify-between mb-6">
        <h2 className="text-2xl font-bold flex items-center gap-2">
          <Download className="w-6 h-6" /> {t.nav.downloads || 'Downloads'}
          {totalActive > 0 && <span className="text-sm font-normal text-gray-400">({totalActive} active)</span>}
        </h2>
      </div>

      {/* Queued */}
      {downloadQueue.length > 0 && (
        <div className="mb-4">
          <div className="text-xs text-gray-500 mb-2">Queued ({downloadQueue.length})</div>
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

      {/* Tabs */}
      <div className="flex gap-2 mb-4">
        <button onClick={() => setTab('browse')} className={`px-4 py-1.5 rounded-lg text-sm font-medium transition-colors ${tab === 'browse' ? 'bg-blue-600 text-white' : 'bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700'}`}>Browse</button>
        <button onClick={() => setTab('active')} className={`px-4 py-1.5 rounded-lg text-sm font-medium transition-colors ${tab === 'active' ? 'bg-blue-600 text-white' : 'bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700'}`}>Active ({activeGroups.length})</button>
        <button onClick={() => setTab('completed')} className={`px-4 py-1.5 rounded-lg text-sm font-medium transition-colors ${tab === 'completed' ? 'bg-blue-600 text-white' : 'bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700'}`}>Completed ({completedGroups.length})</button>
      </div>

      {/* Browse Tab */}
      {tab === 'browse' && (
        <div className="space-y-4">
          {/* Source selector */}
          <div className="flex gap-2">
            <button onClick={() => { setSource('modelscope'); setFiles([]); setStatus('') }}
              className={`px-3 py-1.5 rounded-lg text-sm font-medium ${source === 'modelscope' ? 'bg-blue-600 text-white' : 'bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600'}`}>ModelScope</button>
            <button onClick={() => { setSource('huggingface'); setFiles([]); setStatus('') }}
              className={`px-3 py-1.5 rounded-lg text-sm font-medium ${source === 'huggingface' ? 'bg-blue-600 text-white' : 'bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600'}`}>HuggingFace</button>
          </div>

          {/* Repo ID + Browse */}
          <div className="flex gap-2">
            <input type="text" value={repoId} onChange={e => setRepoId(e.target.value)}
              onKeyDown={e => e.key === 'Enter' && handleBrowse()}
              placeholder={source === 'modelscope' ? t.modelRepo.repoIdPlaceholder : t.modelRepo.hfRepoIdPlaceholder}
              className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
            <button onClick={handleBrowse} disabled={browsing}
              className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-400 text-white rounded-lg">{t.modelRepo.browseFiles}</button>
          </div>

          {status && <p className="text-sm text-gray-500">{status}</p>}

          {/* Save dir + file list */}
          {files.length > 0 && (
            <>
              <div>
                <label className="block text-sm font-medium mb-1">{t.modelRepo.saveDir}</label>
                <div className="flex gap-2">
                  <input type="text" value={saveDir} onChange={e => setSaveDir(e.target.value)}
                    className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 text-sm" />
                  <button onClick={handleBrowseSaveDir} className="px-3 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg">
                    <FolderOpen className="w-5 h-5" />
                  </button>
                </div>
              </div>

              <div className="flex justify-end">
                <button onClick={handleDownloadAll} className="px-3 py-1.5 bg-green-600 hover:bg-green-700 text-white rounded-lg text-sm">
                  ⬇ Download All ({files.length} files · {fmtSize(files.reduce((s,f)=>s+f.size,0))})
                </button>
              </div>

              <div className="border dark:border-gray-700 rounded-lg max-h-80 overflow-y-auto">
                {files.map((f) => {
                  const task = downloadTasks[f.name]

                  if (task?.status === 'paused') {
                    return (
                      <div key={f.path} className="px-4 py-2.5 border-b dark:border-gray-700 last:border-0 space-y-1">
                        <div className="flex items-center justify-between text-xs">
                          <span className="text-sm truncate flex-1 mr-2">{f.name}</span>
                          <span className="text-xs text-gray-400 shrink-0">{modelTypeLabel(f.file_type)} · {fmtSize(f.size)}</span>
                        </div>
                        <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-1.5">
                          <div className="bg-yellow-500 h-1.5 rounded-full" style={{width: `${task.total>0?(task.downloaded/task.total)*100:0}%`}} />
                        </div>
                        <div className="flex items-center gap-2">
                          <span className="text-xs text-yellow-500">{fmtSize(task.downloaded??0)} / {fmtSize(task.total)}</span>
                          <button onClick={() => handleDownloadFile(f)} className="text-xs text-green-500 hover:text-green-700">{t.modelRepo.resume}</button>
                          <button onClick={() => handleCancelFile(f)} className="text-xs text-red-500 hover:text-red-700 ml-auto">{t.modelRepo.cancel}</button>
                        </div>
                      </div>
                    )
                  }

                  return (
                    <div key={f.path} className="px-4 py-2.5 border-b dark:border-gray-700 last:border-0 space-y-1">
                      <div className="flex items-center justify-between text-xs">
                        <span className="text-sm truncate flex-1 mr-2">{f.name}</span>
                        <span className="text-xs text-gray-400 shrink-0">{modelTypeLabel(f.file_type)} · {fmtSize(f.size)}</span>
                      </div>
                      {task && (task.status === 'active' || task.status === 'queued') && (
                        <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-1.5">
                          <div className="bg-blue-600 h-1.5 rounded-full" style={{width: `${task.total>0?(task.downloaded/task.total)*100:0}%`}} />
                        </div>
                      )}
                      <div className="flex items-center gap-2">
                        {(!task || task.status === 'completed' || task.status === 'error' || task.status === 'cancelled') ? (
                          <>
                            {task?.status === 'completed' && <span className="text-xs text-green-500">{t.modelRepo.done}</span>}
                            {task?.status === 'error' && <span className="text-xs text-red-500">{task.error || t.modelRepo.failed}</span>}
                            {task?.status === 'cancelled' && <span className="text-xs text-yellow-500">{t.modelRepo.cancelled}</span>}
                            <button onClick={() => handleDownloadFile(f)} className="text-xs text-blue-500 hover:text-blue-700 ml-auto">{t.modelRepo.downloadBtn}</button>
                          </>
                        ) : task.status === 'active' ? (
                          <>
                            <span className="text-xs text-blue-500">{fmtSize(task.downloaded??0)} / {fmtSize(task.total)}{task.speed ? ` · ${fmtSpeed(task.speed)}` : ''}</span>
                            <button onClick={() => handlePause(f)} className="text-xs text-yellow-500 hover:text-yellow-700">{t.modelRepo.pause}</button>
                            <button onClick={() => handleCancelFile(f)} className="text-xs text-red-500 hover:text-red-700 ml-auto">{t.modelRepo.cancel}</button>
                          </>
                        ) : task.status === 'queued' ? (
                          <span className="text-xs text-gray-400">Queued...</span>
                        ) : null}
                      </div>
                    </div>
                  )
                })}
              </div>
            </>
          )}
        </div>
      )}

      {/* Active Tab */}
      {tab === 'active' && (
        activeGroups.length === 0 ? (
          <div className="text-center text-gray-500 py-12">No active downloads</div>
        ) : (
          <div className="space-y-4">
            {activeGroups.map((group) => (
              <div key={group.key} className="border dark:border-gray-700 rounded-lg overflow-hidden">
                <div className="px-4 py-2.5 bg-gray-100 dark:bg-gray-800 flex items-center justify-between">
                  <div className="font-medium text-sm truncate flex-1">{group.key}</div>
                  <div className="text-xs text-gray-400 shrink-0 ml-2">{group.files.length} file{group.files.length>1?'s':''} · {fmtSize(group.files.reduce((s,f)=>s+f.total,0))}</div>
                </div>
                <div className="divide-y dark:divide-gray-700">
                  {group.files.map((file) => (
                    <div key={file.fileName} className="px-4 py-2.5">
                      <div className="flex items-center justify-between mb-1">
                        <div className="text-sm truncate flex-1">{file.fileName}{file.status==='error'&&<span className="ml-2 text-xs text-red-500">({file.error||'Failed'})</span>}</div>
                        <div className="flex items-center gap-1 shrink-0 ml-2">
                          {file.status === 'active' && (
                            <button onClick={() => cancelFileDownload(file.fileName)} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded text-red-500"><X className="w-3.5 h-3.5" /></button>
                          )}
                          {(file.status==='completed'||file.status==='cancelled'||file.status==='error')&&file.path&&(
                            <button onClick={()=>cancelAndCleanupDownload(file.fileName,file.path!)} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded text-red-400"><Trash2 className="w-3.5 h-3.5" /></button>
                          )}
                        </div>
                      </div>
                      {file.status === 'active' && (
                        <>
                          <div className="h-1.5 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden mb-1">
                            <div className="h-full rounded-full bg-blue-500 transition-all" style={{width:`${file.total>0?Math.min(100,(file.downloaded/file.total)*100):0}%`}} />
                          </div>
                          <div className="flex justify-between text-xs text-gray-400">
                            <span>{fmtSize(file.downloaded)}/{fmtSize(file.total)}({file.total>0?((file.downloaded/file.total)*100).toFixed(0):0}%)</span>
                            <span className="flex gap-3"><span>{fmtSpeed(file.speed||0)}</span><span className="text-gray-500">{fmtETA(file.downloaded,file.total,file.speed||0)}</span></span>
                          </div>
                        </>
                      )}
                      {file.status === 'completed' && <div className="text-xs text-green-500">✓ Completed · {fmtSize(file.total)}</div>}
                      {file.status === 'cancelled' && <div className="text-xs text-gray-400">Cancelled · {fmtSize(file.downloaded)}/{fmtSize(file.total)}</div>}
                      {file.status === 'error' && <div className="text-xs text-red-400">{fmtSize(file.downloaded)}/{fmtSize(file.total)}</div>}
                    </div>
                  ))}
                </div>
              </div>
            ))}
          </div>
        )
      )}

      {/* Completed Tab */}
      {tab === 'completed' && (
        completedGroups.length === 0 ? (
          <div className="text-center text-gray-500 py-12">No completed downloads</div>
        ) : (
          <div className="space-y-4">
            {completedGroups.map((group) => (
              <div key={group.key} className="border dark:border-gray-700 rounded-lg overflow-hidden">
                <div className="px-4 py-2.5 bg-gray-100 dark:bg-gray-800 flex items-center justify-between">
                  <div className="font-medium text-sm truncate flex-1">{group.key}</div>
                  <div className="text-xs text-gray-400 shrink-0 ml-2">{group.files.length} file{group.files.length>1?'s':''} · {fmtSize(group.files.reduce((s,f)=>s+f.total,0))}</div>
                </div>
                <div className="divide-y dark:divide-gray-700">
                  {group.files.map((file) => (
                    <div key={file.fileName} className="px-4 py-2.5 flex items-center justify-between">
                      <div>
                        <div className="text-sm">{file.fileName}</div>
                        <div className={`text-xs ${file.status==='completed'?'text-green-500':'text-gray-400'}`}>
                          {file.status==='completed'?`✓ Completed · ${fmtSize(file.total)}`:`Cancelled · ${fmtSize(file.downloaded)}/${fmtSize(file.total)}`}
                        </div>
                      </div>
                      <div className="flex items-center gap-1">
                        {file.status==='completed'&&file.path&&<button onClick={()=>{}} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded text-blue-500"><FolderOpen className="w-3.5 h-3.5"/></button>}
                        {file.path&&<button onClick={()=>cancelAndCleanupDownload(file.fileName,file.path!)} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded text-red-400"><Trash2 className="w-3.5 h-3.5"/></button>}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            ))}
          </div>
        )
      )}
    </div>
  )
}
