import { useEffect, useState } from 'react'
import type { InferenceRate, MonitoringFrame } from '../../store/types'
import { TrendChart } from './MonitoringPrimitives'
import { formatCompactNumber, formatRate } from './monitoringFormat'
import type { MonitoringTrendPoint } from './monitoringViewModel'
import { aggregateInferenceRates, currentInferenceRate, formatInferenceRate, freshInferenceRate } from './inferenceThroughput'
import { getInferenceLabels, inferenceRateDetail } from '../../i18n/inferenceMonitoring'

export function useMonitoringClock() {
  const [now, setNow] = useState(Date.now)
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(timer) }, [])
  return now
}

export function DualThroughput({ frames, lang, compact = false }: { frames: (MonitoringFrame | undefined)[]; lang: string; compact?: boolean }) {
  const now = useMonitoringClock()
  const labels = getInferenceLabels(lang)
  return <div data-inference-throughput className={compact ? 'space-y-2' : 'grid gap-3 sm:grid-cols-2'}>
    {(['prefill', 'decode'] as const).map(phase => {
      const rates = frames.map(frame => currentInferenceRate(frame, phase, now))
      const rate = rates.length === 1 ? rates[0] : aggregateInferenceRates(rates)
      return <div key={phase} data-phase={phase} data-rate-state={rate.state} className={compact ? 'min-w-0' : 'min-w-0 rounded-xl border border-slate-200 bg-slate-50 p-3 dark:border-slate-700 dark:bg-slate-950/50'}>
        <div className="text-xs font-medium text-slate-600 dark:text-slate-300">{labels[phase]}</div>
        <div className={`${compact ? 'text-xl' : 'mt-1 text-2xl'} font-semibold tabular-nums ${phase === 'prefill' ? 'text-violet-700 dark:text-violet-300' : 'text-blue-700 dark:text-blue-300'}`}>{formatInferenceRate(rate)}</div>
        <div className="mt-1 text-xs text-slate-600 dark:text-slate-300">{inferenceRateDetail(rate, lang)}</div>
      </div>
    })}
  </div>
}

export function DualThroughputTrends({ prefill, decode, start, end, lang }: { prefill: MonitoringTrendPoint[]; decode: MonitoringTrendPoint[]; start: number; end: number; lang: string }) {
  const labels = getInferenceLabels(lang)
  return <div data-inference-trends className="space-y-4">
    {(['prefill', 'decode'] as const).map(phase => <div key={phase}>
      <div className="mb-2 text-sm font-medium text-slate-700 dark:text-slate-200">{phase === 'prefill' ? 'Prefill' : 'Decode'} · tok/s</div>
      <TrendChart points={phase === 'prefill' ? prefill : decode} rangeStart={start} rangeEnd={end} tone={phase === 'prefill' ? 'violet' : 'blue'} unit="tok/s" emptyText={labels.noTrend} />
    </div>)}
  </div>
}

export function InferenceTasks({ entries, lang }: { entries: { frame?: MonitoringFrame; name?: string }[]; lang: string }) {
  const now = useMonitoringClock()
  const labels = getInferenceLabels(lang)
  const tasks = entries.flatMap(({ frame, name }) => frame && now - frame.ts <= 8000
    ? (frame.inference?.tasks || []).map(task => ({ task, name, instance: frame.instanceId })) : [])
  if (!tasks.length) return <p className="py-4 text-sm text-slate-600 dark:text-slate-300">{labels.noTasks}</p>
  const speed = (rate: InferenceRate) => formatInferenceRate(freshInferenceRate(rate, now))
  return <div data-inference-tasks className="space-y-2">{tasks.map(({ task, name, instance }) => {
    const phase = task.phase === 'unknown' ? labels.unknownPhase : task.phase === 'prefill' ? 'Prefill' : 'Decode'
    const reading = freshInferenceRate(task.phase === 'prefill' ? task.prefill : task.decode, now)
    return <div key={`${instance}-${task.slotId}-${task.taskId}`} className="rounded-lg border border-slate-200 bg-slate-50 p-3 dark:border-slate-700 dark:bg-slate-950/50">
      <div className="flex flex-wrap items-center justify-between gap-2 text-sm">
        <span className="min-w-0 break-words font-medium text-slate-900 dark:text-slate-100">#{task.taskId} {name ? `· ${name}` : ''} · slot {task.slotId}</span>
        <span className="text-blue-700 dark:text-blue-300">{phase} · {task.phase === 'unknown' ? '—' : speed(reading)}</span>
      </div>
      <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-slate-600 dark:text-slate-300">
        <span>{labels.processed} {formatCompactNumber(task.processedTokens)} tok</span>
        <span>{labels.cached} {formatCompactNumber(task.cachedTokens)} tok</span>
        <span>{labels.generated} {formatCompactNumber(task.generatedTokens)} tok</span>
        <span>{inferenceRateDetail(reading, lang)}</span>
      </div>
      {task.phase === 'prefill' && task.progress != null ? <div className="mt-2 text-xs text-slate-600 dark:text-slate-300">{labels.progress} {(task.progress * 100).toFixed(0)}%</div> : null}
      <div className="mt-2 text-xs text-slate-600 dark:text-slate-300">{labels.phaseAverage} · Prefill {formatRate(task.prefillAverage)} · Decode {formatRate(task.decodeAverage)}</div>
    </div>
  })}</div>
}
