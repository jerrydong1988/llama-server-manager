export type UsageFilters = { from: number; to: number; keyId: string | null; model: string | null; instanceId: string | null; endpoint: string | null; kind: string | null }
export type Percentiles = { samples: number; p50: number | null; p95: number | null; max: number | null }
export type PerformanceMetric = 'duration' | 'queue' | 'firstOutput'
export type PerformanceSummary = { requests: number; success: number; firstRecordAt: number | null; duration: Percentiles; queue: Percentiles; firstOutput: Percentiles }
export type PerformanceGroup = { id: string; name: string; summary: PerformanceSummary }
export type PerformanceReport = { summary: PerformanceSummary; keys: PerformanceGroup[]; models: PerformanceGroup[]; instances: PerformanceGroup[]; periods: PerformanceGroup[]; periodMs: number; baseline: PerformanceSummary; baselineFrom: number; baselineTo: number; elevated: PerformanceMetric[]; updatedAt: number }
export const milliseconds = (n: number | null | undefined, lang: string) => n == null ? '—' : `${n.toLocaleString(lang)} ms`
