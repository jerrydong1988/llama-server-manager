import { useCallback, useEffect, useMemo, useState } from 'react'
import { Activity, Database, HardDrive, RefreshCw, Save, Zap } from 'lucide-react'
import { invokeApp as invoke } from '../../lib/ipc'
import { useI18n } from '../../i18n'
import { getResidencyCopy } from '../../i18n/residencyCopy'
import { useAppStore } from '../../store'
import type {
  ResidencyDrainStatus,
  ResidencyInspection,
  ResidencyIntent,
  ResidencyOperation,
  ResidencyPolicy,
} from '../../store/types'
import { Badge, Button, InsetSurface, SectionHeader, Surface, TextInput } from '../ui'
import { buildResidencyPolicy, orderedResidencyOperations, RESIDENCY_GIB } from './residencyReconcile'

const errorMessage = (error: unknown) => error instanceof Error ? error.message : String(error)
const toGiB = (bytes: number) => Math.round((bytes / RESIDENCY_GIB) * 100) / 100
const sleep = (ms: number) => new Promise(resolve => window.setTimeout(resolve, ms))

function operationLabel(kind: ResidencyOperation['kind'], copy: ReturnType<typeof getResidencyCopy>) {
  if (kind === 'drain') return copy.operationDrain
  if (kind === 'evict') return copy.operationEvict
  return copy.operationWarm
}

export default function ResidencySchedulerPanel() {
  const { lang } = useI18n()
  const copy = useMemo(() => getResidencyCopy(lang), [lang])
  const instances = useAppStore(state => state.instances)
  const startInstance = useAppStore(state => state.startInstance)
  const stopInstance = useAppStore(state => state.stopInstance)
  const addRuntimeWarning = useAppStore(state => state.addRuntimeWarning)
  const [inspection, setInspection] = useState<ResidencyInspection | null>(null)
  const [enabled, setEnabled] = useState(false)
  const [ramGiB, setRamGiB] = useState(16)
  const [vramGiB, setVramGiB] = useState(0)
  const [drainTimeout, setDrainTimeout] = useState(120)
  const [intents, setIntents] = useState<Record<string, ResidencyIntent>>({})
  const [busy, setBusy] = useState<'loading' | 'saving' | 'applying' | null>('loading')
  const [notice, setNotice] = useState('')
  const [error, setError] = useState('')

  const hydrate = useCallback((next: ResidencyInspection) => {
    setInspection(next)
    setEnabled(next.policy.enabled)
    setRamGiB(next.policy.ramBudgetBytes > 0 ? toGiB(next.policy.ramBudgetBytes) : 16)
    setVramGiB(toGiB(next.policy.vramBudgetBytes))
    setDrainTimeout(next.policy.drainTimeoutSeconds)
    setIntents(Object.fromEntries(next.policy.intents.map(intent => [intent.instanceId, intent])))
  }, [])

  const refresh = useCallback(async () => {
    setBusy('loading')
    setError('')
    try {
      hydrate(await invoke<ResidencyInspection>('inspect_model_residency'))
    } catch (refreshError) {
      setError(errorMessage(refreshError))
    } finally {
      setBusy(null)
    }
  }, [hydrate])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const policy = useCallback((): ResidencyPolicy => buildResidencyPolicy({
    enabled,
    ramGiB,
    vramGiB,
    drainTimeoutSeconds: drainTimeout,
    instanceIds: instances.map(instance => instance.id),
    intents,
  }), [drainTimeout, enabled, instances, intents, ramGiB, vramGiB])

  const savePolicy = useCallback(async (quiet = false) => {
    setBusy('saving')
    setError('')
    setNotice('')
    try {
      const next = await invoke<ResidencyInspection>('save_model_residency_policy', { policy: policy() })
      hydrate(next)
      if (!quiet) setNotice(copy.policySaved)
      return next
    } catch (saveError) {
      const message = errorMessage(saveError)
      setError(message)
      throw saveError
    } finally {
      setBusy(null)
    }
  }, [copy.policySaved, hydrate, policy])

  const complete = useCallback(async (
    operation: ResidencyOperation,
    planId: string,
    success: boolean,
    operationError?: string,
  ) => invoke<ResidencyInspection>('complete_model_residency_operation', {
    action: operation.kind,
    instanceId: operation.instanceId,
    deploymentId: operation.deploymentId,
    revisionId: operation.revisionId,
    planId,
    success,
    error: operationError || null,
  }), [])

  const applyPlan = useCallback(async () => {
    setBusy('applying')
    setError('')
    setNotice('')
    try {
      const saved = await savePolicy(true)
      setBusy('applying')
      const { plan } = saved
      for (const operation of orderedResidencyOperations(plan.operations)) {
        if (operation.kind === 'drain') {
          await invoke<ResidencyDrainStatus>('begin_model_residency_drain', {
            instanceId: operation.instanceId,
            revisionId: operation.revisionId,
            planId: plan.planId,
          })
          const deadline = Date.now() + saved.policy.drainTimeoutSeconds * 1000
          while (true) {
            const status = await invoke<ResidencyDrainStatus>('get_model_residency_drain_status', {
              instanceId: operation.instanceId,
            })
            if (status.activeRequests === 0) break
            if (Date.now() >= deadline) {
              const timeoutError = `drain timeout with ${status.activeRequests} active request(s)`
              await complete({ ...operation, kind: 'evict' }, plan.planId, false, timeoutError)
              throw new Error(timeoutError)
            }
            await sleep(500)
          }
          continue
        }
        if (operation.kind === 'evict') {
          try {
            await stopInstance(operation.instanceId)
            await complete(operation, plan.planId, true)
          } catch (operationError) {
            await complete(operation, plan.planId, false, errorMessage(operationError))
            throw operationError
          }
          continue
        }
        try {
          const instance = useAppStore.getState().instances.find(item => item.id === operation.instanceId)
          if (!instance || instance.status !== 'running') {
            await startInstance(operation.instanceId, false)
          }
          await complete(operation, plan.planId, true)
        } catch (operationError) {
          await complete(operation, plan.planId, false, errorMessage(operationError))
          throw operationError
        }
      }
      const next = await invoke<ResidencyInspection>('inspect_model_residency')
      hydrate(next)
      setNotice(copy.planApplied)
    } catch (applyError) {
      const message = `${copy.failurePrefix}${errorMessage(applyError)}`
      setError(message)
      addRuntimeWarning(message)
    } finally {
      setBusy(null)
    }
  }, [addRuntimeWarning, complete, copy.failurePrefix, copy.planApplied, hydrate, savePolicy, startInstance, stopInstance])

  const setIntent = (instanceId: string, partial: Partial<ResidencyIntent>, index: number) => {
    setIntents(current => {
      const previous = current[instanceId] || {
        instanceId,
        priority: (index + 1) * 10,
        enabled: false,
      }
      return {
        ...current,
        [instanceId]: { ...previous, ...partial, instanceId },
      }
    })
  }

  if (busy === 'loading' && !inspection) {
    return <Surface className="p-6 text-sm text-slate-500" data-testid="residency-panel">{copy.loading}</Surface>
  }

  const plan = inspection?.plan
  const operationCount = plan?.operations.length || 0
  return (
    <Surface className="p-6" data-testid="residency-panel">
      <SectionHeader
        title={copy.title}
        description={copy.description}
        action={(
          <div className="flex flex-wrap gap-2">
            <Button onClick={() => void refresh()} disabled={Boolean(busy)} size="sm" icon={<RefreshCw className="h-3.5 w-3.5" />}>{copy.refresh}</Button>
            <Button onClick={() => void savePolicy()} disabled={Boolean(busy)} size="sm" icon={<Save className="h-3.5 w-3.5" />}>{busy === 'saving' ? copy.saving : copy.savePolicy}</Button>
            <Button onClick={() => void applyPlan()} disabled={Boolean(busy) || !enabled} variant="violet" size="sm" icon={<Zap className="h-3.5 w-3.5" />}>{busy === 'applying' ? copy.applying : `${copy.applyPlan}${operationCount ? ` (${operationCount})` : ''}`}</Button>
          </div>
        )}
      />

      <div className="mt-5 grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(260px,0.42fr)]">
        <div className="space-y-4">
          <InsetSurface className="grid gap-4 p-4 sm:grid-cols-2 xl:grid-cols-4">
            <label className="flex items-center gap-3 text-sm text-slate-700 dark:text-slate-200">
              <input data-testid="residency-enabled" type="checkbox" checked={enabled} onChange={event => setEnabled(event.target.checked)} className="h-4 w-4 rounded" />
              {copy.enabled}
            </label>
            <label className="text-xs text-slate-500">
              {copy.ramBudget}
              <TextInput data-testid="residency-ram-budget" className="mt-2 h-9 w-full" type="number" min="0.01" step="0.25" value={ramGiB} onChange={event => setRamGiB(Number(event.target.value) || 0)} />
            </label>
            <label className="text-xs text-slate-500">
              {copy.vramBudget}
              <TextInput data-testid="residency-vram-budget" className="mt-2 h-9 w-full" type="number" min="0" step="0.25" value={vramGiB} onChange={event => setVramGiB(Number(event.target.value) || 0)} />
            </label>
            <label className="text-xs text-slate-500">
              {copy.drainTimeout}
              <TextInput className="mt-2 h-9 w-full" type="number" min="5" max="3600" value={drainTimeout} onChange={event => setDrainTimeout(Number(event.target.value) || 5)} />
            </label>
          </InsetSurface>

          <div className="space-y-2" data-testid="residency-intents">
            {instances.length === 0 ? (
              <InsetSurface className="p-4 text-sm text-slate-500">{copy.noInstances}</InsetSurface>
            ) : instances.map((instance, index) => {
              const intent = intents[instance.id] || { instanceId: instance.id, priority: (index + 1) * 10, enabled: false }
              const decision = plan?.decisions.find(item => item.instanceId === instance.id)
              return (
                <InsetSurface key={instance.id} className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:justify-between">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-sm font-medium text-slate-900 dark:text-slate-100">{instance.name}</span>
                      {decision ? <Badge tone={decision.selected ? 'emerald' : 'slate'}>{decision.selected ? copy.selected : copy.blocked}</Badge> : null}
                    </div>
                    <p className="mt-1 truncate text-xs text-slate-500">{decision?.reasons.join(' · ') || instance.id}</p>
                  </div>
                  <div className="flex items-center gap-4">
                    <label className="flex items-center gap-2 text-xs text-slate-500">
                      <input data-testid={`residency-intent-${instance.id}`} type="checkbox" checked={intent.enabled} onChange={event => setIntent(instance.id, { enabled: event.target.checked }, index)} />
                      {copy.eligible}
                    </label>
                    <label className="text-xs text-slate-500" title={copy.priorityHint}>
                      {copy.priority}
                      <TextInput className="ml-2 h-8 w-20" type="number" value={intent.priority} onChange={event => setIntent(instance.id, { priority: Number(event.target.value) || 0 }, index)} />
                    </label>
                  </div>
                </InsetSurface>
              )
            })}
          </div>
        </div>

        <div className="space-y-4">
          <InsetSurface className="p-4">
            <div className="flex items-center gap-2 text-sm font-medium text-slate-900 dark:text-slate-100"><Database className="h-4 w-4" />{copy.plan}</div>
            <div className="mt-4 space-y-3 text-xs text-slate-500">
              <div className="flex justify-between gap-3"><span>{copy.ramUsage}</span><span>{toGiB(plan?.ramUsedBytes || 0)} / {toGiB(plan?.ramBudgetBytes || 0)} GiB</span></div>
              <div className="flex justify-between gap-3"><span>{copy.vramUsage}</span><span>{toGiB(plan?.vramUsedBytes || 0)} / {toGiB(plan?.vramBudgetBytes || 0)} GiB</span></div>
              <div><span>{copy.planId}</span><code className="mt-1 block break-all text-[10px] text-slate-400">{plan?.planId || '—'}</code></div>
            </div>
          </InsetSurface>
          <InsetSurface className="p-4">
            <div className="flex items-center gap-2 text-sm font-medium text-slate-900 dark:text-slate-100"><Activity className="h-4 w-4" />{copy.operations}</div>
            <div className="mt-3 space-y-2" data-testid="residency-operations">
              {!plan || plan.operations.length === 0 ? <p className="text-xs text-slate-500">{copy.noOperations}</p> : plan.operations.map(operation => (
                <div key={`${operation.sequence}-${operation.kind}-${operation.instanceId}`} className="flex items-center justify-between gap-3 text-xs">
                  <span className="text-slate-300">{operation.sequence}. {operationLabel(operation.kind, copy)}</span>
                  <span className="truncate text-slate-500">{instances.find(item => item.id === operation.instanceId)?.name || operation.instanceId}</span>
                </div>
              ))}
            </div>
          </InsetSurface>
          <InsetSurface className="p-4">
            <div className="flex items-center gap-2 text-sm font-medium text-slate-900 dark:text-slate-100"><HardDrive className="h-4 w-4" />{copy.workers}: {inspection?.registeredRpcWorkers || 0}</div>
            <p className="mt-2 text-xs leading-5 text-amber-600 dark:text-amber-300">{inspection?.registeredRpcWorkers ? copy.workerBoundary : copy.singleNode}</p>
          </InsetSurface>
        </div>
      </div>

      {inspection?.audit.length ? (
        <div className="mt-5">
          <p className="mb-2 text-xs font-medium uppercase tracking-wide text-slate-500">{copy.audit}</p>
          <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-3">
            {inspection.audit.slice(0, 6).map(event => (
              <InsetSurface key={event.id} className="p-3 text-xs">
                <div className="flex justify-between gap-3"><span className="font-medium text-slate-300">{event.action}</span><Badge tone={event.outcome === 'failed' ? 'red' : 'slate'}>{event.outcome}</Badge></div>
                <p className="mt-2 truncate text-slate-500">{event.instanceId || copy.title}</p>
                {event.message ? <p className="mt-1 text-red-400">{copy.lastError}: {event.message}</p> : null}
              </InsetSurface>
            ))}
          </div>
        </div>
      ) : null}

      {notice ? <div className="mt-4 rounded-lg border border-emerald-500/20 bg-emerald-500/10 p-3 text-sm text-emerald-300">{notice}</div> : null}
      {error ? <div className="mt-4 whitespace-pre-wrap rounded-lg border border-red-500/20 bg-red-500/10 p-3 text-sm text-red-300">{error}</div> : null}
    </Surface>
  )
}
