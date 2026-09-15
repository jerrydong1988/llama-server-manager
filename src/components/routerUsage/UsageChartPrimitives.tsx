import { getRouterUsageChartLabels } from '../../i18n/routerUsageCharts'
import { getRouterUsageLabels } from '../../i18n/routerUsage'
import type { ChartMetric } from './chartData'

export function MetricToggle({ value, onChange, label, lang }: {
  value: ChartMetric; onChange: (value: ChartMetric) => void; label: string; lang: string
}) {
  const l = getRouterUsageLabels(lang)
  const c = getRouterUsageChartLabels(lang)
  return <div role="group" aria-label={label} className="inline-flex shrink-0 rounded-lg bg-slate-100 p-1 dark:bg-slate-950">
    {(['requests', 'tokens'] as const).map(metric => <button key={metric} type="button" aria-pressed={value === metric}
      onClick={() => onChange(metric)} className={`rounded-md px-3 py-1.5 text-xs font-medium outline-none focus-visible:ring-2 focus-visible:ring-blue-500 ${value === metric
        ? 'bg-white text-blue-700 shadow-sm dark:bg-slate-800 dark:text-blue-300' : 'text-slate-600 hover:text-slate-950 dark:text-slate-400 dark:hover:text-slate-100'}`}>
      {metric === 'requests' ? l.requests : c.tokens}
    </button>)}
  </div>
}

export function ChartEmpty({ children }: { children: string }) {
  return <p className="flex min-h-56 items-center justify-center rounded-lg border border-dashed border-slate-200 px-6 text-center text-sm leading-6 text-slate-500 dark:border-slate-700 dark:text-slate-400">{children}</p>
}

export function LegendDot({ className }: { className: string }) {
  return <span aria-hidden="true" className={`inline-block h-2.5 w-2.5 shrink-0 rounded-sm ${className}`} />
}
