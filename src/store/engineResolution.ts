import type { EngineInfo, InstanceConfig } from './types'

export function resolveEffectiveEngine(
  config: Pick<InstanceConfig, 'engine_id'>,
  engines: EngineInfo[],
  defaultEngineId: string | null | undefined,
): EngineInfo | null {
  const configuredId = config.engine_id.trim()
  if (configuredId) return engines.find(engine => engine.id === configuredId) ?? null
  if (defaultEngineId) {
    const defaultEngine = engines.find(engine => engine.id === defaultEngineId)
    if (defaultEngine) return defaultEngine
  }
  return engines[0] ?? null
}
