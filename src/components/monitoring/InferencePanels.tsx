import { useEffect, useState } from 'react'
import type { InferenceRate, MonitoringFrame } from '../../store/types'
import { TrendChart } from './MonitoringPrimitives'
import { formatCompactNumber, formatRate } from './monitoringFormat'
import { activityBands, aggregateInferenceRates, currentInferenceRate, formatInferenceRate, freshInferenceRate, summarizeWallDecode, type ActivityState, type PhasePoint } from './inferenceThroughput'
import { getInferenceLabels, inferenceRateDetail } from '../../i18n/inferenceMonitoring'

export function useMonitoringClock() {
  const [now, setNow] = useState(Date.now)
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(timer) }, [])
  return now
}

export function DualThroughput({ frames, lang, compact = false, wall = false }: { frames: (MonitoringFrame | undefined)[]; lang: string; compact?: boolean; wall?: boolean }) {
  const now = useMonitoringClock()
  const labels = getInferenceLabels(lang)
  return <div data-inference-throughput className={compact ? 'space-y-2' : 'grid gap-3 sm:grid-cols-2'}>
    {(wall ? ['decode', 'prefill'] as const : ['prefill', 'decode'] as const).map(phase => {
      const rates = frames.map(frame => currentInferenceRate(frame, phase, now))
      const rate = rates.length === 1 && !wall ? rates[0] : aggregateInferenceRates(rates)
      return <div key={phase} data-phase={phase} data-rate-state={rate.state} className={compact ? 'min-w-0' : 'min-w-0 rounded-xl border border-slate-200 bg-slate-50 p-3 dark:border-slate-700 dark:bg-slate-950/50'}>
        <div className="text-xs font-medium text-slate-600 dark:text-slate-300">{labels[phase]}</div>
        <div className={`${wall ? phase === 'decode' ? 'text-3xl' : 'text-base' : compact ? 'text-xl' : 'mt-1 text-2xl'} font-semibold tabular-nums ${phase === 'prefill' ? 'text-violet-700 dark:text-violet-300' : 'text-blue-700 dark:text-blue-300'}`}>{formatInferenceRate(rate)}</div>
        <div className="mt-1 text-xs text-slate-600 dark:text-slate-300">{wall && rate.state === 'partial' ? `${labels.reported} · ${labels.coverage} ${rate.covered}/${rate.expected}` : inferenceRateDetail(rate, lang)}</div>
      </div>
    })}
  </div>
}

const activityStyle: Record<ActivityState, string> = {
  idle: 'bg-slate-300 dark:bg-slate-600',
  prefill: 'bg-violet-500 dark:bg-violet-400',
  decode: 'bg-blue-500 dark:bg-blue-400',
  both: 'bg-[linear-gradient(to_bottom,#8b5cf6_50%,#3b82f6_50%)]',
  unknown: 'text-slate-400 dark:text-slate-500 bg-[repeating-linear-gradient(125deg,transparent_0_3px,currentColor_3px_4px)]',
}

export function DualThroughputTrends({ prefill, decode, start, end, lang, wall = false }: { prefill: PhasePoint[]; decode: PhasePoint[]; start: number; end: number; lang: string; wall?: boolean }) {
  const labels = getInferenceLabels(lang)
  const bands = activityBands(prefill, decode, start, end)
  const time = (ts: number) => new Date(ts).toLocaleTimeString([], { hour12: false })
  const names = { ...labels, prefill: 'Prefill', decode: 'Decode' }
  return <div data-inference-trends className="space-y-3">
    {(wall ? ['decode'] as const : ['prefill', 'decode'] as const).map(phase => {
      const series = phase === 'prefill' ? prefill : wall ? summarizeWallDecode(decode) : decode
      return <div key={phase}>
        <div className="mb-1 flex flex-wrap items-baseline justify-between gap-x-3 text-sm font-medium text-slate-700 dark:text-slate-200">{labels[phase]} · tok/s{wall && <span className="text-xs font-normal text-slate-500 dark:text-slate-400">{labels.activeMean}</span>}</div>
        <TrendChart points={series} sharedTimeAxis partialLabel={labels.partial} kind={phase === 'prefill' ? 'bars' : 'line'} rangeStart={start} rangeEnd={end} tone={phase === 'prefill' ? 'violet' : 'blue'} emptyText={series.length && series.every(point => point.state === 'idle') ? labels.inactive : labels.noTrend} />
      </div>
    })}
    <div aria-label={labels.activity} role="group">
      <div className="mb-1 text-sm font-medium">{labels.activity}</div>
      <div className="ml-[calc(3rem+2px)] mr-px">
        <div className="flex h-3 overflow-hidden rounded-sm">
          {bands.map(band => <span key={band.from} data-activity={band.state} data-partial={band.partial || undefined} title={`${time(band.from)}–${time(band.to)} · ${names[band.state]}${band.partial ? ' · ' + labels.partial : ''}`} style={{ width: `${100 * (band.to - band.from) / (end - start)}%` }} className={`${activityStyle[band.state]} ${band.partial ? 'border-b-2 border-dashed border-amber-700 dark:border-amber-300' : ''}`} />)}
        </div>
        <div className="mt-1 flex justify-between text-[11px] tabular-nums text-slate-500 dark:text-slate-400"><span>{time(start)}</span><span>{time((start + end) / 2)}</span><span>{time(end)}</span></div>
      </div>
      <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-xs text-slate-600 dark:text-slate-300">
        {(['prefill', 'decode', 'both', 'idle', 'unknown'] as const).filter(state => bands.some(band => band.state === state)).map(state => <span key={state} className="inline-flex items-center gap-1"><i className={`h-2 w-3 ${activityStyle[state]}`} />{names[state]}</span>)}
        {(prefill.some(point => point.state === 'partial') || decode.some(point => point.state === 'partial')) && <span>○ {labels.partial}</span>}
      </div>
    </div>
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
