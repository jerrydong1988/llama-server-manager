import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  Clock,
  Cpu,
  Database,
  Download,
  Gauge,
  HardDrive,
  Monitor,
  Radio,
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
  LogEntry,
  SystemMetrics,
  TelemetryOverview,
  TelemetrySessionSummary,
} from '../store/types'
import { Badge, joinClassNames } from './ui'

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

const exceptionPattern = /(error|fail|failed|fatal|panic|exception|warn|warning|错误|失败|异常|告警|警告)/i

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
  const [liveLlama, setLiveLlama] = useState<MetricsEvent['llama']>(null)
  const [lastUpdatedAt, setLastUpdatedAt] = useState(Date.now())
  const [refreshing, setRefreshing] = useState(false)
  const [telemetryError, setTelemetryError] = useState<string | null>(null)
  const lastTelemetryRefreshRef = useRef(0)

  const refreshTelemetry = useCallback(async (options: { silent?: boolean } = {}) => {
    lastTelemetryRefreshRef.current = Date.now()
    if (!options.silent) {
      setRefreshing(true)
    }
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
      if (!options.silent) {
        setRefreshing(false)
      }
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

  const allDownloads = useMemo(() => Object.values(downloadTasks), [downloadTasks])
  const runningInstances = useMemo(() => instances.filter(instance => instance.status === 'running'), [instances])
  const attentionInstances = useMemo(
    () => instances.filter(instance => instance.status === 'error' || (instance.status === 'running' && instance.healthCheck === 'fail')),
    [instances],
  )
  const modelCount = useMemo(() => models.filter(model => !model.is_shard && model.file_type === 'model').length, [models])

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
      .slice(0, 6)
  }, [allDownloads])

  const logRows = useMemo(() => {
    const rows = Object.entries(logs)
      .flatMap(([instanceId, entries]) => entries.map(entry => ({ ...entry, instanceId })))
      .sort((left, right) => right.timestamp - left.timestamp)
    const exceptions = rows.filter(entry => exceptionPattern.test(entry.text))
    return (exceptions.length > 0 ? exceptions : rows).slice(0, 8)
  }, [logs])

  const latestSample = overview.latest_samples[0] || null
  const currentTps = liveLlama?.tokens_per_sec ?? latestSample?.tokens_per_sec ?? overview.avg_tokens_per_sec_24h
  const processing = liveLlama?.requests_processing ?? latestSample?.requests_processing ?? 0
  const deferred = liveLlama?.requests_deferred ?? latestSample?.requests_deferred ?? 0
  const vramPercent = sysMetrics?.vram_total_mb ? ((sysMetrics.vram_used_mb || 0) / sysMetrics.vram_total_mb) * 100 : null
  const memoryPercent = sysMetrics?.system_memory_total_mb ? ((sysMetrics.system_memory_used_mb || 0) / sysMetrics.system_memory_total_mb) * 100 : null

  return (
    <div className="min-h-full space-y-4 bg-slate-950 text-slate-100">
      <section className="overflow-hidden rounded-lg border border-cyan-400/20 bg-slate-950 shadow-[0_20px_80px_rgba(8,47,73,0.45)]">
        <div className="grid gap-4 border-b border-cyan-400/15 bg-slate-900/80 px-5 py-4 xl:grid-cols-[minmax(0,1fr)_auto] xl:items-center">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-3">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-cyan-300/30 bg-cyan-400/10 text-cyan-200">
                <Monitor className="h-5 w-5" />
              </div>
              <div className="min-w-0">
                <h2 className="truncate text-2xl font-semibold text-white">{labels.title}</h2>
                <p className="mt-1 truncate text-sm text-slate-400">{labels.subtitle}</p>
              </div>
              <Badge tone="emerald" className="border-emerald-300/30 bg-emerald-400/10 text-emerald-200">{labels.readOnly}</Badge>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-3 text-sm text-slate-300">
            <span className="inline-flex h-9 items-center gap-2 rounded-lg border border-slate-700 bg-slate-900 px-3">
              <Clock className="h-4 w-4 text-cyan-300" />
              {labels.updated}: {formatTime(lastUpdatedAt)}
            </span>
            {telemetryError && (
              <span className="inline-flex h-9 max-w-[360px] items-center rounded-lg border border-amber-400/30 bg-amber-400/10 px-3 text-xs text-amber-100" title={telemetryError}>
                {zh ? '遥测异常' : 'Telemetry stale'}: {telemetryError}
              </span>
            )}
            <button
              type="button"
              onClick={() => void refreshTelemetry()}
              className="inline-flex h-9 items-center gap-2 rounded-lg border border-cyan-300/30 bg-cyan-400/10 px-3 font-medium text-cyan-100 transition hover:bg-cyan-400/20 disabled:cursor-wait disabled:opacity-60"
              disabled={refreshing}
            >
              <Radio className={joinClassNames('h-4 w-4', refreshing && 'animate-pulse')} />
              {refreshing ? labels.refreshing : labels.refresh}
            </button>
          </div>
        </div>

        <div className="grid gap-3 p-4 md:grid-cols-2 xl:grid-cols-6">
          <KpiCard label={labels.running} value={`${runningInstances.length}/${instances.length}`} detail={labels.instances} icon={<Server className="h-5 w-5" />} tone="emerald" />
          <KpiCard label={labels.attention} value={attentionInstances.length} detail={attentionInstances.length > 0 ? labels.needsReview : labels.normal} icon={<AlertTriangle className="h-5 w-5" />} tone={attentionInstances.length > 0 ? 'amber' : 'emerald'} />
          <KpiCard label={labels.modelAssets} value={modelCount} detail={`${engines.length} ${labels.engines}`} icon={<Database className="h-5 w-5" />} tone="blue" />
          <KpiCard label={labels.transfer} value={formatBytesPerSecond(downloadStats.speed)} detail={`${downloadStats.active} ${labels.activeDownloads}`} icon={<Download className="h-5 w-5" />} tone="violet" />
          <KpiCard label={labels.throughput} value={formatRate(currentTps)} detail={`${overview.active_sessions} ${labels.activeSessions}`} icon={<Gauge className="h-5 w-5" />} tone="cyan" />
          <KpiCard label={labels.requests} value={`${processing}/${deferred}`} detail={labels.processingDeferred} icon={<Zap className="h-5 w-5" />} tone={deferred > 0 ? 'amber' : 'blue'} />
        </div>
      </section>

      <div className="grid gap-4 2xl:grid-cols-[minmax(0,1fr)_430px]">
        <div className="space-y-4">
          <section className="grid gap-4 xl:grid-cols-3">
            <Panel title={labels.resources} icon={<Cpu className="h-5 w-5" />}>
              <div className="grid gap-3">
                <Meter label="CPU" value={sysMetrics?.system_cpu_percent ?? sysMetrics?.cpu_percent ?? 0} detail={`${labels.process} ${formatPercent(sysMetrics?.cpu_percent)} / ${labels.system} ${formatPercent(sysMetrics?.system_cpu_percent)}`} tone="blue" />
                <Meter label="GPU" value={sysMetrics?.gpu_percent ?? 0} detail={sysMetrics?.gpu_vendor || labels.gpuUnavailable} tone="emerald" />
                <Meter label={labels.memory} value={memoryPercent ?? 0} detail={formatMemoryPair(sysMetrics?.system_memory_used_mb, sysMetrics?.system_memory_total_mb)} tone="violet" />
                <Meter label={labels.vram} value={vramPercent ?? 0} detail={formatMemoryPair(sysMetrics?.vram_used_mb, sysMetrics?.vram_total_mb)} tone="amber" />
              </div>
            </Panel>

            <Panel title={labels.instanceBoard} icon={<Server className="h-5 w-5" />} className="xl:col-span-2">
              {instances.length === 0 ? (
                <EmptyLine text={labels.noInstances} />
              ) : (
                <div className="grid gap-2 lg:grid-cols-2">
                  {instances.slice(0, 8).map(instance => (
                    <InstanceLine key={instance.id} instance={instance} labels={labels} />
                  ))}
                </div>
              )}
            </Panel>
          </section>

          <section className="grid gap-4 xl:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
            <Panel title={labels.downloads} icon={<Download className="h-5 w-5" />}>
              <div className="mb-3 grid grid-cols-4 gap-2 text-center">
                <MiniCounter label={labels.active} value={downloadStats.active} tone="text-blue-200" />
                <MiniCounter label={labels.queued} value={downloadStats.queued} tone="text-violet-200" />
                <MiniCounter label={labels.failed} value={downloadStats.failed} tone="text-rose-200" />
                <MiniCounter label={labels.completed} value={downloadStats.completed} tone="text-emerald-200" />
              </div>
              <div className="space-y-2">
                {recentDownloads.length === 0 ? (
                  <EmptyLine text={labels.noDownloads} />
                ) : recentDownloads.map(task => (
                  <DownloadLine key={task.id} task={task} labels={labels} />
                ))}
              </div>
            </Panel>

            <Panel title={labels.performance} icon={<Activity className="h-5 w-5" />}>
              <div className="mb-3 grid gap-2 md:grid-cols-3">
                <MiniCounter label={labels.sessions24h} value={overview.sessions_24h} tone="text-cyan-200" />
                <MiniCounter label={labels.avg24h} value={formatRate(overview.avg_tokens_per_sec_24h)} tone="text-emerald-200" />
                <MiniCounter label={labels.peakVram} value={formatMemory(overview.peak_vram_mb_24h)} tone="text-amber-200" />
              </div>
              <RequestTable requests={requests} labels={labels} />
            </Panel>
          </section>
        </div>

        <div className="space-y-4">
          <Panel title={labels.alertLogs} icon={<Terminal className="h-5 w-5" />}>
            <div className="space-y-2">
              {logRows.length === 0 ? (
                <EmptyLine text={labels.noLogs} />
              ) : logRows.map((entry, index) => (
                <LogLine key={`${entry.instanceId}-${entry.timestamp}-${index}`} entry={entry} instances={instances} />
              ))}
            </div>
          </Panel>

          <Panel title={labels.sessionSnapshot} icon={<HardDrive className="h-5 w-5" />}>
            <div className="space-y-2">
              {sessions.length === 0 ? (
                <EmptyLine text={labels.noSessions} />
              ) : sessions.slice(0, 5).map(session => (
                <div key={session.id} className="rounded-lg border border-slate-700 bg-slate-900/85 px-3 py-2">
                  <div className="flex min-w-0 items-center justify-between gap-3">
                    <div className="min-w-0">
                      <div className="truncate text-sm font-semibold text-slate-100" title={session.instance_name}>{session.instance_name}</div>
                      <div className="mt-1 truncate text-xs text-slate-500" title={session.model_name}>{session.model_name || labels.unknown}</div>
                    </div>
                    <Badge tone={session.stopped_at ? 'slate' : 'emerald'} className="shrink-0">
                      {session.stopped_at ? labels.finished : labels.runningNow}
                    </Badge>
                  </div>
                  <div className="mt-2 grid grid-cols-3 gap-2 text-xs text-slate-400">
                    <span className="truncate">{formatRate(session.avg_tokens_per_sec)}</span>
                    <span className="truncate">{formatMemory(session.peak_vram_mb)}</span>
                    <span className="truncate">{formatDuration(session.duration_secs)}</span>
                  </div>
                </div>
              ))}
            </div>
          </Panel>
        </div>
      </div>
    </div>
  )
}

function Panel({
  title,
  icon,
  children,
  className = '',
}: {
  title: string
  icon: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <section className={joinClassNames('min-w-0 rounded-lg border border-slate-700/80 bg-slate-900/90 text-slate-100 shadow-[0_18px_54px_rgba(2,6,23,0.35)]', className)}>
      <div className="flex h-12 items-center justify-between gap-3 border-b border-slate-700/80 px-4">
        <div className="flex min-w-0 items-center gap-2">
          <span className="text-cyan-300">{icon}</span>
          <h3 className="truncate text-sm font-semibold uppercase text-slate-100">{title}</h3>
        </div>
      </div>
      <div className="p-4">{children}</div>
    </section>
  )
}

function KpiCard({
  label,
  value,
  detail,
  icon,
  tone,
}: {
  label: string
  value: ReactNode
  detail: string
  icon: ReactNode
  tone: 'blue' | 'emerald' | 'amber' | 'violet' | 'cyan'
}) {
  const toneClass = {
    blue: 'border-blue-300/20 bg-blue-400/10 text-blue-200',
    emerald: 'border-emerald-300/20 bg-emerald-400/10 text-emerald-200',
    amber: 'border-amber-300/20 bg-amber-400/10 text-amber-200',
    violet: 'border-violet-300/20 bg-violet-400/10 text-violet-200',
    cyan: 'border-cyan-300/20 bg-cyan-400/10 text-cyan-200',
  }[tone]

  return (
    <div className="min-w-0 rounded-lg border border-slate-700 bg-slate-900 px-4 py-3">
      <div className="flex min-w-0 items-center justify-between gap-3">
        <div className="truncate text-xs uppercase text-slate-400">{label}</div>
        <div className={joinClassNames('rounded-lg border p-2', toneClass)}>{icon}</div>
      </div>
      <div className="mt-3 truncate text-3xl font-semibold text-white" title={String(value)}>{value}</div>
      <div className="mt-1 truncate text-xs text-slate-500" title={detail}>{detail}</div>
    </div>
  )
}

function Meter({
  label,
  value,
  detail,
  tone,
}: {
  label: string
  value: number
  detail: string
  tone: 'blue' | 'emerald' | 'amber' | 'violet'
}) {
  const safe = Math.max(0, Math.min(100, Math.round(Number.isFinite(value) ? value : 0)))
  const bar = {
    blue: 'bg-blue-400',
    emerald: 'bg-emerald-400',
    amber: 'bg-amber-400',
    violet: 'bg-violet-400',
  }[tone]

  return (
    <div className="rounded-lg border border-slate-700 bg-slate-950/60 p-3">
      <div className="flex items-center justify-between gap-3">
        <span className="truncate text-sm text-slate-300">{label}</span>
        <span className="text-lg font-semibold text-white">{safe}%</span>
      </div>
      <div className="mt-2 h-2 overflow-hidden rounded-full bg-slate-800">
        <div className={joinClassNames('h-full rounded-full transition-[width]', bar)} style={{ width: `${safe}%` }} />
      </div>
      <div className="mt-2 truncate text-xs text-slate-500" title={detail}>{detail}</div>
    </div>
  )
}

function InstanceLine({ instance, labels }: { instance: Instance; labels: ReturnType<typeof getLabels> }) {
  const isRunning = instance.status === 'running'
  const unhealthy = instance.status === 'error' || (isRunning && instance.healthCheck === 'fail')
  const healthy = isRunning && instance.healthCheck === 'ok'
  const tone = unhealthy ? 'red' : healthy ? 'emerald' : isRunning ? 'amber' : 'slate'

  return (
    <div className="grid min-h-[72px] min-w-0 grid-cols-[minmax(0,1fr)_auto] gap-3 rounded-lg border border-slate-700 bg-slate-950/60 px-3 py-2">
      <div className="min-w-0">
        <div className="truncate text-sm font-semibold text-slate-100" title={instance.name}>{instance.name}</div>
        <div className="mt-1 truncate text-xs text-slate-500" title={instance.model}>{instance.model || labels.unknown}</div>
        <div className="mt-1 truncate font-mono text-xs text-cyan-200">{instance.config.host}:{instance.config.port}</div>
      </div>
      <div className="flex flex-col items-end justify-between gap-2">
        <Badge tone={tone}>{isRunning ? labels.runningNow : instance.status === 'error' ? labels.error : labels.stopped}</Badge>
        <span className="text-xs text-slate-500">{formatDuration(instance.startTime ? (Date.now() - instance.startTime) / 1000 : null)}</span>
      </div>
    </div>
  )
}

function DownloadLine({ task, labels }: { task: DownloadProgress; labels: ReturnType<typeof getLabels> }) {
  const progress = task.total > 0 ? Math.max(0, Math.min(100, (task.downloaded / task.total) * 100)) : 0
  return (
    <div className="rounded-lg border border-slate-700 bg-slate-950/60 px-3 py-2">
      <div className="flex min-w-0 items-center justify-between gap-3">
        <div className="min-w-0 truncate text-sm text-slate-200" title={task.fileName}>{task.fileName}</div>
        <Badge tone={downloadTone(task.status)} className="shrink-0">{downloadStatusText(task.status, labels)}</Badge>
      </div>
      <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-slate-800">
        <div className="h-full rounded-full bg-blue-400" style={{ width: `${progress}%` }} />
      </div>
      <div className="mt-1 flex justify-between gap-3 text-xs text-slate-500">
        <span className="truncate">{task.repoId || labels.unknown}</span>
        <span className="shrink-0">{formatBytesPerSecond(task.speed)}</span>
      </div>
    </div>
  )
}

function RequestTable({ requests, labels }: { requests: InferenceRequestSummary[]; labels: ReturnType<typeof getLabels> }) {
  if (requests.length === 0) {
    return <EmptyLine text={labels.noRequests} />
  }

  return (
    <div className="overflow-hidden rounded-lg border border-slate-700">
      <table className="w-full table-fixed text-left text-xs">
        <thead className="bg-slate-950 text-slate-500">
          <tr>
            <th className="px-3 py-2 font-medium">{labels.task}</th>
            <th className="px-3 py-2 font-medium">{labels.source}</th>
            <th className="px-3 py-2 font-medium">{labels.tokens}</th>
            <th className="px-3 py-2 font-medium">{labels.speed}</th>
            <th className="px-3 py-2 font-medium">{labels.duration}</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-slate-700 bg-slate-900/70 text-slate-300">
          {requests.map(request => (
            <tr key={`${request.session_id}-${request.task_id}-${request.completed_at}`}>
              <td className="truncate px-3 py-2">{request.source === 'proxy' ? (request.model || `#${request.task_id}`) : `#${request.task_id}`}</td>
              <td className="truncate px-3 py-2">{request.source === 'proxy' ? `${labels.proxy}${request.http_status ? ` ${request.http_status}` : ''}` : labels.log}</td>
              <td className="truncate px-3 py-2">{formatNumber(request.total_tokens)}</td>
              <td className="truncate px-3 py-2">{formatRate(request.generation_tps)}</td>
              <td className="truncate px-3 py-2">
                {request.source === 'proxy' ? `${labels.firstResponse} ${formatMs(request.total_time_ms)}` : formatMs(request.total_time_ms)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

function LogLine({ entry, instances }: { entry: LogEntry; instances: Instance[] }) {
  const instanceName = instances.find(instance => instance.id === entry.instanceId)?.name || entry.instanceId
  const important = exceptionPattern.test(entry.text)

  return (
    <div className={joinClassNames('rounded-lg border px-3 py-2', important ? 'border-amber-400/30 bg-amber-400/10' : 'border-slate-700 bg-slate-950/60')}>
      <div className="mb-1 flex min-w-0 items-center justify-between gap-3 text-xs">
        <span className="truncate text-slate-400" title={instanceName}>{instanceName}</span>
        <span className="shrink-0 text-slate-500">{formatTime(entry.timestamp)}</span>
      </div>
      <div className="line-clamp-2 break-words font-mono text-xs leading-5 text-slate-200">{entry.text}</div>
    </div>
  )
}

function MiniCounter({ label, value, tone }: { label: string; value: ReactNode; tone: string }) {
  return (
    <div className="min-w-0 rounded-lg border border-slate-700 bg-slate-950/60 px-3 py-2">
      <div className="truncate text-[11px] uppercase text-slate-500">{label}</div>
      <div className={joinClassNames('mt-1 truncate text-lg font-semibold', tone)} title={String(value)}>{value}</div>
    </div>
  )
}

function EmptyLine({ text }: { text: string }) {
  return (
    <div className="flex min-h-[92px] items-center justify-center rounded-lg border border-dashed border-slate-700 bg-slate-950/45 px-4 text-center text-sm text-slate-500">
      <CheckCircle2 className="mr-2 h-4 w-4 text-slate-600" />
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

function formatRate(value?: number | null) {
  if (!value || !Number.isFinite(value)) return '--'
  return `${value.toFixed(value >= 10 ? 1 : 2)} tok/s`
}

function formatBytesPerSecond(value?: number | null) {
  if (!value || !Number.isFinite(value)) return '--'
  return `${formatBytes(value)}/s`
}

function formatBytes(value?: number | null) {
  if (!value || !Number.isFinite(value)) return '--'
  const gb = value / 1024 / 1024 / 1024
  if (gb >= 1) return `${gb.toFixed(1)} GB`
  const mb = value / 1024 / 1024
  if (mb >= 1) return `${mb.toFixed(0)} MB`
  return `${Math.max(1, Math.round(value / 1024))} KB`
}

function formatMemory(value?: number | null) {
  if (!value || !Number.isFinite(value)) return '--'
  if (value >= 1024) return `${(value / 1024).toFixed(1)} GB`
  return `${Math.round(value)} MB`
}

function formatMemoryPair(used?: number | null, total?: number | null) {
  if (!used || !total) return '--'
  return `${formatMemory(used)} / ${formatMemory(total)}`
}

function formatPercent(value?: number | null) {
  if (value == null || !Number.isFinite(value)) return '--'
  return `${Math.round(value)}%`
}

function formatDuration(seconds?: number | null) {
  if (!seconds || !Number.isFinite(seconds) || seconds < 0) return '--'
  const total = Math.floor(seconds)
  const hours = Math.floor(total / 3600)
  const minutes = Math.floor((total % 3600) / 60)
  const rest = total % 60
  if (hours > 0) return `${hours}h ${minutes}m`
  if (minutes > 0) return `${minutes}m ${rest}s`
  return `${rest}s`
}

function formatMs(value?: number | null) {
  if (value == null || !Number.isFinite(value)) return '--'
  if (value < 1000) return `${value.toFixed(0)} ms`
  return `${(value / 1000).toFixed(1)} s`
}

function formatNumber(value?: number | null) {
  if (value == null || !Number.isFinite(value)) return '--'
  return value.toLocaleString()
}

function formatTime(value: number) {
  return new Date(value).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

function getLabels(zh: boolean) {
  return {
    title: zh ? '只读大屏模式' : 'Read-Only Big Screen',
    subtitle: zh ? '面向投屏的本地 llama-server 总览，自动刷新且不执行写操作。' : 'Projection-ready local llama-server overview with automatic read-only refresh.',
    readOnly: zh ? '只读' : 'Read only',
    updated: zh ? '更新' : 'Updated',
    refresh: zh ? '刷新' : 'Refresh',
    refreshing: zh ? '刷新中' : 'Refreshing',
    running: zh ? '运行' : 'Running',
    runningNow: zh ? '运行中' : 'Running',
    stopped: zh ? '已停止' : 'Stopped',
    error: zh ? '错误' : 'Error',
    attention: zh ? '异常' : 'Attention',
    needsReview: zh ? '需要查看' : 'Needs review',
    normal: zh ? '正常' : 'Normal',
    instances: zh ? '实例' : 'instances',
    modelAssets: zh ? '模型资产' : 'Model Assets',
    engines: zh ? '个引擎' : 'engines',
    transfer: zh ? '传输' : 'Transfer',
    throughput: zh ? '吞吐' : 'Throughput',
    requests: zh ? '请求' : 'Requests',
    activeSessions: zh ? '活动会话' : 'active sessions',
    processingDeferred: zh ? '处理中 / 延迟' : 'Processing / deferred',
    activeDownloads: zh ? '进行中' : 'active',
    resources: zh ? '资源' : 'Resources',
    process: zh ? '进程' : 'Process',
    system: zh ? '系统' : 'System',
    memory: zh ? '内存' : 'Memory',
    vram: zh ? '显存' : 'VRAM',
    gpuUnavailable: zh ? '未检测到 GPU' : 'GPU unavailable',
    instanceBoard: zh ? '实例看板' : 'Instance Board',
    noInstances: zh ? '暂无实例' : 'No instances',
    unknown: zh ? '未知' : 'Unknown',
    downloads: zh ? '下载' : 'Downloads',
    active: zh ? '进行中' : 'Active',
    queued: zh ? '排队' : 'Queued',
    failed: zh ? '失败' : 'Failed',
    completed: zh ? '完成' : 'Completed',
    paused: zh ? '暂停' : 'Paused',
    cancelled: zh ? '取消' : 'Cancelled',
    noDownloads: zh ? '暂无下载任务' : 'No download tasks',
    performance: zh ? '请求 / 性能' : 'Requests / Performance',
    sessions24h: zh ? '24小时会话' : 'Sessions 24h',
    avg24h: zh ? '24小时均速' : 'Avg 24h',
    peakVram: zh ? '峰值显存' : 'Peak VRAM',
    task: zh ? '任务' : 'Task',
    source: zh ? '来源' : 'Source',
    proxy: zh ? '路由' : 'Proxy',
    log: zh ? '日志' : 'Log',
    tokens: zh ? '令牌' : 'Tokens',
    speed: zh ? '速度' : 'Speed',
    duration: zh ? '耗时' : 'Duration',
    firstResponse: zh ? '首响' : 'First byte',
    noRequests: zh ? '暂无已完成请求记录' : 'No completed request records',
    alertLogs: zh ? '异常日志' : 'Exception Logs',
    noLogs: zh ? '暂无日志' : 'No logs',
    sessionSnapshot: zh ? '运行会话' : 'Run Sessions',
    noSessions: zh ? '暂无遥测会话' : 'No telemetry sessions',
    finished: zh ? '已结束' : 'Finished',
  }
}
