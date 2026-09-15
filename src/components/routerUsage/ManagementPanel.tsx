import { useI18n } from '../../i18n'
import { getRouterManagementLabels } from '../../i18n/routerManagement'
import { MetricCard, Surface } from '../ui'
import { useRouterReport } from './useRouterReport'

type Usage = { used: number; partial: number; unknown: number }
type Budget = { id: string; name: string; enabled: boolean; dailyBudget: number; monthlyBudget: number; day: Usage; month: Usage }
type Budgets = { keys: Budget[]; dayFrom: number; monthFrom: number; updatedAt: number; droppedRecords: number; writeErrors: number; health: { pendingRecords: number; interruptedSessions: number } }
type Admission = { active: number; queued: number; limit: number; keys: { id: string; active: number; queued: number; limit: number }[]; models: Record<string, number>; instances: Record<string, number> }
type Status = { running: boolean; admission?: Admission | null }

export default function ManagementPanel({ revision }: { revision: number }) {
  const { lang } = useI18n()
  const l = getRouterManagementLabels(lang)
  const { report: budgets, error } = useRouterReport<Budgets>('get_router_budgets', {}, 15_000, true, revision)
  const { report: status, error: statusError } = useRouterReport<Status>('get_proxy_status', {}, 5_000)
  const live = status?.running ? status.admission : null
  const number = (n: number | null | undefined) => n == null ? '—' : n.toLocaleString(lang)
  const budget = (usage: Usage, limit: number) => <div className="space-y-1">
    <p>{number(usage.used)} / {limit ? number(limit) : l.disabled}</p>
    {limit > 0 ? <><div className="h-1.5 overflow-hidden rounded bg-slate-200 dark:bg-slate-700" role="meter" aria-label={l.current} aria-valuemin={0} aria-valuemax={limit} aria-valuenow={Math.min(usage.used, limit)}><div className={`h-full ${usage.used >= limit ? 'bg-amber-500' : 'bg-blue-500'}`} style={{ width: `${Math.min(100, usage.used / limit * 100)}%` }} /></div>
      <p className={usage.used >= limit * .8 ? 'text-amber-700 dark:text-amber-300' : 'text-slate-500 dark:text-slate-400'}>{(usage.used / limit * 100).toFixed(1)}% {usage.used >= limit ? l.exceeded : usage.used >= limit * .8 ? l.near : ''}</p></> : null}
    {usage.partial + usage.unknown > 0 ? <p className="max-w-64 text-amber-700 dark:text-amber-300">{l.incomplete}</p> : null}
  </div>
  return <Surface as="section" className="min-w-0 p-5" data-testid="router-management-panel">
    <h3 className="font-semibold">{l.current}</h3><p className="mt-2 text-xs leading-5 text-slate-500 dark:text-slate-400">{l.currentHint}</p>
    {error || statusError ? <p role="alert" className="mt-3 text-sm text-red-700 dark:text-red-300">{error || statusError}</p> : null}
    <div className="my-4 grid gap-3 sm:grid-cols-3"><MetricCard label={l.active} value={number(live?.active)} /><MetricCard label={l.queued} value={number(live?.queued)} /><MetricCard label={l.limit} value={number(live?.limit)} /></div>
    <p className="text-xs text-slate-500 dark:text-slate-400">{status && !status.running ? l.inactive : l.liveHint}</p>
    {budgets ? <>
      <p className="mt-4 text-xs text-slate-500 dark:text-slate-400">{l.day}: {new Date(budgets.dayFrom).toISOString().slice(0, 10)} · {l.month}: {new Date(budgets.monthFrom).toISOString().slice(0, 7)} · UTC</p>
      {budgets.health.pendingRecords || budgets.health.interruptedSessions || budgets.droppedRecords || budgets.writeErrors ? <p role="status" className="mt-3 text-xs text-amber-700 dark:text-amber-300">{l.gap}</p> : null}
      <div className="mt-3 overflow-auto"><table className="w-full text-left text-xs"><thead><tr>{[l.keys, l.day, l.month, l.active, l.queued, l.limit].map(h => <th key={h} className="whitespace-nowrap p-2">{h}</th>)}</tr></thead><tbody>{budgets.keys.map(key => {
        const capacity = live?.keys.find(k => k.id === key.id)
        return <tr key={key.id} className="border-t border-slate-200 align-top dark:border-slate-800"><td className="max-w-40 break-words p-2">{key.name || key.id}</td><td className="min-w-40 p-2">{budget(key.day, key.dailyBudget)}</td><td className="min-w-40 p-2">{budget(key.month, key.monthlyBudget)}</td><td className="p-2">{number(capacity?.active)}</td><td className="p-2">{number(capacity?.queued)}</td><td className="p-2">{number(capacity?.limit)}</td></tr>
      })}</tbody></table></div>
      {!budgets.keys.length ? <p className="py-4 text-sm">{l.noKeys}</p> : null}
      <p className="mt-3 text-xs text-slate-500 dark:text-slate-400">{l.updated}: {new Date(budgets.updatedAt).toLocaleString(lang)}</p>
    </> : !error ? <p role="status" className="mt-3 text-sm">{l.loading}</p> : null}
    {live ? <div className="mt-4 grid gap-3 sm:grid-cols-2">{(['models', 'instances'] as const).map(group => <div key={group} className="rounded-lg bg-slate-50 p-3 text-xs dark:bg-slate-950/60"><h4 className="font-medium">{l[group]} · {l.active}</h4><div className="mt-2 flex flex-wrap gap-3">{Object.entries(live[group]).map(([id, active]) => <span key={id} className="break-all">{id}: {number(active)}</span>)}{!Object.keys(live[group]).length ? <span>0</span> : null}</div></div>)}</div> : null}
  </Surface>
}
