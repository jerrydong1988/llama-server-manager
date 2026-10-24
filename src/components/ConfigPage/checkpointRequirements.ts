import type { InstanceConfig } from '../../store'

export function applyCheckpointRequirements(
  config: InstanceConfig,
  reasons: string[],
  set: <K extends keyof InstanceConfig>(key: K, value: InstanceConfig[K]) => void,
) {
  const singleSlot = reasons.includes('parallel_must_be_one')
  if (singleSlot) set('parallel', 1)
  if (reasons.includes('prompt_cache_required')) set('cache_prompt', true)
  if (singleSlot || reasons.includes('prompt_cache_retention_required')) {
    set('cache_idle_slots', true)
    if (config.cache_ram !== -1 && config.cache_ram <= 0) set('cache_ram', 8192)
  } else if (reasons.includes('multi_slot_idle_cache_unsupported')) {
    set('cache_idle_slots', false)
  }
  if (reasons.includes('slots_required')) set('slots_enabled', true)
  if (reasons.includes('sliding_window_requires_full_cache')) set('swa_full', true)
  if (reasons.includes('context_checkpoints_required')) set('ctx_checkpoints', 32)
}
