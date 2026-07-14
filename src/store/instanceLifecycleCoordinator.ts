const activeStarts = new Map<string, Promise<void>>()

export function runInstanceStart(
  instanceId: string,
  operation: () => Promise<void>,
): Promise<void> {
  const active = activeStarts.get(instanceId)
  if (active) return active

  const current = (async () => {
    try {
      await operation()
    } finally {
      activeStarts.delete(instanceId)
    }
  })()
  activeStarts.set(instanceId, current)
  return current
}
