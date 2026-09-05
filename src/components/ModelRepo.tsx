import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { AlertTriangle, Database, File, FolderOpen, FolderTree, HardDrive, Image, RefreshCw, Search, Trash2 } from 'lucide-react'
import { confirm, message } from '@tauri-apps/plugin-dialog'
import { useVirtualizer } from '@tanstack/react-virtual'
import { useAppStore, type ModelDeletionPreview, type ModelInfo } from '../store'
import { invokeApp as invoke } from '../lib/ipc'
import { formatMessage, useI18n } from '../i18n'
import { dedupePaths, formatPathForDisplay, isPathWithinRoot, normalizePath, pathJoin, pathsEqual } from '../utils/path'
import { formatSize } from '../utils/format'
import { Button, InsetSurface, MetricCard, PathText, SegmentedControl, Surface, TextInput } from './ui'

import { ModelAssetGrid } from './ModelRepo/ModelAssetGrid'

interface TreeStats {
  models: number
  mmproj: number
  imatrix: number
  size: number
}

interface TreeNode {
  name: string
  path: string
  isDir: boolean
  children?: Map<string, TreeNode>
  orderedChildren?: TreeNode[]
  model?: ModelInfo
  stats: TreeStats
}

const emptyTreeStats = (): TreeStats => ({ models: 0, mmproj: 0, imatrix: 0, size: 0 })

const modelStats = (model: ModelInfo): TreeStats => ({
  models: model.file_type === 'model' && !model.is_shard ? 1 : 0,
  mmproj: model.file_type === 'mmproj' ? 1 : 0,
  imatrix: model.file_type === 'imatrix' ? 1 : 0,
  size: model.size,
})

const finalizeTree = (node: TreeNode): TreeStats => {
  if (!node.isDir) return node.stats
  const orderedChildren = [...(node.children?.values() || [])].sort((left, right) => {
    if (left.isDir !== right.isDir) return left.isDir ? -1 : 1
    return left.name.localeCompare(right.name)
  })
  node.orderedChildren = orderedChildren
  const stats = emptyTreeStats()
  for (const child of orderedChildren) {
    const childStats = finalizeTree(child)
    stats.models += childStats.models
    stats.mmproj += childStats.mmproj
    stats.imatrix += childStats.imatrix
    stats.size += childStats.size
  }
  node.stats = stats
  return stats
}

const buildTree = (rootDir: string, models: ModelInfo[]): TreeNode => {
  const normalizedRoot = normalizePath(rootDir)
  const root: TreeNode = { name: rootDir, path: normalizedRoot, isDir: true, children: new Map(), stats: emptyTreeStats() }

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
        cursor.children!.set(part, { name: part, path: model.path, isDir: false, model, stats: modelStats(model) })
      } else {
        if (!cursor.children!.has(part)) {
          cursor.children!.set(part, {
            name: part,
            path: pathJoin(cursor.path, part),
            isDir: true,
            children: new Map(),
            stats: emptyTreeStats(),
          })
        }
        cursor = cursor.children!.get(part)!
      }
    }
  }

  finalizeTree(root)
  return root
}

const matchNode = (node: TreeNode, query: string): boolean => {
  if (!query) {
    return true
  }

  const normalizedQuery = query.toLowerCase()
  return (
    formatPathForDisplay(node.name).toLowerCase().includes(normalizedQuery) ||
    !!node.model?.quant_type?.toLowerCase().includes(normalizedQuery) ||
    !!node.model?.architecture?.toLowerCase().includes(normalizedQuery) ||
    !!node.model?.file_type?.toLowerCase().includes(normalizedQuery)
  )
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
      <mark className="rounded bg-blue-500/20 px-0.5 text-[var(--ui-link)]">
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
  const modelScanLoading = useAppStore(state => state.modelScanLoading)
  const loadInitialData = useAppStore(state => state.loadInitialData)
  const deleteModelFile = useAppStore(state => state.deleteModelFile)
  const openModelFolder = useAppStore(state => state.openModelFolder)
  const { t } = useI18n()
  const copy = t.modelRepoWorkspace

  const [searchQuery, setSearchQuery] = useState('')
  const [view, setView] = useState<'tree' | 'cards'>('tree')
  const [scanError, setScanError] = useState('')
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const [selectedPath, setSelectedPath] = useState<string | null>(null)
  const savedCollapsed = useRef<Set<string>>(new Set())
  const treeScrollRef = useRef<HTMLDivElement>(null)

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
    if (selectedPath && !models.some(model => pathsEqual(model.path, selectedPath))) {
      setSelectedPath(models[0]?.path ?? null)
    }
  }, [models, selectedPath])

  const trees = useMemo(() => modelDirs.map(dir => buildTree(dir, models)), [modelDirs, models])
  const cardModels = useMemo(() => models.filter(model => !searchQuery || matchNode({ name: model.name, path: model.path, isDir: false, model, stats: emptyTreeStats() }, searchQuery)), [models, searchQuery])
  const selectedModel = useMemo(
    () => models.find(model => selectedPath && pathsEqual(model.path, selectedPath)) ?? null,
    [models, selectedPath],
  )
  const matchingPaths = useMemo(() => {
    const paths = new Set<string>()
    if (!searchQuery) return paths
    const visit = (node: TreeNode): boolean => {
      const selfMatches = matchNode(node, searchQuery)
      let childMatches = false
      for (const child of node.orderedChildren || []) {
        childMatches = visit(child) || childMatches
      }
      if (selfMatches || childMatches) paths.add(node.path)
      return selfMatches || childMatches
    }
    trees.forEach(visit)
    return paths
  }, [searchQuery, trees])
  const flatNodes = useMemo(() => {
    const rows: { node: TreeNode; depth: number }[] = []
    const visit = (node: TreeNode, depth: number) => {
      if (searchQuery && !matchingPaths.has(node.path)) return
      rows.push({ node, depth })
      if (node.isDir && (!collapsed.has(node.path) || searchQuery)) {
        node.orderedChildren?.forEach(child => visit(child, depth + 1))
      }
    }
    trees.forEach(tree => visit(tree, 0))
    return rows
  }, [collapsed, matchingPaths, searchQuery, trees])
  const treeVirtualizer = useVirtualizer({
    count: flatNodes.length,
    getScrollElement: () => treeScrollRef.current,
    estimateSize: () => 41,
    overscan: 14,
    getItemKey: index => flatNodes[index]?.node.path || index,
  })

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
      const dir = await invoke<string | null>('pick_authorized_directory', { purpose: 'model' })
      if (!dir) {
        return
      }

      const nextDirs = dedupePaths([...modelDirs, dir])
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

    const nextDirs = modelDirs.filter(item => !pathsEqual(item, dir))
    setModelDirs(nextDirs)
    const error = await scanModels(nextDirs)
    setScanError(error ?? '')
  }

  const handleDeleteFile = async (path: string) => {
    setScanError('')
    try {
      const preview = await invoke<ModelDeletionPreview>('preview_model_deletion', { path })
      if (preview.referencedBy.length > 0) {
        await message(
          formatMessage(t.modelRepo.deleteBlocked, { instances: preview.referencedBy.join(', ') }),
          { title: t.modelRepo.deleteBlockedTitle, kind: 'warning' },
        )
        return
      }
      const confirmed = await confirm(
        formatMessage(t.modelRepo.deleteArtifactSetConfirm, {
          count: preview.artifactCount,
          size: formatSize(preview.totalBytes),
        }),
        { title: t.modelRepo.delete, kind: 'warning' },
      )
      if (!confirmed) return
      await deleteModelFile(path)
    } catch (error) {
      setScanError(String(error))
    }
  }

  const renderNode = (node: TreeNode, depth: number): ReactNode => {
    const nodeKey = node.path
    const displayName = formatPathForDisplay(node.name)
    const isCollapsed = collapsed.has(nodeKey)
    const isMatch = !!searchQuery && matchNode(node, searchQuery)
    const hasChildMatch = !!searchQuery && !isMatch && matchingPaths.has(node.path)
    const isVisible = !searchQuery || isMatch || hasChildMatch

    if (!isVisible) {
      return null
    }

    if (node.isDir) {
      const stats = node.stats
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
            <span className={`min-w-0 flex-1 truncate text-sm ${isMatch ? 'text-[var(--ui-link)]' : 'text-slate-100'}`}>
              {highlightText(displayName, searchQuery)}
            </span>
            <span className="shrink-0 text-xs text-slate-500">
              {stats.models > 0 ? `${stats.models} ${t.modelRepo.typeModelShort}` : ''}
              {stats.models > 0 && stats.mmproj > 0 ? ' · ' : ''}
              {stats.mmproj > 0 ? `${stats.mmproj} ${t.modelRepo.mmprojCount}` : ''}
              {(stats.models > 0 || stats.mmproj > 0) && stats.imatrix > 0 ? ' · ' : ''}
              {stats.imatrix > 0 ? `${stats.imatrix} ${t.modelRepo.typeImatrix}` : ''}
            </span>
          </button>
        </div>
      )
    }

    const model = node.model!
    const isSelected = Boolean(selectedPath && pathsEqual(selectedPath, model.path))
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
        <span className={`min-w-0 flex-1 truncate text-sm ${isSelected ? 'text-[var(--ui-link)]' : 'text-slate-100'}`}>
          {highlightText(model.name, searchQuery)}
        </span>
        <span className="ui-chip hidden shrink-0 lg:inline">{model.quant_type ?? ''}</span>
        <span className="hidden shrink-0 rounded-full border border-slate-700 px-2 py-0.5 text-[11px] text-slate-300 md:inline">
          {kindLabel}
        </span>
        <span className="shrink-0 text-xs text-slate-500">{formatSize(model.size)}</span>
      </button>
    )
  }

  return (
    <div className="space-y-5">
      <div className="mb-4 flex flex-col gap-5 xl:flex-row xl:items-end xl:justify-between">
        <div className="space-y-3">
          <div className="flex items-center gap-3">
            <div className="rounded-2xl border border-blue-500/20 bg-blue-500/10 p-3 text-blue-300">
              <Database className="h-5 w-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="ui-page-heading text-[var(--ui-text)]">{t.nav.modelRepo}</h1>
                <span className="rounded-full border border-slate-800 bg-slate-900 px-2.5 py-1 text-xs text-slate-400">
                  {formatMessage(copy.sourceCount, { count: modelDirs.length })}
                </span>
              </div>
              <p className="text-sm text-slate-400">
                {copy.description}
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
            disabled={modelScanLoading}
            icon={<RefreshCw className={`h-4 w-4 ${modelScanLoading ? 'animate-spin' : ''}`} />}
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

      <div className="mb-4 grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[
          { label: t.modelRepo.typeModelShort, value: stats.primaryCount, icon: File, tone: 'text-sky-300 bg-sky-500/10 border-sky-500/20' },
          { label: t.modelRepo.typeMmprojShort, value: stats.projectorCount, icon: Image, tone: 'text-fuchsia-300 bg-fuchsia-500/10 border-fuchsia-500/20' },
          { label: t.modelRepo.typeImatrix, value: stats.imatrixCount, icon: Database, tone: 'text-amber-300 bg-amber-500/10 border-amber-500/20' },
          { label: copy.capacity, value: formatSize(stats.totalSize), icon: HardDrive, tone: 'text-emerald-300 bg-emerald-500/10 border-emerald-500/20' },
        ].map(card => (
          <MetricCard key={card.label} label={card.label} value={card.value} icon={<card.icon className="h-5 w-5" />} tone={card.tone} />
        ))}
      </div>

      <div className="grid items-start gap-4 xl:grid-cols-[minmax(0,1fr)_280px] 2xl:grid-cols-[220px_minmax(0,1fr)_280px]">
        <Surface as="aside" className="p-4 xl:col-span-2 2xl:col-span-1">
          <div className="mb-4 flex items-center justify-between">
            <div>
              <h2 className="text-lg font-semibold text-slate-50">{copy.scanRoots}</h2>
              <p className="mt-1 text-sm text-slate-400">
                {copy.scanRootsDescription}
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
                const treeStats = tree.stats
                return (
                  <InsetSurface key={tree.path} className="p-4">
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <PathText value={tree.name} maxLength={38} className="text-sm font-medium text-slate-100" />
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

        <Surface as="section" className="min-w-0 p-4" data-guide="model-search">
          <div className="mb-5 flex flex-col gap-4 lg:flex-row lg:items-center lg:justify-between">
            <div>
              <h2 className="text-lg font-semibold text-slate-50">{copy.explorer}</h2>
              <p className="mt-1 text-sm text-slate-400">
                {copy.explorerDescription}
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

          <SegmentedControl className="mb-4" value={view} onChange={setView} options={[
            { value: 'tree', label: copy.treeView }, { value: 'cards', label: copy.cardView },
          ]} />
          {view === 'cards' ? <ModelAssetGrid models={cardModels} selectedPath={selectedPath} onSelect={setSelectedPath} /> : models.length === 0 ? (
            <div className="flex min-h-[420px] flex-col items-center justify-center rounded-2xl border border-dashed border-slate-800 text-center">
              <Database className="mb-4 h-12 w-12 text-slate-700" />
              <p className="text-base text-slate-300">{t.modelRepo.noModels}</p>
              <p className="mt-2 max-w-md text-sm text-slate-500">
                {copy.emptyDescription}
              </p>
            </div>
          ) : (
            <div
              ref={treeScrollRef}
              className="h-[520px] overflow-y-auto rounded-2xl border border-slate-800 bg-slate-950/40 p-3"
            >
              <div className="relative w-full" style={{ height: `${treeVirtualizer.getTotalSize()}px` }}>
                {treeVirtualizer.getVirtualItems().map(virtualRow => {
                  const row = flatNodes[virtualRow.index]
                  return (
                    <div
                      key={virtualRow.key}
                      ref={treeVirtualizer.measureElement}
                      data-index={virtualRow.index}
                      className="absolute left-0 top-0 w-full"
                      style={{ transform: `translateY(${virtualRow.start}px)` }}
                    >
                      {renderNode(row.node, row.depth)}
                    </div>
                  )
                })}
              </div>
            </div>
          )}
        </Surface>

        <Surface as="aside" className="p-5">
          <div className="mb-4">
            <h2 className="text-lg font-semibold text-slate-50">{copy.assetDetails}</h2>
            <p className="mt-1 text-sm text-slate-400">
              {copy.assetDetailsDescription}
            </p>
          </div>

          {!selectedModel ? (
            <div className="flex min-h-[280px] flex-col items-center justify-center rounded-2xl border border-dashed border-slate-800 text-center">
              <File className="mb-4 h-10 w-10 text-slate-700" />
              <p className="text-sm text-slate-400">
                {copy.noAssetSelected}
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
                  [copy.type, selectedModel.file_type],
                  [copy.quant, selectedModel.quant_type || '--'],
                  [copy.architecture, selectedModel.architecture || '--'],
                  [copy.size, formatSize(selectedModel.size)],
                  [copy.shard, selectedModel.is_shard ? copy.yes : copy.no],
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
