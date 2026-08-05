type LifecycleOperation = 'start' | 'stop'

type ActiveLifecycleOperation = {
  kind: LifecycleOperation
  promise: Promise<void>
}

const activeOperations = new Map<string, ActiveLifecycleOperation>()

function runInstanceLifecycleOperation(
  instanceId: string,
  kind: LifecycleOperation,
  operation: () => Promise<void>,
): Promise<void> {
  const active = activeOperations.get(instanceId)
  if (active?.kind === kind) return active.promise

  let current: Promise<void>
  current = (async () => {
    if (active) {
      try {
        await active.promise
      } catch {
        // An opposite lifecycle operation still gets its turn after a failure.
      }
    }
    await operation()
  })().finally(() => {
    if (activeOperations.get(instanceId)?.promise === current) {
      activeOperations.delete(instanceId)
    }
  })

  activeOperations.set(instanceId, { kind, promise: current })
  return current
}

export function runInstanceStart(
  instanceId: string,
  operation: () => Promise<void>,
): Promise<void> {
  return runInstanceLifecycleOperation(instanceId, 'start', operation)
}

export function runInstanceStop(
  instanceId: string,
  operation: () => Promise<void>,
): Promise<void> {
  return runInstanceLifecycleOperation(instanceId, 'stop', operation)
}
