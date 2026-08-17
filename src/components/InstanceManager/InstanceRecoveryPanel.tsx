import { ArrowDown, ArrowUp, ShieldCheck, Square, TriangleAlert } from 'lucide-react'
import { formatMessage, useI18n } from '../../i18n'
import type { Instance } from '../../store/types'
import { Badge, Button } from '../ui'

type InstanceRecoveryPanelProps = {
  instance: Instance
  statusLabel: string
  lifecycleBusy: boolean
  onCancel: () => void
}

export const InstanceRecoveryPanel = ({
  instance,
  statusLabel,
  lifecycleBusy,
  onCancel,
}: InstanceRecoveryPanelProps) => {
  const { t } = useI18n()
  const labels = t.instanceWorkspace
  const recovery = instance.recovery
  if (!recovery) return null

  const isTerminal = recovery.phase === 'crash_loop' || recovery.phase === 'failed'
  const failureLabel = (kind: 'startup_failure' | 'unexpected_exit') => (
    kind === 'startup_failure' ? labels.startupFailure : labels.unexpectedExit
  )
  const retryTime = recovery.next_retry_at
    ? new Date(recovery.next_retry_at * 1000).toLocaleTimeString([], {
      hour: '2-digit', minute: '2-digit', second: '2-digit',
    })
    : ''
  const failureMetadata = (failure: typeof recovery.origin_failure) => {
    const occurredAt = new Date(failure.occurred_at * 1000).toLocaleString()
    const exitCode = failure.exit_code == null
      ? null
      : formatMessage(labels.exitCode, { code: failure.exit_code })
    return [formatMessage(labels.occurredAt, { time: occurredAt }), exitCode]
      .filter(Boolean)
      .join(' · ')
  }
  const hasDistinctLatestFailure = recovery.last_failure.kind !== recovery.origin_failure.kind
    || recovery.last_failure.message !== recovery.origin_failure.message
    || recovery.last_failure.exit_code !== recovery.origin_failure.exit_code
    || recovery.last_failure.occurred_at !== recovery.origin_failure.occurred_at

  return (
    <div className={`rounded-lg border p-3 ${isTerminal ? 'border-rose-200 bg-rose-50 dark:border-rose-500/20 dark:bg-rose-500/10' : 'border-amber-200 bg-amber-50 dark:border-amber-500/20 dark:bg-amber-500/10'}`}>
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <TriangleAlert className={`h-4 w-4 shrink-0 ${isTerminal ? 'text-rose-600 dark:text-rose-300' : 'text-amber-600 dark:text-amber-300'}`} />
          <div>
            <div className="text-xs font-semibold text-slate-900 dark:text-slate-100">{labels.recoveryStatus}</div>
            <div className="mt-1 text-[11px] text-slate-600 dark:text-slate-300">
              {formatMessage(labels.recoveryAttempts, {
                current: recovery.restart_attempts,
                max: recovery.max_restart_attempts,
              })}
            </div>
          </div>
        </div>
        <Badge tone={isTerminal ? 'red' : 'amber'}>{statusLabel}</Badge>
      </div>
      {retryTime && (
        <div className="mt-2 text-xs font-medium text-amber-700 dark:text-amber-300">
          {formatMessage(labels.nextRetry, { time: retryTime })}
        </div>
      )}
      <div className="mt-3 space-y-2 text-xs">
        <div>
          <div className="font-medium text-slate-700 dark:text-slate-200">{labels.originFailure} · {failureLabel(recovery.origin_failure.kind)}</div>
          <div className="mt-1 break-words text-slate-600 dark:text-slate-400">{recovery.origin_failure.message}</div>
          <div className="mt-1 text-[11px] text-slate-500 dark:text-slate-500">{failureMetadata(recovery.origin_failure)}</div>
        </div>
        {hasDistinctLatestFailure && (
          <div>
            <div className="font-medium text-slate-700 dark:text-slate-200">{labels.latestFailure} · {failureLabel(recovery.last_failure.kind)}</div>
            <div className="mt-1 break-words text-slate-600 dark:text-slate-400">{recovery.last_failure.message}</div>
            <div className="mt-1 text-[11px] text-slate-500 dark:text-slate-500">{failureMetadata(recovery.last_failure)}</div>
          </div>
        )}
      </div>
      {isTerminal && (
        <Button
          onClick={onCancel}
          disabled={lifecycleBusy}
          variant="subtle"
          size="sm"
          className="mt-3"
          icon={<Square className="h-3.5 w-3.5" />}
        >
          {labels.cancelRecovery}
        </Button>
      )}
    </div>
  )
}

type InstanceRuntimePolicyControlsProps = {
  instance: Instance
  disableMoveUp: boolean
  disableMoveDown: boolean
  onToggleAutoStart: () => void
  onToggleRestartPolicy: () => void
  onMoveUp: () => void
  onMoveDown: () => void
}

export const InstanceRuntimePolicyControls = ({
  instance,
  disableMoveUp,
  disableMoveDown,
  onToggleAutoStart,
  onToggleRestartPolicy,
  onMoveUp,
  onMoveDown,
}: InstanceRuntimePolicyControlsProps) => {
  const { t } = useI18n()
  const labels = t.instanceWorkspace
  const recoveryEnabled = instance.config.restart_policy === 'on-failure'

  return (
    <div className="space-y-2 border-t border-slate-200 pt-4 dark:border-slate-800">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="text-xs font-semibold text-slate-500 dark:text-slate-400">{labels.autoStart}</div>
          <div className="mt-1 text-[11px] text-slate-500 dark:text-slate-500">{labels.autoStartHint}</div>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={!!instance.config.auto_start}
          onClick={onToggleAutoStart}
          className={`relative inline-flex h-6 w-11 shrink-0 rounded-full border-2 border-transparent transition-colors duration-200 ${instance.config.auto_start ? 'bg-blue-600' : 'bg-slate-300 dark:bg-slate-700'}`}
          title={labels.autoStart}
        >
          <span className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow transition duration-200 ${instance.config.auto_start ? 'translate-x-5' : 'translate-x-0'}`} />
        </button>
      </div>
      <div className="flex items-center justify-between gap-3 border-t border-slate-200 pt-3 dark:border-slate-800">
        <div>
          <div className="flex items-center gap-1.5 text-xs font-semibold text-slate-500 dark:text-slate-400"><ShieldCheck className="h-3.5 w-3.5" />{labels.selfHealing}</div>
          <div className="mt-1 max-w-[245px] text-[11px] text-slate-500 dark:text-slate-500">{labels.selfHealingHint}</div>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={recoveryEnabled}
          onClick={onToggleRestartPolicy}
          className={`relative inline-flex h-6 w-11 shrink-0 rounded-full border-2 border-transparent transition-colors duration-200 ${recoveryEnabled ? 'bg-blue-600' : 'bg-slate-300 dark:bg-slate-700'}`}
          title={labels.selfHealing}
        >
          <span className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow transition duration-200 ${recoveryEnabled ? 'translate-x-5' : 'translate-x-0'}`} />
        </button>
      </div>
      <div className="grid grid-cols-2 gap-2 pt-2">
        <Button onClick={onMoveUp} disabled={disableMoveUp} variant="subtle" icon={<ArrowUp className="h-4 w-4" />}>{labels.moveUp}</Button>
        <Button onClick={onMoveDown} disabled={disableMoveDown} variant="subtle" icon={<ArrowDown className="h-4 w-4" />}>{labels.moveDown}</Button>
      </div>
    </div>
  )
}
