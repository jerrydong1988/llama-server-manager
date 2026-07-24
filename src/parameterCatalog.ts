import type { EngineCapabilities, InstanceConfig } from './store'

export type ParameterSource = 'inherited' | 'explicit' | 'managed' | 'inactive' | 'unsupported'
export type ParameterDefaultKind = 'engine' | 'model' | 'automatic' | 'application'

type LocalizedText = { zh: string; en: string }

export type ParameterDefinition = {
  flags?: string[]
  defaultKind?: ParameterDefaultKind
  verifiedDefaults?: Record<number, LocalizedText>
  managed?: boolean
  dependency?: (config: InstanceConfig, isEmbedding: boolean) => boolean
}

const text = (zh: string, en: string): LocalizedText => ({ zh, en })
const specEnabled = (config: InstanceConfig, isEmbedding: boolean) => (
  !isEmbedding && !!config.spec_type && config.spec_type !== 'none'
)
const fitEnabled = (config: InstanceConfig) => (config.fit_mode || (config.fit ? 'on' : '')) === 'on'
const multimodalEnabled = (config: InstanceConfig, isEmbedding: boolean) => (
  !isEmbedding && (
    !!config.mmproj_path.trim()
    || !!config.mmproj_url.trim()
    || (config.mmproj_mode || (config.no_mmproj ? 'off' : config.mmproj_auto ? 'on' : '')) !== 'off'
  )
)

const b10068 = (zh: string, en: string) => ({ 10068: text(zh, en) })
const b10105 = (zh: string, en: string) => ({ 10105: text(zh, en) })

/**
 * Behavioural metadata that cannot be inferred safely from a displayed value.
 * It deliberately does not drive command generation: server.rs remains the
 * authority, while this catalogue explains intent, dependencies and defaults.
 */
export const PARAMETER_CATALOG: Partial<Record<keyof InstanceConfig, ParameterDefinition>> = {
  model_path: { flags: ['--model', '-m'], managed: true },
  host: { flags: ['--host'], managed: true },
  port: { flags: ['--port'], managed: true },
  embedding: { flags: ['--embedding'], managed: true },
  pooling: { flags: ['--pooling'], managed: true },
  reranking: { flags: ['--reranking'], managed: true },
  metrics: { flags: ['--metrics'], managed: true },
  props: { flags: ['--props'], managed: true },
  slots_enabled: { flags: ['--slots', '--no-slots'], managed: true },

  gpu_layers: {
    flags: ['--n-gpu-layers', '-ngl'],
    defaultKind: 'automatic',
    verifiedDefaults: b10068('自动选择', 'automatic'),
  },
  ctx_size: {
    flags: ['--ctx-size', '-c'],
    defaultKind: 'model',
    verifiedDefaults: b10068('读取模型上下文；启用内存适配时可由适配过程调整', 'loaded from the model; memory fitting may adjust it when enabled'),
  },
  threads: { flags: ['--threads', '-t'], defaultKind: 'automatic', verifiedDefaults: b10068('自动选择', 'automatic') },
  threads_batch: { flags: ['--threads-batch', '-tb'], defaultKind: 'automatic', verifiedDefaults: b10068('与主线程策略自动协调', 'automatic with the main thread policy') },
  threads_http: { flags: ['--threads-http'], defaultKind: 'automatic', verifiedDefaults: b10068('自动选择', 'automatic') },
  parallel: { flags: ['--parallel', '-np'], defaultKind: 'automatic', verifiedDefaults: b10068('自动选择', 'automatic') },

  cont_batching: { flags: ['--cont-batching', '--no-cont-batching', '-cb'], verifiedDefaults: b10068('开启', 'enabled') },
  cache_prompt: { flags: ['--cache-prompt', '--no-cache-prompt'], verifiedDefaults: b10068('开启', 'enabled') },
  warmup: { flags: ['--warmup', '--no-warmup'], verifiedDefaults: b10068('开启', 'enabled') },
  jinja: { flags: ['--jinja', '--no-jinja'], verifiedDefaults: b10068('开启', 'enabled') },
  cache_idle_slots: { flags: ['--cache-idle-slots', '--no-cache-idle-slots'], verifiedDefaults: b10068('开启', 'enabled') },
  prefill_assistant: { flags: ['--prefill-assistant', '--no-prefill-assistant'], verifiedDefaults: b10068('开启', 'enabled') },
  models_autoload: { flags: ['--models-autoload', '--no-models-autoload'], verifiedDefaults: b10068('开启', 'enabled') },
  context_shift: { flags: ['--context-shift', '--no-context-shift'], verifiedDefaults: b10068('关闭', 'disabled') },
  perf: { flags: ['--perf', '--no-perf'], verifiedDefaults: b10068('开启（已按源码核验）', 'enabled (source-verified)') },

  load_mode: { flags: ['--load-mode', '-lm'], verifiedDefaults: b10105('mmap', 'mmap') },
  no_mmap: { flags: ['--mmap', '--no-mmap'], verifiedDefaults: b10068('内存映射开启', 'memory mapping enabled') },
  no_repack: { flags: ['--repack', '--no-repack'], verifiedDefaults: b10068('权重重打包开启', 'weight repacking enabled') },
  no_kv_offload: { flags: ['--kv-offload', '--no-kv-offload'], verifiedDefaults: b10068('GPU 卸载开启', 'GPU offload enabled') },
  no_mmproj_offload: {
    flags: ['--mmproj-offload', '--no-mmproj-offload'],
    verifiedDefaults: b10068('投影器 GPU 加速开启', 'projector GPU offload enabled'),
    dependency: multimodalEnabled,
  },
  no_ui: { flags: ['--ui', '--no-ui'] },

  draft_gpu_layers: {
    flags: ['--spec-draft-ngl', '--n-gpu-layers-draft', '-ngld'],
    defaultKind: 'automatic',
    verifiedDefaults: b10068('自动选择', 'automatic'),
    dependency: specEnabled,
  },
  draft_tokens: {
    flags: ['--spec-draft-n-max'],
    verifiedDefaults: b10068('3', '3'),
    dependency: specEnabled,
  },
  spec_draft_n_min: { flags: ['--spec-draft-n-min'], dependency: specEnabled },
  spec_draft_p_min: { flags: ['--spec-draft-p-min'], dependency: specEnabled },
  spec_draft_p_split: { flags: ['--spec-draft-p-split'], dependency: specEnabled },
  spec_draft_device: { flags: ['--spec-draft-device'], dependency: specEnabled },
  draft_model_path: { flags: ['--model-draft', '-md'], dependency: specEnabled },
  cache_type_draft_k: { flags: ['--spec-draft-type-k', '--cache-type-k-draft', '-ctkd'], dependency: specEnabled },
  cache_type_draft_v: { flags: ['--spec-draft-type-v', '--cache-type-v-draft', '-ctvd'], dependency: specEnabled },
  spec_default: { flags: ['--spec-default'], dependency: specEnabled },
  spec_draft_backend_sampling: { flags: ['--spec-draft-backend-sampling', '--no-spec-draft-backend-sampling'], dependency: specEnabled },
  spec_draft_threads: { flags: ['--spec-draft-threads', '-td'], dependency: specEnabled },
  spec_draft_threads_batch: { flags: ['--spec-draft-threads-batch', '-tbd'], dependency: specEnabled },
  lookup_cache_static: { flags: ['--lookup-cache-static', '-lcs'], dependency: specEnabled },
  lookup_cache_dynamic: { flags: ['--lookup-cache-dynamic', '-lcd'], dependency: specEnabled },

  mirostat_lr: { flags: ['--mirostat-lr'], dependency: config => config.mirostat > 0 },
  mirostat_ent: { flags: ['--mirostat-ent'], dependency: config => config.mirostat > 0 },
  xtc_threshold: { flags: ['--xtc-threshold'], dependency: config => config.xtc_probability > 0 },
  dynatemp_exp: { flags: ['--dynatemp-exp'], dependency: config => config.dynatemp_range > 0 },
  dry_base: { flags: ['--dry-base'], dependency: config => config.dry_multiplier > 0 },
  dry_allowed_length: { flags: ['--dry-allowed-length'], dependency: config => config.dry_multiplier > 0 },
  dry_penalty_last_n: { flags: ['--dry-penalty-last-n'], dependency: config => config.dry_multiplier > 0 },
  dry_sequence_breaker: { flags: ['--dry-sequence-breaker'], dependency: config => config.dry_multiplier > 0 },
  adaptive_decay: { flags: ['--adaptive-decay'], dependency: config => config.adaptive_target >= 0 },
  fit_target: { flags: ['--fit-target', '-fitt'], dependency: config => fitEnabled(config) },
  fit_ctx: { flags: ['--fit-ctx', '-fitc'], dependency: config => fitEnabled(config) },

  backend_sampling: { flags: ['--backend-sampling', '-bs'] },
  image_min_tokens: {
    flags: ['--image-min-tokens'],
    defaultKind: 'model',
    verifiedDefaults: b10068('从模型元数据读取', 'read from model metadata'),
    dependency: (_config, isEmbedding) => !isEmbedding,
  },
  image_max_tokens: {
    flags: ['--image-max-tokens'],
    defaultKind: 'model',
    verifiedDefaults: b10068('从模型元数据读取', 'read from model metadata'),
    dependency: (_config, isEmbedding) => !isEmbedding,
  },
}

export const SYSTEM_MANAGED_PARAMETER_KEYS = new Set<keyof InstanceConfig>(
  (Object.entries(PARAMETER_CATALOG) as Array<[keyof InstanceConfig, ParameterDefinition]>)
    .filter(([, definition]) => definition.managed)
    .map(([key]) => key),
)

export function parameterDependencyActive(key: keyof InstanceConfig, config: InstanceConfig, isEmbedding: boolean) {
  return PARAMETER_CATALOG[key]?.dependency?.(config, isEmbedding) ?? true
}

export function parameterFlags(key: keyof InstanceConfig): string[] {
  return PARAMETER_CATALOG[key]?.flags ?? []
}

const engineBuild = (version?: string) => {
  const match = version?.match(/(?:version:\s*)?(\d{4,})/i)
  return match ? Number(match[1]) : undefined
}

const localizeReportedDefault = (value: string, lang: string) => {
  if (lang !== 'zh-CN') return value
  const normalized = value.trim().toLowerCase()
  const common: Record<string, string> = {
    enabled: '开启',
    disabled: '关闭',
    true: '开启',
    false: '关闭',
    auto: '自动',
    automatic: '自动',
    'read from model': '从模型读取',
    'same as --threads': '与 --threads 相同',
  }
  return common[normalized] ?? value
}

export function parameterEngineDefault(
  key: keyof InstanceConfig,
  capabilities: EngineCapabilities | undefined,
  engineVersion: string | undefined,
  lang: string,
  fallbackFlags: string[] = [],
): string | undefined {
  const definition = PARAMETER_CATALOG[key]
  const build = engineBuild(engineVersion)
  const verified = build === undefined ? undefined : definition?.verifiedDefaults?.[build]
  if (verified) return lang === 'zh-CN' ? verified.zh : verified.en

  const reported = capabilities?.reportedDefaults
  for (const flag of definition?.flags ?? fallbackFlags) {
    if (reported?.[flag]) return localizeReportedDefault(reported[flag], lang)
  }
  return undefined
}
