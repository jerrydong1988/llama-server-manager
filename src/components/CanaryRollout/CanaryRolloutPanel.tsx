import { useCallback, useEffect, useMemo, useState } from 'react'
import { Activity, ArrowUpCircle, GitBranch, RefreshCw, ShieldCheck, Undo2, XCircle } from 'lucide-react'
import { useI18n } from '../../i18n'
import { getCanaryRolloutLabels } from '../../i18n/pageLabels'
import { invokeApp as invoke } from '../../lib/ipc'
import { Badge, Button, SelectInput, StatusBadge, Surface, TextInput } from '../ui'
import {
  CanaryRequestEvidence,
  CanaryRollout,
  evidenceRate,
  normalizeCanaryRollouts,
  replaceCanaryRollout,
  shortRevision,
} from './canaryRollout'

export type CanaryTargetOption = {
  instanceId: string
  name: string
  status: 'running' | 'stopped' | 'unknown'
}

type Props = {
  proxyRunning: boolean
  targets: CanaryTargetOption[]
}

function errorMessage(error: unknown) {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  return String(error)
}

function stateTone(state: CanaryRollout['state']) {
  if (state === 'active') return 'blue' as const
  if (state === 'promoted') return 'emerald' as const
  return 'slate' as const
}

function formatPercent(value: number | null) {
  return value == null ? '—' : `${(value * 100).toFixed(1)}%`
}

function EvidenceSummary({ value, labels }: { value: CanaryRequestEvidence | null; labels: ReturnType<typeof getCanaryRolloutLabels> }) {
  if (!value) return <p className="text-xs text-slate-500 dark:text-slate-400">{labels.noEvidence}</p>
  return (
    <div className="grid grid-cols-2 gap-2 text-xs">
      <div className="rounded-lg bg-slate-100 px-3 py-2 dark:bg-slate-950/70">
        <div className="text-slate-500 dark:text-slate-400">{labels.requests}</div>
        <div className="mt-1 font-semibold text-slate-900 dark:text-slate-100">{value.total.toLocaleString()}</div>
      </div>
      <div className="rounded-lg bg-slate-100 px-3 py-2 dark:bg-slate-950/70">
        <div className="text-slate-500 dark:text-slate-400">{labels.successRate}</div>
        <div className="mt-1 font-semibold text-slate-900 dark:text-slate-100">{formatPercent(evidenceRate(value))}</div>
      </div>
    </div>
  )
}

export function CanaryRolloutPanel({ proxyRunning, targets }: Props) {
  const { lang } = useI18n()
  const labels = useMemo(() => getCanaryRolloutLabels(lang), [lang])
  const runningTargets = useMemo(() => targets.filter(target => target.status === 'running'), [targets])
  const [rollouts, setRollouts] = useState<CanaryRollout[]>([])
  const [stableId, setStableId] = useState('')
  const [candidateId, setCandidateId] = useState('')
  const [modelAlias, setModelAlias] = useState('')
  const [candidateWeight, setCandidateWeight] = useState(10)
  const [draftWeights, setDraftWeights] = useState<Record<string, number>>({})
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')

  const unresolved = rollouts.find(rollout => rollout.state === 'active' || rollout.state === 'promoted')

  useEffect(() => {
    const ids = runningTargets.map(target => target.instanceId)
    setStableId(current => ids.includes(current) ? current : ids[0] || '')
    setCandidateId(current => {
      if (ids.includes(current) && current !== (ids[0] || '')) return current
      return ids.find(id => id !== (ids[0] || '')) || ''
    })
  }, [runningTargets])

  const load = useCallback(async () => {
    setLoading(true)
    setError('')
    try {
      const value = await invoke<unknown>('list_canary_rollouts')
      setRollouts(normalizeCanaryRollouts(value))
    } catch (loadError) {
      setError(errorMessage(loadError))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => { void load() }, [load])

  const runAction = async (key: string, command: string, args: Record<string, unknown>, success: string) => {
    setBusy(key)
    setError('')
    setNotice('')
    try {
      const value = await invoke<unknown>(command, args)
      setRollouts(current => replaceCanaryRollout(current, value))
      setNotice(success)
    } catch (actionError) {
      setError(errorMessage(actionError))
    } finally {
      setBusy('')
    }
  }

  const create = async () => {
    await runAction('create', 'create_canary_rollout', {
      stableInstanceId: stableId,
      candidateInstanceId: candidateId,
      modelAlias: modelAlias.trim(),
      candidateWeight,
    }, labels.createdNotice)
  }

  const stateLabel = (rollout: CanaryRollout) => ({
    active: labels.active,
    promoted: labels.promoted,
    aborted: labels.aborted,
    rolled_back: labels.rolledBack,
  })[rollout.state]

  const targetName = (instanceId: string) => targets.find(target => target.instanceId === instanceId)?.name || instanceId
  const date = (timestamp: number) => timestamp > 0 ? new Intl.DateTimeFormat(lang, { dateStyle: 'short', timeStyle: 'medium' }).format(timestamp) : '—'
  const createReady = proxyRunning && runningTargets.length >= 2 && stableId.length > 0 && candidateId.length > 0
    && stableId !== candidateId && modelAlias.trim().length > 0 && !unresolved && !busy

  return (
    <Surface as="section" className="p-5" data-testid="canary-rollout-panel">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
        <div>
          <div className="flex flex-wrap items-center gap-2">
            <GitBranch className="h-5 w-5 text-blue-600 dark:text-blue-400" />
            <h3 className="text-lg font-semibold text-slate-950 dark:text-slate-50">{labels.title}</h3>
            <Badge tone="blue">Phase 2</Badge>
          </div>
          <p className="mt-1 max-w-4xl text-sm leading-6 text-slate-500 dark:text-slate-400">{labels.subtitle}</p>
          <p className="mt-2 flex max-w-4xl items-start gap-2 text-xs leading-5 text-slate-500 dark:text-slate-400">
            <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-emerald-600 dark:text-emerald-400" />
            {labels.safety}
          </p>
        </div>
        <Button onClick={() => void load()} disabled={loading || Boolean(busy)} icon={<RefreshCw className="h-4 w-4" />}>
          {labels.refresh}
        </Button>
      </div>

      {error || notice ? (
        <div className="mt-4 space-y-2">
          {error ? <div className="rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-500/20 dark:bg-red-500/10 dark:text-red-200">{error}</div> : null}
          {notice ? <div className="rounded-lg border border-emerald-200 bg-emerald-50 px-4 py-3 text-sm text-emerald-700 dark:border-emerald-500/20 dark:bg-emerald-500/10 dark:text-emerald-200">{notice}</div> : null}
        </div>
      ) : null}

      {!unresolved ? (
        <div className="mt-5 rounded-xl border border-slate-200 p-4 dark:border-slate-800">
          <h4 className="font-medium text-slate-900 dark:text-slate-100">{labels.newRollout}</h4>
          {!proxyRunning || runningTargets.length < 2 ? (
            <p className="mt-2 text-sm text-amber-700 dark:text-amber-300">{labels.proxyRequired}</p>
          ) : null}
          <div className="mt-4 grid gap-3 md:grid-cols-2 xl:grid-cols-[1fr_1fr_1fr_150px_auto] xl:items-end">
            <label className="block text-xs font-medium text-slate-600 dark:text-slate-300">
              <span className="mb-1 block">{labels.stable}</span>
              <SelectInput value={stableId} onChange={event => setStableId(event.target.value)} className="w-full">
                <option value="">{labels.selectInstance}</option>
                {runningTargets.map(target => <option key={target.instanceId} value={target.instanceId}>{target.name}</option>)}
              </SelectInput>
            </label>
            <label className="block text-xs font-medium text-slate-600 dark:text-slate-300">
              <span className="mb-1 block">{labels.candidate}</span>
              <SelectInput value={candidateId} onChange={event => setCandidateId(event.target.value)} className="w-full">
                <option value="">{labels.selectInstance}</option>
                {runningTargets.filter(target => target.instanceId !== stableId).map(target => <option key={target.instanceId} value={target.instanceId}>{target.name}</option>)}
              </SelectInput>
            </label>
            <label className="block text-xs font-medium text-slate-600 dark:text-slate-300">
              <span className="mb-1 block">{labels.publicAlias}</span>
              <TextInput value={modelAlias} onChange={event => setModelAlias(event.target.value)} placeholder={labels.publicAliasHint} />
            </label>
            <label className="block text-xs font-medium text-slate-600 dark:text-slate-300">
              <span className="mb-1 block">{labels.candidateShare}</span>
              <TextInput type="number" min={1} max={50} value={candidateWeight} onChange={event => setCandidateWeight(Math.min(50, Math.max(1, Number(event.target.value) || 1)))} />
            </label>
            <Button variant="primary" disabled={!createReady} onClick={() => void create()} icon={<GitBranch className="h-4 w-4" />}>
              {busy === 'create' ? labels.creating : labels.create}
            </Button>
          </div>
        </div>
      ) : (
        <p className="mt-4 text-xs text-slate-500 dark:text-slate-400">{labels.oneAtATime}</p>
      )}

      <div className="mt-5 space-y-4">
        {loading && rollouts.length === 0 ? <p className="text-sm text-slate-500 dark:text-slate-400">{labels.applying}</p> : null}
        {!loading && rollouts.length === 0 ? <p className="text-sm text-slate-500 dark:text-slate-400">{labels.noHistory}</p> : null}
        {rollouts.map(rollout => {
          const editableWeight = draftWeights[rollout.id] ?? Math.min(50, Math.max(1, rollout.candidateWeight || 10))
          return (
            <article key={rollout.id} className="rounded-xl border border-slate-200 p-4 dark:border-slate-800" data-rollout-state={rollout.state}>
              <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-semibold text-slate-950 dark:text-slate-50">{rollout.modelAlias}</span>
                    <StatusBadge tone={stateTone(rollout.state)}>{stateLabel(rollout)}</StatusBadge>
                    <Badge tone={rollout.integrityValid ? 'emerald' : 'red'}>{rollout.integrityValid ? labels.integrityOk : labels.integrityBad}</Badge>
                  </div>
                  <p className="mt-1 text-xs text-slate-500 dark:text-slate-400">{date(rollout.updatedAt)}</p>
                </div>
                <div className="flex flex-wrap gap-2">
                  {(rollout.state === 'active' || rollout.state === 'promoted') ? (
                    <Button disabled={Boolean(busy)} onClick={() => void runAction(`observe-${rollout.id}`, 'observe_canary_rollout', { rolloutId: rollout.id }, labels.observedNotice)} icon={<Activity className="h-4 w-4" />}>
                      {labels.observe}
                    </Button>
                  ) : null}
                  {rollout.canPromote ? (
                    <Button variant="success" disabled={Boolean(busy) || !rollout.candidateHealth.ready} onClick={() => window.confirm(labels.confirmPromote) && void runAction(`promote-${rollout.id}`, 'promote_canary_rollout', { rolloutId: rollout.id }, labels.promotedNotice)} icon={<ArrowUpCircle className="h-4 w-4" />}>
                      {labels.promote}
                    </Button>
                  ) : null}
                  {rollout.canAbort ? (
                    <Button variant="danger" disabled={Boolean(busy)} onClick={() => window.confirm(labels.confirmAbort) && void runAction(`abort-${rollout.id}`, 'abort_canary_rollout', { rolloutId: rollout.id }, labels.abortedNotice)} icon={<XCircle className="h-4 w-4" />}>
                      {labels.abort}
                    </Button>
                  ) : null}
                  {rollout.canRollback ? (
                    <Button variant="danger" disabled={Boolean(busy)} onClick={() => window.confirm(labels.confirmRollback) && void runAction(`rollback-${rollout.id}`, 'rollback_canary_rollout', { rolloutId: rollout.id }, labels.rollbackNotice)} icon={<Undo2 className="h-4 w-4" />}>
                      {labels.rollback}
                    </Button>
                  ) : null}
                </div>
              </div>

              {rollout.drift.length > 0 ? (
                <div className="mt-3 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800 dark:border-amber-500/20 dark:bg-amber-500/10 dark:text-amber-200">
                  <div className="font-semibold">{labels.drift}</div>
                  <ul className="mt-1 list-disc space-y-1 pl-4">{rollout.drift.map(item => <li key={item}>{item}</li>)}</ul>
                </div>
              ) : null}

              <div className="mt-4 grid gap-3 lg:grid-cols-2">
                {[
                  { role: labels.stable, id: rollout.stableInstanceId, revision: rollout.stableRevisionId, weight: rollout.stableWeight, health: rollout.stableHealth, evidence: rollout.stableEvidence },
                  { role: labels.candidate, id: rollout.candidateInstanceId, revision: rollout.candidateRevisionId, weight: rollout.candidateWeight, health: rollout.candidateHealth, evidence: rollout.candidateEvidence },
                ].map(target => (
                  <div key={target.role} className="rounded-lg bg-slate-50 p-3 dark:bg-slate-950/50">
                    <div className="flex items-center justify-between gap-3">
                      <div>
                        <div className="text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{target.role}</div>
                        <div className="mt-1 font-medium text-slate-900 dark:text-slate-100">{targetName(target.id)}</div>
                      </div>
                      <StatusBadge tone={target.health.ready ? 'emerald' : 'amber'}>{target.health.ready ? labels.healthy : labels.notReady}</StatusBadge>
                    </div>
                    <div className="mt-3 grid grid-cols-2 gap-2 text-xs text-slate-500 dark:text-slate-400">
                      <div><span className="block">{labels.traffic}</span><strong className="text-slate-900 dark:text-slate-100">{target.weight}%</strong></div>
                      <div><span className="block">{labels.revision}</span><strong className="font-mono text-slate-900 dark:text-slate-100">{shortRevision(target.revision)}</strong></div>
                    </div>
                    <div className="mt-3"><EvidenceSummary value={target.evidence} labels={labels} /></div>
                  </div>
                ))}
              </div>

              {rollout.canChangeTraffic ? (
                <div className="mt-4 flex flex-wrap items-end gap-3">
                  <label className="block w-40 text-xs font-medium text-slate-600 dark:text-slate-300">
                    <span className="mb-1 block">{labels.candidateShare}</span>
                    <TextInput type="number" min={1} max={50} value={editableWeight} onChange={event => setDraftWeights(current => ({ ...current, [rollout.id]: Math.min(50, Math.max(1, Number(event.target.value) || 1)) }))} />
                  </label>
                  <Button disabled={Boolean(busy) || editableWeight === rollout.candidateWeight} onClick={() => void runAction(`weight-${rollout.id}`, 'set_canary_weight', { rolloutId: rollout.id, candidateWeight: editableWeight }, labels.shareNotice)}>
                    {labels.applyShare}
                  </Button>
                </div>
              ) : null}

              <details className="mt-4">
                <summary className="cursor-pointer text-sm font-medium text-slate-700 dark:text-slate-300">{labels.audit} · {rollout.events.length}</summary>
                <ol className="mt-3 space-y-2">
                  {rollout.events.map(event => (
                    <li key={event.sequence} className="rounded-lg border border-slate-200 px-3 py-2 text-xs dark:border-slate-800">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <span className="font-medium text-slate-800 dark:text-slate-200">#{event.sequence} · {event.kind}</span>
                        <span className="text-slate-500 dark:text-slate-400">{date(event.occurredAt)}</span>
                      </div>
                      <p className="mt-1 text-slate-600 dark:text-slate-300">{event.summary}</p>
                    </li>
                  ))}
                </ol>
              </details>
            </article>
          )
        })}
      </div>
    </Surface>
  )
}
