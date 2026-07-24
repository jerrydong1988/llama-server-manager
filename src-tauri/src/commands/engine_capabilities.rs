use crate::commands::model_inventory;
use crate::models::{AppState, EngineCapabilities, EngineInfo};
use std::collections::{BTreeSet, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROBE_STREAM_BYTES: usize = 512 * 1024;
const MIN_CONFIDENT_FLAG_COUNT: usize = 10;
const REPORTED_DEFAULTS_VERSION: u8 = 1;

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
    #[cfg(windows)]
    let normalized_path = path.to_string_lossy().to_ascii_lowercase();
    #[cfg(not(windows))]
    let normalized_path = path.to_string_lossy().into_owned();

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

fn probe_engine(mut engine: EngineInfo) -> EngineInfo {
    let fingerprint_before = executable_fingerprint(&engine.exe);
    let version_output = run_bounded(&engine.exe, "--version");
    let help_output = run_bounded(&engine.exe, "--help");
    let supported_flags = extract_supported_flags(&help_output.text);
    let reported_defaults = extract_reported_defaults(&help_output.text);
    let status = classify_probe_status(&supported_flags, help_output.timed_out);
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
        .find(|engine| engine.id == engine_id)
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
        .find(|engine| engine.id == probed.id)
        .ok_or_else(|| "engine was removed while capability probing was in progress".to_string())?;
    let current_path =
        std::fs::canonicalize(&current.exe).unwrap_or_else(|_| current.exe.clone().into());
    let probed_path =
        std::fs::canonicalize(&probed.exe).unwrap_or_else(|_| probed.exe.clone().into());
    if current_path != probed_path {
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
