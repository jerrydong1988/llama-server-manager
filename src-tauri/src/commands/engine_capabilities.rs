use crate::commands::model_inventory;
use crate::models::{AppState, EngineCapabilities, EngineInfo};
use std::collections::BTreeSet;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROBE_STREAM_BYTES: usize = 512 * 1024;
const MIN_CONFIDENT_FLAG_COUNT: usize = 10;

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

fn spawn_stream_reader<R: Read + Send + 'static>(reader: R) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    let _ = thread::Builder::new()
        .name("engine-probe-output".to_string())
        .spawn(move || {
            let _ = sender.send(read_stream_capped(reader));
        });
    receiver
}

fn run_bounded(executable: &str, argument: &str) -> CommandOutput {
    let mut command = Command::new(executable);
    command
        .arg(argument)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return CommandOutput {
                text: String::new(),
                timed_out: false,
                error: Some(format!("cannot execute llama-server: {error}")),
            }
        }
    };
    let stdout_receiver = child.stdout.take().map(spawn_stream_reader);
    let stderr_receiver = child.stderr.take().map(spawn_stream_reader);

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
                let _ = child.kill();
                break child.wait().err().map(|error| error.to_string());
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Some(error.to_string());
            }
        }
    };

    // A child process can leave inherited pipe handles open in a descendant. Do not let an
    // explicitly probed binary keep the application blocked after the parent has exited.
    let stdout = stdout_receiver
        .and_then(|receiver| receiver.recv_timeout(Duration::from_millis(500)).ok())
        .unwrap_or_default();
    let stderr = stderr_receiver
        .and_then(|receiver| receiver.recv_timeout(Duration::from_millis(500)).ok())
        .unwrap_or_default();
    let mut combined = stdout;
    if !combined.is_empty() && !stderr.is_empty() {
        combined.push(b'\n');
    }
    combined.extend(stderr);

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

fn executable_fingerprint(executable: &str) -> String {
    std::fs::metadata(executable)
        .ok()
        .map(|metadata| {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            format!("{}:{modified}", metadata.len())
        })
        .unwrap_or_default()
}

pub(crate) fn capabilities_match_executable(
    executable: &str,
    capabilities: &EngineCapabilities,
) -> bool {
    !capabilities.executable_fingerprint.is_empty()
        && capabilities.executable_fingerprint == executable_fingerprint(executable)
}

fn probe_engine(mut engine: EngineInfo) -> EngineInfo {
    let version_output = run_bounded(&engine.exe, "--version");
    let help_output = run_bounded(&engine.exe, "--help");
    let supported_flags = extract_supported_flags(&help_output.text);
    let status = classify_probe_status(&supported_flags, help_output.timed_out);

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

    if let Some(version) = first_nonempty_line(&version_output.text) {
        engine.version = version;
    }
    engine.capabilities = EngineCapabilities {
        status: status.to_string(),
        supported_flags,
        help_hash: if help_output.text.is_empty() {
            String::new()
        } else {
            help_hash(&help_output.text)
        },
        executable_fingerprint: executable_fingerprint(&engine.exe),
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
    command
        .iter()
        .skip(1)
        .filter_map(|token| command_flag(token))
        .filter(|flag| !supported.contains(flag))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
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
    let mut probed = tokio::task::spawn_blocking(move || probe_engine(engine))
        .await
        .map_err(|error| format!("engine capability probe task failed: {error}"))?;
    if let Err(error) = model_inventory::update_engine_probe(&probed) {
        let warning = compact_error(format!("capability cache was not persisted: {error}"));
        probed.capabilities.error = Some(match probed.capabilities.error.take() {
            Some(existing) => compact_error(format!("{existing}; {warning}")),
            None => warning,
        });
    }
    let mut engines = state.engines.lock().unwrap();
    let current = engines
        .iter_mut()
        .find(|engine| engine.id == probed.id)
        .ok_or_else(|| "engine was removed while capability probing was in progress".to_string())?;
    *current = probed.clone();
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
