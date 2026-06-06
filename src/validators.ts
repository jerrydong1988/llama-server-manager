import type { InstanceConfig, ModelInfo, EngineInfo } from './store'
import { defaultInstanceConfig } from './store'

export interface Warning {
  field: keyof InstanceConfig
  severity: 'high' | 'medium' | 'low'
  key: string
}

const VISION_ARCHS = [
  'qwen2vl', 'qwen3vl', 'qwen2-vl', 'qwen3-vl',
  'llava', 'minicpmv', 'minicpm-v', 'internvl', 'intern-vl',
  'gemma3', 'glm4v', 'glm-4v', 'phi3v', 'phi3-v', 'phi4v', 'phi4-v',
  'gpt4v', 'cogvlm', 'fuyu',
]
const SWA_UNSUPPORTED = ['qwen2vl', 'qwen3vl', 'qwen2-vl', 'qwen3-vl']

function isVisionArch(arch: string | undefined): boolean {
  if (!arch) return false
  const lower = arch.toLowerCase()
  return VISION_ARCHS.some(a => lower.includes(a))
}

function isSwaUnsupported(arch: string | undefined): boolean {
  if (!arch) return false
  const lower = arch.toLowerCase()
  return SWA_UNSUPPORTED.some(a => lower.includes(a))
}

export function validateConfig(
  config: InstanceConfig,
  model: ModelInfo | null | undefined,
  engine: EngineInfo | null | undefined,
): Warning[] {
  const w: Warning[] = []

  // ═══ A 组：参数逻辑矛盾 ═══

  // A1: reasoning=off 但 reasoning 相关参数仍设置（合并为单条警告）
  if (config.reasoning === 'off' && (
    (config.reasoning_effort && config.reasoning_effort !== '') ||
    (config.reasoning_format && config.reasoning_format !== '' && config.reasoning_format !== 'none') ||
    (config.reasoning_budget && config.reasoning_budget !== '') ||
    (config.reasoning_budget_message && config.reasoning_budget_message !== '')
  ))
    w.push({ field: 'reasoning', severity: 'high', key: 'warnA1' })

  // A2: embedding 模式下生成/采样参数仍设置
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
    if (config.mirostat !== 0 || config.mirostat_lr !== 0 || config.mirostat_ent !== 0 ||
        config.xtc_probability !== 0 || config.xtc_threshold !== 0 ||
        config.dynatemp_range !== 0 || config.dynatemp_exp !== 0 ||
        config.typical_p !== 1.0 || config.dry_multiplier !== 0 ||
        config.dry_base !== 0 || config.dry_allowed_length !== 0) hasGenParam = true
    if (hasGenParam)
      w.push({ field: 'embedding', severity: 'high', key: 'warnA2' })
  }

  // A3: spec_type 非空、非 mtp、非 ngram 但草稿模型未设置
  if (config.spec_type && config.spec_type !== '' && config.spec_type !== 'none') {
    const needsDraft = ['draft-simple', 'draft-eagle3'].some(t => config.spec_type.includes(t))
    if (needsDraft && !config.draft_model_path)
      w.push({ field: 'draft_model_path', severity: 'high', key: 'warnA3' })
  }

  // A4: backend_sampling 在 ROCm 环境
  if (config.backend_sampling && engine?.backend?.toLowerCase().includes('rocm'))
    w.push({ field: 'backend_sampling', severity: 'high', key: 'warnA4' })

  // A5: spec_type 为空但推测解码参数非默认值
  if ((!config.spec_type || config.spec_type === '' || config.spec_type === 'none') &&
      (config.draft_tokens !== 16 && config.draft_tokens !== 0 || config.spec_draft_n_min !== 0 ||
       config.spec_draft_p_min !== 0 || config.spec_draft_p_split !== 0.1))
    w.push({ field: 'spec_type', severity: 'medium', key: 'warnA5' })

  // A6: ctx_size_auto 且 RoPE/YaRN 参数非默认值
  if (config.ctx_size_auto && (
    config.rope_scaling !== '' || config.rope_scale !== 0 ||
    config.rope_freq_base !== 0 || config.rope_freq_scale !== 0 ||
   config.yarn_ext_factor >= 0 || config.yarn_attn_factor !== -1 ||
   config.yarn_beta_slow !== 0 || config.yarn_beta_fast !== -1))
    w.push({ field: 'ctx_size_auto', severity: 'medium', key: 'warnA6' })

  // A7: ctx_size 超过模型上下文 4 倍
  if (!config.ctx_size_auto && config.ctx_size > 0 && model?.context_length && config.ctx_size > model.context_length * 4)
    w.push({ field: 'ctx_size', severity: 'medium', key: 'warnA7' })

  // A8: image tokens 设置了但无投影器
  if ((config.image_min_tokens > 0 || config.image_max_tokens > 0) &&
      !config.mmproj_path && !config.mmproj_url)
    w.push({ field: 'image_min_tokens', severity: 'medium', key: 'warnA8' })

  // A9: swa_full 但模型不支持
  if (config.swa_full && isSwaUnsupported(model?.architecture))
    w.push({ field: 'swa_full', severity: 'medium', key: 'warnA9' })

  // A10: flash_attn=on 但后端为 CPU
  if (config.flash_attn === 'on' && engine?.backend?.toLowerCase() === 'cpu')
    w.push({ field: 'flash_attn', severity: 'medium', key: 'warnA10' })

  // A11: grammar 和 grammar_file 同时设置
  if (config.grammar && config.grammar_file)
    w.push({ field: 'grammar', severity: 'low', key: 'warnA11' })

  // A12: chat_template 和 chat_template_file 同时设置
  if (config.chat_template && config.chat_template_file)
    w.push({ field: 'chat_template', severity: 'low', key: 'warnA12' })

  // A13: api_key 和 api_key_file 同时设置
  if (config.api_key && config.api_key_file)
    w.push({ field: 'api_key', severity: 'low', key: 'warnA13' })

  // ═══ B 组：冗余/无意义 ═══

  // B1: warmup + embedding
  if (config.warmup && config.embedding)
    w.push({ field: 'warmup', severity: 'low', key: 'warnB1' })

  // B2: cache_ram 或 cache_reuse 设置了但 cache_prompt 关闭
  if ((config.cache_ram > 0 || config.cache_reuse > 0) && !config.cache_prompt)
    w.push({ field: 'cache_prompt', severity: 'low', key: 'warnB2' })

  // B3: slot 相关参数设置了但 slots_enabled 关闭
  if (!config.slots_enabled &&
      (config.slot_save_path !== '' || config.slot_prompt_similarity !== 0.1))
    w.push({ field: 'slots_enabled', severity: 'low', key: 'warnB3' })

  // B4: mirostat 开启但 temp 非默认值
  if (config.mirostat > 0 && Math.abs(config.temp - 0.8) > 0.001)
    w.push({ field: 'mirostat', severity: 'low', key: 'warnB4' })

  // B5: samplers 自定义了且单独采样参数也为非默认
  if (config.samplers && config.samplers !== '') {
    if (config.temp !== 0.8 || config.top_k !== 40 || config.top_p !== 0.9 ||
        config.min_p !== 0.05 || config.typical_p !== 1.0)
      w.push({ field: 'samplers', severity: 'low', key: 'warnB5' })
  }

  // B6: pooling 设置了但 embedding 未启用
  if (config.pooling && config.pooling !== '' && !config.embedding)
    w.push({ field: 'pooling', severity: 'low', key: 'warnB6' })

  // B7: gpu_layers=0 但 device 指向 GPU
  if (config.gpu_layers === 0 && config.device && config.device !== '')
    w.push({ field: 'gpu_layers', severity: 'low', key: 'warnB7' })

  // B8: ctx_checkpoints/checkpoint_min_step 设置了但 cache_ram=0
  if (config.cache_ram === 0 && ((config.ctx_checkpoints !== 32 && config.ctx_checkpoints !== 0) || config.checkpoint_min_step !== 0))
    w.push({ field: 'ctx_checkpoints', severity: 'low', key: 'warnB8' })

  // B9: lookup_cache 设置了但 spec_type 不含 ngram
  if ((config.lookup_cache_static || config.lookup_cache_dynamic) &&
      (!config.spec_type || !config.spec_type.includes('ngram')))
    w.push({ field: 'lookup_cache_static', severity: 'low', key: 'warnB9' })

  // B10: --no-mmproj 与 --mmproj 同时设置
  if (config.no_mmproj && config.mmproj_path)
    w.push({ field: 'no_mmproj', severity: 'low', key: 'warnB10' })

  // ═══ C 组：环境感知 ═══

  // C1: mmproj 非空但模型架构不像视觉模型
  if (config.mmproj_path && model?.architecture &&
      !isVisionArch(model.architecture) && model.file_type !== 'mmproj')
    w.push({ field: 'mmproj_path', severity: 'low', key: 'warnC1' })

  // C2: draft_model_path 非空且 spec_type 含 mtp（MTP 使用主模型内部头，不需要单独草稿模型）
  if (config.draft_model_path && config.spec_type && config.spec_type.includes('mtp'))
    w.push({ field: 'draft_model_path', severity: 'low', key: 'warnC2' })

  // C3: n_predict=-1 无限生成 + ignore_eos 忽略结束符
  if (config.n_predict === -1 && config.ignore_eos)
    w.push({ field: 'n_predict', severity: 'medium', key: 'warnC3' })

  return w
}
