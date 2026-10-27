use crate::commands::model_inventory;
use crate::model_artifacts::resolve_engine_runtime_artifacts;
use crate::models::{AppState, EngineCapabilities, EngineInfo};
use crate::path_utils::{path_identity_key, paths_equal};
use std::collections::{BTreeSet, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
#[cfg(windows)]
use std::process::Command;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROBE_STREAM_BYTES: usize = 512 * 1024;
const MIN_CONFIDENT_FLAG_COUNT: usize = 10;
const REPORTED_DEFAULTS_VERSION: u8 = 2;
const CONTEXT_CHECKPOINT_HELP_MARKER: &str = "path to save slot kv cache and context checkpoints";
pub(crate) const PERSISTENCE_REVISION: u8 = 6;
fn single_slot_scope() -> Vec<i32> {
    vec![1]
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelPersistenceScope {
    model_sha256: String,
    draft_sha256: String,
    contexts: Vec<u32>,
    cache_types: Vec<String>,
    #[serde(default = "single_slot_scope")]
    parallel: Vec<i32>,
    #[allow(dead_code)]
    evidence: String,
}

impl ModelPersistenceScope {
    fn matches_config(&self, config: &crate::models::InstanceConfig) -> bool {
        let cache = |s: &str| {
            if s.is_empty() {
                "f16".to_string()
            } else {
                s.to_string()
            }
        };
        let kv = cache(&config.cache_type_k);
        // An explicit-override config can retain values which the command builder
        // intentionally omits. Those values cannot qualify the effective launch.
        let emits = |keys: &[&str]| {
            config.explicit_overrides.as_ref().map_or(true, |fields| {
                keys.iter()
                    .any(|key| fields.iter().any(|field| field.as_str() == *key))
            })
        };
        let emitted_shape = [
            &["ctx_size", "ctx_size_auto"][..],
            &["gpu_layers", "gpu_layers_auto"],
            &["draft_gpu_layers"],
            &["draft_model_path"],
            &["flash_attn"],
            &["fit_mode", "fit"],
            &["parallel"],
            &["ctx_checkpoints"],
            &["draft_tokens"],
            &["spec_type"],
            &["kv_unified", "kv_unified_mode"],
        ]
        .iter()
        .all(|keys| emits(keys))
            && [
                (&config.cache_type_k, "cache_type_k"),
                (&config.cache_type_v, "cache_type_v"),
                (&config.cache_type_draft_k, "cache_type_draft_k"),
                (&config.cache_type_draft_v, "cache_type_draft_v"),
            ]
            .iter()
            .all(|(value, key)| value.is_empty() || emits(&[*key]));
        config.spec_type == "draft-mtp"
            && config.draft_tokens == 3
            && self.parallel.contains(&config.parallel)
            && (config.parallel == 1 || config.ctx_size == 4096 * config.parallel as u32)
            && config.ctx_checkpoints == 2
            && !config.swa_full
            && config.kv_unified
            && matches!(config.kv_unified_mode.as_str(), "" | "on")
            && self.contexts.contains(&config.ctx_size)
            && self.cache_types.contains(&kv)
            && emitted_shape
            && (config.parallel == 1 || emits(&["cache_idle_slots"]))
            && [
                &config.cache_type_v,
                &config.cache_type_draft_k,
                &config.cache_type_draft_v,
            ]
            .iter()
            .all(|s| cache(s) == kv)
    }
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistenceProfile {
    id: String,
    artifact_sha256: String,
    #[serde(default)]
    base_artifact_sha256: Option<String>,
    #[allow(dead_code)]
    launcher_sha256: String,
    draft_types: Vec<String>,
    managed_lookup_cache: bool,
    max_context: u32,
    #[serde(default)]
    model_scopes: Vec<ModelPersistenceScope>,
    #[allow(dead_code)]
    evidence: String,
}

fn profiles() -> &'static [PersistenceProfile] {
    static PROFILES: std::sync::OnceLock<Vec<PersistenceProfile>> = std::sync::OnceLock::new();
    PROFILES.get_or_init(|| serde_json::from_str(include_str!("persistence_profiles.json"))
        .expect("embedded persistence profiles must be valid"))
}

// Newly reviewed bundles may require HIP's external kernel/data trees in
// addition to adjacent DLLs. Existing reviewed identities remain unchanged.
pub(crate) fn requires_auxiliary_runtime(base_artifacts: &str) -> bool {
    profiles().iter().any(|p| p.base_artifact_sha256.as_deref() == Some(base_artifacts))
}

fn profile_for_artifacts(artifacts: Option<&str>) -> Option<&'static PersistenceProfile> {
    cfg!(windows).then(|| profiles().iter().find(|p| Some(p.artifact_sha256.as_str()) == artifacts)).flatten()
}

pub(crate) fn supports_managed_lookup_cache(artifacts: &str) -> bool {
    profile_for_artifacts(Some(artifacts)).is_some_and(|p| p.managed_lookup_cache)
}

#[cfg(test)]
// Full launcher + all 28 adjacent DLL hashes, not a version/path heuristic.
pub(crate) const B11769_ARTIFACTS: &str = "51721bd434546a540548be496a58c9878774d41f7bc594b245031f38a1e0dfeb";

fn persistence_evidence(help: &str, artifacts: Option<&str>) -> (bool, Option<String>, Vec<String>) {
    if let Some(profile) = profile_for_artifacts(artifacts) {
        return (true, Some(profile.id.clone()), profile.draft_types.clone());
    }
    // A help claim establishes SCKP support, not qualification of every advertised draft mode.
    (detects_context_checkpoint_persistence(help), None, Vec::new())
}

#[cfg(test)]
fn checkpoint_types(config: &crate::models::InstanceConfig, caps: &EngineCapabilities) -> Vec<String> {
    checkpoint_types_with_models(config, caps, None)
}

fn hardware_scope(config: &crate::models::InstanceConfig) -> bool {
    !config.ctx_size_auto
        && config.gpu_layers >= 99
        && !config.gpu_layers_auto
        && config.draft_gpu_layers >= 99
        && config.flash_attn == "on"
        && config.fit_mode == "off"
        && !config.no_kv_offload
        && !config.cpu_moe
        && config.moe_cpu_layers == 0
        && config.rpc_servers.trim().is_empty()
        && config.device.trim().is_empty()
        && config.spec_draft_device.trim().is_empty()
}

pub(crate) fn needs_checkpoint_model_identity(
    config: &crate::models::InstanceConfig,
    caps: &EngineCapabilities,
) -> bool {
    config.kv_checkpoint.enabled
        && hardware_scope(config)
        && profiles()
            .iter()
            .filter(|p| Some(p.id.as_str()) == caps.persistence_profile.as_deref())
            .any(|p| p.model_scopes.iter().any(|s| s.matches_config(config)))
}

pub(crate) fn reviewed_windowed_swa(
    config: &crate::models::InstanceConfig,
    caps: &EngineCapabilities,
    models: Option<(&str, &str)>,
) -> bool {
    let Some((model, draft)) = models else {
        return false;
    };
    cfg!(windows)
        && caps.persistence_revision == PERSISTENCE_REVISION
        && hardware_scope(config)
        && caps.checkpoint_draft_types.iter().any(|k| k == "draft-mtp")
        && caps.speculative_types.iter().any(|k| k == "draft-mtp")
        && profiles()
            .iter()
            .filter(|p| Some(p.id.as_str()) == caps.persistence_profile.as_deref())
            .any(|p| {
                p.model_scopes.iter().any(|s| {
                    s.matches_config(config) && s.model_sha256 == model && s.draft_sha256 == draft
                })
            })
}

pub(crate) fn reviewed_multi_slot(
    config: &crate::models::InstanceConfig,
    caps: &EngineCapabilities,
    models: Option<(&str, &str)>,
) -> bool {
    matches!(config.parallel, 2 | 4) && reviewed_windowed_swa(config, caps, models)
}

// Recheck the runtime identity used by the final checkpoint fingerprint, after
// the potentially long model hashing step. A probe is not a launch-time lease.
pub(crate) fn persistence_runtime_matches(caps: &EngineCapabilities, digest: &str) -> bool {
    caps.persistence_profile.as_deref().map_or(true, |id| {
        profile_for_artifacts(Some(digest)).is_some_and(|p| p.id == id)
    })
}

pub(crate) fn checkpoint_types_with_models(
    config: &crate::models::InstanceConfig,
    caps: &EngineCapabilities,
    models: Option<(&str, &str)>,
) -> Vec<String> {
    let profile = profiles()
        .iter()
        .find(|p| Some(p.id.as_str()) == caps.persistence_profile.as_deref());
    let profile_scope = profile
        .is_some_and(|p| config.ctx_size > 0 && config.ctx_size <= p.max_context)
        && hardware_scope(config)
        && [
            &config.cache_type_k,
            &config.cache_type_v,
            &config.cache_type_draft_k,
            &config.cache_type_draft_v,
        ]
        .iter()
        .all(|s| s.is_empty() || s.as_str() == "f16");
    caps.speculative_types
        .iter()
        .filter(|kind| {
            !kind.starts_with("draft-")
                || (kind.as_str() == "draft-mtp" && reviewed_windowed_swa(config, caps, models))
                || (profile_scope
                    && profile.is_some_and(|p| p.draft_types.contains(kind))
                    && caps.checkpoint_draft_types.contains(kind)
                    && (kind.as_str() != "draft-simple"
                        || paths_equal(
                            std::path::Path::new(&config.model_path),
                            std::path::Path::new(&config.draft_model_path),
                        )))
        })
        .cloned()
        .collect()
}

#[derive(Debug)]
struct CommandOutput {
    text: String,
    timed_out: bool,
    error: Option<String>,
}

fn read_stream_capped<R: Read>(mut reader: R) -> Vec<u8> {
    let mut captured = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let remaining = MAX_PROBE_STREAM_BYTES.saturating_sub(captured.len());
                if remaining > 0 {
                    captured.extend_from_slice(&buffer[..count.min(remaining)]);
                }
            }
        }
    }
    captured
}

fn probe_output_file() -> Result<(std::path::PathBuf, File), String> {
    for _ in 0..4 {
        let path = std::env::temp_dir().join(format!(
            "llama-server-manager-probe-{}-{}.log",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("cannot create probe output file: {error}")),
        }
    }
    Err("cannot reserve a unique probe output file".to_string())
}

fn terminate_probe_process_tree(child: &mut std::process::Child) -> Option<String> {
    let pid = child.id();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let tree_killed = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(0x08000000)
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !tree_killed {
            let _ = child.kill();
        }
    }
    #[cfg(unix)]
    {
        let killed_group = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) } == 0;
        if !killed_group {
            let _ = child.kill();
        }
    }
    child.wait().err().map(|error| error.to_string())
}

fn run_bounded(executable: &str, argument: &str) -> CommandOutput {
    let (output_path, mut output_file) = match probe_output_file() {
        Ok(output) => output,
        Err(error) => {
            return CommandOutput {
                text: String::new(),
                timed_out: false,
                error: Some(error),
            }
        }
    };
    let stdout_file = match output_file.try_clone() {
        Ok(file) => file,
        Err(error) => {
            drop(output_file);
            let _ = std::fs::remove_file(output_path);
            return CommandOutput {
                text: String::new(),
                timed_out: false,
                error: Some(format!("cannot clone probe output handle: {error}")),
            };
        }
    };
    let stderr_file = match output_file.try_clone() {
        Ok(file) => file,
        Err(error) => {
            drop(stdout_file);
            drop(output_file);
            let _ = std::fs::remove_file(output_path);
            return CommandOutput {
                text: String::new(),
                timed_out: false,
                error: Some(format!("cannot clone probe error handle: {error}")),
            };
        }
    };
    let mut command = crate::process_environment::external_command(executable);
    command
        .arg(argument)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00000200);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            drop(command);
            drop(output_file);
            let _ = std::fs::remove_file(output_path);
            return CommandOutput {
                text: String::new(),
                timed_out: false,
                error: Some(format!("cannot execute llama-server: {error}")),
            };
        }
    };
    drop(command);
    let started = Instant::now();
    let mut timed_out = false;
    let wait_error = loop {
        match child.try_wait() {
            Ok(Some(_)) => break None,
            Ok(None) if started.elapsed() < PROBE_TIMEOUT => {
                thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                timed_out = true;
                break terminate_probe_process_tree(&mut child);
            }
            Err(error) => {
                let cleanup_error = terminate_probe_process_tree(&mut child);
                break Some(match cleanup_error {
                    Some(cleanup) => format!("{error}; cleanup failed: {cleanup}"),
                    None => error.to_string(),
                });
            }
        }
    };

    let _ = output_file.seek(SeekFrom::Start(0));
    let combined = read_stream_capped(&mut output_file);
    drop(output_file);
    let _ = std::fs::remove_file(output_path);

    CommandOutput {
        text: String::from_utf8_lossy(&combined).into_owned(),
        timed_out,
        error: wait_error,
    }
}

fn is_flag_body(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '-'
}

pub(crate) fn extract_supported_flags(output: &str) -> Vec<String> {
    let characters = output.char_indices().collect::<Vec<_>>();
    let mut flags = BTreeSet::new();
    let mut index = 0;
    while index < characters.len() {
        let (byte_index, character) = characters[index];
        if character != '-' {
            index += 1;
            continue;
        }
        if index > 0 {
            let previous = characters[index - 1].1;
            if previous.is_ascii_alphanumeric() || previous == '_' {
                index += 1;
                continue;
            }
        }

        let mut end = byte_index + character.len_utf8();
        let mut cursor = index + 1;
        while cursor < characters.len() {
            let (candidate_index, candidate) = characters[cursor];
            if candidate == '-' || is_flag_body(candidate) {
                end = candidate_index + candidate.len_utf8();
                cursor += 1;
            } else {
                break;
            }
        }
        let token = &output[byte_index..end];
        let body = token.trim_start_matches('-');
        if !body.is_empty()
            && body
                .chars()
                .any(|candidate| candidate.is_ascii_alphabetic())
            && token.len() <= 96
        {
            flags.insert(token.to_string());
        }
        index = cursor.max(index + 1);
    }
    flags.into_iter().collect()
}

fn is_speculative_type_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        && (value == "none" || value.contains('-'))
}

pub(crate) fn extract_speculative_types(output: &str) -> Vec<String> {
    let Some(marker) = output.find("--spec-type") else {
        return Vec::new();
    };
    let tail = &output[marker + "--spec-type".len()..];
    let lowercase = tail.to_ascii_lowercase();
    let end = lowercase
        .find("comma-separated list")
        .unwrap_or_else(|| tail.len().min(2048));
    let declaration = tail[..end.min(2048)].split_whitespace().collect::<String>();
    let mut types = Vec::new();
    for value in declaration.split(',') {
        let value = value
            .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '-')
            .to_ascii_lowercase();
        if is_speculative_type_name(&value) && !types.contains(&value) {
            types.push(value);
        }
    }
    types
}

fn defaults_from_help_block(flags: &[String], block: &str, defaults: &mut HashMap<String, String>) {
    let Some(marker) = block.find("(default:") else {
        return;
    };
    let value = &block[marker + "(default:".len()..];
    let Some(end) = value.find(')') else {
        return;
    };
    let value = value[..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if value.is_empty() {
        return;
    }
    for flag in flags {
        defaults.insert(flag.clone(), value.clone());
    }
}

/// Extract defaults reported by the selected executable for explanation only.
/// Command generation never relies on this data because upstream help text can
/// itself contain mistakes (b10068's --perf wording is one known example).
pub(crate) fn extract_reported_defaults(output: &str) -> HashMap<String, String> {
    let mut defaults = HashMap::new();
    let mut current_flags = Vec::new();
    let mut block = String::new();

    for line in output.lines() {
        let trimmed = line.trim_start();
        let indentation = line.len().saturating_sub(trimmed.len());
        let candidates = if indentation <= 4 && trimmed.starts_with('-') {
            extract_supported_flags(trimmed)
        } else {
            Vec::new()
        };
        if !candidates.is_empty() {
            defaults_from_help_block(&current_flags, &block, &mut defaults);
            current_flags = candidates;
            block.clear();
        }
        if !block.is_empty() {
            block.push(' ');
        }
        block.push_str(trimmed);
    }
    defaults_from_help_block(&current_flags, &block, &mut defaults);
    defaults
}

fn help_hash(output: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in output
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .bytes()
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(crate) fn detects_context_checkpoint_persistence(output: &str) -> bool {
    output
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
        .contains(CONTEXT_CHECKPOINT_HELP_MARKER)
}

fn first_nonempty_line(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(
                trimmed
                    .chars()
                    .filter(|character| !character.is_control())
                    .take(160)
                    .collect(),
            )
        }
    })
}

fn extract_engine_version(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let trimmed = line.trim();
        let lowercase = trimmed.to_ascii_lowercase();
        let payload = [
            "version:",
            "version =",
            "llama-server version",
            "llama.cpp version",
        ]
        .iter()
        .find_map(|prefix| {
            lowercase
                .strip_prefix(prefix)
                .map(|rest| rest.trim_start_matches([' ', ':', '=']).trim())
        });
        if !matches!(payload, Some(value) if !value.is_empty()) {
            return None;
        }
        let sanitized = trimmed
            .chars()
            .filter(|character| !character.is_control())
            .take(160)
            .collect::<String>();
        (!sanitized.is_empty()).then_some(sanitized)
    })
}

fn compact_error(message: impl Into<String>) -> String {
    message
        .into()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|character| !character.is_control())
        .take(240)
        .collect()
}

fn classify_probe_status(supported_flags: &[String], timed_out: bool) -> &'static str {
    if timed_out {
        return "timeout";
    }
    let has_model = supported_flags
        .iter()
        .any(|flag| flag == "-m" || flag == "--model");
    let has_server = supported_flags
        .iter()
        .any(|flag| flag == "--host" || flag == "--port");
    if has_model && has_server && supported_flags.len() >= MIN_CONFIDENT_FLAG_COUNT {
        "detected"
    } else if !supported_flags.is_empty() {
        "partial"
    } else {
        "failed"
    }
}

fn update_fingerprint_hash(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

pub(crate) fn executable_fingerprint(executable: &str) -> String {
    const SAMPLE_BYTES: u64 = 32 * 1024;

    let path = std::fs::canonicalize(executable).unwrap_or_else(|_| executable.into());
    let Some(artifacts) = resolve_engine_runtime_artifacts(&path) else {
        return String::new();
    };
    let normalized_path = path_identity_key(&path);

    let mut hash = 0xcbf29ce484222325_u64;
    update_fingerprint_hash(&mut hash, normalized_path.as_bytes());
    update_fingerprint_hash(&mut hash, &(artifacts.len() as u32).to_le_bytes());
    for artifact in &artifacts {
        let canonical = match std::fs::canonicalize(artifact) {
            Ok(path) => path,
            Err(_) => return String::new(),
        };
        let metadata = match canonical.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => return String::new(),
        };
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let name = artifact
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        update_fingerprint_hash(&mut hash, name.as_bytes());
        update_fingerprint_hash(&mut hash, &metadata.len().to_le_bytes());
        update_fingerprint_hash(&mut hash, &modified.to_le_bytes());

        let mut file = match File::open(&canonical) {
            Ok(file) => file,
            Err(_) => return String::new(),
        };
        let mut offsets = BTreeSet::new();
        offsets.insert(0_u64);
        offsets.insert(metadata.len().saturating_sub(SAMPLE_BYTES) / 2);
        offsets.insert(metadata.len().saturating_sub(SAMPLE_BYTES));
        let mut buffer = vec![0_u8; SAMPLE_BYTES as usize];
        for offset in offsets {
            if file.seek(SeekFrom::Start(offset)).is_err() {
                return String::new();
            }
            let count = match file.read(&mut buffer) {
                Ok(count) => count,
                Err(_) => return String::new(),
            };
            update_fingerprint_hash(&mut hash, &offset.to_le_bytes());
            update_fingerprint_hash(&mut hash, &buffer[..count]);
        }
        let after = match canonical.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => return String::new(),
        };
        let modified_after = after
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        if after.len() != metadata.len() || modified_after != modified {
            return String::new();
        }
    }

    format!("v3:{normalized_path}:{}:{hash:016x}", artifacts.len())
}

pub(crate) fn capabilities_match_executable(
    executable: &str,
    capabilities: &EngineCapabilities,
) -> bool {
    capabilities.persistence_revision == PERSISTENCE_REVISION
        && !capabilities.executable_fingerprint.is_empty()
        && capabilities.executable_fingerprint == executable_fingerprint(executable)
        && (capabilities.persistence_profile.is_none() ||
            crate::checkpoint::CheckpointStore::new(crate::utils::get_data_dir())
                .engine_artifact_sha256(std::path::Path::new(executable)).ok()
                .and_then(|hash| profile_for_artifacts(Some(&hash)))
                .is_some_and(|profile| capabilities.persistence_profile.as_deref() == Some(profile.id.as_str())
                    && capabilities.checkpoint_draft_types == profile.draft_types))
}

fn probe_engine(mut engine: EngineInfo) -> EngineInfo {
    let fingerprint_before = executable_fingerprint(&engine.exe);
    let version_output = run_bounded(&engine.exe, "--version");
    let help_output = run_bounded(&engine.exe, "--help");
    let supported_flags = extract_supported_flags(&help_output.text);
    let reported_defaults = extract_reported_defaults(&help_output.text);
    let speculative_types = extract_speculative_types(&help_output.text);
    let status = classify_probe_status(&supported_flags, help_output.timed_out);
    let artifacts = if cfg!(windows) && status == "detected" {
        crate::checkpoint::CheckpointStore::new(crate::utils::get_data_dir())
            .engine_artifact_sha256(std::path::Path::new(&engine.exe)).ok()
    } else { None };
    let (context_checkpoint_persistence, persistence_profile, checkpoint_draft_types) =
        if status == "detected" && help_output.error.is_none() {
            persistence_evidence(&help_output.text, artifacts.as_deref())
        } else { (false, None, Vec::new()) };
    let fingerprint = executable_fingerprint(&engine.exe);
    if fingerprint_before.is_empty() || fingerprint_before != fingerprint {
        engine.version.clear();
        engine.capabilities = EngineCapabilities {
            error: Some("engine executable changed while compatibility probing was in progress; probe again".to_string()),
            ..EngineCapabilities::default()
        };
        return engine;
    }
    let detected_version = extract_engine_version(&version_output.text);
    let preserve_existing_version = engine.capabilities.version_status == "detected"
        && engine.capabilities.executable_fingerprint == fingerprint
        && !engine.version.trim().is_empty();

    let mut errors = Vec::new();
    if version_output.timed_out {
        errors.push("--version timed out".to_string());
    }
    if help_output.timed_out {
        errors.push("--help timed out".to_string());
    }
    if let Some(error) = version_output.error {
        errors.push(error);
    }
    if let Some(error) = help_output.error {
        errors.push(error);
    }
    if status == "partial" {
        errors
            .push("help output was incomplete; compatibility enforcement is disabled".to_string());
    } else if status == "failed" && errors.is_empty() {
        errors.push("llama-server help did not expose recognizable command-line flags".to_string());
    }

    if let Some(version) = detected_version {
        engine.version = version;
    } else if !preserve_existing_version {
        engine.version.clear();
    }
    engine.capabilities = EngineCapabilities {
        status: status.to_string(),
        version_status: if engine.version.trim().is_empty() {
            "unknown".to_string()
        } else {
            "detected".to_string()
        },
        version_probe_detail: first_nonempty_line(&version_output.text),
        supported_flags,
        reported_defaults,
        reported_defaults_version: REPORTED_DEFAULTS_VERSION,
        speculative_types,
        context_checkpoint_persistence,
        persistence_revision: PERSISTENCE_REVISION,
        persistence_profile,
        checkpoint_draft_types,
        help_hash: if help_output.text.is_empty() {
            String::new()
        } else {
            help_hash(&help_output.text)
        },
        executable_fingerprint: fingerprint,
        probed_at: Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0),
        ),
        error: if errors.is_empty() {
            None
        } else {
            Some(compact_error(errors.join("; ")))
        },
    };
    engine
}

fn command_flag(token: &str) -> Option<&str> {
    if !token.starts_with('-') {
        return None;
    }
    let flag = token.split_once('=').map(|(name, _)| name).unwrap_or(token);
    let body = flag.trim_start_matches('-');
    if body.is_empty()
        || !body
            .chars()
            .any(|character| character.is_ascii_alphabetic())
    {
        return None;
    }
    Some(flag)
}

fn known_flag_value_count(flag: &str) -> Option<usize> {
    let count = match flag {
        "-m"
        | "--model"
        | "-a"
        | "--lora"
        | "--lora-scaled"
        | "--mmproj"
        | "--mmproj-url"
        | "--chat-template"
        | "--chat-template-file"
        | "--reasoning-format"
        | "--reasoning"
        | "--reasoning-budget"
        | "--reasoning-budget-message"
        | "--chat-template-kwargs"
        | "--grammar-file"
        | "--grammar"
        | "-c"
        | "-ngl"
        | "-t"
        | "-b"
        | "-ub"
        | "-np"
        | "--threads-batch"
        | "--threads-http"
        | "--keep"
        | "--cache-reuse"
        | "-cram"
        | "-ctxcp"
        | "-cms"
        | "--rope-scaling"
        | "--rope-scale"
        | "--rope-freq-base"
        | "--rope-freq-scale"
        | "--yarn-ext-factor"
        | "--yarn-attn-factor"
        | "--yarn-beta-slow"
        | "--yarn-beta-fast"
        | "--yarn-orig-ctx"
        | "-fa"
        | "--n-cpu-moe"
        | "--numa"
        | "--fit"
        | "-fitt"
        | "-fitc"
        | "--load-mode"
        | "-lm"
        | "-ctk"
        | "-ctv"
        | "-ctkd"
        | "-ctvd"
        | "-dev"
        | "-sm"
        | "-ts"
        | "-mg"
        | "--override-kv"
        | "-md"
        | "-ngld"
        | "--spec-draft-n-max"
        | "--spec-draft-n-min"
        | "--spec-draft-p-min"
        | "--spec-draft-p-split"
        | "--spec-draft-device"
        | "--spec-type"
        | "-lcs"
        | "-lcd"
        | "-td"
        | "-tbd"
        | "--api-key"
        | "--api-key-file"
        | "--ssl-key-file"
        | "--ssl-cert-file"
        | "--path"
        | "--api-prefix"
        | "--cors-origins"
        | "--cors-methods"
        | "--cors-headers"
        | "--ui-config-file"
        | "--ui-config"
        | "--pooling"
        | "--embd-normalize"
        | "-n"
        | "--json-schema"
        | "-jf"
        | "--temp"
        | "--top-k"
        | "--top-p"
        | "--repeat-penalty"
        | "--seed"
        | "--min-p"
        | "--xtc-probability"
        | "--xtc-threshold"
        | "--typical-p"
        | "--repeat-last-n"
        | "-r"
        | "--frequency-penalty"
        | "--presence-penalty"
        | "--mirostat"
        | "--mirostat-lr"
        | "--mirostat-ent"
        | "--dynatemp-range"
        | "--dynatemp-exp"
        | "--dry-multiplier"
        | "--dry-base"
        | "--dry-allowed-length"
        | "--dry-penalty-last-n"
        | "--dry-sequence-breaker"
        | "--adaptive-target"
        | "--adaptive-decay"
        | "--top-n-sigma"
        | "-l"
        | "--samplers"
        | "--sampler-seq"
        | "-to"
        | "--sleep-idle-seconds"
        | "--slot-save-path"
        | "--log-prompts-dir"
        | "-sps"
        | "--rpc"
        | "--host"
        | "--port"
        | "--models-dir"
        | "--models-preset"
        | "--models-max"
        | "--sse-ping-interval"
        | "--tags"
        | "--media-path"
        | "--tools"
        | "--tools-runtime"
        | "--mcp-servers-config"
        | "--mcp-servers-json"
        | "--image-min-tokens"
        | "--image-max-tokens"
        | "--mtmd-batch-max-tokens" => 1,
        "--lora-init-without-apply"
        | "--mmproj-auto"
        | "--no-mmproj"
        | "--no-mmproj-offload"
        | "--mmproj-offload"
        | "--skip-chat-parsing"
        | "--reasoning-preserve"
        | "--no-reasoning-preserve"
        | "--jinja"
        | "--no-jinja"
        | "-cb"
        | "--no-cont-batching"
        | "--cache-prompt"
        | "--no-cache-prompt"
        | "--warmup"
        | "--no-warmup"
        | "--swa-full"
        | "--cpu-moe"
        | "--mlock"
        | "--no-mmap"
        | "--mmap"
        | "--no-repack"
        | "--repack"
        | "--direct-io"
        | "--check-tensors"
        | "--perf"
        | "--no-perf"
        | "--kv-unified"
        | "--no-kv-unified"
        | "--no-kv-offload"
        | "--kv-offload"
        | "--cache-idle-slots"
        | "--no-cache-idle-slots"
        | "--spec-default"
        | "--no-spec-draft-backend-sampling"
        | "--spec-draft-backend-sampling"
        | "--no-ui"
        | "--ui"
        | "--offline"
        | "--cors-credentials"
        | "--no-cors-credentials"
        | "--ui-mcp-proxy"
        | "--agent"
        | "--embedding"
        | "--reranking"
        | "--ignore-eos"
        | "-sp"
        | "--spm-infill"
        | "-bs"
        | "--context-shift"
        | "--no-context-shift"
        | "-v"
        | "--metrics"
        | "--props"
        | "--slots"
        | "--no-slots"
        | "--prefill-assistant"
        | "--no-prefill-assistant"
        | "--reuse-port"
        | "--models-autoload"
        | "--no-models-autoload" => 0,
        _ => return None,
    };
    Some(count)
}

#[derive(Debug)]
struct CommandArgumentGroup<'a> {
    flag: &'a str,
    tokens: &'a [String],
}

fn command_argument_groups(command: &[String]) -> Vec<CommandArgumentGroup<'_>> {
    let mut groups = Vec::new();
    let mut index = 1;
    while index < command.len() {
        let Some(flag) = command_flag(&command[index]) else {
            index += 1;
            continue;
        };
        let start = index;
        index += 1;
        let value_count = if command[start].contains('=') {
            0
        } else if let Some(count) = known_flag_value_count(flag) {
            count
        } else if index < command.len() && command_flag(&command[index]).is_none() {
            1
        } else {
            0
        };
        index = (index + value_count).min(command.len());
        groups.push(CommandArgumentGroup {
            flag,
            tokens: &command[start..index],
        });
    }
    groups
}

pub(crate) fn unsupported_command_flags(
    command: &[String],
    capabilities: &EngineCapabilities,
) -> Vec<String> {
    if capabilities.status != "detected" {
        return Vec::new();
    }
    let supported = capabilities
        .supported_flags
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let reported_speculative_types = capabilities
        .speculative_types
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut unsupported = BTreeSet::new();
    for group in command_argument_groups(command) {
        if !supported.contains(group.flag) {
            unsupported.insert(group.flag.to_string());
            continue;
        }
        // Older/forked engines can advertise the flag without printing its enum.
        // Preserve the historical flag-level behavior unless the engine supplied
        // an authoritative value list; checkpoint eligibility remains stricter.
        if group.flag != "--spec-type" || reported_speculative_types.is_empty() {
            continue;
        }
        let value = group.tokens[0]
            .split_once('=')
            .map(|(_, value)| value)
            .or_else(|| group.tokens.get(1).map(String::as_str))
            .unwrap_or_default();
        for speculative_type in crate::speculative::parse_speculative_types(value) {
            if !reported_speculative_types.contains(&speculative_type) {
                unsupported.insert(format!("--spec-type={speculative_type}"));
            }
        }
    }
    unsupported.into_iter().collect()
}

fn preserve_in_conservative_mode(flag: &str) -> bool {
    matches!(
        flag,
        "-m" | "--model"
            | "--host"
            | "--port"
            | "--embedding"
            | "--reranking"
            | "--pooling"
            | "--api-key"
            | "--api-key-file"
            | "--ssl-key-file"
            | "--ssl-cert-file"
            | "--offline"
            | "--no-ui"
    )
}

pub(crate) fn blocked_security_flags(
    command: &[String],
    capabilities: Option<&EngineCapabilities>,
) -> Vec<String> {
    const SECURITY_FLAGS: [&str; 5] = [
        "--cors-origins",
        "--cors-methods",
        "--cors-headers",
        "--cors-credentials",
        "--no-cors-credentials",
    ];
    let supported = capabilities
        .filter(|value| matches!(value.status.as_str(), "detected" | "partial"))
        .map(|value| {
            value
                .supported_flags
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    command_argument_groups(command)
        .into_iter()
        .map(|group| group.flag)
        .filter(|flag| SECURITY_FLAGS.contains(flag) && !supported.contains(flag))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn command_for_capabilities(
    command: &[String],
    capabilities: Option<&EngineCapabilities>,
) -> Vec<String> {
    let Some(executable) = command.first() else {
        return Vec::new();
    };
    if capabilities.is_some_and(|value| value.status == "detected") {
        return command.to_vec();
    }

    let recognized = capabilities
        .filter(|value| value.status == "partial")
        .map(|value| {
            value
                .supported_flags
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let mut projected = vec![executable.clone()];
    for group in command_argument_groups(command) {
        let retain = preserve_in_conservative_mode(group.flag) || recognized.contains(group.flag);
        if retain {
            projected.extend(group.tokens.iter().cloned());
        }
    }
    projected
}

pub async fn probe_engine_capabilities(
    engine_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<EngineInfo, String> {
    let engine = state
        .engines
        .lock()
        .unwrap()
        .iter()
        .find(|engine| {
            paths_equal(
                std::path::Path::new(&engine.id),
                std::path::Path::new(&engine_id),
            )
        })
        .cloned()
        .ok_or_else(|| "engine not found".to_string())?;
    let authorized_root =
        crate::security::require_authorized_engine_root(std::path::Path::new(&engine.dir))?;
    crate::security::require_path_within_root(std::path::Path::new(&engine.exe), &authorized_root)?;
    let mut probed = tokio::task::spawn_blocking(move || probe_engine(engine))
        .await
        .map_err(|error| format!("engine capability probe task failed: {error}"))?;
    let mut engines = state.engines.lock().unwrap();
    let current = engines
        .iter_mut()
        .find(|engine| {
            paths_equal(
                std::path::Path::new(&engine.id),
                std::path::Path::new(&probed.id),
            )
        })
        .ok_or_else(|| "engine was removed while capability probing was in progress".to_string())?;
    let current_path =
        std::fs::canonicalize(&current.exe).unwrap_or_else(|_| current.exe.clone().into());
    let probed_path =
        std::fs::canonicalize(&probed.exe).unwrap_or_else(|_| probed.exe.clone().into());
    if !paths_equal(&current_path, &probed_path) {
        return Err(
            "engine executable changed while capability probing was in progress".to_string(),
        );
    }
    if !capabilities_match_executable(&current.exe, &probed.capabilities) {
        return Err(probed.capabilities.error.clone().unwrap_or_else(|| {
            "engine executable changed while capability probing was in progress".to_string()
        }));
    }
    *current = probed.clone();
    if let Err(error) = model_inventory::update_engine_probe(&probed) {
        let warning = compact_error(format!("capability cache was not persisted: {error}"));
        probed.capabilities.error = Some(match probed.capabilities.error.take() {
            Some(existing) => compact_error(format!("{existing}; {warning}")),
            None => warning,
        });
        current.capabilities = probed.capabilities.clone();
    }
    drop(engines);
    Ok(probed)
}

#[allow(dead_code, unused_imports)]
pub mod ipc {
    use super::*;

    #[tauri::command]
    pub async fn probe_engine_capabilities(
        engine_id: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<EngineInfo> {
        super::probe_engine_capabilities(engine_id, state)
            .await
            .map_err(crate::error::AppError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_unique_bounded_and_does_not_trust_serialized_extra_modes() {
        let mut ids = BTreeSet::new();
        let mut artifacts = BTreeSet::new();
        for profile in profiles() {
            assert!(ids.insert(&profile.id));
            assert!(artifacts.insert(&profile.artifact_sha256));
            for digest in [&profile.artifact_sha256, &profile.launcher_sha256] {
                assert_eq!(digest.len(), 64);
                assert!(digest.bytes().all(|c| c.is_ascii_hexdigit()));
            }
            if let Some(base) = &profile.base_artifact_sha256 {
                assert_eq!(base.len(), 64);
                assert!(base.bytes().all(|c| c.is_ascii_hexdigit()));
                assert_ne!(base, &profile.artifact_sha256);
                assert!(profile_for_artifacts(Some(base)).is_none());
                assert!(requires_auxiliary_runtime(base));
            }
            assert!(profile.max_context > 0 && profile.max_context <= 4096);
            assert!(!profile.evidence.is_empty());
            for scope in &profile.model_scopes {
                for digest in [&scope.model_sha256, &scope.draft_sha256] {
                    assert_eq!(digest.len(), 64);
                    assert!(digest.bytes().all(|c| c.is_ascii_hexdigit()));
                }
                assert!(!scope.contexts.is_empty());
                assert!(scope.contexts.iter().all(|n| [4096, 16384].contains(n)));
                assert!(!scope.cache_types.is_empty());
                assert!(scope.cache_types.iter().all(|k| ["f16", "q8_0"].contains(&k.as_str())));
                assert!(!scope.evidence.is_empty());
            }
            let caps = EngineCapabilities {
                persistence_profile: Some(profile.id.clone()),
                checkpoint_draft_types: vec!["draft-eagle3".into()],
                speculative_types: vec!["draft-eagle3".into()],
                ..Default::default()
            };
            let config = crate::models::InstanceConfig {
                ctx_size: 4096, ctx_size_auto: false, gpu_layers: 99,
                gpu_layers_auto: false, draft_gpu_layers: 99,
                flash_attn: "on".into(), fit_mode: "off".into(), ..Default::default()
            };
            assert!(checkpoint_types(&config, &caps).is_empty());
        }
        assert!(profile_for_artifacts(Some("95ec5d0a")).is_none());
    }

    #[test]
    fn persistence_requires_bundle_evidence_and_separate_draft_qualification() {
        let old_help = "--slot-save-path FNAME path to save slot kv cache (default: disabled)";
        assert_eq!(persistence_evidence(old_help, None), (false, None, vec![]));
        assert!(!persistence_evidence(old_help, Some("b11769")).0);
        assert!(persistence_evidence(CONTEXT_CHECKPOINT_HELP_MARKER, None).0);
        assert!(persistence_evidence(CONTEXT_CHECKPOINT_HELP_MARKER, None).2.is_empty());
        if cfg!(windows) {
            let (supported, profile, kinds) = persistence_evidence(old_help, Some(B11769_ARTIFACTS));
            assert!(supported);
            assert!(!kinds.contains(&"draft-eagle3".to_string()));
            let mut caps = EngineCapabilities { persistence_profile: profile, checkpoint_draft_types: kinds,
                speculative_types: vec!["ngram-cache".into(), "draft-mtp".into(), "draft-eagle3".into()], ..Default::default() };
            let mut config = crate::models::InstanceConfig { ctx_size: 4096, ctx_size_auto: false,
                gpu_layers: 99, gpu_layers_auto: false, draft_gpu_layers: 99,
                flash_attn: "on".into(), fit_mode: "off".into(), ..Default::default() };
            assert_eq!(checkpoint_types(&config, &caps), vec!["ngram-cache", "draft-mtp"]);
            config.cache_type_draft_k = "q8_0".into();
            assert_eq!(checkpoint_types(&config, &caps), vec!["ngram-cache"]);
            config.cache_type_draft_k.clear(); config.cpu_moe = true;
            assert_eq!(checkpoint_types(&config, &caps), vec!["ngram-cache"]);
            config.cpu_moe = false; config.ctx_size = 0;
            assert_eq!(checkpoint_types(&config, &caps), vec!["ngram-cache"]);
            config.ctx_size = 4096; caps.persistence_profile = None;
            assert_eq!(checkpoint_types(&config, &caps), vec!["ngram-cache"]);
        }
    }

    #[test]
    fn model_scopes_require_exact_pair_and_reviewed_launch_shape() {
        let profile = profiles()
            .iter()
            .find(|p| p.id == "windows-rocm-b11849-rocm103-sckp1-sdft1")
            .unwrap();
        let scope = profile
            .model_scopes
            .first()
            .expect("Gemma acceptance scope");
        let caps = EngineCapabilities {
            persistence_profile: Some(profile.id.clone()),
            persistence_revision: PERSISTENCE_REVISION,
            checkpoint_draft_types: vec!["draft-mtp".into()],
            speculative_types: vec!["draft-mtp".into()],
            ..Default::default()
        };
        let config = crate::models::InstanceConfig {
            spec_type: "draft-mtp".into(),
            draft_tokens: 3,
            parallel: 1,
            ctx_checkpoints: 2,
            ctx_size: 16384,
            ctx_size_auto: false,
            kv_unified: true,
            gpu_layers: 99,
            gpu_layers_auto: false,
            draft_gpu_layers: 99,
            flash_attn: "on".into(),
            fit_mode: "off".into(),
            cache_type_k: "q8_0".into(),
            cache_type_v: "q8_0".into(),
            cache_type_draft_k: "q8_0".into(),
            cache_type_draft_v: "q8_0".into(),
            ..Default::default()
        };
        let pair = Some((scope.model_sha256.as_str(), scope.draft_sha256.as_str()));
        for context in [4096, 16384] {
            for kv in ["f16", "q8_0"] {
                let mut tested = config.clone();
                tested.ctx_size = context;
                tested.cache_type_k = kv.into();
                tested.cache_type_v = kv.into();
                tested.cache_type_draft_k = kv.into();
                tested.cache_type_draft_v = kv.into();
                assert_eq!(reviewed_windowed_swa(&tested, &caps, pair), cfg!(windows));
            }
        }
        let mut explicit = config.clone();
        explicit.explicit_overrides = Some(
            [
                "ctx_size",
                "gpu_layers",
                "draft_gpu_layers",
                "draft_model_path",
                "flash_attn",
                "fit_mode",
                "parallel",
                "ctx_checkpoints",
                "draft_tokens",
                "spec_type",
                "kv_unified",
                "cache_type_k",
                "cache_type_v",
                "cache_type_draft_k",
                "cache_type_draft_v",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        );
        assert_eq!(reviewed_windowed_swa(&explicit, &caps, pair), cfg!(windows));
        for key in [
            "ctx_size",
            "draft_model_path",
            "cache_type_v",
            "ctx_checkpoints",
        ] {
            let mut missing = explicit.clone();
            missing
                .explicit_overrides
                .as_mut()
                .unwrap()
                .retain(|field| field != key);
            assert!(
                !reviewed_windowed_swa(&missing, &caps, pair),
                "unemitted {key}"
            );
        }
        for altered in [
            None,
            Some(("changed", scope.draft_sha256.as_str())),
            Some((scope.model_sha256.as_str(), "changed")),
        ] {
            assert!(!reviewed_windowed_swa(&config, &caps, altered));
            assert!(checkpoint_types_with_models(&config, &caps, altered).is_empty());
        }
        let mutations: &[fn(&mut crate::models::InstanceConfig)] = &[
            |c| c.ctx_size = 8192,
            |c| c.parallel = 2,
            |c| c.ctx_checkpoints = 0,
            |c| c.draft_tokens = 1,
            |c| c.cache_type_draft_v = "f16".into(),
            |c| c.cache_type_k = "q4_0".into(),
            |c| c.fit_mode = "on".into(),
            |c| c.gpu_layers = 20,
            |c| c.draft_gpu_layers = 0,
            |c| c.swa_full = true,
            |c| c.kv_unified = false,
            |c| c.kv_unified_mode = "off".into(),
            |c| c.spec_type = "draft-eagle3".into(),
            |c| c.explicit_overrides = Some(vec!["spec_type".into()]),
        ];
        for mutate in mutations {
            let mut altered = config.clone();
            mutate(&mut altered);
            assert!(!reviewed_windowed_swa(&altered, &caps, pair));
        }
        for mutate in [
            (|c: &mut EngineCapabilities| c.persistence_profile = None)
                as fn(&mut EngineCapabilities),
            |c| c.persistence_revision -= 1,
            |c| c.speculative_types.clear(),
            |c| c.checkpoint_draft_types.clear(),
            |c| c.persistence_profile = Some("windows-rocm-b11838-sckp1-sdft1".into()),
        ] {
            let mut altered = caps.clone();
            mutate(&mut altered);
            assert!(!reviewed_windowed_swa(&config, &altered, pair));
        }
        assert_eq!(
            persistence_runtime_matches(&caps, &profile.artifact_sha256),
            cfg!(windows)
        );
        assert!(!persistence_runtime_matches(
            &caps,
            profile.base_artifact_sha256.as_deref().unwrap()
        ));
        assert!(!persistence_runtime_matches(
            &caps,
            "changed-during-model-hash"
        ));
    }

    fn detected(flags: &[&str]) -> EngineCapabilities {
        EngineCapabilities {
            status: "detected".to_string(),
            supported_flags: flags.iter().map(|flag| (*flag).to_string()).collect(),
            ..EngineCapabilities::default()
        }
    }

    #[test]
    fn extracts_short_long_and_negative_flags_without_numeric_values() {
        let flags = extract_supported_flags(
            "  -m, --model FNAME\n  --port PORT\n  --no-warmup\n range -1 and value=-0.5",
        );
        assert!(flags.contains(&"-m".to_string()));
        assert!(flags.contains(&"--model".to_string()));
        assert!(flags.contains(&"--no-warmup".to_string()));
        assert!(!flags.contains(&"-1".to_string()));
    }

    #[test]
    fn extracts_reported_defaults_from_single_and_multiline_help_blocks() {
        let defaults = extract_reported_defaults(
            r#"  -t, --threads N              number of threads
                                  (default: -1)
  --temp N                     sampling temperature (default: 0.8)
  --models-autoload            load models on demand
                                  (default: enabled)
                                  --not-an-option-line (default: ignored)"#,
        );

        assert_eq!(defaults.get("-t").map(String::as_str), Some("-1"));
        assert_eq!(defaults.get("--threads").map(String::as_str), Some("-1"));
        assert_eq!(defaults.get("--temp").map(String::as_str), Some("0.8"));
        assert_eq!(
            defaults.get("--models-autoload").map(String::as_str),
            Some("enabled")
        );
        assert!(!defaults.contains_key("--not-an-option-line"));
    }

    #[test]
    fn extracts_wrapped_speculative_type_choices_in_engine_order() {
        let types = extract_speculative_types(
            "--spec-type none,draft-simple,draft-eagle3,draft-mtp,draft-dflash,draft-dspark,ngram-simple,ngram-map-k,ngram-map-k4v\n,ngram-mod,ngram-cache\n    comma-separated list of types of speculative decoding to use (default: none)\n",
        );
        assert_eq!(
            types,
            vec![
                "none",
                "draft-simple",
                "draft-eagle3",
                "draft-mtp",
                "draft-dflash",
                "draft-dspark",
                "ngram-simple",
                "ngram-map-k",
                "ngram-map-k4v",
                "ngram-mod",
                "ngram-cache",
            ]
        );
        assert!(extract_speculative_types("--model FILE\n").is_empty());
    }

    #[test]
    fn multi_slot_scope_requires_exact_models_topology_and_emitted_shape() {
        for profile in profiles() {
            let caps = EngineCapabilities {
                persistence_profile: Some(profile.id.clone()),
                persistence_revision: PERSISTENCE_REVISION,
                checkpoint_draft_types: vec!["draft-mtp".into()],
                speculative_types: vec!["draft-mtp".into()],
                ..Default::default()
            };
            for scope in profile
                .model_scopes
                .iter()
                .filter(|s| s.parallel.iter().any(|n| *n > 1))
            {
                let cfg = crate::models::InstanceConfig {
                    spec_type: "draft-mtp".into(),
                    draft_tokens: 3,
                    parallel: 4,
                    ctx_checkpoints: 2,
                    ctx_size: 16384,
                    ctx_size_auto: false,
                    kv_unified: true,
                    gpu_layers: 99,
                    gpu_layers_auto: false,
                    draft_gpu_layers: 99,
                    flash_attn: "on".into(),
                    fit_mode: "off".into(),
                    ..Default::default()
                };
                let pair = Some((scope.model_sha256.as_str(), scope.draft_sha256.as_str()));
                assert_eq!(reviewed_multi_slot(&cfg, &caps, pair), cfg!(windows));
                assert!(!reviewed_multi_slot(&cfg, &caps, None));
                assert!(!reviewed_multi_slot(
                    &cfg,
                    &caps,
                    Some(("unknown", &scope.draft_sha256))
                ));
                for mutate in [
                    |c: &mut crate::models::InstanceConfig| c.parallel = 1,
                    |c: &mut crate::models::InstanceConfig| c.parallel = 3,
                    |c: &mut crate::models::InstanceConfig| c.ctx_size = 8192,
                    |c: &mut crate::models::InstanceConfig| c.ctx_checkpoints = 0,
                    |c: &mut crate::models::InstanceConfig| c.kv_unified = false,
                    |c: &mut crate::models::InstanceConfig| c.cache_type_draft_k = "q8_0".into(),
                    |c: &mut crate::models::InstanceConfig| {
                        c.explicit_overrides = Some(vec!["spec_type".into()])
                    },
                ] {
                    let mut changed = cfg.clone();
                    mutate(&mut changed);
                    assert!(!reviewed_multi_slot(&changed, &caps, pair));
                }
            }
        }
    }

    #[test]
    fn previous_persistence_revision_requires_reprobe_even_for_unchanged_executable() {
        let executable = std::env::current_exe().unwrap();
        let executable = executable.to_str().unwrap();
        let mut caps = EngineCapabilities {
            persistence_revision: PERSISTENCE_REVISION,
            executable_fingerprint: executable_fingerprint(executable),
            ..Default::default()
        };
        assert!(capabilities_match_executable(executable, &caps));
        caps.persistence_revision -= 1;
        assert!(!capabilities_match_executable(executable, &caps));
    }

    #[test]
    fn detects_only_the_documented_context_checkpoint_capability_marker() {
        assert!(detects_context_checkpoint_persistence(
            "  --slot-save-path PATH\n      path to save slot KV cache and context checkpoints (default: disabled)\n"
        ));
        assert!(!detects_context_checkpoint_persistence(
            "  --slot-save-path PATH\n      path to save slot kv cache (default: disabled)\n"
        ));
    }

    #[test]
    fn version_extraction_skips_backend_logs_and_requires_a_version_marker() {
        let output = "load_backend: loaded RPC backend\nversion: 9055 (8e52631d5)\nbuilt with MSVC";
        assert_eq!(
            extract_engine_version(output).as_deref(),
            Some("version: 9055 (8e52631d5)")
        );
        assert_eq!(
            extract_engine_version("ggml_cuda_init: found 1 device"),
            None
        );
        assert_eq!(extract_engine_version("version:"), None);
        assert_eq!(
            extract_engine_version("llama-server version 1.2.3").as_deref(),
            Some("llama-server version 1.2.3")
        );
        assert_eq!(
            extract_engine_version(
                "version: 0.2.0 (build 10566, commit bb4caa754)\nbuilt with Clang 23.0.0 for Windows AMD64"
            )
            .as_deref(),
            Some("version: 0.2.0 (build 10566, commit bb4caa754)")
        );
    }

    #[test]
    fn validates_only_detected_capabilities_and_deduplicates_flags() {
        let command = vec![
            "llama-server".to_string(),
            "-m".to_string(),
            "model.gguf".to_string(),
            "--temp".to_string(),
            "-1".to_string(),
            "--future=value".to_string(),
            "--future".to_string(),
        ];
        assert_eq!(
            unsupported_command_flags(&command, &detected(&["-m", "--temp"])),
            vec!["--future".to_string()]
        );

        let mut unknown = detected(&["-m"]);
        unknown.status = "partial".to_string();
        assert!(unsupported_command_flags(&command, &unknown).is_empty());
    }

    #[test]
    fn validates_speculative_type_values_reported_by_the_engine() {
        let command = vec![
            "llama-server".to_string(),
            "--spec-type".to_string(),
            "ngram-mod,ngram-future,draft-mtp".to_string(),
        ];
        let mut capabilities = detected(&["--spec-type"]);
        capabilities.speculative_types = vec!["ngram-mod".into(), "draft-mtp".into()];
        assert_eq!(
            unsupported_command_flags(&command, &capabilities),
            vec!["--spec-type=ngram-future".to_string()]
        );

        capabilities.speculative_types.clear();
        assert!(unsupported_command_flags(&command, &capabilities).is_empty());
    }

    #[test]
    fn conservative_projection_keeps_only_recognized_and_essential_parameters() {
        let command = vec![
            "llama-server".to_string(),
            "-m".to_string(),
            "model.gguf".to_string(),
            "-c".to_string(),
            "8192".to_string(),
            "--temp".to_string(),
            "-1".to_string(),
            "--future=value".to_string(),
            "--host".to_string(),
            "127.0.0.1".to_string(),
            "--port".to_string(),
            "8080".to_string(),
        ];
        let partial = EngineCapabilities {
            status: "partial".to_string(),
            supported_flags: vec!["-c".to_string()],
            ..EngineCapabilities::default()
        };
        assert_eq!(
            command_for_capabilities(&command, Some(&partial)),
            vec![
                "llama-server",
                "-m",
                "model.gguf",
                "-c",
                "8192",
                "--host",
                "127.0.0.1",
                "--port",
                "8080",
            ]
        );
    }

    #[test]
    fn projection_keeps_hyphen_prefixed_values_attached_to_known_flags() {
        let command = vec![
            "llama-server".to_string(),
            "-m".to_string(),
            "model.gguf".to_string(),
            "--api-key".to_string(),
            "-secret value".to_string(),
            "--temp".to_string(),
            "0.7".to_string(),
        ];

        assert_eq!(
            command_for_capabilities(&command, None),
            vec![
                "llama-server",
                "-m",
                "model.gguf",
                "--api-key",
                "-secret value"
            ]
        );
    }

    #[test]
    fn conservative_mode_blocks_unverified_cors_policy() {
        let command = vec![
            "llama-server".to_string(),
            "-m".to_string(),
            "model.gguf".to_string(),
            "--cors-origins".to_string(),
            "https://example.test".to_string(),
            "--no-cors-credentials".to_string(),
        ];
        assert_eq!(
            blocked_security_flags(&command, None),
            vec!["--cors-origins", "--no-cors-credentials"]
        );

        let partial = EngineCapabilities {
            status: "partial".to_string(),
            supported_flags: vec![
                "--cors-origins".to_string(),
                "--no-cors-credentials".to_string(),
            ],
            ..EngineCapabilities::default()
        };
        assert!(blocked_security_flags(&command, Some(&partial)).is_empty());
    }

    #[test]
    fn executable_fingerprint_covers_the_launcher_and_adjacent_runtime_libraries() {
        let directory = std::env::temp_dir().join(format!(
            "lsm-engine-fingerprint-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("llama-server");
        let library = directory.join("llama-server-impl.dll");
        std::fs::write(&path, vec![b'a'; 128 * 1024]).unwrap();
        std::fs::write(&library, vec![b'c'; 128 * 1024]).unwrap();
        let first = executable_fingerprint(&path.to_string_lossy());
        std::fs::write(&path, vec![b'b'; 128 * 1024]).unwrap();
        let second = executable_fingerprint(&path.to_string_lossy());
        assert_ne!(first, second);

        std::fs::write(&library, vec![b'd'; 128 * 1024]).unwrap();
        let third = executable_fingerprint(&path.to_string_lossy());
        assert_ne!(second, third);

        std::fs::write(directory.join("release-notes.txt"), b"not runtime code").unwrap();
        assert_eq!(third, executable_fingerprint(&path.to_string_lossy()));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn minimal_projection_preserves_vector_mode_and_security_but_not_tuning() {
        let command = vec![
            "llama-server".to_string(),
            "-m".to_string(),
            "embedding.gguf".to_string(),
            "-b".to_string(),
            "2048".to_string(),
            "--embedding".to_string(),
            "--pooling".to_string(),
            "rank".to_string(),
            "--reranking".to_string(),
            "--api-key".to_string(),
            "secret".to_string(),
        ];
        assert_eq!(
            command_for_capabilities(&command, None),
            vec![
                "llama-server",
                "-m",
                "embedding.gguf",
                "--embedding",
                "--pooling",
                "rank",
                "--reranking",
                "--api-key",
                "secret",
            ]
        );
    }

    #[test]
    fn capped_reader_drains_input_but_retains_only_the_limit() {
        let input = vec![b'x'; MAX_PROBE_STREAM_BYTES + 8 * 1024];
        assert_eq!(
            read_stream_capped(std::io::Cursor::new(input)).len(),
            MAX_PROBE_STREAM_BYTES
        );
    }

    #[test]
    fn old_and_forked_help_outputs_remain_non_blocking_until_detection_is_confident() {
        let partial = vec!["-m".to_string(), "--port".to_string()];
        assert_eq!(classify_probe_status(&partial, false), "partial");
        assert_eq!(classify_probe_status(&[], false), "failed");
        assert_eq!(classify_probe_status(&partial, true), "timeout");

        let detected = [
            "-m",
            "--model",
            "--host",
            "--port",
            "-c",
            "-ngl",
            "-t",
            "-b",
            "-ub",
            "--metrics",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        assert_eq!(classify_probe_status(&detected, false), "detected");
    }
}
