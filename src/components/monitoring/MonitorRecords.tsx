import { useState } from 'react'
import { ChevronDown } from 'lucide-react'
import type { MonitorCopy } from '../../i18n/monitorWorkspace'
import type {
  ModelWorkload,
  InferenceRequestSummary,
  RecentVectorActivity,
  TelemetrySessionAnalysis,
  TelemetrySessionSummary,
} from '../../store/types'
import { formatDate, formatDuration, formatMemory, formatMs, formatTime } from './monitoringFormat'
import { numberText } from './MonitorReadings'

export function RequestRecords({
  copy,
  requests,
  batches,
  workload,
}: {
  copy: MonitorCopy
  requests: InferenceRequestSummary[]
  batches: RecentVectorActivity[]
  workload: ModelWorkload
}) {
  const vector = workload !== 'inference'
  const [expanded, setExpanded] = useState(false)
  const [selected, setSelected] = useState<string | null>(null)
  const rows = vector
    ? batches.map((r) => ({
        id: String(r.id),
        ts: r.completed_at,
        input: r.input_tokens,
        output: r.item_count,
        rate: !r.error_text && (r.http_status ?? 200) < 400 && r.duration_ms > 0 ? (r.item_count * 1000) / r.duration_ms : null,
        duration: r.duration_ms,
        http: r.http_status,
        error: r.error_text,
        detail: r.source === 'proxy' ? 'API' : copy.engine,
      }))
    : requests.map((r) => ({
        id: String(r.task_id),
        ts: r.completed_at,
        input: r.prompt_tokens,
        output: r.generated_tokens,
        rate: r.generation_tps,
        duration: r.total_time_ms,
        http: r.http_status,
        error: r.error_text,
        detail: [
          r.model,
          r.api_format,
          r.prompt_tps != null ? copy.prefill + ' ' + numberText(r.prompt_tps) + ' tok/s' : null,
        ]
          .filter(Boolean)
          .join(' · '),
      }))
  const shown = expanded ? rows : rows.slice(0, 3)
  const detail = shown.find((r) => r.id === selected)
  return (
    <section className="console-surface monitor-records">
      <header>
        <h3>{vector ? copy.batches : copy.recent}</h3>
        {rows.length > 3 && (
          <button
            type="button"
            className="monitor-link"
            aria-expanded={expanded}
            onClick={() => setExpanded(!expanded)}
          >
            {expanded ? copy.less : copy.more + ' (' + rows.length + ')'} <ChevronDown size={14} />
          </button>
        )}
      </header>
      {rows.length === 0 ? (
        <p className="monitor-empty">{copy.empty}</p>
      ) : (
        <div className="monitor-table-wrap">
          <table>
            <thead>
              <tr>
                {[
                  copy.time,
                  copy.status,
                  copy.input,
                  vector ? copy.count : copy.output,
                  vector ? (workload === 'reranker' ? copy.documents : copy.vector) : copy.speed,
                  copy.latency,
                ].map((label) => (
                  <th key={label} scope="col">
                    {label}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {shown.map((r) => (
                <tr key={r.id} className="console-list-row" data-selected={selected === r.id}>
                  <td>
                    <button
                      type="button"
                      aria-expanded={selected === r.id}
                      className="monitor-link"
                      onClick={() => setSelected(selected === r.id ? null : r.id)}
                    >
                      {formatTime(r.ts)}
                    </button>
                  </td>
                  <td>
                    <span
                      className="console-status"
                      data-state={r.error || (r.http != null && r.http >= 400) ? 'error' : 'running'}
                    >
                      {r.error || (r.http != null && r.http >= 400) ? copy.failed : copy.done}
                    </span>
                  </td>
                  <td>{numberText(r.input, 0)}</td>
                  <td>{numberText(r.output, 0)}</td>
                  <td>
                    {numberText(r.rate)}
                    {r.rate != null && ' ' + (vector ? copy.items : 'tok/s')}
                  </td>
                  <td>{formatMs(r.duration)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {detail && (
        <div className="monitor-record-detail">
          <strong>#{detail.id}</strong> {detail.detail} {detail.http && 'HTTP ' + detail.http} {detail.error}
        </div>
      )}
    </section>
  )
}

export function SessionRecords({
  copy,
  sessions,
  selected,
  analysis,
  choose,
  children,
}: {
  copy: MonitorCopy
  sessions: TelemetrySessionSummary[]
  selected?: TelemetrySessionSummary
  analysis: TelemetrySessionAnalysis | null
  choose: (session: TelemetrySessionSummary) => void
  children: React.ReactNode
}) {
  return (
    <>
      <section className="console-surface monitor-records">
        <header>
          <h3>{copy.session}</h3>
          <span className="monitor-note">{selected?.instance_name}</span>
        </header>
        {sessions.length === 0 ? (
          <p className="monitor-empty">{copy.noSessions}</p>
        ) : (
          <div className="monitor-table-wrap">
            <table>
              <thead>
                <tr>
                  {[copy.sessionStart, copy.duration, copy.average, copy.maxMemory].map((label) => (
                    <th key={label} scope="col">
                      {label}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {sessions.map((s) => {
                  const value =
                    s.workload === 'inference'
                      ? s.avg_tokens_per_sec > 0
                        ? s.avg_tokens_per_sec
                        : null
                      : (s.avg_items_per_sec ??
                        (s.id === selected?.id ? analysis?.vector_analysis?.averageItemsPerSecond : null))
                  return (
                    <tr key={s.id} className="console-list-row" data-selected={s.id === selected?.id}>
                      <td>
                        <button
                          type="button"
                          className="monitor-link"
                          aria-pressed={s.id === selected?.id}
                          onClick={() => choose(s)}
                        >
                          {formatDate(s.started_at)}
                        </button>
                      </td>
                      <td>{formatDuration(s.duration_secs ?? (Date.now() - s.started_at) / 1000)}</td>
                      <td>
                        {numberText(value)}
                        {value != null && ' ' + (s.workload === 'inference' ? 'tok/s' : copy.items)}
                      </td>
                      <td>{formatMemory(s.peak_vram_mb)}</td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          </div>
        )}
      </section>
      {children}
    </>
  )
}
