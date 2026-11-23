import { useEffect, useMemo, useState } from 'react'
import { AlertTriangle, Database, File, FolderOpen, HardDrive, Image, RefreshCw, Search, Trash2 } from 'lucide-react'
import { confirm, message } from '@tauri-apps/plugin-dialog'
import { useAppStore, type ModelDeletionPreview } from '../store'
import { buildModelTrees } from '../modelRepositoryTree'
import { ModelRepositoryTree } from './ModelRepositoryTree'
import { invokeApp as invoke } from '../lib/ipc'
import { formatMessage, useI18n } from '../i18n'
import { dedupePaths, pathsEqual } from '../utils/path'
import { formatSize } from '../utils/format'
import { Button, InsetSurface, MetricCard, PathText, Surface, TextInput } from './ui'

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
  const [scanError, setScanError] = useState('')
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const [selectedPath, setSelectedPath] = useState<string | null>(null)

  useEffect(() => {
    loadInitialData()
  }, [loadInitialData])

  useEffect(() => {
    if (!selectedPath && models.length > 0) {
      setSelectedPath(models[0].path)
      return
    }
    if (selectedPath && !models.some(model => pathsEqual(model.path, selectedPath))) {
      setSelectedPath(models[0]?.path ?? null)
    }
  }, [models, selectedPath])

  const trees = useMemo(() => buildModelTrees(modelDirs, models), [modelDirs, models])
  const selectedModel = useMemo(
    () => models.find(model => selectedPath && pathsEqual(model.path, selectedPath)) ?? null,
    [models, selectedPath],
  )
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

  return (
    <div className="space-y-5">
      <div className="mb-6 flex flex-col gap-5 xl:flex-row xl:items-end xl:justify-between">
        <div className="space-y-3">
          <div className="flex items-center gap-3">
            <div className="rounded-2xl border border-blue-500/20 bg-blue-500/10 p-3 text-ui-info">
              <Database className="h-5 w-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-3xl font-semibold tracking-tight text-ui-ink">{t.nav.modelRepo}</h1>
                <span className="rounded-full border border-ui-line bg-ui-surface px-2.5 py-1 text-xs text-ui-muted">
                  {formatMessage(copy.sourceCount, { count: modelDirs.length })}
                </span>
              </div>
              <p className="text-sm text-ui-muted">
                {copy.description}
              </p>
            </div>
          </div>
          {scanError && (
            <div className="flex items-start gap-3 rounded-2xl border border-amber-500/30 bg-amber-500/10 px-4 py-3 text-sm text-ui-warn">
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
              <span>{scanError}</span>
            </div>
          )}
        </div>

        <div className="flex flex-wrap items-center gap-3" data-guide="model-directories">
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

      <div className="mb-6 grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[
          { label: t.modelRepo.typeModelShort, value: stats.primaryCount, icon: File, tone: 'text-ui-info bg-sky-500/10 border-sky-500/20' },
          { label: t.modelRepo.typeMmprojShort, value: stats.projectorCount, icon: Image, tone: 'text-fuchsia-700 dark:text-fuchsia-300 bg-fuchsia-500/10 border-fuchsia-500/20' },
          { label: t.modelRepo.typeImatrix, value: stats.imatrixCount, icon: Database, tone: 'text-ui-warn bg-amber-500/10 border-amber-500/20' },
          { label: copy.capacity, value: formatSize(stats.totalSize), icon: HardDrive, tone: 'text-ui-positive bg-emerald-500/10 border-emerald-500/20' },
        ].map(card => (
          <MetricCard key={card.label} label={card.label} value={card.value} icon={<card.icon className="h-5 w-5" />} tone={card.tone} />
        ))}
      </div>

      <div className="grid gap-6 2xl:grid-cols-[280px,minmax(0,1.45fr),280px]">
        <Surface as="aside" className="min-w-0 p-5">
          <div className="mb-4 flex items-center justify-between">
            <div>
              <h2 className="text-lg font-semibold text-ui-ink">{copy.scanRoots}</h2>
              <p className="mt-1 text-sm text-ui-muted">
                {copy.scanRootsDescription}
              </p>
            </div>
            <span className="rounded-full border border-ui-line px-2.5 py-1 text-xs text-ui-muted">
              {modelDirs.length}
            </span>
          </div>

          <div className="space-y-3">
            {modelDirs.length === 0 ? (
              <div className="rounded-2xl border border-dashed border-ui-line px-4 py-8 text-center text-sm text-ui-muted">
                {t.modelRepo.noModels}
              </div>
            ) : (
              trees.map(tree => {
                const treeStats = tree.stats
                return (
                  <InsetSurface key={tree.path} className="p-4">
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <PathText value={tree.name} maxLength={38} className="text-sm font-medium text-ui-ink" />
                        <p className="mt-1 text-xs text-ui-muted">
                          {treeStats.models} {t.modelRepo.typeModelShort} · {treeStats.mmproj} {t.modelRepo.typeMmprojShort} · {formatSize(treeStats.size)}
                        </p>
                      </div>
                      <button
                        onClick={() => handleRemoveDir(tree.name)}
                        className="rounded-lg p-2 text-ui-muted transition hover:bg-red-500/10 hover:text-ui-danger"
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

        <Surface as="section" className="min-w-0 overflow-hidden" data-guide="model-search">
          <div className="flex flex-col gap-4 p-5 lg:flex-row lg:items-center lg:justify-between">
            <div>
              <h2 className="text-lg font-semibold text-ui-ink">{copy.explorer}</h2>
              <p className="mt-1 text-sm text-ui-muted">
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

          {models.length === 0 ? (
            <div className="flex min-h-52 flex-col items-center justify-center border-t border-ui-line p-5 text-center">
              <Database className="mb-4 h-12 w-12 text-ui-body" />
              <p className="text-base text-ui-body">{t.modelRepo.noModels}</p>
              <p className="mt-2 max-w-md text-sm text-ui-muted">
                {copy.emptyDescription}
              </p>
            </div>
          ) : (
            <ModelRepositoryTree roots={trees} query={searchQuery} collapsed={collapsed}
              onCollapsedChange={setCollapsed} selectedPath={selectedPath} onSelect={model => setSelectedPath(model.path)} />
          )}
        </Surface>

        <Surface as="aside" className="min-w-0 p-5">
          <div className="mb-4">
            <h2 className="text-lg font-semibold text-ui-ink">{copy.assetDetails}</h2>
            <p className="mt-1 text-sm text-ui-muted">
              {copy.assetDetailsDescription}
            </p>
          </div>

          {!selectedModel ? (
            <div className="flex min-h-[280px] flex-col items-center justify-center rounded-2xl border border-dashed border-ui-line text-center">
              <File className="mb-4 h-10 w-10 text-ui-body" />
              <p className="text-sm text-ui-muted">
                {copy.noAssetSelected}
              </p>
            </div>
          ) : (
            <div className="space-y-5">
              <InsetSurface className="p-4">
                <div className="flex items-start gap-3">
                  <div className={`rounded-2xl p-3 ${selectedModel.file_type === 'mmproj' ? 'bg-fuchsia-500/10 text-fuchsia-700 dark:text-fuchsia-300' : 'bg-sky-500/10 text-ui-info'}`}>
                    {selectedModel.file_type === 'mmproj' ? <Image className="h-5 w-5" /> : <File className="h-5 w-5" />}
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium text-ui-ink" title={selectedModel.name}>
                      {selectedModel.name}
                    </p>
                    <PathText value={selectedModel.path} maxLength={58} className="mt-1 text-ui-muted" />
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
                    <span className="text-sm text-ui-muted">{label}</span>
                    <span className="text-sm text-ui-body">{value}</span>
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
