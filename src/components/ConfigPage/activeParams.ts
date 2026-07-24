import type { InstanceConfig } from '../../store'
import { VECTOR_ALLOWED_FIELDS } from '../../modelPolicy'
import { canonicalConfigFields } from './configWorkspace'

/**
 * Mirror of server.rs generate_command() conditions.
 * Returns the set of config fields that would produce a CLI flag
 * in the generated command (given the current config + isEmbedding state).
 */
export function getActiveParams(config: InstanceConfig, isEmbedding: boolean): Set<keyof InstanceConfig> {
  if (config.launch_mode === 'manual') {
    return new Set(config.manual_command.trim() ? ['manual_command'] : [])
  }

  if (Array.isArray(config.explicit_overrides)) {
    const a = new Set<keyof InstanceConfig>(canonicalConfigFields(config.explicit_overrides as Array<keyof InstanceConfig>))
    a.add('model_path'); a.add('host'); a.add('port')
    if (config.metrics) a.add('metrics')
    if (config.props) a.add('props')
    a.add('slots_enabled')
    if (config.embedding) a.add('embedding')
    if (config.embedding && config.pooling) a.add('pooling')
    if (config.reranking) a.add('reranking')

    const remove = (keys: Array<keyof InstanceConfig>) => keys.forEach(key => a.delete(key))
    const specActive = !isEmbedding && !!config.spec_type && config.spec_type !== 'none' && a.has('spec_type')
    if (!specActive) remove([
      'draft_model_path', 'draft_gpu_layers', 'draft_tokens', 'spec_draft_n_min',
      'spec_draft_p_min', 'spec_draft_p_split', 'spec_draft_device',
      'lookup_cache_static', 'lookup_cache_dynamic', 'spec_default',
      'spec_draft_backend_sampling', 'spec_draft_threads', 'spec_draft_threads_batch',
      'cache_type_draft_k', 'cache_type_draft_v',
    ])
    const fitMode = config.fit_mode || (config.fit ? 'on' : '')
    if (fitMode !== 'on') remove(['fit_target', 'fit_ctx'])
    if (!(config.mirostat > 0 && a.has('mirostat'))) remove(['mirostat_lr', 'mirostat_ent'])
    if (!(config.xtc_probability > 0 && a.has('xtc_probability'))) remove(['xtc_threshold'])
    if (!(config.dynatemp_range > 0 && a.has('dynatemp_range'))) remove(['dynatemp_exp'])
    if (!(config.dry_multiplier > 0 && a.has('dry_multiplier'))) remove([
      'dry_base', 'dry_allowed_length', 'dry_penalty_last_n', 'dry_sequence_breaker',
    ])
    if (!(config.adaptive_target >= 0 && a.has('adaptive_target'))) remove(['adaptive_decay'])
    if (isEmbedding) {
      for (const key of a) {
        if (!VECTOR_ALLOWED_FIELDS.has(key)) a.delete(key)
      }
    }
    return a
  }

  const a = new Set<keyof InstanceConfig>()
  const e = isEmbedding
  const specActive = !e && !!config.spec_type && config.spec_type !== 'none'

  // ── Always active ──
  a.add('model_path'); a.add('host'); a.add('port')

  // ── Basic ──
  if (config.alias) a.add('alias')
  if (!e) {
    if (config.lora_path) a.add('lora_path')
    if (config.lora_init_without_apply) a.add('lora_init_without_apply')
    if (config.lora_scaled) a.add('lora_scaled')
    if (config.mmproj_path) a.add('mmproj_path')
    if (config.mmproj_url) a.add('mmproj_url')
    if (config.mmproj_mode || config.mmproj_auto || config.no_mmproj) a.add('mmproj_mode')
    if (config.no_mmproj_offload) a.add('no_mmproj_offload')
    if (config.chat_template) a.add('chat_template')
    if (config.chat_template_file) a.add('chat_template_file')
    if (config.skip_chat_parsing) a.add('skip_chat_parsing')
    if (config.reasoning_format) a.add('reasoning_format')
    if (config.reasoning) a.add('reasoning')
    if (config.reasoning_preserve) a.add('reasoning_preserve')
    if (config.reasoning_budget) a.add('reasoning_budget')
    if (config.reasoning_budget_message) a.add('reasoning_budget_message')
    if (config.reasoning_effort) a.add('reasoning_effort')
    if (!config.jinja) a.add('jinja')
    if (config.grammar_file) a.add('grammar_file')
    if (config.grammar) a.add('grammar')
  }

  // ── Performance & Context ──
  if (!config.ctx_size_auto) a.add('ctx_size')
  if (!config.gpu_layers_auto) a.add('gpu_layers')
  if (config.threads > 0) a.add('threads')
  if (config.batch_size > 0) a.add('batch_size')
  if (config.ubatch_size > 0) a.add('ubatch_size')
  if (config.parallel > 0 || config.parallel === -1) a.add('parallel')
  if (!config.cont_batching) a.add('cont_batching')
  if (!config.cache_prompt) a.add('cache_prompt')           // negative: --no-cache-prompt when false
  if (config.threads_batch > 0) a.add('threads_batch')
  if (config.threads_http >= 0) a.add('threads_http')
  if (config.keep !== 0) a.add('keep')
  if (config.cache_reuse > 0) a.add('cache_reuse')
  if (config.cache_ram !== 8192) a.add('cache_ram')
  if (!config.warmup) a.add('warmup')
  if (config.ctx_checkpoints !== 32) a.add('ctx_checkpoints')
  if (config.checkpoint_min_step !== 8192) a.add('checkpoint_min_step')
  if (config.swa_full) a.add('swa_full')

  // ── RoPE / YaRN ──
  if (config.rope_scaling) a.add('rope_scaling')
  if (config.rope_scale > 0) a.add('rope_scale')
  if (config.rope_freq_base > 0) a.add('rope_freq_base')
  if (config.rope_freq_scale > 0) a.add('rope_freq_scale')
  if (config.yarn_ext_factor >= 0) a.add('yarn_ext_factor')
  if (config.yarn_attn_factor !== -1) a.add('yarn_attn_factor')
  if (config.yarn_beta_slow > 0) a.add('yarn_beta_slow')
  if (config.yarn_beta_fast !== -1) a.add('yarn_beta_fast')
  if (config.yarn_orig_ctx > 0) a.add('yarn_orig_ctx')

  // ── Flash Attention ──
  if (config.flash_attn !== 'auto' && config.flash_attn !== '') a.add('flash_attn')

  // ── Memory & Loading ──
  if (config.moe_cpu_layers > 0) a.add('moe_cpu_layers')
  if (config.cpu_moe) a.add('cpu_moe')
  if (config.load_mode) a.add('load_mode')
  if (config.no_repack) a.add('no_repack')
  if (config.numa_mode || config.numa) a.add('numa_mode')
  if (config.perf) a.add('perf')
  if (config.check_tensors) a.add('check_tensors')
  if (config.fit_mode || config.fit) a.add('fit_mode')
  if (config.fit_mode !== 'off') {
    if (config.fit_target) a.add('fit_target')
    if (config.fit_ctx !== 4096) a.add('fit_ctx')
  }

  // ── KV Cache ──
  if (config.cache_type_k) a.add('cache_type_k')
  if (config.cache_type_v) a.add('cache_type_v')
  if (config.cache_type_draft_k) a.add('cache_type_draft_k')
  if (config.cache_type_draft_v) a.add('cache_type_draft_v')
  if (config.kv_unified_mode || config.kv_unified) a.add('kv_unified_mode')
  if (!config.cache_idle_slots) a.add('cache_idle_slots')   // negative: --no-cache-idle-slots when false
  if (config.no_kv_offload) a.add('no_kv_offload')

  // ── GPU & Device ──
  if (config.device) a.add('device')
  if (config.split_mode) a.add('split_mode')
  if (config.tensor_split) a.add('tensor_split')
  if (config.main_gpu > 0) a.add('main_gpu')
  if (config.override_kv) a.add('override_kv')

  // ── Speculative Decoding ──
  if (specActive) {
    a.add('spec_type')                                      // --spec-type itself
    if (config.draft_model_path) a.add('draft_model_path')
    if (config.draft_gpu_layers !== 99) a.add('draft_gpu_layers')
    if (config.draft_tokens > 0) a.add('draft_tokens')
    if (config.spec_draft_n_min > 0) a.add('spec_draft_n_min')
    if (config.spec_draft_p_min > 0) a.add('spec_draft_p_min')
    if (Math.abs(config.spec_draft_p_split - 0.1) > 0.001) a.add('spec_draft_p_split')
    if (config.spec_draft_device) a.add('spec_draft_device')
    if (config.lookup_cache_static) a.add('lookup_cache_static')
    if (config.lookup_cache_dynamic) a.add('lookup_cache_dynamic')
    if (config.spec_default) a.add('spec_default')
    if (!config.spec_draft_backend_sampling) a.add('spec_draft_backend_sampling')
    if (config.spec_draft_threads > 0) a.add('spec_draft_threads')
    if (config.spec_draft_threads_batch > 0) a.add('spec_draft_threads_batch')
  }

   // ── Network ──
   if (config.api_key) a.add('api_key')
   if (config.api_key_file) a.add('api_key_file')
   if (config.ssl_key_file) a.add('ssl_key_file')
   if (config.ssl_cert_file) a.add('ssl_cert_file')
   if (config.no_ui) a.add('no_ui')
   if (config.offline) a.add('offline')
   if (config.path_prefix) a.add('path_prefix')
   if (config.api_prefix) a.add('api_prefix')
   if (config.cors_origins) a.add('cors_origins')
   if (config.cors_methods) a.add('cors_methods')
   if (config.cors_headers) a.add('cors_headers')
   if (config.cors_credentials) a.add('cors_credentials')
   if (config.ui_config_file) a.add('ui_config_file')
   if (config.ui_config) a.add('ui_config')
    if (config.ui_mcp_proxy) a.add('ui_mcp_proxy')
    if (config.agent) a.add('agent')
   // New server params
   if (config.rpc_servers) a.add('rpc_servers')
   if (Math.abs(config.sse_ping_interval - 30) > 0.001) a.add('sse_ping_interval')
   if (config.reuse_port) a.add('reuse_port')

  // ── Embedding ──
  if (config.embedding) {
    a.add('embedding')
    if (config.pooling) a.add('pooling')
    if (config.embd_normalize !== 2) a.add('embd_normalize')
    if (config.reranking) a.add('reranking')
  }

  // ── Generation (gated by !isEmbedding) ──
  if (!e) {
    if (config.n_predict !== -1) a.add('n_predict')
    if (config.ignore_eos) a.add('ignore_eos')
    if (config.json_schema) a.add('json_schema')
    if (config.json_schema_file) a.add('json_schema_file')
    if (Math.abs(config.temp - 0.8) > 0.001) a.add('temp')
    if (config.top_k !== 40) a.add('top_k')
    if (Math.abs(config.top_p - 0.95) > 0.001) a.add('top_p')
    if (Math.abs(config.repeat_penalty - 1) > 0.001) a.add('repeat_penalty')
    if (config.seed !== -1) a.add('seed')
    if (Math.abs(config.min_p - 0.05) > 0.001) a.add('min_p')
    if (Math.abs(config.presence_penalty) > 0.001) a.add('presence_penalty')
    if (Math.abs(config.frequency_penalty) > 0.001) a.add('frequency_penalty')
    if (config.repeat_last_n !== 64) a.add('repeat_last_n')
    if (config.reverse_prompt) a.add('reverse_prompt')
    if (config.special) a.add('special')
    if (config.spm_infill) a.add('spm_infill')
    if (config.backend_sampling) a.add('backend_sampling')
    if (config.mirostat > 0) {
      a.add('mirostat')
      if (Math.abs(config.mirostat_lr - 0.1) > 0.001) a.add('mirostat_lr')
      if (Math.abs(config.mirostat_ent - 5) > 0.001) a.add('mirostat_ent')
    }
    if (config.xtc_probability > 0) {
      a.add('xtc_probability')
      if (Math.abs(config.xtc_threshold - 0.1) > 0.001) a.add('xtc_threshold')
    }
    if (config.dynatemp_range > 0) {
      a.add('dynatemp_range')
      if (Math.abs(config.dynatemp_exp - 1) > 0.001) a.add('dynatemp_exp')
    }
    if (Math.abs(config.typical_p - 1) > 0.001) a.add('typical_p')
    if (config.dry_multiplier > 0) {
      a.add('dry_multiplier')
      if (Math.abs(config.dry_base - 1.75) > 0.001) a.add('dry_base')
      if (config.dry_allowed_length !== 2) a.add('dry_allowed_length')
      if (config.dry_penalty_last_n !== -1) a.add('dry_penalty_last_n')
      if (config.dry_sequence_breaker) a.add('dry_sequence_breaker')
    }
    if (Math.abs(config.adaptive_target + 1) > 0.001) {
      a.add('adaptive_target')
      if (Math.abs(config.adaptive_decay - 0.9) > 0.001) a.add('adaptive_decay')
    }
    if (config.top_n_sigma >= 0) a.add('top_n_sigma')
    if (config.logit_bias) a.add('logit_bias')
    if (config.samplers) a.add('samplers')
    if (config.sampler_seq) a.add('sampler_seq')
  }

  // ── Server features ──
  if (config.timeout > 0) a.add('timeout')
  if (config.sleep_idle >= 0) a.add('sleep_idle')
  if (config.context_shift) a.add('context_shift')
  if (config.verbose) a.add('verbose')
  if (config.metrics) a.add('metrics')
  if (config.props) a.add('props')
  if (!config.slots_enabled) a.add('slots_enabled')         // negative: --no-slots when false

  // Float comparison for similarity
  if (Math.abs(config.slot_prompt_similarity - 0.1) > 0.001) a.add('slot_prompt_similarity')
  if (config.slot_save_path) a.add('slot_save_path')
  if (config.log_prompts_dir) a.add('log_prompts_dir')
  if (!config.prefill_assistant) a.add('prefill_assistant')

  // ── Multi-Model & Media ──
  if (config.models_dir) a.add('models_dir')
  if (config.models_preset) a.add('models_preset')
  if (config.models_max !== 4) a.add('models_max')
  if (!config.models_autoload) a.add('models_autoload')
  if (config.image_min_tokens > 0) a.add('image_min_tokens')
  if (config.image_max_tokens > 0) a.add('image_max_tokens')
  if (config.tags) a.add('tags')
  if (config.media_path) a.add('media_path')
  if (config.tools) a.add('tools')

  // ── Custom args ──
  if (config.custom_args.length > 0) a.add('custom_args')

  if (e) {
    for (const key of a) {
      if (!VECTOR_ALLOWED_FIELDS.has(key)) a.delete(key)
    }
  }

  return a
}
