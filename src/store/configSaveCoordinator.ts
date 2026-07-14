type SaveWaiter<R> = {
  resolve: (result: R) => void
  reject: (error: unknown) => void
}

type IdleWaiter = {
  resolve: () => void
  reject: (error: unknown) => void
}

type PendingSave<T, R> = {
  snapshot: T
  waiters: SaveWaiter<R>[]
}

export type LatestSaveCoordinator<T, R> = {
  save: (snapshot: T) => Promise<R>
  waitForIdle: () => Promise<void>
}

export function createLatestSaveCoordinator<T, R>(
  persist: (snapshot: T) => Promise<R>,
): LatestSaveCoordinator<T, R> {
  let active = false
  let drainScheduled = false
  let pending: PendingSave<T, R> | null = null
  let lastError: unknown = null
  let idleWaiters: IdleWaiter[] = []

  const settleIdle = () => {
    const waiters = idleWaiters
    const error = lastError
    idleWaiters = []
    lastError = null
    for (const waiter of waiters) {
      if (error === null) waiter.resolve()
      else waiter.reject(error)
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
        const result = await persist(current.snapshot)
        lastError = null
        current.waiters.forEach(waiter => waiter.resolve(result))
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
    save: (snapshot) => new Promise<R>((resolve, reject) => {
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
        return Promise.resolve()
      }
      return new Promise<void>((resolve, reject) => {
        idleWaiters.push({ resolve, reject })
      })
    },
  }
}
