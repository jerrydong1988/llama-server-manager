import type {
  CheckpointOperation,
  CheckpointOutcome,
  CheckpointPhase,
  CheckpointStatus,
  InstanceLifecyclePhase,
} from './store/types'

export const CHECKPOINT_PHASES: readonly CheckpointPhase[] = [
  'disabled',
  'ineligible',
  'starting',
  'engine_healthy',
  'restoring',
  'ready',
  'ready_cold',
  'draining',
  'saving',
  'stopping',
  'stopped',
]

export type CheckpointViewLabels = {
  phases: Record<CheckpointPhase, string>
  operations: Record<CheckpointOperation, string>
  outcomes: Record<CheckpointOutcome, string>
  reasons: Record<string, string>
  noData: string
}

export const checkpointPhaseLabel = (
  phase: CheckpointPhase,
  labels: CheckpointViewLabels,
) => labels.phases[phase]

export const checkpointOperationLabel = (
  operation: CheckpointOperation,
  labels: CheckpointViewLabels,
) => labels.operations[operation]

export const checkpointOutcomeLabel = (
  outcome: CheckpointOutcome,
  labels: CheckpointViewLabels,
) => labels.outcomes[outcome]

export const checkpointReasonLabel = (
  status: CheckpointStatus | undefined,
  labels: CheckpointViewLabels,
) => {
  if (!status) return labels.noData
  return labels.reasons[status.reason_code] || status.message || status.reason_code || labels.noData
}

export const formatCheckpointBytes = (bytes: number | undefined) => {
  if (bytes === undefined) return '--'
  if (bytes < 1024) return `${bytes} B`
  const units = ['KiB', 'MiB', 'GiB', 'TiB']
  let value = bytes / 1024
  let unit = units[0]
  for (let index = 1; index < units.length && value >= 1024; index += 1) {
    value /= 1024
    unit = units[index]
  }
  return `${value >= 10 ? value.toFixed(1) : value.toFixed(2)} ${unit}`
}

export const canClearCheckpoint = (
  instanceStatus: 'running' | 'stopped' | 'error',
  lifecycle: InstanceLifecyclePhase | undefined,
  checkpoint: CheckpointStatus | undefined,
) => instanceStatus !== 'running'
  && lifecycle === undefined
  && checkpoint?.phase !== 'saving'
  && checkpoint?.phase !== 'restoring'
