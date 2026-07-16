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
    slate: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300',
  }
  return (
    <div className={`${surfaceClassName} min-w-0 px-4 py-4`}>
      <div className={`inline-flex rounded-full px-2.5 py-1 text-[11px] font-medium ${tones[tone]}`}>{label}</div>
      <div className="mt-3 truncate text-2xl font-semibold text-slate-900 dark:text-slate-100" title={String(value)}>{value}</div>
      {detail && <div className="mt-1 truncate text-xs text-slate-500 dark:text-slate-400" title={detail}>{detail}</div>}
    </div>
  )
}

export function StatusBadge({ status, label }: { status: DownloadProgress['status']; label: string }) {
  const colors: Record<string, string> = {
    active: 'bg-blue-100 text-blue-700 dark:bg-blue-950/50 dark:text-blue-300', paused: 'bg-amber-100 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300',
    pausing: 'bg-amber-100 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300', queued: 'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300',
    completed: 'bg-emerald-100 text-emerald-700 dark:bg-emerald-950/50 dark:text-emerald-300', cancelled: 'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300',
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
      <button type="button" onClick={onToggle} className="flex w-full items-center justify-between gap-3 border-b border-slate-200 px-4 py-3 text-left dark:border-slate-800">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-sm font-semibold text-slate-900 dark:text-slate-100">{title}</span>
          <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[11px] text-slate-500 dark:bg-slate-800 dark:text-slate-400">{count}</span>
        </div>
        <div className="flex shrink-0 items-center gap-2" onClick={event => event.stopPropagation()}>
          {extra}
          {collapsed ? <ChevronDown className="h-4 w-4 text-slate-400" /> : <ChevronUp className="h-4 w-4 text-slate-400" />}
        </div>
      </button>
      {!collapsed && children}
    </section>
  )
}
