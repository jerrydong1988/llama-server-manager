use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use crate::models::{AppState, GlobalConfig, InstanceConfig, RunningInstance};
use tauri::{Emitter, Manager};

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
        if !config.mmproj_path.is_empty() { cmd.extend_from_slice(&["--mmproj".into(), config.mmproj_path.clone()]); }
        if !config.mmproj_url.is_empty() { cmd.extend_from_slice(&["--mmproj-url".into(), config.mmproj_url.clone()]); }
        if config.mmproj_auto { cmd.push("--mmproj-auto".into()); }
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
    cmd.extend_from_slice(&["-ngl".into(), config.gpu_layers.to_string()]);
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
    if config.yarn_attn_factor != 1.0 { cmd.extend_from_slice(&["--yarn-attn-factor".into(), config.yarn_attn_factor.to_string()]); }
    if config.yarn_beta_slow > 0.0 { cmd.extend_from_slice(&["--yarn-beta-slow".into(), config.yarn_beta_slow.to_string()]); }
    if config.yarn_beta_fast != 32.0 { cmd.extend_from_slice(&["--yarn-beta-fast".into(), config.yarn_beta_fast.to_string()]); }

    // ── Flash Attention ──
    if !is_emb {
        let fa = config.flash_attn.as_str();
        if fa != "auto" && !fa.is_empty() { cmd.extend_from_slice(&["-fa".into(), fa.to_string()]); }
    }

    // ── Memory & Loading ──
    if config.moe_cpu_layers > 0 { cmd.extend_from_slice(&["--n-cpu-moe".into(), config.moe_cpu_layers.to_string()]); }
    if config.mlock { cmd.push("--mlock".into()); }
    if config.no_mmap { cmd.push("--no-mmap".into()); }
    if config.numa { cmd.extend_from_slice(&["--numa".into(), "distribute".into()]); }
    if config.check_tensors { cmd.push("--check-tensors".into()); }
    if config.fit { cmd.extend_from_slice(&["--fit".into(), "on".into()]); }

    // ── KV Cache ──
    if !config.cache_type_k.is_empty() { cmd.extend_from_slice(&["-ctk".into(), config.cache_type_k.clone()]); }
    if !config.cache_type_v.is_empty() { cmd.extend_from_slice(&["-ctv".into(), config.cache_type_v.clone()]); }
    if !config.cache_type_draft_k.is_empty() { cmd.extend_from_slice(&["-ctdk".into(), config.cache_type_draft_k.clone()]); }
    if !config.cache_type_draft_v.is_empty() { cmd.extend_from_slice(&["-ctdv".into(), config.cache_type_draft_v.clone()]); }
    if config.kv_unified { cmd.push("--kv-unified".into()); }
    if !config.cache_idle_slots { cmd.push("--no-cache-idle-slots".into()); }

    // ── GPU & Device ──
    if !config.device.is_empty() { cmd.extend_from_slice(&["-dev".into(), config.device.clone()]); }
    if !config.split_mode.is_empty() { cmd.extend_from_slice(&["-sm".into(), config.split_mode.clone()]); }
    if !config.tensor_split.is_empty() { cmd.extend_from_slice(&["-ts".into(), config.tensor_split.clone()]); }
    if config.main_gpu > 0 { cmd.extend_from_slice(&["-mg".into(), config.main_gpu.to_string()]); }
    if !config.override_kv.is_empty() { cmd.extend_from_slice(&["--override-kv".into(), config.override_kv.clone()]); }

    // ── Speculative Decoding ──
    if !is_emb {
        if !config.draft_model_path.is_empty() { cmd.extend_from_slice(&["-md".into(), config.draft_model_path.clone()]); }
        if config.draft_gpu_layers > 0 && config.draft_gpu_layers < 99 { cmd.extend_from_slice(&["-ngld".into(), config.draft_gpu_layers.to_string()]); }
        if config.draft_tokens > 0 { cmd.extend_from_slice(&["--spec-draft-n-max".into(), config.draft_tokens.to_string()]); }
        if config.spec_draft_n_min > 0 { cmd.extend_from_slice(&["--spec-draft-n-min".into(), config.spec_draft_n_min.to_string()]); }
        if !config.spec_type.is_empty() { cmd.extend_from_slice(&["--spec-type".into(), config.spec_type.clone()]); }
        if config.spec_draft_p_min > 0.0 { cmd.extend_from_slice(&["--spec-draft-p-min".into(), config.spec_draft_p_min.to_string()]); }
        if config.spec_draft_p_split != 0.1 { cmd.extend_from_slice(&["--spec-draft-p-split".into(), config.spec_draft_p_split.to_string()]); }
        if !config.spec_draft_device.is_empty() { cmd.extend_from_slice(&["--spec-draft-device".into(), config.spec_draft_device.clone()]); }
        if !config.lookup_cache_static.is_empty() { cmd.extend_from_slice(&["-lcs".into(), config.lookup_cache_static.clone()]); }
        if !config.lookup_cache_dynamic.is_empty() { cmd.extend_from_slice(&["-lcd".into(), config.lookup_cache_dynamic.clone()]); }
    }

    // ── Network ──
    cmd.extend_from_slice(&["--host".into(), config.host.clone(), "--port".into(), config.port.to_string()]);
    if !config.api_key.is_empty() { cmd.extend_from_slice(&["--api-key".into(), config.api_key.clone()]); }
    if !config.api_key_file.is_empty() { cmd.extend_from_slice(&["--api-key-file".into(), config.api_key_file.clone()]); }
    if !config.ssl_key_file.is_empty() { cmd.extend_from_slice(&["--ssl-key-file".into(), config.ssl_key_file.clone()]); }
    if !config.ssl_cert_file.is_empty() { cmd.extend_from_slice(&["--ssl-cert-file".into(), config.ssl_cert_file.clone()]); }
    if config.no_ui { cmd.push("--no-ui".into()); }
    if !config.path_prefix.is_empty() { cmd.extend_from_slice(&["--path".into(), config.path_prefix.clone()]); }
    if !config.api_prefix.is_empty() { cmd.extend_from_slice(&["--api-prefix".into(), config.api_prefix.clone()]); }
    if !config.ui_config_file.is_empty() { cmd.extend_from_slice(&["--ui-config-file".into(), config.ui_config_file.clone()]); }

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
        if config.mirostat > 0 { cmd.extend_from_slice(&["--mirostat".into(), config.mirostat.to_string()]); }
        if config.mirostat_lr > 0.0 { cmd.extend_from_slice(&["--mirostat-lr".into(), config.mirostat_lr.to_string()]); }
        if config.mirostat_ent > 0.0 { cmd.extend_from_slice(&["--mirostat-ent".into(), config.mirostat_ent.to_string()]); }
        if config.xtc_probability > 0.0 { cmd.extend_from_slice(&["--xtc-probability".into(), config.xtc_probability.to_string()]); }
        if config.xtc_threshold > 0.0 { cmd.extend_from_slice(&["--xtc-threshold".into(), config.xtc_threshold.to_string()]); }
        if config.dynatemp_range > 0.0 { cmd.extend_from_slice(&["--dynatemp-range".into(), config.dynatemp_range.to_string()]); }
        if config.dynatemp_exp > 0.0 { cmd.extend_from_slice(&["--dynatemp-exp".into(), config.dynatemp_exp.to_string()]); }
        if config.typical_p < 1.0 && config.typical_p > 0.0 { cmd.extend_from_slice(&["--typical-p".into(), config.typical_p.to_string()]); }
        if config.dry_multiplier > 0.0 { cmd.extend_from_slice(&["--dry-multiplier".into(), config.dry_multiplier.to_string()]); }
        if config.dry_base > 0.0 { cmd.extend_from_slice(&["--dry-base".into(), config.dry_base.to_string()]); }
        if config.dry_allowed_length > 0 { cmd.extend_from_slice(&["--dry-allowed-length".into(), config.dry_allowed_length.to_string()]); }
        if config.dry_penalty_last_n > 0 { cmd.extend_from_slice(&["--dry-penalty-last-n".into(), config.dry_penalty_last_n.to_string()]); }
        if !config.dry_sequence_breaker.is_empty() { cmd.extend_from_slice(&["--dry-sequence-breaker".into(), config.dry_sequence_breaker.clone()]); }
        if config.adaptive_target > 0.0 { cmd.extend_from_slice(&["--adaptive-target".into(), config.adaptive_target.to_string()]); }
        if config.adaptive_decay > 0.0 { cmd.extend_from_slice(&["--adaptive-decay".into(), config.adaptive_decay.to_string()]); }
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
    if !config.prefill_assistant.is_empty() { cmd.extend_from_slice(&["--prefill-assistant".into(), config.prefill_assistant.clone()]); }

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

    let mut child = {
        let mut c = Command::new(&cmd[0]);
        c.args(&cmd[1..]).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        { use std::os::windows::process::CommandExt; c.creation_flags(0x08000000); }
        c.spawn().map_err(|e| format!("启动服务器失败: {}\n命令: {}", e, cmd_str))?
    };

    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

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

    // 监控线程：读 stdout/stderr + 1 秒快速检测进程是否存活 + 等待退出
    let id = instance_id.clone();
    let app_clone = app.clone();
    std::thread::spawn(move || {
        let id1 = id.clone();
        let app1 = app_clone.clone();
        let stdout_handle = stdout.map(|s| {
            std::thread::spawn(move || {
                let reader = BufReader::new(s);
                for line in reader.lines() {
                    if let Ok(text) = line {
                        let _ = app1.emit("server-log", serde_json::json!({
                            "instanceId": id1, "text": format!("{}\n", text),
                        }));
                    }
                }
            })
        });

        let id2 = id.clone();
        let app2 = app_clone.clone();
        let stderr_handle = stderr.map(|s| {
            std::thread::spawn(move || {
                let reader = BufReader::new(s);
                for line in reader.lines() {
                    if let Ok(text) = line {
                        let _ = app2.emit("server-log", serde_json::json!({
                            "instanceId": id2, "text": format!("{}\n", text),
                        }));
                    }
                }
            })
        });

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
    }
}
        if let Some(h) = stdout_handle { h.join().ok(); }
        if let Some(h) = stderr_handle { h.join().ok(); }
    });

    let id = instance_id.clone();
    let app_for_health = app.clone();
    let host = if config.host == "0.0.0.0" { "localhost".to_string() } else { config.host.clone() };
    let port = config.port;
    std::thread::spawn(move || {
        health_check_loop(&id, &host, port, pid, app_for_health);
    });

    Ok(())
}

pub fn health_check_loop(instance_id: &str, host: &str, port: u16, expected_pid: u32, app: tauri::AppHandle) {
    let url = format!("http://{}:{}/health", host, port);
    let client = reqwest::blocking::Client::new();

    let is_my_instance = || {
        let st = app.state::<AppState>();
        let guard = st.running.lock().unwrap();
        guard.get(instance_id).map(|r| r.pid == expected_pid).unwrap_or(false)
    };

    for _ in 0..30 {
        if !is_my_instance() { return; }
        if let Ok(resp) = client.get(&url).timeout(std::time::Duration::from_secs(2)).send() {
            if resp.status().is_success() {
                let _ = app.emit("health-status", serde_json::json!({
                    "instanceId": instance_id, "status": "ok",
                }));
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    if !is_my_instance() { return; }
                    if let Ok(resp) = client.get(&url).timeout(std::time::Duration::from_secs(5)).send() {
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
        if let Ok(resp) = client.get(&url).timeout(std::time::Duration::from_secs(5)).send() {
            if resp.status().is_success() {
                let _ = app.emit("health-status", serde_json::json!({
                    "instanceId": instance_id, "status": "ok",
                }));
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    if !is_my_instance() { return; }
                    if let Ok(resp) = client.get(&url).timeout(std::time::Duration::from_secs(5)).send() {
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
        #[cfg(windows)]
        {
            let r = std::os::windows::process::CommandExt::creation_flags(
                &mut Command::new("taskkill"), 0x08000000)
                .args(["/F", "/PID", &ri.pid.to_string()])
                .output();
            killed = r.map(|o| o.status.success()).unwrap_or(false);
        }
        #[cfg(not(target_os = "windows"))]
        { killed = Command::new("kill").arg(ri.pid.to_string()).status().map(|s| s.success()).unwrap_or(false); }
    }

    if !killed {
        if let Some(ref ri) = ri {
            #[cfg(windows)]
            {
                let cmd = format!("try{{$p=Get-NetTCPConnection -LocalPort {} -ErrorAction Stop|Select -First 1 -ExpandProperty OwningProcess;Stop-Process -Id $p -Force;Write-Output $p}}catch{{}}", ri.port);
                let r = std::os::windows::process::CommandExt::creation_flags(
                    &mut Command::new("powershell"), 0x08000000)
                    .args(["-NoProfile", "-Command", &cmd])
                    .output();
                killed = r.map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty()).unwrap_or(false);
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
    #[cfg(windows)]
    { std::os::windows::process::CommandExt::creation_flags(
        &mut std::process::Command::new("cmd"), 0x08000000)
        .args(["/c", "start", &url]).spawn().map_err(|e| format!("{}", e))?; }
    #[cfg(not(target_os = "windows"))]
    { std::process::Command::new("open").arg(&url).spawn().map_err(|e| format!("{}", e))?; }
    Ok(())
}

#[tauri::command]
pub async fn test_connection(host: String, port: u16) -> Result<String, String> {
    let url = format!("http://{}:{}/health", if host == "0.0.0.0" { "localhost" } else { &host }, port);
    let client = reqwest::Client::new();
    match client.get(&url).timeout(std::time::Duration::from_secs(3)).send().await {
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
