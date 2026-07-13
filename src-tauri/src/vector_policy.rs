use crate::models::InstanceConfig;
use std::path::Path;

const EMBEDDING_HINTS: &[&str] = &[
    "embed",
    "embedding",
    "bge",
    "gte",
    "e5",
    "text-embedding",
    "sentence-bert",
    "sentence-t5",
    "instructor",
    "bert",
    "nomic",
    "jina",
];
const RERANKER_HINTS: &[&str] = &["rerank", "reranker", "cross-encoder"];

const VECTOR_ALLOWED_FIELDS: &[&str] = &[
    "id",
    "name",
    "engine_id",
    "model_path",
    "alias",
    "auto_start",
    "ctx_size",
    "ctx_size_auto",
    "gpu_layers_auto",
    "gpu_layers",
    "threads",
    "threads_batch",
    "threads_http",
    "batch_size",
    "ubatch_size",
    "parallel",
    "cont_batching",
    "warmup",
    "rope_scaling",
    "rope_scale",
    "rope_freq_base",
    "rope_freq_scale",
    "yarn_ext_factor",
    "yarn_attn_factor",
    "yarn_beta_slow",
    "yarn_beta_fast",
    "yarn_orig_ctx",
    "flash_attn",
    "moe_cpu_layers",
    "cpu_moe",
    "mlock",
    "no_mmap",
    "no_repack",
    "direct_io",
    "numa",
    "perf",
    "check_tensors",
    "fit",
    "fit_target",
    "fit_ctx",
    "cache_type_k",
    "cache_type_v",
    "kv_unified",
    "cache_idle_slots",
    "no_kv_offload",
    "device",
    "split_mode",
    "tensor_split",
    "main_gpu",
    "override_kv",
    "host",
    "port",
    "api_key",
    "api_key_file",
    "ssl_key_file",
    "ssl_cert_file",
    "path_prefix",
    "api_prefix",
    "no_ui",
    "offline",
    "metrics",
    "props",
    "slots_enabled",
    "timeout",
    "sleep_idle",
    "verbose",
    "rpc_servers",
    "sse_ping_interval",
    "reuse_port",
    "embedding",
    "pooling",
    "embd_normalize",
    "reranking",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelWorkload {
    Inference,
    Embedding,
    Reranker,
}

impl ModelWorkload {
    pub fn is_vector(self) -> bool {
        self != Self::Inference
    }
}

#[derive(Debug)]
pub struct VectorNormalization {
    pub config: InstanceConfig,
    pub workload: ModelWorkload,
}

impl VectorNormalization {
    pub fn into_config(self) -> InstanceConfig {
        debug_assert_eq!(self.config.embedding, self.workload.is_vector());
        self.config
    }
}

pub fn classify_model_workload(architecture: Option<&str>, path: &Path) -> ModelWorkload {
    let basename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let fallback = format!("{} {basename}", architecture.unwrap_or(""));
    if has_hint(&fallback, RERANKER_HINTS) {
        ModelWorkload::Reranker
    } else if has_hint(&fallback, EMBEDDING_HINTS) {
        ModelWorkload::Embedding
    } else {
        ModelWorkload::Inference
    }
}

pub fn normalize_for_vector(config: InstanceConfig) -> VectorNormalization {
    let before = serde_json::to_value(&config).expect("InstanceConfig must serialize");
    let mut normalized = serde_json::to_value(InstanceConfig::default())
        .expect("default InstanceConfig must serialize");
    let source = before
        .as_object()
        .expect("InstanceConfig must be an object");
    let target = normalized
        .as_object_mut()
        .expect("default InstanceConfig must be an object");
    for field in VECTOR_ALLOWED_FIELDS {
        if let Some(value) = source.get(*field) {
            target.insert((*field).to_string(), value.clone());
        }
    }

    let detected = classify_model_workload(None, Path::new(&config.model_path));
    let workload = match detected {
        ModelWorkload::Inference if config.reranking => ModelWorkload::Reranker,
        ModelWorkload::Inference => ModelWorkload::Embedding,
        workload => workload,
    };
    let mut config: InstanceConfig =
        serde_json::from_value(normalized).expect("normalized InstanceConfig must deserialize");
    config.embedding = true;
    if workload == ModelWorkload::Reranker {
        config.reranking = true;
        config.pooling = "rank".into();
    } else {
        config.reranking = false;
        if config.pooling == "rank" {
            config.pooling = InstanceConfig::default().pooling;
        }
    }
    if config.batch_size > config.ubatch_size {
        config.batch_size = config.ubatch_size;
    }
    VectorNormalization { config, workload }
}

pub fn normalize_for_launch(config: InstanceConfig) -> VectorNormalization {
    let detected = classify_model_workload(None, Path::new(&config.model_path));
    if !config.embedding && !config.reranking && !detected.is_vector() {
        return VectorNormalization {
            config,
            workload: ModelWorkload::Inference,
        };
    }

    normalize_for_vector(config)
}

fn has_hint(value: &str, hints: &[&str]) -> bool {
    let normalized = value.to_lowercase();
    hints.iter().any(|hint| normalized.contains(hint))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_normalization_enforces_vector_invariants_and_preserves_inference_mtp() {
        let embedding = InstanceConfig {
            model_path: "C:/models/bge-small.gguf".into(),
            pooling: "rank".into(),
            reranking: true,
            batch_size: 2048,
            ubatch_size: 512,
            spec_type: "draft-mtp".into(),
            cache_type_draft_k: "q8_0".into(),
            custom_args: vec!["--temp 1.5".into()],
            ..InstanceConfig::default()
        };
        let normalized = normalize_for_launch(embedding);
        assert_eq!(normalized.workload, ModelWorkload::Embedding);
        assert!(normalized.config.embedding);
        assert!(!normalized.config.reranking);
        assert_ne!(normalized.config.pooling, "rank");
        assert_eq!(normalized.config.batch_size, 512);
        assert!(normalized.config.spec_type.is_empty());
        assert!(normalized.config.cache_type_draft_k.is_empty());
        assert!(normalized.config.custom_args.is_empty());

        let reranker = normalize_for_launch(InstanceConfig {
            model_path: "C:/models/bge-reranker-v2.gguf".into(),
            ..InstanceConfig::default()
        });
        assert_eq!(reranker.workload, ModelWorkload::Reranker);
        assert!(reranker.config.embedding);
        assert!(reranker.config.reranking);
        assert_eq!(reranker.config.pooling, "rank");

        let inference = InstanceConfig {
            model_path: "C:/models/llama.gguf".into(),
            spec_type: "draft-mtp".into(),
            cache_type_draft_k: "q8_0".into(),
            custom_args: vec!["--temp 1.5".into()],
            ..InstanceConfig::default()
        };
        let normalized = normalize_for_launch(inference.clone());
        assert_eq!(normalized.workload, ModelWorkload::Inference);
        assert_eq!(normalized.config.spec_type, inference.spec_type);
        assert_eq!(
            normalized.config.cache_type_draft_k,
            inference.cache_type_draft_k
        );
        assert_eq!(normalized.config.custom_args, inference.custom_args);
        assert_eq!(
            serde_json::to_value(&normalized.config).unwrap(),
            serde_json::to_value(&inference).unwrap()
        );
    }
}
