export type UsageSummary = {
  requests: number; forwarded: number; success: number; failed: number; rejected: number; cancelled: number; incomplete: number
  complete: number; partial: number; unknown: number; notApplicable: number
  input: number; output: number; cached: number; cacheInput: number; cacheKnown: number; inputKnown: number; outputKnown: number
  items: number; durationMs: number; queueMs: number; firstOutputMs: number; firstOutputCount: number; lastUsed: number
}
export type UsageGroup = { id: string; name: string; summary: UsageSummary }
export type ChartMetric = 'requests' | 'tokens'
export const DAY = 86_400_000
export const utcDate = (time: number) => new Date(time).toISOString().slice(0, 10)
export const reportedTokens = (s: UsageSummary) => s.inputKnown || s.outputKnown ? s.input + s.output : null

export function buildUsageTimeline(days: UsageGroup[], from: number, to: number) {
  const count = Math.round((to - from) / DAY)
  if (!Number.isFinite(count) || count < 1 || count > 365) return { bucketDays: 1, buckets: [] }
  const bucketDays = count <= 31 ? 1 : count <= 180 ? 7 : 30
  // Buckets start at the selected UTC date, with a possibly shorter final bucket.
  const buckets = Array.from({ length: Math.ceil(count / bucketDays) }, (_, i) => ({
    from: from + i * bucketDays * DAY, to: Math.min(to, from + (i + 1) * bucketDays * DAY),
    requests: 0, input: 0, output: 0, inputKnown: 0, outputKnown: 0, complete: 0, partial: 0, unknown: 0,
  }))
  for (const day of days) {
    const date = Number(day.id)
    if (!Number.isFinite(date) || date < from || date >= to) continue
    const bucket = buckets[Math.floor((date - from) / (bucketDays * DAY))]
    for (const field of ['requests', 'input', 'output', 'inputKnown', 'outputKnown', 'complete', 'partial', 'unknown'] as const) {
      bucket[field] += day.summary[field]
    }
  }
  return { bucketDays, buckets }
}

export function rankUsage(groups: UsageGroup[], metric: ChartMetric) {
  return groups.map(group => ({ ...group, value: metric === 'requests' ? group.summary.requests : reportedTokens(group.summary) }))
    .sort((a, b) => (b.value ?? -1) - (a.value ?? -1) || a.id.localeCompare(b.id))
}

export function usageAxisMax(peak: number) {
  if (peak <= 4) return 4
  const magnitude = 10 ** Math.floor(Math.log10(peak / 4))
  return Math.ceil(peak / 4 / magnitude) * magnitude * 4
}
