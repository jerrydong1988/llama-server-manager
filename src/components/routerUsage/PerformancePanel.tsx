import { useState } from 'react'
import { useI18n } from '../../i18n'
import { getRouterManagementLabels } from '../../i18n/routerManagement'
import { MetricCard, SelectInput, Surface } from '../ui'
import { milliseconds, type PerformanceMetric, type PerformanceReport, type UsageFilters } from './performanceTypes'
import { useRouterReport } from './useRouterReport'

export default function PerformancePanel({ query, revision }: { query: UsageFilters; revision: number }) {
  const { lang } = useI18n()
  const l = getRouterManagementLabels(lang)
  const { report, error } = useRouterReport<PerformanceReport>('get_router_performance', { query }, 15_000, true, revision)
  const [metric, setMetric] = useState<PerformanceMetric>('firstOutput')
  const [grouping, setGrouping] = useState<'keys' | 'models' | 'instances'>('keys')
  const [selected, setSelected] = useState('')
  const format = (n: number | null | undefined) => milliseconds(n, lang)
  const metrics: PerformanceMetric[] = ['firstOutput', 'duration', 'queue']
  const byPeriod = new Map(report?.periods.map(p => [Number(p.id), p]))
  const width = report?.periodMs || 86400000
  const periods = Array.from({ length: Math.min(366, Math.ceil((query.to - query.from) / width)) }, (_, i) => {
    const at = query.from + i * width
    return { at, sample: byPeriod.get(at)?.summary[metric] }
  })
  const max = Math.max(1, ...periods.map(p => p.sample?.p95 || 0))
  const active = periods.find(p => String(p.at) === selected) || [...periods].reverse().find(p => p.sample?.samples) || periods[0]
  const date = (at: number) => new Date(at).toISOString().replace('T', ' ').slice(0, width < 86400000 ? 16 : 10)
  const ready = report && report.summary[metric].samples >= 20 && report.baseline[metric].samples >= 20
  return <Surface as="section" className="min-w-0 p-5" data-testid="router-performance-panel">
    <h3 className="font-semibold">{l.performance}</h3><p className="mt-2 text-xs leading-5 text-slate-500 dark:text-slate-400">{l.performanceHint}</p>
    {error ? <p role="alert" className="mt-3 text-sm text-red-700 dark:text-red-300">{error}</p> : !report ? <p role="status">{l.loading}</p> : <>
      <div className="my-4 grid gap-3 sm:grid-cols-3">{metrics.map(m => <div key={m}><MetricCard label={`${l[m]} · P95`} value={format(report.summary[m].p95)} />
        <p className="mt-2 text-xs text-slate-500 dark:text-slate-400">P50 {format(report.summary[m].p50)} · {l.samples} {report.summary[m].samples.toLocaleString(lang)}</p></div>)}</div>
      <div className="flex flex-wrap items-center justify-between gap-3 text-xs"><label>{l.metric}<SelectInput aria-label={l.metric} className="ml-2" value={metric} onChange={e => setMetric(e.target.value as PerformanceMetric)}>{metrics.map(m => <option key={m} value={m}>{l[m]}</option>)}</SelectInput></label>
        <span>{width === 3600000 ? l.hourly : width === 21600000 ? l.sixHourly : l.daily} · UTC · P50 / P95</span></div>
      {!report.summary[metric].samples ? <p className="py-8 text-center text-sm text-slate-500 dark:text-slate-400">{l.empty}</p> : <>
        <div className="mt-5 grid grid-cols-[60px_minmax(0,1fr)] gap-2">
          <div className="flex h-44 flex-col justify-between text-right text-xs text-slate-500 dark:text-slate-400"><span>{format(max)}</span><span>{format(Math.round(max / 2))}</span><span>0 ms</span></div>
          <div className="relative h-44"><svg viewBox="0 0 900 180" preserveAspectRatio="none" className="h-full w-full" aria-hidden="true">
            {[0, 90, 179].map(y => <line key={y} x1={0} x2={900} y1={y} y2={y} className="stroke-slate-200 dark:stroke-slate-700" strokeDasharray="4 5" />)}
            {periods.map((p, i) => p.sample?.samples ? <g key={p.at}><rect x={(i + .15) * 900 / periods.length} width={.7 * 900 / periods.length} y={178 - (p.sample.p95 || 0) / max * 176} height={Math.max(2, (p.sample.p95 || 0) / max * 176)} className="fill-blue-500/40" />
              <rect x={(i + .15) * 900 / periods.length} width={.7 * 900 / periods.length} y={178 - (p.sample.p50 || 0) / max * 176} height={2} className="fill-violet-600 dark:fill-violet-400" /></g> : null)}
          </svg><div className="absolute inset-0 flex" role="group" aria-label={l.performance}>{periods.map(p => <button key={p.at} type="button" className="min-w-0 flex-1 focus-visible:ring-2 focus-visible:ring-blue-500" aria-label={`${date(p.at)} UTC · P50 ${format(p.sample?.p50)} · P95 ${format(p.sample?.p95)} · ${l.samples} ${p.sample?.samples || 0}`}
            tabIndex={active?.at === p.at ? 0 : -1} onMouseEnter={() => setSelected(String(p.at))} onFocus={() => setSelected(String(p.at))} onClick={() => setSelected(String(p.at))}
            onKeyDown={e => { const offset = e.key === 'ArrowLeft' ? -1 : e.key === 'ArrowRight' ? 1 : 0; if (!offset) return; e.preventDefault(); const buttons = e.currentTarget.parentElement?.querySelectorAll('button'); buttons?.[Math.max(0, Math.min(periods.findIndex(v => v.at === p.at) + offset, periods.length - 1))]?.focus() }} />)}</div></div>
        </div>
        <div className="mt-2 flex justify-between text-xs text-slate-500 dark:text-slate-400"><span>{date(query.from)}</span><span>{date(query.to - width)}</span></div>
        <p className="mt-4 rounded-lg bg-slate-50 p-3 text-xs dark:bg-slate-950/60" data-testid="performance-period-detail">{active ? `${date(active.at)} UTC · P50 ${format(active.sample?.p50)} · P95 ${format(active.sample?.p95)} · ${l.samples} ${active.sample?.samples || 0}` : l.empty}</p>
      </>}
      <p className="mt-4 text-xs leading-5 text-slate-500 dark:text-slate-400">{l.baseline}</p>
      <p className={`mt-1 text-xs ${report.elevated.includes(metric) ? 'text-amber-700 dark:text-amber-300' : 'text-slate-500 dark:text-slate-400'}`}>{ready ? report.elevated.includes(metric) ? l.elevated : l.normal : l.warming} · P95 {format(report.baseline[metric].p95)} · {l.samples} {report.baseline[metric].samples}</p>
      <label className="mt-5 block text-xs">{l.distribution}<SelectInput aria-label={l.distribution} className="ml-2" value={grouping} onChange={e => setGrouping(e.target.value as typeof grouping)}>{(['keys', 'models', 'instances'] as const).map(g => <option key={g} value={g}>{l[g]}</option>)}</SelectInput></label>
      <div className="mt-3 max-h-64 overflow-auto"><table className="w-full text-left text-xs"><thead><tr>{[l[grouping], 'P50', 'P95', l.samples].map(h => <th key={h} className="p-2">{h}</th>)}</tr></thead><tbody>{[...report[grouping]].sort((a, b) => (b.summary[metric].p95 || 0) - (a.summary[metric].p95 || 0)).map(g => <tr key={g.id} className="border-t border-slate-200 dark:border-slate-800"><td className="max-w-48 break-words p-2">{g.name || g.id || '—'}</td><td className="whitespace-nowrap p-2">{format(g.summary[metric].p50)}</td><td className="whitespace-nowrap p-2">{format(g.summary[metric].p95)}</td><td className="p-2">{g.summary[metric].samples.toLocaleString(lang)}</td></tr>)}</tbody></table></div>
    </>}
    <p className="mt-4 text-xs leading-5 text-slate-500 dark:text-slate-400">{l.precision}</p>
  </Surface>
}
