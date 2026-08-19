import type { EngineCapabilities, EngineQualificationStatus } from './store'

export const normalizeEngineCapabilityStatus = (capabilities?: EngineCapabilities) => (
  capabilities?.status || 'unprobed'
)

export const normalizeEngineVersionStatus = (capabilities?: EngineCapabilities) => (
  capabilities?.versionStatus || 'unprobed'
)

export const normalizeEngineQualificationStatus = (
  capabilities?: EngineCapabilities,
): EngineQualificationStatus => capabilities?.qualification?.status || 'unqualified'

type EngineCapabilityErrorLabels = {
  executableChanged: string
  executableChangedDuringProbe: string
}

export const localizeEngineCapabilityError = (
  error: string,
  labels: EngineCapabilityErrorLabels,
) => {
  const normalized = error.trim().replace(/[.\s]+$/g, '').toLowerCase()
  if (normalized === 'engine executable changed; compatibility probe required'
    || normalized === 'engine executable changed; compatibility probe and qualification required') {
    return labels.executableChanged
  }
  if (normalized === 'engine executable changed while compatibility probing was in progress; probe again') {
    return labels.executableChangedDuringProbe
  }
  return error
}

export type EngineCompatibilityMode = 'full' | 'recognized' | 'minimal'

export const getEngineCompatibilityMode = (capabilities?: EngineCapabilities): EngineCompatibilityMode => {
  const status = normalizeEngineCapabilityStatus(capabilities)
  if (status === 'detected') return 'full'
  if (status === 'partial') return 'recognized'
  return 'minimal'
}
