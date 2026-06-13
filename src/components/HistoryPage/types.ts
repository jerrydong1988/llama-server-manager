// Shared types for History feature
export interface ConfigSnapshot {
  model_path: string
  engine_path: string
  engine_backend: string
  ctx_size: number
  gpu_layers: number
  batch_size: number
  ubatch_size: number
  threads: number
  threads_batch: number
  flash_attn: string
  spec_type: string
  draft_tokens: number
  cache_type_k: string
  cache_type_v: string
  cache_ram: number
  cont_batching: boolean
  parallel: number
  rope_scaling: string
  rope_scale: number
  mirostat: number
  temp: number
  top_k: number
  top_p: number
  host: string
  port: number
}

export interface SessionSummary {
  data_points: number
  avg_tps: number | null
  peak_tps: number | null
  avg_ptps: number | null
  total_prompt_tok: number | null
  total_gen_tok: number | null
  max_vram_mb: number | null
  vram_total_mb: number | null
  avg_gpu_pct: number | null
  avg_cpu_pct: number | null
  request_count: number | null
  avg_req_gen_tps: number | null
  peak_req_gen_tps: number | null
  avg_req_prompt_tps: number | null
  total_req_prompt_tok: number | null
  total_req_gen_tok: number | null
  avg_spec_accept: number | null
  load_time_secs: number | null
}

export interface SessionMeta {
  id: string
  instance_name: string
  instance_id: string
  started_at: number
  ended_at: number | null
  duration_secs: number | null
  model_path: string
  engine_backend: string
  config_snapshot: ConfigSnapshot
  summary: SessionSummary | null
  unclean: boolean
}

export interface DataPoint {
  ts: number
  cpu: number | null
  mem_mb: number | null
  gpu: number | null
  vram_u: number | null
  vram_t: number | null
  tps: number | null
  ptps: number | null
  p_tok: number | null
  g_tok: number | null
  proc: number | null
  def: number | null
  busy: number | null
  req_prompt_tps: number | null
  req_gen_tps: number | null
  req_prompt_tokens: number | null
  req_gen_tokens: number | null
  spec_accept_rate: number | null
  spec_accepted: number | null
  spec_generated: number | null
  load_time_secs: number | null
}
