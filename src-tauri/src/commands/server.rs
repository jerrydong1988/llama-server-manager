use crate::commands::adlx;
use crate::commands::nvml;
use crate::models::{AppState, InstanceConfig, RunningInstance, SystemMetrics};
use crate::vector_policy::{normalize_for_launch, ModelWorkload};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use sysinfo::{Pid, ProcessesToUpdate, System};
use tauri::{Emitter, Manager};

const MAX_SERVER_LOG_BYTES: u64 = 32 * 1024 * 1024;
const RETAINED_SERVER_LOG_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TRACKED_PERF_TASKS: usize = 1_024;
const LOG_REPLAY_LINES: usize = 2_000;
const LOG_REPLAY_MAX_BYTES: u64 = 4 * 1024 * 1024;
const LOG_EVENT_BATCH_SIZE: usize = 200;

struct CappedLogState {
    file: std::fs::File,
    size: u64,
    max_bytes: u64,
    retained_bytes: u64,
}

struct CappedLogWriter {
    state: Mutex<CappedLogState>,
}

impl CappedLogWriter {
    fn new(path: std::path::PathBuf, max_bytes: u64, retained_bytes: u64) -> std::io::Result<Self> {
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
        }
        state.file.seek(SeekFrom::End(0))?;
        state.file.write_all(bytes)?;
        state.size = state.size.saturating_add(bytes.len() as u64);
        Ok(())
    }
}

fn spawn_log_pump<R>(mut source: R, writer: Arc<CappedLogWriter>)
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match source.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if writer.append(&buffer[..read]).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

// Generate CLI command.

fn mask_api_key_in_cmd(cmd: &str) -> String {
    let mut result = cmd.to_string();
    let mut search_from = 0;
    while let Some(pos) = result[search_from..].find("--api-key ") {
        let pos = search_from + pos;
        let rest = &result[pos + 10..];
        if let Some(end) = rest.find(' ') {
            let key = &rest[..end];
            if !key.is_empty() {
                let masked = "*".repeat(key.len().min(8));
                let tail = &rest[end..];
                result = format!("{}--api-key {}{}", &result[..pos], masked, tail);
                search_from = pos + 10 + masked.len();
            } else {
                search_from = pos + 10;
            }
        } else {
            let key = rest;
            if !key.is_empty() {
                let masked = "*".repeat(key.len().min(8));
                result = format!("{}--api-key {}", &result[..pos], masked);
            }
            break;
        }
    }
    result
}

pub(crate) fn effective_api_key(config: &InstanceConfig) -> String {
    let inline = config.api_key.trim();
    if !inline.is_empty() {
        return inline.to_string();
    }

    if config.api_key_file.trim().is_empty() {
        return String::new();
    }

    std::fs::read_to_string(config.api_key_file.trim())
        .ok()
        .and_then(|content| {
            content.lines().find_map(|line| {
                let key = line.trim().trim_start_matches('\u{feff}');
                if key.is_empty() {
                    None
                } else {
                    Some(key.to_string())
                }
            })
        })
        .unwrap_or_default()
}

fn telemetry_config_hash(config: &InstanceConfig) -> String {
    let mut hasher = DefaultHasher::new();
    serde_json::to_string(config)
        .unwrap_or_default()
        .hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn generate_command(config: &InstanceConfig, engine_path: &str) -> Vec<String> {
    prepare_launch(config.clone(), engine_path).2
}

fn generate_normalized_command(config: &InstanceConfig, engine_path: &str) -> Vec<String> {
    let exe = if engine_path.is_empty() {
        "llama-server".to_string()
    } else {
        engine_path.to_string()
    };
    let mut cmd = vec![exe, "-m".into(), config.model_path.clone()];
    let is_emb = config.embedding;

    // Basic.
    if !config.alias.is_empty() {
        cmd.extend_from_slice(&["-a".into(), config.alias.clone()]);
    }
    if !is_emb {
        if !config.lora_path.is_empty() {
            cmd.extend_from_slice(&["--lora".into(), config.lora_path.clone()]);
        }
        if config.lora_init_without_apply {
            cmd.push("--lora-init-without-apply".into());
        }
        if !config.lora_scaled.is_empty() {
            cmd.extend_from_slice(&["--lora-scaled".into(), config.lora_scaled.clone()]);
        }
        if !config.mmproj_path.is_empty() {
            cmd.extend_from_slice(&["--mmproj".into(), config.mmproj_path.clone()]);
        }
        if !config.mmproj_url.is_empty() {
            cmd.extend_from_slice(&["--mmproj-url".into(), config.mmproj_url.clone()]);
        }
        if config.mmproj_auto {
            cmd.push("--mmproj-auto".into());
        }
        if config.no_mmproj {
            cmd.push("--no-mmproj".into());
        }
        if config.no_mmproj_offload {
            cmd.push("--no-mmproj-offload".into());
        }
        if !config.chat_template.is_empty() {
            cmd.extend_from_slice(&["--chat-template".into(), config.chat_template.clone()]);
        }
        if !config.chat_template_file.is_empty() {
            cmd.extend_from_slice(&[
                "--chat-template-file".into(),
                config.chat_template_file.clone(),
            ]);
        }
        if config.skip_chat_parsing {
            cmd.push("--skip-chat-parsing".into());
        }
        if !config.reasoning_format.is_empty() {
            cmd.extend_from_slice(&["--reasoning-format".into(), config.reasoning_format.clone()]);
        }
        if !config.reasoning.is_empty() {
            cmd.extend_from_slice(&["--reasoning".into(), config.reasoning.clone()]);
        }
        if !config.reasoning_budget.is_empty() {
            cmd.extend_from_slice(&["--reasoning-budget".into(), config.reasoning_budget.clone()]);
        }
        if !config.reasoning_budget_message.is_empty() {
            cmd.extend_from_slice(&[
                "--reasoning-budget-message".into(),
                config.reasoning_budget_message.clone(),
            ]);
        }
        if !config.reasoning_effort.is_empty() {
            let re = format!("{{\"reasoning_effort\": \"{}\"}}", config.reasoning_effort);
            cmd.extend_from_slice(&["--chat-template-kwargs".into(), re]);
        }
        if config.jinja {
            cmd.push("--jinja".into());
        }
        if !config.grammar_file.is_empty() {
            cmd.extend_from_slice(&["--grammar-file".into(), config.grammar_file.clone()]);
        }
        if !config.grammar.is_empty() {
            cmd.extend_from_slice(&["--grammar".into(), config.grammar.clone()]);
        }
    }

    // Performance and context.
    if !config.ctx_size_auto {
        cmd.extend_from_slice(&["-c".into(), config.ctx_size.to_string()]);
    }
    if !config.gpu_layers_auto {
        cmd.extend_from_slice(&["-ngl".into(), config.gpu_layers.to_string()]);
    }
    if config.threads > 0 {
        cmd.extend_from_slice(&["-t".into(), config.threads.to_string()]);
    }
    if config.batch_size > 0 {
        cmd.extend_from_slice(&["-b".into(), config.batch_size.to_string()]);
    }
    if config.ubatch_size > 0 {
        cmd.extend_from_slice(&["-ub".into(), config.ubatch_size.to_string()]);
    }
    if config.parallel > 0 || config.parallel == -1 {
        cmd.extend_from_slice(&["-np".into(), config.parallel.to_string()]);
    }
    if config.cont_batching {
        cmd.push("-cb".into());
    }
    if !is_emb && !config.cache_prompt {
        cmd.push("--no-cache-prompt".into());
    }
    if config.threads_batch > 0 {
        cmd.extend_from_slice(&["--threads-batch".into(), config.threads_batch.to_string()]);
    }
    if config.threads_http >= 0 {
        cmd.extend_from_slice(&["--threads-http".into(), config.threads_http.to_string()]);
    }
    if !is_emb {
        if config.keep > 0 {
            cmd.extend_from_slice(&["--keep".into(), config.keep.to_string()]);
        }
        if config.cache_reuse > 0 {
            cmd.extend_from_slice(&["--cache-reuse".into(), config.cache_reuse.to_string()]);
        }
        if config.cache_ram > 0 {
            cmd.extend_from_slice(&["-cram".into(), config.cache_ram.to_string()]);
        }
    }
    if config.warmup {
        cmd.push("--warmup".into());
    }
    if !is_emb {
        if config.ctx_checkpoints != 32 {
            cmd.extend_from_slice(&["-ctxcp".into(), config.ctx_checkpoints.to_string()]);
        }
        if config.checkpoint_min_step > 0 {
            cmd.extend_from_slice(&["-cms".into(), config.checkpoint_min_step.to_string()]);
        }
        if config.swa_full {
            cmd.push("--swa-full".into());
        }
    }

    // RoPE / YaRN.
    if !config.rope_scaling.is_empty() {
        cmd.extend_from_slice(&["--rope-scaling".into(), config.rope_scaling.clone()]);
    }
    if config.rope_scale > 0.0 {
        cmd.extend_from_slice(&["--rope-scale".into(), config.rope_scale.to_string()]);
    }
    if config.rope_freq_base > 0.0 {
        cmd.extend_from_slice(&["--rope-freq-base".into(), config.rope_freq_base.to_string()]);
    }
    if config.rope_freq_scale > 0.0 {
        cmd.extend_from_slice(&[
            "--rope-freq-scale".into(),
            config.rope_freq_scale.to_string(),
        ]);
    }
    if config.yarn_ext_factor >= 0.0 {
        cmd.extend_from_slice(&[
            "--yarn-ext-factor".into(),
            config.yarn_ext_factor.to_string(),
        ]);
    }
    if config.yarn_attn_factor != -1.0 {
        cmd.extend_from_slice(&[
            "--yarn-attn-factor".into(),
            config.yarn_attn_factor.to_string(),
        ]);
    }
    if config.yarn_beta_slow > 0.0 {
        cmd.extend_from_slice(&["--yarn-beta-slow".into(), config.yarn_beta_slow.to_string()]);
    }
    if config.yarn_beta_fast != -1.0 {
        cmd.extend_from_slice(&["--yarn-beta-fast".into(), config.yarn_beta_fast.to_string()]);
    }
    if config.yarn_orig_ctx > 0 {
        cmd.extend_from_slice(&["--yarn-orig-ctx".into(), config.yarn_orig_ctx.to_string()]);
    }

    // Flash Attention.
    let fa = config.flash_attn.as_str();
    if fa != "auto" && !fa.is_empty() {
        cmd.extend_from_slice(&["-fa".into(), fa.to_string()]);
    }

    // Memory and loading.
    if config.moe_cpu_layers > 0 {
        cmd.extend_from_slice(&["--n-cpu-moe".into(), config.moe_cpu_layers.to_string()]);
    }
    if config.cpu_moe {
        cmd.push("--cpu-moe".into());
    }
    if config.mlock {
        cmd.push("--mlock".into());
    }
    if config.no_mmap {
        cmd.push("--no-mmap".into());
    }
    if config.no_repack {
        cmd.push("--no-repack".into());
    }
    if config.direct_io {
        cmd.push("--direct-io".into());
    }
    if config.numa {
        cmd.extend_from_slice(&["--numa".into(), "distribute".into()]);
    }
    if config.check_tensors {
        cmd.push("--check-tensors".into());
    }
    if config.perf {
        cmd.push("--perf".into());
    }
    if config.fit {
        cmd.extend_from_slice(&["--fit".into(), "on".into()]);
    }
    if !config.fit_target.is_empty() {
        cmd.extend_from_slice(&["-fitt".into(), config.fit_target.clone()]);
    }
    if config.fit_ctx != 4096 {
        cmd.extend_from_slice(&["-fitc".into(), config.fit_ctx.to_string()]);
    }

    // KV cache.
    if !config.cache_type_k.is_empty() {
        cmd.extend_from_slice(&["-ctk".into(), config.cache_type_k.clone()]);
    }
    if !config.cache_type_v.is_empty() {
        cmd.extend_from_slice(&["-ctv".into(), config.cache_type_v.clone()]);
    }
    if !is_emb {
        if !config.cache_type_draft_k.is_empty() {
            cmd.extend_from_slice(&["-ctkd".into(), config.cache_type_draft_k.clone()]);
        }
        if !config.cache_type_draft_v.is_empty() {
            cmd.extend_from_slice(&["-ctvd".into(), config.cache_type_draft_v.clone()]);
        }
    }
    if config.kv_unified {
        cmd.push("--kv-unified".into());
    }
    if config.no_kv_offload {
        cmd.push("--no-kv-offload".into());
    }
    if !config.cache_idle_slots {
        cmd.push("--no-cache-idle-slots".into());
    }

    // GPU and device.
    if !config.device.is_empty() {
        cmd.extend_from_slice(&["-dev".into(), config.device.clone()]);
    }
    if !config.split_mode.is_empty() {
        cmd.extend_from_slice(&["-sm".into(), config.split_mode.clone()]);
    }
    if !config.tensor_split.is_empty() {
        cmd.extend_from_slice(&["-ts".into(), config.tensor_split.clone()]);
    }
    if config.main_gpu > 0 {
        cmd.extend_from_slice(&["-mg".into(), config.main_gpu.to_string()]);
    }
    if !config.override_kv.is_empty() {
        cmd.extend_from_slice(&["--override-kv".into(), config.override_kv.clone()]);
    }

    // Speculative decoding.
    let spec_active = !is_emb && !config.spec_type.is_empty() && config.spec_type != "none";
    if spec_active {
        if !config.draft_model_path.is_empty() {
            cmd.extend_from_slice(&["-md".into(), config.draft_model_path.clone()]);
        }
        if config.draft_gpu_layers > 0 && config.draft_gpu_layers < 99 {
            cmd.extend_from_slice(&["-ngld".into(), config.draft_gpu_layers.to_string()]);
        }
        if config.draft_tokens > 0 {
            cmd.extend_from_slice(&["--spec-draft-n-max".into(), config.draft_tokens.to_string()]);
        }
        if config.spec_draft_n_min > 0 {
            cmd.extend_from_slice(&[
                "--spec-draft-n-min".into(),
                config.spec_draft_n_min.to_string(),
            ]);
        }
        cmd.extend_from_slice(&["--spec-type".into(), config.spec_type.clone()]);
        if config.spec_draft_p_min > 0.0 {
            cmd.extend_from_slice(&[
                "--spec-draft-p-min".into(),
                config.spec_draft_p_min.to_string(),
            ]);
        }
        if config.spec_draft_p_split != 0.1 {
            cmd.extend_from_slice(&[
                "--spec-draft-p-split".into(),
                config.spec_draft_p_split.to_string(),
            ]);
        }
        if !config.spec_draft_device.is_empty() {
            cmd.extend_from_slice(&[
                "--spec-draft-device".into(),
                config.spec_draft_device.clone(),
            ]);
        }
        if !config.lookup_cache_static.is_empty() {
            cmd.extend_from_slice(&["-lcs".into(), config.lookup_cache_static.clone()]);
        }
        if !config.lookup_cache_dynamic.is_empty() {
            cmd.extend_from_slice(&["-lcd".into(), config.lookup_cache_dynamic.clone()]);
        }
        if config.spec_default {
            cmd.push("--spec-default".into());
        }
        if !config.spec_draft_backend_sampling {
            cmd.push("--no-spec-draft-backend-sampling".into());
        }
        if config.spec_draft_threads > 0 {
            cmd.extend_from_slice(&["-td".into(), config.spec_draft_threads.to_string()]);
        }
        if config.spec_draft_threads_batch > 0 {
            cmd.extend_from_slice(&["-tbd".into(), config.spec_draft_threads_batch.to_string()]);
        }
    }

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
    if config.no_ui {
        cmd.push("--no-ui".into());
    }
    if config.offline {
        cmd.push("--offline".into());
    }
    if !config.path_prefix.is_empty() {
        cmd.extend_from_slice(&["--path".into(), config.path_prefix.clone()]);
    }
    if !config.api_prefix.is_empty() {
        cmd.extend_from_slice(&["--api-prefix".into(), config.api_prefix.clone()]);
    }
    if !is_emb {
        if !config.ui_config_file.is_empty() {
            cmd.extend_from_slice(&["--ui-config-file".into(), config.ui_config_file.clone()]);
        }
        if !config.ui_config.is_empty() {
            cmd.extend_from_slice(&["--ui-config".into(), config.ui_config.clone()]);
        }
        if config.ui_mcp_proxy {
            cmd.push("--ui-mcp-proxy".into());
        }
        if config.agent {
            cmd.push("--agent".into());
        }
    }

    // Embedding / generation.
    if config.embedding {
        cmd.push("--embedding".into());
        if !config.pooling.is_empty() {
            cmd.extend_from_slice(&["--pooling".into(), config.pooling.clone()]);
        }
        if config.embd_normalize != 2 {
            cmd.extend_from_slice(&["--embd-normalize".into(), config.embd_normalize.to_string()]);
        }
        if config.reranking {
            cmd.push("--reranking".into());
        }
    } else {
        if config.n_predict > 0 {
            cmd.extend_from_slice(&["-n".into(), config.n_predict.to_string()]);
        } else if config.n_predict == 0 {
        } else {
            cmd.extend_from_slice(&["-n".into(), "-1".into()]);
        }
        if config.ignore_eos {
            cmd.push("--ignore-eos".into());
        }
        if !config.json_schema.is_empty() {
            cmd.extend_from_slice(&["--json-schema".into(), config.json_schema.clone()]);
        }
        if !config.json_schema_file.is_empty() {
            cmd.extend_from_slice(&["-jf".into(), config.json_schema_file.clone()]);
        }
        if config.temp > 0.0 {
            cmd.extend_from_slice(&["--temp".into(), config.temp.to_string()]);
        }
        if config.top_k > 0 {
            cmd.extend_from_slice(&["--top-k".into(), config.top_k.to_string()]);
        }
        if config.top_p > 0.0 {
            cmd.extend_from_slice(&["--top-p".into(), config.top_p.to_string()]);
        }
        if config.repeat_penalty > 0.0 {
            cmd.extend_from_slice(&["--repeat-penalty".into(), config.repeat_penalty.to_string()]);
        }
        if config.seed >= 0 {
            cmd.extend_from_slice(&["--seed".into(), config.seed.to_string()]);
        }
        if config.min_p > 0.0 {
            cmd.extend_from_slice(&["--min-p".into(), config.min_p.to_string()]);
        }
        if config.presence_penalty > 0.0 {
            cmd.extend_from_slice(&[
                "--presence-penalty".into(),
                config.presence_penalty.to_string(),
            ]);
        }
        if config.frequency_penalty > 0.0 {
            cmd.extend_from_slice(&[
                "--frequency-penalty".into(),
                config.frequency_penalty.to_string(),
            ]);
        }
        if config.repeat_last_n > 0 {
            cmd.extend_from_slice(&["--repeat-last-n".into(), config.repeat_last_n.to_string()]);
        }
        if !config.reverse_prompt.is_empty() {
            cmd.extend_from_slice(&["-r".into(), config.reverse_prompt.clone()]);
        }
        if config.special {
            cmd.push("-sp".into());
        }
        if config.spm_infill {
            cmd.push("--spm-infill".into());
        }
        if config.backend_sampling {
            cmd.push("-bs".into());
        }

        // Advanced sampling
        if config.mirostat > 0 {
            cmd.extend_from_slice(&["--mirostat".into(), config.mirostat.to_string()]);
            if config.mirostat_lr > 0.0 {
                cmd.extend_from_slice(&["--mirostat-lr".into(), config.mirostat_lr.to_string()]);
            }
            if config.mirostat_ent > 0.0 {
                cmd.extend_from_slice(&["--mirostat-ent".into(), config.mirostat_ent.to_string()]);
            }
        }
        if config.xtc_probability > 0.0 {
            cmd.extend_from_slice(&[
                "--xtc-probability".into(),
                config.xtc_probability.to_string(),
            ]);
            if config.xtc_threshold > 0.0 {
                cmd.extend_from_slice(&[
                    "--xtc-threshold".into(),
                    config.xtc_threshold.to_string(),
                ]);
            }
        }
        if config.dynatemp_range > 0.0 {
            cmd.extend_from_slice(&["--dynatemp-range".into(), config.dynatemp_range.to_string()]);
            if config.dynatemp_exp > 0.0 {
                cmd.extend_from_slice(&["--dynatemp-exp".into(), config.dynatemp_exp.to_string()]);
            }
        }
        if config.typical_p < 1.0 && config.typical_p > 0.0 {
            cmd.extend_from_slice(&["--typical-p".into(), config.typical_p.to_string()]);
        }
        if config.dry_multiplier > 0.0 {
            cmd.extend_from_slice(&["--dry-multiplier".into(), config.dry_multiplier.to_string()]);
            if config.dry_base > 0.0 {
                cmd.extend_from_slice(&["--dry-base".into(), config.dry_base.to_string()]);
            }
            if config.dry_allowed_length > 0 {
                cmd.extend_from_slice(&[
                    "--dry-allowed-length".into(),
                    config.dry_allowed_length.to_string(),
                ]);
            }
            if config.dry_penalty_last_n > 0 {
                cmd.extend_from_slice(&[
                    "--dry-penalty-last-n".into(),
                    config.dry_penalty_last_n.to_string(),
                ]);
            }
            if !config.dry_sequence_breaker.is_empty() {
                cmd.extend_from_slice(&[
                    "--dry-sequence-breaker".into(),
                    config.dry_sequence_breaker.clone(),
                ]);
            }
        }
        if config.adaptive_target > 0.0 {
            cmd.extend_from_slice(&[
                "--adaptive-target".into(),
                config.adaptive_target.to_string(),
            ]);
            if config.adaptive_decay > 0.0 {
                cmd.extend_from_slice(&[
                    "--adaptive-decay".into(),
                    config.adaptive_decay.to_string(),
                ]);
            }
        }
        if config.top_n_sigma >= 0.0 {
            cmd.extend_from_slice(&["--top-n-sigma".into(), config.top_n_sigma.to_string()]);
        }
        if !config.logit_bias.is_empty() {
            cmd.extend_from_slice(&["-l".into(), config.logit_bias.clone()]);
        }
        if !config.samplers.is_empty() {
            cmd.extend_from_slice(&["--samplers".into(), config.samplers.clone()]);
        }
        if !config.sampler_seq.is_empty() {
            cmd.extend_from_slice(&["--sampler-seq".into(), config.sampler_seq.clone()]);
        }
    }

    // Server features.
    if config.timeout > 0 {
        cmd.extend_from_slice(&["-to".into(), config.timeout.to_string()]);
    }
    if config.sleep_idle >= 0 {
        cmd.extend_from_slice(&["--sleep-idle-seconds".into(), config.sleep_idle.to_string()]);
    }
    if !is_emb && config.context_shift {
        cmd.push("--context-shift".into());
    }
    if config.verbose {
        cmd.push("-v".into());
    }
    if config.metrics {
        cmd.push("--metrics".into());
    }
    if config.props {
        cmd.push("--props".into());
    }
    if !config.slots_enabled {
        cmd.push("--no-slots".into());
    }
    if !is_emb {
        if !config.slot_save_path.is_empty() {
            cmd.extend_from_slice(&["--slot-save-path".into(), config.slot_save_path.clone()]);
        }
        if (config.slot_prompt_similarity - 0.1).abs() > f32::EPSILON {
            cmd.extend_from_slice(&["-sps".into(), config.slot_prompt_similarity.to_string()]);
        }
        if config.prefill_assistant {
            cmd.push("--prefill-assistant".into());
        }
    }

    // New server features aligned with llama.cpp master.
    if !config.rpc_servers.is_empty() {
        cmd.extend_from_slice(&["--rpc".into(), config.rpc_servers.clone()]);
    }
    if config.sse_ping_interval != 30 {
        cmd.extend_from_slice(&[
            "--sse-ping-interval".into(),
            config.sse_ping_interval.to_string(),
        ]);
    }
    if config.reuse_port {
        cmd.push("--reuse-port".into());
    }

    if !is_emb {
        // Multi-model and media.
        if !config.models_dir.is_empty() {
            cmd.extend_from_slice(&["--models-dir".into(), config.models_dir.clone()]);
        }
        if !config.models_preset.is_empty() {
            cmd.extend_from_slice(&["--models-preset".into(), config.models_preset.clone()]);
        }
        if config.models_max != 4 {
            cmd.extend_from_slice(&["--models-max".into(), config.models_max.to_string()]);
        }
        if config.models_autoload {
            cmd.push("--models-autoload".into());
        }
        if config.image_min_tokens > 0 {
            cmd.extend_from_slice(&[
                "--image-min-tokens".into(),
                config.image_min_tokens.to_string(),
            ]);
        }
        if config.image_max_tokens > 0 {
            cmd.extend_from_slice(&[
                "--image-max-tokens".into(),
                config.image_max_tokens.to_string(),
            ]);
        }
        if config.mtmd_batch_max_tokens != 1024 {
            cmd.extend_from_slice(&[
                "--mtmd-batch-max-tokens".into(),
                config.mtmd_batch_max_tokens.to_string(),
            ]);
        }
        if !config.tags.is_empty() {
            cmd.extend_from_slice(&["--tags".into(), config.tags.clone()]);
        }
        if !config.media_path.is_empty() {
            cmd.extend_from_slice(&["--media-path".into(), config.media_path.clone()]);
        }
        if !config.tools.is_empty() {
            cmd.extend_from_slice(&["--tools".into(), config.tools.clone()]);
        }

        // Custom args (#13: support double-quoted arguments).
        for arg in &config.custom_args {
            if !arg.is_empty() {
                cmd.extend(split_args(arg));
            }
        }
    }

    cmd
}

fn prepare_launch(
    config: InstanceConfig,
    engine_path: &str,
) -> (InstanceConfig, ModelWorkload, Vec<String>) {
    let normalized = normalize_for_launch(config);
    let workload = normalized.workload;
    let config = normalized.into_config();
    let command = generate_normalized_command(&config, engine_path);
    (config, workload, command)
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

#[tauri::command]
pub async fn generate_server_command(
    config: InstanceConfig,
    engine_exe: String,
) -> Result<Vec<String>, String> {
    Ok(generate_command(&config, &engine_exe))
}

#[tauri::command]
pub async fn start_server(
    instance_id: String,
    config: InstanceConfig,
    engine_exe: String,
    engine_backend: String,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let _reservation = reserve_instance_start(state.inner(), &instance_id)?;
    let (config, workload, cmd) = prepare_launch(config, &engine_exe);
    let cmd_str = cmd.join(" ");
    let cmd_display = mask_api_key_in_cmd(&cmd_str);

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
    spawn_log_pump(stdout, log_writer.clone());
    spawn_log_pump(stderr, log_writer);

    let (start_time, executable_path) = match read_process_identity(pid) {
        Some(identity) => identity,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Unable to verify the started server process identity".to_string());
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
    {
        let mut running = state.running.lock().unwrap();
        if running.contains_key(&instance_id) {
            let _ = child.kill();
            let _ = crate::commands::telemetry::finish_run_session(
                telemetry_session_id.as_deref(),
                None,
                "duplicate-start",
            );
            return Err("该实例已在运行中".to_string());
        }
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
            },
        );
    }

    // Persist running state to disk immediately via unified atomic writes to avoid races.
    state
        .instances
        .lock()
        .unwrap()
        .insert(instance_id.clone(), config.clone());
    let running_snapshot = state.running.lock().unwrap().clone();
    let persisted_instance_id = instance_id.clone();
    let persisted_config = config.clone();
    let _ = crate::commands::config::update_and_persist(&state, |global| {
        global.running = running_snapshot;
        global
            .instances
            .insert(persisted_instance_id, persisted_config);
    });

    app.emit(
        "server-started",
        serde_json::json!({
            "instanceId": instance_id,
            "pid": pid,
            "port": config.port,
            "command": cmd_display,
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
            telemetry_session_tail,
            workload,
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
                let _ = crate::commands::config::update_and_persist(&st, |global| {
                    if global
                        .running
                        .get(&id)
                        .map(|current| current.pid == pid)
                        .unwrap_or(false)
                    {
                        global.running.remove(&id);
                    }
                });
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
        let st2 = app_clone.state::<AppState>();
        {
            let mut r = st2.running.lock().unwrap();
            if r.get(&id).map(|ri| ri.pid == pid).unwrap_or(false) {
                r.remove(&id);
                {
                    let mut restored = st2.restored_runtime_instances.lock().unwrap();
                    restored.remove(&format!("{}:{}", id, pid));
                }
                let _ = crate::commands::telemetry::finish_run_session(
                    telemetry_session_monitor.as_deref(),
                    exit_code,
                    "process-exited",
                );
                let _ = crate::commands::config::update_and_persist(&st2, |global| {
                    if global
                        .running
                        .get(&id)
                        .map(|current| current.pid == pid)
                        .unwrap_or(false)
                    {
                        global.running.remove(&id);
                    }
                });
                let _ = app_clone.emit("server-stopped", serde_json::json!({ "instanceId": id }));
            }
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
    let port_metrics = config.port;
    let api_key_metrics = effective_api_key(&config);
    let telemetry_session_metrics = telemetry_session_id.clone();
    std::thread::spawn(move || {
        monitor_loop(
            &id_metrics,
            pid,
            &host_metrics,
            port_metrics,
            &api_key_metrics,
            telemetry_session_metrics,
            app_metrics,
        );
    });

    Ok(())
}

/// Background metrics loop that samples every 5 seconds, pushes to the frontend, and records history.
fn monitor_loop(
    instance_id: &str,
    expected_pid: u32,
    host: &str,
    port: u16,
    api_key: &str,
    telemetry_session_id: Option<String>,
    app: tauri::AppHandle,
) {
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
    let metrics_url = crate::utils::http_url(host, port, "/metrics");
    let slots_url = crate::utils::http_url(host, port, "/slots");
    let health_url = crate::utils::http_url(host, port, "/health");
    let models_url = crate::utils::http_url(host, port, "/v1/models");

    // Startup timestamp used to compute uptime.
    let start_instant = std::time::Instant::now();

    // Dedicated System for process metrics; system/GPU metrics reuse the SYSINFO_CACHE singleton.
    let mut proc_sys = System::new_all();
    let mut health_failures = 0_u32;
    let mut health_ready = false;

    loop {
        let iteration_started = std::time::Instant::now();
        if !is_my_instance() {
            break;
        }
        if !is_recorded_process_alive(expected_pid) {
            cleanup_running_instance(&app, instance_id, expected_pid, "process-exited");
            break;
        }

        let probe = |url: &str| {
            let mut request = client.get(url).timeout(std::time::Duration::from_secs(2));
            if !api_key.is_empty() {
                request = request.header("Authorization", format!("Bearer {api_key}"));
            }
            request.send().map(|response| response.status())
        };
        let ready = probe_status_is_success(&probe(&health_url).map_err(|e| e.to_string()))
            || probe_status_is_success(&probe(&models_url).map_err(|e| e.to_string()));
        if ready {
            health_failures = 0;
            if !health_ready {
                health_ready = true;
                let _ = app.emit(
                    "health-status",
                    serde_json::json!({ "instanceId": instance_id, "status": "ok" }),
                );
            }
        } else {
            health_failures = health_failures.saturating_add(1);
            if health_ready && health_failures >= 3 {
                health_ready = false;
                health_failures = 0;
                let _ = app.emit(
                    "health-status",
                    serde_json::json!({ "instanceId": instance_id, "status": "fail" }),
                );
            }
        }

        // System-level and GPU metrics using the collect_gpu_and_system singleton cache.
        let (
            adlx_cpu,
            gpu_pct,
            vram_u,
            vram_t,
            _mem,
            gpu_vendor,
            sys_cpu,
            sys_mem_total,
            sys_mem_used,
        ) = collect_gpu_and_system();

        // Process-level metrics.
        let (cpu, mem) = get_process_metrics(&mut proc_sys, expected_pid);
        let cpu_pct = adlx_cpu.unwrap_or(cpu);
        let mem_mb = if adlx_cpu.is_some() {
            _mem.unwrap_or(mem)
        } else {
            (mem * 10.0).round() / 10.0
        };

        let sys_metrics = SystemMetrics {
            cpu_percent: cpu_pct,
            memory_mb: mem_mb,
            uptime_secs: start_instant.elapsed().as_secs(),
            gpu_percent: gpu_pct,
            vram_used_mb: vram_u,
            vram_total_mb: vram_t,
            system_cpu_percent: sys_cpu,
            system_memory_total_mb: sys_mem_total,
            system_memory_used_mb: sys_mem_used,
            gpu_vendor,
        };

        // llama metrics.
        let mut llama_metrics: Option<serde_json::Value> = None;
        let mut llama_sample: Option<crate::commands::telemetry::LlamaMetricSample> = None;
        let mut req = client.get(&metrics_url);
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }
        if let Ok(resp) = req.timeout(std::time::Duration::from_secs(2)).send() {
            if resp.status().is_success() {
                if let Ok(body) = resp.text() {
                    let sample = parse_llama_metric_sample(&body);
                    llama_metrics = Some(serde_json::json!({
                        "tokens_per_sec": sample.tokens_per_sec,
                        "prompt_tokens": sample.prompt_tokens,
                        "gen_tokens": sample.gen_tokens,
                        "decode_calls_total": sample.decode_calls_total,
                        "max_tokens_observed": sample.max_tokens_observed,
                        "prompt_tokens_per_sec": sample.prompt_tokens_per_sec,
                        "requests_processing": sample.requests_processing,
                        "requests_deferred": sample.requests_deferred,
                        "busy_slots_per_decode": sample.busy_slots_per_decode,
                    }));
                    llama_sample = Some(sample);
                }
            }
        }

        // Emit events.
        let _ = crate::commands::telemetry::record_metric_sample(
            telemetry_session_id.as_deref(),
            instance_id,
            &sys_metrics,
            llama_sample.as_ref(),
        );

        let mut slots_req = client.get(&slots_url);
        if !api_key.is_empty() {
            slots_req = slots_req.header("Authorization", format!("Bearer {}", api_key));
        }
        if let Ok(resp) = slots_req.timeout(std::time::Duration::from_secs(2)).send() {
            if resp.status().is_success() {
                if let Ok(values) = resp.json::<Vec<serde_json::Value>>() {
                    let slots: Vec<crate::commands::telemetry::SlotSnapshotRecord> = values
                        .iter()
                        .enumerate()
                        .map(|(index, value)| {
                            let slot_id = value
                                .get("id")
                                .and_then(|item| item.as_u64())
                                .unwrap_or(index as u64)
                                as u32;
                            let is_processing = value
                                .get("is_processing")
                                .and_then(|item| item.as_bool())
                                .unwrap_or(false);
                            let n_ctx = value
                                .get("n_ctx")
                                .and_then(|item| item.as_u64())
                                .unwrap_or(0) as u32;
                            let n_past = value
                                .get("n_past")
                                .and_then(|item| item.as_u64())
                                .map(|item| item as u32);
                            crate::commands::telemetry::SlotSnapshotRecord {
                                slot_id,
                                is_processing,
                                n_ctx,
                                n_past,
                            }
                        })
                        .collect();
                    let _ = crate::commands::telemetry::record_slot_snapshots(
                        telemetry_session_id.as_deref(),
                        instance_id,
                        &slots,
                    );
                }
            }
        }

        let _ = app.emit(
            "metrics-update",
            serde_json::json!({
                "instanceId": instance_id,
                "system": sys_metrics,
                "llama": llama_metrics,
                "ts": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            }),
        );
        std::thread::sleep(
            std::time::Duration::from_secs(5).saturating_sub(iteration_started.elapsed()),
        );
    }
}

pub fn health_check_loop(
    instance_id: &str,
    host: &str,
    port: u16,
    expected_pid: u32,
    api_key: &str,
    app: tauri::AppHandle,
) {
    let url = crate::utils::http_url(host, port, "/health");
    let models_url = crate::utils::http_url(host, port, "/v1/models");
    let client = reqwest::blocking::Client::new();

    let probe_req = |probe_url: &str, timeout_secs: u64| {
        let mut req = client.get(probe_url);
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }
        req.timeout(std::time::Duration::from_secs(timeout_secs))
            .send()
    };

    let service_ready = |timeout_secs: u64| {
        let health = probe_req(&url, timeout_secs)
            .map(|resp| resp.status())
            .map_err(|err| err.to_string());
        if probe_status_is_success(&health) {
            return true;
        }
        let models = probe_req(&models_url, timeout_secs)
            .map(|resp| resp.status())
            .map_err(|err| err.to_string());
        probe_status_is_success(&models)
    };

    let is_my_instance = || {
        let st = app.state::<AppState>();
        let guard = st.running.lock().unwrap();
        guard
            .get(instance_id)
            .map(|r| r.pid == expected_pid)
            .unwrap_or(false)
    };

    // Fast initial health check: 10 x 1s = 10s
    for _ in 0..10 {
        if !is_my_instance() {
            return;
        }
        if !is_recorded_process_alive(expected_pid) {
            cleanup_running_instance(&app, instance_id, expected_pid, "process-exited");
            return;
        }
        if service_ready(2) {
            let _ = app.emit(
                "health-status",
                serde_json::json!({
                    "instanceId": instance_id, "status": "ok",
                }),
            );
            let mut fail_count = 0u32;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                if !is_my_instance() {
                    return;
                }
                if !is_recorded_process_alive(expected_pid) {
                    cleanup_running_instance(&app, instance_id, expected_pid, "process-exited");
                    return;
                }
                if service_ready(5) {
                    fail_count = 0;
                    let _ = app.emit(
                        "health-status",
                        serde_json::json!({
                            "instanceId": instance_id, "status": "ok",
                        }),
                    );
                } else {
                    fail_count += 1;
                }
                if fail_count >= 3 {
                    if !is_recorded_process_alive(expected_pid) {
                        cleanup_running_instance(&app, instance_id, expected_pid, "process-exited");
                        return;
                    }
                    let _ = app.emit(
                        "health-status",
                        serde_json::json!({
                            "instanceId": instance_id, "status": "fail",
                        }),
                    );
                    fail_count = 0;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    // Slower retry: keep trying indefinitely until server starts or instance stops
    loop {
        std::thread::sleep(std::time::Duration::from_secs(10));
        if !is_my_instance() {
            return;
        }
        if service_ready(5) {
            let _ = app.emit(
                "health-status",
                serde_json::json!({
                    "instanceId": instance_id, "status": "ok",
                }),
            );
            // Enter stable monitoring loop
            let mut fail_count = 0u32;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                if !is_my_instance() {
                    return;
                }
                if service_ready(5) {
                    fail_count = 0;
                    let _ = app.emit(
                        "health-status",
                        serde_json::json!({
                            "instanceId": instance_id, "status": "ok",
                        }),
                    );
                } else {
                    fail_count += 1;
                }
                if fail_count >= 3 {
                    let _ = app.emit(
                        "health-status",
                        serde_json::json!({
                            "instanceId": instance_id, "status": "fail",
                        }),
                    );
                    fail_count = 0;
                }
            }
        }
    }
}

#[tauri::command]
pub async fn stop_server(
    instance_id: String,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let ri = state.running.lock().unwrap().get(&instance_id).cloned();
    let mut killed = false;

    if let Some(ref ri) = ri {
        if !running_instance_matches_live_process(ri) {
            cleanup_running_instance(&app, &instance_id, ri.pid, "identity-mismatch");
            return Ok(());
        }
        // Prefer taskkill /T, including child processes; PowerShell by port is the fallback.
        #[cfg(target_os = "windows")]
        {
            let r = std::os::windows::process::CommandExt::creation_flags(
                &mut Command::new("taskkill"),
                0x08000000,
            )
            .args(["/F", "/T", "/PID", &ri.pid.to_string()])
            .output();
            killed = r.map(|o| o.status.success()).unwrap_or(false);
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            killed = Command::new("kill")
                .arg(ri.pid.to_string())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
        }
    }

    if !killed {
        if let Some(ref ri) = ri {
            killed = !is_recorded_process_alive(ri.pid);
        }
    }

    if killed {
        state.running.lock().unwrap().remove(&instance_id);
        {
            let mut restored = state.restored_runtime_instances.lock().unwrap();
            let prefix = format!("{}:", instance_id);
            restored.retain(|key| !key.starts_with(&prefix));
        }
        let _ = crate::commands::telemetry::finish_run_session(
            ri.as_ref()
                .and_then(|item| item.telemetry_session_id.as_deref()),
            None,
            "manual-stop",
        );
        if let Some(ref ri) = ri {
            let _ = crate::commands::config::update_and_persist(&state, |global| {
                if global
                    .running
                    .get(&instance_id)
                    .map(|current| current.pid == ri.pid)
                    .unwrap_or(false)
                {
                    global.running.remove(&instance_id);
                }
            });
        }
        app.emit(
            "server-stopped",
            serde_json::json!({
                "instanceId": instance_id,
            }),
        )
        .ok();
        Ok(())
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

fn read_process_identity(pid: u32) -> Option<(u64, std::path::PathBuf)> {
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
    let _ = crate::commands::config::update_and_persist(&state, |global| {
        if global
            .running
            .get(instance_id)
            .map(|current| current.pid == expected_pid)
            .unwrap_or(false)
        {
            global.running.remove(instance_id);
        }
    });
    let _ = app.emit(
        "server-stopped",
        serde_json::json!({
            "instanceId": instance_id,
            "reason": reason,
        }),
    );
    true
}

fn browser_url_for_host(host: &str, port: u16) -> Result<String, String> {
    let normalized = if host == "0.0.0.0" || host == "::" {
        "localhost".to_string()
    } else {
        host.trim().to_string()
    };
    if normalized.is_empty() {
        return Err("Invalid host: empty".into());
    }
    if let Ok(ip) = normalized.parse::<std::net::IpAddr>() {
        return Ok(match ip {
            std::net::IpAddr::V4(_) => format!("http://{}:{}", ip, port),
            std::net::IpAddr::V6(_) => format!("http://[{}]:{}", ip, port),
        });
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
    Ok(format!("http://{}:{}", normalized, port))
}

#[tauri::command]
pub async fn open_browser(host: String, port: u16) -> Result<(), String> {
    let url = browser_url_for_host(&host, port)?;
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

#[tauri::command]
pub async fn test_connection(
    host: String,
    port: u16,
    api_key: Option<String>,
    api_key_file: Option<String>,
) -> Result<String, String> {
    let effective_api_key = api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .or_else(|| {
            api_key_file
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .and_then(|path| {
                    std::fs::read_to_string(path).ok().and_then(|content| {
                        content.lines().find_map(|line| {
                            let key = line.trim().trim_start_matches('\u{feff}');
                            if key.is_empty() {
                                None
                            } else {
                                Some(key.to_string())
                            }
                        })
                    })
                })
        });
    let connect_host = if host == "0.0.0.0" || host == "::" {
        "localhost"
    } else {
        &host
    };
    let base_url = crate::utils::http_url(connect_host, port, "");
    let health_url = format!("{}/health", base_url);
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

    let models_url = format!("{}/v1/models", base_url);
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

#[tauri::command]
pub async fn check_port(port: u16, host: Option<String>) -> Result<bool, String> {
    use std::net::TcpListener;
    let bind_host = host
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("127.0.0.1");
    match TcpListener::bind((bind_host, port)) {
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
type GpuSystemSnapshot = (
    Option<f32>,
    Option<f32>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<String>,
    Option<f32>,
    Option<f64>,
    Option<f64>,
);

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
        (
            m.cpu_percent,
            m.gpu_percent,
            m.vram_used_mb,
            m.vram_total_mb,
            None,
            Some("AMD".into()),
            sys_cpu,
            sys_mem_total,
            sys_mem_used,
        )
    } else if let Some(m) = nvml::collect_metrics() {
        (
            None,
            m.gpu_percent,
            m.vram_used_mb,
            m.vram_total_mb,
            None,
            Some("NVIDIA".into()),
            sys_cpu,
            sys_mem_total,
            sys_mem_used,
        )
    } else {
        (
            None,
            None,
            None,
            None,
            None,
            None,
            sys_cpu,
            sys_mem_total,
            sys_mem_used,
        )
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

#[tauri::command]
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
        let (
            adlx_cpu,
            gpu_pct,
            vram_u,
            vram_t,
            _mem,
            gpu_vendor,
            sys_cpu,
            sys_mem_total,
            sys_mem_used,
        ) = collect_gpu_and_system();
        let mut proc_sys = System::new_all();
        let (cpu, mem) = get_process_metrics(&mut proc_sys, pid);
        SystemMetrics {
            cpu_percent: adlx_cpu.unwrap_or(cpu),
            memory_mb: if adlx_cpu.is_some() {
                _mem.unwrap_or(mem)
            } else {
                (mem * 10.0).round() / 10.0
            },
            uptime_secs: uptime,
            gpu_percent: gpu_pct,
            vram_used_mb: vram_u,
            vram_total_mb: vram_t,
            system_cpu_percent: sys_cpu,
            system_memory_total_mb: sys_mem_total,
            system_memory_used_mb: sys_mem_used,
            gpu_vendor,
        }
    })
    .await
    .map_err(|e| format!("系统指标采集失败: {}", e))?;
    Ok(result)
}

/// System health without requiring an instance. Used by Dashboard for the system resource bar.
#[tauri::command]
pub async fn get_system_health() -> Result<SystemMetrics, String> {
    let result = tokio::task::spawn_blocking(|| {
        let (
            adlx_cpu,
            gpu_pct,
            vram_u,
            vram_t,
            mem,
            gpu_vendor,
            sys_cpu,
            sys_mem_total,
            sys_mem_used,
        ) = collect_gpu_and_system();
        SystemMetrics {
            cpu_percent: adlx_cpu.unwrap_or(sys_cpu.unwrap_or(0.0)),
            memory_mb: mem.unwrap_or(sys_mem_used.unwrap_or(0.0)),
            uptime_secs: 0,
            gpu_percent: gpu_pct,
            vram_used_mb: vram_u,
            vram_total_mb: vram_t,
            system_cpu_percent: sys_cpu,
            system_memory_total_mb: sys_mem_total,
            system_memory_used_mb: sys_mem_used,
            gpu_vendor,
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
    let client = reqwest::Client::new();
    let req = client.get(url);
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

#[tauri::command]
pub async fn get_slots(
    host: String,
    port: u16,
    api_key: Option<String>,
) -> Result<Vec<SlotInfo>, String> {
    let h = if host == "0.0.0.0" {
        "localhost"
    } else {
        &host
    };
    let url = crate::utils::http_url(h, port, "/slots");
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

#[tauri::command]
pub async fn get_metrics(
    host: String,
    port: u16,
    api_key: Option<String>,
) -> Result<Option<MetricsInfo>, String> {
    let h = if host == "0.0.0.0" {
        "localhost"
    } else {
        &host
    };
    let url = crate::utils::http_url(h, port, "/metrics");
    let resp = match http_get(&url, api_key.as_deref()).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) if r.status().as_u16() == 404 => return Ok(None),
        _ => return Ok(None),
    };
    let body = resp.text().await.map_err(|e| format!("读取失败: {}", e))?;
    let sample = parse_llama_metric_sample(&body);
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

fn parse_llama_metric_sample(body: &str) -> crate::commands::telemetry::LlamaMetricSample {
    let extract = |key: &str| -> f64 {
        body.lines()
            .find(|line| line.starts_with(key))
            .and_then(|line| line.split_whitespace().last()?.parse().ok())
            .unwrap_or(0.0)
    };
    crate::commands::telemetry::LlamaMetricSample {
        tokens_per_sec: extract("llamacpp:predicted_tokens_seconds"),
        prompt_tokens: extract("llamacpp:prompt_tokens_total") as u64,
        gen_tokens: extract("llamacpp:tokens_predicted_total") as u64,
        decode_calls_total: extract("llamacpp:n_decode_total") as u64,
        max_tokens_observed: extract("llamacpp:n_tokens_max") as u64,
        prompt_tokens_per_sec: extract("llamacpp:prompt_tokens_seconds"),
        requests_processing: extract("llamacpp:requests_processing") as u64,
        requests_deferred: extract("llamacpp:requests_deferred") as u64,
        busy_slots_per_decode: extract("llamacpp:n_busy_slots_per_decode"),
    }
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
    restored.insert(restore_key)
}

pub fn reconnect_running_instance(
    instance_id: &str,
    pid: u32,
    host: &str,
    port: u16,
    config_dir: &std::path::Path,
    api_key: &str,
    app: tauri::AppHandle,
) {
    let log_path = config_dir.join("logs").join(format!("{}.log", instance_id));

    // Log recovery: replay existing content, then tail new content.
    {
        let app_log = app.clone();
        let id_log = instance_id.to_string();
        let telemetry_session_id = crate::commands::telemetry::latest_open_session_id(&id_log)
            .ok()
            .flatten();
        let workload =
            crate::commands::telemetry::session_workload(telemetry_session_id.as_deref())
                .unwrap_or(ModelWorkload::Inference);
        std::thread::spawn(move || {
            tail_log_file(&log_path, &id_log, telemetry_session_id, workload, app_log);
        });
    }

    // Metrics recovery: restart monitor_loop and keep recording with the new session.
    {
        let app_metrics = app.clone();
        let id_metrics = instance_id.to_string();
        let host_m = if host == "0.0.0.0" {
            "localhost".to_string()
        } else {
            host.to_string()
        };
        let ak = api_key.to_string();
        let telemetry_session_id = crate::commands::telemetry::latest_open_session_id(&id_metrics)
            .ok()
            .flatten();
        std::thread::spawn(move || {
            monitor_loop(
                &id_metrics,
                pid,
                &host_m,
                port,
                &ak,
                telemetry_session_id,
                app_metrics,
            );
        });
    }
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

/// Reads existing content from the log file and tails new lines through server-log events.
/// Mirrors the behavior of Docker `docker logs -f` or systemd `journalctl -f`.
fn read_log_tail(path: &std::path::Path, max_lines: usize) -> std::io::Result<(Vec<String>, u64)> {
    let mut file = std::fs::File::open(path)?;
    let file_len = file.metadata()?.len();
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
    let text = String::from_utf8_lossy(&bytes);
    let complete_text = if offset > 0 {
        text.split_once('\n').map(|(_, tail)| tail).unwrap_or("")
    } else {
        text.as_ref()
    };
    let mut lines = complete_text
        .lines()
        .rev()
        .take(max_lines)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    lines.reverse();
    Ok((lines, file_len))
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

fn tail_log_file(
    log_path: &std::path::Path,
    instance_id: &str,
    telemetry_session_id: Option<String>,
    workload: ModelWorkload,
    app: tauri::AppHandle,
) {
    use std::io::{Seek, SeekFrom};

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
    if log_path.exists() {
        if let Ok((lines, replay_end)) = read_log_tail(log_path, LOG_REPLAY_LINES) {
            last_size = replay_end;
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

        // Check whether the instance is still running.
        {
            let st = app.state::<AppState>();
            let guard = st.running.lock().unwrap();
            if !guard.contains_key(instance_id) {
                break;
            }
        }

        // Check file state.
        let current_size = match std::fs::metadata(log_path) {
            Ok(meta) => meta.len(),
            Err(_) => break, // Exit if the file was deleted.
        };

        if current_size < last_size {
            // If the file was truncated, for example by log rotation, restart from the current position.
            last_size = 0;
        }

        if current_size > last_size {
            let mut perf_changed = false;
            let mut log_lines = Vec::new();
            if let Ok(mut file) = std::fs::File::open(log_path) {
                if file.seek(SeekFrom::Start(last_size)).is_ok() {
                    let reader = BufReader::new(file);
                    for text in reader.lines().map_while(Result::ok) {
                        if text.trim().is_empty() {
                            continue;
                        }
                        log_lines.push(format!("{text}\n"));
                        if parse_perf_line(&parser, &text, &mut tasks, &mut last_completed) {
                            perf_changed = true;
                            record_completed(&last_completed, &mut last_recorded_task_id);
                        }
                    }
                }
            }
            emit_log_batches(&app, instance_id, &log_lines);
            last_size = current_size;
            if perf_changed {
                tasks.retain(|_, t| !t.completed);
                emit_perf(&app, &tasks, &last_completed);
            }
        }
    }
}

/// #13: Simple shell-style argument splitting with double-quote support.
fn split_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in input.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
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

    #[test]
    fn generate_server_command_uses_launch_normalization() {
        let cmd = tauri::async_runtime::block_on(generate_server_command(
            polluted_embedding_config(),
            String::new(),
        ))
        .unwrap();

        assert!(cmd.iter().any(|arg| arg == "--embedding"));
        assert!(cmd.windows(2).any(|args| args == ["-fa", "on"]));
        assert!(!cmd.iter().any(|arg| arg == "--spec-type"));
        assert!(!cmd.iter().any(|arg| arg == "--temp"));
    }

    #[test]
    fn start_preparation_hashes_and_launches_the_normalized_config() {
        let (config, workload, cmd) = prepare_launch(polluted_embedding_config(), "");

        assert_eq!(workload, crate::vector_policy::ModelWorkload::Embedding);
        assert!(config.embedding);
        assert!(config.spec_type.is_empty());
        assert!(config.custom_args.is_empty());
        assert!(cmd.iter().any(|arg| arg == "--embedding"));
        assert!(cmd.windows(2).any(|args| args == ["-fa", "on"]));
        assert!(!cmd.iter().any(|arg| arg == "--spec-type"));
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
        );

        assert_eq!(sample.decode_calls_total, 9);
        assert_eq!(sample.max_tokens_observed, 4096);
        assert_eq!(sample.tokens_per_sec, 12.5);
        assert_eq!(sample.prompt_tokens_per_sec, 48.0);
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
    fn browser_url_rejects_command_metacharacters_in_host() {
        let err = browser_url_for_host("127.0.0.1 & calc", 8080).unwrap_err();
        assert!(err.contains("Invalid host"));
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
        let url = browser_url_for_host("0.0.0.0", 8080).unwrap();
        assert_eq!(url, "http://localhost:8080");
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
    fn effective_api_key_reads_first_non_empty_file_line() {
        let dir = std::env::temp_dir().join(format!("lsm-api-key-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("api-key.txt");
        std::fs::write(&path, "\n file-key \n second-key \n").unwrap();

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
}
