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
      <div className="flex items-start gap-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-500/20 dark:bg-red-500/10 dark:text-red-200">
        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
        <div className="min-w-0">
          <p className="font-medium">{labels.engineCompatibilityBlocked}</p>
          <p className="mt-1 text-red-700/80 dark:text-red-200/80">{labels.engineCompatibilityBlockedDesc}</p>
          <div className="mt-2 flex flex-wrap gap-2">
            {unsupportedFlags.map(flag => (
              <code key={flag} className="rounded-md border border-red-200 bg-white/70 px-2 py-1 text-xs dark:border-red-500/20 dark:bg-slate-950/30">{flag}</code>
            ))}
          </div>
        </div>
      </div>
    )
  }
  if (!engine) return null
  if (probing) {
    return (
      <div className="flex items-center gap-3 rounded-lg border border-blue-200 bg-blue-50 px-4 py-3 text-sm text-blue-700 dark:border-blue-500/20 dark:bg-blue-500/10 dark:text-blue-200">
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
      <div className="flex items-start gap-3 rounded-lg border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-700 dark:border-amber-500/20 dark:bg-amber-500/10 dark:text-amber-200">
        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
        <span>{message}</span>
      </div>
    )
  }
  if (normalizeEngineVersionStatus(engine.capabilities) !== 'detected') {
    return (
      <div className="flex items-start gap-3 rounded-lg border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-700 dark:border-amber-500/20 dark:bg-amber-500/10 dark:text-amber-200">
        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
        <span>{labels.engineVersionUnknown}</span>
      </div>
    )
  }
  return null
}
