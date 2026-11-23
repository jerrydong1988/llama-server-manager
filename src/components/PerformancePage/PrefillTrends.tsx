import { useState } from 'react'
import { inferenceRateDetail } from '../../i18n/inferenceMonitoring'
import { getMonitorWorkspaceCopy } from '../../i18n/monitorWorkspace'
import type {
  CompletedPrefill,
  InstancePrefillInterval,
  MonitoringFrame,
  RequestPrefillWindow,
  TelemetrySampleSummary,
} from '../../store/types'
import { currentInferenceRate, inferenceTrend } from '../monitoring/inferenceThroughput'
import { FIVE_MINUTES, MonitorTrace, numberText, type Reading } from '../monitoring/MonitorReadings'
import { TrendChart } from '../monitoring/MonitoringPrimitives'
import { formatTime } from '../monitoring/monitoringFormat'

export function completedPrefillRate(point?: CompletedPrefill | null) {
  if (!point || point.tokens <= 0 || !(point.elapsedMs > 0)) return null
  const value = (point.tokens * 1000) / point.elapsedMs
  return Number.isFinite(value) ? value : null
}

function instanceIntervals(
  frame: MonitoringFrame | null,
  frames: MonitoringFrame[],
  samples: TelemetrySampleSummary[],
  start: number,
  end: number,
) {
  const unique = new Map<number, InstancePrefillInterval>()
  const add = (entries?: InstancePrefillInterval[]) => {
    for (const entry of entries ?? []) if (entry.ts > start && entry.ts <= end) unique.set(entry.ts, entry)
  }
  for (const sample of samples) if (sample.ts >= start) add(sample.inference?.instancePrefill)
  for (const item of frames) if (item.ts >= start) add(item.inference?.instancePrefill)
  add(frame?.inference?.instancePrefill)
  return [...unique.values()].sort((a, b) => a.ts - b.ts)
}

export function PrefillTrends({
  lang,
  now,
  frame,
  frames,
  samples,
  window,
  gufo,
}: {
  lang: string
  now: number
  frame: MonitoringFrame | null
  frames: MonitoringFrame[]
  samples: TelemetrySampleSummary[]
  window?: RequestPrefillWindow
  gufo: boolean
}) {
  const c = getMonitorWorkspaceCopy(lang)
  const [mode, setMode] = useState<'requests' | 'instance' | 'tasks'>('requests')
  const start = now - FIVE_MINUTES
  const requestEnd = window?.asOf || now
  const events = (window?.points ?? []).filter((p) => p.ts > requestEnd - FIVE_MINUTES && p.ts <= requestEnd)
  const intervals = instanceIntervals(frame, frames, samples, start, now)
  const latest = intervals[intervals.length - 1]
  const newest = latest && now - latest.ts <= (latest.basis === 'wall_time' ? 5000 : 12000) ? latest : null
  const basis = newest?.basis ?? latest?.basis ?? (gufo ? 'wall_time' : 'engine_time')
  const rate = currentInferenceRate(frame ?? undefined, 'prefill', now)
  const live: Reading = {
    id: 'speed',
    label: c.prefill,
    unit: 'tok/s',
    value: mode === 'tasks' ? rate.value : (newest?.rate ?? null),
    detail:
      mode === 'tasks'
        ? inferenceRateDetail(rate, lang)
        : basis === 'wall_time'
          ? c.wallElapsed
          : c.engineElapsed,
    points:
      mode === 'tasks'
        ? inferenceTrend(frames, samples, 'prefill', start)
        : intervals.map((p) => ({ ts: p.ts, value: p.rate })),
  }
  const count = events.reduce((n, p) => n + p.count, 0)
  const totalTokens = events.reduce((n, p) => n + p.tokens, 0)
  const totalMs = events.reduce((n, p) => n + p.elapsedMs, 0)
  const approx = events.some((p) => p.estimated)
  const eventLabel = (p: CompletedPrefill) =>
    [
      formatTime(p.ts),
      p.request ?? p.count + ' ' + c.finishedRequests,
      numberText(p.tokens, 0) + ' ' + c.workTokens,
      (p.estimated ? '≈' : '') + numberText(p.elapsedMs, 2) + ' ms',
      numberText(completedPrefillRate(p), 2) + ' tok/s',
    ].join(' · ')
  const modes = { requests: c.requestMode, instance: c.instanceMode, tasks: c.taskMode }
  return (
    <div className="space-y-2" data-prefill-mode={mode}>
      <div className="monitor-actions" role="group" aria-label={c.prefillMeasure}>
        {(Object.keys(modes) as Array<keyof typeof modes>).map((key) => (
          <button
            type="button"
            key={key}
            className="console-filter"
            aria-pressed={mode === key}
            onClick={() => setMode(key)}
          >
            {modes[key]}
          </button>
        ))}
      </div>
      {mode === 'requests' ? (
        <>
          {count > 0 && (
            <p className="monitor-note" data-prefill-summary>
              {count} {c.finishedRequests} · {numberText(totalTokens, 0)} {c.workTokens} · {c.weighted}{' '}
              <strong className="text-ui-ink">
                {approx && '≈'}
                {numberText((totalTokens * 1000) / totalMs, 2)} tok/s
              </strong>
              {window?.bucketed && ' · ' + c.perSecond} · {c.asOf} {formatTime(requestEnd)}
            </p>
          )}
          {count === 0 && newest?.rate != null && (
            <p className="monitor-note">
              {c.instanceMode} <strong className="text-ui-ink">{numberText(newest.rate, 2)} tok/s</strong> ·{' '}
              {basis === 'wall_time' ? c.wallElapsed : c.engineElapsed}
            </p>
          )}
          <TrendChart
            kind="points"
            tone="emerald"
            points={events.map((p) => ({ ts: p.ts, value: completedPrefillRate(p), label: eventLabel(p) }))}
            rangeStart={requestEnd - FIVE_MINUTES}
            rangeEnd={requestEnd}
            unit="tok/s"
            emptyText={c.noCompleted}
          />
          {count > 0 && (
            <details className="monitor-disclosure">
              <summary>{c.timingDetails}</summary>
              {approx && <p className="monitor-note">{c.estimatedTime}</p>}
              <ul className="max-h-48 overflow-y-auto text-xs leading-6 text-ui-body">
                {[...events].reverse().map((p) => (
                  <li key={p.ts + ':' + (p.request ?? '')}>{eventLabel(p)}</li>
                ))}
              </ul>
            </details>
          )}
        </>
      ) : (
        <>
          <p className="monitor-note">
            {live.detail}
            {mode === 'instance' &&
              ' · ' +
                (newest?.rate != null
                  ? numberText(newest.rate, 2) +
                    ' tok/s · ' +
                    c.pollInterval +
                    ' ' +
                    numberText(newest.wallMs / 1000, 2) +
                    ' s'
                  : newest?.tokens === 0
                    ? c.noNewWork
                    : c.noWindow)}
          </p>
          <MonitorTrace reading={live} end={now} empty={c.noData} />
        </>
      )}
    </div>
  )
}
