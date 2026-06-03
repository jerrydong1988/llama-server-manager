import { useState, useEffect } from 'react'
import { Search, Download, FolderOpen, Trash2, RefreshCw, FileText, X, ChevronDown, ChevronRight, File, Image } from 'lucide-react'
import { useAppStore, type MsFileEntry, type ModelInfo } from '../store'
import { useI18n } from '../i18n'
import { listen } from '@tauri-apps/api/event'
import { confirm } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { normalizePath, pathJoin } from '../utils/path'

const DEFAULT_SAVE_DIR = 'models'

const ModelRepo = () => {
  const { models, modelDirs, setModelDirs, scanModels, isLoading, loadInitialData, deleteModelFile, openModelFolder, browseModelscope, downloadModelscopeFiles, pauseFileDownload, cancelAndCleanupDownload } = useAppStore()
  const { t } = useI18n()
  const [searchQuery, setSearchQuery] = useState('')
  const [showMsModal, setShowMsModal] = useState(false)
  const [msRepoId, setMsRepoId] = useState('')
  const [msFiles, setMsFiles] = useState<MsFileEntry[]>([])
  const [msStatus, setMsStatus] = useState('')
  const [msBrowsing, setMsBrowsing] = useState(false)
  const [fileStates, setFileStates] = useState<Record<string, {downloaded: number; total: number; speed?: number; status: 'pending'|'downloading'|'done'|'error'|'cancelled'|'paused'; error?: string}>>({})
  const [msSaveDir, setMsSaveDir] = useState(DEFAULT_SAVE_DIR)
  const [scanError, setScanError] = useState('')
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const [pausedSet, setPausedSet] = useState<Set<string>>(new Set())

  useEffect(() => { loadInitialData() }, [loadInitialData])

  // 下载事件监听
  useEffect(() => {
    const fns: (() => void)[] = []
    listen<{fileName: string; downloaded: number; total: number; speed?: number}>('download-progress', (e) => {
      setFileStates(s => ({...s, [e.payload.fileName]: {downloaded: e.payload.downloaded, total: e.payload.total, speed: e.payload.speed, status: 'downloading'}}))
    }).then(f => fns.push(f))
    listen<{fileName: string; path: string}>('download-complete', (e) => {
      setFileStates(s => ({...s, [e.payload.fileName]: {...(s[e.payload.fileName]||{downloaded:0,total:0}), status: 'done' as const}}))
    }).then(f => fns.push(f))
    listen<{fileName: string; error: string}>('download-error', (e) => {
      setFileStates(s => ({...s, [e.payload.fileName]: {...(s[e.payload.fileName]||{downloaded:0,total:0}), status: 'error' as const, error: e.payload.error}}))
    }).then(f => fns.push(f))
    listen<{fileName: string}>('download-cancelled', (e) => {
      setFileStates(s => {
        const prev = s[e.payload.fileName]
        // 如果前端已标记为 paused，保持暂停状态，不被后端事件覆盖
        if (prev?.status === 'paused') return s
        return {...s, [e.payload.fileName]: {...(prev||{downloaded:0,total:0}), status: 'cancelled' as const}}
      })
    }).then(f => fns.push(f))
    return () => { fns.forEach(fn => fn()) }
  }, [])

  // ── 自适应递归树 ──────────────────────────────────────────
  interface TreeNode {
    name: string
    path: string    // full path for dirs, file path for leaves
    isDir: boolean
    children?: Map<string, TreeNode>
    model?: typeof models[0]  // only for leaf nodes
  }

  // 构建树：从 rootDir 出发，按实际目录结构组织 model 文件
  function buildTree(rootDir: string, models: ModelInfo[]): TreeNode {
    const normDir = normalizePath(rootDir)
    const root: TreeNode = { name: rootDir, path: normDir, isDir: true, children: new Map() }
    const normRoot = normDir.toLowerCase()
    for (const m of models) {
      const normPath = normalizePath(m.path).toLowerCase()
      if (!normPath.startsWith(normRoot)) continue
      const rel = normalizePath(m.path.substring(rootDir.length)).replace(/^\/+/, '')
      const parts = rel.split('/')
      let cur = root
      for (let i = 0; i < parts.length; i++) {
        if (i === parts.length - 1) { cur.children!.set(parts[i], { name: parts[i], path: m.path, isDir: false, model: m }) }
        else {
          if (!cur.children!.has(parts[i])) {
            const childPath = pathJoin(cur.path, parts[i])
            cur.children!.set(parts[i], { name: parts[i], path: childPath, isDir: true, children: new Map() })
          }
          cur = cur.children!.get(parts[i])!
        }
      }
    }
    return root
  }

  const trees = modelDirs.map(d => buildTree(d, models))

  // 统计目录下文件和投影器的数量
  function countDir(node: TreeNode): { models: number; mmproj: number } {
    if (!node.isDir) return node.model!.file_type === 'mmproj' ? { models: 0, mmproj: 1 } : { models: 1, mmproj: 0 }
    let m = 0, p = 0
    if (node.children) for (const child of node.children.values()) {
      const c = countDir(child); m += c.models; p += c.mmproj
    }
    return { models: m, mmproj: p }
  }

  // 递归渲染节点
  const renderNode = (node: TreeNode, depth: number) => {
    const nodeKey = node.path
    const isCollapsed = collapsed.has(nodeKey)
    if (node.isDir) {
      const cnt = countDir(node)
      const paddingLeft = depth * 16 + 16
      return (
        <div key={nodeKey}>
          <button
            onClick={() => { const next = new Set(collapsed); if (next.has(nodeKey)) { next.delete(nodeKey) } else { next.add(nodeKey) }; setCollapsed(next) }}
            style={{ paddingLeft: `${paddingLeft}px` }}
            className="w-full flex items-center gap-2 py-1.5 hover:bg-gray-50 dark:hover:bg-gray-700 transition-colors text-left"
          >
            {isCollapsed ? <ChevronRight className="w-3.5 h-3.5 text-gray-400 shrink-0" /> : <ChevronDown className="w-3.5 h-3.5 text-gray-400 shrink-0" />}
            {depth === 0 ? <FolderOpen className="w-3.5 h-3.5 text-yellow-500 shrink-0" /> : <span className="text-xs text-gray-400 w-3.5 shrink-0">📁</span>}
            <span className="text-xs font-medium truncate flex-1">{node.name}</span>
            <span className="text-xs text-gray-400 shrink-0">
              {cnt.models > 0 && `${cnt.models} ${t.instance.model}`}{cnt.models > 0 && cnt.mmproj > 0 && ' · '}{cnt.mmproj > 0 && `${cnt.mmproj} MMProj`}
            </span>
          </button>
          {!isCollapsed && node.children && (
            <div>
              {[...node.children.values()].sort((a, b) => {
                if (a.isDir !== b.isDir) return a.isDir ? -1 : 1
                return a.name.localeCompare(b.name)
              }).map(child => renderNode(child, depth + 1))}
            </div>
          )}
        </div>
      )
    }
    // 文件节点
    const m = node.model!
    const paddingLeft = depth * 16 + 16 + 20
    return (
      <div key={nodeKey} style={{ paddingLeft: `${paddingLeft}px` }}
        className="flex items-center gap-2 py-1.5 pr-4 hover:bg-gray-50 dark:hover:bg-gray-700 transition-colors">
        {m.file_type === 'mmproj' ? <Image className="w-3.5 h-3.5 text-purple-500 shrink-0" /> : <File className="w-3.5 h-3.5 text-blue-500 shrink-0" />}
        <span className="text-xs truncate flex-1" title={m.name}>{m.name}</span>
        <span className={`text-xs px-1 py-0.5 rounded shrink-0 ${
          m.file_type === 'mmproj' ? 'bg-purple-100 dark:bg-purple-900/30 text-purple-700 dark:text-purple-300' :
          'bg-blue-100 dark:bg-blue-900/30 text-blue-700'
        }`}>
          {m.file_type === 'mmproj' ? t.modelRepo.typeMmprojShort : t.modelRepo.typeModelShort}
        </span>
        {m.quant_type && <span className="text-xs text-gray-400 shrink-0 w-14 text-right">{m.quant_type}</span>}
        <span className="text-xs text-gray-400 shrink-0 w-20 text-right">{formatSize(m.size)}</span>
        <div className="flex items-center gap-0.5 shrink-0">
          <button onClick={() => openModelFolder(m.path)} className="p-0.5 text-blue-600 hover:bg-blue-50 dark:hover:bg-blue-900/30 rounded" title={t.modelRepo.openFolder}>
            <FolderOpen className="w-3 h-3" />
          </button>
          <button onClick={() => handleDeleteFile(m.path)} className="p-0.5 text-red-600 hover:bg-red-50 dark:hover:bg-red-900/30 rounded" title={t.modelRepo.delete}>
            <Trash2 className="w-3 h-3" />
          </button>
        </div>
      </div>
    )
  }

  const formatSize = (bytes: number) => {
    if (bytes < 1024) return bytes + ' B'
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(2) + ' KB'
    if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(2) + ' MB'
    return (bytes / (1024 * 1024 * 1024)).toFixed(2) + ' GB'
  }

  const formatSpeed = (bytesPerSec: number) => {
    if (bytesPerSec < 1024) return bytesPerSec.toFixed(0) + ' B/s'
    if (bytesPerSec < 1024 * 1024) return (bytesPerSec / 1024).toFixed(1) + ' KB/s'
    return (bytesPerSec / 1024 / 1024).toFixed(1) + ' MB/s'
  }

  const modelTypeLabel = (fileType: string) => {
    switch (fileType) { case 'mmproj': return t.modelRepo.typeMmproj; case 'imatrix': return t.modelRepo.typeImatrix; default: return t.modelRepo.typeModel }
  }

  const handleScan = async () => {
    const err = await scanModels(modelDirs)
    if (err) setScanError(err)
    else setScanError('')
  }

  const handleAddDirectory = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const dir = await open({ directory: true, title: t.modelRepo.addDir })
      if (dir) {
        const d = dir as string
        const all = [...new Set([...modelDirs, d])]
        setModelDirs(all)
        const err = await scanModels(all)
        if (err) setScanError(err)
        else setScanError('')
      }
    } catch (_) { await scanModels(modelDirs) }
  }

  const handleRemoveDir = async (dir: string) => {
    if (!await confirm(t.modelRepo.removeDirConfirm, { title: t.modelRepo.remove, kind: 'warning' })) return
    const next = modelDirs.filter(d => d !== dir)
    setModelDirs(next)
    scanModels(next)
  }

  const handleBrowseSaveDir = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const dir = await open({ directory: true, title: t.modelRepo.saveDir })
      if (dir) setMsSaveDir(dir as string)
    } catch (_) {}
  }

  const handleDeleteFile = async (path: string) => {
    if (!await confirm(t.modelRepo.deleteConfirm, { title: t.modelRepo.delete, kind: 'warning' })) return
    await deleteModelFile(path)
  }

  const handleMsBrowse = async () => {
    if (!msRepoId.trim()) { setMsStatus(t.modelRepo.inputRepoId); return }
    setMsBrowsing(true)
    setMsStatus(t.modelRepo.querying)
    try {
      const files = await browseModelscope(msRepoId.trim())
      setMsFiles(files)
      if (files.length === 0) setMsStatus(t.modelRepo.notFound)
      else setMsStatus(`${t.modelRepo.found} ${files.length} ${t.modelRepo.files}`)
    } catch (e: any) {
      setMsStatus(`${t.modelRepo.queryFailed}${typeof e === 'string' ? e : t.modelRepo.networkError}`)
    } finally { setMsBrowsing(false) }
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-4">
          <div className="relative">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400" />
            <input type="text" placeholder={t.modelRepo.searchPlaceholder} value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="pl-10 pr-4 py-2 rounded-lg border dark:border-gray-700 bg-white dark:bg-gray-800 w-80" />
          </div>
          <button onClick={handleScan} disabled={isLoading}
            className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-400 text-white rounded-lg transition-colors">
            <RefreshCw className={`w-4 h-4 ${isLoading ? 'animate-spin' : ''}`} /> {t.modelRepo.scan}
          </button>
          <button onClick={handleAddDirectory}
            className="flex items-center gap-2 px-4 py-2 bg-green-600 hover:bg-green-700 text-white rounded-lg transition-colors">
            <FolderOpen className="w-4 h-4" /> {t.modelRepo.addDir}
          </button>
        </div>
        <button onClick={() => setShowMsModal(true)}
          className="flex items-center gap-2 px-4 py-2 bg-purple-600 hover:bg-purple-700 text-white rounded-lg transition-colors">
          <Download className="w-4 h-4" /> {t.modelRepo.downloadMS}
        </button>
      </div>

      {/* 模型目录列表 */}
      {modelDirs.length > 0 && (
        <div className="bg-gray-50 dark:bg-gray-800/50 rounded-lg p-3 text-sm">
          <div className="flex items-center gap-2 mb-2 text-gray-500 font-medium">{t.modelRepo.modelDirs}</div>
          {modelDirs.map(d => (
            <div key={d} className="flex items-center justify-between py-1 px-2 rounded hover:bg-gray-100 dark:hover:bg-gray-700">
              <span className="text-xs truncate flex-1 mr-2">{d}</span>
              <button onClick={() => handleRemoveDir(d)}
                className="text-xs text-red-500 hover:text-red-700 px-2 py-0.5 rounded hover:bg-red-50 dark:hover:bg-red-900/20">
                {t.modelRepo.remove}
              </button>
            </div>
          ))}
        </div>
      )}

      {scanError && (
        <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg p-3 text-red-700 dark:text-red-400 text-sm">
          {scanError}
        </div>
      )}

      {/* 自适应递归模型树 */}
      {trees.length === 0 ? (
        <div className="bg-white dark:bg-gray-800 rounded-lg p-12 text-center border dark:border-gray-700">
          <FileText className="w-12 h-12 mx-auto mb-3 opacity-50" />
          <p className="text-gray-500 dark:text-gray-400">{t.modelRepo.noModels}</p>
        </div>
      ) : (
        <div className="bg-white dark:bg-gray-800 rounded-lg border dark:border-gray-700 overflow-hidden max-h-[calc(100vh-300px)] overflow-y-auto">
          {trees.map(t => renderNode(t, 0))}
        </div>
      )}

      {/* ModelScope 下载弹窗 */}
      {showMsModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white dark:bg-gray-800 rounded-lg p-6 w-full max-w-2xl max-h-[90vh] overflow-y-auto">
            <div className="flex items-center justify-between mb-6">
              <h3 className="text-lg font-semibold">{t.modelRepo.msTitle}</h3>
              <button onClick={() => setShowMsModal(false)} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded"><X className="w-5 h-5" /></button>
            </div>

            <div className="space-y-4">
              <div className="flex gap-2">
                <input type="text" value={msRepoId} onChange={(e) => setMsRepoId(e.target.value)}
                  onKeyDown={(e) => e.key === 'Enter' && handleMsBrowse()}
                  placeholder={t.modelRepo.repoIdPlaceholder}
                  className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
                <button onClick={handleMsBrowse} disabled={msBrowsing}
                  className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-400 text-white rounded-lg transition-colors">
                  {t.modelRepo.browseFiles}
                </button>
              </div>

              {msStatus && <p className="text-sm text-gray-500">{msStatus}</p>}

              {msFiles.length > 0 && (
                <>
                  <div>
                    <label className="block text-sm font-medium mb-1">{t.modelRepo.saveDir}</label>
                    <div className="flex gap-2">
                      <input type="text" value={msSaveDir} onChange={(e) => setMsSaveDir(e.target.value)}
                        className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 text-sm" />
                      <button onClick={handleBrowseSaveDir}
                        className="px-3 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg transition-colors">
                        <FolderOpen className="w-5 h-5" />
                      </button>
                    </div>
                  </div>

                  <div className="border dark:border-gray-700 rounded-lg max-h-64 overflow-y-auto">
                    {msFiles.map((f) => {
                      const st = fileStates[f.name]
                      const isPaused = pausedSet.has(f.name)
                      const partialPath = pathJoin(msSaveDir, msRepoId, f.name)

                      // 单文件下载
                      const startSingleDownload = async () => {
                        setPausedSet(s => { const n = new Set(s); n.delete(f.name); return n })
                        setFileStates(s => ({...s, [f.name]: {downloaded: 0, total: f.size, status: 'pending' as const}}))
                        try {
                          await downloadModelscopeFiles(msRepoId, [f], msSaveDir)
                          const resolvedDir = await invoke<string>('resolve_path', { path: msSaveDir })
                          const allDirs = [...new Set([...modelDirs, resolvedDir])]
                          setModelDirs(allDirs)
                          await scanModels(allDirs)
                        } catch (e: any) { console.error(e) }
                      }

                      const resumeDownload = async () => {
                        setPausedSet(s => { const n = new Set(s); n.delete(f.name); return n })
                        try {
                          await downloadModelscopeFiles(msRepoId, [f], msSaveDir)
                        } catch (e: any) { console.error(e) }
                      }

                      // 暂停 → 保持 pausedSet
                      if (isPaused) {
                        const prev = st || {downloaded: 0, total: f.size}
                        return (
                          <div key={f.path} className="px-4 py-2.5 border-b dark:border-gray-700 last:border-0 space-y-1">
                            <div className="flex items-center justify-between text-xs">
                              <span className="text-sm truncate flex-1 mr-2">{f.name}</span>
                              <span className="text-xs text-gray-400 shrink-0 ml-2">{modelTypeLabel(f.file_type)} · {formatSize(f.size)}</span>
                            </div>
                            <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-1.5">
                              <div className="bg-yellow-500 h-1.5 rounded-full" style={{width: `${(prev.total ?? f.size)>0?((prev.downloaded ?? 0)/(prev.total ?? f.size))*100:0}%`}} />
                            </div>
                            <div className="flex items-center gap-2">
                              <span className="text-xs text-yellow-500">{formatSize(prev.downloaded ?? 0)} / {formatSize(prev.total ?? f.size)}</span>
                               <button onClick={resumeDownload} className="text-xs text-green-500 hover:text-green-700">{t.modelRepo.resume}</button>
                                <button onClick={() => {
                                  setPausedSet(s => { const n = new Set(s); n.delete(f.name); return n })
                                  cancelAndCleanupDownload(f.name, partialPath)
                                }} className="text-xs text-red-500 hover:text-red-700 ml-auto">{t.modelRepo.cancel}</button>
                            </div>
                          </div>
                        )
                      }

                      return (
                        <div key={f.path} className="px-4 py-2.5 border-b dark:border-gray-700 last:border-0 space-y-1">
                          <div className="flex items-center justify-between text-xs">
                            <span className="text-sm truncate flex-1 mr-2">{f.name}</span>
                            <span className="text-xs text-gray-400 shrink-0 ml-2">{modelTypeLabel(f.file_type)} · {formatSize(f.size)}</span>
                          </div>

                          {/* 进度条 */}
                          {st && (st.status === 'downloading' || st.status === 'pending' || st.status === 'paused') && (
                            <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-1.5">
                              <div className="bg-blue-600 h-1.5 rounded-full" style={{width: `${st.total>0?(st.downloaded/st.total)*100:0}%`}} />
                            </div>
                          )}

                          {/* 状态 + 操作按钮 */}
                          <div className="flex items-center gap-2">
                            {(!st || st.status === 'done' || st.status === 'error' || st.status === 'cancelled') ? (
                              <>
                                {(st?.status === 'done') && <span className="text-xs text-green-500">{t.modelRepo.done}</span>}
                                {(st?.status === 'error') && <span className="text-xs text-red-500">{st.error || t.modelRepo.failed}</span>}
                                {(st?.status === 'cancelled') && <span className="text-xs text-yellow-500">{t.modelRepo.cancelled}</span>}
                                <button onClick={startSingleDownload}
                                  className="text-xs text-blue-500 hover:text-blue-700 ml-auto">{t.modelRepo.downloadBtn}</button>
                              </>
                            ) : st.status === 'paused' ? (
                              <>
                                <span className="text-xs text-yellow-500">{formatSize(st.downloaded)} / {formatSize(st.total)}</span>
                                <button onClick={resumeDownload} className="text-xs text-green-500 hover:text-green-700">{t.modelRepo.resume}</button>
                                <button onClick={() => cancelAndCleanupDownload(f.name, partialPath)}
                                  className="text-xs text-red-500 hover:text-red-700 ml-auto">{t.modelRepo.cancel}</button>
                              </>
                            ) : st.status === 'downloading' || st.status === 'pending' ? (
                              <>
                                <span className="text-xs text-blue-500">{formatSize(st.downloaded ?? 0)} / {formatSize(st.total)}{st.speed ? ` · ${formatSpeed(st.speed)}` : ''}</span>
                                <button onClick={() => { setPausedSet(s => { const n = new Set(s); n.add(f.name); return n }); pauseFileDownload(f.name) }}
                                  className="text-xs text-yellow-500 hover:text-yellow-700">{t.modelRepo.pause}</button>
                                <button onClick={() => cancelAndCleanupDownload(f.name, partialPath)}
                                  className="text-xs text-red-500 hover:text-red-700 ml-auto">{t.modelRepo.cancel}</button>
                              </>
                            ) : null}
                          </div>
                        </div>
                      )
                    })}
                  </div>
                </>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

export default ModelRepo
