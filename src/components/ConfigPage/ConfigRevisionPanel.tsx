import { useEffect, useState } from 'react'
import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  History,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  X,
} from 'lucide-react'
import type { Translations } from '../../i18n'
import {
  useAppStore,
  type ConfigRevisionHistory,
  type ConfigRevisionSummary,
  type ConfigValueSummary,
  type DeploymentIdentityStatus,
  type InstanceConfig,
  type InstanceLifecyclePhase,
} from '../../store'
import { getConfigPageLabels } from '../../i18n/configPageCopy'
import { Badge, Button, InsetSurface } from '../ui'
import { fieldLabel } from './configWorkspace'
import { DeploymentIdentityCard } from './DeploymentIdentityCard'

type Labels = ReturnType<typeof getConfigPageLabels>

const reasonLabel = (revision: ConfigRevisionSummary, labels: Labels) => {
  switch (revision.reason) {
    case 'migration': return labels.revisionReasonMigration
    case 'created': return labels.revisionReasonCreated
    case 'save': return labels.revisionReasonSave
    case 'system': return labels.revisionReasonSystem
    case 'rollback': return labels.revisionReasonRollback
  }
}

const formatValueSummary = (summary: ConfigValueSummary, labels: Labels) => {
  switch (summary.state) {
    case 'empty': return labels.emptyValue
    case 'set': return labels.revisionValueSet
    case 'item_count': return `${summary.itemCount ?? 0} ${labels.revisionItemCount}`
    case 'value': return summary.value ?? labels.emptyValue
  }
}

const shortFingerprint = (fingerprint: string) => {
  const value = fingerprint.replace(/^sha256:/, '')
  return value.length > 16 ? `${value.slice(0, 16)}…` : value
}

const retainedExpandedRevision = (
  current: string | null,
  history: ConfigRevisionHistory,
) => current && history.revisions.some(revision => revision.id === current)
  ? current
  : history.currentRevisionId

export function ConfigRevisionPanel({
  instanceId,
  instanceStatus,
  lifecycle,
  refreshKey,
  lang,
  labels,
  t,
  onRollbackApplied,
}: {
  instanceId: string
  instanceStatus: string
  lifecycle?: InstanceLifecyclePhase
  refreshKey: number
  lang: string
  labels: Labels
  t: Translations
  onRollbackApplied: (config: InstanceConfig) => void
}) {
  const listConfigRevisions = useAppStore(state => state.listConfigRevisions)
  const inspectDeploymentIdentity = useAppStore(state => state.inspectDeploymentIdentity)
  const markConfigRevisionKnownGood = useAppStore(state => state.markConfigRevisionKnownGood)
  const rollbackConfigRevision = useAppStore(state => state.rollbackConfigRevision)
  const addRuntimeWarning = useAppStore(state => state.addRuntimeWarning)
  const [history, setHistory] = useState<ConfigRevisionHistory | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const [busyRevisionId, setBusyRevisionId] = useState<string | null>(null)
  const [rollbackTarget, setRollbackTarget] = useState<ConfigRevisionSummary | null>(null)
  const [identityStatus, setIdentityStatus] = useState<DeploymentIdentityStatus | null>(null)

  const loadHistory = async () => {
    setLoading(true)
    setError('')
    try {
      const next = await listConfigRevisions(instanceId)
      setHistory(next)
      setIdentityStatus(await inspectDeploymentIdentity(instanceId))
      setExpandedId(current => retainedExpandedRevision(current, next))
    } catch (loadError) {
      setError(String(loadError))
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    setError('')
    void Promise.all([
      listConfigRevisions(instanceId),
      inspectDeploymentIdentity(instanceId),
    ])
      .then(([next, status]) => {
        if (cancelled) return
        setHistory(next)
        setIdentityStatus(status)
        setExpandedId(current => retainedExpandedRevision(current, next))
      })
      .catch(loadError => {
        if (!cancelled) setError(String(loadError))
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [instanceId, inspectDeploymentIdentity, listConfigRevisions, refreshKey])

  const markKnownGood = async (revision: ConfigRevisionSummary) => {
    if (!history || busyRevisionId) return
    setBusyRevisionId(revision.id)
    setError('')
    try {
      setHistory(await markConfigRevisionKnownGood(instanceId, revision.id, history.currentFingerprint))
    } catch (actionError) {
      addRuntimeWarning(`${labels.revisionActionFailed}：${String(actionError)}`)
    } finally {
      setBusyRevisionId(null)
    }
  }

  const confirmRollback = async () => {
    if (!history || !rollbackTarget || busyRevisionId) return
    setBusyRevisionId(rollbackTarget.id)
    setError('')
    try {
      const result = await rollbackConfigRevision(
        instanceId,
        rollbackTarget.id,
        history.currentFingerprint,
      )
      setHistory(result.history)
      setExpandedId(result.history.currentRevisionId)
      setRollbackTarget(null)
      onRollbackApplied(result.config)
    } catch (actionError) {
      setError(String(actionError))
    } finally {
      setBusyRevisionId(null)
    }
  }

  const rollbackBlocked = instanceStatus !== 'stopped' || Boolean(lifecycle)

  return (
    <InsetSurface className="p-4" data-testid="config-revision-panel">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <History className="h-4 w-4 shrink-0 text-violet-400" />
            <p className="text-sm font-medium text-slate-900 dark:text-slate-100">{labels.revisionTitle}</p>
          </div>
          <p className="mt-1 text-xs leading-5 text-slate-500">{labels.revisionDesc}</p>
        </div>
        <Button
          variant="subtle"
          size="icon"
          onClick={() => void loadHistory()}
          disabled={loading || Boolean(busyRevisionId)}
          aria-label={labels.revisionRefresh}
          title={labels.revisionRefresh}
        >
          <RefreshCw className={`h-4 w-4 ${loading ? 'animate-spin' : ''}`} />
        </Button>
      </div>

      {identityStatus && <DeploymentIdentityCard status={identityStatus} labels={labels} />}

      {rollbackBlocked && (
        <p className="mt-3 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-xs leading-5 text-amber-700 dark:border-amber-500/20 dark:bg-amber-500/10 dark:text-amber-200">
          {labels.revisionRollbackBlocked}
        </p>
      )}

      {loading && !history ? (
        <p className="mt-3 text-sm text-slate-500">{labels.revisionLoading}</p>
      ) : error && !history ? (
        <div className="mt-3 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700 dark:border-red-500/20 dark:bg-red-500/10 dark:text-red-200">
          <p className="font-medium">{labels.revisionLoadFailed}</p>
          <p className="mt-1 break-words">{error}</p>
        </div>
      ) : !history || history.revisions.length === 0 ? (
        <p className="mt-3 text-sm text-slate-500">{labels.revisionEmpty}</p>
      ) : (
        <div className="mt-3 max-h-[560px] space-y-2 overflow-y-auto pr-1">
          {error && (
            <p className="rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700 dark:border-red-500/20 dark:bg-red-500/10 dark:text-red-200">
              {labels.revisionActionFailed}：{error}
            </p>
          )}
          {history.revisions.map(revision => {
            const expanded = expandedId === revision.id
            const busy = busyRevisionId === revision.id
            return (
              <div
                key={revision.id}
                data-revision-id={revision.id}
                className="rounded-lg border border-slate-200 bg-slate-50 dark:border-slate-800 dark:bg-slate-950/40"
              >
                <button
                  type="button"
                  className="flex w-full items-start gap-2 px-3 py-3 text-left"
                  onClick={() => setExpandedId(expanded ? null : revision.id)}
                  aria-expanded={expanded}
                >
                  {expanded
                    ? <ChevronDown className="mt-0.5 h-4 w-4 shrink-0 text-slate-400" />
                    : <ChevronRight className="mt-0.5 h-4 w-4 shrink-0 text-slate-400" />}
                  <span className="min-w-0 flex-1">
                    <span className="flex flex-wrap items-center gap-1.5">
                      <span className="text-xs font-semibold text-slate-800 dark:text-slate-200">{reasonLabel(revision, labels)}</span>
                      {revision.current && <Badge tone="blue" className="px-1.5 py-0.5 text-[9px]">{labels.revisionCurrent}</Badge>}
                      {revision.knownGood && <Badge tone="emerald" className="px-1.5 py-0.5 text-[9px]">{labels.revisionKnownGood}</Badge>}
                      {!revision.integrityValid && <Badge tone="red" className="px-1.5 py-0.5 text-[9px]">{labels.revisionIntegrityInvalid}</Badge>}
                    </span>
                    <span className="mt-1 block text-[11px] text-slate-500">
                      {new Date(revision.createdAt * 1000).toLocaleString(lang)}
                    </span>
                    <span className="mt-1 block truncate font-mono text-[10px] text-slate-500" title={revision.fingerprint}>
                      {labels.revisionFingerprint}: {shortFingerprint(revision.fingerprint)}
                    </span>
                  </span>
                </button>

                {expanded && (
                  <div className="border-t border-slate-200 px-3 py-3 dark:border-slate-800">
                    {!revision.integrityValid ? (
                      <p className="flex items-start gap-2 text-xs text-red-600 dark:text-red-300">
                        <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                        {labels.revisionIntegrityInvalid}
                      </p>
                    ) : revision.changes.length === 0 ? (
                      <p className="text-xs leading-5 text-slate-500">
                        {revision.parentRevisionId ? labels.revisionDiffUnavailable : labels.revisionReasonMigration}
                      </p>
                    ) : (
                      <div className="space-y-2">
                        {revision.changes.map(change => (
                          <div key={change.field} className="rounded-md border border-slate-200 bg-white px-2.5 py-2 text-xs dark:border-slate-800 dark:bg-slate-900/60">
                            <p className="truncate font-medium text-slate-800 dark:text-slate-200" title={change.field}>
                              {fieldLabel(change.field as keyof InstanceConfig, t)}
                            </p>
                            <div className="mt-1 grid min-w-0 grid-cols-[40px_minmax(0,1fr)] gap-x-2 gap-y-1">
                              <span className="text-slate-500">{labels.before}</span>
                              <span className="truncate text-slate-500" title={formatValueSummary(change.before, labels)}>{formatValueSummary(change.before, labels)}</span>
                              <span className="text-slate-500">{labels.after}</span>
                              <span className="truncate text-slate-800 dark:text-slate-200" title={formatValueSummary(change.after, labels)}>{formatValueSummary(change.after, labels)}</span>
                            </div>
                          </div>
                        ))}
                        {revision.diffTruncated && <p className="text-xs text-amber-600 dark:text-amber-300">{labels.revisionDiffTruncated}</p>}
                      </div>
                    )}

                    <div className="mt-3 flex flex-col gap-2">
                      {!revision.knownGood && revision.integrityValid && (
                        <Button
                          variant="secondary"
                          size="sm"
                          disabled={Boolean(busyRevisionId)}
                          onClick={() => void markKnownGood(revision)}
                          icon={<ShieldCheck className="h-3.5 w-3.5" />}
                        >
                          {labels.revisionMarkKnownGood}
                        </Button>
                      )}
                      {!revision.current && (
                        <Button
                          variant="secondary"
                          size="sm"
                          disabled={rollbackBlocked || !revision.integrityValid || Boolean(busyRevisionId)}
                          onClick={() => setRollbackTarget(revision)}
                          icon={<RotateCcw className={`h-3.5 w-3.5 ${busy ? 'animate-spin' : ''}`} />}
                        >
                          {labels.revisionRollback}
                        </Button>
                      )}
                    </div>
                  </div>
                )}
              </div>
            )
          })}

          {history.audit.length > 0 && (
            <div className="rounded-lg border border-slate-200 bg-slate-50 px-3 py-3 dark:border-slate-800 dark:bg-slate-950/40">
              <p className="text-xs font-medium text-slate-800 dark:text-slate-200">{labels.revisionAuditTitle}</p>
              <div className="mt-2 space-y-1.5">
                {history.audit.slice(0, 4).map(event => (
                  <p key={event.id} className="text-[11px] leading-4 text-slate-500">
                    {new Date(event.createdAt * 1000).toLocaleString(lang)} · {event.action === 'known_good_set' ? labels.revisionAuditSet : labels.revisionAuditInvalidated}
                  </p>
                ))}
              </div>
            </div>
          )}
        </div>
      )}

      {rollbackTarget && history && (
        <div className="fixed inset-0 z-[90] flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm" role="presentation">
          <div className="w-full max-w-lg rounded-xl border border-slate-200 bg-white p-5 shadow-2xl dark:border-slate-800 dark:bg-slate-950" role="alertdialog" aria-modal="true" aria-labelledby="config-rollback-title">
            <div className="flex items-start justify-between gap-4">
              <div>
                <h2 id="config-rollback-title" className="text-lg font-semibold text-slate-950 dark:text-slate-50">{labels.revisionRollbackTitle}</h2>
                <p className="mt-2 text-sm leading-6 text-slate-500">{labels.revisionRollbackDesc}</p>
              </div>
              <Button variant="subtle" size="icon" onClick={() => setRollbackTarget(null)} aria-label={labels.revisionRollbackCancel}>
                <X className="h-4 w-4" />
              </Button>
            </div>
            <div className="mt-4 rounded-lg border border-slate-200 bg-slate-50 px-3 py-3 dark:border-slate-800 dark:bg-slate-900/60">
              <p className="text-sm font-medium text-slate-800 dark:text-slate-200">{reasonLabel(rollbackTarget, labels)}</p>
              <p className="mt-1 font-mono text-xs text-slate-500">{shortFingerprint(rollbackTarget.fingerprint)}</p>
            </div>
            <div className="mt-5 flex justify-end gap-2">
              <Button variant="subtle" disabled={Boolean(busyRevisionId)} onClick={() => setRollbackTarget(null)}>{labels.revisionRollbackCancel}</Button>
              <Button disabled={Boolean(busyRevisionId)} onClick={() => void confirmRollback()} icon={<RotateCcw className={`h-4 w-4 ${busyRevisionId ? 'animate-spin' : ''}`} />}>
                {labels.revisionRollbackConfirm}
              </Button>
            </div>
          </div>
        </div>
      )}
    </InsetSurface>
  )
}
