use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use crate::models::{AppState, DownloadArtifactState, MsFileEntry};
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

fn is_retryable_error(status_code: Option<u16>) -> bool {
    match status_code {
        Some(429) => true,
        Some(code) if code >= 500 => true,
        _ => false,
    }
}

/// 下载单个文件的通用逻辑。
async fn download_single_file(
    task_id: String,
    run_id: String,
    url: String,
    save_path: PathBuf,
    raw_file_name: String,
    file_size: u64,
    repo_id: String,
    source: String,
    remote_path: String,
    app: tauri::AppHandle,
    has_error: Arc<AtomicBool>,
) {
    let file_name = match sanitize_file_name(&raw_file_name) {
        Ok(n) => n,
        Err(e) => {
            has_error.store(true, Ordering::SeqCst);
            let _ = app.emit("download-error", serde_json::json!({
                "taskId": &task_id, "runId": &run_id, "fileName": raw_file_name, "error": e,
                "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                "retryable": false,
            }));
            return;
        }
    };
    let (final_path, temp_path, _metadata_path) = build_download_paths(&save_path, &file_name);
    let shared = app.state::<AppState>();

    {
        let mut active = shared.active_downloads.lock().unwrap();
        if !active.insert(run_id.clone()) {
            // 该文件已有活跃下载任务，跳过（避免两个任务同时写同一个文件）
            has_error.store(true, Ordering::SeqCst);
            let _ = app.emit("download-error", serde_json::json!({
                "taskId": &task_id, "runId": &run_id, "fileName": &file_name,
                "error": "Download task is already active",
                "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                "retryable": false,
            }));
            return;
        }
    }
    // RAII guard: 函数退出时自动从 active_downloads 中移除
    let _guard = ActiveDownloadGuard { app: app.clone(), run_id: run_id.clone() };

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
                let _ = app.emit("download-cancelled", serde_json::json!({
                    "taskId": &task_id, "runId": &run_id, "fileName": &file_name,
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
            let _ = app.emit("download-error", serde_json::json!({
                "taskId": &task_id, "runId": &run_id, "fileName": &file_name, "error": e.to_string(),
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
                let _ = app.emit("download-complete", serde_json::json!({
                    "taskId": &task_id, "runId": &run_id, "fileName": &file_name,
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
            let _ = app.emit("download-error", serde_json::json!({
                "taskId": &task_id, "runId": &run_id, "fileName": &file_name,
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
        let _ = app.emit("download-error", serde_json::json!({
            "taskId": &task_id, "runId": &run_id, "fileName": &file_name,
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
        let _ = app.emit("download-error", serde_json::json!({
            "taskId": &task_id, "runId": &run_id, "fileName": &file_name, "error": msg,
            "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
            "retryable": retryable,
        }));
        return;
    }

    let is_partial = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let mut resume_from = if is_partial { resume_from } else { 0 };

    // A1-05: 200 OK with .part file means server ignored Range header → restart
    if !is_partial && temp_path.exists() {
        let _ = app.emit("download-restarted", serde_json::json!({
            "taskId": &task_id, "runId": &run_id, "fileName": &file_name,
            "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
        }));
        resume_from = 0;
    }

    // A1-06: Check if remote file changed (ETag/Last-Modified mismatch on resume)
    if let Some(ref old_state) = artifact {
        let etag_changed = resp_etag.is_some() && old_state.etag.is_some() && resp_etag != old_state.etag;
        let lm_changed = resp_last_modified.is_some() && old_state.last_modified.is_some() && resp_last_modified != old_state.last_modified;
        if (etag_changed || lm_changed) && temp_path.exists() {
            let _ = app.emit("download-remote-changed", serde_json::json!({
                "taskId": &task_id, "runId": &run_id, "fileName": &file_name,
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
            has_error.store(true, Ordering::SeqCst);
            let _ = app.emit("download-error", serde_json::json!({
                "taskId": &task_id, "runId": &run_id, "fileName": &file_name, "error": format!("File create/write failed: {}", e),
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
                let _ = app.emit("download-cancelled", serde_json::json!({
                    "taskId": &task_id, "runId": &run_id, "fileName": &file_name,
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
                let _ = app.emit("download-paused", serde_json::json!({
                    "taskId": &task_id, "runId": &run_id, "fileName": &file_name,
                    "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                    "downloaded": downloaded, "total": total,
                }));
            }
            return;
        }
        match chunk {
            Ok(bytes) => {
                let len = bytes.len() as u64;
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
                        has_error.store(true, Ordering::SeqCst);
                        let _ = app.emit("download-error", serde_json::json!({
                            "taskId": &task_id, "runId": &run_id, "fileName": &file_name, "error": format!("File create/write failed: {}", e),
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
                    let _ = app.emit("download-progress", serde_json::json!({
                        "taskId": &task_id, "runId": &run_id, "fileName": &file_name, "downloaded": downloaded,
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
                has_error.store(true, Ordering::SeqCst);
                let _ = app.emit("download-error", serde_json::json!({
                    "taskId": &task_id, "runId": &run_id, "fileName": &file_name, "error": e.to_string(),
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
        let _ = app.emit("download-error", serde_json::json!({
            "taskId": &task_id, "runId": &run_id, "fileName": &file_name,
            "error": format!("Failed to finalize download: {}", e),
            "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
            "retryable": false,
        }));
        return;
    }
    cleanup_artifact_state(&temp_path);
    let _ = app.emit("download-complete", serde_json::json!({
        "taskId": &task_id, "runId": &run_id, "fileName": &file_name, "path": final_path.to_string_lossy(),
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
    let save_path = if Path::new(&save_dir).is_absolute() {
        PathBuf::from(&save_dir)
    } else {
        let managed = app.state::<AppState>();
        let config_dir = managed.config_dir.lock().unwrap();
        config_dir.parent().unwrap_or(Path::new(".")).to_path_buf().join(&save_dir)
    };
    let save_path = save_path.join(repo_id.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
    std::fs::create_dir_all(&save_path).map_err(|e| format!("创建目录失败: {}", e))?;

    let mut handles = Vec::new();
    use tokio::sync::Semaphore;
    let max_concurrent = *app.state::<AppState>().download_max_concurrent.lock().unwrap();
    let semaphore = Arc::new(Semaphore::new(max_concurrent.max(1)));
    let has_error = Arc::new(AtomicBool::new(false));

    // 清除本批次文件的 cancel/pause flags（避免上次暂停/取消的标记残留）
    {
        let state = app.state::<AppState>();
        let mut flags = state.cancel_flags.lock().unwrap();
        let mut pause = state.pause_flags.lock().unwrap();
        for file in &files {
            if let Some(run_id) = &file.run_id {
                flags.remove(run_id);
                pause.remove(run_id);
            }
        }
    }

    for file in files {
        let url = format!("https://modelscope.cn/models/{}/resolve/master/{}", repo_id, file.path);
        let dest_dir = save_path.clone();
        let rid = repo_id.clone();
        let task_id = file.task_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let run_id = file.run_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let remote_path = file.path.clone();
        let has_error = Arc::clone(&has_error);
        let handle = tokio::spawn({
            let app = app.clone();
            let permit = semaphore.clone().acquire_owned();
            async move {
                let _permit = permit.await;
                download_single_file(task_id, run_id, url, dest_dir, file.name.clone(), file.size, rid, "modelscope".into(), remote_path, app, has_error).await;
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
pub async fn cancel_and_cleanup_download(task_id: String, file_name: String, file_path: String, run_id: Option<String>, state: tauri::State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
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
    let _ = app.emit("download-removed", serde_json::json!({ "taskId": task_id, "fileName": file_name }));
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
    let save_path = if Path::new(&save_dir).is_absolute() {
        PathBuf::from(&save_dir)
    } else {
        let managed = app.state::<AppState>();
        let config_dir = managed.config_dir.lock().unwrap();
        config_dir.parent().unwrap_or(Path::new(".")).to_path_buf().join(&save_dir)
    };
    let save_path = save_path.join(repo_id.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
    std::fs::create_dir_all(&save_path).map_err(|e| format!("创建目录失败: {}", e))?;

    let max_concurrent = *app.state::<AppState>().download_max_concurrent.lock().unwrap();
    let semaphore = Arc::new(Semaphore::new(max_concurrent.max(1)));
    let has_error = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::new();

    // 清除本批次文件的 cancel/pause flags
    {
        let state = app.state::<AppState>();
        let mut flags = state.cancel_flags.lock().unwrap();
        let mut pause = state.pause_flags.lock().unwrap();
        for file in &files {
            if let Some(run_id) = &file.run_id {
                flags.remove(run_id);
                pause.remove(run_id);
            }
        }
    }

    for file in files {
        let url = format!("https://huggingface.co/{}/resolve/main/{}", repo_id, file.path);
        let dest_dir = save_path.clone();
        let rid = repo_id.clone();
        let task_id = file.task_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let run_id = file.run_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let remote_path = file.path.clone();
        let has_error = Arc::clone(&has_error);
        let handle = tokio::spawn({
            let app = app.clone();
            let permit = semaphore.clone().acquire_owned();
            async move {
                let _permit = permit.await;
                download_single_file(task_id, run_id, url, dest_dir, file.name.clone(), file.size, rid, "huggingface".into(), remote_path, app, has_error).await;
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

use crate::models::{DownloadState, PersistedQueueEntry};

fn persist_manager_queue(state: &AppState) {
    let queue = state.download_queue.lock().unwrap().clone();
    save_download_state(&queue, state);
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
        let max = *state.download_max_concurrent.lock().unwrap();
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
            if queue.len() != old_len { save_download_state(&queue, &state); }
            return;
        }
        let entry = queue.remove(0);
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
        save_download_state(&queue, &state);
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

        if let Err(_e) = result {
            if entry_for_retry.retries < entry_for_retry.max_retries {
                let delay_ms = 2000u64 * 2u64.pow(entry_for_retry.retries.min(5));
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

                let mut retry_entry = entry_for_retry;
                retry_entry.retries += 1;

                {
                    let state = app.state::<AppState>();
                    state.download_active_batches.lock().unwrap().remove(&batch_id);
                    state.download_active_entries.lock().unwrap().remove(&batch_id);
                    let mut queue = state.download_queue.lock().unwrap();
                    queue.insert(0, retry_entry);
                    save_download_state(&queue, &state);
                }

                process_download_queue_inner(app);
                return;
            }
        }

        {
            let state = app.state::<AppState>();
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
    *state.download_queue.lock().unwrap() = queue.iter().filter(|e| is_runtime_queued(e)).cloned().collect();
    save_download_state(&queue, &state);
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
    entry: PersistedQueueEntry,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
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
pub async fn resume_all_downloads(app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let queue = load_download_state(&state);
    let mut runtime = state.download_queue.lock().unwrap();
    for entry in &queue {
        if entry.status == "paused" && !runtime.iter().any(|e| e.id == entry.id) {
            let mut entry = entry.clone();
            entry.status = "queued".into();
            runtime.push(entry);
        }
    }
    save_download_state(&runtime, &state);
    drop(runtime);
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
        if !queue.iter().any(|e| e.id == entry.id) {
            queue.push(entry);
        }
    }
    save_download_state(&queue, &state);
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
    let mut cancel = state.cancel_flags.lock().unwrap();
    let active = state.active_downloads.lock().unwrap();
    for run_id in active.iter() {
        cancel.insert(run_id.clone(), true);
    }
    {
        let mut queue = state.download_queue.lock().unwrap();
        queue.clear();
        save_download_state(&queue, &state);
    }
    for run_id in active.iter() {
        let _ = app.emit("download-cancelled", serde_json::json!({ "taskId": run_id }));
    }
    Ok(())
}

// ── 并发控制命令 ──────────────────────────────────────────────────

#[tauri::command]
pub async fn set_download_concurrency(n: usize, state: tauri::State<'_, AppState>) -> Result<(), String> {
    if n < 1 || n > 8 { return Err("concurrency must be 1-8".into()); }
    *state.download_max_concurrent.lock().unwrap() = n;
    Ok(())
}

#[tauri::command]
pub async fn get_download_concurrency(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    Ok(*state.download_max_concurrent.lock().unwrap())
}

// ── 重置下载状态（重新下载专用） ────────────────────────────────────

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
}

#[tauri::command]
pub async fn get_download_manager_snapshot(state: tauri::State<'_, AppState>) -> Result<DownloadManagerSnapshot, String> {
    let queue = state.download_queue.lock().unwrap().clone();
    let active_count = state.download_active_batches.lock().unwrap().len();
    let max_concurrent = *state.download_max_concurrent.lock().unwrap();
    let config_dir = state.config_dir.lock().unwrap().clone();
    let config = crate::commands::config::read_config_from_disk(&config_dir);
    let resume_policy = config.download_resume_policy;
    Ok(DownloadManagerSnapshot { queue, active_count, max_concurrent, resume_policy })
}
