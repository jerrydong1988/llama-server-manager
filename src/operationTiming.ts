export type OperationOutcome = 'success' | 'failure' | 'cancelled'

type OperationStage = {
  name: string
  elapsedMs: number
}

const SLOW_OPERATION_MS = 250

function now(): number {
  return typeof performance !== 'undefined' ? performance.now() : Date.now()
}

function tracingEnabled(): boolean {
  try {
    return localStorage.getItem('lsm:trace-operations') === '1'
  } catch {
    return false
  }
}

export function beginOperationTiming(name: string) {
  const startedAt = now()
  let previousAt = startedAt
  const stages: OperationStage[] = []

  return {
    mark(stage: string) {
      const markedAt = now()
      stages.push({ name: stage, elapsedMs: markedAt - previousAt })
      previousAt = markedAt
    },
    finish(outcome: OperationOutcome) {
      const finishedAt = now()
      if (finishedAt > previousAt) {
        stages.push({ name: 'complete', elapsedMs: finishedAt - previousAt })
      }
      const totalMs = finishedAt - startedAt
      if (totalMs < SLOW_OPERATION_MS && !tracingEnabled()) return

      const breakdown = stages
        .map(stage => `${stage.name}=${stage.elapsedMs.toFixed(1)}ms`)
        .join(' ')
      console.info(`[operation-timing] ${name} outcome=${outcome} total=${totalMs.toFixed(1)}ms ${breakdown}`)
    },
  }
}
