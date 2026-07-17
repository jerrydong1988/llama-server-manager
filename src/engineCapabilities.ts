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

const commandFlag = (token: string) => {
  if (!token.startsWith('-')) return null
  const flag = token.split('=', 1)[0]
  const body = flag.replace(/^-+/, '')
  return body && /[a-z]/i.test(body) ? flag : null
}

export const findUnsupportedEngineFlags = (command: string[], capabilities?: EngineCapabilities) => {
  if (normalizeEngineCapabilityStatus(capabilities) !== 'detected') return []
  const supported = new Set(capabilities?.supportedFlags ?? [])
  return [...new Set(
    command
      .slice(1)
      .map(commandFlag)
      .filter((flag): flag is string => Boolean(flag) && !supported.has(flag as string)),
  )].sort()
}
