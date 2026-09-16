import { AlertTriangle } from 'lucide-react'
import { useI18n } from '../../i18n'
import { getRouterManagementLabels } from '../../i18n/routerManagement'
import { QuotaUsage, type KeyQuota } from './QuotaUsage'

type Usage = { used: number; partial: number; unknown: number }
export type KeyBudget = { id: string; name: string; enabled: boolean; dailyBudget: number; monthlyBudget: number; day: Usage; month: Usage; quota?: KeyQuota }
export type KeyCapacity = { id: string; active: number; queued: number; limit: number }

export function KeyUsageRow({ budget, capacity, showConcurrency, grid, incompleteId }: {
  budget: KeyBudget; capacity?: KeyCapacity; showConcurrency: boolean; grid: string; incompleteId: string
}) {
  const { lang } = useI18n()
  const l = getRouterManagementLabels(lang)
  const number = (n: number | null | undefined) => n == null ? '—' : n.toLocaleString(lang)
  const name = budget.name || budget.id
  return <li className={`grid min-w-0 grid-cols-2 items-center gap-x-6 gap-y-4 px-5 py-4 transition-colors hover:bg-slate-50/70 dark:hover:bg-slate-800/25 ${grid}`}>
    <div className="col-span-2 flex min-w-0 items-center gap-3 lg:col-span-1">
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-slate-200 bg-slate-50 text-sm font-semibold text-slate-500 dark:border-slate-700/60 dark:bg-slate-800/60 dark:text-slate-300" aria-hidden="true">{Array.from(name)[0]?.toLocaleUpperCase(lang)}</span>
      <div className="min-w-0"><h4 className="break-words text-sm font-medium [overflow-wrap:anywhere]">{name}</h4>{!budget.enabled ? <p className="mt-1 text-xs text-slate-500 dark:text-slate-400">{l.keyDisabled}</p> : null}</div>
    </div>
    {(['day', 'month'] as const).map(period => {
      const usage = budget[period]
      const limit = period === 'day' ? budget.dailyBudget : budget.monthlyBudget
      const percent = limit > 0 ? usage.used / limit * 100 : null
      const warning = percent != null && percent >= 80
      return <dl key={period} className="min-w-0" aria-label={`${name} · ${l[period]}`}>
        <dt className="mb-1 text-xs text-slate-500 dark:text-slate-400 lg:sr-only">{l[period]}</dt>
        <dd>
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="break-all text-lg font-semibold tracking-tight tabular-nums">{number(usage.used)}</span>
            <span className="text-[10px] text-slate-500 dark:text-slate-400">Token</span>
            {usage.partial + usage.unknown > 0 ? <span role="img" aria-label={l.incompleteLabel} aria-describedby={incompleteId} title={l.incomplete} className="text-amber-600 dark:text-amber-400"><AlertTriangle className="h-3.5 w-3.5" aria-hidden="true" /></span> : null}
          </div>
          {percent != null ? <>
            <div className="my-1.5 h-1 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-800" role="meter" aria-label={`${name} · ${l[period]} · ${l.budget}`} aria-valuemin={0} aria-valuemax={limit} aria-valuenow={Math.min(usage.used, limit)} aria-valuetext={`${number(usage.used)} / ${number(limit)} Token (${percent.toFixed(1)}%)`}>
              <div className={`h-full rounded-full ${warning ? 'bg-amber-500' : period === 'day' ? 'bg-blue-500' : 'bg-teal-500'}`} style={{ width: `${Math.min(100, percent)}%` }} />
            </div>
            <p className={`flex flex-wrap justify-between gap-x-2 text-[11px] leading-4 ${warning ? 'text-amber-700 dark:text-amber-300' : 'text-slate-500 dark:text-slate-400'}`}>
              <span>{l.budget} {number(limit)}</span><span className="tabular-nums">{percent.toFixed(1)}%{percent >= 100 ? ` · ${l.exceeded}` : warning ? ` · ${l.near}` : ''}</span>
            </p>
          </> : <p className="mt-1 text-[11px] text-slate-500 dark:text-slate-400">{l.disabled}</p>}
          {budget.quota ? <QuotaUsage quota={budget.quota} period={period} name={name} /> : null}
        </dd>
      </dl>
    })}
    {showConcurrency ? <dl aria-label={`${name} · ${l.concurrency}`} className="col-span-2 grid grid-cols-3 gap-2 border-t border-slate-200/70 pt-3 text-center dark:border-slate-800 lg:col-span-1 lg:border-t-0 lg:pt-0">
      {(['active', 'queued', 'limit'] as const).map(metric => <div key={metric}>
        <dt className="text-[10px] text-slate-500 dark:text-slate-400">{l[metric]}</dt>
        <dd className={`mt-1 text-sm font-semibold tabular-nums ${metric === 'active' && capacity?.active ? 'text-blue-600 dark:text-blue-400' : metric === 'queued' && capacity?.queued ? 'text-amber-700 dark:text-amber-300' : 'text-slate-600 dark:text-slate-300'}`}>{number(capacity?.[metric])}</dd>
      </div>)}
    </dl> : null}
  </li>
}
