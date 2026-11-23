import { useI18n } from '../../i18n'
import { getRouterManagementLabels } from '../../i18n/routerManagement'

type Period = { settled: number; held: number; pending: number; uncertain: number }
export type KeyQuota = { dailyLimit: number; monthlyLimit: number; day: Period; month: Period }

export function QuotaUsage({ quota, period, name }: { quota: KeyQuota; period: 'day' | 'month'; name: string }) {
  const { lang } = useI18n()
  const l = getRouterManagementLabels(lang)
  const number = (n: number) => n.toLocaleString(lang)
  const limit = period === 'day' ? quota.dailyLimit : quota.monthlyLimit
  const { settled, held } = quota[period]
  if (!limit && !settled && !held) return null
  const used = settled + held
  return <div className="mt-3 border-t border-ui-line pt-2 text-[11px]" aria-label={`${name} · ${l[period]} · ${l.quotaAccounting}`}>
    <div className="flex flex-wrap items-center justify-between gap-1 font-medium"><span>{l.hardQuota}</span><span className="tabular-nums">{limit ? number(limit) : l.unlimited}</span></div>
    {limit > 0 ? <div className="my-1.5 flex h-1.5 overflow-hidden rounded-full bg-ui-line" role="meter" aria-label={`${name} · ${l[period]} · ${l.hardQuota}`} aria-valuemin={0} aria-valuemax={limit} aria-valuenow={Math.min(limit, used)} aria-valuetext={`${l.settled} ${number(settled)}, ${l.held} ${number(held)}, ${l.remaining} ${number(Math.max(0, limit - used))}`}>
      <div className="h-full shrink-0 bg-blue-500" style={{ width: `${Math.min(100, settled / limit * 100)}%` }} />
      <div className="h-full bg-amber-500" style={{ width: `${Math.min(100, held / limit * 100)}%` }} />
    </div> : null}
    <div className="mt-1 flex flex-wrap justify-between gap-1 text-ui-muted"><span>{l.settled}</span><span className="tabular-nums">{number(settled)}</span></div>
    <div className={`mt-1 flex flex-wrap justify-between gap-1 ${held ? 'text-amber-700 dark:text-amber-300' : 'text-ui-muted'}`} title={l.quotaNote}><span>{l.held}</span><span className="tabular-nums">{number(held)}</span></div>
    {limit > 0 ? <div className={`mt-1 flex flex-wrap justify-between gap-1 font-medium ${used >= limit ? 'text-rose-700 dark:text-rose-300' : 'text-ui-body'}`}><span>{l.remaining}</span><span className="tabular-nums">{number(Math.max(0, limit - used))}</span></div> : null}
  </div>
}
