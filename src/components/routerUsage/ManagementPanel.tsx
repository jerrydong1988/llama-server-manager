import { useId } from 'react'
import { Activity, AlertTriangle, Clock3, Layers3, Pause, ShieldCheck } from 'lucide-react'
import { useI18n } from '../../i18n'
import { getRouterManagementLabels } from '../../i18n/routerManagement'
import { Surface } from '../ui'
import { KeyUsageRow, type KeyBudget, type KeyCapacity } from './KeyUsageRow'
import { useRouterReport } from './useRouterReport'

type Budgets = { keys: KeyBudget[]; dayFrom: number; monthFrom: number; updatedAt: number; droppedRecords: number; writeErrors: number; health: { pendingRecords: number; interruptedSessions: number } }
type Admission = { active: number; queued: number; limit: number; keys: KeyCapacity[]; models: Record<string, number>; instances: Record<string, number> }
type Status = { running: boolean; admission?: Admission | null }

export default function ManagementPanel({ revision }: { revision: number }) {
  const { lang } = useI18n()
  const l = getRouterManagementLabels(lang)
  const incompleteId = useId()
  const { report: budgets, error } = useRouterReport<Budgets>('get_router_budgets', {}, 15_000, true, revision)
  const { report: status, error: statusError } = useRouterReport<Status>('get_proxy_status', {}, 5_000)
  const live = status?.running ? status.admission : null
  const stopped = status?.running === false
  const number = (n: number | null | undefined) => n == null ? '—' : n.toLocaleString(lang)
  const incomplete = budgets?.keys.some(key => key.day.partial + key.day.unknown + key.month.partial + key.month.unknown > 0)
  const grid = stopped ? 'lg:grid-cols-[minmax(0,.85fr)_minmax(0,1fr)_minmax(0,1fr)]' : 'lg:grid-cols-[minmax(0,.85fr)_minmax(0,1fr)_minmax(0,1fr)_minmax(140px,.65fr)]'

  return <Surface as="section" className="min-w-0 overflow-hidden" data-testid="router-management-panel">
    <div className="px-5 pt-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex items-center gap-3">
          <span className="rounded-lg bg-blue-50 p-2 text-blue-600 dark:bg-blue-500/10 dark:text-blue-400"><Activity className="h-5 w-5" aria-hidden="true" /></span>
          <div><h3 className="font-semibold">{l.current}</h3><p className="mt-1 text-xs text-slate-500 dark:text-slate-400">{l.currentPeriod}</p></div>
        </div>
        <span className={`inline-flex items-center gap-2 rounded-full px-3 py-1.5 text-xs font-medium ${live ? 'bg-emerald-50 text-emerald-700 dark:bg-emerald-500/10 dark:text-emerald-300' : 'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-400'}`}>
          <span className={`h-1.5 w-1.5 rounded-full ${live ? 'bg-emerald-500' : 'bg-slate-400'}`} aria-hidden="true" />
          {stopped ? l.inactive : live ? l.live : statusError || status ? l.unavailable : l.loading}
        </span>
      </div>
      {error || statusError ? <p role="alert" className="mt-3 break-words text-sm text-red-700 dark:text-red-300">{error || statusError}</p> : null}
      <div className="my-4 rounded-lg bg-slate-50 px-4 py-3 dark:bg-slate-950/50" data-testid="router-concurrency-strip">
        {stopped ? <p className="flex items-center gap-2 text-xs leading-5 text-slate-500 dark:text-slate-400"><Pause className="h-4 w-4 shrink-0" aria-hidden="true" />{l.inactiveHint}</p> :
          <dl className="grid grid-cols-3 gap-3">{[
            { label: l.active, value: live?.active, icon: Activity, tone: 'text-blue-600 dark:text-blue-400' },
            { label: l.queued, value: live?.queued, icon: Clock3, tone: 'text-amber-600 dark:text-amber-400' },
            { label: l.globalLimit, value: live?.limit, icon: Layers3, tone: 'text-slate-500 dark:text-slate-400' },
          ].map(({ label, value, icon: Icon, tone }) => <div key={label} className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <dt className="flex items-center gap-2 text-xs text-slate-500 dark:text-slate-400"><Icon className={`h-4 w-4 ${tone}`} aria-hidden="true" />{label}</dt>
            <dd className="text-xl font-semibold tabular-nums">{number(value)}</dd>
          </div>)}</dl>}
      </div>
    </div>

    {budgets ? <>
      <div className={`hidden gap-6 border-y border-slate-200/70 bg-slate-50/60 px-5 py-2.5 text-xs text-slate-500 dark:border-slate-800 dark:bg-slate-950/20 dark:text-slate-400 lg:grid ${grid}`} aria-hidden="true">
        <span>{l.keys} <span className="ml-1 tabular-nums">{budgets.keys.length}</span></span>
        <span>{l.day} <span className="ml-2 text-[11px]">{new Date(budgets.dayFrom).toISOString().slice(0, 10)} · UTC</span></span>
        <span>{l.month} <span className="ml-2 text-[11px]">{new Date(budgets.monthFrom).toISOString().slice(0, 7)} · UTC</span></span>
        {!stopped ? <span>{l.concurrency}</span> : null}
      </div>
      <p className="px-5 text-xs text-slate-500 dark:text-slate-400 lg:hidden">{new Date(budgets.dayFrom).toISOString().slice(0, 10)} / {new Date(budgets.monthFrom).toISOString().slice(0, 7)} · UTC</p>
      <ul aria-label={l.keys} className="divide-y divide-slate-200/70 dark:divide-slate-800">
        {budgets.keys.map(key => <KeyUsageRow key={key.id} budget={key} capacity={live?.keys.find(k => k.id === key.id)} showConcurrency={!stopped} grid={grid} incompleteId={incompleteId} />)}
      </ul>
      {!budgets.keys.length ? <p className="px-5 py-8 text-center text-sm text-slate-500 dark:text-slate-400">{l.noKeys}</p> : null}
      <div className="space-y-2 border-t border-slate-200/70 px-5 py-3 text-xs leading-5 text-slate-500 dark:border-slate-800 dark:text-slate-400">
        {incomplete ? <p id={incompleteId} className="flex items-start gap-2 text-amber-700 dark:text-amber-300"><AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />{l.incomplete}</p> : null}
        {budgets.health.pendingRecords || budgets.health.interruptedSessions || budgets.droppedRecords || budgets.writeErrors ? <p role="status" className="text-amber-700 dark:text-amber-300">{l.gap}</p> : null}
        {budgets.keys.some(key => key.quota && (key.quota.dailyLimit || key.quota.monthlyLimit || key.quota.day.held || key.quota.month.held || key.quota.month.settled)) ? <p>{l.quotaNote}</p> : null}
        <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-1">
          <p className="flex items-center gap-2"><ShieldCheck className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />{l.softBudgetNote}</p>
          <p>{l.updated}: {new Date(budgets.updatedAt).toLocaleString(lang)}</p>
        </div>
      </div>
    </> : !error ? <p role="status" className="px-5 pb-5 text-sm">{l.loading}</p> : null}

    {live ? <div className="border-t border-slate-200/70 px-5 py-3 dark:border-slate-800">
      <div className="grid gap-3 text-xs sm:grid-cols-2">{(['models', 'instances'] as const).map(group => <div key={group} className="flex min-w-0 flex-wrap items-center gap-2">
        <h4 className="text-slate-500 dark:text-slate-400">{l[group]} · {l.active}</h4>
        {Object.entries(live[group]).map(([id, active]) => <span key={id} className="max-w-full break-all rounded bg-slate-100 px-2 py-1 dark:bg-slate-800">{id} <strong className="ml-1 tabular-nums">{number(active)}</strong></span>)}
        {!Object.keys(live[group]).length ? <span>0</span> : null}
      </div>)}</div>
      <p className="mt-2 text-[11px] leading-5 text-slate-500 dark:text-slate-400">{l.liveHint}</p>
    </div> : null}
  </Surface>
}
