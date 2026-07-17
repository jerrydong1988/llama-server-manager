export interface ModelCapabilities {
  metadata_complete?: boolean
  is_embedding_model?: boolean
  is_reranker_model?: boolean
  has_builtin_mtp?: boolean
  mtp_layers?: number
  is_vision_model?: boolean
  vision_family?: string
  is_mmproj?: boolean
  projector_family?: string
}

export interface GgufMetadataSummary {
  architecture?: string
  context_length?: number
  quant_type?: string
  capabilities?: ModelCapabilities
}

export interface ModelInfo {
  id: string
  name: string
  path: string
  size: number
  architecture?: string
  context_length?: number
  quant_type?: string
  has_mtp_head?: boolean
  capabilities?: ModelCapabilities
  file_type: string
  is_shard?: boolean
}

export interface EngineInfo {
  id: string
  name: string
  dir: string
  exe: string
  version: string
  backend: string
  custom_name?: string
  capabilities?: EngineCapabilities
}

export type EngineCapabilityStatus = 'unprobed' | 'detected' | 'partial' | 'timeout' | 'failed'

export interface EngineCapabilities {
  status: EngineCapabilityStatus | string
  supportedFlags: string[]
  helpHash: string
  executableFingerprint: string
  probedAt?: number
  error?: string
}

export interface InstanceConfig {
  // Basic
  id: string; name: string; engine_id: string; model_path: string; alias: string;
  lora_path: string; mmproj_path: string; lora_init_without_apply: boolean; lora_scaled: string;
  chat_template: string; chat_template_file: string; skip_chat_parsing: boolean;
  reasoning_format: string; reasoning_effort: string; reasoning: string;
  reasoning_preserve: string; jinja: boolean; reasoning_budget: string; reasoning_budget_message: string;
  grammar_file: string; grammar: string;
  // Performance & Context
  ctx_size: number; ctx_size_auto: boolean; gpu_layers_auto: boolean; gpu_layers: number;
  threads: number; batch_size: number; ubatch_size: number; parallel: number;
  cont_batching: boolean; cache_prompt: boolean; threads_batch: number;
  threads_http: number; keep: number; cache_reuse: number; cache_ram: number;
  warmup: boolean; ctx_checkpoints: number; checkpoint_min_step: number;
  swa_full: boolean;
  // RoPE / YaRN
  rope_scaling: string; rope_scale: number; rope_freq_base: number; rope_freq_scale: number;
  yarn_ext_factor: number; yarn_attn_factor: number; yarn_beta_slow: number; yarn_beta_fast: number; yarn_orig_ctx: number;
  // Memory & KV Cache
  flash_attn: string; moe_cpu_layers: number; cpu_moe: boolean; mlock: boolean;
  no_mmap: boolean; no_repack: boolean; direct_io: boolean; numa: boolean; numa_mode: string; context_shift: boolean;
  perf: boolean; check_tensors: boolean; fit: boolean; fit_mode: string; fit_target: string; fit_ctx: number; kv_unified: boolean; kv_unified_mode: string; cache_idle_slots: boolean; no_kv_offload: boolean;
  cache_type_k: string; cache_type_v: string;
  cache_type_draft_k: string; cache_type_draft_v: string;
  // Speculative Decoding
  draft_model_path: string; draft_gpu_layers: number; draft_tokens: number;
  spec_draft_n_min: number; spec_type: string;
  spec_draft_p_min: number; spec_draft_p_split: number; spec_draft_device: string;
  lookup_cache_static: string; lookup_cache_dynamic: string;
  spec_default: boolean; spec_draft_backend_sampling: boolean; spec_draft_threads: number; spec_draft_threads_batch: number;
  // GPU & Device
  device: string; split_mode: string; tensor_split: string; main_gpu: number;
  override_kv: string;
  // Server & Network
  host: string; port: number; api_key: string; api_key_file: string;
  ssl_key_file: string; ssl_cert_file: string; path_prefix: string; api_prefix: string;
  cors_origins: string; cors_methods: string; cors_headers: string; cors_credentials: string;
  no_ui: boolean; offline: boolean; ui_config_file: string; ui_config: string; ui_mcp_proxy: boolean; agent: boolean;
  embedding: boolean; pooling: string; embd_normalize: number; reranking: boolean;
  metrics: boolean; props: boolean; slots_enabled: boolean;
  slot_save_path: string; log_prompts_dir: string; slot_prompt_similarity: number; prefill_assistant: boolean;
  // Multi-Model & Media
  models_dir: string; models_preset: string; models_max: number; models_autoload: boolean;
  mmproj_url: string; mmproj_auto: boolean; mmproj_mode: string; no_mmproj: boolean; no_mmproj_offload: boolean; image_min_tokens: number; image_max_tokens: number; mtmd_batch_max_tokens: number;
  tags: string; media_path: string; tools: string;
  // Generation
  n_predict: number; ignore_eos: boolean; json_schema: string; json_schema_file: string;
  temp: number; top_k: number; top_p: number; repeat_penalty: number;
  seed: number; min_p: number; presence_penalty: number;
  frequency_penalty: number; repeat_last_n: number;
  reverse_prompt: string; special: boolean; spm_infill: boolean; backend_sampling: boolean;
  // Advanced Sampling
  mirostat: number; mirostat_lr: number; mirostat_ent: number;
  xtc_probability: number; xtc_threshold: number;
  dynatemp_range: number; dynatemp_exp: number; typical_p: number;
  dry_multiplier: number; dry_base: number;
  dry_allowed_length: number; dry_penalty_last_n: number;
  dry_sequence_breaker: string;
  adaptive_target: number; adaptive_decay: number; top_n_sigma: number;
  logit_bias: string; samplers: string; sampler_seq: string;
  // Misc
  timeout: number; sleep_idle: number; verbose: boolean; custom_args: string[];
  // Server features (aligned with llama.cpp master)
  rpc_servers: string; sse_ping_interval: number; reuse_port: boolean;
  auto_start?: boolean;
}

export interface Instance {
  id: string
  name: string
  status: 'running' | 'stopped' | 'error'
  model: string
  port: number
  healthCheck: 'ok' | 'fail' | 'pending'
  startTime?: number
  config: InstanceConfig
}

export interface LogEntry {
  instanceId: string
  text: string
  timestamp: number
}

export interface MsFileEntry {
  name: string
  path: string
  size: number
  file_type: string
  task_id?: string
  run_id?: string
  downloaded?: number
  version?: number
  status?: string
  error?: string
}

export interface WorkerDevice {
  device_type: string  // CUDA, ROCm, Metal, Vulkan, CPU
  name: string
  vram_mb: number
  free_mb: number
}

export type WorkerStatus = 'Online' | 'Offline' | 'Testing' | 'Unknown'

export interface WorkerInfo {
  id: string
  host: string
  port: number
  name: string
  devices: WorkerDevice[]
  status: WorkerStatus
  last_seen?: string
  auto_discovered: boolean
}

export interface Usb4Adapter {
  name: string
  if_index: number
  description: string
  status: string
  ip?: string
}

export interface DownloadProgress {
  id: string
  runId?: string
  fileName: string
  remotePath: string
  fileType: string
  saveDir: string
  downloaded: number
  total: number
  speed: number
  repoId: string
  source: string
  status: 'active' | 'completed' | 'cancelled' | 'error' | 'paused' | 'pausing' | 'queued'
  path?: string
  error?: string
  version?: number
  remoteChanged?: boolean
  createdAt?: number
  updatedAt?: number
  completedAt?: number
}

export interface DownloadQueueEntry {
  id: string
  repoId: string
  source: 'modelscope' | 'huggingface'
  files: MsFileEntry[]
  saveDir: string
  addedAt: number
}

export interface DownloadGroup {
  repoId: string
  files: DownloadProgress[]
}

export interface PersistedQueueEntry {
  id: string
  repo_id: string
  source: string
  files: MsFileEntry[]
  save_dir: string
  added_at: number
  status: string
}

export interface DownloadManagerSnapshot {
  queue: PersistedQueueEntry[]
  active_count: number
  max_concurrent: number
  resume_policy: string
  bandwidth_limit_bytes_per_sec: number
  low_priority_throttle: boolean
}

export interface SystemMetrics {
  cpu_percent: number
  memory_mb: number
  uptime_secs: number
  gpu_percent: number | null
  vram_used_mb: number | null
  vram_total_mb: number | null
  system_cpu_percent: number | null
  system_memory_total_mb: number | null
  system_memory_used_mb: number | null
  gpu_vendor: string | null
  gpu_name: string | null
}

export type ModelWorkload = 'inference' | 'embedding' | 'reranker'

export interface TelemetrySampleSummary {
  session_id: string
  instance_id: string
  ts: number
  cpu_percent: number | null
  memory_mb: number | null
  gpu_percent: number | null
  vram_used_mb: number | null
  vram_total_mb: number | null
  system_cpu_percent: number | null
  system_memory_used_mb: number | null
  system_memory_total_mb: number | null
  gpu_vendor: string | null
  gpu_name: string | null
  tokens_per_sec: number | null
  prompt_tokens_per_sec: number | null
  prompt_tokens_total: number | null
  generated_tokens_total: number | null
  requests_total: number | null
  decode_calls_total: number | null
  max_tokens_observed: number | null
  requests_processing: number | null
  requests_deferred: number | null
  busy_slots_per_decode: number | null
}

export interface TelemetryOverview {
  active_sessions: number
  sessions_24h: number
  avg_tokens_per_sec_24h: number
  peak_vram_mb_24h: number
  latest_samples: TelemetrySampleSummary[]
}

export interface TelemetrySessionSummary {
  id: string
  instance_id: string
  instance_name: string
  model_name: string
  model_path: string
  engine_id: string
  backend: string
  workload: ModelWorkload
  started_at: number
  stopped_at: number | null
  duration_secs: number | null
  avg_tokens_per_second?: number
  avg_tokens_per_sec: number
  peak_vram_mb: number
  sample_count: number
  stop_reason: string | null
}

export interface InferenceRequestSummary {
  session_id: string
  task_id: number
  slot_id: number
  completed_at: number
  source: string
  model: string | null
  target_instance_id: string | null
  http_status: number | null
  error_text: string | null
  prompt_tokens: number | null
  prompt_time_ms: number | null
  prompt_tps: number | null
  generated_tokens: number | null
  generation_time_ms: number | null
  generation_tps: number | null
  total_tokens: number | null
  total_time_ms: number | null
  spec_accept_rate: number | null
  spec_accepted: number | null
  spec_generated: number | null
  spec_gen_time_ms: number | null
}

export interface RunningInferenceTask {
  slot_id: number
  task_id: number
  started_at_ms: number
  updated_at_ms: number
  n_decoded: number
  tg: number
  tg_3s: number | null
  history: [number, number][]
  prompt_tokens: number | null
  prompt_time_ms: number | null
  prompt_tps: number | null
  gen_tokens: number | null
  gen_time_ms: number | null
  gen_tps: number | null
  total_tokens: number | null
  total_time_ms: number | null
  spec_accept_rate: number | null
  spec_accepted: number | null
  spec_generated: number | null
  spec_gen_time_ms: number | null
  completed: boolean
}

export interface PerfUpdateEvent {
  instanceId: string
  tasks: RunningInferenceTask[]
  lastCompleted: RunningInferenceTask | null
}

export interface MonitoringFrame {
  instanceId: string
  sessionId: string | null
  sessionStartedAt: number
  ts: number
  workload: ModelWorkload
  state: 'active' | 'idle' | 'warming' | 'unavailable'
  throughput: number | null
  throughputUnit: 'tok/s' | 'input tok/s'
  outputTokensPerSecond: number | null
  inputTokensPerSecond: number | null
  itemsPerSecond: number | null
  activeRequests: number
  queuedRequests: number
  slotCapacity: number | null
  busySlots: number | null
  averageLatencyMs: number | null
  successRate: number | null
  source: 'task' | 'llama' | 'vector-log' | 'idle' | 'unavailable'
  dataAgeMs: number
  system: SystemMetrics | null
}

export interface TelemetrySessionAnalysis {
  request_count: number
  avg_prompt_tokens: number
  avg_generated_tokens: number
  avg_total_tokens: number
  avg_prompt_tps: number
  avg_generation_tps: number
  avg_total_time_ms: number
  max_total_tokens: number
  avg_busy_slots: number
  max_busy_slots: number
  avg_cached_slots: number
  max_context_tokens: number
  slot_sample_count: number
  speculative_analysis: SpeculativeTelemetryAnalysis | null
  vector_analysis: VectorTelemetryAnalysis | null
  vector_baseline: VectorTelemetryBaseline | null
}

export interface SpeculativeTelemetryAnalysis {
  request_count: number
  acceptance_rate: number | null
  accepted_tokens: number
  generated_tokens: number
  avg_generation_time_ms: number | null
}

export interface VectorTrendBucket {
  timestamp: number
  inputTokensPerSecond: number | null
  itemsPerSecond: number
}

export interface VectorTelemetryAnalysis {
  workload: Exclude<ModelWorkload, 'inference'>
  logAvailable: boolean
  proxyAvailable: boolean
  completedItems: number | null
  inputTokens: number | null
  averageInputTokensPerSecond: number | null
  averageItemsPerSecond: number | null
  taskDurationP50Ms: number | null
  taskDurationP95Ms: number | null
  proxyRequestCount: number | null
  proxyItemCount: number | null
  proxyDurationP50Ms: number | null
  proxyDurationP95Ms: number | null
  proxySuccessRate: number | null
  proxyFailureRate: number | null
  trend: VectorTrendBucket[]
}

export interface VectorTelemetryBaseline {
  sessionCount: number
  averageInputTokensPerSecond: number | null
  averageItemsPerSecond: number | null
  taskDurationP95Ms: number | null
}

export interface DiagnosticFinding {
  id: string
  severity: 'info' | 'warning' | 'critical' | 'success'
  confidence: number
  title: string
  summary: string
  evidence: string[]
  recommendation: string[]
}

export interface TelemetrySessionDetail {
  samples: TelemetrySampleSummary[]
  requests: InferenceRequestSummary[]
  analysis: TelemetrySessionAnalysis
  diagnostics: DiagnosticFinding[]
}

export interface AppState {
  models: ModelInfo[]
  engines: EngineInfo[]
  instances: Instance[]
  logs: Record<string, LogEntry[]>
  isLoading: boolean
  runtimeWarnings: string[]
  defaultEngineId: string | null
  modelDirs: string[]
  engineDirs: string[]
  activeConfigInstanceId: string | null
  activeTab: string
  darkMode: boolean
  workers: WorkerInfo[]
  clusterScanning: boolean
  downloadTasks: Record<string, DownloadProgress>
  downloadQueue: DownloadQueueEntry[]
  sysMetrics: SystemMetrics | null
  monitoringFramesByInstance: Record<string, MonitoringFrame[]>
  monitoringCurrentByInstance: Record<string, MonitoringFrame>
  runningTasksByInstance: Record<string, RunningInferenceTask[]>
  lastCompletedTaskByInstance: Record<string, RunningInferenceTask | null>
  setActiveTab: (tab: string) => void
  setDarkMode: (dm: boolean) => void
  setActiveConfigInstanceId: (id: string | null) => void
  setSysMetrics: (m: SystemMetrics | null) => void
  addRuntimeWarning: (message: string) => void
  clearRuntimeWarnings: () => void
  ingestMonitoringFrame: (frame: MonitoringFrame) => void
  hydrateMonitoringFrames: (frames: MonitoringFrame[]) => void
  applyPerfUpdate: (event: PerfUpdateEvent) => void

  setModels: (models: ModelInfo[]) => void
  setEngines: (engines: EngineInfo[]) => void
  setModelDirs: (dirs: string[]) => void
  setEngineDirs: (dirs: string[]) => void
  setDefaultEngineId: (id: string | null) => void
  addInstance: (instance: Instance) => void
  updateInstance: (id: string, partial: Partial<Instance>) => void
  deleteInstance: (id: string) => void
  moveInstance: (id: string, direction: 'up' | 'down') => void
  renameInstance: (id: string, name: string) => void
  addLog: (entry: LogEntry) => void
  addLogs: (entries: LogEntry[]) => void
  clearLogs: (instanceId: string) => void

  loadInitialData: () => Promise<void>
  scanModels: (paths: string[]) => Promise<string | null>
  deleteModelFile: (path: string) => Promise<void>
  openModelFolder: (path: string) => Promise<void>
  readGgufMetadata: (path: string) => Promise<GgufMetadataSummary>

  scanEngines: (paths: string[]) => Promise<void>
  probeEngineCapabilities: (id: string) => Promise<EngineInfo>
  deleteEngine: (id: string) => Promise<void>
  renameEngine: (id: string, name: string) => void
  openEngineFolder: (dir: string) => Promise<void>

  generateCommand: (config: InstanceConfig, engineExe: string) => Promise<string[]>
  startInstance: (id: string) => Promise<void>
  stopInstance: (id: string) => Promise<void>
  openBrowser: (host: string, port: number) => Promise<void>

  saveConfig: () => Promise<void>
  loadConfig: () => Promise<void>

  browseModelscope: (repoId: string) => Promise<MsFileEntry[]>
  downloadModelscopeFiles: (repoId: string, files: MsFileEntry[], saveDir: string) => Promise<void>
  browseHuggingface: (repoId: string) => Promise<MsFileEntry[]>
  downloadHuggingfaceFiles: (repoId: string, files: MsFileEntry[], saveDir: string) => Promise<void>
  cancelFileDownload: (taskId: string, runId?: string) => Promise<void>
  pauseFileDownload: (taskId: string, runId?: string) => Promise<void>
  cancelAndCleanupDownload: (taskId: string, fileName: string, filePath: string, runId?: string, version?: number) => Promise<void>
  resumeDownloadTask: (taskId: string) => Promise<void>
  setDownloadTasks: (tasks: Record<string, DownloadProgress>) => void
  addToDownloadQueue: (entry: { repoId: string; source: 'modelscope' | 'huggingface'; files: MsFileEntry[]; saveDir: string }) => Promise<boolean>
  removeFromDownloadQueue: (id: string) => Promise<void>
  processDownloadQueue: () => Promise<void>
  restoreDownloadQueue: (entries: PersistedQueueEntry[]) => void
  persistQueue: () => Promise<void>
  resumeAllDownloads: () => Promise<void>
  pauseAllDownloads: () => Promise<void>
  cancelAllDownloads: () => Promise<void>
  clearCompletedDownloadTasks: () => Promise<void>
  clearFailedDownloadTasks: () => Promise<void>
  retryFailedDownload: (taskId: string) => void
  redownloadFile: (taskId: string) => Promise<void>
  moveQueueEntry: (id: string, direction: 'up' | 'down') => void
  // Cluster
  setWorkers: (workers: WorkerInfo[]) => void
  addWorker: (worker: WorkerInfo) => void
  removeWorker: (id: string) => void
  updateWorker: (id: string, partial: Partial<WorkerInfo>) => void
  setClusterScanning: (scanning: boolean) => void
}
