type SaveWaiter = {
  resolve: () => void
  reject: (error: unknown) => void
}

type PendingSave<T> = {
  snapshot: T
  waiters: SaveWaiter[]
}

export type LatestSaveCoordinator<T> = {
  save: (snapshot: T) => Promise<void>
  waitForIdle: () => Promise<void>
}

export function createLatestSaveCoordinator<T>(
  persist: (snapshot: T) => Promise<void>,
): LatestSaveCoordinator<T> {
  let active = false
  let drainScheduled = false
  let pending: PendingSave<T> | null = null
  let lastError: unknown = null
  let idleWaiters: SaveWaiter[] = []

  const settleIdle = () => {
    const waiters = idleWaiters
    idleWaiters = []
    for (const waiter of waiters) {
      if (lastError === null) waiter.resolve()
      else waiter.reject(lastError)
    }
  }

  const drain = async () => {
    drainScheduled = false
    if (active) return
    active = true

    while (pending) {
      const current = pending
      pending = null
      try {
        await persist(current.snapshot)
        lastError = null
        current.waiters.forEach(waiter => waiter.resolve())
      } catch (error) {
        lastError = error
        current.waiters.forEach(waiter => waiter.reject(error))
      }
    }

    active = false
    settleIdle()
  }

  const scheduleDrain = () => {
    if (active || drainScheduled) return
    drainScheduled = true
    queueMicrotask(() => void drain())
  }

  return {
    save: (snapshot) => new Promise<void>((resolve, reject) => {
      const waiter = { resolve, reject }
      if (pending) {
        pending.snapshot = snapshot
        pending.waiters.push(waiter)
      } else {
        pending = { snapshot, waiters: [waiter] }
      }
      scheduleDrain()
    }),
    waitForIdle: () => {
      if (!active && !pending && !drainScheduled) {
        return lastError === null ? Promise.resolve() : Promise.reject(lastError)
      }
      return new Promise<void>((resolve, reject) => {
        idleWaiters.push({ resolve, reject })
      })
    },
  }
}
