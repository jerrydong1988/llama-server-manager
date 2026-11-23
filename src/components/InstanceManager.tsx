import { useState, useRef, useEffect, useMemo, useCallback } from 'react'
import { Activity, Play, Square, Plus, Trash2, Globe, X, Terminal, Settings, FolderOpen, Wifi, ArrowUp, ArrowDown, Pencil, Search, MoreHorizontal, LoaderCircle } from 'lucide-react'
import { useAppStore, defaultInstanceConfig } from '../store'
import { exportShellCommand } from '../store/commandFormatting'
import { AppInvokeError, invokeApp as invoke } from '../lib/ipc'
import { confirm } from '@tauri-apps/plugin-dialog'
import { formatMessage, useI18n } from '../i18n'
import { formatHostPort } from '../utils/network'
import { normalizeInstanceConfig } from '../modelPolicy'
import type { Instance, InstanceLifecyclePhase } from '../store/types'
import { Button, EmptyState, PathText, SelectInput, Surface, TextInput } from './ui'
import { InstanceModelPicker } from './InstanceManager/InstanceModelPicker'
import { CommandFeedbackModal, MissingEngineBanner } from './InstanceManager/CommandFeedbackModal'
import { configForEngine } from '../gufo'
import { gufoCopy } from '../i18n/gufoCopy'
import { isConfiguredEngineMissing, resolveEffectiveEngine } from '../store/engineResolution'
import { markExplicitOverride } from '../parameterIntent'
import { pathsEqual } from '../utils/path'
import { usePortAvailability } from './InstanceManager/usePortAvailability'
import { updateInstanceListView, useInstanceSelection } from './InstanceManager/useInstanceSelection'
import { useGuideTourStore } from './guide/guideTourStore'
import { CheckpointStatusCard } from './InstanceManager/CheckpointStatusCard'
import { CommandPreviewModal } from './InstanceManager/CommandPreviewModal'
type TestState = 'checking' | `ok:${string}` | `error:${string}`
type CommandErrorState = { instanceId: string; message: string; missingEngine: boolean }
const InstanceManager = () => {
  const instances = useAppStore(s => s.instances)
  const addInstance = useAppStore(s => s.addInstance)
  const deleteInstance = useAppStore(s => s.deleteInstance)
  const startInstance = useAppStore(s => s.startInstance)
  const stopInstance = useAppStore(s => s.stopInstance)
  const instanceLifecycle = useAppStore(s => s.instanceLifecycle)
  const openBrowser = useAppStore(s => s.openBrowser)
  const generateCommand = useAppStore(s => s.generateCommand)
  const models = useAppStore(s => s.models)
  const modelDirs = useAppStore(s => s.modelDirs)
  const engines = useAppStore(s => s.engines)
  const defaultEngineId = useAppStore(s => s.defaultEngineId)
  const setActiveConfigInstanceId = useAppStore(s => s.setActiveConfigInstanceId)
  const setActiveTab = useAppStore(s => s.setActiveTab)
  const moveInstance = useAppStore(s => s.moveInstance)
  const renameInstance = useAppStore(s => s.renameInstance)
  const { t, lang } = useI18n()
  const labels = t.instanceWorkspace
  const { portStatus, setPortStatus, schedulePortCheck } = usePortAvailability(labels)

  const [showCreateModal, setShowCreateModal] = useState(false)
  const [showCmdModal, setShowCmdModal] = useState('')
  const [preparingCommand, setPreparingCommand] = useState('')
  const commandInFlight = useRef(false)
  const [cmdArgs, setCmdArgs] = useState<string[]>([])
  const [cmdRaw, setCmdRaw] = useState('')
  const exportShell = /win/i.test(navigator.platform) ? 'powershell' : 'posix'
  const [commandError, setCommandError] = useState<CommandErrorState | null>(null)
  const [showCreatePicker, setShowCreatePicker] = useState(false)
  const [pickerCollapsed, setPickerCollapsed] = useState<Set<string>>(new Set())
  const [newInst, setNewInst] = useState({ name: '', modelId: '', modelPath: '', mmprojPath: '', port: 8080, engineId: '' })
  const [testResults, setTestResults] = useState<Record<string, TestState>>({})
  const [editingId, setEditingId] = useState('')
  const [editName, setEditName] = useState('')
  const editingCanceledRef = useRef(false)
  const [enginePickerForId, setEnginePickerForId] = useState('')
  const mountedRef = useRef(true)

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  const formatUptime = (startTime?: number) => {
    if (!startTime) return '--'
    const ms = Date.now() - startTime
    if (ms < 0) return '--'
    const hours = Math.floor(ms / 3600000)
    const minutes = Math.floor((ms % 3600000) / 60000)
    if (hours > 24) {
      const days = Math.floor(hours / 24)
      return `${days}d ${hours % 24}h`
    }
    return hours > 0 ? `${hours}h ${minutes}m` : `${minutes}m`
  }

  const engineMissingFor = useCallback((inst: Instance) => isConfiguredEngineMissing(inst.config, engines), [engines])
  const engineNameFor = useCallback((inst: Instance) => resolveEffectiveEngine(inst.config, engines, defaultEngineId)?.name
    || (engineMissingFor(inst) ? labels.configuredEngineMissing : t.instance.sysPath), [defaultEngineId, engineMissingFor, engines, labels.configuredEngineMissing, t.instance.sysPath])

  const renderProcessStatus = (state: Instance['status'] | InstanceLifecyclePhase) => (
    <span className="console-status instance-process-status" data-state={state}>
      {state === 'running' && <Activity size={14} strokeWidth={2.4} aria-hidden="true" />}
      {t.instance[state]}
    </span>
  )

  const healthText = (inst: Instance) => {
    if (inst.status === 'stopped') return labels.offline
    if (inst.status === 'error') return t.instance.error
    if (inst.config.engine_kind === 'gufo' && inst.status === 'running' && inst.healthCheck === 'pending') return gufoCopy(lang).loading
    if (inst.healthCheck === 'ok') return labels.healthy
    if (inst.healthCheck === 'fail') return t.downloadPage.failed
    return labels.pending
  }

  const healthDotClass = (inst: Instance) => {
    if (inst.status === 'stopped') return 'bg-ui-muted'
    if (inst.status === 'error') return 'bg-ui-danger'
    if (inst.healthCheck === 'ok') return 'bg-ui-positive'
    if (inst.healthCheck === 'fail') return 'bg-ui-danger'
    return 'bg-ui-warn'
  }

  const {
    search, statusFilter, engineFilter, filteredInstances, selectedInstance, setSelectedInstanceId, selectedRowRef,
  } = useInstanceSelection(instances, defaultEngineId, engineNameFor)

  const runningCount = instances.filter(inst => inst.status === 'running').length
  const stoppedCount = instances.length - runningCount
  const erroredCount = instances.filter(inst => inst.status === 'error').length
  const autoStartCount = instances.filter(inst => inst.config.auto_start).length
  const missingEngineInstances = useMemo(() => instances.filter(engineMissingFor), [engineMissingFor, instances])
  const selectedIndex = selectedInstance ? filteredInstances.findIndex(inst => inst.id === selectedInstance.id) : -1
  const filteredInstanceIds = useMemo(() => filteredInstances.map(inst => inst.id), [filteredInstances])

  const handleCreate = async () => {
    const model = models.find(m => m.id === newInst.modelId)
    if (!model || !newInst.modelPath) return

    try {
      const portFree = await invoke<boolean>('check_port', { port: newInst.port, host: '127.0.0.1' })
      if (!portFree) {
        setPortStatus(labels.portInUse)
        return
      }
    } catch (error) {
      setPortStatus(labels.portCheckFailed)
      useAppStore.getState().addRuntimeWarning(`${labels.portCheckFailed}: ${String(error)}`)
      return
    }

    const id = crypto.randomUUID()
    let config = defaultInstanceConfig()
    config.id = id
    config.name = newInst.name || model.name.replace('.gguf', '')
    config.model_path = newInst.modelPath
    config = markExplicitOverride(config, 'mmproj_path', newInst.mmprojPath)
    config.port = newInst.port
    config.host = '127.0.0.1'
    config.engine_id = newInst.engineId || defaultEngineId || ''
    const selectedEngine = engines.find(engine => pathsEqual(engine.id, config.engine_id))
    if (selectedEngine) config = configForEngine(config, selectedEngine)
    const normalized = normalizeInstanceConfig(config, model, { context: 'create' })

    if (!defaultEngineId && engines[0]) useAppStore.setState({ defaultEngineId: engines[0].id })

    addInstance({
      id,
      name: config.name,
      status: 'stopped',
      model: model.name,
      port: newInst.port,
      healthCheck: 'pending',
      config: normalized.config,
    })

    setSelectedInstanceId(id)
    setShowCreateModal(false)
    setNewInst({ name: '', modelId: '', modelPath: '', mmprojPath: '', port: newInst.port + 1, engineId: newInst.engineId })
    setPortStatus('')
  }
  const handleDelete = async (id: string) => {
    if (!await confirm(t.instance.confirmDelete, { title: t.instance.delete, kind: 'warning' })) return
    try {
      await stopInstance(id)
      await deleteInstance(id)
      await useAppStore.getState().clearCheckpoint(id)
      await invoke('delete_instance_log', { instanceId: id })
    } catch (error) {
      useAppStore.getState().addRuntimeWarning(`instance deletion cleanup failed: ${error instanceof Error ? error.message : String(error)}`)
    }
  }
  const [copyFeedback, setCopyFeedback] = useState(false)
  const handleCopyCommand = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      setCopyFeedback(true)
      setTimeout(() => { if (mountedRef.current) setCopyFeedback(false) }, 2000)
    } catch {
      // ignore
    }
  }

  const handleShowCommand = async (id: string) => {
    if (commandInFlight.current) return
    const inst = instances.find(i => i.id === id)
    if (!inst) return
    const engine = resolveEffectiveEngine(inst.config, engines, defaultEngineId)
    if (!engine) {
      setCommandError({ instanceId: id, message: engineMissingFor(inst) ? labels.missingEngineCommandError : labels.noEngineCommandError, missingEngine: true })
      return
    }
    setCommandError(null)
    commandInFlight.current = true
    setPreparingCommand(id)
    try {
      const { command: cmd } = await generateCommand(inst.config, engine.exe, true)
      if (!mountedRef.current) return
      const raw = exportShellCommand(cmd, exportShell)
      setCmdRaw(raw)
      setCmdArgs(cmd)
      setCopyFeedback(false)
      setShowCmdModal(id)
    } catch (e) {
      if (!mountedRef.current) return
      console.error(e)
      const error = e instanceof AppInvokeError && ['ENGINE_PREVIEW_PROBE_REQUIRED', 'GUFO_PROBE_REQUIRED'].includes(e.code)
        ? labels.incompleteCommandProbe
        : e instanceof Error ? e.message : String(e)
      setCommandError({ instanceId: id, message: formatMessage(labels.commandGenerationFailed, { error }), missingEngine: false })
    } finally {
      commandInFlight.current = false
      if (mountedRef.current) setPreparingCommand('')
    }
  }

  const handleTestConnection = async (inst: Instance) => {
    const guideConnection = useGuideTourStore.getState().beginConnection(inst.id)
    setTestResults(state => ({ ...state, [inst.id]: 'checking' }))
    try {
      const result = await invoke('test_connection', {
        instanceId: inst.id,
      })
      if (!mountedRef.current) return
      setTestResults(state => ({ ...state, [inst.id]: `ok:${String(result)}` }))
      useGuideTourStore.getState().recordConnection(guideConnection, inst)
    } catch (e: any) {
      if (!mountedRef.current) return
      setTestResults(state => ({ ...state, [inst.id]: `error:${e?.toString() || 'Failed'}` }))
    }
    setTimeout(() => {
      if (!mountedRef.current) return
      setTestResults(state => {
        const next = { ...state }
        delete next[inst.id]
        return next
      })
    }, 5000)
  }

  const toggleAutoStart = (inst: Instance) => {
    const state = useAppStore.getState()
    const idx = state.instances.findIndex(item => item.id === inst.id)
    if (idx < 0) return
    const next = [...state.instances]
    next[idx] = { ...next[idx], config: { ...next[idx].config, auto_start: !inst.config.auto_start } }
    useAppStore.setState({ instances: next })
    void useAppStore.getState().saveConfig().catch(() => {})
  }

  const commitRename = (inst: Instance) => {
    const nextName = editName.trim()
    if (nextName && nextName !== inst.name) renameInstance(inst.id, nextName)
    setEditingId('')
  }

  const renderTestResult = (instId: string) => {
    const status = testResults[instId]
    if (!status) return null
    if (status === 'checking') {
      return <span className="max-w-[180px] truncate text-xs text-ui-info">{labels.checking}</span>
    }
    if (status.startsWith('ok:')) {
      const text = status.slice(3)
      return <span className="max-w-[180px] truncate text-xs text-ui-positive" title={text}>{text}</span>
    }
    const text = status.slice(6)
    return <span className="max-w-[180px] truncate text-xs text-ui-danger" title={text}>{text}</span>
  }

  return (
    <div className="console-page space-y-4" data-workspace="instances">
      {engines.length === 0 && (
        <div className="rounded-lg border border-ui-line bg-ui-warn-bg px-4 py-3 text-sm text-ui-warn">
          {labels.noEnginePrefix}
          {' '}
          <button onClick={() => setActiveTab('engine')} className="font-medium underline underline-offset-4">
            {t.engineMgr.scan}
          </button>
          {' '}
          {labels.noEngineSuffix}
        </div>
      )}
      {missingEngineInstances.length > 0 && <MissingEngineBanner
        message={formatMessage(labels.missingEngineBanner, { count: missingEngineInstances.length })}
        actionLabel={labels.reselectEngine}
        onAction={() => { setSelectedInstanceId(missingEngineInstances[0].id); setEnginePickerForId(missingEngineInstances[0].id) }}
      />}

      <header className="console-heading-row">
        <div className="min-w-0">
          <h2 className="console-title">{labels.listTitle}</h2>
          {(autoStartCount > 0 || erroredCount > 0) && <p className="mt-1 text-sm text-ui-muted">
            {autoStartCount > 0 && <span>{labels.autoStart} {autoStartCount}</span>}
            {erroredCount > 0 && <span className="ml-3 text-ui-danger">{labels.errored} {erroredCount}</span>}
          </p>}
        </div>
        <Button variant="primary" data-guide="instance-create" onClick={() => setShowCreateModal(true)} icon={<Plus size={16} />}>
          {t.instance.create}
        </Button>
      </header>

      <div className="instance-workspace-grid">
        <Surface as="section" className="instance-list-container relative z-10 min-w-0" aria-label={labels.listTitle}>
          <div className="instance-tools">
            <div className="flex flex-wrap items-center gap-1" role="group" aria-label={labels.listTitle}>
              {(['all', 'running', 'stopped'] as const).map(filter => {
                const label = filter === 'all' ? labels.all : filter === 'running' ? t.instance.running : t.instance.stopped
                const count = filter === 'all' ? instances.length : filter === 'running' ? runningCount : stoppedCount
                return <button key={filter} type="button" className="console-filter" aria-pressed={statusFilter === filter}
                  onClick={() => updateInstanceListView({ statusFilter: filter })}>
                  {label}<span className="console-filter-count">{count}</span>
                </button>
              })}
            </div>
            <div className="instance-search-tools">
              <TextInput type="search" value={search} onChange={e => updateInstanceListView({ search: e.target.value })}
                aria-label={labels.searchPlaceholder} placeholder={labels.searchPlaceholder} leadingIcon={<Search size={15} />} className="min-w-0" />
              <SelectInput value={engineFilter} aria-label={labels.allEngines} onChange={e => updateInstanceListView({ engineFilter: e.target.value })}>
                <option value="all">{labels.allEngines}</option>
                {engines.map(engine => <option key={engine.id} value={engine.id}>{engine.name}</option>)}
              </SelectInput>
            </div>
          </div>

          {filteredInstances.length === 0 ? (
            instances.length ? <div className="grid min-h-36 place-content-center justify-items-center gap-2 p-4 text-sm text-ui-muted">
              <p>{labels.noMatches}</p>
              <Button size="sm" variant="subtle" onClick={() => updateInstanceListView({ search: '', engineFilter: 'all', statusFilter: 'all' })}>{labels.clearFilters}</Button>
            </div> : <EmptyState icon={<Play className="h-10 w-10" />} title={t.instance.noInstances} className="rounded-none border-0 shadow-none" />
          ) : (
            <div className="divide-y divide-ui-line">
              {filteredInstances.map((inst, index) => {
                const selected = selectedInstance?.id === inst.id
                const isRunning = inst.status === 'running'
                const lifecyclePhase = instanceLifecycle[inst.id]
                const processState = lifecyclePhase ?? inst.status
                const isLifecycleBusy = Boolean(lifecyclePhase)
                return (
                  <div
                    key={inst.id}
                    ref={selected ? selectedRowRef : undefined}
                    role="button"
                    tabIndex={0}
                    onClick={() => setSelectedInstanceId(inst.id)}
                    onKeyDown={event => {
                      if (event.target === event.currentTarget && (event.key === 'Enter' || event.key === ' ')) {
                        event.preventDefault()
                        setSelectedInstanceId(inst.id)
                      }
                    }}
                    aria-selected={selected}
                    aria-current={selected ? 'true' : undefined}
                    data-selected={selected}
                    className="console-list-row instance-list-row relative grid min-w-0 items-center gap-3 px-4 py-3 text-left last:rounded-b-lg"
                  >
                    <div className="flex min-w-0 items-center gap-2">
                      <div className="min-w-0 flex-1">
                        <div className="flex min-w-0 items-center gap-2">
                          {editingId === inst.id ? (
                            <input
                              type="text"
                              aria-label={labels.rename}
                              value={editName}
                              onClick={e => e.stopPropagation()}
                              onChange={e => setEditName(e.target.value)}
                              onKeyDown={e => {
                                if (e.nativeEvent.isComposing) return
                                if (e.key === 'Enter') commitRename(inst)
                                if (e.key === 'Escape') { editingCanceledRef.current = true; setEditingId('') }
                              }}
                              onBlur={() => {
                                if (editingCanceledRef.current) {
                                  editingCanceledRef.current = false
                                  return
                                }
                                commitRename(inst)
                              }}
                              autoFocus
                              className="min-w-0 flex-1 border-b border-ui-action-line bg-transparent px-1 text-sm font-medium text-ui-ink outline-none"
                            />
                          ) : (
                            <>
                              <div className={`min-w-0 truncate text-ui-ink ${processState === 'running' ? 'font-semibold' : 'font-medium'}`} title={inst.name}>{inst.name}</div>
                              <Button
                                onClick={e => { e.stopPropagation(); editingCanceledRef.current = false; setEditingId(inst.id); setEditName(inst.name) }}
                                variant="subtle"
                                size="icon"
                                className="h-7 w-7 shrink-0 rounded-md"
                                title={labels.rename}
                              >
                                <Pencil className="h-3.5 w-3.5" />
                              </Button>
                            </>
                          )}
                        </div>
                        <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-xs text-ui-muted">
                          <div className="flex min-w-0 max-w-full items-center gap-2">
                            {renderProcessStatus(processState)}
                            {isRunning && <span className={`shrink-0 ${inst.healthCheck === 'fail' ? 'text-ui-danger' : inst.healthCheck === 'ok' ? 'text-ui-muted' : 'text-ui-warn'}`}>{healthText(inst)}</span>}
                            <span className="min-w-0 max-w-[320px] truncate" title={inst.model}>{inst.model}</span>
                          </div>
                          {renderTestResult(inst.id)}
                        </div>
                      </div>
                    </div>

                    <div className="flex min-w-0">
                      <Button
                        onClick={e => { e.stopPropagation(); setEnginePickerForId(inst.id) }}
                        variant={engineMissingFor(inst) ? 'danger' : 'secondary'}
                        size="sm"
                        className="h-8 max-w-[150px] justify-start px-2"
                        title={engineNameFor(inst)}
                      >
                        <span className="min-w-0 truncate">{engineNameFor(inst)}</span>
                      </Button>
                    </div>

                    <div className="min-w-0 text-xs text-ui-muted">
                      <div className="truncate font-mono text-ui-body" title={formatHostPort(inst.config.host, inst.config.port)}>{formatHostPort(inst.config.host, inst.config.port)}</div>
                      <div className="mt-1">{isRunning ? formatUptime(inst.startTime) : '--'}</div>
                    </div>

                    <div className="flex min-w-0 items-center justify-end gap-1.5 whitespace-nowrap" onClick={e => e.stopPropagation()}>
                      <button
                        type="button"
                        role="switch"
                        aria-checked={!!inst.config.auto_start}
                        onClick={() => toggleAutoStart(inst)}
                        className="console-switch"
                        aria-label={labels.autoStartHint}
                        title={labels.autoStartHint}
                      />
                      {isRunning ? (
                        <Button
                          onClick={() => void stopInstance(inst.id).catch(() => {})}
                          disabled={isLifecycleBusy}
                          variant="secondary"
                          size="sm"
                          className="h-8 w-[70px]"
                          icon={isLifecycleBusy ? <LoaderCircle className="h-3.5 w-3.5 animate-spin" /> : <Square className="h-3.5 w-3.5" />}
                        >
                          <span>{lifecyclePhase === 'stopping' ? t.instance.stopping : t.instance.stop}</span>
                        </Button>
                      ) : (
                        <Button
                          onClick={() => void startInstance(inst.id).catch(() => {})}
                          disabled={isLifecycleBusy}
                          variant="success"
                          size="sm"
                          className="h-8 w-[70px]"
                          icon={isLifecycleBusy ? <LoaderCircle className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
                        >
                          <span>{lifecyclePhase === 'starting' ? t.instance.starting : t.instance.start}</span>
                        </Button>
                      )}
                      <details className="group relative">
                        <summary
                          className="flex h-8 w-8 cursor-pointer list-none items-center justify-center rounded-lg border border-ui-line text-ui-muted transition hover:bg-ui-hover hover:text-ui-ink [&::-webkit-details-marker]:hidden"
                          title={labels.more}
                          aria-label={labels.more}
                        >
                          <MoreHorizontal className="h-4 w-4" />
                        </summary>
                        <div className="absolute right-0 z-20 mt-2 w-48 overflow-hidden rounded-lg border border-ui-line bg-ui-surface py-1 text-sm shadow-xl">
                          <button type="button" className="console-menu-item" disabled={!isRunning}
                            onClick={() => openBrowser(inst.id, inst.config.host, inst.config.port, Boolean(inst.config.ssl_key_file && inst.config.ssl_cert_file), inst.config.api_prefix)}>
                            <Globe size={14} />{t.instance.openBrowser}
                          </button>
                          <button
                            type="button"
                            onClick={() => handleTestConnection(inst)}
                            disabled={!isRunning}
                            className="flex w-full items-center gap-2 px-3 py-2 text-left text-ui-body hover:bg-ui-hover disabled:cursor-not-allowed disabled:opacity-45"
                          >
                            <Wifi className="h-3.5 w-3.5" />
                            <span>{t.instance.testConnection}</span>
                          </button>
                          <button
                            type="button"
                            onClick={() => handleShowCommand(inst.id)}
                            aria-disabled={Boolean(preparingCommand)}
                            className="flex w-full items-center gap-2 px-3 py-2 text-left text-ui-body hover:bg-ui-hover"
                          >
                            <Terminal className="h-3.5 w-3.5" />
                            <span>{preparingCommand === inst.id ? labels.preparingCommand : t.instance.genCommand}</span>
                          </button>
                          <button
                            type="button"
                            onClick={() => { setActiveConfigInstanceId(inst.id); setActiveTab('config') }}
                            className="flex w-full items-center gap-2 px-3 py-2 text-left text-ui-body hover:bg-ui-hover"
                          >
                            <Settings className="h-3.5 w-3.5" />
                            <span>{t.instance.configParams}</span>
                          </button>
                          <button
                            type="button"
                            onClick={() => moveInstance(inst.id, 'up', filteredInstanceIds)}
                            disabled={index === 0}
                            className="flex w-full items-center gap-2 border-t border-ui-line px-3 py-2 text-left text-ui-body hover:bg-ui-hover disabled:cursor-not-allowed disabled:opacity-45"
                          >
                            <ArrowUp className="h-3.5 w-3.5" />
                            <span>{labels.moveUp}</span>
                          </button>
                          <button
                            type="button"
                            onClick={() => moveInstance(inst.id, 'down', filteredInstanceIds)}
                            disabled={index === filteredInstances.length - 1}
                            className="flex w-full items-center gap-2 px-3 py-2 text-left text-ui-body hover:bg-ui-hover disabled:cursor-not-allowed disabled:opacity-45"
                          >
                            <ArrowDown className="h-3.5 w-3.5" />
                            <span>{labels.moveDown}</span>
                          </button>
                          <button
                            type="button"
                            onClick={() => handleDelete(inst.id)}
                            className="flex w-full items-center gap-2 border-t border-ui-line px-3 py-2 text-left text-ui-danger hover:bg-ui-danger-bg"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                            <span>{t.instance.delete}</span>
                          </button>
                        </div>
                      </details>
                    </div>
                  </div>
                )
              })}
            </div>
          )}
        </Surface>

        <Surface as="aside" className="instance-inspector min-w-0 p-4">
          {selectedInstance ? (
            <div className="space-y-4">
              <div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
                <div className="min-w-0">
                  <p className="mb-1 text-xs text-ui-muted">{labels.operationTarget}</p>
                  <h3 className="truncate text-lg font-semibold text-ui-ink" title={selectedInstance.name}>{selectedInstance.name}</h3>
                </div>
                {renderProcessStatus(instanceLifecycle[selectedInstance.id] ?? selectedInstance.status)}
              </div>

              <div className="grid grid-cols-2 gap-2" data-guide="instance-runtime">
                {selectedInstance.status === 'running' ? (
                  <Button size="sm" onClick={() => void stopInstance(selectedInstance.id).catch(() => {})} disabled={Boolean(instanceLifecycle[selectedInstance.id])}
                    icon={<Square size={14} />}>{instanceLifecycle[selectedInstance.id] === 'stopping' ? t.instance.stopping : t.instance.stop}</Button>
                ) : (
                  <Button size="sm" variant="success" onClick={() => void startInstance(selectedInstance.id).catch(() => {})} disabled={Boolean(instanceLifecycle[selectedInstance.id])}
                    icon={<Play size={14} />}>{instanceLifecycle[selectedInstance.id] === 'starting' ? t.instance.starting : t.instance.start}</Button>
                )}
                <Button size="sm" variant="primary" icon={<Settings size={14} />} onClick={() => { setActiveConfigInstanceId(selectedInstance.id); setActiveTab('config') }}>
                  {t.instance.configParams}
                </Button>
                <Button size="sm" variant="subtle" data-guide="instance-connection" onClick={() => handleTestConnection(selectedInstance)} disabled={selectedInstance.status !== 'running'} icon={<Wifi size={14} />}>
                  {t.instance.testConnection}
                </Button>
                <Button size="sm" variant="subtle" onClick={() => handleShowCommand(selectedInstance.id)} aria-disabled={Boolean(preparingCommand)}
                  icon={preparingCommand === selectedInstance.id ? <LoaderCircle size={14} className="animate-spin" /> : <Terminal size={14} />}>
                  {preparingCommand === selectedInstance.id ? labels.preparingCommand : t.instance.genCommand}
                </Button>
              </div>
              <div className="grid grid-cols-2 gap-2 border-t border-ui-line pt-3 text-xs text-ui-muted">
                <div>{labels.health}<div className="mt-1 flex items-center gap-1.5 text-sm text-ui-ink"><span className={`h-2 w-2 rounded-full ${healthDotClass(selectedInstance)}`} />{healthText(selectedInstance)}</div></div>
                <div>{labels.uptime}<div className="mt-1 text-sm text-ui-ink">{selectedInstance.status === 'running' ? formatUptime(selectedInstance.startTime) : '—'}</div></div>
              </div>

              <div className="space-y-2 border-t border-ui-line pt-3 text-sm">
                <div className="grid min-w-0 grid-cols-[86px_minmax(0,1fr)] gap-3">
                  <span className="text-xs text-ui-muted">{labels.endpoint}</span>
                  <span className="truncate font-mono text-xs text-ui-body" title={formatHostPort(selectedInstance.config.host, selectedInstance.config.port)}>{formatHostPort(selectedInstance.config.host, selectedInstance.config.port)}</span>
                </div>
                <div className="grid min-w-0 grid-cols-[86px_minmax(0,1fr)] gap-3">
                  <span className="text-xs text-ui-muted">{labels.engine}</span>
                  <button
                    type="button"
                    onClick={() => setEnginePickerForId(selectedInstance.id)}
                    className={`min-w-0 truncate text-left text-xs font-medium hover:underline ${engineMissingFor(selectedInstance) ? 'text-ui-danger' : 'text-ui-info'}`}
                    title={engineNameFor(selectedInstance)}
                  >
                    {engineNameFor(selectedInstance)}
                  </button>
                </div>
                <div className="grid min-w-0 grid-cols-[86px_minmax(0,1fr)] gap-3">
                  <span className="text-xs text-ui-muted">{t.instance.model}</span>
                  <span className="truncate text-xs text-ui-body" title={selectedInstance.model}>{selectedInstance.model}</span>
                </div>
                <div className="grid min-w-0 grid-cols-[86px_minmax(0,1fr)] gap-3">
                  <span className="text-xs text-ui-muted">{labels.modelPath}</span>
                  <PathText value={selectedInstance.config.model_path || selectedInstance.model} maxLength={36} className="text-ui-body" />
                </div>
              </div>

              <div className="space-y-2 border-t border-ui-line pt-4">
                <div className="flex items-center justify-between gap-3">
                  <div>
                    <div className="text-xs font-semibold text-ui-muted">{labels.autoStart}</div>
                    <div className="mt-1 text-[11px] text-ui-muted">{labels.autoStartHint}</div>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={!!selectedInstance.config.auto_start}
                    onClick={() => toggleAutoStart(selectedInstance)}
                    className="console-switch" aria-label={labels.autoStart} title={labels.autoStartHint}
                  />
                </div>
                <details className="border-t border-ui-line pt-2">
                  <summary className="cursor-pointer text-xs font-medium text-ui-muted">{labels.more}</summary>
                  <div className="mt-2 flex flex-wrap gap-1">
                    <Button size="sm" variant="subtle" disabled={selectedInstance.status !== 'running'} icon={<Globe size={14} />}
                      onClick={() => openBrowser(selectedInstance.id, selectedInstance.config.host, selectedInstance.config.port, Boolean(selectedInstance.config.ssl_key_file && selectedInstance.config.ssl_cert_file), selectedInstance.config.api_prefix)}>
                      {t.instance.openBrowser}
                    </Button>
                    <Button size="sm" variant="danger" icon={<Trash2 size={14} />} onClick={() => handleDelete(selectedInstance.id)}>{t.instance.delete}</Button>
                  </div>
                <div className="grid grid-cols-2 gap-2 pt-2">
                  <Button onClick={() => moveInstance(selectedInstance.id, 'up', filteredInstanceIds)} disabled={selectedIndex <= 0} variant="subtle" size="sm" icon={<ArrowUp className="h-4 w-4" />}>{labels.moveUp}</Button>
                  <Button onClick={() => moveInstance(selectedInstance.id, 'down', filteredInstanceIds)} disabled={selectedIndex < 0 || selectedIndex === filteredInstances.length - 1} variant="subtle" size="sm" icon={<ArrowDown className="h-4 w-4" />}>{labels.moveDown}</Button>
                </div>
                </details>
              </div>
              {selectedInstance.config.engine_kind !== 'gufo' && <CheckpointStatusCard instance={selectedInstance} />}

            </div>
          ) : (
            <div className="flex min-h-[260px] flex-col items-center justify-center text-center">
              <Play className="h-10 w-10 text-ui-body" />
              <p className="mt-3 text-sm text-ui-muted">{labels.noSelection}</p>
            </div>
          )}
        </Surface>
      </div>

      {showCreateModal && (
        <div data-guide-dialog="create-instance" role="dialog" aria-modal="true" aria-labelledby="create-instance-title" className="fixed inset-0 z-50 flex items-center justify-center bg-ui-scrim p-4">
          <Surface className="w-full max-w-md overflow-hidden">
            <div className="flex items-center justify-between border-b border-ui-line bg-ui-inset px-6 py-4">
              <h3 id="create-instance-title" className="text-lg font-semibold text-ui-ink">{t.instance.newInstance}</h3>
              <Button onClick={() => setShowCreateModal(false)} variant="subtle" size="icon" aria-label={t.appShell.close}><X className="h-5 w-5" /></Button>
            </div>
            <div className="space-y-4 p-6">
              <div>
                <label className="mb-1 block text-sm font-medium text-ui-body">{t.instance.name}</label>
                <TextInput
                  type="text"
                  value={newInst.name}
                  onChange={e => setNewInst({ ...newInst, name: e.target.value })}
                  placeholder={t.instance.namePlaceholder}
                />
              </div>
              <div>
                <label className="mb-1 block text-sm font-medium text-ui-body">{t.instance.selectModel}</label>
                <div className="flex gap-2">
                  <TextInput
                    type="text"
                    value={models.find(m => m.id === newInst.modelId)?.name || ''}
                    readOnly
                    placeholder={t.instance.selectModelPlaceholder}
                    className="flex-1"
                  />
                  <Button onClick={() => setShowCreatePicker(true)} variant="primary" size="icon" title={t.modelRepo.selectFromRepo}>
                    <FolderOpen className="h-4 w-4" />
                  </Button>
                </div>
              </div>
              <div>
                <label className="mb-1 block text-sm font-medium text-ui-body">{t.instance.selectEngine}</label>
                <SelectInput className="w-full"
                  value={newInst.engineId || defaultEngineId || ''}
                  onChange={e => setNewInst({ ...newInst, engineId: e.target.value })}
                >
                  <option value="">{t.instance.sysPath}</option>
                  {engines.map(engine => <option key={engine.id} value={engine.id}>{engine.name}</option>)}
                </SelectInput>
              </div>
              <div>
                <label className="mb-1 block text-sm font-medium text-ui-body">{t.instance.port}</label>
                <TextInput
                  type="number"
                  min={1}
                  max={65535}
                  value={newInst.port}
                  onChange={e => {
                    const port = parseInt(e.target.value) || 8080
                    setNewInst({ ...newInst, port })
                    schedulePortCheck(port)
                  }}
                />
              </div>
              {portStatus && (
                <p className={`text-xs ${portStatus === labels.portAvailable ? 'text-ui-positive' : portStatus === labels.checkingPort ? 'text-ui-info' : 'text-ui-danger'}`}>
                  {portStatus}
                </p>
              )}
            </div>
            <div className="flex gap-3 border-t border-ui-line px-6 py-4">
              <Button onClick={() => setShowCreateModal(false)} variant="subtle" className="flex-1">{t.instance.cancelCreate}</Button>
              <Button onClick={handleCreate} disabled={!newInst.modelId} variant="primary" className="flex-1">{t.instance.create}</Button>
            </div>
          </Surface>
        </div>
      )}

      {showCreatePicker && (
        <InstanceModelPicker
          models={models}
          modelDirs={modelDirs}
          collapsed={pickerCollapsed}
          onToggle={(path) => setPickerCollapsed(current => {
            const next = new Set(current)
            if (next.has(path)) next.delete(path)
            else next.add(path)
            return next
          })}
          onPick={(model, mmprojPath) => {
            setNewInst(current => ({ ...current, modelId: model.id, modelPath: model.path, mmprojPath }))
            setShowCreatePicker(false)
          }}
          onClose={() => setShowCreatePicker(false)}
        />
      )}

      {showCmdModal && <CommandPreviewModal
        command={cmdArgs} shell={exportShell} copied={copyFeedback}
        onCopy={() => { void handleCopyCommand(cmdRaw) }}
        onClose={() => setShowCmdModal('')}
        onStart={() => { const inst = instances.find(x => x.id === showCmdModal); if (inst) void startInstance(inst.id).catch(() => {}); setShowCmdModal('') }}
      />}

      {commandError && <CommandFeedbackModal
        title={labels.commandErrorTitle} message={commandError.message} closeLabel={t.appShell.close}
        recoveryLabel={commandError.missingEngine ? labels.reselectEngine : undefined}
        onClose={() => setCommandError(null)}
        onRecover={commandError.missingEngine ? () => { setSelectedInstanceId(commandError.instanceId); setEnginePickerForId(commandError.instanceId); setCommandError(null) } : undefined}
      />}

      {enginePickerForId && (() => {
        const inst = instances.find(i => i.id === enginePickerForId)
        if (!inst) return null
        return (
          <div className="fixed inset-0 z-50 flex items-center justify-center bg-ui-scrim p-4">
            <Surface className="w-full max-w-sm overflow-hidden">
              <div className="flex items-center justify-between border-b border-ui-line bg-ui-inset px-6 py-4">
                <h3 className="min-w-0 truncate text-lg font-semibold text-ui-ink" title={`${t.instance.selectEngine} - ${inst.name}`}>{t.instance.selectEngine} - {inst.name}</h3>
                <Button onClick={() => setEnginePickerForId('')} variant="subtle" size="icon" className="shrink-0" aria-label={t.appShell.close}><X className="h-5 w-5" /></Button>
              </div>
              <div className="max-h-64 overflow-y-auto p-2">
                {engines.map(engine => (
                  <button
                    key={engine.id}
                    onClick={() => {
                      const state = useAppStore.getState()
                      const idx = state.instances.findIndex(i => i.id === enginePickerForId)
                      if (idx < 0) { setEnginePickerForId(''); return }
                      const next = [...state.instances]
                      next[idx] = { ...next[idx], config: configForEngine(next[idx].config, engine) }
                      useAppStore.setState({ instances: next })
                      void useAppStore.getState().saveConfig().catch(() => {})
                      setEnginePickerForId('')
                    }}
                    className={`w-full break-all rounded px-4 py-2 text-left text-sm hover:bg-ui-hover ${pathsEqual(inst.config.engine_id || defaultEngineId || '', engine.id) ? 'bg-ui-selected text-ui-action-ink' : ''}`}
                  >
                    {engine.name} <span className="text-xs text-ui-muted">({engine.backend})</span>
                  </button>
                ))}
              </div>
            </Surface>
          </div>
        )
      })()}
    </div>
  )
}

export default InstanceManager
