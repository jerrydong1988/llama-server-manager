import type { EngineCapabilities } from './store'

export const normalizeEngineCapabilityStatus = (capabilities?: EngineCapabilities) => (
  capabilities?.status || 'unprobed'
)

export const normalizeEngineVersionStatus = (capabilities?: EngineCapabilities) => (
  capabilities?.versionStatus || 'unprobed'
)

export type EngineCompatibilityMode = 'full' | 'recognized' | 'minimal'

export const getEngineCompatibilityMode = (capabilities?: EngineCapabilities): EngineCompatibilityMode => {
  const status = normalizeEngineCapabilityStatus(capabilities)
  if (status === 'detected') return 'full'
  if (status === 'partial') return 'recognized'
  return 'minimal'
}
