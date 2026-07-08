use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use crate::models::{AppState, DownloadArtifactState, MsFileEntry, PersistedQueueEntry};
use crate::utils;
use tauri::{Emitter, Manager};
use futures_util::StreamExt;

// Shared HTTP client for all download operations
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default()
});

// ── #9: 共享下载核心 ─────────────────────────────────────────────

const LOW_PRIORITY_FALLBACK_LIMIT_BYTES_PER_SEC: u64 = 2 * 1024 * 1024;

fn effective_download_bandwidth_limit(state: &AppState) -> u64 {
    let configured = *state.download_bandwidth_limit_bytes_per_sec.lock().unwrap();
    let low_priority = *state.download_low_priority_throttle.lock().unwrap();
    if !low_priority {
        return configured;
    }
    if configured > 0 {
        (configured / 2).max(1)
    } else {
        LOW_PRIORITY_FALLBACK_LIMIT_BYTES_PER_SEC
    }
}

fn effective_download_concurrency(state: &AppState) -> usize {
    let configured = (*state.download_max_concurrent.lock().unwrap()).max(1);
    if *state.download_low_priority_throttle.lock().unwrap() {
        1
    } else {
        configured
    }
}

async fn throttle_download_bytes(state: &AppState, bytes: u64) {
    if bytes == 0 {
        return;
    }

    loop {
        let wait = {
            let limit = effective_download_bandwidth_limit(state);
            if limit == 0 {
                return;
            }

            let mut limiter = state.download_bandwidth_limiter.lock().unwrap();
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(limiter.last_refill).as_secs_f64();
            let limit_f64 = limit as f64;
            let capacity = limit_f64.max(bytes as f64);
            limiter.available_bytes = (limiter.available_bytes + elapsed * limit_f64).min(capacity);
            limiter.last_refill = now;

            if limiter.available_bytes >= bytes as f64 {
                limiter.available_bytes -= bytes as f64;
                None
            } else {
                let deficit = bytes as f64 - limiter.available_bytes;
                limiter.available_bytes = 0.0;
                Some(std::time::Duration::from_secs_f64(deficit / limit_f64))
            }
        };

        if let Some(duration) = wait {
            if duration.is_zero() {
                return;
            }
            tokio::time::sleep(duration).await;
        } else {
            return;
        }
    }
}

fn sanitize_repo_id(repo_id: &str) -> Result<String, String> {
    if repo_id.is_empty() {
        return Err("仓库 ID 不能为空".to_string());
    }
    if repo_id.contains("..") || repo_id.contains('\\') {
        return Err(format!("无效的仓库 ID: {}", repo_id));
    }
    #[cfg(target_os = "windows")]
    {
        if repo_id.len() >= 2 && repo_id.as_bytes()[1] == b':' {
            return Err(format!("无效的仓库 ID: {}", repo_id));
        }
    }
    for c in repo_id.chars() {
        if !c.is_alphanumeric() && c != '/' && c != '-' && c != '_' && c != '.' {
            return Err(format!("仓库 ID 包含非法字符: {}", repo_id));
        }
    }
    Ok(repo_id.to_string())
}

fn sanitize_file_name(name: &str) -> Result<String, String> {
    if name.is_empty() { return Err("文件名不能为空".into()); }
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(format!("文件名包含非法路径字符: {}", name));
    }
    #[cfg(target_os = "windows")]
    {
        if name.len() >= 2 && name.as_bytes()[1] == b':' {
            return Err(format!("文件名包含非法路径字符: {}", name));
        }
    }
    // 确保 name 是纯文件名（不含路径分隔符）
    let path = Path::new(name);
    if path.file_name().and_then(|s| s.to_str()) != Some(name) {
        return Err(format!("文件名包含路径分隔符: {}", name));
    }
    Ok(name.to_string())
}

/// RAII guard: removes run_id from active_downloads on drop (all exit paths including panic)
struct ActiveDownloadGuard {
    app: tauri::AppHandle,
    run_id: String,
}
impl Drop for ActiveDownloadGuard {
    fn drop(&mut self) {
        self.app.state::<AppState>().active_downloads.lock().unwrap().remove(&self.run_id);
    }
}

fn build_download_paths(save_dir: &Path, file_name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let final_path = save_dir.join(file_name);
    let temp_path = save_dir.join(format!("{}.part", file_name));
    let metadata_path = save_dir.join(format!("{}.part.json", file_name));
    (final_path, temp_path, metadata_path)
}

fn artifact_state_path(temp_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.json", temp_path.display()))
}

fn load_artifact_state(temp_path: &Path) -> Option<DownloadArtifactState> {
    let metadata_path = artifact_state_path(temp_path);
    std::fs::read_to_string(&metadata_path)
        .ok()
        .and_then(|s| serde_json::from_str::<DownloadArtifactState>(&s).ok())
}

fn save_artifact_state(state: &DownloadArtifactState, temp_path: &Path) {
    let metadata_path = artifact_state_path(temp_path);
    if let Ok(json) = serde_json::to_string(state) {
        let _ = std::fs::write(&metadata_path, json);
    }
}

fn cleanup_artifact_state(temp_path: &Path) {
    let metadata_path = artifact_state_path(temp_path);
    let _ = std::fs::remove_file(temp_path);
    let _ = std::fs::remove_file(&metadata_path);
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone)]
struct DownloadTaskContext {
    task_id: String,
    run_id: String,
    version: u32,
    file_name: String,
    repo_id: String,
    source: String,
    remote_path: String,
}

impl DownloadTaskContext {
    fn emit(&self, app: &tauri::AppHandle, event: &str, extra: serde_json::Value) {
        let mut payload = serde_json::Map::new();
        payload.insert("taskId".into(), serde_json::Value::String(self.task_id.clone()));
        payload.insert("runId".into(), serde_json::Value::String(self.run_id.clone()));
        payload.insert("version".into(), serde_json::Value::Number(self.version.into()));
        payload.insert("fileName".into(), serde_json::Value::String(self.file_name.clone()));
        payload.insert("repoId".into(), serde_json::Value::String(self.repo_id.clone()));
        payload.insert("source".into(), serde_json::Value::String(self.source.clone()));
        payload.insert("remotePath".into(), serde_json::Value::String(self.remote_path.clone()));

        if let serde_json::Value::Object(extra_map) = extra {
            payload.extend(extra_map);
        }

        let _ = app.emit(event, serde_json::Value::Object(payload));
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeDownloadTaskResult {
    pub task_id: String,
    pub run_id: String,
    pub version: u32,
}

fn build_task_context(file: &MsFileEntry, repo_id: &str, source: &str) -> DownloadTaskContext {
    DownloadTaskContext {
        task_id: file.task_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        run_id: file.run_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        version: file.version.unwrap_or(0),
        file_name: file.name.clone(),
        repo_id: repo_id.to_string(),
        source: source.to_string(),
        remote_path: file.path.clone(),
    }
}

fn resolve_repo_save_path(app: &tauri::AppHandle, save_dir: &str, repo_id: &str) -> Result<PathBuf, String> {
    let base_path = if Path::new(save_dir).is_absolute() {
        PathBuf::from(save_dir)
    } else {
        let managed = app.state::<AppState>();
        let config_dir = managed.config_dir.lock().unwrap();
        config_dir.parent().unwrap_or(Path::new(".")).to_path_buf().join(save_dir)
    };
    let save_path = base_path.join(repo_id.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
    std::fs::create_dir_all(&save_path).map_err(|e| format!("Failed to create download directory: {}", e))?;
    Ok(save_path)
}

fn clear_control_flags_for_files(state: &AppState, files: &[MsFileEntry]) {
    let mut flags = state.cancel_flags.lock().unwrap();
    let mut pause = state.pause_flags.lock().unwrap();
    for file in files {
        if let Some(run_id) = &file.run_id {
            flags.remove(run_id);
            pause.remove(run_id);
        }
    }
}

fn is_retryable_error(status_code: Option<u16>) -> bool {
    match status_code {
        Some(429) => true,
        Some(code) if code >= 500 => true,
        _ => false,
    }
}

/// 下载单个文件的通用逻辑。
async fn download_single_file(
    ctx: DownloadTaskContext,
    url: String,
    save_path: PathBuf,
    file_size: u64,
    app: tauri::AppHandle,
    has_error: Arc<AtomicBool>,
) {
    let file_name = match sanitize_file_name(&ctx.file_name) {
        Ok(n) => n,
        Err(e) => {
            has_error.store(true, Ordering::SeqCst);
            ctx.emit(&app, "download-error", serde_json::json!({
                "error": e,
                "retryable": false,
            }));
            return;
        }
    };
    let (final_path, temp_path, _metadata_path) = build_download_paths(&save_path, &file_name);
    let shared = app.state::<AppState>();

    {
        let mut active = shared.active_downloads.lock().unwrap();
        if !active.insert(ctx.run_id.clone()) {
            // 该文件已有活跃下载任务，跳过（避免两个任务同时写同一个文件）
            has_error.store(true, Ordering::SeqCst);
            ctx.emit(&app, "download-error", serde_json::json!({
                "error": "Download task is already active",
                "retryable": false,
            }));
            return;
        }
    }
    // RAII guard: 函数退出时自动从 active_downloads 中移除
    let _guard = ActiveDownloadGuard { app: app.clone(), run_id: ctx.run_id.clone() };
    let task_id = ctx.task_id.clone();
    let run_id = ctx.run_id.clone();
    let repo_id = ctx.repo_id.clone();
    let source = ctx.source.clone();
    let remote_path = ctx.remote_path.clone();

    let artifact = load_artifact_state(&temp_path);
    let mut save_etag = artifact.as_ref().and_then(|a| a.etag.clone());
    let mut save_lm = artifact.as_ref().and_then(|a| a.last_modified.clone());
    let resume_from = artifact
        .as_ref()
        .map(|a| a.downloaded_size)
        .unwrap_or_else(|| temp_path.metadata().map(|m| m.len()).unwrap_or(0));

    if shared.cancel_flags.lock().unwrap().get(&run_id).copied().unwrap_or(false) {
        if !shared.pause_flags.lock().unwrap().get(&run_id).copied().unwrap_or(false) {
                save_artifact_state(&DownloadArtifactState {
                    task_id: task_id.clone(), run_id: run_id.clone(),
                    repo_id: repo_id.clone(), source: source.clone(),
                    remote_path: remote_path.clone(),
                    final_path: final_path.to_string_lossy().to_string(),
                    temp_path: temp_path.to_string_lossy().to_string(),
                    expected_size: file_size, downloaded_size: resume_from,
                    etag: save_etag.clone(), last_modified: save_lm.clone(), updated_at: now_secs(),
                }, &temp_path);
                update_manager_file_state(&shared, &task_id, FileStatePatch {
                    downloaded: Some(resume_from),
                    version: Some(ctx.version),
                    status: Some("cancelled".into()),
                    ..Default::default()
                });
                let _ = app.emit("download-cancelled", serde_json::json!({
                    "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
                    "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                }));
        }
        return;
    }

    let mut req = HTTP_CLIENT.get(&url).header("User-Agent", "Mozilla/5.0");
    if resume_from > 0 {
        req = req.header("Range", format!("bytes={}-", resume_from));
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            save_artifact_state(&DownloadArtifactState {
                task_id: task_id.clone(), run_id: run_id.clone(),
                repo_id: repo_id.clone(), source: source.clone(),
                remote_path: remote_path.clone(),
                final_path: final_path.to_string_lossy().to_string(),
                temp_path: temp_path.to_string_lossy().to_string(),
                expected_size: file_size, downloaded_size: resume_from,
                etag: save_etag.clone(), last_modified: save_lm.clone(), updated_at: now_secs(),
            }, &temp_path);
            has_error.store(true, Ordering::SeqCst);
            update_manager_file_state(&shared, &task_id, FileStatePatch {
                downloaded: Some(resume_from),
                version: Some(ctx.version),
                status: Some("error".into()),
                error: Some(Some(e.to_string())),
                ..Default::default()
            });
            let _ = app.emit("download-error", serde_json::json!({
                "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name, "error": e.to_string(),
                "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                "retryable": true,
            }));
            return;
        }
    };
    // A1-06: Read ETag and Last-Modified from response headers
    let resp_etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(|s| s.to_string());
    let resp_last_modified = resp.headers().get("last-modified").and_then(|v| v.to_str().ok()).map(|s| s.to_string());
    save_etag = resp_etag.clone();
    save_lm = resp_last_modified.clone();
    // A1-06: Persist updated artifact state immediately after reading headers
    save_artifact_state(&DownloadArtifactState {
        task_id: task_id.clone(), run_id: run_id.clone(),
        repo_id: repo_id.clone(), source: source.clone(),
        remote_path: remote_path.clone(),
        final_path: final_path.to_string_lossy().to_string(),
        temp_path: temp_path.to_string_lossy().to_string(),
        expected_size: file_size, downloaded_size: resume_from,
        etag: save_etag.clone(), last_modified: save_lm.clone(),
        updated_at: now_secs(),
    }, &temp_path);

    // A1-05: Handle 416 Range Not Satisfiable with file size intelligence
    if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        let part_size = temp_path.metadata().map(|m| m.len()).unwrap_or(0);
        if part_size >= file_size {
            if final_path.exists() { let _ = std::fs::remove_file(&final_path); }
            if std::fs::rename(&temp_path, &final_path).is_ok() {
                cleanup_artifact_state(&temp_path);
                update_manager_file_state(&shared, &task_id, FileStatePatch {
                    downloaded: Some(file_size),
                    size: Some(file_size),
                    version: Some(ctx.version),
                    status: Some("completed".into()),
                    error: Some(None),
                    ..Default::default()
                });
                let _ = app.emit("download-complete", serde_json::json!({
                    "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
                    "path": final_path.to_string_lossy(),
                    "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                }));
            }
            return;
        }
        if part_size > file_size {
            let _ = std::fs::remove_file(&temp_path);
            save_artifact_state(&DownloadArtifactState {
                task_id: task_id.clone(), run_id: run_id.clone(),
                repo_id: repo_id.clone(), source: source.clone(),
                remote_path: remote_path.clone(),
                final_path: final_path.to_string_lossy().to_string(),
                temp_path: temp_path.to_string_lossy().to_string(),
                expected_size: file_size, downloaded_size: 0,
                etag: save_etag.clone(), last_modified: save_lm.clone(), updated_at: now_secs(),
            }, &temp_path);
            has_error.store(true, Ordering::SeqCst);
            update_manager_file_state(&shared, &task_id, FileStatePatch {
                downloaded: Some(0),
                version: Some(ctx.version),
                status: Some("error".into()),
                error: Some(Some("Download corrupted: part file larger than expected".into())),
                ..Default::default()
            });
        let _ = app.emit("download-error", serde_json::json!({
            "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
            "error": "Download corrupted: part file larger than expected",
            "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
            "retryable": false,
        }));
            return;
        }
        save_artifact_state(&DownloadArtifactState {
            task_id: task_id.clone(), run_id: run_id.clone(),
            repo_id: repo_id.clone(), source: source.clone(),
            remote_path: remote_path.clone(),
            final_path: final_path.to_string_lossy().to_string(),
            temp_path: temp_path.to_string_lossy().to_string(),
            expected_size: file_size, downloaded_size: 0,
            etag: save_etag.clone(), last_modified: save_lm.clone(), updated_at: now_secs(),
        }, &temp_path);
        has_error.store(true, Ordering::SeqCst);
        update_manager_file_state(&shared, &task_id, FileStatePatch {
            downloaded: Some(0),
            version: Some(ctx.version),
            status: Some("error".into()),
            error: Some(Some("Server does not support resume, please restart download".into())),
            ..Default::default()
        });
        let _ = app.emit("download-error", serde_json::json!({
            "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
            "error": "Server does not support resume, please restart download",
            "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
            "retryable": false,
        }));
        return;
    }

    if !resp.status().is_success() && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        let status_code = resp.status().as_u16();
        let msg = match status_code {
            404 => "文件不存在 / File not found",
            403 => "访问被拒绝 / Access denied",
            429 => "请求过于频繁，请稍后重试 / Too many requests, please retry later",
            code => &format!("HTTP {}", code),
        };
        let retryable = is_retryable_error(Some(status_code));
        save_artifact_state(&DownloadArtifactState {
            task_id: task_id.clone(), run_id: run_id.clone(),
            repo_id: repo_id.clone(), source: source.clone(),
            remote_path: remote_path.clone(),
            final_path: final_path.to_string_lossy().to_string(),
            temp_path: temp_path.to_string_lossy().to_string(),
            expected_size: file_size, downloaded_size: resume_from,
            etag: save_etag.clone(), last_modified: save_lm.clone(), updated_at: now_secs(),
        }, &temp_path);
        has_error.store(true, Ordering::SeqCst);
        update_manager_file_state(&shared, &task_id, FileStatePatch {
            downloaded: Some(resume_from),
            version: Some(ctx.version),
            status: Some("error".into()),
            error: Some(Some(msg.to_string())),
            ..Default::default()
        });
        let _ = app.emit("download-error", serde_json::json!({
            "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name, "error": msg,
            "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
            "retryable": retryable,
        }));
        return;
    }

    let is_partial = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let mut resume_from = if is_partial { resume_from } else { 0 };

    // A1-05: 200 OK with .part file means server ignored Range header → restart
    if !is_partial && temp_path.exists() {
        update_manager_file_state(&shared, &task_id, FileStatePatch {
            downloaded: Some(0),
            version: Some(ctx.version),
            status: Some("active".into()),
            error: Some(None),
            ..Default::default()
        });
        let _ = app.emit("download-restarted", serde_json::json!({
            "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
            "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
        }));
        resume_from = 0;
    }

    // A1-06: Check if remote file changed (ETag/Last-Modified mismatch on resume)
    if let Some(ref old_state) = artifact {
        let etag_changed = resp_etag.is_some() && old_state.etag.is_some() && resp_etag != old_state.etag;
        let lm_changed = resp_last_modified.is_some() && old_state.last_modified.is_some() && resp_last_modified != old_state.last_modified;
        if (etag_changed || lm_changed) && temp_path.exists() {
            update_manager_file_state(&shared, &task_id, FileStatePatch {
                downloaded: Some(0),
                version: Some(ctx.version),
                status: Some("active".into()),
                error: Some(None),
                ..Default::default()
            });
            let _ = app.emit("download-remote-changed", serde_json::json!({
                "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
                "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
            }));
            let _ = std::fs::remove_file(&temp_path);
            resume_from = 0;
            save_etag = resp_etag;
            save_lm = resp_last_modified;
        }
    }
    let total = if resume_from > 0 {
        resp.content_length().unwrap_or(0) + resume_from
    } else {
        resp.content_length().unwrap_or(file_size)
    };
    let mut downloaded = resume_from;
    let mut win_start = std::time::Instant::now();
    let mut win_bytes: u64 = 0;
    let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(1); // 首次立即发射

    use std::io::Write;
    let mut file = match std::fs::OpenOptions::new()
        .create(true).append(is_partial).truncate(!is_partial).write(true)
        .open(&temp_path) {
        Ok(f) => f,
        Err(e) => {
            save_artifact_state(&DownloadArtifactState {
                task_id: task_id.clone(), run_id: run_id.clone(),
                repo_id: repo_id.clone(), source: source.clone(),
                remote_path: remote_path.clone(),
                final_path: final_path.to_string_lossy().to_string(),
                temp_path: temp_path.to_string_lossy().to_string(),
                expected_size: total, downloaded_size: downloaded,
                etag: save_etag.clone(), last_modified: save_lm.clone(), updated_at: now_secs(),
            }, &temp_path);
            update_manager_file_state(&shared, &task_id, FileStatePatch {
                downloaded: Some(downloaded),
                size: Some(total),
                version: Some(ctx.version),
                status: Some("error".into()),
                error: Some(Some(format!("File create/write failed: {}", e))),
                ..Default::default()
            });
            has_error.store(true, Ordering::SeqCst);
            let _ = app.emit("download-error", serde_json::json!({
                "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name, "error": format!("File create/write failed: {}", e),
                "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                "retryable": false,
            }));
            return;
        }
    };

    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if shared.cancel_flags.lock().unwrap().get(&run_id).copied().unwrap_or(false) {
            if !shared.pause_flags.lock().unwrap().get(&run_id).copied().unwrap_or(false) {
                save_artifact_state(&DownloadArtifactState {
                    task_id: task_id.clone(), run_id: run_id.clone(),
                    repo_id: repo_id.clone(), source: source.clone(),
                    remote_path: remote_path.clone(),
                    final_path: final_path.to_string_lossy().to_string(),
                    temp_path: temp_path.to_string_lossy().to_string(),
                    expected_size: total, downloaded_size: downloaded,
                    etag: save_etag.clone(), last_modified: save_lm.clone(), updated_at: now_secs(),
                }, &temp_path);
                update_manager_file_state(&shared, &task_id, FileStatePatch {
                    downloaded: Some(downloaded),
                    size: Some(total),
                    version: Some(ctx.version),
                    status: Some("cancelled".into()),
                    ..Default::default()
                });
                let _ = app.emit("download-cancelled", serde_json::json!({
                    "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
                    "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                }));
            } else {
                save_artifact_state(&DownloadArtifactState {
                    task_id: task_id.clone(), run_id: run_id.clone(),
                    repo_id: repo_id.clone(), source: source.clone(),
                    remote_path: remote_path.clone(),
                    final_path: final_path.to_string_lossy().to_string(),
                    temp_path: temp_path.to_string_lossy().to_string(),
                    expected_size: total, downloaded_size: downloaded,
                    etag: save_etag.clone(), last_modified: save_lm.clone(), updated_at: now_secs(),
                }, &temp_path);
                update_manager_file_state(&shared, &task_id, FileStatePatch {
                    downloaded: Some(downloaded),
                    size: Some(total),
                    version: Some(ctx.version),
                    status: Some("paused".into()),
                    ..Default::default()
                });
                let _ = app.emit("download-paused", serde_json::json!({
                    "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
                    "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                    "downloaded": downloaded, "total": total,
                }));
            }
            return;
        }
        match chunk {
            Ok(bytes) => {
                let len = bytes.len() as u64;
                throttle_download_bytes(&shared, len).await;
                if let Err(e) = file.write_all(&bytes) {
                        save_artifact_state(&DownloadArtifactState {
                            task_id: task_id.clone(), run_id: run_id.clone(),
                            repo_id: repo_id.clone(), source: source.clone(),
                            remote_path: remote_path.clone(),
                            final_path: final_path.to_string_lossy().to_string(),
                            temp_path: temp_path.to_string_lossy().to_string(),
                            expected_size: total, downloaded_size: downloaded,
                            etag: save_etag.clone(), last_modified: save_lm.clone(), updated_at: now_secs(),
                        }, &temp_path);
                        update_manager_file_state(&shared, &task_id, FileStatePatch {
                            downloaded: Some(downloaded),
                            size: Some(total),
                            version: Some(ctx.version),
                            status: Some("error".into()),
                            error: Some(Some(format!("File create/write failed: {}", e))),
                            ..Default::default()
                        });
                        has_error.store(true, Ordering::SeqCst);
                        let _ = app.emit("download-error", serde_json::json!({
                            "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name, "error": format!("File create/write failed: {}", e),
                "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                            "retryable": false,
                        }));
                    return;
                }
                downloaded += len;
                let now = std::time::Instant::now();
                win_bytes += len;
                let win_elapsed = now.duration_since(win_start).as_secs_f64();
                let speed = if win_elapsed >= 1.0 {
                    let s = win_bytes as f64 / win_elapsed;
                    win_start = now;
                    win_bytes = 0;
                    s
                } else if win_elapsed > 0.0 {
                    win_bytes as f64 / win_elapsed
                } else { 0.0 };
                if last_emit.elapsed().as_millis() >= 500 {
                    last_emit = now;
                    update_manager_file_state(&shared, &task_id, FileStatePatch {
                        run_id: Some(run_id.clone()),
                        downloaded: Some(downloaded),
                        size: Some(total),
                        version: Some(ctx.version),
                        status: Some("active".into()),
                        error: Some(None),
                        ..Default::default()
                    });
                    let _ = app.emit("download-progress", serde_json::json!({
                        "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name, "downloaded": downloaded,
                        "total": total, "speed": speed, "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                    }));
                    save_artifact_state(&DownloadArtifactState {
                        task_id: task_id.clone(), run_id: run_id.clone(),
                        repo_id: repo_id.clone(), source: source.clone(),
                        remote_path: remote_path.clone(),
                        final_path: final_path.to_string_lossy().to_string(),
                        temp_path: temp_path.to_string_lossy().to_string(),
                        expected_size: total, downloaded_size: downloaded,
                        etag: save_etag.clone(), last_modified: save_lm.clone(), updated_at: now_secs(),
                    }, &temp_path);
                }
            }
            Err(e) => {
                save_artifact_state(&DownloadArtifactState {
                    task_id: task_id.clone(), run_id: run_id.clone(),
                    repo_id: repo_id.clone(), source: source.clone(),
                    remote_path: remote_path.clone(),
                    final_path: final_path.to_string_lossy().to_string(),
                    temp_path: temp_path.to_string_lossy().to_string(),
                    expected_size: total, downloaded_size: downloaded,
                    etag: save_etag.clone(), last_modified: save_lm.clone(), updated_at: now_secs(),
                }, &temp_path);
                update_manager_file_state(&shared, &task_id, FileStatePatch {
                    downloaded: Some(downloaded),
                    size: Some(total),
                    version: Some(ctx.version),
                    status: Some("error".into()),
                    error: Some(Some(e.to_string())),
                    ..Default::default()
                });
                has_error.store(true, Ordering::SeqCst);
                let _ = app.emit("download-error", serde_json::json!({
                    "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name, "error": e.to_string(),
                    "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                    "retryable": true,
                }));
                return;
            }
        }
    }
    if final_path.exists() {
        let _ = std::fs::remove_file(&final_path);
    }
    if let Err(e) = std::fs::rename(&temp_path, &final_path) {
        save_artifact_state(&DownloadArtifactState {
            task_id: task_id.clone(), run_id: run_id.clone(),
            repo_id: repo_id.clone(), source: source.clone(),
            remote_path: remote_path.clone(),
            final_path: final_path.to_string_lossy().to_string(),
            temp_path: temp_path.to_string_lossy().to_string(),
            expected_size: total, downloaded_size: downloaded,
            etag: save_etag.clone(), last_modified: save_lm.clone(), updated_at: now_secs(),
        }, &temp_path);
        has_error.store(true, Ordering::SeqCst);
        update_manager_file_state(&shared, &task_id, FileStatePatch {
            downloaded: Some(downloaded),
            size: Some(total),
            version: Some(ctx.version),
            status: Some("error".into()),
            error: Some(Some(format!("Failed to finalize download: {}", e))),
            ..Default::default()
        });
        let _ = app.emit("download-error", serde_json::json!({
            "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
            "error": format!("Failed to finalize download: {}", e),
            "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
            "retryable": false,
        }));
        return;
    }
    cleanup_artifact_state(&temp_path);
    update_manager_file_state(&shared, &task_id, FileStatePatch {
        downloaded: Some(total),
        size: Some(total),
        version: Some(ctx.version),
        status: Some("completed".into()),
        error: Some(None),
        ..Default::default()
    });
    let _ = app.emit("download-complete", serde_json::json!({
        "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name, "path": final_path.to_string_lossy(),
        "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
    }));
}

// ── ModelScope 浏览 ──────────────────────────────────────────────

#[tauri::command]
pub async fn browse_modelscope(repo_id: String) -> Result<Vec<MsFileEntry>, String> {
    let url = format!("https://www.modelscope.cn/api/v1/models/{}/repo/files?Recursive=true", repo_id);
    let resp = HTTP_CLIENT.get(&url).send().await.map_err(|e| format!("网络错误: {}", e))?;
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("{}", e))?;

    if !body.get("Success").and_then(|v| v.as_bool()).unwrap_or(false) {
        let msg = body.get("Message").and_then(|v| v.as_str()).unwrap_or("未知错误");
        return Err(msg.to_string());
    }

    let empty_vec = vec![];
    let files = body["Data"]["Files"].as_array().unwrap_or(&empty_vec);
    let mut result: Vec<MsFileEntry> = files.iter().filter_map(|f| {
        if f.get("Type")?.as_str()? != "blob" { return None; }
        let name = f.get("Name")?.as_str()?.to_string();
        if !name.ends_with(".gguf") && !name.ends_with(".txt") { return None; }
        Some(MsFileEntry {
            file_type: utils::classify_gguf_file(Path::new(&name)).to_string(),
            name,
            path: f.get("Path")?.as_str()?.to_string(),
            size: f.get("Size")?.as_u64().unwrap_or(0),
            task_id: None,
            run_id: None,
            downloaded: None,
            version: None,
            status: None,
            error: None,
        })
    }).collect();

    result.sort_by_key(|e| {
        match e.file_type.as_str() {
            "mmproj" => 0, "model" => 1, "imatrix" => 2, _ => 9,
        }
    });
    Ok(result)
}

// ── ModelScope 并行下载 ─────────────────────────────────────────

#[tauri::command]
pub async fn download_modelscope_files(
    repo_id: String,
    files: Vec<MsFileEntry>,
    save_dir: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let repo_id = sanitize_repo_id(&repo_id)?;
    let save_path = resolve_repo_save_path(&app, &save_dir, &repo_id)?;

    let mut handles = Vec::new();
    use tokio::sync::Semaphore;
    let max_concurrent = *app.state::<AppState>().download_max_concurrent.lock().unwrap();
    let semaphore = Arc::new(Semaphore::new(max_concurrent.max(1)));
    let has_error = Arc::new(AtomicBool::new(false));

    // 清除本批次文件的 cancel/pause flags（避免上次暂停/取消的标记残留）
    clear_control_flags_for_files(&app.state::<AppState>(), &files);

    for file in files {
        let url = format!("https://modelscope.cn/models/{}/resolve/master/{}", repo_id, file.path);
        let dest_dir = save_path.clone();
        let ctx = build_task_context(&file, &repo_id, "modelscope");
        let has_error = Arc::clone(&has_error);
        let handle = tokio::spawn({
            let app = app.clone();
            let permit = semaphore.clone().acquire_owned();
            async move {
                let _permit = permit.await;
                download_single_file(ctx, url, dest_dir, file.size, app, has_error).await;
            }
        });
        handles.push(handle);
    }

    for h in handles { let _ = h.await; }
    if has_error.load(Ordering::SeqCst) {
        return Err("Download completed with errors".into());
    }
    Ok(())
}

// ── 下载控制 ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn cancel_file_download(task_id: String, run_id: Option<String>, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let key = run_id.unwrap_or(task_id);
    state.cancel_flags.lock().unwrap().insert(key, true);
    Ok(())
}

#[tauri::command]
pub async fn pause_file_download(task_id: String, run_id: Option<String>, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let key = run_id.unwrap_or(task_id);
    let mut pause = state.pause_flags.lock().unwrap();
    let mut cancel = state.cancel_flags.lock().unwrap();
    pause.insert(key.clone(), true);
    cancel.insert(key, true);
    Ok(())
}

#[tauri::command]
pub async fn cancel_and_cleanup_download(task_id: String, file_name: String, file_path: String, run_id: Option<String>, version: Option<u32>, state: tauri::State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    let _ = sanitize_file_name(&file_name)?;
    // Canonicalize + verify the path is within a managed download directory
    let managed = state.config_dir.lock().unwrap();
    let root = managed.parent().unwrap_or(Path::new(".")).to_path_buf();
    let cpath = std::fs::canonicalize(Path::new(&file_path)).unwrap_or_else(|_| Path::new(&file_path).to_path_buf());
    let croot = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
    if !cpath.starts_with(&croot) {
        return Err("文件不在受管目录内".into());
    }
    let key = run_id.unwrap_or_else(|| task_id.clone());
    state.cancel_flags.lock().unwrap().insert(key.clone(), true);
    state.pause_flags.lock().unwrap().remove(&key);
    let _ = std::fs::remove_file(&cpath);
    let part_path = PathBuf::from(format!("{}.part", cpath.display()));
    let part_json_path = PathBuf::from(format!("{}.part.json", cpath.display()));
    let _ = std::fs::remove_file(&part_path);
    let _ = std::fs::remove_file(&part_json_path);
    remove_manager_file(&state, &task_id);
    let _ = app.emit("download-removed", serde_json::json!({ "taskId": task_id, "fileName": file_name, "version": version.unwrap_or(0) }));
    Ok(())
}

// ── HuggingFace 数据结构和浏览 ──────────────────────────────────

#[derive(serde::Deserialize)]
struct HfFileEntry {
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
    size: Option<u64>,
    #[serde(default)]
    lfs: Option<HfLfsInfo>,
}

#[derive(serde::Deserialize)]
struct HfLfsInfo {
    size: u64,
}

#[tauri::command]
pub async fn browse_huggingface(repo_id: String) -> Result<Vec<MsFileEntry>, String> {
    let url = format!("https://huggingface.co/api/models/{}/tree/main?recursive=true", repo_id);
    let resp = HTTP_CLIENT.get(&url).send().await.map_err(|e| format!("网络错误: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("仓库未找到 (HTTP {})", resp.status()));
    }
    let entries: Vec<HfFileEntry> = resp.json().await.map_err(|e| format!("解析失败: {}", e))?;

    let mut result: Vec<MsFileEntry> = entries.iter().filter_map(|e| {
        if e.entry_type != "file" { return None; }
        let name = e.path.split('/').last()?.to_string();
        if !name.ends_with(".gguf") && !name.ends_with(".txt") { return None; }
        let size = e.lfs.as_ref().map(|l| l.size).or(e.size).unwrap_or(0);
        Some(MsFileEntry {
            file_type: utils::classify_gguf_file(Path::new(&name)).to_string(),
            name,
            path: e.path.clone(),
            size,
            task_id: None,
            run_id: None,
            downloaded: None,
            version: None,
            status: None,
            error: None,
        })
    }).collect();

    result.sort_by_key(|e| {
        match e.file_type.as_str() {
            "mmproj" => 0, "model" => 1, "imatrix" => 2, _ => 9,
        }
    });
    Ok(result)
}

// ── HuggingFace 下载 ────────────────────────────────────────────

#[tauri::command]
pub async fn download_huggingface_files(
    repo_id: String,
    files: Vec<MsFileEntry>,
    save_dir: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    use tokio::sync::Semaphore;

    let repo_id = sanitize_repo_id(&repo_id)?;
    let save_path = resolve_repo_save_path(&app, &save_dir, &repo_id)?;

    let max_concurrent = *app.state::<AppState>().download_max_concurrent.lock().unwrap();
    let semaphore = Arc::new(Semaphore::new(max_concurrent.max(1)));
    let has_error = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::new();

    // 清除本批次文件的 cancel/pause flags
    clear_control_flags_for_files(&app.state::<AppState>(), &files);

    for file in files {
        let url = format!("https://huggingface.co/{}/resolve/main/{}", repo_id, file.path);
        let dest_dir = save_path.clone();
        let ctx = build_task_context(&file, &repo_id, "huggingface");
        let has_error = Arc::clone(&has_error);
        let handle = tokio::spawn({
            let app = app.clone();
            let permit = semaphore.clone().acquire_owned();
            async move {
                let _permit = permit.await;
                download_single_file(ctx, url, dest_dir, file.size, app, has_error).await;
            }
        });
        handles.push(handle);
    }

    for h in handles { let _ = h.await; }
    if has_error.load(Ordering::SeqCst) {
        return Err("Download completed with errors".into());
    }
    Ok(())
}

/// Check if local file exists and return its size.
#[tauri::command]
pub async fn check_local_file(path: String) -> Result<Option<u64>, String> {
    let p = std::path::Path::new(&path);
    match std::fs::metadata(p) {
        Ok(m) if m.is_file() => Ok(Some(m.len())),
        Ok(_) => Ok(None),
        Err(_) => Ok(None),
    }
}

// ── 下载队列持久化 ─────────────────────────────────────────────

use crate::models::DownloadState;

fn persist_manager_queue(state: &AppState) {
    let queue = state.download_queue.lock().unwrap().clone();
    let mut persisted = load_download_state(state);
    persisted.retain(|entry| !is_runtime_queued(entry));
    persisted.retain(|entry| !queue.iter().any(|queued| queued.id == entry.id));
    persisted.extend(queue);
    save_download_state(&persisted, state);
}

fn collect_manager_entries(state: &AppState) -> Vec<PersistedQueueEntry> {
    let mut entries = state.download_queue.lock().unwrap().clone();
    let active_entries = state.download_active_entries.lock().unwrap();

    for entry in active_entries.values() {
        let mut active_entry = entry.clone();
        active_entry.status = "active".into();
        if let Some(existing) = entries.iter_mut().find(|queued| queued.id == active_entry.id) {
            *existing = active_entry;
        } else {
            entries.push(active_entry);
        }
    }

    entries
}

fn derive_entry_status(entry: &PersistedQueueEntry) -> String {
    if entry.files.iter().any(|file| matches!(file.status.as_deref(), Some("active") | Some("pausing"))) {
        return "active".into();
    }
    if entry.files.iter().any(|file| matches!(file.status.as_deref(), Some("error"))) {
        return "error".into();
    }
    if entry.files.iter().any(|file| matches!(file.status.as_deref(), Some("paused"))) {
        return "paused".into();
    }
    if !entry.files.is_empty() && entry.files.iter().all(|file| matches!(file.status.as_deref(), Some("completed"))) {
        return "completed".into();
    }
    entry.status.clone()
}

fn persist_active_entries_snapshot(state: &AppState) {
    let inflight: Vec<PersistedQueueEntry> = {
        let entries = state.download_active_entries.lock().unwrap();
        entries.values().cloned().collect()
    };
    save_inflight_state(&inflight, state);
}

#[derive(Default)]
struct FileStatePatch {
    run_id: Option<String>,
    downloaded: Option<u64>,
    size: Option<u64>,
    version: Option<u32>,
    status: Option<String>,
    error: Option<Option<String>>,
}

fn apply_file_patch(file: &mut MsFileEntry, patch: &FileStatePatch) {
    if let Some(run_id) = &patch.run_id {
        file.run_id = Some(run_id.clone());
    }
    if let Some(downloaded) = patch.downloaded {
        file.downloaded = Some(downloaded);
    }
    if let Some(size) = patch.size {
        file.size = size;
    }
    if let Some(version) = patch.version {
        file.version = Some(version);
    }
    if let Some(status) = &patch.status {
        file.status = Some(status.clone());
    }
    if let Some(error) = &patch.error {
        file.error = error.clone();
    }
}

fn update_manager_file_state(state: &AppState, task_id: &str, patch: FileStatePatch) -> bool {
    {
        let mut active_entries = state.download_active_entries.lock().unwrap();
        let mut changed = false;
        for entry in active_entries.values_mut() {
            for file in entry.files.iter_mut() {
                if file.task_id.as_deref() == Some(task_id) {
                    apply_file_patch(file, &patch);
                    changed = true;
                }
            }
            if changed {
                entry.status = derive_entry_status(entry);
            }
        }
        if changed {
            drop(active_entries);
            persist_active_entries_snapshot(state);
            return true;
        }
    }

    let mut queue = state.download_queue.lock().unwrap();
    let mut changed = false;
    for entry in queue.iter_mut() {
        for file in entry.files.iter_mut() {
            if file.task_id.as_deref() == Some(task_id) {
                apply_file_patch(file, &patch);
                changed = true;
            }
        }
        if changed {
            entry.status = derive_entry_status(entry);
        }
    }
    if changed {
        drop(queue);
        persist_manager_queue(state);
    }
    changed
}

fn remove_manager_file(state: &AppState, task_id: &str) -> bool {
    {
        let mut active_entries = state.download_active_entries.lock().unwrap();
        let mut changed = false;
        active_entries.retain(|_, entry| {
            let old_len = entry.files.len();
            entry.files.retain(|file| file.task_id.as_deref() != Some(task_id));
            if entry.files.len() != old_len {
                changed = true;
            }
            entry.status = derive_entry_status(entry);
            !entry.files.is_empty()
        });
        if changed {
            drop(active_entries);
            persist_active_entries_snapshot(state);
            return true;
        }
    }

    let mut queue = state.download_queue.lock().unwrap();
    let mut changed = false;
    queue.retain_mut(|entry| {
        let old_file_len = entry.files.len();
        entry.files.retain(|file| file.task_id.as_deref() != Some(task_id));
        if entry.files.len() != old_file_len {
            changed = true;
        }
        entry.status = derive_entry_status(entry);
        !entry.files.is_empty()
    });
    if changed {
        drop(queue);
        persist_manager_queue(state);
    }
    changed
}

fn persist_terminal_entry(state: &AppState, mut entry: PersistedQueueEntry) {
    entry.files.retain(|file| {
        !matches!(file.status.as_deref(), Some("completed") | Some("cancelled"))
    });

    let runtime_queue = state.download_queue.lock().unwrap().clone();
    let mut persisted = load_download_state(state);
    persisted.retain(|saved| saved.id != entry.id && !runtime_queue.iter().any(|queued| queued.id == saved.id));
    persisted.extend(runtime_queue);

    if !entry.files.is_empty() {
        entry.status = derive_entry_status(&entry);
        persisted.push(entry);
    }

    save_download_state(&persisted, state);
}

fn is_runtime_queued(entry: &PersistedQueueEntry) -> bool {
    entry.status.is_empty() || entry.status == "queued"
}

fn is_restore_runnable(entry: &PersistedQueueEntry) -> bool {
    (entry.status.is_empty() || entry.status == "queued" || entry.status == "active")
        && entry.retries < entry.max_retries
}

async fn run_persisted_entry(entry: PersistedQueueEntry, app: tauri::AppHandle) -> Result<(), String> {
    if entry.source == "modelscope" {
        download_modelscope_files(entry.repo_id, entry.files, entry.save_dir, app).await
    } else {
        download_huggingface_files(entry.repo_id, entry.files, entry.save_dir, app).await
    }
}

fn process_download_queue_inner(app: tauri::AppHandle) {
    let state = app.state::<AppState>();
    {
        let active = state.download_active_batches.lock().unwrap();
        let max = effective_download_concurrency(&state);
        if active.len() >= max {
            return;
        }
    }

    let entry = {
        let mut queue = state.download_queue.lock().unwrap();
        // 一次性清除不可运行的条目（它们无法被处理，留队列中会阻塞后续任务）
        let old_len = queue.len();
        queue.retain(|e| is_restore_runnable(e));
        if queue.is_empty() {
            if queue.len() != old_len {
                drop(queue);
                persist_manager_queue(&state);
            }
            return;
        }
        let mut entry = queue.remove(0);
        for file in entry.files.iter_mut() {
            file.status = Some("active".into());
            file.error = None;
        }
        entry.status = "active".into();
        // B-01: save to inflight before dequeue so crash recovery can find it
        {
            let mut inflight = load_inflight_state(&state);
            inflight.retain(|e| e.id != entry.id);
            let mut inflight_entry = entry.clone();
            inflight_entry.status = "active".into();
            inflight.push(inflight_entry);
            save_inflight_state(&inflight, &state);
        }
        // 只在出队后保存一次，避免循环中反复写磁盘
        drop(queue);
        persist_manager_queue(&state);
        entry
    };

    {
        let mut active_entries = state.download_active_entries.lock().unwrap();
        active_entries.insert(entry.id.clone(), entry.clone());
    }

    {
        let mut active = state.download_active_batches.lock().unwrap();
        active.insert(entry.id.clone());
    }

    for file in &entry.files {
        let _ = app.emit("download-started", serde_json::json!({
            "queueId": &entry.id,
            "taskId": file.task_id.as_deref().unwrap_or(""),
            "runId": file.run_id.as_deref().unwrap_or(""),
            "version": file.version.unwrap_or(0),
            "fileName": &file.name,
            "repoId": &entry.repo_id,
            "source": &entry.source,
            "remotePath": &file.path,
            "downloaded": file.downloaded.unwrap_or(0),
            "total": file.size,
        }));
    }

    tauri::async_runtime::spawn(async move {
        let batch_id = entry.id.clone();
        let entry_for_retry = entry.clone();
        let result = run_persisted_entry(entry, app.clone()).await;
        let finalized_entry = {
            let state = app.state::<AppState>();
            let entries = state.download_active_entries.lock().unwrap();
            entries.get(&batch_id).cloned()
        };

        if let Err(_e) = result {
            if entry_for_retry.retries < entry_for_retry.max_retries {
                let delay_ms = 2000u64 * 2u64.pow(entry_for_retry.retries.min(5));
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

                let mut retry_entry = finalized_entry.unwrap_or(entry_for_retry);
                retry_entry.retries += 1;
                retry_entry.status = "queued".into();
                for file in retry_entry.files.iter_mut() {
                    if matches!(file.status.as_deref(), Some("error") | Some("paused") | Some("cancelled")) {
                        file.status = Some("queued".into());
                    }
                    file.error = None;
                }

                {
                    let state = app.state::<AppState>();
                    state.download_active_batches.lock().unwrap().remove(&batch_id);
                    state.download_active_entries.lock().unwrap().remove(&batch_id);
                    let mut queue = state.download_queue.lock().unwrap();
                    queue.insert(0, retry_entry);
                    drop(queue);
                    persist_manager_queue(&state);
                }

                process_download_queue_inner(app);
                return;
            }
        }

        {
            let state = app.state::<AppState>();
            if let Some(entry) = finalized_entry {
                persist_terminal_entry(&state, entry);
            }
            state.download_active_batches.lock().unwrap().remove(&batch_id);
            state.download_active_entries.lock().unwrap().remove(&batch_id);
            // B-01: remove from inflight on final completion/exhaustion
            let mut inflight = load_inflight_state(&state);
            inflight.retain(|e| e.id != batch_id);
            save_inflight_state(&inflight, &state);
        }
        process_download_queue_inner(app);
    });
}

pub(crate) fn save_download_state(queue: &[PersistedQueueEntry], state: &AppState) {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let _ = std::fs::create_dir_all(&config_dir);
    let path = config_dir.join("downloads.json");
    let ds = DownloadState { queue: queue.to_vec() };
    if let Ok(json) = serde_json::to_string_pretty(&ds) {
        let tmp = config_dir.join("downloads.json.tmp");
        if std::fs::write(&tmp, &json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

pub(crate) fn load_download_state(state: &AppState) -> Vec<PersistedQueueEntry> {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let path = config_dir.join("downloads.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<DownloadState>(&s).ok())
        .map(|ds| ds.queue)
        .unwrap_or_default()
}

fn inflight_path(state: &AppState) -> PathBuf {
    state.config_dir.lock().unwrap().join("downloads_inflight.json")
}

fn save_inflight_state(inflight: &[PersistedQueueEntry], state: &AppState) {
    let path = inflight_path(state);
    if inflight.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let ds = DownloadState { queue: inflight.to_vec() };
    if let Ok(json) = serde_json::to_string_pretty(&ds) {
        let _ = std::fs::write(&path, json);
    }
}

fn load_inflight_state(state: &AppState) -> Vec<PersistedQueueEntry> {
    let path = inflight_path(state);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<DownloadState>(&s).ok())
        .map(|ds| ds.queue)
        .unwrap_or_default()
}

fn clear_inflight_state(state: &AppState) {
    let _ = std::fs::remove_file(inflight_path(state));
}

pub(crate) fn restore_runtime_queue_from_disk(state: &AppState, app: &tauri::AppHandle) -> Vec<PersistedQueueEntry> {
    let mut queue = load_download_state(state);

    let inflight = load_inflight_state(state);
    if !inflight.is_empty() {
        for mut entry in inflight {
            if entry.status == "active" {
                entry.status = "paused".into();
            }
            queue.push(entry);
        }
        clear_inflight_state(state);
    }

    let config_dir = state.config_dir.lock().unwrap().clone();
    let config = crate::commands::config::read_config_from_disk(&config_dir);
    let save_dir_base = config_dir.parent().unwrap_or(Path::new(".")).to_path_buf();

    for entry in queue.iter_mut() {
        let repo_dir = save_dir_base
            .join(&entry.save_dir)
            .join(entry.repo_id.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
        for file in entry.files.iter_mut() {
            if file.status.is_none() {
                file.status = Some(entry.status.clone());
            }
            let (final_path, temp_path, _) = build_download_paths(&repo_dir, &file.name);
            if temp_path.exists() {
                let artifact = load_artifact_state(&temp_path);
                if let Some(ref a) = artifact {
                    file.downloaded = Some(a.downloaded_size);
                    if file.size == 0 {
                        file.size = a.expected_size;
                    }
                } else if let Ok(meta) = std::fs::metadata(&temp_path) {
                    file.downloaded = Some(meta.len());
                }
            } else if final_path.exists() {
                if let Ok(meta) = std::fs::metadata(&final_path) {
                    file.downloaded = Some(meta.len());
                    file.size = meta.len();
                }
            }
        }
    }

    let runnable: Vec<_> = queue.iter().filter(|e| is_restore_runnable(e)).cloned().collect();
    *state.download_queue.lock().unwrap() = runnable;
    save_download_state(&queue, state);

    let policy = config.download_resume_policy;

    if policy == "auto_on_launch" {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            process_download_queue_inner(app);
        });
    }

    queue
}

#[tauri::command]
pub async fn persist_download_queue(
    queue: Vec<PersistedQueueEntry>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let runtime_queue: Vec<PersistedQueueEntry> = queue.iter().filter(|e| is_runtime_queued(e)).cloned().collect();
    *state.download_queue.lock().unwrap() = runtime_queue.clone();

    let mut persisted = load_download_state(&state);
    persisted.retain(|entry| !is_runtime_queued(entry));

    for entry in collect_manager_entries(&state).into_iter().filter(|entry| !is_runtime_queued(entry)) {
        if let Some(existing) = persisted.iter_mut().find(|saved| saved.id == entry.id) {
            *existing = entry;
        } else {
            persisted.push(entry);
        }
    }

    persisted.retain(|entry| !runtime_queue.iter().any(|queued| queued.id == entry.id));
    persisted.extend(runtime_queue);
    save_download_state(&persisted, &state);
    Ok(())
}

#[tauri::command]
pub async fn restore_download_queue(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Vec<PersistedQueueEntry>, String> {
    Ok(restore_runtime_queue_from_disk(&state, &app))
}

#[tauri::command]
pub async fn enqueue_download_queue(
    mut entry: PersistedQueueEntry,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    entry.status = "queued".into();
    for file in entry.files.iter_mut() {
        if file.version.is_none() {
            file.version = Some(0);
        }
        file.status = Some("queued".into());
        file.error = None;
    }
    {
        let mut queue = state.download_queue.lock().unwrap();
        if is_runtime_queued(&entry) && !queue.iter().any(|e| e.id == entry.id) {
            queue.push(entry);
        }
    }
    persist_manager_queue(&state);
    process_download_queue_inner(app);
    Ok(())
}

#[tauri::command]
pub async fn remove_download_queue_entry(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    {
        let mut queue = state.download_queue.lock().unwrap();
        queue.retain(|entry| entry.id != id);
    }
    persist_manager_queue(&state);
    Ok(())
}

#[tauri::command]
pub async fn clear_download_tasks_by_status(
    statuses: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let status_set: std::collections::HashSet<String> = statuses.into_iter().collect();
    let mut persisted = load_download_state(&state);
    persisted.retain_mut(|entry| {
        entry.files.retain(|file| !file.status.as_ref().map(|s| status_set.contains(s)).unwrap_or(false));
        entry.status = derive_entry_status(entry);
        !entry.files.is_empty()
    });

    {
        let mut queue = state.download_queue.lock().unwrap();
        queue.retain_mut(|entry| {
            entry.files.retain(|file| !file.status.as_ref().map(|s| status_set.contains(s)).unwrap_or(false));
            entry.status = derive_entry_status(entry);
            !entry.files.is_empty()
        });
    }

    let runtime_queue = state.download_queue.lock().unwrap().clone();
    persisted.retain(|entry| !runtime_queue.iter().any(|queued| queued.id == entry.id));
    persisted.extend(runtime_queue);
    save_download_state(&persisted, &state);
    Ok(())
}

#[tauri::command]
pub async fn process_download_queue(app: tauri::AppHandle) -> Result<(), String> {
    process_download_queue_inner(app);
    Ok(())
}

#[tauri::command]
pub async fn get_download_resume_policy(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let config = crate::commands::config::read_config_from_disk(&config_dir);
    Ok(config.download_resume_policy)
}

#[tauri::command]
pub async fn set_download_resume_policy(
    policy: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if policy != "manual" && policy != "auto_on_launch" {
        return Err("Invalid policy. Must be 'manual' or 'auto_on_launch'".into());
    }
    let config_dir = state.config_dir.lock().unwrap().clone();
    let mut config = crate::commands::config::read_config_from_disk(&config_dir);
    config.download_resume_policy = policy;
    crate::commands::config::persist_global_config(&config_dir, &config)?;
    Ok(())
}

#[tauri::command]
pub async fn resume_download_task(
    task_id: String,
    app: tauri::AppHandle,
) -> Result<ResumeDownloadTaskResult, String> {
    let state = app.state::<AppState>();

    if state.download_active_entries.lock().unwrap().values().any(|entry| {
        entry.files.iter().any(|file| file.task_id.as_deref() == Some(task_id.as_str()))
    }) {
        return Err("Download task is already active".into());
    }

    if let Some(existing) = state.download_queue.lock().unwrap().iter().find_map(|entry| {
        entry.files.iter().find(|file| file.task_id.as_deref() == Some(task_id.as_str()))
    }) {
        return Ok(ResumeDownloadTaskResult {
            task_id: task_id.clone(),
            run_id: existing.run_id.clone().unwrap_or_default(),
            version: existing.version.unwrap_or(0),
        });
    }

    let mut persisted = load_download_state(&state);
    let mut target_file: Option<MsFileEntry> = None;
    let mut target_meta: Option<(String, String, String, u32, u32)> = None;

    persisted.retain_mut(|entry| {
        if let Some(pos) = entry.files.iter().position(|file| file.task_id.as_deref() == Some(task_id.as_str())) {
            if target_file.is_none() {
                target_file = Some(entry.files.remove(pos));
                target_meta = Some((
                    entry.repo_id.clone(),
                    entry.source.clone(),
                    entry.save_dir.clone(),
                    entry.retries,
                    entry.max_retries,
                ));
            }
            entry.status = derive_entry_status(entry);
        }
        !entry.files.is_empty()
    });

    let mut file = target_file.ok_or_else(|| "Download task not found".to_string())?;
    clear_control_flags_for_files(&state, &[file.clone()]);

    let (repo_id, source, save_dir, retries, max_retries) = target_meta
        .ok_or_else(|| "Download task metadata is missing".to_string())?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let version = file.version.unwrap_or(0) + 1;
    file.run_id = Some(run_id.clone());
    file.version = Some(version);
    file.status = Some("queued".into());
    file.error = None;

    let runtime_entry = PersistedQueueEntry {
        id: uuid::Uuid::new_v4().to_string(),
        repo_id,
        source,
        files: vec![file],
        save_dir,
        added_at: now_secs(),
        status: "queued".into(),
        retries,
        max_retries,
        last_error: None,
    };

    let runtime_queue = {
        let mut runtime = state.download_queue.lock().unwrap();
        runtime.push(runtime_entry);
        runtime.clone()
    };

    persisted.retain(|entry| !runtime_queue.iter().any(|queued| queued.id == entry.id));
    persisted.extend(runtime_queue);
    save_download_state(&persisted, &state);

    process_download_queue_inner(app);

    Ok(ResumeDownloadTaskResult {
        task_id,
        run_id,
        version,
    })
}

#[tauri::command]
pub async fn resume_all_downloads(app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let queue = load_download_state(&state);
    let mut runtime = state.download_queue.lock().unwrap();
    for entry in &queue {
        if entry.status == "paused" && !runtime.iter().any(|e| e.id == entry.id) {
            let mut entry = entry.clone();
            entry.status = "queued".into();
            for file in entry.files.iter_mut() {
                file.status = Some("queued".into());
                file.error = None;
            }
            runtime.push(entry);
        }
    }
    drop(runtime);
    persist_manager_queue(&state);
    process_download_queue_inner(app);
    Ok(())
}

pub fn flush_download_manager_state(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();

    // Snapshot active batch entries before they're removed (need them for disk persistence)
    let active_entries: Vec<PersistedQueueEntry> = {
        let entries = state.download_active_entries.lock().unwrap();
        entries.values().cloned().collect()
    };

    {
        let mut cancel = state.cancel_flags.lock().unwrap();
        let mut pause = state.pause_flags.lock().unwrap();
        let active = state.active_downloads.lock().unwrap();
        for run_id in active.iter() {
            cancel.insert(run_id.clone(), true);
            pause.insert(run_id.clone(), true);
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(500));

    // Write paused entries to disk so they survive restart
    let mut queue = state.download_queue.lock().unwrap().clone();
    for mut entry in active_entries {
        entry.status = "paused".to_string();
        for file in entry.files.iter_mut() {
            file.status = Some("paused".into());
        }
        if !queue.iter().any(|e| e.id == entry.id) {
            queue.push(entry);
        }
    }
    let mut persisted = load_download_state(&state);
    persisted.retain(|entry| !queue.iter().any(|queued| queued.id == entry.id));
    persisted.extend(queue);
    save_download_state(&persisted, &state);
}

// ── 批量控制命令 ──────────────────────────────────────────────────

#[tauri::command]
pub async fn pause_all_downloads(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut cancel = state.cancel_flags.lock().unwrap();
    let mut pause = state.pause_flags.lock().unwrap();
    let active = state.active_downloads.lock().unwrap();
    for run_id in active.iter() {
        cancel.insert(run_id.clone(), true);
        pause.insert(run_id.clone(), true);
    }
    Ok(())
}

#[tauri::command]
pub async fn cancel_all_downloads(state: tauri::State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    let active_entries: Vec<PersistedQueueEntry> = {
        let entries = state.download_active_entries.lock().unwrap();
        entries.values().cloned().collect()
    };
    let mut cancel = state.cancel_flags.lock().unwrap();
    let active = state.active_downloads.lock().unwrap();
    for run_id in active.iter() {
        cancel.insert(run_id.clone(), true);
    }
    {
        let mut queue = state.download_queue.lock().unwrap();
        queue.clear();
        drop(queue);
        persist_manager_queue(&state);
    }
    for entry in active_entries {
        for file in entry.files {
            let _ = app.emit("download-cancelled", serde_json::json!({
                "taskId": file.task_id.as_deref().unwrap_or(""),
                "runId": file.run_id.as_deref().unwrap_or(""),
                "version": file.version.unwrap_or(0),
                "fileName": &file.name,
                "repoId": &entry.repo_id,
                "source": &entry.source,
                "remotePath": &file.path,
            }));
        }
    }
    Ok(())
}

// ── 并发控制命令 ──────────────────────────────────────────────────

#[tauri::command]
pub async fn set_download_concurrency(n: usize, state: tauri::State<'_, AppState>) -> Result<(), String> {
    if n < 1 || n > 8 { return Err("concurrency must be 1-8".into()); }
    *state.download_max_concurrent.lock().unwrap() = n;
    crate::commands::config::update_and_persist(&state, |global| {
        global.download_max_concurrent = n;
    })?;
    Ok(())
}

#[tauri::command]
pub async fn get_download_concurrency(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    Ok(*state.download_max_concurrent.lock().unwrap())
}

// ── 重置下载状态（重新下载专用） ────────────────────────────────────

#[tauri::command]
pub async fn set_download_bandwidth_limit(bytes_per_sec: u64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    const MAX_LIMIT_BYTES_PER_SEC: u64 = 10 * 1024 * 1024 * 1024;
    if bytes_per_sec > MAX_LIMIT_BYTES_PER_SEC {
        return Err("bandwidth limit must be 0-10 GiB/s".into());
    }
    *state.download_bandwidth_limit_bytes_per_sec.lock().unwrap() = bytes_per_sec;
    {
        let mut limiter = state.download_bandwidth_limiter.lock().unwrap();
        limiter.available_bytes = 0.0;
        limiter.last_refill = std::time::Instant::now();
    }
    crate::commands::config::update_and_persist(&state, |global| {
        global.download_bandwidth_limit_bytes_per_sec = bytes_per_sec;
    })?;
    Ok(())
}

#[tauri::command]
pub async fn get_download_bandwidth_limit(state: tauri::State<'_, AppState>) -> Result<u64, String> {
    Ok(*state.download_bandwidth_limit_bytes_per_sec.lock().unwrap())
}

#[tauri::command]
pub async fn set_download_low_priority_throttle(enabled: bool, app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    *state.download_low_priority_throttle.lock().unwrap() = enabled;
    {
        let mut limiter = state.download_bandwidth_limiter.lock().unwrap();
        limiter.available_bytes = 0.0;
        limiter.last_refill = std::time::Instant::now();
    }
    crate::commands::config::update_and_persist(&state, |global| {
        global.download_low_priority_throttle = enabled;
    })?;
    drop(state);
    if !enabled {
        process_download_queue_inner(app);
    }
    Ok(())
}

#[tauri::command]
pub async fn get_download_low_priority_throttle(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(*state.download_low_priority_throttle.lock().unwrap())
}

#[tauri::command]
pub async fn reset_download_for_redownload(
    _task_id: String,
    file_name: String,
    save_dir: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _ = sanitize_file_name(&file_name)?;
    let save_path = if Path::new(&save_dir).is_absolute() {
        PathBuf::from(&save_dir)
    } else {
        let managed = state.config_dir.lock().unwrap();
        managed.parent().unwrap_or(Path::new(".")).to_path_buf().join(&save_dir)
    };
    let (_final_path, temp_path, _meta_path) = build_download_paths(&save_path, &file_name);
    cleanup_artifact_state(&temp_path);
    Ok(())
}

// ── 下载管理器快照 ────────────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct DownloadManagerSnapshot {
    pub queue: Vec<PersistedQueueEntry>,
    pub active_count: usize,
    pub max_concurrent: usize,
    pub resume_policy: String,
    pub bandwidth_limit_bytes_per_sec: u64,
    pub low_priority_throttle: bool,
}

#[tauri::command]
pub async fn get_download_manager_snapshot(state: tauri::State<'_, AppState>) -> Result<DownloadManagerSnapshot, String> {
    let queue = collect_manager_entries(&state);
    let active_count = state.download_active_batches.lock().unwrap().len();
    let max_concurrent = *state.download_max_concurrent.lock().unwrap();
    let bandwidth_limit_bytes_per_sec = *state.download_bandwidth_limit_bytes_per_sec.lock().unwrap();
    let low_priority_throttle = *state.download_low_priority_throttle.lock().unwrap();
    let config_dir = state.config_dir.lock().unwrap().clone();
    let config = crate::commands::config::read_config_from_disk(&config_dir);
    let resume_policy = config.download_resume_policy;
    Ok(DownloadManagerSnapshot {
        queue,
        active_count,
        max_concurrent,
        resume_policy,
        bandwidth_limit_bytes_per_sec,
        low_priority_throttle,
    })
}
