import type { AppStoreSet } from './helpers'
import type { AppState } from './types'

export function createClusterSlice(set: AppStoreSet): Pick<
  AppState,
  'setWorkers' | 'addWorker' | 'removeWorker' | 'updateWorker' | 'setClusterScanning'
> {
  return {
    setWorkers: (workers) => set({ workers }),
    addWorker: (worker) => set((state) => ({ workers: [...state.workers, worker] })),
    removeWorker: (id) => set((state) => ({ workers: state.workers.filter((worker) => worker.id !== id) })),
    updateWorker: (id, partial) => set((state) => ({
      workers: state.workers.map((worker) => (
        worker.id === id ? { ...worker, ...partial } : worker
      )),
    })),
    setClusterScanning: (scanning) => set({ clusterScanning: scanning }),
  }
}
