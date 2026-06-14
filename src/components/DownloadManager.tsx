import { useState, useMemo } from 'react'
import { Trash2, FolderOpen, Pause, X, Download } from 'lucide-react'
import { useAppStore } from '../store'
import type { DownloadProgress } from '../store/types'

type Tab = 'active' | 'completed'

export default function DownloadManager() {
  const { downloadTasks, cancelFileDownload, pauseFileDownload, cancelAndCleanupDownload } = useAppStore()
  const [tab, setTab] = useState<Tab>('active')

  const tasks = Object.values(downloadTasks)

  // Group by repoId
  const groups = useMemo(() => {
    const map = new Map<string, DownloadProgress[]>()
    for (const t of tasks) {
      const key = t.repoId || 'unknown'
      if (!map.has(key)) map.set(key, [])
      map.get(key)!.push(t)
    }
    return Array.from(map.entries()).map(([repoId, files]) => ({ repoId, files }))
  }, [tasks])

  const filterByTab = (g: { repoId: string; files: DownloadProgress[] }) => {
    if (tab === 'active') return g.files.some(f => f.status === 'active' || f.status === 'error')
    return g.files.every(f => f.status === 'completed' || f.status === 'cancelled')
  }

  const filtered = groups.filter(filterByTab)

  const fmtSize = (n: number) => {
    if (n < 1024) return `${n} B`
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
    if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`
    return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`
  }

  const fmtSpeed = (n: number) => {
    if (n < 1024) return `${n.toFixed(0)} B/s`
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB/s`
    return `${(n / (1024 * 1024)).toFixed(1)} MB/s`
  }

  const fmtETA = (downloaded: number, total: number, speed: number) => {
    if (speed <= 0 || total <= 0) return ''
    const remaining = total - downloaded
    const secs = Math.ceil(remaining / speed)
    if (secs < 60) return `${secs}s`
    if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`
    return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`
  }

  const totalActive = tasks.filter(t => t.status === 'active').length

  return (
    <div className="flex-1 p-6 overflow-y-auto">
      <div className="flex items-center justify-between mb-6">
        <h2 className="text-2xl font-bold flex items-center gap-2">
          <Download className="w-6 h-6" /> Downloads
          {totalActive > 0 && <span className="text-sm font-normal text-gray-400">({totalActive} active)</span>}
        </h2>
      </div>

      {/* Tabs */}
      <div className="flex gap-2 mb-4">
        <button onClick={() => setTab('active')}
          className={`px-4 py-1.5 rounded-lg text-sm font-medium transition-colors ${tab === 'active' ? 'bg-blue-600 text-white' : 'bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700'}`}>
          Active ({groups.filter(g => g.files.some(f => f.status === 'active')).length})
        </button>
        <button onClick={() => setTab('completed')}
          className={`px-4 py-1.5 rounded-lg text-sm font-medium transition-colors ${tab === 'completed' ? 'bg-blue-600 text-white' : 'bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700'}`}>
          Completed ({groups.filter(g => g.files.every(f => f.status === 'completed' || f.status === 'cancelled')).length})
        </button>
      </div>

      {filtered.length === 0 ? (
        <div className="text-center text-gray-500 py-12">
          {tab === 'active' ? 'No active downloads' : 'No completed downloads'}
        </div>
      ) : (
        <div className="space-y-4">
          {filtered.map((group) => {
            const totalSize = group.files.reduce((s, f) => s + f.total, 0)

            return (
              <div key={group.repoId} className="border dark:border-gray-700 rounded-lg overflow-hidden">
                {/* Repo header */}
                <div className="px-4 py-2.5 bg-gray-100 dark:bg-gray-800 flex items-center justify-between">
                  <div className="font-medium text-sm truncate flex-1">{group.repoId}</div>
                  <div className="text-xs text-gray-400 shrink-0 ml-2">
                    {group.files.length} file{group.files.length > 1 ? 's' : ''} · {fmtSize(totalSize)}
                  </div>
                </div>

                {/* Files */}
                <div className="divide-y dark:divide-gray-700">
                  {group.files.map((file) => (
                    <div key={file.fileName} className="px-4 py-2.5">
                      {/* File name and status */}
                      <div className="flex items-center justify-between mb-1">
                        <div className="text-sm truncate flex-1 min-w-0">
                          {file.fileName}
                          {file.status === 'error' && <span className="ml-2 text-xs text-red-500">({file.error || 'Failed'})</span>}
                        </div>
                        <div className="flex items-center gap-1 shrink-0 ml-2">
                          {file.status === 'active' && (
                            <>
                              <button onClick={() => pauseFileDownload(file.fileName)} title="Pause" className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded">
                                <Pause className="w-3.5 h-3.5" />
                              </button>
                              <button onClick={() => cancelFileDownload(file.fileName)} title="Cancel" className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded text-red-500">
                                <X className="w-3.5 h-3.5" />
                              </button>
                            </>
                          )}
                          {file.status === 'completed' && file.path && (
                            <button onClick={() => {/* open in explorer */}} title="Open folder" className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded text-blue-500">
                              <FolderOpen className="w-3.5 h-3.5" />
                            </button>
                          )}
                          {(file.status === 'completed' || file.status === 'cancelled' || file.status === 'error') && file.path && (
                            <button onClick={() => cancelAndCleanupDownload(file.fileName, file.path!)} title="Delete" className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded text-red-400">
                              <Trash2 className="w-3.5 h-3.5" />
                            </button>
                          )}
                        </div>
                      </div>

                      {/* Progress bar */}
                      {file.status === 'active' && (
                        <>
                          <div className="h-1.5 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden mb-1">
                            <div className="h-full rounded-full bg-blue-500 transition-all duration-300"
                              style={{ width: `${file.total > 0 ? Math.min(100, (file.downloaded / file.total) * 100) : 0}%` }} />
                          </div>
                          <div className="flex justify-between text-xs text-gray-400">
                            <span>{fmtSize(file.downloaded)} / {fmtSize(file.total)} ({file.total > 0 ? ((file.downloaded / file.total) * 100).toFixed(0) : 0}%)</span>
                            <span className="flex gap-3">
                              <span>{fmtSpeed(file.speed || 0)}</span>
                              <span className="text-gray-500">{fmtETA(file.downloaded, file.total, file.speed || 0)}</span>
                            </span>
                          </div>
                        </>
                      )}

                      {file.status === 'completed' && (
                        <div className="text-xs text-green-500">✓ Completed · {fmtSize(file.total)}</div>
                      )}
                      {file.status === 'cancelled' && (
                        <div className="text-xs text-gray-400">Cancelled · {fmtSize(file.downloaded)} / {fmtSize(file.total)}</div>
                      )}
                      {file.status === 'error' && (
                        <div className="text-xs text-red-400">{fmtSize(file.downloaded)} / {fmtSize(file.total)}</div>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}
