import type { ReactNode } from 'react'
import { ChevronDown, ChevronUp } from 'lucide-react'
import type { DownloadProgress } from '../../store/types'
import { surfaceClassName } from '../ui'

export function MetricTile({ label, value, detail, tone }: {
  label: string
  value: string | number
  detail?: string
  tone: 'blue' | 'emerald' | 'amber' | 'slate'
}) {
  const tones = {
    blue: 'bg-blue-50 text-blue-700 dark:bg-blue-950/40 dark:text-blue-300',
    emerald: 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300',
    amber: 'bg-amber-50 text-amber-700 dark:bg-amber-950/40 dark:text-amber-300',
    slate: 'bg-ui-inset text-ui-body',
  }
  return (
    <div className={`${surfaceClassName} min-w-0 px-4 py-4`}>
      <div className={`inline-flex rounded-full px-2.5 py-1 text-[11px] font-medium ${tones[tone]}`}>{label}</div>
      <div className="mt-3 truncate text-2xl font-semibold text-ui-ink" title={String(value)}>{value}</div>
      {detail && <div className="mt-1 truncate text-xs text-ui-muted" title={detail}>{detail}</div>}
    </div>
  )
}

export function StatusBadge({ status, label }: { status: DownloadProgress['status']; label: string }) {
  const colors: Record<string, string> = {
    active: 'bg-blue-100 text-blue-700 dark:bg-blue-950/50 dark:text-blue-300', paused: 'bg-amber-100 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300',
    pausing: 'bg-amber-100 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300', queued: 'bg-ui-inset text-ui-body',
    completed: 'bg-emerald-100 text-emerald-700 dark:bg-emerald-950/50 dark:text-emerald-300', cancelled: 'bg-ui-inset text-ui-body',
    error: 'bg-rose-100 text-rose-700 dark:bg-rose-950/50 dark:text-rose-300',
  }
  return <span className={`inline-flex shrink-0 rounded-full px-2 py-1 text-[11px] font-medium ${colors[status] || colors.queued}`}>{label}</span>
}

export function SectionPanel({ title, count, collapsed, onToggle, extra, children }: {
  title: string
  count: number
  collapsed: boolean
  onToggle: () => void
  extra?: ReactNode
  children: ReactNode
}) {
  return (
    <section className={`${surfaceClassName} min-w-0 overflow-hidden`}>
      <div className="flex w-full items-center gap-2 border-b border-ui-line px-4 py-3">
        <button type="button" onClick={onToggle} className="flex min-w-0 flex-1 items-center justify-between gap-3 text-left">
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate text-sm font-semibold text-ui-ink">{title}</span>
            <span className="rounded-full bg-ui-inset px-2 py-0.5 text-[11px] text-ui-muted">{count}</span>
          </div>
          {collapsed ? <ChevronDown className="h-4 w-4 text-ui-muted" /> : <ChevronUp className="h-4 w-4 text-ui-muted" />}
        </button>
        {extra && <div className="flex shrink-0 items-center gap-2">{extra}</div>}
      </div>
      {!collapsed && children}
    </section>
  )
}
