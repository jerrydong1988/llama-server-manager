import type {
  InferenceRequestSummary,
  InstanceConfig,
  RunningInferenceTask,
  SpeculativeTelemetryAnalysis,
} from '../../store/types'

type SpeculativeObservation = Pick<
  RunningInferenceTask,
  'spec_accept_rate' | 'spec_accepted' | 'spec_generated' | 'spec_gen_time_ms'
>

type SessionTaskObservation = RunningInferenceTask & { session_id?: string | null }

type Aggregate = {
  requestCount: number
  acceptanceRate: number | null
  acceptedTokens: number
  generatedTokens: number
  avgGenerationTimeMs: number | null
}

export type SpeculativeDecodingSummary = Aggregate & {
  configured: boolean
  hasData: boolean
  latestAcceptanceRate: number | null
  liveRequestCount: number
  source: 'waiting' | 'live' | 'history' | 'mixed'
}

export function isSpeculativeDecodingConfigured(
  config: Pick<InstanceConfig, 'spec_type'> | null | undefined,
): boolean {
  const specType = config?.spec_type?.trim().toLowerCase()
  return Boolean(specType && specType !== 'none' && specType !== 'off')
}

export function buildSpeculativeDecodingSummary({
  configured = false,
  analysis,
  requests = [],
  activeTasks = [],
  lastCompletedTasks = [],
}: {
  configured?: boolean
  analysis?: SpeculativeTelemetryAnalysis | null
  requests?: InferenceRequestSummary[]
  activeTasks?: RunningInferenceTask[]
  lastCompletedTasks?: SessionTaskObservation[]
}): SpeculativeDecodingSummary {
  const persistedTaskKeys = new Set(requests.map(request => taskKey(request.session_id, request.task_id)))
  const liveObservations = [
    ...activeTasks.filter(task => !task.completed && hasSpeculativeData(task)),
    ...lastCompletedTasks.filter(task => (
      hasSpeculativeData(task) && !persistedTaskKeys.has(taskKey(task.session_id, task.task_id))
    )),
  ]
  const history = analysis ? aggregateFromAnalysis(analysis) : aggregateObservations(requests)
  const live = aggregateObservations(liveObservations)
  const combined = combineAggregates(history, live)
  const hasHistory = history.requestCount > 0
  const hasLive = live.requestCount > 0
  const latestObservation = [...lastCompletedTasks, ...activeTasks].reverse().find(hasSpeculativeData)
    || requests.find(hasSpeculativeData)

  return {
    ...combined,
    configured,
    hasData: hasHistory || hasLive,
    latestAcceptanceRate: rateFromObservation(latestObservation) ?? combined.acceptanceRate,
    liveRequestCount: live.requestCount,
    source: hasLive && hasHistory ? 'mixed' : hasLive ? 'live' : hasHistory ? 'history' : 'waiting',
  }
}

function taskKey(sessionId: string | null | undefined, taskId: number): string {
  return `${sessionId || 'live'}:${taskId}`
}

function aggregateFromAnalysis(analysis: SpeculativeTelemetryAnalysis): Aggregate {
  return {
    requestCount: finiteCount(analysis.request_count),
    acceptanceRate: finiteRate(analysis.acceptance_rate),
    acceptedTokens: finiteCount(analysis.accepted_tokens),
    generatedTokens: finiteCount(analysis.generated_tokens),
    avgGenerationTimeMs: finiteNonNegative(analysis.avg_generation_time_ms),
  }
}

function aggregateObservations(observations: SpeculativeObservation[]): Aggregate {
  const observed = observations.filter(hasSpeculativeData)
  const acceptedTokens = observed.reduce((sum, item) => sum + finiteCount(item.spec_accepted), 0)
  const generatedTokens = observed.reduce((sum, item) => sum + finiteCount(item.spec_generated), 0)
  const rates = observed
    .map(rateFromObservation)
    .filter((value): value is number => value != null)
  const generationTimes = observed
    .map(item => finiteNonNegative(item.spec_gen_time_ms))
    .filter((value): value is number => value != null)

  return {
    requestCount: observed.length,
    acceptanceRate: generatedTokens > 0
      ? finiteRate(acceptedTokens / generatedTokens)
      : average(rates),
    acceptedTokens,
    generatedTokens,
    avgGenerationTimeMs: average(generationTimes),
  }
}

function combineAggregates(history: Aggregate, live: Aggregate): Aggregate {
  const requestCount = history.requestCount + live.requestCount
  const acceptedTokens = history.acceptedTokens + live.acceptedTokens
  const generatedTokens = history.generatedTokens + live.generatedTokens
  const acceptanceRate = generatedTokens > 0
    ? finiteRate(acceptedTokens / generatedTokens)
    : weightedAverage(
        history.acceptanceRate,
        history.requestCount,
        live.acceptanceRate,
        live.requestCount,
      )

  return {
    requestCount,
    acceptanceRate,
    acceptedTokens,
    generatedTokens,
    avgGenerationTimeMs: weightedAverage(
      history.avgGenerationTimeMs,
      history.requestCount,
      live.avgGenerationTimeMs,
      live.requestCount,
    ),
  }
}

function hasSpeculativeData(observation: SpeculativeObservation): boolean {
  return observation.spec_accept_rate != null
    || observation.spec_accepted != null
    || observation.spec_generated != null
    || observation.spec_gen_time_ms != null
}

function rateFromObservation(observation?: SpeculativeObservation | null): number | null {
  if (!observation) return null
  const accepted = finiteCount(observation.spec_accepted)
  const generated = finiteCount(observation.spec_generated)
  if (generated > 0) return finiteRate(accepted / generated)
  return finiteRate(observation.spec_accept_rate)
}

function weightedAverage(
  first: number | null,
  firstWeight: number,
  second: number | null,
  secondWeight: number,
): number | null {
  if (first == null) return second
  if (second == null) return first
  const weight = firstWeight + secondWeight
  return weight > 0 ? ((first * firstWeight) + (second * secondWeight)) / weight : null
}

function average(values: number[]): number | null {
  if (values.length === 0) return null
  return values.reduce((sum, value) => sum + value, 0) / values.length
}

function finiteCount(value: number | null | undefined): number {
  return value == null || !Number.isFinite(value) ? 0 : Math.max(0, Math.round(value))
}

function finiteNonNegative(value: number | null | undefined): number | null {
  return value == null || !Number.isFinite(value) ? null : Math.max(0, value)
}

function finiteRate(value: number | null | undefined): number | null {
  return value == null || !Number.isFinite(value) ? null : Math.max(0, Math.min(1, value))
}
