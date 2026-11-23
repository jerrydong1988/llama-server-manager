import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Activity, CheckCircle2 } from 'lucide-react'
import type { ModelWorkload, RunningInferenceTask } from '../../store/types'
import { Badge, joinClassNames } from '../ui'
import { clampPercent, formatCompactNumber, formatDuration, formatRate } from './monitoringFormat'
import { buildChartAxis, type ActivityFeedItem, type SignalTone } from './monitoringViewModel'

const toneText: Record<SignalTone, string> = {
  blue: 'text-blue-600 dark:text-blue-300',
  emerald: 'text-emerald-600 dark:text-emerald-300',
  amber: 'text-amber-600 dark:text-amber-300',
  red: 'text-red-600 dark:text-red-300',
  violet: 'text-violet-600 dark:text-violet-300',
  cyan: 'text-cyan-600 dark:text-cyan-300',
  slate: 'text-ui-body',
}

const toneBorder: Record<SignalTone, string> = {
  blue: 'border-blue-300/40 bg-blue-500/10',
  emerald: 'border-emerald-300/40 bg-emerald-500/10',
  amber: 'border-amber-300/40 bg-amber-500/10',
  red: 'border-red-300/40 bg-red-500/10',
  violet: 'border-violet-300/40 bg-violet-500/10',
  cyan: 'border-cyan-300/40 bg-cyan-500/10',
  slate: 'border-ui-line bg-ui-inset',
}

const toneBar: Record<SignalTone, string> = {
  blue: 'bg-blue-500',
  emerald: 'bg-emerald-500',
  amber: 'bg-amber-500',
  red: 'bg-red-500',
  violet: 'bg-violet-500',
  cyan: 'bg-cyan-500',
  slate: 'bg-ui-control',
}

export function MonitorPanel({
  title,
  icon,
  action,
  children,
  className = '',
  bodyClassName = 'p-4',
}: {
  title: ReactNode
  icon?: ReactNode
  action?: ReactNode
  children: ReactNode
  className?: string
  bodyClassName?: string
}) {
  return (
    <section className={joinClassNames('min-w-0 overflow-hidden rounded-lg border border-ui-line bg-ui-surface text-ui-ink shadow-sm', className)}>
      <div className="flex min-h-[52px] items-center justify-between gap-3 border-b border-ui-line px-4">
        <div className="flex min-w-0 items-center gap-2">
          {icon ? <span className="shrink-0 text-blue-500 dark:text-blue-300">{icon}</span> : null}
          <h3 className="truncate text-base font-semibold">{title}</h3>
        </div>
        {action ? <div className="shrink-0">{action}</div> : null}
      </div>
      <div className={bodyClassName}>{children}</div>
    </section>
  )
}

export function StatusTile({
  label,
  value,
  detail,
  icon,
  tone = 'blue',
  className = '',
}: {
  label: string
  value: ReactNode
  detail: ReactNode
  icon: ReactNode
  tone?: SignalTone
  className?: string
}) {
  return (
    <div className={joinClassNames('min-w-0 rounded-lg border px-4 py-3', toneBorder[tone], className)}>
      <div className="flex min-w-0 items-center justify-between gap-3">
        <div className="truncate text-sm font-medium text-ui-muted">{label}</div>
        <div className={joinClassNames('flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border', toneBorder[tone], toneText[tone])}>
          {icon}
        </div>
      </div>
      <div className="mt-3 truncate text-3xl font-semibold text-ui-ink" title={String(value)}>{value}</div>
      <div className="mt-1 truncate text-xs text-ui-muted" title={typeof detail === 'string' ? detail : undefined}>{detail}</div>
    </div>
  )
}

export function MiniSparkline({
  values,
  tone = 'blue',
  className = '',
}: {
  values: Array<number | null>
  tone?: SignalTone
  className?: string
}) {
  const recent = values.slice(-36)
  const safeValues = recent.filter((value): value is number => value != null && Number.isFinite(value))
  const max = Math.max(...safeValues, 1)
  const min = Math.min(...safeValues, 0)
  const range = Math.max(max - min, 1)
  const width = 140
  const height = 36
  const segments: { x: number; y: number }[][] = []
  let segment: { x: number; y: number }[] = []
  recent.forEach((value, index) => {
    if (value == null || !Number.isFinite(value)) {
      segment = []
      return
    }
    if (!segment.length) segments.push(segment)
    const x = recent.length === 1 ? width - 3 : 3 + (index / (recent.length - 1)) * (width - 6)
    const y = height - ((value - min) / range) * (height - 6) - 3
    segment.push({ x, y })
  })

  return (
    <svg viewBox={`0 0 ${width} ${height}`} className={joinClassNames('h-9 w-full', toneText[tone], className)} aria-hidden="true">
      {segments.map((run, index) => run.length === 1 ? <circle key={index} cx={run[0].x} cy={run[0].y} r="2.5" fill="currentColor" />
        : <polyline key={index} points={run.map(point => `${point.x},${point.y}`).join(' ')} fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />)}
    </svg>
  )
}

export function SignalMeter({
  label,
  value,
  detail,
  tone = 'blue',
  icon,
  sparkline = [],
  compact = false,
}: {
  label: string
  value: number | null
  detail: string
  tone?: SignalTone
  icon?: ReactNode
  sparkline?: Array<number | null>
  compact?: boolean
}) {
  const available = value != null && Number.isFinite(value)
  const safeValue = clampPercent(available ? value : 0)
  return (
    <div className={joinClassNames('min-w-0 rounded-lg border border-ui-line bg-ui-inset p-3', compact ? 'space-y-2' : 'space-y-3')}>
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-3">
          {icon ? <div className={joinClassNames('flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border', toneBorder[tone], toneText[tone])}>{icon}</div> : null}
          <div className="truncate text-sm font-semibold text-ui-ink">{label}</div>
        </div>
        <div className="shrink-0 text-2xl font-semibold text-ui-ink">{available ? `${Math.round(safeValue)}%` : '--'}</div>
      </div>
      <div className="min-h-8 break-words text-xs leading-4 text-ui-muted" title={detail}>{detail}</div>
      <div className="grid min-w-0 grid-cols-[minmax(32px,1fr)_minmax(80px,120px)] items-center gap-3">
        <div className="h-2 overflow-hidden rounded-full bg-ui-line">
          <div className={joinClassNames('h-full rounded-full transition-[width]', toneBar[tone])} style={{ width: `${safeValue}%` }} />
        </div>
        <MiniSparkline values={sparkline} tone={tone} />
      </div>
    </div>
  )
}

export function TrendChart({
  values,
  points,
  rangeStart,
  rangeEnd,
  emptyText,
  tone = 'blue',
  className = '',
  unit,
  valueFormatter = formatAxisValue,
  kind = 'line',
  sharedTimeAxis = false,
  partialLabel,
}: {
  values?: Array<number | null>
  points?: Array<{ ts: number; value: number | null; state?: string; covered?: number; expected?: number; label?: string }>
  rangeStart?: number
  rangeEnd?: number
  emptyText: string
  tone?: SignalTone
  className?: string
  unit?: string
  valueFormatter?: (value: number) => string
  kind?: 'bars' | 'line' | 'points'
  sharedTimeAxis?: boolean
  partialLabel?: string
}) {
  const plot = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(900)
  const chartPoints: NonNullable<typeof points> = points
    ? [...points].filter(point => Number.isFinite(point.ts)).sort((left, right) => left.ts - right.ts)
    : (values || []).map((value, index) => ({ ts: index, value }))
  const safeValues = chartPoints
    .map(point => point.state === 'idle' ? null : point.value)
    .filter((value): value is number => value != null && Number.isFinite(value))
  const empty = !safeValues.length && !sharedTimeAxis
  useEffect(() => {
    if (!plot.current) return
    const observer = new ResizeObserver(([entry]) => setWidth(Math.max(1, Math.round(entry.contentRect.width))))
    observer.observe(plot.current)
    return () => observer.disconnect()
  }, [empty])
  if (empty) {
    return (
      <div className={joinClassNames('flex min-h-[220px] items-center justify-center rounded-lg border border-dashed border-ui-line text-sm text-ui-muted', className)}>
        <Activity className="mr-2 h-4 w-4" />
        {emptyText}
      </div>
    )
  }

  const height = sharedTimeAxis ? 132 : kind === 'points' ? 176 : 260
  const chartPaddingTop = 18
  const chartPaddingBottom = points && !sharedTimeAxis ? 30 : 12
  const axis = buildChartAxis(safeValues)
  const chartHeight = height - chartPaddingTop - chartPaddingBottom
  const domainStart = rangeStart ?? chartPoints[0]?.ts ?? 0
  const domainEnd = Math.max(rangeEnd ?? chartPoints[chartPoints.length - 1]?.ts ?? domainStart + 1, domainStart + 1)
  const segments: string[][] = []
  const bars = new Map<number, { x: number; y: number }>()
  const dots: { x: number; y: number; label?: string }[] = []
  const partials: { x: number; y: number; detail: string }[] = []
  const yAt = (value: number) => height - chartPaddingBottom - (value / axis.max) * chartHeight
  let segment: string[] = []
  for (const point of chartPoints) {
    if (point.value == null || !Number.isFinite(point.value) || point.state === 'idle' || point.state === 'partial') {
      if (segment.length) segments.push(segment)
      segment = []
      if (point.state !== 'partial' || point.value == null || !Number.isFinite(point.value)) continue
    }
    const x = Math.max(0, Math.min(width, ((point.ts - domainStart) / (domainEnd - domainStart)) * width))
    const y = yAt(point.value)
    if (point.state === 'partial') partials.push({ x, y, detail: `${formatTimeLabel(point.ts)} · ${partialLabel ?? ''} ${point.covered}/${point.expected} · ${point.value} tok/s` })
    else if (kind === 'points') dots.push({ x, y, label: point.label })
    else if (kind === 'bars') {
      const column = Math.floor(x / 4)
      if (y < (bars.get(column)?.y ?? Infinity)) bars.set(column, { x, y })
    } else segment.push(`${x},${y}`)
  }
  if (segment.length) segments.push(segment)
  const descendingTicks = [...axis.ticks].reverse()

  return (
    <div data-chart-kind={kind} className={joinClassNames('grid grid-cols-[3rem_minmax(0,1fr)] overflow-hidden rounded-lg border border-ui-line bg-ui-inset', className)}>
      <div className="relative text-right font-mono text-[11px] tabular-nums text-ui-muted" aria-hidden="true">
        {descendingTicks.map(tick => <span key={tick} data-tick={tick} className="absolute right-2" style={{ top: yAt(tick), transform: 'translateY(-50%)' }}>{valueFormatter(tick)}</span>)}
      </div>
      <div ref={plot} className="relative min-w-0 border-l border-ui-line">
        {unit ? <span className="absolute right-3 top-2 z-10 rounded bg-ui-inset px-1 text-[11px] text-ui-muted">{unit}</span> : null}
        <svg viewBox={`0 0 ${width} ${height}`} style={{ height }} className={joinClassNames('w-full', toneText[tone])} preserveAspectRatio="none" aria-hidden="true">
          {axis.ticks.map(tick => {
            const y = yAt(tick)
            return <line key={tick} x1="0" y1={y} x2={width} y2={y} stroke="currentColor" className="text-ui-body" strokeWidth="1" strokeDasharray="5 7" />
          })}
          {dots.map((point, index) => (
            <circle key={index} cx={point.x} cy={point.y} r={4} fill="currentColor">
              <title>{point.label}</title>
            </circle>
          ))}
          {[...bars].map(([column, point]) => <rect key={column} x={Math.min(width - 3.5, Math.max(0, point.x - 1.75))} y={point.y} width="3.5" height={Math.max(1, yAt(0) - point.y)} fill="currentColor" />)}
          {segments.map((line, index) => line.length > 1
            ? <polyline key={index} points={line.join(' ')} fill="none" stroke="currentColor" vectorEffect="non-scaling-stroke" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
            : <circle key={index} cx={line[0].split(',')[0]} cy={line[0].split(',')[1]} r="3.5" fill="currentColor" />)}
          {partials.map((point, index) => <circle key={index} data-partial cx={point.x} cy={point.y} r="3" fill="none" stroke="currentColor" vectorEffect="non-scaling-stroke"><title>{point.detail}</title></circle>)}
        </svg>
        {!safeValues.length && <span className="absolute inset-0 flex items-center justify-center text-xs text-ui-muted">{emptyText}</span>}
        {points && !sharedTimeAxis ? (
          <div className="pointer-events-none absolute inset-x-2 bottom-1 flex justify-between font-mono text-[10px] tabular-nums text-ui-muted" aria-hidden="true">
            <span>{formatTimeLabel(domainStart)}</span>
            <span>{formatTimeLabel(domainEnd)}</span>
          </div>
        ) : null}
      </div>
    </div>
  )
}

function formatTimeLabel(timestamp: number) {
  return new Date(timestamp).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

function formatAxisValue(value: number) {
  if (Math.abs(value) >= 1000) {
    const scaled = value / 1000
    return `${scaled.toFixed(Math.abs(scaled) >= 10 ? 0 : 1).replace(/\.0$/, '')}k`
  }
  if (Math.abs(value) >= 100) return Math.round(value).toString()
  if (Math.abs(value) >= 10) return value.toFixed(1).replace(/\.0$/, '')
  return value.toFixed(2).replace(/\.00$/, '').replace(/(\.\d)0$/, '$1')
}

export function ActivityRow({ item }: { item: ActivityFeedItem }) {
  const tone = item.severity === 'critical' ? 'red' : item.severity === 'warning' ? 'amber' : item.severity === 'success' ? 'emerald' : 'blue'
  return (
    <div className="grid min-w-0 grid-cols-[58px_76px_minmax(0,1fr)] items-center gap-3 border-b border-ui-line px-2 py-2 last:border-b-0">
      <span className="text-xs text-ui-muted">{new Date(item.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</span>
      <Badge tone={tone}>{item.label}</Badge>
      <div className="min-w-0">
        <div className="truncate text-sm font-medium text-ui-ink" title={item.title}>{item.title}</div>
        <div className="truncate text-xs text-ui-muted" title={item.detail}>{item.detail}</div>
      </div>
    </div>
  )
}

export function ActiveRequestRow({
  task,
  instanceName,
  workload,
  workloadText,
}: {
  task: RunningInferenceTask
  instanceName?: string
  workload: ModelWorkload
  workloadText: string
}) {
  const elapsed = formatDuration((Date.now() - task.started_at_ms) / 1000)
  if (workload !== 'inference') {
    return (
      <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_100px_82px] items-center gap-3 rounded-lg border border-ui-line bg-ui-inset px-3 py-2 text-sm">
        <div className="min-w-0">
          <div className="truncate font-medium text-ui-ink" title={instanceName}>#{task.task_id} {instanceName ? `· ${instanceName}` : ''}</div>
          <div className="text-xs text-ui-muted">slot {task.slot_id}</div>
        </div>
        <Badge tone={workload === 'reranker' ? 'blue' : 'violet'}>{workloadText}</Badge>
        <div className="truncate text-right text-ui-muted">{elapsed}</div>
      </div>
    )
  }
  return (
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_80px_92px_82px] items-center gap-3 rounded-lg border border-ui-line bg-ui-inset px-3 py-2 text-sm">
      <div className="min-w-0">
        <div className="truncate font-medium text-ui-ink" title={instanceName}>#{task.task_id} {instanceName ? `· ${instanceName}` : ''}</div>
        <div className="text-xs text-ui-muted">slot {task.slot_id}</div>
      </div>
      <div className="truncate text-ui-body">{formatCompactNumber(task.n_decoded)}</div>
      <div className="truncate text-emerald-600 dark:text-emerald-300">{formatRate(task.tg_3s ?? task.tg)}</div>
      <div className="truncate text-ui-muted">{elapsed}</div>
    </div>
  )
}

export function SessionCard({
  title,
  subtitle,
  meta,
  selected,
  running,
  statusLabel,
  workload,
  workloadTone = 'blue',
  onClick,
}: {
  title: string
  subtitle: string
  meta: string
  selected: boolean
  running: boolean
  statusLabel: string
  workload?: string
  workloadTone?: 'slate' | 'blue' | 'emerald' | 'amber' | 'red' | 'violet'
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={selected}
      data-selected={selected}
      className="console-list-row block w-full rounded-lg border border-ui-line p-3 text-left"
    >
      <div className="flex min-w-0 items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="truncate text-sm font-semibold text-ui-ink" title={title}>{title}</div>
          <div className="mt-1 truncate text-xs text-ui-muted" title={subtitle}>{subtitle}</div>
        </div>
        <div className="flex shrink-0 flex-col items-end gap-1.5">
          <Badge tone={running ? 'emerald' : 'slate'}>{statusLabel}</Badge>
          {workload ? <Badge tone={workloadTone}>{workload}</Badge> : null}
        </div>
      </div>
      <div className="mt-3 truncate text-xs text-ui-muted" title={meta}>{meta}</div>
    </button>
  )
}

export function ComparisonTable({
  rows,
}: {
  rows: Array<{ metric: string; current: string; baseline: string; delta?: string; tone?: SignalTone }>
}) {
  if (rows.length === 0) {
    return (
      <div className="flex min-h-[110px] items-center justify-center rounded-lg border border-dashed border-ui-line text-sm text-ui-muted">
        <CheckCircle2 className="mr-2 h-4 w-4" />
        --
      </div>
    )
  }

  return (
    <div className="overflow-hidden rounded-lg border border-ui-line">
      <table className="w-full table-fixed text-left text-sm">
        <tbody className="divide-y divide-ui-line">
          {rows.map(row => (
            <tr key={row.metric}>
              <td className="px-3 py-2 text-xs text-ui-muted">{row.metric}</td>
              <td className="px-3 py-2 font-medium text-ui-ink">{row.current}</td>
              <td className="px-3 py-2 text-ui-body">{row.baseline}</td>
              <td className={joinClassNames('px-3 py-2 text-right text-xs font-medium', toneText[row.tone || 'slate'])}>{row.delta || '--'}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
