use crate::models::InstanceConfig;
use crate::vector_policy::ModelWorkload;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointPhase {
    #[default]
    Disabled,
    Ineligible,
    Starting,
    EngineHealthy,
    Restoring,
    Ready,
    ReadyCold,
    Draining,
    Saving,
    Stopping,
    Stopped,
}

impl CheckpointPhase {
    pub const fn is_routable(self) -> bool {
        matches!(self, Self::Ready | Self::ReadyCold)
    }

    pub const fn is_busy(self) -> bool {
        matches!(self, Self::Restoring | Self::Saving)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointOperation {
    #[default]
    None,
    Save,
    Restore,
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointOutcome {
    #[default]
    None,
    Success,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointReasonCode {
    #[default]
    None,
    Disabled,
    UnsupportedConfiguration,
    ManagedLocalRequired,
    ManualLaunchUnsupported,
    CustomArgumentsUnsupported,
    MultiModelUnsupported,
    VectorWorkloadUnsupported,
    ParallelMustBeOne,
    PromptCacheRequired,
    SlotsRequired,
    LoopbackHttpRequired,
    CustomEndpointUnsupported,
    EngineCapabilityMissing,
    SpeculativeDecodingUnsupported,
    LoraUnsupported,
    MultimodalUnsupported,
    HybridRecurrentUnsupported,
    ModelArchitectureUnknown,
    ShardedModelUnsupported,
    ConflictingSlotSavePath,
    FingerprintUnavailable,
    FingerprintMismatch,
    NoCheckpoint,
    AutoSaveDisabled,
    AutoRestoreDisabled,
    BelowTokenThreshold,
    BusyTimeout,
    ChecksumMismatch,
    ManifestInvalid,
    RestoreResponseInvalid,
    SlotStateMismatch,
    StorageLimit,
    IoError,
    HttpTimeout,
    SlotApiError,
    StaleProcessEvent,
    UnexpectedExit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointStatus {
    pub instance_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_pid: Option<u32>,
    pub phase: CheckpointPhase,
    pub routable: bool,
    pub last_operation: CheckpointOperation,
    pub last_outcome: CheckpointOutcome,
    pub reason_code: CheckpointReasonCode,
    #[serde(default)]
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub updated_at: u64,
}

impl CheckpointStatus {
    pub fn disabled(instance_id: impl Into<String>, updated_at: u64) -> Self {
        Self {
            instance_id: instance_id.into(),
            expected_pid: None,
            phase: CheckpointPhase::Disabled,
            routable: false,
            last_operation: CheckpointOperation::None,
            last_outcome: CheckpointOutcome::None,
            reason_code: CheckpointReasonCode::Disabled,
            message: String::new(),
            generation_id: None,
            prompt_tokens: None,
            bytes: None,
            duration_ms: None,
            updated_at,
        }
    }

    pub fn with_phase(mut self, phase: CheckpointPhase) -> Self {
        self.phase = phase;
        self.routable = phase.is_routable();
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EngineCheckpointCapabilities {
    pub slots: bool,
    pub slot_save_path: bool,
}

impl EngineCheckpointCapabilities {
    pub fn from_supported_flags(flags: &[String]) -> Self {
        let has = |expected: &str| {
            flags
                .iter()
                .any(|flag| flag.trim().eq_ignore_ascii_case(expected))
        };
        Self {
            slots: has("--slots"),
            slot_save_path: has("--slot-save-path"),
        }
    }

    pub const fn complete(self) -> bool {
        self.slots && self.slot_save_path
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointEligibility {
    pub eligible: bool,
    pub reason_code: CheckpointReasonCode,
    pub reasons: Vec<CheckpointReasonCode>,
}

impl CheckpointEligibility {
    fn from_reasons(reasons: Vec<CheckpointReasonCode>) -> Self {
        let reason_code = reasons.first().copied().unwrap_or_default();
        Self {
            eligible: reasons.is_empty(),
            reason_code,
            reasons,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CheckpointEligibilityContext<'a> {
    pub config: &'a InstanceConfig,
    pub workload: ModelWorkload,
    pub managed_local_engine: bool,
    pub engine_capabilities: EngineCheckpointCapabilities,
    pub model_architecture: Option<&'a str>,
    pub model_is_sharded: bool,
}

fn is_loopback_host(host: &str) -> bool {
    let trimmed = host.trim();
    if trimmed.eq_ignore_ascii_case("localhost") || trimmed.eq_ignore_ascii_case("localhost.") {
        return true;
    }
    let unwrapped = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(trimmed);
    unwrapped
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

fn is_known_hybrid_or_recurrent(architecture: &str) -> bool {
    let normalized: String = architecture
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    const UNSUPPORTED_HINTS: &[&str] = &[
        "qwen3next",
        "recurrentgemma",
        "mamba",
        "jamba",
        "rwkv",
        "falconh1",
        "hgrn",
        "hymba",
        "granitehybrid",
    ];
    UNSUPPORTED_HINTS
        .iter()
        .any(|hint| normalized.contains(hint))
}

fn push_reason(reasons: &mut Vec<CheckpointReasonCode>, reason: CheckpointReasonCode) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

pub fn evaluate_checkpoint_eligibility(
    context: CheckpointEligibilityContext<'_>,
) -> CheckpointEligibility {
    let config = context.config;
    if !config.kv_checkpoint.enabled {
        return CheckpointEligibility::from_reasons(vec![CheckpointReasonCode::Disabled]);
    }

    let mut reasons = Vec::new();
    if !config.launch_mode.eq_ignore_ascii_case("managed") {
        push_reason(&mut reasons, CheckpointReasonCode::ManualLaunchUnsupported);
    }
    if !context.managed_local_engine {
        push_reason(&mut reasons, CheckpointReasonCode::ManagedLocalRequired);
    }
    if !config.custom_args.is_empty() {
        push_reason(
            &mut reasons,
            CheckpointReasonCode::CustomArgumentsUnsupported,
        );
    }
    if !config.models_dir.trim().is_empty() || !config.models_preset.trim().is_empty() {
        push_reason(&mut reasons, CheckpointReasonCode::MultiModelUnsupported);
    }
    if context.workload != ModelWorkload::Inference || config.embedding || config.reranking {
        push_reason(
            &mut reasons,
            CheckpointReasonCode::VectorWorkloadUnsupported,
        );
    }
    if config.parallel != 1 {
        push_reason(&mut reasons, CheckpointReasonCode::ParallelMustBeOne);
    }
    if !config.cache_prompt {
        push_reason(&mut reasons, CheckpointReasonCode::PromptCacheRequired);
    }
    if !config.slots_enabled {
        push_reason(&mut reasons, CheckpointReasonCode::SlotsRequired);
    }
    if !config.slot_save_path.trim().is_empty() {
        push_reason(&mut reasons, CheckpointReasonCode::ConflictingSlotSavePath);
    }
    if !is_loopback_host(&config.host)
        || !config.ssl_key_file.trim().is_empty()
        || !config.ssl_cert_file.trim().is_empty()
    {
        push_reason(&mut reasons, CheckpointReasonCode::LoopbackHttpRequired);
    }
    if !config.path_prefix.trim().is_empty() || !config.api_prefix.trim().is_empty() {
        push_reason(
            &mut reasons,
            CheckpointReasonCode::CustomEndpointUnsupported,
        );
    }
    if !context.engine_capabilities.complete() {
        push_reason(&mut reasons, CheckpointReasonCode::EngineCapabilityMissing);
    }
    if !config.draft_model_path.trim().is_empty()
        || !config.spec_type.trim().is_empty()
        || !config.lookup_cache_static.trim().is_empty()
        || !config.lookup_cache_dynamic.trim().is_empty()
        || config.spec_default
    {
        push_reason(
            &mut reasons,
            CheckpointReasonCode::SpeculativeDecodingUnsupported,
        );
    }
    if !config.lora_path.trim().is_empty()
        || config.lora_init_without_apply
        || !config.lora_scaled.trim().is_empty()
    {
        push_reason(&mut reasons, CheckpointReasonCode::LoraUnsupported);
    }
    if !config.mmproj_path.trim().is_empty()
        || !config.mmproj_url.trim().is_empty()
        || config.mmproj_auto
        || !config.mmproj_mode.trim().is_empty()
        || !config.media_path.trim().is_empty()
    {
        push_reason(&mut reasons, CheckpointReasonCode::MultimodalUnsupported);
    }
    if context.model_is_sharded {
        push_reason(&mut reasons, CheckpointReasonCode::ShardedModelUnsupported);
    }
    match context.model_architecture.map(str::trim) {
        None | Some("") => {
            push_reason(&mut reasons, CheckpointReasonCode::ModelArchitectureUnknown)
        }
        Some(architecture) if is_known_hybrid_or_recurrent(architecture) => push_reason(
            &mut reasons,
            CheckpointReasonCode::HybridRecurrentUnsupported,
        ),
        Some(_) => {}
    }

    CheckpointEligibility::from_reasons(reasons)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{InstanceConfig, KvCheckpointConfig};

    fn eligible_config() -> InstanceConfig {
        InstanceConfig {
            model_path: "model.gguf".into(),
            parallel: 1,
            kv_checkpoint: KvCheckpointConfig {
                enabled: true,
                ..KvCheckpointConfig::default()
            },
            ..InstanceConfig::default()
        }
    }

    fn evaluate(config: &InstanceConfig) -> CheckpointEligibility {
        evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
            config,
            workload: ModelWorkload::Inference,
            managed_local_engine: true,
            engine_capabilities: EngineCheckpointCapabilities {
                slots: true,
                slot_save_path: true,
            },
            model_architecture: Some("llama"),
            model_is_sharded: false,
        })
    }

    fn assert_config_reason(
        reason: CheckpointReasonCode,
        mutate: impl FnOnce(&mut InstanceConfig),
    ) {
        let mut config = eligible_config();
        mutate(&mut config);
        let result = evaluate(&config);
        assert!(!result.eligible);
        assert!(
            result.reasons.contains(&reason),
            "expected {reason:?}, got {:?}",
            result.reasons
        );
    }

    #[test]
    fn disabled_config_is_legacy_safe() {
        let config = InstanceConfig::default();
        let result = evaluate(&config);
        assert!(!result.eligible);
        assert_eq!(result.reason_code, CheckpointReasonCode::Disabled);
        assert_eq!(result.reasons, vec![CheckpointReasonCode::Disabled]);
    }

    #[test]
    fn conservative_supported_candidate_is_eligible() {
        let result = evaluate(&eligible_config());
        assert!(result.eligible);
        assert_eq!(result.reason_code, CheckpointReasonCode::None);
        assert!(result.reasons.is_empty());
    }

    #[test]
    fn eligibility_rejects_every_unsupported_config_row() {
        assert_config_reason(CheckpointReasonCode::ManualLaunchUnsupported, |config| {
            config.launch_mode = "manual".into();
        });
        assert_config_reason(CheckpointReasonCode::CustomArgumentsUnsupported, |config| {
            config.custom_args = vec!["--unknown".into()];
        });
        assert_config_reason(CheckpointReasonCode::MultiModelUnsupported, |config| {
            config.models_preset = "router.json".into();
        });
        assert_config_reason(CheckpointReasonCode::ParallelMustBeOne, |config| {
            config.parallel = 2;
        });
        assert_config_reason(CheckpointReasonCode::PromptCacheRequired, |config| {
            config.cache_prompt = false;
        });
        assert_config_reason(CheckpointReasonCode::SlotsRequired, |config| {
            config.slots_enabled = false;
        });
        assert_config_reason(CheckpointReasonCode::ConflictingSlotSavePath, |config| {
            config.slot_save_path = "user-controlled".into();
        });
        assert_config_reason(CheckpointReasonCode::LoopbackHttpRequired, |config| {
            config.host = "0.0.0.0".into();
        });
        assert_config_reason(CheckpointReasonCode::LoopbackHttpRequired, |config| {
            config.ssl_cert_file = "server.pem".into();
        });
        assert_config_reason(CheckpointReasonCode::CustomEndpointUnsupported, |config| {
            config.api_prefix = "/llama".into();
        });
        assert_config_reason(
            CheckpointReasonCode::SpeculativeDecodingUnsupported,
            |config| config.spec_type = "draft".into(),
        );
        assert_config_reason(CheckpointReasonCode::LoraUnsupported, |config| {
            config.lora_path = "adapter.gguf".into();
        });
        assert_config_reason(CheckpointReasonCode::MultimodalUnsupported, |config| {
            config.mmproj_path = "mmproj.gguf".into();
        });
    }

    #[test]
    fn eligibility_rejects_context_and_engine_boundaries() {
        let config = eligible_config();
        let base = CheckpointEligibilityContext {
            config: &config,
            workload: ModelWorkload::Inference,
            managed_local_engine: true,
            engine_capabilities: EngineCheckpointCapabilities {
                slots: true,
                slot_save_path: true,
            },
            model_architecture: Some("llama"),
            model_is_sharded: false,
        };

        let remote = evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
            managed_local_engine: false,
            ..base
        });
        assert!(remote
            .reasons
            .contains(&CheckpointReasonCode::ManagedLocalRequired));

        let vector = evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
            workload: ModelWorkload::Embedding,
            ..base
        });
        assert!(vector
            .reasons
            .contains(&CheckpointReasonCode::VectorWorkloadUnsupported));

        let missing_capability = evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
            engine_capabilities: EngineCheckpointCapabilities {
                slots: true,
                slot_save_path: false,
            },
            ..base
        });
        assert!(missing_capability
            .reasons
            .contains(&CheckpointReasonCode::EngineCapabilityMissing));

        let hybrid = evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
            model_architecture: Some("qwen3-next"),
            ..base
        });
        assert!(hybrid
            .reasons
            .contains(&CheckpointReasonCode::HybridRecurrentUnsupported));

        let unknown = evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
            model_architecture: None,
            ..base
        });
        assert!(unknown
            .reasons
            .contains(&CheckpointReasonCode::ModelArchitectureUnknown));

        let sharded = evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
            model_is_sharded: true,
            ..base
        });
        assert!(sharded
            .reasons
            .contains(&CheckpointReasonCode::ShardedModelUnsupported));
    }

    #[test]
    fn engine_capabilities_require_both_official_flags() {
        let flags = vec!["--slots".into(), "--slot-save-path".into()];
        assert!(EngineCheckpointCapabilities::from_supported_flags(&flags).complete());
        let incomplete = vec!["--slots".into()];
        assert!(!EngineCheckpointCapabilities::from_supported_flags(&incomplete).complete());
    }

    #[test]
    fn loopback_host_accepts_ipv4_ipv6_and_localhost_only() {
        for host in ["127.0.0.1", "127.7.8.9", "::1", "[::1]", "localhost"] {
            let mut config = eligible_config();
            config.host = host.into();
            assert!(evaluate(&config).eligible, "expected loopback host: {host}");
        }
        for host in ["0.0.0.0", "::", "192.168.1.10", "example.test"] {
            let mut config = eligible_config();
            config.host = host.into();
            assert!(evaluate(&config)
                .reasons
                .contains(&CheckpointReasonCode::LoopbackHttpRequired));
        }
    }

    #[test]
    fn status_contract_serializes_stable_machine_values() {
        let status =
            CheckpointStatus::disabled("instance-1", 123).with_phase(CheckpointPhase::ReadyCold);
        let value = serde_json::to_value(status).unwrap();
        assert_eq!(value["phase"], "ready_cold");
        assert_eq!(value["routable"], true);
        assert_eq!(value["reason_code"], "disabled");
        assert_eq!(value["last_operation"], "none");
        assert_eq!(value["last_outcome"], "none");
        assert!(value.get("expected_pid").is_none());
        assert!(CheckpointPhase::Restoring.is_busy());
        assert!(CheckpointPhase::Saving.is_busy());
        assert!(!CheckpointPhase::Starting.is_busy());
    }

    #[test]
    fn required_failure_reason_codes_remain_stable() {
        let cases = [
            (
                CheckpointReasonCode::UnsupportedConfiguration,
                "unsupported_configuration",
            ),
            (
                CheckpointReasonCode::EngineCapabilityMissing,
                "engine_capability_missing",
            ),
            (
                CheckpointReasonCode::FingerprintUnavailable,
                "fingerprint_unavailable",
            ),
            (
                CheckpointReasonCode::FingerprintMismatch,
                "fingerprint_mismatch",
            ),
            (CheckpointReasonCode::NoCheckpoint, "no_checkpoint"),
            (
                CheckpointReasonCode::BelowTokenThreshold,
                "below_token_threshold",
            ),
            (CheckpointReasonCode::BusyTimeout, "busy_timeout"),
            (CheckpointReasonCode::ChecksumMismatch, "checksum_mismatch"),
            (CheckpointReasonCode::ManifestInvalid, "manifest_invalid"),
            (
                CheckpointReasonCode::RestoreResponseInvalid,
                "restore_response_invalid",
            ),
            (
                CheckpointReasonCode::SlotStateMismatch,
                "slot_state_mismatch",
            ),
            (CheckpointReasonCode::StorageLimit, "storage_limit"),
            (CheckpointReasonCode::IoError, "io_error"),
            (CheckpointReasonCode::HttpTimeout, "http_timeout"),
        ];
        for (reason, expected) in cases {
            assert_eq!(serde_json::to_value(reason).unwrap(), expected);
        }
    }
}
