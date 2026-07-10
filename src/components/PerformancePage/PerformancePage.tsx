import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Activity, AlertTriangle, BarChart3, Clock, Cpu, Gauge, HardDrive, Radio, RefreshCw, Server, Zap } from 'lucide-react'
import { useAppStore } from '../../store'
import { useI18n } from '../../i18n'
import type {
  DiagnosticFinding,
  InferenceRequestSummary,
  PerfUpdateEvent,
  RunningInferenceTask,
  SystemMetrics,
  TelemetryOverview,
  TelemetrySampleSummary,
  TelemetrySessionAnalysis,
  TelemetrySessionSummary,
} from '../../store/types'
import { Badge, Button, EmptyPanel, PageFrame, PageHeader } from '../ui'
import { ActiveRequestRow, ComparisonTable, MonitorPanel, SessionCard, SignalMeter, StatusTile, TrendChart } from '../monitoring/MonitoringPrimitives'
import { formatDuration, formatMemory, formatMs, formatRate, formatTime } from '../monitoring/monitoringFormat'
import { buildRequestPressure, buildResourceSignals, selectCurrentThroughput } from '../monitoring/monitoringViewModel'

type MetricsEvent = {
  instanceId: string
  system: SystemMetrics
  llama?: {
    tokens_per_sec?: number
    prompt_tokens_per_sec?: number
    prompt_tokens?: number
    gen_tokens?: number
    requests?: number
    requests_processing?: number
    requests_deferred?: number
    busy_slots_per_decode?: number
  } | null
  ts: number
}

type SessionBenchmark = {
  historyCount: number
  scope: 'sameConfig' | 'allHistory' | 'none'
  avg: {
    tokensPerSec: number | null
    peakVramMb: number | null
    durationSecs: number | null
  }
  best: TelemetrySessionSummary | null
}

const emptyOverview: TelemetryOverview = {
  active_sessions: 0,
  sessions_24h: 0,
  avg_tokens_per_sec_24h: 0,
  peak_vram_mb_24h: 0,
  latest_samples: [],
}

const TELEMETRY_OVERVIEW_REFRESH_MS = 10000
const TELEMETRY_DETAIL_REFRESH_MS = 5000

export default function PerformancePage() {
  const { lang } = useI18n()
  const zh = lang === 'zh-CN'
  const labels = getLabels(zh)
  const { instances } = useAppStore()
  const running = useMemo(() => instances.filter(instance => instance.status === 'running'), [instances])
  const [selectedInstanceId, setSelectedInstanceId] = useState('')
  const [selectedSessionId, setSelectedSessionId] = useState('')
  const [overview, setOverview] = useState<TelemetryOverview>(emptyOverview)
  const [sessions, setSessions] = useState<TelemetrySessionSummary[]>([])
  const [samples, setSamples] = useState<TelemetrySampleSummary[]>([])
  const [requests, setRequests] = useState<InferenceRequestSummary[]>([])
  const [analysis, setAnalysis] = useState<TelemetrySessionAnalysis | null>(null)
  const [diagnostics, setDiagnostics] = useState<DiagnosticFinding[]>([])
  const [runningTasksByInstance, setRunningTasksByInstance] = useState<Record<string, RunningInferenceTask[]>>({})
  const [liveSystem, setLiveSystem] = useState<SystemMetrics | null>(null)
  const [liveLlama, setLiveLlama] = useState<MetricsEvent['llama']>(null)
  const [loading, setLoading] = useState(false)
  const [telemetryError, setTelemetryError] = useState<string | null>(null)
  const [trendRange, setTrendRange] = useState<'1m' | '5m' | '15m' | '1h'>('5m')
  const lastTelemetryRefreshRef = useRef(0)
  const lastSessionDetailRefreshRef = useRef(0)
  const selectedSessionIdRef = useRef('')

  const selectedSession = sessions.find(session => session.id === selectedSessionId)
  const selectedInstance = instances.find(instance => instance.id === selectedInstanceId)
    || instances.find(instance => instance.id === selectedSession?.instance_id)
    || null
  const latestSample = overview.latest_samples[0] || null
  const activeTasks = selectedSession && !selectedSession.stopped_at
    ? runningTasksByInstance[selectedSession.instance_id] || []
    : selectedInstance
      ? runningTasksByInstance[selectedInstance.id] || []
      : []

  const refreshTelemetry = useCallback(async (options: { silent?: boolean } = {}) => {
    lastTelemetryRefreshRef.current = Date.now()
    if (!options.silent) setLoading(true)
    try {
      const [nextOverview, nextSessions] = await Promise.all([
        invoke<TelemetryOverview>('get_telemetry_overview'),
        invoke<TelemetrySessionSummary[]>('list_telemetry_sessions', { limit: 36 }),
      ])
      setOverview(nextOverview)
      setSessions(nextSessions)
      setSelectedSessionId(current => {
        const nextSessionId = current && nextSessions.some(session => session.id === current)
          ? current
          : nextSessions[0]?.id || ''
        selectedSessionIdRef.current = nextSessionId
        return nextSessionId
      })
      setTelemetryError(null)
    } catch (error) {
      setTelemetryError(error instanceof Error ? error.message : String(error))
    } finally {
      if (!options.silent) setLoading(false)
    }
  }, [])

  const refreshSelectedSessionDetails = useCallback(async (sessionId: string) => {
    if (!sessionId) return
    lastSessionDetailRefreshRef.current = Date.now()
    try {
      const [nextSamples, nextRequests, nextAnalysis, nextDiagnostics] = await Promise.all([
        invoke<TelemetrySampleSummary[]>('get_telemetry_session_samples', { sessionId, limit: 220 }),
        invoke<InferenceRequestSummary[]>('list_inference_requests', { sessionId, limit: 18 }),
        invoke<TelemetrySessionAnalysis>('get_telemetry_session_analysis', { sessionId }),
        invoke<DiagnosticFinding[]>('get_telemetry_session_diagnostics', { sessionId }),
      ])
      if (selectedSessionIdRef.current !== sessionId) return
      setSamples(nextSamples)
      setRequests(nextRequests)
      setAnalysis(nextAnalysis)
      setDiagnostics(nextDiagnostics)
      setTelemetryError(null)
    } catch (error) {
      if (selectedSessionIdRef.current === sessionId) {
        setTelemetryError(error instanceof Error ? error.message : String(error))
      }
    }
  }, [])

  useEffect(() => {
    void refreshTelemetry()
    const timer = window.setInterval(() => void refreshTelemetry({ silent: true }), TELEMETRY_OVERVIEW_REFRESH_MS)
    return () => window.clearInterval(timer)
  }, [refreshTelemetry])

  useEffect(() => {
    if (running.length > 0 && (!selectedInstanceId || !running.some(instance => instance.id === selectedInstanceId))) {
      setSelectedInstanceId(running[0].id)
    }
  }, [running, selectedInstanceId])

  useEffect(() => {
    if (!selectedInstanceId) return
    invoke<SystemMetrics>('get_system_metrics', { instanceId: selectedInstanceId })
      .then(setLiveSystem)
      .catch(() => setLiveSystem(null))

    const unlisten = listen<MetricsEvent>('metrics-update', event => {
      if (event.payload.instanceId !== selectedInstanceId) return
      setLiveSystem(event.payload.system)
      setLiveLlama(event.payload.llama || null)
      const now = Date.now()
      if (now - lastTelemetryRefreshRef.current > TELEMETRY_OVERVIEW_REFRESH_MS) {
        void refreshTelemetry({ silent: true })
      }
      if (selectedSessionIdRef.current && now - lastSessionDetailRefreshRef.current > TELEMETRY_DETAIL_REFRESH_MS) {
        void refreshSelectedSessionDetails(selectedSessionIdRef.current)
      }
    })

    return () => {
      unlisten.then(fn => fn())
    }
  }, [selectedInstanceId, refreshTelemetry, refreshSelectedSessionDetails])

  useEffect(() => {
    const unlisten = listen<PerfUpdateEvent>('perf-update', event => {
      setRunningTasksByInstance(current => ({
        ...current,
        [event.payload.instanceId]: event.payload.tasks || [],
      }))
      const now = Date.now()
      if (selectedSessionIdRef.current && now - lastSessionDetailRefreshRef.current > TELEMETRY_DETAIL_REFRESH_MS) {
        void refreshSelectedSessionDetails(selectedSessionIdRef.current)
      }
    })

    return () => {
      unlisten.then(fn => fn())
    }
  }, [refreshSelectedSessionDetails])

  useEffect(() => {
    selectedSessionIdRef.current = selectedSessionId
    if (!selectedSessionId) {
      setSamples([])
      setRequests([])
      setAnalysis(null)
      setDiagnostics([])
      return
    }
    const session = sessions.find(item => item.id === selectedSessionId)
    if (session) setSelectedInstanceId(session.instance_id)
    void refreshSelectedSessionDetails(selectedSessionId)
    const timer = window.setInterval(
      () => void refreshSelectedSessionDetails(selectedSessionId),
      TELEMETRY_DETAIL_REFRESH_MS,
    )
    return () => window.clearInterval(timer)
  }, [selectedSessionId, sessions, refreshSelectedSessionDetails])

  const trendSamples = useMemo(() => filterSamplesByRange(samples.length > 0 ? samples : overview.latest_samples, trendRange), [samples, overview.latest_samples, trendRange])
  const resourceSignals = useMemo(
    () => buildResourceSignals({
      system: liveSystem,
      latest: latestSample,
      samples: trendSamples,
      labels,
    }),
    [liveSystem, latestSample, trendSamples, labels],
  )
  const currentThroughput = selectCurrentThroughput(liveLlama?.tokens_per_sec, latestSample?.tokens_per_sec ?? selectedSession?.avg_tokens_per_sec)
  const pressure = buildRequestPressure(liveLlama?.requests_processing ?? latestSample?.requests_processing, liveLlama?.requests_deferred ?? latestSample?.requests_deferred)
  const sessionBenchmark = useMemo(() => buildSessionBenchmark(selectedSession, sessions), [selectedSession, sessions])
  const comparisonRows = useMemo(() => buildComparisonRows(selectedSession, sessionBenchmark, labels), [selectedSession, sessionBenchmark, labels])
  const visibleDiagnostics = diagnostics.slice(0, 3)

  return (
    <PageFrame
      className="text-slate-900 dark:text-slate-100"
      header={
        <PageHeader
          title={labels.title}
          description={labels.description}
          meta={<Badge tone="violet">{labels.sqliteBacked}</Badge>}
          actions={
            <Button icon={<RefreshCw className="h-4 w-4" />} onClick={() => void refreshTelemetry()} disabled={loading}>
              {labels.refresh}
            </Button>
          }
        />
      }
    >
      <div className="space-y-4">
        {telemetryError ? (
          <div className="rounded-lg border border-amber-300 bg-amber-50 p-4 text-sm text-amber-900 dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-100">
            <div className="font-semibold">{labels.telemetryError}</div>
            <div className="mt-1 truncate text-xs opacity-90" title={telemetryError}>{telemetryError}</div>
          </div>
        ) : null}

        <div className="grid min-h-[calc(100vh-178px)] gap-4 xl:grid-cols-[310px_minmax(0,1fr)_360px]">
          <aside className="min-w-0 space-y-4">
            <MonitorPanel title={labels.monitoringObject} icon={<Server className="h-5 w-5" />} action={<Badge tone="emerald">{running.length}</Badge>}>
              <div className="space-y-4">
                <div>
                  <label className="mb-2 block text-xs font-medium text-slate-500 dark:text-slate-400">{labels.runningInstance}</label>
                  <select
                    value={selectedInstanceId}
                    onChange={event => {
                      const nextId = event.target.value
                      setSelectedInstanceId(nextId)
                      const nextSession = sessions.find(session => session.instance_id === nextId && !session.stopped_at)
                        || sessions.find(session => session.instance_id === nextId)
                      if (nextSession) setSelectedSessionId(nextSession.id)
                    }}
                    className="select-custom h-11 w-full rounded-lg border border-slate-300 bg-white pl-3 pr-8 text-sm text-slate-900 dark:border-slate-700 dark:bg-slate-950 dark:text-slate-100"
                  >
                    {running.length === 0 ? <option value="">{labels.noRunning}</option> : null}
                    {running.map(instance => (
                      <option key={instance.id} value={instance.id}>{instance.name}</option>
                    ))}
                  </select>
                </div>

                <div className="space-y-2">
                  <div className="flex items-center justify-between gap-3">
                    <div className="text-xs font-semibold uppercase tracking-wide text-slate-500 dark:text-slate-400">{labels.sessionHistory}</div>
                    <Badge tone="blue">{sessions.length}</Badge>
                  </div>
                  <div className="max-h-[calc(100vh-374px)] space-y-2 overflow-y-auto pr-1">
                    {sessions.length === 0 ? (
                      <EmptyPanel title={labels.noHistory} description={labels.noHistoryDesc} />
                    ) : sessions.map(session => (
                      <SessionCard
                        key={session.id}
                        title={session.instance_name}
                        subtitle={session.model_name || labels.unknown}
                        meta={`${formatRate(session.avg_tokens_per_sec)} · ${formatMemory(session.peak_vram_mb)} · ${formatTime(session.started_at)}`}
                        selected={session.id === selectedSessionId}
                        running={!session.stopped_at}
                        onClick={() => {
                          setSelectedSessionId(session.id)
                          setSelectedInstanceId(session.instance_id)
                        }}
                      />
                    ))}
                  </div>
                </div>
              </div>
            </MonitorPanel>
          </aside>

          <main className="min-w-0 space-y-4">
            <section className="rounded-lg border border-slate-200 bg-white p-4 shadow-sm dark:border-slate-800 dark:bg-slate-900/70">
              <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_180px_190px] xl:items-center">
                <div className="min-w-0">
                  <div className="flex min-w-0 flex-wrap items-center gap-2">
                    <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg border border-blue-300/40 bg-blue-500/10 text-blue-600 dark:text-blue-300">
                      <Server className="h-5 w-5" />
                    </div>
                    <div className="min-w-0">
                      <div className="flex min-w-0 items-center gap-2">
                        <h2 className="truncate text-xl font-semibold text-slate-950 dark:text-white" title={selectedInstance?.name || selectedSession?.instance_name || labels.noSelection}>
                          {selectedInstance?.name || selectedSession?.instance_name || labels.noSelection}
                        </h2>
                        <Badge tone={selectedInstance?.status === 'running' || selectedSession?.stopped_at == null ? 'emerald' : 'slate'}>
                          {selectedInstance?.status === 'running' || selectedSession?.stopped_at == null ? labels.running : labels.finished}
                        </Badge>
                      </div>
                      <div className="mt-1 truncate font-mono text-xs text-blue-600 dark:text-blue-300">
                        {selectedInstance ? `${selectedInstance.config.host}:${selectedInstance.config.port}` : selectedSession?.model_name || '--'}
                      </div>
                    </div>
                  </div>
                </div>
                <StatusTile label={labels.currentTps} value={formatRate(currentThroughput)} detail={labels.fromLlamaMetrics} icon={<Gauge className="h-5 w-5" />} tone="blue" className="py-3" />
                <StatusTile
                  label={labels.queuePressure}
                  value={`${pressure.percent}%`}
                  detail={`${pressure.active} / ${pressure.queued} ${labels.processingDeferred}`}
                  icon={<Zap className="h-5 w-5" />}
                  tone={pressure.level === 'high' ? 'amber' : pressure.level === 'medium' ? 'cyan' : 'emerald'}
                  className="py-3"
                />
              </div>
            </section>

            <div className="grid gap-3 lg:grid-cols-2 2xl:grid-cols-4">
              {resourceSignals.map(signal => (
                <SignalMeter
                  key={signal.id}
                  label={signal.label}
                  value={signal.value}
                  detail={signal.detail}
                  tone={signal.tone}
                  icon={signal.id === 'cpu' ? <Cpu className="h-4 w-4" /> : signal.id === 'gpu' ? <Gauge className="h-4 w-4" /> : <HardDrive className="h-4 w-4" />}
                  sparkline={signal.sparkline}
                  compact
                />
              ))}
            </div>

            <MonitorPanel
              title={labels.throughputTrend}
              icon={<BarChart3 className="h-5 w-5" />}
              action={
                <div className="flex rounded-lg border border-slate-200 bg-slate-100 p-1 dark:border-slate-800 dark:bg-slate-950">
                  {(['1m', '5m', '15m', '1h'] as const).map(range => (
                    <button
                      key={range}
                      type="button"
                      onClick={() => setTrendRange(range)}
                      className={`h-7 rounded-md px-2 text-xs font-medium transition ${trendRange === range ? 'bg-blue-600 text-white' : 'text-slate-500 hover:text-slate-900 dark:hover:text-slate-100'}`}
                    >
                      {labels.ranges[range]}
                    </button>
                  ))}
                </div>
              }
            >
              <TrendChart values={trendSamples.map(sample => sample.tokens_per_sec || 0)} emptyText={labels.noSamplesYet} />
            </MonitorPanel>

            <MonitorPanel title={labels.activeRequests} icon={<Radio className="h-5 w-5" />} action={<Badge tone={activeTasks.length > 0 ? 'emerald' : 'slate'}>{activeTasks.length}</Badge>}>
              {activeTasks.length === 0 ? (
                <div className="rounded-lg border border-dashed border-slate-300 px-4 py-8 text-center text-sm text-slate-500 dark:border-slate-700 dark:text-slate-400">
                  {selectedSession?.stopped_at ? labels.sessionFinishedNoActive : labels.noActiveRequests}
                </div>
              ) : (
                <div className="grid gap-2">
                  {activeTasks.map(task => (
                    <ActiveRequestRow key={task.task_id} task={task} instanceName={selectedInstance?.name || selectedSession?.instance_name} />
                  ))}
                </div>
              )}
            </MonitorPanel>
          </main>

          <aside className="min-w-0 space-y-4">
            <MonitorPanel title={labels.diagnosis} icon={<AlertTriangle className="h-5 w-5" />} action={<Badge tone={diagnostics.length > 0 ? 'amber' : 'emerald'}>{diagnostics.length}</Badge>}>
              {visibleDiagnostics.length === 0 ? (
                <div className="rounded-lg border border-dashed border-slate-300 px-3 py-8 text-center text-sm text-slate-500 dark:border-slate-700 dark:text-slate-400">
                  {labels.noDiagnostics}
                </div>
              ) : (
                <div className="space-y-3">
                  {visibleDiagnostics.map(finding => (
                    <div key={finding.id} className={`rounded-lg border p-3 ${diagnosticPanelClass(finding.severity)}`}>
                      <div className="mb-2 flex items-start justify-between gap-3">
                        <div className="min-w-0 font-semibold text-slate-950 dark:text-slate-100">{finding.title}</div>
                        <Badge tone={diagnosticTone(finding.severity)}>{diagnosticSeverityLabel(finding.severity, labels)}</Badge>
                      </div>
                      <p className="text-xs leading-5 text-slate-600 dark:text-slate-400">{finding.summary}</p>
                      {finding.recommendation[0] ? (
                        <div className="mt-2 text-xs font-medium text-blue-600 dark:text-blue-300">{finding.recommendation[0]}</div>
                      ) : null}
                    </div>
                  ))}
                  {diagnostics.length > visibleDiagnostics.length ? (
                    <div className="text-center text-xs text-slate-500">{labels.moreDiagnostics(diagnostics.length - visibleDiagnostics.length)}</div>
                  ) : null}
                </div>
              )}
            </MonitorPanel>

            <MonitorPanel title={labels.historyBaseline} icon={<Clock className="h-5 w-5" />}>
              <div className="mb-3 text-xs leading-5 text-slate-500 dark:text-slate-400">
                {sessionComparisonScopeLabel(sessionBenchmark, labels)}
              </div>
              <ComparisonTable rows={comparisonRows} />
            </MonitorPanel>

            <MonitorPanel title={labels.sessionDigest} icon={<Activity className="h-5 w-5" />}>
              <div className="grid grid-cols-2 gap-2">
                <MiniStat label={labels.requests} value={analysis?.request_count ?? requests.length} />
                <MiniStat label={labels.avgGenerationSpeed} value={formatRate(analysis?.avg_generation_tps)} />
                <MiniStat label={labels.avgTotalTime} value={formatMs(analysis?.avg_total_time_ms)} />
                <MiniStat label={labels.maxBusySlots} value={analysis?.max_busy_slots ?? 0} />
              </div>
            </MonitorPanel>
          </aside>
        </div>
      </div>
    </PageFrame>
  )
}

function MiniStat({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="min-w-0 rounded-lg border border-slate-200 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-950/60">
      <div className="truncate text-xs text-slate-500">{label}</div>
      <div className="mt-2 truncate text-lg font-semibold text-slate-950 dark:text-slate-100" title={String(value)}>{value}</div>
    </div>
  )
}

function filterSamplesByRange(samples: TelemetrySampleSummary[], range: '1m' | '5m' | '15m' | '1h') {
  if (samples.length === 0) return []
  const now = Math.max(...samples.map(sample => sample.ts || 0), Date.now())
  const rangeMs = range === '1m' ? 60000 : range === '5m' ? 300000 : range === '15m' ? 900000 : 3600000
  const filtered = samples.filter(sample => now - sample.ts <= rangeMs)
  return filtered.length >= 2 ? filtered : samples.slice(-80)
}

function buildSessionBenchmark(selectedSession: TelemetrySessionSummary | undefined, sessions: TelemetrySessionSummary[]): SessionBenchmark {
  if (!selectedSession) return emptySessionBenchmark()
  const history = sessions.filter(session => session.id !== selectedSession.id && session.stopped_at && session.sample_count > 0)
  const sameConfig = history.filter(session =>
    session.model_name === selectedSession.model_name &&
    session.engine_id === selectedSession.engine_id &&
    session.backend === selectedSession.backend
  )
  const basis = sameConfig.length > 0 ? sameConfig : history
  if (basis.length === 0) return emptySessionBenchmark()
  return {
    historyCount: basis.length,
    scope: sameConfig.length > 0 ? 'sameConfig' : 'allHistory',
    avg: {
      tokensPerSec: average(basis.map(session => session.avg_tokens_per_sec)),
      peakVramMb: average(basis.map(session => session.peak_vram_mb)),
      durationSecs: average(basis.map(session => session.duration_secs)),
    },
    best: basis.reduce<TelemetrySessionSummary | null>((best, session) => {
      if (!best) return session
      return (session.avg_tokens_per_sec || 0) > (best.avg_tokens_per_sec || 0) ? session : best
    }, null),
  }
}

function emptySessionBenchmark(): SessionBenchmark {
  return {
    historyCount: 0,
    scope: 'none',
    avg: { tokensPerSec: null, peakVramMb: null, durationSecs: null },
    best: null,
  }
}

function average(values: Array<number | null | undefined>) {
  const valid = values.filter((value): value is number => value != null && Number.isFinite(value))
  if (valid.length === 0) return null
  return valid.reduce((sum, value) => sum + value, 0) / valid.length
}

function buildComparisonRows(selectedSession: TelemetrySessionSummary | undefined, benchmark: SessionBenchmark, labels: ReturnType<typeof getLabels>) {
  if (!selectedSession) return []
  return [
    {
      metric: labels.avgTps,
      current: formatRate(selectedSession.avg_tokens_per_sec),
      baseline: formatRate(benchmark.avg.tokensPerSec),
      ...formatDelta(selectedSession.avg_tokens_per_sec, benchmark.avg.tokensPerSec, true),
    },
    {
      metric: labels.peakVram,
      current: formatMemory(selectedSession.peak_vram_mb),
      baseline: formatMemory(benchmark.avg.peakVramMb),
      ...formatDelta(selectedSession.peak_vram_mb, benchmark.avg.peakVramMb, false),
    },
    {
      metric: labels.duration,
      current: formatDuration(selectedSession.duration_secs),
      baseline: formatDuration(benchmark.avg.durationSecs),
    },
  ]
}

function formatDelta(current?: number | null, baseline?: number | null, higherIsBetter = true) {
  if (current == null || baseline == null || !Number.isFinite(current) || !Number.isFinite(baseline) || baseline === 0) {
    return { delta: '--', tone: 'slate' as const }
  }
  const percent = ((current - baseline) / baseline) * 100
  if (Math.abs(percent) < 1) return { delta: '0%', tone: 'slate' as const }
  const better = higherIsBetter ? percent > 0 : percent < 0
  return {
    delta: `${percent > 0 ? '+' : ''}${percent.toFixed(0)}%`,
    tone: better ? 'emerald' as const : 'amber' as const,
  }
}

function sessionComparisonScopeLabel(benchmark: SessionBenchmark, labels: ReturnType<typeof getLabels>) {
  if (benchmark.scope === 'sameConfig') return labels.sameConfigHistory
  if (benchmark.scope === 'allHistory') return labels.allHistoryFallback
  return labels.noHistoryBaseline
}

function diagnosticTone(severity: DiagnosticFinding['severity']) {
  if (severity === 'critical') return 'red'
  if (severity === 'warning') return 'amber'
  if (severity === 'success') return 'emerald'
  return 'blue'
}

function diagnosticPanelClass(severity: DiagnosticFinding['severity']) {
  if (severity === 'critical') return 'border-red-300 bg-red-50 dark:border-red-500/30 dark:bg-red-500/10'
  if (severity === 'warning') return 'border-amber-300 bg-amber-50 dark:border-amber-500/30 dark:bg-amber-500/10'
  if (severity === 'success') return 'border-emerald-300 bg-emerald-50 dark:border-emerald-500/30 dark:bg-emerald-500/10'
  return 'border-blue-300 bg-blue-50 dark:border-blue-500/30 dark:bg-blue-500/10'
}

function diagnosticSeverityLabel(severity: DiagnosticFinding['severity'], labels: ReturnType<typeof getLabels>) {
  if (severity === 'critical') return labels.severityCritical
  if (severity === 'warning') return labels.severityWarning
  if (severity === 'success') return labels.severitySuccess
  return labels.severityInfo
}

function getLabels(zh: boolean) {
  return {
    title: zh ? '性能监控' : 'Performance Monitoring',
    description: zh ? '实例性能、请求吞吐与诊断分析' : 'Instance performance, request throughput, and diagnostics.',
    sqliteBacked: zh ? 'SQLite 遥测' : 'SQLite Telemetry',
    refresh: zh ? '刷新' : 'Refresh',
    telemetryError: zh ? '遥测数据读取异常' : 'Telemetry query failed',
    monitoringObject: zh ? '监控对象' : 'Monitoring Target',
    runningInstance: zh ? '运行中实例' : 'Running Instance',
    noRunning: zh ? '暂无运行中实例' : 'No running instance',
    sessionHistory: zh ? '会话历史' : 'Session History',
    noHistory: zh ? '暂无历史会话' : 'No Session History',
    noHistoryDesc: zh ? '启动实例并产生采样后会显示在这里。' : 'Sessions appear after an instance starts and emits samples.',
    noSelection: zh ? '未选择实例' : 'No instance selected',
    unknown: zh ? '未知' : 'Unknown',
    running: zh ? '运行中' : 'Running',
    finished: zh ? '已结束' : 'Finished',
    currentTps: zh ? '当前吞吐' : 'Current Throughput',
    fromLlamaMetrics: zh ? '来自 llama-server /metrics' : 'From llama-server /metrics',
    queuePressure: zh ? '请求压力' : 'Request Pressure',
    processingDeferred: zh ? '处理中 / 排队' : 'processing / queued',
    process: zh ? '进程' : 'Process',
    system: zh ? '系统' : 'System',
    memory: zh ? '内存' : 'Memory',
    vram: zh ? '显存' : 'VRAM',
    gpuUnavailable: zh ? '未检测到 GPU' : 'GPU unavailable',
    throughputTrend: zh ? '吞吐趋势' : 'Throughput Trend',
    noSamplesYet: zh ? '暂无足够遥测采样' : 'Not enough telemetry samples yet',
    ranges: {
      '1m': zh ? '1分钟' : '1m',
      '5m': zh ? '5分钟' : '5m',
      '15m': zh ? '15分钟' : '15m',
      '1h': zh ? '1小时' : '1h',
    },
    activeRequests: zh ? '运行中请求' : 'Active Requests',
    noActiveRequests: zh ? '当前没有正在处理的推理请求。' : 'No inference request is currently processing.',
    sessionFinishedNoActive: zh ? '该会话已结束，不再有运行中请求。' : 'This session has finished.',
    diagnosis: zh ? '智能诊断' : 'Smart Diagnostics',
    noDiagnostics: zh ? '当前会话暂无诊断结论。' : 'No diagnostics available yet.',
    moreDiagnostics: (count: number) => zh ? `另有 ${count} 条诊断已折叠` : `${count} more diagnostics collapsed`,
    severityCritical: zh ? '严重' : 'Critical',
    severityWarning: zh ? '提醒' : 'Warning',
    severityInfo: zh ? '信息' : 'Info',
    severitySuccess: zh ? '正常' : 'Healthy',
    historyBaseline: zh ? '历史基线' : 'Historical Baseline',
    sameConfigHistory: zh ? '优先对比同模型、同引擎和同后端的历史会话。' : 'Compares with previous sessions using the same model, engine, and backend.',
    allHistoryFallback: zh ? '暂无同配置历史，已回退到全部历史会话。' : 'No same-config history, falling back to all history.',
    noHistoryBaseline: zh ? '暂无可用历史基线。' : 'No historical baseline is available yet.',
    sessionDigest: zh ? '会话摘要' : 'Session Digest',
    requests: zh ? '请求' : 'Requests',
    avgGenerationSpeed: zh ? '平均生成速度' : 'Avg Generation Speed',
    avgTotalTime: zh ? '平均总耗时' : 'Avg Total Time',
    maxBusySlots: zh ? '忙碌 slot 峰值' : 'Max Busy Slots',
    avgTps: zh ? '平均吞吐' : 'Avg Throughput',
    peakVram: zh ? '峰值显存' : 'Peak VRAM',
    duration: zh ? '时长' : 'Duration',
  }
}
