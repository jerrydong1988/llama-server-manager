import { useState, useEffect } from 'react'
import { Search, Download, FolderOpen, Trash2, RefreshCw, FileText, X, ChevronDown, ChevronRight, File, Image } from 'lucide-react'
import { useAppStore, type MsFileEntry } from '../store'
import { listen } from '@tauri-apps/api/event'

const DEFAULT_SAVE_DIR = '..\\models'

const ModelRepo = () => {
  const { models, modelDirs, setModelDirs, scanModels, isLoading, loadInitialData, deleteModelFile, openModelFolder, browseModelscope, downloadModelscopeFiles, cancelFileDownload } = useAppStore()
  const [searchQuery, setSearchQuery] = useState('')
  const [showMsModal, setShowMsModal] = useState(false)
  const [msRepoId, setMsRepoId] = useState('')
  const [msFiles, setMsFiles] = useState<MsFileEntry[]>([])
  const [msSelected, setMsSelected] = useState<Set<string>>(new Set())
  const [msStatus, setMsStatus] = useState('')
  const [msBrowsing, setMsBrowsing] = useState(false)
  const [msDownloading, setMsDownloading] = useState(false)
  const [msCancelled, setMsCancelled] = useState(false)
  const [fileStates, setFileStates] = useState<Record<string, {downloaded: number; total: number; status: 'pending'|'downloading'|'done'|'error'|'cancelled'; error?: string}>>({})
  const [msSaveDir, setMsSaveDir] = useState(DEFAULT_SAVE_DIR)
  const [scanError, setScanError] = useState('')
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())

  // ── 自适应递归树 ──────────────────────────────────────────
  interface TreeNode {
    name: string
    path: string    // full path for dirs, file path for leaves
    isDir: boolean
    children?: Map<string, TreeNode>
    model?: typeof models[0]  // only for leaf nodes
  }

  // 构建树：从 rootDir 出发，按实际目录结构组织 model 文件
  function buildTree(rootDir: string, models: typeof models): TreeNode {
    const root: TreeNode = { name: rootDir, path: rootDir, isDir: true, children: new Map() }
    const normRoot = rootDir.replace(/\\/g, '\\').toLowerCase()
    for (const m of models) {
      const normPath = m.path.replace(/\\/g, '\\').toLowerCase()
      if (!normPath.startsWith(normRoot)) continue
      const rel = m.path.substring(rootDir.length).replace(/^[\\/]+/, '')
      const parts = rel.split(/[\\/]/)
      let cur = root
      for (let i = 0; i < parts.length; i++) {
        if (i === parts.length - 1) { cur.children!.set(parts[i], { name: parts[i], path: m.path, isDir: false, model: m }) }
        else {
          if (!cur.children!.has(parts[i])) {
            const childPath = cur.path + (cur.path.endsWith('\\') ? '' : '\\') + parts[i]
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
              {cnt.models > 0 && `${cnt.models} 模型`}{cnt.models > 0 && cnt.mmproj > 0 && ' · '}{cnt.mmproj > 0 && `${cnt.mmproj} 投影器`}
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
          {m.file_type === 'mmproj' ? '投影器' : '模型'}
        </span>
        {m.quant_type && <span className="text-xs text-gray-400 shrink-0 w-14 text-right">{m.quant_type}</span>}
        <span className="text-xs text-gray-400 shrink-0 w-20 text-right">{formatSize(m.size)}</span>
        <div className="flex items-center gap-0.5 shrink-0">
          <button onClick={() => openModelFolder(m.path)} className="p-0.5 text-blue-600 hover:bg-blue-50 dark:hover:bg-blue-900/30 rounded" title="打开目录">
            <FolderOpen className="w-3 h-3" />
          </button>
          <button onClick={() => handleDeleteFile(m.path)} className="p-0.5 text-red-600 hover:bg-red-50 dark:hover:bg-red-900/30 rounded" title="删除">
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

  const modelTypeLabel = (t: string) => {
    switch (t) { case 'mmproj': return '📷多模态投影器'; case 'imatrix': return '📊权重矩阵'; default: return '📄主模型' }
  }

  const handleScan = async () => {
    const err = await scanModels(modelDirs)
    if (err) setScanError(err)
    else setScanError('')
  }

  const handleAddDirectory = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const dir = await open({ directory: true, title: '选择包含 GGUF 模型的目录' })
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

  const handleRemoveDir = (dir: string) => {
    const next = modelDirs.filter(d => d !== dir)
    setModelDirs(next)
    scanModels(next)
  }

  const handleBrowseSaveDir = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const dir = await open({ directory: true, title: '选择下载保存目录' })
      if (dir) setMsSaveDir(dir as string)
    } catch (_) {}
  }

  const handleDeleteFile = async (path: string) => {
    const name = path.split('\\').pop() || path
    if (!confirm(`确定要删除 ${name} 吗？此操作不可撤销！`)) return
    await deleteModelFile(path)
  }

  const handleMsBrowse = async () => {
    if (!msRepoId.trim()) { setMsStatus('请输入仓库 ID'); return }
    setMsBrowsing(true)
    setMsStatus('查询中...')
    try {
      const files = await browseModelscope(msRepoId.trim())
      setMsFiles(files)
      setMsSelected(new Set())
      if (files.length === 0) setMsStatus('未找到 GGUF 文件')
      else setMsStatus(`找到 ${files.length} 个文件`)
    } catch (e: any) {
      setMsStatus(`查询失败：${typeof e === 'string' ? e : '网络错误'}`)
    } finally { setMsBrowsing(false) }
  }

  const handleMsDownload = async () => {
    if (msSelected.size === 0) return
    setMsDownloading(true)
    setMsCancelled(false)
    const selected = msFiles.filter(f => msSelected.has(f.path))
    const init: Record<string, any> = {}
    selected.forEach(f => { init[f.name] = {downloaded: 0, total: f.size, status: 'pending' as const} })
    setFileStates(init)
    try {
      await downloadModelscopeFiles(msRepoId, selected, msSaveDir)
      await scanModels([msSaveDir])
    } catch (e: any) {
      console.error('download error:', e)
    } finally {
      setMsDownloading(false)
    }
  }

  const toggleMsFile = (path: string) => {
    const next = new Set(msSelected)
    if (next.has(path)) { next.delete(path) } else { next.add(path) }
    setMsSelected(next)
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-4">
          <div className="relative">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400" />
            <input type="text" placeholder="搜索模型..." value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="pl-10 pr-4 py-2 rounded-lg border dark:border-gray-700 bg-white dark:bg-gray-800 w-80" />
          </div>
          <button onClick={handleScan} disabled={isLoading}
            className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-400 text-white rounded-lg transition-colors">
            <RefreshCw className={`w-4 h-4 ${isLoading ? 'animate-spin' : ''}`} /> 扫描模型
          </button>
          <button onClick={handleAddDirectory}
            className="flex items-center gap-2 px-4 py-2 bg-green-600 hover:bg-green-700 text-white rounded-lg transition-colors">
            <FolderOpen className="w-4 h-4" /> 添加模型目录
          </button>
        </div>
        <button onClick={() => setShowMsModal(true)}
          className="flex items-center gap-2 px-4 py-2 bg-purple-600 hover:bg-purple-700 text-white rounded-lg transition-colors">
          <Download className="w-4 h-4" /> 从 ModelScope 下载
        </button>
      </div>

      {/* 模型目录列表 */}
      {modelDirs.length > 0 && (
        <div className="bg-gray-50 dark:bg-gray-800/50 rounded-lg p-3 text-sm">
          <div className="flex items-center gap-2 mb-2 text-gray-500 font-medium">已添加的模型目录：</div>
          {modelDirs.map(d => (
            <div key={d} className="flex items-center justify-between py-1 px-2 rounded hover:bg-gray-100 dark:hover:bg-gray-700">
              <span className="text-xs truncate flex-1 mr-2">{d}</span>
              <button onClick={() => handleRemoveDir(d)}
                className="text-xs text-red-500 hover:text-red-700 px-2 py-0.5 rounded hover:bg-red-50 dark:hover:bg-red-900/20">
                移除
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
          <p className="text-gray-500 dark:text-gray-400">暂无模型，请点击「扫描模型」或「添加模型目录」</p>
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
              <h3 className="text-lg font-semibold">ModelScope 模型下载</h3>
              <button onClick={() => setShowMsModal(false)} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded"><X className="w-5 h-5" /></button>
            </div>

            <div className="space-y-4">
              <div className="flex gap-2">
                <input type="text" value={msRepoId} onChange={(e) => setMsRepoId(e.target.value)}
                  onKeyDown={(e) => e.key === 'Enter' && handleMsBrowse()}
                  placeholder="仓库 ID，例如 unsloth/Qwen3.6-35B-A3B-GGUF"
                  className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
                <button onClick={handleMsBrowse} disabled={msBrowsing}
                  className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-400 text-white rounded-lg transition-colors">
                  浏览文件
                </button>
              </div>

              {msStatus && <p className="text-sm text-gray-500">{msStatus}</p>}

              {msFiles.length > 0 && (
                <>
                  <div>
                    <label className="block text-sm font-medium mb-1">保存目录</label>
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
                      if (st) {
                        return (
                          <div key={f.path} className="px-4 py-2.5 border-b dark:border-gray-700 last:border-0 space-y-1">
                            <div className="flex items-center justify-between text-xs">
                              <span className="text-sm truncate flex-1 mr-2">{f.name}</span>
                              <span className={
                                st.status === 'done' ? 'text-green-500' :
                                st.status === 'error' ? 'text-red-500' :
                                st.status === 'downloading' ? 'text-blue-500' :
                                st.status === 'cancelled' ? 'text-yellow-500' :
                                'text-gray-400'
                              }>
                                {st.status === 'done' ? '✓完成' : st.status === 'error' ? '✗失败' : st.status === 'downloading' ? formatSize(st.downloaded) : st.status === 'cancelled' ? '已取消' : '等待'}
                              </span>
                            </div>
                            {st.status === 'downloading' && (
                              <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-1.5">
                                <div className="bg-blue-600 h-1.5 rounded-full" style={{width: `${st.total>0?(st.downloaded/st.total)*100:0}%`}} />
                              </div>
                            )}
                            {st.status === 'downloading' && (
                              <button onClick={() => cancelFileDownload(f.name)}
                                className="text-xs text-red-500 hover:text-red-700">✕ 取消</button>
                            )}
                            {st.error && <p className="text-xs text-red-500">{st.error}</p>}
                          </div>
                        )
                      }
                      return (
                        <label key={f.path} className="flex items-center gap-3 px-4 py-2.5 hover:bg-gray-50 dark:hover:bg-gray-700 cursor-pointer border-b dark:border-gray-700 last:border-0">
                          <input type="checkbox" checked={msSelected.has(f.path)} onChange={() => toggleMsFile(f.path)}
                            className="w-4 h-4 rounded" disabled={msDownloading} />
                          <div className="flex-1 min-w-0">
                            <div className="text-sm truncate">{f.name}</div>
                            <div className="text-xs text-gray-500">{modelTypeLabel(f.file_type)} · {formatSize(f.size)}</div>
                          </div>
                        </label>
                      )
                    })}
                  </div>

                  {msCancelled ? (
                    <div className="p-2 bg-yellow-50 dark:bg-yellow-900/20 rounded text-sm text-yellow-600">⚠ 部分文件已取消下载</div>
                  ) : (!msDownloading && Object.keys(fileStates).length > 0 && Object.values(fileStates).every(s => s.status === 'done')) && (
                    <div className="p-2 bg-green-50 dark:bg-green-900/20 rounded text-sm text-green-600">✅ 全部下载完成，文件保存在 {msSaveDir}</div>
                  )}

                  <button onClick={handleMsDownload} disabled={msSelected.size === 0 || msDownloading}
                    className="w-full py-2 bg-green-600 hover:bg-green-700 disabled:bg-gray-400 text-white rounded-lg transition-colors">
                    {msDownloading ? '下载中...' : `下载选中文件 (${msSelected.size})`}
                  </button>
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
