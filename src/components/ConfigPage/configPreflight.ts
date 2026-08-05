import type { InstanceConfig, ModelInfo } from '../../store'
import { normalizeConfigForSelectedModel, normalizeInstanceConfig } from '../../modelPolicy'
import { normalizeModelPath } from '../../store/bootstrap'

export function createConfigPreflightKey(config: InstanceConfig, engineExe: string): string {
  return JSON.stringify([engineExe, config])
}

export function canReuseConfigPreflight(
  cachedKey: string | null,
  expectedKey: string,
): boolean {
  return cachedKey === expectedKey
}

export function configForPreflight(
  config: InstanceConfig,
  model: ModelInfo | null,
  committedModelPath: string,
): InstanceConfig {
  if (config.launch_mode === 'manual') return config
  return normalizeModelPath(config.model_path) !== committedModelPath
    ? normalizeConfigForSelectedModel(config, model).config
    : normalizeInstanceConfig(config, model).config
}
