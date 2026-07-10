export function formatRate(value?: number | null): string {
  if (value == null || !Number.isFinite(value) || value <= 0) return '--'
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(value >= 10 ? 1 : 2)} tok/s`
}

export function formatPercent(value?: number | null): string {
  if (value == null || !Number.isFinite(value)) return '--'
  return `${Math.round(value)}%`
}

export function formatMemory(mb?: number | null): string {
  if (mb == null || !Number.isFinite(mb) || mb <= 0) return '--'
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`
  return `${Math.round(mb)} MB`
}

export function formatMemoryPair(used?: number | null, total?: number | null): string {
  if (used == null || total == null || used <= 0 || total <= 0) return '--'
  return `${formatMemory(used)} / ${formatMemory(total)}`
}

export function formatDuration(seconds?: number | null): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds < 0) return '--'
  const total = Math.floor(seconds)
  const hours = Math.floor(total / 3600)
  const minutes = Math.floor((total % 3600) / 60)
  const secs = total % 60
  if (hours > 0) return `${hours}h ${minutes}m`
  if (minutes > 0) return `${minutes}m ${secs}s`
  return `${secs}s`
}

export function formatMs(value?: number | null): string {
  if (value == null || !Number.isFinite(value)) return '--'
  if (value < 1000) return `${value.toFixed(0)} ms`
  return `${(value / 1000).toFixed(1)} s`
}

export function formatCompactNumber(value?: number | null): string {
  if (value == null || !Number.isFinite(value)) return '--'
  return value.toLocaleString()
}

export function formatBytes(value?: number | null): string {
  if (value == null || !Number.isFinite(value) || value <= 0) return '--'
  const gb = value / 1024 / 1024 / 1024
  if (gb >= 1) return `${gb.toFixed(1)} GB`
  const mb = value / 1024 / 1024
  if (mb >= 1) return `${mb.toFixed(0)} MB`
  return `${Math.max(1, Math.round(value / 1024))} KB`
}

export function formatBytesPerSecond(value?: number | null): string {
  if (value == null || !Number.isFinite(value) || value <= 0) return '--'
  return `${formatBytes(value)}/s`
}

export function formatTime(value?: number | null): string {
  if (value == null || !Number.isFinite(value)) return '--'
  return new Date(value).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

export function formatDate(value?: number | null): string {
  if (value == null || !Number.isFinite(value)) return '--'
  return new Date(value).toLocaleString()
}

export function clampPercent(value?: number | null): number {
  if (value == null || !Number.isFinite(value)) return 0
  return Math.max(0, Math.min(100, value))
}
