import { useEffect, useRef, useState } from 'react'
import { LoaderCircle, Trash2 } from 'lucide-react'
import { confirm } from '@tauri-apps/plugin-dialog'
import { useI18n } from '../../i18n'
import { useAppStore } from '../../store'
import type { Instance } from '../../store/types'
import {
  canClearCheckpoint,
  checkpointOperationLabel,
  checkpointOutcomeLabel,
  checkpointPhaseLabel,
  checkpointReasonLabel,
  formatCheckpointBytes,
} from '../../checkpointView'
import { Badge, Button } from '../ui'

export function CheckpointStatusCard({ instance }: { instance: Instance }) {
  const { t } = useI18n()
  const status = useAppStore(state => state.checkpointStatuses[instance.id])
  const lifecycle = useAppStore(state => state.instanceLifecycle[instance.id])
  const clearCheckpoint = useAppStore(state => state.clearCheckpoint)
  const [clearing, setClearing] = useState(false)
  const mountedRef = useRef(true)
  const canClear = canClearCheckpoint(instance.status, lifecycle, status)

  useEffect(() => {
    mountedRef.current = true
    return () => { mountedRef.current = false }
  }, [])

  const handleClear = async () => {
    if (!await confirm(t.checkpoint.clearConfirm, { title: t.checkpoint.clear, kind: 'warning' })) return
    setClearing(true)
    try {
      await clearCheckpoint(instance.id)
    } catch (error) {
      useAppStore.getState().addRuntimeWarning(`${t.checkpoint.clear}: ${String(error)}`)
    } finally {
      if (mountedRef.current) setClearing(false)
    }
  }

  return (
    <div className="space-y-3 rounded-lg border border-violet-200 bg-violet-50/70 p-3 text-sm dark:border-violet-500/20 dark:bg-violet-500/5">
      <div className="flex items-center justify-between gap-3">
        <div className="text-xs font-semibold text-violet-700 dark:text-violet-300">{t.checkpoint.statusTitle}</div>
        {status && (
          <Badge tone={status.last_outcome === 'failed' ? 'red' : status.phase === 'ready' ? 'emerald' : status.phase === 'ready_cold' ? 'amber' : 'violet'}>
            {checkpointPhaseLabel(status.phase, t.checkpoint)}
          </Badge>
        )}
      </div>
      {status ? (
        <div className="grid grid-cols-2 gap-x-4 gap-y-2 text-xs">
          <div>
            <div className="text-slate-500">{t.checkpoint.routable}</div>
            <div className="mt-0.5 text-slate-800 dark:text-slate-200">{status.routable ? t.checkpoint.yes : t.checkpoint.no}</div>
          </div>
          <div>
            <div className="text-slate-500">{t.checkpoint.lastOperation}</div>
            <div className="mt-0.5 text-slate-800 dark:text-slate-200">
              {checkpointOperationLabel(status.last_operation, t.checkpoint)} · {checkpointOutcomeLabel(status.last_outcome, t.checkpoint)}
            </div>
          </div>
          <div>
            <div className="text-slate-500">{t.checkpoint.promptTokens}</div>
            <div className="mt-0.5 text-slate-800 dark:text-slate-200">{status.prompt_tokens?.toLocaleString() ?? '--'}</div>
          </div>
          <div>
            <div className="text-slate-500">{t.checkpoint.bytes}</div>
            <div className="mt-0.5 text-slate-800 dark:text-slate-200">{formatCheckpointBytes(status.bytes)}</div>
          </div>
          <div>
            <div className="text-slate-500">{t.checkpoint.duration}</div>
            <div className="mt-0.5 text-slate-800 dark:text-slate-200">{status.duration_ms === undefined ? '--' : `${status.duration_ms.toLocaleString()} ms`}</div>
          </div>
          <div>
            <div className="text-slate-500">{t.checkpoint.updatedAt}</div>
            <div className="mt-0.5 text-slate-800 dark:text-slate-200">{status.updated_at > 0 ? new Date(status.updated_at).toLocaleString() : '--'}</div>
          </div>
          <div className="col-span-2">
            <div className="text-slate-500">{t.checkpoint.reason}</div>
            <div className="mt-0.5 leading-5 text-slate-800 dark:text-slate-200">{checkpointReasonLabel(status, t.checkpoint)}</div>
          </div>
        </div>
      ) : (
        <p className="text-xs leading-5 text-slate-500 dark:text-slate-400">{t.checkpoint.noData}</p>
      )}
      <Button
        onClick={() => void handleClear()}
        disabled={!canClear || clearing}
        variant="danger"
        size="sm"
        className="w-full"
        icon={clearing ? <LoaderCircle className="h-3.5 w-3.5 animate-spin" /> : <Trash2 className="h-3.5 w-3.5" />}
      >
        {t.checkpoint.clear}
      </Button>
    </div>
  )
}
