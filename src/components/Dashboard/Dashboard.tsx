import { useMemo, useState } from 'react'
import { Activity, AlertTriangle, ArrowUpRight, BarChart3, Download, Gauge, Grid3X3, List, Play, RefreshCw, Search, Server, Settings2, Square, Zap } from 'lucide-react'
import { useAppStore } from '../../store'
import { useI18n } from '../../i18n'
import { Badge, Button, InsetSurface, MetricCard, SectionHeader, SelectInput, Surface, TextInput } from '../ui'

type StatusScope = 'all' | 'running' | 'stopped'
type SortMode = 'name' | 'port' | 'uptime'
type ViewMode = 'table' | 'cards'

function formatUptime(startTime?: number) {
  if (!startTime) return '--'
  const ms = Date.now() - startTime
  if (ms <= 0) return '--'
  const totalMinutes = Math.floor(ms / 60000)
  const hours = Math.floor(totalMinutes / 60)
  const minutes = totalMinutes % 60
  if (hours <= 0) return `${minutes}m`
  if (hours < 24) return `${hours}h ${minutes}m`
  const days = Math.floor(hours / 24)
  return `${days}d ${hours % 24}h`
}

function percentText(value?: number | null) {
  return `${Math.round(value ?? 0)}%`
}

function formatBytes(value?: number | null) {
  const bytes = value ?? 0
  if (bytes <= 0) return '--'
  const gb = bytes / 1024 / 1024 / 1024
  if (gb >= 1) return `${gb.toFixed(1)} GB`
  return `${(bytes / 1024 / 1024).toFixed(0)} MB`
}

function formatRate(value?: number | null) {
  const bytes = value ?? 0
  if (bytes <= 0) return '--'
  return `${formatBytes(bytes)}/s`
}

function usageBarClass(kind: 'cpu' | 'gpu' | 'memory') {
  switch (kind) {
    case 'cpu':
      return 'bg-blue-500'
    case 'gpu':
      return 'bg-emerald-500'
    case 'memory':
      return 'bg-violet-500'
  }
}

function downloadTone(status: string): 'slate' | 'blue' | 'emerald' | 'amber' | 'red' | 'violet' {
  if (status === 'active') return 'blue'
  if (status === 'completed') return 'emerald'
  if (status === 'error') return 'red'
  if (status === 'paused' || status === 'pausing') return 'amber'
  if (status === 'queued') return 'violet'
  return 'slate'
}

export default function Dashboard() {
  const { t, lang } = useI18n()
  const zh = lang === 'zh-CN'
  const instances = useAppStore(state => state.instances)
  const models = useAppStore(state => state.models)
  const engines = useAppStore(state => state.engines)
  const sysMetrics = useAppStore(state => state.sysMetrics)
  const defaultEngineId = useAppStore(state => state.defaultEngineId)
  const loadInitialData = useAppStore(state => state.loadInitialData)
  const setActiveTab = useAppStore(state => state.setActiveTab)
  const startInstance = useAppStore(state => state.startInstance)
  const stopInstance = useAppStore(state => state.stopInstance)
  const openBrowser = useAppStore(state => state.openBrowser)
  const setActiveConfigInstanceId = useAppStore(state => state.setActiveConfigInstanceId)
  const downloadTasks = useAppStore(state => state.downloadTasks)
  const downloadQueue = useAppStore(state => state.downloadQueue)

  const [statusScope, setStatusScope] = useState<StatusScope>('running')
  const [search, setSearch] = useState('')
  const [engineFilter, setEngineFilter] = useState('all')
  const [sortMode, setSortMode] = useState<SortMode>('name')
  const [viewMode, setViewMode] = useState<ViewMode>('table')

  const runningCount = instances.filter(instance => instance.status === 'running').length
  const stoppedCount = instances.filter(instance => instance.status !== 'running').length
  const erroredCount = instances.filter(instance => instance.status === 'error').length
  const healthyCount = instances.filter(instance => instance.status === 'running' && instance.healthCheck === 'ok').length
  const attentionCount = instances.filter(instance => instance.status === 'error' || (instance.status === 'running' && instance.healthCheck === 'fail')).length
  const autoStartCount = instances.filter(instance => instance.config.auto_start).length
  const modelCount = models.filter(model => !model.is_shard && model.file_type === 'model').length
  const downloadItems = Object.values(downloadTasks)
  const activeDownloadCount = downloadItems.filter(task => task.status === 'active').length
  const queuedDownloadCount = downloadItems.filter(task => task.status === 'queued').length
  const failedDownloadCount = downloadItems.filter(task => task.status === 'error').length
  const transferBytes = downloadItems.reduce((total, task) => total + (task.speed || 0), 0)

  const labels = {
    title: t.dashboard?.title || (zh ? '\u4eea\u8868\u76d8' : 'Dashboard'),
    workspace: zh ? '\u672c\u5730\u5de5\u4f5c\u533a' : 'Local Workspace',
    healthy: zh ? '\u72b6\u6001\u6b63\u5e38' : 'Healthy',
    subtitle: zh ? '\u7edf\u4e00\u67e5\u770b\u5b9e\u4f8b\u3001\u8d44\u6e90\u538b\u529b\u548c\u5feb\u901f\u64cd\u4f5c\u3002' : 'Monitor instances, resource pressure, and quick actions in one place.',
    newInstance: zh ? '\u65b0\u5efa\u5b9e\u4f8b' : 'New Instance',
    refresh: zh ? '\u5237\u65b0' : 'Refresh',
    config: zh ? '\u914d\u7f6e' : 'Config',
    running: zh ? '\u8fd0\u884c\u4e2d' : 'Running',
    stopped: zh ? '\u5df2\u505c\u6b62' : 'Stopped',
    all: zh ? '\u5168\u90e8' : 'All',
    models: zh ? '\u6a21\u578b' : 'Models',
    engines: zh ? '\u5f15\u64ce' : 'Engines',
    instances: zh ? '\u5b9e\u4f8b' : 'Instances',
    systemSummary: zh ? '\u7cfb\u7edf\u6458\u8981' : 'System Summary',
    downloads: zh ? '\u4e0b\u8f7d\u4efb\u52a1' : 'Downloads',
    activeDownloads: zh ? '\u4f20\u8f93\u4e2d' : 'Active Transfers',
    queued: zh ? '\u6392\u961f' : 'Queued',
    failed: zh ? '\u5931\u8d25' : 'Failed',
    transfer: zh ? '\u4f20\u8f93' : 'Transfer',
    queueDepth: zh ? '\u961f\u5217\u6df1\u5ea6' : 'Queue Depth',
    process: zh ? '\u8fdb\u7a0b' : 'Process',
    system: zh ? '\u7cfb\u7edf' : 'System',
    memory: zh ? '\u5185\u5b58' : 'Memory',
    uptime: zh ? '\u8fd0\u884c\u65f6\u95f4' : 'Uptime',
    waitingMetrics: zh ? '\u7b49\u5f85\u7cfb\u7edf\u6307\u6807...' : 'Waiting for metrics...',
    instanceList: zh ? '\u5b9e\u4f8b\u5217\u8868' : 'Instance List',
    listDesc: zh ? '\u6309\u72b6\u6001\u3001\u5f15\u64ce\u548c\u5173\u952e\u5b57\u7b5b\u9009\uff0c\u5e76\u76f4\u63a5\u6267\u884c\u5e38\u7528\u64cd\u4f5c\u3002' : 'Filter by status, engine, and keyword, then run common actions directly.',
    searchPlaceholder: zh ? '\u641c\u7d22\u5b9e\u4f8b\u3001\u6a21\u578b\u3001\u7aef\u53e3...' : 'Search instances, model, port...',
    allEngines: zh ? '\u5168\u90e8\u5f15\u64ce' : 'All Engines',
    sortByName: zh ? '\u6309\u540d\u79f0\u6392\u5e8f' : 'Sort by Name',
    sortByPort: zh ? '\u6309\u7aef\u53e3\u6392\u5e8f' : 'Sort by Port',
    sortByUptime: zh ? '\u6309\u8fd0\u884c\u65f6\u95f4\u6392\u5e8f' : 'Sort by Uptime',
    name: zh ? '\u540d\u79f0' : 'Name',
    model: zh ? '\u6a21\u578b' : 'Model',
    engine: zh ? '\u5f15\u64ce' : 'Engine',
    status: zh ? '\u72b6\u6001' : 'Status',
    health: zh ? '\u5065\u5eb7' : 'Health',
    port: zh ? '\u7aef\u53e3' : 'Port',
    actions: zh ? '\u64cd\u4f5c' : 'Actions',
    quickActions: zh ? '\u5feb\u901f\u64cd\u4f5c' : 'Quick Actions',
    openDownloads: zh ? '\u6253\u5f00\u4e0b\u8f7d' : 'Open Downloads',
    openLogs: zh ? '\u67e5\u770b\u65e5\u5fd7' : 'View Logs',
    openPerformance: zh ? '\u6027\u80fd\u76d1\u63a7' : 'Performance',
    recentDownloads: zh ? '\u8fd1\u671f\u4e0b\u8f7d' : 'Recent Downloads',
    noDownloads: zh ? '\u6682\u65e0\u4e0b\u8f7d\u4efb\u52a1' : 'No download tasks yet',
    statusContext: zh ? '\u72b6\u6001\u4e0a\u4e0b\u6587' : 'Status Context',
    healthyRunning: zh ? '\u5065\u5eb7\u8fd0\u884c' : 'Healthy Running',
    needsAttention: zh ? '\u9700\u5173\u6ce8' : 'Needs Attention',
    autoStart: zh ? '\u81ea\u542f\u52a8' : 'Auto Start',
    compactCards: zh ? '\u5361\u7247\u89c6\u56fe' : 'Card View',
    tableView: zh ? '\u8868\u683c\u89c6\u56fe' : 'Table View',
    workload: zh ? '\u5de5\u4f5c\u8d1f\u8f7d' : 'Workload',
    resources: zh ? '\u8d44\u6e90' : 'Resources',
    endpoint: zh ? '\u7aef\u70b9' : 'Endpoint',
    noEngine: zh ? '\u672a\u6307\u5b9a\u5f15\u64ce' : 'No engine',
    noMatches: zh ? '\u5f53\u524d\u7b5b\u9009\u6761\u4ef6\u4e0b\u6ca1\u6709\u5b9e\u4f8b' : 'No instances match the current filters',
    healthyStatus: zh ? '\u5065\u5eb7' : 'Healthy',
    offline: zh ? '\u79bb\u7ebf' : 'Offline',
    pending: zh ? '\u68c0\u67e5\u4e2d' : 'Pending',
    open: zh ? '\u6253\u5f00' : 'Open',
  }

  const engineNameFor = (instance: typeof instances[number]) =>
    engines.find(engine => engine.id === (instance.config.engine_id || defaultEngineId || ''))?.name
    || engines.find(engine => engine.id === defaultEngineId)?.name
    || engines[0]?.name
    || labels.noEngine

  const healthText = (instance: typeof instances[number]) => {
    if (instance.status === 'stopped') return labels.offline
    if (instance.status === 'error') return 'Error'
    if (instance.healthCheck === 'ok') return labels.healthyStatus
    if (instance.healthCheck === 'fail') return 'Fail'
    return labels.pending
  }

  const healthDotClass = (instance: typeof instances[number]) => {
    if (instance.status === 'stopped') return 'bg-slate-400'
    if (instance.status === 'error') return 'bg-rose-500'
    if (instance.healthCheck === 'ok') return 'bg-emerald-500'
    if (instance.healthCheck === 'fail') return 'bg-rose-500'
    return 'bg-amber-500'
  }

  const filteredInstances = useMemo(() => {
    const normalizedSearch = search.trim().toLowerCase()
    const scoped = instances.filter(instance => {
      if (statusScope === 'running') return instance.status === 'running'
      if (statusScope === 'stopped') return instance.status !== 'running'
      return true
    })

    const byEngine = engineFilter === 'all'
      ? scoped
      : scoped.filter(instance => (instance.config.engine_id || defaultEngineId || '') === engineFilter)

    const bySearch = normalizedSearch
      ? byEngine.filter(instance => {
          const engineName = engineNameFor(instance).toLowerCase()
          return instance.name.toLowerCase().includes(normalizedSearch)
            || instance.model.toLowerCase().includes(normalizedSearch)
            || engineName.includes(normalizedSearch)
            || String(instance.config.port).includes(normalizedSearch)
        })
      : byEngine

    const sorted = [...bySearch]
    sorted.sort((left, right) => {
      if (sortMode === 'port') return left.config.port - right.config.port
      if (sortMode === 'uptime') return (right.startTime || 0) - (left.startTime || 0)
      return left.name.localeCompare(right.name)
    })
    return sorted
  }, [instances, statusScope, engineFilter, search, sortMode, defaultEngineId, engines])

  const recentDownloads = useMemo(() => {
    const priority: Record<string, number> = {
      active: 0,
      error: 1,
      paused: 2,
      pausing: 2,
      queued: 3,
      completed: 4,
      cancelled: 5,
    }

    return [...downloadItems]
      .sort((left, right) => (priority[left.status] ?? 9) - (priority[right.status] ?? 9))
      .slice(0, 4)
  }, [downloadItems])

  const metricPanels = [
    {
      title: 'CPU',
      percent: sysMetrics?.system_cpu_percent ?? sysMetrics?.cpu_percent ?? 0,
      detailA: `${labels.process} ${percentText(sysMetrics?.cpu_percent)}`,
      detailB: `${labels.system} ${percentText(sysMetrics?.system_cpu_percent)}`,
      bar: 'cpu' as const,
    },
    {
      title: 'GPU',
      percent: sysMetrics?.gpu_percent ?? 0,
      detailA: sysMetrics?.gpu_vendor || 'N/A',
      detailB: sysMetrics?.vram_total_mb ? `${((sysMetrics?.vram_used_mb ?? 0) / 1024).toFixed(1)} / ${(sysMetrics.vram_total_mb / 1024).toFixed(1)} GB` : 'VRAM N/A',
      bar: 'gpu' as const,
    },
    {
      title: labels.memory,
      percent: sysMetrics?.system_memory_total_mb
        ? ((sysMetrics.system_memory_used_mb ?? sysMetrics.memory_mb ?? 0) / sysMetrics.system_memory_total_mb) * 100
        : 0,
      detailA: sysMetrics?.system_memory_total_mb
        ? `${((sysMetrics.system_memory_used_mb ?? sysMetrics.memory_mb ?? 0) / 1024).toFixed(1)} / ${(sysMetrics.system_memory_total_mb / 1024).toFixed(1)} GB`
        : 'Memory N/A',
      detailB: sysMetrics?.uptime_secs ? `${labels.uptime} ${formatUptime(Date.now() - sysMetrics.uptime_secs * 1000)}` : labels.waitingMetrics,
      bar: 'memory' as const,
    },
  ]

  return (
    <div className="space-y-5">
      <Surface as="section">
        <div className="flex flex-wrap items-start justify-between gap-4 px-5 py-5">
          <div>
            <h2 className="text-2xl font-semibold text-slate-50">{labels.title}</h2>
            <p className="mt-1 text-sm text-slate-400">{labels.subtitle}</p>
            <div className="mt-3 flex flex-wrap items-center gap-2 text-xs text-slate-400">
              <Badge>{labels.workspace}</Badge>
              <Badge tone="emerald">{labels.healthy}</Badge>
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <Button
              onClick={() => setActiveTab('instances')}
              variant="primary"
              icon={<Zap className="h-4 w-4" />}
            >
              {labels.newInstance}
            </Button>
            <Button
              onClick={() => loadInitialData()}
              icon={<RefreshCw className="h-4 w-4" />}
            >
              {labels.refresh}
            </Button>
            <Button
              onClick={() => setActiveTab('config')}
              icon={<Settings2 className="h-4 w-4" />}
            >
              {labels.config}
            </Button>
          </div>
        </div>
      </Surface>

      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4" data-guide="dashboard">
        {[
          { label: labels.running, value: `${runningCount}/${instances.length}`, icon: <Server className="h-5 w-5" />, tone: 'text-emerald-300 bg-emerald-500/10 border-emerald-500/20' },
          { label: labels.needsAttention, value: attentionCount, icon: <AlertTriangle className="h-5 w-5" />, tone: attentionCount > 0 ? 'text-amber-300 bg-amber-500/10 border-amber-500/20' : 'text-slate-300 bg-slate-800 border-slate-700' },
          { label: labels.resources, value: percentText(sysMetrics?.system_cpu_percent ?? sysMetrics?.cpu_percent), icon: <Gauge className="h-5 w-5" />, tone: 'text-blue-300 bg-blue-500/10 border-blue-500/20' },
          { label: labels.transfer, value: formatRate(transferBytes), icon: <Download className="h-5 w-5" />, tone: 'text-violet-300 bg-violet-500/10 border-violet-500/20' },
        ].map(card => (
          <MetricCard key={card.label} label={card.label} value={card.value} icon={card.icon} tone={card.tone} valueClassName="text-2xl" />
        ))}
      </div>

      <div className="grid gap-5 xl:grid-cols-[minmax(0,2.2fr)_320px]">
        <Surface as="section" className="overflow-hidden">
          <div className="grid md:grid-cols-3">
            {metricPanels.map(panel => (
              <div key={panel.title} className="border-b border-slate-800 px-5 py-5 last:border-b-0 md:border-b-0 md:border-r last:md:border-r-0">
                <div className="flex items-center justify-between">
                  <div className="text-sm font-medium text-slate-400">{panel.title}</div>
                  <Activity className="h-4 w-4 text-slate-500" />
                </div>
                <div className="mt-4 text-4xl font-semibold text-slate-50">{Math.round(panel.percent)}%</div>
                <div className="mt-4 h-2 overflow-hidden rounded-full bg-slate-800">
                  <div className={`h-full rounded-full ${usageBarClass(panel.bar)}`} style={{ width: `${Math.max(0, Math.min(100, panel.percent))}%` }} />
                </div>
                <div className="mt-4 space-y-1 text-xs text-slate-500">
                  <div>{panel.detailA}</div>
                  <div>{panel.detailB}</div>
                </div>
              </div>
            ))}
          </div>
        </Surface>

        <Surface as="aside" className="space-y-4 p-5">
          <SectionHeader title={labels.systemSummary} />
          <InsetSurface className="space-y-3 p-4 text-sm">
            {[
              [labels.running, runningCount],
              [labels.stopped, stoppedCount],
              [labels.models, modelCount],
              [labels.engines, engines.length],
            ].map(([label, value]) => (
              <div key={label} className="flex items-center justify-between gap-3">
                <span className="text-slate-500">{label}</span>
                <span className="font-medium text-slate-100">{value}</span>
              </div>
            ))}
          </InsetSurface>

          <InsetSurface className="space-y-3 p-4 text-sm">
            <div className="flex items-center justify-between">
              <span className="font-medium text-slate-300">{labels.statusContext}</span>
              <Badge tone={attentionCount > 0 ? 'amber' : 'emerald'}>
                {attentionCount > 0 ? labels.needsAttention : labels.healthy}
              </Badge>
            </div>
            {[
              [labels.healthyRunning, healthyCount, 'bg-emerald-500'],
              [labels.needsAttention, attentionCount, 'bg-amber-500'],
              [labels.autoStart, autoStartCount, 'bg-blue-500'],
            ].map(([label, value, dot]) => (
              <div key={label} className="flex items-center justify-between gap-3">
                <span className="inline-flex min-w-0 items-center gap-2 text-slate-500">
                  <span className={`h-2 w-2 rounded-full ${dot}`} />
                  {label}
                </span>
                <span className="font-medium text-slate-100">{value}</span>
              </div>
            ))}
          </InsetSurface>

          <InsetSurface className="space-y-3 p-4 text-sm">
            <div className="flex items-center justify-between">
              <span className="font-medium text-slate-300">{labels.downloads}</span>
              <span className="text-xs text-slate-500">{formatRate(transferBytes)}</span>
            </div>
            <div className="grid grid-cols-3 gap-2 text-center text-xs">
              {[
                [labels.activeDownloads, activeDownloadCount, 'text-blue-300'],
                [labels.queued, queuedDownloadCount, 'text-violet-300'],
                [labels.failed, failedDownloadCount, 'text-rose-300'],
              ].map(([label, value, tone]) => (
                <div key={label} className="rounded-md border border-slate-800 bg-slate-950/70 px-2 py-2">
                  <div className={`text-base font-semibold ${tone}`}>{value}</div>
                  <div className="mt-1 truncate text-slate-500" title={String(label)}>{label}</div>
                </div>
              ))}
            </div>
            <div className="flex items-center justify-between text-xs text-slate-500">
              <span>{labels.queueDepth}</span>
              <span>{downloadQueue.length}</span>
            </div>
          </InsetSurface>

          <div className="grid grid-cols-1 gap-2">
            <Button onClick={() => setActiveTab('downloads')} icon={<Download className="h-4 w-4" />}>{labels.openDownloads}</Button>
            <Button onClick={() => setActiveTab('perf')} icon={<Activity className="h-4 w-4" />}>{labels.openPerformance}</Button>
            <Button onClick={() => setActiveTab('logs')} icon={<BarChart3 className="h-4 w-4" />}>{labels.openLogs}</Button>
          </div>
        </Surface>
      </div>

      <Surface as="section" className="overflow-hidden">
        <div className="border-b border-slate-800 px-5 py-4">
          <div className="mb-4">
            <SectionHeader title={labels.instanceList} description={labels.listDesc} />
          </div>
          <div className="flex flex-wrap items-center justify-between gap-4">
            <div className="flex flex-wrap items-center gap-2">
              {(['running', 'stopped', 'all'] as const).map(scope => (
                <Button
                  key={scope}
                  onClick={() => setStatusScope(scope)}
                  variant={statusScope === scope ? 'primary' : 'subtle'}
                  size="sm"
                >
                  {scope === 'running' ? labels.running : scope === 'stopped' ? labels.stopped : labels.all}
                </Button>
              ))}
            </div>

            <div className="flex flex-wrap items-center gap-2">
              <TextInput
                value={search}
                onChange={event => setSearch(event.target.value)}
                placeholder={labels.searchPlaceholder}
                leadingIcon={<Search className="h-4 w-4" />}
                className="w-full sm:w-[260px]"
              />
              <SelectInput
                value={engineFilter}
                onChange={event => setEngineFilter(event.target.value)}
              >
                <option value="all">{labels.allEngines}</option>
                {engines.map(engine => (
                  <option key={engine.id} value={engine.id}>{engine.name}</option>
                ))}
              </SelectInput>
              <SelectInput
                value={sortMode}
                onChange={event => setSortMode(event.target.value as SortMode)}
              >
                <option value="name">{labels.sortByName}</option>
                <option value="port">{labels.sortByPort}</option>
                <option value="uptime">{labels.sortByUptime}</option>
              </SelectInput>
              <div className="flex items-center rounded-lg border border-slate-800 bg-slate-950/70 p-1">
                <Button
                  onClick={() => setViewMode('table')}
                  variant={viewMode === 'table' ? 'primary' : 'subtle'}
                  size="icon"
                  title={labels.tableView}
                  aria-label={labels.tableView}
                >
                  <List className="h-4 w-4" />
                </Button>
                <Button
                  onClick={() => setViewMode('cards')}
                  variant={viewMode === 'cards' ? 'primary' : 'subtle'}
                  size="icon"
                  title={labels.compactCards}
                  aria-label={labels.compactCards}
                >
                  <Grid3X3 className="h-4 w-4" />
                </Button>
              </div>
            </div>
          </div>

          <div className="mt-4 grid gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(280px,0.9fr)]">
            <InsetSurface className="grid grid-cols-2 gap-3 p-3 text-xs sm:grid-cols-4">
              {[
                [labels.running, runningCount, 'text-emerald-300'],
                [labels.stopped, stoppedCount, 'text-slate-300'],
                [t.instance.error, erroredCount, 'text-rose-300'],
                [labels.healthyRunning, healthyCount, 'text-blue-300'],
              ].map(([label, value, tone]) => (
                <div key={label} className="min-w-0">
                  <div className="truncate text-slate-500" title={String(label)}>{label}</div>
                  <div className={`mt-1 text-lg font-semibold ${tone}`}>{value}</div>
                </div>
              ))}
            </InsetSurface>

            <InsetSurface className="p-3">
              <div className="mb-2 flex items-center justify-between gap-3 text-xs">
                <span className="font-medium text-slate-300">{labels.recentDownloads}</span>
                <Button onClick={() => setActiveTab('downloads')} variant="subtle" size="sm">
                  {labels.open}
                </Button>
              </div>
              {recentDownloads.length === 0 ? (
                <div className="text-xs text-slate-500">{labels.noDownloads}</div>
              ) : (
                <div className="space-y-2">
                  {recentDownloads.map(task => {
                    const progress = task.total > 0 ? Math.min(100, Math.max(0, (task.downloaded / task.total) * 100)) : 0
                    return (
                      <div key={task.id} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 text-xs">
                        <div className="min-w-0">
                          <div className="truncate text-slate-300" title={task.fileName}>{task.fileName}</div>
                          <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-slate-800">
                            <div className="h-full rounded-full bg-blue-500" style={{ width: `${progress}%` }} />
                          </div>
                        </div>
                        <Badge tone={downloadTone(task.status)}>{task.status}</Badge>
                      </div>
                    )
                  })}
                </div>
              )}
            </InsetSurface>
          </div>
        </div>

        {filteredInstances.length === 0 ? (
          <div className="px-5 py-12 text-center">
            <Server className="mx-auto h-10 w-10 text-slate-700" />
            <div className="mt-3 text-sm text-slate-500">{labels.noMatches}</div>
          </div>
        ) : viewMode === 'cards' ? (
          <div className="grid gap-3 p-4 md:grid-cols-2 xl:grid-cols-3">
            {filteredInstances.map(instance => (
              <InsetSurface key={instance.id} className="p-4">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className={`h-2.5 w-2.5 rounded-full ${healthDotClass(instance)}`} />
                      <h3 className="truncate font-medium text-slate-100" title={instance.name}>{instance.name}</h3>
                    </div>
                    <div className="mt-1 text-xs text-slate-500">{instance.config.host}:{instance.config.port}</div>
                  </div>
                  <Badge tone={instance.status === 'running' ? 'emerald' : instance.status === 'error' ? 'red' : 'slate'}>
                    {instance.status === 'running' ? t.instance.running : instance.status === 'stopped' ? t.instance.stopped : t.instance.error}
                  </Badge>
                </div>

                <div className="mt-4 grid grid-cols-2 gap-3 text-xs">
                  <div className="min-w-0 rounded-md border border-slate-800 bg-slate-950/60 px-3 py-2">
                    <div className="text-slate-500">{labels.engine}</div>
                    <div className="mt-1 truncate text-slate-200" title={engineNameFor(instance)}>{engineNameFor(instance)}</div>
                  </div>
                  <div className="min-w-0 rounded-md border border-slate-800 bg-slate-950/60 px-3 py-2">
                    <div className="text-slate-500">{labels.uptime}</div>
                    <div className="mt-1 text-slate-200">{formatUptime(instance.startTime)}</div>
                  </div>
                  <div className="col-span-2 min-w-0 rounded-md border border-slate-800 bg-slate-950/60 px-3 py-2">
                    <div className="text-slate-500">{labels.model}</div>
                    <div className="mt-1 truncate text-slate-200" title={instance.model}>{instance.model}</div>
                  </div>
                </div>

                <div className="mt-4 flex flex-wrap items-center gap-2">
                  {instance.status === 'running' ? (
                    <>
                      <Button
                        onClick={() => openBrowser(instance.config.host, instance.config.port)}
                        size="sm"
                        icon={<ArrowUpRight className="h-3.5 w-3.5" />}
                      >
                        {labels.open}
                      </Button>
                      <Button
                        onClick={() => stopInstance(instance.id)}
                        variant="danger"
                        size="sm"
                        icon={<Square className="h-3.5 w-3.5" />}
                      >
                        {t.instance.stop}
                      </Button>
                    </>
                  ) : (
                    <Button
                      onClick={() => startInstance(instance.id)}
                      variant="primary"
                      size="sm"
                      icon={<Play className="h-3.5 w-3.5" />}
                    >
                      {t.instance.start}
                    </Button>
                  )}
                  <Button
                    onClick={() => { setActiveConfigInstanceId(instance.id); setActiveTab('config') }}
                    size="sm"
                    icon={<Settings2 className="h-3.5 w-3.5" />}
                  >
                    {labels.config}
                  </Button>
                </div>
              </InsetSurface>
            ))}
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="min-w-full text-sm">
              <thead className="bg-slate-950/60">
                <tr className="text-left text-slate-400">
                  <th className="px-5 py-3 font-medium">{labels.name}</th>
                  <th className="px-5 py-3 font-medium">{labels.model}</th>
                  <th className="px-5 py-3 font-medium">{labels.engine}</th>
                  <th className="px-5 py-3 font-medium">{labels.status}</th>
                  <th className="px-5 py-3 font-medium">{labels.health}</th>
                  <th className="px-5 py-3 font-medium">{labels.port}</th>
                  <th className="px-5 py-3 font-medium">{labels.uptime}</th>
                  <th className="px-5 py-3 font-medium">{labels.actions}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-800">
                {filteredInstances.map(instance => (
                  <tr key={instance.id} className="text-slate-200 transition hover:bg-slate-900/70">
                    <td className="px-5 py-4">
                      <div className="font-medium">{instance.name}</div>
                      <div className="text-xs text-slate-500">:{instance.config.port}</div>
                    </td>
                    <td className="max-w-[280px] px-5 py-4">
                      <div className="truncate" title={instance.model}>{instance.model}</div>
                    </td>
                    <td className="px-5 py-4 text-slate-400">{engineNameFor(instance)}</td>
                    <td className="px-5 py-4">
                      <Badge tone={instance.status === 'running' ? 'emerald' : instance.status === 'error' ? 'red' : 'slate'}>
                        {instance.status === 'running' ? t.instance.running : instance.status === 'stopped' ? t.instance.stopped : t.instance.error}
                      </Badge>
                    </td>
                    <td className="px-5 py-4">
                      <span className="inline-flex items-center gap-2 text-slate-400">
                        <span className={`h-2 w-2 rounded-full ${healthDotClass(instance)}`} />
                        {healthText(instance)}
                      </span>
                    </td>
                    <td className="px-5 py-4 text-slate-400">{instance.config.port}</td>
                    <td className="px-5 py-4 text-slate-400">{formatUptime(instance.startTime)}</td>
                    <td className="px-5 py-4">
                      <div className="flex flex-wrap items-center gap-2">
                        {instance.status === 'running' ? (
                          <>
                            <Button
                              onClick={() => openBrowser(instance.config.host, instance.config.port)}
                              size="sm"
                              icon={<ArrowUpRight className="h-3.5 w-3.5" />}
                            >
                              {labels.open}
                            </Button>
                            <Button
                              onClick={() => stopInstance(instance.id)}
                              variant="danger"
                              size="sm"
                              icon={<Square className="h-3.5 w-3.5" />}
                            >
                              {t.instance.stop}
                            </Button>
                          </>
                        ) : (
                          <Button
                            onClick={() => startInstance(instance.id)}
                            variant="primary"
                            size="sm"
                            icon={<Play className="h-3.5 w-3.5" />}
                          >
                            {t.instance.start}
                          </Button>
                        )}
                        <Button
                          onClick={() => { setActiveConfigInstanceId(instance.id); setActiveTab('config') }}
                          size="sm"
                          icon={<Settings2 className="h-3.5 w-3.5" />}
                        >
                          {labels.config}
                        </Button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Surface>
    </div>
  )
}
