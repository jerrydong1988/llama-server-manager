import { useCallback, useEffect, useState } from 'react'
import { RefreshCw } from 'lucide-react'
import { useI18n } from '../../i18n'
import { useAppStore } from '../../store'
import type { DeploymentInspection, DeploymentState } from '../../store/types'
import { Badge, Button } from '../ui'

const shortId = (value?: string | null) => {
  if (!value) return '--'
  const parts = value.split(':')
  const suffix = parts[parts.length - 1] || value
  return suffix.length > 14 ? `${suffix.slice(0, 7)}…${suffix.slice(-7)}` : suffix
}

const toneForState = (state: DeploymentState) => {
  if (state === 'ready') return 'emerald' as const
  if (state === 'unmaterialized') return 'blue' as const
  if (state === 'stale') return 'amber' as const
  return 'red' as const
}

export function DeploymentPanel({
  instanceId,
  refreshKey,
}: {
  instanceId: string
  refreshKey: string
}) {
  const { t } = useI18n()
  const labels = t.instanceWorkspace
  const inspectDeployment = useAppStore(state => state.inspectDeployment)
  const [deployment, setDeployment] = useState<DeploymentInspection | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')

  const refresh = useCallback(async () => {
    setLoading(true)
    setError('')
    try {
      setDeployment(await inspectDeployment(instanceId))
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setLoading(false)
    }
  }, [inspectDeployment, instanceId])

  useEffect(() => {
    void refresh()
  }, [refresh, refreshKey])

  const stateLabel = deployment ? {
    ready: labels.deploymentReady,
    unmaterialized: labels.deploymentUnmaterialized,
    stale: labels.deploymentStale,
    invalid: labels.deploymentInvalid,
  }[deployment.state] : labels.deploymentLoading
  const current = deployment?.revisions.find(revision => revision.current)
  const stateHint = deployment ? {
    ready: '',
    unmaterialized: labels.deploymentUnmaterializedHint,
    stale: labels.deploymentStaleHint,
    invalid: labels.deploymentInvalidHint,
  }[deployment.state] : ''

  return (
    <section
      className="space-y-3 rounded-lg border border-slate-200 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-950/60"
      data-testid="deployment-panel"
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2">
            <h4 className="text-xs font-semibold text-slate-800 dark:text-slate-100">{labels.deploymentTitle}</h4>
            {deployment && <Badge tone={toneForState(deployment.state)}>{stateLabel}</Badge>}
          </div>
          <p className="mt-1 text-[11px] leading-4 text-slate-500 dark:text-slate-400">{labels.deploymentDescription}</p>
        </div>
        <Button
          variant="subtle"
          size="icon"
          aria-label={labels.deploymentRefresh}
          title={labels.deploymentRefresh}
          disabled={loading}
          onClick={() => void refresh()}
          icon={<RefreshCw className={`h-3.5 w-3.5 ${loading ? 'animate-spin' : ''}`} />}
        />
      </div>

      {loading && !deployment && <p className="text-xs text-slate-500">{labels.deploymentLoading}</p>}
      {error && <p className="text-xs text-rose-600 dark:text-rose-300">{labels.deploymentLoadFailed}: {error}</p>}
      {deployment && (
        <>
          {stateHint && (
            <p className="rounded-md bg-white px-2.5 py-2 text-xs text-slate-600 dark:bg-slate-900 dark:text-slate-300">
              {stateHint}
            </p>
          )}
          <div className="grid grid-cols-2 gap-2 text-[11px]">
            <div>
              <div className="text-slate-500">{labels.deploymentId}</div>
              <div className="mt-0.5 font-mono text-slate-800 dark:text-slate-100" title={deployment.deploymentId}>{shortId(deployment.deploymentId)}</div>
            </div>
            <div>
              <div className="text-slate-500">{labels.deploymentCurrentRevision}</div>
              <div className="mt-0.5 font-mono text-slate-800 dark:text-slate-100" title={deployment.currentRevisionId || ''}>{shortId(deployment.currentRevisionId)}</div>
            </div>
            <div>
              <div className="text-slate-500">{labels.deploymentRollbackTarget}</div>
              <div className="mt-0.5 font-mono text-slate-800 dark:text-slate-100" title={deployment.rollbackTargetRevisionId || ''}>{shortId(deployment.rollbackTargetRevisionId)}</div>
            </div>
            <div>
              <div className="text-slate-500">{labels.deploymentRunningRevision}</div>
              <div className="mt-0.5 font-mono text-slate-800 dark:text-slate-100" title={deployment.runningRevisionId || ''}>{shortId(deployment.runningRevisionId)}</div>
            </div>
          </div>

          {current && (
            <div className="rounded-md border border-slate-200 bg-white px-2.5 py-2 text-[11px] text-slate-600 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-300">
              <div className="flex flex-wrap gap-x-3 gap-y-1">
                <span>{labels.deploymentPolicy}: {current.runtimePolicy.autoStart ? labels.deploymentAutoStart : labels.deploymentManualStart} · {current.runtimePolicy.restartPolicy}</span>
                <span>{labels.deploymentRouting}: {current.routing.proxyEnabled ? labels.deploymentProxyEnabled : labels.deploymentProxyDisabled} · {current.routing.routes.length} {labels.deploymentRoutes}</span>
              </div>
              <div className="mt-1 truncate font-mono" title={current.deploymentIdentity.deploymentId}>
                {labels.deploymentCompositeIdentity}: {shortId(current.deploymentIdentity.deploymentId)}
              </div>
            </div>
          )}

          {deployment.revisions.length > 0 && (
            <details>
              <summary className="cursor-pointer text-xs font-medium text-blue-600 dark:text-blue-300">
                {labels.deploymentHistory} ({deployment.revisions.length})
              </summary>
              <div className="mt-2 space-y-1.5">
                {deployment.revisions.slice(0, 6).map(revision => (
                  <div key={revision.id} className="flex items-center justify-between gap-2 rounded-md bg-white px-2 py-1.5 text-[11px] dark:bg-slate-900">
                    <span className="min-w-0 truncate font-mono text-slate-700 dark:text-slate-200" title={revision.id}>{shortId(revision.id)}</span>
                    <span className="flex shrink-0 gap-1">
                      {revision.current && <Badge tone="emerald">{labels.deploymentCurrent}</Badge>}
                      {revision.rollbackTarget && <Badge tone="amber">{labels.deploymentRollback}</Badge>}
                      {!revision.integrityValid && <Badge tone="red">{labels.deploymentInvalid}</Badge>}
                    </span>
                  </div>
                ))}
              </div>
            </details>
          )}
        </>
      )}
    </section>
  )
}
