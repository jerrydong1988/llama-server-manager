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
  const counting = query.kind === 'count'
  const endpoint = counting ? '/v1/messages/count_tokens' : '/v1/chat/completions'
  const groups = (counting ? [
    { id: 'count-only-key', name: 'Preflight client', summary: { ...empty(), requests: 3, forwarded: 3, success: 3, notApplicable: 3, lastUsed: now } },
  ] : [
    { id: 'key-a', name: 'WorkBuddy', summary: s },
    { id: 'key-b', name: '=Imported Client', summary: p },
  ]).filter(g => !cleared && now >= Number(query.from) && now < Number(query.to)
    && (!query.keyId || query.keyId === g.id) && (!query.kind || query.kind === 'generation' || counting)
    && (!query.endpoint || query.endpoint === endpoint) && (!query.model || query.model === 'public-model') && (!query.instanceId || query.instanceId === 'instance-one'))
  const summary = empty()
  for (const group of groups) for (const field of Object.keys(summary) as Array<keyof typeof summary>) {
    summary[field] = field === 'lastUsed' ? Math.max(summary[field], group.summary[field]) : summary[field] + group.summary[field]
  }
  const slice = (id: string) => groups.length ? [{ id, name: id, summary }] : []
  return { summary, keys: groups, models: slice('public-model'), instances: slice('instance-one'), endpoints: slice(endpoint),
    days: slice(String(Math.floor(now / day) * day)), recent: groups.map(g => ({ requestId: `usage-${g.id}`, keyId: g.id, keyName: g.name,
      model: 'public-model', instanceId: 'instance-one', endpoint, kind: counting ? 'count' : 'generation', startedAt: now - 1000, completedAt: now,
      httpStatus: 200, forwarded: true, outcome: g.id === 'key-b' ? 'cancelled' : 'success', quality: counting ? 'not_applicable' : g.id === 'key-b' ? 'partial' : 'complete', source: counting ? 'none' : 'upstream_usage',
      tokens: { input: counting ? null : g.summary.input, output: !counting && g.id === 'key-a' ? 1000 : null, cached: !counting && g.id === 'key-a' ? 8000 : null, cacheWrite: null, reasoning: null },
      durationMs: 1000, queueMs: 2, firstOutputMs: 100, finishReason: null, items: null })),
    recentTruncated: false, droppedRecords: 1, writeErrors: 0, lastWriteError: null, updatedAt: now, detailDays: 90, summaryDays: 365,
    storage: { pendingRecords: 2, lastCommitAt: now - 500, writeDelayMs: 120, interruptedSessions: 1, lastInterruptionAt: now - day } }
}

export function routerUsageRequestsMock(query: Record<string, unknown>) {
  const report = routerUsageMock(query)
  let records = report.recent.map(r => ({ ...r, failure: r.outcome === 'cancelled' ? { stage: 'delivery', code: 'client_cancelled', reason: 'Request was dropped.' } : null,
    responseRequestId: `req-${r.requestId}`, upstreamRequestId: null as string | null, contextBudget: null as object | null }))
  if (new URLSearchParams(window.location.search).has('usageDiagnostics') && records.length) {
    records = Array.from({ length: 125 }, (_, i) => ({ ...records[0], requestId: `usage-page-${String(i).padStart(3, '0')}`, responseRequestId: `req-page-${i}`,
      completedAt: report.updatedAt - i * 1000, httpStatus: 400, forwarded: false, outcome: 'rejected', quality: 'not_applicable', source: 'none',
      tokens: { input: null, output: null, cached: null, cacheWrite: null, reasoning: null },
      failure: { stage: 'preflight', code: 'context_length_exceeded', reason: 'Input and requested output exceed the route context window.' },
      contextBudget: { inputTokens: 91675, requestedOutputTokens: 65536, contextWindow: 131072, excessTokens: 26139, inputSource: 'exact' } }))
  }
  records = records.filter(r => r.completedAt >= Number(query.from) && r.completedAt < Number(query.to)
    && (!query.outcome || r.outcome === query.outcome) && (!query.failureCode || r.failure?.code === query.failureCode)
    && (!query.requestId || [r.requestId, r.responseRequestId, r.upstreamRequestId].includes(String(query.requestId))))
  const cursor = query.cursor as { requestId: string } | null
  const start = cursor ? records.findIndex(r => r.requestId === cursor.requestId) + 1 : 0
  const page = records.slice(start, start + 50)
  const last = page[page.length - 1]
  return { records: page, nextCursor: start + 50 < records.length && last ? { completedAt: last.completedAt, requestId: last.requestId, snapshotRowId: 125 } : null,
    firstCursor: { completedAt: query.to, requestId: '', snapshotRowId: 125 } }
}
