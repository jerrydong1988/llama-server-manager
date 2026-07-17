import type { InstanceConfig, ModelInfo, EngineInfo } from './store/types'
import { defaultInstanceConfig } from './store/defaults'

export interface Warning {
  field: keyof InstanceConfig
  severity: 'high' | 'medium' | 'low'
  key: string
}

// Known CLI flags collected from server.rs generate_command().
// If custom args contain these flags, guide the user to the matching config UI.
export const KNOWN_FLAGS = new Set([
  // Basic
  '-m', '-a', '--alias', '--lora', '--lora-init-without-apply', '--lora-scaled',
  '--mmproj', '--mmproj-url', '--mmproj-auto', '--no-mmproj', '--no-mmproj-offload',
  '--chat-template', '--chat-template-file', '--skip-chat-parsing',
  '--reasoning-format', '--reasoning', '--reasoning-budget', '--reasoning-budget-message',
  '--reasoning-preserve', '--no-reasoning-preserve',
  '--chat-template-kwargs', '--jinja', '--no-jinja', '--grammar-file', '--grammar',
  // Performance & Context
  '-c', '-ngl', '-t', '-b', '-ub', '-np', '-cb', '--no-cont-batching', '--cache-prompt', '--no-cache-prompt',
  '--threads-batch', '--threads-http', '--keep', '--cache-reuse', '-cram', '--warmup', '--no-warmup',
  '-ctxcp', '-cms', '--swa-full',
  // RoPE / YaRN
  '--rope-scaling', '--rope-scale', '--rope-freq-base', '--rope-freq-scale',
  '--yarn-ext-factor', '--yarn-attn-factor', '--yarn-beta-slow', '--yarn-beta-fast', '--yarn-orig-ctx',
  // Flash Attention & Memory
  '-fa', '--n-cpu-moe', '--cpu-moe', '-cmoe', '--mlock', '--no-mmap', '--no-repack', '--numa',
  '--check-tensors', '--perf', '--fit', '-fitt', '-fitc', '--direct-io', '-dio',
  // KV Cache
  '-ctk', '-ctv', '-ctkd', '-ctvd', '--kv-unified', '--no-kv-unified', '--no-kv-offload', '--cache-idle-slots', '--no-cache-idle-slots',
  // GPU & Device
  '-dev', '-sm', '-ts', '-mg', '--override-kv',
  // Server & Network
  '--host', '--port', '--api-key', '--api-key-file',
  '--ssl-key-file', '--ssl-cert-file', '--path', '--api-prefix',
  '--cors-origins', '--cors-methods', '--cors-headers', '--cors-credentials', '--no-cors-credentials',
  '--no-ui', '--offline', '--ui-config-file', '--ui-config', '--ui-mcp-proxy', '--agent', '-ag',
  // Embedding & Generation
  '--embedding', '--pooling', '--embd-normalize', '--reranking',
  // Server features
  '--metrics', '--props', '--slots', '--no-slots', '--slot-save-path', '--log-prompts-dir', '-sps',
  '--context-shift', '--no-context-shift',
  '--prefill-assistant', '--no-prefill-assistant',
  '--rpc', '--sse-ping-interval', '--reuse-port',
  // Multi-Model & Media
  '--models-dir', '--models-preset', '--models-max', '--models-autoload', '--no-models-autoload',
  '--image-min-tokens', '--image-max-tokens', '--mtmd-batch-max-tokens', '--tags', '--media-path', '--tools',
  // Generation
  '-n', '--ignore-eos', '--json-schema', '-jf',
  '--temp', '--top-k', '--top-p', '--repeat-penalty', '--seed', '--min-p',
  '--presence-penalty', '--frequency-penalty', '--repeat-last-n', '-r',
  '-sp', '--spm-infill', '-bs',
  // Advanced Sampling
  '--mirostat', '--mirostat-lr', '--mirostat-ent',
  '--xtc-probability', '--xtc-threshold',
  '--dynatemp-range', '--dynatemp-exp', '--typical-p',
  '--dry-multiplier', '--dry-base', '--dry-allowed-length', '--dry-penalty-last-n', '--dry-sequence-breaker',
  '--adaptive-target', '--adaptive-decay', '--top-n-sigma', '-l',
  '--samplers', '--sampler-seq',
  // Speculative Decoding
  '--spec-type', '-md', '-ngld', '--spec-draft-n-max', '--spec-draft-n-min',
  '--spec-draft-p-min', '--spec-draft-p-split', '--spec-draft-device',
  '-lcs', '-lcd', '--spec-default', '--no-spec-draft-backend-sampling', '-td', '-tbd',
  // Misc
  '-to', '--sleep-idle-seconds', '--context-shift', '-v',
])

const SWA_UNSUPPORTED = ['qwen2vl', 'qwen3vl', 'qwen2-vl', 'qwen3-vl']

function hasMetadata(model: ModelInfo | null | undefined): boolean {
  return !!model?.capabilities?.metadata_complete
}

function hasBuiltinMtp(model: ModelInfo | null | undefined): boolean {
  return !!(model?.capabilities?.has_builtin_mtp ?? model?.has_mtp_head)
}

function isVisionModel(model: ModelInfo | null | undefined): boolean | null {
  if (!model) return null
  if (model.capabilities?.is_vision_model) return true
  if (hasMetadata(model)) return false
  return null
}

function isMmprojArtifact(model: ModelInfo | null | undefined): boolean {
  return !!(model?.capabilities?.is_mmproj || model?.file_type === 'mmproj')
}

function isSwaUnsupported(model: ModelInfo | null | undefined): boolean {
  const family = model?.capabilities?.vision_family?.toLowerCase() || ''
  if (family === 'qwen-vl') return true
  const arch = model?.architecture?.toLowerCase()
  return !!arch && SWA_UNSUPPORTED.some(a => arch.includes(a))
}

export function validateConfig(
  config: InstanceConfig,
  model: ModelInfo | null | undefined,
  engine: EngineInfo | null | undefined,
): Warning[] {
  const w: Warning[] = []

  // Group A: parameter logic conflicts.

  // A1: reasoning=off but reasoning-related parameters are still set; merge into one warning.
  if (config.reasoning === 'off' && (
    (config.reasoning_effort && config.reasoning_effort !== '') ||
    (config.reasoning_format && config.reasoning_format !== '' && config.reasoning_format !== 'none') ||
    (config.reasoning_budget && config.reasoning_budget !== '') ||
    (config.reasoning_budget_message && config.reasoning_budget_message !== '') ||
    (config.reasoning_preserve && config.reasoning_preserve !== '')
  ))
    w.push({ field: 'reasoning', severity: 'high', key: 'warnA1' })

  // A2: generation/sampling parameters are still set in embedding mode.
  if (config.embedding) {
    const defaults = defaultInstanceConfig()
    const genDefaults: Record<string, string | number | boolean> = {
      n_predict: defaults.n_predict, temp: defaults.temp, top_k: defaults.top_k,
      top_p: defaults.top_p, repeat_penalty: defaults.repeat_penalty,
      seed: defaults.seed, min_p: defaults.min_p,
      presence_penalty: defaults.presence_penalty, frequency_penalty: defaults.frequency_penalty,
      repeat_last_n: defaults.repeat_last_n, ignore_eos: defaults.ignore_eos,
      json_schema: defaults.json_schema, reverse_prompt: defaults.reverse_prompt,
      special: defaults.special, spm_infill: defaults.spm_infill, backend_sampling: defaults.backend_sampling,
    }
    let hasGenParam = false
    for (const [k, defVal] of Object.entries(genDefaults)) {
      const val = (config as any)[k]
      if (typeof defVal === 'number' && typeof val === 'number' && Math.abs(val - defVal) > 0.001) {
        hasGenParam = true; break
      }
      if (k === 'json_schema' && val !== '') { hasGenParam = true; break }
      if (k === 'reverse_prompt' && val !== '') { hasGenParam = true; break }
      if (typeof defVal === 'boolean' && val !== defVal) { hasGenParam = true; break }
    }
    if (config.mirostat !== 0 || Math.abs(config.mirostat_lr - 0.1) > 0.001 || Math.abs(config.mirostat_ent - 5.0) > 0.001 ||
        Math.abs(config.xtc_probability) > 0.001 || Math.abs(config.xtc_threshold - 0.1) > 0.001 ||
        config.dynatemp_range !== 0 || config.dynatemp_exp !== 1.0 ||
        config.typical_p !== 1.0 || config.dry_multiplier !== 0 ||
        Math.abs(config.dry_base - 1.75) > 0.001 || config.dry_allowed_length !== 2) hasGenParam = true
    if (hasGenParam)
      w.push({ field: 'embedding', severity: 'high', key: 'warnA2' })
  }

  // A3: external draft requirements depend on the speculative decoding mode.
  if (config.spec_type && config.spec_type !== '' && config.spec_type !== 'none') {
    const isDraftMtp = config.spec_type.includes('draft-mtp')
    const needsExternalDraft = ['draft-simple', 'draft-eagle3', 'draft-dflash'].some(t => config.spec_type.includes(t))
    if (needsExternalDraft && !config.draft_model_path) {
      w.push({ field: 'draft_model_path', severity: 'medium', key: 'warnA3' })
    } else if (isDraftMtp && !config.draft_model_path && !hasBuiltinMtp(model)) {
      w.push({
        field: 'draft_model_path',
        severity: hasMetadata(model) ? 'medium' : 'low',
        key: hasMetadata(model) ? 'warnA3MtpNeedsDraft' : 'warnA3MtpUnknown',
      })
    }
  }

  // A4: backend_sampling on ROCm.
  if (config.backend_sampling && engine?.backend?.toLowerCase().includes('rocm'))
    w.push({ field: 'backend_sampling', severity: 'high', key: 'warnA4' })

  // A5: spec_type is empty but speculative decoding parameters are non-default.
  if ((!config.spec_type || config.spec_type === '' || config.spec_type === 'none') &&
      (config.draft_tokens !== 3 && config.draft_tokens !== 0 || config.spec_draft_n_min !== 0 ||
       config.spec_draft_p_min !== 0 || config.spec_draft_p_split !== 0.1))
    w.push({ field: 'spec_type', severity: 'medium', key: 'warnA5' })

  // A6: ctx_size_auto is enabled while RoPE/YaRN parameters are non-default.
  if (config.ctx_size_auto && (
    config.rope_scaling !== '' || config.rope_scale !== 0 ||
    config.rope_freq_base !== 0 || config.rope_freq_scale !== 0 ||
   config.yarn_ext_factor >= 0 || config.yarn_attn_factor !== -1 ||
   config.yarn_beta_slow > 0 || config.yarn_beta_fast !== -1))
    w.push({ field: 'ctx_size_auto', severity: 'medium', key: 'warnA6' })

  // A7: ctx_size exceeds four times the model context.
  if (!config.ctx_size_auto && config.ctx_size > 0 && model?.context_length && config.ctx_size > model.context_length * 4)
    w.push({ field: 'ctx_size', severity: 'medium', key: 'warnA7' })

  // A8: image tokens are set without a projector.
  if ((config.image_min_tokens > 0 || config.image_max_tokens > 0) &&
      !config.mmproj_path && !config.mmproj_url)
    w.push({ field: 'image_min_tokens', severity: 'medium', key: 'warnA8' })

  // A9: swa_full is enabled on a model family known to reject it.
  if (config.swa_full && isSwaUnsupported(model))
    w.push({ field: 'swa_full', severity: 'medium', key: 'warnA9' })

  // A10: flash_attn=on while backend is CPU.
  if (config.flash_attn === 'on' && engine?.backend?.toLowerCase() === 'cpu')
    w.push({ field: 'flash_attn', severity: 'medium', key: 'warnA10' })

  // A11: grammar and grammar_file are both set.
  if (config.grammar && config.grammar_file)
    w.push({ field: 'grammar', severity: 'low', key: 'warnA11' })

  // A12: chat_template and chat_template_file are both set.
  if (config.chat_template && config.chat_template_file)
    w.push({ field: 'chat_template', severity: 'low', key: 'warnA12' })

  // A13: api_key and api_key_file are both set.
  if (config.api_key && config.api_key_file)
    w.push({ field: 'api_key', severity: 'low', key: 'warnA13' })

  // Group B: redundant or meaningless settings.

  // B2: cache_ram or cache_reuse is set while cache_prompt is disabled.
  if ((config.cache_ram !== 0 || config.cache_reuse > 0) && !config.cache_prompt)
    w.push({ field: 'cache_prompt', severity: 'low', key: 'warnB2' })

  // B3: slot-related parameters are set while slots_enabled is disabled.
  if (!config.slots_enabled &&
      (config.slot_save_path !== '' || config.slot_prompt_similarity !== 0.1))
    w.push({ field: 'slots_enabled', severity: 'low', key: 'warnB3' })

  // B4: mirostat is enabled but temp is non-default.
  if (config.mirostat > 0 && Math.abs(config.temp - 0.8) > 0.001)
    w.push({ field: 'mirostat', severity: 'low', key: 'warnB4' })

  // B5: samplers are customized while individual sampling parameters are also non-default.
  if (config.samplers && config.samplers !== '') {
    if (config.temp !== 0.8 || config.top_k !== 40 || config.top_p !== 0.95 ||
        config.min_p !== 0.05 || config.typical_p !== 1.0)
      w.push({ field: 'samplers', severity: 'low', key: 'warnB5' })
  }

  // B6: pooling is set while embedding is disabled.
  if (config.pooling && config.pooling !== '' && !config.embedding)
    w.push({ field: 'pooling', severity: 'low', key: 'warnB6' })

  // B7: gpu_layers=0 while device points to GPU.
  if (config.gpu_layers === 0 && config.device && config.device !== '')
    w.push({ field: 'gpu_layers', severity: 'low', key: 'warnB7' })

  // B8: ctx_checkpoints/checkpoint_min_step is set while cache_ram=0.
  if (config.cache_ram === 0 && ((config.ctx_checkpoints !== 32 && config.ctx_checkpoints !== 0) || config.checkpoint_min_step !== 0))
    w.push({ field: 'ctx_checkpoints', severity: 'low', key: 'warnB8' })

  // B9: lookup_cache is set while spec_type does not include ngram.
  if ((config.lookup_cache_static || config.lookup_cache_dynamic) &&
      (!config.spec_type || !config.spec_type.includes('ngram')))
    w.push({ field: 'lookup_cache_static', severity: 'low', key: 'warnB9' })

  // B10: --no-mmproj and --mmproj are both set (new tri-state or legacy boolean).
  if ((config.mmproj_mode === 'off' || (!config.mmproj_mode && config.no_mmproj)) && config.mmproj_path)
    w.push({ field: 'no_mmproj', severity: 'low', key: 'warnB10' })

  // B11: json_schema and json_schema_file are both set.
  if (config.json_schema && config.json_schema_file)
    w.push({ field: 'json_schema', severity: 'low', key: 'warnB11' })

  // Group C: environment-aware checks.

  // C1: mmproj is set but model metadata cannot confirm a vision-capable model.
  if (config.mmproj_path) {
    const vision = isVisionModel(model)
    if (isMmprojArtifact(model)) {
      w.push({ field: 'model_path', severity: 'medium', key: 'warnC5' })
    } else if (vision === false) {
      w.push({ field: 'mmproj_path', severity: 'low', key: 'warnC1' })
    } else if (vision === null) {
      w.push({ field: 'mmproj_path', severity: 'low', key: 'warnC1Unknown' })
    }
  }

  // C2: removed; draft-mtp draft model requirements depend on builtin MTP heads and cannot be inferred from config only.

  // C3: n_predict=-1 enables infinite generation and ignore_eos ignores stop tokens.
  if (config.n_predict === -1 && config.ignore_eos)
    w.push({ field: 'n_predict', severity: 'medium', key: 'warnC3' })

  // C4: draft-mtp with builtin MTP heads plus an extra draft model may be redundant.
  if (config.draft_model_path && config.spec_type?.includes('draft-mtp') && hasBuiltinMtp(model))
    w.push({ field: 'draft_model_path', severity: 'low', key: 'warnC4' })

  // D1: custom args conflict with known config fields.
  if (config.custom_args.length > 0) {
    const conflicts: string[] = []
    for (const arg of config.custom_args) {
      if (KNOWN_FLAGS.has(arg)) conflicts.push(arg)
    }
    if (conflicts.length > 0) {
      w.push({ field: 'custom_args', severity: 'medium', key: 'warnD1' })
    }
  }

  return w
}
