import { Activity, ChevronDown, Layers, Thermometer, Zap } from 'lucide-react'
import type { MonitorCopy } from '../../i18n/monitorWorkspace'
import type { SystemMetrics, SystemMetricsSample } from '../../store/types'
import { formatTime } from './monitoringFormat'

export type ReadingPoint = { ts: number; value: number | null; state?: string }
export type Reading = {
  id: 'speed' | 'memory' | 'temperature' | 'power'
  label: string
  value: number | null
  unit: string
  detail: string
  points: ReadingPoint[]
  digits?: number
}
export const FIVE_MINUTES = 300_000

export function numberText(value: number | null | undefined, digits = 1) {
  return value == null || !Number.isFinite(value)
    ? '—'
    : value.toLocaleString(undefined, { maximumFractionDigits: digits })
}

export function currentHost(system: SystemMetrics | null, history: SystemMetricsSample[], now: number) {
  const last = history[history.length - 1]
  return last && now - last.ts <= 15_000 ? system : null
}

export function hostReadings(
  system: SystemMetrics | null,
  history: SystemMetricsSample[],
  now: number,
  copy: MonitorCopy,
) {
  const current = currentHost(system, history, now)
  const recent = history.filter(
    (s) =>
      s.ts >= now - FIVE_MINUTES &&
      s.ts <= now &&
      s.gpu_name === system?.gpu_name &&
      s.gpu_vendor === system?.gpu_vendor,
  )
  const sources = (read: (s: SystemMetrics) => number | null | undefined) =>
    recent.map((s) => ({ ts: s.ts, value: read(s) ?? null }))
  const nvidia = system?.gpu_vendor === 'NVIDIA'
  const tempPoints = sources((s) => s.gpu_temperature_c)
  const temps = tempPoints.flatMap((p) => (p.value == null ? [] : [p.value]))
  const used = current?.vram_used_mb
  const temp = current?.gpu_temperature_c
  const power = current?.gpu_power_w
  const memory: Reading = {
    id: 'memory',
    label: copy.memory,
    value: used == null ? null : used / 1024,
    unit: current?.vram_total_mb ? '/ ' + numberText(current.vram_total_mb / 1024) + ' GB' : 'GB',
    detail: copy.gpuLoad + ' ' + numberText(current?.gpu_percent) + (current?.gpu_percent == null ? '' : '%'),
    points: sources((s) => (s.vram_used_mb == null ? null : s.vram_used_mb / 1024)),
  }
  const temperature: Reading = {
    id: 'temperature',
    label: copy.temperature,
    value: temp ?? null,
    unit: '°C',
    digits: 0,
    detail: temp == null ? copy.missing : copy.peak + ' ' + numberText(Math.max(...temps), 0) + ' °C',
    points: tempPoints,
  }
  const wattage: Reading = {
    id: 'power',
    label: copy.power,
    value: power ?? null,
    unit: 'W',
    detail: power == null ? copy.missing : nvidia ? copy.nvmlPower : 'AMD ADLX · GPUPower',
    points: sources((s) => s.gpu_power_w),
  }
  return { memory, temperature, power: wattage }
}

export function MonitorTrace({
  reading,
  end,
  compact = false,
  empty,
}: {
  reading: Reading
  end: number
  compact?: boolean
  empty: string
}) {
  const start = end - FIVE_MINUTES
  const points = reading.points.filter((p) => p.ts >= start && p.ts <= end)
  const values = points.flatMap((p) =>
    p.value != null && Number.isFinite(p.value) && p.state !== 'idle' ? [p.value] : [],
  )
  if (!values.length)
    return (
      <div className={compact ? 'monitor-trace-empty small' : 'monitor-trace-empty'}>{!compact && empty}</div>
    )
  const high = Math.max(1, ...values) * 1.06
  const low = compact ? Math.min(0.9 * Math.min(...values), high - 1) : Math.min(0, ...values)
  const partials: Array<[number, number]> = []
  const segments: Array<Array<[number, number]>> = []
  let segment: Array<[number, number]> = []
  let lastTs = 0
  for (const point of points) {
    if (
      point.value == null ||
      !Number.isFinite(point.value) ||
      point.state === 'idle' ||
      point.state === 'partial' ||
      (lastTs && point.ts - lastTs > 22_500)
    ) {
      if (segment.length) segments.push(segment)
      segment = []
    }
    if (point.value != null && Number.isFinite(point.value) && point.state !== 'idle') {
      const coordinate: [number, number] = [
        ((point.ts - start) / FIVE_MINUTES) * 500,
        48 - ((point.value - low) / (high - low)) * 45,
      ]
      if (point.state === 'partial') partials.push(coordinate)
      else segment.push(coordinate)
    }
    lastTs = point.ts
  }
  if (segment.length) segments.push(segment)
  return (
    <div className={compact ? 'monitor-trace small' : 'monitor-trace'} data-color={reading.id}>
      {!compact && (
        <div className="monitor-y" aria-hidden="true">
          <span>{numberText(high, 0)}</span>
          <span>{numberText(low, 0)}</span>
        </div>
      )}
      <svg
        viewBox="0 0 500 50"
        preserveAspectRatio="none"
        role={compact ? undefined : 'img'}
        aria-hidden={compact || undefined}
        aria-label={compact ? undefined : reading.label}
      >
        {!compact &&
          [3, 25, 48].map((y) => (
            <line
              key={y}
              x1="0"
              x2="500"
              y1={y}
              y2={y}
              className="monitor-grid-line"
              vectorEffect="non-scaling-stroke"
            />
          ))}
        {partials.map(([x,y],i) => <circle key={i} data-partial cx={x} cy={y} r="1.7" fill="none" stroke="currentColor" vectorEffect="non-scaling-stroke" />)}
        {segments.map((group, i) => {
          const path = group.map((p) => p.join(',')).join(' ')
          return (
            <g key={i}>
              {group.length === 1 ? (
                <circle cx={group[0][0]} cy={group[0][1]} r="1.3" fill="currentColor" />
              ) : (
                <>
                  <polygon
                    points={group[0][0] + ',49 ' + path + ' ' + group[group.length - 1][0] + ',49'}
                    fill="currentColor"
                    opacity=".09"
                  />
                  <polyline
                    points={path}
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.7"
                    vectorEffect="non-scaling-stroke"
                  />
                </>
              )}
            </g>
          )
        })}
      </svg>
      {!compact && (
        <div className="monitor-x">
          <span>{formatTime(start)}</span>
          <span>{formatTime(end)}</span>
        </div>
      )}
    </div>
  )
}

const icons = { speed: Activity, memory: Layers, temperature: Thermometer, power: Zap }
export function ReadingTile({
  reading,
  open,
  toggle,
  end,
  empty,
}: {
  reading: Reading
  open: boolean
  toggle: () => void
  end: number
  empty: string
}) {
  const Icon = icons[reading.id]
  return (
    <button
      type="button"
      className="monitor-tile"
      data-reading={reading.id}
      aria-expanded={open}
      onClick={toggle}
    >
      <span className="monitor-tile-label">
        <span>
          <Icon size={15} />
          {reading.label}
        </span>
        <ChevronDown size={14} />
      </span>
      <span className="monitor-value">
        {numberText(reading.value, reading.digits)}
        {reading.value != null && <small>{reading.unit}</small>}
      </span>
      <span className="monitor-note">{reading.detail}</span>
      <MonitorTrace reading={reading} end={end} compact empty={empty} />
    </button>
  )
}
