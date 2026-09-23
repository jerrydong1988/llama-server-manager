import { getRouterUsageLabels } from '../../i18n/routerUsage'
import { getRouterUsageChartLabels } from '../../i18n/routerUsageCharts'
import { SelectInput, Surface } from '../ui'
import { rankUsage, type ChartMetric, type UsageGroup } from './chartData'
import { ChartEmpty, LegendDot, MetricToggle } from './UsageChartPrimitives'

export type RankingDimension = 'keys' | 'models' | 'instances'
export default function UsageRankingChart({ groups, lang, onFilter, dimension, onDimensionChange, metric, onMetricChange }: {
  groups: Record<RankingDimension, UsageGroup[]>; lang: string; onFilter: (dimension: RankingDimension, id: string) => void
  dimension: RankingDimension; onDimensionChange: (dimension: RankingDimension) => void; metric: ChartMetric; onMetricChange: (metric: ChartMetric) => void
}) {
  const l = getRouterUsageLabels(lang)
  const c = getRouterUsageChartLabels(lang)
  const ranked = rankUsage(groups[dimension], metric)
  const total = ranked.reduce((sum, g) => sum + (g.value ?? 0), 0)
  const peak = ranked.reduce((peak, g) => Math.max(peak, g.value ?? 0), 1)
  const name = (g: UsageGroup) => g.id === 'anonymous' ? l.anonymous : g.id === 'unauthenticated' ? l.unauthenticated : g.name || g.id || l.unknownName
  return <Surface as="section" className="min-w-0 p-5" aria-label={c.ranking} data-testid="usage-ranking-chart">
    <div className="flex flex-wrap items-center justify-between gap-3"><h3 className="font-semibold">{c.ranking}</h3>
      <SelectInput className="h-9 max-w-full text-xs" aria-label={c.rankBy} value={dimension} onChange={e => onDimensionChange(e.target.value as RankingDimension)}>
        <option value="keys">{l.key}</option><option value="models">{l.model}</option><option value="instances">{l.instance}</option>
      </SelectInput>
    </div>
    <div className="my-4"><MetricToggle value={metric} onChange={onMetricChange} label={c.rankMetric} lang={lang} /></div>
    {metric === 'tokens' ? <div className="mb-2 flex flex-wrap gap-x-4 gap-y-2 text-xs text-slate-500 dark:text-slate-400">
      <span className="flex items-center gap-2"><LegendDot className="bg-blue-500" />{l.input}</span><span className="flex items-center gap-2"><LegendDot className="bg-violet-500" />{l.output}</span>
    </div> : null}
    {!ranked.length ? <ChartEmpty>{l.empty}</ChartEmpty> : <>
      <div className="space-y-1">{ranked.slice(0, 6).map((g, i) => <button key={g.id} type="button" disabled={!g.id} onClick={() => onFilter(dimension, g.id)}
        aria-label={`${c.filter} ${name(g)} · ${g.id}`} title={`${name(g)} · ${g.id}`}
        className="block w-full min-w-0 rounded-lg px-2 py-2.5 text-left outline-none hover:bg-slate-50 focus-visible:ring-2 focus-visible:ring-blue-500 disabled:cursor-default dark:hover:bg-slate-950/60">
        <span className="mb-2 flex min-w-0 items-center gap-2 text-xs">
          <span className="w-4 shrink-0 text-slate-400">{i + 1}</span><span className="min-w-0 flex-1 truncate font-medium">{name(g)}</span>
          <strong className="shrink-0 tabular-nums">{g.value == null ? c.unreported : g.value.toLocaleString(lang)}</strong>
          <span className="w-14 shrink-0 text-right tabular-nums text-slate-500 dark:text-slate-400">{g.value != null && total > 0 ? `${(g.value / total * 100).toFixed(1)}%` : '—'}</span>
        </span>
        <span className="ml-6 flex h-2 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800" aria-hidden="true">
          {metric === 'requests' ? <span className="bg-blue-500" style={{ width: `${(g.value ?? 0) / peak * 100}%` }} /> : <>
            <span className="bg-blue-500" style={{ width: `${g.summary.input / peak * 100}%` }} /><span className="bg-violet-500" style={{ width: `${g.summary.output / peak * 100}%` }} />
          </>}
        </span>
      </button>)}</div>
      <p className="mt-3 text-xs leading-5 text-slate-500 dark:text-slate-400">{c.rankHint} {ranked.length > 6 ? c.top : ''}</p>
      {metric === 'tokens' ? <p className="mt-2 text-xs leading-5 text-slate-500 dark:text-slate-400">{c.partialHint}</p> : null}
    </>}
  </Surface>
}
