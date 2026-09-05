import type { Instance, WorkerInfo } from './store'
import { parseHostPort } from './utils/network'

export const AUTO_START_STAGGER_MS = 3_000

type AutoStartSequenceOptions = {
  instanceIds: string[]
  getInstance: (id: string) => Instance | undefined
  getWorkers: () => Promise<WorkerInfo[]>
  startInstance: (id: string) => Promise<void>
  shouldCancel?: () => boolean
  delayMs?: number
  onMissingWorker?: (instance: Instance) => void
}

export type AutoStartSequenceResult = {
  missingWorkerIds: string[]
}

const matchingWorkerIsOnline = (instance: Instance, workers: WorkerInfo[]) => {
  if (!instance.config.rpc_servers) return true
  const configuredServers = instance.config.rpc_servers.split(/[, ]+/).filter(Boolean)
  return configuredServers.every(server => {
    const endpoint = parseHostPort(server, 50052)
    return workers.some(worker => (
      worker.status === 'Online' && worker.host === endpoint.host && worker.port === endpoint.port
    ))
  })
}

const wait = (delayMs: number) => new Promise(resolve => globalThis.setTimeout(resolve, delayMs))

export async function runAutoStartSequence({
  instanceIds,
  getInstance,
  getWorkers,
  startInstance,
  shouldCancel = () => false,
  delayMs = AUTO_START_STAGGER_MS,
  onMissingWorker,
}: AutoStartSequenceOptions): Promise<AutoStartSequenceResult> {
  let attemptedStart = false
  const missingWorkerIds: string[] = []

  for (const instanceId of instanceIds) {
    if (shouldCancel()) return { missingWorkerIds }
    let instance = getInstance(instanceId)
    if (!instance || !instance.config.auto_start || instance.status === 'running') continue
    let workers = await getWorkers()
    if (!matchingWorkerIsOnline(instance, workers)) {
      onMissingWorker?.(instance)
      missingWorkerIds.push(instanceId)
      continue
    }

    if (attemptedStart) {
      await wait(delayMs)
      if (shouldCancel()) return { missingWorkerIds }
      instance = getInstance(instanceId)
      if (!instance || !instance.config.auto_start || instance.status === 'running') continue
      workers = await getWorkers()
      if (!matchingWorkerIsOnline(instance, workers)) {
        onMissingWorker?.(instance)
        missingWorkerIds.push(instanceId)
        continue
      }
    }

    attemptedStart = true
    try {
      await startInstance(instanceId)
    } catch {
      // A failed automatic start must not prevent later configured instances from starting.
    }
  }
  return { missingWorkerIds }
}
