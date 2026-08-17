import type { Instance, InstanceRecoveryStatus } from './types'

export function instanceStatusFromRecovery(
  recovery: InstanceRecoveryStatus,
): Instance['status'] {
  if (recovery.phase === 'crash_loop') return 'crash_loop'
  if (recovery.phase === 'failed') return 'error'
  return 'recovering'
}

export function isAutoStartEligible(instance: Instance): boolean {
  return Boolean(instance.config.auto_start) && instance.status === 'stopped'
}
