import { useI18n } from '../../i18n'
import { getRouterManagementLabels } from '../../i18n/routerManagement'
import { Button, MetricCard, Surface } from '../ui'
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
  return <Surface as="section" className="p-5" data-testid="router-overview"><div className="flex items-center justify-between gap-3"><h3 className="font-semibold">{l.overview}</h3><Button onClick={onOpen}>{l.open}</Button></div>
    {error || performanceError ? <p role="status" className="mt-2 text-xs text-amber-700 dark:text-amber-300">{error || performanceError}</p> : null}
    <div className="mt-3 grid gap-3 sm:grid-cols-3"><MetricCard label={l.successRate} value={s ? ratio(s.success, s.requests) : '—'} /><MetricCard label={`${l.firstOutput} · P95`} value={milliseconds(performance?.summary.firstOutput.p95, lang)} /><MetricCard label={l.coverage} value={s ? ratio(s.complete, s.complete + s.partial + s.unknown) : '—'} /></div>
    <p className="mt-3 text-xs text-slate-500 dark:text-slate-400">{l.rejected}: {s?.rejected ?? '—'} · {l.cancelled}: {s?.cancelled ?? '—'} · {l.failed}: {s?.failed ?? '—'} · {l.incompleteCalls}: {s?.incomplete ?? '—'}</p>
  </Surface>
}
