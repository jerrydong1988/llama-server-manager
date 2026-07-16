import type { DownloadBandwidthUnit, DownloadResumePolicy } from './DownloadSettingsPanel'

export const DEFAULT_SAVE_DIR = 'models'
export const DEFAULT_BANDWIDTH_LIMIT = 0
export const DEFAULT_BANDWIDTH_UNIT: DownloadBandwidthUnit = 'MiB/s'
export const LOCAL_FILE_CHECK_CONCURRENCY = 8

export const clampConcurrency = (value: number) => Math.max(1, Math.min(8, Number.isFinite(value) ? value : 1))
export const normalizeResumePolicy = (policy: string): DownloadResumePolicy => policy === 'auto_on_launch' ? 'auto_on_launch' : 'manual'

const bandwidthUnitMultiplier = (unit: DownloadBandwidthUnit) => unit === 'MiB/s' ? 1024 * 1024 : 1024

export const bandwidthToBytes = (value: number, unit: DownloadBandwidthUnit) => (
  Math.max(0, Math.round((Number.isFinite(value) ? value : 0) * bandwidthUnitMultiplier(unit)))
)

export const bytesToBandwidth = (bytes: number, unit: DownloadBandwidthUnit) => {
  if (!Number.isFinite(bytes) || bytes <= 0) return 0
  const value = bytes / bandwidthUnitMultiplier(unit)
  return Number.isInteger(value) ? value : Number(value.toFixed(unit === 'MiB/s' ? 2 : 0))
}

export const preferredBandwidthDisplay = (bytes: number, fallbackUnit: DownloadBandwidthUnit) => {
  if (!Number.isFinite(bytes) || bytes <= 0) return { limit: 0, unit: fallbackUnit }
  if (bytes % (1024 * 1024) === 0) return { limit: bytes / (1024 * 1024), unit: 'MiB/s' as DownloadBandwidthUnit }
  return { limit: Math.round(bytes / 1024), unit: 'KiB/s' as DownloadBandwidthUnit }
}

export const downloadFileKey = (source: string, repoId: string, remotePath: string, saveDir: string) => (
  `${source}\u0000${repoId}\u0000${remotePath}\u0000${saveDir}`
)
