use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::{Arc, Mutex};
use std::time::Instant;

// Compact GGUF capability summary used by UI validation. Keep field names stable
// because cached scan results and TypeScript models deserialize this structure.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct ModelCapabilities {
    #[serde(default)]
    pub metadata_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_embedding_model: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_reranker_model: Option<bool>,
    #[serde(default)]
    pub has_builtin_mtp: bool,
    #[serde(default)]
    pub mtp_layers: Option<u32>,
    #[serde(default)]
    pub is_vision_model: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision_status: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vision_evidence: Vec<String>,
    #[serde(default)]
    pub vision_family: Option<String>,
    #[serde(default)]
    pub is_mmproj: bool,
    #[serde(default)]
    pub projector_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projector_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_basename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_model_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_model_repo: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[cfg(test)]
mod model_capability_tests {
    use super::{
        ensure_managed_public_model_alias, public_model_id, InstanceConfig, ModelCapabilities,
    };

    #[test]
    fn vector_capabilities_distinguish_missing_cache_fields_from_explicit_false() {
        let cached: ModelCapabilities =
            serde_json::from_str(r#"{"metadata_complete":true}"#).unwrap();
        assert_eq!(cached.is_embedding_model, None);
        assert_eq!(cached.is_reranker_model, None);
        assert_eq!(cached.vision_status, None);
        assert!(cached.vision_evidence.is_empty());
        assert!(cached.tags.is_empty());
        let cached_json = serde_json::to_value(&cached).unwrap();
        assert!(cached_json.get("is_embedding_model").is_none());
        assert!(cached_json.get("is_reranker_model").is_none());
        assert!(cached_json.get("vision_status").is_none());
        assert!(cached_json.get("vision_evidence").is_none());
        assert!(cached_json.get("tags").is_none());

        let scanned: ModelCapabilities = serde_json::from_str(
            r#"{"metadata_complete":true,"is_embedding_model":false,"is_reranker_model":false}"#,
        )
        .unwrap();
        assert_eq!(scanned.is_embedding_model, Some(false));
        assert_eq!(scanned.is_reranker_model, Some(false));
        let scanned_json = serde_json::to_value(&scanned).unwrap();
        assert_eq!(scanned_json["is_embedding_model"], false);
        assert_eq!(scanned_json["is_reranker_model"], false);
    }

    #[test]
    fn legacy_instance_config_uses_business_defaults_for_missing_fields() {
        let config: InstanceConfig = serde_json::from_str("{}").unwrap();
        let expected = InstanceConfig::default();
        assert_eq!(config.threads_http, expected.threads_http);
        assert_eq!(config.cache_ram, expected.cache_ram);
        assert_eq!(config.warmup, expected.warmup);
        assert_eq!(config.ctx_checkpoints, expected.ctx_checkpoints);
        assert_eq!(config.checkpoint_min_step, expected.checkpoint_min_step);
        assert_eq!(config.yarn_ext_factor, expected.yarn_ext_factor);
        assert_eq!(config.yarn_attn_factor, expected.yarn_attn_factor);
        assert_eq!(config.yarn_beta_slow, expected.yarn_beta_slow);
        assert_eq!(config.yarn_beta_fast, expected.yarn_beta_fast);
        assert_eq!(config.cache_idle_slots, expected.cache_idle_slots);
        assert_eq!(config.spec_draft_p_split, expected.spec_draft_p_split);
        assert_eq!(config.embd_normalize, expected.embd_normalize);
        assert_eq!(config.metrics, expected.metrics);
        assert_eq!(config.props, expected.props);
        assert_eq!(config.slots_enabled, expected.slots_enabled);
        assert_eq!(
            config.slot_prompt_similarity,
            expected.slot_prompt_similarity
        );
        assert_eq!(config.models_max, expected.models_max);
        assert_eq!(config.models_autoload, expected.models_autoload);
        assert_eq!(config.mtmd_batch_max_tokens, expected.mtmd_batch_max_tokens);
        assert_eq!(config.adaptive_target, expected.adaptive_target);
        assert_eq!(config.adaptive_decay, expected.adaptive_decay);
        assert_eq!(config.top_n_sigma, expected.top_n_sigma);
        assert_eq!(config.sse_ping_interval, expected.sse_ping_interval);
    }

    #[test]
    fn public_model_id_never_falls_back_to_a_filesystem_path() {
        let mut config = InstanceConfig {
            model_path: r"C:\Users\Jerry\models\Qwen3.6-27B-Q6_K.gguf".into(),
            ..InstanceConfig::default()
        };
        assert_eq!(public_model_id(&config), "Qwen3.6-27B-Q6_K");

        config.name = "Friendly model".into();
        assert_eq!(public_model_id(&config), "Friendly model");

        config.alias = "public-alias".into();
        assert_eq!(public_model_id(&config), "public-alias");
    }

    #[test]
    fn managed_alias_is_visible_and_emitted_but_manual_commands_are_untouched() {
        let mut managed = InstanceConfig {
            name: "Public model".into(),
            explicit_overrides: Some(Vec::new()),
            ..InstanceConfig::default()
        };
        assert!(ensure_managed_public_model_alias(&mut managed));
        assert_eq!(managed.alias, "Public model");
        assert_eq!(managed.explicit_overrides, Some(vec!["alias".into()]));

        let mut manual = InstanceConfig {
            launch_mode: "manual".into(),
            name: "Manual model".into(),
            explicit_overrides: Some(Vec::new()),
            ..InstanceConfig::default()
        };
        assert!(!ensure_managed_public_model_alias(&mut manual));
        assert!(manual.alias.is_empty());
        assert_eq!(manual.explicit_overrides, Some(Vec::new()));
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct GgufMetadataSummary {
    pub architecture: Option<String>,
    pub context_length: Option<u32>,
    pub quant_type: Option<String>,
    pub capabilities: ModelCapabilities,
}

// Model information.
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
    #[serde(default)]
    pub capabilities: ModelCapabilities,
    pub file_type: String,
    #[serde(default)]
    pub is_shard: bool,
}

// Engine information.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineCapabilities {
    #[serde(default = "default_engine_capability_status")]
    pub status: String,
    #[serde(default = "default_engine_version_status")]
    pub version_status: String,
    #[serde(default)]
    pub version_probe_detail: Option<String>,
    #[serde(default)]
    pub supported_flags: Vec<String>,
    #[serde(default)]
    pub help_hash: String,
    #[serde(default)]
    pub executable_fingerprint: String,
    #[serde(default)]
    pub probed_at: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

fn default_engine_capability_status() -> String {
    "unprobed".to_string()
}

fn default_engine_version_status() -> String {
    "unprobed".to_string()
}

impl Default for EngineCapabilities {
    fn default() -> Self {
        Self {
            status: default_engine_capability_status(),
            version_status: default_engine_version_status(),
            version_probe_detail: None,
            supported_flags: Vec::new(),
            help_hash: String::new(),
            executable_fingerprint: String::new(),
            probed_at: None,
            error: None,
        }
    }
}

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
    #[serde(default)]
    pub capabilities: EngineCapabilities,
}

// Instance config.
// Container-level #[serde(default)]: missing fields fall back to Default.
// Prevent older or hand-edited configs from failing all instance deserialization because one field is missing.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct InstanceConfig {
    /// Managed mode builds argv from structured fields. Manual mode launches
    /// the selected executable with the exact argv supplied by the user.
    #[serde(default = "default_launch_mode")]
    pub launch_mode: String,
    #[serde(default)]
    pub manual_command: String,
    /// None means a legacy configuration whose emission intent is unknown.
    /// Some (including an empty vector) enables intent-based emission.
    #[serde(default)]
    pub explicit_overrides: Option<Vec<String>>,
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
    pub reasoning_preserve: String,
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
    #[serde(default = "default_negative_one_i32")]
    pub threads_http: i32,
    #[serde(default)]
    pub keep: i32,
    #[serde(default)]
    pub cache_reuse: u32,
    #[serde(default = "default_cache_ram")]
    pub cache_ram: i32,
    #[serde(default = "default_true")]
    pub warmup: bool,
    #[serde(default = "default_ctx_checkpoints")]
    pub ctx_checkpoints: u32,
    #[serde(default = "default_checkpoint_min_step")]
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
    #[serde(default = "default_negative_one_f32")]
    pub yarn_ext_factor: f32,
    #[serde(default = "default_negative_one_f32")]
    pub yarn_attn_factor: f32,
    #[serde(default = "default_negative_one_f32")]
    pub yarn_beta_slow: f32,
    #[serde(default = "default_negative_one_f32")]
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
    pub numa_mode: String,
    pub context_shift: bool,
    #[serde(default)]
    pub check_tensors: bool,
    #[serde(default)]
    pub perf: bool,
    #[serde(default)]
    pub fit: bool,
    pub fit_mode: String,
    #[serde(default)]
    pub fit_target: String,
    #[serde(default = "default_fit_ctx")]
    pub fit_ctx: u32,
    #[serde(default)]
    pub kv_unified: bool,
    pub kv_unified_mode: String,
    #[serde(default = "default_true")]
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
    #[serde(default = "default_point_one")]
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
    pub cors_origins: String,
    pub cors_methods: String,
    pub cors_headers: String,
    pub cors_credentials: String,
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
    #[serde(default = "default_embd_normalize")]
    pub embd_normalize: i32,
    pub reranking: bool,
    #[serde(default = "default_true")]
    pub metrics: bool,
    #[serde(default = "default_true")]
    pub props: bool,
    #[serde(default = "default_true")]
    pub slots_enabled: bool,
    #[serde(default)]
    pub slot_save_path: String,
    pub log_prompts_dir: String,
    #[serde(default = "default_point_one")]
    pub slot_prompt_similarity: f32,
    pub prefill_assistant: bool,
    // Multi-Model & Media
    #[serde(default)]
    pub models_dir: String,
    #[serde(default)]
    pub models_preset: String,
    #[serde(default = "default_models_max")]
    pub models_max: u32,
    #[serde(default = "default_true")]
    pub models_autoload: bool,
    #[serde(default)]
    pub mmproj_url: String,
    #[serde(default)]
    pub mmproj_auto: bool,
    pub mmproj_mode: String,
    #[serde(default)]
    pub no_mmproj: bool,
    #[serde(default)]
    pub no_mmproj_offload: bool,
    #[serde(default)]
    pub image_min_tokens: u32,
    #[serde(default)]
    pub image_max_tokens: u32,
    #[serde(default = "default_mtmd_batch_max_tokens")]
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
    #[serde(default = "default_negative_one_f32")]
    pub adaptive_target: f32,
    #[serde(default = "default_adaptive_decay")]
    pub adaptive_decay: f32,
    #[serde(default = "default_negative_one_f32")]
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
    #[serde(default = "default_sse_ping_interval")]
    pub sse_ping_interval: i32,
    #[serde(default)]
    pub reuse_port: bool,
    #[serde(default)]
    pub auto_start: bool,
}

fn model_file_stem(value: &str) -> Option<String> {
    let file_name = value
        .trim()
        .rsplit(['/', '\\'])
        .find(|component| !component.trim().is_empty())?
        .trim();
    if file_name.is_empty() {
        return None;
    }
    let stem = file_name
        .to_ascii_lowercase()
        .strip_suffix(".gguf")
        .map(|_| &file_name[..file_name.len() - ".gguf".len()])
        .unwrap_or(file_name)
        .trim();
    (!stem.is_empty()).then(|| stem.to_string())
}

/// Stable model identifier exposed to API clients. Internal instance UUIDs and
/// filesystem paths are deliberately excluded from the fallback chain.
pub(crate) fn public_model_id(config: &InstanceConfig) -> String {
    let alias = config.alias.trim();
    if !alias.is_empty() {
        return alias.to_string();
    }

    let name = config.name.trim();
    if !name.is_empty() {
        if !name.contains(['/', '\\']) {
            return name.to_string();
        }
        if let Some(stem) = model_file_stem(name) {
            return stem;
        }
    }

    model_file_stem(&config.model_path).unwrap_or_else(|| "model".to_string())
}

/// Managed instances always receive a visible llama-server alias so upstream
/// JSON and SSE responses cannot fall back to exposing the model file path.
pub(crate) fn ensure_managed_public_model_alias(config: &mut InstanceConfig) -> bool {
    if config.launch_mode.eq_ignore_ascii_case("manual") {
        return false;
    }

    if config.alias.trim().is_empty()
        && config.name.trim().is_empty()
        && model_file_stem(&config.model_path).is_none()
    {
        return false;
    }

    let mut changed = false;
    let alias = public_model_id(config);
    if config.alias != alias {
        config.alias = alias;
        changed = true;
    }
    if let Some(overrides) = config.explicit_overrides.as_mut() {
        if !overrides.iter().any(|field| field == "alias") {
            overrides.push("alias".to_string());
            changed = true;
        }
    }
    changed
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            launch_mode: default_launch_mode(),
            manual_command: String::new(),
            explicit_overrides: None,
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
            reasoning_preserve: String::new(),
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
            checkpoint_min_step: 8192,
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
            numa_mode: String::new(),
            context_shift: false,
            perf: false,
            check_tensors: false,
            fit: false,
            fit_mode: String::new(),
            fit_target: String::new(),
            fit_ctx: 4096,
            kv_unified: false,
            kv_unified_mode: String::new(),
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
            cors_origins: String::new(),
            cors_methods: String::new(),
            cors_headers: String::new(),
            cors_credentials: String::new(),
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
            props: true,
            slots_enabled: true,
            slot_save_path: String::new(),
            log_prompts_dir: String::new(),
            slot_prompt_similarity: 0.1,
            prefill_assistant: true,
            models_dir: String::new(),
            models_preset: String::new(),
            models_max: 4,
            models_autoload: true,
            mmproj_url: String::new(),
            mmproj_auto: false,
            mmproj_mode: String::new(),
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

// Running instances.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunningInstance {
    pub instance_id: String,
    pub pid: u32,
    pub port: u16,
    pub host: String,
    #[serde(default)]
    pub start_time: u64,
    #[serde(default)]
    pub executable_path: String,
    #[serde(default)]
    pub telemetry_session_id: Option<String>,
    #[serde(default)]
    pub workload: String,
    /// Immutable configuration snapshot used by runtime consumers until restart.
    #[serde(default)]
    pub launch_config: Option<InstanceConfig>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    /// Driver-reported GPU model name, for example "AMD Radeon(TM) 8060S Graphics".
    pub gpu_name: Option<String>,
}

// Cluster management / Worker.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkerDevice {
    pub device_type: String, // CUDA, ROCm, Metal, Vulkan, CPU
    pub name: String,
    pub vram_mb: u64,
    pub free_mb: u64,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum WorkerStatus {
    Online,
    Offline,
    Testing,
    #[default]
    Unknown,
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
    /// Legacy tray keep-alive preference retained for config compatibility.
    pub background_service_mode: bool,
    /// Runs routing and managed instances in the independent per-user runtime.
    pub runtime_service_enabled: bool,
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
            background_service_mode: false,
            runtime_service_enabled: false,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProxyStatus {
    pub running: bool,
    pub bound_addr: String,
    pub active_routes: usize,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProxyTarget {
    pub instance_id: String,
    pub name: String,
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub running: bool,
}

// Application global state.
pub struct AppState {
    pub models: Mutex<Vec<ModelInfo>>,
    pub engines: Mutex<Vec<EngineInfo>>,
    pub model_scan_generation: AtomicU64,
    pub engine_scan_generation: AtomicU64,
    pub engine_names: Mutex<HashMap<String, String>>,
    pub instances: Mutex<HashMap<String, InstanceConfig>>,
    pub running: Mutex<HashMap<String, RunningInstance>>,
    pub starting: Mutex<std::collections::HashSet<String>>,
    pub config_dir: Mutex<PathBuf>,
    pub cancel_flags: Mutex<HashMap<String, bool>>,
    pub pause_flags: Mutex<HashMap<String, bool>>,
    pub active_downloads: Mutex<std::collections::HashSet<String>>,
    pub active_download_paths: Mutex<std::collections::HashSet<String>>,
    pub download_queue: Mutex<Vec<PersistedQueueEntry>>,
    pub download_active_batches: Mutex<std::collections::HashSet<String>>,
    pub download_active_entries: Mutex<HashMap<String, PersistedQueueEntry>>,
    pub download_last_inflight_persist: Mutex<Instant>,
    pub download_scheduler_lock: Mutex<()>,
    pub download_inflight_lock: Mutex<()>,
    pub download_shutting_down: AtomicBool,
    pub download_active_file_slots: AtomicUsize,
    pub download_slot_notify: Arc<tokio::sync::Notify>,
    pub download_max_concurrent: Mutex<usize>,
    pub download_bandwidth_limit_bytes_per_sec: Mutex<u64>,
    pub download_low_priority_throttle: Mutex<bool>,
    pub download_bandwidth_limiter: Mutex<DownloadBandwidthLimiter>,
    pub workers: Mutex<Vec<WorkerInfo>>,
    pub usb4_adapters: Mutex<Vec<Usb4Adapter>>,
    pub proxy_config: Mutex<ProxyConfig>,
    pub proxy_shutdown: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    pub proxy_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub proxy_bound_addr: Mutex<Option<String>>,
    pub proxy_last_error: Mutex<Option<String>>,
    pub proxy_lifecycle_lock: tokio::sync::Mutex<()>,
    pub runtime_managed_instances: Mutex<std::collections::HashSet<String>>,
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

// Global config structure for JSON serialization.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct GlobalConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_load_warning: Option<String>,
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

// Window state.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

// Download queue persistence.
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

// ModelScope file information.
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

// Download artifact state.
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

fn default_launch_mode() -> String {
    "managed".to_string()
}

fn default_negative_one_i32() -> i32 {
    -1
}

fn default_negative_one_f32() -> f32 {
    -1.0
}

fn default_cache_ram() -> i32 {
    8192
}

fn default_ctx_checkpoints() -> u32 {
    32
}

fn default_checkpoint_min_step() -> u32 {
    8192
}

fn default_point_one() -> f32 {
    0.1
}

fn default_embd_normalize() -> i32 {
    2
}

fn default_models_max() -> u32 {
    4
}

fn default_mtmd_batch_max_tokens() -> u32 {
    1024
}

fn default_adaptive_decay() -> f32 {
    0.9
}

fn default_sse_ping_interval() -> i32 {
    30
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
