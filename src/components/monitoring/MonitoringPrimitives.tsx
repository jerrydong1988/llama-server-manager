import type { ReactNode } from 'react'
import { Activity, CheckCircle2 } from 'lucide-react'
import type { RunningInferenceTask } from '../../store/types'
import { Badge, joinClassNames } from '../ui'
import { clampPercent, formatCompactNumber, formatDuration, formatRate } from './monitoringFormat'
import type { ActivityFeedItem, SignalTone } from './monitoringViewModel'

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
  emptyText,
  tone = 'blue',
  className = '',
}: {
  values: number[]
  emptyText: string
  tone?: SignalTone
  className?: string
}) {
  const safeValues = values.filter(value => Number.isFinite(value))
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
  const max = Math.max(...safeValues, 1)
  const min = Math.min(...safeValues, 0)
  const range = Math.max(max - min, 1)
  const points = safeValues.map((value, index) => {
    const x = safeValues.length === 1 ? 0 : (index / (safeValues.length - 1)) * width
    const y = height - ((value - min) / range) * (height - 36) - 18
    return `${x},${y}`
  }).join(' ')

  return (
    <div className={joinClassNames('min-h-[260px] overflow-hidden rounded-lg border border-slate-200 bg-slate-50 dark:border-slate-800 dark:bg-slate-950/40', className)}>
      <svg viewBox={`0 0 ${width} ${height}`} className={joinClassNames('h-[260px] w-full', toneText[tone])} preserveAspectRatio="none">
        {[0.25, 0.5, 0.75].map(ratio => (
          <line key={ratio} x1="0" y1={height * ratio} x2={width} y2={height * ratio} stroke="currentColor" className="text-slate-300 dark:text-slate-800" strokeWidth="1" strokeDasharray="5 7" />
        ))}
        <polyline points={points} fill="none" stroke="currentColor" strokeWidth="4" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    </div>
  )
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
}: {
  task: RunningInferenceTask
  instanceName?: string
}) {
  return (
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_80px_92px_82px] items-center gap-3 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-sm dark:border-slate-800 dark:bg-slate-950/60">
      <div className="min-w-0">
        <div className="truncate font-medium text-slate-900 dark:text-slate-100" title={instanceName}>#{task.task_id} {instanceName ? `· ${instanceName}` : ''}</div>
        <div className="text-xs text-slate-500">slot {task.slot_id}</div>
      </div>
      <div className="truncate text-slate-600 dark:text-slate-300">{formatCompactNumber(task.n_decoded)}</div>
      <div className="truncate text-emerald-600 dark:text-emerald-300">{formatRate(task.tg)}</div>
      <div className="truncate text-slate-500">{formatDuration((Date.now() - task.started_at_ms) / 1000)}</div>
    </div>
  )
}

export function SessionCard({
  title,
  subtitle,
  meta,
  selected,
  running,
  onClick,
}: {
  title: string
  subtitle: string
  meta: string
  selected: boolean
  running: boolean
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
        <Badge tone={running ? 'emerald' : 'slate'}>{running ? '运行中' : '已结束'}</Badge>
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
