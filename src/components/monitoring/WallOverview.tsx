import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Expand, Minimize2, Moon, Shrink, Sun, Thermometer, Zap } from 'lucide-react'
import { useI18n } from '../../i18n'
import { getMonitorWorkspaceCopy } from '../../i18n/monitorWorkspace'
import { inferenceRateDetail } from '../../i18n/inferenceMonitoring'
import { useAppStore } from '../../store'
import type { Instance, MonitoringFrame, SystemMetrics, SystemMetricsSample } from '../../store/types'
import { Button, IconButton } from '../ui'
import { workloadLabel } from '../PerformancePage/vectorPerformance'
import { aggregateInferenceRates, currentInferenceRate, summarizeWallDecode, type PhasePoint } from './inferenceThroughput'
import { hostReadings, MonitorTrace, numberText, type Reading } from './MonitorReadings'
import { formatTime } from './monitoringFormat'
import type { FleetThroughputSeries } from './monitoringViewModel'
import mark from '../../../src-tauri/icons/128x128.png'

export function WallOverview({
  instances,
  frames,
  system,
  history,
  now,
  inferenceIds,
  decode,
  fleet,
  children,
}: {
  instances: Instance[]
  frames: Record<string, MonitoringFrame>
  system: SystemMetrics | null
  history: SystemMetricsSample[]
  now: number
  inferenceIds: string[]
  decode: PhasePoint[]
  fleet: FleetThroughputSeries
  children: ReactNode
}) {
  const { lang, t } = useI18n()
  const copy = getMonitorWorkspaceCopy(lang)
  const dark = useAppStore((s) => s.darkMode),
    setDark = useAppStore((s) => s.setDarkMode),
    navigate = useAppStore((s) => s.setActiveTab)
  const [all, setAll] = useState(false),
    [full, setFull] = useState(false),
    [fullError, setFullError] = useState('')
  const [detailsOpen, setDetailsOpen] = useState(false)
  const wall = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const sync = () => setFull(document.fullscreenElement === wall.current)
    document.addEventListener('fullscreenchange', sync)
    return () => document.removeEventListener('fullscreenchange', sync)
  }, [])
  const running = instances.filter((i) => i.status === 'running')
  const ordered = [...running, ...instances.filter((i) => i.status !== 'running')]
  const fresh = running
    .map((i) => frames[i.id])
    .filter((f) => f && now - f.ts <= 8000 && f.state !== 'unavailable' && f.state !== 'warming')
  const count = (key: 'activeRequests' | 'queuedRequests') =>
    fresh.length === running.length ? fresh.reduce((n, f) => n + f[key], 0) : null
  const vector = !inferenceIds.length && running.length > 0
  const rate = aggregateInferenceRates(
    inferenceIds.map((id) => currentInferenceRate(frames[id], 'decode', now)),
  )
  const main: Reading = {
    id: 'speed',
    label: vector ? copy.totalVector : copy.totalSpeed,
    unit: vector ? copy.items : 'tok/s',
    value: vector ? fleet.current : rate.value,
    detail: '',
    points: vector ? fleet.points : summarizeWallDecode(decode),
  }
  const host = hostReadings(system, history, now, copy)
  const memoryPercent =
    host.memory.value != null && system?.vram_total_mb
      ? ((host.memory.value * 1024) / system.vram_total_mb) * 100
      : null
  const themeLabel = dark ? t.appShell.switchToLight : t.appShell.switchToDark
  return (
    <div className="monitor-wall" ref={wall} data-guide="monitoring-wall">
      <header className="monitor-wall-head" data-guide="monitoring-header">
        <div className="monitor-wall-brand">
          <img src={mark} alt="" />
          <h1>{copy.wall}</h1>
          <span className="console-status" data-state={running.length ? 'running' : 'stopped'}>
            {running.length} {copy.online}
          </span>
        </div>
        <div className="monitor-actions">
          <time>{formatTime(now)}</time>
          <IconButton
            label={themeLabel}
            icon={dark ? <Sun size={17} /> : <Moon size={17} />}
            onClick={() => setDark(!dark)}
          />
          {document.fullscreenEnabled && (
            <IconButton
              label={full ? copy.exitFullscreen : copy.fullscreen}
              icon={full ? <Shrink size={16} /> : <Expand size={16} />}
              onClick={async () => {
                try {
                  setFullError('')
                  if (full) await document.exitFullscreen()
                  else await wall.current?.requestFullscreen()
                } catch {
                  setFullError(copy.fullscreenError)
                }
              }}
            />
          )}
          <Button size="sm" icon={<Minimize2 size={15} />} onClick={() => navigate('perf')}>
            {copy.back}
          </Button>
        </div>
      </header>
      {fullError && <p role="alert">{fullError}</p>}
      <div className="monitor-wall-grid">
        <section className="console-surface monitor-wall-chart">
          <header>
            <h2>{main.label}</h2>
            <span className="monitor-note">{copy.five}</span>
          </header>
          <p className="monitor-hero-value" data-wall-rate={vector ? 'items' : rate.state}>
            {numberText(main.value)}
            {main.value != null && <small>{main.unit}</small>}
          </p>
          <p className="monitor-note">
            {copy.active} {numberText(count('activeRequests'), 0)}　·　{copy.queued}{' '}
            {numberText(count('queuedRequests'), 0)}
            {!vector && rate.state === 'partial' && '　·　' + inferenceRateDetail(rate, lang)}
          </p>
          <MonitorTrace reading={main} end={now} empty={copy.noData} />
        </section>
        <section className="monitor-wall-hardware">
          <div>
            <h2>{copy.local}</h2>
            <p className="monitor-note">{system?.gpu_name || copy.missing}</p>
          </div>
          {[host.temperature, host.power].map((m) => (
            <div className="monitor-wall-sensor" key={m.id} data-wall-sensor={m.id}>
              <span>
                {m.id === 'temperature' ? <Thermometer size={18} /> : <Zap size={18} />} {m.label}
              </span>
              <strong>
                {numberText(m.value, m.digits)}
                <small>{m.value == null ? copy.missing : m.unit}</small>
              </strong>
            </div>
          ))}
          <div>
            <div className="monitor-pair">
              <span>{copy.memory}</span>
              <span>
                {host.memory.value == null ? '—' : numberText(host.memory.value) + ' ' + host.memory.unit}
              </span>
            </div>
            {memoryPercent != null && (
              <progress aria-label={copy.memory} max={100} value={Math.min(100, memoryPercent)} />
            )}
          </div>
        </section>
      </div>
      <section className="monitor-fleet">
        <header>
          <h2>{copy.instances}</h2>
          {ordered.length > 6 && (
            <button type="button" className="monitor-link" aria-expanded={all} onClick={() => setAll(!all)}>
              {all ? copy.less : copy.more + ' (' + ordered.length + ')'}
            </button>
          )}
        </header>
        <div className="monitor-fleet-list">
          {(all ? ordered : ordered.slice(0, 6)).map((i) => {
            const frame = frames[i.id]
            const workload =
              frame?.workload ??
              (i.config.reranking ? 'reranker' : i.config.embedding ? 'embedding' : 'inference')
            const activity =
              i.status !== 'running'
                ? i.status === 'error'
                  ? copy.failed
                  : copy.stopped
                : !frame ||
                    now - frame.ts > 8000 ||
                    frame.state === 'unavailable' ||
                    frame.state === 'warming'
                  ? copy.unknown
                  : frame.state === 'idle'
                    ? copy.idle
                    : workload === 'inference'
                      ? copy.generating
                      : workload === 'embedding'
                        ? copy.embedding
                        : copy.reranking
            return (
              <div key={i.id}>
                <div>
                  <strong>{i.name}</strong>
                  <small>
                    {i.config.engine_kind === 'gufo' ? 'Gufo' : 'llama.cpp'} · {workloadLabel(workload, lang)}
                  </small>
                </div>
                <span
                  className="console-status"
                  data-state={
                    i.status === 'error'
                      ? 'error'
                      : i.status === 'running' && frame?.state === 'active' && now - frame.ts <= 8000
                        ? 'running'
                        : 'stopped'
                  }
                >
                  {activity}
                </span>
              </div>
            )
          })}
        </div>
        {!instances.length && <p className="monitor-note">{copy.noInstances}</p>}
      </section>
      <details
        className="monitor-disclosure"
        data-monitor-operations
        open={detailsOpen}
        onToggle={(e) => setDetailsOpen(e.currentTarget.open)}
      >
        <summary>{copy.detail}</summary>
        {detailsOpen && children}
      </details>
    </div>
  )
}
