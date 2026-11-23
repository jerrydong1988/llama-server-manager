import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { invokeApp as invoke } from '../../lib/ipc'
import { Activity, AlertTriangle, BarChart3, Clock, Cpu, FastForward, Gauge, HardDrive, Radio, Maximize2, RefreshCw, Server, Zap } from 'lucide-react'
import { useAppStore } from '../../store'
import { formatHostPort } from '../../utils/network'
import { pathsEqual } from '../../utils/path'
import { useI18n } from '../../i18n'
import { getPerformanceLabels, getSpeculativeDecodingLabels } from '../../i18n/pageLabels'
import { getInferenceLabels } from '../../i18n/inferenceMonitoring'
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
import { Badge, Button, SelectInput } from '../ui'
import { ActiveRequestRow, ComparisonTable, MonitorPanel, SignalMeter, StatusTile, TrendChart } from '../monitoring/MonitoringPrimitives'
import { formatDuration, formatMemory, formatMs, formatRate, formatTime } from '../monitoring/monitoringFormat'
import { DualThroughput, DualThroughputTrends, InferenceTasks } from '../monitoring/InferencePanels'
import { SessionRecords } from '../monitoring/MonitorRecords'
import { LiveOverview } from './LiveOverview'
import { getMonitorWorkspaceCopy } from '../../i18n/monitorWorkspace'
import { currentHost, numberText } from '../monitoring/MonitorReadings'
import '../monitoring/workspace.css'
import { inferenceTrend } from '../monitoring/inferenceThroughput'
import {
  buildRequestPressure,
  formatRequestPressureDetail,
  buildResourceSignals,
} from '../monitoring/monitoringViewModel'
import {
  buildSpeculativeDecodingSummary,
  isSpeculativeDecodingConfigured,
} from '../monitoring/speculativeDecoding'
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
  dropped_writes: 0,
  latest_samples: [],
}

const TELEMETRY_OVERVIEW_REFRESH_MS = 10000
const TELEMETRY_DETAIL_REFRESH_MS = 5000

export default function PerformancePage() {
  const { lang } = useI18n()
  const copy = getMonitorWorkspaceCopy(lang)
  const [tab, setTab] = useState<'live' | 'history' | 'diagnostics'>('live')
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false)
  const setActiveTab = useAppStore(state => state.setActiveTab)
  const inferenceLabels = getInferenceLabels(lang)
  const labels = useMemo(() => getPerformanceLabels(lang), [lang])
  const speculativeLabels = useMemo(() => getSpeculativeDecodingLabels(lang), [lang])
  const instances = useAppStore(state => state.instances)
  const sysMetrics = useAppStore(state => state.sysMetrics)
  const systemMetricsHistory = useAppStore(state => state.systemMetricsHistory)
  const running = useMemo(() => instances.filter(instance => instance.status === 'running'), [instances])
  const selectedInstanceId = useAppStore(state => state.monitoringInstanceId)
  const setSelectedInstanceId = useAppStore(state => state.setMonitoringInstanceId)
  const [selectedSessionId, setSelectedSessionId] = useState('')
  const [overview, setOverview] = useState<TelemetryOverview>(emptyOverview)
  const [sessions, setSessions] = useState<TelemetrySessionSummary[]>([])
  const [loadedSamples, setSamples] = useState<TelemetrySampleSummary[]>([])
  const [loadedRequests, setRequests] = useState<InferenceRequestSummary[]>([])
  const [prefillWindow, setPrefillWindow] = useState<TelemetrySessionDetail['request_prefill']>()
  const [vectorRecords, setVectorRecords] = useState<TelemetrySessionDetail['vector_activity']>([])
  const [slotUsage, setSlotUsage] = useState<TelemetrySessionDetail['context_usage']>(null)
  const [loadedAnalysis, setAnalysis] = useState<TelemetrySessionAnalysis | null>(null)
  const [loadedDiagnostics, setDiagnostics] = useState<DiagnosticFinding[]>([])
  const [detailSessionId, setDetailSessionId] = useState('')
  const [loading, setLoading] = useState(false)
  const [telemetryError, setTelemetryError] = useState<string | null>(null)
  const [trendRange, setTrendRange] = useState<'1m' | '5m' | '15m' | '1h'>('5m')
  const [vectorTrendMetric, setVectorTrendMetric] = useState<'input' | 'items'>('input')
  const selectedSessionIdRef = useRef('')
  const telemetryInFlightRef = useRef(false)
  const detailInFlightRef = useRef(new Set<string>())
  const detailReady = Boolean(selectedSessionId) && detailSessionId === selectedSessionId
  const samples = useMemo(
    () => detailReady ? loadedSamples : [],
    [detailReady, loadedSamples],
  )
  const requests = detailReady ? loadedRequests : []
  const analysis = detailReady ? loadedAnalysis : null
  const diagnostics = detailReady ? loadedDiagnostics : []

  const selectedSession = sessions.find(session => session.id === selectedSessionId)
  const selectedInstance = selectedSession
    ? instances.find(instance => instance.id === selectedSession.instance_id) || null
    : instances.find(instance => instance.id === selectedInstanceId) || null
  const liveTargetInstance = selectedSession
    ? selectedSession.stopped_at
      ? null
      : running.find(instance => instance.id === selectedSession.instance_id) || null
    : running.find(instance => instance.id === selectedInstanceId) || null
  const liveTargetId = liveTargetInstance?.id
  const activeTasks = useAppStore(state => liveTargetId ? state.runningTasksByInstance[liveTargetId] : undefined) || []
  const lastCompletedTask = useAppStore(state => liveTargetId ? state.lastCompletedTaskByInstance[liveTargetId] : undefined) || null
  const currentFrame = useAppStore(state => liveTargetId ? state.monitoringCurrentByInstance[liveTargetId] : undefined) || null
  const selectedInstanceFrames = useAppStore(state => liveTargetId ? state.monitoringFramesByInstance[liveTargetId] : undefined)
  const liveFrames = useMemo(
    () => liveTargetInstance
      ? (selectedInstanceFrames || []).filter(frame => (
          !selectedSession || frame.sessionId === selectedSession.id
        ))
      : [],
    [liveTargetInstance, selectedInstanceFrames, selectedSession],
  )

  const refreshTelemetry = useCallback(async (options: { silent?: boolean } = {}) => {
    if (telemetryInFlightRef.current) return
    telemetryInFlightRef.current = true
    if (!options.silent) setLoading(true)
    try {
      const [nextOverview, nextSessions] = await Promise.all([
        invoke<TelemetryOverview>('get_telemetry_overview'),
        invoke<TelemetrySessionSummary[]>('list_telemetry_sessions', { limit: 200 }),
      ])
      setOverview(nextOverview)
      setSessions(nextSessions)
      const targetId = useAppStore.getState().monitoringInstanceId
      const matching = targetId ? nextSessions.filter(session => session.instance_id === targetId) : nextSessions
      if (!matching.some(session => session.id === selectedSessionIdRef.current)) {
        const next = (matching.find(session => !session.stopped_at) ?? matching[0])?.id ?? ''
        selectedSessionIdRef.current = next
        setSelectedSessionId(next)
      }
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
        sampleLimit: 1000,
        requestLimit: 18,
      })
      if (selectedSessionIdRef.current !== sessionId) return
      setSamples(detail.samples)
      setRequests(detail.requests)
      setPrefillWindow(detail.request_prefill)
      setVectorRecords(detail.vector_activity ?? [])
      setSlotUsage(detail.context_usage)
      setAnalysis(detail.analysis)
      setDiagnostics(detail.diagnostics)
      setDetailSessionId(sessionId)
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
    if (instances.length === 0) {
      if (selectedInstanceId && !sessions.some(s => s.instance_id === selectedInstanceId)) setSelectedInstanceId('')
      return
    }
    if (!selectedInstanceId || (!instances.some(instance => instance.id === selectedInstanceId) && !sessions.some(session => session.instance_id === selectedInstanceId))) {
      setSelectedInstanceId(running[0]?.id || instances[0].id)
    }
  }, [instances, running, selectedInstanceId, sessions, setSelectedInstanceId])

  useEffect(() => {
    selectedSessionIdRef.current = selectedSessionId
    setDetailSessionId('')
    setSamples([])
    setRequests([])
    setPrefillWindow(undefined)
    setVectorRecords([])
    setSlotUsage(null)
    setAnalysis(null)
    setDiagnostics([])
    if (!selectedSessionId) {
      return
    }
    void refreshSelectedSessionDetails(selectedSessionId)
    if (selectedSession?.stopped_at) return
    const timer = window.setInterval(
      () => void refreshSelectedSessionDetails(selectedSessionId),
      TELEMETRY_DETAIL_REFRESH_MS,
    )
    return () => window.clearInterval(timer)
  }, [selectedSessionId, selectedSession?.stopped_at, refreshSelectedSessionDetails])

  useEffect(() => {
    if (selectedSession) setSelectedInstanceId(selectedSession.instance_id)
  }, [selectedSession, setSelectedInstanceId])

  const historicalAnchor = selectedSession?.stopped_at || Date.now()
  const selectedOverviewSamples = useMemo(
    () => overview.latest_samples
      .filter(sample => selectedSession
        ? sample.session_id === selectedSession.id
        : sample.instance_id === selectedInstance?.id)
      .sort((left, right) => left.ts - right.ts),
    [overview.latest_samples, selectedInstance?.id, selectedSession],
  )
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
      system: sysMetrics,
      history: systemMetricsHistory,
      labels,
    }),
    [sysMetrics, systemMetricsHistory, labels],
  )
  const fallbackWorkload: ModelWorkload = selectedInstance?.config.reranking
    ? 'reranker'
    : selectedInstance?.config.embedding ? 'embedding' : 'inference'
  const performanceMode = buildPerformanceMode(
    selectedSession ?? { workload: fallbackWorkload },
    analysis,
    lang,
  )
  const speculativeSummary = buildSpeculativeDecodingSummary({
    configured: performanceMode.kind === 'inference'
      && isSpeculativeDecodingConfigured(selectedInstance?.config),
    analysis: performanceMode.kind === 'inference' ? analysis?.speculative_analysis : null,
    requests: performanceMode.kind === 'inference' ? requests : [],
    activeTasks: performanceMode.kind === 'inference' ? activeTasks : [],
    lastCompletedTasks: performanceMode.kind === 'inference' && lastCompletedTask
      ? [{
          ...lastCompletedTask,
          session_id: selectedSession?.id || currentFrame?.sessionId || null,
        }]
      : [],
  })
  const selectedRunning = selectedSession
    ? selectedSession.stopped_at == null
    : selectedInstance?.status === 'running'
  const trendEnd = selectedSession?.stopped_at || Date.now()
  const trendStart = trendEnd - trendRangeToMs(trendRange)
  const prefillTrend = inferenceTrend(liveFrames, trendSamples, 'prefill', trendStart)
  const decodeTrend = inferenceTrend(liveFrames, trendSamples, 'decode', trendStart)
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
  const chooseInstance = (id: string) => {
    setSelectedInstanceId(id)
    const target = sessions.find(s => s.instance_id === id && !s.stopped_at) ?? sessions.find(s => s.instance_id === id)
    selectedSessionIdRef.current = target?.id || ''
    setSelectedSessionId(selectedSessionIdRef.current)
  }
  const currentSystem = currentHost(sysMetrics, systemMetricsHistory, Date.now())
  const names = new Map(instances.map(i => [i.id, i.name]))
  for (const session of sessions) if (!names.has(session.instance_id)) names.set(session.instance_id, session.instance_name)
  return (
    <section className="monitor-workspace" data-monitor-view={tab}>
      <div className="monitor-heading"><h2>{copy.monitor}</h2><div className="monitor-actions">
        <label data-guide="perf-select">{labels.monitoringInstance}
          <SelectInput value={selectedInstanceId} onChange={event => chooseInstance(event.target.value)}>
            {!names.size && <option value="">{labels.noInstances}</option>}
            {[...names].map(([id,name]) => <option key={id} value={id}>{name}</option>)}
          </SelectInput>
        </label>
        <Button size="sm" icon={<Maximize2 size={14}/>} onClick={() => setActiveTab('bigscreen')}>{copy.wall}</Button>
        <Button size="sm" aria-label={labels.refresh} title={labels.refresh} disabled={loading} onClick={() => void refreshTelemetry()} icon={<RefreshCw size={14}/>}/>
      </div></div>
      <nav className="monitor-tabs" aria-label={copy.monitor}>
        {(['live','history','diagnostics'] as const).map(id => <button type="button" key={id} aria-current={tab === id ? 'page' : undefined} onClick={() => {
          if (id === 'live') chooseInstance(selectedInstanceId)
          setTab(id)
        }}>{copy[id]}</button>)}
      </nav>
        {telemetryError ? (
          <div className="rounded-lg border border-amber-300 bg-amber-50 p-4 text-sm text-amber-900 dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-100">
            <div className="font-semibold">{labels.telemetryError}</div>
            <div className="mt-1 truncate text-xs opacity-90" title={telemetryError}>{telemetryError}</div>
          </div>
        ) : null}
        {overview.dropped_writes > 0 ? (
          <div className="rounded-lg border border-amber-300 bg-amber-50 p-4 text-sm text-amber-900 dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-100">
            <div className="font-semibold">
              {labels.telemetryDropped}
            </div>
            <div className="mt-1 text-xs opacity-90">
              {labels.telemetryDroppedDetail(overview.dropped_writes)}
            </div>
          </div>
        ) : null}

      {tab === 'live' && <LiveOverview key={selectedInstanceId} lang={lang} instance={selectedInstance} workload={performanceMode.workload}
        frame={liveTargetId ? currentFrame : null} frames={liveFrames} samples={samples} system={sysMetrics} history={systemMetricsHistory}
        requests={requests} prefillWindow={detailReady ? prefillWindow : undefined} batches={detailReady ? vectorRecords ?? [] : []} context={detailReady ? slotUsage : null}/>}
      {tab === 'history' && <SessionRecords copy={copy} sessions={sessions.filter(s => s.instance_id === selectedInstanceId)}
        selected={selectedSession} analysis={analysis} choose={s => { selectedSessionIdRef.current = s.id; setSelectedSessionId(s.id) }}>
        {selectedSession && <div className="console-surface monitor-detail"><h3>{formatTime(selectedSession.started_at)} · {selectedSession.model_name || selectedSession.instance_name}</h3>
          <p className="monitor-note">{performanceMode.kind === 'vector' ? (performanceMode.itemName === 'document' ? labels.completedDocuments : labels.completedVectors) : labels.requests}　{numberText(performanceMode.kind === 'vector' ? analysis?.vector_analysis?.completedItems : analysis?.request_count, 0)}　·　{copy.maxMemory}　{formatMemory(selectedSession.peak_vram_mb)}</p>
          <ComparisonTable rows={comparisonRows}/></div>}
      </SessionRecords>}
      {tab === 'diagnostics' && <>
        <section className="console-surface monitor-detail"><header><h3>{copy.hardware}</h3><span className="monitor-note">{sysMetrics?.gpu_name}</span></header>
          <dl className="monitor-readouts">
            {[[copy.gpuLoad, currentSystem?.gpu_percent, '%'],[copy.temperature, currentSystem?.gpu_temperature_c, '°C'],
              [copy.power, currentSystem?.gpu_power_w, 'W'],[copy.board, currentSystem?.gpu_board_power_w, 'W'],
              [copy.cpu, currentSystem?.system_cpu_percent, '%']].map(([label,value,unit]) =>
              <div key={String(label)}><dt>{label}</dt><dd>{typeof value === 'number' ? numberText(value) + ' ' + unit : '—'}</dd></div>)}
            <div><dt>{copy.ram}</dt><dd>{formatMemory(currentSystem?.system_memory_used_mb)} / {formatMemory(currentSystem?.system_memory_total_mb)}</dd></div>
          </dl>
          <p className="monitor-note">{currentSystem?.gpu_vendor === 'NVIDIA' ? copy.nvmlMax + ' · ' + copy.nvmlPower
            : currentSystem?.gpu_vendor === 'AMD' ? 'AMD ADLX · GPUTemperature / GPUPower / GPUTotalBoardPower' : copy.missing}</p>
        </section>
        <details className="monitor-disclosure" data-monitor-diagnostics open={diagnosticsOpen} onToggle={event => setDiagnosticsOpen(event.currentTarget.open)}><summary>{copy.inferenceDetails}</summary>
          {diagnosticsOpen && <div className="grid min-w-0 gap-3 pt-3 xl:grid-cols-[minmax(0,1.8fr)_minmax(260px,1fr)]">
          <div className="min-w-0 space-y-4">
            <section className="rounded-lg border border-ui-line bg-ui-surface p-4 shadow-sm">
              <div className="flex min-w-0 flex-wrap items-center gap-3">
                <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg border border-blue-300/40 bg-blue-500/10 text-blue-600 dark:text-blue-300">
                  <Server className="h-5 w-5" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex min-w-0 flex-wrap items-center gap-2">
                    <h2 className="truncate text-xl font-semibold text-ui-ink" title={selectedSession?.instance_name || selectedInstance?.name || labels.noSelection}>
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
                  <div className="lg:col-span-2"><DualThroughput frames={[selectedRunning ? currentFrame ?? undefined : undefined]} lang={lang} /></div>
                )}
                <StatusTile
                  label={labels.queuePressure}
                  value={pressure.percent == null ? '--' : `${pressure.percent}%`}
                  detail={formatRequestPressureDetail(pressure, labels)}
                  icon={<Zap className="h-5 w-5" />}
                  tone={pressure.level === 'high' ? 'amber' : pressure.level === 'medium' ? 'cyan' : pressure.level === 'unknown' ? 'blue' : 'emerald'}
                  className="py-3"
                />
              </div>
            </section>

            <section className="space-y-2" aria-label={labels.systemResources}>
              <div className="flex items-center justify-between gap-3">
                <div className="text-xs font-semibold uppercase tracking-wide text-ui-muted">
                  {labels.systemResources}
                </div>
                <Badge tone="emerald">{labels.realtime}</Badge>
              </div>
              <div className="grid gap-3 sm:grid-cols-2">
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
            </section>

            <MonitorPanel
              title={labels.throughputTrend}
              icon={<BarChart3 className="h-5 w-5" />}
              action={
                <div data-vector-trend-control={performanceMode.kind === 'vector' || undefined} className="flex rounded-lg border border-ui-line bg-ui-inset p-1">
                  {performanceMode.kind === 'vector' ? (['input', 'items'] as const).map(metric => (
                    <button
                      key={metric}
                      type="button"
                      onClick={() => setVectorTrendMetric(metric)}
                      className={`h-7 min-w-[84px] rounded-md px-2 text-xs font-medium transition ${vectorTrendMetric === metric ? 'bg-ui-action text-ui-action-ink' : 'text-ui-muted hover:text-ui-ink'}`}
                    >
                      {metric === 'input' ? labels.inputThroughput : performanceMode.itemName === 'document' ? labels.documentThroughput : labels.vectorThroughput}
                    </button>
                  )) : (['1m', '5m', '15m', '1h'] as const).map(range => (
                    <button
                      key={range}
                      type="button"
                      onClick={() => setTrendRange(range)}
                      className={`h-7 rounded-md px-2 text-xs font-medium transition ${trendRange === range ? 'bg-ui-action text-ui-action-ink' : 'text-ui-muted hover:text-ui-ink'}`}
                    >
                      {labels.ranges[range]}
                    </button>
                  ))}
                </div>
              }
            >
              {performanceMode.kind === 'inference' ? (
                <DualThroughputTrends prefill={prefillTrend} decode={decodeTrend} start={trendStart} end={trendEnd} lang={lang} />
              ) : <TrendChart
                points={vectorTrend}
                rangeStart={selectedRunning ? trendStart : undefined}
                rangeEnd={selectedRunning ? trendEnd : undefined}
                emptyText={performanceMode.kind === 'vector' ? performanceMode.source.summary : labels.noSamplesYet}
                tone={performanceMode.kind === 'vector' && vectorTrendMetric === 'items' ? 'violet' : 'blue'}
                unit={performanceMode.kind === 'vector' && vectorTrendMetric === 'items'
                  ? performanceMode.itemName === 'document' ? labels.documentsPerSecondShort : labels.itemsPerSecondShort
                  : performanceMode.kind === 'vector' ? 'input tok/s' : 'tok/s'}
              />}
              {performanceMode.kind === 'vector' ? (
                <div data-vector-source-state={performanceMode.source.kind} className="mt-3 flex min-w-0 flex-wrap items-center justify-between gap-2 border-t border-ui-line pt-3 text-xs text-ui-body">
                  <span className="min-w-0 flex-1">{performanceMode.source.summary}</span>
                  <div className="flex flex-wrap gap-2">
                    <Badge tone={performanceMode.analysis?.logAvailable ? 'emerald' : 'slate'}>{performanceMode.source.log}</Badge>
                    <Badge tone={performanceMode.analysis?.proxyAvailable ? 'blue' : 'slate'}>{performanceMode.source.proxy}</Badge>
                  </div>
                </div>
              ) : null}
            </MonitorPanel>

            <MonitorPanel title={labels.activeRequests} icon={<Radio className="h-5 w-5" />} action={<Badge tone={(performanceMode.kind === 'inference' ? currentFrame?.inference?.tasks.length : activeTasks.length) ? 'emerald' : 'slate'}>{performanceMode.kind === 'inference' ? currentFrame?.inference?.tasks.length ?? 0 : activeTasks.length}</Badge>}>
              {performanceMode.kind === 'inference' ? (
                <InferenceTasks entries={[{ frame: selectedRunning ? currentFrame ?? undefined : undefined, name: selectedSession?.instance_name || selectedInstance?.name }]} lang={lang} />
              ) : activeTasks.length === 0 ? (
                <div className="rounded-lg border border-dashed border-ui-line px-4 py-8 text-center text-sm text-ui-muted">
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

            {performanceMode.kind === 'inference' && (speculativeSummary.configured || speculativeSummary.hasData) ? (
              <MonitorPanel
                title={speculativeLabels.title}
                icon={<FastForward className="h-5 w-5" />}
                action={
                  <Badge tone={speculativeSummary.hasData ? 'violet' : 'slate'}>
                    {speculativeSummary.hasData ? speculativeLabels.observed : speculativeLabels.configured}
                  </Badge>
                }
              >
                {speculativeSummary.hasData ? (
                  <div data-speculative-analysis className="grid gap-4 md:grid-cols-[minmax(210px,0.9fr)_minmax(0,1.5fr)] md:items-center">
                    <div className="min-w-0 md:border-r md:border-ui-line md:pr-5">
                      <div className="text-xs font-medium text-ui-muted">{speculativeLabels.acceptanceRate}</div>
                      <div className="mt-2 text-4xl font-semibold text-violet-700 dark:text-violet-300">
                        {formatRatio(speculativeSummary.acceptanceRate)}
                      </div>
                      <div className="mt-3 h-2 overflow-hidden rounded-full bg-ui-line">
                        <div
                          className="h-full rounded-full bg-violet-500 transition-[width] duration-500"
                          style={{ width: `${Math.round((speculativeSummary.acceptanceRate || 0) * 100)}%` }}
                        />
                      </div>
                      <div className="mt-2 text-xs text-ui-muted">
                        {speculativeLabels.tokenRatio(
                          speculativeSummary.acceptedTokens.toLocaleString(),
                          speculativeSummary.generatedTokens.toLocaleString(),
                        )}
                      </div>
                    </div>
                    <div className="grid grid-cols-2 gap-x-5 gap-y-4 sm:grid-cols-4 md:grid-cols-2 xl:grid-cols-4">
                      <InlineStat label={speculativeLabels.latestAcceptance} value={formatRatio(speculativeSummary.latestAcceptanceRate)} />
                      <InlineStat label={speculativeLabels.acceptedTokens} value={speculativeSummary.acceptedTokens.toLocaleString()} />
                      <InlineStat label={speculativeLabels.observedRequests} value={speculativeSummary.requestCount.toLocaleString()} />
                      <InlineStat label={speculativeLabels.avgDraftTime} value={formatMs(speculativeSummary.avgGenerationTimeMs)} />
                    </div>
                  </div>
                ) : (
                  <div data-speculative-analysis="waiting" className="rounded-lg border border-dashed border-ui-line px-4 py-7 text-center text-sm text-ui-muted">
                    {speculativeLabels.waiting}
                  </div>
                )}
              </MonitorPanel>
            ) : null}
          </div>

          <aside className="min-w-0 space-y-4">
            <MonitorPanel title={labels.diagnosis} icon={<AlertTriangle className="h-5 w-5" />} action={<Badge tone={workloadDiagnostics.length > 0 ? 'amber' : 'emerald'}>{workloadDiagnostics.length}</Badge>}>
              {visibleDiagnostics.length === 0 ? (
                <div className="rounded-lg border border-dashed border-ui-line px-3 py-8 text-center text-sm text-ui-muted">
                  {labels.noDiagnostics}
                </div>
              ) : (
                <div className="space-y-3">
                  {visibleDiagnostics.map(finding => (
                    <div key={finding.id} className={`rounded-lg border p-3 ${diagnosticPanelClass(finding.severity)}`}>
                      <div className="mb-2 flex items-start justify-between gap-3">
                        <div className="min-w-0 font-semibold text-ui-ink">{finding.title}</div>
                        <Badge tone={diagnosticTone(finding.severity)}>{diagnosticSeverityLabel(finding.severity, labels)}</Badge>
                      </div>
                      <p className="text-xs leading-5 text-ui-muted">{finding.summary}</p>
                      {finding.recommendation[0] ? (
                        <div className="mt-2 text-xs font-medium text-blue-600 dark:text-blue-300">{finding.recommendation[0]}</div>
                      ) : null}
                    </div>
                  ))}
                  {workloadDiagnostics.length > visibleDiagnostics.length ? (
                    <div className="text-center text-xs text-ui-muted">{labels.moreDiagnostics(workloadDiagnostics.length - visibleDiagnostics.length)}</div>
                  ) : null}
                </div>
              )}
            </MonitorPanel>

            <MonitorPanel title={labels.historyBaseline} icon={<Clock className="h-5 w-5" />}>
              <div className="mb-3 text-xs leading-5 text-ui-muted">
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
                  <MiniStat label={inferenceLabels.decodeAverage} value={formatRate(analysis?.avg_generation_tps)} />
                  <MiniStat label={inferenceLabels.prefillAverage} value={formatRate(analysis?.avg_prompt_tps)} />
                  <MiniStat label={inferenceLabels.prefillTime} value={formatMs(average(requests.map(request => request.prompt_time_ms)))} />
                  <MiniStat label={labels.avgTotalTime} value={formatMs(analysis?.avg_total_time_ms)} />
                  <MiniStat label={labels.maxBusySlots} value={analysis?.max_busy_slots ?? 0} />
                </div>
              )}
            </MonitorPanel>
          </aside>
          </div>}
        </details>
      </>}
    </section>
  )
}

function MiniStat({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="min-w-0 rounded-lg border border-ui-line bg-ui-inset p-3">
      <div className="truncate text-xs text-ui-muted">{label}</div>
      <div className="mt-2 truncate text-lg font-semibold text-ui-ink" title={String(value)}>{value}</div>
    </div>
  )
}

function InlineStat({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="min-w-0">
      <div className="truncate text-xs text-ui-muted" title={label}>{label}</div>
      <div className="mt-1 truncate text-lg font-semibold text-ui-ink" title={String(value)}>{value}</div>
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
    session.config_hash === selectedSession.config_hash &&
    pathsEqual(session.model_path, selectedSession.model_path) &&
    pathsEqual(session.engine_id, selectedSession.engine_id) &&
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
