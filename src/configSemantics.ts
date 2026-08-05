import type { InstanceConfig } from './store/types'

export function effectiveMmprojMode(config: InstanceConfig): '' | 'on' | 'off' {
  const mode = config.mmproj_mode || (config.no_mmproj ? 'off' : config.mmproj_auto ? 'on' : '')
  return mode === 'on' || mode === 'off' ? mode : ''
}

export function projectorEnabled(config: InstanceConfig, isEmbedding = config.embedding): boolean {
  if (isEmbedding || effectiveMmprojMode(config) === 'off') return false
  return effectiveMmprojMode(config) === 'on'
    || Boolean(config.mmproj_path.trim())
    || Boolean(config.mmproj_url.trim())
}

export function speculativeType(config: InstanceConfig): string {
  const type = config.spec_type.trim()
  return type === 'none' ? '' : type
}

export function speculativeEnabled(config: InstanceConfig, isEmbedding = config.embedding): boolean {
  return !isEmbedding && speculativeType(config) !== ''
}

export function ngramCacheEnabled(config: InstanceConfig, isEmbedding = config.embedding): boolean {
  return !isEmbedding && speculativeType(config) === 'ngram-cache'
}

export function reasoningBudgetEnabled(config: InstanceConfig): boolean {
  const raw = config.reasoning_budget.trim()
  if (!raw) return false
  const value = Number(raw)
  return Number.isFinite(value) && value >= 0
}
