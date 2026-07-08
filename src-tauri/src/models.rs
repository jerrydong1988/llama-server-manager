use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

// ── 模型信息 ──────────────────────────────────────────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub size: u64,
    pub architecture: Option<String>,
    pub context_length: Option<u32>,
    pub quant_type: Option<String>,
    pub has_mtp_head: bool,
    pub file_type: String,
    #[serde(default)]
    pub is_shard: bool,
}

// ── 引擎信息 ──────────────────────────────────────────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EngineInfo {
    pub id: String,
    pub name: String,
    pub dir: String,
    pub exe: String,
    pub version: String,
    pub backend: String,
    #[serde(default)]
    pub custom_name: Option<String>,
}

// ── 实例配置 ──────────────────────────────────────────────────────
// #[serde(default)] 容器级别：任何缺失字段都回退到 Default impl，
// 防止旧版/手改配置因缺少单个字段导致全部实例反序列化失败。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct InstanceConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub engine_id: String,
    pub model_path: String,
    pub alias: String,
    pub lora_path: String,
    pub mmproj_path: String,
    #[serde(default)]
    pub lora_init_without_apply: bool,
    #[serde(default)]
    pub lora_scaled: String,
    pub chat_template: String,
    #[serde(default)]
    pub chat_template_file: String,
    #[serde(default)]
    pub skip_chat_parsing: bool,
    pub reasoning_format: String,
    pub reasoning_effort: String,
    pub reasoning: String,
    pub jinja: bool,
    pub reasoning_budget: String,
    #[serde(default)]
    pub reasoning_budget_message: String,
    pub grammar_file: String,
    #[serde(default)]
    pub grammar: String,
    pub ctx_size: u32,
    pub ctx_size_auto: bool,
    #[serde(default = "default_true")]
    pub gpu_layers_auto: bool,
    pub gpu_layers: u32,
    pub threads: u32,
    pub batch_size: u32,
    pub ubatch_size: u32,
    pub parallel: i32,
    pub cont_batching: bool,
    pub cache_prompt: bool,
    pub threads_batch: u32,
    #[serde(default)]
    pub threads_http: i32,
    #[serde(default)]
    pub keep: u32,
    #[serde(default)]
    pub cache_reuse: u32,
    #[serde(default)]
    pub cache_ram: i32,
    #[serde(default)]
    pub warmup: bool,
    #[serde(default)]
    pub ctx_checkpoints: u32,
    #[serde(default)]
    pub checkpoint_min_step: u32,
    #[serde(default)]
    pub swa_full: bool,
    // RoPE / YaRN
    #[serde(default)]
    pub rope_scaling: String,
    #[serde(default)]
    pub rope_scale: f32,
    #[serde(default)]
    pub rope_freq_base: f32,
    #[serde(default)]
    pub rope_freq_scale: f32,
    #[serde(default)]
    pub yarn_ext_factor: f32,
    #[serde(default)]
    pub yarn_attn_factor: f32,
    #[serde(default)]
    pub yarn_beta_slow: f32,
    #[serde(default)]
    pub yarn_beta_fast: f32,
    #[serde(default)]
    pub yarn_orig_ctx: u32,
    pub flash_attn: String,
    pub moe_cpu_layers: u32,
    #[serde(default)]
    pub cpu_moe: bool,
    pub mlock: bool,
    pub no_mmap: bool,
    #[serde(default)]
    pub no_repack: bool,
    #[serde(default)]
    pub direct_io: bool,
    pub numa: bool,
    pub context_shift: bool,
    #[serde(default)]
    pub check_tensors: bool,
    #[serde(default)]
    pub perf: bool,
    #[serde(default)]
    pub fit: bool,
    #[serde(default)]
    pub fit_target: String,
    #[serde(default = "default_fit_ctx")]
    pub fit_ctx: u32,
    #[serde(default)]
    pub kv_unified: bool,
    #[serde(default)]
    pub cache_idle_slots: bool,
    #[serde(default)]
    pub no_kv_offload: bool,
    pub cache_type_k: String,
    pub cache_type_v: String,
    #[serde(default)]
    pub cache_type_draft_k: String,
    #[serde(default)]
    pub cache_type_draft_v: String,
    pub draft_model_path: String,
    pub draft_gpu_layers: u32,
    pub draft_tokens: u32,
    pub spec_draft_n_min: u32,
    pub spec_type: String,
    #[serde(default)]
    pub spec_draft_p_min: f32,
    #[serde(default)]
    pub spec_draft_p_split: f32,
    #[serde(default)]
    pub spec_draft_device: String,
    #[serde(default)]
    pub lookup_cache_static: String,
    #[serde(default)]
    pub lookup_cache_dynamic: String,
    #[serde(default)]
    pub spec_default: bool,
    #[serde(default = "default_true")]
    pub spec_draft_backend_sampling: bool,
    #[serde(default)]
    pub spec_draft_threads: u32,
    #[serde(default)]
    pub spec_draft_threads_batch: u32,
    // GPU & Device
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub split_mode: String,
    #[serde(default)]
    pub tensor_split: String,
    #[serde(default)]
    pub main_gpu: u32,
    #[serde(default)]
    pub override_kv: String,
    pub host: String,
    pub port: u16,
    pub api_key: String,
    #[serde(default)]
    pub api_key_file: String,
    pub ssl_key_file: String,
    pub ssl_cert_file: String,
    #[serde(default)]
    pub path_prefix: String,
    #[serde(default)]
    pub api_prefix: String,
    pub no_ui: bool,
    #[serde(default)]
    pub offline: bool,
    #[serde(default)]
    pub ui_config_file: String,
    #[serde(default)]
    pub ui_config: String,
    #[serde(default)]
    pub ui_mcp_proxy: bool,
    #[serde(default)]
    pub agent: bool,
    pub embedding: bool,
    pub pooling: String,
    #[serde(default)]
    pub embd_normalize: u32,
    pub reranking: bool,
    #[serde(default)]
    pub metrics: bool,
    #[serde(default)]
    pub props: bool,
    #[serde(default)]
    pub slots_enabled: bool,
    #[serde(default)]
    pub slot_save_path: String,
    #[serde(default)]
    pub slot_prompt_similarity: f32,
    pub prefill_assistant: bool,
    // Multi-Model & Media
    #[serde(default)]
    pub models_dir: String,
    #[serde(default)]
    pub models_preset: String,
    #[serde(default)]
    pub models_max: u32,
    #[serde(default)]
    pub models_autoload: bool,
    #[serde(default)]
    pub mmproj_url: String,
    #[serde(default)]
    pub mmproj_auto: bool,
    #[serde(default)]
    pub no_mmproj: bool,
    #[serde(default)]
    pub no_mmproj_offload: bool,
    #[serde(default)]
    pub image_min_tokens: u32,
    #[serde(default)]
    pub image_max_tokens: u32,
    #[serde(default)]
    pub mtmd_batch_max_tokens: u32,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub media_path: String,
    #[serde(default)]
    pub tools: String,
    pub n_predict: i32,
    pub ignore_eos: bool,
    pub json_schema: String,
    #[serde(default)]
    pub json_schema_file: String,
    pub temp: f32,
    pub top_k: u32,
    pub top_p: f32,
    pub repeat_penalty: f32,
    pub seed: i64,
    pub min_p: f32,
    pub presence_penalty: f32,
    pub frequency_penalty: f32,
    pub repeat_last_n: i32,
    #[serde(default)]
    pub reverse_prompt: String,
    #[serde(default)]
    pub special: bool,
    #[serde(default)]
    pub spm_infill: bool,
    #[serde(default)]
    pub backend_sampling: bool,
    pub mirostat: u8,
    pub mirostat_lr: f32,
    pub mirostat_ent: f32,
    pub xtc_probability: f32,
    pub xtc_threshold: f32,
    pub dynatemp_range: f32,
    pub dynatemp_exp: f32,
    pub typical_p: f32,
    pub dry_multiplier: f32,
    pub dry_base: f32,
    pub dry_allowed_length: u32,
    pub dry_penalty_last_n: i32,
    pub dry_sequence_breaker: String,
    #[serde(default)]
    pub adaptive_target: f32,
    #[serde(default)]
    pub adaptive_decay: f32,
    #[serde(default)]
    pub top_n_sigma: f32,
    #[serde(default)]
    pub logit_bias: String,
    #[serde(default)]
    pub samplers: String,
    #[serde(default)]
    pub sampler_seq: String,
    pub timeout: u32,
    pub sleep_idle: i32,
    pub verbose: bool,
    pub custom_args: Vec<String>,
    // Server features (aligned with llama.cpp master)
    #[serde(default)]
    pub rpc_servers: String,
    #[serde(default)]
    pub sse_ping_interval: u32,
    #[serde(default)]
    pub reuse_port: bool,
    #[serde(default)]
    pub auto_start: bool,
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            engine_id: String::new(),
            model_path: String::new(),
            alias: String::new(),
            lora_path: String::new(),
            mmproj_path: String::new(),
            lora_init_without_apply: false,
            lora_scaled: String::new(),
            chat_template: String::new(),
            chat_template_file: String::new(),
            skip_chat_parsing: false,
            reasoning_format: String::new(),
            reasoning_effort: String::new(),
            reasoning: String::new(),
            jinja: true,
            reasoning_budget: String::new(),
            reasoning_budget_message: String::new(),
            grammar_file: String::new(),
            grammar: String::new(),
            ctx_size: 0,
            ctx_size_auto: false,
            gpu_layers_auto: true,
            gpu_layers: 99,
            threads: 0,
            batch_size: 2048,
            ubatch_size: 512,
            parallel: -1,
            cont_batching: true,
            cache_prompt: true,
            threads_batch: 0,
            threads_http: -1,
            keep: 0,
            cache_reuse: 0,
            cache_ram: 8192,
            warmup: true,
            ctx_checkpoints: 32,
            checkpoint_min_step: 256,
            swa_full: false,
            rope_scaling: String::new(),
            rope_scale: 0.0,
            rope_freq_base: 0.0,
            rope_freq_scale: 0.0,
            yarn_ext_factor: -1.0,
            yarn_attn_factor: -1.0,
            yarn_beta_slow: -1.0,
            yarn_beta_fast: -1.0,
            yarn_orig_ctx: 0,
            flash_attn: "auto".into(),
            moe_cpu_layers: 0,
            cpu_moe: false,
            mlock: false,
            no_mmap: false,
            no_repack: false,
            direct_io: false,
            numa: false,
            context_shift: false,
            perf: false,
            check_tensors: false,
            fit: false,
            fit_target: String::new(),
            fit_ctx: 4096,
            kv_unified: false,
            cache_idle_slots: true,
            no_kv_offload: false,
            cache_type_k: String::new(),
            cache_type_v: String::new(),
            cache_type_draft_k: String::new(),
            cache_type_draft_v: String::new(),
            draft_model_path: String::new(),
            draft_gpu_layers: 99,
            draft_tokens: 3,
            spec_draft_n_min: 0,
            spec_type: String::new(),
            spec_draft_p_min: 0.0,
            spec_draft_p_split: 0.1,
            spec_draft_device: String::new(),
            lookup_cache_static: String::new(),
            lookup_cache_dynamic: String::new(),
            spec_default: false,
            spec_draft_backend_sampling: true,
            spec_draft_threads: 0,
            spec_draft_threads_batch: 0,
            device: String::new(),
            split_mode: String::new(),
            tensor_split: String::new(),
            main_gpu: 0,
            override_kv: String::new(),
            host: "127.0.0.1".into(),
            port: 8080,
            api_key: String::new(),
            api_key_file: String::new(),
            ssl_key_file: String::new(),
            ssl_cert_file: String::new(),
            path_prefix: String::new(),
            api_prefix: String::new(),
            no_ui: false,
            offline: false,
            ui_config_file: String::new(),
            ui_config: String::new(),
            ui_mcp_proxy: false,
            agent: false,
            embedding: false,
            pooling: String::new(),
            embd_normalize: 2,
            reranking: false,
            metrics: true,
            props: false,
            slots_enabled: true,
            slot_save_path: String::new(),
            slot_prompt_similarity: 0.1,
            prefill_assistant: true,
            models_dir: String::new(),
            models_preset: String::new(),
            models_max: 4,
            models_autoload: true,
            mmproj_url: String::new(),
            mmproj_auto: false,
            no_mmproj: false,
            no_mmproj_offload: false,
            image_min_tokens: 0,
            image_max_tokens: 0,
            mtmd_batch_max_tokens: 1024,
            tags: String::new(),
            media_path: String::new(),
            tools: String::new(),
            n_predict: -1,
            ignore_eos: false,
            json_schema: String::new(),
            json_schema_file: String::new(),
            temp: 0.8,
            top_k: 40,
            top_p: 0.95,
            repeat_penalty: 1.0,
            seed: -1,
            min_p: 0.05,
            presence_penalty: 0.0,
            frequency_penalty: 0.0,
            repeat_last_n: 64,
            reverse_prompt: String::new(),
            special: false,
            spm_infill: false,
            backend_sampling: false,
            mirostat: 0,
            mirostat_lr: 0.10,
            mirostat_ent: 5.0,
            xtc_probability: 0.0,
            xtc_threshold: 0.10,
            dynatemp_range: 0.0,
            dynatemp_exp: 1.0,
            typical_p: 1.0,
            dry_multiplier: 0.0,
            dry_base: 1.75,
            dry_allowed_length: 2,
            dry_penalty_last_n: -1,
            dry_sequence_breaker: String::new(),
            adaptive_target: -1.0,
            adaptive_decay: 0.90,
            top_n_sigma: -1.0,
            logit_bias: String::new(),
            samplers: String::new(),
            sampler_seq: String::new(),
            timeout: 3600,
            sleep_idle: -1,
            verbose: false,
            custom_args: vec![],
            rpc_servers: String::new(),
            sse_ping_interval: 30,
            reuse_port: false,
            auto_start: false,
        }
    }
}

// ── 运行中实例 ────────────────────────────────────────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunningInstance {
    pub instance_id: String,
    pub pid: u32,
    pub port: u16,
    pub host: String,
    #[serde(default)]
    pub start_time: u64,
    #[serde(default)]
    pub telemetry_session_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SystemMetrics {
    pub cpu_percent: f32,
    pub memory_mb: f64,
    pub uptime_secs: u64,
    pub gpu_percent: Option<f32>,
    pub vram_used_mb: Option<f64>,
    pub vram_total_mb: Option<f64>,
    /// System-wide CPU usage (all cores, 0-100%)
    pub system_cpu_percent: Option<f32>,
    /// System-wide total physical RAM in MB
    pub system_memory_total_mb: Option<f64>,
    /// System-wide used RAM in MB
    pub system_memory_used_mb: Option<f64>,
    /// GPU vendor: "AMD" | "NVIDIA" | null (unknown/not detected)
    pub gpu_vendor: Option<String>,
}

// ── 集群管理 / Worker ─────────────────────────────────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkerDevice {
    pub device_type: String, // CUDA, ROCm, Metal, Vulkan, CPU
    pub name: String,
    pub vram_mb: u64,
    pub free_mb: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum WorkerStatus {
    Online,
    Offline,
    Testing,
    Unknown,
}

impl Default for WorkerStatus {
    fn default() -> Self {
        WorkerStatus::Unknown
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkerInfo {
    pub id: String,
    pub host: String,
    pub port: u16,
    pub name: String,
    #[serde(default)]
    pub devices: Vec<WorkerDevice>,
    #[serde(default)]
    pub status: WorkerStatus,
    #[serde(default)]
    pub last_seen: Option<String>,
    #[serde(default)]
    pub auto_discovered: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Usb4Adapter {
    pub name: String,
    pub if_index: u32,
    pub description: String,
    pub status: String,
    pub ip: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProxyRoute {
    pub id: String,
    pub enabled: bool,
    pub model_alias: String,
    pub target_instance_id: String,
    pub priority: i32,
}

impl Default for ProxyRoute {
    fn default() -> Self {
        Self {
            id: String::new(),
            enabled: true,
            model_alias: String::new(),
            target_instance_id: String::new(),
            priority: 0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProxyConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub public_api_key: String,
    pub default_instance_id: String,
    pub routes: Vec<ProxyRoute>,
    pub routing_strategy: String,
    pub timeout_ms: u64,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".into(),
            port: 11435,
            public_api_key: String::new(),
            default_instance_id: String::new(),
            routes: Vec::new(),
            routing_strategy: "firstHealthy".into(),
            timeout_ms: 600_000,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProxyStatus {
    pub running: bool,
    pub bound_addr: String,
    pub active_routes: usize,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProxyTarget {
    pub instance_id: String,
    pub name: String,
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub running: bool,
}

// ── 应用全局状态 ──────────────────────────────────────────────────
pub struct AppState {
    pub models: Mutex<Vec<ModelInfo>>,
    pub engines: Mutex<Vec<EngineInfo>>,
    pub engine_names: Mutex<HashMap<String, String>>,
    pub instances: Mutex<HashMap<String, InstanceConfig>>,
    pub running: Mutex<HashMap<String, RunningInstance>>,
    pub config_dir: Mutex<PathBuf>,
    pub cancel_flags: Mutex<HashMap<String, bool>>,
    pub pause_flags: Mutex<HashMap<String, bool>>,
    pub active_downloads: Mutex<std::collections::HashSet<String>>,
    pub download_queue: Mutex<Vec<PersistedQueueEntry>>,
    pub download_active_batches: Mutex<std::collections::HashSet<String>>,
    pub download_active_entries: Mutex<HashMap<String, PersistedQueueEntry>>,
    pub download_max_concurrent: Mutex<usize>,
    pub download_bandwidth_limit_bytes_per_sec: Mutex<u64>,
    pub download_low_priority_throttle: Mutex<bool>,
    pub download_bandwidth_limiter: Mutex<DownloadBandwidthLimiter>,
    pub workers: Mutex<Vec<WorkerInfo>>,
    pub usb4_adapters: Mutex<Vec<Usb4Adapter>>,
    pub proxy_config: Mutex<ProxyConfig>,
    pub proxy_shutdown: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    pub proxy_bound_addr: Mutex<Option<String>>,
    pub proxy_last_error: Mutex<Option<String>>,
    pub restored_runtime_instances: Mutex<std::collections::HashSet<String>>,
}

pub struct DownloadBandwidthLimiter {
    pub available_bytes: f64,
    pub last_refill: Instant,
}

impl Default for DownloadBandwidthLimiter {
    fn default() -> Self {
        Self {
            available_bytes: 0.0,
            last_refill: Instant::now(),
        }
    }
}

// ── 全局配置结构（用于 JSON 序列化） ──────────────────────────────
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct GlobalConfig {
    pub instances: HashMap<String, InstanceConfig>,
    pub model_dirs: Vec<String>,
    pub engine_dirs: Vec<String>,
    pub default_engine_id: String,
    pub running: HashMap<String, RunningInstance>,
    pub instance_order: Vec<String>,
    #[serde(default)]
    pub last_tab: String,
    #[serde(default)]
    pub dark_mode: bool,
    #[serde(default)]
    pub engine_names: HashMap<String, String>,
    #[serde(default = "default_download_resume_policy")]
    pub download_resume_policy: String,
    #[serde(default = "default_download_max_concurrent")]
    pub download_max_concurrent: usize,
    #[serde(default)]
    pub download_bandwidth_limit_bytes_per_sec: u64,
    #[serde(default)]
    pub download_low_priority_throttle: bool,
    #[serde(default)]
    pub proxy_config: ProxyConfig,
}

// ── 窗口状态 ─────────────────────────────────────────────────────
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

// ── 下载队列持久化 ─────────────────────────────────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersistedQueueEntry {
    pub id: String,
    pub repo_id: String,
    pub source: String,
    pub files: Vec<MsFileEntry>,
    pub save_dir: String,
    pub added_at: u64,
    #[serde(default)]
    pub status: String, // "queued" | "active" | "paused"
    #[serde(default)]
    pub retries: u32,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct DownloadState {
    pub queue: Vec<PersistedQueueEntry>,
}

// ── ModelScope 文件信息 ───────────────────────────────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MsFileEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub file_type: String,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub downloaded: Option<u64>,
    #[serde(default)]
    pub version: Option<u32>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

// ── 下载工件状态 ──────────────────────────────────────────────────
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct DownloadArtifactState {
    pub task_id: String,
    pub run_id: String,
    pub repo_id: String,
    pub source: String,
    pub remote_path: String,
    pub final_path: String,
    pub temp_path: String,
    pub expected_size: u64,
    pub downloaded_size: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub updated_at: u64,
}

fn default_true() -> bool {
    true
}
fn default_fit_ctx() -> u32 {
    4096
}
fn default_max_retries() -> u32 {
    3
}
fn default_download_resume_policy() -> String {
    "manual".into()
}
fn default_download_max_concurrent() -> usize {
    1
}
