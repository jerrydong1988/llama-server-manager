use crate::models::InstanceConfig;
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
    SaveResponseInvalid,
    RestoreResponseInvalid,
    SlotStateMismatch,
    StorageLimit,
    IoError,
    HttpTimeout,
    SlotApiError,
    StaleProcessEvent,
    InvalidStateTransition,
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

pub const CHECKPOINT_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const CHECKPOINT_FINGERPRINT_SCHEMA_VERSION: u32 = 1;
const HASH_CACHE_SCHEMA_VERSION: u32 = 1;
const LATEST_POINTER_SCHEMA_VERSION: u32 = 1;
const USAGE_SCHEMA_VERSION: u32 = 1;
const STATE_FORMAT: &str = "llama.cpp-slot-state";
const SLOT_FILENAME: &str = "slot-0.bin";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 16 * 1024 * 1024;

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
    fn new(reason_code: CheckpointReasonCode, message: &'static str) -> Self {
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
    let grant = format!("*{sid}:(OI)(CI)F");
    let status = Command::new("icacls.exe")
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(grant)
        .args(["/T", "/C", "/Q"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| CheckpointStoreError::io("checkpoint ACL update failed"))?;
    if !status.success() {
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
    sha256: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckpointFingerprint {
    pub algorithm: String,
    pub digest: String,
    pub model_sha256: String,
    pub engine_sha256: String,
    pub engine_version: String,
    pub backend: String,
}

impl CheckpointFingerprint {
    fn validate(&self) -> StoreResult<()> {
        if self.algorithm != "sha256"
            || !is_lower_hex_digest(&self.digest)
            || !is_lower_hex_digest(&self.model_sha256)
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
    pub engine_sha256: String,
    pub engine_version: String,
    pub backend: String,
    pub chat_template_file_sha256: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalFingerprintV1<'a> {
    fingerprint_schema_version: u32,
    manifest_schema_version: u32,
    state_format: &'static str,
    model_sha256: &'a str,
    engine_sha256: &'a str,
    engine_version: &'a str,
    backend: &'a str,
    ctx_size: u32,
    ctx_size_auto: bool,
    parallel: i32,
    cont_batching: bool,
    cache_prompt: bool,
    cache_reuse: u32,
    cache_ram: i32,
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
    let canonical = CanonicalFingerprintV1 {
        fingerprint_schema_version: CHECKPOINT_FINGERPRINT_SCHEMA_VERSION,
        manifest_schema_version: CHECKPOINT_MANIFEST_SCHEMA_VERSION,
        state_format: STATE_FORMAT,
        model_sha256: &materials.model_sha256,
        engine_sha256: &materials.engine_sha256,
        engine_version: materials.engine_version.trim(),
        backend: materials.backend.trim(),
        ctx_size: config.ctx_size,
        ctx_size_auto: config.ctx_size_auto,
        parallel: config.parallel,
        cont_batching: config.cont_batching,
        cache_prompt: config.cache_prompt,
        cache_reuse: config.cache_reuse,
        cache_ram: config.cache_ram,
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
    AfterPayloadCopy,
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
        let key = canonical_path_cache_key(&canonical);
        let cache_path = self.root.join("fingerprints-v1.json");
        let mut cache = match read_bounded(&cache_path, MAX_METADATA_BYTES) {
            Ok(bytes) => serde_json::from_slice::<HashCacheFile>(&bytes)
                .ok()
                .filter(|cache| cache.schema_version == HASH_CACHE_SCHEMA_VERSION)
                .unwrap_or_default(),
            Err(_) => HashCacheFile::default(),
        };
        if let Some(entry) = cache.entries.get(&key) {
            if entry.size == before.len()
                && entry.modified_unix_nanos == modified_before
                && is_lower_hex_digest(&entry.sha256)
            {
                return Ok(entry.sha256.clone());
            }
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
        if before.len() != after.len() || modified_before != modified_after {
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
                sha256: digest.clone(),
            },
        );
        let encoded = serde_json::to_vec_pretty(&cache).map_err(|_| {
            CheckpointStoreError::new(
                CheckpointReasonCode::FingerprintUnavailable,
                "fingerprint cache serialization failed",
            )
        })?;
        crate::persistence::atomic_write(&cache_path, &encoded, None).map_err(|_| {
            CheckpointStoreError::new(
                CheckpointReasonCode::FingerprintUnavailable,
                "fingerprint cache persistence failed",
            )
        })?;
        Ok(digest)
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
        let materials = FingerprintMaterials {
            model_sha256: self.content_sha256(model_path)?,
            engine_sha256: self.content_sha256(engine_path)?,
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
            let mut source = File::open(scratch_payload)
                .map_err(|_| CheckpointStoreError::io("scratch payload open failed"))?;
            let mut options = OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut target = options
                .open(&destination)
                .map_err(|_| CheckpointStoreError::io("generation payload create failed"))?;
            std::io::copy(&mut source, &mut target)
                .map_err(|_| CheckpointStoreError::io("generation payload copy failed"))?;
            target
                .flush()
                .map_err(|_| CheckpointStoreError::io("generation payload flush failed"))?;
            inject(StoreFaultPoint::AfterPayloadCopy)?;
            target
                .sync_all()
                .map_err(|_| CheckpointStoreError::io("generation payload sync failed"))?;
            drop(target);
            protect_file(&destination)?;
            inject(StoreFaultPoint::AfterPayloadSync)?;
            if validate_regular_file(&destination)?.len() != slot.bytes
                || sha256_file(&destination)? != slot.sha256
            {
                return Err(CheckpointStoreError::new(
                    CheckpointReasonCode::ChecksumMismatch,
                    "copied generation payload failed verification",
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
        let mut latest_protected = HashSet::new();
        let mut generations = Vec::new();
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
            let generations_root = fingerprint_root.join("generations");
            if validate_directory(&generations_root).is_err() {
                continue;
            }
            let mut newest_valid: Option<(DateTime<chrono::FixedOffset>, String)> = None;
            if let Ok(latest) = self.read_latest_pointer(&fingerprint_root) {
                if self
                    .load_generation(
                        instance_id,
                        &fingerprint,
                        &generations_root,
                        &latest.generation_id,
                        Some(&latest.manifest_sha256),
                    )
                    .is_ok()
                {
                    latest_protected.insert(format!("{fingerprint}/{}", latest.generation_id));
                }
            }
            let entries = match fs::read_dir(&generations_root) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let generation_id = entry.file_name().to_string_lossy().to_string();
                if !validate_uuid(&generation_id) {
                    continue;
                }
                let Ok(loaded) = self.load_generation(
                    instance_id,
                    &fingerprint,
                    &generations_root,
                    &generation_id,
                    None,
                ) else {
                    continue;
                };
                let manifest_size = fs::metadata(loaded.generation_dir.join("manifest.json"))
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                let bytes = loaded.manifest.slot().bytes.saturating_add(manifest_size);
                let key = format!("{fingerprint}/{generation_id}");
                if let Ok(created_at) = DateTime::parse_from_rfc3339(&loaded.manifest.created_at) {
                    let is_newest = match newest_valid.as_ref() {
                        Some((newest, _)) => created_at > *newest,
                        None => true,
                    };
                    if is_newest {
                        newest_valid = Some((created_at, key.clone()));
                    }
                }
                let last_used = usage.entries.get(&key).copied().unwrap_or_else(|| {
                    DateTime::parse_from_rfc3339(&loaded.manifest.created_at)
                        .map(|value| value.timestamp_millis().max(0) as u64)
                        .unwrap_or(0)
                });
                generations.push((last_used, key, bytes, loaded.generation_dir));
            }
            if !latest_protected
                .iter()
                .any(|key| key.starts_with(&format!("{fingerprint}/")))
            {
                if let Some((_, newest_key)) = newest_valid {
                    latest_protected.insert(newest_key);
                }
            }
        }

        let mut remaining_bytes = generations.iter().fold(0_u64, |total, (_, _, bytes, _)| {
            total.saturating_add(*bytes)
        });
        generations.sort_by_key(|(last_used, _, _, _)| *last_used);
        let mut removed_generations = 0;
        let mut usage_next = usage;
        for (_, key, bytes, path) in generations {
            if remaining_bytes <= storage_limit_bytes {
                break;
            }
            if explicit_protected.contains(&key) || latest_protected.contains(&key) {
                continue;
            }
            let Some(generations_root) = path.parent() else {
                continue;
            };
            safe_remove_directory(&path, generations_root)?;
            remaining_bytes = remaining_bytes.saturating_sub(bytes);
            removed_generations += 1;
            usage_next.entries.remove(&key);
        }
        if usage_next.entries != self.read_usage(instance_id).entries {
            self.write_usage(instance_id, &usage_next)?;
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
            let slot = generation.manifest.slot();
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

pub struct LlamaSlotClient {
    client: reqwest::blocking::Client,
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
        let timeout = timeout
            .max(Duration::from_millis(250))
            .min(Duration::from_secs(30));
        let connect_timeout = timeout.min(Duration::from_secs(2));
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .connect_timeout(connect_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| CheckpointStoreError::io("slot HTTP client creation failed"))?;
        Ok(Self {
            client,
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
            self.client.get(format!("{}/health", self.base_url)),
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
            self.client.get(format!("{}/slots", self.base_url)),
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
            self.client
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
            self.client
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
            self.client.post(self.action_url("erase")),
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

    pub fn register_start(
        &self,
        instance_id: &str,
        expected_pid: u32,
        eligibility: &CheckpointEligibility,
    ) -> StoreResult<CheckpointStatus> {
        if !validate_identifier(instance_id) || expected_pid == 0 {
            return Err(CheckpointStoreError::new(
                CheckpointReasonCode::UnsupportedConfiguration,
                "checkpoint process identity is invalid",
            ));
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
            },
        );
        Ok(status)
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
        let _ = backend.erase();
        self.finish_ready_cold(
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
        entry.gate_active = false;
        entry.status.phase = CheckpointPhase::Stopped;
        entry.status.routable = false;
        entry.status.last_outcome = CheckpointOutcome::Skipped;
        entry.status.reason_code = CheckpointReasonCode::UnexpectedExit;
        entry.status.message = "process exited before checkpoint save".into();
        entry.status.updated_at = checkpoint_now_millis();
        Ok(entry.status.clone())
    }
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
        let eligibility = evaluate(&eligible_config());
        let starting = coordinator
            .register_start(instance_id, pid, &eligibility)
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
    fn fingerprint_excludes_transport_ui_sampling_and_manager_policy_fields() {
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

        let cache = fs::read(store.root().join("fingerprints-v1.json")).unwrap();
        let parsed: HashCacheFile = serde_json::from_slice(&cache).unwrap();
        assert_eq!(parsed.schema_version, HASH_CACHE_SCHEMA_VERSION);
        assert_eq!(parsed.entries.len(), 2);
        assert!(parsed.entries.keys().all(|key| is_lower_hex_digest(key)));
        assert!(!String::from_utf8(cache).unwrap().contains("model.gguf"));
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
            StoreFaultPoint::AfterPayloadCopy,
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
    fn coordinator_rejects_illegal_transitions_and_stale_pid_events() {
        let sandbox = TestSandbox::new("coordinator-transitions");
        let store = sandbox.store();
        let coordinator = CheckpointCoordinator::new(store);
        let eligibility = evaluate(&eligible_config());

        let starting = coordinator
            .register_start("instance-1", 101, &eligibility)
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
            .register_start("instance-1", 202, &eligibility)
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
