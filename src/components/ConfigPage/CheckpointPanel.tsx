import { useEffect, useRef, useState } from 'react'
import { AlertTriangle, DatabaseZap, ShieldCheck } from 'lucide-react'
import { confirm } from '@tauri-apps/plugin-dialog'
import type { Translations } from '../../i18n'
import { useAppStore, type CheckpointEligibility, type InstanceConfig, type KvCheckpointConfig } from '../../store'
import { invokeApp as invoke } from '../../lib/ipc'
import { Badge, Button, InsetSurface, SectionHeader, Surface, TextInput } from '../ui'
import { Toggle } from './shared'

const REQUIRED_SETTING_REASONS = new Set([
  'parallel_must_be_one',
  'prompt_cache_required',
  'slots_required',
])

const boundedInteger = (value: string, minimum: number, maximum: number) => {
  const parsed = Number.parseInt(value, 10)
  if (!Number.isFinite(parsed)) return minimum
  return Math.min(maximum, Math.max(minimum, parsed))
}

export function CheckpointPanel({
  config,
  engineExe,
  instanceId,
  labels,
  onCheckpointChange,
  onApplyRequirements,
}: {
  config: InstanceConfig
  engineExe?: string
  instanceId: string
  labels: Translations['checkpoint']
  onCheckpointChange: (next: KvCheckpointConfig) => void
  onApplyRequirements: () => void
}) {
  const [eligibility, setEligibility] = useState<CheckpointEligibility | null>(null)
  const [loading, setLoading] = useState(false)
  const mountedRef = useRef(true)
  const policy = config.kv_checkpoint
  const reasons = eligibility?.reasons ?? []
  const canRepair = policy.enabled && (
    config.parallel !== 1
    || !config.cache_prompt
    || !config.slots_enabled
    || reasons.some(reason => REQUIRED_SETTING_REASONS.has(reason))
  )
  const update = <K extends keyof KvCheckpointConfig>(key: K, value: KvCheckpointConfig[K]) => {
    onCheckpointChange({ ...policy, [key]: value })
  }

  useEffect(() => {
    mountedRef.current = true
    return () => { mountedRef.current = false }
  }, [])

  useEffect(() => {
    let disposed = false
    if (!policy.enabled) {
      setEligibility({ eligible: false, reason_code: 'disabled', reasons: ['disabled'] })
      setLoading(false)
      return () => { disposed = true }
    }
    if (config.launch_mode !== 'managed') {
      setEligibility({ eligible: false, reason_code: 'manual_launch_unsupported', reasons: ['manual_launch_unsupported'] })
      setLoading(false)
      return () => { disposed = true }
    }
    if (!engineExe) {
      setEligibility({ eligible: false, reason_code: 'engine_capability_missing', reasons: ['engine_capability_missing'] })
      setLoading(false)
      return () => { disposed = true }
    }

    setLoading(true)
    const timer = setTimeout(() => {
      void invoke<CheckpointEligibility>('get_checkpoint_eligibility', { config, engineExe })
        .then(result => { if (!disposed) setEligibility(result) })
        .catch(() => {
          if (!disposed) setEligibility({
            eligible: false,
            reason_code: 'unsupported_configuration',
            reasons: ['unsupported_configuration'],
          })
        })
        .finally(() => { if (!disposed) setLoading(false) })
    }, 250)
    return () => {
      disposed = true
      clearTimeout(timer)
    }
  }, [config, engineExe, policy.enabled])

  const applyRequirements = async () => {
    if (!await confirm(labels.applyRequirementsConfirm, { title: labels.configTitle, kind: 'warning' })) return
    if (mountedRef.current && useAppStore.getState().activeConfigInstanceId === instanceId) onApplyRequirements()
  }

  return (
    <Surface as="section" id="config-checkpoint" className="p-5">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="flex min-w-0 items-start gap-3">
          <div className="rounded-lg border border-violet-500/20 bg-violet-500/10 p-3 text-violet-400">
            <DatabaseZap className="h-5 w-5" />
          </div>
          <SectionHeader title={labels.configTitle} description={labels.configDescription} />
        </div>
        <Badge tone="violet" className="shrink-0">{labels.experimentalBadge}</Badge>
      </div>

      <div className="mt-4 grid gap-3 lg:grid-cols-3">
        <InsetSurface className="p-3 text-xs leading-5 text-amber-700 dark:text-amber-200">
          <AlertTriangle className="mb-2 h-4 w-4" />
          {labels.experimental}
        </InsetSurface>
        <InsetSurface className="p-3 text-xs leading-5 text-slate-600 dark:text-slate-300">
          <ShieldCheck className="mb-2 h-4 w-4 text-emerald-400" />
          {labels.sensitive}
        </InsetSurface>
        <InsetSurface className="p-3 text-xs leading-5 text-blue-700 dark:text-blue-200">
          <DatabaseZap className="mb-2 h-4 w-4" />
          {labels.proxyRequirement}
        </InsetSurface>
      </div>

      <div className="mt-5 grid gap-4 sm:grid-cols-2 xl:grid-cols-5">
        <Toggle label={labels.enabled} value={policy.enabled} onChange={value => update('enabled', value)} title={labels.enabledTip} />
        <Toggle label={labels.autoSave} value={policy.auto_save} onChange={value => update('auto_save', value)} title={labels.autoSaveTip} disabled={!policy.enabled} />
        <Toggle label={labels.autoRestore} value={policy.auto_restore} onChange={value => update('auto_restore', value)} title={labels.autoRestoreTip} disabled={!policy.enabled} />
        <label className={!policy.enabled ? 'opacity-50' : ''} title={labels.storageLimitTip}>
          <span className="mb-1 block text-xs font-medium text-slate-500 dark:text-slate-400">{labels.storageLimit}</span>
          <TextInput
            type="number"
            min={1}
            max={1024}
            step={1}
            value={policy.storage_limit_gib}
            disabled={!policy.enabled}
            onChange={event => update('storage_limit_gib', boundedInteger(event.target.value, 1, 1024))}
            className="h-10 w-full"
          />
        </label>
        <label className={!policy.enabled ? 'opacity-50' : ''} title={labels.minimumTokensTip}>
          <span className="mb-1 block text-xs font-medium text-slate-500 dark:text-slate-400">{labels.minimumTokens}</span>
          <TextInput
            type="number"
            min={1}
            max={1_048_576}
            step={1}
            value={policy.minimum_prompt_tokens}
            disabled={!policy.enabled}
            onChange={event => update('minimum_prompt_tokens', boundedInteger(event.target.value, 1, 1_048_576))}
            className="h-10 w-full"
          />
        </label>
      </div>

      {policy.enabled && (
        <InsetSurface className="mt-5 p-4">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="min-w-0">
              <p className="text-xs font-semibold uppercase tracking-wide text-slate-500">{labels.eligibility}</p>
              <p className={`mt-1 text-sm font-medium ${eligibility?.eligible ? 'text-emerald-600 dark:text-emerald-300' : 'text-amber-700 dark:text-amber-200'}`}>
                {loading ? labels.checking : eligibility?.eligible ? labels.eligible : labels.ineligible}
              </p>
              {!loading && reasons.length > 0 && (
                <ul className="mt-2 space-y-1 text-xs text-slate-600 dark:text-slate-300">
                  {reasons.map(reason => (
                    <li key={reason}>• {labels.reasons[reason as keyof typeof labels.reasons] || reason}</li>
                  ))}
                </ul>
              )}
              <p className="mt-3 text-xs text-slate-500 dark:text-slate-400">{labels.requiredSettings}</p>
            </div>
            {canRepair && (
              <Button type="button" onClick={() => void applyRequirements()} variant="secondary" size="sm" className="shrink-0">
                {labels.applyRequirements}
              </Button>
            )}
          </div>
        </InsetSurface>
      )}
    </Surface>
  )
}
