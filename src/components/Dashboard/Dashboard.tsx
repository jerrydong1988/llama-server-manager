import { useCallback, useMemo, useState } from 'react'
import {
  Activity,
  AlertTriangle,
  ArrowUpRight,
  BarChart3,
  CheckCircle2,
  Cpu,
  Download,
  Gauge,
  HardDrive,
  Package,
  Play,
  RefreshCw,
  Search,
  Server,
  Settings2,
  Square,
  Wrench,
  Zap,
} from 'lucide-react'
import { useAppStore } from '../../store'
import { formatHostPort } from '../../utils/network'
import type { DownloadProgress, Instance } from '../../store'
import { useI18n } from '../../i18n'
import { getDashboardLabels } from '../../i18n/pageLabels'
import { Badge, Button, InsetSurface, SectionHeader, SelectInput, Surface, TextInput } from '../ui'

type StatusScope = 'running' | 'stopped' | 'all'
type SortMode = 'name' | 'port' | 'uptime'
type MeterTone = 'cpu' | 'gpu' | 'memory'

function clampPercent(value?: number | null) {
  return Math.max(0, Math.min(100, Math.round(value ?? 0)))
}

function percentText(value?: number | null) {
  return `${clampPercent(value)}%`
}

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

function formatMb(value?: number | null) {
  const mb = value ?? 0
  if (mb <= 0) return '--'
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`
  return `${Math.round(mb)} MB`
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

function meterColor(tone: MeterTone) {
  if (tone === 'gpu') return 'bg-emerald-500'
  if (tone === 'memory') return 'bg-violet-500'
  return 'bg-blue-500'
}

function statusTone(instance: Instance): 'slate' | 'emerald' | 'red' {
  if (instance.status === 'running') return 'emerald'
  if (instance.status === 'error') return 'red'
  return 'slate'
}

function healthDotClass(instance: Instance) {
  if (instance.status === 'stopped') return 'bg-slate-500'
  if (instance.status === 'error' || instance.healthCheck === 'fail') return 'bg-rose-500'
  if (instance.healthCheck === 'ok') return 'bg-emerald-500'
  return 'bg-amber-500'
}

function downloadTone(status: DownloadProgress['status']): 'slate' | 'blue' | 'emerald' | 'amber' | 'red' | 'violet' {
  if (status === 'active') return 'blue'
  if (status === 'completed') return 'emerald'
  if (status === 'error') return 'red'
  if (status === 'paused' || status === 'pausing') return 'amber'
  if (status === 'queued') return 'violet'
  return 'slate'
}

function MiniStat({
  label,
  value,
  tone = 'text-slate-900 dark:text-slate-100',
}: {
  label: string
  value: string | number
  tone?: string
}) {
  return (
    <div className="min-w-0 rounded-md border border-slate-200 bg-slate-50 px-3 py-2 dark:border-slate-800 dark:bg-slate-950/60">
      <div className="truncate text-[11px] uppercase tracking-wide text-slate-500 dark:text-slate-500" title={label}>{label}</div>
      <div className={`mt-1 truncate text-lg font-semibold ${tone}`} title={String(value)}>{value}</div>
    </div>
  )
}

function ResourceMeter({
  label,
  percent,
  primary,
  secondary,
  tone,
  icon,
}: {
  label: string
  percent: number
  primary: string
  secondary: string
  tone: MeterTone
  icon: React.ReactNode
}) {
  const safePercent = clampPercent(percent)

  return (
    <div className="min-w-0 border-b border-slate-200 px-4 py-3 last:border-b-0 md:border-b-0 md:border-r md:last:border-r-0 dark:border-slate-800">
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-slate-200 bg-slate-100 text-slate-500 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-400">
            {icon}
          </span>
          <div className="min-w-0">
            <div className="truncate text-sm font-medium text-slate-800 dark:text-slate-200" title={label}>{label}</div>
            <div className="truncate text-xs text-slate-500 dark:text-slate-500" title={secondary}>{secondary}</div>
          </div>
        </div>
        <div className="shrink-0 text-xl font-semibold text-slate-950 dark:text-slate-50">{safePercent}%</div>
      </div>
      <div className="mt-3 h-2 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-800">
        <div className={`h-full rounded-full ${meterColor(tone)}`} style={{ width: `${safePercent}%` }} />
      </div>
      <div className="mt-2 truncate text-xs text-slate-500 dark:text-slate-500" title={primary}>{primary}</div>
    </div>
  )
}

function ActionIconButton({
  title,
  children,
  onClick,
  disabled = false,
}: {
  title: string
  children: React.ReactNode
  onClick: () => void
  disabled?: boolean
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      aria-label={title}
      className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-slate-300 bg-white text-slate-600 transition hover:border-slate-400 hover:bg-slate-50 disabled:cursor-not-allowed disabled:opacity-35 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-300 dark:hover:border-slate-600 dark:hover:bg-slate-800"
    >
      {children}
    </button>
  )
}

export default function Dashboard() {
  const { t, lang } = useI18n()
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

  const labels = getDashboardLabels(lang, t.dashboard?.title)

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

  const engineNameFor = useCallback((instance: Instance) =>
    engines.find(engine => engine.id === (instance.config.engine_id || defaultEngineId || ''))?.name
    || engines.find(engine => engine.id === defaultEngineId)?.name
    || engines[0]?.name
    || labels.noEngine, [defaultEngineId, engines, labels.noEngine])

  const healthText = (instance: Instance) => {
    if (instance.status === 'stopped') return labels.offline
    if (instance.status === 'error') return t.instance.error
    if (instance.healthCheck === 'ok') return labels.healthy
    if (instance.healthCheck === 'fail') return labels.fail
    return labels.pending
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

    return [...bySearch].sort((left, right) => {
      if (sortMode === 'port') return left.config.port - right.config.port
      if (sortMode === 'uptime') return (right.startTime || 0) - (left.startTime || 0)
      return left.name.localeCompare(right.name)
    })
  }, [instances, statusScope, engineFilter, search, sortMode, defaultEngineId, engineNameFor])

  const recentDownloads = useMemo(() => {
    const priority: Record<DownloadProgress['status'], number> = {
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

  const memoryPercent = sysMetrics?.system_memory_total_mb
    ? ((sysMetrics.system_memory_used_mb ?? sysMetrics.memory_mb ?? 0) / sysMetrics.system_memory_total_mb) * 100
    : 0

  const resourceMeters = [
    {
      label: 'CPU',
      percent: sysMetrics?.system_cpu_percent ?? sysMetrics?.cpu_percent ?? 0,
      primary: `${labels.process} ${percentText(sysMetrics?.cpu_percent)} / ${labels.system} ${percentText(sysMetrics?.system_cpu_percent)}`,
      secondary: labels.system,
      tone: 'cpu' as const,
      icon: <Cpu className="h-4 w-4" />,
    },
    {
      label: 'GPU',
      percent: sysMetrics?.gpu_percent ?? 0,
      primary: sysMetrics?.vram_total_mb
        ? `${labels.vram} ${formatMb(sysMetrics.vram_used_mb)} / ${formatMb(sysMetrics.vram_total_mb)}`
        : `${labels.vram} --`,
      secondary: sysMetrics?.gpu_name || sysMetrics?.gpu_vendor || 'N/A',
      tone: 'gpu' as const,
      icon: <Gauge className="h-4 w-4" />,
    },
    {
      label: labels.memory,
      percent: memoryPercent,
      primary: sysMetrics?.system_memory_total_mb
        ? `${formatMb(sysMetrics.system_memory_used_mb ?? sysMetrics.memory_mb)} / ${formatMb(sysMetrics.system_memory_total_mb)}`
        : '--',
      secondary: sysMetrics?.system_memory_total_mb ? labels.system : labels.noMetrics,
      tone: 'memory' as const,
      icon: <HardDrive className="h-4 w-4" />,
    },
  ]

  const cockpitActions = useMemo(() => {
    const actions: Array<{
      id: string
      title: string
      description: string
      tone: 'blue' | 'emerald' | 'amber' | 'red' | 'violet'
      icon: React.ReactNode
      action: () => void
    }> = []

    if (engines.length === 0) {
      actions.push({
        id: 'setup-engine',
        title: labels.setupEngineTitle,
        description: labels.setupEngineDesc,
        tone: 'red',
        icon: <Wrench className="h-4 w-4" />,
        action: () => setActiveTab('engine'),
      })
    }

    if (modelCount === 0) {
      actions.push({
        id: 'setup-model',
        title: labels.setupModelTitle,
        description: labels.setupModelDesc,
        tone: 'amber',
        icon: <Package className="h-4 w-4" />,
        action: () => setActiveTab('model-repo'),
      })
    }

    if (instances.length === 0) {
      actions.push({
        id: 'create-instance',
        title: labels.createInstanceTitle,
        description: labels.createInstanceDesc,
        tone: 'blue',
        icon: <Server className="h-4 w-4" />,
        action: () => setActiveTab('instances'),
      })
    }

    if (attentionCount > 0) {
      actions.push({
        id: 'inspect-health',
        title: labels.inspectHealthTitle,
        description: labels.inspectHealthDesc,
        tone: 'red',
        icon: <AlertTriangle className="h-4 w-4" />,
        action: () => setActiveTab('instances'),
      })
    }

    if (failedDownloadCount > 0) {
      actions.push({
        id: 'retry-download',
        title: labels.retryDownloadTitle,
        description: labels.retryDownloadDesc,
        tone: 'amber',
        icon: <Download className="h-4 w-4" />,
        action: () => setActiveTab('downloads'),
      })
    }

    if (activeDownloadCount > 0) {
      actions.push({
        id: 'monitor-transfer',
        title: labels.monitorTransferTitle,
        description: labels.monitorTransferDesc,
        tone: 'blue',
        icon: <Download className="h-4 w-4" />,
        action: () => setActiveTab('downloads'),
      })
    }

    if (actions.length === 0) {
      actions.push({
        id: 'review-performance',
        title: labels.reviewPerfTitle,
        description: labels.reviewPerfDesc,
        tone: 'emerald',
        icon: <CheckCircle2 className="h-4 w-4" />,
        action: () => setActiveTab('perf'),
      })
    }

    return actions.slice(0, 3)
  }, [activeDownloadCount, attentionCount, engines.length, failedDownloadCount, instances.length, labels, modelCount, setActiveTab])

  const openConfig = (instanceId: string) => {
    setActiveConfigInstanceId(instanceId)
    setActiveTab('config')
  }

  return (
    <div className="space-y-4" data-guide="dashboard">
      <Surface as="section">
        <div className="flex flex-col gap-4 px-5 py-4 lg:flex-row lg:items-center lg:justify-between">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="text-xl font-semibold text-slate-950 dark:text-slate-50">{labels.title}</h2>
              <Badge tone={attentionCount > 0 ? 'amber' : 'emerald'}>{attentionCount > 0 ? labels.attention : labels.healthy}</Badge>
              <Badge>{labels.workspace}</Badge>
            </div>
            <p className="mt-1 truncate text-sm text-slate-500 dark:text-slate-400">{labels.subtitle}</p>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <Button onClick={() => setActiveTab('instances')} variant="primary" icon={<Zap className="h-4 w-4" />}>
              {labels.create}
            </Button>
            <Button onClick={() => loadInitialData()} icon={<RefreshCw className="h-4 w-4" />}>
              {labels.refresh}
            </Button>
          </div>
        </div>
      </Surface>

      <Surface as="section" className="p-5">
        <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
          <SectionHeader title={labels.actionCenter} description={labels.actionCenterDesc} />
          <div className="grid w-full gap-3 xl:max-w-[920px] xl:grid-cols-3">
            {cockpitActions.map(action => (
              <button
                key={action.id}
                type="button"
                onClick={action.action}
                className="group flex min-h-[116px] min-w-0 flex-col justify-between rounded-lg border border-slate-200 bg-slate-50 p-4 text-left transition hover:border-blue-300 hover:bg-blue-50 dark:border-slate-800 dark:bg-slate-950/55 dark:hover:border-blue-500/40 dark:hover:bg-blue-500/10"
              >
                <span className="flex items-start gap-3">
                  <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-slate-200 bg-white text-blue-600 dark:border-slate-800 dark:bg-slate-950 dark:text-blue-300">
                    {action.icon}
                  </span>
                  <span className="min-w-0">
                    <span className="block truncate text-sm font-semibold text-slate-950 dark:text-slate-100">{action.title}</span>
                    <span className="mt-1 line-clamp-2 block text-xs leading-5 text-slate-500 dark:text-slate-400">{action.description}</span>
                  </span>
                </span>
                <span className="mt-3 inline-flex items-center gap-1.5 text-xs font-semibold text-blue-600 transition group-hover:text-blue-500 dark:text-blue-300">
                  {labels.goHandle}
                  <ArrowUpRight className="h-3.5 w-3.5" />
                </span>
              </button>
            ))}
          </div>
        </div>
      </Surface>

      <Surface as="section" className="overflow-hidden">
        <div className="border-b border-slate-200 px-5 py-3 dark:border-slate-800">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <SectionHeader title={labels.resources} />
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
              <MiniStat label={labels.running} value={`${runningCount}/${instances.length}`} tone="text-emerald-600 dark:text-emerald-300" />
              <MiniStat label={labels.attention} value={attentionCount} tone={attentionCount > 0 ? 'text-amber-600 dark:text-amber-300' : 'text-slate-700 dark:text-slate-300'} />
              <MiniStat label={labels.transfer} value={formatRate(transferBytes)} tone="text-blue-600 dark:text-blue-300" />
              <MiniStat label={labels.queue} value={downloadQueue.length} tone="text-violet-600 dark:text-violet-300" />
            </div>
          </div>
        </div>
        <div className="grid md:grid-cols-3">
          {resourceMeters.map(meter => (
            <ResourceMeter key={meter.label} {...meter} />
          ))}
        </div>
      </Surface>

      <div className="grid gap-4 2xl:grid-cols-[minmax(0,1fr)_320px]">
        <Surface as="section" className="min-w-0 overflow-hidden">
          <div className="border-b border-slate-200 px-5 py-4 dark:border-slate-800">
            <div className="flex flex-col gap-4 2xl:flex-row 2xl:items-end 2xl:justify-between">
              <div className="min-w-0">
                <SectionHeader
                  title={statusScope === 'running' ? labels.running : statusScope === 'stopped' ? labels.stopped : labels.instances}
                  description={`${labels.visibleRows}: ${filteredInstances.length} / ${instances.length}`}
                />
                <div className="mt-3 inline-flex rounded-lg border border-slate-200 bg-slate-100 p-1 dark:border-slate-800 dark:bg-slate-950/70">
                  {(['running', 'stopped', 'all'] as const).map(scope => (
                    <button
                      type="button"
                      key={scope}
                      onClick={() => setStatusScope(scope)}
                      className={`h-8 rounded-md px-3 text-xs font-medium transition ${
                        statusScope === scope
                          ? 'bg-blue-600 text-white'
                          : 'text-slate-600 hover:bg-white hover:text-slate-950 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-slate-100'
                      }`}
                    >
                      {scope === 'running' ? labels.running : scope === 'stopped' ? labels.stopped : labels.all}
                    </button>
                  ))}
                </div>
              </div>

              <div className="grid gap-2 sm:grid-cols-[minmax(220px,1fr)_180px_150px] 2xl:w-[610px]">
                <TextInput
                  value={search}
                  onChange={event => setSearch(event.target.value)}
                  placeholder={labels.searchPlaceholder}
                  leadingIcon={<Search className="h-4 w-4" />}
                />
                <SelectInput value={engineFilter} onChange={event => setEngineFilter(event.target.value)}>
                  <option value="all">{labels.allEngines}</option>
                  {engines.map(engine => (
                    <option key={engine.id} value={engine.id}>{engine.name}</option>
                  ))}
                </SelectInput>
                <SelectInput value={sortMode} onChange={event => setSortMode(event.target.value as SortMode)}>
                  <option value="name">{labels.sortName}</option>
                  <option value="port">{labels.sortPort}</option>
                  <option value="uptime">{labels.sortUptime}</option>
                </SelectInput>
              </div>
            </div>

            <div className="mt-4 grid grid-cols-2 gap-2 md:grid-cols-4">
              <MiniStat label={labels.running} value={runningCount} tone="text-emerald-600 dark:text-emerald-300" />
              <MiniStat label={labels.stopped} value={stoppedCount} tone="text-slate-700 dark:text-slate-300" />
              <MiniStat label={t.instance.error} value={erroredCount} tone="text-rose-600 dark:text-rose-300" />
              <MiniStat label={labels.autoStart} value={autoStartCount} tone="text-blue-600 dark:text-blue-300" />
            </div>
          </div>

          {filteredInstances.length === 0 ? (
            <div className="px-5 py-12 text-center">
              <Server className="mx-auto h-10 w-10 text-slate-300 dark:text-slate-700" />
              <div className="mt-3 text-sm text-slate-500 dark:text-slate-500">{labels.noMatches}</div>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full min-w-[860px] table-fixed text-sm">
                <colgroup>
                  <col className="w-[22%]" />
                  <col className="w-[16%]" />
                  <col className="w-[12%]" />
                  <col className="w-[13%]" />
                  <col className="w-[17%]" />
                  <col className="w-[20%]" />
                </colgroup>
                <thead className="bg-slate-50 dark:bg-slate-950/70">
                  <tr className="text-left text-xs uppercase tracking-wide text-slate-500 dark:text-slate-500">
                    <th className="px-5 py-3 font-medium">{labels.name}</th>
                    <th className="px-5 py-3 font-medium">{labels.engine}</th>
                    <th className="px-5 py-3 font-medium">{labels.status}</th>
                    <th className="px-5 py-3 font-medium">{labels.health}</th>
                    <th className="px-5 py-3 font-medium">{labels.endpoint}</th>
                    <th className="px-5 py-3 text-right font-medium">{labels.actions}</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-slate-200 dark:divide-slate-800">
                  {filteredInstances.map(instance => {
                    const isRunning = instance.status === 'running'
                    const endpoint = formatHostPort(instance.config.host, instance.config.port)
                    const engineName = engineNameFor(instance)

                    return (
                      <tr key={instance.id} className="h-[68px] text-slate-700 transition hover:bg-slate-50 dark:text-slate-200 dark:hover:bg-slate-900/70">
                        <td className="px-5 py-3 align-middle">
                          <div className="min-w-0">
                            <div className="truncate font-medium text-slate-950 dark:text-slate-100" title={instance.name}>{instance.name}</div>
                            <div className="mt-1 text-xs text-slate-500 dark:text-slate-500">{labels.uptime} {formatUptime(instance.startTime)}</div>
                          </div>
                        </td>
                        <td className="px-5 py-3 align-middle">
                          <div className="truncate text-slate-600 dark:text-slate-400" title={engineName}>{engineName}</div>
                        </td>
                        <td className="px-5 py-3 align-middle">
                          <Badge tone={statusTone(instance)}>
                            {isRunning ? t.instance.running : instance.status === 'stopped' ? t.instance.stopped : t.instance.error}
                          </Badge>
                        </td>
                        <td className="px-5 py-3 align-middle">
                          <span className="inline-flex min-w-0 items-center gap-2 text-slate-600 dark:text-slate-400">
                            <span className={`h-2 w-2 shrink-0 rounded-full ${healthDotClass(instance)}`} />
                            <span className="truncate">{healthText(instance)}</span>
                          </span>
                        </td>
                        <td className="px-5 py-3 align-middle">
                          <div className="truncate font-mono text-xs text-slate-600 dark:text-slate-400" title={endpoint}>{endpoint}</div>
                        </td>
                        <td className="px-5 py-3 align-middle">
                          <div className="ml-auto grid w-[172px] grid-cols-[92px_34px_34px] items-center justify-end gap-2">
                            {isRunning ? (
                              <Button
                                onClick={() => void stopInstance(instance.id).catch(() => {})}
                                variant="danger"
                                size="sm"
                                className="h-8 whitespace-nowrap px-2"
                                icon={<Square className="h-3.5 w-3.5" />}
                              >
                                {t.instance.stop}
                              </Button>
                            ) : (
                              <Button
                                onClick={() => void startInstance(instance.id).catch(() => {})}
                                variant="primary"
                                size="sm"
                                className="h-8 whitespace-nowrap px-2"
                                icon={<Play className="h-3.5 w-3.5" />}
                              >
                                {t.instance.start}
                              </Button>
                            )}
                            <ActionIconButton
                              title={labels.open}
                              onClick={() => openBrowser(instance.config.host, instance.config.port)}
                              disabled={!isRunning}
                            >
                              <ArrowUpRight className="h-4 w-4" />
                            </ActionIconButton>
                            <ActionIconButton title={labels.config} onClick={() => openConfig(instance.id)}>
                              <Settings2 className="h-4 w-4" />
                            </ActionIconButton>
                          </div>
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            </div>
          )}
        </Surface>

        <div className="space-y-4">
          <Surface as="aside" className="space-y-3 p-4">
            <SectionHeader title={labels.operations} />
            <InsetSurface className="space-y-3 p-3 text-sm">
              <div className="flex items-center justify-between gap-3">
                <span className="text-slate-500 dark:text-slate-500">{labels.healthy}</span>
                <span className="font-medium text-emerald-600 dark:text-emerald-300">{healthyCount}</span>
              </div>
              <div className="flex items-center justify-between gap-3">
                <span className="text-slate-500 dark:text-slate-500">{labels.attention}</span>
                <span className={attentionCount > 0 ? 'font-medium text-amber-600 dark:text-amber-300' : 'font-medium text-slate-700 dark:text-slate-300'}>{attentionCount}</span>
              </div>
              <div className="flex items-center justify-between gap-3">
                <span className="text-slate-500 dark:text-slate-500">{labels.autoStart}</span>
                <span className="font-medium text-blue-600 dark:text-blue-300">{autoStartCount}</span>
              </div>
            </InsetSurface>

            <div className="grid grid-cols-1 gap-2">
              <Button onClick={() => setActiveTab('downloads')} icon={<Download className="h-4 w-4" />}>{labels.openDownloads}</Button>
              <Button onClick={() => setActiveTab('perf')} icon={<Activity className="h-4 w-4" />}>{labels.openPerformance}</Button>
              <Button onClick={() => setActiveTab('logs')} icon={<BarChart3 className="h-4 w-4" />}>{labels.openLogs}</Button>
            </div>
          </Surface>

          <Surface as="aside" className="space-y-3 p-4">
            <SectionHeader title={labels.inventory} />
            <div className="grid grid-cols-2 gap-2">
              <MiniStat label={labels.instances} value={instances.length} />
              <MiniStat label={labels.models} value={modelCount} />
              <MiniStat label={labels.engines} value={engines.length} />
              <MiniStat label={labels.downloads} value={downloadItems.length} />
            </div>
          </Surface>

          <Surface as="aside" className="space-y-3 p-4">
            <div className="flex items-center justify-between gap-3">
              <SectionHeader title={labels.recentDownloads} />
              <Badge tone="blue">{formatRate(transferBytes)}</Badge>
            </div>
            <InsetSurface className="grid grid-cols-3 gap-2 p-3 text-center text-xs">
              <div>
                <div className="font-semibold text-blue-600 dark:text-blue-300">{activeDownloadCount}</div>
                <div className="mt-1 truncate text-slate-500 dark:text-slate-500">{labels.activeDownloads}</div>
              </div>
              <div>
                <div className="font-semibold text-violet-600 dark:text-violet-300">{queuedDownloadCount}</div>
                <div className="mt-1 truncate text-slate-500 dark:text-slate-500">{labels.queuedDownloads}</div>
              </div>
              <div>
                <div className="font-semibold text-rose-600 dark:text-rose-300">{failedDownloadCount}</div>
                <div className="mt-1 truncate text-slate-500 dark:text-slate-500">{labels.failedDownloads}</div>
              </div>
            </InsetSurface>
            {recentDownloads.length === 0 ? (
              <div className="rounded-md border border-slate-200 bg-slate-50 px-3 py-4 text-center text-xs text-slate-500 dark:border-slate-800 dark:bg-slate-950/60 dark:text-slate-500">
                {labels.noDownloads}
              </div>
            ) : (
              <div className="space-y-2">
                {recentDownloads.map(task => {
                  const progress = task.total > 0 ? Math.min(100, Math.max(0, (task.downloaded / task.total) * 100)) : 0
                  return (
                    <div key={task.id} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 rounded-md border border-slate-200 bg-slate-50 px-3 py-2 text-xs dark:border-slate-800 dark:bg-slate-950/60">
                      <div className="min-w-0">
                        <div className="truncate text-slate-700 dark:text-slate-300" title={task.fileName}>{task.fileName}</div>
                        <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-800">
                          <div className="h-full rounded-full bg-blue-500" style={{ width: `${progress}%` }} />
                        </div>
                      </div>
                      <Badge tone={downloadTone(task.status)}>{task.status}</Badge>
                    </div>
                  )
                })}
              </div>
            )}
          </Surface>
        </div>
      </div>
    </div>
  )
}
