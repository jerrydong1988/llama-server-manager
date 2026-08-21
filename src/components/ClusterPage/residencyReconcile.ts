import type { ResidencyIntent, ResidencyOperation, ResidencyPolicy } from '../../store/types'

export const RESIDENCY_GIB = 1024 ** 3

export function residencyBytesFromGiB(gib: number) {
  return Math.max(0, Math.round(gib * RESIDENCY_GIB))
}

export function buildResidencyPolicy(input: {
  enabled: boolean
  ramGiB: number
  vramGiB: number
  drainTimeoutSeconds: number
  instanceIds: string[]
  intents: Record<string, ResidencyIntent>
}): ResidencyPolicy {
  return {
    enabled: input.enabled,
    ramBudgetBytes: residencyBytesFromGiB(input.ramGiB),
    vramBudgetBytes: residencyBytesFromGiB(input.vramGiB),
    drainTimeoutSeconds: Math.max(5, Math.min(3600, Math.round(input.drainTimeoutSeconds))),
    intents: input.instanceIds
      .map((instanceId, index) => input.intents[instanceId] || {
        instanceId,
        priority: (index + 1) * 10,
        enabled: false,
      })
      .sort((left, right) => left.priority - right.priority || left.instanceId.localeCompare(right.instanceId)),
  }
}

const operationRank: Record<ResidencyOperation['kind'], number> = {
  drain: 0,
  evict: 1,
  warm: 2,
}

export function orderedResidencyOperations(operations: ResidencyOperation[]) {
  return [...operations].sort((left, right) => (
    operationRank[left.kind] - operationRank[right.kind]
    || left.sequence - right.sequence
    || left.instanceId.localeCompare(right.instanceId)
  ))
}
