import { getRouterUsageLabels } from '../../i18n/routerUsage'
import { getRouterUsageChartLabels } from '../../i18n/routerUsageCharts'
import { Surface } from '../ui'
import type { UsageSummary } from './chartData'
import { ChartEmpty, LegendDot } from './UsageChartPrimitives'

export default function UsageOutcomeChart({ summary: s, lang }: { summary: UsageSummary; lang: string }) {
  const l = getRouterUsageLabels(lang)
  const c = getRouterUsageChartLabels(lang)
  const number = (n: number) => n.toLocaleString(lang)
  const percent = (n: number, total: number) => total ? `${(n / total * 100).toFixed(1)}%` : '—'
  const results = [
    { label: l.success, count: s.success, color: 'bg-emerald-500', stroke: 'stroke-emerald-500' },
    { label: l.failed, count: s.failed, color: 'bg-red-500', stroke: 'stroke-red-500' },
    { label: l.rejected, count: s.rejected, color: 'bg-amber-500', stroke: 'stroke-amber-500' },
    { label: l.cancelled, count: s.cancelled, color: 'bg-slate-400', stroke: 'stroke-slate-400' },
    { label: l.incomplete, count: s.incomplete, color: 'bg-violet-500', stroke: 'stroke-violet-500' },
  ]
  const qualities = [
    { label: l.complete, count: s.complete, color: 'bg-blue-500' }, { label: l.partial, count: s.partial, color: 'bg-amber-500' },
    { label: l.unknown, count: s.unknown, color: 'bg-slate-400' }, { label: l.not_applicable, count: s.notApplicable, color: 'bg-slate-200 dark:bg-slate-700' },
  ]
  let offset = 0
  return <Surface as="section" className="min-w-0 p-5" aria-label={c.results} data-testid="usage-outcome-chart">
    <h3 className="font-semibold">{c.results}</h3>
    {!s.requests ? <div className="mt-4"><ChartEmpty>{l.empty}</ChartEmpty></div> : <>
      <div className="my-5 flex flex-wrap items-center justify-center gap-6">
        <div className="relative h-40 w-40 shrink-0">
          <svg viewBox="0 0 160 160" className="h-full w-full -rotate-90" aria-hidden="true">
            <circle cx="80" cy="80" r="66" fill="none" strokeWidth="16" className="stroke-slate-100 dark:stroke-slate-800" />
            {results.map(result => {
              const share = result.count / s.requests * 100
              const start = offset
              offset += share
              return <circle key={result.label} cx="80" cy="80" r="66" fill="none" strokeWidth="16" pathLength="100"
                strokeDasharray={`${share} ${100 - share}`} strokeDashoffset={-start} className={result.stroke} />
            })}
          </svg>
          <div className="absolute inset-0 flex flex-col items-center justify-center">
            <span className="text-2xl font-semibold tabular-nums">{percent(s.success, s.requests)}</span>
            <span className="mt-1 text-xs text-slate-500 dark:text-slate-400">{c.successRate}</span>
          </div>
        </div>
        <ul className="min-w-[170px] flex-1 space-y-3 text-xs">{results.map(result => <li key={result.label} className="flex items-center gap-2">
          <LegendDot className={result.color} /><span className="flex-1">{result.label}</span><strong className="tabular-nums">{number(result.count)}</strong>
          <span className="w-14 text-right tabular-nums text-slate-500 dark:text-slate-400">{percent(result.count, s.requests)}</span>
        </li>)}</ul>
      </div>
      <p className="text-xs leading-5 text-slate-500 dark:text-slate-400">{c.resultHint}</p>
      <div className="mt-5 border-t border-slate-200 pt-4 dark:border-slate-800">
        <div className="flex flex-wrap justify-between gap-2 text-xs"><span className="font-medium">{l.quality}</span><span>{l.coverage}: <strong>{percent(s.complete, s.complete + s.partial + s.unknown)}</strong></span></div>
        <div className="my-3 flex h-2 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800" aria-hidden="true">
          {qualities.map(q => <span key={q.label} className={q.color} style={{ width: `${q.count / s.requests * 100}%` }} />)}
        </div>
        <div className="flex flex-wrap gap-x-4 gap-y-2 text-xs text-slate-600 dark:text-slate-400">{qualities.map(q => <span key={q.label} className="flex items-center gap-1.5"><LegendDot className={q.color} />{q.label}: {number(q.count)}</span>)}</div>
        <p className="mt-3 text-xs text-slate-600 dark:text-slate-400">{l.cacheRatio}: {percent(s.cached, s.cacheInput)}</p>
      </div>
    </>}
  </Surface>
}
