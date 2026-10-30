import { useEffect, useRef, useState } from 'react'
import type { Translations } from '../../i18n'
import type { CheckpointEligibility, InstanceConfig } from '../../store'
import { invokeApp } from '../../lib/ipc'
import { Button } from '../ui'
import { Toggle } from './shared'

export function CheckpointValidation({ config, engineExe, eligibility, labels, onChange, onValidated }: {
  config: InstanceConfig
  engineExe: string
  eligibility: CheckpointEligibility | null
  labels: Translations['checkpoint']
  onChange: (enabled: boolean) => void
  onValidated: () => void
}) {
  const [running, setRunning] = useState(false)
  const [message, setMessage] = useState('')
  const mounted = useRef(true)
  const validationId = useRef<string | null>(null)
  useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])
  useEffect(() => { setMessage('') }, [config, engineExe])

  const validate = async () => {
    validationId.current = crypto.randomUUID()
    setRunning(true)
    setMessage(labels.validationRunning)
    try {
      await invokeApp('validate_checkpoint_locally', { config, engineExe, validationId: validationId.current })
      if (mounted.current) { setMessage(labels.validationPassed) }
    } catch (error) {
      if (mounted.current) setMessage(`${labels.validationFailed}: ${error instanceof Error ? error.message : String(error)}`)
    } finally {
      validationId.current = null
      if (mounted.current) { setRunning(false); onValidated() }
    }
  }
  const cancel = async () => {
    const id = validationId.current
    setMessage(labels.validationCancelling)
    try {
      await invokeApp('cancel_checkpoint_validation', { validationId: id })
    } catch (error) {
      if (mounted.current && validationId.current === id) setMessage(`${labels.validationFailed}: ${error instanceof Error ? error.message : String(error)}`)
    }
  }
  const qualification = eligibility?.qualification
  return (
    <div className="mt-5 space-y-3 border-t border-slate-200 pt-4 dark:border-slate-700">
      <Toggle label={labels.allowLocalValidation} value={config.kv_checkpoint.allow_local_validation ?? false}
        onChange={onChange} title={labels.localValidationTip} disabled={!config.kv_checkpoint.enabled || running} />
      <p className="max-w-3xl text-xs leading-5 text-slate-600 dark:text-slate-300">{labels.localValidationTip}</p>
      {qualification && <p className="text-sm font-medium text-slate-700 dark:text-slate-200">{labels.qualification[qualification]}</p>}
      {config.kv_checkpoint.allow_local_validation && (
        <div className="flex flex-wrap items-center gap-3">
          <Button type="button" variant="secondary" size="sm" disabled={running || !eligibility?.can_validate || !engineExe}
            onClick={() => void validate()}>{running ? labels.validationRunning : labels.validateLocally}</Button>
          {running && <Button type="button" variant="secondary" size="sm" onClick={() => void cancel()}>{labels.cancelValidation}</Button>}
        </div>
      )}
      {message && <p role="status" aria-live="polite" className="max-w-3xl break-words text-xs leading-5 text-slate-600 dark:text-slate-300">{message}</p>}
    </div>
  )
}
