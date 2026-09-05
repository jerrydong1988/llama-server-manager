use crate::model_artifacts::{resolve_engine_runtime_artifacts, resolve_model_artifacts};
use crate::models::{
    InstanceConfig, KV_CHECKPOINT_MINIMUM_PROMPT_TOKENS_MAX,
    KV_CHECKPOINT_MINIMUM_PROMPT_TOKENS_MIN, KV_CHECKPOINT_STORAGE_LIMIT_GIB_MAX,
    KV_CHECKPOINT_STORAGE_LIMIT_GIB_MIN,
};
use crate::speculative::{
    checkpoint_speculative_types_supported, checkpoint_uses_draft_state,
    normalize_speculative_types,
};
use crate::vector_policy::ModelWorkload;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

pub const CHECKPOINT_SLOT_OPERATION_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CHECKPOINT_SLOT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const CHECKPOINT_SLOT_CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
const CHECKPOINT_DISK_HEADROOM_BYTES: u64 = 64 * 1024 * 1024;

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

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Ineligible => "ineligible",
            Self::Starting => "starting",
            Self::EngineHealthy => "engine_healthy",
            Self::Restoring => "restoring",
            Self::Ready => "ready",
            Self::ReadyCold => "ready_cold",
            Self::Draining => "draining",
            Self::Saving => "saving",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
        }
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
    PromptCacheRetentionRequired,
    SlotsRequired,
    LoopbackHttpRequired,
    CustomEndpointUnsupported,
    EngineCapabilityMissing,
    SpeculativeDecodingUnsupported,
    LoraUnsupported,
    MultimodalUnsupported,
    HybridRecurrentUnsupported,
    SlidingWindowRequiresFullCache,
    ModelArchitectureUnknown,
    ShardedModelUnsupported,
    ModelArtifactsIncomplete,
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
    SaveResponseInvalid,
    RestoreResponseInvalid,
    SlotStateMismatch,
    StorageLimit,
    InsufficientDiskSpace,
    IoError,
    HttpTimeout,
    SlotApiError,
    StaleProcessEvent,
    InvalidStateTransition,
    ClearWhileRunning,
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
    pub cache_ram: bool,
    pub cache_idle_slots: bool,
    pub swa_full: bool,
    pub context_checkpoint_persistence: bool,
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
            cache_ram: has("--cache-ram"),
            cache_idle_slots: has("--cache-idle-slots"),
            swa_full: has("--swa-full"),
            context_checkpoint_persistence: false,
        }
    }

    pub const fn with_context_checkpoint_persistence(mut self, supported: bool) -> Self {
        self.context_checkpoint_persistence = supported;
        self
    }

    pub const fn complete(self) -> bool {
        self.slots && self.slot_save_path && self.cache_ram && self.cache_idle_slots
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointEligibility {
    pub eligible: bool,
    pub reason_code: CheckpointReasonCode,
    pub reasons: Vec<CheckpointReasonCode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_argument_blockers: Vec<String>,
}

impl CheckpointEligibility {
    fn from_reasons(reasons: Vec<CheckpointReasonCode>) -> Self {
        Self::from_reasons_and_blockers(reasons, Vec::new())
    }

    fn from_reasons_and_blockers(
        reasons: Vec<CheckpointReasonCode>,
        custom_argument_blockers: Vec<String>,
    ) -> Self {
        let reason_code = reasons.first().copied().unwrap_or_default();
        Self {
            eligible: reasons.is_empty(),
            reason_code,
            reasons,
            custom_argument_blockers,
        }
    }

    pub fn ineligible(reason_code: CheckpointReasonCode) -> Self {
        Self::from_reasons(vec![reason_code])
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CheckpointEligibilityContext<'a> {
    pub config: &'a InstanceConfig,
    pub workload: ModelWorkload,
    pub managed_local_engine: bool,
    pub engine_capabilities: EngineCheckpointCapabilities,
    pub engine_speculative_types: &'a [String],
    pub model_architecture: Option<&'a str>,
    pub model_artifacts_complete: bool,
    pub model_has_swa: Option<bool>,
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
        "qwen4exp",
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

const CHECKPOINT_SAFE_LAZY_FLAGS: &[&str] = &["--lazy-mode", "-lzm", "--tensor-read-lazy"];

fn push_custom_argument_blocker(blockers: &mut Vec<String>, blocker: &str) {
    if !blockers.iter().any(|existing| existing == blocker) {
        blockers.push(blocker.to_string());
    }
}

fn looks_like_flag(token: &str) -> bool {
    token.starts_with('-')
        && token
            .trim_start_matches('-')
            .chars()
            .any(|character| character.is_ascii_alphabetic())
}

fn checkpoint_custom_argument_blockers(config: &InstanceConfig) -> Vec<String> {
    let mut blockers = Vec::new();
    let mut tokens = Vec::new();
    for row in &config.custom_args {
        match crate::utils::split_command_line_checked(row) {
            Ok(parsed) => tokens.extend(parsed),
            Err(_) => push_custom_argument_blocker(&mut blockers, "<malformed-custom-arguments>"),
        }
    }

    let structured_lazy_mode = config.lazy_mode.trim().to_ascii_lowercase();
    let mut observed_lazy_mode: Option<String> = None;
    let mut pending_unknown_value = false;
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        let (flag, inline_value) = token
            .split_once('=')
            .map_or((token.as_str(), None), |(flag, value)| (flag, Some(value)));
        if CHECKPOINT_SAFE_LAZY_FLAGS.contains(&flag) {
            pending_unknown_value = false;
            let mut consumed_next = false;
            let value = if let Some(value) = inline_value {
                Some(value)
            } else {
                let next = tokens.get(index + 1).map(String::as_str);
                consumed_next = next.is_some_and(|candidate| !looks_like_flag(candidate));
                if consumed_next {
                    next
                } else {
                    None
                }
            };
            let normalized = value.map(str::trim).map(str::to_ascii_lowercase);
            let valid = normalized
                .as_deref()
                .is_some_and(|mode| matches!(mode, "auto" | "on" | "off"));
            let conflicts_with_structured = valid
                && !structured_lazy_mode.is_empty()
                && normalized.as_deref() != Some(structured_lazy_mode.as_str());
            let conflicts_with_custom = valid
                && observed_lazy_mode
                    .as_deref()
                    .is_some_and(|mode| normalized.as_deref() != Some(mode));
            if !valid || conflicts_with_structured || conflicts_with_custom {
                push_custom_argument_blocker(&mut blockers, flag);
            } else if observed_lazy_mode.is_none() {
                observed_lazy_mode = normalized;
            }
            index += if consumed_next { 2 } else { 1 };
            continue;
        }
        if looks_like_flag(flag) {
            push_custom_argument_blocker(&mut blockers, flag);
            pending_unknown_value = inline_value.is_none();
        } else if pending_unknown_value {
            pending_unknown_value = false;
        } else {
            push_custom_argument_blocker(&mut blockers, "<positional-argument>");
        }
        index += 1;
    }
    blockers
}

pub fn evaluate_checkpoint_eligibility(
    context: CheckpointEligibilityContext<'_>,
) -> CheckpointEligibility {
    let config = context.config;
    if !config.kv_checkpoint.enabled {
        return CheckpointEligibility::from_reasons(vec![CheckpointReasonCode::Disabled]);
    }

    let mut reasons = Vec::new();
    if !(KV_CHECKPOINT_STORAGE_LIMIT_GIB_MIN..=KV_CHECKPOINT_STORAGE_LIMIT_GIB_MAX)
        .contains(&config.kv_checkpoint.storage_limit_gib)
        || !(KV_CHECKPOINT_MINIMUM_PROMPT_TOKENS_MIN..=KV_CHECKPOINT_MINIMUM_PROMPT_TOKENS_MAX)
            .contains(&config.kv_checkpoint.minimum_prompt_tokens)
    {
        push_reason(&mut reasons, CheckpointReasonCode::UnsupportedConfiguration);
    }
    if !config.launch_mode.eq_ignore_ascii_case("managed") {
        push_reason(&mut reasons, CheckpointReasonCode::ManualLaunchUnsupported);
    }
    if !context.managed_local_engine {
        push_reason(&mut reasons, CheckpointReasonCode::ManagedLocalRequired);
    }
    let custom_argument_blockers = checkpoint_custom_argument_blockers(config);
    if !custom_argument_blockers.is_empty() {
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
    if !config.cache_idle_slots || !(config.cache_ram == -1 || config.cache_ram > 0) {
        push_reason(
            &mut reasons,
            CheckpointReasonCode::PromptCacheRetentionRequired,
        );
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
    if (!config.draft_model_path.trim().is_empty()
        && !checkpoint_uses_draft_state(&config.spec_type))
        || !checkpoint_speculative_types_supported(
            &config.spec_type,
            context.engine_speculative_types,
            context.engine_capabilities.context_checkpoint_persistence,
        )
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
    if !context.model_artifacts_complete {
        push_reason(&mut reasons, CheckpointReasonCode::ModelArtifactsIncomplete);
    }
    if context.model_has_swa == Some(true) && !config.swa_full {
        push_reason(
            &mut reasons,
            CheckpointReasonCode::SlidingWindowRequiresFullCache,
        );
    }
    if context.model_has_swa == Some(true)
        && config.swa_full
        && !context.engine_capabilities.swa_full
    {
        push_reason(&mut reasons, CheckpointReasonCode::EngineCapabilityMissing);
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

    CheckpointEligibility::from_reasons_and_blockers(reasons, custom_argument_blockers)
}

pub const CHECKPOINT_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const CHECKPOINT_FINGERPRINT_SCHEMA_VERSION: u32 = 3;
const HASH_CACHE_SCHEMA_VERSION: u32 = 2;
const LATEST_POINTER_SCHEMA_VERSION: u32 = 1;
const USAGE_SCHEMA_VERSION: u32 = 1;
const STATE_FORMAT: &str = "llama.cpp-slot-state";
const SLOT_FILENAME: &str = "slot-0.bin";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 16 * 1024 * 1024;
const MAX_HASH_CACHE_ENTRIES: usize = 256;
const HASH_CACHE_TTL_SECS: u64 = 90 * 24 * 60 * 60;

fn is_manager_slot_filename(filename: &str) -> bool {
    !filename.is_empty()
        && filename.len() <= 128
        && filename.starts_with("slot-0-")
        && filename.ends_with(".bin")
        && filename
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointStoreError {
    pub reason_code: CheckpointReasonCode,
    pub message: &'static str,
}

impl CheckpointStoreError {
    pub(crate) fn new(reason_code: CheckpointReasonCode, message: &'static str) -> Self {
        Self {
            reason_code,
            message,
        }
    }

    fn io(message: &'static str) -> Self {
        Self::new(CheckpointReasonCode::IoError, message)
    }

    fn manifest(message: &'static str) -> Self {
        Self::new(CheckpointReasonCode::ManifestInvalid, message)
    }
}

impl std::fmt::Display for CheckpointStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for CheckpointStoreError {}

type StoreResult<T> = Result<T, CheckpointStoreError>;

fn sha256_bytes(contents: &[u8]) -> String {
    let digest = Sha256::digest(contents);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn sha256_file(path: &Path) -> StoreResult<String> {
    validate_regular_file(path)?;
    let file = File::open(path).map_err(|_| CheckpointStoreError::io("file open failed"))?;
    let mut reader = BufReader::with_capacity(1024 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| CheckpointStoreError::io("file read failed"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    Ok(encoded)
}

fn is_lower_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn validate_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|parsed| parsed.to_string() == value)
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn validate_regular_file(path: &Path) -> StoreResult<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| CheckpointStoreError::io("file inspection failed"))?;
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) || !metadata.is_file() {
        return Err(CheckpointStoreError::manifest(
            "checkpoint payload is not a regular file",
        ));
    }
    Ok(metadata)
}

fn validate_directory(path: &Path) -> StoreResult<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| CheckpointStoreError::io("directory inspection failed"))?;
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) || !metadata.is_dir() {
        return Err(CheckpointStoreError::manifest(
            "checkpoint directory is not a regular directory",
        ));
    }
    Ok(metadata)
}

#[cfg(unix)]
fn protect_directory(path: &Path) -> StoreResult<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| CheckpointStoreError::io("directory permission update failed"))
}

#[cfg(not(unix))]
fn protect_directory(_path: &Path) -> StoreResult<()> {
    Ok(())
}

#[cfg(unix)]
fn protect_file(path: &Path) -> StoreResult<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| CheckpointStoreError::io("file permission update failed"))
}

#[cfg(not(unix))]
fn protect_file(_path: &Path) -> StoreResult<()> {
    Ok(())
}

#[cfg(windows)]
fn protect_windows_checkpoint_root(path: &Path) -> StoreResult<()> {
    use std::process::{Command, Stdio};

    let identity = Command::new("whoami.exe")
        .args(["/user", "/fo", "csv", "/nh"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| CheckpointStoreError::io("current user identity lookup failed"))?;
    if !identity.status.success() {
        return Err(CheckpointStoreError::io(
            "current user identity lookup failed",
        ));
    }
    let output = String::from_utf8_lossy(&identity.stdout);
    let sid = output
        .trim()
        .rsplit(',')
        .next()
        .map(|value| value.trim().trim_matches('"'))
        .filter(|value| value.starts_with("S-1-"))
        .ok_or_else(|| CheckpointStoreError::io("current user identity lookup failed"))?;
    // Existing files need an effective ACE of their own. Applying only an
    // inheritable (OI)(CI) ACE recursively leaves regular files with an empty
    // DACL on Windows, because files cannot pass that ACE to children. This is
    // observable when the main process and runtime supervisor each initialize
    // a CheckpointStore: the second pass makes fingerprints-v1.json unreadable.
    let direct_grant = format!("*{sid}:F");
    let inheritable_grant = format!("*{sid}:(OI)(CI)F");
    let direct_status = Command::new("icacls.exe")
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(direct_grant)
        .args(["/T", "/Q"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| CheckpointStoreError::io("checkpoint ACL update failed"))?;
    if !direct_status.success() {
        return Err(CheckpointStoreError::io("checkpoint ACL update failed"));
    }

    // Add inheritance separately so every existing directory also protects
    // files created after this pass. The direct ACE above remains effective on
    // regular files, making repeated hardening safe and idempotent.
    let inheritable_status = Command::new("icacls.exe")
        .arg(path)
        .arg("/grant")
        .arg(inheritable_grant)
        .args(["/T", "/Q"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| CheckpointStoreError::io("checkpoint ACL update failed"))?;
    if !inheritable_status.success() {
        return Err(CheckpointStoreError::io("checkpoint ACL update failed"));
    }
    Ok(())
}

#[cfg(not(windows))]
fn protect_windows_checkpoint_root(_path: &Path) -> StoreResult<()> {
    Ok(())
}

fn ensure_private_directory(path: &Path) -> StoreResult<()> {
    fs::create_dir_all(path)
        .map_err(|_| CheckpointStoreError::io("checkpoint directory creation failed"))?;
    validate_directory(path)?;
    protect_directory(path)
}

fn sync_directory(path: &Path) -> StoreResult<()> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| CheckpointStoreError::io("directory sync failed"))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn is_path_within(candidate: &Path, root: &Path) -> bool {
    let candidate_components: Vec<_> = candidate.components().collect();
    let root_components: Vec<_> = root.components().collect();
    if candidate_components.len() < root_components.len() {
        return false;
    }
    if cfg!(windows) {
        candidate_components
            .iter()
            .zip(root_components.iter())
            .all(|(left, right)| {
                left.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
            })
    } else {
        candidate_components.starts_with(&root_components)
    }
}

fn verified_child_path(path: &Path, root: &Path) -> StoreResult<(PathBuf, PathBuf)> {
    validate_directory(root)?;
    validate_directory(path)?;
    let canonical_root = fs::canonicalize(root)
        .map_err(|_| CheckpointStoreError::io("checkpoint root resolution failed"))?;
    let canonical_path = fs::canonicalize(path)
        .map_err(|_| CheckpointStoreError::io("checkpoint path resolution failed"))?;
    if canonical_path == canonical_root || !is_path_within(&canonical_path, &canonical_root) {
        return Err(CheckpointStoreError::manifest(
            "checkpoint path escaped its private root",
        ));
    }
    Ok((canonical_path, canonical_root))
}

fn safe_remove_directory(path: &Path, root: &Path) -> StoreResult<()> {
    let (canonical_path, _) = verified_child_path(path, root)?;
    fs::remove_dir_all(canonical_path)
        .map_err(|_| CheckpointStoreError::io("checkpoint directory removal failed"))
}

fn read_bounded(path: &Path, limit: u64) -> StoreResult<Vec<u8>> {
    let metadata = validate_regular_file(path)?;
    if metadata.len() > limit {
        return Err(CheckpointStoreError::manifest(
            "checkpoint metadata exceeds its size limit",
        ));
    }
    fs::read(path).map_err(|_| CheckpointStoreError::io("checkpoint metadata read failed"))
}

fn modified_unix_nanos(metadata: &fs::Metadata) -> StoreResult<u64> {
    let nanos = metadata
        .modified()
        .and_then(|modified| {
            modified
                .duration_since(UNIX_EPOCH)
                .map_err(std::io::Error::other)
        })
        .map_err(|_| CheckpointStoreError::io("file timestamp unavailable"))?
        .as_nanos();
    Ok(nanos.min(u128::from(u64::MAX)) as u64)
}

#[cfg(unix)]
fn file_identity(_path: &Path, metadata: &fs::Metadata) -> StoreResult<String> {
    use std::os::unix::fs::MetadataExt;
    Ok(format!(
        "unix:{}:{}:{}:{}",
        metadata.dev(),
        metadata.ino(),
        metadata.ctime(),
        metadata.ctime_nsec()
    ))
}

#[cfg(windows)]
fn file_identity(path: &Path, _metadata: &fs::Metadata) -> StoreResult<String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO,
    };

    let file = File::open(path)
        .map_err(|_| CheckpointStoreError::io("fingerprint input identity unavailable"))?;
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let success =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut information) };
    if success == 0 {
        return Err(CheckpointStoreError::io(
            "fingerprint input identity unavailable",
        ));
    }
    let mut basic = FILE_BASIC_INFO::default();
    let success = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle() as HANDLE,
            FileBasicInfo,
            std::ptr::addr_of_mut!(basic).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    };
    if success == 0 {
        return Err(CheckpointStoreError::io(
            "fingerprint input change time unavailable",
        ));
    }
    let file_index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok(format!(
        "windows:{}:{file_index}:{}:{}",
        information.dwVolumeSerialNumber, basic.CreationTime, basic.ChangeTime
    ))
}

#[cfg(not(any(unix, windows)))]
fn file_identity(_path: &Path, metadata: &fs::Metadata) -> StoreResult<String> {
    let created = metadata
        .created()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |value| value.as_nanos().min(u128::from(u64::MAX)) as u64);
    Ok(format!("portable:{created}"))
}

fn ensure_available_checkpoint_space(path: &Path, payload_bytes: u64) -> StoreResult<()> {
    let available = fs2::available_space(path)
        .map_err(|_| CheckpointStoreError::io("checkpoint disk capacity query failed"))?;
    let required = payload_bytes.saturating_add(CHECKPOINT_DISK_HEADROOM_BYTES);
    if available < required {
        return Err(CheckpointStoreError::new(
            CheckpointReasonCode::InsufficientDiskSpace,
            "checkpoint storage does not have enough free space",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn canonical_path_cache_key(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    sha256_bytes(path.as_os_str().as_bytes())
}

#[cfg(windows)]
fn canonical_path_cache_key(path: &Path) -> String {
    use std::os::windows::ffi::OsStrExt;
    let mut encoded = Vec::new();
    for unit in path.as_os_str().encode_wide() {
        encoded.extend_from_slice(&unit.to_le_bytes());
    }
    sha256_bytes(&encoded)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HashCacheEntry {
    size: u64,
    modified_unix_nanos: u64,
    file_identity: String,
    sha256: String,
    #[serde(default)]
    last_used_unix_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HashCacheFile {
    schema_version: u32,
    entries: BTreeMap<String, HashCacheEntry>,
}

impl Default for HashCacheFile {
    fn default() -> Self {
        Self {
            schema_version: HASH_CACHE_SCHEMA_VERSION,
            entries: BTreeMap::new(),
        }
    }
}

fn maintain_hash_cache(cache: &mut HashCacheFile, now_unix_secs: u64) {
    for entry in cache.entries.values_mut() {
        if entry.last_used_unix_secs == 0 {
            entry.last_used_unix_secs = now_unix_secs;
        }
    }
    cache.entries.retain(|_, entry| {
        now_unix_secs.saturating_sub(entry.last_used_unix_secs) <= HASH_CACHE_TTL_SECS
    });
    while cache.entries.len() > MAX_HASH_CACHE_ENTRIES {
        let Some(oldest) = cache
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_used_unix_secs)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        cache.entries.remove(&oldest);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckpointFingerprint {
    pub algorithm: String,
    pub digest: String,
    pub model_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_model_sha256: Option<String>,
    pub engine_sha256: String,
    pub engine_version: String,
    pub backend: String,
}

impl CheckpointFingerprint {
    pub(crate) fn validate(&self) -> StoreResult<()> {
        if self.algorithm != "sha256"
            || !is_lower_hex_digest(&self.digest)
            || !is_lower_hex_digest(&self.model_sha256)
            || self
                .draft_model_sha256
                .as_deref()
                .is_some_and(|digest| !is_lower_hex_digest(digest))
            || !is_lower_hex_digest(&self.engine_sha256)
            || self.engine_version.trim().is_empty()
            || self.backend.trim().is_empty()
        {
            return Err(CheckpointStoreError::manifest(
                "checkpoint fingerprint is invalid",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintMaterials {
    pub model_sha256: String,
    pub draft_model_sha256: Option<String>,
    pub engine_sha256: String,
    pub engine_version: String,
    pub backend: String,
    pub chat_template_file_sha256: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalFingerprintV3<'a> {
    fingerprint_schema_version: u32,
    manifest_schema_version: u32,
    state_format: &'static str,
    model_sha256: &'a str,
    draft_model_sha256: Option<&'a str>,
    engine_sha256: &'a str,
    engine_version: &'a str,
    backend: &'a str,
    spec_type: String,
    cache_type_draft_k: &'a str,
    cache_type_draft_v: &'a str,
    draft_gpu_layers: u32,
    draft_tokens: u32,
    spec_draft_n_min: u32,
    spec_draft_p_min: f32,
    spec_draft_p_split: f32,
    spec_draft_device: &'a str,
    spec_draft_backend_sampling: bool,
    spec_draft_threads: u32,
    spec_draft_threads_batch: u32,
    ctx_size: u32,
    ctx_size_auto: bool,
    parallel: i32,
    cont_batching: bool,
    cache_prompt: bool,
    cache_reuse: u32,
    cache_ram: i32,
    cache_idle_slots: bool,
    ctx_checkpoints: u32,
    checkpoint_min_step: u32,
    slots_enabled: bool,
    slot_prompt_similarity: f32,
    prefill_assistant: bool,
    kv_unified: bool,
    kv_unified_mode: &'a str,
    cache_type_k: &'a str,
    cache_type_v: &'a str,
    no_kv_offload: bool,
    flash_attn: &'a str,
    swa_full: bool,
    context_shift: bool,
    rope_scaling: &'a str,
    rope_scale: f32,
    rope_freq_base: f32,
    rope_freq_scale: f32,
    yarn_ext_factor: f32,
    yarn_attn_factor: f32,
    yarn_beta_slow: f32,
    yarn_beta_fast: f32,
    yarn_orig_ctx: u32,
    batch_size: u32,
    ubatch_size: u32,
    device: &'a str,
    gpu_layers_auto: bool,
    gpu_layers: u32,
    split_mode: &'a str,
    tensor_split: &'a str,
    main_gpu: u32,
    moe_cpu_layers: u32,
    cpu_moe: bool,
    override_kv: &'a str,
    jinja: bool,
    chat_template: &'a str,
    chat_template_file_sha256: Option<&'a str>,
    skip_chat_parsing: bool,
    reasoning_format: &'a str,
    reasoning_effort: &'a str,
    reasoning: &'a str,
    reasoning_preserve: &'a str,
    reasoning_budget: &'a str,
    reasoning_budget_message: &'a str,
}

fn canonical_fingerprint_bytes(
    config: &InstanceConfig,
    materials: &FingerprintMaterials,
) -> StoreResult<Vec<u8>> {
    if !is_lower_hex_digest(&materials.model_sha256)
        || materials
            .draft_model_sha256
            .as_deref()
            .is_some_and(|digest| !is_lower_hex_digest(digest))
        || (config.draft_model_path.trim().is_empty() != materials.draft_model_sha256.is_none())
        || !is_lower_hex_digest(&materials.engine_sha256)
        || materials.engine_version.trim().is_empty()
        || materials.backend.trim().is_empty()
        || materials
            .chat_template_file_sha256
            .as_deref()
            .is_some_and(|digest| !is_lower_hex_digest(digest))
        || (!config.chat_template_file.trim().is_empty()
            && materials.chat_template_file_sha256.is_none())
    {
        return Err(CheckpointStoreError::new(
            CheckpointReasonCode::FingerprintUnavailable,
            "checkpoint fingerprint material is unavailable",
        ));
    }
    let canonical = CanonicalFingerprintV3 {
        fingerprint_schema_version: CHECKPOINT_FINGERPRINT_SCHEMA_VERSION,
        manifest_schema_version: CHECKPOINT_MANIFEST_SCHEMA_VERSION,
        state_format: STATE_FORMAT,
        model_sha256: &materials.model_sha256,
        draft_model_sha256: materials.draft_model_sha256.as_deref(),
        engine_sha256: &materials.engine_sha256,
        engine_version: materials.engine_version.trim(),
        backend: materials.backend.trim(),
        spec_type: normalize_speculative_types(&config.spec_type),
        cache_type_draft_k: config.cache_type_draft_k.trim(),
        cache_type_draft_v: config.cache_type_draft_v.trim(),
        draft_gpu_layers: config.draft_gpu_layers,
        draft_tokens: config.draft_tokens,
        spec_draft_n_min: config.spec_draft_n_min,
        spec_draft_p_min: config.spec_draft_p_min,
        spec_draft_p_split: config.spec_draft_p_split,
        spec_draft_device: config.spec_draft_device.trim(),
        spec_draft_backend_sampling: config.spec_draft_backend_sampling,
        spec_draft_threads: config.spec_draft_threads,
        spec_draft_threads_batch: config.spec_draft_threads_batch,
        ctx_size: config.ctx_size,
        ctx_size_auto: config.ctx_size_auto,
        parallel: config.parallel,
        cont_batching: config.cont_batching,
        cache_prompt: config.cache_prompt,
        cache_reuse: config.cache_reuse,
        cache_ram: config.cache_ram,
        cache_idle_slots: config.cache_idle_slots,
        ctx_checkpoints: config.ctx_checkpoints,
        checkpoint_min_step: config.checkpoint_min_step,
        slots_enabled: config.slots_enabled,
        slot_prompt_similarity: config.slot_prompt_similarity,
        prefill_assistant: config.prefill_assistant,
        kv_unified: config.kv_unified,
        kv_unified_mode: config.kv_unified_mode.trim(),
        cache_type_k: config.cache_type_k.trim(),
        cache_type_v: config.cache_type_v.trim(),
        no_kv_offload: config.no_kv_offload,
        flash_attn: config.flash_attn.trim(),
        swa_full: config.swa_full,
        context_shift: config.context_shift,
        rope_scaling: config.rope_scaling.trim(),
        rope_scale: config.rope_scale,
        rope_freq_base: config.rope_freq_base,
        rope_freq_scale: config.rope_freq_scale,
        yarn_ext_factor: config.yarn_ext_factor,
        yarn_attn_factor: config.yarn_attn_factor,
        yarn_beta_slow: config.yarn_beta_slow,
        yarn_beta_fast: config.yarn_beta_fast,
        yarn_orig_ctx: config.yarn_orig_ctx,
        batch_size: config.batch_size,
        ubatch_size: config.ubatch_size,
        device: config.device.trim(),
        gpu_layers_auto: config.gpu_layers_auto,
        gpu_layers: config.gpu_layers,
        split_mode: config.split_mode.trim(),
        tensor_split: config.tensor_split.trim(),
        main_gpu: config.main_gpu,
        moe_cpu_layers: config.moe_cpu_layers,
        cpu_moe: config.cpu_moe,
        override_kv: config.override_kv.trim(),
        jinja: config.jinja,
        chat_template: &config.chat_template,
        chat_template_file_sha256: materials.chat_template_file_sha256.as_deref(),
        skip_chat_parsing: config.skip_chat_parsing,
        reasoning_format: config.reasoning_format.trim(),
        reasoning_effort: config.reasoning_effort.trim(),
        reasoning: config.reasoning.trim(),
        reasoning_preserve: config.reasoning_preserve.trim(),
        reasoning_budget: config.reasoning_budget.trim(),
        reasoning_budget_message: &config.reasoning_budget_message,
    };
    serde_json::to_vec(&canonical).map_err(|_| {
        CheckpointStoreError::new(
            CheckpointReasonCode::FingerprintUnavailable,
            "checkpoint fingerprint serialization failed",
        )
    })
}

pub fn build_checkpoint_fingerprint(
    config: &InstanceConfig,
    materials: &FingerprintMaterials,
) -> StoreResult<CheckpointFingerprint> {
    let canonical = canonical_fingerprint_bytes(config, materials)?;
    let fingerprint = CheckpointFingerprint {
        algorithm: "sha256".into(),
        digest: sha256_bytes(&canonical),
        model_sha256: materials.model_sha256.clone(),
        draft_model_sha256: materials.draft_model_sha256.clone(),
        engine_sha256: materials.engine_sha256.clone(),
        engine_version: materials.engine_version.trim().to_string(),
        backend: materials.backend.trim().to_string(),
    };
    fingerprint.validate()?;
    Ok(fingerprint)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckpointSlotManifest {
    pub id: u32,
    pub filename: String,
    pub prompt_tokens: u64,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckpointManifestV1 {
    pub schema_version: u32,
    pub state_format: String,
    pub generation_id: String,
    pub instance_id: String,
    pub fingerprint: CheckpointFingerprint,
    pub created_at: String,
    pub slots: Vec<CheckpointSlotManifest>,
}

impl CheckpointManifestV1 {
    pub fn new(
        instance_id: impl Into<String>,
        fingerprint: CheckpointFingerprint,
        prompt_tokens: u64,
        bytes: u64,
        payload_sha256: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: CHECKPOINT_MANIFEST_SCHEMA_VERSION,
            state_format: STATE_FORMAT.into(),
            generation_id: uuid::Uuid::new_v4().to_string(),
            instance_id: instance_id.into(),
            fingerprint,
            created_at: Utc::now().to_rfc3339(),
            slots: vec![CheckpointSlotManifest {
                id: 0,
                filename: SLOT_FILENAME.into(),
                prompt_tokens,
                bytes,
                sha256: payload_sha256.into(),
            }],
        }
    }

    pub fn parse_and_validate(
        bytes: &[u8],
        expected_instance_id: &str,
        expected_fingerprint: &str,
    ) -> StoreResult<Self> {
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(CheckpointStoreError::manifest(
                "checkpoint manifest exceeds its size limit",
            ));
        }
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|_| CheckpointStoreError::manifest("checkpoint manifest is malformed"))?;
        manifest.validate(expected_instance_id, expected_fingerprint)?;
        Ok(manifest)
    }

    pub fn validate(
        &self,
        expected_instance_id: &str,
        expected_fingerprint: &str,
    ) -> StoreResult<()> {
        if self.schema_version != CHECKPOINT_MANIFEST_SCHEMA_VERSION
            || self.state_format != STATE_FORMAT
            || !validate_identifier(&self.instance_id)
            || self.instance_id != expected_instance_id
            || !validate_uuid(&self.generation_id)
            || DateTime::parse_from_rfc3339(&self.created_at).is_err()
        {
            return Err(CheckpointStoreError::manifest(
                "checkpoint manifest identity is invalid",
            ));
        }
        self.fingerprint.validate()?;
        if self.fingerprint.digest != expected_fingerprint || self.slots.len() != 1 {
            return Err(CheckpointStoreError::manifest(
                "checkpoint manifest does not match the requested state",
            ));
        }
        let slot = &self.slots[0];
        if slot.id != 0
            || slot.filename != SLOT_FILENAME
            || slot.prompt_tokens == 0
            || slot.bytes == 0
            || !is_lower_hex_digest(&slot.sha256)
        {
            return Err(CheckpointStoreError::manifest(
                "checkpoint slot manifest is invalid",
            ));
        }
        Ok(())
    }

    pub fn slot(&self) -> &CheckpointSlotManifest {
        &self.slots[0]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LatestPointerV1 {
    schema_version: u32,
    generation_id: String,
    manifest_sha256: String,
    updated_at: String,
}

impl LatestPointerV1 {
    fn validate(&self) -> StoreResult<()> {
        if self.schema_version != LATEST_POINTER_SCHEMA_VERSION
            || !validate_uuid(&self.generation_id)
            || !is_lower_hex_digest(&self.manifest_sha256)
            || DateTime::parse_from_rfc3339(&self.updated_at).is_err()
        {
            return Err(CheckpointStoreError::manifest(
                "checkpoint latest pointer is invalid",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UsageFileV1 {
    schema_version: u32,
    entries: BTreeMap<String, u64>,
}

impl Default for UsageFileV1 {
    fn default() -> Self {
        Self {
            schema_version: USAGE_SCHEMA_VERSION,
            entries: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedGeneration {
    pub manifest: CheckpointManifestV1,
    generation_dir: PathBuf,
    payload_path: PathBuf,
}

impl LoadedGeneration {
    pub fn payload_path(&self) -> &Path {
        &self.payload_path
    }

    pub fn generation_dir(&self) -> &Path {
        &self.generation_dir
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreFaultPoint {
    AfterPayloadMove,
    AfterPayloadSync,
    AfterManifestWrite,
    BeforeGenerationRename,
    BeforeLatestUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedGeneration {
    pub generation_id: String,
    pub bytes: u64,
    pub prompt_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PruneResult {
    pub removed_generations: usize,
    pub remaining_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct CheckpointStore {
    root: PathBuf,
    hash_cache_lock: Arc<Mutex<()>>,
    root_ready: Arc<Mutex<bool>>,
}

impl CheckpointStore {
    pub fn new(app_data_dir: impl AsRef<Path>) -> Self {
        Self::from_root(app_data_dir.as_ref().join("kv-checkpoints"))
    }

    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            hash_cache_lock: Arc::new(Mutex::new(())),
            root_ready: Arc::new(Mutex::new(false)),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn ensure_root(&self) -> StoreResult<()> {
        let mut ready = self
            .root_ready
            .lock()
            .map_err(|_| CheckpointStoreError::io("checkpoint root lock failed"))?;
        if *ready {
            validate_directory(&self.root)?;
            return Ok(());
        }
        ensure_private_directory(&self.root)?;
        protect_windows_checkpoint_root(&self.root)?;
        *ready = true;
        Ok(())
    }

    fn instance_root(&self, instance_id: &str) -> StoreResult<PathBuf> {
        if !validate_identifier(instance_id) {
            return Err(CheckpointStoreError::manifest(
                "checkpoint instance identity is invalid",
            ));
        }
        Ok(self.root.join(instance_id))
    }

    fn fingerprint_root(&self, instance_id: &str, fingerprint: &str) -> StoreResult<PathBuf> {
        if !is_lower_hex_digest(fingerprint) {
            return Err(CheckpointStoreError::manifest(
                "checkpoint fingerprint path is invalid",
            ));
        }
        Ok(self.instance_root(instance_id)?.join(fingerprint))
    }

    pub fn prepare_instance(&self, instance_id: &str) -> StoreResult<PathBuf> {
        self.ensure_root()?;
        let instance_root = self.instance_root(instance_id)?;
        ensure_private_directory(&instance_root)?;
        let scratch = instance_root.join("scratch");
        ensure_private_directory(&scratch)?;
        Ok(scratch)
    }

    pub fn new_scratch_slot_path(&self, instance_id: &str, pid: u32) -> StoreResult<PathBuf> {
        let scratch = self.prepare_instance(instance_id)?;
        Ok(scratch.join(format!("slot-0-{pid}-{}.bin", uuid::Uuid::new_v4())))
    }

    pub fn cleanup_scratch(&self, instance_id: &str) -> StoreResult<usize> {
        let scratch = self.prepare_instance(instance_id)?;
        let mut removed = 0;
        for entry in fs::read_dir(&scratch)
            .map_err(|_| CheckpointStoreError::io("scratch directory scan failed"))?
        {
            let entry = entry.map_err(|_| CheckpointStoreError::io("scratch entry read failed"))?;
            let filename = entry.file_name();
            let filename = filename.to_str().unwrap_or_default();
            if !is_manager_slot_filename(filename) {
                continue;
            }
            let path = entry.path();
            validate_regular_file(&path)?;
            self.remove_scratch_payload(instance_id, &path)?;
            removed += 1;
        }
        Ok(removed)
    }

    pub fn clear_instance(&self, instance_id: &str) -> StoreResult<bool> {
        self.ensure_root()?;
        let instance_root = self.instance_root(instance_id)?;
        match fs::symlink_metadata(&instance_root) {
            Ok(_) => {
                validate_directory(&instance_root)?;
                safe_remove_directory(&instance_root, &self.root)?;
                sync_directory(&self.root)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(CheckpointStoreError::io(
                "checkpoint instance inspection failed",
            )),
        }
    }

    fn validate_scratch_payload(&self, instance_id: &str, path: &Path) -> StoreResult<()> {
        let scratch = self.prepare_instance(instance_id)?;
        let metadata = validate_regular_file(path)?;
        if metadata.len() == 0 {
            return Err(CheckpointStoreError::manifest(
                "checkpoint payload is empty",
            ));
        }
        protect_file(path)?;
        let canonical_scratch = fs::canonicalize(&scratch)
            .map_err(|_| CheckpointStoreError::io("scratch path resolution failed"))?;
        let canonical_payload = fs::canonicalize(path)
            .map_err(|_| CheckpointStoreError::io("scratch payload resolution failed"))?;
        let filename = canonical_payload
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !is_path_within(&canonical_payload, &canonical_scratch)
            || !is_manager_slot_filename(filename)
        {
            return Err(CheckpointStoreError::manifest(
                "checkpoint scratch payload path is invalid",
            ));
        }
        Ok(())
    }

    fn write_hash_cache(&self, path: &Path, cache: &HashCacheFile) -> StoreResult<()> {
        let encoded = serde_json::to_vec_pretty(cache).map_err(|_| {
            CheckpointStoreError::new(
                CheckpointReasonCode::FingerprintUnavailable,
                "fingerprint cache serialization failed",
            )
        })?;
        crate::persistence::atomic_write(path, &encoded, None).map_err(|_| {
            CheckpointStoreError::new(
                CheckpointReasonCode::FingerprintUnavailable,
                "fingerprint cache persistence failed",
            )
        })
    }

    pub fn content_sha256(&self, path: &Path) -> StoreResult<String> {
        self.ensure_root()?;
        let _guard = self
            .hash_cache_lock
            .lock()
            .map_err(|_| CheckpointStoreError::io("fingerprint cache lock failed"))?;
        let canonical = fs::canonicalize(path).map_err(|_| {
            CheckpointStoreError::new(
                CheckpointReasonCode::FingerprintUnavailable,
                "fingerprint input is unavailable",
            )
        })?;
        let before = validate_regular_file(&canonical).map_err(|_| {
            CheckpointStoreError::new(
                CheckpointReasonCode::FingerprintUnavailable,
                "fingerprint input is unavailable",
            )
        })?;
        let modified_before = modified_unix_nanos(&before)?;
        let identity_before = file_identity(&canonical, &before)?;
        let key = canonical_path_cache_key(&canonical);
        let cache_path = self.root.join("fingerprints-v1.json");
        let mut cache = match read_bounded(&cache_path, MAX_METADATA_BYTES) {
            Ok(bytes) => serde_json::from_slice::<HashCacheFile>(&bytes)
                .ok()
                .filter(|cache| cache.schema_version == HASH_CACHE_SCHEMA_VERSION)
                .unwrap_or_default(),
            Err(_) => HashCacheFile::default(),
        };
        let now_unix_secs = Utc::now().timestamp().max(0) as u64;
        maintain_hash_cache(&mut cache, now_unix_secs);
        let cached_digest = cache.entries.get(&key).and_then(|entry| {
            (entry.size == before.len()
                && entry.modified_unix_nanos == modified_before
                && entry.file_identity == identity_before
                && is_lower_hex_digest(&entry.sha256))
            .then(|| entry.sha256.clone())
        });
        if let Some(digest) = cached_digest {
            if let Some(entry) = cache.entries.get_mut(&key) {
                entry.last_used_unix_secs = now_unix_secs;
            }
            self.write_hash_cache(&cache_path, &cache)?;
            return Ok(digest);
        }

        let digest = sha256_file(&canonical).map_err(|_| {
            CheckpointStoreError::new(
                CheckpointReasonCode::FingerprintUnavailable,
                "fingerprint input could not be hashed",
            )
        })?;
        let after = validate_regular_file(&canonical).map_err(|_| {
            CheckpointStoreError::new(
                CheckpointReasonCode::FingerprintUnavailable,
                "fingerprint input changed while hashing",
            )
        })?;
        let modified_after = modified_unix_nanos(&after)?;
        let identity_after = file_identity(&canonical, &after)?;
        if before.len() != after.len()
            || modified_before != modified_after
            || identity_before != identity_after
        {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::FingerprintUnavailable,
                "fingerprint input changed while hashing",
            ));
        }
        cache.entries.insert(
            key,
            HashCacheEntry {
                size: after.len(),
                modified_unix_nanos: modified_after,
                file_identity: identity_after,
                sha256: digest.clone(),
                last_used_unix_secs: now_unix_secs,
            },
        );
        maintain_hash_cache(&mut cache, now_unix_secs);
        self.write_hash_cache(&cache_path, &cache)?;
        Ok(digest)
    }

    pub fn model_artifact_sha256(&self, model_path: &Path) -> StoreResult<String> {
        let artifacts = resolve_model_artifacts(model_path).map_err(|_| {
            CheckpointStoreError::new(
                CheckpointReasonCode::ModelArtifactsIncomplete,
                "model artifact set is unavailable or incomplete",
            )
        })?;
        if artifacts.len() == 1 {
            return self.content_sha256(&artifacts[0]);
        }

        let mut canonical = Vec::with_capacity(32 + artifacts.len() * 72);
        canonical.extend_from_slice(b"llama-model-artifact-set-v1\0");
        canonical.extend_from_slice(&(artifacts.len() as u32).to_le_bytes());
        for (index, artifact) in artifacts.iter().enumerate() {
            canonical.extend_from_slice(&((index + 1) as u32).to_le_bytes());
            canonical.extend_from_slice(self.content_sha256(artifact)?.as_bytes());
        }
        Ok(sha256_bytes(&canonical))
    }

    pub fn engine_artifact_sha256(&self, engine_path: &Path) -> StoreResult<String> {
        let artifacts = resolve_engine_runtime_artifacts(engine_path).ok_or_else(|| {
            CheckpointStoreError::new(
                CheckpointReasonCode::FingerprintUnavailable,
                "engine runtime artifact set is unavailable",
            )
        })?;
        if artifacts.len() == 1 {
            return self.content_sha256(&artifacts[0]);
        }

        let mut canonical = Vec::with_capacity(40 + artifacts.len() * 112);
        canonical.extend_from_slice(b"llama-engine-artifact-set-v1\0");
        canonical.extend_from_slice(&(artifacts.len() as u32).to_le_bytes());
        for artifact in &artifacts {
            let name = artifact
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            canonical.extend_from_slice(&(name.len() as u32).to_le_bytes());
            canonical.extend_from_slice(name.as_bytes());
            canonical.extend_from_slice(self.content_sha256(artifact)?.as_bytes());
        }
        Ok(sha256_bytes(&canonical))
    }

    pub fn build_fingerprint(
        &self,
        config: &InstanceConfig,
        model_path: &Path,
        engine_path: &Path,
        engine_version: &str,
        backend: &str,
    ) -> StoreResult<CheckpointFingerprint> {
        let chat_template_file_sha256 = if config.chat_template_file.trim().is_empty() {
            None
        } else {
            Some(self.content_sha256(Path::new(&config.chat_template_file))?)
        };
        let draft_model_sha256 = if config.draft_model_path.trim().is_empty() {
            None
        } else {
            Some(self.model_artifact_sha256(Path::new(&config.draft_model_path))?)
        };
        let materials = FingerprintMaterials {
            model_sha256: self.model_artifact_sha256(model_path)?,
            draft_model_sha256,
            engine_sha256: self.engine_artifact_sha256(engine_path)?,
            engine_version: engine_version.into(),
            backend: backend.into(),
            chat_template_file_sha256,
        };
        build_checkpoint_fingerprint(config, &materials)
    }

    fn ensure_generation_roots(
        &self,
        instance_id: &str,
        fingerprint: &str,
    ) -> StoreResult<(PathBuf, PathBuf)> {
        self.prepare_instance(instance_id)?;
        let fingerprint_root = self.fingerprint_root(instance_id, fingerprint)?;
        ensure_private_directory(&fingerprint_root)?;
        let generations = fingerprint_root.join("generations");
        ensure_private_directory(&generations)?;
        Ok((fingerprint_root, generations))
    }

    pub fn commit_generation(
        &self,
        manifest: &CheckpointManifestV1,
        scratch_payload: &Path,
        storage_limit_bytes: u64,
    ) -> StoreResult<CommittedGeneration> {
        self.commit_generation_with_fault(
            manifest,
            scratch_payload,
            storage_limit_bytes,
            |_| Ok(()),
        )
    }

    pub fn commit_generation_with_fault(
        &self,
        manifest: &CheckpointManifestV1,
        scratch_payload: &Path,
        storage_limit_bytes: u64,
        inject: impl Fn(StoreFaultPoint) -> StoreResult<()>,
    ) -> StoreResult<CommittedGeneration> {
        manifest.validate(&manifest.instance_id, &manifest.fingerprint.digest)?;
        self.validate_scratch_payload(&manifest.instance_id, scratch_payload)?;
        let source_metadata = validate_regular_file(scratch_payload)?;
        let slot = manifest.slot();
        if source_metadata.len() != slot.bytes || sha256_file(scratch_payload)? != slot.sha256 {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::ChecksumMismatch,
                "checkpoint scratch payload does not match its manifest",
            ));
        }
        let manifest_bytes = serde_json::to_vec_pretty(manifest)
            .map_err(|_| CheckpointStoreError::manifest("manifest serialization failed"))?;
        let generation_bytes = slot.bytes.saturating_add(manifest_bytes.len() as u64);
        if generation_bytes > storage_limit_bytes {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::StorageLimit,
                "checkpoint generation exceeds the storage limit",
            ));
        }

        let (fingerprint_root, generations_root) =
            self.ensure_generation_roots(&manifest.instance_id, &manifest.fingerprint.digest)?;
        let pending = generations_root.join(format!(".pending-{}", manifest.generation_id));
        let final_dir = generations_root.join(&manifest.generation_id);
        if pending.exists() || final_dir.exists() {
            return Err(CheckpointStoreError::manifest(
                "checkpoint generation identity already exists",
            ));
        }
        ensure_private_directory(&pending)?;
        let mut renamed = false;
        let result = (|| {
            let destination = pending.join(SLOT_FILENAME);
            fs::rename(scratch_payload, &destination)
                .map_err(|_| CheckpointStoreError::io("generation payload move failed"))?;
            inject(StoreFaultPoint::AfterPayloadMove)?;
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&destination)
                .and_then(|payload| payload.sync_all())
                .map_err(|_| CheckpointStoreError::io("generation payload sync failed"))?;
            protect_file(&destination)?;
            inject(StoreFaultPoint::AfterPayloadSync)?;
            if validate_regular_file(&destination)?.len() != slot.bytes
                || sha256_file(&destination)? != slot.sha256
            {
                return Err(CheckpointStoreError::new(
                    CheckpointReasonCode::ChecksumMismatch,
                    "moved generation payload failed verification",
                ));
            }

            crate::persistence::atomic_write(&pending.join("manifest.json"), &manifest_bytes, None)
                .map_err(|_| CheckpointStoreError::io("manifest persistence failed"))?;
            inject(StoreFaultPoint::AfterManifestWrite)?;
            sync_directory(&pending)?;
            inject(StoreFaultPoint::BeforeGenerationRename)?;
            fs::rename(&pending, &final_dir)
                .map_err(|_| CheckpointStoreError::io("generation commit rename failed"))?;
            renamed = true;
            sync_directory(&generations_root)?;

            inject(StoreFaultPoint::BeforeLatestUpdate)?;
            let latest = LatestPointerV1 {
                schema_version: LATEST_POINTER_SCHEMA_VERSION,
                generation_id: manifest.generation_id.clone(),
                manifest_sha256: sha256_bytes(&manifest_bytes),
                updated_at: Utc::now().to_rfc3339(),
            };
            let latest_bytes = serde_json::to_vec_pretty(&latest).map_err(|_| {
                CheckpointStoreError::manifest("latest pointer serialization failed")
            })?;
            crate::persistence::atomic_write(
                &fingerprint_root.join("latest.json"),
                &latest_bytes,
                None,
            )
            .map_err(|_| CheckpointStoreError::io("latest pointer persistence failed"))?;
            self.touch_usage(
                &manifest.instance_id,
                &manifest.fingerprint.digest,
                &manifest.generation_id,
            )?;
            Ok(CommittedGeneration {
                generation_id: manifest.generation_id.clone(),
                bytes: slot.bytes,
                prompt_tokens: slot.prompt_tokens,
            })
        })();
        if result.is_err() && !renamed && pending.exists() {
            let _ = safe_remove_directory(&pending, &generations_root);
        }
        result
    }

    fn load_generation(
        &self,
        instance_id: &str,
        fingerprint: &str,
        generations_root: &Path,
        generation_id: &str,
        expected_manifest_sha256: Option<&str>,
    ) -> StoreResult<LoadedGeneration> {
        if !validate_uuid(generation_id) {
            return Err(CheckpointStoreError::manifest(
                "checkpoint generation identity is invalid",
            ));
        }
        let generation_dir = generations_root.join(generation_id);
        verified_child_path(&generation_dir, generations_root)?;
        let manifest_path = generation_dir.join("manifest.json");
        let manifest_bytes = read_bounded(&manifest_path, MAX_MANIFEST_BYTES)?;
        if expected_manifest_sha256
            .is_some_and(|expected| sha256_bytes(&manifest_bytes) != expected)
        {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::ChecksumMismatch,
                "checkpoint manifest checksum mismatch",
            ));
        }
        let manifest =
            CheckpointManifestV1::parse_and_validate(&manifest_bytes, instance_id, fingerprint)?;
        if manifest.generation_id != generation_id {
            return Err(CheckpointStoreError::manifest(
                "checkpoint generation directory does not match its manifest",
            ));
        }
        let payload_path = generation_dir.join(SLOT_FILENAME);
        let payload_metadata = validate_regular_file(&payload_path)?;
        if payload_metadata.len() != manifest.slot().bytes
            || sha256_file(&payload_path)? != manifest.slot().sha256
        {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::ChecksumMismatch,
                "checkpoint payload checksum mismatch",
            ));
        }
        Ok(LoadedGeneration {
            manifest,
            generation_dir,
            payload_path,
        })
    }

    fn read_latest_pointer(&self, fingerprint_root: &Path) -> StoreResult<LatestPointerV1> {
        let bytes = read_bounded(&fingerprint_root.join("latest.json"), MAX_MANIFEST_BYTES)?;
        let latest: LatestPointerV1 = serde_json::from_slice(&bytes).map_err(|_| {
            CheckpointStoreError::manifest("checkpoint latest pointer is malformed")
        })?;
        latest.validate()?;
        Ok(latest)
    }

    pub fn load_latest(
        &self,
        instance_id: &str,
        fingerprint: &str,
    ) -> StoreResult<Option<LoadedGeneration>> {
        self.ensure_root()?;
        let fingerprint_root = self.fingerprint_root(instance_id, fingerprint)?;
        if !fingerprint_root.exists() {
            return Ok(None);
        }
        validate_directory(&fingerprint_root)?;
        let generations_root = fingerprint_root.join("generations");
        if !generations_root.exists() {
            return Ok(None);
        }
        validate_directory(&generations_root)?;

        let latest_path = fingerprint_root.join("latest.json");
        let mut latest_error = None;
        if latest_path.exists() {
            match self
                .read_latest_pointer(&fingerprint_root)
                .and_then(|latest| {
                    self.load_generation(
                        instance_id,
                        fingerprint,
                        &generations_root,
                        &latest.generation_id,
                        Some(&latest.manifest_sha256),
                    )
                }) {
                Ok(loaded) => {
                    let _ =
                        self.touch_usage(instance_id, fingerprint, &loaded.manifest.generation_id);
                    return Ok(Some(loaded));
                }
                Err(error) => latest_error = Some(error),
            }
        }

        let mut candidates = Vec::new();
        let entries = fs::read_dir(&generations_root)
            .map_err(|_| CheckpointStoreError::io("generation scan failed"))?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !validate_uuid(&name) {
                continue;
            }
            if let Ok(loaded) =
                self.load_generation(instance_id, fingerprint, &generations_root, &name, None)
            {
                if let Ok(created_at) = DateTime::parse_from_rfc3339(&loaded.manifest.created_at) {
                    candidates.push((created_at, loaded));
                }
            }
        }
        candidates.sort_by_key(|(created_at, _)| *created_at);
        let loaded = candidates.pop().map(|(_, loaded)| loaded);
        if let Some(loaded) = loaded.as_ref() {
            let _ = self.touch_usage(instance_id, fingerprint, &loaded.manifest.generation_id);
            return Ok(Some(loaded.clone()));
        }
        if let Some(error) = latest_error {
            return Err(error);
        }
        Ok(None)
    }

    pub fn cleanup_pending(&self, instance_id: &str, fingerprint: &str) -> StoreResult<usize> {
        let (_, generations_root) = self.ensure_generation_roots(instance_id, fingerprint)?;
        let mut removed = 0;
        let entries = fs::read_dir(&generations_root)
            .map_err(|_| CheckpointStoreError::io("pending generation scan failed"))?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(generation_id) = name.strip_prefix(".pending-") else {
                continue;
            };
            if !validate_uuid(generation_id) {
                continue;
            }
            let path = entry.path();
            if safe_remove_directory(&path, &generations_root).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn usage_path(&self, instance_id: &str) -> StoreResult<PathBuf> {
        Ok(self.instance_root(instance_id)?.join("usage-v1.json"))
    }

    fn read_usage(&self, instance_id: &str) -> UsageFileV1 {
        self.usage_path(instance_id)
            .ok()
            .and_then(|path| read_bounded(&path, MAX_METADATA_BYTES).ok())
            .and_then(|bytes| serde_json::from_slice::<UsageFileV1>(&bytes).ok())
            .filter(|usage| usage.schema_version == USAGE_SCHEMA_VERSION)
            .unwrap_or_default()
    }

    fn write_usage(&self, instance_id: &str, usage: &UsageFileV1) -> StoreResult<()> {
        let bytes = serde_json::to_vec_pretty(usage)
            .map_err(|_| CheckpointStoreError::io("usage metadata serialization failed"))?;
        crate::persistence::atomic_write(&self.usage_path(instance_id)?, &bytes, None)
            .map_err(|_| CheckpointStoreError::io("usage metadata persistence failed"))
    }

    fn touch_usage(
        &self,
        instance_id: &str,
        fingerprint: &str,
        generation_id: &str,
    ) -> StoreResult<()> {
        if !is_lower_hex_digest(fingerprint) || !validate_uuid(generation_id) {
            return Err(CheckpointStoreError::manifest(
                "usage metadata identity is invalid",
            ));
        }
        let mut usage = self.read_usage(instance_id);
        let now = Utc::now().timestamp_millis().max(0) as u64;
        usage
            .entries
            .insert(format!("{fingerprint}/{generation_id}"), now);
        self.write_usage(instance_id, &usage)
    }

    fn reconcile_latest_pointer(&self, instance_id: &str, fingerprint: &str) -> StoreResult<()> {
        let fingerprint_root = self.fingerprint_root(instance_id, fingerprint)?;
        if !fingerprint_root.exists() {
            return Ok(());
        }
        validate_directory(&fingerprint_root)?;
        let generations_root = fingerprint_root.join("generations");
        if !generations_root.exists() {
            return Ok(());
        }
        validate_directory(&generations_root)?;

        let mut newest = None;
        for entry in fs::read_dir(&generations_root)
            .map_err(|_| CheckpointStoreError::io("generation scan failed"))?
            .flatten()
        {
            let generation_id = entry.file_name().to_string_lossy().to_string();
            if !validate_uuid(&generation_id) {
                continue;
            }
            let Ok(loaded) = self.load_generation(
                instance_id,
                fingerprint,
                &generations_root,
                &generation_id,
                None,
            ) else {
                continue;
            };
            let Ok(created_at) = DateTime::parse_from_rfc3339(&loaded.manifest.created_at) else {
                continue;
            };
            let replace = newest.as_ref().map_or(
                true,
                |(current, _): &(DateTime<chrono::FixedOffset>, LoadedGeneration)| {
                    created_at > *current
                },
            );
            if replace {
                newest = Some((created_at, loaded));
            }
        }

        let latest_path = fingerprint_root.join("latest.json");
        if let Some((_, loaded)) = newest {
            let manifest_bytes = read_bounded(
                &loaded.generation_dir.join("manifest.json"),
                MAX_MANIFEST_BYTES,
            )?;
            let latest = LatestPointerV1 {
                schema_version: LATEST_POINTER_SCHEMA_VERSION,
                generation_id: loaded.manifest.generation_id,
                manifest_sha256: sha256_bytes(&manifest_bytes),
                updated_at: Utc::now().to_rfc3339(),
            };
            let latest_bytes = serde_json::to_vec_pretty(&latest).map_err(|_| {
                CheckpointStoreError::manifest("latest pointer serialization failed")
            })?;
            crate::persistence::atomic_write(&latest_path, &latest_bytes, None)
                .map_err(|_| CheckpointStoreError::io("latest pointer persistence failed"))?;
        } else {
            match fs::symlink_metadata(&latest_path) {
                Ok(_) => {
                    validate_regular_file(&latest_path)?;
                    fs::remove_file(&latest_path)
                        .map_err(|_| CheckpointStoreError::io("latest pointer removal failed"))?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(CheckpointStoreError::io("latest pointer inspection failed")),
            }
        }
        Ok(())
    }

    pub fn prune_instance(
        &self,
        instance_id: &str,
        storage_limit_bytes: u64,
        protected: &[(String, String)],
    ) -> StoreResult<PruneResult> {
        self.ensure_root()?;
        let instance_root = self.instance_root(instance_id)?;
        if !instance_root.exists() {
            return Ok(PruneResult::default());
        }
        validate_directory(&instance_root)?;
        let usage = self.read_usage(instance_id);
        let explicit_protected: HashSet<String> = protected
            .iter()
            .filter(|(fingerprint, generation)| {
                is_lower_hex_digest(fingerprint) && validate_uuid(generation)
            })
            .map(|(fingerprint, generation)| format!("{fingerprint}/{generation}"))
            .collect();
        let mut fallback_protected = HashSet::new();
        let mut generations = Vec::new();
        let mut fingerprint_ids = Vec::new();
        let mut valid_keys = HashSet::new();
        let mut newest_global: Option<(DateTime<chrono::FixedOffset>, String, u64)> = None;
        let mut removed_generations = 0;
        let mut usage_next = usage.clone();
        let fingerprint_entries = fs::read_dir(&instance_root)
            .map_err(|_| CheckpointStoreError::io("instance generation scan failed"))?;
        for fingerprint_entry in fingerprint_entries.flatten() {
            let fingerprint = fingerprint_entry.file_name().to_string_lossy().to_string();
            if !is_lower_hex_digest(&fingerprint) {
                continue;
            }
            let fingerprint_root = fingerprint_entry.path();
            if validate_directory(&fingerprint_root).is_err() {
                continue;
            }
            fingerprint_ids.push(fingerprint.clone());
            let generations_root = fingerprint_root.join("generations");
            if validate_directory(&generations_root).is_err() {
                continue;
            }
            let _ = self.cleanup_pending(instance_id, &fingerprint);
            let entries = match fs::read_dir(&generations_root) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let generation_id = entry.file_name().to_string_lossy().to_string();
                if !validate_uuid(&generation_id) {
                    continue;
                }
                let loaded = match self.load_generation(
                    instance_id,
                    &fingerprint,
                    &generations_root,
                    &generation_id,
                    None,
                ) {
                    Ok(loaded) => loaded,
                    Err(_) => {
                        let key = format!("{fingerprint}/{generation_id}");
                        usage_next.entries.remove(&key);
                        if safe_remove_directory(&entry.path(), &generations_root).is_ok() {
                            removed_generations += 1;
                        }
                        continue;
                    }
                };
                let manifest_size = fs::metadata(loaded.generation_dir.join("manifest.json"))
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                let bytes = loaded.manifest.slot().bytes.saturating_add(manifest_size);
                let key = format!("{fingerprint}/{generation_id}");
                if let Ok(created_at) = DateTime::parse_from_rfc3339(&loaded.manifest.created_at) {
                    let replace = newest_global
                        .as_ref()
                        .map_or(true, |(newest, _, _)| created_at > *newest);
                    if replace {
                        newest_global = Some((created_at, key.clone(), bytes));
                    }
                }
                let last_used = usage.entries.get(&key).copied().unwrap_or_else(|| {
                    DateTime::parse_from_rfc3339(&loaded.manifest.created_at)
                        .map(|value| value.timestamp_millis().max(0) as u64)
                        .unwrap_or(0)
                });
                valid_keys.insert(key.clone());
                generations.push((last_used, key, bytes, loaded.generation_dir));
            }
        }

        if explicit_protected.is_empty() {
            if let Some((_, key, bytes)) = newest_global {
                if bytes <= storage_limit_bytes {
                    fallback_protected.insert(key);
                }
            }
        }

        let mut remaining_bytes = generations.iter().fold(0_u64, |total, (_, _, bytes, _)| {
            total.saturating_add(*bytes)
        });
        generations.sort_by_key(|(last_used, _, _, _)| *last_used);
        for (_, key, bytes, path) in generations {
            if remaining_bytes <= storage_limit_bytes {
                break;
            }
            if explicit_protected.contains(&key) || fallback_protected.contains(&key) {
                continue;
            }
            let Some(generations_root) = path.parent() else {
                continue;
            };
            safe_remove_directory(&path, generations_root)?;
            remaining_bytes = remaining_bytes.saturating_sub(bytes);
            removed_generations += 1;
            usage_next.entries.remove(&key);
            valid_keys.remove(&key);
        }
        usage_next.entries.retain(|key, _| valid_keys.contains(key));
        if usage_next.entries != self.read_usage(instance_id).entries {
            self.write_usage(instance_id, &usage_next)?;
        }
        for fingerprint in fingerprint_ids {
            self.reconcile_latest_pointer(instance_id, &fingerprint)?;
        }
        Ok(PruneResult {
            removed_generations,
            remaining_bytes,
        })
    }

    pub fn copy_generation_to_scratch(
        &self,
        instance_id: &str,
        pid: u32,
        generation: &LoadedGeneration,
    ) -> StoreResult<PathBuf> {
        if generation.manifest.instance_id != instance_id {
            return Err(CheckpointStoreError::manifest(
                "checkpoint generation belongs to another instance",
            ));
        }
        let slot = generation.manifest.slot();
        self.ensure_scratch_capacity(instance_id, slot.bytes)?;
        let destination = self.new_scratch_slot_path(instance_id, pid)?;
        let mut source = File::open(generation.payload_path())
            .map_err(|_| CheckpointStoreError::io("generation payload open failed"))?;
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut target = options
            .open(&destination)
            .map_err(|_| CheckpointStoreError::io("scratch restore payload create failed"))?;
        let copy_result = (|| {
            std::io::copy(&mut source, &mut target)
                .map_err(|_| CheckpointStoreError::io("scratch restore payload copy failed"))?;
            target
                .flush()
                .map_err(|_| CheckpointStoreError::io("scratch restore payload flush failed"))?;
            target
                .sync_all()
                .map_err(|_| CheckpointStoreError::io("scratch restore payload sync failed"))?;
            drop(target);
            protect_file(&destination)?;
            if validate_regular_file(&destination)?.len() != slot.bytes
                || sha256_file(&destination)? != slot.sha256
            {
                return Err(CheckpointStoreError::new(
                    CheckpointReasonCode::ChecksumMismatch,
                    "scratch restore payload failed verification",
                ));
            }
            Ok(())
        })();
        if copy_result.is_err() {
            let _ = self.remove_scratch_payload(instance_id, &destination);
        }
        copy_result.map(|_| destination)
    }

    pub fn ensure_scratch_capacity(
        &self,
        instance_id: &str,
        payload_bytes: u64,
    ) -> StoreResult<()> {
        let scratch = self.prepare_instance(instance_id)?;
        ensure_available_checkpoint_space(&scratch, payload_bytes)
    }

    pub fn verify_scratch_payload(
        &self,
        instance_id: &str,
        path: &Path,
        expected_bytes: u64,
    ) -> StoreResult<String> {
        self.validate_scratch_payload(instance_id, path)?;
        if validate_regular_file(path)?.len() != expected_bytes || expected_bytes == 0 {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::ChecksumMismatch,
                "scratch payload byte count mismatch",
            ));
        }
        sha256_file(path)
    }

    pub fn remove_scratch_payload(&self, instance_id: &str, path: &Path) -> StoreResult<()> {
        let scratch = self.prepare_instance(instance_id)?;
        if !path.exists() {
            return Ok(());
        }
        validate_regular_file(path)?;
        let canonical_scratch = fs::canonicalize(&scratch)
            .map_err(|_| CheckpointStoreError::io("scratch path resolution failed"))?;
        let canonical_payload = fs::canonicalize(path)
            .map_err(|_| CheckpointStoreError::io("scratch payload resolution failed"))?;
        let filename = canonical_payload
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !is_path_within(&canonical_payload, &canonical_scratch)
            || !is_manager_slot_filename(filename)
        {
            return Err(CheckpointStoreError::manifest(
                "scratch payload escaped its private root",
            ));
        }
        fs::remove_file(canonical_payload)
            .map_err(|_| CheckpointStoreError::io("scratch payload removal failed"))
    }

    pub fn has_other_fingerprint_generation(
        &self,
        instance_id: &str,
        current_fingerprint: &str,
    ) -> StoreResult<bool> {
        self.ensure_root()?;
        if !is_lower_hex_digest(current_fingerprint) {
            return Err(CheckpointStoreError::manifest(
                "current fingerprint is invalid",
            ));
        }
        let instance_root = self.instance_root(instance_id)?;
        if !instance_root.exists() {
            return Ok(false);
        }
        validate_directory(&instance_root)?;
        let entries = fs::read_dir(instance_root)
            .map_err(|_| CheckpointStoreError::io("fingerprint generation scan failed"))?;
        for entry in entries.flatten() {
            let fingerprint = entry.file_name().to_string_lossy().to_string();
            if fingerprint == current_fingerprint || !is_lower_hex_digest(&fingerprint) {
                continue;
            }
            if self
                .load_latest(instance_id, &fingerprint)
                .ok()
                .flatten()
                .is_some()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotSnapshot {
    pub id: u32,
    pub is_processing: bool,
    pub n_ctx: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotSaveResult {
    pub id_slot: u32,
    pub filename: String,
    pub n_saved: u64,
    pub n_written: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotRestoreResult {
    pub id_slot: u32,
    pub filename: String,
    pub n_restored: u64,
    pub n_read: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotEraseResult {
    pub id_slot: u32,
    pub n_erased: u64,
}

pub trait SlotBackend: Send + Sync {
    fn health(&self) -> StoreResult<()>;
    fn slots(&self) -> StoreResult<Vec<SlotSnapshot>>;
    fn save(&self, filename: &str) -> StoreResult<SlotSaveResult>;
    fn restore(&self, filename: &str) -> StoreResult<SlotRestoreResult>;
    fn erase(&self) -> StoreResult<SlotEraseResult>;
}

#[derive(Deserialize)]
struct HealthResponse {
    status: String,
}

#[derive(Deserialize)]
struct RawSlotSnapshot {
    id: u32,
    #[serde(default)]
    is_processing: bool,
    #[serde(default)]
    n_ctx: u64,
}

#[derive(Deserialize)]
struct RawSlotSaveResult {
    id_slot: u32,
    filename: String,
    n_saved: u64,
    n_written: u64,
}

#[derive(Deserialize)]
struct RawSlotRestoreResult {
    id_slot: u32,
    filename: String,
    n_restored: u64,
    n_read: u64,
}

#[derive(Deserialize)]
struct RawSlotEraseResult {
    id_slot: u32,
    n_erased: u64,
}

fn normalize_slot_http_timeout(timeout: Duration) -> Duration {
    timeout.max(Duration::from_millis(250))
}

fn build_slot_http_client(timeout: Duration) -> StoreResult<reqwest::blocking::Client> {
    let timeout = normalize_slot_http_timeout(timeout);
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(2)))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| CheckpointStoreError::io("slot HTTP client creation failed"))
}

pub struct LlamaSlotClient {
    probe_client: reqwest::blocking::Client,
    operation_client: reqwest::blocking::Client,
    cleanup_client: reqwest::blocking::Client,
    base_url: String,
    api_key: String,
}

impl LlamaSlotClient {
    pub fn new(config: &InstanceConfig, timeout: Duration) -> StoreResult<Self> {
        if !is_loopback_host(&config.host)
            || !config.ssl_key_file.trim().is_empty()
            || !config.ssl_cert_file.trim().is_empty()
            || !config.path_prefix.trim().is_empty()
            || !config.api_prefix.trim().is_empty()
        {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::LoopbackHttpRequired,
                "slot client requires a loopback HTTP endpoint",
            ));
        }
        let operation_timeout = normalize_slot_http_timeout(timeout);
        let probe_timeout = operation_timeout.min(CHECKPOINT_SLOT_PROBE_TIMEOUT);
        let probe_client = build_slot_http_client(probe_timeout)?;
        let operation_client = build_slot_http_client(operation_timeout)?;
        let cleanup_client = build_slot_http_client(CHECKPOINT_SLOT_CLEANUP_TIMEOUT)?;
        Ok(Self {
            probe_client,
            operation_client,
            cleanup_client,
            base_url: format!(
                "http://{}",
                crate::utils::format_host_port(config.host.trim(), config.port)
            ),
            api_key: crate::commands::server::effective_api_key(config),
        })
    }

    fn execute_json<T: DeserializeOwned>(
        &self,
        request: reqwest::blocking::RequestBuilder,
        invalid_reason: CheckpointReasonCode,
        invalid_message: &'static str,
    ) -> StoreResult<T> {
        const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
        let request = if self.api_key.is_empty() {
            request
        } else {
            request.bearer_auth(&self.api_key)
        };
        let mut response = request.send().map_err(|error| {
            if error.is_timeout() {
                CheckpointStoreError::new(
                    CheckpointReasonCode::HttpTimeout,
                    "slot HTTP request timed out",
                )
            } else {
                CheckpointStoreError::new(
                    CheckpointReasonCode::SlotApiError,
                    "slot HTTP request failed",
                )
            }
        })?;
        if !response.status().is_success() {
            let reason = if matches!(
                response.status(),
                reqwest::StatusCode::REQUEST_TIMEOUT | reqwest::StatusCode::GATEWAY_TIMEOUT
            ) {
                CheckpointReasonCode::HttpTimeout
            } else {
                CheckpointReasonCode::SlotApiError
            };
            return Err(CheckpointStoreError::new(
                reason,
                "slot HTTP endpoint returned an error",
            ));
        }
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| {
                CheckpointStoreError::new(
                    CheckpointReasonCode::SlotApiError,
                    "slot HTTP response read failed",
                )
            })?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::SlotApiError,
                "slot HTTP response exceeded its size limit",
            ));
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| CheckpointStoreError::new(invalid_reason, invalid_message))
    }

    fn manager_filename(filename: &str) -> StoreResult<()> {
        if !is_manager_slot_filename(filename) {
            return Err(CheckpointStoreError::manifest(
                "slot filename is not manager generated",
            ));
        }
        Ok(())
    }

    fn action_url(&self, action: &str) -> String {
        format!("{}/slots/0?action={action}", self.base_url)
    }
}

impl SlotBackend for LlamaSlotClient {
    fn health(&self) -> StoreResult<()> {
        let health: HealthResponse = self.execute_json(
            self.probe_client.get(format!("{}/health", self.base_url)),
            CheckpointReasonCode::SlotApiError,
            "slot health response was malformed",
        )?;
        if health.status != "ok" {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::SlotApiError,
                "llama server is not healthy",
            ));
        }
        Ok(())
    }

    fn slots(&self) -> StoreResult<Vec<SlotSnapshot>> {
        let slots: Vec<RawSlotSnapshot> = self.execute_json(
            self.probe_client.get(format!("{}/slots", self.base_url)),
            CheckpointReasonCode::SlotApiError,
            "slot state response was malformed",
        )?;
        let mut seen = HashSet::new();
        if slots.is_empty() || slots.iter().any(|slot| !seen.insert(slot.id)) {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::SlotStateMismatch,
                "slot state response was ambiguous",
            ));
        }
        Ok(slots
            .into_iter()
            .map(|slot| SlotSnapshot {
                id: slot.id,
                is_processing: slot.is_processing,
                n_ctx: slot.n_ctx,
            })
            .collect())
    }

    fn save(&self, filename: &str) -> StoreResult<SlotSaveResult> {
        Self::manager_filename(filename)?;
        let raw: RawSlotSaveResult = self.execute_json(
            self.operation_client
                .post(self.action_url("save"))
                .json(&serde_json::json!({ "filename": filename })),
            CheckpointReasonCode::SaveResponseInvalid,
            "slot save response was malformed",
        )?;
        if raw.id_slot != 0 || raw.filename != filename || raw.n_written == 0 {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::SaveResponseInvalid,
                "slot save response did not match the request",
            ));
        }
        Ok(SlotSaveResult {
            id_slot: raw.id_slot,
            filename: raw.filename,
            n_saved: raw.n_saved,
            n_written: raw.n_written,
        })
    }

    fn restore(&self, filename: &str) -> StoreResult<SlotRestoreResult> {
        Self::manager_filename(filename)?;
        let raw: RawSlotRestoreResult = self.execute_json(
            self.operation_client
                .post(self.action_url("restore"))
                .json(&serde_json::json!({ "filename": filename })),
            CheckpointReasonCode::RestoreResponseInvalid,
            "slot restore response was malformed",
        )?;
        if raw.id_slot != 0 || raw.filename != filename || raw.n_read == 0 {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::RestoreResponseInvalid,
                "slot restore response did not match the request",
            ));
        }
        Ok(SlotRestoreResult {
            id_slot: raw.id_slot,
            filename: raw.filename,
            n_restored: raw.n_restored,
            n_read: raw.n_read,
        })
    }

    fn erase(&self) -> StoreResult<SlotEraseResult> {
        let raw: RawSlotEraseResult = self.execute_json(
            self.cleanup_client.post(self.action_url("erase")),
            CheckpointReasonCode::RestoreResponseInvalid,
            "slot erase response was malformed",
        )?;
        if raw.id_slot != 0 {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::RestoreResponseInvalid,
                "slot erase response did not match slot zero",
            ));
        }
        Ok(SlotEraseResult {
            id_slot: raw.id_slot,
            n_erased: raw.n_erased,
        })
    }
}

#[derive(Debug, Clone)]
struct CoordinatorEntry {
    gate_active: bool,
    status: CheckpointStatus,
    fingerprint: Option<CheckpointFingerprint>,
    policy: crate::models::KvCheckpointConfig,
}

#[derive(Clone)]
pub struct CheckpointCoordinator {
    store: CheckpointStore,
    entries: Arc<Mutex<HashMap<String, CoordinatorEntry>>>,
}

struct CheckpointOperationUpdate<'a> {
    operation: CheckpointOperation,
    outcome: CheckpointOutcome,
    reason_code: CheckpointReasonCode,
    message: &'static str,
    generation: Option<&'a CheckpointManifestV1>,
    duration_ms: u64,
}

impl<'a> CheckpointOperationUpdate<'a> {
    fn new(
        operation: CheckpointOperation,
        outcome: CheckpointOutcome,
        reason_code: CheckpointReasonCode,
        message: &'static str,
        generation: Option<&'a CheckpointManifestV1>,
        duration_ms: u64,
    ) -> Self {
        Self {
            operation,
            outcome,
            reason_code,
            message,
            generation,
            duration_ms,
        }
    }
}

fn checkpoint_now_millis() -> u64 {
    Utc::now().timestamp_millis().max(0) as u64
}

fn legal_checkpoint_transition(from: CheckpointPhase, to: CheckpointPhase) -> bool {
    matches!(
        (from, to),
        (CheckpointPhase::Starting, CheckpointPhase::EngineHealthy)
            | (CheckpointPhase::Starting, CheckpointPhase::Stopping)
            | (CheckpointPhase::EngineHealthy, CheckpointPhase::Restoring)
            | (CheckpointPhase::EngineHealthy, CheckpointPhase::ReadyCold)
            | (CheckpointPhase::EngineHealthy, CheckpointPhase::Stopping)
            | (CheckpointPhase::Restoring, CheckpointPhase::Ready)
            | (CheckpointPhase::Restoring, CheckpointPhase::ReadyCold)
            | (CheckpointPhase::Restoring, CheckpointPhase::Stopping)
            | (CheckpointPhase::Ready, CheckpointPhase::Draining)
            | (CheckpointPhase::ReadyCold, CheckpointPhase::Draining)
            | (CheckpointPhase::Ready, CheckpointPhase::Stopping)
            | (CheckpointPhase::ReadyCold, CheckpointPhase::Stopping)
            | (CheckpointPhase::Draining, CheckpointPhase::Saving)
            | (CheckpointPhase::Draining, CheckpointPhase::Stopping)
            | (CheckpointPhase::Saving, CheckpointPhase::Stopping)
            | (CheckpointPhase::Stopping, CheckpointPhase::Stopped)
            | (CheckpointPhase::Stopping, CheckpointPhase::Ready)
            | (CheckpointPhase::Stopping, CheckpointPhase::ReadyCold)
            | (CheckpointPhase::Disabled, CheckpointPhase::Stopped)
            | (CheckpointPhase::Ineligible, CheckpointPhase::Stopped)
    )
}

impl CheckpointCoordinator {
    pub fn new(store: CheckpointStore) -> Self {
        Self {
            store,
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn store(&self) -> &CheckpointStore {
        &self.store
    }

    pub fn register_start_with_context(
        &self,
        instance_id: &str,
        expected_pid: u32,
        eligibility: &CheckpointEligibility,
        fingerprint: Option<CheckpointFingerprint>,
        policy: crate::models::KvCheckpointConfig,
    ) -> StoreResult<CheckpointStatus> {
        if !validate_identifier(instance_id) || expected_pid == 0 {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::UnsupportedConfiguration,
                "checkpoint process identity is invalid",
            ));
        }
        if eligibility.eligible {
            let registered_fingerprint = fingerprint.as_ref().ok_or_else(|| {
                CheckpointStoreError::new(
                    CheckpointReasonCode::FingerprintUnavailable,
                    "eligible checkpoint registration requires a fingerprint",
                )
            })?;
            registered_fingerprint.validate()?;
        }
        let (gate_active, phase, reason_code, last_outcome) = if eligibility.eligible {
            (
                true,
                CheckpointPhase::Starting,
                CheckpointReasonCode::None,
                CheckpointOutcome::None,
            )
        } else if eligibility.reason_code == CheckpointReasonCode::Disabled {
            (
                false,
                CheckpointPhase::Disabled,
                CheckpointReasonCode::Disabled,
                CheckpointOutcome::None,
            )
        } else {
            (
                false,
                CheckpointPhase::Ineligible,
                eligibility.reason_code,
                CheckpointOutcome::Skipped,
            )
        };
        let status = CheckpointStatus {
            instance_id: instance_id.into(),
            expected_pid: Some(expected_pid),
            phase,
            // Disabled/ineligible runs retain legacy routing behavior. Their
            // engine health remains the proxy's independent readiness signal.
            routable: !gate_active,
            last_operation: CheckpointOperation::None,
            last_outcome,
            reason_code,
            message: String::new(),
            generation_id: None,
            prompt_tokens: None,
            bytes: None,
            duration_ms: None,
            updated_at: checkpoint_now_millis(),
        };
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CheckpointStoreError::io("checkpoint status lock failed"))?;
        entries.insert(
            instance_id.into(),
            CoordinatorEntry {
                gate_active,
                status: status.clone(),
                fingerprint,
                policy,
            },
        );
        Ok(status)
    }

    pub fn registered_checkpoint(
        &self,
        instance_id: &str,
        expected_pid: u32,
    ) -> StoreResult<Option<(CheckpointFingerprint, crate::models::KvCheckpointConfig)>> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| CheckpointStoreError::io("checkpoint status lock failed"))?;
        let entry = entries.get(instance_id).ok_or_else(|| {
            CheckpointStoreError::new(
                CheckpointReasonCode::StaleProcessEvent,
                "checkpoint event has no active process",
            )
        })?;
        if entry.status.expected_pid != Some(expected_pid) {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::StaleProcessEvent,
                "checkpoint event belongs to a stale process",
            ));
        }
        Ok(entry
            .fingerprint
            .clone()
            .map(|fingerprint| (fingerprint, entry.policy.clone())))
    }

    pub fn status(&self, instance_id: &str) -> Option<CheckpointStatus> {
        self.entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(instance_id).map(|entry| entry.status.clone()))
    }

    pub fn statuses(&self) -> HashMap<String, CheckpointStatus> {
        self.entries
            .lock()
            .map(|entries| {
                entries
                    .iter()
                    .map(|(id, entry)| (id.clone(), entry.status.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn blocked_phase(&self) -> Option<CheckpointPhase> {
        fn priority(phase: CheckpointPhase) -> u8 {
            match phase {
                CheckpointPhase::Restoring => 0,
                CheckpointPhase::Saving => 1,
                CheckpointPhase::Draining => 2,
                CheckpointPhase::EngineHealthy => 3,
                CheckpointPhase::Starting => 4,
                CheckpointPhase::Stopping => 5,
                _ => 6,
            }
        }

        self.entries.lock().ok().and_then(|entries| {
            entries
                .values()
                .filter(|entry| entry.gate_active && !entry.status.routable)
                .map(|entry| entry.status.phase)
                .min_by_key(|phase| priority(*phase))
        })
    }

    pub fn clear_instance(&self, instance_id: &str) -> StoreResult<CheckpointStatus> {
        if !validate_identifier(instance_id) {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::ManifestInvalid,
                "checkpoint instance identity is invalid",
            ));
        }
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CheckpointStoreError::io("checkpoint status lock failed"))?;
        if entries.get(instance_id).is_some_and(|entry| {
            entry.status.expected_pid.is_some() && entry.status.phase != CheckpointPhase::Stopped
        }) {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::ClearWhileRunning,
                "checkpoint cannot be cleared while the instance is running",
            ));
        }
        let removed = self.store.clear_instance(instance_id)?;
        let mut status = entries
            .get(instance_id)
            .map(|entry| entry.status.clone())
            .unwrap_or_else(|| {
                CheckpointStatus::disabled(instance_id, checkpoint_now_millis())
                    .with_phase(CheckpointPhase::Stopped)
            });
        status.expected_pid = None;
        status.phase = CheckpointPhase::Stopped;
        status.routable = false;
        status.last_operation = CheckpointOperation::Clear;
        status.last_outcome = if removed {
            CheckpointOutcome::Success
        } else {
            CheckpointOutcome::Skipped
        };
        status.reason_code = if removed {
            CheckpointReasonCode::None
        } else {
            CheckpointReasonCode::NoCheckpoint
        };
        status.message = if removed {
            "checkpoint data cleared"
        } else {
            "no checkpoint data was present"
        }
        .into();
        status.generation_id = None;
        status.prompt_tokens = None;
        status.bytes = None;
        status.duration_ms = Some(0);
        status.updated_at = checkpoint_now_millis();
        entries.insert(
            instance_id.into(),
            CoordinatorEntry {
                gate_active: false,
                status: status.clone(),
                fingerprint: None,
                policy: crate::models::KvCheckpointConfig::default(),
            },
        );
        Ok(status)
    }

    pub fn gate_allows_routing(&self, instance_id: &str) -> bool {
        self.entries
            .lock()
            .ok()
            .and_then(|entries| {
                entries
                    .get(instance_id)
                    .map(|entry| !entry.gate_active || entry.status.routable)
            })
            .unwrap_or(true)
    }

    pub fn gate_active(&self, instance_id: &str) -> bool {
        self.entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(instance_id).map(|entry| entry.gate_active))
            .unwrap_or(false)
    }

    fn transition(
        &self,
        instance_id: &str,
        expected_pid: u32,
        next: CheckpointPhase,
    ) -> StoreResult<CheckpointStatus> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CheckpointStoreError::io("checkpoint status lock failed"))?;
        let entry = entries.get_mut(instance_id).ok_or_else(|| {
            CheckpointStoreError::new(
                CheckpointReasonCode::StaleProcessEvent,
                "checkpoint event has no active process",
            )
        })?;
        if entry.status.expected_pid != Some(expected_pid) {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::StaleProcessEvent,
                "checkpoint event belongs to a stale process",
            ));
        }
        if !legal_checkpoint_transition(entry.status.phase, next) {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::InvalidStateTransition,
                "checkpoint state transition is invalid",
            ));
        }
        entry.status.phase = next;
        entry.status.routable = next.is_routable();
        entry.status.updated_at = checkpoint_now_millis();
        Ok(entry.status.clone())
    }

    fn update_operation(
        &self,
        instance_id: &str,
        expected_pid: u32,
        update: CheckpointOperationUpdate<'_>,
    ) -> StoreResult<CheckpointStatus> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CheckpointStoreError::io("checkpoint status lock failed"))?;
        let entry = entries.get_mut(instance_id).ok_or_else(|| {
            CheckpointStoreError::new(
                CheckpointReasonCode::StaleProcessEvent,
                "checkpoint event has no active process",
            )
        })?;
        if entry.status.expected_pid != Some(expected_pid) {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::StaleProcessEvent,
                "checkpoint event belongs to a stale process",
            ));
        }
        entry.status.last_operation = update.operation;
        entry.status.last_outcome = update.outcome;
        entry.status.reason_code = update.reason_code;
        entry.status.message = update.message.into();
        entry.status.duration_ms = Some(update.duration_ms);
        if let Some(manifest) = update.generation {
            entry.status.generation_id = Some(manifest.generation_id.clone());
            entry.status.prompt_tokens = Some(manifest.slot().prompt_tokens);
            entry.status.bytes = Some(manifest.slot().bytes);
        }
        entry.status.updated_at = checkpoint_now_millis();
        Ok(entry.status.clone())
    }

    pub fn on_engine_healthy(
        &self,
        instance_id: &str,
        expected_pid: u32,
    ) -> StoreResult<CheckpointStatus> {
        if !self.gate_active(instance_id) {
            return self.status(instance_id).ok_or_else(|| {
                CheckpointStoreError::new(
                    CheckpointReasonCode::StaleProcessEvent,
                    "checkpoint event has no active process",
                )
            });
        }
        self.transition(instance_id, expected_pid, CheckpointPhase::EngineHealthy)
    }

    fn finish_ready_cold(
        &self,
        instance_id: &str,
        expected_pid: u32,
        update: CheckpointOperationUpdate<'_>,
    ) -> StoreResult<CheckpointStatus> {
        self.transition(instance_id, expected_pid, CheckpointPhase::ReadyCold)?;
        self.update_operation(instance_id, expected_pid, update)
    }

    fn restore_failure<B: SlotBackend + ?Sized>(
        &self,
        instance_id: &str,
        expected_pid: u32,
        backend: &B,
        error: CheckpointStoreError,
        started: Instant,
    ) -> StoreResult<CheckpointStatus> {
        let cleanup_required = self
            .status(instance_id)
            .is_some_and(|status| status.phase == CheckpointPhase::Restoring);
        let cleanup = backend.erase();
        let update = CheckpointOperationUpdate::new(
            CheckpointOperation::Restore,
            CheckpointOutcome::Failed,
            error.reason_code,
            error.message,
            None,
            started.elapsed().as_millis() as u64,
        );
        if cleanup_required && cleanup.is_err() {
            // A restore request may have modified slot state even when its
            // response failed. Keep the routing gate closed until a later
            // health pass can prove the erase request succeeded.
            self.update_operation(instance_id, expected_pid, update)
        } else {
            self.finish_ready_cold(instance_id, expected_pid, update)
        }
    }

    pub fn restore_cleanup_pending(&self, instance_id: &str, expected_pid: u32) -> bool {
        self.status(instance_id).is_some_and(|status| {
            status.expected_pid == Some(expected_pid)
                && status.phase == CheckpointPhase::Restoring
                && status.last_operation == CheckpointOperation::Restore
                && status.last_outcome == CheckpointOutcome::Failed
        })
    }

    pub fn retry_failed_restore_cleanup<B: SlotBackend + ?Sized>(
        &self,
        instance_id: &str,
        expected_pid: u32,
        backend: &B,
    ) -> StoreResult<CheckpointStatus> {
        if !self.restore_cleanup_pending(instance_id, expected_pid) {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::InvalidStateTransition,
                "checkpoint restore cleanup is not pending",
            ));
        }
        backend.erase()?;
        self.transition(instance_id, expected_pid, CheckpointPhase::ReadyCold)
    }

    pub fn fail_restore_setup(
        &self,
        instance_id: &str,
        expected_pid: u32,
        error: CheckpointStoreError,
    ) -> StoreResult<CheckpointStatus> {
        self.finish_ready_cold(
            instance_id,
            expected_pid,
            CheckpointOperationUpdate::new(
                CheckpointOperation::Restore,
                CheckpointOutcome::Failed,
                error.reason_code,
                error.message,
                None,
                0,
            ),
        )
    }

    pub fn restore_or_cold<B: SlotBackend + ?Sized>(
        &self,
        instance_id: &str,
        expected_pid: u32,
        fingerprint: &CheckpointFingerprint,
        auto_restore: bool,
        backend: &B,
    ) -> StoreResult<CheckpointStatus> {
        if !self.gate_active(instance_id) {
            return self.status(instance_id).ok_or_else(|| {
                CheckpointStoreError::new(
                    CheckpointReasonCode::StaleProcessEvent,
                    "checkpoint event has no active process",
                )
            });
        }
        let started = Instant::now();
        if !auto_restore {
            return self.finish_ready_cold(
                instance_id,
                expected_pid,
                CheckpointOperationUpdate::new(
                    CheckpointOperation::Restore,
                    CheckpointOutcome::Skipped,
                    CheckpointReasonCode::AutoRestoreDisabled,
                    "automatic checkpoint restore is disabled",
                    None,
                    started.elapsed().as_millis() as u64,
                ),
            );
        }
        if let Err(error) = fingerprint.validate() {
            return self.finish_ready_cold(
                instance_id,
                expected_pid,
                CheckpointOperationUpdate::new(
                    CheckpointOperation::Restore,
                    CheckpointOutcome::Failed,
                    error.reason_code,
                    error.message,
                    None,
                    started.elapsed().as_millis() as u64,
                ),
            );
        }
        let generation = match self.store.load_latest(instance_id, &fingerprint.digest) {
            Ok(Some(generation)) => generation,
            Ok(None) => {
                let mismatch = match self
                    .store
                    .has_other_fingerprint_generation(instance_id, &fingerprint.digest)
                {
                    Ok(mismatch) => mismatch,
                    Err(error) => {
                        return self.restore_failure(
                            instance_id,
                            expected_pid,
                            backend,
                            error,
                            started,
                        );
                    }
                };
                let (reason, message) = if mismatch {
                    (
                        CheckpointReasonCode::FingerprintMismatch,
                        "available checkpoint fingerprint does not match",
                    )
                } else {
                    (
                        CheckpointReasonCode::NoCheckpoint,
                        "no compatible checkpoint is available",
                    )
                };
                return self.finish_ready_cold(
                    instance_id,
                    expected_pid,
                    CheckpointOperationUpdate::new(
                        CheckpointOperation::Restore,
                        CheckpointOutcome::Skipped,
                        reason,
                        message,
                        None,
                        started.elapsed().as_millis() as u64,
                    ),
                );
            }
            Err(error) => {
                return self.restore_failure(instance_id, expected_pid, backend, error, started);
            }
        };

        self.transition(instance_id, expected_pid, CheckpointPhase::Restoring)?;
        let restore_path =
            match self
                .store
                .copy_generation_to_scratch(instance_id, expected_pid, &generation)
            {
                Ok(path) => path,
                Err(error) => {
                    return self.restore_failure(
                        instance_id,
                        expected_pid,
                        backend,
                        error,
                        started,
                    );
                }
            };
        let restore_filename = restore_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let mut verification_path = None;
        let restore_result = (|| {
            backend.health()?;
            let restored = backend.restore(&restore_filename)?;
            let slot = generation.manifest.slot();
            if restored.id_slot != 0
                || restored.filename != restore_filename
                || restored.n_restored != slot.prompt_tokens
                || restored.n_read != slot.bytes
            {
                return Err(CheckpointStoreError::new(
                    CheckpointReasonCode::RestoreResponseInvalid,
                    "slot restore response did not match the manifest",
                ));
            }

            // The restore endpoint has finished reading the source file when it
            // returns. Remove it before asking llama.cpp to save the verification copy so
            // a restore never needs generation + restore + verification payloads
            // on disk at the same time.
            self.store
                .remove_scratch_payload(instance_id, &restore_path)?;
            self.store
                .ensure_scratch_capacity(instance_id, slot.bytes)?;

            let verify_path = self
                .store
                .new_scratch_slot_path(instance_id, expected_pid)?;
            let verify_filename = verify_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            verification_path = Some(verify_path.clone());
            let verified = backend.save(&verify_filename)?;
            if verified.id_slot != 0
                || verified.filename != verify_filename
                || verified.n_saved != slot.prompt_tokens
                || verified.n_written != slot.bytes
            {
                return Err(CheckpointStoreError::new(
                    CheckpointReasonCode::SlotStateMismatch,
                    "restored slot did not retain the expected prompt state",
                ));
            }
            let verified_sha256 =
                self.store
                    .verify_scratch_payload(instance_id, &verify_path, verified.n_written)?;
            if verified_sha256 != slot.sha256 {
                return Err(CheckpointStoreError::new(
                    CheckpointReasonCode::SlotStateMismatch,
                    "restored slot payload did not match the checkpoint",
                ));
            }
            let slots = backend.slots()?;
            if slots.len() != 1 || slots[0].id != 0 || slots[0].is_processing || slots[0].n_ctx == 0
            {
                return Err(CheckpointStoreError::new(
                    CheckpointReasonCode::SlotStateMismatch,
                    "restored slot state was not idle and usable",
                ));
            }
            Ok(())
        })();

        let restore_cleanup = self
            .store
            .remove_scratch_payload(instance_id, &restore_path);
        let verification_cleanup = verification_path
            .as_deref()
            .map(|path| self.store.remove_scratch_payload(instance_id, path))
            .transpose();
        if let Err(error) = restore_result
            .and(restore_cleanup)
            .and(verification_cleanup.map(|_| ()))
        {
            return self.restore_failure(instance_id, expected_pid, backend, error, started);
        }

        self.transition(instance_id, expected_pid, CheckpointPhase::Ready)?;
        self.update_operation(
            instance_id,
            expected_pid,
            CheckpointOperationUpdate::new(
                CheckpointOperation::Restore,
                CheckpointOutcome::Success,
                CheckpointReasonCode::None,
                "checkpoint restored and verified",
                Some(&generation.manifest),
                started.elapsed().as_millis() as u64,
            ),
        )
    }

    pub fn begin_draining(
        &self,
        instance_id: &str,
        expected_pid: u32,
    ) -> StoreResult<CheckpointStatus> {
        if !self.gate_active(instance_id) {
            return self.status(instance_id).ok_or_else(|| {
                CheckpointStoreError::new(
                    CheckpointReasonCode::StaleProcessEvent,
                    "checkpoint event has no active process",
                )
            });
        }
        self.transition(instance_id, expected_pid, CheckpointPhase::Draining)
    }

    fn finish_stopping(
        &self,
        instance_id: &str,
        expected_pid: u32,
        update: CheckpointOperationUpdate<'_>,
    ) -> StoreResult<CheckpointStatus> {
        self.transition(instance_id, expected_pid, CheckpointPhase::Stopping)?;
        self.update_operation(instance_id, expected_pid, update)
    }

    pub fn skip_save_busy(
        &self,
        instance_id: &str,
        expected_pid: u32,
        duration_ms: u64,
    ) -> StoreResult<CheckpointStatus> {
        self.finish_stopping(
            instance_id,
            expected_pid,
            CheckpointOperationUpdate::new(
                CheckpointOperation::Save,
                CheckpointOutcome::Skipped,
                CheckpointReasonCode::BusyTimeout,
                "checkpoint drain timed out",
                None,
                duration_ms,
            ),
        )
    }

    pub fn skip_save_not_ready(
        &self,
        instance_id: &str,
        expected_pid: u32,
    ) -> StoreResult<CheckpointStatus> {
        self.finish_stopping(
            instance_id,
            expected_pid,
            CheckpointOperationUpdate::new(
                CheckpointOperation::Save,
                CheckpointOutcome::Skipped,
                CheckpointReasonCode::BusyTimeout,
                "checkpoint startup was not ready to save",
                None,
                0,
            ),
        )
    }

    pub fn fail_save_setup(
        &self,
        instance_id: &str,
        expected_pid: u32,
        error: CheckpointStoreError,
        duration_ms: u64,
    ) -> StoreResult<CheckpointStatus> {
        self.finish_stopping(
            instance_id,
            expected_pid,
            CheckpointOperationUpdate::new(
                CheckpointOperation::Save,
                CheckpointOutcome::Failed,
                error.reason_code,
                error.message,
                None,
                duration_ms,
            ),
        )
    }

    pub fn save_before_stop<B: SlotBackend + ?Sized>(
        &self,
        instance_id: &str,
        expected_pid: u32,
        fingerprint: &CheckpointFingerprint,
        config: &crate::models::KvCheckpointConfig,
        backend: &B,
    ) -> StoreResult<CheckpointStatus> {
        if !self.gate_active(instance_id) {
            return self.status(instance_id).ok_or_else(|| {
                CheckpointStoreError::new(
                    CheckpointReasonCode::StaleProcessEvent,
                    "checkpoint event has no active process",
                )
            });
        }
        let started = Instant::now();
        if !config.auto_save {
            return self.finish_stopping(
                instance_id,
                expected_pid,
                CheckpointOperationUpdate::new(
                    CheckpointOperation::Save,
                    CheckpointOutcome::Skipped,
                    CheckpointReasonCode::AutoSaveDisabled,
                    "automatic checkpoint save is disabled",
                    None,
                    started.elapsed().as_millis() as u64,
                ),
            );
        }
        if let Err(error) = fingerprint.validate() {
            return self.finish_stopping(
                instance_id,
                expected_pid,
                CheckpointOperationUpdate::new(
                    CheckpointOperation::Save,
                    CheckpointOutcome::Failed,
                    error.reason_code,
                    error.message,
                    None,
                    started.elapsed().as_millis() as u64,
                ),
            );
        }
        let slots = match backend.slots() {
            Ok(slots) => slots,
            Err(error) => {
                return self.finish_stopping(
                    instance_id,
                    expected_pid,
                    CheckpointOperationUpdate::new(
                        CheckpointOperation::Save,
                        CheckpointOutcome::Failed,
                        error.reason_code,
                        error.message,
                        None,
                        started.elapsed().as_millis() as u64,
                    ),
                );
            }
        };
        if slots.len() != 1 || slots[0].id != 0 || slots[0].n_ctx == 0 {
            return self.finish_stopping(
                instance_id,
                expected_pid,
                CheckpointOperationUpdate::new(
                    CheckpointOperation::Save,
                    CheckpointOutcome::Failed,
                    CheckpointReasonCode::SlotStateMismatch,
                    "slot zero state was unavailable",
                    None,
                    started.elapsed().as_millis() as u64,
                ),
            );
        }
        if slots[0].is_processing {
            return self.finish_stopping(
                instance_id,
                expected_pid,
                CheckpointOperationUpdate::new(
                    CheckpointOperation::Save,
                    CheckpointOutcome::Skipped,
                    CheckpointReasonCode::BusyTimeout,
                    "slot remained busy at the save boundary",
                    None,
                    started.elapsed().as_millis() as u64,
                ),
            );
        }

        self.transition(instance_id, expected_pid, CheckpointPhase::Saving)?;
        let scratch_path = match self.store.new_scratch_slot_path(instance_id, expected_pid) {
            Ok(path) => path,
            Err(error) => {
                return self.finish_stopping(
                    instance_id,
                    expected_pid,
                    CheckpointOperationUpdate::new(
                        CheckpointOperation::Save,
                        CheckpointOutcome::Failed,
                        error.reason_code,
                        error.message,
                        None,
                        started.elapsed().as_millis() as u64,
                    ),
                );
            }
        };
        let filename = scratch_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let operation = (|| {
            let saved = backend.save(&filename)?;
            if saved.id_slot != 0 || saved.filename != filename || saved.n_written == 0 {
                return Err(CheckpointStoreError::new(
                    CheckpointReasonCode::SaveResponseInvalid,
                    "slot save response did not match the request",
                ));
            }
            if saved.n_saved < u64::from(config.minimum_prompt_tokens) {
                return Err(CheckpointStoreError::new(
                    CheckpointReasonCode::BelowTokenThreshold,
                    "slot prompt state is below the save threshold",
                ));
            }
            let payload_sha256 =
                self.store
                    .verify_scratch_payload(instance_id, &scratch_path, saved.n_written)?;
            let manifest = CheckpointManifestV1::new(
                instance_id,
                fingerprint.clone(),
                saved.n_saved,
                saved.n_written,
                payload_sha256,
            );
            let storage_limit_bytes = u64::from(config.storage_limit_gib)
                .saturating_mul(1024)
                .saturating_mul(1024)
                .saturating_mul(1024);
            self.store
                .commit_generation(&manifest, &scratch_path, storage_limit_bytes)?;
            self.store.prune_instance(
                instance_id,
                storage_limit_bytes,
                &[(fingerprint.digest.clone(), manifest.generation_id.clone())],
            )?;
            Ok(manifest)
        })();
        let cleanup = self
            .store
            .remove_scratch_payload(instance_id, &scratch_path);
        match (operation, cleanup) {
            (Ok(manifest), Ok(())) => self.finish_stopping(
                instance_id,
                expected_pid,
                CheckpointOperationUpdate::new(
                    CheckpointOperation::Save,
                    CheckpointOutcome::Success,
                    CheckpointReasonCode::None,
                    "checkpoint saved and committed",
                    Some(&manifest),
                    started.elapsed().as_millis() as u64,
                ),
            ),
            (Ok(manifest), Err(error)) => self.finish_stopping(
                instance_id,
                expected_pid,
                CheckpointOperationUpdate::new(
                    CheckpointOperation::Save,
                    CheckpointOutcome::Failed,
                    error.reason_code,
                    error.message,
                    Some(&manifest),
                    started.elapsed().as_millis() as u64,
                ),
            ),
            (Err(_), Err(cleanup_error)) => self.finish_stopping(
                instance_id,
                expected_pid,
                CheckpointOperationUpdate::new(
                    CheckpointOperation::Save,
                    CheckpointOutcome::Failed,
                    cleanup_error.reason_code,
                    cleanup_error.message,
                    None,
                    started.elapsed().as_millis() as u64,
                ),
            ),
            (Err(error), Ok(()))
                if error.reason_code == CheckpointReasonCode::BelowTokenThreshold =>
            {
                self.finish_stopping(
                    instance_id,
                    expected_pid,
                    CheckpointOperationUpdate::new(
                        CheckpointOperation::Save,
                        CheckpointOutcome::Skipped,
                        error.reason_code,
                        error.message,
                        None,
                        started.elapsed().as_millis() as u64,
                    ),
                )
            }
            (Err(error), Ok(())) => self.finish_stopping(
                instance_id,
                expected_pid,
                CheckpointOperationUpdate::new(
                    CheckpointOperation::Save,
                    CheckpointOutcome::Failed,
                    error.reason_code,
                    error.message,
                    None,
                    started.elapsed().as_millis() as u64,
                ),
            ),
        }
    }

    pub fn mark_stopped(
        &self,
        instance_id: &str,
        expected_pid: u32,
    ) -> StoreResult<CheckpointStatus> {
        let status = if self.gate_active(instance_id) {
            self.transition(instance_id, expected_pid, CheckpointPhase::Stopped)?
        } else {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| CheckpointStoreError::io("checkpoint status lock failed"))?;
            let entry = entries.get_mut(instance_id).ok_or_else(|| {
                CheckpointStoreError::new(
                    CheckpointReasonCode::StaleProcessEvent,
                    "checkpoint event has no active process",
                )
            })?;
            if entry.status.expected_pid != Some(expected_pid) {
                return Err(CheckpointStoreError::new(
                    CheckpointReasonCode::StaleProcessEvent,
                    "checkpoint event belongs to a stale process",
                ));
            }
            entry.status.phase = CheckpointPhase::Stopped;
            entry.status.routable = false;
            entry.status.updated_at = checkpoint_now_millis();
            entry.status.clone()
        };
        if let Ok(mut entries) = self.entries.lock() {
            if let Some(entry) = entries.get_mut(instance_id) {
                entry.gate_active = false;
            }
        }
        Ok(status)
    }

    pub fn resume_after_stop_failure(
        &self,
        instance_id: &str,
        expected_pid: u32,
        ready_phase: CheckpointPhase,
    ) -> StoreResult<CheckpointStatus> {
        if !matches!(
            ready_phase,
            CheckpointPhase::Ready | CheckpointPhase::ReadyCold
        ) {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::InvalidStateTransition,
                "checkpoint stop recovery phase is invalid",
            ));
        }
        self.transition(instance_id, expected_pid, ready_phase)
    }

    pub fn mark_unexpected_exit(
        &self,
        instance_id: &str,
        expected_pid: u32,
    ) -> StoreResult<CheckpointStatus> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CheckpointStoreError::io("checkpoint status lock failed"))?;
        let entry = entries.get_mut(instance_id).ok_or_else(|| {
            CheckpointStoreError::new(
                CheckpointReasonCode::StaleProcessEvent,
                "checkpoint event has no active process",
            )
        })?;
        if entry.status.expected_pid != Some(expected_pid) {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::StaleProcessEvent,
                "checkpoint event belongs to a stale process",
            ));
        }
        let expected_stop = entry.status.phase == CheckpointPhase::Stopping;
        entry.gate_active = false;
        entry.status.phase = CheckpointPhase::Stopped;
        entry.status.routable = false;
        if !expected_stop {
            entry.status.last_outcome = CheckpointOutcome::Skipped;
            entry.status.reason_code = CheckpointReasonCode::UnexpectedExit;
            entry.status.message = "process exited before checkpoint save".into();
        }
        entry.status.updated_at = checkpoint_now_millis();
        Ok(entry.status.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{InstanceConfig, KvCheckpointConfig};
    use axum::body::Bytes;
    use axum::extract::{Query, State};
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use tokio::sync::oneshot;

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

    #[derive(Debug)]
    struct FakeLlamaHttpState {
        health_ok: bool,
        processing: bool,
        prompt_tokens: u64,
        last_saved_tokens: u64,
        payload: Vec<u8>,
        health_delay: Duration,
        save_delay: Duration,
        restore_delay: Duration,
        restore_applies: bool,
        malformed_action: Option<String>,
        erase_count: u32,
        events: Vec<String>,
    }

    impl Default for FakeLlamaHttpState {
        fn default() -> Self {
            Self {
                health_ok: true,
                processing: false,
                prompt_tokens: 0,
                last_saved_tokens: 0,
                payload: Vec::new(),
                health_delay: Duration::ZERO,
                save_delay: Duration::ZERO,
                restore_delay: Duration::ZERO,
                restore_applies: true,
                malformed_action: None,
                erase_count: 0,
                events: Vec::new(),
            }
        }
    }

    #[derive(Clone)]
    struct FakeLlamaHttpContext {
        scratch: PathBuf,
        state: Arc<Mutex<FakeLlamaHttpState>>,
        action_lock: Arc<tokio::sync::Mutex<()>>,
    }

    struct FakeLlamaHttpServer {
        port: u16,
        state: Arc<Mutex<FakeLlamaHttpState>>,
        shutdown: Option<oneshot::Sender<()>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl FakeLlamaHttpServer {
        async fn start(scratch: PathBuf) -> Self {
            let state = Arc::new(Mutex::new(FakeLlamaHttpState::default()));
            let context = FakeLlamaHttpContext {
                scratch,
                state: state.clone(),
                action_lock: Arc::new(tokio::sync::Mutex::new(())),
            };
            let router = Router::new()
                .route("/health", get(fake_llama_health))
                .route("/slots", get(fake_llama_slots))
                .route("/slots/0", post(fake_llama_slot_action))
                .with_state(context);
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let port = listener.local_addr().unwrap().port();
            let (shutdown, receiver) = oneshot::channel();
            let task = tokio::spawn(async move {
                axum::serve(listener, router)
                    .with_graceful_shutdown(async move {
                        let _ = receiver.await;
                    })
                    .await
                    .unwrap();
            });
            Self {
                port,
                state,
                shutdown: Some(shutdown),
                task,
            }
        }

        fn configure(&self, configure: impl FnOnce(&mut FakeLlamaHttpState)) {
            configure(&mut self.state.lock().unwrap());
        }

        fn events(&self) -> Vec<String> {
            self.state.lock().unwrap().events.clone()
        }

        async fn stop(mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            self.task.await.unwrap();
        }
    }

    fn malformed_json_response() -> Response {
        (
            StatusCode::OK,
            [("content-type", "application/json")],
            "not-json",
        )
            .into_response()
    }

    async fn fake_llama_health(State(context): State<FakeLlamaHttpContext>) -> Response {
        let (health_ok, delay, malformed) = {
            let mut state = context.state.lock().unwrap();
            state.events.push("health".into());
            (
                state.health_ok,
                state.health_delay,
                state.malformed_action.as_deref() == Some("health"),
            )
        };
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        if malformed {
            return malformed_json_response();
        }
        Json(serde_json::json!({
            "status": if health_ok { "ok" } else { "loading model" }
        }))
        .into_response()
    }

    async fn fake_llama_slots(State(context): State<FakeLlamaHttpContext>) -> Response {
        let (processing, prompt_tokens, malformed) = {
            let mut state = context.state.lock().unwrap();
            state.events.push("slots".into());
            (
                state.processing,
                state.prompt_tokens,
                state.malformed_action.as_deref() == Some("slots"),
            )
        };
        if malformed {
            return malformed_json_response();
        }
        Json(serde_json::json!([{
            "id": 0,
            "is_processing": processing,
            "n_ctx": prompt_tokens
        }]))
        .into_response()
    }

    async fn fake_llama_slot_action(
        State(context): State<FakeLlamaHttpContext>,
        Query(query): Query<HashMap<String, String>>,
        body: Bytes,
    ) -> Response {
        let _action_guard = context.action_lock.lock().await;
        let action = query.get("action").map(String::as_str).unwrap_or_default();
        let (delay, malformed) = {
            let mut state = context.state.lock().unwrap();
            state.events.push(action.to_string());
            let delay = match action {
                "save" => state.save_delay,
                "restore" => state.restore_delay,
                _ => Duration::ZERO,
            };
            (delay, state.malformed_action.as_deref() == Some(action))
        };
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        if malformed {
            return malformed_json_response();
        }

        if action == "erase" {
            let mut state = context.state.lock().unwrap();
            let n_erased = state.prompt_tokens;
            state.prompt_tokens = 0;
            state.payload.clear();
            state.erase_count += 1;
            return Json(serde_json::json!({ "id_slot": 0, "n_erased": n_erased })).into_response();
        }

        let filename = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| value["filename"].as_str().map(str::to_string));
        let Some(filename) = filename.filter(|value| is_manager_slot_filename(value)) else {
            return (StatusCode::BAD_REQUEST, "invalid filename").into_response();
        };
        let path = context.scratch.join(&filename);
        match action {
            "save" => {
                let (tokens, payload) = {
                    let state = context.state.lock().unwrap();
                    (state.prompt_tokens, state.payload.clone())
                };
                if fs::write(&path, &payload).is_err() {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "save failed").into_response();
                }
                context.state.lock().unwrap().last_saved_tokens = tokens;
                Json(serde_json::json!({
                    "id_slot": 0,
                    "filename": filename,
                    "n_saved": tokens,
                    "n_written": payload.len()
                }))
                .into_response()
            }
            "restore" => {
                let Ok(payload) = fs::read(&path) else {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "restore failed").into_response();
                };
                let mut state = context.state.lock().unwrap();
                let restored_tokens = state.last_saved_tokens.max(1);
                if state.restore_applies {
                    state.prompt_tokens = restored_tokens;
                    state.payload.clone_from(&payload);
                }
                Json(serde_json::json!({
                    "id_slot": 0,
                    "filename": filename,
                    "n_restored": restored_tokens,
                    "n_read": payload.len()
                }))
                .into_response()
            }
            _ => (StatusCode::BAD_REQUEST, "unsupported action").into_response(),
        }
    }

    fn evaluate_with_context_persistence(
        config: &InstanceConfig,
        context_checkpoint_persistence: bool,
    ) -> CheckpointEligibility {
        evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
            config,
            workload: ModelWorkload::Inference,
            managed_local_engine: true,
            engine_capabilities: EngineCheckpointCapabilities {
                slots: true,
                slot_save_path: true,
                cache_ram: true,
                cache_idle_slots: true,
                swa_full: true,
                context_checkpoint_persistence,
            },
            engine_speculative_types: &[
                "ngram-mod".into(),
                "ngram-cache".into(),
                "draft-mtp".into(),
                "draft-dflash".into(),
            ],
            model_architecture: Some("llama"),
            model_artifacts_complete: true,
            model_has_swa: Some(false),
        })
    }

    fn evaluate(config: &InstanceConfig) -> CheckpointEligibility {
        evaluate_with_context_persistence(config, true)
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_slot_checkpoint_round_trip_gates_restore_and_erases_partial_state() {
        const INSTANCE_ID: &str = "http-round-trip";
        const PRIVATE_PAYLOAD: &[u8] = b"private prompt-derived slot bytes";
        const PRIVATE_API_KEY: &str = "private-test-api-key";

        let sandbox = TestSandbox::new("http-round-trip");
        let store = sandbox.store();
        let scratch = store.prepare_instance(INSTANCE_ID).unwrap();
        let server = FakeLlamaHttpServer::start(scratch).await;
        let mut config = eligible_config();
        config.port = server.port;
        config.api_key = PRIVATE_API_KEY.into();
        config.kv_checkpoint.minimum_prompt_tokens = 1;
        let fingerprint = test_fingerprint(&config);
        let eligibility = evaluate(&config);

        server.configure(|state| {
            state.prompt_tokens = 1_536;
            state.payload = PRIVATE_PAYLOAD.to_vec();
        });
        let first = CheckpointCoordinator::new(store.clone());
        let starting = first
            .register_start_with_context(
                INSTANCE_ID,
                901,
                &eligibility,
                Some(fingerprint.clone()),
                config.kv_checkpoint.clone(),
            )
            .unwrap();
        assert_eq!(starting.phase, CheckpointPhase::Starting);
        assert!(!first.gate_allows_routing(INSTANCE_ID));
        assert_eq!(
            first.on_engine_healthy(INSTANCE_ID, 901).unwrap().phase,
            CheckpointPhase::EngineHealthy
        );
        let cold_coordinator = first.clone();
        let cold_config = config.clone();
        let cold_fingerprint = fingerprint.clone();
        let cold = tokio::task::spawn_blocking(move || {
            let client = LlamaSlotClient::new(&cold_config, Duration::from_secs(2)).unwrap();
            cold_coordinator.restore_or_cold(INSTANCE_ID, 901, &cold_fingerprint, true, &client)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(cold.phase, CheckpointPhase::ReadyCold);
        assert_eq!(cold.reason_code, CheckpointReasonCode::NoCheckpoint);
        assert!(first.gate_allows_routing(INSTANCE_ID));

        assert_eq!(
            first.begin_draining(INSTANCE_ID, 901).unwrap().phase,
            CheckpointPhase::Draining
        );
        assert!(!first.gate_allows_routing(INSTANCE_ID));
        let save_coordinator = first.clone();
        let save_config = config.clone();
        let save_fingerprint = fingerprint.clone();
        let saved = tokio::task::spawn_blocking(move || {
            let client = LlamaSlotClient::new(&save_config, Duration::from_secs(2)).unwrap();
            save_coordinator.save_before_stop(
                INSTANCE_ID,
                901,
                &save_fingerprint,
                &save_config.kv_checkpoint,
                &client,
            )
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(saved.phase, CheckpointPhase::Stopping);
        assert_eq!(saved.last_outcome, CheckpointOutcome::Success);
        first.mark_stopped(INSTANCE_ID, 901).unwrap();
        let committed = store
            .load_latest(INSTANCE_ID, &fingerprint.digest)
            .unwrap()
            .unwrap();
        assert_eq!(committed.manifest.slot().prompt_tokens, 1_536);
        assert_eq!(
            committed.manifest.slot().bytes,
            PRIVATE_PAYLOAD.len() as u64
        );

        server.configure(|state| {
            state.prompt_tokens = 0;
            state.payload.clear();
            state.restore_delay = Duration::from_millis(300);
            state.events.clear();
        });
        let second = CheckpointCoordinator::new(store.clone());
        second
            .register_start_with_context(
                INSTANCE_ID,
                902,
                &eligibility,
                Some(fingerprint.clone()),
                config.kv_checkpoint.clone(),
            )
            .unwrap();
        second.on_engine_healthy(INSTANCE_ID, 902).unwrap();
        let restore_coordinator = second.clone();
        let restore_config = config.clone();
        let restore_fingerprint = fingerprint.clone();
        let restore = tokio::task::spawn_blocking(move || {
            let client = LlamaSlotClient::new(&restore_config, Duration::from_secs(2)).unwrap();
            restore_coordinator.restore_or_cold(
                INSTANCE_ID,
                902,
                &restore_fingerprint,
                true,
                &client,
            )
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if second
                    .status(INSTANCE_ID)
                    .is_some_and(|status| status.phase == CheckpointPhase::Restoring)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(!second.gate_allows_routing(INSTANCE_ID));
        let restored = restore.await.unwrap().unwrap();
        assert_eq!(restored.phase, CheckpointPhase::Ready);
        assert_eq!(restored.last_outcome, CheckpointOutcome::Success);
        assert!(second.gate_allows_routing(INSTANCE_ID));
        assert_eq!(server.events(), ["health", "restore", "save", "slots"]);
        {
            let state = server.state.lock().unwrap();
            assert_eq!(state.prompt_tokens, 1_536);
            assert_eq!(state.payload, PRIVATE_PAYLOAD);
        }
        let public_status = serde_json::to_string(&restored).unwrap();
        assert!(!public_status.contains(std::str::from_utf8(PRIVATE_PAYLOAD).unwrap()));
        assert!(!public_status.contains(PRIVATE_API_KEY));

        server.configure(|state| {
            state.prompt_tokens = 7;
            state.payload = b"unverified state".to_vec();
            state.restore_delay = Duration::ZERO;
            state.restore_applies = false;
            state.erase_count = 0;
            state.events.clear();
        });
        let third = CheckpointCoordinator::new(store.clone());
        third
            .register_start_with_context(
                INSTANCE_ID,
                903,
                &eligibility,
                Some(fingerprint.clone()),
                config.kv_checkpoint.clone(),
            )
            .unwrap();
        third.on_engine_healthy(INSTANCE_ID, 903).unwrap();
        let partial_coordinator = third.clone();
        let partial_config = config.clone();
        let partial_fingerprint = fingerprint.clone();
        let partial = tokio::task::spawn_blocking(move || {
            let client = LlamaSlotClient::new(&partial_config, Duration::from_secs(2)).unwrap();
            partial_coordinator.restore_or_cold(
                INSTANCE_ID,
                903,
                &partial_fingerprint,
                true,
                &client,
            )
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(partial.phase, CheckpointPhase::ReadyCold);
        assert_eq!(partial.last_outcome, CheckpointOutcome::Failed);
        assert_eq!(partial.reason_code, CheckpointReasonCode::SlotStateMismatch);
        assert!(third.gate_allows_routing(INSTANCE_ID));
        {
            let state = server.state.lock().unwrap();
            assert_eq!(state.erase_count, 1);
            assert_eq!(state.prompt_tokens, 0);
            assert!(state.payload.is_empty());
        }
        assert_eq!(server.events(), ["health", "restore", "save", "erase"]);

        server.configure(|state| {
            state.restore_applies = true;
            state.malformed_action = Some("restore".into());
            state.erase_count = 0;
            state.events.clear();
        });
        let fourth = CheckpointCoordinator::new(store);
        fourth
            .register_start_with_context(
                INSTANCE_ID,
                904,
                &eligibility,
                Some(fingerprint.clone()),
                config.kv_checkpoint.clone(),
            )
            .unwrap();
        fourth.on_engine_healthy(INSTANCE_ID, 904).unwrap();
        let malformed_coordinator = fourth.clone();
        let malformed_config = config.clone();
        let malformed_fingerprint = fingerprint.clone();
        let malformed = tokio::task::spawn_blocking(move || {
            let client = LlamaSlotClient::new(&malformed_config, Duration::from_secs(2)).unwrap();
            malformed_coordinator.restore_or_cold(
                INSTANCE_ID,
                904,
                &malformed_fingerprint,
                true,
                &client,
            )
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(malformed.phase, CheckpointPhase::ReadyCold);
        assert_eq!(
            malformed.reason_code,
            CheckpointReasonCode::RestoreResponseInvalid
        );
        assert_eq!(server.state.lock().unwrap().erase_count, 1);

        server.stop().await;
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
    fn checkpoint_allows_draft_state_only_with_a_compatible_context_appendix() {
        let mut config = eligible_config();
        config.spec_type = "ngram-cache,ngram-mod".into();
        assert!(evaluate(&config).eligible);

        config.spec_type = "ngram-mod,draft-mtp".into();
        assert!(evaluate_with_context_persistence(&config, false)
            .reasons
            .contains(&CheckpointReasonCode::SpeculativeDecodingUnsupported));
        assert!(evaluate(&config).eligible);

        config.spec_type = "draft-dflash".into();
        config.draft_model_path = "draft.gguf".into();
        assert!(evaluate(&config).eligible);

        config.spec_type = "ngram-mod".into();
        assert!(evaluate(&config)
            .reasons
            .contains(&CheckpointReasonCode::SpeculativeDecodingUnsupported));
        config.draft_model_path.clear();

        config.spec_type = "ngram-cache".into();
        config.lookup_cache_static = "external-cache.bin".into();
        assert!(evaluate(&config)
            .reasons
            .contains(&CheckpointReasonCode::SpeculativeDecodingUnsupported));

        config.lookup_cache_static.clear();
        config.spec_type = "ngram-future".into();
        assert!(evaluate(&config)
            .reasons
            .contains(&CheckpointReasonCode::SpeculativeDecodingUnsupported));

        config.spec_type = "ngram-simple".into();
        assert!(evaluate(&config)
            .reasons
            .contains(&CheckpointReasonCode::SpeculativeDecodingUnsupported));

        config.spec_type = "none".into();
        assert!(evaluate(&config).eligible);
    }

    #[test]
    fn checkpoint_allows_only_valid_lazy_loading_custom_arguments() {
        for custom_args in [
            vec!["--tensor-read-lazy".into(), "on".into()],
            vec!["--tensor-read-lazy=auto".into()],
            vec!["--lazy-mode off".into()],
            vec!["-lzm".into(), "on".into()],
        ] {
            let mut config = eligible_config();
            config.custom_args = custom_args;
            let result = evaluate(&config);
            assert!(result.eligible, "unexpected blockers: {result:?}");
            assert!(result.custom_argument_blockers.is_empty());
        }

        let mut same_as_structured = eligible_config();
        same_as_structured.lazy_mode = "on".into();
        same_as_structured.custom_args = vec!["--tensor-read-lazy on".into()];
        assert!(evaluate(&same_as_structured).eligible);
    }

    #[test]
    fn checkpoint_reports_unsafe_invalid_and_conflicting_custom_flags() {
        for (custom_args, structured, expected) in [
            (vec!["--ctx-size=4096"], "", vec!["--ctx-size"]),
            (vec!["--tensor-read-lazy"], "", vec!["--tensor-read-lazy"]),
            (
                vec!["--tensor-read-lazy maybe"],
                "",
                vec!["--tensor-read-lazy"],
            ),
            (
                vec!["--lazy-mode on --tensor-read-lazy off"],
                "",
                vec!["--tensor-read-lazy"],
            ),
            (
                vec!["--tensor-read-lazy off"],
                "on",
                vec!["--tensor-read-lazy"],
            ),
            (
                vec!["--tensor-read-lazy on unexpected"],
                "",
                vec!["<positional-argument>"],
            ),
        ] {
            let mut config = eligible_config();
            config.lazy_mode = structured.into();
            config.custom_args = custom_args.into_iter().map(str::to_string).collect();
            let result = evaluate(&config);
            assert!(!result.eligible);
            assert!(result
                .reasons
                .contains(&CheckpointReasonCode::CustomArgumentsUnsupported));
            assert_eq!(result.custom_argument_blockers, expected);
        }
    }

    #[test]
    fn stable_v040_custom_parameters_do_not_relax_checkpoint_eligibility() {
        for argument in [
            "--kv-unified-per-slot 4096",
            "--n-cpu-ffn 2",
            "-ncffn 2",
            "--spec-synth-len 4",
            "--spec-synth-rates 0.8,0.5",
            "--video-fps 4",
            "--video-timestamp-interval 5000",
            "--video-ffmpeg-dir /tmp/ffmpeg",
        ] {
            let mut config = eligible_config();
            config.custom_args = vec![argument.into()];
            let result = evaluate(&config);
            assert!(!result.eligible, "unexpectedly accepted {argument}");
            assert!(result
                .reasons
                .contains(&CheckpointReasonCode::CustomArgumentsUnsupported));
            assert_eq!(
                result.custom_argument_blockers,
                vec![argument.split_whitespace().next().unwrap()]
            );
        }
    }

    #[test]
    fn eligibility_rejects_every_unsupported_config_row() {
        assert_config_reason(CheckpointReasonCode::UnsupportedConfiguration, |config| {
            config.kv_checkpoint.storage_limit_gib = 0
        });
        assert_config_reason(CheckpointReasonCode::UnsupportedConfiguration, |config| {
            config.kv_checkpoint.minimum_prompt_tokens = 1_048_577
        });
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
        assert_config_reason(
            CheckpointReasonCode::PromptCacheRetentionRequired,
            |config| config.cache_idle_slots = false,
        );
        assert_config_reason(
            CheckpointReasonCode::PromptCacheRetentionRequired,
            |config| config.cache_ram = 0,
        );
        assert_config_reason(
            CheckpointReasonCode::PromptCacheRetentionRequired,
            |config| config.cache_ram = -2,
        );
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
                cache_ram: true,
                cache_idle_slots: true,
                swa_full: true,
                context_checkpoint_persistence: true,
            },
            engine_speculative_types: &[],
            model_architecture: Some("llama"),
            model_artifacts_complete: true,
            model_has_swa: Some(false),
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
                cache_ram: true,
                cache_idle_slots: true,
                swa_full: true,
                context_checkpoint_persistence: true,
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

        let qwen4exp = evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
            model_architecture: Some("qwen4exp"),
            ..base
        });
        assert!(qwen4exp
            .reasons
            .contains(&CheckpointReasonCode::HybridRecurrentUnsupported));

        let unknown = evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
            model_architecture: None,
            ..base
        });
        assert!(unknown
            .reasons
            .contains(&CheckpointReasonCode::ModelArchitectureUnknown));

        let incomplete = evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
            model_artifacts_complete: false,
            ..base
        });
        assert!(incomplete
            .reasons
            .contains(&CheckpointReasonCode::ModelArtifactsIncomplete));

        let sliding_window = evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
            model_has_swa: Some(true),
            ..base
        });
        assert!(sliding_window
            .reasons
            .contains(&CheckpointReasonCode::SlidingWindowRequiresFullCache));

        let mut full_swa_config = config.clone();
        full_swa_config.swa_full = true;
        let full_sliding_window = evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
            config: &full_swa_config,
            model_has_swa: Some(true),
            ..base
        });
        assert!(full_sliding_window.eligible);

        let missing_swa_capability =
            evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
                config: &full_swa_config,
                engine_capabilities: EngineCheckpointCapabilities {
                    swa_full: false,
                    ..base.engine_capabilities
                },
                model_has_swa: Some(true),
                ..base
            });
        assert!(missing_swa_capability
            .reasons
            .contains(&CheckpointReasonCode::EngineCapabilityMissing));
    }

    #[test]
    fn engine_capabilities_require_all_core_official_flags() {
        let flags = vec![
            "--slots".into(),
            "--slot-save-path".into(),
            "--cache-ram".into(),
            "--cache-idle-slots".into(),
            "--swa-full".into(),
        ];
        assert!(EngineCheckpointCapabilities::from_supported_flags(&flags).complete());
        let incomplete = vec![
            "--slots".into(),
            "--slot-save-path".into(),
            "--cache-ram".into(),
        ];
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
        assert_eq!(CheckpointPhase::Restoring.as_str(), "restoring");
        assert!(CheckpointPhase::Saving.is_busy());
        assert!(!CheckpointPhase::Starting.is_busy());
    }

    #[test]
    fn eligibility_contract_omits_empty_blockers_and_accepts_legacy_payloads() {
        let eligible = serde_json::to_value(evaluate(&eligible_config())).unwrap();
        assert!(eligible.get("custom_argument_blockers").is_none());

        let mut blocked_config = eligible_config();
        blocked_config.custom_args = vec!["--ctx-size 4096".into()];
        let blocked = serde_json::to_value(evaluate(&blocked_config)).unwrap();
        assert_eq!(
            blocked["custom_argument_blockers"],
            serde_json::json!(["--ctx-size"])
        );

        let legacy: CheckpointEligibility = serde_json::from_value(serde_json::json!({
            "eligible": false,
            "reason_code": "disabled",
            "reasons": ["disabled"]
        }))
        .unwrap();
        assert!(legacy.custom_argument_blockers.is_empty());
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
                CheckpointReasonCode::SaveResponseInvalid,
                "save_response_invalid",
            ),
            (
                CheckpointReasonCode::RestoreResponseInvalid,
                "restore_response_invalid",
            ),
            (
                CheckpointReasonCode::SlotStateMismatch,
                "slot_state_mismatch",
            ),
            (CheckpointReasonCode::StorageLimit, "storage_limit"),
            (
                CheckpointReasonCode::InsufficientDiskSpace,
                "insufficient_disk_space",
            ),
            (CheckpointReasonCode::IoError, "io_error"),
            (CheckpointReasonCode::HttpTimeout, "http_timeout"),
        ];
        for (reason, expected) in cases {
            assert_eq!(serde_json::to_value(reason).unwrap(), expected);
        }
    }

    struct TestSandbox {
        path: PathBuf,
    }

    impl TestSandbox {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("lsm-kv-checkpoint-{name}-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn store(&self) -> CheckpointStore {
            CheckpointStore::from_root(self.path.join("checkpoints"))
        }
    }

    impl Drop for TestSandbox {
        fn drop(&mut self) {
            let temp = std::env::temp_dir();
            let safe_name = self
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("lsm-kv-checkpoint-"));
            if safe_name && self.path.parent() == Some(temp.as_path()) {
                let _ = fs::remove_dir_all(&self.path);
            }
        }
    }

    fn digest(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn fingerprint_materials() -> FingerprintMaterials {
        FingerprintMaterials {
            model_sha256: digest(0x11),
            draft_model_sha256: None,
            engine_sha256: digest(0x22),
            engine_version: "b10679".into(),
            backend: "hip".into(),
            chat_template_file_sha256: None,
        }
    }

    fn test_fingerprint(config: &InstanceConfig) -> CheckpointFingerprint {
        build_checkpoint_fingerprint(config, &fingerprint_materials()).unwrap()
    }

    fn assert_fingerprint_changes(base: &InstanceConfig, mutate: impl FnOnce(&mut InstanceConfig)) {
        let expected = test_fingerprint(base);
        let mut changed = base.clone();
        mutate(&mut changed);
        assert_ne!(expected.digest, test_fingerprint(&changed).digest);
    }

    fn assert_fingerprint_stable(base: &InstanceConfig, mutate: impl FnOnce(&mut InstanceConfig)) {
        let expected = test_fingerprint(base);
        let mut changed = base.clone();
        mutate(&mut changed);
        assert_eq!(expected.digest, test_fingerprint(&changed).digest);
    }

    fn write_scratch_payload(
        store: &CheckpointStore,
        instance_id: &str,
        pid: u32,
        contents: &[u8],
    ) -> PathBuf {
        let path = store.new_scratch_slot_path(instance_id, pid).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }

    fn manifest_for_payload(
        instance_id: &str,
        fingerprint: &CheckpointFingerprint,
        payload: &Path,
        prompt_tokens: u64,
    ) -> CheckpointManifestV1 {
        let metadata = fs::metadata(payload).unwrap();
        CheckpointManifestV1::new(
            instance_id,
            fingerprint.clone(),
            prompt_tokens,
            metadata.len(),
            sha256_file(payload).unwrap(),
        )
    }

    #[derive(Debug)]
    struct FakeSlotState {
        n_ctx: u64,
        is_processing: bool,
        prompt_tokens: u64,
        payload: Vec<u8>,
        restore_tokens: u64,
        restore_applies: bool,
        restore_claim_tokens: Option<u64>,
        restore_claim_bytes: Option<u64>,
        save_claim_tokens: Option<u64>,
        save_claim_bytes: Option<u64>,
        save_payload_override: Option<Vec<u8>>,
        health_error: Option<CheckpointStoreError>,
        slots_error: Option<CheckpointStoreError>,
        save_error: Option<CheckpointStoreError>,
        restore_error: Option<CheckpointStoreError>,
        erase_error: Option<CheckpointStoreError>,
        erase_count: u32,
        events: Vec<String>,
    }

    impl Default for FakeSlotState {
        fn default() -> Self {
            Self {
                n_ctx: 4096,
                is_processing: false,
                prompt_tokens: 0,
                payload: Vec::new(),
                restore_tokens: 0,
                restore_applies: true,
                restore_claim_tokens: None,
                restore_claim_bytes: None,
                save_claim_tokens: None,
                save_claim_bytes: None,
                save_payload_override: None,
                health_error: None,
                slots_error: None,
                save_error: None,
                restore_error: None,
                erase_error: None,
                erase_count: 0,
                events: Vec::new(),
            }
        }
    }

    #[derive(Clone)]
    struct FakeSlotBackend {
        scratch: PathBuf,
        state: Arc<Mutex<FakeSlotState>>,
    }

    impl FakeSlotBackend {
        fn new(store: &CheckpointStore, instance_id: &str) -> Self {
            Self {
                scratch: store.prepare_instance(instance_id).unwrap(),
                state: Arc::new(Mutex::new(FakeSlotState::default())),
            }
        }

        fn configure(&self, configure: impl FnOnce(&mut FakeSlotState)) {
            configure(&mut self.state.lock().unwrap());
        }

        fn events(&self) -> Vec<String> {
            self.state.lock().unwrap().events.clone()
        }

        fn erase_count(&self) -> u32 {
            self.state.lock().unwrap().erase_count
        }
    }

    impl SlotBackend for FakeSlotBackend {
        fn health(&self) -> StoreResult<()> {
            let mut state = self.state.lock().unwrap();
            state.events.push("health".into());
            if let Some(error) = state.health_error.clone() {
                return Err(error);
            }
            Ok(())
        }

        fn slots(&self) -> StoreResult<Vec<SlotSnapshot>> {
            let mut state = self.state.lock().unwrap();
            state.events.push("slots".into());
            if let Some(error) = state.slots_error.clone() {
                return Err(error);
            }
            Ok(vec![SlotSnapshot {
                id: 0,
                is_processing: state.is_processing,
                n_ctx: state.n_ctx,
            }])
        }

        fn save(&self, filename: &str) -> StoreResult<SlotSaveResult> {
            LlamaSlotClient::manager_filename(filename)?;
            let mut state = self.state.lock().unwrap();
            state.events.push(format!("save:{filename}"));
            if let Some(error) = state.save_error.clone() {
                return Err(error);
            }
            let payload = state
                .save_payload_override
                .clone()
                .unwrap_or_else(|| state.payload.clone());
            fs::write(self.scratch.join(filename), &payload)
                .map_err(|_| CheckpointStoreError::io("fake slot save failed"))?;
            Ok(SlotSaveResult {
                id_slot: 0,
                filename: filename.into(),
                n_saved: state.save_claim_tokens.unwrap_or(state.prompt_tokens),
                n_written: state.save_claim_bytes.unwrap_or(payload.len() as u64),
            })
        }

        fn restore(&self, filename: &str) -> StoreResult<SlotRestoreResult> {
            LlamaSlotClient::manager_filename(filename)?;
            let mut state = self.state.lock().unwrap();
            state.events.push(format!("restore:{filename}"));
            if let Some(error) = state.restore_error.clone() {
                return Err(error);
            }
            let payload = fs::read(self.scratch.join(filename))
                .map_err(|_| CheckpointStoreError::io("fake slot restore failed"))?;
            let restore_tokens = state.restore_tokens;
            if state.restore_applies {
                state.prompt_tokens = restore_tokens;
                state.payload.clone_from(&payload);
            }
            Ok(SlotRestoreResult {
                id_slot: 0,
                filename: filename.into(),
                n_restored: state.restore_claim_tokens.unwrap_or(restore_tokens),
                n_read: state.restore_claim_bytes.unwrap_or(payload.len() as u64),
            })
        }

        fn erase(&self) -> StoreResult<SlotEraseResult> {
            let mut state = self.state.lock().unwrap();
            state.events.push("erase".into());
            state.erase_count += 1;
            if let Some(error) = state.erase_error.clone() {
                return Err(error);
            }
            let n_erased = state.prompt_tokens;
            state.prompt_tokens = 0;
            state.payload.clear();
            Ok(SlotEraseResult {
                id_slot: 0,
                n_erased,
            })
        }
    }

    fn coordinator_with_healthy_engine(
        store: &CheckpointStore,
        instance_id: &str,
        pid: u32,
    ) -> CheckpointCoordinator {
        let coordinator = CheckpointCoordinator::new(store.clone());
        let config = eligible_config();
        let eligibility = evaluate(&config);
        let starting = coordinator
            .register_start_with_context(
                instance_id,
                pid,
                &eligibility,
                Some(test_fingerprint(&config)),
                config.kv_checkpoint,
            )
            .unwrap();
        assert_eq!(starting.phase, CheckpointPhase::Starting);
        assert!(!coordinator.gate_allows_routing(instance_id));
        let healthy = coordinator.on_engine_healthy(instance_id, pid).unwrap();
        assert_eq!(healthy.phase, CheckpointPhase::EngineHealthy);
        coordinator
    }

    fn commit_test_generation(
        store: &CheckpointStore,
        instance_id: &str,
        fingerprint: &CheckpointFingerprint,
        prompt_tokens: u64,
        payload: &[u8],
    ) -> CheckpointManifestV1 {
        let scratch = write_scratch_payload(store, instance_id, 999, payload);
        let manifest = manifest_for_payload(instance_id, fingerprint, &scratch, prompt_tokens);
        store
            .commit_generation(&manifest, &scratch, 1024 * 1024)
            .unwrap();
        store.remove_scratch_payload(instance_id, &scratch).unwrap();
        manifest
    }

    #[test]
    fn fingerprint_is_sensitive_to_every_state_bearing_field() {
        let base = eligible_config();
        assert_fingerprint_changes(&base, |config| config.ctx_size = 4096);
        assert_fingerprint_changes(&base, |config| config.ctx_size_auto = true);
        assert_fingerprint_changes(&base, |config| config.parallel = 2);
        assert_fingerprint_changes(&base, |config| config.cont_batching = false);
        assert_fingerprint_changes(&base, |config| config.cache_prompt = false);
        assert_fingerprint_changes(&base, |config| config.cache_reuse = 128);
        assert_fingerprint_changes(&base, |config| config.cache_ram = 4096);
        assert_fingerprint_changes(&base, |config| config.cache_idle_slots = false);
        assert_fingerprint_changes(&base, |config| config.ctx_checkpoints = 16);
        assert_fingerprint_changes(&base, |config| config.checkpoint_min_step = 4096);
        assert_fingerprint_changes(&base, |config| config.slots_enabled = false);
        assert_fingerprint_changes(&base, |config| config.slot_prompt_similarity = 0.5);
        assert_fingerprint_changes(&base, |config| config.prefill_assistant = false);
        assert_fingerprint_changes(&base, |config| config.kv_unified = true);
        assert_fingerprint_changes(&base, |config| config.kv_unified_mode = "on".into());
        assert_fingerprint_changes(&base, |config| config.cache_type_k = "q8_0".into());
        assert_fingerprint_changes(&base, |config| config.cache_type_v = "q8_0".into());
        assert_fingerprint_changes(&base, |config| config.no_kv_offload = true);
        assert_fingerprint_changes(&base, |config| config.flash_attn = "on".into());
        assert_fingerprint_changes(&base, |config| config.swa_full = true);
        assert_fingerprint_changes(&base, |config| config.context_shift = true);
        assert_fingerprint_changes(&base, |config| config.rope_scaling = "yarn".into());
        assert_fingerprint_changes(&base, |config| config.rope_scale = 2.0);
        assert_fingerprint_changes(&base, |config| config.rope_freq_base = 10_000.0);
        assert_fingerprint_changes(&base, |config| config.rope_freq_scale = 0.5);
        assert_fingerprint_changes(&base, |config| config.yarn_ext_factor = 1.0);
        assert_fingerprint_changes(&base, |config| config.yarn_attn_factor = 1.0);
        assert_fingerprint_changes(&base, |config| config.yarn_beta_slow = 1.0);
        assert_fingerprint_changes(&base, |config| config.yarn_beta_fast = 1.0);
        assert_fingerprint_changes(&base, |config| config.yarn_orig_ctx = 32_768);
        assert_fingerprint_changes(&base, |config| config.batch_size = 1024);
        assert_fingerprint_changes(&base, |config| config.ubatch_size = 256);
        assert_fingerprint_changes(&base, |config| config.device = "HIP0".into());
        assert_fingerprint_changes(&base, |config| config.gpu_layers_auto = false);
        assert_fingerprint_changes(&base, |config| config.gpu_layers = 48);
        assert_fingerprint_changes(&base, |config| config.split_mode = "row".into());
        assert_fingerprint_changes(&base, |config| config.tensor_split = "0.5,0.5".into());
        assert_fingerprint_changes(&base, |config| config.main_gpu = 1);
        assert_fingerprint_changes(&base, |config| config.moe_cpu_layers = 4);
        assert_fingerprint_changes(&base, |config| config.cpu_moe = true);
        assert_fingerprint_changes(&base, |config| config.override_kv = "key=value".into());
        assert_fingerprint_changes(&base, |config| config.jinja = false);
        assert_fingerprint_changes(&base, |config| config.chat_template = "chatml".into());
        assert_fingerprint_changes(&base, |config| config.skip_chat_parsing = true);
        assert_fingerprint_changes(&base, |config| config.reasoning_format = "deepseek".into());
        assert_fingerprint_changes(&base, |config| config.reasoning_effort = "high".into());
        assert_fingerprint_changes(&base, |config| config.reasoning = "on".into());
        assert_fingerprint_changes(&base, |config| config.spec_type = "ngram-mod".into());
        assert_fingerprint_changes(&base, |config| config.cache_type_draft_k = "q8_0".into());
        assert_fingerprint_changes(&base, |config| config.cache_type_draft_v = "q8_0".into());
        assert_fingerprint_changes(&base, |config| config.draft_gpu_layers = 48);
        assert_fingerprint_changes(&base, |config| config.draft_tokens = 15);
        assert_fingerprint_changes(&base, |config| config.spec_draft_n_min = 1);
        assert_fingerprint_changes(&base, |config| config.spec_draft_p_min = 0.2);
        assert_fingerprint_changes(&base, |config| config.spec_draft_p_split = 0.3);
        assert_fingerprint_changes(&base, |config| config.spec_draft_device = "HIP0".into());
        assert_fingerprint_changes(&base, |config| {
            config.spec_draft_backend_sampling = false;
        });
        assert_fingerprint_changes(&base, |config| config.spec_draft_threads = 4);
        assert_fingerprint_changes(&base, |config| config.spec_draft_threads_batch = 8);
        assert_fingerprint_changes(&base, |config| config.reasoning_preserve = "true".into());
        assert_fingerprint_changes(&base, |config| config.reasoning_budget = "8192".into());
        assert_fingerprint_changes(&base, |config| {
            config.reasoning_budget_message = "budget".into();
        });

        let expected = test_fingerprint(&base);
        for mutate in [
            |materials: &mut FingerprintMaterials| materials.model_sha256 = digest(0x33),
            |materials: &mut FingerprintMaterials| materials.engine_sha256 = digest(0x44),
            |materials: &mut FingerprintMaterials| materials.engine_version = "b10680".into(),
            |materials: &mut FingerprintMaterials| materials.backend = "vulkan".into(),
        ] {
            let mut materials = fingerprint_materials();
            mutate(&mut materials);
            assert_ne!(
                expected.digest,
                build_checkpoint_fingerprint(&base, &materials)
                    .unwrap()
                    .digest
            );
        }
    }

    #[test]
    fn fingerprint_uses_template_contents_not_template_path() {
        let config = InstanceConfig {
            chat_template_file: "first-template.jinja".into(),
            ..eligible_config()
        };
        let mut first = fingerprint_materials();
        first.chat_template_file_sha256 = Some(digest(0x55));
        let expected = build_checkpoint_fingerprint(&config, &first).unwrap();

        let relocated = InstanceConfig {
            chat_template_file: "relocated-template.jinja".into(),
            ..config.clone()
        };
        assert_eq!(
            expected.digest,
            build_checkpoint_fingerprint(&relocated, &first)
                .unwrap()
                .digest
        );

        let mut changed = first;
        changed.chat_template_file_sha256 = Some(digest(0x66));
        assert_ne!(
            expected.digest,
            build_checkpoint_fingerprint(&relocated, &changed)
                .unwrap()
                .digest
        );
    }

    #[test]
    fn fingerprint_uses_draft_model_contents_not_draft_path() {
        let config = InstanceConfig {
            draft_model_path: "first-draft.gguf".into(),
            spec_type: "draft-dflash".into(),
            ..eligible_config()
        };
        let mut first = fingerprint_materials();
        first.draft_model_sha256 = Some(digest(0x66));
        let expected = build_checkpoint_fingerprint(&config, &first).unwrap();
        assert_eq!(expected.draft_model_sha256, Some(digest(0x66)));

        let relocated = InstanceConfig {
            draft_model_path: "relocated-draft.gguf".into(),
            ..config.clone()
        };
        assert_eq!(
            expected.digest,
            build_checkpoint_fingerprint(&relocated, &first)
                .unwrap()
                .digest
        );

        let mut changed = first.clone();
        changed.draft_model_sha256 = Some(digest(0x67));
        assert_ne!(
            expected.digest,
            build_checkpoint_fingerprint(&config, &changed)
                .unwrap()
                .digest
        );

        changed.draft_model_sha256 = None;
        assert_eq!(
            build_checkpoint_fingerprint(&config, &changed)
                .unwrap_err()
                .reason_code,
            CheckpointReasonCode::FingerprintUnavailable
        );
    }

    #[test]
    fn fingerprint_excludes_transport_ui_sampling_load_io_and_manager_policy_fields() {
        let base = eligible_config();
        assert_fingerprint_stable(&base, |config| config.model_path = "relocated.gguf".into());
        assert_fingerprint_stable(&base, |config| config.host = "localhost".into());
        assert_fingerprint_stable(&base, |config| config.port = 9999);
        assert_fingerprint_stable(&base, |config| config.api_key = "secret".into());
        assert_fingerprint_stable(&base, |config| config.api_key_file = "secret.txt".into());
        assert_fingerprint_stable(&base, |config| config.ssl_key_file = "key.pem".into());
        assert_fingerprint_stable(&base, |config| config.ssl_cert_file = "cert.pem".into());
        assert_fingerprint_stable(&base, |config| config.cors_origins = "*".into());
        assert_fingerprint_stable(&base, |config| config.cors_methods = "POST".into());
        assert_fingerprint_stable(&base, |config| config.cors_headers = "x-test".into());
        assert_fingerprint_stable(&base, |config| config.cors_credentials = "true".into());
        assert_fingerprint_stable(&base, |config| config.log_prompts_dir = "logs".into());
        assert_fingerprint_stable(&base, |config| config.metrics = false);
        assert_fingerprint_stable(&base, |config| config.props = false);
        assert_fingerprint_stable(&base, |config| config.no_ui = true);
        assert_fingerprint_stable(&base, |config| config.lazy_mode = "on".into());
        assert_fingerprint_stable(&base, |config| config.name = "renamed".into());
        assert_fingerprint_stable(&base, |config| config.alias = "alias".into());
        assert_fingerprint_stable(&base, |config| config.tags = "tag".into());
        assert_fingerprint_stable(&base, |config| config.auto_start = true);
        assert_fingerprint_stable(&base, |config| config.temp = 0.1);
        assert_fingerprint_stable(&base, |config| config.top_k = 10);
        assert_fingerprint_stable(&base, |config| config.top_p = 0.5);
        assert_fingerprint_stable(&base, |config| config.seed = 7);
        assert_fingerprint_stable(&base, |config| config.n_predict = 32);
        assert_fingerprint_stable(&base, |config| config.kv_checkpoint.auto_save = false);
        assert_fingerprint_stable(&base, |config| config.kv_checkpoint.auto_restore = false);
        assert_fingerprint_stable(&base, |config| config.kv_checkpoint.storage_limit_gib = 16);
        assert_fingerprint_stable(&base, |config| {
            config.kv_checkpoint.minimum_prompt_tokens = 512;
        });
    }

    #[test]
    fn content_hash_cache_tracks_full_file_identity_and_invalidates_on_mutation() {
        let sandbox = TestSandbox::new("hash-cache");
        let store = sandbox.store();
        let model = sandbox.path.join("model.gguf");
        let engine = sandbox.path.join("llama-server");
        fs::write(&model, b"model version one").unwrap();
        fs::write(&engine, b"engine version one").unwrap();

        let model_first = store.content_sha256(&model).unwrap();
        let engine_first = store.content_sha256(&engine).unwrap();
        assert_eq!(model_first, sha256_bytes(b"model version one"));
        assert_eq!(engine_first, sha256_bytes(b"engine version one"));
        assert_eq!(store.content_sha256(&model).unwrap(), model_first);

        fs::write(&model, b"model version two with a different size").unwrap();
        fs::write(&engine, b"engine version two with a different size").unwrap();
        let model_second = store.content_sha256(&model).unwrap();
        let engine_second = store.content_sha256(&engine).unwrap();
        assert_ne!(model_first, model_second);
        assert_ne!(engine_first, engine_second);

        let previous_metadata = fs::metadata(&model).unwrap();
        let previous_modified = previous_metadata.modified().unwrap();
        let replacement = vec![b'x'; previous_metadata.len() as usize];
        fs::remove_file(&model).unwrap();
        fs::write(&model, &replacement).unwrap();
        File::options()
            .write(true)
            .open(&model)
            .unwrap()
            .set_modified(previous_modified)
            .unwrap();
        let replacement_hash = store.content_sha256(&model).unwrap();
        assert_eq!(replacement_hash, sha256_bytes(&replacement));
        assert_ne!(replacement_hash, model_second);

        let previous_modified = fs::metadata(&model).unwrap().modified().unwrap();
        let in_place = vec![b'y'; replacement.len()];
        let mut open = File::options()
            .write(true)
            .truncate(true)
            .open(&model)
            .unwrap();
        open.write_all(&in_place).unwrap();
        open.sync_all().unwrap();
        open.set_modified(previous_modified).unwrap();
        drop(open);
        let in_place_hash = store.content_sha256(&model).unwrap();
        assert_eq!(in_place_hash, sha256_bytes(&in_place));
        assert_ne!(in_place_hash, replacement_hash);

        let cache = fs::read(store.root().join("fingerprints-v1.json")).unwrap();
        let parsed: HashCacheFile = serde_json::from_slice(&cache).unwrap();
        assert_eq!(parsed.schema_version, HASH_CACHE_SCHEMA_VERSION);
        assert_eq!(parsed.entries.len(), 2);
        assert!(parsed.entries.keys().all(|key| is_lower_hex_digest(key)));
        assert!(!String::from_utf8(cache).unwrap().contains("model.gguf"));
    }

    #[test]
    fn model_artifact_hash_covers_every_shard_and_preserves_single_file_behavior() {
        let sandbox = TestSandbox::new("artifact-hash");
        let store = sandbox.store();
        let single = sandbox.path.join("single.gguf");
        fs::write(&single, b"single model").unwrap();
        assert_eq!(
            store.model_artifact_sha256(&single).unwrap(),
            store.content_sha256(&single).unwrap()
        );

        let first = sandbox.path.join("Qwen-00001-of-00003.gguf");
        let second = sandbox.path.join("Qwen-00002-of-00003.gguf");
        let third = sandbox.path.join("Qwen-00003-of-00003.gguf");
        fs::write(&first, b"shard one").unwrap();
        fs::write(&second, b"shard two").unwrap();
        fs::write(&third, b"shard three").unwrap();
        let initial = store.model_artifact_sha256(&first).unwrap();
        assert_eq!(store.model_artifact_sha256(&second).unwrap(), initial);

        fs::write(&third, b"shard three changed").unwrap();
        assert_ne!(store.model_artifact_sha256(&first).unwrap(), initial);
        fs::remove_file(&second).unwrap();
        let error = store.model_artifact_sha256(&first).unwrap_err();
        assert_eq!(
            error.reason_code,
            CheckpointReasonCode::ModelArtifactsIncomplete
        );
    }

    #[test]
    fn engine_artifact_hash_covers_adjacent_runtime_libraries_but_not_documents() {
        let sandbox = TestSandbox::new("engine-artifact-hash");
        let store = sandbox.store();
        let engine = sandbox.path.join("llama-server.exe");
        let implementation = sandbox.path.join("llama-server-impl.dll");
        let notes = sandbox.path.join("release-notes.txt");
        fs::write(&engine, b"launcher").unwrap();
        fs::write(&implementation, b"implementation one").unwrap();
        fs::write(&notes, b"notes one").unwrap();

        let initial = store.engine_artifact_sha256(&engine).unwrap();
        assert_ne!(initial, store.content_sha256(&engine).unwrap());

        fs::write(&notes, b"notes two").unwrap();
        assert_eq!(store.engine_artifact_sha256(&engine).unwrap(), initial);

        fs::write(&implementation, b"implementation two").unwrap();
        assert_ne!(store.engine_artifact_sha256(&engine).unwrap(), initial);
    }

    #[test]
    fn fingerprint_normalizes_equivalent_speculative_type_order() {
        let mut first = eligible_config();
        first.spec_type = "ngram-cache,ngram-mod".into();
        let mut second = first.clone();
        second.spec_type = "ngram-mod,ngram-cache".into();
        assert_eq!(
            test_fingerprint(&first).digest,
            test_fingerprint(&second).digest
        );
    }

    #[test]
    fn manifest_parser_rejects_future_ambiguous_or_unsafe_state() {
        let fingerprint = test_fingerprint(&eligible_config());
        let manifest =
            CheckpointManifestV1::new("instance-1", fingerprint.clone(), 256, 1024, digest(0x77));
        let valid = serde_json::to_vec(&manifest).unwrap();
        CheckpointManifestV1::parse_and_validate(&valid, "instance-1", &fingerprint.digest)
            .unwrap();

        let mut cases = Vec::new();
        let mut future = serde_json::to_value(&manifest).unwrap();
        future["schemaVersion"] = 2.into();
        cases.push(future);

        let mut duplicate = serde_json::to_value(&manifest).unwrap();
        let duplicate_slot = duplicate["slots"][0].clone();
        duplicate["slots"]
            .as_array_mut()
            .unwrap()
            .push(duplicate_slot);
        cases.push(duplicate);

        let mut traversal = serde_json::to_value(&manifest).unwrap();
        traversal["slots"][0]["filename"] = "../slot-0.bin".into();
        cases.push(traversal);

        let mut bad_digest = serde_json::to_value(&manifest).unwrap();
        bad_digest["slots"][0]["sha256"] = "ABC".into();
        cases.push(bad_digest);

        let mut zero_bytes = serde_json::to_value(&manifest).unwrap();
        zero_bytes["slots"][0]["bytes"] = 0.into();
        cases.push(zero_bytes);

        let mut zero_tokens = serde_json::to_value(&manifest).unwrap();
        zero_tokens["slots"][0]["promptTokens"] = 0.into();
        cases.push(zero_tokens);

        let mut negative_slot = serde_json::to_value(&manifest).unwrap();
        negative_slot["slots"][0]["id"] = (-1).into();
        cases.push(negative_slot);

        let mut unknown = serde_json::to_value(&manifest).unwrap();
        unknown["unexpected"] = true.into();
        cases.push(unknown);

        for value in cases {
            let bytes = serde_json::to_vec(&value).unwrap();
            let error =
                CheckpointManifestV1::parse_and_validate(&bytes, "instance-1", &fingerprint.digest)
                    .unwrap_err();
            assert_eq!(error.reason_code, CheckpointReasonCode::ManifestInvalid);
        }

        assert!(CheckpointManifestV1::parse_and_validate(
            &valid,
            "different-instance",
            &fingerprint.digest,
        )
        .is_err());
        assert!(
            CheckpointManifestV1::parse_and_validate(&valid, "instance-1", &digest(0x99),).is_err()
        );
    }

    #[test]
    fn private_store_rejects_unsafe_identifiers_and_protects_paths() {
        let sandbox = TestSandbox::new("private-paths");
        let store = sandbox.store();
        assert!(store.prepare_instance("../escape").is_err());
        assert!(store.prepare_instance("absolute/path").is_err());
        assert!(store.load_latest("instance-1", "not-a-digest").is_err());

        let scratch = store.prepare_instance("instance-1").unwrap();
        validate_directory(store.root()).unwrap();
        validate_directory(&scratch).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(store.root()).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&scratch).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        #[cfg(windows)]
        {
            let status = std::process::Command::new("icacls.exe")
                .arg(store.root())
                .arg("/verify")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success());
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_acl_hardening_is_idempotent_across_checkpoint_stores() {
        let sandbox = TestSandbox::new("windows-acl-idempotent");
        let model = sandbox.path.join("model.gguf");
        fs::write(&model, b"first model contents").unwrap();

        // The application process computes the fingerprint cache first.
        let application_store = sandbox.store();
        let first_digest = application_store.content_sha256(&model).unwrap();
        let cache_path = application_store.root().join("fingerprints-v1.json");
        assert!(!fs::read(&cache_path).unwrap().is_empty());

        // The runtime supervisor owns a separate CheckpointStore for the same
        // root. Its first access reapplies Windows ACL protection recursively.
        let runtime_store = sandbox.store();
        let scratch = runtime_store.prepare_instance("runtime-instance").unwrap();

        // Existing regular files must remain readable and writable after that
        // second protection pass, and newly created descendants must inherit
        // an effective current-user ACE.
        assert!(!fs::read(&cache_path).unwrap().is_empty());
        assert_eq!(runtime_store.content_sha256(&model).unwrap(), first_digest);
        fs::write(&model, b"second model contents with a new size").unwrap();
        let second_digest = runtime_store.content_sha256(&model).unwrap();
        assert_ne!(second_digest, first_digest);
        assert!(!fs::read(&cache_path).unwrap().is_empty());

        let descendant = scratch.join("created-after-protection.json");
        fs::write(&descendant, b"readable").unwrap();
        assert_eq!(fs::read(&descendant).unwrap(), b"readable");

        // A later process restart may initialize yet another store. Repeating
        // hardening must not regress either existing or newly created files.
        let restarted_store = sandbox.store();
        restarted_store
            .prepare_instance("restarted-instance")
            .unwrap();
        assert!(!fs::read(&cache_path).unwrap().is_empty());
        assert_eq!(fs::read(&descendant).unwrap(), b"readable");
    }

    #[test]
    fn clear_removes_only_the_exact_instance_root_and_is_idempotent() {
        let sandbox = TestSandbox::new("clear-instance");
        let store = sandbox.store();
        let first = store.prepare_instance("instance-1").unwrap();
        let second = store.prepare_instance("instance-2").unwrap();
        fs::write(first.join("local-marker"), b"one").unwrap();
        fs::write(second.join("local-marker"), b"two").unwrap();

        assert!(store.clear_instance("instance-1").unwrap());
        assert!(!store.root().join("instance-1").exists());
        assert!(store.root().join("instance-2").is_dir());
        assert!(!store.clear_instance("instance-1").unwrap());
        assert!(store.clear_instance("../escape").is_err());
    }

    #[test]
    fn scratch_payload_symlink_or_reparse_point_is_rejected_when_supported() {
        let sandbox = TestSandbox::new("symlink");
        let store = sandbox.store();
        let scratch = store.prepare_instance("instance-1").unwrap();
        let target = sandbox.path.join("outside.bin");
        fs::write(&target, b"outside").unwrap();
        let link = scratch.join(format!("slot-0-1-{}.bin", uuid::Uuid::new_v4()));

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(validate_regular_file(&link).is_err());
        }
        #[cfg(windows)]
        {
            if std::os::windows::fs::symlink_file(&target, &link).is_ok() {
                assert!(validate_regular_file(&link).is_err());
            }
        }
    }

    #[test]
    fn launch_cleanup_removes_only_manager_generated_scratch_files() {
        let sandbox = TestSandbox::new("scratch-cleanup");
        let store = sandbox.store();
        let managed = write_scratch_payload(&store, "instance-1", 99, b"stale slot");
        let scratch = store.prepare_instance("instance-1").unwrap();
        let unrelated = scratch.join("operator-note.txt");
        fs::write(&unrelated, b"keep").unwrap();

        assert_eq!(store.cleanup_scratch("instance-1").unwrap(), 1);
        assert!(!managed.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn generation_commit_is_manifest_last_and_detects_payload_corruption() {
        let sandbox = TestSandbox::new("commit-load");
        let store = sandbox.store();
        let config = eligible_config();
        let fingerprint = test_fingerprint(&config);
        let payload = write_scratch_payload(&store, "instance-1", 10, b"checkpoint payload");
        let manifest = manifest_for_payload("instance-1", &fingerprint, &payload, 512);
        let committed = store
            .commit_generation(&manifest, &payload, 1024 * 1024)
            .unwrap();
        assert_eq!(committed.generation_id, manifest.generation_id);
        assert!(
            !payload.exists(),
            "generation commit must move, not copy, scratch payloads"
        );

        let loaded = store
            .load_latest("instance-1", &fingerprint.digest)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.manifest, manifest);
        assert_eq!(
            fs::read(loaded.payload_path()).unwrap(),
            b"checkpoint payload"
        );
        assert!(loaded.generation_dir().join("manifest.json").is_file());

        fs::write(loaded.payload_path(), b"truncated").unwrap();
        let error = store
            .load_latest("instance-1", &fingerprint.digest)
            .unwrap_err();
        assert_eq!(error.reason_code, CheckpointReasonCode::ChecksumMismatch);

        fs::remove_file(loaded.payload_path()).unwrap();
        fs::create_dir(loaded.payload_path()).unwrap();
        let non_regular = store
            .load_latest("instance-1", &fingerprint.digest)
            .unwrap_err();
        assert_eq!(
            non_regular.reason_code,
            CheckpointReasonCode::ManifestInvalid
        );
    }

    #[test]
    fn latest_pointer_corruption_falls_back_to_newest_complete_generation() {
        let sandbox = TestSandbox::new("latest-fallback");
        let store = sandbox.store();
        let fingerprint = test_fingerprint(&eligible_config());

        let first_payload = write_scratch_payload(&store, "instance-1", 11, b"first payload");
        let first = manifest_for_payload("instance-1", &fingerprint, &first_payload, 256);
        store
            .commit_generation(&first, &first_payload, 1024 * 1024)
            .unwrap();

        let second_payload = write_scratch_payload(&store, "instance-1", 12, b"second payload");
        let mut second = manifest_for_payload("instance-1", &fingerprint, &second_payload, 512);
        second.created_at = (Utc::now() + chrono::Duration::seconds(1)).to_rfc3339();
        store
            .commit_generation(&second, &second_payload, 1024 * 1024)
            .unwrap();

        let latest_path = store
            .fingerprint_root("instance-1", &fingerprint.digest)
            .unwrap()
            .join("latest.json");
        fs::write(latest_path, b"not json").unwrap();
        let loaded = store
            .load_latest("instance-1", &fingerprint.digest)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.manifest.generation_id, second.generation_id);
    }

    #[test]
    fn every_commit_fault_preserves_the_previous_latest_generation() {
        let sandbox = TestSandbox::new("faults");
        let store = sandbox.store();
        let fingerprint = test_fingerprint(&eligible_config());
        let stable_payload = write_scratch_payload(&store, "instance-1", 20, b"stable payload");
        let stable = manifest_for_payload("instance-1", &fingerprint, &stable_payload, 256);
        store
            .commit_generation(&stable, &stable_payload, 1024 * 1024)
            .unwrap();

        let points = [
            StoreFaultPoint::AfterPayloadMove,
            StoreFaultPoint::AfterPayloadSync,
            StoreFaultPoint::AfterManifestWrite,
            StoreFaultPoint::BeforeGenerationRename,
            StoreFaultPoint::BeforeLatestUpdate,
        ];
        for (index, point) in points.into_iter().enumerate() {
            let contents = format!("replacement payload {index}");
            let payload =
                write_scratch_payload(&store, "instance-1", 21 + index as u32, contents.as_bytes());
            let manifest =
                manifest_for_payload("instance-1", &fingerprint, &payload, 512 + index as u64);
            let error = store
                .commit_generation_with_fault(&manifest, &payload, 1024 * 1024, |candidate| {
                    if candidate == point {
                        Err(CheckpointStoreError::io("injected storage fault"))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert_eq!(error.reason_code, CheckpointReasonCode::IoError);
            let loaded = store
                .load_latest("instance-1", &fingerprint.digest)
                .unwrap()
                .unwrap();
            assert_eq!(loaded.manifest.generation_id, stable.generation_id);
        }
    }

    #[test]
    fn pending_cleanup_and_storage_limit_never_replace_good_state() {
        let sandbox = TestSandbox::new("pending-limit");
        let store = sandbox.store();
        let fingerprint = test_fingerprint(&eligible_config());
        let payload = write_scratch_payload(&store, "instance-1", 30, b"good payload");
        let stable = manifest_for_payload("instance-1", &fingerprint, &payload, 256);
        store
            .commit_generation(&stable, &payload, 1024 * 1024)
            .unwrap();

        let (_, generations_root) = store
            .ensure_generation_roots("instance-1", &fingerprint.digest)
            .unwrap();
        let pending_id = uuid::Uuid::new_v4().to_string();
        ensure_private_directory(&generations_root.join(format!(".pending-{pending_id}"))).unwrap();
        assert_eq!(
            store
                .cleanup_pending("instance-1", &fingerprint.digest)
                .unwrap(),
            1
        );

        let oversized_payload =
            write_scratch_payload(&store, "instance-1", 31, &vec![0x55; 16 * 1024]);
        let oversized = manifest_for_payload("instance-1", &fingerprint, &oversized_payload, 1024);
        let error = store
            .commit_generation(&oversized, &oversized_payload, 4096)
            .unwrap_err();
        assert_eq!(error.reason_code, CheckpointReasonCode::StorageLimit);
        let loaded = store
            .load_latest("instance-1", &fingerprint.digest)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.manifest.generation_id, stable.generation_id);
    }

    #[test]
    fn scratch_capacity_preflight_reports_insufficient_disk_space() {
        let sandbox = TestSandbox::new("scratch-capacity");
        let store = sandbox.store();
        let error = store
            .ensure_scratch_capacity("instance-1", u64::MAX)
            .unwrap_err();
        assert_eq!(
            error.reason_code,
            CheckpointReasonCode::InsufficientDiskSpace
        );
    }

    #[test]
    fn per_instance_lru_prunes_old_generations_but_protects_latest() {
        let sandbox = TestSandbox::new("lru");
        let store = sandbox.store();
        let fingerprint = test_fingerprint(&eligible_config());
        let mut latest_id = String::new();
        for index in 0..3_u32 {
            let payload = write_scratch_payload(
                &store,
                "instance-1",
                40 + index,
                &vec![index as u8 + 1; 2048],
            );
            let mut manifest =
                manifest_for_payload("instance-1", &fingerprint, &payload, 256 + index as u64);
            manifest.created_at =
                (Utc::now() + chrono::Duration::seconds(index.into())).to_rfc3339();
            latest_id.clone_from(&manifest.generation_id);
            store.commit_generation(&manifest, &payload, 4096).unwrap();
        }

        let latest_path = store
            .fingerprint_root("instance-1", &fingerprint.digest)
            .unwrap()
            .join("latest.json");
        fs::write(latest_path, b"corrupt latest pointer").unwrap();

        let result = store.prune_instance("instance-1", 4096, &[]).unwrap();
        assert_eq!(result.removed_generations, 2);
        assert!(result.remaining_bytes <= 4096);
        let loaded = store
            .load_latest("instance-1", &fingerprint.digest)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.manifest.generation_id, latest_id);
    }

    #[test]
    fn per_instance_limit_evicts_historical_fingerprint_generations() {
        let sandbox = TestSandbox::new("cross-fingerprint-lru");
        let store = sandbox.store();
        let first_fingerprint = test_fingerprint(&eligible_config());
        let mut second_config = eligible_config();
        second_config.cache_type_k = "q8_0".into();
        let second_fingerprint = test_fingerprint(&second_config);

        let first_payload = write_scratch_payload(&store, "instance-1", 50, &vec![1; 2048]);
        let first = manifest_for_payload("instance-1", &first_fingerprint, &first_payload, 512);
        store
            .commit_generation(&first, &first_payload, 4096)
            .unwrap();

        let second_payload = write_scratch_payload(&store, "instance-1", 51, &vec![2; 2048]);
        let second = manifest_for_payload("instance-1", &second_fingerprint, &second_payload, 768);
        store
            .commit_generation(&second, &second_payload, 4096)
            .unwrap();

        let result = store
            .prune_instance(
                "instance-1",
                4096,
                &[(
                    second_fingerprint.digest.clone(),
                    second.generation_id.clone(),
                )],
            )
            .unwrap();

        assert_eq!(result.removed_generations, 1);
        assert!(result.remaining_bytes <= 4096);
        assert!(store
            .load_latest("instance-1", &first_fingerprint.digest)
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .load_latest("instance-1", &second_fingerprint.digest)
                .unwrap()
                .unwrap()
                .manifest
                .generation_id,
            second.generation_id
        );
    }

    #[test]
    fn pruning_removes_unloadable_final_generation_directories() {
        let sandbox = TestSandbox::new("corrupt-generation-gc");
        let store = sandbox.store();
        let fingerprint = test_fingerprint(&eligible_config());
        let (_, generations_root) = store
            .ensure_generation_roots("instance-1", &fingerprint.digest)
            .unwrap();
        let corrupt = generations_root.join(uuid::Uuid::new_v4().to_string());
        ensure_private_directory(&corrupt).unwrap();
        fs::write(corrupt.join("manifest.json"), b"not-json").unwrap();

        let result = store
            .prune_instance("instance-1", 1024 * 1024, &[])
            .unwrap();

        assert_eq!(result.removed_generations, 1);
        assert!(!corrupt.exists());
    }

    #[test]
    fn fingerprint_cache_has_ttl_and_lru_bounds() {
        let now = 10_000_000;
        let mut cache = HashCacheFile::default();
        cache.entries.insert(
            "stale".into(),
            HashCacheEntry {
                size: 1,
                modified_unix_nanos: 1,
                file_identity: "stale".into(),
                sha256: digest(1),
                last_used_unix_secs: now - HASH_CACHE_TTL_SECS - 1,
            },
        );
        for index in 0..=MAX_HASH_CACHE_ENTRIES {
            cache.entries.insert(
                format!("entry-{index:03}"),
                HashCacheEntry {
                    size: 1,
                    modified_unix_nanos: 1,
                    file_identity: format!("entry-{index:03}"),
                    sha256: digest(2),
                    last_used_unix_secs: now - (MAX_HASH_CACHE_ENTRIES - index) as u64,
                },
            );
        }

        maintain_hash_cache(&mut cache, now);

        assert_eq!(cache.entries.len(), MAX_HASH_CACHE_ENTRIES);
        assert!(!cache.entries.contains_key("stale"));
        assert!(!cache.entries.contains_key("entry-000"));
        assert!(cache
            .entries
            .contains_key(&format!("entry-{MAX_HASH_CACHE_ENTRIES:03}")));
    }

    #[test]
    fn coordinator_rejects_illegal_transitions_and_stale_pid_events() {
        let sandbox = TestSandbox::new("coordinator-transitions");
        let store = sandbox.store();
        let coordinator = CheckpointCoordinator::new(store);
        let config = eligible_config();
        let eligibility = evaluate(&config);
        let fingerprint = test_fingerprint(&config);
        let missing = coordinator
            .register_start_with_context(
                "missing-fingerprint",
                100,
                &eligibility,
                None,
                config.kv_checkpoint.clone(),
            )
            .unwrap_err();
        assert_eq!(
            missing.reason_code,
            CheckpointReasonCode::FingerprintUnavailable
        );

        let starting = coordinator
            .register_start_with_context(
                "instance-1",
                101,
                &eligibility,
                Some(fingerprint.clone()),
                config.kv_checkpoint.clone(),
            )
            .unwrap();
        assert_eq!(starting.phase, CheckpointPhase::Starting);
        assert!(!coordinator.gate_allows_routing("instance-1"));
        let illegal = coordinator
            .transition("instance-1", 101, CheckpointPhase::Ready)
            .unwrap_err();
        assert_eq!(
            illegal.reason_code,
            CheckpointReasonCode::InvalidStateTransition
        );
        assert_eq!(
            coordinator.status("instance-1").unwrap().phase,
            CheckpointPhase::Starting
        );

        coordinator.on_engine_healthy("instance-1", 101).unwrap();
        coordinator
            .register_start_with_context(
                "instance-1",
                202,
                &eligibility,
                Some(fingerprint),
                config.kv_checkpoint,
            )
            .unwrap();
        let stale = coordinator
            .on_engine_healthy("instance-1", 101)
            .unwrap_err();
        assert_eq!(stale.reason_code, CheckpointReasonCode::StaleProcessEvent);
        let current = coordinator.status("instance-1").unwrap();
        assert_eq!(current.expected_pid, Some(202));
        assert_eq!(current.phase, CheckpointPhase::Starting);
    }

    #[test]
    fn coordinator_rejects_running_clear_and_reports_exact_clear_outcome() {
        let sandbox = TestSandbox::new("coordinator-clear");
        let store = sandbox.store();
        store.prepare_instance("instance-1").unwrap();
        let coordinator = CheckpointCoordinator::new(store.clone());
        let config = eligible_config();
        let eligibility = evaluate(&config);
        coordinator
            .register_start_with_context(
                "instance-1",
                303,
                &eligibility,
                Some(test_fingerprint(&config)),
                config.kv_checkpoint,
            )
            .unwrap();

        let running = coordinator.clear_instance("instance-1").unwrap_err();
        assert_eq!(running.reason_code, CheckpointReasonCode::ClearWhileRunning);
        assert!(store.root().join("instance-1").is_dir());

        coordinator.mark_unexpected_exit("instance-1", 303).unwrap();
        let cleared = coordinator.clear_instance("instance-1").unwrap();
        assert_eq!(cleared.phase, CheckpointPhase::Stopped);
        assert_eq!(cleared.last_operation, CheckpointOperation::Clear);
        assert_eq!(cleared.last_outcome, CheckpointOutcome::Success);
        assert_eq!(cleared.reason_code, CheckpointReasonCode::None);
        assert!(!store.root().join("instance-1").exists());

        let repeated = coordinator.clear_instance("instance-1").unwrap();
        assert_eq!(repeated.last_outcome, CheckpointOutcome::Skipped);
        assert_eq!(repeated.reason_code, CheckpointReasonCode::NoCheckpoint);
    }

    #[test]
    fn restore_is_routable_only_after_round_trip_verification() {
        let sandbox = TestSandbox::new("coordinator-restore-success");
        let store = sandbox.store();
        let fingerprint = test_fingerprint(&eligible_config());
        let payload = b"deterministic serialized slot state";
        let manifest = commit_test_generation(&store, "instance-1", &fingerprint, 768, payload);
        let backend = FakeSlotBackend::new(&store, "instance-1");
        backend.configure(|state| state.restore_tokens = 768);
        let coordinator = coordinator_with_healthy_engine(&store, "instance-1", 301);

        let status = coordinator
            .restore_or_cold("instance-1", 301, &fingerprint, true, &backend)
            .unwrap();
        assert_eq!(status.phase, CheckpointPhase::Ready);
        assert!(status.routable);
        assert!(coordinator.gate_allows_routing("instance-1"));
        assert_eq!(status.last_operation, CheckpointOperation::Restore);
        assert_eq!(status.last_outcome, CheckpointOutcome::Success);
        assert_eq!(status.generation_id, Some(manifest.generation_id));
        assert_eq!(status.prompt_tokens, Some(768));
        assert_eq!(status.bytes, Some(payload.len() as u64));
        assert_eq!(backend.erase_count(), 0);

        let events = backend.events();
        assert_eq!(events[0], "health");
        assert!(events[1].starts_with("restore:slot-0-301-"));
        assert!(events[2].starts_with("save:slot-0-301-"));
        assert_eq!(events[3], "slots");
        let scratch = store.prepare_instance("instance-1").unwrap();
        assert_eq!(fs::read_dir(scratch).unwrap().count(), 0);

        let serialized = serde_json::to_string(&status).unwrap();
        assert!(!serialized.contains(&sandbox.path.to_string_lossy().to_string()));
        assert!(!serialized.contains("serialized slot state"));
    }

    #[test]
    fn restore_noop_or_invalid_response_erases_slot_and_fails_open() {
        let sandbox = TestSandbox::new("coordinator-restore-noop");
        let store = sandbox.store();
        let fingerprint = test_fingerprint(&eligible_config());
        commit_test_generation(
            &store,
            "instance-1",
            &fingerprint,
            512,
            b"expected durable slot bytes",
        );

        let backend = FakeSlotBackend::new(&store, "instance-1");
        backend.configure(|state| {
            state.restore_tokens = 512;
            state.restore_applies = false;
            state.payload = b"cold slot".to_vec();
        });
        let coordinator = coordinator_with_healthy_engine(&store, "instance-1", 401);
        let status = coordinator
            .restore_or_cold("instance-1", 401, &fingerprint, true, &backend)
            .unwrap();
        assert_eq!(status.phase, CheckpointPhase::ReadyCold);
        assert!(status.routable);
        assert_eq!(status.last_outcome, CheckpointOutcome::Failed);
        assert_eq!(status.reason_code, CheckpointReasonCode::SlotStateMismatch);
        assert_eq!(backend.erase_count(), 1);
        assert_eq!(backend.events().last().unwrap(), "erase");

        let backend = FakeSlotBackend::new(&store, "instance-2");
        commit_test_generation(
            &store,
            "instance-2",
            &fingerprint,
            512,
            b"another durable slot",
        );
        backend.configure(|state| {
            state.restore_tokens = 512;
            state.restore_claim_tokens = Some(511);
        });
        let coordinator = coordinator_with_healthy_engine(&store, "instance-2", 402);
        let status = coordinator
            .restore_or_cold("instance-2", 402, &fingerprint, true, &backend)
            .unwrap();
        assert_eq!(status.phase, CheckpointPhase::ReadyCold);
        assert_eq!(
            status.reason_code,
            CheckpointReasonCode::RestoreResponseInvalid
        );
        assert_eq!(backend.erase_count(), 1);

        let backend = FakeSlotBackend::new(&store, "instance-3");
        commit_test_generation(
            &store,
            "instance-3",
            &fingerprint,
            512,
            b"cleanup retry durable slot",
        );
        backend.configure(|state| {
            state.restore_error = Some(CheckpointStoreError::new(
                CheckpointReasonCode::HttpTimeout,
                "injected restore timeout",
            ));
            state.erase_error = Some(CheckpointStoreError::new(
                CheckpointReasonCode::SlotApiError,
                "injected erase failure",
            ));
        });
        let coordinator = coordinator_with_healthy_engine(&store, "instance-3", 403);
        let blocked = coordinator
            .restore_or_cold("instance-3", 403, &fingerprint, true, &backend)
            .unwrap();
        assert_eq!(blocked.phase, CheckpointPhase::Restoring);
        assert!(!blocked.routable);
        assert!(!coordinator.gate_allows_routing("instance-3"));
        assert!(coordinator.restore_cleanup_pending("instance-3", 403));
        assert_eq!(blocked.reason_code, CheckpointReasonCode::HttpTimeout);
        assert_eq!(backend.erase_count(), 1);

        backend.configure(|state| state.erase_error = None);
        let cold = coordinator
            .retry_failed_restore_cleanup("instance-3", 403, &backend)
            .unwrap();
        assert_eq!(cold.phase, CheckpointPhase::ReadyCold);
        assert!(cold.routable);
        assert_eq!(cold.reason_code, CheckpointReasonCode::HttpTimeout);
        assert_eq!(backend.erase_count(), 2);
    }

    #[test]
    fn restore_timeout_fails_open_while_no_checkpoint_and_mismatch_skip_io() {
        let sandbox = TestSandbox::new("coordinator-restore-fallbacks");
        let store = sandbox.store();
        let fingerprint = test_fingerprint(&eligible_config());

        let no_checkpoint_backend = FakeSlotBackend::new(&store, "empty-instance");
        let coordinator = coordinator_with_healthy_engine(&store, "empty-instance", 501);
        let status = coordinator
            .restore_or_cold(
                "empty-instance",
                501,
                &fingerprint,
                true,
                &no_checkpoint_backend,
            )
            .unwrap();
        assert_eq!(status.phase, CheckpointPhase::ReadyCold);
        assert_eq!(status.last_outcome, CheckpointOutcome::Skipped);
        assert_eq!(status.reason_code, CheckpointReasonCode::NoCheckpoint);
        assert!(no_checkpoint_backend.events().is_empty());

        commit_test_generation(
            &store,
            "mismatch-instance",
            &fingerprint,
            512,
            b"old fingerprint payload",
        );
        let mut changed = eligible_config();
        changed.ctx_size = 8192;
        let changed_fingerprint = test_fingerprint(&changed);
        let mismatch_backend = FakeSlotBackend::new(&store, "mismatch-instance");
        let coordinator = coordinator_with_healthy_engine(&store, "mismatch-instance", 502);
        let status = coordinator
            .restore_or_cold(
                "mismatch-instance",
                502,
                &changed_fingerprint,
                true,
                &mismatch_backend,
            )
            .unwrap();
        assert_eq!(
            status.reason_code,
            CheckpointReasonCode::FingerprintMismatch
        );
        assert!(mismatch_backend.events().is_empty());

        commit_test_generation(
            &store,
            "timeout-instance",
            &fingerprint,
            512,
            b"timeout payload",
        );
        let timeout_backend = FakeSlotBackend::new(&store, "timeout-instance");
        timeout_backend.configure(|state| {
            state.restore_error = Some(CheckpointStoreError::new(
                CheckpointReasonCode::HttpTimeout,
                "injected restore timeout",
            ));
        });
        let coordinator = coordinator_with_healthy_engine(&store, "timeout-instance", 503);
        let status = coordinator
            .restore_or_cold(
                "timeout-instance",
                503,
                &fingerprint,
                true,
                &timeout_backend,
            )
            .unwrap();
        assert_eq!(status.phase, CheckpointPhase::ReadyCold);
        assert_eq!(status.reason_code, CheckpointReasonCode::HttpTimeout);
        assert_eq!(timeout_backend.erase_count(), 1);
    }

    #[test]
    fn save_commits_generation_then_moves_to_stopping() {
        let sandbox = TestSandbox::new("coordinator-save-success");
        let store = sandbox.store();
        let config = eligible_config();
        let fingerprint = test_fingerprint(&config);
        let backend = FakeSlotBackend::new(&store, "instance-1");
        backend.configure(|state| {
            state.prompt_tokens = 1024;
            state.payload = b"live slot payload ready for persistence".to_vec();
        });
        let coordinator = coordinator_with_healthy_engine(&store, "instance-1", 601);
        coordinator
            .restore_or_cold("instance-1", 601, &fingerprint, true, &backend)
            .unwrap();
        coordinator.begin_draining("instance-1", 601).unwrap();
        let status = coordinator
            .save_before_stop(
                "instance-1",
                601,
                &fingerprint,
                &config.kv_checkpoint,
                &backend,
            )
            .unwrap();

        assert_eq!(status.phase, CheckpointPhase::Stopping);
        assert!(!status.routable);
        assert_eq!(status.last_operation, CheckpointOperation::Save);
        assert_eq!(status.last_outcome, CheckpointOutcome::Success);
        assert_eq!(status.reason_code, CheckpointReasonCode::None);
        assert_eq!(status.prompt_tokens, Some(1024));
        let loaded = store
            .load_latest("instance-1", &fingerprint.digest)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.manifest.generation_id, status.generation_id.unwrap());
        assert_eq!(loaded.manifest.slot().prompt_tokens, 1024);
        assert_eq!(
            fs::read(loaded.payload_path()).unwrap(),
            b"live slot payload ready for persistence"
        );
        let stopped = coordinator.mark_stopped("instance-1", 601).unwrap();
        assert_eq!(stopped.phase, CheckpointPhase::Stopped);
        assert!(!coordinator.gate_active("instance-1"));
    }

    #[test]
    fn save_threshold_busy_timeout_and_backend_error_never_block_stop() {
        let sandbox = TestSandbox::new("coordinator-save-fallbacks");
        let store = sandbox.store();
        let config = eligible_config();
        let fingerprint = test_fingerprint(&config);

        let below = FakeSlotBackend::new(&store, "below");
        below.configure(|state| {
            state.prompt_tokens = u64::from(config.kv_checkpoint.minimum_prompt_tokens - 1);
            state.payload = b"small but valid slot".to_vec();
        });
        let coordinator = coordinator_with_healthy_engine(&store, "below", 701);
        coordinator
            .restore_or_cold("below", 701, &fingerprint, true, &below)
            .unwrap();
        coordinator.begin_draining("below", 701).unwrap();
        let status = coordinator
            .save_before_stop("below", 701, &fingerprint, &config.kv_checkpoint, &below)
            .unwrap();
        assert_eq!(status.phase, CheckpointPhase::Stopping);
        assert_eq!(status.last_outcome, CheckpointOutcome::Skipped);
        assert_eq!(
            status.reason_code,
            CheckpointReasonCode::BelowTokenThreshold
        );
        assert!(store
            .load_latest("below", &fingerprint.digest)
            .unwrap()
            .is_none());

        let busy = FakeSlotBackend::new(&store, "busy");
        busy.configure(|state| state.is_processing = true);
        let coordinator = coordinator_with_healthy_engine(&store, "busy", 702);
        coordinator
            .restore_or_cold("busy", 702, &fingerprint, true, &busy)
            .unwrap();
        coordinator.begin_draining("busy", 702).unwrap();
        let status = coordinator
            .save_before_stop("busy", 702, &fingerprint, &config.kv_checkpoint, &busy)
            .unwrap();
        assert_eq!(status.phase, CheckpointPhase::Stopping);
        assert_eq!(status.last_outcome, CheckpointOutcome::Skipped);
        assert_eq!(status.reason_code, CheckpointReasonCode::BusyTimeout);

        let timeout = FakeSlotBackend::new(&store, "save-timeout");
        timeout.configure(|state| {
            state.slots_error = Some(CheckpointStoreError::new(
                CheckpointReasonCode::HttpTimeout,
                "injected slot timeout",
            ));
        });
        let coordinator = coordinator_with_healthy_engine(&store, "save-timeout", 703);
        coordinator
            .restore_or_cold("save-timeout", 703, &fingerprint, true, &timeout)
            .unwrap();
        coordinator.begin_draining("save-timeout", 703).unwrap();
        let status = coordinator
            .save_before_stop(
                "save-timeout",
                703,
                &fingerprint,
                &config.kv_checkpoint,
                &timeout,
            )
            .unwrap();
        assert_eq!(status.phase, CheckpointPhase::Stopping);
        assert_eq!(status.last_outcome, CheckpointOutcome::Failed);
        assert_eq!(status.reason_code, CheckpointReasonCode::HttpTimeout);
    }

    #[test]
    fn failed_termination_can_resume_routing_and_expected_exit_preserves_save_result() {
        let sandbox = TestSandbox::new("coordinator-stop-recovery");
        let store = sandbox.store();
        let config = eligible_config();
        let fingerprint = test_fingerprint(&config);
        let backend = FakeSlotBackend::new(&store, "instance-1");
        let coordinator = coordinator_with_healthy_engine(&store, "instance-1", 750);
        coordinator
            .restore_or_cold("instance-1", 750, &fingerprint, true, &backend)
            .unwrap();

        coordinator.begin_draining("instance-1", 750).unwrap();
        coordinator
            .skip_save_busy("instance-1", 750, 15_000)
            .unwrap();
        let resumed = coordinator
            .resume_after_stop_failure("instance-1", 750, CheckpointPhase::ReadyCold)
            .unwrap();
        assert_eq!(resumed.phase, CheckpointPhase::ReadyCold);
        assert!(resumed.routable);

        coordinator.begin_draining("instance-1", 750).unwrap();
        let stopping = coordinator
            .skip_save_busy("instance-1", 750, 15_000)
            .unwrap();
        let stopped = coordinator.mark_unexpected_exit("instance-1", 750).unwrap();
        assert_eq!(stopped.phase, CheckpointPhase::Stopped);
        assert_eq!(stopped.last_outcome, stopping.last_outcome);
        assert_eq!(stopped.reason_code, CheckpointReasonCode::BusyTimeout);
    }

    #[test]
    fn invalid_fingerprint_and_unexpected_exit_resolve_terminally() {
        let sandbox = TestSandbox::new("coordinator-terminal-fallbacks");
        let store = sandbox.store();
        let config = eligible_config();
        let valid_fingerprint = test_fingerprint(&config);
        let mut invalid_fingerprint = valid_fingerprint.clone();
        invalid_fingerprint.algorithm = "md5".into();

        let backend = FakeSlotBackend::new(&store, "invalid-restore");
        let coordinator = coordinator_with_healthy_engine(&store, "invalid-restore", 801);
        let status = coordinator
            .restore_or_cold("invalid-restore", 801, &invalid_fingerprint, true, &backend)
            .unwrap();
        assert_eq!(status.phase, CheckpointPhase::ReadyCold);
        assert_eq!(status.last_outcome, CheckpointOutcome::Failed);
        assert_eq!(status.reason_code, CheckpointReasonCode::ManifestInvalid);

        let coordinator = coordinator_with_healthy_engine(&store, "invalid-save", 802);
        coordinator
            .restore_or_cold("invalid-save", 802, &valid_fingerprint, true, &backend)
            .unwrap();
        coordinator.begin_draining("invalid-save", 802).unwrap();
        let status = coordinator
            .save_before_stop(
                "invalid-save",
                802,
                &invalid_fingerprint,
                &config.kv_checkpoint,
                &backend,
            )
            .unwrap();
        assert_eq!(status.phase, CheckpointPhase::Stopping);
        assert_eq!(status.reason_code, CheckpointReasonCode::ManifestInvalid);

        let backend = FakeSlotBackend::new(&store, "unexpected");
        let coordinator = coordinator_with_healthy_engine(&store, "unexpected", 803);
        coordinator
            .restore_or_cold("unexpected", 803, &valid_fingerprint, true, &backend)
            .unwrap();
        let status = coordinator.mark_unexpected_exit("unexpected", 803).unwrap();
        assert_eq!(status.phase, CheckpointPhase::Stopped);
        assert_eq!(status.reason_code, CheckpointReasonCode::UnexpectedExit);
        assert!(store
            .load_latest("unexpected", &valid_fingerprint.digest)
            .unwrap()
            .is_none());
    }

    fn one_shot_http_server(
        status: &'static str,
        body: Vec<u8>,
        delay: Duration,
    ) -> (u16, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request);
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
            let headers = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(headers.as_bytes());
            let _ = stream.write_all(&body);
        });
        (port, handle)
    }

    fn slot_client_for_port(port: u16, timeout: Duration) -> LlamaSlotClient {
        let config = InstanceConfig {
            host: "127.0.0.1".into(),
            port,
            ..eligible_config()
        };
        LlamaSlotClient::new(&config, timeout).unwrap()
    }

    #[test]
    fn slot_client_rejects_unsafe_names_malformed_results_and_timeouts() {
        assert_eq!(
            normalize_slot_http_timeout(Duration::from_secs(31 * 60)),
            Duration::from_secs(31 * 60),
            "large checkpoint operations must not be clamped back to 30 seconds"
        );
        for filename in [
            "../slot-0-bad.bin",
            "slot-1-1-deadbeef.bin",
            "slot-0-name with spaces.bin",
            "arbitrary.bin",
        ] {
            assert!(LlamaSlotClient::manager_filename(filename).is_err());
        }
        assert!(LlamaSlotClient::manager_filename(
            "slot-0-1-123e4567-e89b-12d3-a456-426614174000.bin"
        )
        .is_ok());

        let (port, server) = one_shot_http_server("200 OK", b"not-json".to_vec(), Duration::ZERO);
        let error = slot_client_for_port(port, Duration::from_secs(1))
            .health()
            .unwrap_err();
        assert_eq!(error.reason_code, CheckpointReasonCode::SlotApiError);
        server.join().unwrap();

        let invalid_save = serde_json::to_vec(&serde_json::json!({
            "id_slot": 0,
            "filename": "wrong.bin",
            "n_saved": 512,
            "n_written": 4096
        }))
        .unwrap();
        let (port, server) = one_shot_http_server("200 OK", invalid_save, Duration::ZERO);
        let error = slot_client_for_port(port, Duration::from_secs(1))
            .save("slot-0-1-123e4567-e89b-12d3-a456-426614174000.bin")
            .unwrap_err();
        assert_eq!(error.reason_code, CheckpointReasonCode::SaveResponseInvalid);
        server.join().unwrap();

        let (port, server) = one_shot_http_server(
            "200 OK",
            br#"{"status":"ok"}"#.to_vec(),
            Duration::from_millis(500),
        );
        let error = slot_client_for_port(port, Duration::from_millis(10))
            .health()
            .unwrap_err();
        assert_eq!(error.reason_code, CheckpointReasonCode::HttpTimeout);
        server.join().unwrap();
    }
}
