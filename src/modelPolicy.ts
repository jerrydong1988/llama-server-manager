import { defaultInstanceConfig } from './store/defaults'
import type { InstanceConfig, ModelInfo } from './store/types'

export type ModelWorkload = 'inference' | 'embedding' | 'reranker'

export interface VectorCleanupChange {
  key: keyof InstanceConfig
  group: 'speculative' | 'generation' | 'chat' | 'multimodal' | 'custom' | 'runtime'
  before: unknown
  after: unknown
}

export interface NormalizeInstanceConfigOptions {
  context?: 'create' | 'update'
}

type VectorCleanupGroup = VectorCleanupChange['group']

const EMBEDDING_HINTS = [
  'embed', 'embedding', 'bge', 'gte', 'e5', 'text-embedding',
  'sentence-bert', 'sentence-t5', 'instructor', 'bert', 'nomic', 'jina',
]

const RERANKER_HINTS = ['rerank', 'reranker', 'cross-encoder']

export const VECTOR_ALLOWED_FIELDS = new Set<keyof InstanceConfig>([
  'id', 'name', 'engine_id', 'model_path', 'alias', 'auto_start',
  'ctx_size', 'ctx_size_auto', 'gpu_layers_auto', 'gpu_layers', 'threads',
  'threads_batch', 'threads_http', 'batch_size', 'ubatch_size', 'parallel',
  'cont_batching', 'warmup',
  'rope_scaling', 'rope_scale', 'rope_freq_base', 'rope_freq_scale',
  'yarn_ext_factor', 'yarn_attn_factor', 'yarn_beta_slow', 'yarn_beta_fast',
  'yarn_orig_ctx', 'flash_attn', 'moe_cpu_layers', 'cpu_moe', 'mlock',
  'no_mmap', 'no_repack', 'direct_io', 'numa', 'perf', 'check_tensors',
  'fit', 'fit_target', 'fit_ctx', 'cache_type_k', 'cache_type_v',
  'kv_unified', 'cache_idle_slots', 'no_kv_offload', 'device', 'split_mode',
  'tensor_split', 'main_gpu', 'override_kv',
  'host', 'port', 'api_key', 'api_key_file', 'ssl_key_file', 'ssl_cert_file',
  'path_prefix', 'api_prefix', 'no_ui', 'offline', 'metrics', 'props',
  'slots_enabled', 'timeout', 'sleep_idle', 'verbose', 'rpc_servers',
  'sse_ping_interval', 'reuse_port',
  'embedding', 'pooling', 'embd_normalize', 'reranking',
])

export const VECTOR_INCOMPATIBLE_FIELDS = Object.keys(defaultInstanceConfig())
  .filter((key): key is keyof InstanceConfig => !VECTOR_ALLOWED_FIELDS.has(key as keyof InstanceConfig))

export const VECTOR_CLASSIFIED_FIELDS = new Set<keyof InstanceConfig>([
  ...VECTOR_ALLOWED_FIELDS,
  ...VECTOR_INCOMPATIBLE_FIELDS,
])

const MODEL_WORKLOAD_FIELDS = new Set<keyof InstanceConfig>([
  'embedding',
  'pooling',
  'reranking',
])

const SPECULATIVE_FIELDS = new Set<keyof InstanceConfig>([
  'cache_type_draft_k', 'cache_type_draft_v', 'draft_model_path',
  'draft_gpu_layers', 'draft_tokens', 'spec_draft_n_min', 'spec_type',
  'spec_draft_p_min', 'spec_draft_p_split', 'spec_draft_device',
  'lookup_cache_static', 'lookup_cache_dynamic', 'spec_default',
  'spec_draft_backend_sampling', 'spec_draft_threads', 'spec_draft_threads_batch',
])

const CHAT_FIELDS = new Set<keyof InstanceConfig>([
  'lora_path', 'lora_init_without_apply', 'lora_scaled', 'chat_template',
  'chat_template_file', 'skip_chat_parsing', 'reasoning_format',
  'reasoning_effort', 'reasoning', 'jinja', 'reasoning_budget',
  'reasoning_budget_message', 'grammar_file', 'grammar', 'prefill_assistant',
])

const MULTIMODAL_FIELDS = new Set<keyof InstanceConfig>([
  'mmproj_path', 'mmproj_url', 'mmproj_auto', 'no_mmproj',
  'no_mmproj_offload', 'image_min_tokens', 'image_max_tokens',
  'mtmd_batch_max_tokens', 'tags', 'media_path',
])

const GENERATION_FIELDS = new Set<keyof InstanceConfig>([
  'n_predict', 'ignore_eos', 'json_schema', 'json_schema_file', 'temp',
  'top_k', 'top_p', 'repeat_penalty', 'seed', 'min_p', 'presence_penalty',
  'frequency_penalty', 'repeat_last_n', 'reverse_prompt', 'special',
  'spm_infill', 'backend_sampling', 'mirostat', 'mirostat_lr', 'mirostat_ent',
  'xtc_probability', 'xtc_threshold', 'dynatemp_range', 'dynatemp_exp',
  'typical_p', 'dry_multiplier', 'dry_base', 'dry_allowed_length',
  'dry_penalty_last_n', 'dry_sequence_breaker', 'adaptive_target',
  'adaptive_decay', 'top_n_sigma', 'logit_bias', 'samplers', 'sampler_seq',
])

function hasHint(value: string, hints: string[]): boolean {
  const normalized = value.toLowerCase()
  return hints.some(hint => normalized.includes(hint))
}

function modelBasename(value: string | undefined): string {
  return value?.split(/[\\/]/).pop() ?? ''
}

function hasAuthoritativeInference(model: ModelInfo | null | undefined): boolean {
  const capabilities = model?.capabilities
  return capabilities?.metadata_complete === true &&
    capabilities.is_embedding_model === false &&
    capabilities.is_reranker_model === false
}

export function detectModelWorkload(
  model?: ModelInfo | null,
  modelPath = '',
  config?: Pick<InstanceConfig, 'embedding' | 'reranking'>,
): ModelWorkload {
  const capabilities = model?.capabilities
  if (capabilities?.is_reranker_model) return 'reranker'
  if (capabilities?.is_embedding_model) return 'embedding'
  if (hasAuthoritativeInference(model)) return 'inference'

  const fallback = [
    model?.architecture ?? '',
    modelBasename(model?.name),
    modelBasename(modelPath),
    modelBasename(model?.path),
  ].join(' ')
  if (hasHint(fallback, RERANKER_HINTS)) return 'reranker'
  if (hasHint(fallback, EMBEDDING_HINTS)) return 'embedding'

  if (config?.reranking) return 'reranker'
  if (config?.embedding) return 'embedding'
  return 'inference'
}

export function isModelWorkloadLocked(
  model?: ModelInfo | null,
  modelPath = '',
): boolean {
  return hasAuthoritativeInference(model) || detectModelWorkload(model, modelPath) !== 'inference'
}

export function getResettableFields(
  fields: Array<keyof InstanceConfig>,
  vectorMode: boolean,
  modelWorkloadLocked: boolean,
): Array<keyof InstanceConfig> {
  return fields.filter(key =>
    (!vectorMode || VECTOR_ALLOWED_FIELDS.has(key)) &&
    (!modelWorkloadLocked || !MODEL_WORKLOAD_FIELDS.has(key)),
  )
}

function cleanupGroup(key: keyof InstanceConfig): VectorCleanupGroup {
  if (SPECULATIVE_FIELDS.has(key)) return 'speculative'
  if (GENERATION_FIELDS.has(key)) return 'generation'
  if (CHAT_FIELDS.has(key)) return 'chat'
  if (MULTIMODAL_FIELDS.has(key)) return 'multimodal'
  if (key === 'custom_args') return 'custom'
  return 'runtime'
}

function sameValue(left: unknown, right: unknown): boolean {
  if (Array.isArray(left) && Array.isArray(right)) {
    return left.length === right.length && left.every((value, index) => value === right[index])
  }
  return Object.is(left, right)
}

function diffVectorCleanup(before: InstanceConfig, after: InstanceConfig): VectorCleanupChange[] {
  return (Object.keys(after) as Array<keyof InstanceConfig>)
    .filter(key => !sameValue(before[key], after[key]))
    .map(key => ({ key, group: cleanupGroup(key), before: before[key], after: after[key] }))
}

export function normalizeInstanceConfig(
  config: InstanceConfig,
  model?: ModelInfo | null,
  options: NormalizeInstanceConfigOptions = {},
) {
  const workload = detectModelWorkload(model, config.model_path, config)
  if (workload === 'inference') {
    const next: InstanceConfig = { ...config }
    if (hasAuthoritativeInference(model)) {
      const defaults = defaultInstanceConfig()
      next.embedding = defaults.embedding
      next.pooling = defaults.pooling
      next.reranking = defaults.reranking
    }
    return {
      config: next,
      workload,
      vectorMode: false,
      changes: options.context === 'create' ? [] : diffVectorCleanup(config, next),
    }
  }

  const defaults = defaultInstanceConfig()
  const next: InstanceConfig = { ...config, embedding: true }
  for (const key of VECTOR_INCOMPATIBLE_FIELDS) next[key] = defaults[key] as never

  if (workload === 'reranker') {
    next.reranking = true
    next.pooling = 'rank'
  } else {
    next.reranking = false
    if (next.pooling === 'rank') next.pooling = defaults.pooling
  }

  if (next.batch_size > next.ubatch_size) next.batch_size = next.ubatch_size

  return {
    config: next,
    workload,
    vectorMode: true,
    changes: options.context === 'create' ? [] : diffVectorCleanup(config, next),
  }
}

export function normalizeConfigForSelectedModel(
  config: InstanceConfig,
  model?: ModelInfo | null,
  options: NormalizeInstanceConfigOptions = {},
) {
  const defaults = defaultInstanceConfig()
  const candidate: InstanceConfig = {
    ...config,
    embedding: defaults.embedding,
    pooling: defaults.pooling,
    reranking: defaults.reranking,
  }
  const normalized = normalizeInstanceConfig(candidate, model, options)

  return {
    ...normalized,
    changes: options.context === 'create' ? [] : diffVectorCleanup(config, normalized.config),
  }
}
