import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { AlertTriangle, Database, File, FolderOpen, FolderTree, HardDrive, Image, RefreshCw, Search, Trash2 } from 'lucide-react'
import { confirm, open } from '@tauri-apps/plugin-dialog'
import { useAppStore, type ModelInfo } from '../store'
import { useI18n } from '../i18n'
import { isPathWithinRoot, normalizePath, pathJoin } from '../utils/path'
import { formatSize } from '../utils/format'
import { Button, InsetSurface, MetricCard, PathText, Surface, TextInput } from './ui'

interface TreeNode {
  name: string
  path: string
  isDir: boolean
  children?: Map<string, TreeNode>
  model?: ModelInfo
}

const buildTree = (rootDir: string, models: ModelInfo[]): TreeNode => {
  const normalizedRoot = normalizePath(rootDir)
  const root: TreeNode = { name: rootDir, path: normalizedRoot, isDir: true, children: new Map() }

  for (const model of models) {
    const normalizedPath = normalizePath(model.path)
    if (!isPathWithinRoot(normalizedPath, normalizedRoot)) {
      continue
    }

    const relative = normalizedPath.slice(normalizedRoot.length).replace(/^\/+/, '')
    if (!relative) {
      continue
    }

    const parts = relative.split('/')
    let cursor = root

    for (let index = 0; index < parts.length; index += 1) {
      const part = parts[index]
      if (index === parts.length - 1) {
        cursor.children!.set(part, { name: part, path: model.path, isDir: false, model })
      } else {
        if (!cursor.children!.has(part)) {
          cursor.children!.set(part, {
            name: part,
            path: pathJoin(cursor.path, part),
            isDir: true,
            children: new Map(),
          })
        }
        cursor = cursor.children!.get(part)!
      }
    }
  }

  return root
}

const countTree = (node: TreeNode): { models: number; mmproj: number; imatrix: number; size: number } => {
  if (!node.isDir) {
    const model = node.model!
    if (model.file_type === 'mmproj') {
      return { models: 0, mmproj: 1, imatrix: 0, size: model.size }
    }
    if (model.file_type === 'imatrix') {
      return { models: 0, mmproj: 0, imatrix: 1, size: model.size }
    }
    if (model.is_shard) {
      return { models: 0, mmproj: 0, imatrix: 0, size: model.size }
    }
    return { models: 1, mmproj: 0, imatrix: 0, size: model.size }
  }

  let models = 0
  let mmproj = 0
  let imatrix = 0
  let size = 0

  if (node.children) {
    for (const child of node.children.values()) {
      const childStats = countTree(child)
      models += childStats.models
      mmproj += childStats.mmproj
      imatrix += childStats.imatrix
      size += childStats.size
    }
  }

  return { models, mmproj, imatrix, size }
}

const matchNode = (node: TreeNode, query: string): boolean => {
  if (!query) {
    return true
  }

  const normalizedQuery = query.toLowerCase()
  return (
    node.name.toLowerCase().includes(normalizedQuery) ||
    !!node.model?.quant_type?.toLowerCase().includes(normalizedQuery) ||
    !!node.model?.architecture?.toLowerCase().includes(normalizedQuery) ||
    !!node.model?.file_type?.toLowerCase().includes(normalizedQuery)
  )
}

const dirHasMatch = (node: TreeNode, query: string): boolean => {
  if (!node.isDir || !node.children) {
    return false
  }

  for (const child of node.children.values()) {
    if (child.isDir ? dirHasMatch(child, query) : matchNode(child, query)) {
      return true
    }
  }

  return false
}

const highlightText = (text: string, query: string): ReactNode => {
  if (!query) {
    return text
  }

  const index = text.toLowerCase().indexOf(query.toLowerCase())
  if (index < 0) {
    return text
  }

  return (
    <>
      {text.slice(0, index)}
      <mark className="rounded bg-blue-500/20 px-0.5 text-blue-100">
        {text.slice(index, index + query.length)}
      </mark>
      {text.slice(index + query.length)}
    </>
  )
}

const ModelRepo = () => {
  const models = useAppStore(state => state.models)
  const modelDirs = useAppStore(state => state.modelDirs)
  const setModelDirs = useAppStore(state => state.setModelDirs)
  const scanModels = useAppStore(state => state.scanModels)
  const isLoading = useAppStore(state => state.isLoading)
  const loadInitialData = useAppStore(state => state.loadInitialData)
  const deleteModelFile = useAppStore(state => state.deleteModelFile)
  const openModelFolder = useAppStore(state => state.openModelFolder)
  const { t, lang } = useI18n()

  const [searchQuery, setSearchQuery] = useState('')
  const [scanError, setScanError] = useState('')
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const [selectedPath, setSelectedPath] = useState<string | null>(null)
  const savedCollapsed = useRef<Set<string>>(new Set())

  useEffect(() => {
    loadInitialData()
  }, [loadInitialData])

  useEffect(() => {
    if (searchQuery && savedCollapsed.current.size === 0 && collapsed.size > 0) {
      savedCollapsed.current = collapsed
      setCollapsed(new Set())
    } else if (!searchQuery && savedCollapsed.current.size > 0) {
      setCollapsed(savedCollapsed.current)
      savedCollapsed.current = new Set()
    }
  }, [searchQuery, collapsed])

  useEffect(() => {
    if (!selectedPath && models.length > 0) {
      setSelectedPath(models[0].path)
      return
    }
    if (selectedPath && !models.some(model => model.path === selectedPath)) {
      setSelectedPath(models[0]?.path ?? null)
    }
  }, [models, selectedPath])

  const trees = useMemo(() => modelDirs.map(dir => buildTree(dir, models)), [modelDirs, models])
  const selectedModel = useMemo(() => models.find(model => model.path === selectedPath) ?? null, [models, selectedPath])

  const stats = useMemo(() => {
    const primaryModels = models.filter(model => model.file_type !== 'mmproj' && model.file_type !== 'imatrix' && !model.is_shard)
    const projectorModels = models.filter(model => model.file_type === 'mmproj')
    const matrices = models.filter(model => model.file_type === 'imatrix')
    const totalSize = models.reduce((sum, model) => sum + model.size, 0)

    return {
      primaryCount: primaryModels.length,
      projectorCount: projectorModels.length,
      imatrixCount: matrices.length,
      totalSize,
    }
  }, [models])

  const handleScan = async () => {
    const error = await scanModels(modelDirs)
    setScanError(error ?? '')
  }

  const handleAddDirectory = async () => {
    try {
      const dir = await open({ directory: true, title: t.modelRepo.addDir })
      if (!dir) {
        return
      }

      const nextDirs = [...new Set([...modelDirs, dir as string])]
      setModelDirs(nextDirs)
      const error = await scanModels(nextDirs)
      setScanError(error ?? '')
    } catch {
      const error = await scanModels(modelDirs)
      setScanError(error ?? '')
    }
  }

  const handleRemoveDir = async (dir: string) => {
    const confirmed = await confirm(t.modelRepo.removeDirConfirm, { title: t.modelRepo.remove, kind: 'warning' })
    if (!confirmed) {
      return
    }

    const nextDirs = modelDirs.filter(item => item !== dir)
    setModelDirs(nextDirs)
    const error = await scanModels(nextDirs)
    setScanError(error ?? '')
  }

  const handleDeleteFile = async (path: string) => {
    const confirmed = await confirm(t.modelRepo.deleteConfirm, { title: t.modelRepo.delete, kind: 'warning' })
    if (!confirmed) {
      return
    }

    await deleteModelFile(path)
  }

  const renderNode = (node: TreeNode, depth: number): ReactNode => {
    const nodeKey = node.path
    const isCollapsed = collapsed.has(nodeKey)
    const isMatch = !!searchQuery && matchNode(node, searchQuery)
    const hasChildMatch = !!searchQuery && !isMatch && node.isDir && dirHasMatch(node, searchQuery)
    const isVisible = !searchQuery || isMatch || hasChildMatch

    if (!isVisible) {
      return null
    }

    if (node.isDir) {
      const stats = countTree(node)
      return (
        <div key={nodeKey}>
          <button
            onClick={() => {
              const next = new Set(collapsed)
              if (next.has(nodeKey)) {
                next.delete(nodeKey)
              } else {
                next.add(nodeKey)
              }
              setCollapsed(next)
            }}
            style={{ paddingLeft: `${depth * 18 + 14}px` }}
            className={`flex w-full items-center gap-2 rounded-xl py-2 pr-3 text-left transition hover:bg-slate-800/80 ${
              !isMatch && hasChildMatch ? 'opacity-80' : ''
            }`}
          >
            <span className="w-4 shrink-0 text-slate-500">{isCollapsed ? '>' : 'v'}</span>
            <FolderTree className="h-4 w-4 shrink-0 text-amber-400" />
            <span className={`min-w-0 flex-1 truncate text-sm ${isMatch ? 'text-blue-100' : 'text-slate-100'}`}>
              {highlightText(node.name, searchQuery)}
            </span>
            <span className="shrink-0 text-xs text-slate-500">
              {stats.models > 0 ? `${stats.models} ${t.modelRepo.typeModelShort}` : ''}
              {stats.models > 0 && stats.mmproj > 0 ? ' · ' : ''}
              {stats.mmproj > 0 ? `${stats.mmproj} ${t.modelRepo.mmprojCount}` : ''}
              {(stats.models > 0 || stats.mmproj > 0) && stats.imatrix > 0 ? ' · ' : ''}
              {stats.imatrix > 0 ? `${stats.imatrix} ${t.modelRepo.typeImatrix}` : ''}
            </span>
          </button>
          {!isCollapsed && node.children && (
            <div>
              {[...node.children.values()]
                .sort((left, right) => {
                  if (left.isDir !== right.isDir) {
                    return left.isDir ? -1 : 1
                  }
                  return left.name.localeCompare(right.name)
                })
                .map(child => renderNode(child, depth + 1))}
            </div>
          )}
        </div>
      )
    }

    const model = node.model!
    const isSelected = selectedPath === model.path
    const kindLabel = model.file_type === 'mmproj'
      ? t.modelRepo.typeMmprojShort
      : model.file_type === 'imatrix'
        ? t.modelRepo.typeImatrix
        : t.modelRepo.typeModelShort

    return (
      <button
        key={nodeKey}
        onClick={() => setSelectedPath(model.path)}
        style={{ paddingLeft: `${depth * 18 + 34}px` }}
        className={`flex w-full items-center gap-2 rounded-xl py-2 pr-3 text-left transition ${
          isSelected ? 'bg-blue-500/12 ring-1 ring-blue-500/40' : 'hover:bg-slate-800/80'
        } ${model.is_shard ? 'opacity-60' : ''}`}
      >
        {model.file_type === 'mmproj' ? (
          <Image className="h-4 w-4 shrink-0 text-fuchsia-400" />
        ) : (
          <File className="h-4 w-4 shrink-0 text-sky-400" />
        )}
        <span className={`min-w-0 flex-1 truncate text-sm ${isSelected ? 'text-blue-100' : 'text-slate-100'}`}>
          {highlightText(model.name, searchQuery)}
        </span>
        <span className="hidden shrink-0 text-xs text-slate-500 lg:inline">{model.quant_type ?? ''}</span>
        <span className="hidden shrink-0 rounded-full border border-slate-700 px-2 py-0.5 text-[11px] text-slate-300 md:inline">
          {kindLabel}
        </span>
        <span className="shrink-0 text-xs text-slate-500">{formatSize(model.size)}</span>
      </button>
    )
  }

  return (
    <div className="space-y-5">
      <div className="mb-6 flex flex-col gap-5 xl:flex-row xl:items-end xl:justify-between">
        <div className="space-y-3">
          <div className="flex items-center gap-3">
            <div className="rounded-2xl border border-blue-500/20 bg-blue-500/10 p-3 text-blue-300">
              <Database className="h-5 w-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-3xl font-semibold tracking-tight text-slate-50">{t.nav.modelRepo}</h1>
                <span className="rounded-full border border-slate-800 bg-slate-900 px-2.5 py-1 text-xs text-slate-400">
                  {modelDirs.length} {lang === 'zh-CN' ? '\u4E2A\u6765\u6E90' : `source${modelDirs.length === 1 ? '' : 's'}`}
                </span>
              </div>
              <p className="text-sm text-slate-400">
                {lang === 'zh-CN'
                  ? '\u7BA1\u7406\u672C\u5730\u6A21\u578B\u8D44\u4EA7\uFF0C\u4FDD\u6301\u626B\u63CF\u6839\u76EE\u5F55\u6E05\u6670\uFF0C\u5E76\u4EE5\u7EDF\u4E00\u89C6\u56FE\u68C0\u89C6\u4ED3\u5E93\u3002'
                  : 'Curate local model assets, keep scan roots clean, and inspect the repo as one operational surface.'}
              </p>
            </div>
          </div>
          {scanError && (
            <div className="flex items-start gap-3 rounded-2xl border border-amber-500/30 bg-amber-500/10 px-4 py-3 text-sm text-amber-200">
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
              <span>{scanError}</span>
            </div>
          )}
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <Button
            onClick={handleScan}
            disabled={isLoading}
            icon={<RefreshCw className={`h-4 w-4 ${isLoading ? 'animate-spin' : ''}`} />}
          >
            {t.modelRepo.scan}
          </Button>
          <Button
            onClick={handleAddDirectory}
            variant="primary"
            icon={<FolderOpen className="h-4 w-4" />}
          >
            {t.modelRepo.addDir}
          </Button>
        </div>
      </div>

      <div className="mb-6 grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[
          { label: t.modelRepo.typeModelShort, value: stats.primaryCount, icon: File, tone: 'text-sky-300 bg-sky-500/10 border-sky-500/20' },
          { label: t.modelRepo.typeMmprojShort, value: stats.projectorCount, icon: Image, tone: 'text-fuchsia-300 bg-fuchsia-500/10 border-fuchsia-500/20' },
          { label: t.modelRepo.typeImatrix, value: stats.imatrixCount, icon: Database, tone: 'text-amber-300 bg-amber-500/10 border-amber-500/20' },
          { label: lang === 'zh-CN' ? '\u5BB9\u91CF' : 'Capacity', value: formatSize(stats.totalSize), icon: HardDrive, tone: 'text-emerald-300 bg-emerald-500/10 border-emerald-500/20' },
        ].map(card => (
          <MetricCard key={card.label} label={card.label} value={card.value} icon={<card.icon className="h-5 w-5" />} tone={card.tone} />
        ))}
      </div>

      <div className="grid gap-6 2xl:grid-cols-[280px,minmax(0,1.45fr),280px]">
        <Surface as="aside" className="p-5">
          <div className="mb-4 flex items-center justify-between">
            <div>
              <h2 className="text-lg font-semibold text-slate-50">{lang === 'zh-CN' ? '\u626B\u63CF\u6839\u76EE\u5F55' : 'Scan Roots'}</h2>
              <p className="mt-1 text-sm text-slate-400">
                {lang === 'zh-CN' ? '\u7EB3\u5165\u4ED3\u5E93\u626B\u63CF\u7684\u672C\u5730\u76EE\u5F55\u3002' : 'Local directories included in repository scans.'}
              </p>
            </div>
            <span className="rounded-full border border-slate-700 px-2.5 py-1 text-xs text-slate-400">
              {modelDirs.length}
            </span>
          </div>

          <div className="space-y-3">
            {modelDirs.length === 0 ? (
              <div className="rounded-2xl border border-dashed border-slate-700 px-4 py-8 text-center text-sm text-slate-500">
                {t.modelRepo.noModels}
              </div>
            ) : (
              trees.map(tree => {
                const treeStats = countTree(tree)
                return (
                  <InsetSurface key={tree.path} className="p-4">
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <p className="truncate text-sm font-medium text-slate-100" title={tree.name}>
                          {tree.name}
                        </p>
                        <p className="mt-1 text-xs text-slate-500">
                          {treeStats.models} {t.modelRepo.typeModelShort} · {treeStats.mmproj} {t.modelRepo.typeMmprojShort} · {formatSize(treeStats.size)}
                        </p>
                      </div>
                      <button
                        onClick={() => handleRemoveDir(tree.name)}
                        className="rounded-lg p-2 text-slate-500 transition hover:bg-red-500/10 hover:text-red-300"
                        title={t.modelRepo.remove}
                      >
                        <Trash2 className="h-4 w-4" />
                      </button>
                    </div>
                  </InsetSurface>
                )
              })
            )}
          </div>
        </Surface>

        <Surface as="section" className="min-h-[620px] p-5" data-guide="model-search">
          <div className="mb-5 flex flex-col gap-4 lg:flex-row lg:items-center lg:justify-between">
            <div>
              <h2 className="text-lg font-semibold text-slate-50">{lang === 'zh-CN' ? '\u4ED3\u5E93\u6D4F\u89C8\u5668' : 'Repository Explorer'}</h2>
              <p className="mt-1 text-sm text-slate-400">
                {lang === 'zh-CN' ? '\u6309\u6587\u4EF6\u540D\u3001\u67B6\u6784\u3001\u91CF\u5316\u6216\u8D44\u4EA7\u7C7B\u578B\u641C\u7D22\u3002' : 'Search by file name, architecture, quantization, or artifact type.'}
              </p>
            </div>
            <TextInput
              value={searchQuery}
              onChange={event => setSearchQuery(event.target.value)}
              placeholder={t.modelRepo.searchPlaceholder}
              leadingIcon={<Search className="h-4 w-4" />}
              className="w-full max-w-md"
            />
          </div>

          {models.length === 0 ? (
            <div className="flex min-h-[420px] flex-col items-center justify-center rounded-2xl border border-dashed border-slate-800 text-center">
              <Database className="mb-4 h-12 w-12 text-slate-700" />
              <p className="text-base text-slate-300">{t.modelRepo.noModels}</p>
              <p className="mt-2 max-w-md text-sm text-slate-500">
                {lang === 'zh-CN' ? '\u6DFB\u52A0\u4E00\u4E2A\u6216\u591A\u4E2A\u6A21\u578B\u76EE\u5F55\uFF0C\u7136\u540E\u626B\u63CF\u4EE5\u586B\u5145\u4ED3\u5E93\u6D4F\u89C8\u5668\u3002' : 'Add one or more model directories, then run a scan to populate the repository explorer.'}
              </p>
            </div>
          ) : (
            <div className="space-y-1 rounded-2xl border border-slate-800 bg-slate-950/40 p-3">
              {trees.map(tree => renderNode(tree, 0))}
            </div>
          )}
        </Surface>

        <Surface as="aside" className="p-5">
          <div className="mb-4">
            <h2 className="text-lg font-semibold text-slate-50">{lang === 'zh-CN' ? '\u8D44\u4EA7\u8BE6\u60C5' : 'Asset Details'}</h2>
            <p className="mt-1 text-sm text-slate-400">
              {lang === 'zh-CN' ? '\u67E5\u770B\u5F53\u524D\u9009\u4E2D\u6587\u4EF6\uFF0C\u5E76\u5FEB\u901F\u6253\u5F00\u5176\u6240\u5728\u4F4D\u7F6E\u3002' : 'Inspect the currently selected file and jump to its location.'}
            </p>
          </div>

          {!selectedModel ? (
            <div className="flex min-h-[280px] flex-col items-center justify-center rounded-2xl border border-dashed border-slate-800 text-center">
              <File className="mb-4 h-10 w-10 text-slate-700" />
              <p className="text-sm text-slate-400">
                {lang === 'zh-CN' ? '\u4ECE\u6D4F\u89C8\u5668\u9009\u62E9\u4E00\u4E2A\u6A21\u578B\u8D44\u4EA7\u540E\u53EF\u5728\u8FD9\u91CC\u67E5\u770B\u8BE6\u60C5\u3002' : 'Select a model artifact from the explorer to inspect it here.'}
              </p>
            </div>
          ) : (
            <div className="space-y-5">
              <InsetSurface className="p-4">
                <div className="flex items-start gap-3">
                  <div className={`rounded-2xl p-3 ${selectedModel.file_type === 'mmproj' ? 'bg-fuchsia-500/10 text-fuchsia-300' : 'bg-sky-500/10 text-sky-300'}`}>
                    {selectedModel.file_type === 'mmproj' ? <Image className="h-5 w-5" /> : <File className="h-5 w-5" />}
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium text-slate-100" title={selectedModel.name}>
                      {selectedModel.name}
                    </p>
                    <PathText value={selectedModel.path} maxLength={58} className="mt-1 text-slate-500" />
                  </div>
                </div>
              </InsetSurface>

              <InsetSurface className="space-y-3 p-4">
                {[
                  [lang === 'zh-CN' ? '\u7C7B\u578B' : 'Type', selectedModel.file_type],
                  [lang === 'zh-CN' ? '\u91CF\u5316' : 'Quant', selectedModel.quant_type || '--'],
                  [lang === 'zh-CN' ? '\u67B6\u6784' : 'Architecture', selectedModel.architecture || '--'],
                  [lang === 'zh-CN' ? '\u5927\u5C0F' : 'Size', formatSize(selectedModel.size)],
                  [lang === 'zh-CN' ? '\u5206\u7247' : 'Shard', selectedModel.is_shard ? (lang === 'zh-CN' ? '\u662F' : 'Yes') : (lang === 'zh-CN' ? '\u5426' : 'No')],
                ].map(([label, value]) => (
                  <div key={label} className="flex items-center justify-between gap-3">
                    <span className="text-sm text-slate-500">{label}</span>
                    <span className="text-sm text-slate-200">{value}</span>
                  </div>
                ))}
              </InsetSurface>

              <div className="grid gap-3">
                <Button
                  onClick={() => openModelFolder(selectedModel.path)}
                  icon={<FolderOpen className="h-4 w-4" />}
                >
                  {t.modelRepo.openFolder}
                </Button>
                <Button
                  onClick={() => handleDeleteFile(selectedModel.path)}
                  variant="danger"
                  icon={<Trash2 className="h-4 w-4" />}
                >
                  {t.modelRepo.delete}
                </Button>
              </div>
            </div>
          )}
        </Surface>
      </div>
    </div>
  )
}

export default ModelRepo
