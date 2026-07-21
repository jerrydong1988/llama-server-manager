use crate::commands::adlx;
use crate::commands::engine_capabilities::{
    blocked_security_flags, capabilities_match_executable, command_for_capabilities,
    unsupported_command_flags,
};
use crate::commands::nvml;
use crate::error::{AppError, AppResult};
use crate::models::{
    ensure_managed_public_model_alias, AppState, EngineCapabilities, InstanceConfig,
    RunningInstance, SystemMetrics,
};
use crate::vector_policy::{normalize_for_launch, ModelWorkload};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use sysinfo::{Pid, ProcessesToUpdate, System};
use tauri::{Emitter, Manager};

pub(crate) const MAX_SERVER_LOG_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const RETAINED_SERVER_LOG_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TRACKED_PERF_TASKS: usize = 1_024;
const LOG_REPLAY_LINES: usize = 2_000;
const LOG_REPLAY_MAX_BYTES: u64 = 4 * 1024 * 1024;
const LOG_EVENT_BATCH_SIZE: usize = 200;
static SERVER_HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

struct CappedLogState {
    file: std::fs::File,
    size: u64,
    max_bytes: u64,
    retained_bytes: u64,
    generation: u64,
    rotation_boundary: u64,
}

pub(crate) struct CappedLogWriter {
    state: Mutex<CappedLogState>,
}

struct LogReadChunk {
    bytes: Vec<u8>,
    end_offset: u64,
    generation: u64,
    rotated: bool,
}

const MAX_PENDING_LOG_BYTES: usize = 256 * 1024;
pub(crate) const INITIAL_HEALTH_GRACE: std::time::Duration = std::time::Duration::from_secs(120);
const HEALTH_FAILURE_THRESHOLD: u32 = 3;

impl CappedLogWriter {
    pub(crate) fn new(
        path: std::path::PathBuf,
        max_bytes: u64,
        retained_bytes: u64,
    ) -> std::io::Result<Self> {
        if retained_bytes >= max_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "retained log size must be smaller than maximum log size",
            ));
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        Ok(Self {
            state: Mutex::new(CappedLogState {
                file,
                size: 0,
                max_bytes,
                retained_bytes,
                generation: 0,
                rotation_boundary: 0,
            }),
        })
    }

    fn append(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut state = self.state.lock().unwrap();
        if state.size.saturating_add(bytes.len() as u64) > state.max_bytes {
            let keep = state.size.min(state.retained_bytes);
            let mut tail = vec![0_u8; keep as usize];
            let tail_start = state.size.saturating_sub(keep);
            state.file.flush()?;
            state.file.seek(SeekFrom::Start(tail_start))?;
            state.file.read_exact(&mut tail)?;
            state.file.set_len(0)?;
            state.file.seek(SeekFrom::Start(0))?;
            state.file.write_all(&tail)?;
            state.size = keep;
            state.generation = state.generation.saturating_add(1);
            state.rotation_boundary = keep;
        }
        state.file.seek(SeekFrom::End(0))?;
        state.file.write_all(bytes)?;
        state.size = state.size.saturating_add(bytes.len() as u64);
        Ok(())
    }

    fn read_since(&self, offset: u64, generation: u64) -> std::io::Result<LogReadChunk> {
        let mut state = self.state.lock().unwrap();
        let rotated = state.generation != generation;
        let start = if rotated {
            state.rotation_boundary.min(state.size)
        } else {
            offset.min(state.size)
        };
        let available = state.size.saturating_sub(start);
        let mut bytes = Vec::with_capacity(usize::try_from(available).unwrap_or(0));
        state.file.flush()?;
        state.file.seek(SeekFrom::Start(start))?;
        Read::take(&mut state.file, available).read_to_end(&mut bytes)?;
        state.file.seek(SeekFrom::End(0))?;
        Ok(LogReadChunk {
            bytes,
            end_offset: state.size,
            generation: state.generation,
            rotated,
        })
    }

    fn read_tail(&self, max_lines: usize) -> std::io::Result<(Vec<String>, u64, u64)> {
        let mut state = self.state.lock().unwrap();
        let file_len = state.size;
        let generation = state.generation;
        state.file.flush()?;
        let result = read_log_tail_from_file(&mut state.file, file_len, max_lines);
        state.file.seek(SeekFrom::End(0))?;
        result.map(|(lines, offset)| (lines, offset, generation))
    }
}

fn take_complete_log_lines(pending: &mut Vec<u8>, bytes: &[u8]) -> Vec<String> {
    pending.extend_from_slice(bytes);
    let truncated = pending.len() > MAX_PENDING_LOG_BYTES;
    if truncated {
        let overflow = pending.len() - MAX_PENDING_LOG_BYTES;
        pending.drain(..overflow);
    }
    let Some(last_newline) = pending.iter().rposition(|byte| *byte == b'\n') else {
        return if truncated {
            vec!["[log line truncated after 256 KiB without a terminator]".to_string()]
        } else {
            Vec::new()
        };
    };
    let complete = pending.drain(..=last_newline).collect::<Vec<_>>();
    let mut lines = complete
        .split(|byte| *byte == b'\n')
        .filter_map(|line| {
            if line.is_empty() {
                return None;
            }
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            Some(String::from_utf8_lossy(line).into_owned())
        })
        .collect::<Vec<_>>();
    if truncated {
        lines.insert(
            0,
            "[log line truncated after 256 KiB without a terminator]".to_string(),
        );
    }
    lines
}

pub(crate) fn spawn_log_pump<R>(
    mut source: R,
    writer: Arc<CappedLogWriter>,
) -> std::thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        let mut log_write_failed = false;
        loop {
            match source.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if !log_write_failed {
                        if let Err(error) = writer.append(&buffer[..read]) {
                            eprintln!("Server log persistence failed; continuing to drain output: {error}");
                            log_write_failed = true;
                        }
                    }
                }
            }
        }
    })
}

pub(crate) fn spawn_runtime_log_pump<R>(
    mut source: R,
    writer: Arc<CappedLogWriter>,
    tracker: Arc<Mutex<RuntimePerfTracker>>,
) -> std::thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        let mut pending = Vec::new();
        let mut log_write_failed = false;
        loop {
            match source.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if !log_write_failed {
                        if let Err(error) = writer.append(&buffer[..read]) {
                            eprintln!("Runtime log persistence failed; continuing to supervise output: {error}");
                            log_write_failed = true;
                        }
                    }
                    for line in take_complete_log_lines(&mut pending, &buffer[..read]) {
                        tracker.lock().unwrap().process_line(&line);
                    }
                }
            }
        }
        if !pending.is_empty() {
            let line = String::from_utf8_lossy(&pending).trim().to_string();
            if !line.is_empty() {
                tracker.lock().unwrap().process_line(&line);
            }
        }
    })
}

fn terminate_spawned_child(child: &mut std::process::Child) -> Result<(), String> {
    match child.kill() {
        Ok(()) => child
            .wait()
            .map(|_| ())
            .map_err(|error| format!("Failed to reap the terminated server process: {error}")),
        Err(kill_error) => match child.try_wait() {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(format!(
                "Failed to terminate the spawned server process: {kill_error}"
            )),
            Err(wait_error) => Err(format!(
                "Failed to terminate the spawned server process: {kill_error}; failed to query its state: {wait_error}"
            )),
        },
    }
}

// Generate CLI command.

fn format_command_for_display(command: &[String]) -> String {
    let mut masked = Vec::with_capacity(command.len());
    let mut hide_next = false;
    for argument in command {
        let value = if hide_next {
            hide_next = false;
            "********".to_string()
        } else if argument == "--api-key" {
            hide_next = true;
            argument.clone()
        } else if argument.starts_with("--api-key=") {
            "--api-key=********".to_string()
        } else {
            argument.clone()
        };
        if value.chars().any(char::is_whitespace) {
            masked.push(format!("\"{}\"", value.replace('"', "\\\"")));
        } else {
            masked.push(value);
        }
    }
    masked.join(" ")
}

pub(crate) fn effective_api_key(config: &InstanceConfig) -> String {
    let inline = config.api_key.trim();
    if !inline.is_empty() {
        return inline
            .split(',')
            .map(str::trim)
            .find(|key| !key.is_empty())
            .unwrap_or_default()
            .to_string();
    }

    if config.api_key_file.trim().is_empty() {
        return String::new();
    }

    std::fs::read_to_string(config.api_key_file.trim())
        .ok()
        .and_then(|content| {
            content.lines().find_map(|line| {
                let key = line.trim().trim_start_matches('\u{feff}');
                if key.is_empty() || key.starts_with('#') {
                    None
                } else {
                    Some(key.to_string())
                }
            })
        })
        .unwrap_or_default()
}

pub(crate) fn effective_server_scheme(config: &InstanceConfig) -> &'static str {
    if !config.ssl_key_file.trim().is_empty() && !config.ssl_cert_file.trim().is_empty() {
        "https"
    } else {
        "http"
    }
}

fn validate_tls_configuration(config: &InstanceConfig) -> AppResult<()> {
    let has_key = !config.ssl_key_file.trim().is_empty();
    let has_cert = !config.ssl_cert_file.trim().is_empty();
    if has_key != has_cert {
        return Err(AppError::new(
            "TLS_KEY_CERT_PAIR_REQUIRED",
            "TLS 私钥和证书必须同时配置，不能只填写其中一个。",
            false,
        ));
    }
    Ok(())
}

pub(crate) fn telemetry_config_hash(config: &InstanceConfig) -> String {
    let mut hasher = DefaultHasher::new();
    serde_json::to_string(config)
        .unwrap_or_default()
        .hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
pub fn generate_command(config: &InstanceConfig, engine_path: &str) -> Vec<String> {
    prepare_launch(config.clone(), engine_path).2
}

fn uses_manual_command(config: &InstanceConfig) -> bool {
    config.launch_mode.eq_ignore_ascii_case("manual")
}

fn should_emit(config: &InstanceConfig, field: &str, legacy_condition: bool) -> bool {
    config
        .explicit_overrides
        .as_ref()
        .map_or(legacy_condition, |fields| {
            fields.iter().any(|candidate| candidate == field)
        })
}

fn should_emit_any(config: &InstanceConfig, fields: &[&str], legacy_condition: bool) -> bool {
    config
        .explicit_overrides
        .as_ref()
        .map_or(legacy_condition, |configured| {
            fields
                .iter()
                .any(|field| configured.iter().any(|candidate| candidate == field))
        })
}

fn append_basic_flags(config: &InstanceConfig, is_emb: bool, cmd: &mut Vec<String>) {
    // Basic.
    if should_emit(config, "alias", !config.alias.is_empty()) && !config.alias.is_empty() {
        cmd.extend_from_slice(&["-a".into(), config.alias.clone()]);
    }
    if !is_emb {
        if should_emit(config, "lora_path", !config.lora_path.is_empty())
            && !config.lora_path.is_empty()
        {
            cmd.extend_from_slice(&["--lora".into(), config.lora_path.clone()]);
        }
        if should_emit(
            config,
            "lora_init_without_apply",
            config.lora_init_without_apply,
        ) && config.lora_init_without_apply
        {
            cmd.push("--lora-init-without-apply".into());
        }
        if should_emit(config, "lora_scaled", !config.lora_scaled.is_empty())
            && !config.lora_scaled.is_empty()
        {
            cmd.extend_from_slice(&["--lora-scaled".into(), config.lora_scaled.clone()]);
        }
        if should_emit(config, "mmproj_path", !config.mmproj_path.is_empty())
            && !config.mmproj_path.is_empty()
        {
            cmd.extend_from_slice(&["--mmproj".into(), config.mmproj_path.clone()]);
        }
        if should_emit(config, "mmproj_url", !config.mmproj_url.is_empty())
            && !config.mmproj_url.is_empty()
        {
            cmd.extend_from_slice(&["--mmproj-url".into(), config.mmproj_url.clone()]);
        }
        let mmproj_mode = if !config.mmproj_mode.is_empty() {
            config.mmproj_mode.as_str()
        } else if config.no_mmproj {
            "off"
        } else if config.mmproj_auto {
            "on"
        } else {
            ""
        };
        if should_emit_any(
            config,
            &["mmproj_mode", "mmproj_auto", "no_mmproj"],
            !mmproj_mode.is_empty(),
        ) {
            match mmproj_mode {
                "on" => cmd.push("--mmproj-auto".into()),
                "off" => cmd.push("--no-mmproj".into()),
                _ => {}
            }
        }
        let projector_active =
            !config.mmproj_path.is_empty() || !config.mmproj_url.is_empty() || mmproj_mode != "off";
        if projector_active && should_emit(config, "no_mmproj_offload", config.no_mmproj_offload) {
            cmd.push(
                if config.no_mmproj_offload {
                    "--no-mmproj-offload"
                } else {
                    "--mmproj-offload"
                }
                .into(),
            );
        }
        if should_emit(config, "chat_template", !config.chat_template.is_empty())
            && !config.chat_template.is_empty()
        {
            cmd.extend_from_slice(&["--chat-template".into(), config.chat_template.clone()]);
        }
        if should_emit(
            config,
            "chat_template_file",
            !config.chat_template_file.is_empty(),
        ) && !config.chat_template_file.is_empty()
        {
            cmd.extend_from_slice(&[
                "--chat-template-file".into(),
                config.chat_template_file.clone(),
            ]);
        }
        if should_emit(config, "skip_chat_parsing", config.skip_chat_parsing)
            && config.skip_chat_parsing
        {
            cmd.push("--skip-chat-parsing".into());
        }
        if should_emit(
            config,
            "reasoning_format",
            !config.reasoning_format.is_empty(),
        ) && !config.reasoning_format.is_empty()
        {
            cmd.extend_from_slice(&["--reasoning-format".into(), config.reasoning_format.clone()]);
        }
        if should_emit(config, "reasoning", !config.reasoning.is_empty())
            && !config.reasoning.is_empty()
        {
            cmd.extend_from_slice(&["--reasoning".into(), config.reasoning.clone()]);
        }
        if should_emit(
            config,
            "reasoning_preserve",
            !config.reasoning_preserve.is_empty(),
        ) {
            match config.reasoning_preserve.as_str() {
                "on" => cmd.push("--reasoning-preserve".into()),
                "off" => cmd.push("--no-reasoning-preserve".into()),
                _ => {}
            }
        }
        if should_emit(
            config,
            "reasoning_budget",
            !config.reasoning_budget.is_empty(),
        ) && !config.reasoning_budget.is_empty()
        {
            cmd.extend_from_slice(&["--reasoning-budget".into(), config.reasoning_budget.clone()]);
        }
        if should_emit(
            config,
            "reasoning_budget_message",
            !config.reasoning_budget_message.is_empty(),
        ) && !config.reasoning_budget_message.is_empty()
        {
            cmd.extend_from_slice(&[
                "--reasoning-budget-message".into(),
                config.reasoning_budget_message.clone(),
            ]);
        }
        if should_emit(
            config,
            "reasoning_effort",
            !config.reasoning_effort.is_empty(),
        ) && !config.reasoning_effort.is_empty()
        {
            let re = format!("{{\"reasoning_effort\": \"{}\"}}", config.reasoning_effort);
            cmd.extend_from_slice(&["--chat-template-kwargs".into(), re]);
        }
        if should_emit(config, "jinja", true) {
            cmd.push(if config.jinja {
                "--jinja".into()
            } else {
                "--no-jinja".into()
            });
        }
        if should_emit(config, "grammar_file", !config.grammar_file.is_empty())
            && !config.grammar_file.is_empty()
        {
            cmd.extend_from_slice(&["--grammar-file".into(), config.grammar_file.clone()]);
        }
        if should_emit(config, "grammar", !config.grammar.is_empty()) && !config.grammar.is_empty()
        {
            cmd.extend_from_slice(&["--grammar".into(), config.grammar.clone()]);
        }
    }
}

fn append_context_flags(config: &InstanceConfig, is_emb: bool, cmd: &mut Vec<String>) {
    // Performance and context.
    if !config.ctx_size_auto
        && should_emit_any(
            config,
            &["ctx_size", "ctx_size_auto"],
            !config.ctx_size_auto,
        )
    {
        cmd.extend_from_slice(&["-c".into(), config.ctx_size.to_string()]);
    }
    if !config.gpu_layers_auto
        && should_emit_any(
            config,
            &["gpu_layers", "gpu_layers_auto"],
            !config.gpu_layers_auto,
        )
    {
        cmd.extend_from_slice(&["-ngl".into(), config.gpu_layers.to_string()]);
    }
    if should_emit(config, "threads", config.threads > 0) && config.threads > 0 {
        cmd.extend_from_slice(&["-t".into(), config.threads.to_string()]);
    }
    if should_emit(config, "batch_size", config.batch_size > 0) && config.batch_size > 0 {
        cmd.extend_from_slice(&["-b".into(), config.batch_size.to_string()]);
    }
    if should_emit(config, "ubatch_size", config.ubatch_size > 0) && config.ubatch_size > 0 {
        cmd.extend_from_slice(&["-ub".into(), config.ubatch_size.to_string()]);
    }
    if should_emit(
        config,
        "parallel",
        config.parallel > 0 || config.parallel == -1,
    ) && (config.parallel > 0 || config.parallel == -1)
    {
        cmd.extend_from_slice(&["-np".into(), config.parallel.to_string()]);
    }
    if should_emit(config, "cont_batching", true) {
        cmd.push(if config.cont_batching {
            "-cb".into()
        } else {
            "--no-cont-batching".into()
        });
    }
    if !is_emb && should_emit(config, "cache_prompt", true) {
        cmd.push(if config.cache_prompt {
            "--cache-prompt".into()
        } else {
            "--no-cache-prompt".into()
        });
    }
    if should_emit(config, "threads_batch", config.threads_batch > 0) && config.threads_batch > 0 {
        cmd.extend_from_slice(&["--threads-batch".into(), config.threads_batch.to_string()]);
    }
    if should_emit(config, "threads_http", config.threads_http >= 0) && config.threads_http >= 0 {
        cmd.extend_from_slice(&["--threads-http".into(), config.threads_http.to_string()]);
    }
    if !is_emb {
        if should_emit(config, "keep", true) {
            cmd.extend_from_slice(&["--keep".into(), config.keep.to_string()]);
        }
        if should_emit(config, "cache_reuse", true) {
            cmd.extend_from_slice(&["--cache-reuse".into(), config.cache_reuse.to_string()]);
        }
        if should_emit(config, "cache_ram", true) {
            cmd.extend_from_slice(&["-cram".into(), config.cache_ram.to_string()]);
        }
    }
    if should_emit(config, "warmup", true) {
        cmd.push(if config.warmup {
            "--warmup".into()
        } else {
            "--no-warmup".into()
        });
    }
    if !is_emb {
        if should_emit(config, "ctx_checkpoints", true) {
            cmd.extend_from_slice(&["-ctxcp".into(), config.ctx_checkpoints.to_string()]);
        }
        if should_emit(config, "checkpoint_min_step", true) {
            cmd.extend_from_slice(&["-cms".into(), config.checkpoint_min_step.to_string()]);
        }
        if should_emit(config, "swa_full", config.swa_full) && config.swa_full {
            cmd.push("--swa-full".into());
        }
    }
}

fn append_rope_flags(config: &InstanceConfig, cmd: &mut Vec<String>) {
    // RoPE / YaRN.
    if should_emit(config, "rope_scaling", !config.rope_scaling.is_empty())
        && !config.rope_scaling.is_empty()
    {
        cmd.extend_from_slice(&["--rope-scaling".into(), config.rope_scaling.clone()]);
    }
    if should_emit(config, "rope_scale", config.rope_scale > 0.0) {
        cmd.extend_from_slice(&["--rope-scale".into(), config.rope_scale.to_string()]);
    }
    if should_emit(config, "rope_freq_base", config.rope_freq_base > 0.0) {
        cmd.extend_from_slice(&["--rope-freq-base".into(), config.rope_freq_base.to_string()]);
    }
    if should_emit(config, "rope_freq_scale", config.rope_freq_scale > 0.0) {
        cmd.extend_from_slice(&[
            "--rope-freq-scale".into(),
            config.rope_freq_scale.to_string(),
        ]);
    }
    if should_emit(config, "yarn_ext_factor", config.yarn_ext_factor >= 0.0)
        && config.yarn_ext_factor >= 0.0
    {
        cmd.extend_from_slice(&[
            "--yarn-ext-factor".into(),
            config.yarn_ext_factor.to_string(),
        ]);
    }
    if should_emit(config, "yarn_attn_factor", config.yarn_attn_factor != -1.0) {
        cmd.extend_from_slice(&[
            "--yarn-attn-factor".into(),
            config.yarn_attn_factor.to_string(),
        ]);
    }
    if should_emit(config, "yarn_beta_slow", config.yarn_beta_slow > 0.0) {
        cmd.extend_from_slice(&["--yarn-beta-slow".into(), config.yarn_beta_slow.to_string()]);
    }
    if should_emit(config, "yarn_beta_fast", config.yarn_beta_fast != -1.0) {
        cmd.extend_from_slice(&["--yarn-beta-fast".into(), config.yarn_beta_fast.to_string()]);
    }
    if should_emit(config, "yarn_orig_ctx", config.yarn_orig_ctx > 0) {
        cmd.extend_from_slice(&["--yarn-orig-ctx".into(), config.yarn_orig_ctx.to_string()]);
    }
}

fn append_flash_attention_flags(config: &InstanceConfig, cmd: &mut Vec<String>) {
    // Flash Attention.
    let fa = config.flash_attn.as_str();
    if should_emit(config, "flash_attn", fa != "auto" && !fa.is_empty()) && !fa.is_empty() {
        cmd.extend_from_slice(&["-fa".into(), fa.to_string()]);
    }
}

fn append_memory_flags(config: &InstanceConfig, cmd: &mut Vec<String>) {
    // Memory and loading.
    if should_emit(config, "moe_cpu_layers", config.moe_cpu_layers > 0) && config.moe_cpu_layers > 0
    {
        cmd.extend_from_slice(&["--n-cpu-moe".into(), config.moe_cpu_layers.to_string()]);
    }
    if should_emit(config, "cpu_moe", config.cpu_moe) && config.cpu_moe {
        cmd.push("--cpu-moe".into());
    }
    if should_emit(config, "mlock", config.mlock) && config.mlock {
        cmd.push("--mlock".into());
    }
    if should_emit(config, "no_mmap", config.no_mmap) {
        cmd.push(
            if config.no_mmap {
                "--no-mmap"
            } else {
                "--mmap"
            }
            .into(),
        );
    }
    if should_emit(config, "no_repack", config.no_repack) {
        cmd.push(
            if config.no_repack {
                "--no-repack"
            } else {
                "--repack"
            }
            .into(),
        );
    }
    if should_emit(config, "direct_io", config.direct_io) && config.direct_io {
        cmd.push("--direct-io".into());
    }
    let numa_mode = if config.numa_mode.is_empty() && config.numa {
        "distribute"
    } else {
        config.numa_mode.as_str()
    };
    if should_emit_any(config, &["numa_mode", "numa"], !numa_mode.is_empty())
        && !numa_mode.is_empty()
    {
        cmd.extend_from_slice(&["--numa".into(), numa_mode.into()]);
    }
    if should_emit(config, "check_tensors", config.check_tensors) && config.check_tensors {
        cmd.push("--check-tensors".into());
    }
    if should_emit(config, "perf", config.perf) {
        cmd.push(if config.perf { "--perf" } else { "--no-perf" }.into());
    }
    let fit_mode = if config.fit_mode.is_empty() && config.fit {
        "on"
    } else {
        config.fit_mode.as_str()
    };
    if should_emit_any(
        config,
        &["fit_mode", "fit"],
        matches!(fit_mode, "on" | "off"),
    ) && matches!(fit_mode, "on" | "off")
    {
        cmd.extend_from_slice(&["--fit".into(), fit_mode.into()]);
    }
    if fit_mode == "on" {
        if should_emit(config, "fit_target", !config.fit_target.is_empty())
            && !config.fit_target.is_empty()
        {
            cmd.extend_from_slice(&["-fitt".into(), config.fit_target.clone()]);
        }
        if should_emit(config, "fit_ctx", true) {
            cmd.extend_from_slice(&["-fitc".into(), config.fit_ctx.to_string()]);
        }
    }
}

fn append_kv_cache_flags(config: &InstanceConfig, is_emb: bool, cmd: &mut Vec<String>) {
    // KV cache.
    if should_emit(config, "cache_type_k", !config.cache_type_k.is_empty())
        && !config.cache_type_k.is_empty()
    {
        cmd.extend_from_slice(&["-ctk".into(), config.cache_type_k.clone()]);
    }
    if should_emit(config, "cache_type_v", !config.cache_type_v.is_empty())
        && !config.cache_type_v.is_empty()
    {
        cmd.extend_from_slice(&["-ctv".into(), config.cache_type_v.clone()]);
    }
    if !is_emb {
        if should_emit(
            config,
            "cache_type_draft_k",
            !config.cache_type_draft_k.is_empty(),
        ) && !config.cache_type_draft_k.is_empty()
        {
            cmd.extend_from_slice(&["-ctkd".into(), config.cache_type_draft_k.clone()]);
        }
        if should_emit(
            config,
            "cache_type_draft_v",
            !config.cache_type_draft_v.is_empty(),
        ) && !config.cache_type_draft_v.is_empty()
        {
            cmd.extend_from_slice(&["-ctvd".into(), config.cache_type_draft_v.clone()]);
        }
    }
    let kv_unified_mode = if config.kv_unified_mode.is_empty() && config.kv_unified {
        "on"
    } else {
        config.kv_unified_mode.as_str()
    };
    if should_emit_any(
        config,
        &["kv_unified_mode", "kv_unified"],
        !kv_unified_mode.is_empty(),
    ) {
        match kv_unified_mode {
            "on" => cmd.push("--kv-unified".into()),
            "off" => cmd.push("--no-kv-unified".into()),
            _ => {}
        }
    }
    if should_emit(config, "no_kv_offload", config.no_kv_offload) {
        cmd.push(
            if config.no_kv_offload {
                "--no-kv-offload"
            } else {
                "--kv-offload"
            }
            .into(),
        );
    }
    if should_emit(config, "cache_idle_slots", true) {
        cmd.push(if config.cache_idle_slots {
            "--cache-idle-slots".into()
        } else {
            "--no-cache-idle-slots".into()
        });
    }
}

fn append_device_flags(config: &InstanceConfig, cmd: &mut Vec<String>) {
    // GPU and device.
    if should_emit(config, "device", !config.device.is_empty()) && !config.device.is_empty() {
        cmd.extend_from_slice(&["-dev".into(), config.device.clone()]);
    }
    if should_emit(config, "split_mode", !config.split_mode.is_empty())
        && !config.split_mode.is_empty()
    {
        cmd.extend_from_slice(&["-sm".into(), config.split_mode.clone()]);
    }
    if should_emit(config, "tensor_split", !config.tensor_split.is_empty())
        && !config.tensor_split.is_empty()
    {
        cmd.extend_from_slice(&["-ts".into(), config.tensor_split.clone()]);
    }
    if should_emit(config, "main_gpu", config.main_gpu > 0) {
        cmd.extend_from_slice(&["-mg".into(), config.main_gpu.to_string()]);
    }
    if should_emit(config, "override_kv", !config.override_kv.is_empty())
        && !config.override_kv.is_empty()
    {
        cmd.extend_from_slice(&["--override-kv".into(), config.override_kv.clone()]);
    }
}

fn append_speculative_flags(config: &InstanceConfig, is_emb: bool, cmd: &mut Vec<String>) {
    // Speculative decoding.
    let spec_active = !is_emb
        && !config.spec_type.is_empty()
        && config.spec_type != "none"
        && should_emit(config, "spec_type", true);
    if spec_active {
        if should_emit(
            config,
            "draft_model_path",
            !config.draft_model_path.is_empty(),
        ) && !config.draft_model_path.is_empty()
        {
            cmd.extend_from_slice(&["-md".into(), config.draft_model_path.clone()]);
        }
        if should_emit(config, "draft_gpu_layers", true) {
            cmd.extend_from_slice(&["-ngld".into(), config.draft_gpu_layers.to_string()]);
        }
        if should_emit(config, "draft_tokens", true) {
            cmd.extend_from_slice(&["--spec-draft-n-max".into(), config.draft_tokens.to_string()]);
        }
        if should_emit(config, "spec_draft_n_min", true) {
            cmd.extend_from_slice(&[
                "--spec-draft-n-min".into(),
                config.spec_draft_n_min.to_string(),
            ]);
        }
        cmd.extend_from_slice(&["--spec-type".into(), config.spec_type.clone()]);
        if should_emit(config, "spec_draft_p_min", true) {
            cmd.extend_from_slice(&[
                "--spec-draft-p-min".into(),
                config.spec_draft_p_min.to_string(),
            ]);
        }
        if should_emit(config, "spec_draft_p_split", true) {
            cmd.extend_from_slice(&[
                "--spec-draft-p-split".into(),
                config.spec_draft_p_split.to_string(),
            ]);
        }
        if should_emit(
            config,
            "spec_draft_device",
            !config.spec_draft_device.is_empty(),
        ) && !config.spec_draft_device.is_empty()
        {
            cmd.extend_from_slice(&[
                "--spec-draft-device".into(),
                config.spec_draft_device.clone(),
            ]);
        }
        if should_emit(
            config,
            "lookup_cache_static",
            !config.lookup_cache_static.is_empty(),
        ) && !config.lookup_cache_static.is_empty()
        {
            cmd.extend_from_slice(&["-lcs".into(), config.lookup_cache_static.clone()]);
        }
        if should_emit(
            config,
            "lookup_cache_dynamic",
            !config.lookup_cache_dynamic.is_empty(),
        ) && !config.lookup_cache_dynamic.is_empty()
        {
            cmd.extend_from_slice(&["-lcd".into(), config.lookup_cache_dynamic.clone()]);
        }
        if should_emit(config, "spec_default", config.spec_default) && config.spec_default {
            cmd.push("--spec-default".into());
        }
        if should_emit(
            config,
            "spec_draft_backend_sampling",
            !config.spec_draft_backend_sampling,
        ) {
            cmd.push(
                if config.spec_draft_backend_sampling {
                    "--spec-draft-backend-sampling"
                } else {
                    "--no-spec-draft-backend-sampling"
                }
                .into(),
            );
        }
        if should_emit(config, "spec_draft_threads", config.spec_draft_threads > 0)
            && config.spec_draft_threads > 0
        {
            cmd.extend_from_slice(&["-td".into(), config.spec_draft_threads.to_string()]);
        }
        if should_emit(
            config,
            "spec_draft_threads_batch",
            config.spec_draft_threads_batch > 0,
        ) && config.spec_draft_threads_batch > 0
        {
            cmd.extend_from_slice(&["-tbd".into(), config.spec_draft_threads_batch.to_string()]);
        }
    }
}

fn append_network_flags(config: &InstanceConfig, is_emb: bool, cmd: &mut Vec<String>) {
    // Network.
    cmd.extend_from_slice(&[
        "--host".into(),
        config.host.clone(),
        "--port".into(),
        config.port.to_string(),
    ]);
    if !config.api_key.is_empty() {
        cmd.extend_from_slice(&["--api-key".into(), config.api_key.clone()]);
    }
    if !config.api_key_file.is_empty() {
        cmd.extend_from_slice(&["--api-key-file".into(), config.api_key_file.clone()]);
    }
    if !config.ssl_key_file.is_empty() {
        cmd.extend_from_slice(&["--ssl-key-file".into(), config.ssl_key_file.clone()]);
    }
    if !config.ssl_cert_file.is_empty() {
        cmd.extend_from_slice(&["--ssl-cert-file".into(), config.ssl_cert_file.clone()]);
    }
    if should_emit(config, "no_ui", config.no_ui) {
        cmd.push(if config.no_ui { "--no-ui" } else { "--ui" }.into());
    }
    if should_emit(config, "offline", config.offline) && config.offline {
        cmd.push("--offline".into());
    }
    if should_emit(config, "path_prefix", !config.path_prefix.is_empty())
        && !config.path_prefix.is_empty()
    {
        cmd.extend_from_slice(&["--path".into(), config.path_prefix.clone()]);
    }
    if should_emit(config, "api_prefix", !config.api_prefix.is_empty())
        && !config.api_prefix.is_empty()
    {
        cmd.extend_from_slice(&["--api-prefix".into(), config.api_prefix.clone()]);
    }
    if should_emit(config, "cors_origins", !config.cors_origins.is_empty())
        && !config.cors_origins.is_empty()
    {
        cmd.extend_from_slice(&["--cors-origins".into(), config.cors_origins.clone()]);
    }
    if should_emit(config, "cors_methods", !config.cors_methods.is_empty())
        && !config.cors_methods.is_empty()
    {
        cmd.extend_from_slice(&["--cors-methods".into(), config.cors_methods.clone()]);
    }
    if should_emit(config, "cors_headers", !config.cors_headers.is_empty())
        && !config.cors_headers.is_empty()
    {
        cmd.extend_from_slice(&["--cors-headers".into(), config.cors_headers.clone()]);
    }
    if should_emit(
        config,
        "cors_credentials",
        !config.cors_credentials.is_empty(),
    ) {
        match config.cors_credentials.as_str() {
            "on" => cmd.push("--cors-credentials".into()),
            "off" => cmd.push("--no-cors-credentials".into()),
            _ => {}
        }
    }
    if !is_emb {
        if should_emit(config, "ui_config_file", !config.ui_config_file.is_empty())
            && !config.ui_config_file.is_empty()
        {
            cmd.extend_from_slice(&["--ui-config-file".into(), config.ui_config_file.clone()]);
        }
        if should_emit(config, "ui_config", !config.ui_config.is_empty())
            && !config.ui_config.is_empty()
        {
            cmd.extend_from_slice(&["--ui-config".into(), config.ui_config.clone()]);
        }
        if should_emit(config, "ui_mcp_proxy", config.ui_mcp_proxy) && config.ui_mcp_proxy {
            cmd.push("--ui-mcp-proxy".into());
        }
        if should_emit(config, "agent", config.agent) && config.agent {
            cmd.push("--agent".into());
        }
    }
}

fn append_workload_flags(config: &InstanceConfig, cmd: &mut Vec<String>) {
    // Embedding / generation.
    if config.embedding {
        cmd.push("--embedding".into());
        if !config.pooling.is_empty() {
            cmd.extend_from_slice(&["--pooling".into(), config.pooling.clone()]);
        }
        if should_emit(config, "embd_normalize", true) {
            cmd.extend_from_slice(&["--embd-normalize".into(), config.embd_normalize.to_string()]);
        }
        if config.reranking {
            cmd.push("--reranking".into());
        }
    } else {
        if should_emit(config, "n_predict", config.n_predict != -1) {
            cmd.extend_from_slice(&["-n".into(), config.n_predict.to_string()]);
        }
        if should_emit(config, "ignore_eos", config.ignore_eos) && config.ignore_eos {
            cmd.push("--ignore-eos".into());
        }
        if should_emit(config, "json_schema", !config.json_schema.is_empty())
            && !config.json_schema.is_empty()
        {
            cmd.extend_from_slice(&["--json-schema".into(), config.json_schema.clone()]);
        }
        if should_emit(
            config,
            "json_schema_file",
            !config.json_schema_file.is_empty(),
        ) && !config.json_schema_file.is_empty()
        {
            cmd.extend_from_slice(&["-jf".into(), config.json_schema_file.clone()]);
        }
        if should_emit(config, "temp", (config.temp - 0.8).abs() > f32::EPSILON) {
            cmd.extend_from_slice(&["--temp".into(), config.temp.to_string()]);
        }
        if should_emit(config, "top_k", config.top_k != 40) {
            cmd.extend_from_slice(&["--top-k".into(), config.top_k.to_string()]);
        }
        if should_emit(config, "top_p", (config.top_p - 0.95).abs() > f32::EPSILON) {
            cmd.extend_from_slice(&["--top-p".into(), config.top_p.to_string()]);
        }
        if should_emit(
            config,
            "repeat_penalty",
            (config.repeat_penalty - 1.0).abs() > f32::EPSILON,
        ) {
            cmd.extend_from_slice(&["--repeat-penalty".into(), config.repeat_penalty.to_string()]);
        }
        if should_emit(config, "seed", config.seed != -1) {
            cmd.extend_from_slice(&["--seed".into(), config.seed.to_string()]);
        }
        if should_emit(config, "min_p", (config.min_p - 0.05).abs() > f32::EPSILON) {
            cmd.extend_from_slice(&["--min-p".into(), config.min_p.to_string()]);
        }
        if should_emit(
            config,
            "presence_penalty",
            config.presence_penalty.abs() > f32::EPSILON,
        ) {
            cmd.extend_from_slice(&[
                "--presence-penalty".into(),
                config.presence_penalty.to_string(),
            ]);
        }
        if should_emit(
            config,
            "frequency_penalty",
            config.frequency_penalty.abs() > f32::EPSILON,
        ) {
            cmd.extend_from_slice(&[
                "--frequency-penalty".into(),
                config.frequency_penalty.to_string(),
            ]);
        }
        if should_emit(config, "repeat_last_n", config.repeat_last_n != 64) {
            cmd.extend_from_slice(&["--repeat-last-n".into(), config.repeat_last_n.to_string()]);
        }
        if should_emit(config, "reverse_prompt", !config.reverse_prompt.is_empty())
            && !config.reverse_prompt.is_empty()
        {
            cmd.extend_from_slice(&["-r".into(), config.reverse_prompt.clone()]);
        }
        if should_emit(config, "special", config.special) && config.special {
            cmd.push("-sp".into());
        }
        if should_emit(config, "spm_infill", config.spm_infill) && config.spm_infill {
            cmd.push("--spm-infill".into());
        }
        if should_emit(config, "backend_sampling", config.backend_sampling)
            && config.backend_sampling
        {
            cmd.push("-bs".into());
        }

        // Advanced sampling
        if should_emit(config, "mirostat", config.mirostat > 0) && config.mirostat > 0 {
            cmd.extend_from_slice(&["--mirostat".into(), config.mirostat.to_string()]);
            if should_emit(config, "mirostat_lr", true) {
                cmd.extend_from_slice(&["--mirostat-lr".into(), config.mirostat_lr.to_string()]);
            }
            if should_emit(config, "mirostat_ent", true) {
                cmd.extend_from_slice(&["--mirostat-ent".into(), config.mirostat_ent.to_string()]);
            }
        }
        if should_emit(config, "xtc_probability", config.xtc_probability > 0.0)
            && config.xtc_probability > 0.0
        {
            cmd.extend_from_slice(&[
                "--xtc-probability".into(),
                config.xtc_probability.to_string(),
            ]);
            if should_emit(config, "xtc_threshold", true) {
                cmd.extend_from_slice(&[
                    "--xtc-threshold".into(),
                    config.xtc_threshold.to_string(),
                ]);
            }
        }
        if should_emit(config, "dynatemp_range", config.dynatemp_range > 0.0)
            && config.dynatemp_range > 0.0
        {
            cmd.extend_from_slice(&["--dynatemp-range".into(), config.dynatemp_range.to_string()]);
            if should_emit(config, "dynatemp_exp", true) {
                cmd.extend_from_slice(&["--dynatemp-exp".into(), config.dynatemp_exp.to_string()]);
            }
        }
        if should_emit(
            config,
            "typical_p",
            (config.typical_p - 1.0).abs() > f32::EPSILON,
        ) {
            cmd.extend_from_slice(&["--typical-p".into(), config.typical_p.to_string()]);
        }
        if should_emit(config, "dry_multiplier", config.dry_multiplier > 0.0)
            && config.dry_multiplier > 0.0
        {
            cmd.extend_from_slice(&["--dry-multiplier".into(), config.dry_multiplier.to_string()]);
            if should_emit(config, "dry_base", true) {
                cmd.extend_from_slice(&["--dry-base".into(), config.dry_base.to_string()]);
            }
            if should_emit(config, "dry_allowed_length", true) {
                cmd.extend_from_slice(&[
                    "--dry-allowed-length".into(),
                    config.dry_allowed_length.to_string(),
                ]);
            }
            if should_emit(config, "dry_penalty_last_n", true) {
                cmd.extend_from_slice(&[
                    "--dry-penalty-last-n".into(),
                    config.dry_penalty_last_n.to_string(),
                ]);
            }
        }
        if config.dry_multiplier > 0.0
            && should_emit(
                config,
                "dry_sequence_breaker",
                !config.dry_sequence_breaker.is_empty(),
            )
            && !config.dry_sequence_breaker.is_empty()
        {
            cmd.extend_from_slice(&[
                "--dry-sequence-breaker".into(),
                config.dry_sequence_breaker.clone(),
            ]);
        }
        if should_emit(config, "adaptive_target", config.adaptive_target >= 0.0)
            && config.adaptive_target >= 0.0
        {
            cmd.extend_from_slice(&[
                "--adaptive-target".into(),
                config.adaptive_target.to_string(),
            ]);
            if should_emit(config, "adaptive_decay", true) {
                cmd.extend_from_slice(&[
                    "--adaptive-decay".into(),
                    config.adaptive_decay.to_string(),
                ]);
            }
        }
        if should_emit(config, "top_n_sigma", config.top_n_sigma >= 0.0)
            && config.top_n_sigma >= 0.0
        {
            cmd.extend_from_slice(&["--top-n-sigma".into(), config.top_n_sigma.to_string()]);
        }
        if should_emit(config, "logit_bias", !config.logit_bias.is_empty())
            && !config.logit_bias.is_empty()
        {
            cmd.extend_from_slice(&["-l".into(), config.logit_bias.clone()]);
        }
        if should_emit(config, "samplers", !config.samplers.is_empty())
            && !config.samplers.is_empty()
        {
            cmd.extend_from_slice(&["--samplers".into(), config.samplers.clone()]);
        }
        if should_emit(config, "sampler_seq", !config.sampler_seq.is_empty())
            && !config.sampler_seq.is_empty()
        {
            cmd.extend_from_slice(&["--sampler-seq".into(), config.sampler_seq.clone()]);
        }
    }
}

fn append_server_flags(config: &InstanceConfig, is_emb: bool, cmd: &mut Vec<String>) {
    // Server features.
    if should_emit(config, "timeout", config.timeout > 0) && config.timeout > 0 {
        cmd.extend_from_slice(&["-to".into(), config.timeout.to_string()]);
    }
    if should_emit(config, "sleep_idle", config.sleep_idle >= 0) && config.sleep_idle >= 0 {
        cmd.extend_from_slice(&["--sleep-idle-seconds".into(), config.sleep_idle.to_string()]);
    }
    if !is_emb && should_emit(config, "context_shift", true) {
        cmd.push(if config.context_shift {
            "--context-shift".into()
        } else {
            "--no-context-shift".into()
        });
    }
    if should_emit(config, "verbose", config.verbose) && config.verbose {
        cmd.push("-v".into());
    }
    if config.metrics {
        cmd.push("--metrics".into());
    }
    if config.props {
        cmd.push("--props".into());
    }
    cmd.push(if config.slots_enabled {
        "--slots".into()
    } else {
        "--no-slots".into()
    });
    if !is_emb {
        if should_emit(config, "slot_save_path", !config.slot_save_path.is_empty())
            && !config.slot_save_path.is_empty()
        {
            cmd.extend_from_slice(&["--slot-save-path".into(), config.slot_save_path.clone()]);
        }
        if should_emit(
            config,
            "log_prompts_dir",
            !config.log_prompts_dir.is_empty(),
        ) && !config.log_prompts_dir.is_empty()
        {
            cmd.extend_from_slice(&["--log-prompts-dir".into(), config.log_prompts_dir.clone()]);
        }
        if should_emit(config, "slot_prompt_similarity", true) {
            cmd.extend_from_slice(&["-sps".into(), config.slot_prompt_similarity.to_string()]);
        }
        if should_emit(config, "prefill_assistant", true) {
            cmd.push(if config.prefill_assistant {
                "--prefill-assistant".into()
            } else {
                "--no-prefill-assistant".into()
            });
        }
    }
}

fn append_extended_server_flags(config: &InstanceConfig, is_emb: bool, cmd: &mut Vec<String>) {
    // New server features aligned with llama.cpp master.
    if should_emit(config, "rpc_servers", !config.rpc_servers.is_empty())
        && !config.rpc_servers.is_empty()
    {
        cmd.extend_from_slice(&["--rpc".into(), config.rpc_servers.clone()]);
    }
    if should_emit(config, "sse_ping_interval", config.sse_ping_interval != 30) {
        cmd.extend_from_slice(&[
            "--sse-ping-interval".into(),
            config.sse_ping_interval.to_string(),
        ]);
    }
    if should_emit(config, "reuse_port", config.reuse_port) && config.reuse_port {
        cmd.push("--reuse-port".into());
    }

    if !is_emb {
        // Multi-model and media.
        if should_emit(config, "models_dir", !config.models_dir.is_empty())
            && !config.models_dir.is_empty()
        {
            cmd.extend_from_slice(&["--models-dir".into(), config.models_dir.clone()]);
        }
        if should_emit(config, "models_preset", !config.models_preset.is_empty())
            && !config.models_preset.is_empty()
        {
            cmd.extend_from_slice(&["--models-preset".into(), config.models_preset.clone()]);
        }
        if should_emit(config, "models_max", true) {
            cmd.extend_from_slice(&["--models-max".into(), config.models_max.to_string()]);
        }
        if should_emit(config, "models_autoload", true) {
            cmd.push(if config.models_autoload {
                "--models-autoload".into()
            } else {
                "--no-models-autoload".into()
            });
        }
        if should_emit(config, "image_min_tokens", config.image_min_tokens > 0) {
            cmd.extend_from_slice(&[
                "--image-min-tokens".into(),
                config.image_min_tokens.to_string(),
            ]);
        }
        if should_emit(config, "image_max_tokens", config.image_max_tokens > 0) {
            cmd.extend_from_slice(&[
                "--image-max-tokens".into(),
                config.image_max_tokens.to_string(),
            ]);
        }
        if should_emit(
            config,
            "mtmd_batch_max_tokens",
            config.mtmd_batch_max_tokens != 1024,
        ) {
            cmd.extend_from_slice(&[
                "--mtmd-batch-max-tokens".into(),
                config.mtmd_batch_max_tokens.to_string(),
            ]);
        }
        if should_emit(config, "tags", !config.tags.is_empty()) && !config.tags.is_empty() {
            cmd.extend_from_slice(&["--tags".into(), config.tags.clone()]);
        }
        if should_emit(config, "media_path", !config.media_path.is_empty())
            && !config.media_path.is_empty()
        {
            cmd.extend_from_slice(&["--media-path".into(), config.media_path.clone()]);
        }
        if should_emit(config, "tools", !config.tools.is_empty()) && !config.tools.is_empty() {
            cmd.extend_from_slice(&["--tools".into(), config.tools.clone()]);
        }
    }

    // User-defined arguments are the final managed-mode escape hatch for every workload.
    if should_emit(config, "custom_args", !config.custom_args.is_empty()) {
        for arg in &config.custom_args {
            if !arg.is_empty() {
                cmd.extend(split_args(arg));
            }
        }
    }
}

fn generate_normalized_command(config: &InstanceConfig, engine_path: &str) -> Vec<String> {
    let exe = if engine_path.is_empty() {
        "llama-server".to_string()
    } else {
        engine_path.to_string()
    };
    let mut cmd = vec![exe, "-m".into(), config.model_path.clone()];
    let is_emb = config.embedding;

    append_basic_flags(config, is_emb, &mut cmd);
    append_context_flags(config, is_emb, &mut cmd);
    append_rope_flags(config, &mut cmd);
    append_flash_attention_flags(config, &mut cmd);
    append_memory_flags(config, &mut cmd);
    append_kv_cache_flags(config, is_emb, &mut cmd);
    append_device_flags(config, &mut cmd);
    append_speculative_flags(config, is_emb, &mut cmd);
    append_network_flags(config, is_emb, &mut cmd);
    append_workload_flags(config, &mut cmd);
    append_server_flags(config, is_emb, &mut cmd);
    append_extended_server_flags(config, is_emb, &mut cmd);

    cmd
}

fn canonical_override_key(key: &str) -> &str {
    match key {
        "gpu_layers_auto" => "gpu_layers",
        "ctx_size_auto" => "ctx_size",
        "mmproj_auto" | "no_mmproj" => "mmproj_mode",
        "numa" => "numa_mode",
        "fit" => "fit_mode",
        "kv_unified" => "kv_unified_mode",
        _ => key,
    }
}

fn emitted_override_keys(
    config: &InstanceConfig,
    engine_path: &str,
    capabilities: Option<&EngineCapabilities>,
    projected_command: &[String],
) -> Vec<String> {
    let Some(overrides) = config.explicit_overrides.as_ref() else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    overrides
        .iter()
        .map(|key| canonical_override_key(key).to_string())
        .filter(|key| seen.insert(key.clone()))
        .filter(|key| {
            let mut reduced = config.clone();
            reduced.explicit_overrides = Some(
                overrides
                    .iter()
                    .filter(|candidate| canonical_override_key(candidate) != key.as_str())
                    .cloned()
                    .collect(),
            );
            let reduced_command = generate_normalized_command(&reduced, engine_path);
            command_for_capabilities(&reduced_command, capabilities) != projected_command
        })
        .collect()
}

fn prepare_launch(
    mut config: InstanceConfig,
    engine_path: &str,
) -> (InstanceConfig, ModelWorkload, Vec<String>) {
    ensure_managed_public_model_alias(&mut config);
    let normalized = normalize_for_launch(config);
    let workload = normalized.workload;
    let config = normalized.into_config();
    let command = generate_normalized_command(&config, engine_path);
    (config, workload, command)
}

fn manual_option_value(arguments: &[String], names: &[&str]) -> AppResult<Option<String>> {
    let mut value = None;
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        if names.iter().any(|name| argument == name) {
            let Some(next) = arguments.get(index + 1) else {
                return Err(AppError::new(
                    "MANUAL_COMMAND_VALUE_MISSING",
                    format!("手动命令中的参数 {argument} 缺少取值。"),
                    false,
                ));
            };
            value = Some(next.clone());
            index += 2;
            continue;
        }
        for name in names {
            if let Some(inline) = argument.strip_prefix(&format!("{name}=")) {
                value = Some(inline.to_string());
            }
        }
        index += 1;
    }
    Ok(value)
}

fn same_executable(left: &str, right: &str) -> bool {
    let left_path = normalized_engine_path(left);
    let right_path = normalized_engine_path(right);
    if left_path == right_path {
        return true;
    }
    left_path
        .file_name()
        .and_then(|value| value.to_str())
        .zip(right_path.file_name().and_then(|value| value.to_str()))
        .map(|(left, right)| left.eq_ignore_ascii_case(right))
        .unwrap_or(false)
}

fn prepare_manual_launch(
    mut config: InstanceConfig,
    engine_path: &str,
) -> AppResult<(InstanceConfig, ModelWorkload, Vec<String>)> {
    let mut arguments = split_args_checked(config.manual_command.trim())
        .map_err(|message| AppError::new("MANUAL_COMMAND_PARSE_FAILED", message, false))?;
    if arguments.is_empty() {
        return Err(AppError::new(
            "MANUAL_COMMAND_EMPTY",
            "手动启动命令不能为空。",
            false,
        ));
    }

    if !arguments[0].starts_with('-') {
        let supplied_executable = arguments.remove(0);
        if !same_executable(&supplied_executable, engine_path) {
            return Err(AppError::new(
                "MANUAL_COMMAND_ENGINE_MISMATCH",
                "手动命令中的可执行文件与当前选中的引擎不一致。请删除命令开头的可执行文件，或改为当前引擎。",
                false,
            ));
        }
    }
    if arguments.is_empty() {
        return Err(AppError::new(
            "MANUAL_COMMAND_ARGUMENTS_EMPTY",
            "手动命令至少需要一个 llama-server 参数。",
            false,
        ));
    }

    config.host =
        manual_option_value(&arguments, &["--host"])?.unwrap_or_else(|| "127.0.0.1".to_string());
    if config.host.trim().is_empty() {
        return Err(AppError::new(
            "MANUAL_COMMAND_HOST_INVALID",
            "手动命令中的监听地址不能为空。",
            false,
        ));
    }
    let port = manual_option_value(&arguments, &["--port"])?.unwrap_or_else(|| "8080".to_string());
    config.port = port.parse::<u16>().map_err(|_| {
        AppError::new(
            "MANUAL_COMMAND_PORT_INVALID",
            format!("手动命令中的端口 {port} 无效，必须为 1 到 65535。"),
            false,
        )
    })?;
    if config.port == 0 {
        return Err(AppError::new(
            "MANUAL_COMMAND_PORT_INVALID",
            "手动命令中的端口必须为 1 到 65535。",
            false,
        ));
    }
    if let Some(model_path) = manual_option_value(&arguments, &["-m", "--model"])? {
        config.model_path = model_path;
    }
    if let Some(alias) = manual_option_value(&arguments, &["-a", "--alias"])? {
        config.alias = alias;
    }
    config.api_key = manual_option_value(&arguments, &["--api-key"])?.unwrap_or_default();
    config.api_key_file = manual_option_value(&arguments, &["--api-key-file"])?.unwrap_or_default();
    config.ssl_key_file = manual_option_value(&arguments, &["--ssl-key-file"])?.unwrap_or_default();
    config.ssl_cert_file =
        manual_option_value(&arguments, &["--ssl-cert-file"])?.unwrap_or_default();
    config.path_prefix = manual_option_value(&arguments, &["--path"])?.unwrap_or_default();
    config.api_prefix = manual_option_value(&arguments, &["--api-prefix"])?.unwrap_or_default();

    let reranking = arguments.iter().any(|argument| argument == "--reranking");
    let embedding = reranking || arguments.iter().any(|argument| argument == "--embedding");
    config.embedding = embedding;
    config.reranking = reranking;
    let workload = if reranking {
        ModelWorkload::Reranker
    } else if embedding {
        ModelWorkload::Embedding
    } else {
        ModelWorkload::Inference
    };

    let executable = if engine_path.trim().is_empty() {
        "llama-server".to_string()
    } else {
        engine_path.to_string()
    };
    let mut command = Vec::with_capacity(arguments.len() + 1);
    command.push(executable);
    command.extend(arguments);
    Ok((config, workload, command))
}

fn prepare_launch_checked(
    config: InstanceConfig,
    engine_path: &str,
) -> AppResult<(InstanceConfig, ModelWorkload, Vec<String>)> {
    if uses_manual_command(&config) {
        prepare_manual_launch(config, engine_path)
    } else {
        Ok(prepare_launch(config, engine_path))
    }
}

enum EngineCapabilityResolution {
    Available(Box<EngineCapabilities>),
    Missing,
    Stale,
}

fn normalized_engine_path(path: &str) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| std::path::PathBuf::from(path))
}

fn trusted_engine_capabilities(
    state: &AppState,
    config: &InstanceConfig,
    engine_exe: &str,
) -> EngineCapabilityResolution {
    let requested_path = normalized_engine_path(engine_exe);
    let mut engines = state.engines.lock().unwrap();
    let selected_index = if config.engine_id.trim().is_empty() {
        engines
            .iter()
            .position(|engine| normalized_engine_path(&engine.exe) == requested_path)
    } else {
        engines
            .iter()
            .position(|engine| engine.id == config.engine_id)
    };
    let Some(selected_index) = selected_index else {
        return EngineCapabilityResolution::Missing;
    };
    let engine = &mut engines[selected_index];
    if normalized_engine_path(&engine.exe) != requested_path {
        return EngineCapabilityResolution::Stale;
    }
    if engine.capabilities.executable_fingerprint.is_empty() {
        return EngineCapabilityResolution::Missing;
    }
    if capabilities_match_executable(engine_exe, &engine.capabilities) {
        return EngineCapabilityResolution::Available(Box::new(engine.capabilities.clone()));
    }
    engine.version.clear();
    engine.capabilities = EngineCapabilities {
        error: Some("engine executable changed; compatibility probe required".to_string()),
        ..EngineCapabilities::default()
    };
    let _ = crate::commands::model_inventory::update_engine_probe(engine);
    EngineCapabilityResolution::Stale
}

fn validate_configured_engine(
    state: &AppState,
    config: &InstanceConfig,
    engine_exe: &str,
) -> AppResult<()> {
    if config.engine_id.trim().is_empty() {
        return Ok(());
    }
    let requested_path = normalized_engine_path(engine_exe);
    let engines = state.engines.lock().unwrap();
    let Some(engine) = engines.iter().find(|engine| engine.id == config.engine_id) else {
        return Err(AppError::new(
            "CONFIGURED_ENGINE_NOT_FOUND",
            "实例配置引用的 llama-server 引擎已不存在，请重新选择引擎。",
            false,
        ));
    };
    if normalized_engine_path(&engine.exe) != requested_path {
        return Err(AppError::new(
            "CONFIGURED_ENGINE_MISMATCH",
            "请求启动的引擎与实例配置引用的引擎不一致。",
            false,
        ));
    }
    Ok(())
}

fn stale_engine_error() -> AppError {
    AppError::new(
        "ENGINE_CAPABILITIES_STALE",
        "llama-server 引擎文件已发生变化，必须重新探测版本与参数能力后才能继续。",
        true,
    )
}

fn validate_custom_argument_capabilities(
    config: &InstanceConfig,
    capabilities: Option<&EngineCapabilities>,
) -> AppResult<()> {
    if config.custom_args.is_empty() || capabilities.is_some_and(|value| value.status == "detected")
    {
        return Ok(());
    }
    Err(AppError::new(
        "ENGINE_CUSTOM_ARGUMENTS_REQUIRE_FULL_CAPABILITY_PROBE",
        "用户自定义参数不会被静默过滤。请先完成引擎参数能力探测；旧引擎请改用手动命令模式。",
        false,
    ))
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedServerCommand {
    command: Vec<String>,
    unsupported_flags: Vec<String>,
    emitted_override_keys: Vec<String>,
}

fn reserve_start_slot(
    running: &std::collections::HashMap<String, RunningInstance>,
    starting: &mut std::collections::HashSet<String>,
    instance_id: &str,
) -> Result<(), String> {
    if running.contains_key(instance_id) || !starting.insert(instance_id.to_string()) {
        return Err("Instance is already running or starting".to_string());
    }
    Ok(())
}

struct StartReservation<'a> {
    instance_id: String,
    starting: &'a Mutex<std::collections::HashSet<String>>,
}

impl Drop for StartReservation<'_> {
    fn drop(&mut self) {
        self.starting.lock().unwrap().remove(&self.instance_id);
    }
}

fn reserve_instance_start<'a>(
    state: &'a AppState,
    instance_id: &str,
) -> Result<StartReservation<'a>, String> {
    let running = state.running.lock().unwrap();
    let mut starting = state.starting.lock().unwrap();
    reserve_start_slot(&running, &mut starting, instance_id)?;
    Ok(StartReservation {
        instance_id: instance_id.to_string(),
        starting: &state.starting,
    })
}

pub async fn generate_server_command(
    config: InstanceConfig,
    engine_exe: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<GeneratedServerCommand> {
    validate_configured_engine(state.inner(), &config, &engine_exe)?;
    let manual = uses_manual_command(&config);
    let (config, _, command) = prepare_launch_checked(config, &engine_exe)?;
    validate_tls_configuration(&config)?;
    if manual {
        return Ok(GeneratedServerCommand {
            command,
            unsupported_flags: Vec::new(),
            emitted_override_keys: Vec::new(),
        });
    }
    let capabilities = match trusted_engine_capabilities(state.inner(), &config, &engine_exe) {
        EngineCapabilityResolution::Available(capabilities) => Some(capabilities),
        EngineCapabilityResolution::Missing => None,
        EngineCapabilityResolution::Stale => return Err(stale_engine_error()),
    };
    validate_custom_argument_capabilities(&config, capabilities.as_deref())?;
    let unsupported_flags = capabilities
        .as_deref()
        .map(|value| unsupported_command_flags(&command, value))
        .unwrap_or_default();
    let blocked = blocked_security_flags(&command, capabilities.as_deref());
    if !blocked.is_empty() {
        return Err(AppError::new(
            "ENGINE_SECURITY_PARAMETER_UNVERIFIED",
            format!(
                "当前引擎尚未确认支持以下安全参数：{}。请先完成引擎能力探测。",
                blocked.join(", ")
            ),
            false,
        ));
    }
    let command = command_for_capabilities(&command, capabilities.as_deref());
    let emitted_override_keys =
        emitted_override_keys(&config, &engine_exe, capabilities.as_deref(), &command);
    Ok(GeneratedServerCommand {
        command,
        unsupported_flags,
        emitted_override_keys,
    })
}

#[tauri::command]
pub async fn start_server(
    instance_id: String,
    config: InstanceConfig,
    engine_exe: String,
    engine_backend: String,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> AppResult<()> {
    validate_configured_engine(state.inner(), &config, &engine_exe)?;
    let _reservation = reserve_instance_start(state.inner(), &instance_id)?;
    let manual = uses_manual_command(&config);
    let (config, workload, generated_cmd) = prepare_launch_checked(config, &engine_exe)?;
    validate_tls_configuration(&config)?;
    let cmd = if manual {
        generated_cmd
    } else {
        let engine_capabilities =
            match trusted_engine_capabilities(state.inner(), &config, &engine_exe) {
                EngineCapabilityResolution::Available(capabilities) => Some(capabilities),
                EngineCapabilityResolution::Missing => None,
                EngineCapabilityResolution::Stale => return Err(stale_engine_error()),
            };
        validate_custom_argument_capabilities(&config, engine_capabilities.as_deref())?;
        if let Some(capabilities) = engine_capabilities.as_deref() {
            let unsupported = unsupported_command_flags(&generated_cmd, capabilities);
            if !unsupported.is_empty() {
                return Err(AppError::new(
                    "ENGINE_PARAMETER_UNSUPPORTED",
                    format!(
                        "当前 llama-server 不支持以下已启用参数：{}。请返回参数配置清除这些参数，或更换兼容的引擎版本。",
                        unsupported.join(", ")
                    ),
                    false,
                ));
            }
        }
        let blocked = blocked_security_flags(&generated_cmd, engine_capabilities.as_deref());
        if !blocked.is_empty() {
            return Err(AppError::new(
                "ENGINE_SECURITY_PARAMETER_UNVERIFIED",
                format!(
                    "当前引擎尚未确认支持以下安全参数：{}。请先完成引擎能力探测。",
                    blocked.join(", ")
                ),
                false,
            ));
        }
        command_for_capabilities(&generated_cmd, engine_capabilities.as_deref())
    };
    let cmd_display = format_command_for_display(&cmd);

    if crate::runtime_service::manages_instances() {
        let running =
            crate::runtime_service::start_instance(crate::runtime_service::RuntimeLaunchSpec {
                instance_id: instance_id.clone(),
                config: config.clone(),
                engine_backend: engine_backend.clone(),
                command: cmd.clone(),
                command_display: cmd_display.clone(),
                workload: workload.as_str().to_string(),
                working_directory: std::env::current_dir()
                    .ok()
                    .map(|path| path.to_string_lossy().to_string()),
            })
            .await
            .map_err(AppError::from)?;

        let previous_instance = {
            state
                .running
                .lock()
                .unwrap()
                .insert(instance_id.clone(), running.clone());
            let previous = state
                .instances
                .lock()
                .unwrap()
                .insert(instance_id.clone(), config.clone());
            state
                .runtime_managed_instances
                .lock()
                .unwrap()
                .insert(instance_id.clone());
            previous
        };
        let running_snapshot = state.running.lock().unwrap().clone();
        let persisted_instance_id = instance_id.clone();
        let persisted_config = config.clone();
        if let Err(error) = crate::commands::config::update_and_persist(&state, |global| {
            global.running = running_snapshot;
            global
                .instances
                .insert(persisted_instance_id, persisted_config);
        }) {
            let _ = crate::runtime_service::stop_instance(instance_id.clone()).await;
            state.running.lock().unwrap().remove(&instance_id);
            state
                .runtime_managed_instances
                .lock()
                .unwrap()
                .remove(&instance_id);
            let mut instances = state.instances.lock().unwrap();
            if let Some(previous) = previous_instance {
                instances.insert(instance_id.clone(), previous);
            } else {
                instances.remove(&instance_id);
            }
            return Err(AppError::new(
                "RUNTIME_STATE_PERSIST_FAILED",
                format!("后台运行时已回滚实例启动，因为配置持久化失败: {error}"),
                true,
            ));
        }

        app.emit(
            "server-started",
            serde_json::json!({
                "instanceId": instance_id,
                "pid": running.pid,
                "port": config.port,
                "host": config.host,
                "command": cmd_display,
                "effectiveConfig": {
                    "model_path": config.model_path,
                    "alias": config.alias,
                    "host": config.host,
                    "port": config.port,
                    "api_key": config.api_key,
                    "api_key_file": config.api_key_file,
                    "ssl_key_file": config.ssl_key_file,
                    "ssl_cert_file": config.ssl_cert_file,
                    "path_prefix": config.path_prefix,
                    "api_prefix": config.api_prefix,
                    "embedding": config.embedding,
                    "reranking": config.reranking,
                },
            }),
        )
        .ok();

        let config_dir = state.config_dir.lock().unwrap().clone();
        if register_restored_runtime_instance(&app, &running.instance_id, running.pid) {
            reconnect_runtime_instance_logs(&running.instance_id, running.pid, &config_dir, app);
        }
        return Ok(());
    }

    // Create a log file; llama-server writes here directly without pipe forwarding.
    let config_dir = state.config_dir.lock().unwrap().clone();
    let log_dir = config_dir.join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join(format!("{}.log", instance_id));
    let log_writer = Arc::new(
        CappedLogWriter::new(
            log_path.clone(),
            MAX_SERVER_LOG_BYTES,
            RETAINED_SERVER_LOG_BYTES,
        )
        .map_err(|e| format!("无法创建日志文件: {}", e))?,
    );

    let mut child = {
        let mut c = Command::new(&cmd[0]);
        c.args(&cmd[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x08000000);
        }
        c.spawn()
            .map_err(|e| format!("启动服务器失败: {}\n命令: {}", e, cmd_display))?
    };

    let pid = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Unable to capture server stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Unable to capture server stderr".to_string())?;
    let stdout_pump = spawn_log_pump(stdout, log_writer.clone());
    let stderr_pump = spawn_log_pump(stderr, log_writer.clone());

    let (start_time, executable_path) = match read_process_identity(pid) {
        Some(identity) => identity,
        None => {
            let message = "Unable to verify the started server process identity";
            let message = match terminate_spawned_child(&mut child) {
                Ok(()) => message.to_string(),
                Err(cleanup_error) => format!("{message}; {cleanup_error}"),
            };
            return Err(AppError::new("PROCESS_IDENTITY", message, false)
                .with_context("instanceId", instance_id));
        }
    };
    let telemetry_session_id = crate::commands::telemetry::begin_run_session(
        &instance_id,
        &config,
        &engine_backend,
        &telemetry_config_hash(&config),
        &cmd_display,
        workload,
    )
    .ok();

    // Atomic check-and-insert prevents starting the same instance twice.
    let duplicate_start = {
        let mut running = state.running.lock().unwrap();
        if running.contains_key(&instance_id) {
            true
        } else {
            running.insert(
                instance_id.clone(),
                RunningInstance {
                    instance_id: instance_id.clone(),
                    pid,
                    port: config.port,
                    host: config.host.clone(),
                    start_time,
                    executable_path: executable_path.to_string_lossy().to_string(),
                    telemetry_session_id: telemetry_session_id.clone(),
                    workload: workload.as_str().to_string(),
                    launch_config: Some(config.clone()),
                },
            );
            false
        }
    };
    if duplicate_start {
        let cleanup_result = terminate_spawned_child(&mut child);
        let _ = crate::commands::telemetry::finish_run_session(
            telemetry_session_id.as_deref(),
            None,
            "duplicate-start",
        );
        let message = match cleanup_result {
            Ok(()) => "该实例已在运行中".to_string(),
            Err(cleanup_error) => format!("该实例已在运行中；{cleanup_error}"),
        };
        return Err(AppError::new("INSTANCE_ALREADY_RUNNING", message, false)
            .with_context("instanceId", instance_id));
    }

    // Persist running state to disk immediately via unified atomic writes to avoid races.
    let previous_instance = state
        .instances
        .lock()
        .unwrap()
        .insert(instance_id.clone(), config.clone());
    let running_snapshot = state.running.lock().unwrap().clone();
    let persisted_instance_id = instance_id.clone();
    let persisted_config = config.clone();
    if let Err(persist_error) = crate::commands::config::update_and_persist(&state, |global| {
        global.running = running_snapshot;
        global
            .instances
            .insert(persisted_instance_id, persisted_config);
    }) {
        {
            let mut running = state.running.lock().unwrap();
            if running
                .get(&instance_id)
                .map(|current| current.pid == pid)
                .unwrap_or(false)
            {
                running.remove(&instance_id);
            }
        }
        {
            let mut instances = state.instances.lock().unwrap();
            if let Some(previous) = previous_instance {
                instances.insert(instance_id.clone(), previous);
            } else {
                instances.remove(&instance_id);
            }
        }
        let _ = crate::commands::telemetry::finish_run_session(
            telemetry_session_id.as_deref(),
            None,
            "persistence-failed",
        );
        let message = match terminate_spawned_child(&mut child) {
            Ok(()) => format!(
                "Server start was rolled back because runtime state could not be persisted: {persist_error}"
            ),
            Err(cleanup_error) => format!(
                "Runtime state persistence failed: {persist_error}; {cleanup_error}"
            ),
        };
        return Err(AppError::new("RUNTIME_STATE_PERSIST_FAILED", message, true)
            .with_context("instanceId", instance_id));
    }

    app.emit(
        "server-started",
        serde_json::json!({
            "instanceId": instance_id,
            "pid": pid,
            "port": config.port,
            "host": config.host,
            "command": cmd_display,
            "effectiveConfig": {
                "model_path": config.model_path,
                "alias": config.alias,
                "host": config.host,
                "port": config.port,
                "api_key": config.api_key,
                "api_key_file": config.api_key_file,
                "ssl_key_file": config.ssl_key_file,
                "ssl_cert_file": config.ssl_cert_file,
                "path_prefix": config.path_prefix,
                "api_prefix": config.api_prefix,
                "embedding": config.embedding,
                "reranking": config.reranking,
            },
        }),
    )
    .ok();

    // Log tail thread: read the log file written directly by llama-server.
    let app_tail = app.clone();
    let id_tail = instance_id.clone();
    let log_path_tail = log_path.clone();
    let telemetry_session_tail = telemetry_session_id.clone();
    std::thread::spawn(move || {
        tail_log_file(
            &log_path_tail,
            &id_tail,
            pid,
            telemetry_session_tail,
            workload,
            Some(log_writer),
            app_tail,
        );
    });

    // Process liveness monitoring and exit cleanup.
    let id = instance_id.clone();
    let app_clone = app.clone();
    let log_path_mon = log_path.clone();
    let telemetry_session_monitor = telemetry_session_id.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        match child.try_wait() {
            Ok(Some(status)) if !status.success() => {
                let _ = stdout_pump.join();
                let _ = stderr_pump.join();
                let st = app_clone.state::<AppState>();
                {
                    let mut r = st.running.lock().unwrap();
                    if r.get(&id).map(|ri| ri.pid == pid).unwrap_or(false) {
                        r.remove(&id);
                    }
                }
                {
                    let mut restored = st.restored_runtime_instances.lock().unwrap();
                    restored.remove(&format!("{}:{}", id, pid));
                }
                let _ = crate::commands::telemetry::finish_run_session(
                    telemetry_session_monitor.as_deref(),
                    status.code(),
                    "startup-failed",
                );
                if let Err(error) = crate::commands::config::update_and_persist(&st, |global| {
                    if global
                        .running
                        .get(&id)
                        .map(|current| current.pid == pid)
                        .unwrap_or(false)
                    {
                        global.running.remove(&id);
                    }
                }) {
                    eprintln!("Failed to persist startup failure for {id}: {error}");
                }
                // Emit captured stderr/log content so errors are never lost on quick exit
                if let Ok(log_content) = std::fs::read_to_string(&log_path_mon) {
                    if !log_content.trim().is_empty() {
                        let _ = app_clone.emit(
                            "server-log",
                            serde_json::json!({
                                "instanceId": id,
                                "text": log_content,
                            }),
                        );
                    }
                }
                let _ = app_clone.emit(
                    "server-error",
                    serde_json::json!({
                        "instanceId": id,
                        "error": format!("进程启动后立即退出 (exit code: {:?})", status.code()),
                    }),
                );
                return;
            }
            Ok(None) => {}
            _ => {}
        }

        let exit_code = child.wait().ok().and_then(|status| status.code());
        let _ = stdout_pump.join();
        let _ = stderr_pump.join();
        let st2 = app_clone.state::<AppState>();
        let removed = {
            let mut r = st2.running.lock().unwrap();
            if r.get(&id).map(|ri| ri.pid == pid).unwrap_or(false) {
                r.remove(&id).is_some()
            } else {
                false
            }
        };
        if removed {
            {
                let mut restored = st2.restored_runtime_instances.lock().unwrap();
                restored.remove(&format!("{}:{}", id, pid));
            }
            let _ = crate::commands::telemetry::finish_run_session(
                telemetry_session_monitor.as_deref(),
                exit_code,
                "process-exited",
            );
            if let Err(error) = crate::commands::config::update_and_persist(&st2, |global| {
                if global
                    .running
                    .get(&id)
                    .map(|current| current.pid == pid)
                    .unwrap_or(false)
                {
                    global.running.remove(&id);
                }
            }) {
                eprintln!("Failed to persist process exit for {id}: {error}");
            }
            let _ = app_clone.emit(
                "server-stopped",
                serde_json::json!({
                    "instanceId": id,
                    "expected": false,
                    "reason": "process-exited",
                    "exitCode": exit_code,
                }),
            );
        }
    });

    // Combined health and metrics supervisor.
    let id_metrics = instance_id.clone();
    let app_metrics = app.clone();
    let host_metrics = if config.host == "0.0.0.0" {
        "localhost".to_string()
    } else {
        config.host.clone()
    };
    let endpoint_base_metrics = crate::utils::service_url(
        effective_server_scheme(&config),
        &host_metrics,
        config.port,
        &config.api_prefix,
        "",
    );
    let api_key_metrics = effective_api_key(&config);
    let telemetry_session_metrics = telemetry_session_id.clone();
    std::thread::spawn(move || {
        monitor_loop(
            &id_metrics,
            pid,
            MonitorLoopConfig {
                endpoint_base: endpoint_base_metrics,
                api_key: api_key_metrics,
                telemetry_session_id: telemetry_session_metrics,
                workload,
            },
            app_metrics,
        );
    });

    Ok(())
}

/// Background metrics loop that samples every 5 seconds, pushes to the frontend, and records history.
struct MonitorLoopConfig {
    endpoint_base: String,
    api_key: String,
    telemetry_session_id: Option<String>,
    workload: ModelWorkload,
}

pub(crate) struct InstanceMonitorSample {
    pub ready: bool,
    pub system: SystemMetrics,
    pub llama: Option<crate::commands::telemetry::LlamaMetricSample>,
    pub slots: Option<Vec<crate::commands::telemetry::SlotSnapshotRecord>>,
}

pub(crate) fn collect_instance_monitor_sample(
    client: &reqwest::blocking::Client,
    endpoint_base: &str,
    api_key: &str,
    process_system: &mut System,
    pid: u32,
    uptime_secs: u64,
) -> InstanceMonitorSample {
    let authenticated_get = |url: &str| {
        let request = client.get(url).timeout(std::time::Duration::from_secs(2));
        if api_key.is_empty() {
            request
        } else {
            request.header("Authorization", format!("Bearer {api_key}"))
        }
    };
    let probe = |path: &str| {
        authenticated_get(&format!("{endpoint_base}{path}"))
            .send()
            .map(|response| response.status())
    };
    let ready = probe_status_is_success(&probe("/health").map_err(|error| error.to_string()))
        || probe_status_is_success(&probe("/v1/models").map_err(|error| error.to_string()));

    let gpu_system = collect_gpu_and_system();
    let (process_cpu, process_memory) = get_process_metrics(process_system, pid);
    let cpu_percent = gpu_system.cpu_percent.unwrap_or(process_cpu);
    let memory_mb = if gpu_system.cpu_percent.is_some() {
        gpu_system.memory_mb.unwrap_or(process_memory)
    } else {
        (process_memory * 10.0).round() / 10.0
    };
    let system = SystemMetrics {
        cpu_percent,
        memory_mb,
        uptime_secs,
        gpu_percent: gpu_system.gpu_percent,
        vram_used_mb: gpu_system.vram_used_mb,
        vram_total_mb: gpu_system.vram_total_mb,
        system_cpu_percent: gpu_system.system_cpu_percent,
        system_memory_total_mb: gpu_system.system_memory_total_mb,
        system_memory_used_mb: gpu_system.system_memory_used_mb,
        gpu_vendor: gpu_system.gpu_vendor,
        gpu_name: gpu_system.gpu_name,
    };

    let llama = authenticated_get(&format!("{endpoint_base}/metrics"))
        .send()
        .ok()
        .filter(|response| response.status().is_success())
        .and_then(|response| response.text().ok())
        .and_then(|body| parse_llama_metric_sample(&body).ok());
    let slots = authenticated_get(&format!("{endpoint_base}/slots"))
        .send()
        .ok()
        .filter(|response| response.status().is_success())
        .and_then(|response| response.json::<Vec<serde_json::Value>>().ok())
        .map(|values| {
            values
                .iter()
                .enumerate()
                .map(
                    |(index, value)| crate::commands::telemetry::SlotSnapshotRecord {
                        slot_id: value
                            .get("id")
                            .and_then(|item| item.as_u64())
                            .unwrap_or(index as u64) as u32,
                        is_processing: value
                            .get("is_processing")
                            .and_then(|item| item.as_bool())
                            .unwrap_or(false),
                        n_ctx: value
                            .get("n_ctx")
                            .and_then(|item| item.as_u64())
                            .unwrap_or(0) as u32,
                        n_past: value
                            .get("n_past")
                            .and_then(|item| item.as_u64())
                            .map(|item| item as u32),
                    },
                )
                .collect()
        });

    InstanceMonitorSample {
        ready,
        system,
        llama,
        slots,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HealthTransition {
    None,
    Ready,
    Failed,
}

pub(crate) fn advance_health_state(
    ready: bool,
    initial_grace_expired: bool,
    health_failures: &mut u32,
    last_health_ready: &mut Option<bool>,
) -> HealthTransition {
    if ready {
        *health_failures = 0;
        if *last_health_ready != Some(true) {
            *last_health_ready = Some(true);
            return HealthTransition::Ready;
        }
        return HealthTransition::None;
    }

    *health_failures = health_failures.saturating_add(1);
    let recovered_service_failed = *last_health_ready == Some(true);
    if *health_failures >= HEALTH_FAILURE_THRESHOLD
        && (recovered_service_failed || (*last_health_ready).is_none() && initial_grace_expired)
    {
        *last_health_ready = Some(false);
        *health_failures = 0;
        return HealthTransition::Failed;
    }
    HealthTransition::None
}

fn monitor_loop(
    instance_id: &str,
    expected_pid: u32,
    config: MonitorLoopConfig,
    app: tauri::AppHandle,
) {
    let MonitorLoopConfig {
        endpoint_base,
        api_key,
        telemetry_session_id,
        workload,
    } = config;
    crate::commands::monitoring::start_frame_loop(
        instance_id.to_string(),
        expected_pid,
        telemetry_session_id.clone(),
        workload,
        app.clone(),
    );
    // Wait for llama-server startup, giving it 3 seconds.
    std::thread::sleep(std::time::Duration::from_secs(3));

    let is_my_instance = || {
        let st = app.state::<AppState>();
        let guard = st.running.lock().unwrap();
        guard
            .get(instance_id)
            .map(|r| r.pid == expected_pid)
            .unwrap_or(false)
    };

    let client = reqwest::blocking::Client::new();

    // Startup timestamp used to compute uptime.
    let start_instant = std::time::Instant::now();

    // Dedicated System for process metrics; system/GPU metrics reuse the SYSINFO_CACHE singleton.
    let mut proc_sys = System::new_all();
    let mut health_failures = 0_u32;
    let mut last_health_ready: Option<bool> = None;

    loop {
        let iteration_started = std::time::Instant::now();
        if !is_my_instance() {
            break;
        }
        if !is_recorded_process_alive(expected_pid) {
            cleanup_running_instance(&app, instance_id, expected_pid, "process-exited");
            break;
        }

        let sample = collect_instance_monitor_sample(
            &client,
            &endpoint_base,
            &api_key,
            &mut proc_sys,
            expected_pid,
            start_instant.elapsed().as_secs(),
        );
        match advance_health_state(
            sample.ready,
            start_instant.elapsed() >= INITIAL_HEALTH_GRACE,
            &mut health_failures,
            &mut last_health_ready,
        ) {
            HealthTransition::Ready => {
                let _ = app.emit(
                    "health-status",
                    serde_json::json!({ "instanceId": instance_id, "status": "ok" }),
                );
            }
            HealthTransition::Failed => {
                let _ = app.emit(
                    "health-status",
                    serde_json::json!({ "instanceId": instance_id, "status": "fail" }),
                );
            }
            HealthTransition::None => {}
        }

        let _ = crate::commands::telemetry::record_metric_sample(
            telemetry_session_id.as_deref(),
            instance_id,
            &sample.system,
            sample.llama.as_ref(),
        );
        crate::commands::monitoring::update_metrics(
            instance_id,
            telemetry_session_id.as_deref(),
            workload,
            sample.system,
            sample.llama,
        );

        if let Some(slots) = sample.slots {
            crate::commands::monitoring::update_slots(
                instance_id,
                telemetry_session_id.as_deref(),
                workload,
                slots.len() as u64,
                slots.iter().filter(|slot| slot.is_processing).count() as u64,
            );
            let _ = crate::commands::telemetry::record_slot_snapshots(
                telemetry_session_id.as_deref(),
                instance_id,
                &slots,
            );
        }

        std::thread::sleep(
            std::time::Duration::from_secs(5).saturating_sub(iteration_started.elapsed()),
        );
    }
}

fn wait_for_recorded_process_exit(ri: &RunningInstance, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if !running_instance_matches_live_process(ri) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    !running_instance_matches_live_process(ri)
}

#[cfg(unix)]
fn terminate_unix_process(ri: &RunningInstance) -> bool {
    let pid = ri.pid.to_string();
    let term_sent = Command::new("kill")
        .args(["-TERM", &pid])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if term_sent && wait_for_recorded_process_exit(ri, std::time::Duration::from_secs(3)) {
        return true;
    }
    if !running_instance_matches_live_process(ri) {
        return true;
    }
    let kill_sent = Command::new("kill")
        .args(["-KILL", &pid])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    kill_sent && wait_for_recorded_process_exit(ri, std::time::Duration::from_secs(2))
}

pub(crate) fn terminate_running_instance(ri: &RunningInstance) -> bool {
    if !running_instance_matches_live_process(ri) {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        CommandExt::creation_flags(&mut Command::new("taskkill"), 0x08000000)
            .args(["/F", "/T", "/PID", &ri.pid.to_string()])
            .output()
            .map(|output| {
                (output.status.success()
                    && wait_for_recorded_process_exit(ri, std::time::Duration::from_secs(3)))
                    || !running_instance_matches_live_process(ri)
            })
            .unwrap_or_else(|_| !running_instance_matches_live_process(ri))
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        terminate_unix_process(ri)
    }
}

pub(crate) fn terminate_all_servers_for_exit(app: &tauri::AppHandle) -> Vec<String> {
    let state = app.state::<AppState>();
    let running = state.running.lock().unwrap().clone();
    let mut failures = Vec::new();

    for (instance_id, recorded) in running {
        if !terminate_running_instance(&recorded) {
            failures.push(format!("{instance_id} (PID {})", recorded.pid));
            continue;
        }

        let removed = {
            let mut current = state.running.lock().unwrap();
            if current
                .get(&instance_id)
                .is_some_and(|candidate| candidate.pid == recorded.pid)
            {
                current.remove(&instance_id)
            } else {
                None
            }
        };
        if let Some(removed) = removed {
            let _ = crate::commands::telemetry::finish_run_session(
                removed.telemetry_session_id.as_deref(),
                None,
                "app-exit",
            );
        }
    }

    let remaining_ids = state
        .running
        .lock()
        .unwrap()
        .keys()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    state
        .restored_runtime_instances
        .lock()
        .unwrap()
        .retain(|key| {
            key.split_once(':')
                .is_some_and(|(instance_id, _)| remaining_ids.contains(instance_id))
        });
    failures
}

pub async fn stop_server(
    instance_id: String,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let ri = state.running.lock().unwrap().get(&instance_id).cloned();
    if ri.is_none() {
        return Ok(());
    }
    if crate::runtime_service::manages_instances()
        && crate::runtime_service::is_instance_managed(&instance_id).await?
    {
        crate::runtime_service::stop_instance(instance_id.clone()).await?;
        let removed = state.running.lock().unwrap().remove(&instance_id);
        state
            .runtime_managed_instances
            .lock()
            .unwrap()
            .remove(&instance_id);
        {
            let mut restored = state.restored_runtime_instances.lock().unwrap();
            let prefix = format!("{}:", instance_id);
            restored.retain(|key| !key.starts_with(&prefix));
        }
        if let Some(removed) = removed {
            crate::commands::config::update_and_persist(&state, |global| {
                if global
                    .running
                    .get(&instance_id)
                    .is_some_and(|current| current.pid == removed.pid)
                {
                    global.running.remove(&instance_id);
                }
            })?;
        }
        app.emit(
            "server-stopped",
            serde_json::json!({
                "instanceId": instance_id,
                "expected": true,
                "reason": "manual-stop",
                "exitCode": null,
            }),
        )
        .ok();
        return Ok(());
    }
    state
        .runtime_managed_instances
        .lock()
        .unwrap()
        .remove(&instance_id);
    if let Some(ref ri) = ri {
        if !running_instance_matches_live_process(ri) {
            cleanup_running_instance(&app, &instance_id, ri.pid, "identity-mismatch");
            return Ok(());
        }
    }

    let killed = ri.as_ref().is_some_and(terminate_running_instance);

    if killed {
        let removed = {
            let mut running = state.running.lock().unwrap();
            if running
                .get(&instance_id)
                .is_some_and(|current| Some(current.pid) == ri.as_ref().map(|item| item.pid))
            {
                running.remove(&instance_id)
            } else {
                None
            }
        };
        if removed.is_none() {
            return Ok(());
        }
        {
            let mut restored = state.restored_runtime_instances.lock().unwrap();
            let prefix = format!("{}:", instance_id);
            restored.retain(|key| !key.starts_with(&prefix));
        }
        let _ = crate::commands::telemetry::finish_run_session(
            removed
                .as_ref()
                .and_then(|item| item.telemetry_session_id.as_deref()),
            None,
            "manual-stop",
        );
        let persist_result = if let Some(ref ri) = ri {
            crate::commands::config::update_and_persist(&state, |global| {
                if global
                    .running
                    .get(&instance_id)
                    .map(|current| current.pid == ri.pid)
                    .unwrap_or(false)
                {
                    global.running.remove(&instance_id);
                }
            })
        } else {
            Ok(())
        };
        app.emit(
            "server-stopped",
            serde_json::json!({
                "instanceId": instance_id,
                "expected": true,
                "reason": "manual-stop",
                "exitCode": null,
            }),
        )
        .ok();
        persist_result.map_err(|error| {
            format!("Server stopped, but its runtime state could not be persisted: {error}")
        })
    } else {
        // Process is still running; do not emit server-stopped so the frontend stays running.
        // The monitor thread cleans up when the process actually exits.
        Err("无法终止进程".into())
    }
}

fn is_recorded_process_alive(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            CloseHandle(handle);
            true
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
}

pub(crate) fn read_process_identity(pid: u32) -> Option<(u64, std::path::PathBuf)> {
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    let process = system.process(pid)?;
    Some((process.start_time(), process.exe()?.to_path_buf()))
}

fn normalized_executable_path(path: &std::path::Path) -> String {
    let normalized = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let value = normalized.to_string_lossy().to_string();
    #[cfg(target_os = "windows")]
    {
        value.to_lowercase()
    }
    #[cfg(not(target_os = "windows"))]
    {
        value
    }
}

fn process_identity_matches(
    running: &RunningInstance,
    actual_start_time: u64,
    actual_executable: &std::path::Path,
) -> bool {
    !running.executable_path.trim().is_empty()
        && running.start_time == actual_start_time
        && normalized_executable_path(std::path::Path::new(&running.executable_path))
            == normalized_executable_path(actual_executable)
}

pub(crate) fn running_instance_matches_live_process(running: &RunningInstance) -> bool {
    read_process_identity(running.pid)
        .map(|(start_time, executable)| process_identity_matches(running, start_time, &executable))
        .unwrap_or(false)
}

fn cleanup_running_instance(
    app: &tauri::AppHandle,
    instance_id: &str,
    expected_pid: u32,
    reason: &str,
) -> bool {
    let state = app.state::<AppState>();
    let removed = {
        let mut running = state.running.lock().unwrap();
        match running.get(instance_id) {
            Some(current) if current.pid == expected_pid => running.remove(instance_id),
            _ => None,
        }
    };
    let Some(ri) = removed else {
        return false;
    };
    {
        let mut restored = state.restored_runtime_instances.lock().unwrap();
        let prefix = format!("{}:", instance_id);
        restored.retain(|key| !key.starts_with(&prefix));
    }
    let _ = crate::commands::telemetry::finish_run_session(
        ri.telemetry_session_id.as_deref(),
        None,
        reason,
    );
    if let Err(error) = crate::commands::config::update_and_persist(&state, |global| {
        if global
            .running
            .get(instance_id)
            .map(|current| current.pid == expected_pid)
            .unwrap_or(false)
        {
            global.running.remove(instance_id);
        }
    }) {
        eprintln!("Failed to persist cleanup for {instance_id}: {error}");
    }
    let _ = app.emit(
        "server-stopped",
        serde_json::json!({
            "instanceId": instance_id,
            "reason": reason,
            "expected": false,
            "exitCode": null,
        }),
    );
    true
}

fn browser_url_for_host(
    host: &str,
    port: u16,
    use_tls: bool,
    api_prefix: &str,
) -> Result<String, String> {
    let normalized = if host == "0.0.0.0" || host == "::" {
        "localhost".to_string()
    } else {
        host.trim().to_string()
    };
    if normalized.is_empty() {
        return Err("Invalid host: empty".into());
    }
    if normalized.parse::<std::net::IpAddr>().is_ok() {
        return Ok(crate::utils::service_url(
            if use_tls { "https" } else { "http" },
            &normalized,
            port,
            api_prefix,
            "",
        ));
    }
    let valid_hostname = normalized
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.'))
        && normalized
            .split('.')
            .all(|label| !label.is_empty() && !label.starts_with('-') && !label.ends_with('-'));
    if !valid_hostname {
        return Err(format!("Invalid host: {}", host));
    }
    Ok(crate::utils::service_url(
        if use_tls { "https" } else { "http" },
        &normalized,
        port,
        api_prefix,
        "",
    ))
}

pub async fn open_browser(
    host: String,
    port: u16,
    use_tls: Option<bool>,
    api_prefix: Option<String>,
    instance_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let launch_config = instance_id.as_deref().and_then(|instance_id| {
        state
            .running
            .lock()
            .unwrap()
            .get(instance_id)
            .and_then(|running| running.launch_config.clone())
    });
    let url = browser_url_for_host(
        launch_config
            .as_ref()
            .map(|config| config.host.as_str())
            .unwrap_or(&host),
        launch_config
            .as_ref()
            .map(|config| config.port)
            .unwrap_or(port),
        launch_config
            .as_ref()
            .map(|config| effective_server_scheme(config) == "https")
            .unwrap_or_else(|| use_tls.unwrap_or(false)),
        launch_config
            .as_ref()
            .map(|config| config.api_prefix.as_str())
            .unwrap_or_else(|| api_prefix.as_deref().unwrap_or("")),
    )?;
    #[cfg(target_os = "windows")]
    {
        std::os::windows::process::CommandExt::creation_flags(
            &mut std::process::Command::new("explorer.exe"),
            0x08000000,
        )
        .arg(&url)
        .spawn()
        .map_err(|e| format!("{}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)] // Tauri expands the endpoint snapshot into IPC fields.
pub async fn test_connection(
    host: String,
    port: u16,
    api_key: Option<String>,
    api_key_file: Option<String>,
    use_tls: Option<bool>,
    api_prefix: Option<String>,
    instance_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let launch_config = instance_id.as_deref().and_then(|instance_id| {
        state
            .running
            .lock()
            .unwrap()
            .get(instance_id)
            .and_then(|running| running.launch_config.clone())
    });
    let effective_api_key = launch_config.as_ref().map(effective_api_key).or_else(|| {
        api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .and_then(|keys| {
                keys.split(',').find_map(|key| {
                    let key = key.trim();
                    (!key.is_empty()).then(|| key.to_string())
                })
            })
            .or_else(|| {
                api_key_file
                    .as_deref()
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .and_then(|path| {
                        std::fs::read_to_string(path).ok().and_then(|content| {
                            content.lines().find_map(|line| {
                                let key = line.trim().trim_start_matches('\u{feff}');
                                if key.is_empty() || key.starts_with('#') {
                                    None
                                } else {
                                    Some(key.to_string())
                                }
                            })
                        })
                    })
            })
    });
    let endpoint_host = launch_config
        .as_ref()
        .map(|config| config.host.as_str())
        .unwrap_or(&host);
    let endpoint_port = launch_config
        .as_ref()
        .map(|config| config.port)
        .unwrap_or(port);
    let connect_host = if endpoint_host == "0.0.0.0" || endpoint_host == "::" {
        "localhost"
    } else {
        endpoint_host
    };
    let scheme = launch_config
        .as_ref()
        .map(effective_server_scheme)
        .unwrap_or_else(|| {
            if use_tls.unwrap_or(false) {
                "https"
            } else {
                "http"
            }
        });
    let prefix = launch_config
        .as_ref()
        .map(|config| config.api_prefix.as_str())
        .unwrap_or_else(|| api_prefix.as_deref().unwrap_or(""));
    let health_url =
        crate::utils::service_url(scheme, connect_host, endpoint_port, prefix, "/health");
    let health_status = match http_get(&health_url, effective_api_key.as_deref())
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => Ok(resp.status()),
        Ok(resp) => Err(format!("HTTP {}", resp.status())),
        Err(e) => Err(e.to_string()),
    };

    if health_status.is_ok() {
        return classify_test_connection_result(health_status, Err("not checked".to_string()));
    }

    let models_url =
        crate::utils::service_url(scheme, connect_host, endpoint_port, prefix, "/v1/models");
    let models_status = match http_get(&models_url, effective_api_key.as_deref())
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => Ok(resp.status()),
        Ok(resp) => Err(format!("HTTP {}", resp.status())),
        Err(e) => Err(e.to_string()),
    };

    classify_test_connection_result(health_status, models_status)
}

fn classify_test_connection_result(
    health: Result<reqwest::StatusCode, String>,
    models: Result<reqwest::StatusCode, String>,
) -> Result<String, String> {
    if health.is_ok() {
        return Ok("✓ 连接成功，服务正常".into());
    }

    if models.is_ok() {
        return Ok(format!(
            "连接成功，OpenAI API 可访问（/health 返回：{}）",
            health.unwrap_err()
        ));
    }

    Err(format!(
        "服务健康检查失败：{}；/v1/models 也失败：{}",
        health.unwrap_err(),
        models.unwrap_err()
    ))
}

pub async fn check_port(port: u16, host: Option<String>) -> Result<bool, String> {
    let bind_host = host
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("127.0.0.1");
    match tokio::net::TcpListener::bind((bind_host, port)).await {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

// System performance metrics via sysinfo.

struct SystemSampler {
    system: System,
    snapshot: Option<(std::time::Instant, GpuSystemSnapshot)>,
}

static SYSINFO_CACHE: LazyLock<Mutex<SystemSampler>> = LazyLock::new(|| {
    Mutex::new(SystemSampler {
        system: System::new_all(),
        snapshot: None,
    })
});
#[derive(Clone)]
struct GpuSystemSnapshot {
    cpu_percent: Option<f32>,
    gpu_percent: Option<f32>,
    vram_used_mb: Option<f64>,
    vram_total_mb: Option<f64>,
    memory_mb: Option<f64>,
    gpu_vendor: Option<String>,
    gpu_name: Option<String>,
    system_cpu_percent: Option<f32>,
    system_memory_total_mb: Option<f64>,
    system_memory_used_mb: Option<f64>,
}

/// Collect GPU + system-level metrics. Uses cached System instance, no sleep.
fn collect_gpu_and_system() -> GpuSystemSnapshot {
    let mut guard = SYSINFO_CACHE.lock().unwrap();
    if let Some((sampled_at, snapshot)) = &guard.snapshot {
        if sampled_at.elapsed() < std::time::Duration::from_secs(4) {
            return snapshot.clone();
        }
    }
    guard.system.refresh_cpu_all();
    guard.system.refresh_memory();
    let (sys_cpu, sys_mem_total, sys_mem_used) = get_system_level_metrics(&guard.system);

    let snapshot = if let Some(m) = adlx::collect_metrics() {
        GpuSystemSnapshot {
            cpu_percent: m.cpu_percent,
            gpu_percent: m.gpu_percent,
            vram_used_mb: m.vram_used_mb,
            vram_total_mb: m.vram_total_mb,
            memory_mb: m.memory_mb,
            gpu_vendor: Some("AMD".into()),
            gpu_name: m.gpu_name,
            system_cpu_percent: sys_cpu,
            system_memory_total_mb: sys_mem_total,
            system_memory_used_mb: sys_mem_used,
        }
    } else if let Some(m) = nvml::collect_metrics() {
        GpuSystemSnapshot {
            cpu_percent: None,
            gpu_percent: m.gpu_percent,
            vram_used_mb: m.vram_used_mb,
            vram_total_mb: m.vram_total_mb,
            memory_mb: None,
            gpu_vendor: Some("NVIDIA".into()),
            gpu_name: m.gpu_name,
            system_cpu_percent: sys_cpu,
            system_memory_total_mb: sys_mem_total,
            system_memory_used_mb: sys_mem_used,
        }
    } else {
        GpuSystemSnapshot {
            cpu_percent: None,
            gpu_percent: None,
            vram_used_mb: None,
            vram_total_mb: None,
            memory_mb: None,
            gpu_vendor: None,
            gpu_name: None,
            system_cpu_percent: sys_cpu,
            system_memory_total_mb: sys_mem_total,
            system_memory_used_mb: sys_mem_used,
        }
    };
    guard.snapshot = Some((std::time::Instant::now(), snapshot.clone()));
    snapshot
}

/// #6: Reuse the System instance to avoid repeated new -> refresh -> sleep -> refresh 300ms stalls.
fn get_process_metrics(sys: &mut System, pid: u32) -> (f32, f64) {
    let pid = Pid::from_u32(pid);
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    sys.refresh_cpu_all();
    if let Some(p) = sys.process(pid) {
        let raw = p.cpu_usage();
        // Always normalize by CPU count because sysinfo reports total across all cores.
        let cpu = if sys.cpus().is_empty() {
            0.0
        } else {
            raw / sys.cpus().len() as f32
        };
        (cpu, p.memory() as f64 / (1024.0 * 1024.0))
    } else {
        (0.0, 0.0)
    }
}

fn get_system_level_metrics(sys: &System) -> (Option<f32>, Option<f64>, Option<f64>) {
    let cpu = {
        let usage = sys.global_cpu_usage();
        if usage > 0.0 {
            Some(usage)
        } else {
            None
        }
    };
    let total_mb = Some(sys.total_memory() as f64 / (1024.0 * 1024.0));
    let used_mb = Some(sys.used_memory() as f64 / (1024.0 * 1024.0));
    (cpu, total_mb, used_mb)
}

pub async fn get_system_metrics(
    instance_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<SystemMetrics, String> {
    let (pid, start_time) = {
        let running = state.running.lock().unwrap();
        let ri = running.get(&instance_id).ok_or("实例未运行")?;
        (ri.pid, ri.start_time)
    };
    let uptime = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        - start_time;

    let result = tokio::task::spawn_blocking(move || {
        let gpu_system = collect_gpu_and_system();
        let mut proc_sys = System::new_all();
        let (cpu, mem) = get_process_metrics(&mut proc_sys, pid);
        SystemMetrics {
            cpu_percent: cpu,
            memory_mb: (mem * 10.0).round() / 10.0,
            uptime_secs: uptime,
            gpu_percent: gpu_system.gpu_percent,
            vram_used_mb: gpu_system.vram_used_mb,
            vram_total_mb: gpu_system.vram_total_mb,
            system_cpu_percent: gpu_system.system_cpu_percent,
            system_memory_total_mb: gpu_system.system_memory_total_mb,
            system_memory_used_mb: gpu_system.system_memory_used_mb,
            gpu_vendor: gpu_system.gpu_vendor,
            gpu_name: gpu_system.gpu_name,
        }
    })
    .await
    .map_err(|e| format!("系统指标采集失败: {}", e))?;
    Ok(result)
}

/// System health without requiring an instance. Used by Dashboard for the system resource bar.
pub async fn get_system_health() -> Result<SystemMetrics, String> {
    let result = tokio::task::spawn_blocking(|| {
        let gpu_system = collect_gpu_and_system();
        SystemMetrics {
            cpu_percent: gpu_system
                .cpu_percent
                .unwrap_or(gpu_system.system_cpu_percent.unwrap_or(0.0)),
            memory_mb: gpu_system
                .memory_mb
                .unwrap_or(gpu_system.system_memory_used_mb.unwrap_or(0.0)),
            uptime_secs: 0,
            gpu_percent: gpu_system.gpu_percent,
            vram_used_mb: gpu_system.vram_used_mb,
            vram_total_mb: gpu_system.vram_total_mb,
            system_cpu_percent: gpu_system.system_cpu_percent,
            system_memory_total_mb: gpu_system.system_memory_total_mb,
            system_memory_used_mb: gpu_system.system_memory_used_mb,
            gpu_vendor: gpu_system.gpu_vendor,
            gpu_name: gpu_system.gpu_name,
        }
    })
    .await
    .map_err(|e| format!("系统指标采集失败: {}", e))?;
    Ok(result)
}

#[derive(serde::Serialize)]
pub struct SlotInfo {
    id: u32,
    is_processing: bool,
    n_ctx: u32,
}

fn http_get(url: &str, api_key: Option<&str>) -> reqwest::RequestBuilder {
    let req = SERVER_HTTP_CLIENT.get(url);
    if let Some(key) = api_key {
        req.header("Authorization", format!("Bearer {}", key))
    } else {
        req
    }
}

fn probe_status_is_success(status: &Result<reqwest::StatusCode, String>) -> bool {
    status
        .as_ref()
        .map(|status| status.is_success())
        .unwrap_or(false)
}

pub async fn get_slots(
    host: String,
    port: u16,
    api_key: Option<String>,
    use_tls: Option<bool>,
    api_prefix: Option<String>,
) -> Result<Vec<SlotInfo>, String> {
    let h = if host == "0.0.0.0" {
        "localhost"
    } else {
        &host
    };
    let url = crate::utils::service_url(
        if use_tls.unwrap_or(false) {
            "https"
        } else {
            "http"
        },
        h,
        port,
        api_prefix.as_deref().unwrap_or(""),
        "/slots",
    );
    let resp = http_get(&url, api_key.as_deref())
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;
    let arr: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
    Ok(arr
        .iter()
        .enumerate()
        .map(|(i, v)| SlotInfo {
            id: v.get("id").and_then(|s| s.as_u64()).unwrap_or(i as u64) as u32,
            is_processing: v
                .get("is_processing")
                .and_then(|s| s.as_bool())
                .unwrap_or(false),
            n_ctx: v.get("n_ctx").and_then(|s| s.as_u64()).unwrap_or(0) as u32,
        })
        .collect())
}

#[derive(serde::Serialize)]
pub struct MetricsInfo {
    pub tokens_per_sec: f64,
    pub prompt_tokens: u64,
    pub gen_tokens: u64,
    pub decode_calls_total: u64,
    pub max_tokens_observed: u64,
    pub prompt_tokens_per_sec: f64,
    pub requests_processing: u64,
    pub requests_deferred: u64,
    pub busy_slots_per_decode: f64,
}

pub async fn get_metrics(
    host: String,
    port: u16,
    api_key: Option<String>,
    use_tls: Option<bool>,
    api_prefix: Option<String>,
) -> Result<Option<MetricsInfo>, String> {
    let h = if host == "0.0.0.0" {
        "localhost"
    } else {
        &host
    };
    let url = crate::utils::service_url(
        if use_tls.unwrap_or(false) {
            "https"
        } else {
            "http"
        },
        h,
        port,
        api_prefix.as_deref().unwrap_or(""),
        "/metrics",
    );
    let resp = match http_get(&url, api_key.as_deref()).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) if r.status().as_u16() == 404 => return Ok(None),
        _ => return Ok(None),
    };
    let body = resp.text().await.map_err(|e| format!("读取失败: {}", e))?;
    let sample = parse_llama_metric_sample(&body)?;
    Ok(Some(MetricsInfo {
        tokens_per_sec: sample.tokens_per_sec,
        prompt_tokens: sample.prompt_tokens,
        gen_tokens: sample.gen_tokens,
        decode_calls_total: sample.decode_calls_total,
        max_tokens_observed: sample.max_tokens_observed,
        prompt_tokens_per_sec: sample.prompt_tokens_per_sec,
        requests_processing: sample.requests_processing,
        requests_deferred: sample.requests_deferred,
        busy_slots_per_decode: sample.busy_slots_per_decode,
    }))
}

fn parse_llama_metric_sample(
    body: &str,
) -> Result<crate::commands::telemetry::LlamaMetricSample, String> {
    let extract = |key: &str| -> Result<f64, String> {
        body.lines()
            .find(|line| line.starts_with(key))
            .and_then(|line| line.split_whitespace().last()?.parse().ok())
            .ok_or_else(|| format!("Missing or invalid llama metric: {key}"))
    };
    Ok(crate::commands::telemetry::LlamaMetricSample {
        tokens_per_sec: extract("llamacpp:predicted_tokens_seconds")?,
        prompt_tokens: extract("llamacpp:prompt_tokens_total")? as u64,
        gen_tokens: extract("llamacpp:tokens_predicted_total")? as u64,
        decode_calls_total: extract("llamacpp:n_decode_total")? as u64,
        max_tokens_observed: extract("llamacpp:n_tokens_max")? as u64,
        prompt_tokens_per_sec: extract("llamacpp:prompt_tokens_seconds")?,
        requests_processing: extract("llamacpp:requests_processing")? as u64,
        requests_deferred: extract("llamacpp:requests_deferred")? as u64,
        busy_slots_per_decode: extract("llamacpp:n_busy_slots_per_decode")?,
    })
}

// Restore logs and monitoring after app restart.

/// Reconnects log capture and metrics push for instances that are still running after app restart.
/// stdout/stderr pipes are unavailable after restart, so read from the log file in tail mode.
/// Also creates a new history session to continue recording.
pub fn register_restored_runtime_instance(
    app: &tauri::AppHandle,
    instance_id: &str,
    pid: u32,
) -> bool {
    let restore_key = format!("{}:{}", instance_id, pid);
    let state = app.state::<AppState>();
    let mut restored = state.restored_runtime_instances.lock().unwrap();
    let prefix = format!("{}:", instance_id);
    restored.retain(|key| !key.starts_with(&prefix) || key == &restore_key);
    restored.insert(restore_key)
}

pub fn reconnect_running_instance(
    instance_id: &str,
    pid: u32,
    config: &InstanceConfig,
    config_dir: &std::path::Path,
    app: tauri::AppHandle,
) {
    reconnect_instance_logs(instance_id, pid, config_dir, app.clone(), true);

    // Metrics recovery: restart monitor_loop and keep recording with the new session.
    {
        let app_metrics = app.clone();
        let id_metrics = instance_id.to_string();
        let host_m = if config.host == "0.0.0.0" {
            "localhost".to_string()
        } else {
            config.host.clone()
        };
        let endpoint_base = crate::utils::service_url(
            effective_server_scheme(config),
            &host_m,
            config.port,
            &config.api_prefix,
            "",
        );
        let ak = effective_api_key(config);
        let telemetry_session_id = crate::commands::telemetry::latest_open_session_id(&id_metrics)
            .ok()
            .flatten();
        let workload =
            crate::commands::telemetry::session_workload(telemetry_session_id.as_deref())
                .unwrap_or(ModelWorkload::Inference);
        std::thread::spawn(move || {
            monitor_loop(
                &id_metrics,
                pid,
                MonitorLoopConfig {
                    endpoint_base,
                    api_key: ak,
                    telemetry_session_id,
                    workload,
                },
                app_metrics,
            );
        });
    }
}

/// Replays and tails a runtime-service-owned instance log without starting a
/// second lifecycle or metrics monitor in the GUI process.
pub fn reconnect_runtime_instance_logs(
    instance_id: &str,
    pid: u32,
    config_dir: &std::path::Path,
    app: tauri::AppHandle,
) {
    reconnect_instance_logs(instance_id, pid, config_dir, app, false);
}

fn reconnect_instance_logs(
    instance_id: &str,
    pid: u32,
    config_dir: &std::path::Path,
    app: tauri::AppHandle,
    parse_performance: bool,
) {
    let log_path = config_dir.join("logs").join(format!("{}.log", instance_id));
    let id_log = instance_id.to_string();
    let telemetry_session_id = crate::commands::telemetry::latest_open_session_id(&id_log)
        .ok()
        .flatten();
    let workload = crate::commands::telemetry::session_workload(telemetry_session_id.as_deref())
        .unwrap_or(ModelWorkload::Inference);
    std::thread::spawn(move || {
        if parse_performance {
            tail_log_file(
                &log_path,
                &id_log,
                pid,
                telemetry_session_id,
                workload,
                None,
                app,
            );
        } else {
            tail_log_file_read_only(&log_path, &id_log, pid, app);
        }
    });
}

// Performance analysis parser.

use std::collections::HashMap;

/// Performance profile for one inference task.
#[derive(Clone, serde::Serialize)]
struct TaskPerfState {
    slot_id: u32,
    task_id: u32,
    started_at_ms: i64,
    updated_at_ms: i64,
    n_decoded: u64,
    tg: f64,
    tg_3s: Option<f64>,
    /// Historical (n_decoded, tg) samples used for speed curves.
    history: Vec<(u64, f64)>,
    // Final summary.
    prompt_tokens: Option<u64>,
    prompt_time_ms: Option<f64>,
    prompt_tps: Option<f64>,
    gen_tokens: Option<u64>,
    gen_time_ms: Option<f64>,
    gen_tps: Option<f64>,
    total_tokens: Option<u64>,
    total_time_ms: Option<f64>,
    input_tokens: Option<u64>,
    // Speculative decoding.
    spec_accept_rate: Option<f64>,
    spec_accepted: Option<u64>,
    spec_generated: Option<u64>,
    spec_gen_time_ms: Option<f64>,
    completed: bool,
}

#[derive(Debug, Clone)]
struct VectorTaskEvent {
    task_id: u32,
    workload: ModelWorkload,
    started_at: i64,
    completed_at: i64,
    duration_ms: f64,
    item_count: u64,
    input_tokens: Option<u64>,
}

/// Precompiled regex collection to avoid recompiling for every line.
struct PerfParser {
    workload: ModelWorkload,
    re_ids: regex_lite::Regex,
    re_launch: regex_lite::Regex,
    re_release: regex_lite::Regex,
    re_decoded: regex_lite::Regex,
    re_tg_3s: regex_lite::Regex,
    re_prompt: regex_lite::Regex,
    re_eval: regex_lite::Regex,
    re_total: regex_lite::Regex,
    re_draft: regex_lite::Regex,
    re_stats: regex_lite::Regex,
    re_stop_tokens: regex_lite::Regex,
}

impl PerfParser {
    fn new(workload: ModelWorkload) -> Self {
        PerfParser {
            workload,
            re_ids: regex_lite::Regex::new(r"id\s+(\d+)\s*\|\s*task\s+(\d+)").unwrap(),
            re_launch: regex_lite::Regex::new(r"launch_slot_.*processing\s+task").unwrap(),
            re_release: regex_lite::Regex::new(r"slot\s+release.*id\s+\d+\s*\|\s*task\s+\d+\s*\|\s*stop").unwrap(),
            re_decoded: regex_lite::Regex::new(r"n_decoded\s*=\s*(\d+).*?tg\s*=\s*([\d.]+)\s*t/s").unwrap(),
            re_tg_3s: regex_lite::Regex::new(r"tg_3s\s*=\s*([\d.]+)\s*t/s").unwrap(),
            re_prompt: regex_lite::Regex::new(r"prompt eval time\s*=\s*([\d.]+)\s*ms\s*/\s*(\d+)\s*tokens.*?,\s*([\d.]+)\s*tokens per second\)").unwrap(),
            re_eval: regex_lite::Regex::new(r"eval time\s*=\s*([\d.]+)\s*ms\s*/\s*(\d+)\s*tokens.*?,\s*([\d.]+)\s*tokens per second\)").unwrap(),
            re_total: regex_lite::Regex::new(r"total time\s*=\s*([\d.]+)\s*ms\s*/\s*(\d+)\s*tokens").unwrap(),
            re_draft: regex_lite::Regex::new(r"draft acceptance\s*=\s*([\d.]+)\s*\(\s*(\d+)\s*accepted\s*/\s*(\d+)\s*generated\)").unwrap(),
            re_stats: regex_lite::Regex::new(r"statistics\s+draft-mtp.*?#gen tokens\s*=\s*(\d+).*?#acc tokens\s*=\s*(\d+).*?dur\(g\)\s*=\s*([\d.]+)").unwrap(),
            re_stop_tokens: regex_lite::Regex::new(
                r"stop processing:\s*n_tokens\s*=\s*(\d+)",
            )
            .unwrap(),
        }
    }

    fn extract_ids(&self, line: &str) -> Option<(u32, u32)> {
        self.re_ids.captures(line).and_then(|c| {
            let slot = c.get(1)?.as_str().parse::<u32>().ok()?;
            let task = c.get(2)?.as_str().parse::<u32>().ok()?;
            Some((slot, task))
        })
    }

    fn is_launch(&self, line: &str) -> bool {
        self.re_launch.is_match(line)
    }
    fn is_release(&self, line: &str) -> bool {
        self.re_release.is_match(line)
    }

    fn vector_event(&self, task: &TaskPerfState) -> Option<VectorTaskEvent> {
        if !self.workload.is_vector() || !task.completed || task.input_tokens.is_none() {
            return None;
        }
        Some(VectorTaskEvent {
            task_id: task.task_id,
            workload: self.workload,
            started_at: task.started_at_ms,
            completed_at: task.updated_at_ms,
            duration_ms: task.updated_at_ms.saturating_sub(task.started_at_ms) as f64,
            item_count: 1,
            input_tokens: task.input_tokens,
        })
    }
}

fn persist_completed_task(
    telemetry_session_id: Option<&str>,
    parser: &PerfParser,
    task: &TaskPerfState,
) -> Result<(), String> {
    if let Some(event) = parser.vector_event(task) {
        let record = crate::commands::telemetry::VectorActivityRecord {
            source: crate::commands::vector_metrics::VectorEventSource::Log,
            source_event_id: i64::from(event.task_id),
            workload: event.workload,
            endpoint: None,
            started_at: event.started_at,
            completed_at: event.completed_at,
            duration_ms: event.duration_ms,
            item_count: event.item_count,
            input_tokens: event.input_tokens,
            http_status: None,
            error_text: None,
        };
        crate::commands::telemetry::record_vector_activity(telemetry_session_id, &record)?;
        return Ok(());
    }
    if parser.workload.is_vector() {
        return Ok(());
    }

    let record = crate::commands::telemetry::InferenceRequestRecord {
        task_id: task.task_id,
        slot_id: task.slot_id,
        prompt_tokens: task.prompt_tokens,
        prompt_time_ms: task.prompt_time_ms,
        prompt_tps: task.prompt_tps,
        generated_tokens: task.gen_tokens,
        generation_time_ms: task.gen_time_ms,
        generation_tps: task.gen_tps,
        total_tokens: task.total_tokens,
        total_time_ms: task.total_time_ms,
        spec_accept_rate: task.spec_accept_rate,
        spec_accepted: task.spec_accepted,
        spec_generated: task.spec_generated,
        spec_gen_time_ms: task.spec_gen_time_ms,
    };
    crate::commands::telemetry::record_inference_request(telemetry_session_id, &record)
}

/// Parses one log line and updates task performance state. Returns whether perf-update should be emitted.
fn parse_perf_line(
    parser: &PerfParser,
    line: &str,
    tasks: &mut HashMap<u32, TaskPerfState>,
    last_completed: &mut Option<TaskPerfState>,
) -> bool {
    let Some((slot_id, task_id)) = parser.extract_ids(line) else {
        return false;
    };

    // Task creation.
    if parser.is_launch(line) {
        let timestamp = crate::commands::telemetry::current_time_ms();
        if tasks.len() >= MAX_TRACKED_PERF_TASKS {
            if let Some(oldest_task_id) = tasks
                .values()
                .min_by_key(|task| task.updated_at_ms)
                .map(|task| task.task_id)
            {
                tasks.remove(&oldest_task_id);
            }
        }
        tasks.insert(
            task_id,
            TaskPerfState {
                slot_id,
                task_id,
                started_at_ms: timestamp,
                updated_at_ms: timestamp,
                n_decoded: 0,
                tg: 0.0,
                tg_3s: None,
                history: Vec::new(),
                prompt_tokens: None,
                prompt_time_ms: None,
                prompt_tps: None,
                gen_tokens: None,
                gen_time_ms: None,
                gen_tps: None,
                total_tokens: None,
                total_time_ms: None,
                input_tokens: None,
                spec_accept_rate: None,
                spec_accepted: None,
                spec_generated: None,
                spec_gen_time_ms: None,
                completed: false,
            },
        );
        return true;
    }

    let task = match tasks.get_mut(&task_id) {
        Some(t) => t,
        None => return false, // stale line from before app started
    };
    task.updated_at_ms = crate::commands::telemetry::current_time_ms();

    // Progress update.
    if let Some(c) = parser.re_decoded.captures(line) {
        if let (Ok(n), Ok(tg)) = (
            c.get(1).unwrap().as_str().parse::<u64>(),
            c.get(2).unwrap().as_str().parse::<f64>(),
        ) {
            task.n_decoded = n;
            task.tg = tg;
            task.tg_3s = parser
                .re_tg_3s
                .captures(line)
                .and_then(|captures| captures.get(1))
                .and_then(|value| value.as_str().parse::<f64>().ok());
            if task.history.len() < 500 {
                task.history.push((n, tg));
            }
            return true;
        }
    }

    // Prompt phase summary.
    if let Some(c) = parser.re_prompt.captures(line) {
        task.prompt_time_ms = c.get(1).and_then(|m| m.as_str().parse::<f64>().ok());
        task.prompt_tokens = c.get(2).and_then(|m| m.as_str().parse::<u64>().ok());
        task.prompt_tps = c.get(3).and_then(|m| m.as_str().parse::<f64>().ok());
        return true;
    }

    // Generation phase summary.
    if let Some(c) = parser.re_eval.captures(line) {
        task.gen_time_ms = c.get(1).and_then(|m| m.as_str().parse::<f64>().ok());
        task.gen_tokens = c.get(2).and_then(|m| m.as_str().parse::<u64>().ok());
        task.gen_tps = c.get(3).and_then(|m| m.as_str().parse::<f64>().ok());
        return true;
    }

    // Total timing.
    if let Some(c) = parser.re_total.captures(line) {
        task.total_time_ms = c.get(1).and_then(|m| m.as_str().parse::<f64>().ok());
        task.total_tokens = c.get(2).and_then(|m| m.as_str().parse::<u64>().ok());
        return true;
    }

    // Speculative decoding.
    if let Some(c) = parser.re_draft.captures(line) {
        task.spec_accept_rate = c.get(1).and_then(|m| m.as_str().parse::<f64>().ok());
        task.spec_accepted = c.get(2).and_then(|m| m.as_str().parse::<u64>().ok());
        task.spec_generated = c.get(3).and_then(|m| m.as_str().parse::<u64>().ok());
        return true;
    }

    // Detailed draft-mtp statistics.
    if let Some(c) = parser.re_stats.captures(line) {
        task.spec_generated = c
            .get(1)
            .and_then(|m| m.as_str().parse::<u64>().ok())
            .or(task.spec_generated);
        task.spec_accepted = c
            .get(2)
            .and_then(|m| m.as_str().parse::<u64>().ok())
            .or(task.spec_accepted);
        task.spec_gen_time_ms = c.get(3).and_then(|m| m.as_str().parse::<f64>().ok());
        return true;
    }

    // Task completion.
    if parser.is_release(line) {
        task.input_tokens = parser
            .re_stop_tokens
            .captures(line)
            .and_then(|captures| captures.get(1))
            .and_then(|value| value.as_str().parse::<u64>().ok());
        task.completed = true;
        *last_completed = Some(task.clone());
        return true;
    }

    false
}

pub(crate) struct RuntimePerfTracker {
    instance_id: String,
    telemetry_session_id: Option<String>,
    workload: ModelWorkload,
    parser: PerfParser,
    tasks: HashMap<u32, TaskPerfState>,
    last_completed: Option<TaskPerfState>,
    last_recorded_task_id: Option<u32>,
}

impl RuntimePerfTracker {
    pub(crate) fn new(
        instance_id: String,
        telemetry_session_id: Option<String>,
        workload: ModelWorkload,
    ) -> Self {
        Self {
            instance_id,
            telemetry_session_id,
            workload,
            parser: PerfParser::new(workload),
            tasks: HashMap::new(),
            last_completed: None,
            last_recorded_task_id: None,
        }
    }

    fn update_live_tasks(&self) {
        let active = self
            .tasks
            .values()
            .filter(|task| !task.completed)
            .collect::<Vec<_>>();
        let throughput = if self.workload == ModelWorkload::Inference {
            active
                .iter()
                .map(|task| task.tg_3s.unwrap_or(task.tg).max(0.0))
                .sum()
        } else {
            active
                .iter()
                .filter_map(|task| task.prompt_tps)
                .map(|value| value.max(0.0))
                .sum()
        };
        crate::commands::monitoring::update_tasks(
            &self.instance_id,
            self.telemetry_session_id.as_deref(),
            self.workload,
            active.len() as u64,
            throughput,
        );
    }

    pub(crate) fn process_line(&mut self, line: &str) {
        if !parse_perf_line(
            &self.parser,
            line,
            &mut self.tasks,
            &mut self.last_completed,
        ) {
            return;
        }
        if let Some(task) = self.last_completed.as_ref() {
            if self.last_recorded_task_id != Some(task.task_id) {
                if let Some(event) = self.parser.vector_event(task) {
                    crate::commands::monitoring::record_vector_activity(
                        &self.instance_id,
                        self.telemetry_session_id.as_deref(),
                        event.workload,
                        crate::commands::monitoring::VectorMetricSource::Log,
                        event.completed_at,
                        event.item_count,
                        event.input_tokens,
                        event.duration_ms,
                        true,
                    );
                }
                if let Err(error) =
                    persist_completed_task(self.telemetry_session_id.as_deref(), &self.parser, task)
                {
                    eprintln!(
                        "Runtime telemetry warning for {}: {error}",
                        self.instance_id
                    );
                }
                self.last_recorded_task_id = Some(task.task_id);
            }
        }
        self.tasks.retain(|_, task| !task.completed);
        self.update_live_tasks();
    }

    pub(crate) fn finish(&mut self) {
        self.tasks.clear();
        crate::commands::monitoring::update_tasks(
            &self.instance_id,
            self.telemetry_session_id.as_deref(),
            self.workload,
            0,
            0.0,
        );
    }

    pub(crate) fn snapshot(&self) -> serde_json::Value {
        let active = self
            .tasks
            .values()
            .filter(|task| !task.completed)
            .collect::<Vec<_>>();
        serde_json::json!({
            "instanceId": self.instance_id,
            "tasks": active,
            "lastCompleted": self.last_completed,
        })
    }
}

/// Reads existing content from the log file and tails new lines through server-log events.
/// Mirrors the behavior of Docker `docker logs -f` or systemd `journalctl -f`.
fn read_log_tail(path: &std::path::Path, max_lines: usize) -> std::io::Result<(Vec<String>, u64)> {
    let mut file = std::fs::File::open(path)?;
    let file_len = file.metadata()?.len();
    read_log_tail_from_file(&mut file, file_len, max_lines)
}

fn read_log_tail_from_file(
    file: &mut std::fs::File,
    file_len: u64,
    max_lines: usize,
) -> std::io::Result<(Vec<String>, u64)> {
    let min_offset = file_len.saturating_sub(LOG_REPLAY_MAX_BYTES);
    let mut offset = file_len;
    let mut chunks = Vec::new();
    let mut newline_count = 0_usize;
    while offset > min_offset && newline_count <= max_lines {
        let chunk_start = offset.saturating_sub(64 * 1024).max(min_offset);
        let chunk_len = usize::try_from(offset - chunk_start).unwrap_or(0);
        if chunk_len == 0 {
            break;
        }
        let mut chunk = vec![0_u8; chunk_len];
        file.seek(SeekFrom::Start(chunk_start))?;
        file.read_exact(&mut chunk)?;
        newline_count += chunk.iter().filter(|byte| **byte == b'\n').count();
        chunks.push(chunk);
        offset = chunk_start;
    }
    chunks.reverse();
    let bytes = chunks.into_iter().flatten().collect::<Vec<_>>();
    let valid_start = if offset > 0 {
        bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(bytes.len())
    } else {
        0
    };
    let complete_end = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|index| index + 1)
        .filter(|end| *end >= valid_start)
        .unwrap_or(valid_start);
    let text = String::from_utf8_lossy(&bytes[valid_start..complete_end]);
    let mut lines = text
        .lines()
        .rev()
        .take(max_lines)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    lines.reverse();
    Ok((lines, offset.saturating_add(complete_end as u64)))
}

fn emit_log_batches(app: &tauri::AppHandle, instance_id: &str, lines: &[String]) {
    for chunk in lines.chunks(LOG_EVENT_BATCH_SIZE) {
        let _ = app.emit(
            "server-log-batch",
            serde_json::json!({
                "instanceId": instance_id,
                "lines": chunk,
            }),
        );
    }
}

fn tail_log_file_read_only(
    log_path: &std::path::Path,
    instance_id: &str,
    expected_pid: u32,
    app: tauri::AppHandle,
) {
    let mut last_size = 0_u64;
    let mut pending = Vec::new();
    if log_path.exists() {
        if let Ok((lines, replay_end)) = read_log_tail(log_path, LOG_REPLAY_LINES) {
            last_size = replay_end;
            let replay_lines = lines
                .into_iter()
                .filter(|line| !line.trim().is_empty())
                .map(|line| format!("{line}\n"))
                .collect::<Vec<_>>();
            emit_log_batches(&app, instance_id, &replay_lines);
        }
    }

    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let current_size = match std::fs::metadata(log_path) {
            Ok(metadata) => metadata.len(),
            Err(_) => 0,
        };
        if current_size < last_size {
            pending.clear();
            last_size = current_size;
        }
        if current_size > last_size {
            let mut bytes = Vec::new();
            if let Ok(mut file) = std::fs::File::open(log_path) {
                if file.seek(SeekFrom::Start(last_size)).is_ok() {
                    let _ = Read::take(&mut file, current_size - last_size).read_to_end(&mut bytes);
                }
            }
            last_size = current_size;
            let lines = take_complete_log_lines(&mut pending, &bytes)
                .into_iter()
                .filter(|line| !line.trim().is_empty())
                .map(|line| format!("{line}\n"))
                .collect::<Vec<_>>();
            emit_log_batches(&app, instance_id, &lines);
        }

        let still_running = {
            let state = app.state::<AppState>();
            let running = state
                .running
                .lock()
                .unwrap()
                .get(instance_id)
                .is_some_and(|running| running.pid == expected_pid);
            running
        };
        if !still_running {
            if !pending.is_empty() {
                let line = String::from_utf8_lossy(&pending).trim().to_string();
                if !line.is_empty() {
                    emit_log_batches(&app, instance_id, &[format!("{line}\n")]);
                }
            }
            break;
        }
    }
}

fn tail_log_file(
    log_path: &std::path::Path,
    instance_id: &str,
    expected_pid: u32,
    telemetry_session_id: Option<String>,
    workload: ModelWorkload,
    log_writer: Option<Arc<CappedLogWriter>>,
    app: tauri::AppHandle,
) {
    let parser = PerfParser::new(workload);
    let mut tasks: HashMap<u32, TaskPerfState> = HashMap::new();
    let mut last_completed: Option<TaskPerfState> = None;
    let mut last_recorded_task_id: Option<u32> = None;

    let record_completed = |last: &Option<TaskPerfState>, recorded: &mut Option<u32>| {
        let Some(task) = last else {
            return;
        };
        if *recorded == Some(task.task_id) {
            return;
        }
        if let Some(event) = parser.vector_event(task) {
            crate::commands::monitoring::record_vector_activity(
                instance_id,
                telemetry_session_id.as_deref(),
                event.workload,
                crate::commands::monitoring::VectorMetricSource::Log,
                event.completed_at,
                event.item_count,
                event.input_tokens,
                event.duration_ms,
                true,
            );
        }
        if let Err(error) = persist_completed_task(telemetry_session_id.as_deref(), &parser, task) {
            let _ = app.emit(
                "server-log",
                serde_json::json!({
                    "instanceId": instance_id,
                    "text": format!("Telemetry warning: {error}\n"),
                }),
            );
        }
        *recorded = Some(task.task_id);
    };

    let emit_perf = |app: &tauri::AppHandle,
                     tasks: &HashMap<u32, TaskPerfState>,
                     last: &Option<TaskPerfState>| {
        let active: Vec<&TaskPerfState> = tasks.values().filter(|t| !t.completed).collect();
        let throughput = if workload == ModelWorkload::Inference {
            active
                .iter()
                .map(|task| task.tg_3s.unwrap_or(task.tg).max(0.0))
                .sum()
        } else {
            active
                .iter()
                .filter_map(|task| task.prompt_tps)
                .map(|value| value.max(0.0))
                .sum()
        };
        crate::commands::monitoring::update_tasks(
            instance_id,
            telemetry_session_id.as_deref(),
            workload,
            active.len() as u64,
            throughput,
        );
        let _ = app.emit(
            "perf-update",
            serde_json::json!({
                "instanceId": instance_id,
                "tasks": active,
                "lastCompleted": last,
            }),
        );
    };

    // Phase 1: replay only the tail needed by the frontend cap.
    let mut last_size = 0;
    let mut observed_generation = 0;
    let mut pending = Vec::new();
    if log_path.exists() {
        let replay = if let Some(writer) = log_writer.as_ref() {
            writer.read_tail(LOG_REPLAY_LINES)
        } else {
            read_log_tail(log_path, LOG_REPLAY_LINES)
                .map(|(lines, offset)| (lines, offset, observed_generation))
        };
        if let Ok((lines, replay_end, generation)) = replay {
            last_size = replay_end;
            observed_generation = generation;
            let replay_lines = lines
                .iter()
                .filter(|line| !line.trim().is_empty())
                .map(|line| format!("{line}\n"))
                .collect::<Vec<_>>();
            emit_log_batches(&app, instance_id, &replay_lines);
            for line in &lines {
                if line.trim().is_empty() {
                    continue;
                }
                if parse_perf_line(&parser, line, &mut tasks, &mut last_completed) {
                    record_completed(&last_completed, &mut last_recorded_task_id);
                }
            }
            // Clean up completed tasks from replay
            tasks.retain(|_, t| !t.completed);
        }
    }
    emit_perf(&app, &tasks, &last_completed);

    // Phase 2: continuously tail new content.
    // Wait briefly for in-flight writes to finish so half-lines are avoided.
    std::thread::sleep(std::time::Duration::from_millis(200));

    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));

        let read_chunk = if let Some(writer) = log_writer.as_ref() {
            match writer.read_since(last_size, observed_generation) {
                Ok(chunk) => chunk,
                Err(_) => break,
            }
        } else {
            let current_size = match std::fs::metadata(log_path) {
                Ok(meta) => meta.len(),
                Err(_) => break,
            };
            if current_size < last_size {
                pending.clear();
                last_size = current_size;
            }
            let mut bytes = Vec::new();
            if current_size > last_size {
                if let Ok(mut file) = std::fs::File::open(log_path) {
                    if file.seek(SeekFrom::Start(last_size)).is_ok() {
                        let available = current_size.saturating_sub(last_size);
                        let _ = file.take(available).read_to_end(&mut bytes);
                    }
                }
            }
            LogReadChunk {
                bytes,
                end_offset: current_size,
                generation: observed_generation,
                rotated: false,
            }
        };

        if read_chunk.rotated {
            debug_assert_ne!(observed_generation, read_chunk.generation);
            pending.clear();
        }
        last_size = read_chunk.end_offset;
        observed_generation = read_chunk.generation;

        if !read_chunk.bytes.is_empty() {
            let mut perf_changed = false;
            let mut log_lines = Vec::new();
            for text in take_complete_log_lines(&mut pending, &read_chunk.bytes) {
                if text.trim().is_empty() {
                    continue;
                }
                log_lines.push(format!("{text}\n"));
                if parse_perf_line(&parser, &text, &mut tasks, &mut last_completed) {
                    perf_changed = true;
                    record_completed(&last_completed, &mut last_recorded_task_id);
                }
            }
            emit_log_batches(&app, instance_id, &log_lines);
            if perf_changed {
                tasks.retain(|_, t| !t.completed);
                emit_perf(&app, &tasks, &last_completed);
            }
        }

        let still_running = {
            let st = app.state::<AppState>();
            let guard = st.running.lock().unwrap();
            guard
                .get(instance_id)
                .map(|running| running.pid == expected_pid)
                .unwrap_or(false)
        };
        if !still_running {
            if !pending.is_empty() {
                let text = String::from_utf8_lossy(&pending).trim().to_string();
                if !text.is_empty() {
                    emit_log_batches(&app, instance_id, &[format!("{text}\n")]);
                    if parse_perf_line(&parser, &text, &mut tasks, &mut last_completed) {
                        record_completed(&last_completed, &mut last_recorded_task_id);
                        tasks.retain(|_, task| !task.completed);
                        emit_perf(&app, &tasks, &last_completed);
                    }
                }
                pending.clear();
            }
            break;
        }
    }
    crate::commands::monitoring::update_tasks(
        instance_id,
        telemetry_session_id.as_deref(),
        workload,
        0,
        0.0,
    );
    emit_perf(&app, &HashMap::new(), &last_completed);
}

/// Split custom argument strings while preserving ordinary Windows path separators.
fn split_args(input: &str) -> Vec<String> {
    split_args_checked(input).unwrap_or_default()
}

fn split_args_checked(input: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut token_started = false;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                token_started = true;
            }
            '\\' => {
                token_started = true;
                let mut count = 1;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    count += 1;
                }
                if chars.peek() == Some(&'"') {
                    for _ in 0..count / 2 {
                        current.push('\\');
                    }
                    chars.next();
                    if count % 2 == 0 {
                        in_quotes = !in_quotes;
                    } else {
                        current.push('"');
                    }
                } else {
                    for _ in 0..count {
                        current.push('\\');
                    }
                }
            }
            ' ' | '\t' | '\r' | '\n' if !in_quotes => {
                if token_started {
                    args.push(std::mem::take(&mut current));
                    token_started = false;
                }
            }
            _ => {
                token_started = true;
                current.push(ch);
            }
        }
    }
    if in_quotes {
        return Err("手动命令包含未闭合的双引号。".to_string());
    }
    if token_started {
        args.push(current);
    }
    Ok(args)
}

#[cfg(test)]
mod perf_parser_tests {
    use super::*;

    fn collect_vector_events(workload: ModelWorkload, lines: &[&str]) -> Vec<VectorTaskEvent> {
        let parser = PerfParser::new(workload);
        let mut tasks = HashMap::new();
        let mut last_completed = None;
        let mut last_event_id = None;
        let mut events = Vec::new();
        for line in lines {
            if !parse_perf_line(&parser, line, &mut tasks, &mut last_completed) {
                continue;
            }
            let Some(task) = last_completed.as_ref() else {
                continue;
            };
            if last_event_id == Some(task.task_id) {
                continue;
            }
            if let Some(event) = parser.vector_event(task) {
                last_event_id = Some(task.task_id);
                events.push(event);
            }
        }
        events
    }

    fn polluted_embedding_config() -> InstanceConfig {
        InstanceConfig {
            model_path: "C:/models/bge-small.gguf".into(),
            flash_attn: "on".into(),
            spec_type: "draft-mtp".into(),
            custom_args: vec!["--temp 1.5".into()],
            ..InstanceConfig::default()
        }
    }

    fn has_flag_value(command: &[String], flag: &str, value: &str) -> bool {
        command
            .windows(2)
            .any(|arguments| arguments[0] == flag && arguments[1] == value)
    }

    #[test]
    fn custom_argument_splitter_handles_escaped_quotes_and_windows_paths() {
        assert_eq!(
            split_args(r#"--prompt "say \"hello\"" --model C:\models\chat.gguf"#),
            vec![
                "--prompt",
                "say \"hello\"",
                "--model",
                r"C:\models\chat.gguf",
            ]
        );
        assert_eq!(split_args(r#"--value "a\\b""#), vec!["--value", r"a\\b"]);
        assert_eq!(
            split_args(r#"--model "\\server\share\chat.gguf""#),
            vec!["--model", r"\\server\share\chat.gguf"]
        );
    }

    #[test]
    fn tracked_defaults_emit_only_application_required_arguments() {
        let config = InstanceConfig {
            model_path: "C:/models/chat.gguf".into(),
            explicit_overrides: Some(Vec::new()),
            ..InstanceConfig::default()
        };

        assert_eq!(
            generate_normalized_command(&config, "llama-server"),
            vec![
                "llama-server",
                "-m",
                "C:/models/chat.gguf",
                "--host",
                "127.0.0.1",
                "--port",
                "8080",
                "--metrics",
                "--props",
                "--slots",
            ]
        );
    }

    #[test]
    fn managed_launch_emits_a_safe_alias_when_the_user_left_it_empty() {
        let config = InstanceConfig {
            name: "Public model".into(),
            model_path: r"C:\private\models\model.gguf".into(),
            explicit_overrides: Some(Vec::new()),
            ..InstanceConfig::default()
        };

        let (normalized, _, command) = prepare_launch(config, "llama-server");

        assert_eq!(normalized.alias, "Public model");
        assert!(normalized
            .explicit_overrides
            .as_ref()
            .is_some_and(|fields| fields.iter().any(|field| field == "alias")));
        assert!(has_flag_value(&command, "-a", "Public model"));
        assert!(!command
            .iter()
            .any(|argument| argument.contains("private") && argument != &normalized.model_path));
    }

    #[test]
    fn tracked_speculative_command_omits_inherited_children() {
        let config = InstanceConfig {
            model_path: "model.gguf".into(),
            spec_type: "draft-mtp".into(),
            draft_tokens: 2,
            explicit_overrides: Some(vec!["spec_type".into(), "draft_tokens".into()]),
            ..InstanceConfig::default()
        };

        let command = generate_normalized_command(&config, "llama-server");
        assert!(has_flag_value(&command, "--spec-type", "draft-mtp"));
        assert!(has_flag_value(&command, "--spec-draft-n-max", "2"));
        for inherited in [
            "-ngld",
            "--spec-draft-n-min",
            "--spec-draft-p-min",
            "--spec-draft-p-split",
        ] {
            assert!(!command.iter().any(|argument| argument == inherited));
        }
    }

    #[test]
    fn tracked_default_valued_and_sentinel_overrides_are_preserved() {
        let config = InstanceConfig {
            model_path: "model.gguf".into(),
            temp: 0.8,
            n_predict: -1,
            explicit_overrides: Some(vec!["temp".into(), "n_predict".into()]),
            ..InstanceConfig::default()
        };
        let command = generate_normalized_command(&config, "llama-server");
        assert!(has_flag_value(&command, "--temp", "0.8"));
        assert!(has_flag_value(&command, "-n", "-1"));
    }

    #[test]
    fn tracked_positive_and_negative_boolean_choices_are_both_explicit() {
        let override_keys = vec![
            "no_mmap".into(),
            "no_repack".into(),
            "no_kv_offload".into(),
            "no_mmproj_offload".into(),
            "no_ui".into(),
            "perf".into(),
            "spec_type".into(),
            "spec_draft_backend_sampling".into(),
        ];
        let positive = InstanceConfig {
            model_path: "model.gguf".into(),
            spec_type: "draft-mtp".into(),
            explicit_overrides: Some(override_keys.clone()),
            ..InstanceConfig::default()
        };
        let positive_command = generate_normalized_command(&positive, "llama-server");
        for flag in [
            "--mmap",
            "--repack",
            "--kv-offload",
            "--mmproj-offload",
            "--ui",
            "--no-perf",
            "--spec-draft-backend-sampling",
        ] {
            assert!(
                positive_command.iter().any(|argument| argument == flag),
                "missing positive/default-valued pin {flag}: {positive_command:?}"
            );
        }

        let negative = InstanceConfig {
            no_mmap: true,
            no_repack: true,
            no_kv_offload: true,
            no_mmproj_offload: true,
            no_ui: true,
            perf: true,
            spec_draft_backend_sampling: false,
            ..positive
        };
        let negative_command = generate_normalized_command(&negative, "llama-server");
        for flag in [
            "--no-mmap",
            "--no-repack",
            "--no-kv-offload",
            "--no-mmproj-offload",
            "--no-ui",
            "--perf",
            "--no-spec-draft-backend-sampling",
        ] {
            assert!(
                negative_command.iter().any(|argument| argument == flag),
                "missing inverse pin {flag}: {negative_command:?}"
            );
        }
    }

    #[test]
    fn projector_offload_is_inactive_only_when_no_projector_can_be_used() {
        let disabled_auto = InstanceConfig {
            model_path: "model.gguf".into(),
            mmproj_mode: "off".into(),
            no_mmproj_offload: true,
            explicit_overrides: Some(vec!["mmproj_mode".into(), "no_mmproj_offload".into()]),
            ..InstanceConfig::default()
        };
        let without_projector = generate_normalized_command(&disabled_auto, "llama-server");
        assert!(!without_projector
            .iter()
            .any(|argument| argument == "--no-mmproj-offload"));

        let with_projector = InstanceConfig {
            mmproj_path: "projector.gguf".into(),
            explicit_overrides: Some(vec![
                "mmproj_path".into(),
                "mmproj_mode".into(),
                "no_mmproj_offload".into(),
            ]),
            ..disabled_auto
        };
        let with_projector_command = generate_normalized_command(&with_projector, "llama-server");
        assert!(with_projector_command
            .iter()
            .any(|argument| argument == "--no-mmproj-offload"));
    }

    #[test]
    fn tracked_dependent_parameter_does_not_activate_its_controller() {
        let config = InstanceConfig {
            model_path: "model.gguf".into(),
            spec_type: "draft-mtp".into(),
            spec_draft_n_min: 1,
            explicit_overrides: Some(vec!["spec_draft_n_min".into()]),
            ..InstanceConfig::default()
        };
        let command = generate_normalized_command(&config, "llama-server");
        assert!(!command.iter().any(|argument| argument == "--spec-type"));
        assert!(!command
            .iter()
            .any(|argument| argument == "--spec-draft-n-min"));
    }

    #[test]
    fn custom_arguments_are_never_silently_projected_from_incomplete_capabilities() {
        let config = InstanceConfig {
            custom_args: vec!["--future-flag 1".into()],
            ..InstanceConfig::default()
        };
        let partial = EngineCapabilities {
            status: "partial".into(),
            ..EngineCapabilities::default()
        };
        let detected = EngineCapabilities {
            status: "detected".into(),
            ..EngineCapabilities::default()
        };

        assert!(validate_custom_argument_capabilities(&config, None).is_err());
        assert!(validate_custom_argument_capabilities(&config, Some(&partial)).is_err());
        assert!(validate_custom_argument_capabilities(&config, Some(&detected)).is_ok());
    }

    #[test]
    fn emitted_override_metadata_tracks_only_arguments_that_reach_the_final_command() {
        let config = InstanceConfig {
            model_path: "model.gguf".into(),
            temp: 0.6,
            metrics: true,
            spec_type: "none".into(),
            spec_draft_n_min: 2,
            kv_unified: true,
            kv_unified_mode: "on".into(),
            explicit_overrides: Some(vec![
                "temp".into(),
                "metrics".into(),
                "spec_draft_n_min".into(),
                "kv_unified".into(),
                "kv_unified_mode".into(),
                "temp".into(),
            ]),
            ..InstanceConfig::default()
        };
        let command = generate_normalized_command(&config, "llama-server");
        let capabilities = EngineCapabilities {
            status: "detected".into(),
            ..EngineCapabilities::default()
        };
        let keys = emitted_override_keys(&config, "llama-server", Some(&capabilities), &command);

        assert_eq!(keys, vec!["temp", "kv_unified_mode"]);
        assert!(command.iter().any(|value| value == "--metrics"));
        assert!(command.iter().any(|value| value == "--kv-unified"));
        assert!(!command.iter().any(|value| value == "--spec-draft-n-min"));

        let conservative = command_for_capabilities(&command, None);
        assert!(emitted_override_keys(&config, "llama-server", None, &conservative).is_empty());
    }

    #[test]
    fn emitted_override_metadata_collapses_automatic_value_aliases() {
        let config = InstanceConfig {
            model_path: "model.gguf".into(),
            gpu_layers: 7,
            gpu_layers_auto: false,
            ctx_size: 4096,
            ctx_size_auto: false,
            explicit_overrides: Some(vec![
                "gpu_layers".into(),
                "gpu_layers_auto".into(),
                "ctx_size".into(),
                "ctx_size_auto".into(),
            ]),
            ..InstanceConfig::default()
        };
        let command = generate_normalized_command(&config, "llama-server");
        let capabilities = EngineCapabilities {
            status: "detected".into(),
            ..EngineCapabilities::default()
        };

        assert_eq!(
            emitted_override_keys(&config, "llama-server", Some(&capabilities), &command),
            vec!["gpu_layers", "ctx_size"]
        );
    }

    #[test]
    fn manual_command_uses_selected_executable_and_syncs_runtime_metadata() {
        let engine = r"C:\Program Files\llama\llama-server.exe";
        let config = InstanceConfig {
            launch_mode: "manual".into(),
            manual_command: format!(
                "\"{engine}\" -m \"C:\\models\\embed model.gguf\"\n--host 0.0.0.0 --port=9001 --api-key secret --embedding"
            ),
            ..InstanceConfig::default()
        };

        let (effective, workload, command) = prepare_launch_checked(config, engine).unwrap();
        assert_eq!(command[0], engine);
        assert!(has_flag_value(
            &command,
            "-m",
            r"C:\models\embed model.gguf"
        ));
        assert_eq!(effective.host, "0.0.0.0");
        assert_eq!(effective.port, 9001);
        assert_eq!(effective.api_key, "secret");
        assert_eq!(workload, ModelWorkload::Embedding);
        let display = format_command_for_display(&command);
        assert!(display.contains("--api-key ********"));
        assert!(!display.contains("--api-key secret"));
    }

    #[test]
    fn manual_command_rejects_mismatched_executable_and_unclosed_quotes() {
        let mismatched = InstanceConfig {
            launch_mode: "manual".into(),
            manual_command: r#""C:\other\llama-server.exe" -m model.gguf"#.into(),
            ..InstanceConfig::default()
        };
        assert!(prepare_launch_checked(mismatched, r"C:\selected\different-server.exe").is_err());

        let malformed = InstanceConfig {
            launch_mode: "manual".into(),
            manual_command: r#"-m "model.gguf"#.into(),
            ..InstanceConfig::default()
        };
        assert!(prepare_launch_checked(malformed, "llama-server").is_err());

        let zero_port = InstanceConfig {
            launch_mode: "manual".into(),
            manual_command: "-m model.gguf --port 0".into(),
            ..InstanceConfig::default()
        };
        assert!(prepare_launch_checked(zero_port, "llama-server").is_err());
    }

    #[test]
    fn non_sampling_application_defaults_remain_explicit() {
        let command = generate_normalized_command(&InstanceConfig::default(), "llama-server");

        for flag in [
            "--jinja",
            "--warmup",
            "-cb",
            "--prefill-assistant",
            "--models-autoload",
            "--cache-prompt",
            "--cache-idle-slots",
            "--no-context-shift",
            "--slots",
        ] {
            assert!(
                command.iter().any(|argument| argument == flag),
                "default command is missing explicit {flag}: {command:?}"
            );
        }
        for (flag, value) in [("-cram", "8192"), ("-cms", "8192"), ("--models-max", "4")] {
            assert!(
                has_flag_value(&command, flag, value),
                "default command is missing explicit {flag} {value}: {command:?}"
            );
        }

        for flag in ["-n", "--temp", "--top-k", "--top-p", "--min-p"] {
            assert!(
                !command.iter().any(|argument| argument == flag),
                "default sampling flag should be omitted: {flag}: {command:?}"
            );
        }
    }

    #[test]
    fn explicit_zero_negative_and_default_true_overrides_reach_llama_server() {
        let config = InstanceConfig {
            model_path: "C:/models/chat.gguf".into(),
            reasoning_preserve: "off".into(),
            jinja: false,
            cont_batching: false,
            keep: -1,
            cache_ram: 0,
            warmup: false,
            checkpoint_min_step: 0,
            numa_mode: "isolate".into(),
            fit_mode: "off".into(),
            kv_unified_mode: "off".into(),
            mmproj_mode: "off".into(),
            spec_type: "draft-simple".into(),
            draft_gpu_layers: 0,
            cors_origins: "https://example.test".into(),
            cors_methods: "GET, POST".into(),
            cors_headers: "Authorization, Content-Type".into(),
            cors_credentials: "off".into(),
            n_predict: 0,
            temp: 0.0,
            top_k: 0,
            min_p: 0.0,
            presence_penalty: -0.5,
            frequency_penalty: -0.25,
            repeat_last_n: 0,
            dry_multiplier: 1.0,
            dry_penalty_last_n: 0,
            adaptive_target: 0.0,
            prefill_assistant: false,
            log_prompts_dir: "C:/logs/prompts".into(),
            models_autoload: false,
            ..InstanceConfig::default()
        };
        let command = generate_normalized_command(&config, "llama-server");

        for flag in [
            "--no-reasoning-preserve",
            "--no-jinja",
            "--no-cont-batching",
            "--no-warmup",
            "--no-cors-credentials",
            "--no-kv-unified",
            "--no-mmproj",
            "--no-prefill-assistant",
            "--no-models-autoload",
        ] {
            assert!(
                command.iter().any(|argument| argument == flag),
                "missing {flag}"
            );
        }
        for (flag, value) in [
            ("--keep", "-1"),
            ("-cram", "0"),
            ("-cms", "0"),
            ("--numa", "isolate"),
            ("--fit", "off"),
            ("-ngld", "0"),
            ("--cors-origins", "https://example.test"),
            ("--cors-methods", "GET, POST"),
            ("--cors-headers", "Authorization, Content-Type"),
            ("-n", "0"),
            ("--temp", "0"),
            ("--top-k", "0"),
            ("--min-p", "0"),
            ("--presence-penalty", "-0.5"),
            ("--frequency-penalty", "-0.25"),
            ("--repeat-last-n", "0"),
            ("--dry-penalty-last-n", "0"),
            ("--adaptive-target", "0"),
            ("--log-prompts-dir", "C:/logs/prompts"),
        ] {
            assert!(
                has_flag_value(&command, flag, value),
                "missing {flag} {value}: {command:?}"
            );
        }
    }

    #[test]
    fn embedding_normalization_accepts_all_official_normalization_modes() {
        let command = generate_normalized_command(
            &InstanceConfig {
                model_path: "C:/models/embedding.gguf".into(),
                embedding: true,
                embd_normalize: -1,
                ..InstanceConfig::default()
            },
            "llama-server",
        );

        assert!(has_flag_value(&command, "--embd-normalize", "-1"));
    }

    #[test]
    fn command_preview_uses_launch_normalization() {
        let (_, _, cmd) = prepare_launch(polluted_embedding_config(), "");

        assert!(cmd.iter().any(|arg| arg == "--embedding"));
        assert!(cmd.windows(2).any(|args| args == ["-fa", "on"]));
        assert!(!cmd.iter().any(|arg| arg == "--spec-type"));
        assert!(has_flag_value(&cmd, "--temp", "1.5"));
    }

    #[test]
    fn start_preparation_hashes_and_launches_the_normalized_config() {
        let (config, workload, cmd) = prepare_launch(polluted_embedding_config(), "");

        assert_eq!(workload, crate::vector_policy::ModelWorkload::Embedding);
        assert!(config.embedding);
        assert!(config.spec_type.is_empty());
        assert_eq!(config.custom_args, vec!["--temp 1.5"]);
        assert!(cmd.iter().any(|arg| arg == "--embedding"));
        assert!(cmd.windows(2).any(|args| args == ["-fa", "on"]));
        assert!(!cmd.iter().any(|arg| arg == "--spec-type"));
        assert!(has_flag_value(&cmd, "--temp", "1.5"));
        assert_eq!(
            telemetry_config_hash(&config),
            telemetry_config_hash(&normalize_for_launch(config.clone()).config)
        );
    }

    #[test]
    fn llama_metrics_use_unambiguous_decode_names() {
        let sample = parse_llama_metric_sample(
            "llamacpp:predicted_tokens_seconds 12.5\n\
             llamacpp:prompt_tokens_total 120\n\
             llamacpp:tokens_predicted_total 80\n\
             llamacpp:n_decode_total 9\n\
             llamacpp:n_tokens_max 4096\n\
             llamacpp:prompt_tokens_seconds 48.0\n\
             llamacpp:requests_processing 2\n\
             llamacpp:requests_deferred 1\n\
             llamacpp:n_busy_slots_per_decode 1.5\n",
        )
        .unwrap();

        assert_eq!(sample.decode_calls_total, 9);
        assert_eq!(sample.max_tokens_observed, 4096);
        assert_eq!(sample.tokens_per_sec, 12.5);
        assert_eq!(sample.prompt_tokens_per_sec, 48.0);
    }

    #[test]
    fn malformed_llama_metrics_are_not_reported_as_zero() {
        let error =
            parse_llama_metric_sample("llamacpp:predicted_tokens_seconds nope").unwrap_err();
        assert!(error.contains("Missing or invalid llama metric"));
    }

    #[test]
    fn start_slot_rejects_duplicate_reservations_and_recovers_after_release() {
        let running = HashMap::new();
        let mut starting = std::collections::HashSet::new();

        assert!(reserve_start_slot(&running, &mut starting, "instance-a").is_ok());
        assert!(reserve_start_slot(&running, &mut starting, "instance-a").is_err());
        starting.remove("instance-a");
        assert!(reserve_start_slot(&running, &mut starting, "instance-a").is_ok());
    }

    #[test]
    fn normalized_embedding_generator_rejects_context_shift() {
        let config = InstanceConfig {
            embedding: true,
            context_shift: true,
            ..InstanceConfig::default()
        };

        let cmd = generate_normalized_command(&config, "");

        assert!(!cmd.iter().any(|arg| arg == "--context-shift"));
    }

    #[test]
    fn command_groups_preserve_workload_and_device_boundaries() {
        struct Case {
            name: &'static str,
            config: InstanceConfig,
            required: &'static [&'static str],
            forbidden: &'static [&'static str],
        }

        let cases = [
            Case {
                name: "inference-vulkan-speculative",
                config: InstanceConfig {
                    model_path: "C:/models/chat.gguf".into(),
                    device: "Vulkan0".into(),
                    spec_type: "draft".into(),
                    draft_model_path: "C:/models/draft.gguf".into(),
                    custom_args: vec!["--temp 0.7".into()],
                    ..InstanceConfig::default()
                },
                required: &["-dev", "Vulkan0", "--spec-type", "draft", "--temp", "0.7"],
                forbidden: &["--embedding", "--reranking"],
            },
            Case {
                name: "embedding-cuda-cleans-generation",
                config: InstanceConfig {
                    model_path: "C:/models/Qwen3-Embedding-8B.gguf".into(),
                    device: "CUDA0".into(),
                    spec_type: "draft-mtp".into(),
                    custom_args: vec!["--top-p 0.2".into()],
                    ..InstanceConfig::default()
                },
                required: &["-dev", "CUDA0", "--embedding", "--top-p", "0.2"],
                forbidden: &["--reranking", "--spec-type"],
            },
            Case {
                name: "reranker-cpu-cleans-generation",
                config: InstanceConfig {
                    model_path: "C:/models/Qwen3-Reranker-8B.gguf".into(),
                    device: "none".into(),
                    spec_type: "draft".into(),
                    ..InstanceConfig::default()
                },
                required: &["-dev", "none", "--embedding", "--reranking"],
                forbidden: &["--spec-type", "--context-shift"],
            },
        ];

        for case in cases {
            let (_, _, command) = prepare_launch(case.config, "llama-server");
            for expected in case.required {
                assert!(
                    command.iter().any(|arg| arg == expected),
                    "{} missing {expected}: {command:?}",
                    case.name
                );
            }
            for forbidden in case.forbidden {
                assert!(
                    !command.iter().any(|arg| arg == forbidden),
                    "{} leaked {forbidden}: {command:?}",
                    case.name
                );
            }
        }
    }

    #[test]
    fn parses_llama_cpp_print_timing_token_stats() {
        let parser = PerfParser::new(ModelWorkload::Inference);
        let mut tasks = HashMap::new();
        let mut last_completed = None;

        let lines = [
            "0.34.994.322 I slot launch_slot_: id  3 | task 6 | processing task, is_child = 0",
            "0.38.002.100 I slot print_timing: id  3 | task 6 | n_decoded = 120, tg = 40.00 t/s, tg_3s = 52.50 t/s",
            "1.29.406.958 I slot print_timing: id  3 | task 6 | prompt eval time =    1193.36 ms /   707 tokens (    1.69 ms per token,   592.45 tokens per second)",
            "1.29.406.983 I slot print_timing: id  3 | task 6 |        eval time =   53204.70 ms /  3519 tokens (   15.12 ms per token,    66.14 tokens per second)",
            "1.29.406.993 I slot print_timing: id  3 | task 6 |       total time =   54398.06 ms /  4226 tokens",
            "1.29.407.011 I slot print_timing: id  3 | task 6 | draft acceptance = 0.92624 ( 2587 accepted /  2793 generated), mean len =  3.78",
            "1.29.409.737 I slot      release: id  3 | task 6 | stop processing: n_tokens = 4225, truncated = 0",
        ];

        for line in lines {
            assert!(parse_perf_line(
                &parser,
                line,
                &mut tasks,
                &mut last_completed
            ));
        }

        let task = last_completed.expect("completed task should be captured");
        assert_eq!(task.prompt_tokens, Some(707));
        assert_eq!(task.gen_tokens, Some(3519));
        assert_eq!(task.total_tokens, Some(4226));
        assert_eq!(task.prompt_tps, Some(592.45));
        assert_eq!(task.gen_tps, Some(66.14));
        assert_eq!(task.tg_3s, Some(52.5));
        assert_eq!(task.spec_accept_rate, Some(0.92624));
    }

    #[test]
    fn older_timing_lines_without_rolling_rate_remain_supported() {
        let parser = PerfParser::new(ModelWorkload::Inference);
        let mut tasks = HashMap::new();
        let mut last_completed = None;
        assert!(parse_perf_line(
            &parser,
            "0 I slot launch_slot_: id 0 | task 7 | processing task, is_child = 0",
            &mut tasks,
            &mut last_completed,
        ));
        assert!(parse_perf_line(
            &parser,
            "3 I slot print_timing: id 0 | task 7 | n_decoded = 90, tg = 30.00 t/s",
            &mut tasks,
            &mut last_completed,
        ));

        let task = &tasks[&7];
        assert_eq!(task.tg, 30.0);
        assert_eq!(task.tg_3s, None);
    }

    #[test]
    fn embedding_child_tasks_emit_one_log_event_each() {
        let lines = [
            "0 I slot launch_slot_: id 0 | task 10 | processing task, is_child = 1",
            "1 I slot release: id 0 | task 10 | stop processing: n_tokens = 12, truncated = 0",
            "2 I slot launch_slot_: id 1 | task 11 | processing task, is_child = 1",
            "3 I slot release: id 1 | task 11 | stop processing: n_tokens = 20, truncated = 0",
            "4 I slot launch_slot_: id 2 | task 12 | processing task, is_child = 1",
            "5 I slot release: id 2 | task 12 | stop processing: n_tokens = 64, truncated = 0",
        ];

        let events = collect_vector_events(ModelWorkload::Embedding, &lines);

        assert_eq!(events.len(), 3);
        assert_eq!(events.iter().map(|event| event.item_count).sum::<u64>(), 3);
        assert_eq!(
            events
                .iter()
                .map(|event| event.input_tokens.unwrap())
                .sum::<u64>(),
            96
        );
        assert!(events
            .iter()
            .all(|event| event.workload == ModelWorkload::Embedding));
    }

    #[test]
    fn reranker_documents_emit_document_item_events() {
        let lines = [
            "0 I slot launch_slot_: id 0 | task 20 | processing task, is_child = 1",
            "1 I slot release: id 0 | task 20 | stop processing: n_tokens = 40, truncated = 0",
            "2 I slot launch_slot_: id 1 | task 21 | processing task, is_child = 1",
            "3 I slot release: id 1 | task 21 | stop processing: n_tokens = 48, truncated = 0",
        ];

        let events = collect_vector_events(ModelWorkload::Reranker, &lines);

        assert_eq!(events.len(), 2);
        assert_eq!(events.iter().map(|event| event.item_count).sum::<u64>(), 2);
        assert!(events
            .iter()
            .all(|event| event.workload == ModelWorkload::Reranker));
    }

    #[test]
    fn inference_and_incomplete_vector_tasks_do_not_emit_vector_events() {
        let completed = [
            "0 I slot launch_slot_: id 0 | task 30 | processing task, is_child = 0",
            "1 I slot release: id 0 | task 30 | stop processing: n_tokens = 10, truncated = 0",
        ];
        let incomplete = [
            "0 I slot launch_slot_: id 0 | task 31 | processing task, is_child = 1",
            "1 I slot release: id 0 | task 31 | stop without token summary",
        ];

        assert!(collect_vector_events(ModelWorkload::Inference, &completed).is_empty());
        assert!(collect_vector_events(ModelWorkload::Embedding, &incomplete).is_empty());
    }

    #[test]
    fn malformed_logs_cannot_grow_pending_task_state_without_bound() {
        let parser = PerfParser::new(ModelWorkload::Embedding);
        let mut tasks = HashMap::new();
        let mut last_completed = None;

        for task_id in 0..(MAX_TRACKED_PERF_TASKS as u32 + 50) {
            let line = format!(
                "0 I slot launch_slot_: id 0 | task {task_id} | processing task, is_child = 1"
            );
            assert!(parse_perf_line(
                &parser,
                &line,
                &mut tasks,
                &mut last_completed
            ));
        }

        assert_eq!(tasks.len(), MAX_TRACKED_PERF_TASKS);
    }

    #[test]
    fn unterminated_log_lines_are_capped_and_report_truncation() {
        let mut pending = Vec::new();
        let oversized = vec![b'x'; MAX_PENDING_LOG_BYTES + 4096];

        let lines = take_complete_log_lines(&mut pending, &oversized);

        assert_eq!(pending.len(), MAX_PENDING_LOG_BYTES);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("truncated"));

        let lines = take_complete_log_lines(&mut pending, b"\n");
        assert!(pending.is_empty());
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("truncated"));
        assert_eq!(lines[1].len(), MAX_PENDING_LOG_BYTES - 1);
    }

    #[test]
    fn never_ready_service_transitions_to_failed_after_startup_grace() {
        let mut failures = 0;
        let mut last_ready = None;

        for _ in 0..HEALTH_FAILURE_THRESHOLD {
            assert_eq!(
                advance_health_state(false, false, &mut failures, &mut last_ready),
                HealthTransition::None
            );
        }
        assert_eq!(last_ready, None);

        assert_eq!(
            advance_health_state(false, true, &mut failures, &mut last_ready),
            HealthTransition::Failed
        );
        assert_eq!(last_ready, Some(false));
    }

    #[test]
    fn recovered_service_emits_ready_after_a_failure() {
        let mut failures = HEALTH_FAILURE_THRESHOLD - 1;
        let mut last_ready = Some(true);
        assert_eq!(
            advance_health_state(false, true, &mut failures, &mut last_ready),
            HealthTransition::Failed
        );
        assert_eq!(
            advance_health_state(true, true, &mut failures, &mut last_ready),
            HealthTransition::Ready
        );
        assert_eq!(last_ready, Some(true));
    }

    #[test]
    fn browser_url_rejects_command_metacharacters_in_host() {
        let err = browser_url_for_host("127.0.0.1 & calc", 8080, false, "").unwrap_err();
        assert!(err.contains("Invalid host"));
    }

    #[test]
    fn command_display_masks_api_keys_as_argument_values() {
        assert_eq!(
            format_command_for_display(&[
                "llama-server".to_string(),
                "--api-key".to_string(),
                "secret value".to_string(),
                "--port".to_string(),
                "8080".to_string(),
            ]),
            "llama-server --api-key ******** --port 8080"
        );
        assert_eq!(
            format_command_for_display(&[
                "llama-server".to_string(),
                "--api-key=secret value".to_string(),
            ]),
            "llama-server --api-key=********"
        );
    }

    #[test]
    fn test_connection_accepts_models_when_health_is_bad_gateway() {
        let result = classify_test_connection_result(
            Err("HTTP 502 Bad Gateway".to_string()),
            Ok(reqwest::StatusCode::OK),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn fallback_probe_treats_successful_models_as_ready() {
        assert!(!probe_status_is_success(&Err(
            "HTTP 502 Bad Gateway".to_string()
        )));
        assert!(probe_status_is_success(&Ok(reqwest::StatusCode::OK)));
    }

    #[test]
    fn browser_url_maps_wildcard_host_to_localhost() {
        let url = browser_url_for_host("0.0.0.0", 8080, false, "").unwrap();
        assert_eq!(url, "http://localhost:8080");
    }

    #[test]
    fn browser_url_uses_tls_and_api_prefix() {
        let url = browser_url_for_host("::1", 8443, true, "/llama/").unwrap();
        assert_eq!(url, "https://[::1]:8443/llama");
    }

    #[test]
    fn tls_configuration_requires_an_atomic_key_certificate_pair() {
        let key_only = InstanceConfig {
            ssl_key_file: "server.key".into(),
            ..InstanceConfig::default()
        };
        assert!(validate_tls_configuration(&key_only).is_err());

        let complete = InstanceConfig {
            ssl_key_file: "server.key".into(),
            ssl_cert_file: "server.crt".into(),
            ..InstanceConfig::default()
        };
        assert!(validate_tls_configuration(&complete).is_ok());
        assert_eq!(effective_server_scheme(&complete), "https");
    }

    #[test]
    fn effective_api_key_prefers_inline_key() {
        let config = InstanceConfig {
            api_key: " inline-key ".into(),
            api_key_file: "missing-key-file.txt".into(),
            ..InstanceConfig::default()
        };

        assert_eq!(effective_api_key(&config), "inline-key");
    }

    #[test]
    fn effective_api_key_uses_first_inline_csv_key() {
        let config = InstanceConfig {
            api_key: " first-key, second-key ".into(),
            ..InstanceConfig::default()
        };
        assert_eq!(effective_api_key(&config), "first-key");
    }

    #[test]
    fn effective_api_key_reads_first_non_empty_file_line() {
        let dir = std::env::temp_dir().join(format!("lsm-api-key-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("api-key.txt");
        std::fs::write(&path, "\n # comment \n file-key \n second-key \n").unwrap();

        let config = InstanceConfig {
            api_key_file: path.to_string_lossy().to_string(),
            ..InstanceConfig::default()
        };

        assert_eq!(effective_api_key(&config), "file-key");
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn recorded_process_identity_rejects_pid_reuse() {
        let running = RunningInstance {
            instance_id: "instance-1".into(),
            pid: 42,
            port: 8080,
            host: "127.0.0.1".into(),
            start_time: 100,
            executable_path: "C:\\tools\\llama-server.exe".into(),
            telemetry_session_id: None,
            workload: "inference".into(),
            launch_config: None,
        };

        assert!(!process_identity_matches(
            &running,
            101,
            std::path::Path::new("C:\\tools\\llama-server.exe")
        ));
        assert!(!process_identity_matches(
            &running,
            100,
            std::path::Path::new("C:\\other\\process.exe")
        ));
    }

    #[test]
    fn process_metrics_refresh_discovers_a_new_process() {
        let mut system = System::new_all();
        #[cfg(target_os = "windows")]
        let mut child = Command::new("cmd")
            .args(["/C", "ping -n 4 127.0.0.1 >NUL"])
            .spawn()
            .unwrap();
        #[cfg(not(target_os = "windows"))]
        let mut child = Command::new("sleep").arg("3").spawn().unwrap();

        let (_, memory_mb) = get_process_metrics(&mut system, child.id());
        let _ = child.kill();
        let _ = child.wait();

        assert!(memory_mb > 0.0);
    }

    #[test]
    fn capped_log_writer_keeps_recent_output_within_limit() {
        let dir = std::env::temp_dir().join(format!("lsm-capped-log-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("server.log");
        let writer = CappedLogWriter::new(path.clone(), 64, 16).unwrap();

        writer.append(&[b'a'; 48]).unwrap();
        writer.append(&[b'b'; 48]).unwrap();

        let content = std::fs::read(&path).unwrap();
        assert!(content.len() <= 64);
        assert!(content.ends_with(&[b'b'; 48]));
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn complete_log_lines_wait_for_the_line_terminator() {
        let mut pending = Vec::new();
        assert!(take_complete_log_lines(&mut pending, b"partial").is_empty());
        assert_eq!(
            take_complete_log_lines(&mut pending, b" line\nnext\r\n"),
            vec!["partial line".to_string(), "next".to_string()]
        );
        assert!(pending.is_empty());
    }

    #[test]
    fn capped_log_reader_skips_the_retained_tail_after_rotation() {
        let dir =
            std::env::temp_dir().join(format!("lsm-capped-log-reader-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("server.log");
        let writer = CappedLogWriter::new(path.clone(), 32, 8).unwrap();
        writer.append(b"old-line-0000000000\n").unwrap();
        let (_, old_offset, old_generation) = writer.read_tail(20).unwrap();

        writer.append(b"new-line-1111111111\n").unwrap();
        let chunk = writer.read_since(old_offset, old_generation).unwrap();

        assert!(chunk.rotated);
        assert_eq!(chunk.bytes, b"new-line-1111111111\n");
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn log_replay_reads_only_requested_tail_lines() {
        let dir = std::env::temp_dir().join(format!("lsm-log-tail-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("server.log");
        let content = (0..3_000)
            .map(|index| format!("line-{index:04}\n"))
            .collect::<String>();
        std::fs::write(&path, content).unwrap();

        let (lines, end_offset) = read_log_tail(&path, 2_000).unwrap();

        assert_eq!(lines.len(), 2_000);
        assert_eq!(end_offset, std::fs::metadata(&path).unwrap().len());
        assert_eq!(lines.first().map(String::as_str), Some("line-1000"));
        assert_eq!(lines.last().map(String::as_str), Some("line-2999"));
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn log_replay_leaves_an_unterminated_line_for_the_tail_reader() {
        let dir = std::env::temp_dir().join(format!("lsm-log-partial-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("server.log");
        std::fs::write(&path, b"complete\npartial").unwrap();

        let (lines, end_offset) = read_log_tail(&path, 20).unwrap();

        assert_eq!(lines, vec!["complete".to_string()]);
        assert_eq!(end_offset, b"complete\n".len() as u64);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir(dir);
    }
}

// IPC compatibility boundary: legacy command internals keep their existing error flow,
// while every registered command serializes a stable AppError object.
#[allow(dead_code, unused_imports, unused_mut)] // Tauri references adapters through generated macros.
pub mod ipc {
    use super::*;

    #[tauri::command]
    pub async fn generate_server_command(
        config: InstanceConfig,
        engine_exe: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<GeneratedServerCommand> {
        super::generate_server_command(config, engine_exe, state).await
    }

    #[tauri::command]
    pub async fn stop_server(
        instance_id: String,
        state: tauri::State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<()> {
        super::stop_server(instance_id, state, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn open_browser(
        host: String,
        port: u16,
        use_tls: Option<bool>,
        api_prefix: Option<String>,
        instance_id: Option<String>,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::open_browser(host, port, use_tls, api_prefix, instance_id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    #[allow(clippy::too_many_arguments)] // IPC compatibility wrapper mirrors the command fields.
    pub async fn test_connection(
        host: String,
        port: u16,
        api_key: Option<String>,
        api_key_file: Option<String>,
        use_tls: Option<bool>,
        api_prefix: Option<String>,
        instance_id: Option<String>,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<String> {
        super::test_connection(
            host,
            port,
            api_key,
            api_key_file,
            use_tls,
            api_prefix,
            instance_id,
            state,
        )
        .await
        .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn check_port(port: u16, host: Option<String>) -> crate::error::AppResult<bool> {
        super::check_port(port, host)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_system_metrics(
        instance_id: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<SystemMetrics> {
        super::get_system_metrics(instance_id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_system_health() -> crate::error::AppResult<SystemMetrics> {
        super::get_system_health()
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_slots(
        host: String,
        port: u16,
        api_key: Option<String>,
        use_tls: Option<bool>,
        api_prefix: Option<String>,
    ) -> crate::error::AppResult<Vec<SlotInfo>> {
        super::get_slots(host, port, api_key, use_tls, api_prefix)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_metrics(
        host: String,
        port: u16,
        api_key: Option<String>,
        use_tls: Option<bool>,
        api_prefix: Option<String>,
    ) -> crate::error::AppResult<Option<MetricsInfo>> {
        super::get_metrics(host, port, api_key, use_tls, api_prefix)
            .await
            .map_err(crate::error::AppError::from)
    }
}
