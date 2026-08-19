const zhCN = {
  title: '资源规划',
  description: '保存和启动前，按当前配置与实时可用容量估算。',
  loading: '正在重新估算…',
  unavailable: '尚无有效规划结果。',
  refresh: '刷新',
  confidence: '置信度',
  context: '上下文',
  slots: '并行槽位',
  offload: 'GPU 卸载',
  shards: '模型分片',
  ram: '系统内存',
  vram: '显存',
  required: '预计 / 范围',
  available: '当前可用',
  reserve: '安全预留',
  headroom: '预计余量',
  reasons: '判断依据',
  assumptions: '不确定性与假设',
  statuses: { feasible: '可行', constrained: '余量受限', infeasible: '不可行', unknown: '无法确认' },
  confidences: { high: '高', medium: '中', low: '低' },
}

const enUS: typeof zhCN = {
  title: 'Resource plan',
  description: 'Estimated from the current configuration and live capacity before save and launch.',
  loading: 'Recalculating…',
  unavailable: 'No valid resource plan is available yet.',
  refresh: 'Refresh',
  confidence: 'Confidence',
  context: 'Context',
  slots: 'Parallel slots',
  offload: 'GPU offload',
  shards: 'Model shards',
  ram: 'System RAM',
  vram: 'VRAM',
  required: 'Expected / range',
  available: 'Available now',
  reserve: 'Safety reserve',
  headroom: 'Expected headroom',
  reasons: 'Decision basis',
  assumptions: 'Uncertainty and assumptions',
  statuses: { feasible: 'Feasible', constrained: 'Constrained', infeasible: 'Infeasible', unknown: 'Unknown' },
  confidences: { high: 'High', medium: 'Medium', low: 'Low' },
}

const reasonCopy: Record<string, { zh: string; en: string }> = {
  manual_command_not_inspectable: { zh: '手动命令无法安全解析，结果不能确认。', en: 'The manual command cannot be safely inspected.' },
  custom_arguments_may_change_resources: { zh: '自定义参数可能改变资源需求。', en: 'Custom arguments may change resource requirements.' },
  multi_model_residency_is_dynamic: { zh: '多模型驻留会随运行时请求变化。', en: 'Multi-model residency changes with runtime demand.' },
  remote_offload_capacity_not_measured: { zh: '未测量远端 RPC 设备容量。', en: 'Remote RPC device capacity is not measured.' },
  per_device_gpu_capacity_not_measured: { zh: '多 GPU 仅有聚合容量，无法验证每张卡的分配。', en: 'Only aggregate GPU capacity is available; per-device placement cannot be verified.' },
  metadata_overrides_not_interpreted: { zh: '元数据覆盖参数未纳入估算。', en: 'Metadata overrides are not interpreted by the estimator.' },
  model_artifact_unavailable: { zh: '主模型文件不可用。', en: 'The primary model artifact is unavailable.' },
  projector_artifact_unavailable: { zh: '投影模型文件不可用。', en: 'The projector artifact is unavailable.' },
  draft_artifact_unavailable: { zh: '草稿模型文件不可用。', en: 'The draft model artifact is unavailable.' },
  adapter_artifact_unavailable: { zh: '适配器文件不可用。', en: 'The adapter artifact is unavailable.' },
  model_metadata_unavailable: { zh: '无法读取主模型 GGUF 元数据。', en: 'Primary-model GGUF metadata could not be read.' },
  projector_metadata_unavailable: { zh: '无法读取投影模型 GGUF 元数据。', en: 'Projector GGUF metadata could not be read.' },
  draft_metadata_unavailable: { zh: '无法读取草稿模型 GGUF 元数据。', en: 'Draft-model GGUF metadata could not be read.' },
  incomplete_model_shards: { zh: '主模型分片不完整。', en: 'The primary model shard set is incomplete.' },
  incomplete_projector_shards: { zh: '投影模型分片不完整。', en: 'The projector shard set is incomplete.' },
  incomplete_draft_shards: { zh: '草稿模型分片不完整。', en: 'The draft-model shard set is incomplete.' },
  incomplete_adapter_shards: { zh: '适配器分片不完整。', en: 'The adapter shard set is incomplete.' },
  model_shard_count_exceeds_limit: { zh: '主模型声明的分片数超过安全扫描上限。', en: 'The primary model declares more shards than the safe scan limit.' },
  projector_shard_count_exceeds_limit: { zh: '投影模型声明的分片数超过安全扫描上限。', en: 'The projector declares more shards than the safe scan limit.' },
  draft_shard_count_exceeds_limit: { zh: '草稿模型声明的分片数超过安全扫描上限。', en: 'The draft model declares more shards than the safe scan limit.' },
  adapter_shard_count_exceeds_limit: { zh: '适配器声明的分片数超过安全扫描上限。', en: 'The adapter declares more shards than the safe scan limit.' },
  context_length_unavailable: { zh: '模型上下文长度未知。', en: 'The model context length is unavailable.' },
  automatic_gpu_layers_need_vram_capacity: { zh: '自动 GPU 层数需要可用显存数据。', en: 'Automatic GPU layers require live VRAM capacity.' },
  model_layer_count_unavailable: { zh: '模型层数未知，GPU 卸载比例是近似值。', en: 'Model layer count is unavailable, so GPU offload is approximate.' },
  draft_layer_count_unavailable: { zh: '草稿模型层数未知，GPU 卸载比例是近似值。', en: 'Draft-model layer count is unavailable, so GPU offload is approximate.' },
  moe_tensor_distribution_is_approximate: { zh: 'MoE 张量分布只能区间估算。', en: 'MoE tensor distribution is represented as a range.' },
  kv_metadata_incomplete: { zh: 'KV 缓存形状元数据不完整。', en: 'KV-cache shape metadata is incomplete.' },
  unknown_kv_cache_type: { zh: 'KV 缓存类型未知，按 F16 估算。', en: 'Unknown KV-cache type; F16 is assumed.' },
  llama_fit_may_reduce_unset_parameters: { zh: 'llama.cpp --fit 可能缩减未显式设置的参数。', en: 'llama.cpp --fit may reduce parameters that were not explicitly set.' },
  insufficient_available_ram: { zh: '当前可用系统内存低于最低需求。', en: 'Available system RAM is below the minimum estimate.' },
  insufficient_available_vram: { zh: '当前可用显存低于最低需求。', en: 'Available VRAM is below the minimum estimate.' },
  ram_capacity_unavailable: { zh: '无法读取系统内存容量。', en: 'System RAM capacity is unavailable.' },
  vram_capacity_unavailable: { zh: '无法读取显存容量。', en: 'VRAM capacity is unavailable.' },
  estimate_range_exceeds_available_headroom: { zh: '估算上界超过当前安全余量。', en: 'The upper estimate exceeds current safe headroom.' },
  context_uses_model_training_limit: { zh: '上下文按模型训练上限计算。', en: 'Context uses the model training limit.' },
  context_uses_safe_default: { zh: '上下文按 4096 token 安全默认值估算。', en: 'Context uses a conservative 4096-token default.' },
  fit_context_range_starts_at_minimum: { zh: '--fit 可调整的上下文区间以下限为起点。', en: 'The adjustable --fit context range starts at its configured minimum.' },
  parallel_slots_are_engine_selected: { zh: '并行槽位由引擎自动选择。', en: 'Parallel slots are selected by the engine.' },
  automatic_gpu_layers_follow_current_free_vram: { zh: '自动 GPU 层数按当前空闲显存近似。', en: 'Automatic GPU layers are approximated from current free VRAM.' },
  compute_buffers_use_default_embedding_width: { zh: '计算缓冲区按 4096 维默认宽度估算。', en: 'Compute buffers use a default embedding width of 4096.' },
  kv_range_covers_unknown_attention_shape: { zh: 'KV 区间覆盖未知的注意力头形状。', en: 'The KV range covers an unknown attention-head shape.' },
  sliding_window_reduces_part_of_kv_cache: { zh: '滑动窗口可能降低部分 KV 驻留量。', en: 'Sliding-window attention may reduce part of KV residency.' },
  prompt_cache_is_demand_driven_up_to_configured_limit: { zh: '提示缓存按需增长，最大值为配置上限。', en: 'Prompt cache grows on demand up to the configured limit.' },
  context_checkpoint_usage_is_workload_dependent: { zh: '上下文检查点占用取决于实际工作负载。', en: 'Context-checkpoint usage depends on the workload.' },
  metal_uses_unified_system_memory: { zh: 'Metal 的模型与运行缓冲区计入统一系统内存。', en: 'Metal model and runtime buffers are counted against unified system memory.' },
}

const launchZh = {
  planning: '正在规划资源…',
  infeasibleTitle: '资源规划不可行',
  infeasibleBody: '当前可用内存不足，资源规划器已阻止启动。请释放资源，或调整上下文、缓存和 GPU 卸载设置。',
  riskTitle: '确认资源风险',
  constrainedBody: '资源估算范围超过了当前安全余量，启动可能失败。是否仍要继续？',
  unknownBody: '资源规划器缺少足够信息，无法确认此次启动是否可行。是否仍要继续？',
}

const launchEn: typeof launchZh = {
  planning: 'Planning resources…',
  infeasibleTitle: 'Resource plan infeasible',
  infeasibleBody: 'Available memory is insufficient, so the resource planner blocked this launch. Free resources or adjust context, cache, and GPU offload settings.',
  riskTitle: 'Confirm resource risk',
  constrainedBody: 'The resource estimate exceeds current safe headroom and launch may fail. Continue anyway?',
  unknownBody: 'The resource planner lacks enough information to confirm feasibility. Continue anyway?',
}

export type ResourcePlanLabels = typeof zhCN

export function getResourcePlanLabels(lang: string): ResourcePlanLabels {
  return lang === 'zh-CN' ? zhCN : enUS
}

export function getResourcePlanReason(lang: string, code: string): string {
  const copy = reasonCopy[code]
  if (!copy) return code.split('_').join(' ')
  return lang === 'zh-CN' ? copy.zh : copy.en
}

export function getResourcePlanLaunchCopy(lang: string): typeof launchZh {
  return lang === 'zh-CN' ? launchZh : launchEn
}
