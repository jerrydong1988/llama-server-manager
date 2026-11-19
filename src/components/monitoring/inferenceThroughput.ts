import type { InferenceRate, InferenceThroughput, MonitoringFrame, TelemetrySampleSummary } from '../../store/types'

export const unavailableRate = (): InferenceRate => ({
  value: null, state: 'unavailable', source: 'none', sampledAt: null, windowMs: null, covered: 0, expected: 1,
})

export function currentInferenceRate(frame: MonitoringFrame | undefined, phase: 'prefill' | 'decode', now: number): InferenceRate {
  if (!frame || frame.workload !== 'inference' || frame.ts > now + 1000 || now - frame.ts > 8000) return unavailableRate()
  return freshInferenceRate(frame.inference?.[phase], now)
}

export function freshInferenceRate(rate: InferenceRate | undefined, now: number): InferenceRate {
  if (!rate) return unavailableRate()
  if (rate.value != null && (!Number.isFinite(rate.value) || rate.value < 0)) return unavailableRate()
  if ((rate.state === 'live' || rate.state === 'partial')
    && (rate.sampledAt == null || rate.sampledAt > now + 1000 || now - rate.sampledAt > 5000)) {
    return { ...rate, value: null, state: 'stale', covered: 0 }
  }
  return rate
}

export function aggregateInferenceRates(rates: InferenceRate[]): InferenceRate {
  if (!rates.length) return unavailableRate()
  const known = rates.filter(rate => rate.value != null)
  if (!known.length) return { ...unavailableRate(), state: rates.some(r => r.state === 'stale') ? 'stale' : rates.some(r => r.state === 'sampling') ? 'sampling' : 'unavailable', expected: rates.length }
  const complete = known.length === rates.length && known.every(r => r.state !== 'partial')
  const times = known.flatMap(r => r.sampledAt == null ? [] : [r.sampledAt])
  const windows = known.flatMap(r => r.windowMs == null ? [] : [r.windowMs])
  const sources = new Set(known.filter(r => r.state !== 'idle').map(r => r.source))
  return {
    value: known.reduce((sum, rate) => sum + rate.value!, 0),
    state: !complete ? 'partial' : known.every(r => r.state === 'idle') ? 'idle' : 'live',
    source: sources.size === 1 ? [...sources][0] : 'mixed',
    sampledAt: times.length ? Math.min(...times) : null, windowMs: windows.length ? Math.max(...windows) : null,
    covered: rates.filter(r => r.value != null && r.state !== 'partial').length, expected: rates.length,
  }
}

type Phase = 'prefill' | 'decode'
export type PhasePoint = Pick<InferenceRate, 'value' | 'state' | 'covered' | 'expected'> & { ts: number; active: boolean }

function phaseReading(inference: InferenceThroughput | null | undefined, phase: Phase, ts: number) {
  const rate = freshInferenceRate(inference?.[phase], ts)
  const active = rate.state === 'live' || (rate.state === 'partial' && (rate.value ?? 0) > 0)
    || !!inference?.tasks?.some(task => task.phase === phase)
  return { rate, active }
}

function withSamplingGaps(points: PhasePoint[]): PhasePoint[] {
  return points.flatMap((point, index) => index > 0 && point.ts - points[index - 1].ts > 8000
    ? [{ ...unavailableRate(), ts: points[index - 1].ts + 1000, active: false }, point] : [point])
}

export function inferenceTrend(
  frames: MonitoringFrame[], samples: TelemetrySampleSummary[], phase: 'prefill' | 'decode', start: number,
): PhasePoint[] {
  const points = new Map<number, PhasePoint>()
  const add = (ts: number, inference?: InferenceThroughput | null) => {
    if (ts < start) return
    const { rate, active } = phaseReading(inference, phase, ts)
    points.set(ts, { ...rate, ts, active })
  }
  for (const sample of samples) add(sample.ts, sample.inference)
  for (const frame of frames) add(frame.ts, frame.inference)
  return withSamplingGaps([...points.values()].sort((a, b) => a.ts - b.ts))
}

export function fleetInferenceTrend(
  byInstance: Record<string, MonitoringFrame[]>, ids: string[], phase: 'prefill' | 'decode', start: number,
): PhasePoint[] {
  const buckets = new Map<number, Map<string, ReturnType<typeof phaseReading>>>()
  for (const id of ids) {
    for (const frame of byInstance[id] || []) {
      if (frame.ts < start) continue
      const ts = Math.max(start, Math.floor(frame.ts / 1000) * 1000)
      const bucket = buckets.get(ts) || new Map<string, ReturnType<typeof phaseReading>>()
      bucket.set(id, phaseReading(frame.inference, phase, frame.ts))
      buckets.set(ts, bucket)
    }
  }
  return withSamplingGaps([...buckets].sort(([a], [b]) => a - b).map(([ts, readings]) => {
    const rate = aggregateInferenceRates(ids.map(id => readings.get(id)?.rate || unavailableRate()))
    return { ...rate, ts, active: [...readings.values()].some(reading => reading.active) }
  }))
}

export type ActivityState = 'prefill' | 'decode' | 'both' | 'idle' | 'unknown'

export function summarizeWallDecode(points: PhasePoint[]) {
  const summary: PhasePoint[] = []
  let count = 0
  let currentWindow: number | undefined
  for (const point of points) {
    const bucket = Math.floor(point.ts / 15_000)
    const last = summary[summary.length - 1]
    if (point.state !== 'live' || point.value == null) {
      summary.push(point)
      currentWindow = undefined
    } else if (bucket === currentWindow && last?.value != null) {
      count += 1
      last.ts += (point.ts - last.ts) / count
      last.value += (point.value - last.value) / count
    } else {
      currentWindow = bucket
      count = 1
      summary.push({ ...point })
    }
  }
  return summary
}

export function activityBands(prefill: PhasePoint[], decode: PhasePoint[], start: number, end: number) {
  const bands: { from: number; to: number; state: ActivityState; partial: boolean }[] = []
  const add = (from: number, to: number, state: ActivityState, partial = false) => {
    if (to <= from) return
    const last = bands[bands.length - 1]
    if (last?.to === from && last.state === state && last.partial === partial) last.to = to
    else bands.push({ from, to, state, partial })
  }
  let cursor = start
  for (const [index, input] of prefill.entries()) {
    const output = decode[index]
    if (input.ts > end || !output || output.ts !== input.ts) continue
    add(cursor, input.ts, 'unknown')
    const state = input.active && output.active ? 'both' : input.active ? 'prefill' : output.active ? 'decode'
      : input.state === 'idle' && output.state === 'idle' ? 'idle' : 'unknown'
    const next = prefill[index + 1]?.ts
    const to = Math.min(end, next ?? input.ts + 1000)
    add(Math.max(start, input.ts), to, state, input.state === 'partial' || output.state === 'partial')
    cursor = to
  }
  add(cursor, end, 'unknown')
  return bands
}

export function formatInferenceRate(rate: InferenceRate): string {
  if (rate.value == null || !Number.isFinite(rate.value)) return '—'
  return `${rate.value.toLocaleString(undefined, { maximumFractionDigits: 1 })} tok/s`
}
