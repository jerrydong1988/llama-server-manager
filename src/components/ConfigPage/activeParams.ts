import type { InstanceConfig } from '../../store'

/**
 * Mirror of server.rs generate_command() conditions.
 * Returns the set of config fields that would produce a CLI flag
 * in the generated command (given the current config + isEmbedding state).
 */
export function getActiveParams(config: InstanceConfig, isEmbedding: boolean): Set<keyof InstanceConfig> {
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
    if (config.mmproj_auto) a.add('mmproj_auto')
    if (config.no_mmproj) a.add('no_mmproj')
    if (config.no_mmproj_offload) a.add('no_mmproj_offload')
    if (config.chat_template) a.add('chat_template')
    if (config.chat_template_file) a.add('chat_template_file')
    if (config.skip_chat_parsing) a.add('skip_chat_parsing')
    if (config.reasoning_format) a.add('reasoning_format')
    if (config.reasoning) a.add('reasoning')
    if (config.reasoning_budget) a.add('reasoning_budget')
    if (config.reasoning_budget_message) a.add('reasoning_budget_message')
    if (config.reasoning_effort) a.add('reasoning_effort')
    if (config.jinja) a.add('jinja')
    if (config.grammar_file) a.add('grammar_file')
    if (config.grammar) a.add('grammar')
  }

  // ── Performance & Context ──
  if (!config.ctx_size_auto) a.add('ctx_size')  // -c flag
  // -ngl always emitted
  if (config.threads > 0) a.add('threads')
  if (config.batch_size > 0) a.add('batch_size')
  if (config.ubatch_size > 0) a.add('ubatch_size')
  if (config.parallel > 0 || config.parallel === -1) a.add('parallel')
  if (config.cont_batching) a.add('cont_batching')
  if (!config.cache_prompt) a.add('cache_prompt')           // negative: --no-cache-prompt when false
  if (config.threads_batch > 0) a.add('threads_batch')
  if (config.threads_http >= 0) a.add('threads_http')
  if (config.keep > 0) a.add('keep')
  if (config.cache_reuse > 0) a.add('cache_reuse')
  if (config.cache_ram > 0) a.add('cache_ram')
  if (config.warmup) a.add('warmup')
  if (config.ctx_checkpoints !== 32) a.add('ctx_checkpoints')
  if (config.checkpoint_min_step > 0) a.add('checkpoint_min_step')
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
  if (!e && config.flash_attn !== 'auto' && config.flash_attn !== '') a.add('flash_attn')

  // ── Memory & Loading ──
  if (config.moe_cpu_layers > 0) a.add('moe_cpu_layers')
  if (config.mlock) a.add('mlock')
  if (config.no_mmap) a.add('no_mmap')
  if (config.no_repack) a.add('no_repack')
  if (config.numa) a.add('numa')
  if (config.check_tensors) a.add('check_tensors')
  if (config.fit) a.add('fit')

  // ── KV Cache ──
  if (config.cache_type_k) a.add('cache_type_k')
  if (config.cache_type_v) a.add('cache_type_v')
  if (config.cache_type_draft_k) a.add('cache_type_draft_k')
  if (config.cache_type_draft_v) a.add('cache_type_draft_v')
  if (config.kv_unified) a.add('kv_unified')
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
    if (config.draft_gpu_layers > 0 && config.draft_gpu_layers < 99) a.add('draft_gpu_layers')
    if (config.draft_tokens > 0) a.add('draft_tokens')
    if (config.spec_draft_n_min > 0) a.add('spec_draft_n_min')
    if (config.spec_draft_p_min > 0) a.add('spec_draft_p_min')
    if (Math.abs(config.spec_draft_p_split - 0.1) > 0.001) a.add('spec_draft_p_split')
    if (config.spec_draft_device) a.add('spec_draft_device')
    if (config.lookup_cache_static) a.add('lookup_cache_static')
    if (config.lookup_cache_dynamic) a.add('lookup_cache_dynamic')
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
   if (config.ui_config_file) a.add('ui_config_file')
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
    if (config.n_predict !== 0) a.add('n_predict')
    if (config.ignore_eos) a.add('ignore_eos')
    if (config.json_schema) a.add('json_schema')
    if (config.temp > 0) a.add('temp')
    if (config.top_k > 0) a.add('top_k')
    if (config.top_p > 0) a.add('top_p')
    if (config.repeat_penalty > 0) a.add('repeat_penalty')
    if (config.seed >= 0) a.add('seed')
    if (config.min_p > 0) a.add('min_p')
    if (config.presence_penalty > 0) a.add('presence_penalty')
    if (config.frequency_penalty > 0) a.add('frequency_penalty')
    if (config.repeat_last_n > 0) a.add('repeat_last_n')
    if (config.reverse_prompt) a.add('reverse_prompt')
    if (config.special) a.add('special')
    if (config.spm_infill) a.add('spm_infill')
    if (config.backend_sampling) a.add('backend_sampling')
    if (config.mirostat > 0) a.add('mirostat')
    if (config.mirostat_lr > 0) a.add('mirostat_lr')
    if (config.mirostat_ent > 0) a.add('mirostat_ent')
    if (config.xtc_probability > 0) a.add('xtc_probability')
    if (config.xtc_threshold > 0) a.add('xtc_threshold')
    if (config.dynatemp_range > 0) a.add('dynatemp_range')
    if (config.dynatemp_exp > 0) a.add('dynatemp_exp')
    if (config.typical_p < 1 && config.typical_p > 0) a.add('typical_p')
    if (config.dry_multiplier > 0) a.add('dry_multiplier')
    if (config.dry_base > 0) a.add('dry_base')
    if (config.dry_allowed_length > 0) a.add('dry_allowed_length')
    if (config.dry_penalty_last_n > 0) a.add('dry_penalty_last_n')
    if (config.dry_sequence_breaker) a.add('dry_sequence_breaker')
    if (config.adaptive_target > 0) a.add('adaptive_target')
    if (config.adaptive_decay > 0) a.add('adaptive_decay')
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
  if (config.prefill_assistant) a.add('prefill_assistant')

  // ── Multi-Model & Media ──
  if (config.models_dir) a.add('models_dir')
  if (config.models_preset) a.add('models_preset')
  if (config.models_max !== 4) a.add('models_max')
  if (config.models_autoload) a.add('models_autoload')
  if (config.image_min_tokens > 0) a.add('image_min_tokens')
  if (config.image_max_tokens > 0) a.add('image_max_tokens')
  if (config.tags) a.add('tags')
  if (config.media_path) a.add('media_path')
  if (config.tools) a.add('tools')

  // ── Custom args ──
  if (config.custom_args.length > 0) a.add('custom_args')

  return a
}
