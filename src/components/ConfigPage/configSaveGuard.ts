export type RevisionGuardedResult<T> =
  | { stale: false; value: T }
  | { stale: true }

export const runRevisionGuarded = async <T>(
  saveRevision: number,
  getCurrentRevision: () => number,
  operation: () => Promise<T>,
): Promise<RevisionGuardedResult<T>> => {
  const value = await operation()
  if (getCurrentRevision() !== saveRevision) {
    return { stale: true }
  }
  return { stale: false, value }
}
