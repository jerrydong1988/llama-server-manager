import { routerUsageMock } from './routerUsageMock'
const day = 86400000
const empty = () => ({ samples: 0, p50: null, p95: null, max: null })
export function routerPerformanceMock(query: Record<string, unknown>) {
  const summary = { requests: 0, success: 0, firstRecordAt: null as number | null, duration: empty(), queue: empty(), firstOutput: empty() }
  let groups: ReturnType<typeof routerUsageMock>['keys'] = []
  try { groups = routerUsageMock(query).keys } catch { /* The usage report tests its own error state. */ }
  const active = groups.length > 0 && query.kind !== 'count'
  const make = (p50: number, p95: number) => ({ samples: active ? 30 : 0, p50: active ? p50 : null, p95: active ? p95 : null, max: active ? p95 : null })
  const current = { ...summary, requests: active ? 30 : 0, success: active ? 30 : 0, duration: make(1000, 5000), queue: make(0, 120), firstOutput: make(100, 800) }
  const periodMs = Number(query.to) - Number(query.from) <= 2 * day ? 3600000 : 21600000
  const bucket = Math.floor(Date.now() / periodMs) * periodMs
  const sample = (id: string, name = id) => ({ id, name, summary: current })
  return { summary: current, keys: groups.map(g => sample(g.id, g.name)), models: active ? [sample('public-model')] : [], instances: active ? [sample('instance-one')] : [], periods: active ? [sample(String(bucket))] : [], periodMs,
    baseline: { ...summary, duration: make(500,1000), queue: make(0,100), firstOutput: empty() }, baselineFrom: Number(query.from) - 7 * day, baselineTo: query.from, elevated: active ? ['duration'] : [], updatedAt: Date.now() }
}

export function routerBudgetsMock(keys: { id: string; name: string; enabled: boolean; daily_token_budget?: number; monthly_token_budget?: number }[]) {
  if (new URLSearchParams(location.search).has('usageManagement')) keys = [{ id: 'key-a', name: 'WorkBuddy', enabled: true, daily_token_budget: 10000, monthly_token_budget: 100000 }]
  const now = Date.now()
  return { keys: keys.map(k => ({ id: k.id, name: k.name, enabled: k.enabled, dailyBudget: k.daily_token_budget || 0, monthlyBudget: k.monthly_token_budget || 0,
    day: { used: 11000, partial: 0, unknown: 0 }, month: { used: 91000, partial: 1, unknown: 0 } })), dayFrom: Math.floor(now / day) * day,
    monthFrom: Date.UTC(new Date(now).getUTCFullYear(), new Date(now).getUTCMonth(), 1), updatedAt: now,
    droppedRecords: 0, writeErrors: 0, health: { pendingRecords: 0, interruptedSessions: 0 } }
}
