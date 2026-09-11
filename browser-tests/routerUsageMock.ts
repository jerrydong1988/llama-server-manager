const day = 86_400_000
const empty = () => ({ requests: 0, forwarded: 0, success: 0, failed: 0, rejected: 0, cancelled: 0, incomplete: 0,
  complete: 0, partial: 0, unknown: 0, notApplicable: 0, input: 0, output: 0, cached: 0, cacheInput: 0,
  inputKnown: 0, outputKnown: 0, cacheKnown: 0, items: 0, durationMs: 0, queueMs: 0, firstOutputMs: 0, firstOutputCount: 0, lastUsed: 0 })
let cleared = false
export function clearUsageMock() { cleared = true }
export function routerUsageMock(query: Record<string, unknown>) {
  if (new URLSearchParams(window.location.search).has('usageError')) throw new Error('Usage storage unavailable')
  const now = Date.now()
  const s = { ...empty(), requests: 2, forwarded: 1, success: 1, rejected: 1, complete: 1, notApplicable: 1,
    input: 10000, output: 1000, cached: 8000, cacheInput: 10000, inputKnown: 1, outputKnown: 1, cacheKnown: 1, lastUsed: now }
  const p = { ...empty(), requests: 2, forwarded: 2, cancelled: 1, incomplete: 1, partial: 1, unknown: 1, input: 23, inputKnown: 1, lastUsed: now - 1000 }
  const groups = [
    { id: 'key-a', name: 'WorkBuddy', summary: s },
    { id: 'key-b', name: '=Imported Client', summary: p },
  ].filter(g => !cleared && (!query.keyId || query.keyId === g.id) && !query.kind && (!query.model || query.model === 'public-model') && (!query.instanceId || query.instanceId === 'instance-one'))
  const summary = empty()
  for (const group of groups) for (const field of Object.keys(summary) as Array<keyof typeof summary>) {
    summary[field] = field === 'lastUsed' ? Math.max(summary[field], group.summary[field]) : summary[field] + group.summary[field]
  }
  const slice = (id: string) => groups.length ? [{ id, name: id, summary }] : []
  return { summary, keys: groups, models: slice('public-model'), instances: slice('instance-one'), endpoints: slice('/v1/chat/completions'),
    days: slice(String(Math.floor(now / day) * day)), recent: groups.map((g, i) => ({ requestId: `usage-${g.id}`, keyId: g.id, keyName: g.name,
      model: 'public-model', instanceId: 'instance-one', endpoint: '/v1/chat/completions', kind: 'generation', startedAt: now - 1000, completedAt: now,
      httpStatus: 200, forwarded: true, outcome: i ? 'cancelled' : 'success', quality: i ? 'partial' : 'complete', source: 'upstream_usage',
      tokens: { input: g.summary.input, output: i ? null : 1000, cached: i ? null : 8000, cacheWrite: null, reasoning: null },
      durationMs: 1000, queueMs: 2, firstOutputMs: 100, finishReason: null, items: null })),
    recentTruncated: false, droppedRecords: 1, writeErrors: 0, lastWriteError: null, updatedAt: now, detailDays: 90, summaryDays: 365 }
}
