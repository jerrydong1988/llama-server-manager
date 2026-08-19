import { ShieldCheck } from 'lucide-react'
import type { DeploymentIdentityStatus } from '../../store'
import { getConfigPageLabels } from '../../i18n/configPageCopy'
import { Badge } from '../ui'

type Labels = ReturnType<typeof getConfigPageLabels>

const shortIdentity = (identity: string) => {
  const parts = identity.split(':')
  const value = parts[parts.length - 1] || identity
  return value.length > 16 ? `${value.slice(0, 16)}…` : value
}

export function DeploymentIdentityCard({
  status,
  labels,
}: {
  status: DeploymentIdentityStatus
  labels: Labels
}) {
  return (
    <div
      className={`mt-3 rounded-lg border px-3 py-3 ${status.ready
        ? 'border-emerald-200 bg-emerald-50 dark:border-emerald-500/20 dark:bg-emerald-500/10'
        : 'border-amber-200 bg-amber-50 dark:border-amber-500/20 dark:bg-amber-500/10'}`}
      data-testid="deployment-identity-status"
    >
      <div className="flex items-center justify-between gap-2">
        <p className="flex items-center gap-2 text-xs font-semibold text-slate-800 dark:text-slate-200">
          <ShieldCheck className="h-3.5 w-3.5" />
          {labels.deploymentIdentityTitle}
        </p>
        <Badge tone={status.ready ? 'emerald' : 'amber'} className="px-1.5 py-0.5 text-[9px]">
          {status.ready ? labels.deploymentIdentityReady : labels.deploymentIdentityBlocked}
        </Badge>
      </div>
      <p className="mt-1 text-[11px] leading-4 text-slate-500">{labels.deploymentIdentityDesc}</p>
      {status.identity ? (
        <div className="mt-2 space-y-1 font-mono text-[10px] text-slate-500">
          <p title={status.identity.deploymentId}>{labels.deploymentIdentityId}: {shortIdentity(status.identity.deploymentId)}</p>
          <p title={status.identity.configurationId}>{labels.deploymentIdentityConfig}: {shortIdentity(status.identity.configurationId)}</p>
          <p title={status.identity.qualificationEvidenceId}>{labels.deploymentIdentityEvidence}: {shortIdentity(status.identity.qualificationEvidenceId)}</p>
        </div>
      ) : (
        <div className="mt-2 text-[11px] leading-4 text-amber-700 dark:text-amber-200">
          {status.errorCode && <p className="font-mono">{status.errorCode}</p>}
          <p className="mt-1">{labels.deploymentIdentityBlockedHint}</p>
        </div>
      )}
    </div>
  )
}
