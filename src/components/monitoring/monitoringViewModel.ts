import type {
  DownloadProgress,
  InferenceRequestSummary,
  Instance,
  LogEntry,
  ModelWorkload,
  MonitoringFrame,
  RunningInferenceTask,
  SystemMetrics,
  TelemetrySampleSummary,
  TelemetrySessionSummary,
} from '../../store/types'

export type ServiceStatus = 'healthy' | 'attention' | 'critical'
export type RequestPressureLevel = 'unknown' | 'normal' | 'medium' | 'high'
export type SignalTone = 'blue' | 'emerald' | 'amber' | 'red' | 'violet' | 'cyan' | 'slate'
export type LiveThroughputSource = 'active-tasks' | 'llama-metrics' | 'mixed' | 'idle'

export type ThroughputPoint = {
  ts: number
  value: number
}

export type MonitoringTrendPoint = {
  ts: number
  value: number | null
}

export type FleetThroughputSeries = {
  mode: 'inference' | 'vector' | 'mixed' | 'empty'
  unit: 'tok/s' | 'input tok/s'
  points: MonitoringTrendPoint[]
  current: number | null
  vectorItemsPerSecond: number
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

export function buildTelemetryThroughputPoints(
  samples: Array<Pick<TelemetrySampleSummary, 'ts' | 'tokens_per_sec' | 'requests_processing'>>,
): ThroughputPoint[] {
  return samples.map(sample => ({
    ts: sample.ts,
    value: sample.requests_processing === 0
      ? 0
      : validThroughput(sample.tokens_per_sec) ? sample.tokens_per_sec : 0,
  }))
}

export function mergeThroughputPoints(
  historical: ThroughputPoint[],
  live: ThroughputPoint[],
  cutoff = 0,
  maxPoints = 720,
): ThroughputPoint[] {
  const validLive = live.filter(point => (
    point.ts >= cutoff && Number.isFinite(point.ts) && validThroughput(point.value)
  ))
  const liveFrom = validLive.reduce(
    (earliest, point) => Math.min(earliest, point.ts),
    Number.POSITIVE_INFINITY,
  )
  const byTimestamp = new Map<number, ThroughputPoint>()
  for (const point of historical) {
    if (point.ts < cutoff || !Number.isFinite(point.ts) || !validThroughput(point.value)) continue
    // Once the live stream starts, persisted samples must not overwrite its timeline.
    if (point.ts >= liveFrom) continue
    byTimestamp.set(point.ts, point)
  }
  for (const point of validLive) {
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

export function buildRequestPressure(
  processing?: number | null,
  deferred?: number | null,
  capacity?: number | null,
) {
  const active = Math.max(processing || 0, 0)
  const queued = Math.max(deferred || 0, 0)
  const normalizedCapacity = Math.max(capacity || 0, 0)
  const capacityKnown = normalizedCapacity > 0
  const utilization = capacityKnown ? Math.min((active / normalizedCapacity) * 100, 100) : 0
  const percent = queued > 0 ? 100 : capacityKnown ? Math.round(utilization) : null
  const level: RequestPressureLevel = queued > 0 || (percent != null && percent >= 90)
    ? 'high'
    : percent == null
      ? 'unknown'
      : percent >= 70
      ? 'medium'
      : 'normal'
  return { active, queued, capacity: capacityKnown ? normalizedCapacity : null, percent, level }
}

export function formatRequestPressureDetail(
  pressure: ReturnType<typeof buildRequestPressure>,
  labels: { processing: string; capacity: string; queued: string },
) {
  const parts = [`${labels.processing} ${pressure.active}`]
  if (pressure.capacity != null) parts.push(`${labels.capacity} ${pressure.capacity}`)
  parts.push(`${labels.queued} ${pressure.queued}`)
  return parts.join(' · ')
}

export function monitoringFramePoints(
  frames: MonitoringFrame[],
  workload: ModelWorkload,
): MonitoringTrendPoint[] {
  return frames
    .filter(frame => frame.workload === workload)
    .map(frame => ({
      ts: frame.ts,
      value: frame.state === 'unavailable' ? null : frame.throughput,
    }))
}

export function downsampleMonitoringPoints(
  points: MonitoringTrendPoint[],
  maxPoints = 240,
): MonitoringTrendPoint[] {
  if (points.length <= maxPoints) return points
  if (maxPoints <= 1) return points.slice(-1)
  const lastIndex = points.length - 1
  return Array.from(
    { length: maxPoints },
    (_, index) => points[Math.round((index / (maxPoints - 1)) * lastIndex)],
  )
}

export function buildFleetThroughputSeries(
  framesByInstance: Record<string, MonitoringFrame[]>,
  currentByInstance: Record<string, MonitoringFrame>,
  instanceIds: string[],
  startTs = Number.NEGATIVE_INFINITY,
): FleetThroughputSeries {
  const currentFrames = instanceIds
    .map(instanceId => currentByInstance[instanceId])
    .filter((frame): frame is MonitoringFrame => Boolean(frame))
  const representativeFrames = instanceIds
    .map(instanceId => {
      const frames = framesByInstance[instanceId]
      return currentByInstance[instanceId] || frames?.[frames.length - 1]
    })
    .filter((frame): frame is MonitoringFrame => Boolean(frame))
  const hasInference = representativeFrames.some(frame => frame.workload === 'inference')
  const hasVector = representativeFrames.some(frame => frame.workload !== 'inference')
  const mode = hasInference && hasVector
    ? 'mixed'
    : hasInference
      ? 'inference'
      : hasVector
        ? 'vector'
        : 'empty'
  const primaryIsInference = hasInference || !hasVector
  const included = (frame: MonitoringFrame) => primaryIsInference
    ? frame.workload === 'inference'
    : frame.workload !== 'inference'
  const representedIds = new Set(representativeFrames.map(frame => frame.instanceId))
  const expectedInstanceIds = new Set([
    ...representativeFrames.filter(included).map(frame => frame.instanceId),
    ...instanceIds.filter(instanceId => !representedIds.has(instanceId)),
  ])
  const singleInstanceId = expectedInstanceIds.size === 1
    ? expectedInstanceIds.values().next().value as string
    : null
  const singleInstancePoints = singleInstanceId
    ? (framesByInstance[singleInstanceId] || [])
      .filter(frame => frame.ts >= startTs && included(frame))
      .map(frame => ({
        ts: frame.ts,
        value: frame.state !== 'unavailable' && frame.throughput != null && Number.isFinite(frame.throughput)
          ? Math.max(frame.throughput, 0)
          : null,
      }))
    : null
  const byTimestamp = new Map<number, { ts: number; values: Map<string, number> }>()
  for (const instanceId of instanceIds) {
    if (singleInstancePoints) break
    const frames = framesByInstance[instanceId] || []
    let low = 0
    let high = frames.length
    while (low < high) {
      const middle = Math.floor((low + high) / 2)
      if (frames[middle].ts < startTs) low = middle + 1
      else high = middle
    }
    for (let index = low; index < frames.length; index += 1) {
      const frame = frames[index]
      if (!included(frame)) continue
      const bucketTs = Math.floor(frame.ts / 5000) * 5000
      const bucket = byTimestamp.get(bucketTs) || { ts: frame.ts, values: new Map<string, number>() }
      if (frame.state !== 'unavailable' && frame.throughput != null && Number.isFinite(frame.throughput)) {
        bucket.values.set(instanceId, Math.max(frame.throughput, 0))
      }
      bucket.ts = Math.max(bucket.ts, frame.ts)
      byTimestamp.set(bucketTs, bucket)
    }
  }
  const points = singleInstancePoints || [...byTimestamp.values()]
    .filter(bucket => bucket.values.size === expectedInstanceIds.size)
    .map(bucket => ({
      ts: bucket.ts,
      value: [...bucket.values.values()].reduce((total, value) => total + value, 0),
    }))
    .sort((left, right) => left.ts - right.ts)
  const currentValues = currentFrames.filter(included)
  const current = currentValues.length === expectedInstanceIds.size
    && currentValues.every(frame => frame.state !== 'unavailable' && frame.throughput != null)
    ? currentValues.reduce((total, frame) => total + Math.max(frame.throughput || 0, 0), 0)
    : null
  const vectorItemsPerSecond = currentFrames
    .filter(frame => frame.workload !== 'inference')
    .reduce((total, frame) => total + Math.max(frame.itemsPerSecond || 0, 0), 0)
  return {
    mode,
    unit: primaryIsInference ? 'tok/s' : 'input tok/s',
    points,
    current,
    vectorItemsPerSecond,
  }
}

export function buildServiceStatus(options: {
  instances: Instance[]
  downloads: DownloadProgress[]
  logs: LogEntry[]
  telemetryError?: string | null
  droppedWrites?: number
  lastWriteError?: string | null
  now?: number
  logWindowMs?: number
}): { status: ServiceStatus; alertCount: number } {
  const errorInstances = options.instances.filter(instance => instance.status === 'error')
  const unhealthyInstances = options.instances.filter(instance => (
    instance.status === 'running' && instance.healthCheck === 'fail'
  ))
  const failedDownloads = options.downloads.filter(task => task.status === 'error')
  const logCutoff = (options.now ?? Date.now()) - (options.logWindowMs ?? 5 * 60 * 1000)
  const recentLogs = options.logs.filter(entry => entry.timestamp >= logCutoff)
  const criticalLogs = recentLogs.filter(entry => entry.level === 'error')
  const warningLogs = recentLogs.filter(entry => entry.level === 'warning')
  const telemetryUnhealthy = Boolean(options.telemetryError || options.lastWriteError || (options.droppedWrites || 0) > 0)
  const alertCount = errorInstances.length + unhealthyInstances.length + failedDownloads.length + criticalLogs.length + warningLogs.length + (telemetryUnhealthy ? 1 : 0)
  if (telemetryUnhealthy || errorInstances.length > 0 || unhealthyInstances.length > 0 || criticalLogs.length > 0) {
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
      detail: options.system?.gpu_name
        || options.latest?.gpu_name
        || options.system?.gpu_vendor
        || options.latest?.gpu_vendor
        || options.labels.gpuUnavailable,
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
      ts: task.completedAt ?? task.updatedAt ?? task.createdAt ?? 0,
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
    severity: entry.level === 'error' ? 'critical' as const : entry.level === 'warning' ? 'warning' as const : 'info' as const,
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
