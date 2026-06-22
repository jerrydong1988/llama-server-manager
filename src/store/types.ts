export interface ModelInfo {
  id: string
  name: string
  path: string
  size: number
  architecture?: string
  context_length?: number
  quant_type?: string
  has_mtp_head?: boolean
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
}

export interface InstanceConfig {
  // Basic
  id: string; name: string; engine_id: string; model_path: string; alias: string;
  lora_path: string; mmproj_path: string; lora_init_without_apply: boolean; lora_scaled: string;
  chat_template: string; chat_template_file: string; skip_chat_parsing: boolean;
  reasoning_format: string; reasoning_effort: string; reasoning: string;
  jinja: boolean; reasoning_budget: string; reasoning_budget_message: string;
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
  flash_attn: string; moe_cpu_layers: number; mlock: boolean;
  no_mmap: boolean; no_repack: boolean; numa: boolean; context_shift: boolean;
  perf: boolean; check_tensors: boolean; fit: boolean; fit_target: string; fit_ctx: number; kv_unified: boolean; cache_idle_slots: boolean; no_kv_offload: boolean;
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
  no_ui: boolean; offline: boolean; ui_config_file: string; ui_config: string; ui_mcp_proxy: boolean;
  embedding: boolean; pooling: string; embd_normalize: number; reranking: boolean;
  metrics: boolean; props: boolean; slots_enabled: boolean;
  slot_save_path: string; slot_prompt_similarity: number; prefill_assistant: boolean;
  // Multi-Model & Media
  models_dir: string; models_preset: string; models_max: number; models_autoload: boolean;
  mmproj_url: string; mmproj_auto: boolean; no_mmproj: boolean; no_mmproj_offload: boolean; image_min_tokens: number; image_max_tokens: number;
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
  fileName: string
  downloaded: number
  total: number
  speed: number
  repoId: string
  source: string
  status: 'active' | 'completed' | 'cancelled' | 'error' | 'paused' | 'queued'
  path?: string
  error?: string
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
}

export interface AppState {
  models: ModelInfo[]
  engines: EngineInfo[]
  instances: Instance[]
  logs: Record<string, LogEntry[]>
  isLoading: boolean
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
  setActiveTab: (tab: string) => void
  setDarkMode: (dm: boolean) => void
  setActiveConfigInstanceId: (id: string | null) => void

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
  clearLogs: (instanceId: string) => void

  loadInitialData: () => Promise<void>
  scanModels: (paths: string[]) => Promise<string | null>
  deleteModelFile: (path: string) => Promise<void>
  openModelFolder: (path: string) => Promise<void>
  readGgufMetadata: (path: string) => Promise<[string | null, number | null, string | null]>

  scanEngines: (paths: string[]) => Promise<void>
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
  cancelFileDownload: (fileName: string) => Promise<void>
  pauseFileDownload: (fileName: string) => Promise<void>
  cancelAndCleanupDownload: (fileName: string, filePath: string) => Promise<void>
  setDownloadTasks: (tasks: Record<string, DownloadProgress>) => void
  addToDownloadQueue: (entry: { repoId: string; source: 'modelscope' | 'huggingface'; files: MsFileEntry[]; saveDir: string }) => void
  removeFromDownloadQueue: (id: string) => void
  processDownloadQueue: () => void
  restoreDownloadQueue: (entries: PersistedQueueEntry[]) => void
  persistQueue: () => void
  // Cluster
  setWorkers: (workers: WorkerInfo[]) => void
  addWorker: (worker: WorkerInfo) => void
  removeWorker: (id: string) => void
  updateWorker: (id: string, partial: Partial<WorkerInfo>) => void
  setClusterScanning: (scanning: boolean) => void
}
