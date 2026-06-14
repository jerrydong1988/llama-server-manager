import { useState, useEffect } from 'react'
import { listen } from '@tauri-apps/api/event'

interface TaskPerfState {
  slot_id: number
  task_id: number
  n_decoded: number
  tg: number
  history: [number, number][]  // (n_decoded, tg)
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
  const [tasks, setTasks] = useState<TaskPerfState[]>([])
  const [lastCompleted, setLastCompleted] = useState<TaskPerfState | null>(null)

  useEffect(() => {
    const unlisten = listen<PerfUpdate>('perf-update', (event) => {
      if (event.payload.instanceId !== instanceId) return
      setTasks(event.payload.tasks || [])
      if (event.payload.lastCompleted) {
        setLastCompleted(event.payload.lastCompleted)
      }
    })
    return () => { unlisten.then(fn => fn()) }
  }, [instanceId])

  if (tasks.length === 0 && !lastCompleted) {
    return (
      <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-4 mt-4">
        <div className="text-xs text-gray-500 mb-3">Performance Analysis</div>
        <div className="text-sm text-gray-400 text-center py-6">
          Waiting for inference activity...
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-4 mt-4">
      {/* Active tasks */}
      {tasks.map(task => (
        <div key={task.task_id} className="bg-gray-50 dark:bg-gray-800 rounded-lg p-4">
          <div className="flex items-center justify-between mb-3">
            <div className="text-xs text-gray-500">
              Task {task.task_id} · Slot {task.slot_id}
              <span className="ml-2 inline-block w-2 h-2 rounded-full bg-green-500 animate-pulse" />
            </div>
            <div className="text-xs font-mono text-orange-500 font-medium">
              tg {task.tg.toFixed(1)} t/s
            </div>
          </div>

          {/* Progress bar */}
          {task.gen_tokens != null && task.gen_tokens > 0 ? (
            <div className="mb-3">
              <div className="flex justify-between text-xs text-gray-400 mb-1">
                <span>Generated: {task.n_decoded.toLocaleString()} / ~{task.gen_tokens.toLocaleString()} tokens</span>
                <span>{((task.n_decoded / task.gen_tokens) * 100).toFixed(1)}%</span>
              </div>
              <div className="h-2 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
                <div className="h-full rounded-full bg-gradient-to-r from-blue-400 to-blue-600 transition-all duration-700"
                  style={{ width: `${Math.min(100, (task.n_decoded / task.gen_tokens) * 100)}%` }} />
              </div>
            </div>
          ) : (
            <div className="mb-3 text-xs text-gray-400">
              Generated: {task.n_decoded.toLocaleString()} tokens
            </div>
          )}

          {/* Speed curve */}
          {task.history.length >= 3 && (
            <SpeedCurve history={task.history} />
          )}

          {/* Spec decode */}
          {task.spec_accept_rate != null && (
            <div className="text-xs text-indigo-500 mt-2">
              Spec Accept: {(task.spec_accept_rate * 100).toFixed(1)}%
              {task.spec_accepted != null && task.spec_generated != null && (
                <span className="text-gray-400 ml-1">
                  ({task.spec_accepted.toLocaleString()}/{task.spec_generated.toLocaleString()})
                </span>
              )}
            </div>
          )}
        </div>
      ))}

      {/* Last completed summary */}
      {lastCompleted && (
        <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-4">
          <div className="text-xs text-gray-500 mb-3">Last Completed · Task {lastCompleted.task_id}</div>
          <div className="grid grid-cols-2 gap-2 text-xs">
            {lastCompleted.prompt_tokens != null && (
              <>
                <Stat label="Prompt" value={`${lastCompleted.prompt_tokens.toLocaleString()} tok`} />
                <Stat label="Prompt Speed" value={`${lastCompleted.prompt_tps?.toFixed(1) || '—'} t/s`} color="text-amber-500" />
              </>
            )}
            {lastCompleted.gen_tokens != null && (
              <>
                <Stat label="Generated" value={`${lastCompleted.gen_tokens.toLocaleString()} tok`} />
                <Stat label="Gen Speed" value={`${lastCompleted.gen_tps?.toFixed(1) || '—'} t/s`} color="text-orange-500" />
              </>
            )}
            {lastCompleted.total_tokens != null && (
              <Stat label="Total" value={`${lastCompleted.total_tokens.toLocaleString()} tok`} />
            )}
            {lastCompleted.total_time_ms != null && (
              <Stat label="Total Time" value={fmtMs(lastCompleted.total_time_ms)} />
            )}
            {lastCompleted.spec_accept_rate != null && (
              <Stat label="Spec Accept" value={`${(lastCompleted.spec_accept_rate * 100).toFixed(1)}%`} color="text-indigo-500" />
            )}
            {lastCompleted.spec_gen_time_ms != null && (
              <Stat label="Spec Gen Time" value={fmtMs(lastCompleted.spec_gen_time_ms)} />
            )}
          </div>
        </div>
      )}
    </div>
  )
}

function SpeedCurve({ history }: { history: [number, number][] }) {
  const width = 320; const height = 70
  const pad = { top: 8, right: 4, bottom: 14, left: 4 }
  const pw = width - pad.left - pad.right; const ph = height - pad.top - pad.bottom

  const tgs = history.map(h => h[1])
  const min = Math.min(...tgs); const max = Math.max(...tgs)
  const range = max - min || 1

  const points = history.map(([n, tg]) => {
    const x = pad.left + (n / history[history.length - 1][0]) * pw
    const y = pad.top + ph - ((tg - min) / range) * ph
    return `${x},${y}`
  }).join(' ')

  return (
    <div>
      <div className="text-xs text-gray-500 mb-1">Speed Curve (t/s)</div>
      <svg viewBox={`0 0 ${width} ${height}`} className="w-full" style={{ maxWidth: width }}>
        <line x1={pad.left} y1={pad.top + ph} x2={pad.left + pw} y2={pad.top + ph} stroke="#e5e7eb" strokeWidth="0.5" className="dark:stroke-gray-700" />
        <polyline points={points} fill="none" stroke="#f97316" strokeWidth="1.5" strokeLinejoin="round" />
        <text x={pad.left} y={pad.top + 3} className="fill-gray-400" fontSize="8">{max.toFixed(0)}</text>
        <text x={pad.left} y={height - 2} className="fill-gray-400" fontSize="8">{min.toFixed(0)}</text>
      </svg>
    </div>
  )
}

function Stat({ label, value, color }: { label: string; value: string; color?: string }) {
  return (
    <div className="text-center">
      <div className="text-gray-400">{label}</div>
      <div className={`font-bold ${color || ''}`}>{value}</div>
    </div>
  )
}

function fmtMs(ms: number): string {
  if (ms < 1000) return `${ms.toFixed(0)}ms`
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
  return `${(ms / 60000).toFixed(1)}min`
}
