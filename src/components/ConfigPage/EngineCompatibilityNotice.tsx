import { AlertTriangle, LoaderCircle } from 'lucide-react'
import type { EngineInfo } from '../../store'
import { normalizeEngineCapabilityStatus, normalizeEngineVersionStatus } from '../../engineCapabilities'
import type { getConfigPageLabels } from '../../i18n/configPageCopy'

type Props = {
  engine: EngineInfo | null
  unsupportedFlags: string[]
  probing: boolean
  labels: ReturnType<typeof getConfigPageLabels>
}

export function EngineCompatibilityNotice({ engine, unsupportedFlags, probing, labels }: Props) {
  if (unsupportedFlags.length > 0) {
    return (
      <div className="flex items-start gap-3 rounded-lg border border-ui-line bg-ui-danger-bg px-4 py-3 text-sm text-ui-danger">
        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
        <div className="min-w-0">
          <p className="font-medium">{labels.engineCompatibilityBlocked}</p>
          <p className="mt-1 text-ui-danger">{labels.engineCompatibilityBlockedDesc}</p>
          <div className="mt-2 flex flex-wrap gap-2">
            {unsupportedFlags.map(flag => (
              <code key={flag} className="rounded-md border border-ui-line bg-ui-surface px-2 py-1 text-xs">{flag}</code>
            ))}
          </div>
        </div>
      </div>
    )
  }
  if (!engine) return null
  if (probing) {
    return (
      <div className="flex items-center gap-3 rounded-lg border border-ui-line bg-ui-info-bg px-4 py-3 text-sm text-ui-info">
        <LoaderCircle className="h-4 w-4 shrink-0 animate-spin" />
        <span>{labels.engineCompatibilityChecking}</span>
      </div>
    )
  }
  const capabilityStatus = normalizeEngineCapabilityStatus(engine.capabilities)
  if (capabilityStatus !== 'detected') {
    const message = capabilityStatus === 'partial'
      ? labels.engineCompatibilityPartial
      : labels.engineCompatibilityMinimal
    return (
      <div className="flex items-start gap-3 rounded-lg border border-ui-line bg-ui-warn-bg px-4 py-3 text-sm text-ui-warn">
        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
        <span>{message}</span>
      </div>
    )
  }
  if (normalizeEngineVersionStatus(engine.capabilities) !== 'detected') {
    return (
      <div className="flex items-start gap-3 rounded-lg border border-ui-line bg-ui-warn-bg px-4 py-3 text-sm text-ui-warn">
        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
        <span>{labels.engineVersionUnknown}</span>
      </div>
    )
  }
  return null
}
