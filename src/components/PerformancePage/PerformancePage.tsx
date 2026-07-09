import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Activity, BarChart3, ChevronDown, Database, Gauge, HardDrive, RefreshCw, Server } from 'lucide-react'
import { useAppStore } from '../../store'
import { useI18n } from '../../i18n'
import type { DiagnosticFinding, InferenceRequestSummary, SystemMetrics, TelemetryOverview, TelemetrySampleSummary, TelemetrySessionAnalysis, TelemetrySessionSummary } from '../../store/types'
import { Badge, Button, EmptyPanel, InsetSurface, MetricCard, PageFrame, PageHeader, ResourceMeter, SectionHeader, Surface } from '../ui'

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

type ModelBackendAggregate = {
  key: string
  model: string
  engine: string
  backend: string
  count: number
  avgTps: number
  bestTps: number
  peakVram: number
  avgDuration: number | null
  lastStartedAt: number
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
  const running = instances.filter(instance => instance.status === 'running')
  const [selectedInstanceId, setSelectedInstanceId] = useState('')
  const [selectedSessionId, setSelectedSessionId] = useState('')
  const [overview, setOverview] = useState<TelemetryOverview>(emptyOverview)
  const [sessions, setSessions] = useState<TelemetrySessionSummary[]>([])
  const [samples, setSamples] = useState<TelemetrySampleSummary[]>([])
  const [requests, setRequests] = useState<InferenceRequestSummary[]>([])
  const [analysis, setAnalysis] = useState<TelemetrySessionAnalysis | null>(null)
  const [diagnostics, setDiagnostics] = useState<DiagnosticFinding[]>([])
  const [expandedFindings, setExpandedFindings] = useState<Record<string, boolean>>({})
  const [liveSystem, setLiveSystem] = useState<SystemMetrics | null>(null)
  const [liveLlama, setLiveLlama] = useState<MetricsEvent['llama']>(null)
  const [loading, setLoading] = useState(false)
  const [telemetryError, setTelemetryError] = useState<string | null>(null)
  const lastTelemetryRefreshRef = useRef(0)
  const lastSessionDetailRefreshRef = useRef(0)
  const selectedSessionIdRef = useRef('')

  const selectedInstance = instances.find(instance => instance.id === selectedInstanceId)
  const selectedSession = sessions.find(session => session.id === selectedSessionId)

  const refreshTelemetry = useCallback(async (options: { silent?: boolean } = {}) => {
    lastTelemetryRefreshRef.current = Date.now()
    if (!options.silent) {
      setLoading(true)
    }
    try {
      const [nextOverview, nextSessions] = await Promise.all([
        invoke<TelemetryOverview>('get_telemetry_overview'),
        invoke<TelemetrySessionSummary[]>('list_telemetry_sessions', { limit: 24 }),
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
      if (!options.silent) {
        setLoading(false)
      }
    }
  }, [])

  const refreshSelectedSessionDetails = useCallback(async (sessionId: string) => {
    if (!sessionId) {
      return
    }
    lastSessionDetailRefreshRef.current = Date.now()
    try {
      const [nextSamples, nextRequests, nextAnalysis, nextDiagnostics] = await Promise.all([
        invoke<TelemetrySampleSummary[]>('get_telemetry_session_samples', { sessionId, limit: 160 }),
        invoke<InferenceRequestSummary[]>('list_inference_requests', { sessionId, limit: 16 }),
        invoke<TelemetrySessionAnalysis>('get_telemetry_session_analysis', { sessionId }),
        invoke<DiagnosticFinding[]>('get_telemetry_session_diagnostics', { sessionId }),
      ])
      if (selectedSessionIdRef.current !== sessionId) {
        return
      }
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
    if (!selectedInstanceId) {
      return
    }
    invoke<SystemMetrics>('get_system_metrics', { instanceId: selectedInstanceId })
      .then(setLiveSystem)
      .catch(() => setLiveSystem(null))

    const unlisten = listen<MetricsEvent>('metrics-update', event => {
      if (event.payload.instanceId !== selectedInstanceId) {
        return
      }
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
    selectedSessionIdRef.current = selectedSessionId
    if (!selectedSessionId) {
      setSamples([])
      setRequests([])
      setAnalysis(null)
      setDiagnostics([])
      setExpandedFindings({})
      return
    }
    setExpandedFindings({})
    void refreshSelectedSessionDetails(selectedSessionId)
    const timer = window.setInterval(
      () => void refreshSelectedSessionDetails(selectedSessionId),
      TELEMETRY_DETAIL_REFRESH_MS,
    )
    return () => window.clearInterval(timer)
  }, [selectedSessionId, refreshSelectedSessionDetails])

  const latestSample = overview.latest_samples[0] || null
  const diagnosticsBySeverity = useMemo(() => groupDiagnosticsBySeverity(diagnostics), [diagnostics])
  const sessionBenchmark = useMemo(() => buildSessionBenchmark(selectedSession, sessions), [selectedSession, sessions])
  const modelBackendRank = useMemo(() => {
    const byConfig = new Map<string, ModelBackendAggregate & { totalTps: number; durationSum: number; durationCount: number }>()
    for (const session of sessions) {
      const model = session.model_name || '--'
      const engine = session.engine_id || '--'
      const backend = session.backend || '--'
      const key = `${model}\u0000${engine}\u0000${backend}`
      const current = byConfig.get(key) || {
        key,
        model,
        engine,
        backend,
        count: 0,
        avgTps: 0,
        bestTps: 0,
        peakVram: 0,
        avgDuration: null,
        lastStartedAt: 0,
        totalTps: 0,
        durationSum: 0,
        durationCount: 0,
      }
      current.count += 1
      current.totalTps += session.avg_tokens_per_sec || 0
      current.bestTps = Math.max(current.bestTps, session.avg_tokens_per_sec || 0)
      current.peakVram = Math.max(current.peakVram, session.peak_vram_mb || 0)
      current.lastStartedAt = Math.max(current.lastStartedAt, session.started_at || 0)
      if (session.duration_secs) {
        current.durationSum += session.duration_secs
        current.durationCount += 1
      }
      byConfig.set(key, current)
    }
    return Array.from(byConfig.values())
      .map(({ totalTps, durationSum, durationCount, ...item }) => ({
        ...item,
        avgTps: item.count ? totalTps / item.count : 0,
        avgDuration: durationCount ? durationSum / durationCount : null,
      }))
      .sort((a, b) => b.avgTps - a.avgTps || b.count - a.count || b.lastStartedAt - a.lastStartedAt)
      .slice(0, 5)
  }, [sessions])

  const liveInsight = buildInsight(labels, liveSystem, latestSample)
  const toggleFinding = useCallback((id: string) => {
    setExpandedFindings(current => ({ ...current, [id]: !current[id] }))
  }, [])

  return (
    <PageFrame
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
      inspector={
        <div className="space-y-4">
          <Surface as="aside" className="p-4">
            <SectionHeader title={labels.diagnosis} description={selectedSession ? labels.diagnosisDesc : liveInsight.description} />
            {selectedSession ? (
              <div className="mt-4 space-y-3">
                {diagnostics.length === 0 ? (
                  <div className="rounded-lg border border-dashed border-slate-300 px-3 py-6 text-center text-sm text-slate-500 dark:border-slate-700 dark:text-slate-400">
                    {labels.noDiagnostics}
                  </div>
                ) : diagnosticsBySeverity.map(group => (
                  <div key={group.severity} className="space-y-2">
                    <div className="flex items-center justify-between gap-3">
                      <div className="text-xs font-semibold uppercase tracking-wide text-slate-500 dark:text-slate-400">
                        {diagnosticSeverityLabel(group.severity, labels)}
                      </div>
                      <Badge tone={diagnosticTone(group.severity)}>{group.findings.length}</Badge>
                    </div>
                    <div className="space-y-2">
                      {group.findings.map(finding => (
                        <DiagnosticCard
                          key={finding.id}
                          finding={finding}
                          labels={labels}
                          expanded={!!expandedFindings[finding.id]}
                          onToggle={() => toggleFinding(finding.id)}
                        />
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              <div className="mt-4 space-y-3">
                {liveInsight.items.map(item => (
                  <div key={item.label} className="rounded-lg border border-slate-200 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-950">
                    <div className="flex items-center justify-between gap-3">
                      <span className="text-sm text-slate-600 dark:text-slate-400">{item.label}</span>
                      <Badge tone={item.tone}>{item.value}</Badge>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </Surface>

          <Surface as="aside" className="p-4">
            <SectionHeader title={labels.modelRank} description={labels.modelRankDesc} />
            <div className="mt-4 space-y-3">
              {modelBackendRank.length === 0 ? (
                <p className="py-4 text-center text-sm text-slate-500 dark:text-slate-400">{labels.noHistory}</p>
              ) : modelBackendRank.map(item => (
                <div key={item.key} className="rounded-lg border border-slate-200 bg-white p-3 dark:border-slate-800 dark:bg-slate-950">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="truncate text-sm font-semibold text-slate-900 dark:text-slate-100" title={item.model}>{item.model}</div>
                      <div className="mt-1 flex flex-wrap items-center gap-1.5">
                        <Badge tone="blue" className="max-w-full truncate">{item.engine}</Badge>
                        <Badge tone="violet">{item.backend}</Badge>
                      </div>
                    </div>
                    <div className="text-right">
                      <div className="text-sm font-semibold text-slate-950 dark:text-slate-50">{formatRate(item.avgTps)}</div>
                      <div className="text-xs text-slate-500">{labels.avgTps}</div>
                    </div>
                  </div>
                  <div className="mt-3 grid grid-cols-3 gap-2 text-xs text-slate-500 dark:text-slate-400">
                    <span>{item.count} {labels.sessions}</span>
                    <span>{labels.best}: {formatRate(item.bestTps)}</span>
                    <span>{formatGb(item.peakVram)}</span>
                  </div>
                </div>
              ))}
            </div>
          </Surface>
        </div>
      }
    >
      <div className="space-y-4">
        {telemetryError && (
          <Surface className="border-amber-300 bg-amber-50 p-4 text-sm text-amber-900 dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-100">
            <div className="font-semibold">{zh ? '遥测数据读取异常' : 'Telemetry query failed'}</div>
            <div className="mt-1 truncate text-xs opacity-90" title={telemetryError}>{telemetryError}</div>
          </Surface>
        )}
        <div className="grid gap-4 md:grid-cols-2 2xl:grid-cols-4">
          <MetricCard label={labels.activeSessions} value={overview.active_sessions} icon={<Server className="h-5 w-5" />} tone="border-emerald-500/20 bg-emerald-500/10 text-emerald-300" />
          <MetricCard label={labels.sessions24h} value={overview.sessions_24h} icon={<Database className="h-5 w-5" />} tone="border-blue-500/20 bg-blue-500/10 text-blue-300" />
          <MetricCard label={labels.avgTps24h} value={formatRate(overview.avg_tokens_per_sec_24h)} icon={<Gauge className="h-5 w-5" />} tone="border-amber-500/20 bg-amber-500/10 text-amber-300" valueClassName="text-2xl" />
          <MetricCard label={labels.peakVram24h} value={formatGb(overview.peak_vram_mb_24h)} icon={<HardDrive className="h-5 w-5" />} tone="border-violet-500/20 bg-violet-500/10 text-violet-300" valueClassName="text-2xl" />
        </div>

        <Surface as="section" className="p-5" data-guide="perf-select">
          <div className="mb-4 flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
            <SectionHeader title={labels.liveMonitor} description={labels.liveMonitorDesc} />
            <select
              value={selectedInstanceId}
              onChange={event => setSelectedInstanceId(event.target.value)}
              className="select-custom h-11 min-w-[220px] rounded-lg border border-slate-300 bg-white pl-3 pr-8 text-sm text-slate-900 dark:border-slate-700 dark:bg-slate-950 dark:text-slate-100"
            >
              {running.length === 0 ? <option value="">{labels.noRunning}</option> : null}
              {running.map(instance => (
                <option key={instance.id} value={instance.id}>{instance.name}</option>
              ))}
            </select>
          </div>

          {running.length === 0 ? (
            <EmptyPanel title={labels.noRunning} description={labels.noRunningDesc} />
          ) : (
            <div className="grid gap-3 xl:grid-cols-3">
              <ResourceMeter
                label="CPU"
                value={liveSystem?.cpu_percent || 0}
                tone="blue"
                description={`${labels.system} ${Math.round(liveSystem?.system_cpu_percent || 0)}%`}
              />
              <ResourceMeter
                label="GPU"
                value={liveSystem?.gpu_percent || 0}
                tone="emerald"
                description={liveSystem?.gpu_vendor || labels.unknownGpu}
              />
              <ResourceMeter
                label={labels.memory}
                value={liveSystem?.system_memory_used_mb || liveSystem?.memory_mb || 0}
                max={liveSystem?.system_memory_total_mb || Math.max(liveSystem?.memory_mb || 1, 1)}
                unit="MB"
                tone="violet"
                description={formatMemoryPair(liveSystem?.system_memory_used_mb || liveSystem?.memory_mb, liveSystem?.system_memory_total_mb)}
              />
              <InsetSurface className="p-4">
                <div className="text-sm text-slate-500 dark:text-slate-400">{labels.selectedInstance}</div>
                <div className="mt-2 truncate text-base font-semibold text-slate-950 dark:text-slate-50">{selectedInstance?.name || '--'}</div>
                <div className="mt-1 text-xs text-slate-500">{selectedInstance?.config.host}:{selectedInstance?.config.port}</div>
              </InsetSurface>
              <InsetSurface className="p-4">
                <div className="text-sm text-slate-500 dark:text-slate-400">{labels.currentTps}</div>
                <div className="mt-2 text-2xl font-semibold text-slate-950 dark:text-slate-50">{formatRate(liveLlama?.tokens_per_sec || latestSample?.tokens_per_sec || 0)}</div>
                <div className="mt-1 text-xs text-slate-500">{labels.fromLlamaMetrics}</div>
              </InsetSurface>
              <InsetSurface className="p-4">
                <div className="text-sm text-slate-500 dark:text-slate-400">{labels.queuePressure}</div>
                <div className="mt-2 text-2xl font-semibold text-slate-950 dark:text-slate-50">{liveLlama?.requests_processing ?? latestSample?.requests_processing ?? 0} / {liveLlama?.requests_deferred ?? latestSample?.requests_deferred ?? 0}</div>
                <div className="mt-1 text-xs text-slate-500">{labels.processingDeferred}</div>
              </InsetSurface>
            </div>
          )}
        </Surface>

        <div className="grid gap-4 2xl:grid-cols-[minmax(0,420px)_minmax(0,1fr)]">
          <Surface as="section" className="overflow-hidden">
            <div className="border-b border-slate-200 px-5 py-4 dark:border-slate-800">
              <SectionHeader title={labels.sessionHistory} description={labels.sessionHistoryDesc} />
            </div>
            <div className="max-h-[520px] overflow-y-auto p-3">
              {sessions.length === 0 ? (
                <EmptyPanel title={labels.noHistory} description={labels.noHistoryDesc} />
              ) : sessions.map(session => (
                <button
                  key={session.id}
                  type="button"
                  onClick={() => {
                    selectedSessionIdRef.current = session.id
                    setSelectedSessionId(session.id)
                  }}
                  className={`mb-2 block w-full rounded-lg border p-3 text-left transition ${
                    session.id === selectedSessionId
                      ? 'border-blue-400 bg-blue-50 dark:border-blue-500/40 dark:bg-blue-950/30'
                      : 'border-slate-200 bg-white hover:border-slate-300 dark:border-slate-800 dark:bg-slate-950 dark:hover:border-slate-700'
                  }`}
                >
                  <div className="flex min-w-0 items-center justify-between gap-3">
                    <div className="min-w-0">
                      <div className="truncate text-sm font-semibold text-slate-950 dark:text-slate-100" title={session.instance_name}>{session.instance_name}</div>
                      <div className="mt-1 truncate text-xs text-slate-500" title={session.model_name}>{session.model_name}</div>
                    </div>
                    <Badge tone={session.stopped_at ? 'slate' : 'emerald'}>{session.stopped_at ? labels.finished : labels.running}</Badge>
                  </div>
                  <div className="mt-3 grid grid-cols-3 gap-2 text-xs text-slate-500 dark:text-slate-400">
                    <span>{formatRate(session.avg_tokens_per_sec)}</span>
                    <span>{formatGb(session.peak_vram_mb)}</span>
                    <span>{session.sample_count} {labels.samples}</span>
                  </div>
                </button>
              ))}
            </div>
          </Surface>

          <Surface as="section" className="p-5">
            <SectionHeader
              title={labels.sessionReport}
              description={selectedSession ? `${selectedSession.instance_name} · ${formatDate(selectedSession.started_at)}` : labels.selectSession}
              action={selectedSession ? <Badge tone={selectedSession.stopped_at ? 'slate' : 'emerald'}>{selectedSession.stopped_at ? labels.finished : labels.running}</Badge> : null}
            />

            {!selectedSession ? (
              <EmptyPanel className="mt-4" title={labels.selectSession} description={labels.selectSessionDesc} />
            ) : (
              <div className="mt-4 space-y-4">
                <div className="grid gap-3 md:grid-cols-4">
                  <InsetSurface className="p-3">
                    <div className="text-xs text-slate-500">{labels.duration}</div>
                    <div className="mt-2 text-lg font-semibold text-slate-950 dark:text-slate-50">{formatDuration(selectedSession.duration_secs)}</div>
                  </InsetSurface>
                  <InsetSurface className="p-3">
                    <div className="text-xs text-slate-500">{labels.avgTps}</div>
                    <div className="mt-2 text-lg font-semibold text-slate-950 dark:text-slate-50">{formatRate(selectedSession.avg_tokens_per_sec)}</div>
                  </InsetSurface>
                  <InsetSurface className="p-3">
                    <div className="text-xs text-slate-500">{labels.peakVram}</div>
                    <div className="mt-2 text-lg font-semibold text-slate-950 dark:text-slate-50">{formatGb(selectedSession.peak_vram_mb)}</div>
                  </InsetSurface>
                  <InsetSurface className="p-3">
                    <div className="text-xs text-slate-500">{labels.samples}</div>
                    <div className="mt-2 text-lg font-semibold text-slate-950 dark:text-slate-50">{selectedSession.sample_count}</div>
                  </InsetSurface>
                </div>

                <SessionComparison benchmark={sessionBenchmark} selectedSession={selectedSession} labels={labels} />

                <div className="grid gap-3 md:grid-cols-3 xl:grid-cols-6">
                  <InsetSurface className="p-3">
                    <div className="text-xs text-slate-500">{labels.requests}</div>
                    <div className="mt-2 text-lg font-semibold text-slate-950 dark:text-slate-50">{analysis?.request_count ?? 0}</div>
                  </InsetSurface>
                  <InsetSurface className="p-3">
                    <div className="text-xs text-slate-500">{labels.avgGenerationSpeed}</div>
                    <div className="mt-2 text-lg font-semibold text-slate-950 dark:text-slate-50">{formatRate(analysis?.avg_generation_tps)}</div>
                  </InsetSurface>
                  <InsetSurface className="p-3">
                    <div className="text-xs text-slate-500">{labels.avgTotalTime}</div>
                    <div className="mt-2 text-lg font-semibold text-slate-950 dark:text-slate-50">{formatMs(analysis?.avg_total_time_ms)}</div>
                  </InsetSurface>
                  <InsetSurface className="p-3">
                    <div className="text-xs text-slate-500">{labels.maxBusySlots}</div>
                    <div className="mt-2 text-lg font-semibold text-slate-950 dark:text-slate-50">{analysis?.max_busy_slots ?? 0}</div>
                  </InsetSurface>
                  <InsetSurface className="p-3">
                    <div className="text-xs text-slate-500">{labels.avgCachedSlots}</div>
                    <div className="mt-2 text-lg font-semibold text-slate-950 dark:text-slate-50">{formatFixed(analysis?.avg_cached_slots)}</div>
                  </InsetSurface>
                  <InsetSurface className="p-3">
                    <div className="text-xs text-slate-500">{labels.maxContext}</div>
                    <div className="mt-2 text-lg font-semibold text-slate-950 dark:text-slate-50">{formatTokenCount(analysis?.max_context_tokens)}</div>
                  </InsetSurface>
                </div>

                <div className="rounded-lg border border-slate-200 bg-white p-4 dark:border-slate-800 dark:bg-slate-950">
                  <div className="mb-3 flex items-center gap-2 text-sm font-medium text-slate-900 dark:text-slate-100">
                    <BarChart3 className="h-4 w-4 text-blue-400" />
                    {labels.throughputTrend}
                  </div>
                  <TelemetryCurve samples={samples} emptyText={labels.noSamplesYet} />
                </div>

                <div className="grid gap-3 xl:grid-cols-2">
                  <Detail label={labels.model} value={selectedSession.model_name} />
                  <Detail label={labels.engine} value={`${selectedSession.engine_id || '--'} · ${selectedSession.backend || '--'}`} />
                  <Detail label={labels.startedAt} value={formatDate(selectedSession.started_at)} />
                  <Detail label={labels.stoppedAt} value={selectedSession.stopped_at ? formatDate(selectedSession.stopped_at) : labels.stillRunning} />
                </div>

                <div className="rounded-lg border border-slate-200 bg-white p-4 dark:border-slate-800 dark:bg-slate-950">
                  <div className="mb-3 flex flex-col gap-1 sm:flex-row sm:items-center sm:justify-between">
                    <div>
                      <div className="text-sm font-medium text-slate-900 dark:text-slate-100">{labels.recentRequests}</div>
                      <div className="text-xs text-slate-500 dark:text-slate-400">{labels.recentRequestsDesc}</div>
                    </div>
                    <Badge tone={requests.length > 0 ? 'emerald' : 'slate'}>{requests.length} {labels.requests}</Badge>
                  </div>
                  {requests.length === 0 ? (
                    <div className="rounded-lg border border-dashed border-slate-300 px-4 py-8 text-center text-sm text-slate-500 dark:border-slate-700 dark:text-slate-400">
                      {labels.noRequestsYet}
                    </div>
                  ) : (
                    <div className="overflow-hidden rounded-lg border border-slate-200 dark:border-slate-800">
                      <table className="w-full table-fixed text-left text-sm">
                        <thead className="bg-slate-50 text-xs text-slate-500 dark:bg-slate-900 dark:text-slate-400">
                          <tr>
                            <th className="px-3 py-2 font-medium">{labels.task}</th>
                            <th className="px-3 py-2 font-medium">{labels.source}</th>
                            <th className="px-3 py-2 font-medium">{labels.prompt}</th>
                            <th className="px-3 py-2 font-medium">{labels.generated}</th>
                            <th className="px-3 py-2 font-medium">{labels.generationSpeed}</th>
                            <th className="px-3 py-2 font-medium">{labels.totalTime}</th>
                            <th className="px-3 py-2 font-medium">{labels.specAccept}</th>
                          </tr>
                        </thead>
                        <tbody className="divide-y divide-slate-200 dark:divide-slate-800">
                          {requests.map(request => (
                            <tr key={`${request.session_id}-${request.task_id}`} className="text-slate-700 dark:text-slate-200">
                              <td className="px-3 py-2">
                                <div className="font-medium">#{request.task_id}</div>
                                <div className="text-xs text-slate-500">{request.source === 'proxy' ? (request.model || '--') : `slot ${request.slot_id}`}</div>
                              </td>
                              <td className="px-3 py-2">
                                <Badge tone={request.source === 'proxy' ? 'blue' : 'slate'}>{request.source === 'proxy' ? labels.proxy : labels.log}</Badge>
                                {request.http_status ? <div className="mt-1 text-xs text-slate-500">HTTP {request.http_status}</div> : null}
                              </td>
                              <td className="px-3 py-2">{formatTokenCount(request.prompt_tokens)}</td>
                              <td className="px-3 py-2">{formatTokenCount(request.generated_tokens)}</td>
                              <td className="px-3 py-2">{formatRate(request.generation_tps)}</td>
                              <td className="px-3 py-2">
                                {request.source === 'proxy' ? `${labels.firstResponse} ${formatMs(request.total_time_ms)}` : formatMs(request.total_time_ms)}
                              </td>
                              <td className="px-3 py-2">{formatPercent(request.spec_accept_rate)}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  )}
                </div>
              </div>
            )}
          </Surface>
        </div>
      </div>
    </PageFrame>
  )
}

function SessionComparison({
  benchmark,
  selectedSession,
  labels,
}: {
  benchmark: SessionBenchmark
  selectedSession: TelemetrySessionSummary
  labels: ReturnType<typeof getLabels>
}) {
  const rows = [
    {
      id: 'throughput',
      label: labels.avgTps,
      current: formatRate(selectedSession.avg_tokens_per_sec),
      history: formatRate(benchmark.avg.tokensPerSec),
      best: formatRate(benchmark.best?.avg_tokens_per_sec),
      delta: formatDelta(selectedSession.avg_tokens_per_sec, benchmark.avg.tokensPerSec, true),
    },
    {
      id: 'vram',
      label: labels.peakVram,
      current: formatGb(selectedSession.peak_vram_mb),
      history: formatGb(benchmark.avg.peakVramMb),
      best: formatGb(benchmark.best?.peak_vram_mb),
      delta: formatDelta(selectedSession.peak_vram_mb, benchmark.avg.peakVramMb, false),
    },
    {
      id: 'duration',
      label: labels.duration,
      current: formatDuration(selectedSession.duration_secs),
      history: formatDuration(benchmark.avg.durationSecs),
      best: formatDuration(benchmark.best?.duration_secs),
      delta: null,
    },
  ]

  return (
    <div className="rounded-lg border border-slate-200 bg-white p-4 dark:border-slate-800 dark:bg-slate-950">
      <div className="mb-3 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <div className="text-sm font-medium text-slate-900 dark:text-slate-100">{labels.sessionComparison}</div>
          <div className="text-xs text-slate-500 dark:text-slate-400">{sessionComparisonScopeLabel(benchmark, labels)}</div>
        </div>
        <Badge tone={benchmark.historyCount > 0 ? 'blue' : 'slate'}>{benchmark.historyCount} {labels.sessions}</Badge>
      </div>
      <div className="overflow-hidden rounded-lg border border-slate-200 dark:border-slate-800">
        <table className="w-full table-fixed text-left text-sm">
          <thead className="bg-slate-50 text-xs text-slate-500 dark:bg-slate-900 dark:text-slate-400">
            <tr>
              <th className="px-3 py-2 font-medium">{labels.metric}</th>
              <th className="px-3 py-2 font-medium">{labels.currentSession}</th>
              <th className="px-3 py-2 font-medium">{labels.historyAverage}</th>
              <th className="px-3 py-2 font-medium">{labels.bestSession}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-slate-200 dark:divide-slate-800">
            {rows.map(row => (
              <tr key={row.id} className="text-slate-700 dark:text-slate-200">
                <td className="px-3 py-2 font-medium text-slate-900 dark:text-slate-100">{row.label}</td>
                <td className="px-3 py-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <span>{row.current}</span>
                    {row.delta ? <Badge tone={row.delta.tone}>{row.delta.text}</Badge> : null}
                  </div>
                </td>
                <td className="px-3 py-2">{row.history}</td>
                <td className="px-3 py-2">{row.best}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {benchmark.best ? (
        <div className="mt-3 flex flex-wrap items-center gap-2 text-xs text-slate-500 dark:text-slate-400">
          <span>{labels.bestSession}</span>
          <span className="truncate" title={benchmark.best.instance_name}>{benchmark.best.instance_name}</span>
          <span>{formatDate(benchmark.best.started_at)}</span>
        </div>
      ) : null}
    </div>
  )
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-lg border border-slate-200 bg-white p-3 dark:border-slate-800 dark:bg-slate-950">
      <div className="text-xs text-slate-500">{label}</div>
      <div className="mt-2 truncate text-sm font-medium text-slate-900 dark:text-slate-100" title={value}>{value}</div>
    </div>
  )
}

function DiagnosticCard({
  finding,
  labels,
  expanded,
  onToggle,
}: {
  finding: DiagnosticFinding
  labels: ReturnType<typeof getLabels>
  expanded: boolean
  onToggle: () => void
}) {
  const hasDetails = finding.evidence.length > 0 || finding.recommendation.length > 0

  return (
    <div className="rounded-lg border border-slate-200 bg-white p-3 dark:border-slate-800 dark:bg-slate-950">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-semibold text-slate-950 dark:text-slate-100">{finding.title}</div>
          <p className="mt-1 text-xs leading-5 text-slate-500 dark:text-slate-400">{finding.summary}</p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Badge tone={diagnosticTone(finding.severity)}>
            {diagnosticSeverityLabel(finding.severity, labels)}
          </Badge>
          {hasDetails ? (
            <button
              type="button"
              onClick={onToggle}
              className="inline-flex h-7 w-7 items-center justify-center rounded-md border border-slate-200 text-slate-500 transition hover:bg-slate-50 dark:border-slate-800 dark:text-slate-400 dark:hover:bg-slate-900"
              aria-label={expanded ? labels.collapseDiagnostic : labels.expandDiagnostic}
            >
              <ChevronDown className={`h-4 w-4 transition ${expanded ? 'rotate-180' : ''}`} />
            </button>
          ) : null}
        </div>
      </div>
      {expanded ? (
        <div className="mt-3 space-y-3">
          {finding.evidence.length > 0 ? (
            <div className="rounded-md bg-slate-50 p-2 dark:bg-slate-900">
              <div className="mb-1 text-xs font-medium text-slate-500 dark:text-slate-400">{labels.evidence}</div>
              <div className="space-y-1 text-xs text-slate-600 dark:text-slate-300">
                {finding.evidence.map(item => (
                  <div key={item} className="break-words">{item}</div>
                ))}
              </div>
            </div>
          ) : null}
          {finding.recommendation.length > 0 ? (
            <div className="rounded-md border border-slate-200 p-2 dark:border-slate-800">
              <div className="mb-1 text-xs font-medium text-slate-500 dark:text-slate-400">{labels.recommendation}</div>
              <div className="space-y-1 text-xs leading-5 text-slate-600 dark:text-slate-300">
                {finding.recommendation.map(item => (
                  <div key={item}>{item}</div>
                ))}
              </div>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

function groupDiagnosticsBySeverity(diagnostics: DiagnosticFinding[]) {
  const order: DiagnosticFinding['severity'][] = ['critical', 'warning', 'info', 'success']
  return order
    .map(severity => ({
      severity,
      findings: diagnostics.filter(finding => finding.severity === severity),
    }))
    .filter(group => group.findings.length > 0)
}

function buildSessionBenchmark(selectedSession: TelemetrySessionSummary | undefined, sessions: TelemetrySessionSummary[]): SessionBenchmark {
  if (!selectedSession) {
    return emptySessionBenchmark()
  }

  const history = sessions.filter(session => session.id !== selectedSession.id && session.sample_count > 0)
  const sameConfig = history.filter(session =>
    session.model_name === selectedSession.model_name &&
    session.engine_id === selectedSession.engine_id &&
    session.backend === selectedSession.backend
  )
  const basis = sameConfig.length > 0 ? sameConfig : history
  if (basis.length === 0) {
    return emptySessionBenchmark()
  }

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
    avg: {
      tokensPerSec: null,
      peakVramMb: null,
      durationSecs: null,
    },
    best: null,
  }
}

function average(values: Array<number | null | undefined>) {
  const valid = values.filter((value): value is number => value != null && Number.isFinite(value))
  if (valid.length === 0) return null
  return valid.reduce((sum, value) => sum + value, 0) / valid.length
}

function formatDelta(current?: number | null, baseline?: number | null, higherIsBetter = true) {
  if (current == null || baseline == null || !Number.isFinite(current) || !Number.isFinite(baseline) || baseline === 0) {
    return null
  }
  const percent = ((current - baseline) / baseline) * 100
  if (Math.abs(percent) < 1) {
    return { text: '0%', tone: 'slate' as const }
  }
  const better = higherIsBetter ? percent > 0 : percent < 0
  const prefix = percent > 0 ? '+' : ''
  return {
    text: `${prefix}${percent.toFixed(0)}%`,
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

function diagnosticSeverityLabel(severity: DiagnosticFinding['severity'], labels: ReturnType<typeof getLabels>) {
  if (severity === 'critical') return labels.severityCritical
  if (severity === 'warning') return labels.severityWarning
  if (severity === 'success') return labels.severitySuccess
  return labels.severityInfo
}

function TelemetryCurve({ samples, emptyText }: { samples: TelemetrySampleSummary[]; emptyText: string }) {
  if (samples.length < 2) {
    return (
      <div className="flex min-h-[180px] items-center justify-center text-sm text-slate-500 dark:text-slate-400">
        <Activity className="mr-2 h-4 w-4" />
        {emptyText}
      </div>
    )
  }
  const values = samples.map(sample => sample.tokens_per_sec || 0)
  const max = Math.max(...values, 1)
  const width = 720
  const height = 180
  const points = values.map((value, index) => {
    const x = values.length === 1 ? 0 : (index / (values.length - 1)) * width
    const y = height - (value / max) * (height - 16) - 8
    return `${x},${y}`
  }).join(' ')

  return (
    <svg viewBox={`0 0 ${width} ${height}`} className="h-[180px] w-full">
      <line x1="0" y1={height - 8} x2={width} y2={height - 8} stroke="currentColor" className="text-slate-200 dark:text-slate-800" strokeWidth="1" />
      <polyline points={points} fill="none" stroke="#3b82f6" strokeWidth="3" strokeLinejoin="round" strokeLinecap="round" />
      {values.map((value, index) => {
        const x = values.length === 1 ? 0 : (index / (values.length - 1)) * width
        const y = height - (value / max) * (height - 16) - 8
        return <circle key={`${index}-${value}`} cx={x} cy={y} r="3" fill="#60a5fa" />
      })}
    </svg>
  )
}

function buildInsight(labels: ReturnType<typeof getLabels>, system: SystemMetrics | null, latest: TelemetrySampleSummary | null) {
  const gpu = system?.gpu_percent ?? latest?.gpu_percent ?? 0
  const vram = system?.vram_used_mb && system.vram_total_mb ? (system.vram_used_mb / system.vram_total_mb) * 100 : 0
  const tps = latest?.tokens_per_sec || 0
  const items = [
    { label: labels.gpuPressure, value: `${Math.round(gpu)}%`, tone: gpu > 85 ? 'amber' as const : 'emerald' as const },
    { label: labels.vramPressure, value: `${Math.round(vram)}%`, tone: vram > 90 ? 'red' as const : 'blue' as const },
    { label: labels.throughput, value: formatRate(tps), tone: tps > 0 ? 'emerald' as const : 'slate' as const },
  ]
  return {
    description: labels.diagnosisDesc,
    items,
  }
}

function getLabels(zh: boolean) {
  return {
    title: zh ? '\u6027\u80fd\u5206\u6790' : 'Performance Analytics',
    description: zh ? '\u57fa\u4e8e SQLite \u6301\u4e45\u5316\u9065\u6d4b\uff0c\u8ffd\u8e2a\u5b9e\u4f8b\u8fd0\u884c\u3001\u6a21\u578b\u541e\u5410\u4e0e\u8d44\u6e90\u538b\u529b\u3002' : 'SQLite-backed telemetry for instance runtime, model throughput, and resource pressure.',
    sqliteBacked: zh ? 'SQLite \u9065\u6d4b' : 'SQLite telemetry',
    refresh: zh ? '\u5237\u65b0' : 'Refresh',
    activeSessions: zh ? '\u6d3b\u52a8\u4f1a\u8bdd' : 'Active Sessions',
    sessions24h: zh ? '24\u5c0f\u65f6\u4f1a\u8bdd' : 'Sessions 24h',
    avgTps24h: zh ? '24\u5c0f\u65f6\u5e73\u5747\u541e\u5410' : 'Avg Throughput 24h',
    peakVram24h: zh ? '24\u5c0f\u65f6\u5cf0\u503c\u663e\u5b58' : 'Peak VRAM 24h',
    liveMonitor: zh ? '\u5b9e\u65f6\u76d1\u63a7' : 'Live Monitor',
    liveMonitorDesc: zh ? '\u540e\u7aef\u6301\u7eed\u91c7\u96c6\u5e76\u5199\u5165\u9065\u6d4b\u5e93\uff0c\u9875\u9762\u53ea\u8d1f\u8d23\u67e5\u770b\u548c\u5206\u6790\u3002' : 'The backend keeps collecting telemetry; this view focuses on inspection and analysis.',
    noRunning: zh ? '\u5f53\u524d\u6ca1\u6709\u8fd0\u884c\u4e2d\u7684\u5b9e\u4f8b' : 'No running instances',
    noRunningDesc: zh ? '\u542f\u52a8\u5b9e\u4f8b\u540e\u5c06\u81ea\u52a8\u521b\u5efa\u8fd0\u884c\u4f1a\u8bdd\u5e76\u5199\u5165 SQLite\u3002' : 'Start an instance to create a telemetry session automatically.',
    system: zh ? '\u7cfb\u7edf' : 'System',
    unknownGpu: zh ? '\u672a\u68c0\u6d4b GPU' : 'GPU not detected',
    memory: zh ? '\u5185\u5b58' : 'Memory',
    selectedInstance: zh ? '\u76d1\u63a7\u5b9e\u4f8b' : 'Monitored Instance',
    currentTps: zh ? '\u5f53\u524d\u751f\u6210\u541e\u5410' : 'Current Generation',
    fromLlamaMetrics: zh ? '\u6765\u81ea llama-server /metrics' : 'From llama-server /metrics',
    queuePressure: zh ? '\u8bf7\u6c42\u538b\u529b' : 'Request Pressure',
    processingDeferred: zh ? '\u5904\u7406\u4e2d / \u5ef6\u8fdf' : 'Processing / deferred',
    sessionHistory: zh ? '\u8fd0\u884c\u4f1a\u8bdd' : 'Run Sessions',
    sessionHistoryDesc: zh ? '\u6bcf\u6b21\u5b9e\u4f8b\u8fd0\u884c\u90fd\u4f1a\u5f62\u6210\u4e00\u4efd\u53ef\u56de\u770b\u7684\u6027\u80fd\u8bb0\u5f55\u3002' : 'Each instance run becomes a reviewable performance record.',
    noHistory: zh ? '\u6682\u65e0\u5386\u53f2\u9065\u6d4b' : 'No telemetry history',
    noHistoryDesc: zh ? '\u542f\u52a8\u5b9e\u4f8b\u5e76\u7b49\u5f85\u81f3\u5c11\u4e00\u6b21\u91c7\u6837\u540e\uff0c\u8fd9\u91cc\u4f1a\u51fa\u73b0\u4f1a\u8bdd\u8bb0\u5f55\u3002' : 'Start an instance and wait for at least one sample.',
    sessionReport: zh ? '\u4f1a\u8bdd\u62a5\u544a' : 'Session Report',
    selectSession: zh ? '\u9009\u62e9\u4e00\u4e2a\u8fd0\u884c\u4f1a\u8bdd' : 'Select a run session',
    selectSessionDesc: zh ? '\u4ece\u5de6\u4fa7\u9009\u62e9\u4f1a\u8bdd\u540e\u67e5\u770b\u541e\u5410\u66f2\u7ebf\u4e0e\u8d44\u6e90\u6982\u8981\u3002' : 'Pick a session to inspect throughput and resource summary.',
    finished: zh ? '\u5df2\u7ed3\u675f' : 'Finished',
    running: zh ? '\u8fd0\u884c\u4e2d' : 'Running',
    sessions: zh ? '\u6b21' : 'sessions',
    samples: zh ? '\u91c7\u6837' : 'samples',
    best: zh ? '\u6700\u4f73' : 'Best',
    duration: zh ? '\u65f6\u957f' : 'Duration',
    avgTps: zh ? '\u5e73\u5747\u541e\u5410' : 'Avg TPS',
    peakVram: zh ? '\u5cf0\u503c\u663e\u5b58' : 'Peak VRAM',
    sessionComparison: zh ? '\u4f1a\u8bdd\u5bf9\u6bd4' : 'Session Comparison',
    metric: zh ? '\u6307\u6807' : 'Metric',
    currentSession: zh ? '\u672c\u6b21' : 'Current',
    historyAverage: zh ? '\u5386\u53f2\u5e73\u5747' : 'History Avg',
    bestSession: zh ? '\u6700\u4f73\u4f1a\u8bdd' : 'Best Session',
    sameConfigHistory: zh ? '\u4f18\u5148\u5bf9\u6bd4\u540c\u6a21\u578b\u3001\u540c\u5f15\u64ce\u548c\u540c\u540e\u7aef\u7684\u5386\u53f2\u4f1a\u8bdd\u3002' : 'Compares against previous sessions with the same model, engine, and backend.',
    allHistoryFallback: zh ? '\u6682\u65e0\u540c\u914d\u7f6e\u5386\u53f2\uff0c\u5df2\u9000\u56de\u5168\u90e8\u5386\u53f2\u4f1a\u8bdd\u3002' : 'No same-config history yet, so this falls back to all previous sessions.',
    noHistoryBaseline: zh ? '\u6682\u65e0\u53ef\u7528\u5386\u53f2\u57fa\u7ebf\u3002' : 'No historical baseline is available yet.',
    throughputTrend: zh ? '\u541e\u5410\u8d8b\u52bf' : 'Throughput Trend',
    noSamplesYet: zh ? '\u6682\u65e0\u8db3\u591f\u9065\u6d4b\u91c7\u6837' : 'No telemetry samples yet',
    model: zh ? '\u6a21\u578b' : 'Model',
    engine: zh ? '\u5f15\u64ce' : 'Engine',
    startedAt: zh ? '\u5f00\u59cb\u65f6\u95f4' : 'Started',
    stoppedAt: zh ? '\u7ed3\u675f\u65f6\u95f4' : 'Stopped',
    stillRunning: zh ? '\u4ecd\u5728\u8fd0\u884c' : 'Still running',
    diagnosis: zh ? '\u667a\u80fd\u8bca\u65ad' : 'Smart Diagnostics',
    diagnosisDesc: zh ? '\u57fa\u4e8e\u8d44\u6e90\u91c7\u6837\u3001\u8bf7\u6c42\u62c6\u89e3\u3001slot \u72b6\u6001\u548c\u5386\u53f2\u57fa\u7ebf\u751f\u6210\u8bca\u65ad\u7ed3\u8bba\u3002' : 'Findings based on resource samples, request breakdown, slot state, and historical baselines.',
    noDiagnostics: zh ? '\u5f53\u524d\u4f1a\u8bdd\u6682\u65e0\u53ef\u7528\u8bca\u65ad\u7ed3\u8bba' : 'No diagnostics available for this session yet',
    severityCritical: zh ? '\u9ad8\u98ce\u9669' : 'Critical',
    severityWarning: zh ? '\u63d0\u9192' : 'Warning',
    severityInfo: zh ? '\u4fe1\u606f' : 'Info',
    severitySuccess: zh ? '\u6b63\u5e38' : 'Healthy',
    evidence: zh ? '\u8bc1\u636e' : 'Evidence',
    recommendation: zh ? '\u5efa\u8bae' : 'Recommendation',
    expandDiagnostic: zh ? '\u5c55\u5f00\u8bca\u65ad\u8be6\u60c5' : 'Expand diagnostic details',
    collapseDiagnostic: zh ? '\u6536\u8d77\u8bca\u65ad\u8be6\u60c5' : 'Collapse diagnostic details',
    gpuPressure: zh ? 'GPU \u538b\u529b' : 'GPU Pressure',
    vramPressure: zh ? '\u663e\u5b58\u538b\u529b' : 'VRAM Pressure',
    throughput: zh ? '\u541e\u5410' : 'Throughput',
    modelRank: zh ? '\u6a21\u578b\u8868\u73b0' : 'Model Performance',
    modelRankDesc: zh ? '\u6309\u6a21\u578b + \u5f15\u64ce/\u540e\u7aef\u805a\u5408\u7684\u5386\u53f2\u541e\u5410\u5361\u7247\u3002' : 'Historical throughput cards grouped by model plus engine/backend.',
    recentRequests: zh ? '\u6700\u8fd1\u63a8\u7406\u8bf7\u6c42' : 'Recent Inference Requests',
    recentRequestsDesc: zh ? '\u6765\u81ea llama-server \u65e5\u5fd7\u89e3\u6790\u7684\u8bf7\u6c42\u7ea7 token\u3001\u8017\u65f6\u4e0e\u541e\u5410\u8bb0\u5f55\u3002' : 'Request-level tokens, duration, and throughput parsed from llama-server logs.',
    requests: zh ? '\u8bf7\u6c42' : 'requests',
    noRequestsYet: zh ? '\u6682\u65e0\u5df2\u5b8c\u6210\u7684\u63a8\u7406\u8bf7\u6c42\u8bb0\u5f55' : 'No completed inference request records yet',
    task: zh ? '\u4efb\u52a1' : 'Task',
    source: zh ? '\u6765\u6e90' : 'Source',
    proxy: zh ? '\u8def\u7531' : 'Proxy',
    log: zh ? '\u65e5\u5fd7' : 'Log',
    prompt: zh ? '\u63d0\u793a\u8bcd' : 'Prompt',
    generated: zh ? '\u751f\u6210' : 'Generated',
    generationSpeed: zh ? '\u751f\u6210\u901f\u5ea6' : 'Gen Speed',
    totalTime: zh ? '\u603b\u8017\u65f6' : 'Total Time',
    firstResponse: zh ? '\u9996\u54cd' : 'First byte',
    specAccept: zh ? '\u63a8\u6d4b\u63a5\u53d7' : 'Spec Accept',
    avgGenerationSpeed: zh ? '\u5e73\u5747\u751f\u6210\u901f\u5ea6' : 'Avg Gen Speed',
    avgTotalTime: zh ? '\u5e73\u5747\u603b\u8017\u65f6' : 'Avg Total Time',
    maxBusySlots: zh ? '\u5fd9\u788c slot \u5cf0\u503c' : 'Peak Busy Slots',
    avgCachedSlots: zh ? '\u5e73\u5747\u7f13\u5b58 slot' : 'Avg Cached Slots',
    maxContext: zh ? '\u6700\u5927\u4e0a\u4e0b\u6587' : 'Max Context',
  }
}

function formatRate(value?: number | null) {
  if (!value || !Number.isFinite(value)) return '--'
  return `${value.toFixed(value >= 10 ? 1 : 2)} tok/s`
}

function formatGb(mb?: number | null) {
  if (!mb || !Number.isFinite(mb)) return '--'
  return `${(mb / 1024).toFixed(1)} GB`
}

function formatMemoryPair(used?: number | null, total?: number | null) {
  if (!used || !total) return '--'
  return `${(used / 1024).toFixed(1)} / ${(total / 1024).toFixed(1)} GB`
}

function formatDate(ms: number) {
  return new Date(ms).toLocaleString()
}

function formatDuration(seconds?: number | null) {
  if (!seconds) return '--'
  const h = Math.floor(seconds / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  const s = Math.floor(seconds % 60)
  if (h > 0) return `${h}h ${m}m`
  if (m > 0) return `${m}m ${s}s`
  return `${s}s`
}

function formatTokenCount(value?: number | null) {
  if (value == null) return '--'
  return value.toLocaleString()
}

function formatMs(value?: number | null) {
  if (value == null || !Number.isFinite(value)) return '--'
  if (value < 1000) return `${value.toFixed(0)} ms`
  if (value < 60000) return `${(value / 1000).toFixed(1)} s`
  return `${(value / 60000).toFixed(1)} min`
}

function formatPercent(value?: number | null) {
  if (value == null || !Number.isFinite(value)) return '--'
  return `${(value * 100).toFixed(1)}%`
}

function formatFixed(value?: number | null) {
  if (value == null || !Number.isFinite(value)) return '--'
  return value.toFixed(value >= 10 ? 1 : 2)
}
