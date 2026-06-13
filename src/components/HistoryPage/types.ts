// Shared types for History feature
export interface ConfigSnapshot {
  model_path: string
  engine_path: string
  engine_backend: string
  ctx_size: number
  gpu_layers: number
  batch_size: number
  threads: number
  flash_attn: string
  cont_batching: boolean
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
}
