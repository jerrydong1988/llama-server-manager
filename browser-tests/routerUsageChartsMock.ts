import type { UsageGroup, UsageSummary } from '../src/components/routerUsage/chartData'

export const emptyChartSummary = (): UsageSummary => ({ requests: 0, forwarded: 0, success: 0, failed: 0, rejected: 0, cancelled: 0, incomplete: 0,
  complete: 0, partial: 0, unknown: 0, notApplicable: 0, input: 0, output: 0, cached: 0, cacheInput: 0,
  inputKnown: 0, outputKnown: 0, cacheKnown: 0, items: 0, durationMs: 0, queueMs: 0, firstOutputMs: 0, firstOutputCount: 0, lastUsed: 0 })

export function routerUsageChartsMock(query: Record<string, unknown>) {
  const day = 86_400_000
  const now = Date.now()
  const today = Math.floor(now / day) * day
  const names = ['WorkBuddy', 'DeepSeek Harness', 'OpenCode', 'Research workspace', 'Embedding client', 'Batch runner', 'CLI', 'A very long client name for verifying that the usage ranking stays within the page']
  const rows = Array.from({ length: 365 * 8 }, (_, n) => {
    const age = Math.floor(n / 8)
    const key = n % 8
    const requests = 20 + ((age * 7 + key * 13) % 57)
    const count = query.kind === 'count'
    const missing = age === 1
    const zero = age === 2
    return { date: today - age * day, keyId: `chart-key-${key}`, keyName: names[key], model: key % 2 ? 'Qwen3.5' : 'localmodel', instanceId: key % 2 ? 'instance-b' : 'instance-a',
      endpoint: count ? '/v1/messages/count_tokens' : '/v1/chat/completions', summary: { ...emptyChartSummary(), requests, forwarded: requests - 2,
        success: requests - 7, failed: 2, rejected: 2, cancelled: 2, incomplete: 1, complete: count || missing ? 0 : requests - 5,
        partial: count || missing ? 0 : 2, unknown: count ? 0 : missing ? requests - 2 : 1, notApplicable: count ? requests : 2,
        input: count || missing || zero ? 0 : requests * (800 + key * 120), output: count || missing || zero ? 0 : requests * 110,
        inputKnown: count || missing ? 0 : requests - 3, outputKnown: count || missing ? 0 : requests - 5,
        lastUsed: Math.min(now, today - age * day + 12 * 3600_000) } }
  }).filter(r => r.date !== today - 3 * day && r.date >= Number(query.from) && r.date < Number(query.to)
    && (!query.keyId || query.keyId === r.keyId) && (!query.model || query.model === r.model)
    && (!query.instanceId || query.instanceId === r.instanceId) && (!query.endpoint || query.endpoint === r.endpoint)
    && (!query.kind || query.kind === 'generation' || query.kind === 'count'))
  const sum = (records: typeof rows) => {
    const s = emptyChartSummary()
    for (const r of records) for (const field of Object.keys(s) as Array<keyof UsageSummary>) {
      s[field] = field === 'lastUsed' ? Math.max(s[field], r.summary[field]) : s[field] + r.summary[field]
    }
    return s
  }
  const group = (field: 'keyId' | 'model' | 'instanceId' | 'endpoint' | 'date'): UsageGroup[] => Array.from(new Set(rows.map(r => r[field]))).map(id => {
    const records = rows.filter(r => r[field] === id)
    return { id: String(id), name: field === 'keyId' ? records[0].keyName : String(id), summary: sum(records) }
  })
  return { summary: sum(rows), keys: group('keyId'), models: group('model'), instances: group('instanceId'), endpoints: group('endpoint'), days: group('date'),
    recent: [], recentTruncated: false, droppedRecords: 0, writeErrors: 0, lastWriteError: null, updatedAt: now, detailDays: 90, summaryDays: 365,
    storage: { pendingRecords: 0, lastCommitAt: now, writeDelayMs: 0, interruptedSessions: 0, lastInterruptionAt: null } }
}
