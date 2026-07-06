import { useState, useRef, useEffect, useMemo } from 'react'
import { Play, Square, Plus, Trash2, Copy, Globe, X, Terminal, Settings, File, Image, FolderOpen, ChevronRight, ChevronDown, Wifi, ArrowUp, ArrowDown, Pencil, Search } from 'lucide-react'
import { useAppStore, defaultInstanceConfig } from '../store'
import { formatStartupCommand } from '../store'
import { invoke } from '@tauri-apps/api/core'
import { confirm } from '@tauri-apps/plugin-dialog'
import { useI18n } from '../i18n'
import { normalizePath, pathJoin, pathDirname } from '../utils/path'
import type { Instance, ModelInfo } from '../store/types'
import { Badge, Button, EmptyState, MetricCard, SelectInput, Surface, TextInput } from './ui'

type TestState = 'checking' | `ok:${string}` | `error:${string}`

const InstanceManager = () => {
  const instances = useAppStore(s => s.instances)
  const addInstance = useAppStore(s => s.addInstance)
  const deleteInstance = useAppStore(s => s.deleteInstance)
  const startInstance = useAppStore(s => s.startInstance)
  const stopInstance = useAppStore(s => s.stopInstance)
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

  const [showCreateModal, setShowCreateModal] = useState(false)
  const [showCmdModal, setShowCmdModal] = useState('')
  const [cmdText, setCmdText] = useState('')
  const [cmdRaw, setCmdRaw] = useState('')
  const [showCreatePicker, setShowCreatePicker] = useState(false)
  const [pickerCollapsed, setPickerCollapsed] = useState<Set<string>>(new Set())
  const [newInst, setNewInst] = useState({ name: '', modelId: '', modelPath: '', mmprojPath: '', port: 8080, engineId: '' })
  const [testResults, setTestResults] = useState<Record<string, TestState>>({})
  const [editingId, setEditingId] = useState('')
  const [editName, setEditName] = useState('')
  const editingCanceledRef = useRef(false)
  const [enginePickerForId, setEnginePickerForId] = useState('')
  const [portStatus, setPortStatus] = useState('')
  const [search, setSearch] = useState('')
  const [statusFilter, setStatusFilter] = useState<'all' | 'running' | 'stopped'>('all')
  const [engineFilter, setEngineFilter] = useState('all')
  const portCheckTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const mountedRef = useRef(true)

  useEffect(() => {
    return () => {
      mountedRef.current = false
      if (portCheckTimerRef.current) clearTimeout(portCheckTimerRef.current)
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

  const engineNameFor = (inst: Instance) =>
    engines.find(e => e.id === (inst.config.engine_id || defaultEngineId || ''))?.name
    || engines.find(e => e.id === defaultEngineId)?.name
    || engines[0]?.name
    || t.instance.sysPath

  const statusText = (inst: Instance) =>
    inst.status === 'running' ? t.instance.running : inst.status === 'stopped' ? t.instance.stopped : t.instance.error

  const healthText = (inst: Instance) => {
    if (inst.status === 'stopped') return lang === 'zh-CN' ? '\u79bb\u7ebf' : 'Offline'
    if (inst.status === 'error') return 'Error'
    if (inst.healthCheck === 'ok') return lang === 'zh-CN' ? '\u6b63\u5e38' : 'Healthy'
    if (inst.healthCheck === 'fail') return 'Fail'
    return lang === 'zh-CN' ? '\u68c0\u67e5\u4e2d' : 'Pending'
  }

  const healthDotClass = (inst: Instance) => {
    if (inst.status === 'stopped') return 'bg-slate-400'
    if (inst.status === 'error') return 'bg-rose-500'
    if (inst.healthCheck === 'ok') return 'bg-emerald-500'
    if (inst.healthCheck === 'fail') return 'bg-rose-500'
    return 'bg-amber-500'
  }

  const filteredInstances = useMemo(() => {
    const query = search.trim().toLowerCase()
    return instances.filter(inst => {
      if (statusFilter === 'running' && inst.status !== 'running') return false
      if (statusFilter === 'stopped' && inst.status === 'running') return false
      if (engineFilter !== 'all' && (inst.config.engine_id || defaultEngineId || '') !== engineFilter) return false
      if (!query) return true
      return inst.name.toLowerCase().includes(query)
        || inst.model.toLowerCase().includes(query)
        || engineNameFor(inst).toLowerCase().includes(query)
        || String(inst.config.port).includes(query)
    })
  }, [instances, search, statusFilter, engineFilter, defaultEngineId, engines])

  const runningCount = instances.filter(inst => inst.status === 'running').length
  const stoppedCount = instances.filter(inst => inst.status === 'stopped').length
  const erroredCount = instances.filter(inst => inst.status === 'error').length
  const autoStartCount = instances.filter(inst => inst.config.auto_start).length
  const labels = {
    instances: lang === 'zh-CN' ? '\u5b9e\u4f8b' : 'instances',
    running: lang === 'zh-CN' ? '\u8fd0\u884c\u4e2d' : 'running',
    runningTitle: lang === 'zh-CN' ? '\u8fd0\u884c\u4e2d' : 'Running',
    stoppedTitle: lang === 'zh-CN' ? '\u5df2\u505c\u6b62' : 'Stopped',
    errored: lang === 'zh-CN' ? '\u5f02\u5e38' : 'Errored',
    autoStart: lang === 'zh-CN' ? '\u81ea\u52a8\u542f\u52a8' : 'Auto Start',
    all: lang === 'zh-CN' ? '\u5168\u90e8' : 'All',
    offline: lang === 'zh-CN' ? '\u79bb\u7ebf' : 'Offline',
    healthy: lang === 'zh-CN' ? '\u6b63\u5e38' : 'Healthy',
    pending: lang === 'zh-CN' ? '\u68c0\u67e5\u4e2d' : 'Pending',
    checkingPort: lang === 'zh-CN' ? '\u68c0\u67e5\u7aef\u53e3\u4e2d...' : 'Checking port...',
    portAvailable: lang === 'zh-CN' ? '\u7aef\u53e3\u53ef\u7528' : 'Port is available',
    portInUse: lang === 'zh-CN' ? '\u7aef\u53e3\u5df2\u88ab\u5360\u7528' : 'Port is already in use',
    noEnginePrefix: lang === 'zh-CN' ? '\u5c1a\u672a\u68c0\u6d4b\u5230\u5f15\u64ce\uff0c\u8bf7\u5148\u524d\u5f80' : 'No engine detected yet. Please open',
    noEngineSuffix: lang === 'zh-CN' ? '\u5e76\u6dfb\u52a0 llama-server\u3002' : 'and add a llama-server installation.',
    searchPlaceholder: lang === 'zh-CN' ? '\u641c\u7d22\u5b9e\u4f8b\u3001\u6a21\u578b\u3001\u5f15\u64ce\u3001\u7aef\u53e3...' : 'Search instances, model, engine, port...',
    allEngines: lang === 'zh-CN' ? '\u6240\u6709\u5f15\u64ce' : 'All Engines',
    name: lang === 'zh-CN' ? '\u540d\u79f0' : 'Name',
    status: lang === 'zh-CN' ? '\u72b6\u6001' : 'Status',
    health: lang === 'zh-CN' ? '\u5065\u5eb7' : 'Health',
    uptime: lang === 'zh-CN' ? '\u8fd0\u884c\u65f6\u957f' : 'Uptime',
    actions: lang === 'zh-CN' ? '\u64cd\u4f5c' : 'Actions',
    rename: lang === 'zh-CN' ? '\u91cd\u547d\u540d' : 'Rename',
    moveUp: lang === 'zh-CN' ? '\u4e0a\u79fb' : 'Move up',
    moveDown: lang === 'zh-CN' ? '\u4e0b\u79fb' : 'Move down',
    checking: lang === 'zh-CN' ? '\u6d4b\u8bd5\u4e2d...' : 'Checking...',
  }

  const schedulePortCheck = (port: number) => {
    if (portCheckTimerRef.current) clearTimeout(portCheckTimerRef.current)
    setPortStatus(labels.checkingPort)
    portCheckTimerRef.current = setTimeout(() => {
      invoke<boolean>('check_port', { port })
        .then(free => setPortStatus(free ? labels.portAvailable : labels.portInUse))
        .catch(() => setPortStatus(''))
    }, 300)
  }

  const handleCreate = async () => {
    const model = models.find(m => m.id === newInst.modelId)
    if (!model || !newInst.modelPath) return

    try {
      const portFree = await invoke<boolean>('check_port', { port: newInst.port })
      if (!portFree) {
        setPortStatus(labels.portInUse)
        return
      }
    } catch {
      // proceed
    }

    const id = crypto.randomUUID()
    const config = defaultInstanceConfig()
    config.id = id
    config.name = newInst.name || model.name.replace('.gguf', '')
    config.model_path = newInst.modelPath
    config.mmproj_path = newInst.mmprojPath
    config.port = newInst.port
    config.host = '127.0.0.1'
    config.engine_id = newInst.engineId || defaultEngineId || ''

    if (!defaultEngineId && engines[0]) useAppStore.setState({ defaultEngineId: engines[0].id })

    addInstance({
      id,
      name: config.name,
      status: 'stopped',
      model: model.name,
      port: newInst.port,
      healthCheck: 'pending',
      config,
    })

    setShowCreateModal(false)
    setNewInst({ name: '', modelId: '', modelPath: '', mmprojPath: '', port: newInst.port + 1, engineId: newInst.engineId })
    setPortStatus('')
  }

  const handleDelete = async (id: string) => {
    if (!await confirm(t.instance.confirmDelete, { title: t.instance.delete, kind: 'warning' })) return
    deleteInstance(id)
    useAppStore.getState().saveConfig()
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
    const inst = instances.find(i => i.id === id)
    if (!inst) return
    const engine = engines.find(e => e.id === inst.config.engine_id) || engines.find(e => e.id === defaultEngineId) || engines[0]
    if (!engine) return
    try {
      const cmd = await generateCommand(inst.config, engine.exe)
      const raw = cmd.map(arg => arg.includes(' ') ? `"${arg}"` : arg).join(' ')
      setCmdRaw(raw)
      setCmdText(formatStartupCommand(raw))
      setShowCmdModal(id)
    } catch (e) {
      console.error(e)
    }
  }

  const handleTestConnection = async (inst: Instance) => {
    setTestResults(state => ({ ...state, [inst.id]: 'checking' }))
    try {
      const result = await invoke('test_connection', { host: inst.config.host, port: inst.config.port, apiKey: inst.config.api_key || null })
      if (!mountedRef.current) return
      setTestResults(state => ({ ...state, [inst.id]: `ok:${String(result)}` }))
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
    useAppStore.getState().saveConfig()
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
      return <span className="text-xs text-blue-500">{labels.checking}</span>
    }
    if (status.startsWith('ok:')) {
      return <span className="text-xs text-emerald-500">{status.slice(3)}</span>
    }
    return <span className="text-xs text-rose-500">{status.slice(6)}</span>
  }

  return (
    <div className="space-y-5">
      {engines.length === 0 && (
        <div className="rounded-2xl border border-amber-500/20 bg-amber-500/10 px-4 py-3 text-sm text-amber-200">
          {labels.noEnginePrefix}
          {' '}
          <button onClick={() => setActiveTab('engine')} className="font-medium underline underline-offset-4">
            {t.engineMgr.scan}
          </button>
          {' '}
          {labels.noEngineSuffix}
        </div>
      )}

      <Surface as="section">
        <div className="flex flex-wrap items-start justify-between gap-4 px-5 py-5">
          <div>
            <h2 className="text-2xl font-semibold text-slate-50">{t.instance.title}</h2>
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-slate-400">
              <Badge tone="slate">
                {instances.length} {labels.instances}
              </Badge>
              <Badge tone="emerald">
                {runningCount} {labels.running}
              </Badge>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              onClick={() => setShowCreateModal(true)}
              variant="primary"
              data-guide="instance-create"
              icon={<Plus className="h-4 w-4" />}
            >
              <span>{t.instance.create}</span>
            </Button>
          </div>
        </div>
      </Surface>

      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {[
          { label: labels.runningTitle, value: runningCount, tone: 'text-emerald-300 bg-emerald-500/10 border-emerald-500/20' },
          { label: labels.stoppedTitle, value: stoppedCount, tone: 'text-slate-300 bg-slate-800 border-slate-700' },
          { label: labels.errored, value: erroredCount, tone: 'text-rose-300 bg-rose-500/10 border-rose-500/20' },
          { label: labels.autoStart, value: autoStartCount, tone: 'text-blue-300 bg-blue-500/10 border-blue-500/20' },
        ].map(card => (
          <MetricCard key={card.label} label={card.label} value={card.value} tone={card.tone} />
        ))}
      </div>

      <Surface as="section" className="overflow-hidden">
        <div className="border-b border-slate-200 px-5 py-4 dark:border-slate-800">
          <div className="flex flex-wrap items-center justify-between gap-4">
            <div className="flex flex-wrap items-center gap-2">
              {(['all', 'running', 'stopped'] as const).map(filter => (
                <Button
                  key={filter}
                  onClick={() => setStatusFilter(filter)}
                  variant={statusFilter === filter ? 'primary' : 'subtle'}
                  size="sm"
                >
                  {filter === 'all' ? labels.all : filter === 'running' ? t.instance.running : t.instance.stopped}
                </Button>
              ))}
            </div>

            <div className="flex flex-wrap items-center gap-2">
              <TextInput
                value={search}
                onChange={e => setSearch(e.target.value)}
                placeholder={labels.searchPlaceholder}
                leadingIcon={<Search className="h-4 w-4" />}
                className="w-[280px] max-w-full"
              />
              <SelectInput
                value={engineFilter}
                onChange={e => setEngineFilter(e.target.value)}
                className="min-w-[160px]"
              >
                <option value="all">{labels.allEngines}</option>
                {engines.map(engine => (
                  <option key={engine.id} value={engine.id}>{engine.name}</option>
                ))}
              </SelectInput>
            </div>
          </div>
        </div>

        {filteredInstances.length === 0 ? (
          <EmptyState icon={<Play className="h-10 w-10" />} title={t.instance.noInstances} className="rounded-none border-0 shadow-none" />
        ) : (
          <div className="overflow-x-auto">
            <table className="min-w-full text-sm">
              <thead className="bg-slate-950/60">
                <tr className="text-left text-slate-400">
                  <th className="px-5 py-3 font-medium">{labels.name}</th>
                  <th className="px-5 py-3 font-medium">{t.instance.model}</th>
                  <th className="px-5 py-3 font-medium">{t.instance.engine}</th>
                  <th className="px-5 py-3 font-medium">{labels.status}</th>
                  <th className="px-5 py-3 font-medium">{labels.health}</th>
                  <th className="px-5 py-3 font-medium">{t.instance.port}</th>
                  <th className="px-5 py-3 font-medium">{labels.uptime}</th>
                  <th className="px-5 py-3 font-medium">{labels.actions}</th>
                </tr>
              </thead>
              <tbody>
                {filteredInstances.map((inst, index) => (
                  <tr key={inst.id} className="border-t border-slate-200 align-top dark:border-slate-800">
                    <td className="px-5 py-4">
                      <div className="flex items-start gap-3">
                        <div className={`mt-2 h-2.5 w-2.5 rounded-full ${inst.status === 'running' ? 'bg-emerald-500' : inst.status === 'error' ? 'bg-rose-500' : 'bg-slate-400'}`} />
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-2">
                            {editingId === inst.id ? (
                              <input
                                type="text"
                                value={editName}
                                onChange={e => setEditName(e.target.value)}
                                onKeyDown={e => {
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
                                className="w-48 border-b border-blue-500 bg-transparent px-1 text-sm font-medium text-slate-900 outline-none dark:text-slate-100"
                              />
                            ) : (
                              <>
                                <div className="truncate font-medium text-slate-900 dark:text-slate-100">{inst.name}</div>
                                <Button
                                  onClick={() => { setEditingId(inst.id); setEditName(inst.name) }}
                                  variant="subtle"
                                  size="icon"
                                  title={labels.rename}
                                >
                                  <Pencil className="h-3.5 w-3.5" />
                                </Button>
                              </>
                            )}
                            <button
                              type="button"
                              role="switch"
                              aria-checked={!!inst.config.auto_start}
                              onClick={() => toggleAutoStart(inst)}
                              className={`relative inline-flex h-5 w-9 shrink-0 rounded-full border-2 border-transparent transition-colors duration-200 ${inst.config.auto_start ? 'bg-blue-600' : 'bg-slate-300 dark:bg-slate-700'}`}
                              title={t.instance.autoStart}
                            >
                              <span className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200 ${inst.config.auto_start ? 'translate-x-4' : 'translate-x-0'}`} />
                            </button>
                          </div>
                          <div className="mt-1 text-xs text-slate-500 dark:text-slate-400">
                            {inst.config.host}:{inst.config.port}
                          </div>
                          <div className="mt-2">{renderTestResult(inst.id)}</div>
                        </div>
                      </div>
                    </td>
                    <td className="px-5 py-4">
                      <div className="max-w-[280px] truncate text-slate-700 dark:text-slate-200" title={inst.config.model_path}>{inst.model}</div>
                    </td>
                    <td className="px-5 py-4">
                      <Button
                        onClick={() => setEnginePickerForId(inst.id)}
                        size="sm"
                      >
                        {engineNameFor(inst)}
                      </Button>
                    </td>
                    <td className="px-5 py-4">
                      <Badge tone={inst.status === 'running' ? 'emerald' : inst.status === 'error' ? 'red' : 'slate'}>
                        {statusText(inst)}
                      </Badge>
                    </td>
                    <td className="px-5 py-4">
                      <div className="flex items-center gap-2">
                        <span className={`h-2.5 w-2.5 rounded-full ${healthDotClass(inst)}`} />
                        <span className="text-slate-700 dark:text-slate-200">{healthText(inst)}</span>
                      </div>
                    </td>
                    <td className="px-5 py-4 text-slate-700 dark:text-slate-200">{inst.config.port}</td>
                    <td className="px-5 py-4 text-slate-700 dark:text-slate-200">{inst.status === 'running' ? `${t.instance.uptime} ${formatUptime(inst.startTime)}` : '--'}</td>
                    <td className="px-5 py-4">
                      <div className="flex flex-wrap items-center gap-2">
                        {inst.status !== 'running' ? (
                          <Button
                            onClick={() => startInstance(inst.id)}
                            variant="success"
                            size="sm"
                            icon={<Play className="h-3.5 w-3.5" />}
                          >
                            <span>{t.instance.start}</span>
                          </Button>
                        ) : (
                          <Button
                            onClick={() => stopInstance(inst.id)}
                            variant="danger"
                            size="sm"
                            icon={<Square className="h-3.5 w-3.5" />}
                          >
                            <span>{t.instance.stop}</span>
                          </Button>
                        )}

                        {inst.status === 'running' && (
                          <>
                            <Button
                              onClick={() => openBrowser(inst.config.host, inst.config.port)}
                              variant="subtle"
                              size="icon"
                              title={t.instance.openBrowser}
                            >
                              <Globe className="h-4 w-4" />
                            </Button>
                            <Button
                              onClick={() => handleTestConnection(inst)}
                              variant="subtle"
                              size="icon"
                              title={t.instance.testConnection}
                            >
                              <Wifi className="h-4 w-4" />
                            </Button>
                          </>
                        )}

                        <Button
                          onClick={() => handleShowCommand(inst.id)}
                          variant="subtle"
                          size="icon"
                          title={t.instance.genCommand}
                        >
                          <Terminal className="h-4 w-4" />
                        </Button>
                        <Button
                          onClick={() => { setActiveConfigInstanceId(inst.id); setActiveTab('config') }}
                          variant="subtle"
                          size="icon"
                          title={t.instance.configParams}
                          data-guide="instance-config"
                        >
                          <Settings className="h-4 w-4" />
                        </Button>
                        <Button
                          onClick={() => handleDelete(inst.id)}
                          variant="danger"
                          size="icon"
                          title={t.instance.delete}
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                        <div className="flex items-center gap-1">
                          <Button
                            onClick={() => moveInstance(inst.id, 'up')}
                            disabled={index === 0}
                            variant="subtle"
                            size="icon"
                            title={labels.moveUp}
                          >
                            <ArrowUp className="h-3.5 w-3.5" />
                          </Button>
                          <Button
                            onClick={() => moveInstance(inst.id, 'down')}
                            disabled={index === filteredInstances.length - 1}
                            variant="subtle"
                            size="icon"
                            title={labels.moveDown}
                          >
                            <ArrowDown className="h-3.5 w-3.5" />
                          </Button>
                        </div>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Surface>

      {showCreateModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
          <Surface className="w-full max-w-md overflow-hidden">
            <div className="flex items-center justify-between border-b border-slate-800 bg-slate-950/90 px-6 py-4">
              <h3 className="text-lg font-semibold text-slate-50">{t.instance.newInstance}</h3>
              <Button onClick={() => setShowCreateModal(false)} variant="subtle" size="icon" aria-label="Close"><X className="h-5 w-5" /></Button>
            </div>
            <div className="space-y-4 p-6">
              <div>
                <label className="mb-1 block text-sm font-medium text-slate-300">{t.instance.name}</label>
                <TextInput
                  type="text"
                  value={newInst.name}
                  onChange={e => setNewInst({ ...newInst, name: e.target.value })}
                  placeholder={t.instance.namePlaceholder}
                />
              </div>
              <div>
                <label className="mb-1 block text-sm font-medium text-slate-300">{t.instance.selectModel}</label>
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
                <label className="mb-1 block text-sm font-medium text-slate-300">{t.instance.selectEngine}</label>
                <SelectInput
                  value={newInst.engineId || defaultEngineId || ''}
                  onChange={e => setNewInst({ ...newInst, engineId: e.target.value })}
                >
                  <option value="">{t.instance.sysPath}</option>
                  {engines.map(engine => <option key={engine.id} value={engine.id}>{engine.name}</option>)}
                </SelectInput>
              </div>
              <div>
                <label className="mb-1 block text-sm font-medium text-slate-300">{t.instance.port}</label>
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
                <p className={`text-xs ${portStatus === labels.portAvailable ? 'text-emerald-400' : portStatus === labels.checkingPort ? 'text-blue-400' : 'text-rose-400'}`}>
                  {portStatus}
                </p>
              )}
            </div>
            <div className="flex gap-3 border-t border-slate-800 px-6 py-4">
              <Button onClick={() => setShowCreateModal(false)} variant="subtle" className="flex-1">{t.instance.cancelCreate}</Button>
              <Button onClick={handleCreate} disabled={!newInst.modelId} variant="primary" className="flex-1">{t.instance.create}</Button>
            </div>
          </Surface>
        </div>
      )}

      {showCreatePicker && (
        <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 p-4">
          <Surface className="flex max-h-[80vh] w-full max-w-2xl flex-col overflow-hidden">
            <div className="flex items-center justify-between border-b border-slate-800 bg-slate-950/90 px-6 py-4">
              <h3 className="text-lg font-semibold text-slate-50">{t.modelRepo.selectFromRepo}</h3>
              <Button onClick={() => setShowCreatePicker(false)} variant="subtle" size="icon" aria-label="Close"><X className="h-5 w-5" /></Button>
            </div>
            <div className="flex-1 overflow-y-auto p-4">
              {(function TreeRenderer() {
                interface TNode { name: string; path: string; isDir: boolean; children?: Map<string, TNode>; model?: ModelInfo }
                function buildTree(rootDir: string): TNode {
                  const normDir = normalizePath(rootDir)
                  const root: TNode = { name: rootDir, path: normDir, isDir: true, children: new Map() }
                  const normRoot = normDir.toLowerCase()
                  for (const model of models) {
                    const path = normalizePath(model.path).toLowerCase()
                    if (!path.startsWith(normRoot)) continue
                    const rel = normalizePath(model.path.substring(rootDir.length)).replace(/^\/+/, '')
                    if (!rel) continue
                    const parts = rel.split('/')
                    let cur = root
                    for (let i = 0; i < parts.length; i++) {
                      if (i === parts.length - 1) {
                        cur.children!.set(parts[i], { name: parts[i], path: model.path, isDir: false, model })
                      } else {
                        if (!cur.children!.has(parts[i])) {
                          cur.children!.set(parts[i], { name: parts[i], path: pathJoin(cur.path, parts[i]), isDir: true, children: new Map() })
                        }
                        cur = cur.children!.get(parts[i])!
                      }
                    }
                  }
                  return root
                }

                const togglePath = (path: string) => {
                  const next = new Set(pickerCollapsed)
                  if (next.has(path)) next.delete(path)
                  else next.add(path)
                  setPickerCollapsed(next)
                }

                const pickForCreate = (model: ModelInfo) => {
                  const dir = pathDirname(model.path)
                  const mmproj = models.find(entry => pathDirname(entry.path) === dir && entry.file_type === 'mmproj')
                  setNewInst(prev => ({ ...prev, modelId: model.id, modelPath: model.path, mmprojPath: mmproj?.path || '' }))
                  setShowCreatePicker(false)
                }

                function renderNode(node: TNode, depth: number): any {
                  if (node.isDir) {
                    const collapsed = pickerCollapsed.has(node.path)
                    return (
                      <div key={node.path}>
                        <button
                          onClick={() => togglePath(node.path)}
                          style={{ paddingLeft: `${depth * 12 + 4}px` }}
                          className="flex w-full items-center gap-1.5 rounded py-1 text-left text-xs hover:bg-slate-100 dark:hover:bg-slate-800"
                        >
                          {collapsed ? <ChevronRight className="h-3 w-3 shrink-0 text-slate-400" /> : <ChevronDown className="h-3 w-3 shrink-0 text-slate-400" />}
                          <FolderOpen className="h-3 w-3 shrink-0 text-amber-500" />
                          <span className="truncate font-medium">{node.name}</span>
                        </button>
                        {!collapsed && node.children && [...node.children.values()]
                          .sort((a, b) => a.isDir !== b.isDir ? (a.isDir ? -1 : 1) : a.name.localeCompare(b.name))
                          .map(child => renderNode(child, depth + 1))}
                      </div>
                    )
                  }

                  const model = node.model!
                  if (model.file_type === 'mmproj') {
                    return (
                      <div key={node.path} style={{ paddingLeft: `${depth * 12 + 20}px` }} className="flex items-center gap-2 py-1 pr-2 text-xs text-slate-500">
                        <Image className="h-3 w-3 shrink-0 text-purple-500" />
                        <span className="truncate flex-1">{model.name}</span>
                        <span className="shrink-0 text-xs text-purple-400">{t.modelRepo.typeMmprojShort}</span>
                      </div>
                    )
                  }

                  return (
                    <button
                      key={node.path}
                      onClick={() => pickForCreate(model)}
                      style={{ paddingLeft: `${depth * 12 + 20}px` }}
                      className="flex w-full items-center gap-2 rounded py-1 pr-2 text-left text-xs hover:bg-blue-50 dark:hover:bg-blue-950/30"
                    >
                      <File className="h-3 w-3 shrink-0 text-blue-500" />
                      <span className="truncate flex-1">{model.name}</span>
                      <span className="shrink-0 text-slate-400">{model.quant_type || ''}</span>
                    </button>
                  )
                }

                return modelDirs.map(dir => buildTree(dir)).map(tree => renderNode(tree, 0))
              })()}
            </div>
          </Surface>
        </div>
      )}

      {showCmdModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
          <Surface className="w-full max-w-3xl p-6">
            <div className="mb-4 flex items-center justify-between">
              <h3 className="text-lg font-semibold text-slate-50">{t.instance.genCommandTitle}</h3>
              <Button onClick={() => setShowCmdModal('')} variant="subtle" size="icon" aria-label="Close"><X className="h-5 w-5" /></Button>
            </div>
            <pre className="max-h-80 overflow-y-auto overflow-x-auto rounded-lg bg-slate-950 p-4 text-sm text-slate-200">{cmdText}</pre>
            {copyFeedback && <div className="mt-3 text-center text-xs font-medium text-emerald-500">{t.common.copySuccess}</div>}
            <div className="mt-4 flex gap-2">
              <Button onClick={() => handleCopyCommand(cmdRaw)} variant="primary" icon={<Copy className="h-4 w-4" />}>{t.instance.copyClipboard}</Button>
              <Button onClick={() => { const inst = instances.find(x => x.id === showCmdModal); if (inst) startInstance(inst.id); setShowCmdModal('') }} variant="success">{t.instance.directStart}</Button>
            </div>
          </Surface>
        </div>
      )}

      {enginePickerForId && (() => {
        const inst = instances.find(i => i.id === enginePickerForId)
        if (!inst) return null
        return (
          <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
            <Surface className="w-full max-w-sm overflow-hidden">
              <div className="flex items-center justify-between border-b border-slate-800 bg-slate-950/90 px-6 py-4">
                <h3 className="text-lg font-semibold text-slate-50">{t.instance.selectEngine} - {inst.name}</h3>
                <Button onClick={() => setEnginePickerForId('')} variant="subtle" size="icon" aria-label="Close"><X className="h-5 w-5" /></Button>
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
                      next[idx] = { ...next[idx], config: { ...next[idx].config, engine_id: engine.id } }
                      useAppStore.setState({ instances: next })
                      useAppStore.getState().saveConfig()
                      setEnginePickerForId('')
                    }}
                    className={`w-full rounded px-4 py-2 text-left text-sm hover:bg-slate-100 dark:hover:bg-slate-800 ${(inst.config.engine_id || defaultEngineId) === engine.id ? 'bg-blue-50 text-blue-600 dark:bg-blue-950/30 dark:text-blue-300' : ''}`}
                  >
                    {engine.name} <span className="text-xs text-slate-400">({engine.backend})</span>
                  </button>
                ))}
                <button
                  onClick={() => {
                    const state = useAppStore.getState()
                    const idx = state.instances.findIndex(i => i.id === enginePickerForId)
                    if (idx < 0) { setEnginePickerForId(''); return }
                    const next = [...state.instances]
                    next[idx] = { ...next[idx], config: { ...next[idx].config, engine_id: '' } }
                    useAppStore.setState({ instances: next })
                    useAppStore.getState().saveConfig()
                    setEnginePickerForId('')
                  }}
                  className={`w-full border-t border-slate-200 px-4 py-2 text-left text-sm hover:bg-slate-100 dark:border-slate-800 dark:hover:bg-slate-800 ${!inst.config.engine_id ? 'bg-blue-50 text-blue-600 dark:bg-blue-950/30 dark:text-blue-300' : ''}`}
                >
                  {t.instance.sysPath}
                </button>
              </div>
            </Surface>
          </div>
        )
      })()}
    </div>
  )
}

export default InstanceManager
