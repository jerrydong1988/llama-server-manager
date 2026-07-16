import type {
  ModelWorkload,
  VectorTelemetryAnalysis,
} from '../../store/types'
import { getVectorPerformanceLabels } from '../../i18n/pageLabels'

export type PerformanceLocale = 'zh' | 'zh-CN' | 'en' | 'en-US'
export type VectorMetricKey = 'input' | 'items' | 'p95'
export type VectorSourceKind = 'full' | 'log-only' | 'proxy-only' | 'none'
export type VectorItemName = 'vector' | 'document'
export type ActiveTaskColumn = 'slot' | 'elapsed' | 'workload' | 'generated' | 'speed' | 'speculative'

export interface VectorKpi {
  key: VectorMetricKey
  label: string
  value: string
  available: boolean
}

export interface VectorSourceState {
  kind: VectorSourceKind
  summary: string
  log: string
  proxy: string
}

export interface VectorTrendPoint {
  timestamp: number
  value: number | null
}

export interface VectorBaseline {
  averageInputTokensPerSecond: number | null
  averageItemsPerSecond: number | null
  taskDurationP95Ms: number | null
  sessionCount: number
}

export interface VectorComparisonRow {
  key: VectorMetricKey
  label: string
  current: string
  baseline: string
  favorable: boolean | null
}

export type PerformanceMode =
  | {
      kind: 'inference'
      workload: 'inference'
    }
  | {
      kind: 'vector'
      workload: 'embedding' | 'reranker'
      itemName: VectorItemName
      inputThroughput: number | null
      itemThroughput: number | null
      taskP95Ms: number | null
      completedItems: number | null
      proxyRequestCount: number | null
      proxyFailureRate: number | null
      source: VectorSourceState
      analysis: VectorTelemetryAnalysis | null
    }

type WorkloadSession = { workload?: ModelWorkload | null }
type AnalysisEnvelope = { vector_analysis?: VectorTelemetryAnalysis | null }

const isAvailable = (value: number | null): value is number =>
  value !== null && Number.isFinite(value)

function formatRate(value: number | null, suffix: string): string {
  return isAvailable(value) ? `${value.toFixed(1)} ${suffix}` : '--'
}

function formatMilliseconds(value: number | null): string {
  return isAvailable(value) ? `${Math.round(value)} ms` : '--'
}

export function workloadLabel(workload: ModelWorkload, locale: PerformanceLocale): string {
  if (workload === 'embedding') return 'Embedding'
  if (workload === 'reranker') return 'Reranker'
  return getVectorPerformanceLabels(locale).generation
}

export function buildVectorSourceState(
  analysis: VectorTelemetryAnalysis,
  locale: PerformanceLocale,
): VectorSourceState {
  const labels = getVectorPerformanceLabels(locale)
  const kind: VectorSourceKind = analysis.logAvailable
    ? analysis.proxyAvailable ? 'full' : 'log-only'
    : analysis.proxyAvailable ? 'proxy-only' : 'none'
  return {
    kind,
    summary: labels.sourceSummaries[kind],
    log: analysis.logAvailable ? labels.logAvailable : labels.logUnavailable,
    proxy: analysis.proxyAvailable ? labels.proxyAvailable : labels.proxyUnavailable,
  }
}

export function buildPerformanceMode(
  session: WorkloadSession | null,
  analysis: AnalysisEnvelope | null,
  locale: PerformanceLocale = 'zh-CN',
): PerformanceMode {
  const workload = session?.workload ?? 'inference'
  if (workload === 'inference') return { kind: 'inference', workload }

  const vector = analysis?.vector_analysis?.workload === workload
    ? analysis.vector_analysis
    : null
  const emptyAnalysis: VectorTelemetryAnalysis = {
    workload,
    logAvailable: false,
    proxyAvailable: false,
    completedItems: null,
    inputTokens: null,
    averageInputTokensPerSecond: null,
    averageItemsPerSecond: null,
    taskDurationP50Ms: null,
    taskDurationP95Ms: null,
    proxyRequestCount: null,
    proxyItemCount: null,
    proxyDurationP50Ms: null,
    proxyDurationP95Ms: null,
    proxySuccessRate: null,
    proxyFailureRate: null,
    trend: [],
  }
  const sourceAnalysis = vector ?? emptyAnalysis
  return {
    kind: 'vector',
    workload,
    itemName: workload === 'reranker' ? 'document' : 'vector',
    inputThroughput: vector?.averageInputTokensPerSecond ?? null,
    itemThroughput: vector?.averageItemsPerSecond ?? null,
    taskP95Ms: vector?.taskDurationP95Ms ?? null,
    completedItems: vector?.completedItems ?? null,
    proxyRequestCount: vector?.proxyRequestCount ?? null,
    proxyFailureRate: vector?.proxyFailureRate ?? null,
    source: buildVectorSourceState(sourceAnalysis, locale),
    analysis: vector,
  }
}

export function buildVectorKpis(
  analysis: VectorTelemetryAnalysis,
  locale: PerformanceLocale,
): VectorKpi[] {
  const labels = getVectorPerformanceLabels(locale)
  const itemLabel = analysis.workload === 'reranker'
    ? labels.documentThroughput
    : labels.vectorThroughput
  return [
    {
      key: 'input',
      label: labels.inputThroughput,
      value: formatRate(analysis.averageInputTokensPerSecond, 'tok/s'),
      available: isAvailable(analysis.averageInputTokensPerSecond),
    },
    {
      key: 'items',
      label: itemLabel,
      value: formatRate(analysis.averageItemsPerSecond, labels.itemsPerSecond),
      available: isAvailable(analysis.averageItemsPerSecond),
    },
    {
      key: 'p95',
      label: labels.taskP95,
      value: formatMilliseconds(analysis.taskDurationP95Ms),
      available: isAvailable(analysis.taskDurationP95Ms),
    },
  ]
}

export function buildVectorTrendSeries(
  analysis: VectorTelemetryAnalysis,
  metric: 'input' | 'items',
): VectorTrendPoint[] {
  if (!analysis.logAvailable) return []
  return analysis.trend.map(bucket => ({
    timestamp: bucket.timestamp,
    value: metric === 'input' ? bucket.inputTokensPerSecond : bucket.itemsPerSecond,
  }))
}

export function buildVectorComparisonRows(
  current: VectorTelemetryAnalysis,
  baseline: VectorBaseline,
  locale: PerformanceLocale,
): VectorComparisonRow[] {
  const labels = getVectorPerformanceLabels(locale)
  const values = [
    {
      key: 'input' as const,
      label: labels.inputThroughput,
      current: current.averageInputTokensPerSecond,
      baseline: baseline.averageInputTokensPerSecond,
      format: (value: number | null) => formatRate(value, 'tok/s'),
      lowerIsBetter: false,
    },
    {
      key: 'items' as const,
      label: current.workload === 'reranker'
        ? labels.documentThroughput
        : labels.vectorThroughput,
      current: current.averageItemsPerSecond,
      baseline: baseline.averageItemsPerSecond,
      format: (value: number | null) => formatRate(value, labels.itemsPerSecond),
      lowerIsBetter: false,
    },
    {
      key: 'p95' as const,
      label: labels.taskP95,
      current: current.taskDurationP95Ms,
      baseline: baseline.taskDurationP95Ms,
      format: formatMilliseconds,
      lowerIsBetter: true,
    },
  ]
  return values.map(value => ({
    key: value.key,
    label: value.label,
    current: value.format(value.current),
    baseline: value.format(value.baseline),
    favorable: isAvailable(value.current) && isAvailable(value.baseline)
      ? value.lowerIsBetter
        ? value.current <= value.baseline
        : value.current >= value.baseline
      : null,
  }))
}

export function getActiveTaskColumns(workload: ModelWorkload): ActiveTaskColumn[] {
  return workload === 'inference'
    ? ['slot', 'elapsed', 'generated', 'speed', 'speculative']
    : ['slot', 'elapsed', 'workload']
}
