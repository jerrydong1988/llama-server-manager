import type { EngineInfo, InstanceConfig } from './types'
import { pathsEqual } from '../utils/path'

export function isConfiguredEngineMissing(
  config: Pick<InstanceConfig, 'engine_id'> & Partial<Pick<InstanceConfig, 'engine_kind'>>,
  engines: EngineInfo[],
) {
  const configuredId = config.engine_id.trim()
  return Boolean(configuredId && !engines.some(engine => pathsEqual(engine.id, configuredId)
    && (engine.kind ?? 'llama_cpp') === (config.engine_kind ?? 'llama_cpp')))
}

export function resolveEffectiveEngine(
  config: Pick<InstanceConfig, 'engine_id'> & Partial<Pick<InstanceConfig, 'engine_kind'>>,
  engines: EngineInfo[],
  defaultEngineId: string | null | undefined,
): EngineInfo | null {
  engines = engines.filter(engine => (engine.kind ?? 'llama_cpp') === (config.engine_kind ?? 'llama_cpp'))
  const configuredId = config.engine_id.trim()
  if (configuredId) return engines.find(engine => pathsEqual(engine.id, configuredId)) ?? null
  if (defaultEngineId) {
    const defaultEngine = engines.find(engine => pathsEqual(engine.id, defaultEngineId))
    if (defaultEngine) return defaultEngine
  }
  return engines[0] ?? null
}
