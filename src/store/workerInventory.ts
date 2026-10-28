import { invokeApp as invoke } from '../lib/ipc'
import type { WorkerInfo } from './types'

let workerInventoryGeneration = 0

// Page reads, discovery scans, management events and local removals share one
// ordering boundary because all of them publish to the same worker inventory.
export async function refreshWorkerInventory(
  apply: (workers: WorkerInfo[]) => void,
  command: 'get_workers' | 'scan_workers_tcp' = 'get_workers',
  isActive: () => boolean = () => true,
): Promise<WorkerInfo[]> {
  const generation = ++workerInventoryGeneration
  const current = () => isActive() && generation === workerInventoryGeneration
  try {
    const workers = await invoke<WorkerInfo[]>(command)
    if (!current()) return []
    apply(workers)
    return workers
  } catch (error) {
    if (current()) throw error
    return []
  }
}
