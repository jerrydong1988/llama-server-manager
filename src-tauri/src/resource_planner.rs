use crate::models::{GgufMetadataSummary, GgufResourceMetadata, InstanceConfig};
use crate::utils::parse_gguf_metadata;
use regex_lite::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MIB: u64 = 1024 * 1024;
const MAX_SHARDS: u32 = 1024;
const DEFAULT_CONTEXT: u32 = 4096;
const DEFAULT_BLOCKS: u32 = 32;
const DEFAULT_EMBEDDING: u32 = 4096;

#[derive(Debug, Clone, Copy, Default)]
pub struct CapacitySnapshot {
    pub ram_total_bytes: Option<u64>,
    pub ram_available_bytes: Option<u64>,
    pub vram_total_bytes: Option<u64>,
    pub vram_available_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRange {
    pub min_bytes: u64,
    pub expected_bytes: u64,
    pub max_bytes: u64,
}

impl ResourceRange {
    fn exact(bytes: u64) -> Self {
        Self {
            min_bytes: bytes,
            expected_bytes: bytes,
            max_bytes: bytes,
        }
    }

    fn new(min_bytes: u64, expected_bytes: u64, max_bytes: u64) -> Self {
        Self {
            min_bytes,
            expected_bytes: expected_bytes.max(min_bytes),
            max_bytes: max_bytes.max(expected_bytes).max(min_bytes),
        }
    }

    fn add(self, other: Self) -> Self {
        Self::new(
            self.min_bytes.saturating_add(other.min_bytes),
            self.expected_bytes.saturating_add(other.expected_bytes),
            self.max_bytes.saturating_add(other.max_bytes),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceBudget {
    pub required: ResourceRange,
    pub total_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
    pub reserved_bytes: u64,
    pub expected_headroom_bytes: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceComponentEstimate {
    pub kind: String,
    pub target: String,
    pub required: ResourceRange,
    pub exact: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePlanFacts {
    pub context_tokens: u32,
    pub parallel_slots: u32,
    pub model_shards_found: u32,
    pub model_shards_expected: u32,
    pub gpu_offload_percent: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePlan {
    pub schema_version: u8,
    pub status: String,
    pub confidence: String,
    pub ram: ResourceBudget,
    pub vram: ResourceBudget,
    pub components: Vec<ResourceComponentEstimate>,
    pub facts: ResourcePlanFacts,
    pub reasons: Vec<String>,
    pub assumptions: Vec<String>,
}

impl ResourcePlan {
    pub fn explicitly_infeasible(&self) -> bool {
        self.status == "infeasible"
    }
}

#[derive(Debug, Clone)]
struct ArtifactEstimate {
    bytes: ResourceRange,
    exact: bool,
    found_shards: u32,
    expected_shards: u32,
    metadata: Option<GgufMetadataSummary>,
}

#[derive(Debug, Clone, Copy)]
struct FractionRange {
    min: f64,
    expected: f64,
    max: f64,
}

impl FractionRange {
    fn exact(value: f64) -> Self {
        let value = value.clamp(0.0, 1.0);
        Self {
            min: value,
            expected: value,
            max: value,
        }
    }

    fn new(min: f64, expected: f64, max: f64) -> Self {
        Self {
            min: min.clamp(0.0, 1.0),
            expected: expected.clamp(0.0, 1.0),
            max: max.clamp(0.0, 1.0),
        }
    }
}

#[derive(Default)]
struct PlanNotes {
    reasons: Vec<String>,
    assumptions: Vec<String>,
    confidence_penalty: u8,
    force_unknown: bool,
}

impl PlanNotes {
    fn reason(&mut self, code: &str) {
        push_unique(&mut self.reasons, code);
    }

    fn assumption(&mut self, code: &str) {
        push_unique(&mut self.assumptions, code);
    }

    fn lower_confidence(&mut self, amount: u8) {
        self.confidence_penalty = self.confidence_penalty.max(amount);
    }

    fn unknown(&mut self, code: &str) {
        self.reason(code);
        self.force_unknown = true;
        self.lower_confidence(2);
    }
}

pub fn plan_instance_resources(
    config: &InstanceConfig,
    engine_backend: &str,
    capacity: CapacitySnapshot,
) -> ResourcePlan {
    let mut notes = PlanNotes::default();
    if config.launch_mode.eq_ignore_ascii_case("manual") {
        notes.unknown("manual_command_not_inspectable");
    }
    if !config.custom_args.is_empty() {
        notes.unknown("custom_arguments_may_change_resources");
    }
    if !config.models_dir.trim().is_empty() {
        notes.unknown("multi_model_residency_is_dynamic");
    }
    if !config.rpc_servers.trim().is_empty() {
        notes.unknown("remote_offload_capacity_not_measured");
    }
    if !config.tensor_split.trim().is_empty()
        || matches!(
            config.split_mode.trim().to_ascii_lowercase().as_str(),
            "row" | "tensor"
        )
    {
        notes.unknown("per_device_gpu_capacity_not_measured");
    }
    if !config.override_kv.trim().is_empty() {
        notes.unknown("metadata_overrides_not_interpreted");
    }

    let model = inspect_artifact(&config.model_path, "model", true, &mut notes);
    let projector = if !config.no_mmproj && !config.mmproj_path.trim().is_empty() {
        Some(inspect_artifact(
            &config.mmproj_path,
            "projector",
            true,
            &mut notes,
        ))
    } else {
        None
    };
    let draft = if !config.draft_model_path.trim().is_empty() {
        Some(inspect_artifact(
            &config.draft_model_path,
            "draft",
            true,
            &mut notes,
        ))
    } else {
        None
    };
    let lora = if !config.lora_path.trim().is_empty() {
        Some(inspect_artifact(
            &config.lora_path,
            "adapter",
            false,
            &mut notes,
        ))
    } else {
        None
    };

    let main_meta = model.metadata.as_ref();
    let context_tokens = effective_context_tokens(config, main_meta, &mut notes);
    let parallel_slots = if config.parallel > 0 {
        config.parallel as u32
    } else {
        notes.assumption("parallel_slots_are_engine_selected");
        1
    };
    let fit_enabled = effective_fit_enabled(config);
    if fit_enabled {
        notes.reason("llama_fit_may_reduce_unset_parameters");
        notes.lower_confidence(1);
    }

    let main_offload = main_offload_fraction(
        config,
        engine_backend,
        main_meta.map(|metadata| &metadata.resource),
        model.bytes,
        capacity,
        &mut notes,
    );
    let draft_offload = draft.as_ref().map(|artifact| {
        manual_offload_fraction(
            config.draft_gpu_layers,
            artifact
                .metadata
                .as_ref()
                .and_then(|value| value.resource.block_count),
            "draft_layer_count_unavailable",
            &mut notes,
        )
    });
    let projector_offload = if projector.is_some() && !config.no_mmproj_offload {
        if accelerator_requested(engine_backend, config) {
            FractionRange::exact(1.0)
        } else {
            FractionRange::exact(0.0)
        }
    } else {
        FractionRange::exact(0.0)
    };

    let mut components = Vec::new();
    let mut ram_required = ResourceRange::default();
    let mut vram_required = ResourceRange::default();

    add_weight_components(
        "model_weights",
        &model,
        main_offload,
        uses_file_backed_loading(config),
        &mut ram_required,
        &mut vram_required,
        &mut components,
    );
    if let Some(artifact) = projector.as_ref() {
        add_weight_components(
            "projector_weights",
            artifact,
            projector_offload,
            uses_file_backed_loading(config),
            &mut ram_required,
            &mut vram_required,
            &mut components,
        );
    }
    if let (Some(artifact), Some(offload)) = (draft.as_ref(), draft_offload) {
        add_weight_components(
            "draft_weights",
            artifact,
            offload,
            uses_file_backed_loading(config),
            &mut ram_required,
            &mut vram_required,
            &mut components,
        );
    }
    if let Some(artifact) = lora.as_ref() {
        add_weight_components(
            "adapter_weights",
            artifact,
            main_offload,
            false,
            &mut ram_required,
            &mut vram_required,
            &mut components,
        );
    }

    let fit_min_context =
        if fit_enabled && !parameter_explicit(config, &["ctx_size", "ctx_size_auto"]) {
            let minimum = if parameter_explicit(config, &["fit_ctx"]) {
                config.fit_ctx.max(1)
            } else {
                DEFAULT_CONTEXT
            };
            notes.assumption("fit_context_range_starts_at_minimum");
            Some(minimum.min(context_tokens))
        } else {
            None
        };
    let main_kv = estimate_kv_cache(
        main_meta,
        context_tokens,
        fit_min_context,
        &config.cache_type_k,
        &config.cache_type_v,
        config.swa_full,
        &mut notes,
    );
    let main_kv_offload = if config.no_kv_offload {
        FractionRange::exact(0.0)
    } else {
        main_offload
    };
    add_split_component(
        "kv_cache",
        main_kv,
        main_kv_offload,
        false,
        &mut ram_required,
        &mut vram_required,
        &mut components,
    );

    if let Some(artifact) = draft.as_ref() {
        let draft_kv = estimate_kv_cache(
            artifact.metadata.as_ref(),
            context_tokens,
            fit_min_context,
            &config.cache_type_draft_k,
            &config.cache_type_draft_v,
            config.swa_full,
            &mut notes,
        );
        add_split_component(
            "draft_kv_cache",
            draft_kv,
            draft_offload.unwrap_or(FractionRange::exact(0.0)),
            false,
            &mut ram_required,
            &mut vram_required,
            &mut components,
        );
    }

    let embedding = main_meta
        .and_then(|value| value.resource.embedding_length)
        .unwrap_or(DEFAULT_EMBEDDING);
    if main_meta
        .and_then(|value| value.resource.embedding_length)
        .is_none()
    {
        notes.assumption("compute_buffers_use_default_embedding_width");
        notes.lower_confidence(1);
    }
    let (host_runtime, gpu_runtime) = estimate_runtime_overhead(config, embedding, main_offload);
    add_component(
        "runtime_buffers",
        "host",
        host_runtime,
        false,
        &mut ram_required,
        &mut components,
    );
    if gpu_runtime.max_bytes > 0 {
        add_component(
            "runtime_buffers",
            "accelerator",
            gpu_runtime,
            false,
            &mut vram_required,
            &mut components,
        );
    }

    if config.cache_ram > 0 {
        let maximum = (config.cache_ram as u64).saturating_mul(MIB);
        let prompt_cache = ResourceRange::new(0, maximum / 4, maximum);
        add_component(
            "prompt_cache",
            "host",
            prompt_cache,
            false,
            &mut ram_required,
            &mut components,
        );
        notes.assumption("prompt_cache_is_demand_driven_up_to_configured_limit");
        notes.lower_confidence(1);
    }
    if config.ctx_checkpoints > 0 {
        notes.assumption("context_checkpoint_usage_is_workload_dependent");
        notes.lower_confidence(1);
    }

    if engine_backend.trim().to_ascii_lowercase().contains("metal")
        && capacity.vram_total_bytes.is_none()
    {
        ram_required = ram_required.add(vram_required);
        vram_required = ResourceRange::default();
        for component in &mut components {
            if component.target == "accelerator" {
                component.target = "unified".to_string();
            }
        }
        notes.assumption("metal_uses_unified_system_memory");
        notes.lower_confidence(1);
    }

    let host_reserve = reserve_bytes(capacity.ram_total_bytes, 512 * MIB, 0.05);
    let mut gpu_reserve = reserve_bytes(capacity.vram_total_bytes, 256 * MIB, 0.05);
    if fit_enabled {
        gpu_reserve = gpu_reserve.max(
            parse_fit_target(&config.fit_target)
                .unwrap_or(1024)
                .saturating_mul(MIB),
        );
    }
    let ram = budget(
        ram_required,
        capacity.ram_total_bytes,
        capacity.ram_available_bytes,
        host_reserve,
    );
    let vram = budget(
        vram_required,
        capacity.vram_total_bytes,
        capacity.vram_available_bytes,
        gpu_reserve,
    );

    let ram_status = evaluate_budget(&ram);
    let vram_status = evaluate_budget(&vram);
    let status = combine_status(ram_status, vram_status, notes.force_unknown);
    if ram_status == "infeasible" {
        notes.reason("insufficient_available_ram");
    }
    if vram_status == "infeasible" {
        notes.reason("insufficient_available_vram");
    }
    if ram_status == "unknown" {
        notes.reason("ram_capacity_unavailable");
        notes.lower_confidence(2);
    }
    if vram_required.max_bytes > 0 && vram_status == "unknown" {
        notes.reason("vram_capacity_unavailable");
        notes.lower_confidence(2);
    }
    if status == "constrained" {
        notes.reason("estimate_range_exceeds_available_headroom");
    }

    let confidence = match notes.confidence_penalty {
        0 => "high",
        1 => "medium",
        _ => "low",
    }
    .to_string();

    ResourcePlan {
        schema_version: 1,
        status: status.to_string(),
        confidence,
        ram,
        vram,
        components,
        facts: ResourcePlanFacts {
            context_tokens,
            parallel_slots,
            model_shards_found: model.found_shards,
            model_shards_expected: model.expected_shards,
            gpu_offload_percent: (main_offload.expected * 100.0).round() as u8,
        },
        reasons: notes.reasons,
        assumptions: notes.assumptions,
    }
}

fn inspect_artifact(
    raw_path: &str,
    role: &str,
    parse_metadata: bool,
    notes: &mut PlanNotes,
) -> ArtifactEstimate {
    let path = PathBuf::from(raw_path.trim());
    if raw_path.trim().is_empty() || !path.is_file() {
        notes.unknown(&format!("{role}_artifact_unavailable"));
        return ArtifactEstimate {
            bytes: ResourceRange::default(),
            exact: false,
            found_shards: 0,
            expected_shards: 1,
            metadata: None,
        };
    }

    let selected_size = path.metadata().map(|value| value.len()).unwrap_or(0);
    let mut bytes = ResourceRange::exact(selected_size);
    let mut exact = true;
    let mut found_shards = 1;
    let mut expected_shards = 1;
    let mut metadata_path = path.clone();

    if let Some((base, _index, total)) = parse_shard_name(&path) {
        expected_shards = total;
        if total > MAX_SHARDS {
            notes.unknown(&format!("{role}_shard_count_exceeds_limit"));
            exact = false;
        } else {
            let mut shards = BTreeMap::new();
            if let Some(parent) = path.parent() {
                if let Ok(entries) = std::fs::read_dir(parent) {
                    for entry in entries.flatten() {
                        let candidate = entry.path();
                        let Some((candidate_base, index, candidate_total)) =
                            parse_shard_name(&candidate)
                        else {
                            continue;
                        };
                        if candidate_total == total
                            && candidate_base.eq_ignore_ascii_case(&base)
                            && (1..=total).contains(&index)
                        {
                            if let Ok(metadata) = candidate.metadata() {
                                shards.entry(index).or_insert((candidate, metadata.len()));
                            }
                        }
                    }
                }
            }
            found_shards = shards.len() as u32;
            if let Some((first_path, _)) = shards.get(&1) {
                metadata_path = first_path.clone();
            }
            let known_bytes = shards
                .values()
                .fold(0_u64, |sum, (_, size)| sum.saturating_add(*size));
            if found_shards == total && known_bytes > 0 {
                bytes = ResourceRange::exact(known_bytes);
            } else {
                exact = false;
                notes.unknown(&format!("incomplete_{role}_shards"));
                let divisor = found_shards.max(1) as u64;
                let average = known_bytes.max(selected_size) / divisor;
                let largest = shards
                    .values()
                    .map(|(_, size)| *size)
                    .max()
                    .unwrap_or(selected_size);
                bytes = ResourceRange::new(
                    known_bytes.max(selected_size),
                    average.saturating_mul(total as u64),
                    largest.saturating_mul(total as u64),
                );
            }
        }
    }

    let metadata = if parse_metadata
        && metadata_path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("gguf"))
    {
        match parse_gguf_metadata(&metadata_path) {
            Ok(metadata) => Some(metadata),
            Err(_) => {
                notes.reason(&format!("{role}_metadata_unavailable"));
                notes.lower_confidence(2);
                None
            }
        }
    } else {
        None
    };

    ArtifactEstimate {
        bytes,
        exact,
        found_shards,
        expected_shards,
        metadata,
    }
}

fn parse_shard_name(path: &Path) -> Option<(String, u32, u32)> {
    let name = path.file_name()?.to_str()?;
    let regex = Regex::new(r"(?i)^(.+?)-([0-9]{5})-of-([0-9]{5})\.gguf$").ok()?;
    let captures = regex.captures(name)?;
    let base = captures.get(1)?.as_str().to_string();
    let index = captures.get(2)?.as_str().parse().ok()?;
    let total = captures.get(3)?.as_str().parse().ok()?;
    if total <= 1 || index == 0 || index > total {
        return None;
    }
    Some((base, index, total))
}

fn effective_context_tokens(
    config: &InstanceConfig,
    metadata: Option<&GgufMetadataSummary>,
    notes: &mut PlanNotes,
) -> u32 {
    if config.ctx_size > 0 && !config.ctx_size_auto {
        return config.ctx_size;
    }
    if let Some(context) = metadata
        .and_then(|value| value.context_length)
        .filter(|value| *value > 0)
    {
        notes.assumption("context_uses_model_training_limit");
        return context;
    }
    notes.reason("context_length_unavailable");
    notes.assumption("context_uses_safe_default");
    notes.lower_confidence(2);
    DEFAULT_CONTEXT
}

fn effective_fit_enabled(config: &InstanceConfig) -> bool {
    match config.fit_mode.trim().to_ascii_lowercase().as_str() {
        "on" => true,
        "off" => false,
        _ => config.fit || !parameter_explicit(config, &["fit", "fit_mode"]),
    }
}

fn parameter_explicit(config: &InstanceConfig, keys: &[&str]) -> bool {
    config
        .explicit_overrides
        .as_ref()
        .map_or(true, |overrides| {
            overrides.iter().any(|value| keys.contains(&value.as_str()))
        })
}

fn accelerator_requested(engine_backend: &str, config: &InstanceConfig) -> bool {
    let backend = engine_backend.trim().to_ascii_lowercase();
    let device = config.device.trim().to_ascii_lowercase();
    if backend == "cpu" || device == "cpu" {
        return false;
    }
    !matches!(backend.as_str(), "" | "unknown") || config.gpu_layers_auto || config.gpu_layers > 0
}

fn main_offload_fraction(
    config: &InstanceConfig,
    engine_backend: &str,
    metadata: Option<&GgufResourceMetadata>,
    weights: ResourceRange,
    capacity: CapacitySnapshot,
    notes: &mut PlanNotes,
) -> FractionRange {
    if !accelerator_requested(engine_backend, config) {
        return FractionRange::exact(0.0);
    }
    if !config.gpu_layers_auto {
        let mut result = manual_offload_fraction(
            config.gpu_layers,
            metadata.and_then(|value| value.block_count),
            "model_layer_count_unavailable",
            notes,
        );
        apply_cpu_moe_adjustment(config, metadata, &mut result, notes);
        return result;
    }

    notes.assumption("automatic_gpu_layers_follow_current_free_vram");
    notes.lower_confidence(1);
    let Some(available) = capacity.vram_available_bytes else {
        notes.unknown("automatic_gpu_layers_need_vram_capacity");
        return FractionRange::new(0.0, 0.5, 1.0);
    };
    let reserve = reserve_bytes(capacity.vram_total_bytes, 512 * MIB, 0.08);
    let usable = available.saturating_sub(reserve);
    let expected = if weights.expected_bytes == 0 {
        0.0
    } else {
        usable as f64 / weights.expected_bytes as f64
    }
    .clamp(0.0, 1.0);
    let layer_step = metadata
        .and_then(|value| value.block_count)
        .map(|blocks| 1.0 / blocks.max(1) as f64)
        .unwrap_or(0.10);
    let mut result = FractionRange::new(expected - layer_step, expected, expected + layer_step);
    apply_cpu_moe_adjustment(config, metadata, &mut result, notes);
    result
}

fn manual_offload_fraction(
    layers: u32,
    block_count: Option<u32>,
    unavailable_reason: &str,
    notes: &mut PlanNotes,
) -> FractionRange {
    if layers == 0 {
        return FractionRange::exact(0.0);
    }
    if let Some(blocks) = block_count.filter(|value| *value > 0) {
        return FractionRange::exact(layers as f64 / blocks as f64);
    }
    notes.reason(unavailable_reason);
    notes.lower_confidence(2);
    if layers >= 99 {
        FractionRange::new(0.85, 1.0, 1.0)
    } else {
        let expected = (layers as f64 / 80.0).clamp(0.05, 0.95);
        FractionRange::new(0.0, expected, 1.0)
    }
}

fn apply_cpu_moe_adjustment(
    config: &InstanceConfig,
    metadata: Option<&GgufResourceMetadata>,
    fraction: &mut FractionRange,
    notes: &mut PlanNotes,
) {
    if !config.cpu_moe && config.moe_cpu_layers == 0 {
        return;
    }
    notes.reason("moe_tensor_distribution_is_approximate");
    notes.lower_confidence(2);
    let cpu_ratio = if config.cpu_moe {
        0.35
    } else if let Some(blocks) = metadata
        .and_then(|value| value.block_count)
        .filter(|value| *value > 0)
    {
        (config.moe_cpu_layers as f64 / blocks as f64).clamp(0.0, 1.0) * 0.35
    } else {
        0.15
    };
    fraction.min *= 1.0 - cpu_ratio;
    fraction.expected *= 1.0 - cpu_ratio;
    fraction.max *= 1.0 - cpu_ratio * 0.5;
}

fn uses_file_backed_loading(config: &InstanceConfig) -> bool {
    matches!(
        config.load_mode.trim().to_ascii_lowercase().as_str(),
        "" | "auto" | "mmap"
    )
}

fn add_weight_components(
    kind: &str,
    artifact: &ArtifactEstimate,
    offload: FractionRange,
    file_backed: bool,
    ram_total: &mut ResourceRange,
    vram_total: &mut ResourceRange,
    components: &mut Vec<ResourceComponentEstimate>,
) {
    let (host, accelerator) = split_range(artifact.bytes, offload);
    let host = if file_backed {
        ResourceRange::new(
            scale_bytes(host.min_bytes, 0.20),
            scale_bytes(host.expected_bytes, 0.70),
            host.max_bytes,
        )
    } else {
        host
    };
    add_component(
        kind,
        "host",
        host,
        artifact.exact && offload.min == offload.max && !file_backed,
        ram_total,
        components,
    );
    if accelerator.max_bytes > 0 {
        add_component(
            kind,
            "accelerator",
            accelerator,
            artifact.exact && offload.min == offload.max,
            vram_total,
            components,
        );
    }
}

fn add_split_component(
    kind: &str,
    required: ResourceRange,
    offload: FractionRange,
    exact: bool,
    ram_total: &mut ResourceRange,
    vram_total: &mut ResourceRange,
    components: &mut Vec<ResourceComponentEstimate>,
) {
    let (host, accelerator) = split_range(required, offload);
    add_component(kind, "host", host, exact, ram_total, components);
    if accelerator.max_bytes > 0 {
        add_component(
            kind,
            "accelerator",
            accelerator,
            exact,
            vram_total,
            components,
        );
    }
}

fn split_range(total: ResourceRange, offload: FractionRange) -> (ResourceRange, ResourceRange) {
    let accelerator = ResourceRange::new(
        scale_bytes(total.min_bytes, offload.min),
        scale_bytes(total.expected_bytes, offload.expected),
        scale_bytes(total.max_bytes, offload.max),
    );
    let host = ResourceRange::new(
        scale_bytes(total.min_bytes, 1.0 - offload.max),
        scale_bytes(total.expected_bytes, 1.0 - offload.expected),
        scale_bytes(total.max_bytes, 1.0 - offload.min),
    );
    (host, accelerator)
}

fn add_component(
    kind: &str,
    target: &str,
    required: ResourceRange,
    exact: bool,
    total: &mut ResourceRange,
    components: &mut Vec<ResourceComponentEstimate>,
) {
    *total = total.add(required);
    components.push(ResourceComponentEstimate {
        kind: kind.to_string(),
        target: target.to_string(),
        required,
        exact,
    });
}

fn estimate_kv_cache(
    metadata: Option<&GgufMetadataSummary>,
    context_tokens: u32,
    minimum_context_tokens: Option<u32>,
    cache_type_k: &str,
    cache_type_v: &str,
    swa_full: bool,
    notes: &mut PlanNotes,
) -> ResourceRange {
    let resource = metadata.map(|value| &value.resource);
    let blocks = resource
        .and_then(|value| value.block_count)
        .unwrap_or(DEFAULT_BLOCKS);
    let embedding = resource
        .and_then(|value| value.embedding_length)
        .unwrap_or(DEFAULT_EMBEDDING);
    let heads = resource.and_then(|value| value.attention_head_count);
    let kv_heads = resource.and_then(|value| value.attention_head_count_kv);
    let key_width = attention_width(
        resource.and_then(|value| value.attention_key_length),
        embedding,
        heads,
        kv_heads,
    );
    let value_width = attention_width(
        resource.and_then(|value| value.attention_value_length),
        embedding,
        heads,
        kv_heads,
    );
    let key_bytes = cache_type_bytes(cache_type_k, notes);
    let value_bytes = cache_type_bytes(cache_type_v, notes);
    let per_token = blocks as f64 * (key_width * key_bytes + value_width * value_bytes);
    let full = scale_float(per_token * context_tokens.max(1) as f64);

    let metadata_complete = resource.is_some_and(|value| {
        value.block_count.is_some()
            && value.embedding_length.is_some()
            && value.attention_head_count.is_some()
            && value.attention_head_count_kv.is_some()
    });
    if !metadata_complete {
        notes.reason("kv_metadata_incomplete");
        notes.assumption("kv_range_covers_unknown_attention_shape");
        notes.lower_confidence(2);
    }
    let shape_min = if metadata_complete { 1.0 } else { 0.125 };
    let shape_expected = if metadata_complete { 1.0 } else { 0.35 };
    let active_context = resource
        .and_then(|value| value.sliding_window)
        .filter(|window| !swa_full && *window < context_tokens);
    if active_context.is_some() {
        notes.assumption("sliding_window_reduces_part_of_kv_cache");
        notes.lower_confidence(1);
    }
    let context_min = active_context
        .map(|window| window as f64 / context_tokens.max(1) as f64)
        .unwrap_or(1.0)
        .min(
            minimum_context_tokens
                .map(|minimum| minimum as f64 / context_tokens.max(1) as f64)
                .unwrap_or(1.0),
        )
        .clamp(0.01, 1.0);
    let context_expected = if active_context.is_some() {
        (context_min + 1.0) / 2.0
    } else {
        1.0
    };
    ResourceRange::new(
        scale_bytes(full, shape_min * context_min),
        scale_bytes(full, shape_expected * context_expected),
        full,
    )
}

fn attention_width(
    explicit_length: Option<u32>,
    embedding: u32,
    heads: Option<u32>,
    kv_heads: Option<u32>,
) -> f64 {
    if let (Some(length), Some(kv_heads)) = (explicit_length, kv_heads) {
        return length as f64 * kv_heads as f64;
    }
    match (heads, kv_heads) {
        (Some(heads), Some(kv_heads)) if heads > 0 => {
            embedding as f64 * kv_heads as f64 / heads as f64
        }
        _ => embedding as f64,
    }
}

fn cache_type_bytes(value: &str, notes: &mut PlanNotes) -> f64 {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "f16" | "bf16" => 2.0,
        "f32" => 4.0,
        "q8_0" => 34.0 / 32.0,
        "q5_0" => 22.0 / 32.0,
        "q5_1" => 24.0 / 32.0,
        "q4_0" | "iq4_nl" => 18.0 / 32.0,
        "q4_1" => 20.0 / 32.0,
        _ => {
            notes.reason("unknown_kv_cache_type");
            notes.lower_confidence(2);
            2.0
        }
    }
}

fn estimate_runtime_overhead(
    config: &InstanceConfig,
    embedding: u32,
    offload: FractionRange,
) -> (ResourceRange, ResourceRange) {
    let batch = config.batch_size.max(config.ubatch_size).max(32) as u64;
    let compute = batch
        .saturating_mul(embedding as u64)
        .saturating_mul(4)
        .saturating_mul(3);
    let host = ResourceRange::new(
        128 * MIB + compute / 2,
        256 * MIB + compute,
        768 * MIB + compute.saturating_mul(2),
    );
    if offload.max <= 0.0 {
        return (host, ResourceRange::default());
    }
    let gpu = ResourceRange::new(
        scale_bytes(128 * MIB + compute / 4, offload.min),
        scale_bytes(384 * MIB + compute / 2, offload.expected),
        scale_bytes(1024 * MIB + compute, offload.max),
    );
    (host, gpu)
}

fn reserve_bytes(total: Option<u64>, minimum: u64, ratio: f64) -> u64 {
    total
        .map(|value| scale_bytes(value, ratio).max(minimum))
        .unwrap_or(minimum)
}

fn parse_fit_target(value: &str) -> Option<u64> {
    value
        .split([',', '/'])
        .map(str::trim)
        .find(|value| !value.is_empty())?
        .parse()
        .ok()
}

fn budget(
    required: ResourceRange,
    total_bytes: Option<u64>,
    available_bytes: Option<u64>,
    reserved_bytes: u64,
) -> ResourceBudget {
    let expected_headroom_bytes = available_bytes.map(|available| {
        clamp_i128_to_i64(
            available as i128 - reserved_bytes as i128 - required.expected_bytes as i128,
        )
    });
    ResourceBudget {
        required,
        total_bytes,
        available_bytes,
        reserved_bytes,
        expected_headroom_bytes,
    }
}

fn evaluate_budget(value: &ResourceBudget) -> &'static str {
    if value.required.max_bytes == 0 {
        return "feasible";
    }
    let Some(available) = value.available_bytes else {
        return "unknown";
    };
    let usable = available.saturating_sub(value.reserved_bytes);
    if value.required.min_bytes > usable {
        "infeasible"
    } else if value.required.max_bytes > usable {
        "constrained"
    } else {
        "feasible"
    }
}

fn combine_status(ram: &str, vram: &str, force_unknown: bool) -> &'static str {
    if force_unknown {
        "unknown"
    } else if ram == "infeasible" || vram == "infeasible" {
        "infeasible"
    } else if ram == "unknown" || vram == "unknown" {
        "unknown"
    } else if ram == "constrained" || vram == "constrained" {
        "constrained"
    } else {
        "feasible"
    }
}

fn scale_bytes(value: u64, factor: f64) -> u64 {
    scale_float(value as f64 * factor)
}

fn scale_float(value: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        0
    } else if value >= u64::MAX as f64 {
        u64::MAX
    } else {
        value.round() as u64
    }
}

fn clamp_i128_to_i64(value: i128) -> i64 {
    value.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|current| current == value) {
        values.push(value.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn capacity(ram_mib: u64, vram_mib: u64) -> CapacitySnapshot {
        CapacitySnapshot {
            ram_total_bytes: Some(ram_mib * MIB),
            ram_available_bytes: Some(ram_mib * MIB),
            vram_total_bytes: Some(vram_mib * MIB),
            vram_available_bytes: Some(vram_mib * MIB),
        }
    }

    fn config_with_model(path: &Path) -> InstanceConfig {
        InstanceConfig {
            model_path: path.to_string_lossy().to_string(),
            launch_mode: "managed".into(),
            ctx_size: 4096,
            ctx_size_auto: false,
            gpu_layers_auto: false,
            gpu_layers: 0,
            cache_ram: 0,
            fit_mode: "off".into(),
            explicit_overrides: None,
            ..InstanceConfig::default()
        }
    }

    #[test]
    fn budget_status_uses_full_range_and_reserve() {
        let required = ResourceRange::new(100, 200, 300);
        let constrained = budget(required, Some(1000), Some(350), 100);
        assert_eq!(evaluate_budget(&constrained), "constrained");
        let infeasible = budget(required, Some(1000), Some(150), 100);
        assert_eq!(evaluate_budget(&infeasible), "infeasible");
        let feasible = budget(required, Some(1000), Some(500), 100);
        assert_eq!(evaluate_budget(&feasible), "feasible");
    }

    #[test]
    fn quantized_kv_types_reduce_the_estimate() {
        let metadata = GgufMetadataSummary {
            context_length: Some(4096),
            resource: GgufResourceMetadata {
                block_count: Some(32),
                embedding_length: Some(4096),
                attention_head_count: Some(32),
                attention_head_count_kv: Some(8),
                ..GgufResourceMetadata::default()
            },
            ..GgufMetadataSummary::default()
        };
        let mut notes = PlanNotes::default();
        let f16 = estimate_kv_cache(Some(&metadata), 4096, None, "f16", "f16", false, &mut notes);
        let q4 = estimate_kv_cache(
            Some(&metadata),
            4096,
            None,
            "q4_0",
            "q4_0",
            false,
            &mut notes,
        );
        assert!(q4.expected_bytes < f16.expected_bytes);
        assert_eq!(f16.min_bytes, f16.max_bytes);
    }

    #[test]
    fn full_and_partial_manual_offload_follow_layer_metadata() {
        let metadata = GgufResourceMetadata {
            block_count: Some(32),
            ..GgufResourceMetadata::default()
        };
        let mut notes = PlanNotes::default();
        let full = manual_offload_fraction(99, metadata.block_count, "missing", &mut notes);
        let partial = manual_offload_fraction(16, metadata.block_count, "missing", &mut notes);
        assert_eq!(full.expected, 1.0);
        assert_eq!(partial.expected, 0.5);
        assert_eq!(partial.min, partial.max);
    }

    #[test]
    fn cpu_only_plan_does_not_require_vram() {
        let root = std::env::temp_dir().join(format!("lsm-resource-plan-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let model = root.join("model.gguf");
        fs::write(&model, vec![0_u8; 2 * MIB as usize]).unwrap();
        let plan = plan_instance_resources(&config_with_model(&model), "cpu", capacity(16_384, 0));
        assert_eq!(plan.vram.required.max_bytes, 0);
        assert_ne!(plan.status, "infeasible");
        assert_eq!(plan.confidence, "low");
        assert!(plan
            .reasons
            .contains(&"model_metadata_unavailable".to_string()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn draft_and_projector_artifacts_are_included() {
        let root =
            std::env::temp_dir().join(format!("lsm-resource-extra-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let model = root.join("model.gguf");
        let projector = root.join("projector.gguf");
        let draft = root.join("draft.gguf");
        fs::write(&model, vec![0_u8; 1024]).unwrap();
        fs::write(&projector, vec![0_u8; 2048]).unwrap();
        fs::write(&draft, vec![0_u8; 4096]).unwrap();
        let config = InstanceConfig {
            mmproj_path: projector.to_string_lossy().to_string(),
            draft_model_path: draft.to_string_lossy().to_string(),
            ..config_with_model(&model)
        };
        let plan = plan_instance_resources(&config, "cpu", capacity(16_384, 0));
        assert!(plan
            .components
            .iter()
            .any(|value| value.kind == "projector_weights"));
        assert!(plan
            .components
            .iter()
            .any(|value| value.kind == "draft_weights"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn complete_shards_are_summed_and_incomplete_sets_are_unknown() {
        let root =
            std::env::temp_dir().join(format!("lsm-resource-shards-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let first = root.join("model-00001-of-00002.gguf");
        let second = root.join("model-00002-of-00002.gguf");
        fs::write(&first, vec![0_u8; 1024]).unwrap();
        fs::write(&second, vec![0_u8; 2048]).unwrap();
        let mut notes = PlanNotes::default();
        let complete = inspect_artifact(first.to_str().unwrap(), "model", false, &mut notes);
        assert_eq!(complete.bytes, ResourceRange::exact(3072));
        assert!(complete.exact);
        fs::remove_file(&second).unwrap();
        let mut notes = PlanNotes::default();
        let incomplete = inspect_artifact(first.to_str().unwrap(), "model", false, &mut notes);
        assert!(!incomplete.exact);
        assert!(notes.force_unknown);
        assert!(notes
            .reasons
            .contains(&"incomplete_model_shards".to_string()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn explicit_insufficient_ram_is_infeasible() {
        let root = std::env::temp_dir().join(format!("lsm-resource-ram-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let model = root.join("large.gguf");
        fs::write(&model, vec![0_u8; 8 * MIB as usize]).unwrap();
        let mut config = config_with_model(&model);
        config.load_mode = "mlock".into();
        let plan = plan_instance_resources(&config, "cpu", capacity(256, 0));
        assert_eq!(plan.status, "infeasible");
        assert!(plan
            .reasons
            .contains(&"insufficient_available_ram".to_string()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn manual_and_multi_gpu_plans_never_claim_certainty() {
        let config = InstanceConfig {
            launch_mode: "manual".into(),
            tensor_split: "1,1".into(),
            ..InstanceConfig::default()
        };
        let plan = plan_instance_resources(&config, "cuda", capacity(16_384, 16_384));
        assert_eq!(plan.status, "unknown");
        assert_eq!(plan.confidence, "low");
        assert!(plan
            .reasons
            .contains(&"manual_command_not_inspectable".to_string()));
        assert!(plan
            .reasons
            .contains(&"per_device_gpu_capacity_not_measured".to_string()));
    }

    #[test]
    fn metal_without_dedicated_vram_uses_the_unified_ram_budget() {
        let root =
            std::env::temp_dir().join(format!("lsm-resource-metal-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let model = root.join("model.gguf");
        fs::write(&model, vec![0_u8; 2 * MIB as usize]).unwrap();
        let mut config = config_with_model(&model);
        config.gpu_layers = 99;
        let capacity = CapacitySnapshot {
            ram_total_bytes: Some(16_384 * MIB),
            ram_available_bytes: Some(12_000 * MIB),
            ..CapacitySnapshot::default()
        };
        let plan = plan_instance_resources(&config, "Metal", capacity);
        assert_eq!(plan.vram.required.max_bytes, 0);
        assert!(plan
            .components
            .iter()
            .any(|value| value.target == "unified"));
        assert!(!plan
            .reasons
            .contains(&"vram_capacity_unavailable".to_string()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn report_never_serializes_paths_or_secrets() {
        let root =
            std::env::temp_dir().join(format!("lsm-resource-secret-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let model = root.join("private-model.gguf");
        fs::write(&model, vec![0_u8; 1024]).unwrap();
        let config = InstanceConfig {
            api_key: "top-secret-api-key".into(),
            custom_args: vec!["--api-key another-secret".into()],
            ..config_with_model(&model)
        };
        let serialized = serde_json::to_string(&plan_instance_resources(
            &config,
            "cpu",
            capacity(16_384, 0),
        ))
        .unwrap();
        assert!(!serialized.contains("private-model"));
        assert!(!serialized.contains("top-secret"));
        assert!(!serialized.contains("another-secret"));
        let _ = fs::remove_dir_all(root);
    }
}
