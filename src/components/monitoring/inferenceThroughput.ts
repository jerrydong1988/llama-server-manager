import type { InferenceRate, MonitoringFrame, TelemetrySampleSummary } from '../../store/types'
import type { MonitoringTrendPoint } from './monitoringViewModel'

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

function withSamplingGaps(points: MonitoringTrendPoint[]): MonitoringTrendPoint[] {
  return points.flatMap((point, index) => index > 0 && point.ts - points[index - 1].ts > 8000
    ? [{ ts: points[index - 1].ts + 1, value: null }, point] : [point])
}

export function inferenceTrend(
  frames: MonitoringFrame[], samples: TelemetrySampleSummary[], phase: 'prefill' | 'decode', start: number,
): MonitoringTrendPoint[] {
  const points = new Map<number, MonitoringTrendPoint>()
  const add = (ts: number, rate?: InferenceRate) => {
    if (ts < start) return
    const reading = freshInferenceRate(rate, ts)
    points.set(ts, { ts, value: reading.state === 'live' || reading.state === 'idle' ? reading.value : null })
  }
  for (const sample of samples) add(sample.ts, sample.inference?.[phase])
  for (const frame of frames) add(frame.ts, frame.inference?.[phase])
  return withSamplingGaps([...points.values()].sort((a, b) => a.ts - b.ts))
}

export function fleetInferenceTrend(
  byInstance: Record<string, MonitoringFrame[]>, ids: string[], phase: 'prefill' | 'decode', start: number,
): MonitoringTrendPoint[] {
  const buckets = new Map<number, Map<string, InferenceRate>>()
  for (const id of ids) {
    for (const frame of byInstance[id] || []) {
      if (frame.ts < start) continue
      const ts = Math.floor(frame.ts / 3000) * 3000
      const bucket = buckets.get(ts) || new Map<string, InferenceRate>()
      bucket.set(id, freshInferenceRate(frame.inference?.[phase], frame.ts))
      buckets.set(ts, bucket)
    }
  }
  return withSamplingGaps([...buckets].sort(([a], [b]) => a - b).map(([ts, readings]) => {
    const rate = aggregateInferenceRates(ids.map(id => readings.get(id) || unavailableRate()))
    return { ts, value: rate.state === 'live' || rate.state === 'idle' ? rate.value : null }
  }))
}

export function formatInferenceRate(rate: InferenceRate): string {
  if (rate.value == null || !Number.isFinite(rate.value)) return '—'
  return `${rate.value.toLocaleString(undefined, { maximumFractionDigits: 1 })} tok/s`
}
