use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::models::{ConfigSnapshot, DataPoint, InstanceConfig, SessionMeta, SessionSummary, SessionsIndex};

// ── 内存中的运行聚合（避免每次写入时重新扫描文件） ──
static RUNNING_AGGREGATES: std::sync::LazyLock<Mutex<HashMap<String, RunningAggregate>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

struct RunningAggregate {
    data_points: u64,
    sum_tps: f64,
    max_tps: f64,
    sum_ptps: f64,
    sum_gpu_pct: f64,
    max_gpu_pct: f64,
    max_vram_mb: f64,
    vram_total_mb: f64,
    sum_cpu_pct: f64,
    last_prompt_tok: u64,
    last_gen_tok: u64,
    // 日志提取的请求级聚合
    request_count: u64,
    sum_req_gen_tps: f64,
    max_req_gen_tps: f64,
    sum_req_prompt_tps: f64,
    total_req_prompt_tok: u64,
    total_req_gen_tok: u64,
    sum_spec_accept: f64,
    spec_accept_count: u64,
    load_time_secs: f64,
}

// ── 内部辅助 ────────────────────────────────────────────────────

fn history_dir(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("history")
}

fn index_path(config_dir: &std::path::Path) -> PathBuf {
    history_dir(config_dir).join("index.json")
}

fn session_data_path(config_dir: &std::path::Path, session_id: &str) -> PathBuf {
    history_dir(config_dir).join("sessions").join(format!("{}.jsonl", session_id))
}

fn load_index(config_dir: &std::path::Path) -> SessionsIndex {
    let path = index_path(config_dir);
    if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        SessionsIndex::default()
    }
}

fn save_index(config_dir: &std::path::Path, index: &SessionsIndex) {
    let path = index_path(config_dir);
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    if let Ok(json) = serde_json::to_string_pretty(index) {
        let _ = std::fs::write(&path, json);
    }
}

fn build_config_snapshot(config: &InstanceConfig, engine_path: &str, engine_backend: &str) -> ConfigSnapshot {
    ConfigSnapshot {
        model_path: config.model_path.clone(),
        engine_path: engine_path.to_string(),
        engine_backend: engine_backend.to_string(),
        ctx_size: config.ctx_size,
        gpu_layers: config.gpu_layers,
        batch_size: config.batch_size,
        ubatch_size: config.ubatch_size,
        threads: config.threads,
        threads_batch: config.threads_batch,
        flash_attn: config.flash_attn.clone(),
        spec_type: config.spec_type.clone(),
        draft_tokens: config.draft_tokens,
        cache_type_k: config.cache_type_k.clone(),
        cache_type_v: config.cache_type_v.clone(),
        cache_ram: config.cache_ram,
        cont_batching: config.cont_batching,
        parallel: config.parallel,
        rope_scaling: config.rope_scaling.clone(),
        rope_scale: config.rope_scale,
        mirostat: config.mirostat,
        temp: config.temp,
        top_k: config.top_k,
        top_p: config.top_p,
        host: config.host.clone(),
        port: config.port,
    }
}

// ── 公共录制接口 ────────────────────────────────────────────────

/// 创建新 Session 并写入 index.json。返回 session_id。
pub fn create_session(
    config_dir: &std::path::Path,
    instance_id: &str,
    instance_name: &str,
    config: &InstanceConfig,
    engine_path: &str,
    engine_backend: &str,
) -> Result<String, String> {
    let session_id = format!(
        "{}_{}",
        chrono::Local::now().format("%Y%m%d_%H%M%S"),
        &uuid::Uuid::new_v4().as_simple().to_string()[..8]
    );

    let now = chrono::Utc::now().timestamp();
    let meta = SessionMeta {
        id: session_id.clone(),
        instance_name: instance_name.to_string(),
        instance_id: instance_id.to_string(),
        started_at: now,
        ended_at: None,
        duration_secs: None,
        model_path: config.model_path.clone(),
        engine_backend: engine_backend.to_string(),
        config_snapshot: build_config_snapshot(config, engine_path, engine_backend),
        summary: None,
        unclean: true, // 默认为 unclean，normal shutdown 时会改为 false
    };

    // Update index
    let mut index = load_index(config_dir);
    // 清理超额：保留最近 200 个 session
    while index.sessions.len() >= 200 {
        let oldest = index.sessions.remove(0);
        // 删除旧数据文件
        let old_path = session_data_path(config_dir, &oldest.id);
        let _ = std::fs::remove_file(&old_path);
    }
    index.sessions.push(meta);
    save_index(config_dir, &index);

    // 初始化运行聚合
    RUNNING_AGGREGATES.lock().unwrap().insert(session_id.clone(), RunningAggregate {
        data_points: 0,
        sum_tps: 0.0, max_tps: 0.0, sum_ptps: 0.0,
        sum_gpu_pct: 0.0, max_gpu_pct: 0.0,
        max_vram_mb: 0.0, vram_total_mb: 0.0, sum_cpu_pct: 0.0,
        last_prompt_tok: 0, last_gen_tok: 0,
        request_count: 0,
        sum_req_gen_tps: 0.0, max_req_gen_tps: 0.0,
        sum_req_prompt_tps: 0.0,
        total_req_prompt_tok: 0, total_req_gen_tok: 0,
        sum_spec_accept: 0.0, spec_accept_count: 0,
        load_time_secs: 0.0,
    });

    // 确保 sessions 目录存在
    let data_dir = history_dir(config_dir).join("sessions");
    let _ = std::fs::create_dir_all(&data_dir);

    Ok(session_id)
}

/// 记录一条数据点到 session 的 JSONL 文件，并更新运行聚合。
pub fn record_data_point(
    config_dir: &std::path::Path,
    session_id: &str,
    dp: &DataPoint,
) {
    // 写入 JSONL
    let path = session_data_path(config_dir, session_id);
    let parent = path.parent().unwrap();
    let _ = std::fs::create_dir_all(parent);

    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        if let Ok(line) = serde_json::to_string(dp) {
            let _ = writeln!(f, "{}", line);
        }
    }

    // 更新运行聚合
    let mut aggs = RUNNING_AGGREGATES.lock().unwrap();
    if let Some(agg) = aggs.get_mut(session_id) {
        agg.data_points += 1;
        if let Some(tps) = dp.tps {
            agg.sum_tps += tps;
            if tps > agg.max_tps { agg.max_tps = tps; }
        }
        if let Some(ptps) = dp.ptps { agg.sum_ptps += ptps; }
        if let Some(gpu) = dp.gpu {
            agg.sum_gpu_pct += gpu as f64;
            if gpu as f64 > agg.max_gpu_pct { agg.max_gpu_pct = gpu as f64; }
        }
        if let Some(vm) = dp.vram_u {
            if vm > agg.max_vram_mb { agg.max_vram_mb = vm; }
        }
        if let Some(vt) = dp.vram_t { agg.vram_total_mb = vt; }
        if let Some(cpu) = dp.cpu { agg.sum_cpu_pct += cpu as f64; }
        if let Some(pt) = dp.p_tok { agg.last_prompt_tok = pt; }
        if let Some(gt) = dp.g_tok { agg.last_gen_tok = gt; }
        // 日志提取的请求级聚合
        if let Some(rtps) = dp.req_gen_tps {
            if rtps > 0.0 {
                agg.request_count += 1;
                agg.sum_req_gen_tps += rtps;
                if rtps > agg.max_req_gen_tps { agg.max_req_gen_tps = rtps; }
            }
        }
        if let Some(ptps) = dp.req_prompt_tps {
            if ptps > 0.0 { agg.sum_req_prompt_tps += ptps; }
        }
        if let Some(pt) = dp.req_prompt_tokens { agg.total_req_prompt_tok += pt; }
        if let Some(gt) = dp.req_gen_tokens { agg.total_req_gen_tok += gt; }
        if let Some(sr) = dp.spec_accept_rate {
            if sr > 0.0 { agg.sum_spec_accept += sr; agg.spec_accept_count += 1; }
        }
        if let Some(lt) = dp.load_time_secs {
            if lt > 0.0 { agg.load_time_secs = lt; }
        }
    }
}

/// 完成 Session（正常停止时调用）。计算摘要，更新 index。
pub fn finalize_session(
    config_dir: &std::path::Path,
    session_id: &str,
) {
    let ended_at = chrono::Utc::now().timestamp();

    let summary = {
        let aggs = RUNNING_AGGREGATES.lock().unwrap();
        aggs.get(session_id).map(|agg| {
            let n = agg.data_points as f64;
            SessionSummary {
                data_points: agg.data_points,
                avg_tps: if n > 0.0 && agg.sum_tps > 0.0 { Some(agg.sum_tps / n) } else { None },
                peak_tps: if agg.max_tps > 0.0 { Some(agg.max_tps) } else { None },
                avg_ptps: if n > 0.0 && agg.sum_ptps > 0.0 { Some(agg.sum_ptps / n) } else { None },
                total_prompt_tok: if agg.last_prompt_tok > 0 { Some(agg.last_prompt_tok) } else { None },
                total_gen_tok: if agg.last_gen_tok > 0 { Some(agg.last_gen_tok) } else { None },
                max_vram_mb: if agg.max_vram_mb > 0.0 { Some(agg.max_vram_mb) } else { None },
                vram_total_mb: if agg.vram_total_mb > 0.0 { Some(agg.vram_total_mb) } else { None },
                avg_gpu_pct: if n > 0.0 && agg.sum_gpu_pct > 0.0 { Some(agg.sum_gpu_pct / n) } else { None },
                avg_cpu_pct: if n > 0.0 && agg.sum_cpu_pct > 0.0 { Some(agg.sum_cpu_pct / n) } else { None },
                // 日志提取的请求级聚合
                request_count: if agg.request_count > 0 { Some(agg.request_count) } else { None },
                avg_req_gen_tps: if agg.request_count > 0 && agg.sum_req_gen_tps > 0.0 { Some(agg.sum_req_gen_tps / agg.request_count as f64) } else { None },
                peak_req_gen_tps: if agg.max_req_gen_tps > 0.0 { Some(agg.max_req_gen_tps) } else { None },
                avg_req_prompt_tps: if agg.request_count > 0 && agg.sum_req_prompt_tps > 0.0 { Some(agg.sum_req_prompt_tps / agg.request_count as f64) } else { None },
                total_req_prompt_tok: if agg.total_req_prompt_tok > 0 { Some(agg.total_req_prompt_tok) } else { None },
                total_req_gen_tok: if agg.total_req_gen_tok > 0 { Some(agg.total_req_gen_tok) } else { None },
                avg_spec_accept: if agg.spec_accept_count > 0 { Some(agg.sum_spec_accept / agg.spec_accept_count as f64) } else { None },
                load_time_secs: if agg.load_time_secs > 0.0 { Some(agg.load_time_secs) } else { None },
            }
        })
    };

    // 更新 index
    let mut index = load_index(config_dir);
    if let Some(meta) = index.sessions.iter_mut().rev().find(|s| s.id == *session_id) {
        meta.ended_at = Some(ended_at);
        let started = meta.started_at;
        if ended_at > started {
            meta.duration_secs = Some((ended_at - started) as u64);
        }
        meta.summary = summary;
        meta.unclean = false;
    }
    save_index(config_dir, &index);

    // 清理运行聚合
    RUNNING_AGGREGATES.lock().unwrap().remove(session_id);
}

// ── Tauri 命令 ──────────────────────────────────────────────────

#[tauri::command]
pub async fn list_sessions(
    state: tauri::State<'_, crate::models::AppState>,
    instance_id: Option<String>,
) -> Result<Vec<SessionMeta>, String> {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let mut index = load_index(&config_dir);

    // Filter by instance_id if specified
    if let Some(ref iid) = instance_id {
        index.sessions.retain(|s| s.instance_id == *iid);
    }

    // Sort by started_at descending (newest first)
    index.sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    Ok(index.sessions)
}

#[tauri::command]
pub async fn get_session_data(
    state: tauri::State<'_, crate::models::AppState>,
    session_id: String,
) -> Result<Vec<DataPoint>, String> {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let path = session_data_path(&config_dir, &session_id);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取失败: {}", e))?;
    let mut points = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() { continue; }
        if let Ok(dp) = serde_json::from_str::<DataPoint>(line) {
            points.push(dp);
        }
    }
    Ok(points)
}

#[tauri::command]
pub async fn delete_session(
    state: tauri::State<'_, crate::models::AppState>,
    session_id: String,
) -> Result<(), String> {
    let config_dir = state.config_dir.lock().unwrap().clone();

    // Remove from index
    let mut index = load_index(&config_dir);
    index.sessions.retain(|s| s.id != session_id);
    save_index(&config_dir, &index);

    // Remove data file
    let path = session_data_path(&config_dir, &session_id);
    let _ = std::fs::remove_file(&path);

    Ok(())
}

#[tauri::command]
pub async fn clear_all_history(
    state: tauri::State<'_, crate::models::AppState>,
) -> Result<(), String> {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let dir = history_dir(&config_dir);

    // Remove entire history directory
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("清理失败: {}", e))?;
    }

    // Recreate empty index
    save_index(&config_dir, &SessionsIndex::default());

    Ok(())
}

/// 应用退出时调用：将所有未完成 (ended_at 为 null) 的 session 标记为完成。
/// 正常停止时 monitor_loop 会调用 finalize_session，此函数处理异常退出场景。
pub fn finalize_all_running(config_dir: &std::path::Path) {
    let mut index = load_index(config_dir);
    let now = chrono::Utc::now().timestamp();
    let mut changed = false;

    for meta in index.sessions.iter_mut() {
        if meta.ended_at.is_none() {
            meta.ended_at = Some(now);
            meta.unclean = true; // 异常退出标记
            if now > meta.started_at {
                meta.duration_secs = Some((now - meta.started_at) as u64);
            }
            changed = true;
        }
    }

    if changed {
        save_index(config_dir, &index);
    }
}
