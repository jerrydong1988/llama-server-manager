import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  Cpu,
  Database,
  Download,
  Gauge,
  HardDrive,
  Monitor,
  Server,
  Terminal,
  Zap,
} from 'lucide-react'
import { useI18n } from '../i18n'
import { useAppStore } from '../store'
import type {
  DownloadProgress,
  InferenceRequestSummary,
  Instance,
  PerfUpdateEvent,
  RunningInferenceTask,
  SystemMetrics,
  TelemetryOverview,
  TelemetrySessionSummary,
} from '../store/types'
import { Badge, joinClassNames } from './ui'
import { MiniSparkline, TrendChart } from './monitoring/MonitoringPrimitives'
import { formatBytes, formatBytesPerSecond, formatDuration, formatRate, formatTime } from './monitoring/monitoringFormat'
import { buildActivityFeed, buildRequestPressure, buildResourceSignals, buildServiceStatus, selectCurrentThroughput, type ActivityFeedItem, type SignalTone } from './monitoring/monitoringViewModel'

type MetricsEvent = {
  instanceId: string
  system: SystemMetrics
  llama?: {
    tokens_per_sec?: number
    prompt_tokens_per_sec?: number
    requests?: number
    requests_processing?: number
    requests_deferred?: number
    busy_slots_per_decode?: number
  } | null
  ts: number
}

const emptyOverview: TelemetryOverview = {
  active_sessions: 0,
  sessions_24h: 0,
  avg_tokens_per_sec_24h: 0,
  peak_vram_mb_24h: 0,
  latest_samples: [],
}

const BIG_SCREEN_TELEMETRY_REFRESH_MS = 10000
const BIG_SCREEN_EVENT_REFRESH_MS = 5000

export default function BigScreenPage() {
  const { lang } = useI18n()
  const zh = lang === 'zh-CN'
  const labels = getLabels(zh)
  const instances = useAppStore(state => state.instances)
  const models = useAppStore(state => state.models)
  const engines = useAppStore(state => state.engines)
  const logs = useAppStore(state => state.logs)
  const sysMetrics = useAppStore(state => state.sysMetrics)
  const downloadTasks = useAppStore(state => state.downloadTasks)
  const downloadQueue = useAppStore(state => state.downloadQueue)
  const loadInitialData = useAppStore(state => state.loadInitialData)

  const [overview, setOverview] = useState<TelemetryOverview>(emptyOverview)
  const [sessions, setSessions] = useState<TelemetrySessionSummary[]>([])
  const [requests, setRequests] = useState<InferenceRequestSummary[]>([])
  const [runningTasksByInstance, setRunningTasksByInstance] = useState<Record<string, RunningInferenceTask[]>>({})
  const [liveLlama, setLiveLlama] = useState<MetricsEvent['llama']>(null)
  const [lastUpdatedAt, setLastUpdatedAt] = useState(Date.now())
  const [refreshing, setRefreshing] = useState(false)
  const [telemetryError, setTelemetryError] = useState<string | null>(null)
  const lastTelemetryRefreshRef = useRef(0)

  const refreshTelemetry = useCallback(async (options: { silent?: boolean } = {}) => {
    lastTelemetryRefreshRef.current = Date.now()
    if (!options.silent) setRefreshing(true)
    try {
      const [nextOverview, nextSessions] = await Promise.all([
        invoke<TelemetryOverview>('get_telemetry_overview'),
        invoke<TelemetrySessionSummary[]>('list_telemetry_sessions', { limit: 12 }),
      ])
      setOverview(nextOverview)
      setSessions(nextSessions)

      const sessionId = nextSessions[0]?.id
      if (sessionId) {
        const nextRequests = await invoke<InferenceRequestSummary[]>('list_inference_requests', { sessionId, limit: 8 })
        setRequests(nextRequests)
      } else {
        setRequests([])
      }
      setTelemetryError(null)
      setLastUpdatedAt(Date.now())
    } catch (error) {
      setTelemetryError(error instanceof Error ? error.message : String(error))
    } finally {
      if (!options.silent) setRefreshing(false)
    }
  }, [])

  useEffect(() => {
    void loadInitialData().catch(() => undefined)
  }, [loadInitialData])

  useEffect(() => {
    void refreshTelemetry()
    const timer = window.setInterval(() => void refreshTelemetry({ silent: true }), BIG_SCREEN_TELEMETRY_REFRESH_MS)
    return () => window.clearInterval(timer)
  }, [refreshTelemetry])

  useEffect(() => {
    const unlisten = listen<MetricsEvent>('metrics-update', event => {
      setLiveLlama(event.payload.llama || null)
      setLastUpdatedAt(Date.now())
      if (Date.now() - lastTelemetryRefreshRef.current > BIG_SCREEN_EVENT_REFRESH_MS) {
        void refreshTelemetry({ silent: true })
      }
    })
    return () => {
      unlisten.then(dispose => dispose()).catch(() => {})
    }
  }, [refreshTelemetry])

  useEffect(() => {
    const unlisten = listen<PerfUpdateEvent>('perf-update', event => {
      setRunningTasksByInstance(current => ({
        ...current,
        [event.payload.instanceId]: event.payload.tasks || [],
      }))
      setLastUpdatedAt(Date.now())
    })
    return () => {
      unlisten.then(dispose => dispose()).catch(() => {})
    }
  }, [])

  const allDownloads = useMemo(() => Object.values(downloadTasks), [downloadTasks])
  const runningInstances = useMemo(() => instances.filter(instance => instance.status === 'running'), [instances])
  const stoppedCount = instances.filter(instance => instance.status === 'stopped').length
  const errorCount = instances.filter(instance => instance.status === 'error').length
  const latestSample = overview.latest_samples[0] || null
  const trendSamples = useMemo(
    () => [...overview.latest_samples].sort((left, right) => left.ts - right.ts),
    [overview.latest_samples],
  )
  const trendValues = trendSamples.map(sample => sample.tokens_per_sec || 0)
  const currentTps = selectCurrentThroughput(liveLlama?.tokens_per_sec, latestSample?.tokens_per_sec ?? overview.avg_tokens_per_sec_24h)
  const peakTps = Math.max(...trendValues, currentTps, 0)
  const avgTps = average(trendValues.slice(-12))
  const pressure = buildRequestPressure(liveLlama?.requests_processing ?? latestSample?.requests_processing, liveLlama?.requests_deferred ?? latestSample?.requests_deferred)

  const flatLogs = useMemo(
    () => Object.entries(logs)
      .flatMap(([instanceId, entries]) => {
        const instanceName = instances.find(instance => instance.id === instanceId)?.name || instanceId
        return entries.map(entry => ({ ...entry, instanceName }))
      })
      .sort((left, right) => right.timestamp - left.timestamp),
    [instances, logs],
  )

  const serviceStatus = useMemo(
    () => buildServiceStatus({ instances, downloads: allDownloads, logs: flatLogs, telemetryError }),
    [instances, allDownloads, flatLogs, telemetryError],
  )

  const activeRequestTasks = useMemo(
    () => Object.entries(runningTasksByInstance)
      .flatMap(([instanceId, tasks]) => {
        const instanceName = instances.find(instance => instance.id === instanceId)?.name || instanceId
        return tasks.map(task => ({ ...task, instanceName }))
      })
      .sort((left, right) => right.updated_at_ms - left.updated_at_ms)
      .slice(0, 4),
    [instances, runningTasksByInstance],
  )

  const downloadStats = useMemo(() => {
    const active = allDownloads.filter(task => task.status === 'active')
    const failed = allDownloads.filter(task => task.status === 'error')
    const completed = allDownloads.filter(task => task.status === 'completed')
    return {
      active: active.length,
      failed: failed.length,
      completed: completed.length,
      queued: allDownloads.filter(task => task.status === 'queued').length + downloadQueue.length,
      speed: active.reduce((sum, task) => sum + (task.speed || 0), 0),
    }
  }, [allDownloads, downloadQueue.length])

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
    return [...allDownloads]
      .sort((left, right) => (priority[left.status] ?? 9) - (priority[right.status] ?? 9))
      .slice(0, 3)
  }, [allDownloads])

  const resourceSignals = useMemo(
    () => buildResourceSignals({
      system: sysMetrics,
      latest: latestSample,
      samples: trendSamples,
      labels,
    }),
    [sysMetrics, latestSample, trendSamples, labels],
  )

  const activityFeed = useMemo(
    () => buildActivityFeed({
      requests,
      activeTasks: activeRequestTasks,
      downloads: recentDownloads,
      logs: flatLogs.slice(0, 8),
      labels: {
        request: labels.request,
        download: labels.download,
        log: labels.log,
        active: labels.active,
        failed: labels.failed,
        completed: labels.completed,
      },
      limit: 8,
    }),
    [requests, activeRequestTasks, recentDownloads, flatLogs, labels],
  )

  const modelCount = models.filter(model => !model.is_shard && model.file_type === 'model').length
  const statusLabel = serviceStatus.status === 'critical' ? labels.abnormal : serviceStatus.status === 'attention' ? labels.needsAttention : labels.normal
  const statusTone: SignalTone = serviceStatus.status === 'critical' ? 'red' : serviceStatus.status === 'attention' ? 'amber' : 'emerald'

  return (
    <div className="min-h-[calc(100vh-96px)] space-y-3 bg-slate-100 p-3 text-slate-950 dark:bg-slate-950 dark:text-slate-100">
      <header className="grid min-h-[72px] grid-cols-[minmax(220px,1fr)_minmax(260px,1.2fr)_minmax(220px,1fr)] items-center rounded-lg border border-slate-200 bg-white px-5 shadow-[0_20px_80px_rgba(15,23,42,0.08)] dark:border-slate-700/80 dark:bg-slate-900/90 dark:shadow-[0_20px_80px_rgba(2,6,23,0.45)]">
        <div className="flex min-w-0 items-center gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-blue-200 bg-blue-50 text-blue-700 dark:border-blue-300/25 dark:bg-blue-400/10 dark:text-blue-200">
            <Monitor className="h-5 w-5" />
          </div>
          <div className="min-w-0">
            <div className="truncate text-lg font-semibold text-slate-950 dark:text-white">Llama Server Manager</div>
            <div className="truncate text-xs text-slate-500">{labels.wallboard}</div>
          </div>
        </div>
        <div className="text-center">
          <h1 className="text-3xl font-semibold tracking-wide text-slate-950 dark:text-white">{labels.title}</h1>
          <p className="mt-1 text-sm text-slate-600 dark:text-slate-400">{labels.subtitle}</p>
        </div>
        <div className="text-right">
          <div className="font-mono text-xl font-semibold text-slate-950 dark:text-white">{formatTime(lastUpdatedAt)}</div>
          <div className="mt-1 text-xs text-slate-500">{refreshing ? labels.refreshing : labels.updated}</div>
        </div>
      </header>

      <section className="grid gap-3 md:grid-cols-2 xl:grid-cols-5">
        <WallKpi label={labels.serviceStatus} value={statusLabel} detail={telemetryError || labels.allServicesNormal} tone={statusTone} icon={<CheckCircle2 className="h-8 w-8" />} />
        <WallKpi label={labels.runningInstances} value={`${runningInstances.length} / ${instances.length}`} detail={`${labels.stopped} ${stoppedCount}`} tone="blue" icon={<Server className="h-8 w-8" />} />
        <WallKpi label={labels.currentThroughput} value={formatRate(currentTps)} detail={`${labels.peak} ${formatRate(peakTps)}`} tone="cyan" icon={<Gauge className="h-8 w-8" />} />
        <WallKpi label={labels.requestPressure} value={`${pressure.percent}%`} detail={pressure.level === 'high' ? labels.high : pressure.level === 'medium' ? labels.medium : labels.normal} tone={pressure.level === 'high' ? 'amber' : 'emerald'} icon={<Zap className="h-8 w-8" />} />
        <WallKpi label={labels.alerts} value={serviceStatus.alertCount} detail={`${labels.error} ${errorCount} · ${labels.failed} ${downloadStats.failed}`} tone={serviceStatus.alertCount > 0 ? 'red' : 'emerald'} icon={<AlertTriangle className="h-8 w-8" />} />
      </section>

      <section className="grid gap-3 xl:grid-cols-[minmax(0,1.08fr)_minmax(420px,0.92fr)]">
        <WallPanel title={labels.realtimeThroughput} icon={<Activity className="h-5 w-5" />} action={<Badge tone="blue">5分钟</Badge>}>
          <div className="grid gap-4 lg:grid-cols-[190px_minmax(0,1fr)]">
            <div className="space-y-4">
              <div>
                <div className="text-xs text-slate-500">{labels.currentThroughput}</div>
                <div className="mt-2 text-5xl font-semibold text-blue-700 dark:text-blue-300">{formatRate(currentTps).replace(' tok/s', '')}</div>
                <div className="mt-1 text-sm text-blue-600 dark:text-blue-200">tok/s</div>
              </div>
              <div className="grid gap-2">
                <MiniWallStat label={labels.peak} value={formatRate(peakTps)} />
                <MiniWallStat label={labels.avg5m} value={formatRate(avgTps)} />
              </div>
            </div>
            <TrendChart values={trendValues} emptyText={labels.noSamples} className="border-slate-200 bg-white/70 dark:border-slate-700 dark:bg-slate-950/50" />
          </div>
          <div className="mt-4 rounded-lg border border-slate-200 bg-white/70 p-3 dark:border-slate-700 dark:bg-slate-950/50">
            <div className="mb-2 flex items-center justify-between gap-3">
              <div className="text-sm font-semibold text-slate-900 dark:text-slate-100">{labels.activeRequests}</div>
              <Badge tone={activeRequestTasks.length > 0 ? 'emerald' : 'slate'}>{activeRequestTasks.length}</Badge>
            </div>
            {activeRequestTasks.length === 0 ? (
              <EmptyDark text={labels.noActiveRequests} />
            ) : (
              <div className="space-y-2">
                {activeRequestTasks.map(task => (
                  <ActiveWallRequest key={`${task.instanceName}-${task.task_id}`} task={task} />
                ))}
              </div>
            )}
          </div>
        </WallPanel>

        <WallPanel title={labels.resourcePressure} icon={<Cpu className="h-5 w-5" />} action={<Badge tone="blue">5分钟</Badge>}>
          <div className="space-y-3">
            {resourceSignals.map(signal => (
              <ResourcePressureRow
                key={signal.id}
                label={signal.label}
                value={signal.value}
                detail={signal.detail}
                tone={signal.tone}
                sparkline={signal.sparkline}
                icon={signal.id === 'cpu' ? <Cpu className="h-5 w-5" /> : signal.id === 'gpu' ? <Gauge className="h-5 w-5" /> : signal.id === 'memory' ? <Database className="h-5 w-5" /> : <HardDrive className="h-5 w-5" />}
              />
            ))}
          </div>
        </WallPanel>
      </section>

      <section className="grid gap-3 xl:grid-cols-3">
        <WallPanel
          title={labels.instanceStatus}
          icon={<Server className="h-5 w-5" />}
          action={<div className="flex gap-3 text-sm"><span className="text-emerald-700 dark:text-emerald-300">{labels.running} {runningInstances.length}</span><span className="text-slate-500 dark:text-slate-400">{labels.stopped} {stoppedCount}</span><span className="text-red-700 dark:text-red-300">{labels.error} {errorCount}</span></div>}
        >
          <div className="space-y-2">
            {instances.length === 0 ? (
              <EmptyDark text={labels.noInstances} />
            ) : instances.slice(0, 5).map(instance => (
              <InstanceWallRow key={instance.id} instance={instance} labels={labels} sessions={sessions} />
            ))}
          </div>
        </WallPanel>

        <WallPanel
          title={labels.downloadQueue}
          icon={<Download className="h-5 w-5" />}
          action={<div className="flex gap-3 text-sm"><span className="text-blue-700 dark:text-blue-300">{labels.active} {downloadStats.active}</span><span className="text-amber-700 dark:text-amber-300">{labels.queued} {downloadStats.queued}</span><span className="text-red-700 dark:text-red-300">{labels.failed} {downloadStats.failed}</span></div>}
        >
          <div className="space-y-3">
            <div className="rounded-lg border border-slate-200 bg-white/70 p-3 dark:border-slate-700 dark:bg-slate-950/50">
              <div className="text-xs text-slate-500">{labels.totalSpeed}</div>
              <div className="mt-1 text-2xl font-semibold text-blue-700 dark:text-blue-300">{formatBytesPerSecond(downloadStats.speed)}</div>
            </div>
            {recentDownloads.length === 0 ? (
              <EmptyDark text={labels.noDownloads} />
            ) : recentDownloads.map(task => (
              <DownloadWallRow key={task.id} task={task} labels={labels} />
            ))}
          </div>
        </WallPanel>

        <WallPanel title={labels.activityFeed} icon={<Terminal className="h-5 w-5" />} action={<Badge tone="slate">{labels.all}</Badge>}>
          {activityFeed.length === 0 ? (
            <EmptyDark text={labels.noActivity} />
          ) : (
            <div className="space-y-1">
              {activityFeed.map(item => (
                <ActivityWallRow key={item.id} item={item} />
              ))}
            </div>
          )}
        </WallPanel>
      </section>

      <footer className="grid gap-3 rounded-lg border border-slate-200 bg-white px-4 py-3 text-xs text-slate-500 md:grid-cols-[minmax(0,1fr)_auto] dark:border-slate-800 dark:bg-slate-900/70">
        <div className="flex min-w-0 flex-wrap gap-5">
          <span>{labels.models}: {modelCount}</span>
          <span>{labels.engines}: {engines.length}</span>
          <span>{labels.sessions24h}: {overview.sessions_24h}</span>
          <span>{labels.lastUpdated}: {formatTime(lastUpdatedAt)}</span>
        </div>
        <div className="text-emerald-700 dark:text-emerald-300">{labels.dataLinkNormal}</div>
      </footer>
    </div>
  )
}

function WallPanel({ title, icon, action, children }: { title: string; icon: ReactNode; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="min-w-0 overflow-hidden rounded-lg border border-slate-200 bg-white text-slate-950 shadow-[0_18px_54px_rgba(15,23,42,0.08)] dark:border-slate-700/80 dark:bg-slate-900/90 dark:text-slate-100 dark:shadow-[0_18px_54px_rgba(2,6,23,0.32)]">
      <div className="flex h-12 items-center justify-between gap-3 border-b border-slate-200 px-4 dark:border-slate-700/80">
        <div className="flex min-w-0 items-center gap-2">
          <span className="text-blue-700 dark:text-blue-300">{icon}</span>
          <h3 className="truncate text-base font-semibold">{title}</h3>
        </div>
        {action ? <div className="shrink-0">{action}</div> : null}
      </div>
      <div className="p-4">{children}</div>
    </section>
  )
}

function WallKpi({ label, value, detail, tone, icon }: { label: string; value: ReactNode; detail: string; tone: SignalTone; icon: ReactNode }) {
  const toneClass = wallTone(tone)
  return (
    <div className="min-w-0 rounded-lg border border-slate-200 bg-white px-5 py-4 shadow-[0_12px_42px_rgba(15,23,42,0.08)] dark:border-slate-700 dark:bg-slate-900/90 dark:shadow-[0_12px_42px_rgba(2,6,23,0.22)]">
      <div className="flex min-w-0 items-center gap-4">
        <div className={joinClassNames('flex h-14 w-14 shrink-0 items-center justify-center rounded-lg border', toneClass.box)}>
          {icon}
        </div>
        <div className="min-w-0">
          <div className="truncate text-sm text-slate-600 dark:text-slate-400">{label}</div>
          <div className={joinClassNames('mt-1 truncate text-3xl font-semibold', toneClass.text)} title={String(value)}>{value}</div>
          <div className="mt-1 truncate text-xs text-slate-500" title={detail}>{detail}</div>
        </div>
      </div>
    </div>
  )
}

function ResourcePressureRow({ label, value, detail, tone, sparkline, icon }: { label: string; value: number; detail: string; tone: SignalTone; sparkline: number[]; icon: ReactNode }) {
  const safe = Math.max(0, Math.min(100, Math.round(Number.isFinite(value) ? value : 0)))
  const toneClass = wallTone(tone)
  return (
    <div className="grid min-w-0 grid-cols-[48px_110px_78px_minmax(120px,1fr)_140px] items-center gap-3 rounded-lg border border-slate-200 bg-slate-50 px-4 py-3 dark:border-slate-700 dark:bg-slate-950/50">
      <div className={joinClassNames('flex h-10 w-10 items-center justify-center rounded-lg border', toneClass.box)}>{icon}</div>
      <div className="min-w-0">
        <div className="truncate text-base font-semibold text-slate-900 dark:text-slate-100">{label}</div>
        <div className="truncate text-xs text-slate-500" title={detail}>{detail}</div>
      </div>
      <div className="text-3xl font-semibold text-slate-950 dark:text-white">{safe}%</div>
      <div className="h-2 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-800">
        <div className={joinClassNames('h-full rounded-full', toneClass.bar)} style={{ width: `${safe}%` }} />
      </div>
      <MiniSparkline values={sparkline} tone={tone} />
    </div>
  )
}

function ActiveWallRequest({ task }: { task: RunningInferenceTask & { instanceName: string } }) {
  const total = Math.max(task.total_tokens || 8192, task.n_decoded || 1)
  const progress = Math.max(4, Math.min(100, ((task.n_decoded || 0) / total) * 100))
  return (
    <div className="grid min-w-0 grid-cols-[72px_minmax(0,1fr)_110px_82px] items-center gap-3 text-sm">
      <span className="text-emerald-700 dark:text-emerald-300">#{task.task_id}</span>
      <div className="min-w-0">
        <div className="truncate text-slate-700 dark:text-slate-200" title={task.instanceName}>{task.instanceName}</div>
        <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-800">
          <div className="h-full rounded-full bg-blue-400" style={{ width: `${progress}%` }} />
        </div>
      </div>
      <span className="text-blue-700 dark:text-blue-200">{formatRate(task.tg)}</span>
      <span className="text-slate-500 dark:text-slate-400">{formatDuration((Date.now() - task.started_at_ms) / 1000)}</span>
    </div>
  )
}

function InstanceWallRow({ instance, labels, sessions }: { instance: Instance; labels: ReturnType<typeof getLabels>; sessions: TelemetrySessionSummary[] }) {
  const isRunning = instance.status === 'running'
  const latestSession = sessions.find(session => session.instance_id === instance.id)
  const tone = instance.status === 'error' ? 'red' : isRunning ? 'emerald' : 'slate'
  return (
    <div className="grid min-w-0 grid-cols-[84px_minmax(0,1fr)_112px] items-center gap-3 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 dark:border-slate-700 dark:bg-slate-950/50">
      <Badge tone={tone}>{isRunning ? labels.running : instance.status === 'error' ? labels.error : labels.stopped}</Badge>
      <div className="min-w-0">
        <div className="truncate text-sm font-semibold text-slate-900 dark:text-slate-100" title={instance.name}>{instance.name}</div>
        <div className="truncate font-mono text-xs text-slate-500">{instance.config.host}:{instance.config.port}</div>
      </div>
      <div className="text-right text-sm text-blue-700 dark:text-blue-200">{formatRate(latestSession?.avg_tokens_per_sec)}</div>
    </div>
  )
}

function DownloadWallRow({ task, labels }: { task: DownloadProgress; labels: ReturnType<typeof getLabels> }) {
  const progress = task.total > 0 ? Math.max(0, Math.min(100, (task.downloaded / task.total) * 100)) : 0
  return (
    <div className="rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 dark:border-slate-700 dark:bg-slate-950/50">
      <div className="flex min-w-0 items-center justify-between gap-3">
        <div className="min-w-0 truncate text-sm font-semibold text-slate-900 dark:text-slate-100" title={task.fileName}>{task.fileName}</div>
        <Badge tone={downloadTone(task.status)}>{downloadStatusText(task.status, labels)}</Badge>
      </div>
      <div className="mt-2 grid grid-cols-[minmax(0,1fr)_54px] items-center gap-3">
        <div className="h-2 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-800">
          <div className="h-full rounded-full bg-blue-400" style={{ width: `${progress}%` }} />
        </div>
        <span className="text-right text-xs text-slate-500 dark:text-slate-400">{Math.round(progress)}%</span>
      </div>
      <div className="mt-1 flex justify-between gap-3 text-xs text-slate-500">
        <span>{formatBytes(task.downloaded)} / {formatBytes(task.total)}</span>
        <span>{formatBytesPerSecond(task.speed)}</span>
      </div>
    </div>
  )
}

function ActivityWallRow({ item }: { item: ActivityFeedItem }) {
  const tone = item.severity === 'critical' ? 'red' : item.severity === 'warning' ? 'amber' : item.severity === 'success' ? 'emerald' : 'blue'
  return (
    <div className="grid min-w-0 grid-cols-[58px_70px_minmax(0,1fr)] items-center gap-3 border-b border-slate-200 px-1 py-2 last:border-b-0 dark:border-slate-800">
      <span className="text-xs text-slate-500">{formatTime(item.ts).slice(0, 5)}</span>
      <Badge tone={tone}>{item.label}</Badge>
      <div className="min-w-0">
        <div className="truncate text-sm text-slate-700 dark:text-slate-200" title={item.title}>{item.title}</div>
        <div className="truncate text-xs text-slate-500" title={item.detail}>{item.detail}</div>
      </div>
    </div>
  )
}

function MiniWallStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-slate-200 bg-slate-50 p-3 dark:border-slate-700 dark:bg-slate-950/50">
      <div className="text-xs text-slate-500">{label}</div>
      <div className="mt-1 text-lg font-semibold text-slate-900 dark:text-slate-100">{value}</div>
    </div>
  )
}

function EmptyDark({ text }: { text: string }) {
  return (
    <div className="flex min-h-[88px] items-center justify-center rounded-lg border border-dashed border-slate-300 bg-slate-50 px-4 text-center text-sm text-slate-500 dark:border-slate-700 dark:bg-slate-950/40">
      <CheckCircle2 className="mr-2 h-4 w-4 text-slate-400 dark:text-slate-600" />
      {text}
    </div>
  )
}

function downloadTone(status: DownloadProgress['status']) {
  if (status === 'active') return 'blue'
  if (status === 'completed') return 'emerald'
  if (status === 'error') return 'red'
  if (status === 'paused' || status === 'pausing') return 'amber'
  if (status === 'queued') return 'violet'
  return 'slate'
}

function downloadStatusText(status: DownloadProgress['status'], labels: ReturnType<typeof getLabels>) {
  if (status === 'active') return labels.active
  if (status === 'completed') return labels.completed
  if (status === 'error') return labels.failed
  if (status === 'paused' || status === 'pausing') return labels.paused
  if (status === 'queued') return labels.queued
  return labels.cancelled
}

function wallTone(tone: SignalTone) {
  const map: Record<SignalTone, { text: string; box: string; bar: string }> = {
    blue: { text: 'text-blue-700 dark:text-blue-300', box: 'border-blue-200 bg-blue-50 text-blue-700 dark:border-blue-300/25 dark:bg-blue-400/10 dark:text-blue-200', bar: 'bg-blue-500 dark:bg-blue-400' },
    emerald: { text: 'text-emerald-700 dark:text-emerald-300', box: 'border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-300/25 dark:bg-emerald-400/10 dark:text-emerald-200', bar: 'bg-emerald-500 dark:bg-emerald-400' },
    amber: { text: 'text-amber-700 dark:text-amber-300', box: 'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-300/25 dark:bg-amber-400/10 dark:text-amber-200', bar: 'bg-amber-500 dark:bg-amber-400' },
    red: { text: 'text-red-700 dark:text-red-300', box: 'border-red-200 bg-red-50 text-red-700 dark:border-red-300/25 dark:bg-red-400/10 dark:text-red-200', bar: 'bg-red-500 dark:bg-red-400' },
    violet: { text: 'text-violet-700 dark:text-violet-300', box: 'border-violet-200 bg-violet-50 text-violet-700 dark:border-violet-300/25 dark:bg-violet-400/10 dark:text-violet-200', bar: 'bg-violet-500 dark:bg-violet-400' },
    cyan: { text: 'text-cyan-700 dark:text-cyan-300', box: 'border-cyan-200 bg-cyan-50 text-cyan-700 dark:border-cyan-300/25 dark:bg-cyan-400/10 dark:text-cyan-200', bar: 'bg-cyan-500 dark:bg-cyan-400' },
    slate: { text: 'text-slate-700 dark:text-slate-300', box: 'border-slate-200 bg-slate-100 text-slate-700 dark:border-slate-600 dark:bg-slate-800/70 dark:text-slate-200', bar: 'bg-slate-500' },
  }
  return map[tone]
}

function average(values: number[]) {
  const valid = values.filter(value => Number.isFinite(value) && value > 0)
  if (valid.length === 0) return 0
  return valid.reduce((sum, value) => sum + value, 0) / valid.length
}

function getLabels(zh: boolean) {
  return {
    title: zh ? '大屏模式' : 'Big Screen',
    subtitle: zh ? '运行态势、请求吞吐与资源压力' : 'Runtime posture, request throughput, and resource pressure',
    wallboard: zh ? '只读态势屏' : 'Read-only wallboard',
    updated: zh ? '数据已更新' : 'Updated',
    refreshing: zh ? '刷新中' : 'Refreshing',
    serviceStatus: zh ? '服务状态' : 'Service Status',
    normal: zh ? '正常' : 'Normal',
    needsAttention: zh ? '需关注' : 'Needs Attention',
    abnormal: zh ? '异常' : 'Abnormal',
    allServicesNormal: zh ? '所有服务运行正常' : 'All services normal',
    runningInstances: zh ? '运行实例' : 'Running Instances',
    currentThroughput: zh ? '当前吞吐' : 'Current Throughput',
    requestPressure: zh ? '请求压力' : 'Request Pressure',
    alerts: zh ? '告警' : 'Alerts',
    stopped: zh ? '已停止' : 'Stopped',
    error: zh ? '异常' : 'Error',
    failed: zh ? '失败' : 'Failed',
    peak: zh ? '峰值' : 'Peak',
    high: zh ? '高' : 'High',
    medium: zh ? '中等' : 'Medium',
    realtimeThroughput: zh ? '实时吞吐' : 'Realtime Throughput',
    avg5m: zh ? '5分钟平均' : '5m Avg',
    noSamples: zh ? '暂无足够采样' : 'Not enough samples',
    activeRequests: zh ? '运行中请求' : 'Active Requests',
    noActiveRequests: zh ? '暂无运行中请求' : 'No active requests',
    resourcePressure: zh ? '资源压力' : 'Resource Pressure',
    process: zh ? '进程' : 'Process',
    system: zh ? '系统' : 'System',
    memory: zh ? '内存' : 'Memory',
    vram: zh ? '显存' : 'VRAM',
    gpuUnavailable: zh ? '未检测到 GPU' : 'GPU unavailable',
    instanceStatus: zh ? '实例状态' : 'Instance Status',
    running: zh ? '运行中' : 'Running',
    noInstances: zh ? '暂无实例' : 'No instances',
    downloadQueue: zh ? '下载队列' : 'Download Queue',
    active: zh ? '进行中' : 'Active',
    queued: zh ? '排队中' : 'Queued',
    paused: zh ? '已暂停' : 'Paused',
    completed: zh ? '已完成' : 'Completed',
    cancelled: zh ? '已取消' : 'Cancelled',
    noDownloads: zh ? '暂无下载任务' : 'No download tasks',
    totalSpeed: zh ? '总下载速度' : 'Total Speed',
    activityFeed: zh ? '活动流' : 'Activity Feed',
    all: zh ? '全部' : 'All',
    noActivity: zh ? '暂无活动' : 'No activity',
    request: zh ? '请求' : 'Request',
    download: zh ? '下载' : 'Download',
    log: zh ? '日志' : 'Log',
    models: zh ? '模型' : 'Models',
    engines: zh ? '引擎' : 'Engines',
    sessions24h: zh ? '24小时会话' : 'Sessions 24h',
    lastUpdated: zh ? '最后更新' : 'Last Updated',
    dataLinkNormal: zh ? '数据连接正常' : 'Data link healthy',
  }
}
