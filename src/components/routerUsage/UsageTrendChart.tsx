import { useMemo, useState } from 'react'
import { getRouterUsageLabels } from '../../i18n/routerUsage'
import { getRouterUsageChartLabels } from '../../i18n/routerUsageCharts'
import { Surface } from '../ui'
import { buildUsageTimeline, DAY, utcDate, usageAxisMax, type ChartMetric, type UsageGroup } from './chartData'
import { ChartEmpty, LegendDot, MetricToggle } from './UsageChartPrimitives'

export default function UsageTrendChart({ days, from, to, lang, metric, onMetricChange }: {
  days: UsageGroup[]; from: number; to: number; lang: string; metric: ChartMetric; onMetricChange: (metric: ChartMetric) => void
}) {
  const l = getRouterUsageLabels(lang)
  const c = getRouterUsageChartLabels(lang)
  const [selection, setSelection] = useState<number | null>(null)
  const { buckets, bucketDays } = useMemo(() => buildUsageTimeline(days, from, to), [days, from, to])
  const lastRecorded = buckets.reduce((last, b, i) => b.requests ? i : last, 0)
  const selected = Math.min(selection ?? lastRecorded, buckets.length - 1)
  const active = buckets[selected]
  const hasRequests = buckets.some(b => b.requests > 0)
  const hasTokens = buckets.some(b => b.inputKnown || b.outputKnown)
  const tokens = metric === 'tokens'
  const maximum = usageAxisMax(buckets.reduce((peak, b) => Math.max(peak, tokens ? b.input + b.output : b.requests), 0))
  const number = (n: number) => n.toLocaleString(lang)
  const compact = (n: number) => n.toLocaleString(lang, { notation: 'compact', maximumFractionDigits: 1 })
  const date = (b: typeof active) => b.to - b.from === DAY ? utcDate(b.from) : `${utcDate(b.from)} – ${utcDate(b.to - DAY)}`
  const tokenValue = (n: number, known: number) => known ? number(n) : c.unreported
  const describe = (b: typeof active) => `${date(b)} · ${l.requests}: ${number(b.requests)} · ${l.input}: ${tokenValue(b.input, b.inputKnown)} · ${l.output}: ${tokenValue(b.output, b.outputKnown)}`
  const width = 900
  const height = 200
  const step = width / buckets.length
  const barWidth = Math.min(48, step * 0.6)

  return <Surface as="section" className="min-w-0 p-5" data-testid="usage-trend-chart" aria-label={c.trend}>
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div><h3 className="font-semibold">{c.trend}</h3><p className="mt-1 text-xs text-slate-500 dark:text-slate-400">{c.trendHint}</p></div>
      <MetricToggle value={metric} onChange={onMetricChange} label={c.trendMetric} lang={lang} />
    </div>
    <div className="my-4 flex flex-wrap items-center justify-between gap-2 text-xs text-slate-600 dark:text-slate-400">
      <span>{bucketDays === 1 ? c.daily : bucketDays === 7 ? c.sevenDays : c.thirtyDays}</span>
      <div className="flex flex-wrap gap-4">{tokens ? <><span className="flex items-center gap-2"><LegendDot className="bg-blue-500" />{l.input}</span>
        <span className="flex items-center gap-2"><LegendDot className="bg-violet-500" />{l.output}</span></> : <span className="flex items-center gap-2"><LegendDot className="bg-blue-500" />{l.requests}</span>}</div>
    </div>
    {!hasRequests || (tokens && !hasTokens) ? <ChartEmpty>{!hasRequests ? l.empty : c.noTokens}</ChartEmpty> : <>
      <div className="grid grid-cols-[48px_minmax(0,1fr)] gap-x-2 pt-2">
        <div className="relative h-[200px] text-right text-[11px] tabular-nums text-slate-500 dark:text-slate-400" aria-hidden="true">
          {[4, 3, 2, 1, 0].map(tick => <span key={tick} className="absolute right-0 -translate-y-1/2" style={{ top: `${(4 - tick) * 25}%` }}>{compact(maximum * tick / 4)}</span>)}
        </div>
        <div className="relative h-[200px] min-w-0">
          <svg viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" className="h-full w-full overflow-visible" aria-hidden="true">
            {[0, 1, 2, 3, 4].map(tick => <line key={tick} x1="0" x2={width} y1={tick * height / 4} y2={tick * height / 4}
              className="stroke-slate-200 dark:stroke-slate-800" strokeDasharray={tick === 4 ? undefined : '4 5'} vectorEffect="non-scaling-stroke" />)}
            <rect x={selected * step} width={step} height={height} className="fill-blue-500/5 dark:fill-blue-400/10" />
            {buckets.map((b, i) => {
              const inputHeight = (tokens ? b.input : b.requests) / maximum * height
              const outputHeight = (tokens ? b.output : 0) / maximum * height
              const x = i * step + (step - barWidth) / 2
              return <g key={b.from}>
                <rect x={x} y={height - inputHeight} width={barWidth} height={inputHeight} className="fill-blue-500" />
                <rect x={x} y={height - inputHeight - outputHeight} width={barWidth} height={outputHeight} className="fill-violet-500" />
              </g>
            })}
          </svg>
          <div role="group" aria-label={c.explore} className="absolute inset-0 flex">
            {buckets.map((b, i) => <button type="button" key={b.from} aria-label={describe(b)} aria-pressed={selected === i} tabIndex={selected === i ? 0 : -1}
              onMouseEnter={() => setSelection(i)} onFocus={() => setSelection(i)} onClick={() => setSelection(i)}
              onKeyDown={e => {
                const next = e.key === 'ArrowLeft' ? i - 1 : e.key === 'ArrowRight' ? i + 1 : e.key === 'Home' ? 0 : e.key === 'End' ? buckets.length - 1 : null
                if (next == null) return
                e.preventDefault()
                const buttons = e.currentTarget.parentElement?.querySelectorAll('button')
                buttons?.[Math.max(0, Math.min(next, buckets.length - 1))]?.focus()
              }} className="relative min-w-0 flex-1 rounded outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-blue-500">
              {tokens && !b.inputKnown && !b.outputKnown ? <span aria-hidden="true" className="absolute inset-x-0 bottom-0 text-xs text-slate-400">—</span> : null}
            </button>)}
          </div>
        </div>
        <div className="col-start-2 mt-3 flex justify-between gap-2 text-[11px] tabular-nums text-slate-500 dark:text-slate-400" aria-hidden="true">
          <span>{utcDate(from)}</span>{buckets.length > 4 ? <span className="hidden sm:inline">{utcDate(buckets[Math.floor(buckets.length / 2)].from)}</span> : null}
          {to - from > DAY ? <span>{utcDate(to - DAY)}</span> : null}
        </div>
      </div>
      {active ? <div data-testid="usage-trend-detail" className="mt-4 flex min-h-14 flex-wrap items-center gap-x-5 gap-y-2 rounded-lg bg-slate-50 px-4 py-3 text-xs dark:bg-slate-950/60">
        <span className="font-medium tabular-nums">{date(active)}</span><span>{l.requests}: <strong>{number(active.requests)}</strong></span>
        {active.requests ? <><span>{l.input}: <strong>{tokenValue(active.input, active.inputKnown)}</strong></span><span>{l.output}: <strong>{tokenValue(active.output, active.outputKnown)}</strong></span>
          {active.partial || active.unknown ? <span className="text-amber-700 dark:text-amber-300">{l.partial}: {number(active.partial)} · {l.unknown}: {number(active.unknown)}</span> : null}</> : <span>{c.noRecorded}</span>}
      </div> : null}
    </>}
    <p className="mt-3 text-xs leading-5 text-slate-500 dark:text-slate-400">{tokens ? c.tokenHint : c.explore} {bucketDays > 1 ? c.bucketsHint : ''}</p>
  </Surface>
}
