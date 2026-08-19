use crate::commands::model_inventory;
use crate::models::{
    AppState, EngineCapabilities, EngineInfo, EngineQualificationCheck, EngineQualificationReport,
    ModelInfo,
};
use crate::path_utils::{path_identity_key, paths_equal};
use std::collections::{BTreeSet, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROBE_STREAM_BYTES: usize = 512 * 1024;
const MIN_CONFIDENT_FLAG_COUNT: usize = 10;
const REPORTED_DEFAULTS_VERSION: u8 = 1;
pub(crate) const QUALIFICATION_PROFILE_VERSION: u8 = 1;
const QUALIFICATION_STARTUP_TIMEOUT: Duration = Duration::from_secs(180);
const QUALIFICATION_HEALTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const QUALIFICATION_INFERENCE_TIMEOUT: Duration = Duration::from_secs(60);
const QUALIFICATION_POLL_INTERVAL: Duration = Duration::from_millis(250);
const QUALIFICATION_PROMPT: &str = "LSM qualification probe.";
const MAX_QUALIFICATION_DIAGNOSTIC_CHARS: usize = 2_000;

static ACTIVE_QUALIFICATIONS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn active_qualifications() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    ACTIVE_QUALIFICATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

struct QualificationReservation {
    key: String,
    cancelled: Arc<AtomicBool>,
}

impl QualificationReservation {
    fn reserve(engine_id: &str) -> Result<Self, String> {
        let key = path_identity_key(std::path::Path::new(engine_id));
        let mut active = active_qualifications().lock().unwrap();
        if active.contains_key(&key) {
            return Err("engine qualification is already running".to_string());
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        active.insert(key.clone(), cancelled.clone());
        Ok(Self { key, cancelled })
    }
}

impl Drop for QualificationReservation {
    fn drop(&mut self) {
        active_qualifications().lock().unwrap().remove(&self.key);
    }
}

#[derive(Debug)]
struct QualificationLaunch {
    executable: String,
    arguments: Vec<String>,
    port: u16,
    environment: Vec<(String, String)>,
    startup_timeout: Duration,
    health_request_timeout: Duration,
    inference_timeout: Duration,
    poll_interval: Duration,
}

#[derive(Debug)]
struct RuntimeQualificationResult {
    status: String,
    checks: Vec<EngineQualificationCheck>,
    diagnostic: Option<String>,
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
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
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
    let mut command = Command::new(executable);
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
    let metadata = match path.metadata() {
        Ok(metadata) if metadata.is_file() => metadata,
        _ => return String::new(),
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let normalized_path = path_identity_key(&path);

    let mut hash = 0xcbf29ce484222325_u64;
    update_fingerprint_hash(&mut hash, normalized_path.as_bytes());
    update_fingerprint_hash(&mut hash, &metadata.len().to_le_bytes());
    update_fingerprint_hash(&mut hash, &modified.to_le_bytes());

    let mut file = match File::open(&path) {
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

    format!(
        "v2:{normalized_path}:{}:{modified}:{hash:016x}",
        metadata.len()
    )
}

pub(crate) fn capabilities_match_executable(
    executable: &str,
    capabilities: &EngineCapabilities,
) -> bool {
    !capabilities.executable_fingerprint.is_empty()
        && capabilities.executable_fingerprint == executable_fingerprint(executable)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(crate) fn stale_engine_qualification(
    mut qualification: EngineQualificationReport,
    reason: impl Into<String>,
) -> EngineQualificationReport {
    if qualification.status != "unqualified" {
        qualification.status = "stale".to_string();
        qualification.invalidated_at = Some(now_secs());
        qualification.diagnostic = Some(compact_error(reason));
    }
    let _ = crate::deployment_identity::seal_qualification_report(&mut qualification);
    qualification
}

pub(crate) fn invalidate_engine_evidence(engine: &mut EngineInfo, reason: impl Into<String>) {
    let reason = compact_error(reason);
    let qualification = stale_engine_qualification(
        std::mem::take(&mut engine.capabilities.qualification),
        reason.clone(),
    );
    engine.version.clear();
    engine.capabilities = EngineCapabilities {
        error: Some(reason),
        qualification,
        ..EngineCapabilities::default()
    };
}

pub(crate) fn qualification_matches_executable(
    executable: &str,
    qualification: &EngineQualificationReport,
) -> bool {
    qualification_report_is_complete(qualification)
        && !qualification.executable_fingerprint.is_empty()
        && qualification.executable_fingerprint == executable_fingerprint(executable)
        && crate::deployment_identity::artifact_identity_for_path(
            "engine",
            std::path::Path::new(executable),
        )
        .is_ok_and(|identity| identity.artifact_id == qualification.engine_artifact_id)
}

fn qualification_report_is_complete(qualification: &EngineQualificationReport) -> bool {
    const REQUIRED_CHECKS: [&str; 5] =
        ["version", "capabilities", "startup", "health", "inference"];

    qualification.schema_version == 2
        && qualification.profile_version == QUALIFICATION_PROFILE_VERSION
        && qualification.status == "passed"
        && !qualification.engine_version.trim().is_empty()
        && !qualification.help_hash.is_empty()
        && !qualification.model_id.is_empty()
        && !qualification.model_name.is_empty()
        && qualification.model_size > 0
        && qualification.started_at.is_some()
        && qualification.completed_at.is_some()
        && crate::deployment_identity::qualification_evidence_valid(qualification)
        && REQUIRED_CHECKS.iter().all(|required| {
            qualification
                .checks
                .iter()
                .any(|check| check.name == *required && check.status == "passed")
        })
}

fn qualification_after_capability_probe(
    qualification: EngineQualificationReport,
    executable_fingerprint: &str,
    engine_version: &str,
    current_help_hash: &str,
) -> EngineQualificationReport {
    if qualification.status == "unqualified"
        || (qualification.executable_fingerprint == executable_fingerprint
            && qualification.engine_version == engine_version
            && qualification.help_hash == current_help_hash)
    {
        qualification
    } else {
        stale_engine_qualification(
            qualification,
            "engine version or capability evidence changed; qualification required",
        )
    }
}

fn qualification_duration_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn qualification_check(
    name: &str,
    status: &str,
    duration_ms: u64,
    detail: impl Into<Option<String>>,
) -> EngineQualificationCheck {
    EngineQualificationCheck {
        name: name.to_string(),
        status: status.to_string(),
        duration_ms,
        detail: detail.into().map(compact_error),
    }
}

fn skipped_qualification_check(name: &str, detail: &str) -> EngineQualificationCheck {
    qualification_check(name, "skipped", 0, Some(detail.to_string()))
}

fn bounded_qualification_diagnostic(message: impl Into<String>) -> Option<String> {
    let normalized = message
        .into()
        .replace(QUALIFICATION_PROMPT, "[qualification prompt]")
        .chars()
        .filter(|character| !character.is_control() || character.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        None
    } else {
        Some(
            normalized
                .chars()
                .take(MAX_QUALIFICATION_DIAGNOSTIC_CHARS)
                .collect(),
        )
    }
}

fn supported_qualification_flag<'a>(
    capabilities: &'a EngineCapabilities,
    candidates: &[&'a str],
) -> Option<&'a str> {
    candidates.iter().copied().find(|candidate| {
        capabilities
            .supported_flags
            .iter()
            .any(|flag| flag == candidate)
    })
}

fn reserve_qualification_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("cannot reserve qualification port: {error}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("cannot inspect qualification port: {error}"))
}

fn qualification_arguments(
    capabilities: &EngineCapabilities,
    model_path: &std::path::Path,
    port: u16,
) -> Result<Vec<String>, String> {
    let model_flag = supported_qualification_flag(capabilities, &["--model", "-m"])
        .ok_or_else(|| "engine help did not confirm the model flag".to_string())?;
    let host_flag = supported_qualification_flag(capabilities, &["--host"])
        .ok_or_else(|| "engine help did not confirm the host flag".to_string())?;
    let port_flag = supported_qualification_flag(capabilities, &["--port"])
        .ok_or_else(|| "engine help did not confirm the port flag".to_string())?;

    let mut arguments = vec![
        model_flag.to_string(),
        model_path.to_string_lossy().to_string(),
        host_flag.to_string(),
        "127.0.0.1".to_string(),
        port_flag.to_string(),
        port.to_string(),
    ];
    for (candidates, value) in [
        (&["--ctx-size", "-c"][..], Some("512")),
        (&["--threads", "-t"][..], Some("2")),
        (&["--n-gpu-layers", "-ngl"][..], Some("0")),
        (&["--no-ui"][..], None),
        (&["--offline"][..], None),
        (&["--log-disable"][..], None),
    ] {
        if let Some(flag) = supported_qualification_flag(capabilities, candidates) {
            arguments.push(flag.to_string());
            if let Some(value) = value {
                arguments.push(value.to_string());
            }
        }
    }
    Ok(arguments)
}

fn eligible_qualification_model(model: &ModelInfo) -> bool {
    model.file_type == "model"
        && !model.is_shard
        && !model.capabilities.is_mmproj
        && model.capabilities.is_embedding_model != Some(true)
        && model.capabilities.is_reranker_model != Some(true)
}

fn run_runtime_qualification(
    launch: QualificationLaunch,
    cancelled: Arc<AtomicBool>,
) -> RuntimeQualificationResult {
    let startup_started = Instant::now();
    let (output_path, mut output_file) = match probe_output_file() {
        Ok(output) => output,
        Err(error) => {
            return RuntimeQualificationResult {
                status: "failed".to_string(),
                checks: vec![
                    qualification_check("startup", "failed", 0, Some(error.clone())),
                    skipped_qualification_check("health", "startup failed"),
                    skipped_qualification_check("inference", "startup failed"),
                ],
                diagnostic: bounded_qualification_diagnostic(error),
            }
        }
    };
    let stdout_file = match output_file.try_clone() {
        Ok(file) => file,
        Err(error) => {
            drop(output_file);
            let _ = std::fs::remove_file(output_path);
            let message = format!("cannot clone qualification output handle: {error}");
            return RuntimeQualificationResult {
                status: "failed".to_string(),
                checks: vec![
                    qualification_check("startup", "failed", 0, Some(message.clone())),
                    skipped_qualification_check("health", "startup failed"),
                    skipped_qualification_check("inference", "startup failed"),
                ],
                diagnostic: bounded_qualification_diagnostic(message),
            };
        }
    };
    let stderr_file = match output_file.try_clone() {
        Ok(file) => file,
        Err(error) => {
            drop(stdout_file);
            drop(output_file);
            let _ = std::fs::remove_file(output_path);
            let message = format!("cannot clone qualification error handle: {error}");
            return RuntimeQualificationResult {
                status: "failed".to_string(),
                checks: vec![
                    qualification_check("startup", "failed", 0, Some(message.clone())),
                    skipped_qualification_check("health", "startup failed"),
                    skipped_qualification_check("inference", "startup failed"),
                ],
                diagnostic: bounded_qualification_diagnostic(message),
            };
        }
    };

    let mut command = Command::new(&launch.executable);
    command
        .args(&launch.arguments)
        .envs(launch.environment.iter().cloned())
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
            let message = format!("cannot start qualification server: {error}");
            return RuntimeQualificationResult {
                status: "failed".to_string(),
                checks: vec![
                    qualification_check(
                        "startup",
                        "failed",
                        qualification_duration_ms(startup_started),
                        Some(message.clone()),
                    ),
                    skipped_qualification_check("health", "startup failed"),
                    skipped_qualification_check("inference", "startup failed"),
                ],
                diagnostic: bounded_qualification_diagnostic(message),
            };
        }
    };
    drop(command);

    let health_client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(launch.health_request_timeout)
        .build();
    let health_started = Instant::now();
    let mut startup_check = None;
    let health_check: Option<EngineQualificationCheck>;
    let inference_check: Option<EngineQualificationCheck>;
    let mut status = "failed".to_string();
    let mut terminal_reason = None;
    let mut last_health_error: Option<String>;
    let health_url = format!("http://127.0.0.1:{}/health", launch.port);

    match health_client {
        Err(error) => {
            let message = format!("cannot create qualification health client: {error}");
            startup_check = Some(qualification_check(
                "startup",
                "failed",
                qualification_duration_ms(startup_started),
                Some(message.clone()),
            ));
            health_check = Some(skipped_qualification_check("health", "startup failed"));
            terminal_reason = Some(message);
        }
        Ok(client) => loop {
            if cancelled.load(Ordering::SeqCst) {
                status = "cancelled".to_string();
                if startup_check.is_none() {
                    startup_check = Some(qualification_check(
                        "startup",
                        "cancelled",
                        qualification_duration_ms(startup_started),
                        Some("operator cancelled qualification".to_string()),
                    ));
                }
                health_check = Some(qualification_check(
                    "health",
                    "cancelled",
                    qualification_duration_ms(health_started),
                    Some("operator cancelled qualification".to_string()),
                ));
                terminal_reason = Some("operator cancelled qualification".to_string());
                break;
            }
            match child.try_wait() {
                Ok(Some(exit)) => {
                    let message =
                        format!("qualification server exited before becoming healthy: {exit}");
                    if startup_check.is_none() {
                        startup_check = Some(qualification_check(
                            "startup",
                            "failed",
                            qualification_duration_ms(startup_started),
                            Some(message.clone()),
                        ));
                        health_check =
                            Some(skipped_qualification_check("health", "startup failed"));
                    } else {
                        health_check = Some(qualification_check(
                            "health",
                            "failed",
                            qualification_duration_ms(health_started),
                            Some(message.clone()),
                        ));
                    }
                    terminal_reason = Some(message);
                    break;
                }
                Err(error) => {
                    let message = format!("cannot inspect qualification server: {error}");
                    if startup_check.is_none() {
                        startup_check = Some(qualification_check(
                            "startup",
                            "failed",
                            qualification_duration_ms(startup_started),
                            Some(message.clone()),
                        ));
                        health_check =
                            Some(skipped_qualification_check("health", "startup failed"));
                    } else {
                        health_check = Some(qualification_check(
                            "health",
                            "failed",
                            qualification_duration_ms(health_started),
                            Some(message.clone()),
                        ));
                    }
                    terminal_reason = Some(message);
                    break;
                }
                Ok(None) => {}
            }
            if startup_started.elapsed() >= Duration::from_millis(500) && startup_check.is_none() {
                startup_check = Some(qualification_check(
                    "startup",
                    "passed",
                    qualification_duration_ms(startup_started),
                    Some("qualification server remained running".to_string()),
                ));
            }
            match client.get(&health_url).send() {
                Ok(response) if response.status().is_success() => {
                    if startup_check.is_none() {
                        startup_check = Some(qualification_check(
                            "startup",
                            "passed",
                            qualification_duration_ms(startup_started),
                            Some("qualification server accepted a health request".to_string()),
                        ));
                    }
                    health_check = Some(qualification_check(
                        "health",
                        "passed",
                        qualification_duration_ms(health_started),
                        Some(format!("GET /health returned HTTP {}", response.status())),
                    ));
                    break;
                }
                Ok(response) => {
                    last_health_error =
                        Some(format!("GET /health returned HTTP {}", response.status()));
                }
                Err(error) => {
                    last_health_error = Some(format!("GET /health failed: {error}"));
                }
            }
            if health_started.elapsed() >= launch.startup_timeout {
                let message = last_health_error
                    .clone()
                    .unwrap_or_else(|| "qualification health check timed out".to_string());
                if startup_check.is_none() {
                    startup_check = Some(qualification_check(
                        "startup",
                        "failed",
                        qualification_duration_ms(startup_started),
                        Some("qualification server did not remain available".to_string()),
                    ));
                }
                health_check = Some(qualification_check(
                    "health",
                    "failed",
                    qualification_duration_ms(health_started),
                    Some(message.clone()),
                ));
                terminal_reason = Some(message);
                break;
            }
            thread::sleep(launch.poll_interval);
        },
    }

    let health_passed = health_check
        .as_ref()
        .is_some_and(|check| check.status == "passed");
    if health_passed {
        let inference_started = Instant::now();
        if cancelled.load(Ordering::SeqCst) {
            status = "cancelled".to_string();
            inference_check = Some(qualification_check(
                "inference",
                "cancelled",
                0,
                Some("operator cancelled qualification".to_string()),
            ));
            terminal_reason = Some("operator cancelled qualification".to_string());
        } else {
            let inference_url = format!("http://127.0.0.1:{}/completion", launch.port);
            let inference_client = reqwest::blocking::Client::builder()
                .no_proxy()
                .timeout(launch.inference_timeout)
                .build();
            match inference_client.and_then(|client| {
                client
                    .post(inference_url)
                    .json(&serde_json::json!({
                        "prompt": QUALIFICATION_PROMPT,
                        "n_predict": 4,
                        "temperature": 0,
                        "cache_prompt": false,
                    }))
                    .send()
            }) {
                Ok(response) if response.status().is_success() => {
                    let http_status = response.status();
                    match response.json::<serde_json::Value>() {
                        Ok(payload)
                            if payload
                                .get("tokens_predicted")
                                .and_then(serde_json::Value::as_u64)
                                .unwrap_or(0)
                                > 0
                                || payload
                                    .get("content")
                                    .and_then(serde_json::Value::as_str)
                                    .is_some_and(|content| !content.is_empty()) =>
                        {
                            status = "passed".to_string();
                            inference_check = Some(qualification_check(
                                "inference",
                                "passed",
                                qualification_duration_ms(inference_started),
                                Some(format!(
                                    "POST /completion returned HTTP {http_status} with predicted output"
                                )),
                            ));
                        }
                        Ok(_) => {
                            let message =
                                "POST /completion returned no predicted output".to_string();
                            inference_check = Some(qualification_check(
                                "inference",
                                "failed",
                                qualification_duration_ms(inference_started),
                                Some(message.clone()),
                            ));
                            terminal_reason = Some(message);
                        }
                        Err(error) => {
                            let message =
                                format!("POST /completion returned invalid JSON evidence: {error}");
                            inference_check = Some(qualification_check(
                                "inference",
                                "failed",
                                qualification_duration_ms(inference_started),
                                Some(message.clone()),
                            ));
                            terminal_reason = Some(message);
                        }
                    }
                }
                Ok(response) => {
                    let message = format!("POST /completion returned HTTP {}", response.status());
                    inference_check = Some(qualification_check(
                        "inference",
                        "failed",
                        qualification_duration_ms(inference_started),
                        Some(message.clone()),
                    ));
                    terminal_reason = Some(message);
                }
                Err(error) => {
                    let message = format!("POST /completion failed: {error}");
                    inference_check = Some(qualification_check(
                        "inference",
                        "failed",
                        qualification_duration_ms(inference_started),
                        Some(message.clone()),
                    ));
                    terminal_reason = Some(message);
                }
            }
        }
    } else {
        inference_check = Some(skipped_qualification_check(
            "inference",
            "health check did not pass",
        ));
    }

    let cleanup_error = match child.try_wait() {
        Ok(Some(_)) => child.wait().err().map(|error| error.to_string()),
        _ => terminate_probe_process_tree(&mut child),
    };
    if let Some(error) = cleanup_error {
        status = "failed".to_string();
        terminal_reason = Some(format!("qualification server cleanup failed: {error}"));
    }

    let _ = output_file.seek(SeekFrom::Start(0));
    let output = String::from_utf8_lossy(&read_stream_capped(&mut output_file)).into_owned();
    drop(output_file);
    let _ = std::fs::remove_file(output_path);
    let diagnostic = if status == "passed" {
        None
    } else {
        bounded_qualification_diagnostic(match (terminal_reason, output.is_empty()) {
            (Some(reason), false) => format!("{reason}; {output}"),
            (Some(reason), true) => reason,
            (None, false) => output,
            (None, true) => "qualification failed without diagnostic output".to_string(),
        })
    };

    RuntimeQualificationResult {
        status,
        checks: vec![
            startup_check.unwrap_or_else(|| {
                skipped_qualification_check("startup", "qualification did not start")
            }),
            health_check.unwrap_or_else(|| {
                skipped_qualification_check("health", "qualification did not start")
            }),
            inference_check.unwrap_or_else(|| {
                skipped_qualification_check("inference", "qualification did not start")
            }),
        ],
        diagnostic,
    }
}

fn probe_engine(mut engine: EngineInfo) -> EngineInfo {
    let previous_qualification = std::mem::take(&mut engine.capabilities.qualification);
    let fingerprint_before = executable_fingerprint(&engine.exe);
    let version_output = run_bounded(&engine.exe, "--version");
    let help_output = run_bounded(&engine.exe, "--help");
    let supported_flags = extract_supported_flags(&help_output.text);
    let reported_defaults = extract_reported_defaults(&help_output.text);
    let status = classify_probe_status(&supported_flags, help_output.timed_out);
    let fingerprint = executable_fingerprint(&engine.exe);
    if fingerprint_before.is_empty() || fingerprint_before != fingerprint {
        engine.capabilities.qualification = previous_qualification;
        invalidate_engine_evidence(
            &mut engine,
            "engine executable changed while compatibility probing was in progress; probe again",
        );
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
    let current_help_hash = if help_output.text.is_empty() {
        String::new()
    } else {
        help_hash(&help_output.text)
    };
    let qualification = qualification_after_capability_probe(
        previous_qualification,
        &fingerprint,
        &engine.version,
        &current_help_hash,
    );
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
        help_hash: current_help_hash,
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
        qualification,
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
    command_argument_groups(command)
        .into_iter()
        .map(|group| group.flag)
        .filter(|flag| !supported.contains(flag))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
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

fn model_file_evidence(path: &std::path::Path) -> Result<(u64, Option<u64>), String> {
    let metadata = path
        .metadata()
        .map_err(|error| format!("cannot inspect qualification model: {error}"))?;
    if !metadata.is_file() {
        return Err("qualification model is not a file".to_string());
    }
    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());
    Ok((metadata.len(), modified_at))
}

fn persist_engine_qualification(
    state: &AppState,
    engine_id: &str,
    expected_executable: &str,
    mut qualification: EngineQualificationReport,
) -> Result<EngineInfo, String> {
    let mut engines = state.engines.lock().unwrap();
    let current = engines
        .iter_mut()
        .find(|engine| {
            paths_equal(
                std::path::Path::new(&engine.id),
                std::path::Path::new(engine_id),
            )
        })
        .ok_or_else(|| "engine was removed while qualification was in progress".to_string())?;
    if !paths_equal(
        std::path::Path::new(&current.exe),
        std::path::Path::new(expected_executable),
    ) {
        return Err("engine executable changed while qualification was in progress".to_string());
    }
    let current_fingerprint = executable_fingerprint(&current.exe);
    if current_fingerprint.is_empty() || current_fingerprint != qualification.executable_fingerprint
    {
        qualification = stale_engine_qualification(
            qualification,
            "engine executable changed while qualification was in progress",
        );
        invalidate_engine_evidence(
            current,
            "engine executable changed; compatibility probe and qualification required",
        );
    }
    match crate::deployment_identity::artifact_identity_for_path(
        "engine",
        std::path::Path::new(expected_executable),
    ) {
        Ok(identity) if identity.artifact_id == qualification.engine_artifact_id => {
            current.artifact_identity = identity;
        }
        Ok(_) | Err(_) => {
            qualification = stale_engine_qualification(
                qualification,
                "engine artifact identity changed while qualification was in progress",
            );
        }
    }
    crate::deployment_identity::seal_qualification_report(&mut qualification)?;
    current.capabilities.qualification = qualification;
    if let Err(error) = model_inventory::update_engine_probe(current) {
        current.capabilities.qualification.status = "failed".to_string();
        current.capabilities.qualification.diagnostic = bounded_qualification_diagnostic(format!(
            "qualification evidence was not persisted: {error}"
        ));
        let _ = crate::deployment_identity::seal_qualification_report(
            &mut current.capabilities.qualification,
        );
    }
    Ok(current.clone())
}

pub fn cancel_engine_qualification(engine_id: String) -> bool {
    let key = path_identity_key(std::path::Path::new(&engine_id));
    let active = active_qualifications().lock().unwrap();
    let Some(cancelled) = active.get(&key) else {
        return false;
    };
    cancelled.store(true, Ordering::SeqCst);
    true
}

pub async fn qualify_engine(
    engine_id: String,
    model_id: String,
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
    let model = state
        .models
        .lock()
        .unwrap()
        .iter()
        .find(|model| {
            paths_equal(
                std::path::Path::new(&model.id),
                std::path::Path::new(&model_id),
            ) || paths_equal(
                std::path::Path::new(&model.path),
                std::path::Path::new(&model_id),
            )
        })
        .cloned()
        .ok_or_else(|| "qualification model not found in the scanned inventory".to_string())?;
    if !eligible_qualification_model(&model) {
        return Err(
            "qualification requires a primary generative model, not a shard, projector, embedding, or reranker"
                .to_string(),
        );
    }

    let authorized_engine_root =
        crate::security::require_authorized_engine_root(std::path::Path::new(&engine.dir))?;
    let executable = crate::security::require_path_within_root(
        std::path::Path::new(&engine.exe),
        &authorized_engine_root,
    )?;
    let model_path =
        crate::security::require_authorized_model_path(std::path::Path::new(&model.path))?;
    let engine_artifact_identity =
        crate::deployment_identity::artifact_identity_for_path("engine", &executable)?;
    let model_artifact_identity =
        crate::deployment_identity::artifact_identity_for_path("model", &model_path)?;
    let (model_size, model_modified_at) = model_file_evidence(&model_path)?;
    let reservation = QualificationReservation::reserve(&engine.id)?;
    let started_at = now_secs();
    let probe_started = Instant::now();
    let probed = match probe_engine_capabilities(engine.id.clone(), state.clone()).await {
        Ok(probed) => probed,
        Err(error) => {
            let fingerprint = executable_fingerprint(&engine.exe);
            let mut report = EngineQualificationReport {
                profile_version: QUALIFICATION_PROFILE_VERSION,
                status: "incomplete".to_string(),
                executable_fingerprint: fingerprint,
                engine_artifact_id: engine_artifact_identity.artifact_id.clone(),
                model_id: model.id.clone(),
                model_artifact_id: model_artifact_identity.artifact_id.clone(),
                model_name: model.name.clone(),
                model_size,
                model_modified_at,
                started_at: Some(started_at),
                completed_at: Some(now_secs()),
                diagnostic: bounded_qualification_diagnostic(error.clone()),
                ..EngineQualificationReport::default()
            };
            report.checks = vec![
                qualification_check(
                    "version",
                    "failed",
                    qualification_duration_ms(probe_started),
                    Some(error.clone()),
                ),
                qualification_check(
                    "capabilities",
                    "failed",
                    qualification_duration_ms(probe_started),
                    Some(error),
                ),
                skipped_qualification_check("startup", "capability probe did not pass"),
                skipped_qualification_check("health", "capability probe did not pass"),
                skipped_qualification_check("inference", "capability probe did not pass"),
            ];
            return persist_engine_qualification(state.inner(), &engine.id, &engine.exe, report);
        }
    };
    let probe_duration_ms = qualification_duration_ms(probe_started);
    let version_passed =
        probed.capabilities.version_status == "detected" && !probed.version.trim().is_empty();
    let version_check = qualification_check(
        "version",
        if version_passed { "passed" } else { "failed" },
        probe_duration_ms,
        Some(if version_passed {
            "engine version was detected".to_string()
        } else {
            "engine version is unknown".to_string()
        }),
    );
    let capability_result = if probed.capabilities.status == "detected" {
        qualification_arguments(&probed.capabilities, &model_path, 1).map(|_| ())
    } else {
        Err(format!(
            "engine capability status is {}",
            probed.capabilities.status
        ))
    };
    let capabilities_passed = capability_result.is_ok();
    let capability_detail = capability_result
        .err()
        .unwrap_or_else(|| "required model, host, and port flags were confirmed".to_string());
    let capability_check = qualification_check(
        "capabilities",
        if capabilities_passed {
            "passed"
        } else {
            "failed"
        },
        probe_duration_ms,
        Some(capability_detail.clone()),
    );
    let mut report = EngineQualificationReport {
        profile_version: QUALIFICATION_PROFILE_VERSION,
        status: "incomplete".to_string(),
        executable_fingerprint: probed.capabilities.executable_fingerprint.clone(),
        engine_artifact_id: engine_artifact_identity.artifact_id.clone(),
        engine_version: probed.version.clone(),
        help_hash: probed.capabilities.help_hash.clone(),
        model_id: model.id.clone(),
        model_artifact_id: model_artifact_identity.artifact_id.clone(),
        model_name: model.name.clone(),
        model_size,
        model_modified_at,
        started_at: Some(started_at),
        checks: vec![version_check, capability_check],
        ..EngineQualificationReport::default()
    };
    if !version_passed || !capabilities_passed {
        report.checks.extend([
            skipped_qualification_check("startup", "capability evidence is incomplete"),
            skipped_qualification_check("health", "capability evidence is incomplete"),
            skipped_qualification_check("inference", "capability evidence is incomplete"),
        ]);
        report.completed_at = Some(now_secs());
        report.diagnostic = bounded_qualification_diagnostic(
            probed
                .capabilities
                .error
                .clone()
                .unwrap_or(capability_detail),
        );
        return persist_engine_qualification(state.inner(), &engine.id, &engine.exe, report);
    }
    if reservation.cancelled.load(Ordering::SeqCst) {
        report.status = "cancelled".to_string();
        report.checks.extend([
            qualification_check(
                "startup",
                "cancelled",
                0,
                Some("operator cancelled qualification".to_string()),
            ),
            skipped_qualification_check("health", "operator cancelled qualification"),
            skipped_qualification_check("inference", "operator cancelled qualification"),
        ]);
        report.completed_at = Some(now_secs());
        report.diagnostic = Some("operator cancelled qualification".to_string());
        return persist_engine_qualification(state.inner(), &engine.id, &engine.exe, report);
    }

    let port = match reserve_qualification_port() {
        Ok(port) => port,
        Err(error) => {
            report.status = "failed".to_string();
            report.checks.extend([
                qualification_check("startup", "failed", 0, Some(error.clone())),
                skipped_qualification_check("health", "startup failed"),
                skipped_qualification_check("inference", "startup failed"),
            ]);
            report.completed_at = Some(now_secs());
            report.diagnostic = bounded_qualification_diagnostic(error);
            return persist_engine_qualification(state.inner(), &engine.id, &engine.exe, report);
        }
    };
    let arguments = qualification_arguments(&probed.capabilities, &model_path, port)?;
    let launch = QualificationLaunch {
        executable: executable.to_string_lossy().to_string(),
        arguments,
        port,
        environment: Vec::new(),
        startup_timeout: QUALIFICATION_STARTUP_TIMEOUT,
        health_request_timeout: QUALIFICATION_HEALTH_REQUEST_TIMEOUT,
        inference_timeout: QUALIFICATION_INFERENCE_TIMEOUT,
        poll_interval: QUALIFICATION_POLL_INTERVAL,
    };
    let cancelled = reservation.cancelled.clone();
    let runtime = tokio::task::spawn_blocking(move || run_runtime_qualification(launch, cancelled))
        .await
        .map_err(|error| format!("engine qualification task failed: {error}"))?;
    report.status = runtime.status;
    report.checks.extend(runtime.checks);
    report.diagnostic = runtime.diagnostic;
    report.completed_at = Some(now_secs());

    let final_model_evidence = model_file_evidence(&model_path);
    let final_model_identity =
        crate::deployment_identity::artifact_identity_for_path("model", &model_path);
    if !matches!(
        final_model_evidence,
        Ok((final_size, final_modified))
            if final_size == model_size && final_modified == model_modified_at
    ) || !matches!(
        final_model_identity,
        Ok(ref identity) if identity.artifact_id == report.model_artifact_id
    ) {
        report.status = "failed".to_string();
        report.diagnostic = bounded_qualification_diagnostic(
            "qualification model changed while representative inference was in progress",
        );
    }
    persist_engine_qualification(state.inner(), &engine.id, &engine.exe, report)
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

    #[tauri::command]
    pub async fn qualify_engine(
        engine_id: String,
        model_id: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<EngineInfo> {
        super::qualify_engine(engine_id, model_id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub fn cancel_engine_qualification(engine_id: String) -> bool {
        super::cancel_engine_qualification(engine_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn detected(flags: &[&str]) -> EngineCapabilities {
        EngineCapabilities {
            status: "detected".to_string(),
            supported_flags: flags.iter().map(|flag| (*flag).to_string()).collect(),
            ..EngineCapabilities::default()
        }
    }

    fn qualification_fixture_launch(mode: &str, port: u16) -> QualificationLaunch {
        QualificationLaunch {
            executable: std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            arguments: vec![
                "qualification_http_fixture_process".to_string(),
                "--ignored".to_string(),
                "--test-threads=1".to_string(),
            ],
            port,
            environment: vec![
                (
                    "LSM_QUALIFICATION_FIXTURE_PORT".to_string(),
                    port.to_string(),
                ),
                (
                    "LSM_QUALIFICATION_FIXTURE_MODE".to_string(),
                    mode.to_string(),
                ),
            ],
            startup_timeout: Duration::from_secs(1),
            health_request_timeout: Duration::from_millis(100),
            inference_timeout: Duration::from_secs(1),
            poll_interval: Duration::from_millis(20),
        }
    }

    fn assert_port_released(port: u16) {
        for _ in 0..20 {
            if TcpListener::bind(("127.0.0.1", port)).is_ok() {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("qualification fixture port {port} was not released");
    }

    fn write_fixture_response(stream: &mut std::net::TcpStream, status: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    #[test]
    #[ignore]
    fn qualification_http_fixture_process() {
        let Ok(port) = std::env::var("LSM_QUALIFICATION_FIXTURE_PORT") else {
            return;
        };
        let port = port.parse::<u16>().unwrap();
        let mode = std::env::var("LSM_QUALIFICATION_FIXTURE_MODE")
            .unwrap_or_else(|_| "success".to_string());
        let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
        for incoming in listener.incoming() {
            let mut stream = incoming.unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut request = [0_u8; 8 * 1024];
            let count = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..count]);
            if request.starts_with("GET /health ") {
                if mode == "health-fail" {
                    write_fixture_response(
                        &mut stream,
                        "503 Service Unavailable",
                        r#"{"status":"loading"}"#,
                    );
                    continue;
                }
                write_fixture_response(&mut stream, "200 OK", r#"{"status":"ok"}"#);
                continue;
            }
            if request.starts_with("POST /completion ") {
                if mode == "inference-fail" {
                    write_fixture_response(
                        &mut stream,
                        "500 Internal Server Error",
                        r#"{"error":"fixture"}"#,
                    );
                } else {
                    write_fixture_response(
                        &mut stream,
                        "200 OK",
                        r#"{"content":"OK","tokens_predicted":1}"#,
                    );
                }
                break;
            }
            write_fixture_response(&mut stream, "404 Not Found", r#"{"error":"not found"}"#);
        }
    }

    #[test]
    #[ignore]
    fn qualification_exit_fixture_process() {}

    #[test]
    fn legacy_capabilities_default_to_an_unqualified_report() {
        let capabilities: EngineCapabilities = serde_json::from_str("{}").unwrap();
        assert_eq!(capabilities.qualification.status, "unqualified");
        assert_eq!(capabilities.qualification.schema_version, 2);
        assert_eq!(capabilities.qualification.profile_version, 1);
        assert!(capabilities.qualification.checks.is_empty());
    }

    #[test]
    fn stale_invalidation_preserves_completed_qualification_evidence() {
        let mut qualification = EngineQualificationReport {
            status: "passed".to_string(),
            executable_fingerprint: "old-fingerprint".to_string(),
            checks: vec![qualification_check("health", "passed", 25, None)],
            completed_at: Some(10),
            ..EngineQualificationReport::default()
        };
        qualification = stale_engine_qualification(qualification, "artifact changed");
        assert_eq!(qualification.status, "stale");
        assert_eq!(qualification.executable_fingerprint, "old-fingerprint");
        assert_eq!(qualification.checks.len(), 1);
        assert_eq!(qualification.completed_at, Some(10));
        assert!(qualification.invalidated_at.is_some());
    }

    #[test]
    fn qualification_evidence_is_bound_to_the_current_executable_artifact() {
        let path = std::env::temp_dir().join(format!(
            "lsm-qualified-engine-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, vec![b'a'; 128 * 1024]).unwrap();
        let engine_artifact_id =
            crate::deployment_identity::artifact_identity_for_path("engine", &path)
                .unwrap()
                .artifact_id;
        let mut qualification = EngineQualificationReport {
            status: "passed".to_string(),
            executable_fingerprint: executable_fingerprint(&path.to_string_lossy()),
            engine_artifact_id,
            engine_version: "version: 1".to_string(),
            help_hash: "help-hash".to_string(),
            model_id: "model-id".to_string(),
            model_artifact_id: "urn:lsm:model:v1:sha256:test".to_string(),
            model_name: "model.gguf".to_string(),
            model_size: 1024,
            started_at: Some(1),
            completed_at: Some(2),
            checks: ["version", "capabilities", "startup", "health", "inference"]
                .into_iter()
                .map(|name| qualification_check(name, "passed", 1, None))
                .collect(),
            ..EngineQualificationReport::default()
        };
        crate::deployment_identity::seal_qualification_report(&mut qualification).unwrap();
        assert!(qualification_matches_executable(
            &path.to_string_lossy(),
            &qualification
        ));

        std::fs::write(&path, vec![b'b'; 128 * 1024]).unwrap();
        assert!(!qualification_matches_executable(
            &path.to_string_lossy(),
            &qualification
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn passed_status_without_complete_current_profile_evidence_fails_closed() {
        let qualification = EngineQualificationReport {
            status: "passed".to_string(),
            executable_fingerprint: "fingerprint".to_string(),
            ..EngineQualificationReport::default()
        };
        assert!(!qualification_report_is_complete(&qualification));
    }

    #[test]
    fn qualification_diagnostics_redact_the_fixed_probe_prompt() {
        let diagnostic = bounded_qualification_diagnostic(format!(
            "server echoed sensitive request: {QUALIFICATION_PROMPT}"
        ))
        .unwrap();
        assert!(!diagnostic.contains(QUALIFICATION_PROMPT));
        assert!(diagnostic.contains("[qualification prompt]"));
    }

    #[test]
    fn qualification_profile_is_loopback_only_and_uses_a_cpu_baseline() {
        let capabilities = detected(&[
            "--model",
            "--host",
            "--port",
            "--ctx-size",
            "--threads",
            "--n-gpu-layers",
            "--offline",
            "--no-ui",
            "--log-disable",
        ]);
        let arguments =
            qualification_arguments(&capabilities, std::path::Path::new("model.gguf"), 18432)
                .unwrap();
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--host", "127.0.0.1"]));
        assert!(arguments.windows(2).any(|pair| pair == ["--port", "18432"]));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--n-gpu-layers", "0"]));
        assert!(arguments.contains(&"--offline".to_string()));
        assert!(arguments.contains(&"--log-disable".to_string()));
    }

    #[test]
    fn real_process_qualification_proves_health_and_inference() {
        let port = reserve_qualification_port().unwrap();
        let result = run_runtime_qualification(
            qualification_fixture_launch("success", port),
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(result.status, "passed", "{:?}", result.diagnostic);
        assert_eq!(
            result
                .checks
                .iter()
                .map(|check| check.status.as_str())
                .collect::<Vec<_>>(),
            vec!["passed", "passed", "passed"]
        );
        assert!(result.diagnostic.is_none());
        assert_port_released(port);
    }

    #[test]
    fn health_timeout_fails_and_terminates_the_fixture_process() {
        let port = reserve_qualification_port().unwrap();
        let result = run_runtime_qualification(
            qualification_fixture_launch("health-fail", port),
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(result.status, "failed");
        assert_eq!(result.checks[0].status, "passed");
        assert_eq!(result.checks[1].status, "failed");
        assert_eq!(result.checks[2].status, "skipped");
        assert_port_released(port);
    }

    #[test]
    fn inference_failure_is_reported_after_startup_and_health_pass() {
        let port = reserve_qualification_port().unwrap();
        let result = run_runtime_qualification(
            qualification_fixture_launch("inference-fail", port),
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(result.status, "failed");
        assert_eq!(result.checks[0].status, "passed");
        assert_eq!(result.checks[1].status, "passed");
        assert_eq!(result.checks[2].status, "failed");
        assert_port_released(port);
    }

    #[test]
    fn operator_cancellation_terminates_the_fixture_process() {
        let port = reserve_qualification_port().unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_signal = cancelled.clone();
        let trigger = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            cancel_signal.store(true, Ordering::SeqCst);
        });
        let result =
            run_runtime_qualification(qualification_fixture_launch("health-fail", port), cancelled);
        trigger.join().unwrap();
        assert_eq!(result.status, "cancelled");
        assert_eq!(result.checks[0].status, "cancelled");
        assert_port_released(port);
    }

    #[test]
    fn early_process_exit_is_a_startup_failure() {
        let port = reserve_qualification_port().unwrap();
        let mut launch = qualification_fixture_launch("success", port);
        launch.arguments[0] = "qualification_exit_fixture_process".to_string();
        let result = run_runtime_qualification(launch, Arc::new(AtomicBool::new(false)));
        assert_eq!(result.status, "failed");
        assert_eq!(result.checks[0].status, "failed");
        assert_eq!(result.checks[1].status, "skipped");
        assert_port_released(port);
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
    fn executable_fingerprint_includes_sampled_file_content() {
        let path = std::env::temp_dir().join(format!(
            "lsm-engine-fingerprint-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, vec![b'a'; 128 * 1024]).unwrap();
        let first = executable_fingerprint(&path.to_string_lossy());
        std::fs::write(&path, vec![b'b'; 128 * 1024]).unwrap();
        let second = executable_fingerprint(&path.to_string_lossy());
        assert_ne!(first, second);
        let _ = std::fs::remove_file(path);
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
