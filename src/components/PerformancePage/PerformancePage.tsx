import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { invokeApp as invoke } from '../../lib/ipc'
import { Activity, AlertTriangle, BarChart3, Clock, Cpu, Gauge, HardDrive, Radio, RefreshCw, Server, Zap } from 'lucide-react'
import { useAppStore } from '../../store'
import { formatHostPort } from '../../utils/network'
import { useI18n } from '../../i18n'
import { getPerformanceLabels } from '../../i18n/pageLabels'
import type {
  DiagnosticFinding,
  InferenceRequestSummary,
  ModelWorkload,
  TelemetryOverview,
  TelemetrySampleSummary,
  TelemetrySessionDetail,
  TelemetrySessionAnalysis,
  TelemetrySessionSummary,
} from '../../store/types'
import { Badge, Button, EmptyPanel, PageFrame, PageHeader } from '../ui'
import { ActiveRequestRow, ComparisonTable, MonitorPanel, SessionCard, SignalMeter, StatusTile, TrendChart } from '../monitoring/MonitoringPrimitives'
import { formatDuration, formatMemory, formatMs, formatRate, formatTime } from '../monitoring/monitoringFormat'
import {
  buildRequestPressure,
  buildResourceSignals,
  buildTelemetryThroughputPoints,
  mergeThroughputPoints,
  monitoringFramePoints,
} from '../monitoring/monitoringViewModel'
import {
  buildPerformanceMode,
  buildVectorComparisonRows,
  buildVectorKpis,
  buildVectorTrendSeries,
  workloadLabel,
} from './vectorPerformance'

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
  const labels = useMemo(() => getPerformanceLabels(lang), [lang])
  const instances = useAppStore(state => state.instances)
  const monitoringFramesByInstance = useAppStore(state => state.monitoringFramesByInstance)
  const monitoringCurrentByInstance = useAppStore(state => state.monitoringCurrentByInstance)
  const runningTasksByInstance = useAppStore(state => state.runningTasksByInstance)
  const running = useMemo(() => instances.filter(instance => instance.status === 'running'), [instances])
  const [selectedInstanceId, setSelectedInstanceId] = useState('')
  const [selectedSessionId, setSelectedSessionId] = useState('')
  const [overview, setOverview] = useState<TelemetryOverview>(emptyOverview)
  const [sessions, setSessions] = useState<TelemetrySessionSummary[]>([])
  const [samples, setSamples] = useState<TelemetrySampleSummary[]>([])
  const [requests, setRequests] = useState<InferenceRequestSummary[]>([])
  const [analysis, setAnalysis] = useState<TelemetrySessionAnalysis | null>(null)
  const [diagnostics, setDiagnostics] = useState<DiagnosticFinding[]>([])
  const [loading, setLoading] = useState(false)
  const [telemetryError, setTelemetryError] = useState<string | null>(null)
  const [trendRange, setTrendRange] = useState<'1m' | '5m' | '15m' | '1h'>('5m')
  const [vectorTrendMetric, setVectorTrendMetric] = useState<'input' | 'items'>('input')
  const selectedSessionIdRef = useRef('')
  const telemetryInFlightRef = useRef(false)
  const detailInFlightRef = useRef(new Set<string>())

  const selectedSession = sessions.find(session => session.id === selectedSessionId)
  const selectedInstance = running.find(instance => instance.id === selectedInstanceId)
    || instances.find(instance => instance.id === selectedSession?.instance_id)
    || null
  const liveTargetInstance = selectedSession
    ? selectedSession.stopped_at
      ? null
      : running.find(instance => instance.id === selectedSession.instance_id) || null
    : running.find(instance => instance.id === selectedInstanceId) || null
  const activeTasks = liveTargetInstance ? runningTasksByInstance[liveTargetInstance.id] || [] : []
  const currentFrame = liveTargetInstance
    ? monitoringCurrentByInstance[liveTargetInstance.id] || null
    : null
  const liveFrames = liveTargetInstance
    ? (monitoringFramesByInstance[liveTargetInstance.id] || []).filter(frame => (
        !selectedSession || frame.sessionId === selectedSession.id
      ))
    : []

  const refreshTelemetry = useCallback(async (options: { silent?: boolean } = {}) => {
    if (telemetryInFlightRef.current) return
    telemetryInFlightRef.current = true
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
      telemetryInFlightRef.current = false
      if (!options.silent) setLoading(false)
    }
  }, [])

  const refreshSelectedSessionDetails = useCallback(async (sessionId: string) => {
    if (!sessionId || detailInFlightRef.current.has(sessionId)) return
    detailInFlightRef.current.add(sessionId)
    try {
      const detail = await invoke<TelemetrySessionDetail>('get_telemetry_session_detail', {
        sessionId,
        sampleLimit: 220,
        requestLimit: 18,
      })
      if (selectedSessionIdRef.current !== sessionId) return
      setSamples(detail.samples)
      setRequests(detail.requests)
      setAnalysis(detail.analysis)
      setDiagnostics(detail.diagnostics)
      setTelemetryError(null)
    } catch (error) {
      if (selectedSessionIdRef.current === sessionId) {
        setTelemetryError(error instanceof Error ? error.message : String(error))
      }
    } finally {
      detailInFlightRef.current.delete(sessionId)
    }
  }, [])

  useEffect(() => {
    void refreshTelemetry()
    const timer = window.setInterval(() => void refreshTelemetry({ silent: true }), TELEMETRY_OVERVIEW_REFRESH_MS)
    return () => window.clearInterval(timer)
  }, [refreshTelemetry])

  useEffect(() => {
    if (running.length === 0) {
      if (selectedInstanceId) setSelectedInstanceId('')
      return
    }
    if (running.length > 0 && (!selectedInstanceId || !running.some(instance => instance.id === selectedInstanceId))) {
      setSelectedInstanceId(running[0].id)
    }
  }, [running, selectedInstanceId])

  useEffect(() => {
    selectedSessionIdRef.current = selectedSessionId
    if (!selectedSessionId) {
      setSamples([])
      setRequests([])
      setAnalysis(null)
      setDiagnostics([])
      return
    }
    void refreshSelectedSessionDetails(selectedSessionId)
    const timer = window.setInterval(
      () => void refreshSelectedSessionDetails(selectedSessionId),
      TELEMETRY_DETAIL_REFRESH_MS,
    )
    return () => window.clearInterval(timer)
  }, [selectedSessionId, refreshSelectedSessionDetails])

  useEffect(() => {
    if (selectedSession) setSelectedInstanceId(selectedSession.instance_id)
  }, [selectedSession])

  const historicalAnchor = selectedSession?.stopped_at || Date.now()
  const selectedOverviewSamples = overview.latest_samples.filter(sample => (
    sample.instance_id === (selectedSession?.instance_id || selectedInstance?.id)
  ))
  const trendSamples = useMemo(
    () => filterSamplesByRange(
      samples.length > 0 ? samples : selectedOverviewSamples,
      trendRange,
      historicalAnchor,
    ),
    [historicalAnchor, samples, selectedOverviewSamples, trendRange],
  )
  const latestSample = trendSamples[trendSamples.length - 1]
    || selectedOverviewSamples[selectedOverviewSamples.length - 1]
    || null
  const resourceSignals = useMemo(
    () => buildResourceSignals({
      system: currentFrame?.system || null,
      latest: latestSample,
      samples: trendSamples,
      labels,
    }),
    [currentFrame?.system, latestSample, trendSamples, labels],
  )
  const fallbackWorkload: ModelWorkload = selectedInstance?.config.reranking
    ? 'reranker'
    : selectedInstance?.config.embedding ? 'embedding' : 'inference'
  const performanceMode = buildPerformanceMode(
    selectedSession ?? { workload: fallbackWorkload },
    analysis,
    lang,
  )
  const selectedRunning = selectedSession
    ? selectedSession.stopped_at == null
    : selectedInstance?.status === 'running'
  const currentThroughput = currentFrame?.throughput ?? 0
  const trendEnd = selectedSession?.stopped_at || Date.now()
  const trendStart = trendEnd - trendRangeToMs(trendRange)
  const inferenceTrendPoints = useMemo(
    () => {
      const live = monitoringFramePoints(liveFrames, 'inference')
        .filter(point => point.ts >= trendStart && point.value != null)
        .map(point => ({ ts: point.ts, value: point.value as number }))
      return mergeThroughputPoints(
        buildTelemetryThroughputPoints(trendSamples),
        live,
        trendStart,
        240,
      )
    },
    [liveFrames, trendSamples, trendStart],
  )
  const currentThroughputDetail = currentFrame?.source === 'task'
    ? labels.fromLiveTasks
    : currentFrame?.source === 'llama'
        ? labels.fromLlamaMetrics
        : labels.idleThroughput
  const pressure = buildRequestPressure(
    currentFrame?.activeRequests ?? latestSample?.requests_processing,
    currentFrame?.queuedRequests ?? latestSample?.requests_deferred,
    currentFrame?.slotCapacity,
  )
  const sessionBenchmark = useMemo(() => buildSessionBenchmark(selectedSession, sessions), [selectedSession, sessions])
  const comparisonRows = performanceMode.kind === 'vector' && performanceMode.analysis && analysis?.vector_baseline
    ? buildVectorComparisonRows(performanceMode.analysis, analysis.vector_baseline, lang).map(row => ({
        metric: row.label,
        current: row.current,
        baseline: row.baseline,
        delta: row.favorable == null ? '--' : row.favorable ? labels.better : labels.regressed,
        tone: row.favorable == null ? 'slate' as const : row.favorable ? 'emerald' as const : 'amber' as const,
      }))
    : performanceMode.kind === 'inference'
      ? buildComparisonRows(selectedSession, sessionBenchmark, labels)
      : []
  const vectorKpis = performanceMode.kind === 'vector' && performanceMode.analysis
    ? buildVectorKpis(performanceMode.analysis, lang)
    : performanceMode.kind === 'vector' && currentFrame
      ? [
          {
            key: 'input' as const,
            label: labels.inputThroughput,
            value: formatRate(currentFrame.inputTokensPerSecond, 'tok/s'),
            available: currentFrame.inputTokensPerSecond != null,
          },
          {
            key: 'items' as const,
            label: performanceMode.itemName === 'document' ? labels.documentThroughput : labels.vectorThroughput,
            value: formatRate(currentFrame.itemsPerSecond, labels.itemsPerSecondShort),
            available: currentFrame.itemsPerSecond != null,
          },
          {
            key: 'p95' as const,
            label: labels.taskP95,
            value: formatMs(currentFrame.averageLatencyMs),
            available: currentFrame.averageLatencyMs != null,
          },
        ]
      : []
  const displayedVectorKpis = vectorKpis.map(kpi => {
    if (!selectedRunning || !currentFrame) return kpi
    if (kpi.key === 'input') return { ...kpi, value: formatRate(currentFrame.inputTokensPerSecond, 'tok/s'), available: currentFrame.inputTokensPerSecond != null }
    if (kpi.key === 'items') return { ...kpi, value: formatRate(currentFrame.itemsPerSecond, labels.itemsPerSecondShort), available: currentFrame.itemsPerSecond != null }
    return { ...kpi, value: formatMs(currentFrame.averageLatencyMs), available: currentFrame.averageLatencyMs != null }
  })
  const vectorHistoricalTrend = performanceMode.kind === 'vector' && performanceMode.analysis
    ? buildVectorTrendSeries(performanceMode.analysis, vectorTrendMetric)
    : []
  const vectorLiveTrend = liveFrames
    .filter(frame => frame.workload !== 'inference' && frame.ts >= trendStart)
    .map(frame => ({
      ts: frame.ts,
      value: vectorTrendMetric === 'input' ? frame.inputTokensPerSecond : frame.itemsPerSecond,
    }))
  const vectorTrend = selectedRunning && vectorLiveTrend.length > 1
    ? vectorLiveTrend
    : vectorHistoricalTrend.map(point => ({ ts: point.timestamp, value: point.value }))
  const inferenceOnlyDiagnosticIds = new Set([
    'throughput_regression',
    'no_request_records',
    'prompt_eval_bottleneck',
    'long_request_latency',
    'slot_cache_observed',
    'large_context_window',
  ])
  const workloadDiagnostics = performanceMode.kind === 'vector'
    ? diagnostics.filter(finding => !inferenceOnlyDiagnosticIds.has(finding.id))
    : diagnostics
  const visibleDiagnostics = workloadDiagnostics.slice(0, 3)
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

        <div className="grid min-h-[calc(100vh-178px)] gap-4 xl:grid-cols-[260px_minmax(0,1fr)] 2xl:grid-cols-[310px_minmax(0,1fr)_360px]">
          <aside className="min-w-0 space-y-4">
            <MonitorPanel title={labels.monitoringObject} icon={<Server className="h-5 w-5" />} action={<Badge tone="emerald">{running.length}</Badge>}>
              <div className="space-y-4">
                <div data-guide="perf-select">
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
                        meta={`${session.workload === 'inference' ? formatRate(session.avg_tokens_per_sec) : workloadLabel(session.workload, lang)} · ${formatMemory(session.peak_vram_mb)} · ${formatTime(session.started_at)}`}
                        selected={session.id === selectedSessionId}
                        running={!session.stopped_at}
                        statusLabel={!session.stopped_at ? labels.running : labels.finished}
                        workload={workloadLabel(session.workload, lang)}
                        workloadTone={session.workload === 'embedding' ? 'violet' : session.workload === 'reranker' ? 'blue' : 'slate'}
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
              <div className="flex min-w-0 flex-wrap items-center gap-3">
                <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg border border-blue-300/40 bg-blue-500/10 text-blue-600 dark:text-blue-300">
                  <Server className="h-5 w-5" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex min-w-0 flex-wrap items-center gap-2">
                    <h2 className="truncate text-xl font-semibold text-slate-950 dark:text-white" title={selectedSession?.instance_name || selectedInstance?.name || labels.noSelection}>
                      {selectedSession?.instance_name || selectedInstance?.name || labels.noSelection}
                    </h2>
                    <Badge tone={selectedRunning ? 'emerald' : 'slate'}>
                      {selectedRunning ? labels.running : labels.finished}
                    </Badge>
                    <span data-workload-badge>
                      <Badge tone={performanceMode.workload === 'embedding' ? 'violet' : performanceMode.workload === 'reranker' ? 'blue' : 'slate'}>
                        {workloadLabel(performanceMode.workload, lang)}
                      </Badge>
                    </span>
                  </div>
                  <div className="mt-1 truncate font-mono text-xs text-blue-600 dark:text-blue-300">
                    {selectedSession?.model_name || (selectedInstance ? formatHostPort(selectedInstance.config.host, selectedInstance.config.port) : '--')}
                  </div>
                </div>
              </div>
              <div className={`mt-4 grid gap-3 ${performanceMode.kind === 'vector' ? 'sm:grid-cols-2 2xl:grid-cols-4' : 'lg:grid-cols-2'}`}>
                {performanceMode.kind === 'vector' ? displayedVectorKpis.map(kpi => (
                  <StatusTile
                    key={kpi.key}
                    label={kpi.label}
                    value={kpi.value}
                    detail={kpi.available ? labels.fromTaskLog : performanceMode.source.summary}
                    icon={kpi.key === 'p95' ? <Clock className="h-5 w-5" /> : kpi.key === 'items' ? <Activity className="h-5 w-5" /> : <Gauge className="h-5 w-5" />}
                    tone={kpi.key === 'p95' ? 'amber' : kpi.key === 'items' ? 'violet' : 'blue'}
                    className="py-3"
                  />
                )) : (
                  <StatusTile label={labels.currentTps} value={formatRate(currentThroughput)} detail={currentThroughputDetail} icon={<Gauge className="h-5 w-5" />} tone="blue" className="py-3" />
                )}
                <StatusTile
                  label={labels.queuePressure}
                  value={`${pressure.percent}%`}
                  detail={`${pressure.active}${pressure.capacity ? ` / ${pressure.capacity}` : ''} · ${pressure.queued} ${labels.processingDeferred}`}
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
                <div data-vector-trend-control={performanceMode.kind === 'vector' || undefined} className="flex rounded-lg border border-slate-200 bg-slate-100 p-1 dark:border-slate-800 dark:bg-slate-950">
                  {performanceMode.kind === 'vector' ? (['input', 'items'] as const).map(metric => (
                    <button
                      key={metric}
                      type="button"
                      onClick={() => setVectorTrendMetric(metric)}
                      className={`h-7 min-w-[84px] rounded-md px-2 text-xs font-medium transition ${vectorTrendMetric === metric ? 'bg-blue-600 text-white' : 'text-slate-600 hover:text-slate-900 dark:text-slate-400 dark:hover:text-slate-100'}`}
                    >
                      {metric === 'input' ? labels.inputThroughput : performanceMode.itemName === 'document' ? labels.documentThroughput : labels.vectorThroughput}
                    </button>
                  )) : (['1m', '5m', '15m', '1h'] as const).map(range => (
                    <button
                      key={range}
                      type="button"
                      onClick={() => setTrendRange(range)}
                      className={`h-7 rounded-md px-2 text-xs font-medium transition ${trendRange === range ? 'bg-blue-600 text-white' : 'text-slate-600 hover:text-slate-900 dark:text-slate-400 dark:hover:text-slate-100'}`}
                    >
                      {labels.ranges[range]}
                    </button>
                  ))}
                </div>
              }
            >
              <TrendChart
                points={performanceMode.kind === 'vector' ? vectorTrend : inferenceTrendPoints}
                rangeStart={selectedRunning ? trendStart : undefined}
                rangeEnd={selectedRunning ? trendEnd : undefined}
                emptyText={performanceMode.kind === 'vector' ? performanceMode.source.summary : labels.noSamplesYet}
                tone={performanceMode.kind === 'vector' && vectorTrendMetric === 'items' ? 'violet' : 'blue'}
                unit={performanceMode.kind === 'vector' && vectorTrendMetric === 'items'
                  ? performanceMode.itemName === 'document' ? labels.documentsPerSecondShort : labels.itemsPerSecondShort
                  : performanceMode.kind === 'vector' ? 'input tok/s' : 'tok/s'}
              />
              {performanceMode.kind === 'vector' ? (
                <div data-vector-source-state={performanceMode.source.kind} className="mt-3 flex min-w-0 flex-wrap items-center justify-between gap-2 border-t border-slate-200 pt-3 text-xs text-slate-600 dark:border-slate-800 dark:text-slate-300">
                  <span className="min-w-0 flex-1">{performanceMode.source.summary}</span>
                  <div className="flex flex-wrap gap-2">
                    <Badge tone={performanceMode.analysis?.logAvailable ? 'emerald' : 'slate'}>{performanceMode.source.log}</Badge>
                    <Badge tone={performanceMode.analysis?.proxyAvailable ? 'blue' : 'slate'}>{performanceMode.source.proxy}</Badge>
                  </div>
                </div>
              ) : null}
            </MonitorPanel>

            <MonitorPanel title={labels.activeRequests} icon={<Radio className="h-5 w-5" />} action={<Badge tone={activeTasks.length > 0 ? 'emerald' : 'slate'}>{activeTasks.length}</Badge>}>
              {activeTasks.length === 0 ? (
                <div className="rounded-lg border border-dashed border-slate-300 px-4 py-8 text-center text-sm text-slate-500 dark:border-slate-700 dark:text-slate-400">
                  {selectedSession?.stopped_at ? labels.sessionFinishedNoActive : labels.noActiveRequests}
                </div>
              ) : (
                <div className="grid gap-2">
                  {activeTasks.map(task => (
                    <ActiveRequestRow
                      key={task.task_id}
                      task={task}
                      instanceName={selectedSession?.instance_name || selectedInstance?.name}
                      workload={performanceMode.workload}
                      workloadText={workloadLabel(performanceMode.workload, lang)}
                    />
                  ))}
                </div>
              )}
            </MonitorPanel>
          </main>

          <aside className="min-w-0 space-y-4 xl:col-span-2 2xl:col-span-1">
            <MonitorPanel title={labels.diagnosis} icon={<AlertTriangle className="h-5 w-5" />} action={<Badge tone={workloadDiagnostics.length > 0 ? 'amber' : 'emerald'}>{workloadDiagnostics.length}</Badge>}>
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
                  {workloadDiagnostics.length > visibleDiagnostics.length ? (
                    <div className="text-center text-xs text-slate-500">{labels.moreDiagnostics(workloadDiagnostics.length - visibleDiagnostics.length)}</div>
                  ) : null}
                </div>
              )}
            </MonitorPanel>

            <MonitorPanel title={labels.historyBaseline} icon={<Clock className="h-5 w-5" />}>
              <div className="mb-3 text-xs leading-5 text-slate-500 dark:text-slate-400">
                {performanceMode.kind === 'vector'
                  ? analysis?.vector_baseline && analysis.vector_baseline.sessionCount > 0
                    ? labels.vectorHistory(analysis.vector_baseline.sessionCount)
                    : labels.noVectorHistory
                  : sessionComparisonScopeLabel(sessionBenchmark, labels)}
              </div>
              <ComparisonTable rows={comparisonRows} />
            </MonitorPanel>

            <MonitorPanel title={labels.sessionDigest} icon={<Activity className="h-5 w-5" />}>
              {performanceMode.kind === 'vector' ? (
                <div className="grid grid-cols-2 gap-2">
                  <MiniStat label={performanceMode.itemName === 'document' ? labels.completedDocuments : labels.completedVectors} value={formatCount(performanceMode.analysis?.completedItems)} />
                  <MiniStat label={labels.avgInputSpeed} value={formatVectorRate(performanceMode.analysis?.averageInputTokensPerSecond, 'tok/s')} />
                  <MiniStat label={labels.taskP50} value={formatMs(performanceMode.analysis?.taskDurationP50Ms)} />
                  <MiniStat label={labels.taskP95} value={formatMs(performanceMode.analysis?.taskDurationP95Ms)} />
                  {performanceMode.analysis?.proxyAvailable ? (
                    <>
                      <MiniStat label={labels.proxyRequests} value={formatCount(performanceMode.analysis.proxyRequestCount)} />
                      <MiniStat label={labels.proxyP50} value={formatMs(performanceMode.analysis.proxyDurationP50Ms)} />
                      <MiniStat label={labels.proxyP95} value={formatMs(performanceMode.analysis.proxyDurationP95Ms)} />
                      <MiniStat label={labels.failureRate} value={formatRatio(performanceMode.analysis.proxyFailureRate)} />
                    </>
                  ) : null}
                </div>
              ) : (
                <div className="grid grid-cols-2 gap-2">
                  <MiniStat label={labels.requests} value={analysis?.request_count ?? requests.length} />
                  <MiniStat label={labels.avgGenerationSpeed} value={formatRate(analysis?.avg_generation_tps)} />
                  <MiniStat label={labels.avgTotalTime} value={formatMs(analysis?.avg_total_time_ms)} />
                  <MiniStat label={labels.maxBusySlots} value={analysis?.max_busy_slots ?? 0} />
                </div>
              )}
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

function formatCount(value?: number | null): string {
  return value == null || !Number.isFinite(value) ? '--' : Math.max(0, value).toLocaleString()
}

function formatVectorRate(value: number | null | undefined, suffix: string): string {
  return value == null || !Number.isFinite(value) ? '--' : `${value.toFixed(1)} ${suffix}`
}

function formatRatio(value?: number | null): string {
  return value == null || !Number.isFinite(value) ? '--' : `${(value * 100).toFixed(1)}%`
}

function filterSamplesByRange(
  samples: TelemetrySampleSummary[],
  range: '1m' | '5m' | '15m' | '1h',
  anchor: number,
) {
  if (samples.length === 0) return []
  const end = Math.max(...samples.map(sample => sample.ts || 0), anchor)
  const rangeMs = trendRangeToMs(range)
  const filtered = samples.filter(sample => end - sample.ts <= rangeMs)
  return filtered.length >= 2 ? filtered : samples.slice(-80)
}

function trendRangeToMs(range: '1m' | '5m' | '15m' | '1h') {
  if (range === '1m') return 60000
  if (range === '5m') return 300000
  if (range === '15m') return 900000
  return 3600000
}

function buildSessionBenchmark(selectedSession: TelemetrySessionSummary | undefined, sessions: TelemetrySessionSummary[]): SessionBenchmark {
  if (!selectedSession) return emptySessionBenchmark()
  const history = sessions.filter(session =>
    session.workload === selectedSession.workload &&
    session.id !== selectedSession.id &&
    session.stopped_at &&
    session.sample_count > 0
  )
  const sameConfig = history.filter(session =>
    session.model_name === selectedSession.model_name &&
    session.engine_id === selectedSession.engine_id &&
    session.backend === selectedSession.backend &&
    session.workload === selectedSession.workload
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

function buildComparisonRows(selectedSession: TelemetrySessionSummary | undefined, benchmark: SessionBenchmark, labels: ReturnType<typeof getPerformanceLabels>) {
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

function sessionComparisonScopeLabel(benchmark: SessionBenchmark, labels: ReturnType<typeof getPerformanceLabels>) {
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

function diagnosticSeverityLabel(severity: DiagnosticFinding['severity'], labels: ReturnType<typeof getPerformanceLabels>) {
  if (severity === 'critical') return labels.severityCritical
  if (severity === 'warning') return labels.severityWarning
  if (severity === 'success') return labels.severitySuccess
  return labels.severityInfo
}
