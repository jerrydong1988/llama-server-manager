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

const matchingWorkerIsOnline = (instance: Instance, workers: WorkerInfo[]) => {
  if (!instance.config.rpc_servers) return true
  const configuredServers = instance.config.rpc_servers.split(/[, ]+/).filter(Boolean)
  return workers.some(worker => (
    worker.status === 'Online' && configuredServers.some(server => {
      const endpoint = parseHostPort(server, 50052)
      return worker.host === endpoint.host && worker.port === endpoint.port
    })
  ))
}

const wait = (delayMs: number) => new Promise(resolve => window.setTimeout(resolve, delayMs))

export async function runAutoStartSequence({
  instanceIds,
  getInstance,
  getWorkers,
  startInstance,
  shouldCancel = () => false,
  delayMs = AUTO_START_STAGGER_MS,
  onMissingWorker,
}: AutoStartSequenceOptions) {
  const workers = await getWorkers()
  let attemptedStart = false

  for (const instanceId of instanceIds) {
    if (shouldCancel()) return
    let instance = getInstance(instanceId)
    if (!instance || !instance.config.auto_start || instance.status === 'running') continue
    if (!matchingWorkerIsOnline(instance, workers)) {
      onMissingWorker?.(instance)
      continue
    }

    if (attemptedStart) {
      await wait(delayMs)
      if (shouldCancel()) return
      instance = getInstance(instanceId)
      if (!instance || !instance.config.auto_start || instance.status === 'running') continue
      if (!matchingWorkerIsOnline(instance, workers)) {
        onMissingWorker?.(instance)
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
}
