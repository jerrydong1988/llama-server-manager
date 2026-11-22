import { useState } from 'react'
import { X } from 'lucide-react'
import type {
  InferenceRequestSummary,
  Instance,
  ModelWorkload,
  MonitoringFrame,
  RecentVectorActivity,
  SystemMetrics,
  SystemMetricsSample,
  TelemetrySessionDetail,
  TelemetrySampleSummary,
} from '../../store/types'
import { getMonitorWorkspaceCopy } from '../../i18n/monitorWorkspace'
import { inferenceRateDetail } from '../../i18n/inferenceMonitoring'
import { currentInferenceRate, inferenceTrend } from '../monitoring/inferenceThroughput'
import { useMonitoringClock } from '../monitoring/InferencePanels'
import {
  FIVE_MINUTES,
  hostReadings,
  MonitorTrace,
  numberText,
  ReadingTile,
  type Reading,
} from '../monitoring/MonitorReadings'
import { RequestRecords } from '../monitoring/MonitorRecords'
import { formatTime } from '../monitoring/monitoringFormat'

export function LiveOverview({
  lang,
  instance,
  workload,
  frame,
  frames,
  samples,
  system,
  history,
  requests,
  batches,
  context,
}: {
  lang: string
  instance: Instance | null
  workload: ModelWorkload
  frame: MonitoringFrame | null
  frames: MonitoringFrame[]
  samples: TelemetrySampleSummary[]
  system: SystemMetrics | null
  history: SystemMetricsSample[]
  requests: InferenceRequestSummary[]
  batches: RecentVectorActivity[]
  context?: TelemetrySessionDetail['context_usage']
}) {
  const copy = getMonitorWorkspaceCopy(lang)
  const now = useMonitoringClock()
  const [detail, setDetail] = useState<Reading['id'] | null>(null)
  const [phase, setPhase] = useState<'decode' | 'prefill'>('decode')
  const recent = frame && now - frame.ts <= 8000 && instance?.status === 'running' ? frame : null
  const available = recent && recent.state !== 'unavailable' && recent.state !== 'warming' ? recent : null
  const vector = workload !== 'inference'
  const active = available?.state === 'active'
  const title =
    instance?.status === 'stopped'
      ? copy.stopped
      : !available
        ? copy.unknown
        : active
          ? workload === 'embedding'
            ? copy.embedding
            : workload === 'reranker'
              ? copy.reranking
              : available.inference?.tasks.length &&
                  available.inference.tasks.every((t) => t.phase === 'prefill')
                ? copy.prefilling
                : copy.generating
          : copy.idle
  const decode = currentInferenceRate(recent ?? undefined, 'decode', now)
  const prefill = currentInferenceRate(recent ?? undefined, 'prefill', now)
  const hardware = hostReadings(system, history, now, copy)
  const latest = requests[0]
  const speed: Reading = {
    id: 'speed',
    label: vector ? (workload === 'reranker' ? copy.documents : copy.vector) : copy.speed,
    value: vector ? (available?.itemsPerSecond ?? null) : decode.value,
    unit: vector ? copy.items : 'tok/s',
    points: vector
      ? frames.map((f) => ({ ts: f.ts, value: f.state === 'unavailable' ? null : f.itemsPerSecond }))
      : inferenceTrend(frames, samples, 'decode', now - FIVE_MINUTES),
    detail: vector
      ? title
      : available?.state === 'idle' && latest?.generation_tps != null
        ? copy.lastRequest + ' ' + numberText(latest.generation_tps) + ' tok/s'
        : latest?.prompt_tps != null
          ? copy.prefill + ' ' + numberText(latest.prompt_tps) + ' tok/s'
          : inferenceRateDetail(decode, lang),
  }
  const readings = [speed, hardware.memory, hardware.temperature, hardware.power]
  const selected =
    detail === 'speed' && !vector && phase === 'prefill'
      ? {
          ...speed,
          label: copy.prefill,
          points: inferenceTrend(frames, samples, 'prefill', now - FIVE_MINUTES),
          detail: inferenceRateDetail(prefill, lang),
        }
      : readings.find((m) => m.id === detail)
  const usage = recent && context && now - context.ts <= 15_000 && context.capacity > 0 ? context : null
  const capacity =
    !instance?.config.ctx_size_auto && instance?.config.ctx_size ? instance.config.ctx_size : null
  const percent = usage ? Math.min(100, Math.max(0, (usage.used / usage.capacity) * 100)) : null
  const task = recent?.inference?.tasks?.[0]
  return (
    <>
      <section className="monitor-state" data-active={active} aria-label={copy.status}>
        <strong className="console-status" data-state={active ? 'running' : 'stopped'}>
          {title}
        </strong>
        <div className="monitor-note">
          {task && <span>#{task.taskId} · </span>}
          {copy.active} {numberText(available?.activeRequests, 0)} · {copy.queued}{' '}
          {numberText(available?.queuedRequests, 0)}
        </div>
        <div>
          <div className="monitor-pair">
            <span>{vector ? copy.batch : usage ? copy.context : copy.limit}</span>
            <span>
              {vector
                ? batches[0]
                  ? numberText(batches[0].item_count, 0) + ' ' + copy.batchItems
                  : '—'
                : usage
                  ? numberText(usage.used, 0) +
                    ' / ' +
                    numberText(usage.capacity, 0) +
                    ' · ' +
                    numberText(percent, 0) +
                    '%'
                  : numberText(capacity, 0)}
            </span>
          </div>
          {percent != null && !vector && <progress max={100} value={percent} aria-label={copy.context} />}
        </div>
      </section>
      <div className="monitor-section-title">
        <h3>{copy.five}</h3>
        <span className="monitor-note">
          {copy.local}
          {system?.gpu_name && ' · ' + system.gpu_name}
        </span>
      </div>
      <div className="monitor-tiles">
        {readings.map((m) => (
          <ReadingTile
            key={m.id}
            reading={m}
            open={m.id === detail}
            toggle={() => {
              setDetail(m.id === detail ? null : m.id)
              setPhase('decode')
            }}
            end={now}
            empty={copy.noData}
          />
        ))}
      </div>
      {selected && (
        <section className="console-surface monitor-detail">
          <header>
            <h3>
              {selected.label} · {copy.trend}
            </h3>
            <div className="monitor-actions">
              {detail === 'speed' &&
                !vector &&
                (['decode', 'prefill'] as const).map((p) => (
                  <button
                    type="button"
                    className="console-filter"
                    key={p}
                    aria-pressed={phase === p}
                    onClick={() => setPhase(p)}
                  >
                    {p === 'decode' ? copy.speed : copy.prefill}
                  </button>
                ))}
              <button
                type="button"
                aria-label={copy.close}
                className="console-btn monitor-icon"
                onClick={() => setDetail(null)}
              >
                <X size={15} />
              </button>
            </div>
          </header>
          <p className="monitor-note">{selected.detail}</p>
          <MonitorTrace reading={selected} end={now} empty={copy.noData} />
        </section>
      )}
      <RequestRecords copy={copy} requests={requests} batches={batches} workload={workload} />
      <p className="monitor-sample-time">
        {copy.last}　{formatTime(history[history.length - 1]?.ts)}
      </p>
    </>
  )
}
