import { defaultInstanceConfig } from './store/defaults'
import type { InstanceConfig } from './store/types'

const INTENT_METADATA_FIELDS = new Set<keyof InstanceConfig>([
  'launch_mode',
  'manual_command',
  'explicit_overrides',
  'id',
  'name',
  'engine_id',
  'model_path',
  'host',
  'port',
  'auto_start',
])

const sameValue = (left: unknown, right: unknown) => {
  if (Array.isArray(left) && Array.isArray(right)) {
    return left.length === right.length && left.every((value, index) => Object.is(value, right[index]))
  }
  return Object.is(left, right)
}

const isInstanceConfigKey = (value: string): value is keyof InstanceConfig => (
  Object.prototype.hasOwnProperty.call(defaultInstanceConfig(), value)
)

export function sanitizeExplicitOverrides(values: unknown): string[] {
  if (!Array.isArray(values)) return []
  return Array.from(new Set(values.filter((value): value is string => (
    typeof value === 'string' && isInstanceConfigKey(value) && !INTENT_METADATA_FIELDS.has(value)
  ))))
}

/**
 * Legacy configs did not record user intent. Their non-default values are the
 * only reliable evidence of an override, so migrate those once and thereafter
 * keep an explicit empty list distinct from legacy/unknown state.
 */
export function migrateParameterIntent(config: InstanceConfig): InstanceConfig {
  const defaults = defaultInstanceConfig()
  const merged = { ...defaults, ...config }
  if (Array.isArray(config.explicit_overrides)) {
    return { ...merged, explicit_overrides: sanitizeExplicitOverrides(config.explicit_overrides) }
  }

  const explicit_overrides = (Object.keys(defaults) as Array<keyof InstanceConfig>)
    .filter(key => !INTENT_METADATA_FIELDS.has(key) && !sameValue(merged[key], defaults[key]))
  return { ...merged, explicit_overrides }
}

export function markExplicitOverride(
  config: InstanceConfig,
  key: keyof InstanceConfig,
  value: InstanceConfig[keyof InstanceConfig],
): InstanceConfig {
  if (INTENT_METADATA_FIELDS.has(key)) return { ...config, [key]: value }
  const overrides = new Set(sanitizeExplicitOverrides(config.explicit_overrides))
  const defaultValue = defaultInstanceConfig()[key]
  if (sameValue(value, defaultValue) && (value === '' || (Array.isArray(value) && value.length === 0))) {
    overrides.delete(key)
  } else {
    overrides.add(key)
  }
  return { ...config, [key]: value, explicit_overrides: [...overrides] }
}

export function inheritParameters(
  config: InstanceConfig,
  keys: Array<keyof InstanceConfig>,
): InstanceConfig {
  const defaults = defaultInstanceConfig()
  const overrides = new Set(sanitizeExplicitOverrides(config.explicit_overrides))
  const next = { ...config }
  for (const key of keys) {
    if (INTENT_METADATA_FIELDS.has(key)) continue
    next[key] = defaults[key] as never
    overrides.delete(key)
  }
  next.explicit_overrides = [...overrides]
  return next
}

export function applyExplicitOverrides(
  config: InstanceConfig,
  changes: Partial<InstanceConfig>,
): InstanceConfig {
  const overrides = new Set(sanitizeExplicitOverrides(config.explicit_overrides))
  for (const key of Object.keys(changes) as Array<keyof InstanceConfig>) {
    if (!INTENT_METADATA_FIELDS.has(key)) overrides.add(key)
  }
  return { ...config, ...changes, explicit_overrides: [...overrides] }
}

export function hasExplicitOverride(config: InstanceConfig, key: keyof InstanceConfig): boolean {
  return sanitizeExplicitOverrides(config.explicit_overrides).includes(key)
}

export function removeExplicitOverrides(
  config: InstanceConfig,
  keys: Iterable<keyof InstanceConfig>,
): InstanceConfig {
  const overrides = new Set(sanitizeExplicitOverrides(config.explicit_overrides))
  for (const key of keys) overrides.delete(key)
  return { ...config, explicit_overrides: [...overrides] }
}

export function explicitOverrideKeys(config: InstanceConfig): Array<keyof InstanceConfig> {
  return sanitizeExplicitOverrides(config.explicit_overrides) as Array<keyof InstanceConfig>
}
