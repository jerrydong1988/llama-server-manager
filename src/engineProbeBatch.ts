import type { EngineInfo } from './store'
import { forEachConcurrent } from './utils/async'

export const ENGINE_PROBE_BATCH_CONCURRENCY = 2

export type EngineProbeTarget = Pick<EngineInfo, 'id' | 'name'>

export interface EngineProbeBatchProgress {
  completed: number
  total: number
}

export interface EngineProbeFailure {
  id: string
  name: string
  error: string
}

export interface EngineProbeBatchResult {
  total: number
  succeeded: number
  failed: number
  failures: EngineProbeFailure[]
}

interface EngineProbeBatchOptions {
  concurrency?: number
  onProgress?: (progress: EngineProbeBatchProgress) => void
  onActiveChange?: (id: string, active: boolean) => void
}

const describeProbeError = (error: unknown) => (
  error instanceof Error ? error.message : String(error)
)

export async function probeEngineBatch(
  targets: readonly EngineProbeTarget[],
  probe: (id: string) => Promise<unknown>,
  options: EngineProbeBatchOptions = {},
): Promise<EngineProbeBatchResult> {
  const total = targets.length
  let completed = 0
  let succeeded = 0
  const failures: EngineProbeFailure[] = []

  options.onProgress?.({ completed, total })
  await forEachConcurrent(
    targets,
    options.concurrency ?? ENGINE_PROBE_BATCH_CONCURRENCY,
    async target => {
      options.onActiveChange?.(target.id, true)
      try {
        await probe(target.id)
        succeeded += 1
      } catch (error) {
        failures.push({
          id: target.id,
          name: target.name,
          error: describeProbeError(error),
        })
      } finally {
        options.onActiveChange?.(target.id, false)
        completed += 1
        options.onProgress?.({ completed, total })
      }
    },
  )

  return {
    total,
    succeeded,
    failed: failures.length,
    failures,
  }
}
