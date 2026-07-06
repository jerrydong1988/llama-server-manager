import { useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { useI18n } from '../../i18n'

interface TaskPerfState {
  slot_id: number
  task_id: number
  n_decoded: number
  tg: number
  history: [number, number][]
  prompt_tokens: number | null
  prompt_time_ms: number | null
  prompt_tps: number | null
  gen_tokens: number | null
  gen_time_ms: number | null
  gen_tps: number | null
  total_tokens: number | null
  total_time_ms: number | null
  spec_accept_rate: number | null
  spec_accepted: number | null
  spec_generated: number | null
  spec_gen_time_ms: number | null
  completed: boolean
}

interface PerfUpdate {
  instanceId: string
  tasks: TaskPerfState[]
  lastCompleted: TaskPerfState | null
}

export default function PerfAnalysis({ instanceId }: { instanceId: string }) {
  const { t } = useI18n()
  const [tasks, setTasks] = useState<TaskPerfState[]>([])
  const [lastCompleted, setLastCompleted] = useState<TaskPerfState | null>(null)

  useEffect(() => {
    const unlisten = listen<PerfUpdate>('perf-update', event => {
      if (event.payload.instanceId !== instanceId) {
        return
      }
      setTasks(event.payload.tasks || [])
      if (event.payload.lastCompleted) {
        setLastCompleted(event.payload.lastCompleted)
      }
    })

    return () => {
      unlisten.then(fn => fn())
    }
  }, [instanceId])

  if (tasks.length === 0 && !lastCompleted) {
    return (
      <div className="rounded-lg border border-slate-800 bg-slate-900/70 p-5">
        <div className="mb-3 text-xs uppercase tracking-[0.14em] text-slate-500">{t.perfBlock.perfTitle}</div>
        <div className="py-6 text-center text-sm text-slate-400">{t.perfBlock.waitingActivity}</div>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      {tasks.map(task => (
        <div key={task.task_id} className="rounded-lg border border-slate-800 bg-slate-900/70 p-5">
          <div className="mb-3 flex items-center justify-between">
            <div className="text-xs text-slate-500">
              {t.perfBlock.task} {task.task_id} · {t.perfBlock.slot} {task.slot_id}
              <span className="ml-2 inline-block h-2 w-2 rounded-full bg-green-500 animate-pulse" />
            </div>
            <div className="text-xs font-mono font-medium text-orange-400">tg {task.tg.toFixed(1)} t/s</div>
          </div>

          {task.gen_tokens != null && task.gen_tokens > 0 ? (
            <div className="mb-3">
              <div className="mb-1 flex justify-between text-xs text-slate-400">
                <span>
                  {t.perfBlock.generated}: {task.n_decoded.toLocaleString()} / ~{task.gen_tokens.toLocaleString()} {t.perfBlock.tokens}
                </span>
                <span>{((task.n_decoded / task.gen_tokens) * 100).toFixed(1)}%</span>
              </div>
              <div className="h-2 overflow-hidden rounded-full bg-slate-800">
                <div
                  className="h-full rounded-full bg-gradient-to-r from-blue-400 to-blue-600 transition-all duration-700"
                  style={{ width: `${Math.min(100, (task.n_decoded / task.gen_tokens) * 100)}%` }}
                />
              </div>
            </div>
          ) : (
            <div className="mb-3 text-xs text-slate-400">
              {t.perfBlock.generated}: {task.n_decoded.toLocaleString()} {t.perfBlock.tokens}
            </div>
          )}

          {task.history.length >= 3 && <SpeedCurve history={task.history} title={t.perfBlock.speedCurve} />}

          {task.spec_accept_rate != null && (
            <div className="mt-2 text-xs text-indigo-300">
              {t.perfBlock.specAccept}: {(task.spec_accept_rate * 100).toFixed(1)}%
              {task.spec_accepted != null && task.spec_generated != null && (
                <span className="ml-1 text-slate-500">
                  ({task.spec_accepted.toLocaleString()}/{task.spec_generated.toLocaleString()})
                </span>
              )}
            </div>
          )}
        </div>
      ))}

      {lastCompleted && (
        <div className="rounded-lg border border-slate-800 bg-slate-900/70 p-5">
          <div className="mb-3 text-xs uppercase tracking-[0.14em] text-slate-500">
            {t.perfBlock.lastCompleted} · {t.perfBlock.task} {lastCompleted.task_id}
          </div>
          <div className="grid grid-cols-2 gap-2 text-xs">
            {lastCompleted.prompt_tokens != null && (
              <>
                <Stat label={t.perfBlock.prompt} value={`${lastCompleted.prompt_tokens.toLocaleString()} ${t.perfBlock.tokens}`} />
                <Stat label={t.perfBlock.promptSpeed} value={`${lastCompleted.prompt_tps?.toFixed(1) || '--'} t/s`} color="text-amber-300" />
              </>
            )}
            {lastCompleted.gen_tokens != null && (
              <>
                <Stat label={t.perfBlock.generated} value={`${lastCompleted.gen_tokens.toLocaleString()} ${t.perfBlock.tokens}`} />
                <Stat label={t.perfBlock.genSpeed} value={`${lastCompleted.gen_tps?.toFixed(1) || '--'} t/s`} color="text-orange-300" />
              </>
            )}
            {lastCompleted.total_tokens != null && (
              <Stat label={t.perfBlock.total} value={`${lastCompleted.total_tokens.toLocaleString()} ${t.perfBlock.tokens}`} />
            )}
            {lastCompleted.total_time_ms != null && (
              <Stat label={t.perfBlock.totalTime} value={fmtMs(lastCompleted.total_time_ms)} />
            )}
            {lastCompleted.spec_accept_rate != null && (
              <Stat label={t.perfBlock.specAccept} value={`${(lastCompleted.spec_accept_rate * 100).toFixed(1)}%`} color="text-indigo-300" />
            )}
            {lastCompleted.spec_gen_time_ms != null && (
              <Stat label={t.perfBlock.specGenTime} value={fmtMs(lastCompleted.spec_gen_time_ms)} />
            )}
          </div>
        </div>
      )}
    </div>
  )
}

function SpeedCurve({ history, title }: { history: [number, number][]; title: string }) {
  const width = 320
  const height = 70
  const pad = { top: 8, right: 4, bottom: 14, left: 4 }
  const plotWidth = width - pad.left - pad.right
  const plotHeight = height - pad.top - pad.bottom

  const speeds = history.map(item => item[1])
  const min = Math.min(...speeds)
  const max = Math.max(...speeds)
  const range = max - min || 1

  const points = history.map(([decoded, speed]) => {
    const x = pad.left + (decoded / history[history.length - 1][0]) * plotWidth
    const y = pad.top + plotHeight - ((speed - min) / range) * plotHeight
    return `${x},${y}`
  }).join(' ')

  return (
    <div>
      <div className="mb-1 text-xs text-slate-500">{title}</div>
      <svg viewBox={`0 0 ${width} ${height}`} className="w-full" style={{ maxWidth: width }}>
        <line x1={pad.left} y1={pad.top + plotHeight} x2={pad.left + plotWidth} y2={pad.top + plotHeight} stroke="#334155" strokeWidth="0.5" />
        <polyline points={points} fill="none" stroke="#f97316" strokeWidth="1.5" strokeLinejoin="round" />
        <text x={pad.left} y={pad.top + 3} className="fill-slate-500" fontSize="8">{max.toFixed(0)}</text>
        <text x={pad.left} y={height - 2} className="fill-slate-500" fontSize="8">{min.toFixed(0)}</text>
      </svg>
    </div>
  )
}

function Stat({ label, value, color }: { label: string; value: string; color?: string }) {
  return (
    <div className="rounded-lg border border-slate-800 bg-slate-950/50 p-3 text-center">
      <div className="text-slate-500">{label}</div>
      <div className={`font-bold text-slate-100 ${color || ''}`}>{value}</div>
    </div>
  )
}

function fmtMs(ms: number): string {
  if (ms < 1000) return `${ms.toFixed(0)}ms`
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
  return `${(ms / 60000).toFixed(1)}min`
}
