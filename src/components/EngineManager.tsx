import { useEffect, useMemo, useRef, useState } from 'react'
import { Cpu, FolderOpen, LoaderCircle, Pencil, Plus, RefreshCw, Search, ShieldCheck, Star, Trash2 } from 'lucide-react'
import { confirm, message, open } from '@tauri-apps/plugin-dialog'
import { useAppStore, type EngineInfo } from '../store'
import { formatMessage, useI18n } from '../i18n'
import { getEngineLabels } from '../i18n/pageLabels'
import { normalizeEngineCapabilityStatus, normalizeEngineVersionStatus } from '../engineCapabilities'
import { isPathWithinRoot } from '../utils/path'
import { Button, InsetSurface, MetricCard, PathText, SelectInput, Surface, TextInput } from './ui'

const backendTone = (backend: string) => {
  switch (backend) {
    case 'CUDA':
      return 'border-emerald-500/20 bg-emerald-500/10 text-emerald-300'
    case 'ROCm':
      return 'border-rose-500/20 bg-rose-500/10 text-rose-300'
    case 'Vulkan':
      return 'border-violet-500/20 bg-violet-500/10 text-violet-300'
    default:
      return 'border-slate-700 bg-slate-800 text-slate-300'
  }
}

const capabilityTone = (status: string) => {
  if (status === 'detected') return 'border-emerald-500/20 bg-emerald-500/10 text-emerald-300'
  if (status === 'unprobed') return 'border-slate-700 bg-slate-800 text-slate-300'
  return 'border-amber-500/20 bg-amber-500/10 text-amber-300'
}

const EngineManager = () => {
  const engines = useAppStore(state => state.engines)
  const scanEngines = useAppStore(state => state.scanEngines)
  const probeEngineCapabilities = useAppStore(state => state.probeEngineCapabilities)
  const addRuntimeWarning = useAppStore(state => state.addRuntimeWarning)
  const loadInitialData = useAppStore(state => state.loadInitialData)
  const isLoading = useAppStore(state => state.isLoading)
  const deleteEngine = useAppStore(state => state.deleteEngine)
  const renameEngine = useAppStore(state => state.renameEngine)
  const openEngineFolder = useAppStore(state => state.openEngineFolder)
  const defaultEngineId = useAppStore(state => state.defaultEngineId)
  const setDefaultEngineId = useAppStore(state => state.setDefaultEngineId)
  const engineDirs = useAppStore(state => state.engineDirs)
  const setEngineDirs = useAppStore(state => state.setEngineDirs)
  const { t, lang } = useI18n()

  const [editingId, setEditingId] = useState<string | null>(null)
  const [editName, setEditName] = useState('')
  const [selectedEngineId, setSelectedEngineId] = useState<string | null>(null)
  const [searchQuery, setSearchQuery] = useState('')
  const [backendFilter, setBackendFilter] = useState('all')
  const [probingEngineId, setProbingEngineId] = useState<string | null>(null)
  const editingCanceledRef = useRef(false)

  const labels = useMemo(() => getEngineLabels(lang), [lang])

  useEffect(() => {
    loadInitialData()
  }, [loadInitialData])

  useEffect(() => {
    if (!selectedEngineId && engines.length > 0) {
      setSelectedEngineId(defaultEngineId ?? engines[0].id)
      return
    }
    if (selectedEngineId && !engines.some(engine => engine.id === selectedEngineId)) {
      setSelectedEngineId(defaultEngineId ?? engines[0]?.id ?? null)
    }
  }, [defaultEngineId, engines, selectedEngineId])

  const backendCounts = useMemo(() => {
    const counts = new Map<string, number>()
    for (const engine of engines) {
      counts.set(engine.backend, (counts.get(engine.backend) ?? 0) + 1)
    }
    return counts
  }, [engines])

  const filteredEngines = useMemo(() => {
    const query = searchQuery.trim().toLowerCase()
    return engines.filter(engine => {
      const matchesBackend = backendFilter === 'all' || engine.backend === backendFilter
      const matchesQuery =
        query.length === 0 ||
        engine.name.toLowerCase().includes(query) ||
        engine.version.toLowerCase().includes(query) ||
        engine.backend.toLowerCase().includes(query) ||
        engine.dir.toLowerCase().includes(query)

      return matchesBackend && matchesQuery
    })
  }, [backendFilter, engines, searchQuery])

  const selectedEngine = useMemo(
    () => engines.find(engine => engine.id === selectedEngineId) ?? null,
    [engines, selectedEngineId],
  )

  const handleScan = async () => {
    await scanEngines(engineDirs)
  }

  const handleAddDirectory = async () => {
    try {
      const dir = await open({ directory: true, title: t.engineMgr.addDirTitle })
      if (!dir) return

      const nextDirs = [...new Set([...engineDirs, dir as string])]
      setEngineDirs(nextDirs)
      await scanEngines(nextDirs)
    } catch {
      await scanEngines(engineDirs)
    }
  }

  const handleRemoveDir = async (dir: string) => {
    const removedEngineIds = new Set(engines.filter(engine => isPathWithinRoot(engine.dir, dir)).map(engine => engine.id))
    const referenced = useAppStore.getState().instances.filter(instance => removedEngineIds.has(instance.config.engine_id))
    if (referenced.length > 0) {
      await message(formatMessage(t.engineMgr.removeDirInUse, { count: referenced.length }), { title: t.engineMgr.removeDirInUseTitle, kind: 'warning' })
      return
    }
    const confirmed = await confirm(t.engineMgr.removeDirConfirm, { title: t.engineMgr.remove, kind: 'warning' })
    if (!confirmed) return

    const nextDirs = engineDirs.filter(item => item !== dir)
    setEngineDirs(nextDirs)
    await scanEngines(nextDirs)
  }

  const handleDelete = async (id: string) => {
    const engine = engines.find(item => item.id === id)
    if (!engine) return

    const confirmed = await confirm(t.engineMgr.removeConfirm, { title: t.engineMgr.remove, kind: 'warning' })
    if (!confirmed) return

    await deleteEngine(id)
    if (defaultEngineId === id) {
      setDefaultEngineId(null)
    }
  }

  const commitRename = (id: string) => {
    const trimmed = editName.trim()
    if (trimmed) {
      renameEngine(id, trimmed)
    }
    setEditingId(null)
  }

  const handleProbe = async (id: string) => {
    if (probingEngineId) return
    setProbingEngineId(id)
    try {
      await probeEngineCapabilities(id)
    } catch (error) {
      addRuntimeWarning(formatMessage(labels.probeFailed, { error: String(error) }))
    } finally {
      setProbingEngineId(null)
    }
  }

  const capabilityLabel = (status: string) => {
    if (status === 'detected') return labels.capabilityDetected
    if (status === 'partial') return labels.capabilityPartial
    if (status === 'timeout') return labels.capabilityTimeout
    if (status === 'failed') return labels.capabilityFailed
    return labels.capabilityUnprobed
  }

  const versionStatusLabel = (engine: EngineInfo) => {
    const status = normalizeEngineVersionStatus(engine.capabilities)
    if (status === 'detected') return labels.versionDetected
    if (status === 'unknown') return labels.versionUnknown
    return labels.versionUnprobed
  }

  const versionLabel = (engine: EngineInfo) => (
    normalizeEngineVersionStatus(engine.capabilities) === 'detected' && engine.version.trim()
      ? engine.version
      : versionStatusLabel(engine)
  )

  return (
    <div className="space-y-5">
      <div className="mb-6 flex flex-col gap-5 xl:flex-row xl:items-end xl:justify-between">
        <div className="space-y-3">
          <div className="flex items-center gap-3">
            <div className="rounded-2xl border border-violet-500/20 bg-violet-500/10 p-3 text-violet-300">
              <Cpu className="h-5 w-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-3xl font-semibold tracking-tight text-slate-50">{t.nav.engine}</h1>
                <span className="rounded-full border border-slate-800 bg-slate-900 px-2.5 py-1 text-xs text-slate-400">
                  {formatMessage(labels.engineCount, { count: engines.length })}
                </span>
              </div>
              <p className="text-sm text-slate-400">
                {labels.subtitle}
              </p>
            </div>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-3" data-guide="engine-scan">
          <Button
            onClick={handleScan}
            disabled={isLoading}
            icon={<RefreshCw className={`h-4 w-4 ${isLoading ? 'animate-spin' : ''}`} />}
          >
            {t.engineMgr.scan}
          </Button>
          <Button
            onClick={handleAddDirectory}
            variant="primary"
            icon={<Plus className="h-4 w-4" />}
          >
            {t.engineMgr.addDir}
          </Button>
        </div>
      </div>

      <div className="mb-6 grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[
          { label: labels.registered, value: engines.length, icon: Cpu, tone: 'text-sky-300 bg-sky-500/10 border-sky-500/20' },
          { label: labels.default, value: defaultEngineId ? 1 : 0, icon: Star, tone: 'text-amber-300 bg-amber-500/10 border-amber-500/20' },
          { label: labels.backends, value: backendCounts.size, icon: RefreshCw, tone: 'text-violet-300 bg-violet-500/10 border-violet-500/20' },
          { label: labels.scanRoots, value: engineDirs.length, icon: FolderOpen, tone: 'text-emerald-300 bg-emerald-500/10 border-emerald-500/20' },
        ].map(card => (
          <MetricCard key={card.label} label={card.label} value={card.value} icon={<card.icon className="h-5 w-5" />} tone={card.tone} />
        ))}
      </div>

      <div className="grid gap-6 2xl:grid-cols-[280px,minmax(0,1.45fr),280px]">
        <Surface as="aside" className="p-5">
          <div className="mb-4">
            <h2 className="text-lg font-semibold text-slate-50">{labels.scanRoots}</h2>
            <p className="mt-1 text-sm text-slate-400">
              {labels.scanRootsDescription}
            </p>
          </div>

          <div className="space-y-3">
            {engineDirs.length === 0 ? (
              <div className="rounded-2xl border border-dashed border-slate-700 px-4 py-8 text-center text-sm text-slate-500">
                {t.engineMgr.noEngines}
              </div>
            ) : (
              engineDirs.map(dir => (
                <InsetSurface key={dir} className="p-4">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium text-slate-100" title={dir}>
                        {dir}
                      </p>
                      <p className="mt-1 text-xs text-slate-500">
                        {engines.filter(engine => engine.dir.startsWith(dir)).length} {labels.discovered}
                      </p>
                    </div>
                    <button
                      onClick={() => handleRemoveDir(dir)}
                      className="rounded-lg p-2 text-slate-500 transition hover:bg-red-500/10 hover:text-red-300"
                      title={t.engineMgr.remove}
                    >
                      <Trash2 className="h-4 w-4" />
                    </button>
                  </div>
                </InsetSurface>
              ))
            )}
          </div>

          {backendCounts.size > 0 && (
            <InsetSurface className="mt-5 p-4">
              <p className="text-sm font-medium text-slate-100">{labels.backendMix}</p>
              <div className="mt-3 flex flex-wrap gap-2">
                {[...backendCounts.entries()].map(([backend, count]) => (
                  <span key={backend} className={`rounded-full border px-2.5 py-1 text-xs ${backendTone(backend)}`}>
                    {backend} {labels.backendUnit} {count}
                  </span>
                ))}
              </div>
            </InsetSurface>
          )}
        </Surface>

        <Surface as="section" className="min-h-[620px] p-5">
          <div className="mb-5 flex flex-col gap-4 lg:flex-row lg:items-center lg:justify-between">
            <div>
              <h2 className="text-lg font-semibold text-slate-50">{labels.inventory}</h2>
              <p className="mt-1 text-sm text-slate-400">
                {labels.inventoryDescription}
              </p>
            </div>
            <div className="flex w-full flex-col gap-3 lg:max-w-xl lg:flex-row">
              <TextInput
                value={searchQuery}
                onChange={event => setSearchQuery(event.target.value)}
                placeholder={labels.search}
                leadingIcon={<Search className="h-4 w-4" />}
                className="flex-1"
              />
              <SelectInput
                value={backendFilter}
                onChange={event => setBackendFilter(event.target.value)}
              >
                <option value="all">{labels.allBackends}</option>
                {[...backendCounts.keys()].sort().map(backend => (
                  <option key={backend} value={backend}>
                    {backend}
                  </option>
                ))}
              </SelectInput>
            </div>
          </div>

          {filteredEngines.length === 0 ? (
            <div className="flex min-h-[420px] flex-col items-center justify-center rounded-2xl border border-dashed border-slate-800 text-center">
              <Cpu className="mb-4 h-12 w-12 text-slate-700" />
              <p className="text-base text-slate-300">{t.engineMgr.noEngines}</p>
              <p className="mt-2 max-w-md text-sm text-slate-500">
                {labels.emptyDescription}
              </p>
            </div>
          ) : (
            <div className="overflow-hidden rounded-2xl border border-slate-800">
              <div className="grid grid-cols-[minmax(0,2.1fr)_120px_140px_180px] gap-4 border-b border-slate-800 bg-slate-950/80 px-4 py-3 text-xs uppercase tracking-[0.14em] text-slate-500">
                <span>{labels.engine}</span>
                <span>{labels.backend}</span>
                <span>{labels.version}</span>
                <span className="text-right">{labels.actions}</span>
              </div>
              <div className="divide-y divide-slate-800 bg-slate-950/30">
                {filteredEngines.map(engine => {
                  const isSelected = selectedEngineId === engine.id
                  const isDefault = defaultEngineId === engine.id
                  return (
                    <div
                      key={engine.id}
                      className={`grid grid-cols-[minmax(0,2.1fr)_120px_140px_180px] gap-4 px-4 py-4 transition ${
                        isSelected ? 'bg-blue-500/10' : 'hover:bg-slate-900/80'
                      }`}
                      onClick={() => setSelectedEngineId(engine.id)}
                    >
                      <div className="min-w-0">
                        <div className="flex items-center gap-2">
                          {editingId === engine.id ? (
                            <input
                              type="text"
                              value={editName}
                              onChange={event => setEditName(event.target.value)}
                              onClick={event => event.stopPropagation()}
                              onKeyDown={event => {
                                if (event.key === 'Enter') commitRename(engine.id)
                                if (event.key === 'Escape') {
                                  editingCanceledRef.current = true
                                  setEditingId(null)
                                }
                              }}
                              onBlur={() => {
                                if (editingCanceledRef.current) {
                                  editingCanceledRef.current = false
                                  return
                                }
                                commitRename(engine.id)
                              }}
                              autoFocus
                              className="h-8 w-full rounded-lg border border-blue-500/50 bg-slate-950 px-3 text-sm text-slate-100 outline-none"
                            />
                          ) : (
                            <>
                              <p className="truncate text-sm font-medium text-slate-100">{engine.name}</p>
                              <button
                                onClick={event => {
                                  event.stopPropagation()
                                  setEditingId(engine.id)
                                  setEditName(engine.name)
                                }}
                                className="rounded-md p-1 text-slate-500 transition hover:bg-slate-800 hover:text-slate-200"
                                title={labels.rename}
                              >
                                <Pencil className="h-3.5 w-3.5" />
                              </button>
                              {isDefault && (
                                <span className="rounded-full border border-amber-500/20 bg-amber-500/10 px-2 py-0.5 text-[11px] text-amber-300">
                                  {t.engineMgr.defaultEngine}
                                </span>
                              )}
                            </>
                          )}
                        </div>
                        <p className="mt-1 truncate text-xs text-slate-500" title={engine.dir}>
                          {engine.dir}
                        </p>
                      </div>
                      <div className="flex items-center">
                        <span className={`rounded-full border px-2.5 py-1 text-xs ${backendTone(engine.backend)}`}>
                          {engine.backend}
                        </span>
                      </div>
                      <div className="min-w-0 self-center">
                        <p className="truncate text-sm text-slate-300" title={versionLabel(engine)}>{versionLabel(engine)}</p>
                        <span className={`mt-1 inline-flex rounded-full border px-2 py-0.5 text-[11px] ${capabilityTone(normalizeEngineCapabilityStatus(engine.capabilities))}`}>
                          {capabilityLabel(normalizeEngineCapabilityStatus(engine.capabilities))}
                        </span>
                      </div>
                      <div className="flex items-center justify-end gap-2">
                        <button
                          onClick={event => {
                            event.stopPropagation()
                            setDefaultEngineId(engine.id)
                          }}
                          className={`rounded-lg px-3 py-1.5 text-xs transition ${
                            isDefault
                              ? 'bg-blue-600 text-white'
                              : 'border border-slate-700 bg-slate-900 text-slate-200 hover:border-slate-600 hover:bg-slate-800'
                          }`}
                        >
                          {isDefault ? t.engineMgr.defaultEngine : t.engineMgr.setDefault}
                        </button>
                        <button
                          onClick={event => {
                            event.stopPropagation()
                            openEngineFolder(engine.dir)
                          }}
                          className="rounded-lg p-2 text-slate-400 transition hover:bg-slate-800 hover:text-slate-100"
                          title={t.engineMgr.openFolder}
                        >
                          <FolderOpen className="h-4 w-4" />
                        </button>
                        <button
                          onClick={event => {
                            event.stopPropagation()
                            void handleDelete(engine.id)
                          }}
                          className="rounded-lg p-2 text-slate-400 transition hover:bg-red-500/10 hover:text-red-300"
                          title={t.engineMgr.remove}
                        >
                          <Trash2 className="h-4 w-4" />
                        </button>
                      </div>
                    </div>
                  )
                })}
              </div>
            </div>
          )}
        </Surface>

        <Surface as="aside" className="p-5">
          <div className="mb-4">
            <h2 className="text-lg font-semibold text-slate-50">{labels.details}</h2>
            <p className="mt-1 text-sm text-slate-400">
              {labels.detailsDescription}
            </p>
          </div>

          {!selectedEngine ? (
            <div className="flex min-h-[280px] flex-col items-center justify-center rounded-2xl border border-dashed border-slate-800 text-center">
              <Cpu className="mb-4 h-10 w-10 text-slate-700" />
              <p className="text-sm text-slate-400">
                {labels.noSelection}
              </p>
            </div>
          ) : (
            <div className="space-y-5">
              <InsetSurface className="p-4">
                <div className="flex items-start gap-3">
                  <div className={`rounded-2xl border p-3 ${backendTone(selectedEngine.backend)}`}>
                    <Cpu className="h-5 w-5" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium text-slate-100">{selectedEngine.name}</p>
                    <PathText value={selectedEngine.dir} maxLength={58} className="mt-1 text-slate-500" />
                  </div>
                </div>
              </InsetSurface>

              <InsetSurface className="space-y-3 p-4">
                {[
                  [labels.backend, selectedEngine.backend],
                  [labels.version, versionLabel(selectedEngine)],
                  [labels.versionRecognition, versionStatusLabel(selectedEngine)],
                  [labels.compatibility, capabilityLabel(normalizeEngineCapabilityStatus(selectedEngine.capabilities))],
                  [labels.supportedFlags, selectedEngine.capabilities?.supportedFlags?.length ?? 0],
                  [labels.lastProbe, selectedEngine.capabilities?.probedAt
                    ? new Date(selectedEngine.capabilities.probedAt * 1000).toLocaleString(lang)
                    : labels.never],
                  [labels.default, defaultEngineId === selectedEngine.id ? (labels.yes) : (labels.no)],
                  [labels.scanRoot, engineDirs.find(dir => selectedEngine.dir.startsWith(dir)) || '--'],
                ].map(([label, value]) => (
                  <div key={label} className="flex items-center justify-between gap-3">
                    <span className="text-sm text-slate-500">{label}</span>
                    <span className="min-w-0 text-right text-sm text-slate-200">
                      {label === (labels.scanRoot) && value !== '--'
                        ? <PathText value={String(value)} maxLength={32} className="justify-end text-slate-200" />
                        : value}
                    </span>
                  </div>
                ))}
              </InsetSurface>

              {selectedEngine.capabilities?.error && (
                <div className="rounded-lg border border-amber-500/20 bg-amber-500/10 px-3 py-2 text-xs leading-5 text-amber-200">
                  {selectedEngine.capabilities.error}
                </div>
              )}

              {normalizeEngineVersionStatus(selectedEngine.capabilities) === 'unknown' && (
                <div className="rounded-lg border border-amber-500/20 bg-amber-500/10 px-3 py-2 text-xs leading-5 text-amber-200">
                  {labels.versionUnknownWarning}
                </div>
              )}

              <div className="grid gap-3">
                <Button
                  onClick={() => void handleProbe(selectedEngine.id)}
                  disabled={probingEngineId !== null}
                  icon={probingEngineId === selectedEngine.id
                    ? <LoaderCircle className="h-4 w-4 animate-spin" />
                    : <ShieldCheck className="h-4 w-4" />}
                >
                  {probingEngineId === selectedEngine.id ? labels.probing : labels.probeNow}
                </Button>
                <Button
                  onClick={() => setDefaultEngineId(selectedEngine.id)}
                  variant="primary"
                  icon={<Star className="h-4 w-4" />}
                >
                  {defaultEngineId === selectedEngine.id ? t.engineMgr.defaultEngine : t.engineMgr.setDefault}
                </Button>
                <Button
                  onClick={() => openEngineFolder(selectedEngine.dir)}
                  icon={<FolderOpen className="h-4 w-4" />}
                >
                  {t.engineMgr.openFolder}
                </Button>
                <Button
                  onClick={() => void handleDelete(selectedEngine.id)}
                  variant="danger"
                  icon={<Trash2 className="h-4 w-4" />}
                >
                  {t.engineMgr.remove}
                </Button>
              </div>
            </div>
          )}
        </Surface>
      </div>
    </div>
  )
}

export default EngineManager
