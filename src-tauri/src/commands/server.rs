use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use crate::models::{AppState, GlobalConfig, InstanceConfig, RunningInstance, SystemMetrics, DataPoint};
use crate::commands::adlx;
use crate::commands::nvml;
use crate::commands::history;
use tauri::{Emitter, Manager};
use sysinfo::System;

// ── 生成 CLI 命令 ────────────────────────────────────────────────
pub fn generate_command(config: &InstanceConfig, engine_path: &str) -> Vec<String> {
    let exe = if engine_path.is_empty() { "llama-server".to_string() } else { engine_path.to_string() };
    let mut cmd = vec![exe, "-m".into(), config.model_path.clone()];
    let is_emb = config.embedding;

    // ── Basic ──
    if !config.alias.is_empty() { cmd.extend_from_slice(&["-a".into(), config.alias.clone()]); }
    if !is_emb {
        if !config.lora_path.is_empty() { cmd.extend_from_slice(&["--lora".into(), config.lora_path.clone()]); }
        if config.lora_init_without_apply { cmd.push("--lora-init-without-apply".into()); }
        if !config.lora_scaled.is_empty() { cmd.extend_from_slice(&["--lora-scaled".into(), config.lora_scaled.clone()]); }
        if !config.mmproj_path.is_empty() { cmd.extend_from_slice(&["--mmproj".into(), config.mmproj_path.clone()]); }
        if !config.mmproj_url.is_empty() { cmd.extend_from_slice(&["--mmproj-url".into(), config.mmproj_url.clone()]); }
        if config.mmproj_auto { cmd.push("--mmproj-auto".into()); }
        if config.no_mmproj { cmd.push("--no-mmproj".into()); }
        if config.no_mmproj_offload { cmd.push("--no-mmproj-offload".into()); }
        if !config.chat_template.is_empty() { cmd.extend_from_slice(&["--chat-template".into(), config.chat_template.clone()]); }
        if !config.chat_template_file.is_empty() { cmd.extend_from_slice(&["--chat-template-file".into(), config.chat_template_file.clone()]); }
        if config.skip_chat_parsing { cmd.push("--skip-chat-parsing".into()); }
        if !config.reasoning_format.is_empty() { cmd.extend_from_slice(&["--reasoning-format".into(), config.reasoning_format.clone()]); }
        if !config.reasoning.is_empty() { cmd.extend_from_slice(&["--reasoning".into(), config.reasoning.clone()]); }
        if !config.reasoning_budget.is_empty() { cmd.extend_from_slice(&["--reasoning-budget".into(), config.reasoning_budget.clone()]); }
        if !config.reasoning_budget_message.is_empty() { cmd.extend_from_slice(&["--reasoning-budget-message".into(), config.reasoning_budget_message.clone()]); }
        if !config.reasoning_effort.is_empty() {
            let re = format!("{{\"reasoning_effort\": \"{}\"}}", config.reasoning_effort);
            cmd.extend_from_slice(&["--chat-template-kwargs".into(), re]);
        }
        if config.jinja { cmd.push("--jinja".into()); }
        if !config.grammar_file.is_empty() { cmd.extend_from_slice(&["--grammar-file".into(), config.grammar_file.clone()]); }
        if !config.grammar.is_empty() { cmd.extend_from_slice(&["--grammar".into(), config.grammar.clone()]); }
    }

    // ── Performance & Context ──
    if !config.ctx_size_auto { cmd.extend_from_slice(&["-c".into(), config.ctx_size.to_string()]); }
    if !config.gpu_layers_auto { cmd.extend_from_slice(&["-ngl".into(), config.gpu_layers.to_string()]); }
    if config.threads > 0 { cmd.extend_from_slice(&["-t".into(), config.threads.to_string()]); }
    if config.batch_size > 0 { cmd.extend_from_slice(&["-b".into(), config.batch_size.to_string()]); }
    if config.ubatch_size > 0 { cmd.extend_from_slice(&["-ub".into(), config.ubatch_size.to_string()]); }
    if config.parallel > 0 || config.parallel == -1 { cmd.extend_from_slice(&["-np".into(), config.parallel.to_string()]); }
    if config.cont_batching { cmd.push("-cb".into()); }
    if !config.cache_prompt { cmd.push("--no-cache-prompt".into()); }
    if config.threads_batch > 0 { cmd.extend_from_slice(&["--threads-batch".into(), config.threads_batch.to_string()]); }
    if config.threads_http >= 0 { cmd.extend_from_slice(&["--threads-http".into(), config.threads_http.to_string()]); }
    if config.keep > 0 { cmd.extend_from_slice(&["--keep".into(), config.keep.to_string()]); }
    if config.cache_reuse > 0 { cmd.extend_from_slice(&["--cache-reuse".into(), config.cache_reuse.to_string()]); }
    if config.cache_ram > 0 { cmd.extend_from_slice(&["-cram".into(), config.cache_ram.to_string()]); }
    if config.warmup { cmd.push("--warmup".into()); }
    if config.ctx_checkpoints != 32 { cmd.extend_from_slice(&["-ctxcp".into(), config.ctx_checkpoints.to_string()]); }
    if config.checkpoint_min_step > 0 { cmd.extend_from_slice(&["-cms".into(), config.checkpoint_min_step.to_string()]); }
    if config.swa_full { cmd.push("--swa-full".into()); }

    // ── RoPE / YaRN ──
    if !config.rope_scaling.is_empty() { cmd.extend_from_slice(&["--rope-scaling".into(), config.rope_scaling.clone()]); }
    if config.rope_scale > 0.0 { cmd.extend_from_slice(&["--rope-scale".into(), config.rope_scale.to_string()]); }
    if config.rope_freq_base > 0.0 { cmd.extend_from_slice(&["--rope-freq-base".into(), config.rope_freq_base.to_string()]); }
    if config.rope_freq_scale > 0.0 { cmd.extend_from_slice(&["--rope-freq-scale".into(), config.rope_freq_scale.to_string()]); }
    if config.yarn_ext_factor >= 0.0 { cmd.extend_from_slice(&["--yarn-ext-factor".into(), config.yarn_ext_factor.to_string()]); }
    if config.yarn_attn_factor != -1.0 { cmd.extend_from_slice(&["--yarn-attn-factor".into(), config.yarn_attn_factor.to_string()]); }
    if config.yarn_beta_slow > 0.0 { cmd.extend_from_slice(&["--yarn-beta-slow".into(), config.yarn_beta_slow.to_string()]); }
    if config.yarn_beta_fast != -1.0 { cmd.extend_from_slice(&["--yarn-beta-fast".into(), config.yarn_beta_fast.to_string()]); }
    if config.yarn_orig_ctx > 0 { cmd.extend_from_slice(&["--yarn-orig-ctx".into(), config.yarn_orig_ctx.to_string()]); }

    // ── Flash Attention ──
    if !is_emb {
        let fa = config.flash_attn.as_str();
        if fa != "auto" && !fa.is_empty() { cmd.extend_from_slice(&["-fa".into(), fa.to_string()]); }
    }

    // ── Memory & Loading ──
    if config.moe_cpu_layers > 0 { cmd.extend_from_slice(&["--n-cpu-moe".into(), config.moe_cpu_layers.to_string()]); }
    if config.mlock { cmd.push("--mlock".into()); }
    if config.no_mmap { cmd.push("--no-mmap".into()); }
    if config.no_repack { cmd.push("--no-repack".into()); }
    if config.numa { cmd.extend_from_slice(&["--numa".into(), "distribute".into()]); }
    if config.check_tensors { cmd.push("--check-tensors".into()); }
    if config.perf { cmd.push("--perf".into()); }
    if config.fit { cmd.extend_from_slice(&["--fit".into(), "on".into()]); }
    if !config.fit_target.is_empty() { cmd.extend_from_slice(&["-fitt".into(), config.fit_target.clone()]); }
    if config.fit_ctx != 4096 { cmd.extend_from_slice(&["-fitc".into(), config.fit_ctx.to_string()]); }

    // ── KV Cache ──
    if !config.cache_type_k.is_empty() { cmd.extend_from_slice(&["-ctk".into(), config.cache_type_k.clone()]); }
    if !config.cache_type_v.is_empty() { cmd.extend_from_slice(&["-ctv".into(), config.cache_type_v.clone()]); }
    if !config.cache_type_draft_k.is_empty() { cmd.extend_from_slice(&["-ctkd".into(), config.cache_type_draft_k.clone()]); }
    if !config.cache_type_draft_v.is_empty() { cmd.extend_from_slice(&["-ctvd".into(), config.cache_type_draft_v.clone()]); }
    if config.kv_unified { cmd.push("--kv-unified".into()); }
    if config.no_kv_offload { cmd.push("--no-kv-offload".into()); }
    if !config.cache_idle_slots { cmd.push("--no-cache-idle-slots".into()); }

    // ── GPU & Device ──
    if !config.device.is_empty() { cmd.extend_from_slice(&["-dev".into(), config.device.clone()]); }
    if !config.split_mode.is_empty() { cmd.extend_from_slice(&["-sm".into(), config.split_mode.clone()]); }
    if !config.tensor_split.is_empty() { cmd.extend_from_slice(&["-ts".into(), config.tensor_split.clone()]); }
    if config.main_gpu > 0 { cmd.extend_from_slice(&["-mg".into(), config.main_gpu.to_string()]); }
    if !config.override_kv.is_empty() { cmd.extend_from_slice(&["--override-kv".into(), config.override_kv.clone()]); }

    // ── Speculative Decoding ──
    let spec_active = !is_emb && !config.spec_type.is_empty() && config.spec_type != "none";
    if spec_active {
        if !config.draft_model_path.is_empty() { cmd.extend_from_slice(&["-md".into(), config.draft_model_path.clone()]); }
        if config.draft_gpu_layers > 0 && config.draft_gpu_layers < 99 { cmd.extend_from_slice(&["-ngld".into(), config.draft_gpu_layers.to_string()]); }
        if config.draft_tokens > 0 { cmd.extend_from_slice(&["--spec-draft-n-max".into(), config.draft_tokens.to_string()]); }
        if config.spec_draft_n_min > 0 { cmd.extend_from_slice(&["--spec-draft-n-min".into(), config.spec_draft_n_min.to_string()]); }
        cmd.extend_from_slice(&["--spec-type".into(), config.spec_type.clone()]);
        if config.spec_draft_p_min > 0.0 { cmd.extend_from_slice(&["--spec-draft-p-min".into(), config.spec_draft_p_min.to_string()]); }
        if config.spec_draft_p_split != 0.1 { cmd.extend_from_slice(&["--spec-draft-p-split".into(), config.spec_draft_p_split.to_string()]); }
        if !config.spec_draft_device.is_empty() { cmd.extend_from_slice(&["--spec-draft-device".into(), config.spec_draft_device.clone()]); }
        if !config.lookup_cache_static.is_empty() { cmd.extend_from_slice(&["-lcs".into(), config.lookup_cache_static.clone()]); }
        if !config.lookup_cache_dynamic.is_empty() { cmd.extend_from_slice(&["-lcd".into(), config.lookup_cache_dynamic.clone()]); }
        if config.spec_default { cmd.push("--spec-default".into()); }
        if !config.spec_draft_backend_sampling { cmd.push("--no-spec-draft-backend-sampling".into()); }
        if config.spec_draft_threads > 0 { cmd.extend_from_slice(&["-td".into(), config.spec_draft_threads.to_string()]); }
        if config.spec_draft_threads_batch > 0 { cmd.extend_from_slice(&["-tbd".into(), config.spec_draft_threads_batch.to_string()]); }
    }

    // ── Network ──
    cmd.extend_from_slice(&["--host".into(), config.host.clone(), "--port".into(), config.port.to_string()]);
    if !config.api_key.is_empty() { cmd.extend_from_slice(&["--api-key".into(), config.api_key.clone()]); }
    if !config.api_key_file.is_empty() { cmd.extend_from_slice(&["--api-key-file".into(), config.api_key_file.clone()]); }
    if !config.ssl_key_file.is_empty() { cmd.extend_from_slice(&["--ssl-key-file".into(), config.ssl_key_file.clone()]); }
    if !config.ssl_cert_file.is_empty() { cmd.extend_from_slice(&["--ssl-cert-file".into(), config.ssl_cert_file.clone()]); }
    if config.no_ui { cmd.push("--no-ui".into()); }
    if config.offline { cmd.push("--offline".into()); }
    if !config.path_prefix.is_empty() { cmd.extend_from_slice(&["--path".into(), config.path_prefix.clone()]); }
    if !config.api_prefix.is_empty() { cmd.extend_from_slice(&["--api-prefix".into(), config.api_prefix.clone()]); }
    if !config.ui_config_file.is_empty() { cmd.extend_from_slice(&["--ui-config-file".into(), config.ui_config_file.clone()]); }
    if !config.ui_config.is_empty() { cmd.extend_from_slice(&["--ui-config".into(), config.ui_config.clone()]); }
    if config.ui_mcp_proxy { cmd.push("--ui-mcp-proxy".into()); }

    // ── Embedding / Generation ──
    if config.embedding {
        cmd.push("--embedding".into());
        if !config.pooling.is_empty() { cmd.extend_from_slice(&["--pooling".into(), config.pooling.clone()]); }
        if config.embd_normalize != 2 { cmd.extend_from_slice(&["--embd-normalize".into(), config.embd_normalize.to_string()]); }
        if config.reranking { cmd.push("--reranking".into()); }
    } else {
        if config.n_predict > 0 { cmd.extend_from_slice(&["-n".into(), config.n_predict.to_string()]); }
        else if config.n_predict == 0 {} else { cmd.extend_from_slice(&["-n".into(), "-1".into()]); }
        if config.ignore_eos { cmd.push("--ignore-eos".into()); }
        if !config.json_schema.is_empty() { cmd.extend_from_slice(&["--json-schema".into(), config.json_schema.clone()]); }
        if !config.json_schema_file.is_empty() { cmd.extend_from_slice(&["-jf".into(), config.json_schema_file.clone()]); }
        if config.temp > 0.0 { cmd.extend_from_slice(&["--temp".into(), config.temp.to_string()]); }
        if config.top_k > 0 { cmd.extend_from_slice(&["--top-k".into(), config.top_k.to_string()]); }
        if config.top_p > 0.0 { cmd.extend_from_slice(&["--top-p".into(), config.top_p.to_string()]); }
        if config.repeat_penalty > 0.0 { cmd.extend_from_slice(&["--repeat-penalty".into(), config.repeat_penalty.to_string()]); }
        if config.seed >= 0 { cmd.extend_from_slice(&["--seed".into(), config.seed.to_string()]); }
        if config.min_p > 0.0 { cmd.extend_from_slice(&["--min-p".into(), config.min_p.to_string()]); }
        if config.presence_penalty > 0.0 { cmd.extend_from_slice(&["--presence-penalty".into(), config.presence_penalty.to_string()]); }
        if config.frequency_penalty > 0.0 { cmd.extend_from_slice(&["--frequency-penalty".into(), config.frequency_penalty.to_string()]); }
        if config.repeat_last_n > 0 { cmd.extend_from_slice(&["--repeat-last-n".into(), config.repeat_last_n.to_string()]); }
        if !config.reverse_prompt.is_empty() { cmd.extend_from_slice(&["-r".into(), config.reverse_prompt.clone()]); }
        if config.special { cmd.push("-sp".into()); }
        if config.spm_infill { cmd.push("--spm-infill".into()); }
        if config.backend_sampling { cmd.push("-bs".into()); }

        // Advanced sampling
        if config.mirostat > 0 {
            cmd.extend_from_slice(&["--mirostat".into(), config.mirostat.to_string()]);
            if config.mirostat_lr > 0.0 { cmd.extend_from_slice(&["--mirostat-lr".into(), config.mirostat_lr.to_string()]); }
            if config.mirostat_ent > 0.0 { cmd.extend_from_slice(&["--mirostat-ent".into(), config.mirostat_ent.to_string()]); }
        }
        if config.xtc_probability > 0.0 {
            cmd.extend_from_slice(&["--xtc-probability".into(), config.xtc_probability.to_string()]);
            if config.xtc_threshold > 0.0 { cmd.extend_from_slice(&["--xtc-threshold".into(), config.xtc_threshold.to_string()]); }
        }
        if config.dynatemp_range > 0.0 {
            cmd.extend_from_slice(&["--dynatemp-range".into(), config.dynatemp_range.to_string()]);
            if config.dynatemp_exp > 0.0 { cmd.extend_from_slice(&["--dynatemp-exp".into(), config.dynatemp_exp.to_string()]); }
        }
        if config.typical_p < 1.0 && config.typical_p > 0.0 { cmd.extend_from_slice(&["--typical-p".into(), config.typical_p.to_string()]); }
        if config.dry_multiplier > 0.0 {
            cmd.extend_from_slice(&["--dry-multiplier".into(), config.dry_multiplier.to_string()]);
            if config.dry_base > 0.0 { cmd.extend_from_slice(&["--dry-base".into(), config.dry_base.to_string()]); }
            if config.dry_allowed_length > 0 { cmd.extend_from_slice(&["--dry-allowed-length".into(), config.dry_allowed_length.to_string()]); }
            if config.dry_penalty_last_n > 0 { cmd.extend_from_slice(&["--dry-penalty-last-n".into(), config.dry_penalty_last_n.to_string()]); }
            if !config.dry_sequence_breaker.is_empty() { cmd.extend_from_slice(&["--dry-sequence-breaker".into(), config.dry_sequence_breaker.clone()]); }
        }
        if config.adaptive_target > 0.0 {
            cmd.extend_from_slice(&["--adaptive-target".into(), config.adaptive_target.to_string()]);
            if config.adaptive_decay > 0.0 { cmd.extend_from_slice(&["--adaptive-decay".into(), config.adaptive_decay.to_string()]); }
        }
        if config.top_n_sigma >= 0.0 { cmd.extend_from_slice(&["--top-n-sigma".into(), config.top_n_sigma.to_string()]); }
        if !config.logit_bias.is_empty() { cmd.extend_from_slice(&["-l".into(), config.logit_bias.clone()]); }
        if !config.samplers.is_empty() { cmd.extend_from_slice(&["--samplers".into(), config.samplers.clone()]); }
        if !config.sampler_seq.is_empty() { cmd.extend_from_slice(&["--sampler-seq".into(), config.sampler_seq.clone()]); }
    }

    // ── Server features ──
    if config.timeout > 0 { cmd.extend_from_slice(&["-to".into(), config.timeout.to_string()]); }
    if config.sleep_idle >= 0 { cmd.extend_from_slice(&["--sleep-idle-seconds".into(), config.sleep_idle.to_string()]); }
    if config.context_shift { cmd.push("--context-shift".into()); }
    if config.verbose { cmd.push("-v".into()); }
    if config.metrics { cmd.push("--metrics".into()); }
    if config.props { cmd.push("--props".into()); }
    if !config.slots_enabled { cmd.push("--no-slots".into()); }
    if !config.slot_save_path.is_empty() { cmd.extend_from_slice(&["--slot-save-path".into(), config.slot_save_path.clone()]); }
    if (config.slot_prompt_similarity - 0.1).abs() > f32::EPSILON { cmd.extend_from_slice(&["-sps".into(), config.slot_prompt_similarity.to_string()]); }
    if config.prefill_assistant { cmd.push("--prefill-assistant".into()); }

    // ── New server features (aligned with llama.cpp master) ──
    if !config.rpc_servers.is_empty() { cmd.extend_from_slice(&["--rpc".into(), config.rpc_servers.clone()]); }
    if config.sse_ping_interval != 30 { cmd.extend_from_slice(&["--sse-ping-interval".into(), config.sse_ping_interval.to_string()]); }
    if config.reuse_port { cmd.push("--reuse-port".into()); }

    // ── Multi-Model & Media ──
    if !config.models_dir.is_empty() { cmd.extend_from_slice(&["--models-dir".into(), config.models_dir.clone()]); }
    if !config.models_preset.is_empty() { cmd.extend_from_slice(&["--models-preset".into(), config.models_preset.clone()]); }
    if config.models_max != 4 { cmd.extend_from_slice(&["--models-max".into(), config.models_max.to_string()]); }
    if config.models_autoload { cmd.push("--models-autoload".into()); }
    if config.image_min_tokens > 0 { cmd.extend_from_slice(&["--image-min-tokens".into(), config.image_min_tokens.to_string()]); }
    if config.image_max_tokens > 0 { cmd.extend_from_slice(&["--image-max-tokens".into(), config.image_max_tokens.to_string()]); }
    if !config.tags.is_empty() { cmd.extend_from_slice(&["--tags".into(), config.tags.clone()]); }
    if !config.media_path.is_empty() { cmd.extend_from_slice(&["--media-path".into(), config.media_path.clone()]); }
    if !config.tools.is_empty() { cmd.extend_from_slice(&["--tools".into(), config.tools.clone()]); }

    // Custom args
    for arg in &config.custom_args {
        if !arg.is_empty() {
            cmd.extend(arg.split_whitespace().map(String::from));
        }
    }

    cmd
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
    {
        let running = state.running.lock().unwrap();
        if running.contains_key(&instance_id) {
            return Err("该实例已在运行中".to_string());
        }
    }

    let cmd = generate_command(&config, &engine_exe);
    let cmd_str = cmd.join(" ");

    // 创建日志文件 — llama-server 直接写到这里（不经过 pipe 中转）
    let config_dir = state.config_dir.lock().unwrap().clone();
    let log_dir = config_dir.join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join(format!("{}.log", instance_id));
    let log_file = std::fs::File::create(&log_path)
        .map_err(|e| format!("无法创建日志文件: {}", e))?;
    // clone 一份 handle 给 stderr（Stdio::from 会消耗 File）
    let log_stderr = log_file.try_clone()
        .map_err(|e| format!("无法复制文件句柄: {}", e))?;

    let mut child = {
        let mut c = Command::new(&cmd[0]);
        c.args(&cmd[1..])
         .stdout(Stdio::from(log_file))
         .stderr(Stdio::from(log_stderr));
        #[cfg(windows)]
        { use std::os::windows::process::CommandExt; c.creation_flags(0x08000000); }
        c.spawn().map_err(|e| format!("启动服务器失败: {}\n命令: {}", e, cmd_str))?
    };

    let pid = child.id();

    state.running.lock().unwrap().insert(instance_id.clone(), RunningInstance {
        instance_id: instance_id.clone(),
        pid,
        port: config.port,
        host: config.host.clone(),
        start_time: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
    });

    // 立即同步写 running 到磁盘
    {
        let config_dir = state.config_dir.lock().unwrap().clone();
        let path = config_dir.join("instances.json");
        let _ = std::fs::create_dir_all(&config_dir);
        if let Ok(json) = std::fs::read_to_string(&path) {
            if let Ok(mut global) = serde_json::from_str::<GlobalConfig>(&json) {
                global.running = state.running.lock().unwrap().clone();
                let _ = std::fs::write(&path, serde_json::to_string_pretty(&global).unwrap_or_default());
            }
        }
    }

    app.emit("server-started", serde_json::json!({
        "instanceId": instance_id,
        "pid": pid,
        "port": config.port,
        "command": cmd_str,
    })).ok();

    // P2: 创建历史记录 Session
    let session_id = history::create_session(
        &config_dir,
        &instance_id,
        &config.name,
        &config,
        &engine_exe,
        &engine_backend,
    ).unwrap_or_default();

    // ── 日志 tail 线程：读取 llama-server 直接写入的日志文件 ──
    let app_tail = app.clone();
    let id_tail = instance_id.clone();
    let log_path_tail = log_path.clone();
    std::thread::spawn(move || {
        tail_log_file(&log_path_tail, &id_tail, app_tail);
    });

    // ── 进程存活监控 + 退出清理 ──
    let id = instance_id.clone();
    let app_clone = app.clone();
    let config_dir_clone = config_dir.clone();
    let sid_clone = session_id.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        match child.try_wait() {
            Ok(Some(status)) if !status.success() => {
                let st = app_clone.state::<AppState>();
                { let mut r = st.running.lock().unwrap();
                  if r.get(&id).map(|ri| ri.pid == pid).unwrap_or(false) { r.remove(&id); } }
                let _ = app_clone.emit("server-error", serde_json::json!({
                    "instanceId": id,
                    "error": format!("进程启动后立即退出 (exit code: {:?})", status.code()),
                }));
                return;
            }
            Ok(None) => {}
            _ => {}
        }

        let _ = child.wait();
        let st2 = app_clone.state::<AppState>();
        { let mut r = st2.running.lock().unwrap();
          if r.get(&id).map(|ri| ri.pid == pid).unwrap_or(false) {
            r.remove(&id);
            let _ = app_clone.emit("server-stopped", serde_json::json!({ "instanceId": id }));
        } }
        // P2: 完成历史记录
        if !sid_clone.is_empty() {
            history::finalize_session(&config_dir_clone, &sid_clone);
        }
    });

    let id = instance_id.clone();
    let app_for_health = app.clone();
    let host = if config.host == "0.0.0.0" { "localhost".to_string() } else { config.host.clone() };
    let port = config.port;
    let api_key_health = config.api_key.clone();
    std::thread::spawn(move || {
        health_check_loop(&id, &host, port, pid, &api_key_health, app_for_health);
    });

    // P1/P2: 后台指标推送 + 历史记录线程
    let id_metrics = instance_id.clone();
    let app_metrics = app.clone();
    let host_metrics = if config.host == "0.0.0.0" { "localhost".to_string() } else { config.host.clone() };
    let port_metrics = config.port;
    let api_key_metrics = config.api_key.clone();
    let sid_metrics = session_id.clone();
    std::thread::spawn(move || {
        monitor_loop(&id_metrics, pid, &host_metrics, port_metrics, &api_key_metrics, &config_dir, &sid_metrics, app_metrics);
    });

    Ok(())
}

/// 后台指标采集循环 — 每 5 秒采集并推送到前端 + 记录历史
fn monitor_loop(
    instance_id: &str,
    expected_pid: u32,
    host: &str,
    port: u16,
    api_key: &str,
    config_dir: &std::path::Path,
    session_id: &str,
    app: tauri::AppHandle,
) {
    // 等待 llama-server 启动完成（给 3 秒启动时间）
    std::thread::sleep(std::time::Duration::from_secs(3));

    let is_my_instance = || {
        let st = app.state::<AppState>();
        let guard = st.running.lock().unwrap();
        guard.get(instance_id).map(|r| r.pid == expected_pid).unwrap_or(false)
    };

    let client = reqwest::blocking::Client::new();
    let metrics_url = format!("http://{}:{}/metrics", host, port);

    loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
        if !is_my_instance() { break; }

        // ── 系统指标 ──
        let (sys_cpu, sys_mem_total, sys_mem_used) = get_system_level_metrics();
        let mut gpu_vendor: Option<String> = None;
        let mut gpu_pct: Option<f32> = None;
        let mut vram_u: Option<f64> = None;
        let mut vram_t: Option<f64> = None;

        // 进程级指标
        let (cpu, mem) = get_process_metrics(expected_pid);
        let mut cpu_pct = cpu;
        let mut mem_mb = (mem * 10.0).round() / 10.0;

        // GPU: ADLX → NVML
        if let Some(m) = adlx::collect_metrics() {
            if let Some(c) = m.cpu_percent { cpu_pct = c; }
            if let Some(mb) = m.memory_mb { mem_mb = mb; }
            gpu_pct = m.gpu_percent;
            vram_u = m.vram_used_mb;
            vram_t = m.vram_total_mb;
            gpu_vendor = Some("AMD".into());
        } else if let Some(m) = nvml::collect_metrics() {
            gpu_pct = m.gpu_percent;
            vram_u = m.vram_used_mb;
            vram_t = m.vram_total_mb;
            gpu_vendor = Some("NVIDIA".into());
        }

        let sys_metrics = serde_json::json!({
            "cpu_percent": cpu_pct,
            "memory_mb": mem_mb,
            "uptime_secs": 0, // uptime not tracked in event stream
            "gpu_percent": gpu_pct,
            "vram_used_mb": vram_u,
            "vram_total_mb": vram_t,
            "system_cpu_percent": sys_cpu,
            "system_memory_total_mb": sys_mem_total,
            "system_memory_used_mb": sys_mem_used,
            "gpu_vendor": gpu_vendor,
        });

        // ── llama 指标 ──
        let mut llama_metrics: Option<serde_json::Value> = None;
        let mut req = client.get(&metrics_url);
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }
        if let Ok(resp) = req
            .timeout(std::time::Duration::from_secs(2))
            .send()
        {
            if resp.status().is_success() {
                if let Ok(body) = resp.text() {
                    let extract = |key: &str| -> f64 {
                        body.lines()
                            .find(|l| l.starts_with(key))
                            .and_then(|l| l.split_whitespace().last()?.parse().ok())
                            .unwrap_or(0.0)
                    };
                    llama_metrics = Some(serde_json::json!({
                        "tokens_per_sec": extract("llamacpp:predicted_tokens_seconds"),
                        "prompt_tokens": extract("llamacpp:prompt_tokens_total") as u64,
                        "gen_tokens": extract("llamacpp:tokens_predicted_total") as u64,
                        "requests": extract("llamacpp:n_decode_total") as u64,
                        "prompt_tokens_per_sec": extract("llamacpp:prompt_tokens_seconds"),
                        "requests_processing": extract("llamacpp:requests_processing") as u64,
                        "requests_deferred": extract("llamacpp:requests_deferred") as u64,
                        "busy_slots_per_decode": extract("llamacpp:n_busy_slots_per_decode"),
                    }));
                }
            }
        }

        // ── P2: 记录历史数据点 ──
        if !session_id.is_empty() {
            let tps_val = llama_metrics.as_ref()
                .and_then(|m| m.get("tokens_per_sec"))
                .and_then(|v| v.as_f64());
            let ptps_val = llama_metrics.as_ref()
                .and_then(|m| m.get("prompt_tokens_per_sec"))
                .and_then(|v| v.as_f64());
            let p_tok_val = llama_metrics.as_ref()
                .and_then(|m| m.get("prompt_tokens"))
                .and_then(|v| v.as_u64());
            let g_tok_val = llama_metrics.as_ref()
                .and_then(|m| m.get("gen_tokens"))
                .and_then(|v| v.as_u64());
            let proc_val = llama_metrics.as_ref()
                .and_then(|m| m.get("requests_processing"))
                .and_then(|v| v.as_u64());
            let def_val = llama_metrics.as_ref()
                .and_then(|m| m.get("requests_deferred"))
                .and_then(|v| v.as_u64());
            let busy_val = llama_metrics.as_ref()
                .and_then(|m| m.get("busy_slots_per_decode"))
                .and_then(|v| v.as_f64());

            // ── 日志提取：每请求剖面 ──
            let log_path = config_dir.join("logs").join(format!("{}.log", instance_id));
            let perf = parse_perf_log(&log_path);

            history::record_data_point(config_dir, session_id, &DataPoint {
                ts: chrono::Utc::now().timestamp(),
                cpu: Some(cpu_pct),
                mem_mb: Some(mem_mb),
                gpu: gpu_pct,
                vram_u: vram_u,
                vram_t: vram_t,
                tps: tps_val,
                ptps: ptps_val,
                p_tok: p_tok_val,
                g_tok: g_tok_val,
                proc: proc_val,
                def: def_val,
                busy: busy_val,
                req_prompt_tps: perf.req_prompt_tps,
                req_gen_tps: perf.req_gen_tps,
                req_prompt_tokens: perf.req_prompt_tokens,
                req_gen_tokens: perf.req_gen_tokens,
                spec_accept_rate: perf.spec_accept_rate,
                spec_accepted: perf.spec_accepted,
                spec_generated: perf.spec_generated,
                load_time_secs: perf.load_time_secs,
            });
        }

        // ── 发射事件 ──
        let _ = app.emit("metrics-update", serde_json::json!({
            "instanceId": instance_id,
            "system": sys_metrics,
            "llama": llama_metrics,
            "ts": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }));
    }
}

pub fn health_check_loop(instance_id: &str, host: &str, port: u16, expected_pid: u32, api_key: &str, app: tauri::AppHandle) {
    let url = format!("http://{}:{}/health", host, port);
    let client = reqwest::blocking::Client::new();

    let health_req = |timeout_secs: u64| {
        let mut req = client.get(&url);
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }
        req.timeout(std::time::Duration::from_secs(timeout_secs)).send()
    };

    let is_my_instance = || {
        let st = app.state::<AppState>();
        let guard = st.running.lock().unwrap();
        guard.get(instance_id).map(|r| r.pid == expected_pid).unwrap_or(false)
    };

    for _ in 0..30 {
        if !is_my_instance() { return; }
        if let Ok(resp) = health_req(2) {
            if resp.status().is_success() {
                let _ = app.emit("health-status", serde_json::json!({
                    "instanceId": instance_id, "status": "ok",
                }));
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    if !is_my_instance() { return; }
                    if let Ok(resp) = health_req(5) {
                        if !resp.status().is_success() {
                            std::thread::sleep(std::time::Duration::from_secs(3));
                        } else {
                            let _ = app.emit("health-status", serde_json::json!({
                                "instanceId": instance_id, "status": "ok",
                            }));
                        }
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    loop {
        std::thread::sleep(std::time::Duration::from_secs(3));
        if !is_my_instance() { return; }
        if let Ok(resp) = health_req(5) {
            if resp.status().is_success() {
                let _ = app.emit("health-status", serde_json::json!({
                    "instanceId": instance_id, "status": "ok",
                }));
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    if !is_my_instance() { return; }
                    if let Ok(resp) = health_req(5) {
                        if !resp.status().is_success() {
                            std::thread::sleep(std::time::Duration::from_secs(3));
                        } else {
                            let _ = app.emit("health-status", serde_json::json!({
                                "instanceId": instance_id, "status": "ok",
                            }));
                        }
                    }
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
        #[cfg(target_os = "windows")]
        {
            let r = std::os::windows::process::CommandExt::creation_flags(
                &mut Command::new("taskkill"), 0x08000000)
                .args(["/F", "/PID", &ri.pid.to_string()])
                .output();
            killed = r.map(|o| o.status.success()).unwrap_or(false);
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        { killed = Command::new("kill").arg(ri.pid.to_string()).status().map(|s| s.success()).unwrap_or(false); }
    }

    if !killed {
        if let Some(ref ri) = ri {
            #[cfg(target_os = "windows")]
            {
                let cmd = format!("try{{$p=Get-NetTCPConnection -LocalPort {} -ErrorAction Stop|Select -First 1 -ExpandProperty OwningProcess;Stop-Process -Id $p -Force;Write-Output $p}}catch{{}}", ri.port);
                let r = std::os::windows::process::CommandExt::creation_flags(
                    &mut Command::new("powershell"), 0x08000000)
                    .args(["-NoProfile", "-Command", &cmd])
                    .output();
                killed = r.map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty()).unwrap_or(false);
            }
            #[cfg(target_os = "macos")]
            {
                let port_str = ri.port.to_string();
                if let Ok(out) = Command::new("lsof").args(["-ti", &format!(":{}", port_str)]).output() {
                    let pids = String::from_utf8_lossy(&out.stdout);
                    for pid in pids.lines() {
                        let _ = Command::new("kill").arg("-9").arg(pid).status();
                        killed = true;
                    }
                }
            }
            #[cfg(target_os = "linux")]
            {
                let _ = Command::new("fuser").args(["-k", &format!("{}/tcp", ri.port)]).status();
                // fuser -k exits 0 even if nothing was killed; treat as best-effort
                killed = true;
            }
        }
    }

    if killed {
        state.running.lock().unwrap().remove(&instance_id);
    }

    app.emit("server-stopped", serde_json::json!({
        "instanceId": instance_id,
    })).ok();

    Ok(())
}

#[tauri::command]
pub async fn open_browser(host: String, port: u16) -> Result<(), String> {
    let host = if host == "0.0.0.0" { "localhost" } else { &host };
    let url = format!("http://{}:{}", host, port);
    #[cfg(target_os = "windows")]
    { std::os::windows::process::CommandExt::creation_flags(
        &mut std::process::Command::new("cmd"), 0x08000000)
        .args(["/c", "start", &url]).spawn().map_err(|e| format!("{}", e))?; }
    #[cfg(target_os = "macos")]
    { std::process::Command::new("open").arg(&url).spawn().map_err(|e| format!("{}", e))?; }
    #[cfg(target_os = "linux")]
    { std::process::Command::new("xdg-open").arg(&url).spawn().map_err(|e| format!("{}", e))?; }
    Ok(())
}

#[tauri::command]
pub async fn test_connection(host: String, port: u16, api_key: Option<String>) -> Result<String, String> {
    let url = format!("http://{}:{}/health", if host == "0.0.0.0" { "localhost" } else { &host }, port);
    match http_get(&url, api_key.as_deref()).timeout(std::time::Duration::from_secs(3)).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                Ok("✓ 连接成功，服务正常".into())
            } else {
                Err(format!("服务返回 HTTP {}", resp.status()))
            }
        }
        Err(e) => Err(format!("无法连接: {}", e))
    }
}

#[tauri::command]
pub async fn check_port(port: u16) -> Result<bool, String> {
    use std::net::TcpListener;
    match TcpListener::bind(format!("127.0.0.1:{}", port)) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

// ── 系统性能指标（sysinfo） ──────────────────────────────────

fn get_process_metrics(pid: u32) -> (f32, f64) {
    let mut sys = System::new_all();
    sys.refresh_all();
    std::thread::sleep(std::time::Duration::from_millis(300));
    sys.refresh_cpu_all();
    if let Some(p) = sys.processes().values().find(|p| p.pid().as_u32() == pid) {
        let raw = p.cpu_usage();
        let cpu = if raw > 100.0 { raw / sys.cpus().len() as f32 } else { raw };
        (cpu, p.memory() as f64 / (1024.0 * 1024.0))
    } else {
        (0.0, 0.0)
    }
}

fn get_system_level_metrics() -> (Option<f32>, Option<f64>, Option<f64>) {
    let mut sys = System::new_all();
    sys.refresh_all();
    std::thread::sleep(std::time::Duration::from_millis(100));
    sys.refresh_cpu_all();
    sys.refresh_memory();

    let cpu = {
        let usage = sys.global_cpu_usage();
        if usage > 0.0 { Some(usage) } else { None }
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
        .unwrap().as_secs() - start_time;

    // System-level CPU/RAM (always collected, independent of GPU)
    let (sys_cpu, sys_mem_total, sys_mem_used) = get_system_level_metrics();

    // ── GPU: ADLX → NVML → sysinfo fallback ──
    // Try ADLX first (AMD)
    if let Some(m) = adlx::collect_metrics() {
        return Ok(SystemMetrics {
            cpu_percent: m.cpu_percent.unwrap_or(0.0),
            memory_mb: m.memory_mb.unwrap_or(0.0),
            uptime_secs: uptime,
            gpu_percent: m.gpu_percent,
            vram_used_mb: m.vram_used_mb,
            vram_total_mb: m.vram_total_mb,
            system_cpu_percent: sys_cpu,
            system_memory_total_mb: sys_mem_total,
            system_memory_used_mb: sys_mem_used,
            gpu_vendor: Some("AMD".into()),
        });
    }

    // Try NVML (NVIDIA)
    if let Some(m) = nvml::collect_metrics() {
        let (cpu, mem) = get_process_metrics(pid);
        return Ok(SystemMetrics {
            cpu_percent: cpu,
            memory_mb: (mem * 10.0).round() / 10.0,
            uptime_secs: uptime,
            gpu_percent: m.gpu_percent,
            vram_used_mb: m.vram_used_mb,
            vram_total_mb: m.vram_total_mb,
            system_cpu_percent: sys_cpu,
            system_memory_total_mb: sys_mem_total,
            system_memory_used_mb: sys_mem_used,
            gpu_vendor: Some("NVIDIA".into()),
        });
    }

    // Fallback: sysinfo only
    let (cpu, mem) = get_process_metrics(pid);
    Ok(SystemMetrics {
        cpu_percent: cpu,
        memory_mb: (mem * 10.0).round() / 10.0,
        uptime_secs: uptime,
        gpu_percent: None,
        vram_used_mb: None,
        vram_total_mb: None,
        system_cpu_percent: sys_cpu,
        system_memory_total_mb: sys_mem_total,
        system_memory_used_mb: sys_mem_used,
        gpu_vendor: None,
    })
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

#[tauri::command]
pub async fn get_slots(host: String, port: u16, api_key: Option<String>) -> Result<Vec<SlotInfo>, String> {
    let h = if host == "0.0.0.0" { "localhost" } else { &host };
    let url = format!("http://{}:{}/slots", h, port);
    let resp = http_get(&url, api_key.as_deref()).send().await.map_err(|e| format!("请求失败: {}", e))?;
    let arr: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
    Ok(arr.iter().enumerate().map(|(i, v)| SlotInfo {
        id: v.get("id").and_then(|s| s.as_u64()).unwrap_or(i as u64) as u32,
        is_processing: v.get("is_processing").and_then(|s| s.as_bool()).unwrap_or(false),
        n_ctx: v.get("n_ctx").and_then(|s| s.as_u64()).unwrap_or(0) as u32,
    }).collect())
}

#[derive(serde::Serialize)]
pub struct MetricsInfo {
    pub tokens_per_sec: f64,
    pub prompt_tokens: u64,
    pub gen_tokens: u64,
    pub requests: u64,
    pub prompt_tokens_per_sec: f64,
    pub requests_processing: u64,
    pub requests_deferred: u64,
    pub busy_slots_per_decode: f64,
}

#[tauri::command]
pub async fn get_metrics(host: String, port: u16, api_key: Option<String>) -> Result<Option<MetricsInfo>, String> {
    let h = if host == "0.0.0.0" { "localhost" } else { &host };
    let url = format!("http://{}:{}/metrics", h, port);
    let resp = match http_get(&url, api_key.as_deref()).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) if r.status().as_u16() == 404 => return Ok(None),
        _ => return Ok(None),
    };
    let body = resp.text().await.map_err(|e| format!("读取失败: {}", e))?;
    let extract = |key: &str| -> f64 {
        body.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().last()?.parse().ok())
            .unwrap_or(0.0)
    };
    Ok(Some(MetricsInfo {
        tokens_per_sec: extract("llamacpp:predicted_tokens_seconds"),
        prompt_tokens: extract("llamacpp:prompt_tokens_total") as u64,
        gen_tokens: extract("llamacpp:tokens_predicted_total") as u64,
        requests: extract("llamacpp:n_decode_total") as u64,
        prompt_tokens_per_sec: extract("llamacpp:prompt_tokens_seconds"),
        requests_processing: extract("llamacpp:requests_processing") as u64,
        requests_deferred: extract("llamacpp:requests_deferred") as u64,
        busy_slots_per_decode: extract("llamacpp:n_busy_slots_per_decode"),
    }))
}

// ── 重启后恢复日志和监控 ─────────────────────────────────────────

/// 应用重启后，为已运行的实例重新建立日志捕获和指标推送。
/// stdout/stderr 管道已不可用，改为从日志文件读取（tail 模式）。
/// 同时创建新的 history session 继续记录。
pub fn reconnect_running_instance(
    instance_id: &str,
    pid: u32,
    host: &str,
    port: u16,
    config_dir: &std::path::Path,
    session_id: &str,
    app: tauri::AppHandle,
) {
    let log_path = config_dir.join("logs").join(format!("{}.log", instance_id));

    // ── 日志恢复：先回放已有内容，再 tail 新内容 ──
    {
        let app_log = app.clone();
        let id_log = instance_id.to_string();
        std::thread::spawn(move || {
            tail_log_file(&log_path, &id_log, app_log);
        });
    }

    // ── 指标恢复：重新启动 monitor_loop，使用新 session 继续记录 ──
    {
        let app_metrics = app.clone();
        let id_metrics = instance_id.to_string();
        let host_m = if host == "0.0.0.0" { "localhost".to_string() } else { host.to_string() };
        let config_dir_m = config_dir.to_path_buf();
        let sid_m = session_id.to_string();
        std::thread::spawn(move || {
            monitor_loop(&id_metrics, pid, &host_m, port, "", &config_dir_m, &sid_m, app_metrics);
        });
    }
}

/// 从日志文件读取已有内容并持续 tail 新行，通过 server-log 事件推送。
/// 对标 Docker `docker logs -f` / systemd `journalctl -f` 的实现模式。
fn tail_log_file(log_path: &std::path::Path, instance_id: &str, app: tauri::AppHandle) {
    use std::io::{Seek, SeekFrom};

    // ── 阶段 1: 回放已有内容（最后 2000 行，覆盖前端 1000 条上限） ──
    if log_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&log_path) {
            let lines: Vec<&str> = content.lines().collect();
            let start = if lines.len() > 2000 { lines.len() - 2000 } else { 0 };
            for line in &lines[start..] {
                if line.trim().is_empty() { continue; }
                let _ = app.emit("server-log", serde_json::json!({
                    "instanceId": instance_id,
                    "text": format!("{}\n", line),
                }));
            }
        }
    }

    // ── 阶段 2: 持续 tail 新内容 ──
    // 等待一小段时间让任何正在进行的写入完成（避免读到半行）
    std::thread::sleep(std::time::Duration::from_millis(200));

    let mut last_size = std::fs::metadata(log_path).map(|m| m.len()).unwrap_or(0);

    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));

        // 检查实例是否还在运行
        {
            let st = app.state::<AppState>();
            let guard = st.running.lock().unwrap();
            if !guard.contains_key(instance_id) { break; }
        }

        // 检查文件状态
        let current_size = match std::fs::metadata(log_path) {
            Ok(meta) => meta.len(),
            Err(_) => break, // 文件被删除，退出
        };

        if current_size < last_size {
            // 文件被截断（例如日志轮转），从当前位置重新开始
            last_size = 0;
        }

        if current_size > last_size {
            // 有新增内容
            if let Ok(mut file) = std::fs::File::open(log_path) {
                if file.seek(SeekFrom::Start(last_size)).is_ok() {
                    let reader = BufReader::new(file);
                    for line in reader.lines() {
                        if let Ok(text) = line {
                            if text.trim().is_empty() { continue; }
                            let _ = app.emit("server-log", serde_json::json!({
                                "instanceId": instance_id,
                                "text": format!("{}\n", text),
                            }));
                        }
                    }
                }
            }
            last_size = current_size;
        }
    }
}

// ── 日志性能解析 ────────────────────────────────────────────────

struct PerfSnapshot {
    req_prompt_tps: Option<f64>,
    req_gen_tps: Option<f64>,
    req_prompt_tokens: Option<u64>,
    req_gen_tokens: Option<u64>,
    spec_accept_rate: Option<f64>,
    spec_accepted: Option<u64>,
    spec_generated: Option<u64>,
    load_time_secs: Option<f64>,
}

/// 从日志文件尾部提取 llama-server --perf 输出的性能事件。
/// 只读取最后 ~2000 字节，解析结构化的计时和推测解码统计。
fn parse_perf_log(log_path: &std::path::Path) -> PerfSnapshot {
    let mut snap = PerfSnapshot {
        req_prompt_tps: None, req_gen_tps: None,
        req_prompt_tokens: None, req_gen_tokens: None,
        spec_accept_rate: None, spec_accepted: None, spec_generated: None,
        load_time_secs: None,
    };

    let content = match std::fs::read_to_string(log_path) {
        Ok(c) => c,
        Err(_) => return snap,
    };

    // 只解析尾部（最近的输出）
    let tail: &str = if content.len() > 4000 {
        &content[content.len() - 4000..]
    } else {
        &content
    };

    for line in tail.lines() {
        // "slot 0: prompt eval time = 8864.03 ms / 7997 tokens ( 902.19 t/s)"
        if line.contains("prompt eval time") {
            if let Some(tps) = extract_value(line, "(", " t/s") {
                snap.req_prompt_tps = Some(tps);
            }
            // extract prompt token count
            if let Some(pos) = line.find("/") {
                let rest = &line[pos..];
                if let Some(end) = rest.find(" tokens") {
                    let tok_str = rest[1..end].trim();
                    snap.req_prompt_tokens = tok_str.parse::<u64>().ok();
                }
            }
        }

        // "slot 0: eval time = 185707.23 ms / 9377 tokens ( 50.49 t/s)"
        if line.contains("eval time") && !line.contains("prompt eval") {
            if let Some(tps) = extract_value(line, "(", " t/s") {
                snap.req_gen_tps = Some(tps);
            }
            if let Some(pos) = line.find("/") {
                let rest = &line[pos..];
                if let Some(end) = rest.find(" tokens") {
                    let tok_str = rest[1..end].trim();
                    snap.req_gen_tokens = tok_str.parse::<u64>().ok();
                }
            }
        }

        // "drafted 8636 / accepted 7217 (83.57%)"
        if line.contains("drafted") && line.contains("accepted") {
            if let Some(rate) = extract_value(line, "(", "%)") {
                snap.spec_accept_rate = Some(rate / 100.0);
            }
            if let Some(pos) = line.find("drafted ") {
                let rest = &line[pos + 8..];
                if let Some(slash) = rest.find(" /") {
                    snap.spec_generated = rest[..slash].trim().parse::<u64>().ok();
                }
            }
            if let Some(pos) = line.find("accepted ") {
                let rest = &line[pos + 9..];
                if let Some(paren) = rest.find(" (") {
                    snap.spec_accepted = rest[..paren].trim().parse::<u64>().ok();
                }
            }
        }

        // "load_tensors: loaded 35B params in 29.00s" or "load_tensors: buffer size = XX MiB"
        if line.contains("load_tensors") && line.contains("in ") {
            if let Some(pos) = line.find("in ") {
                let rest = &line[pos + 3..];
                if let Some(end) = rest.find('s') {
                    snap.load_time_secs = rest[..end].trim().parse::<f64>().ok();
                }
            }
        }
    }

    snap
}

/// 从文本中提取 `prefix ... value suffix` 中的数值。
fn extract_value(line: &str, prefix: &str, suffix: &str) -> Option<f64> {
    let start = line.find(prefix)? + prefix.len();
    let rest = &line[start..];
    let end = rest.find(suffix)?;
    rest[..end].trim().parse::<f64>().ok()
}
