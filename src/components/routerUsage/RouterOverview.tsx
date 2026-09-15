import { useI18n } from '../../i18n'
import { getRouterManagementLabels } from '../../i18n/routerManagement'
import { ArrowUpRight, Route } from 'lucide-react'
import { Surface } from '../ui'
import { useRouterReport } from './useRouterReport'
import { DAY } from './chartData'
import { milliseconds, type PerformanceReport } from './performanceTypes'

export default function RouterOverview({ onOpen }: { onOpen: () => void }) {
  const { lang } = useI18n()
  const l = getRouterManagementLabels(lang)
  const today = Math.floor(Date.now() / DAY) * DAY
  const { report, error } = useRouterReport<{ summary: { requests: number; success: number; complete: number; partial: number; unknown: number; rejected: number; cancelled: number; failed: number; incomplete: number } }>('get_router_usage', { query: { from: today, to: today + DAY } }, 30_000)
  const { report: performance, error: performanceError } = useRouterReport<PerformanceReport>('get_router_performance', { query: { from: today, to: today + DAY } }, 30_000)
  const s = report?.summary
  const ratio = (n: number, total: number) => total ? `${(100 * n / total).toFixed(1)}%` : '—'
  const firstOutput = performance?.summary.firstOutput.p95
  const latency = firstOutput == null ? '—' : firstOutput >= 1000 ? `${(firstOutput / 1000).toLocaleString(lang, { maximumFractionDigits: 2 })} s` : milliseconds(firstOutput, lang)
  return <Surface as="section" className="min-w-0 overflow-hidden" data-testid="router-overview">
    <div className="border-b border-slate-200/70 px-4 py-3 dark:border-slate-800">
      <div className="flex items-center gap-2"><Route className="h-4 w-4 text-blue-600 dark:text-blue-400" aria-hidden="true" /><h3 className="text-sm font-semibold">{l.overview}</h3></div>
      <p className="mt-1 text-xs text-slate-500 dark:text-slate-400">{l.requestsToday} <span className="font-medium tabular-nums text-slate-700 dark:text-slate-300">{s?.requests.toLocaleString(lang) ?? '—'}</span></p>
    </div>
    <div className="px-4 py-3">
    {error || performanceError ? <p role="status" className="mt-2 text-xs text-amber-700 dark:text-amber-300">{error || performanceError}</p> : null}
      <dl className="grid grid-cols-2 gap-4">
        {[{ label: l.successRate, value: s ? ratio(s.success, s.requests) : '—' }, { label: l.coverage, value: s ? ratio(s.complete, s.complete + s.partial + s.unknown) : '—' }].map(metric => <div key={metric.label}>
          <dt className="text-xs text-slate-500 dark:text-slate-400">{metric.label}</dt><dd className="mt-1 text-2xl font-semibold tracking-tight tabular-nums">{metric.value}</dd>
        </div>)}
        <div className="col-span-2 flex flex-wrap items-center justify-between gap-2 border-t border-slate-200/70 pt-3 dark:border-slate-800">
          <dt className="text-xs text-slate-500 dark:text-slate-400">{l.firstOutput} · P95</dt><dd className="text-base font-semibold tabular-nums" title={milliseconds(firstOutput, lang)}>{latency}</dd>
        </div>
      </dl>
      <dl className="mt-3 grid grid-cols-2 gap-x-4 gap-y-2 text-xs">{(['rejected', 'cancelled', 'failed', 'incomplete'] as const).map(kind => <div key={kind} className="flex items-center justify-between gap-2">
        <dt className="text-slate-500 dark:text-slate-400">{kind === 'incomplete' ? l.incompleteCalls : l[kind]}</dt><dd className={`tabular-nums ${s?.[kind] ? 'text-amber-700 dark:text-amber-300' : 'text-slate-500 dark:text-slate-400'}`}>{s?.[kind].toLocaleString(lang) ?? '—'}</dd>
      </div>)}</dl>
    </div>
    <button type="button" onClick={onOpen} className="flex w-full items-center justify-between border-t border-slate-200/70 bg-slate-50 px-4 py-3 text-xs font-medium text-blue-600 transition hover:bg-blue-50 focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-blue-500 dark:border-slate-800 dark:bg-slate-950/30 dark:text-blue-400 dark:hover:bg-blue-500/10">
      {l.open}<ArrowUpRight className="h-4 w-4" aria-hidden="true" />
    </button>
  </Surface>
}
