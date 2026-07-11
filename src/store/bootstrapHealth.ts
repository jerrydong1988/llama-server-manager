import type { Instance } from './types'

type HydratedInstanceHealth = Pick<Instance, 'id' | 'status' | 'healthCheck'>

export function resolveHydratedHealth(
  id: string,
  status: Instance['status'],
  existingInstances: HydratedInstanceHealth[],
): Instance['healthCheck'] {
  if (status !== 'running') return 'pending'

  const existing = existingInstances.find(instance => instance.id === id)
  if (existing?.status === 'running' && existing.healthCheck !== 'pending') {
    return existing.healthCheck
  }

  return 'pending'
}
