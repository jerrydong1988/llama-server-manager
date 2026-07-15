import type { ReactNode } from 'react'
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
  slate: 'text-slate-600 dark:text-slate-300',
}

const toneBorder: Record<SignalTone, string> = {
  blue: 'border-blue-300/40 bg-blue-500/10',
  emerald: 'border-emerald-300/40 bg-emerald-500/10',
  amber: 'border-amber-300/40 bg-amber-500/10',
  red: 'border-red-300/40 bg-red-500/10',
  violet: 'border-violet-300/40 bg-violet-500/10',
  cyan: 'border-cyan-300/40 bg-cyan-500/10',
  slate: 'border-slate-300 bg-slate-100 dark:border-slate-700 dark:bg-slate-800/70',
}

const toneBar: Record<SignalTone, string> = {
  blue: 'bg-blue-500',
  emerald: 'bg-emerald-500',
  amber: 'bg-amber-500',
  red: 'bg-red-500',
  violet: 'bg-violet-500',
  cyan: 'bg-cyan-500',
  slate: 'bg-slate-500',
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
    <section className={joinClassNames('min-w-0 overflow-hidden rounded-lg border border-slate-200 bg-white text-slate-900 shadow-sm dark:border-slate-800 dark:bg-slate-900/70 dark:text-slate-100', className)}>
      <div className="flex min-h-[52px] items-center justify-between gap-3 border-b border-slate-200 px-4 dark:border-slate-800">
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
        <div className="truncate text-sm font-medium text-slate-600 dark:text-slate-400">{label}</div>
        <div className={joinClassNames('flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border', toneBorder[tone], toneText[tone])}>
          {icon}
        </div>
      </div>
      <div className="mt-3 truncate text-3xl font-semibold text-slate-950 dark:text-white" title={String(value)}>{value}</div>
      <div className="mt-1 truncate text-xs text-slate-500 dark:text-slate-400" title={typeof detail === 'string' ? detail : undefined}>{detail}</div>
    </div>
  )
}

export function MiniSparkline({
  values,
  tone = 'blue',
  className = '',
}: {
  values: number[]
  tone?: SignalTone
  className?: string
}) {
  const safeValues = values.length > 1 ? values.slice(-36) : [0, 0]
  const max = Math.max(...safeValues, 1)
  const min = Math.min(...safeValues, 0)
  const range = Math.max(max - min, 1)
  const width = 140
  const height = 36
  const points = safeValues.map((value, index) => {
    const x = safeValues.length === 1 ? 0 : (index / (safeValues.length - 1)) * width
    const y = height - ((value - min) / range) * (height - 6) - 3
    return `${x},${y}`
  }).join(' ')

  return (
    <svg viewBox={`0 0 ${width} ${height}`} className={joinClassNames('h-9 w-full', toneText[tone], className)} aria-hidden="true">
      <polyline points={points} fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
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
  value: number
  detail: string
  tone?: SignalTone
  icon?: ReactNode
  sparkline?: number[]
  compact?: boolean
}) {
  const safeValue = clampPercent(value)
  return (
    <div className={joinClassNames('min-w-0 rounded-lg border border-slate-200 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-950/60', compact ? 'space-y-2' : 'space-y-3')}>
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-3">
          {icon ? <div className={joinClassNames('flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border', toneBorder[tone], toneText[tone])}>{icon}</div> : null}
          <div className="min-w-0">
            <div className="truncate text-sm font-semibold text-slate-900 dark:text-slate-100">{label}</div>
            <div className="truncate text-xs text-slate-500 dark:text-slate-400" title={detail}>{detail}</div>
          </div>
        </div>
        <div className="shrink-0 text-2xl font-semibold text-slate-950 dark:text-white">{Math.round(safeValue)}%</div>
      </div>
      <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_120px] items-center gap-3">
        <div className="h-2 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-800">
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
}: {
  values?: Array<number | null>
  points?: Array<{ ts: number; value: number | null }>
  rangeStart?: number
  rangeEnd?: number
  emptyText: string
  tone?: SignalTone
  className?: string
  unit?: string
  valueFormatter?: (value: number) => string
}) {
  const chartPoints = points
    ? [...points].filter(point => Number.isFinite(point.ts)).sort((left, right) => left.ts - right.ts)
    : (values || []).map((value, index) => ({ ts: index, value }))
  const safeValues = chartPoints
    .map(point => point.value)
    .filter((value): value is number => value != null && Number.isFinite(value))
  if (safeValues.length < 2) {
    return (
      <div className={joinClassNames('flex min-h-[220px] items-center justify-center rounded-lg border border-dashed border-slate-300 text-sm text-slate-500 dark:border-slate-700 dark:text-slate-400', className)}>
        <Activity className="mr-2 h-4 w-4" />
        {emptyText}
      </div>
    )
  }

  const width = 900
  const height = 260
  const chartPaddingTop = 18
  const chartPaddingBottom = points ? 30 : 18
  const axis = buildChartAxis(safeValues)
  const chartHeight = height - chartPaddingTop - chartPaddingBottom
  const domainStart = rangeStart ?? chartPoints[0]?.ts ?? 0
  const domainEnd = Math.max(rangeEnd ?? chartPoints[chartPoints.length - 1]?.ts ?? domainStart + 1, domainStart + 1)
  const segments: string[] = []
  let segment: string[] = []
  for (const point of chartPoints) {
    if (point.value == null || !Number.isFinite(point.value)) {
      if (segment.length > 1) segments.push(segment.join(' '))
      segment = []
      continue
    }
    const x = Math.max(0, Math.min(width, ((point.ts - domainStart) / (domainEnd - domainStart)) * width))
    const y = height - chartPaddingBottom - (point.value / axis.max) * chartHeight
    segment.push(`${x},${y}`)
  }
  if (segment.length > 1) segments.push(segment.join(' '))
  const descendingTicks = [...axis.ticks].reverse()

  return (
    <div className={joinClassNames('grid min-h-[260px] grid-cols-[68px_minmax(0,1fr)] overflow-hidden rounded-lg border border-slate-200 bg-slate-50 dark:border-slate-800 dark:bg-slate-950/40', className)}>
      <div className="flex h-[260px] flex-col justify-between py-[10px] pr-2 text-right font-mono text-[11px] tabular-nums text-slate-500 dark:text-slate-400" aria-hidden="true">
        {descendingTicks.map(tick => <span key={tick}>{valueFormatter(tick)}</span>)}
      </div>
      <div className="relative min-w-0 border-l border-slate-200 dark:border-slate-800">
        {unit ? <span className="absolute right-3 top-2 z-10 rounded bg-slate-50/90 px-1 text-[11px] text-slate-500 dark:bg-slate-950/80 dark:text-slate-400">{unit}</span> : null}
        <svg viewBox={`0 0 ${width} ${height}`} className={joinClassNames('h-[260px] w-full', toneText[tone])} preserveAspectRatio="none" aria-hidden="true">
          {axis.ticks.map(tick => {
            const y = height - chartPaddingBottom - (tick / axis.max) * chartHeight
            return <line key={tick} x1="0" y1={y} x2={width} y2={y} stroke="currentColor" className="text-slate-300 dark:text-slate-800" strokeWidth="1" strokeDasharray="5 7" />
          })}
          {segments.map((line, index) => (
            <polyline key={`${index}-${line.slice(0, 16)}`} points={line} fill="none" stroke="currentColor" strokeWidth="4" strokeLinecap="round" strokeLinejoin="round" />
          ))}
        </svg>
        {points ? (
          <div className="pointer-events-none absolute inset-x-2 bottom-1 flex justify-between font-mono text-[10px] tabular-nums text-slate-500 dark:text-slate-400" aria-hidden="true">
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
    <div className="grid min-w-0 grid-cols-[58px_76px_minmax(0,1fr)] items-center gap-3 border-b border-slate-200 px-2 py-2 last:border-b-0 dark:border-slate-800">
      <span className="text-xs text-slate-500">{new Date(item.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</span>
      <Badge tone={tone}>{item.label}</Badge>
      <div className="min-w-0">
        <div className="truncate text-sm font-medium text-slate-900 dark:text-slate-100" title={item.title}>{item.title}</div>
        <div className="truncate text-xs text-slate-500" title={item.detail}>{item.detail}</div>
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
      <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_100px_82px] items-center gap-3 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-sm dark:border-slate-800 dark:bg-slate-950/60">
        <div className="min-w-0">
          <div className="truncate font-medium text-slate-900 dark:text-slate-100" title={instanceName}>#{task.task_id} {instanceName ? `· ${instanceName}` : ''}</div>
          <div className="text-xs text-slate-500">slot {task.slot_id}</div>
        </div>
        <Badge tone={workload === 'reranker' ? 'blue' : 'violet'}>{workloadText}</Badge>
        <div className="truncate text-right text-slate-500">{elapsed}</div>
      </div>
    )
  }
  return (
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_80px_92px_82px] items-center gap-3 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-sm dark:border-slate-800 dark:bg-slate-950/60">
      <div className="min-w-0">
        <div className="truncate font-medium text-slate-900 dark:text-slate-100" title={instanceName}>#{task.task_id} {instanceName ? `· ${instanceName}` : ''}</div>
        <div className="text-xs text-slate-500">slot {task.slot_id}</div>
      </div>
      <div className="truncate text-slate-600 dark:text-slate-300">{formatCompactNumber(task.n_decoded)}</div>
      <div className="truncate text-emerald-600 dark:text-emerald-300">{formatRate(task.tg_3s ?? task.tg)}</div>
      <div className="truncate text-slate-500">{elapsed}</div>
    </div>
  )
}

export function SessionCard({
  title,
  subtitle,
  meta,
  selected,
  running,
  workload,
  workloadTone = 'blue',
  onClick,
}: {
  title: string
  subtitle: string
  meta: string
  selected: boolean
  running: boolean
  workload?: string
  workloadTone?: 'slate' | 'blue' | 'emerald' | 'amber' | 'red' | 'violet'
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={joinClassNames(
        'block w-full rounded-lg border p-3 text-left transition',
        selected
          ? 'border-blue-400 bg-blue-50 shadow-[inset_0_0_0_1px_rgba(59,130,246,0.22)] dark:border-blue-500/50 dark:bg-blue-500/20'
          : 'border-slate-200 bg-white hover:border-slate-300 hover:bg-slate-50 dark:border-slate-800 dark:bg-slate-950/60 dark:hover:border-slate-700',
      )}
    >
      <div className="flex min-w-0 items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="truncate text-sm font-semibold text-slate-950 dark:text-slate-100" title={title}>{title}</div>
          <div className="mt-1 truncate text-xs text-slate-500" title={subtitle}>{subtitle}</div>
        </div>
        <div className="flex shrink-0 flex-col items-end gap-1.5">
          <Badge tone={running ? 'emerald' : 'slate'}>{running ? '运行中' : '已结束'}</Badge>
          {workload ? <Badge tone={workloadTone}>{workload}</Badge> : null}
        </div>
      </div>
      <div className="mt-3 truncate text-xs text-slate-500" title={meta}>{meta}</div>
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
      <div className="flex min-h-[110px] items-center justify-center rounded-lg border border-dashed border-slate-300 text-sm text-slate-500 dark:border-slate-700">
        <CheckCircle2 className="mr-2 h-4 w-4" />
        --
      </div>
    )
  }

  return (
    <div className="overflow-hidden rounded-lg border border-slate-200 dark:border-slate-800">
      <table className="w-full table-fixed text-left text-sm">
        <tbody className="divide-y divide-slate-200 dark:divide-slate-800">
          {rows.map(row => (
            <tr key={row.metric}>
              <td className="px-3 py-2 text-xs text-slate-500">{row.metric}</td>
              <td className="px-3 py-2 font-medium text-slate-900 dark:text-slate-100">{row.current}</td>
              <td className="px-3 py-2 text-slate-600 dark:text-slate-300">{row.baseline}</td>
              <td className={joinClassNames('px-3 py-2 text-right text-xs font-medium', toneText[row.tone || 'slate'])}>{row.delta || '--'}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
