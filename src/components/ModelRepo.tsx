import { useState, useEffect, useRef, useMemo } from "react"
import { Search, FolderOpen, Trash2, RefreshCw, ChevronDown, ChevronRight, File, Image } from "lucide-react"
import { useAppStore, type ModelInfo } from "../store"
import { useI18n } from "../i18n"
import { confirm } from "@tauri-apps/plugin-dialog"
import { normalizePath, pathJoin } from "../utils/path"
import { formatSize } from "../utils/format"

const ModelRepo = () => {
  const models = useAppStore(s => s.models)
  const modelDirs = useAppStore(s => s.modelDirs)
  const setModelDirs = useAppStore(s => s.setModelDirs)
  const scanModels = useAppStore(s => s.scanModels)
  const isLoading = useAppStore(s => s.isLoading)
  const loadInitialData = useAppStore(s => s.loadInitialData)
  const deleteModelFile = useAppStore(s => s.deleteModelFile)
  const openModelFolder = useAppStore(s => s.openModelFolder)
  const { t } = useI18n()
  const [searchQuery, setSearchQuery] = useState("")
  const [scanError, setScanError] = useState("")
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const savedCollapsed = useRef<Set<string>>(new Set())

  // Auto-expand all on search, restore on clear
  useEffect(() => {
    if (searchQuery && savedCollapsed.current.size === 0 && collapsed.size > 0) {
      savedCollapsed.current = collapsed
      setCollapsed(new Set())
    } else if (!searchQuery && savedCollapsed.current.size > 0) {
      setCollapsed(savedCollapsed.current)
      savedCollapsed.current = new Set()
    }
  }, [searchQuery, collapsed])

  useEffect(() => { loadInitialData() }, [loadInitialData])

  interface TreeNode { name: string; path: string; isDir: boolean; children?: Map<string, TreeNode>; model?: typeof models[0] }
  function buildTree(rootDir: string, models: ModelInfo[]): TreeNode {
    const normDir = normalizePath(rootDir)
    const root: TreeNode = { name: rootDir, path: normDir, isDir: true, children: new Map() }
    const normRoot = normDir.toLowerCase()
    for (const m of models) {
      const normPath = normalizePath(m.path).toLowerCase()
      if (!normPath.startsWith(normRoot)) continue
      const rel = normalizePath(m.path.substring(rootDir.length)).replace(/^\/+/, "")
      const parts = rel.split("/")
      let cur = root
      for (let i = 0; i < parts.length; i++) {
        if (i === parts.length - 1) { cur.children!.set(parts[i], { name: parts[i], path: m.path, isDir: false, model: m }) }
        else {
          if (!cur.children!.has(parts[i])) cur.children!.set(parts[i], { name: parts[i], path: pathJoin(cur.path, parts[i]), isDir: true, children: new Map() })
          cur = cur.children!.get(parts[i])!
        }
      }
    }
    return root
  }
  const trees = useMemo(() => modelDirs.map(d => buildTree(d, models)), [modelDirs, models])
  function countDir(node: TreeNode): { models: number; mmproj: number; imatrix: number } {
    if (!node.isDir) {
      const ft = node.model!.file_type
      if (ft === "mmproj") return { models: 0, mmproj: 1, imatrix: 0 }
      if (ft === "imatrix") return { models: 0, mmproj: 0, imatrix: 1 }
      // Sharded model files: show in tree but don't count toward model total
      if (node.model!.is_shard) return { models: 0, mmproj: 0, imatrix: 0 }
      return { models: 1, mmproj: 0, imatrix: 0 }
    }
    let m = 0, p = 0, x = 0
    if (node.children) for (const child of node.children.values()) { const c = countDir(child); m += c.models; p += c.mmproj; x += c.imatrix }
    return { models: m, mmproj: p, imatrix: x }
  }
  const handleScan = async () => { const err = await scanModels(modelDirs); if (err) setScanError(err); else setScanError("") }
  const handleAddDirectory = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog")
      const dir = await open({ directory: true, title: t.modelRepo.addDir })
      if (dir) { const all = [...new Set([...modelDirs, dir as string])]; setModelDirs(all); const err = await scanModels(all); if (err) setScanError(err); else setScanError("") }
    } catch (_) { await scanModels(modelDirs) }
  }
  const handleRemoveDir = async (dir: string) => {
    if (!await confirm(t.modelRepo.removeDirConfirm, { title: t.modelRepo.remove, kind: "warning" })) return
    const next = modelDirs.filter(d => d !== dir); setModelDirs(next); scanModels(next)
  }
  const handleDeleteFile = async (path: string) => {
    if (!await confirm(t.modelRepo.deleteConfirm, { title: t.modelRepo.delete, kind: "warning" })) return
    await deleteModelFile(path)
  }

  // ── Search helpers ──
  const matchNode = (node: TreeNode, q: string): boolean => {
    if (!q) return true
    const lower = q.toLowerCase()
    if (node.name.toLowerCase().includes(lower)) return true
    if (node.model?.quant_type?.toLowerCase().includes(lower)) return true
    if (node.model?.architecture?.toLowerCase().includes(lower)) return true
    return false
  }
  const dirHasMatch = (node: TreeNode, q: string): boolean => {
    if (!node.isDir || !node.children) return false
    for (const child of node.children.values()) {
      if (child.isDir ? dirHasMatch(child, q) : matchNode(child, q)) return true
    }
    return false
  }
  const highlightText = (text: string, query: string): React.ReactNode => {
    if (!query) return text
    const idx = text.toLowerCase().indexOf(query.toLowerCase())
    if (idx < 0) return text
    return <>{text.slice(0, idx)}<mark className="bg-yellow-200 dark:bg-yellow-800/50 rounded-sm px-0.5">{text.slice(idx, idx + query.length)}</mark>{text.slice(idx + query.length)}</>
  }

  const renderNode = (node: TreeNode, depth: number) => {
    const nodeKey = node.path; const isCollapsed = collapsed.has(nodeKey)
    const hasSearch = !!searchQuery
    const isMatch = hasSearch && matchNode(node, searchQuery)
    const hasChildMatch = hasSearch && !isMatch && node.isDir && dirHasMatch(node, searchQuery)
    const isVisible = !hasSearch || isMatch || hasChildMatch
    // During search, expand all directories except user-toggled ones
    const isExpanded = hasSearch ? !isCollapsed : !isCollapsed

    if (!isVisible) return null

    if (node.isDir) {
      const cnt = countDir(node); const pl = depth * 16 + 16
      const rowOpacity = hasSearch && !isMatch && hasChildMatch ? 'opacity-80' : hasSearch && !isMatch ? 'opacity-40' : ''
      return (<div key={nodeKey}>
        <button onClick={() => { const next = new Set(collapsed); if (next.has(nodeKey)) next.delete(nodeKey); else next.add(nodeKey); setCollapsed(next) }}
          style={{ paddingLeft: pl + "px" }} className={`w-full flex items-center gap-2 py-1.5 hover:bg-gray-50 dark:hover:bg-gray-700 transition-colors text-left ${rowOpacity}`}>
          {isExpanded ? <ChevronDown className="w-3.5 h-3.5 text-gray-400 shrink-0" /> : <ChevronRight className="w-3.5 h-3.5 text-gray-400 shrink-0" />}
          {depth === 0 ? <FolderOpen className="w-3.5 h-3.5 text-yellow-500 shrink-0" /> : <span className="text-xs text-gray-400 w-3.5 shrink-0">{'\uD83D\uDCC1'}</span>}
          <span className={`text-xs font-medium truncate flex-1 ${isMatch ? 'text-amber-600 dark:text-amber-400' : ''}`}>{highlightText(node.name, searchQuery)}</span>
          <span className="text-xs text-gray-400 shrink-0">{cnt.models > 0 && cnt.models + " " + t.modelRepo.typeModelShort}{cnt.models > 0 && cnt.mmproj > 0 && " · "}{cnt.mmproj > 0 && cnt.mmproj + " " + t.modelRepo.mmprojCount}{(cnt.models > 0 || cnt.mmproj > 0) && cnt.imatrix > 0 && " · "}{cnt.imatrix > 0 && cnt.imatrix + " " + t.modelRepo.typeImatrix}</span>
        </button>
        {isExpanded && node.children && <div>{[...node.children.values()].sort((a,b) => { if (a.isDir !== b.isDir) return a.isDir ? -1 : 1; return a.name.localeCompare(b.name) }).map(c => renderNode(c, depth + 1)).filter(Boolean)}</div>}
      </div>)
    }
    const m = node.model!; const pl = depth * 16 + 16 + 20
    return (<div key={nodeKey} style={{ paddingLeft: pl + "px" }} className={`flex items-center gap-2 py-1.5 pr-4 hover:bg-gray-50 dark:hover:bg-gray-700 transition-colors ${m.is_shard ? 'opacity-60' : ''} ${hasSearch && !isMatch ? 'opacity-40' : ''}`}>
      {m.file_type === "mmproj" ? <Image className="w-3.5 h-3.5 text-purple-500 shrink-0" /> : <File className={`w-3.5 h-3.5 shrink-0 ${m.is_shard ? 'text-gray-400' : 'text-blue-500'}`} />}
      <span className={`text-xs truncate flex-1 ${isMatch ? 'text-amber-600 dark:text-amber-400 font-medium' : ''}`} title={m.name}>{highlightText(m.name, searchQuery)}</span>
      <span className={"text-xs px-1 py-0.5 rounded shrink-0 " + (m.file_type === "mmproj" ? "bg-purple-100 dark:bg-purple-900/30 text-purple-700 dark:text-purple-300" : m.file_type === "imatrix" ? "bg-amber-100 dark:bg-amber-900/30 text-amber-700 dark:text-amber-300" : "bg-blue-100 dark:bg-blue-900/30 text-blue-700")}>{m.file_type === "mmproj" ? t.modelRepo.typeMmprojShort : m.file_type === "imatrix" ? t.modelRepo.typeImatrix : t.modelRepo.typeModelShort}</span>
      {m.quant_type && <span className={`text-xs shrink-0 w-14 text-right ${isMatch ? 'text-amber-600 dark:text-amber-400' : 'text-gray-400'}`}>{m.quant_type}</span>}
      <span className="text-xs text-gray-400 shrink-0 w-20 text-right">{formatSize(m.size)}</span>
      <div className="flex items-center gap-0.5 shrink-0">
        <button onClick={() => openModelFolder(m.path)} className="p-0.5 text-blue-600 hover:bg-blue-50 dark:hover:bg-blue-900/30 rounded" title={t.modelRepo.openFolder}><FolderOpen className="w-3 h-3" /></button>
        <button onClick={() => handleDeleteFile(m.path)} className="p-0.5 text-red-600 hover:bg-red-50 dark:hover:bg-red-900/30 rounded" title={t.modelRepo.delete}><Trash2 className="w-3 h-3" /></button>
      </div>
    </div>)
  }
  return (<div className="flex-1 p-6 overflow-y-auto">
    <div className="flex items-center justify-between mb-6">
      <h2 className="text-2xl font-bold">{t.nav.modelRepo}</h2>
      <div className="flex items-center gap-2">
        <div className="relative"><Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400" /><input type="text" value={searchQuery} onChange={e => setSearchQuery(e.target.value)} placeholder={t.modelRepo.searchPlaceholder} className="pl-10 pr-4 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" data-guide="model-search" /></div>
        <button onClick={handleScan} disabled={isLoading} className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-400 text-white rounded-lg transition-colors"><RefreshCw className={"w-4 h-4 " + (isLoading ? "animate-spin" : "")} /> {t.modelRepo.scan}</button>
        <button onClick={handleAddDirectory} className="flex items-center gap-2 px-4 py-2 bg-green-600 hover:bg-green-700 text-white rounded-lg transition-colors"><FolderOpen className="w-4 h-4" /> {t.modelRepo.addDir}</button>
      </div>
    </div>
    {scanError && <div className="mb-4 p-3 bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-400 rounded-lg text-sm">{scanError}</div>}
    {models.length === 0 ? (<div className="text-center text-gray-500 py-12">{t.modelRepo.noModels}</div>) : (<div>
      <div className="flex items-center gap-2 mb-2 text-gray-500 font-medium">{t.modelRepo.modelDirs}</div>
      {modelDirs.map(d => (<div key={d} className="flex items-center justify-between py-1 px-2 rounded hover:bg-gray-100 dark:hover:bg-gray-700"><span className="text-xs truncate flex-1 mr-2">{d}</span><button onClick={() => handleRemoveDir(d)} className="p-0.5 text-red-400 hover:text-red-600 rounded" title={t.modelRepo.remove}><Trash2 className="w-3 h-3" /></button></div>))}
      <div className="border-t dark:border-gray-700 mt-2 pt-2">{trees.map(t => renderNode(t, 0))}</div>
    </div>)}
  </div>)
}
export default ModelRepo
