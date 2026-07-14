import type {
  DownloadProgress,
  InferenceRequestSummary,
  Instance,
  LogEntry,
  RunningInferenceTask,
  SystemMetrics,
  TelemetrySampleSummary,
  TelemetrySessionSummary,
} from '../../store/types'

export type ServiceStatus = 'healthy' | 'attention' | 'critical'
export type RequestPressureLevel = 'normal' | 'medium' | 'high'
export type SignalTone = 'blue' | 'emerald' | 'amber' | 'red' | 'violet' | 'cyan' | 'slate'
export type LiveThroughputSource = 'active-tasks' | 'llama-metrics' | 'mixed' | 'idle'

export type ThroughputPoint = {
  ts: number
  value: number
}

export type LiveThroughput = {
  value: number
  source: LiveThroughputSource
  activeCount: number
}

export type ActivityFeedItem = {
  id: string
  ts: number
  kind: 'request' | 'download' | 'log'
  severity: 'info' | 'success' | 'warning' | 'critical'
  label: string
  title: string
  detail: string
}

export type ResourceSignal = {
  id: 'cpu' | 'gpu' | 'memory' | 'vram'
  label: string
  value: number
  detail: string
  tone: SignalTone
  sparkline: number[]
}

const criticalLogPattern = /(error|fail|failed|fatal|panic|exception|错误|失败|异常)/i
const warningLogPattern = /(warn|warning|警告|提醒)/i

function validThroughput(value?: number | null): value is number {
  return value != null && Number.isFinite(value) && value >= 0
}

export function buildLiveThroughput(
  tasks: RunningInferenceTask[],
  llamaTps?: number | null,
  requestsProcessing?: number | null,
  taskStreamObserved = false,
): LiveThroughput {
  const activeTasks = tasks.filter(task => !task.completed)
  if (activeTasks.length > 0) {
    const value = activeTasks.reduce((total, task) => {
      if (validThroughput(task.tg_3s)) return total + task.tg_3s
      return validThroughput(task.tg) ? total + task.tg : total
    }, 0)
    return { value, source: 'active-tasks', activeCount: activeTasks.length }
  }

  if (taskStreamObserved) return { value: 0, source: 'idle', activeCount: 0 }

  if ((requestsProcessing || 0) > 0) {
    return {
      value: validThroughput(llamaTps) ? llamaTps : 0,
      source: 'llama-metrics',
      activeCount: Math.max(requestsProcessing || 0, 0),
    }
  }

  return { value: 0, source: 'idle', activeCount: 0 }
}

export function aggregateLiveThroughput(values: LiveThroughput[]): LiveThroughput {
  const value = values.reduce((total, item) => total + item.value, 0)
  const activeCount = values.reduce((total, item) => total + item.activeCount, 0)
  const hasActiveTasks = values.some(item => item.source === 'active-tasks' || item.source === 'mixed')
  const hasLlamaMetrics = values.some(item => item.source === 'llama-metrics' || item.source === 'mixed')
  const source: LiveThroughputSource = hasActiveTasks && hasLlamaMetrics
    ? 'mixed'
    : hasActiveTasks
      ? 'active-tasks'
      : hasLlamaMetrics
        ? 'llama-metrics'
        : 'idle'
  return { value, source, activeCount }
}

export function appendThroughputPoint(
  points: ThroughputPoint[],
  point: ThroughputPoint,
  maxAgeMs = 60 * 60 * 1000,
  maxPoints = 720,
): ThroughputPoint[] {
  if (!Number.isFinite(point.ts) || !validThroughput(point.value)) return points
  const cutoff = point.ts - maxAgeMs
  const next = points.filter(item => item.ts >= cutoff && item.ts !== point.ts)
  next.push(point)
  return next.slice(-maxPoints)
}

export function mergeThroughputPoints(
  historical: ThroughputPoint[],
  live: ThroughputPoint[],
  cutoff = 0,
  maxPoints = 720,
): ThroughputPoint[] {
  const byTimestamp = new Map<number, ThroughputPoint>()
  for (const point of [...historical, ...live]) {
    if (point.ts < cutoff || !Number.isFinite(point.ts) || !validThroughput(point.value)) continue
    byTimestamp.set(point.ts, point)
  }
  const sorted = [...byTimestamp.values()].sort((left, right) => left.ts - right.ts)
  if (sorted.length <= maxPoints) return sorted
  if (maxPoints <= 1) return sorted.slice(-1)
  const lastIndex = sorted.length - 1
  return Array.from(
    { length: maxPoints },
    (_, index) => sorted[Math.round((index / (maxPoints - 1)) * lastIndex)],
  )
}

export function buildChartAxis(values: number[], targetIntervals = 4) {
  const safeValues = values.filter(value => validThroughput(value))
  const dataMax = Math.max(...safeValues, 1)
  const rawStep = dataMax / Math.max(targetIntervals, 1)
  const magnitude = 10 ** Math.floor(Math.log10(rawStep))
  const normalized = rawStep / magnitude
  const niceNormalized = normalized <= 1
    ? 1
    : normalized <= 2
      ? 2
      : normalized <= 2.5
        ? 2.5
        : normalized <= 5
          ? 5
          : 10
  const step = niceNormalized * magnitude
  const max = Math.ceil(dataMax / step) * step
  const intervalCount = Math.max(1, Math.round(max / step))
  const ticks = Array.from({ length: intervalCount + 1 }, (_, index) => index * step)
  return { min: 0, max, step, ticks }
}

export function buildRequestPressure(processing?: number | null, deferred?: number | null) {
  const active = Math.max(processing || 0, 0)
  const queued = Math.max(deferred || 0, 0)
  const total = Math.max(active + queued, 1)
  const percent = Math.round((active / total) * 100)
  const level: RequestPressureLevel = percent >= 80 ? 'high' : percent >= 50 ? 'medium' : 'normal'
  return { active, queued, percent, level }
}

export function buildServiceStatus(options: {
  instances: Instance[]
  downloads: DownloadProgress[]
  logs: LogEntry[]
  telemetryError?: string | null
}): { status: ServiceStatus; alertCount: number } {
  const errorInstances = options.instances.filter(instance => instance.status === 'error')
  const failedDownloads = options.downloads.filter(task => task.status === 'error')
  const criticalLogs = options.logs.filter(entry => criticalLogPattern.test(entry.text))
  const warningLogs = options.logs.filter(entry => warningLogPattern.test(entry.text))
  const alertCount = errorInstances.length + failedDownloads.length + criticalLogs.length + warningLogs.length + (options.telemetryError ? 1 : 0)
  if (options.telemetryError || errorInstances.length > 0 || criticalLogs.length > 0) {
    return { status: 'critical', alertCount }
  }
  if (failedDownloads.length > 0 || warningLogs.length > 0) {
    return { status: 'attention', alertCount }
  }
  return { status: 'healthy', alertCount }
}

export function buildResourceSignals(options: {
  system: SystemMetrics | null
  latest: TelemetrySampleSummary | null
  samples: TelemetrySampleSummary[]
  labels: { memory: string; vram: string; process: string; system: string; gpuUnavailable: string }
}): ResourceSignal[] {
  const cpuValue = options.system?.system_cpu_percent ?? options.system?.cpu_percent ?? options.latest?.system_cpu_percent ?? options.latest?.cpu_percent ?? 0
  const gpuValue = options.system?.gpu_percent ?? options.latest?.gpu_percent ?? 0
  const memoryUsed = options.system?.system_memory_used_mb ?? options.latest?.system_memory_used_mb ?? options.latest?.memory_mb ?? 0
  const memoryTotal = options.system?.system_memory_total_mb ?? options.latest?.system_memory_total_mb ?? 0
  const memoryValue = memoryTotal > 0 ? (memoryUsed / memoryTotal) * 100 : 0
  const vramUsed = options.system?.vram_used_mb ?? options.latest?.vram_used_mb ?? 0
  const vramTotal = options.system?.vram_total_mb ?? options.latest?.vram_total_mb ?? 0
  const vramValue = vramTotal > 0 ? (vramUsed / vramTotal) * 100 : 0

  return [
    {
      id: 'cpu',
      label: 'CPU',
      value: cpuValue,
      detail: `${options.labels.process} ${Math.round(options.system?.cpu_percent ?? options.latest?.cpu_percent ?? 0)}% / ${options.labels.system} ${Math.round(cpuValue)}%`,
      tone: cpuValue >= 85 ? 'amber' : 'blue',
      sparkline: options.samples.map(sample => sample.system_cpu_percent ?? sample.cpu_percent ?? 0),
    },
    {
      id: 'gpu',
      label: 'GPU',
      value: gpuValue,
      detail: options.system?.gpu_vendor || options.labels.gpuUnavailable,
      tone: gpuValue >= 85 ? 'amber' : 'emerald',
      sparkline: options.samples.map(sample => sample.gpu_percent ?? 0),
    },
    {
      id: 'memory',
      label: options.labels.memory,
      value: memoryValue,
      detail: memoryTotal > 0 ? `${(memoryUsed / 1024).toFixed(1)} / ${(memoryTotal / 1024).toFixed(1)} GB` : '--',
      tone: memoryValue >= 90 ? 'red' : 'violet',
      sparkline: options.samples.map(sample => {
        const used = sample.system_memory_used_mb ?? sample.memory_mb ?? 0
        const total = sample.system_memory_total_mb ?? memoryTotal
        return total > 0 ? (used / total) * 100 : 0
      }),
    },
    {
      id: 'vram',
      label: options.labels.vram,
      value: vramValue,
      detail: vramTotal > 0 ? `${(vramUsed / 1024).toFixed(1)} / ${(vramTotal / 1024).toFixed(1)} GB` : '--',
      tone: vramValue >= 90 ? 'red' : 'amber',
      sparkline: options.samples.map(sample => {
        const total = sample.vram_total_mb ?? vramTotal
        return total > 0 ? ((sample.vram_used_mb ?? 0) / total) * 100 : 0
      }),
    },
  ]
}

export function buildActivityFeed(options: {
  requests: InferenceRequestSummary[]
  activeTasks: Array<RunningInferenceTask & { instanceName?: string }>
  downloads: DownloadProgress[]
  logs: Array<LogEntry & { instanceName?: string }>
  labels: { request: string; download: string; log: string; active: string; failed: string; completed: string }
  limit?: number
}): ActivityFeedItem[] {
  const requestItems: ActivityFeedItem[] = [
    ...options.activeTasks.map(task => ({
      id: `active-${task.instanceName || 'instance'}-${task.task_id}`,
      ts: task.updated_at_ms,
      kind: 'request' as const,
      severity: 'info' as const,
      label: options.labels.request,
      title: `#${task.task_id} ${options.labels.active}`,
      detail: task.instanceName || `slot ${task.slot_id}`,
    })),
    ...options.requests.map(request => ({
      id: `request-${request.session_id}-${request.task_id}-${request.completed_at}`,
      ts: request.completed_at,
      kind: 'request' as const,
      severity: request.error_text ? 'warning' as const : 'success' as const,
      label: options.labels.request,
      title: `#${request.task_id}`,
      detail: request.model || request.source,
    })),
  ]

  const downloadItems: ActivityFeedItem[] = options.downloads
    .filter(task => task.status === 'active' || task.status === 'queued' || task.status === 'error' || task.status === 'completed')
    .map(task => ({
      id: `download-${task.id}`,
      ts: Date.now(),
      kind: 'download' as const,
      severity: task.status === 'error' ? 'critical' as const : task.status === 'completed' ? 'success' as const : 'info' as const,
      label: options.labels.download,
      title: task.fileName,
      detail: task.status === 'error' ? options.labels.failed : task.status === 'completed' ? options.labels.completed : options.labels.active,
    }))

  const logItems: ActivityFeedItem[] = options.logs.map(entry => ({
    id: `log-${entry.instanceId}-${entry.timestamp}-${entry.text.slice(0, 20)}`,
    ts: entry.timestamp,
    kind: 'log' as const,
    severity: criticalLogPattern.test(entry.text) ? 'critical' as const : warningLogPattern.test(entry.text) ? 'warning' as const : 'info' as const,
    label: options.labels.log,
    title: entry.instanceName || entry.instanceId,
    detail: entry.text,
  }))

  return [...requestItems, ...downloadItems, ...logItems]
    .sort((left, right) => right.ts - left.ts)
    .slice(0, options.limit ?? 8)
}

export function buildSessionCards(sessions: TelemetrySessionSummary[]) {
  return sessions.map(session => ({
    id: session.id,
    instanceName: session.instance_name,
    modelName: session.model_name,
    startedAt: session.started_at,
    stoppedAt: session.stopped_at,
    workload: session.workload,
    avgTokensPerSec: session.workload === 'inference' ? session.avg_tokens_per_sec : null,
    peakVramMb: session.peak_vram_mb,
    sampleCount: session.sample_count,
    running: !session.stopped_at,
  }))
}
