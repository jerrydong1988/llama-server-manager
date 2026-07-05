use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use crate::models::{AppState, MsFileEntry};
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

/// RAII guard: removes file_name from active_downloads on drop (all exit paths including panic)
struct ActiveDownloadGuard {
    app: tauri::AppHandle,
    file_name: String,
}
impl Drop for ActiveDownloadGuard {
    fn drop(&mut self) {
        self.app.state::<AppState>().active_downloads.lock().unwrap().remove(&self.file_name);
    }
}

/// 下载单个文件的通用逻辑。
async fn download_single_file(
    url: String,
    save_path: PathBuf,
    raw_file_name: String,
    file_size: u64,
    repo_id: String,
    source: String,
    app: tauri::AppHandle,
) {
    let file_name = match sanitize_file_name(&raw_file_name) {
        Ok(n) => n,
        Err(e) => {
            let _ = app.emit("download-error", serde_json::json!({
                "fileName": raw_file_name, "error": e, "repoId": repo_id, "source": source,
            }));
            return;
        }
    };
    let shared = app.state::<AppState>();

    {
        let mut active = shared.active_downloads.lock().unwrap();
        if !active.insert(file_name.clone()) {
            // 该文件已有活跃下载任务，跳过（避免两个任务同时写同一个文件）
            return;
        }
    }
    // RAII guard: 函数退出时自动从 active_downloads 中移除
    let _guard = ActiveDownloadGuard { app: app.clone(), file_name: file_name.clone() };

    if shared.cancel_flags.lock().unwrap().get(&file_name).copied().unwrap_or(false) {
        if !shared.pause_flags.lock().unwrap().get(&file_name).copied().unwrap_or(false) {
            let _ = app.emit("download-cancelled", serde_json::json!({ "fileName": file_name, "repoId": repo_id, "source": source }));
        }
        return;
    }

    let dest = save_path.join(&file_name);
    let resume_from = dest.metadata().map(|m| m.len()).unwrap_or(0);

    let mut req = HTTP_CLIENT.get(&url).header("User-Agent", "Mozilla/5.0");
    if resume_from > 0 {
        req = req.header("Range", format!("bytes={}-", resume_from));
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            let _ = app.emit("download-error", serde_json::json!({
                "fileName": file_name, "error": e.to_string(), "repoId": repo_id, "source": source,
            }));
            return;
        }
    };
    if !resp.status().is_success() && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        let msg = match resp.status().as_u16() {
            416 => "服务器不支持断点续传，请重新下载 / Range not satisfiable, please restart download",
            404 => "文件不存在 / File not found",
            403 => "访问被拒绝 / Access denied",
            429 => "请求过于频繁，请稍后重试 / Too many requests, please retry later",
            code => &format!("HTTP {}", code),
        };
        let _ = app.emit("download-error", serde_json::json!({
            "fileName": file_name, "error": msg, "repoId": repo_id, "source": source,
        }));
        return;
    }

    let is_partial = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let resume_from = if is_partial { resume_from } else { 0 };
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
        .open(&dest) {
        Ok(f) => f,
        Err(e) => {
            let _ = app.emit("download-error", serde_json::json!({
                "fileName": file_name, "error": format!("文件创建失败: {}", e), "repoId": repo_id, "source": source,
            }));
            return;
        }
    };

    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if shared.cancel_flags.lock().unwrap().get(&file_name).copied().unwrap_or(false) {
            if !shared.pause_flags.lock().unwrap().get(&file_name).copied().unwrap_or(false) {
                let _ = app.emit("download-cancelled", serde_json::json!({ "fileName": file_name, "repoId": repo_id, "source": source }));
            } else {
                let _ = app.emit("download-paused", serde_json::json!({ "fileName": file_name, "repoId": repo_id, "source": source, "downloaded": downloaded, "total": total }));
            }
            return;
        }
        match chunk {
            Ok(bytes) => {
                let len = bytes.len() as u64;
                if let Err(e) = file.write_all(&bytes) {
                        let _ = app.emit("download-error", serde_json::json!({
                            "fileName": file_name, "error": format!("写入失败: {}", e), "repoId": repo_id, "source": source,
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
                // 限流：最多 500ms 发一次事件，避免前端被事件洪泛冲垮
                if last_emit.elapsed().as_millis() >= 500 {
                    last_emit = now;
                    let _ = app.emit("download-progress", serde_json::json!({
                        "fileName": file_name, "downloaded": downloaded,
                        "total": total, "speed": speed, "repoId": repo_id, "source": source,
                    }));
                }
            }
            Err(e) => {
                let _ = app.emit("download-error", serde_json::json!({
                    "fileName": file_name, "error": e.to_string(), "repoId": repo_id, "source": source,
                }));
                return;
            }
        }
    }
    let _ = app.emit("download-complete", serde_json::json!({
        "fileName": file_name, "path": dest.to_string_lossy(), "repoId": repo_id, "source": source,
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
    use std::sync::Arc;
    use tokio::sync::Semaphore;
    let semaphore = Arc::new(Semaphore::new(3));

    // 清除本批次文件的 cancel/pause flags（避免上次暂停/取消的标记残留）
    {
        let state = app.state::<AppState>();
        let mut flags = state.cancel_flags.lock().unwrap();
        let mut pause = state.pause_flags.lock().unwrap();
        for file in &files { flags.remove(&file.name); pause.remove(&file.name); }
    }

    for file in files {
        let url = format!("https://modelscope.cn/models/{}/resolve/master/{}", repo_id, file.path);
        let dest_dir = save_path.clone();
        let rid = repo_id.clone();
        let handle = tokio::spawn({
            let app = app.clone();
            let permit = semaphore.clone().acquire_owned();
            async move {
                let _permit = permit.await;
                download_single_file(url, dest_dir, file.name.clone(), file.size, rid, "modelscope".into(), app).await;
            }
        });
        handles.push(handle);
    }

    for h in handles { let _ = h.await; }
    Ok(())
}

// ── 下载控制 ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn cancel_file_download(file_name: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.cancel_flags.lock().unwrap().insert(file_name, true);
    Ok(())
}

#[tauri::command]
pub async fn pause_file_download(file_name: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut pause = state.pause_flags.lock().unwrap();
    let mut cancel = state.cancel_flags.lock().unwrap();
    pause.insert(file_name.clone(), true);
    cancel.insert(file_name, true);
    Ok(())
}

#[tauri::command]
pub async fn cancel_and_cleanup_download(file_name: String, file_path: String, state: tauri::State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    let _ = sanitize_file_name(&file_name)?;
    // Canonicalize + verify the path is within a managed download directory
    let managed = state.config_dir.lock().unwrap();
    let root = managed.parent().unwrap_or(Path::new(".")).to_path_buf();
    let cpath = std::fs::canonicalize(Path::new(&file_path)).unwrap_or_else(|_| Path::new(&file_path).to_path_buf());
    let croot = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
    if !cpath.starts_with(&croot) {
        return Err("文件不在受管目录内".into());
    }
    state.cancel_flags.lock().unwrap().insert(file_name.clone(), true);
    state.pause_flags.lock().unwrap().remove(&file_name);
    let _ = std::fs::remove_file(&cpath);
    let _ = app.emit("download-removed", serde_json::json!({ "fileName": file_name }));
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
    use std::sync::Arc;
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

    let semaphore = Arc::new(Semaphore::new(3));
    let mut handles = Vec::new();

    // 清除本批次文件的 cancel/pause flags
    {
        let state = app.state::<AppState>();
        let mut flags = state.cancel_flags.lock().unwrap();
        let mut pause = state.pause_flags.lock().unwrap();
        for file in &files { flags.remove(&file.name); pause.remove(&file.name); }
    }

    for file in files {
        let url = format!("https://huggingface.co/{}/resolve/main/{}", repo_id, file.path);
        let dest_dir = save_path.clone();
        let rid = repo_id.clone();
        let handle = tokio::spawn({
            let app = app.clone();
            let permit = semaphore.clone().acquire_owned();
            async move {
                let _permit = permit.await;
                download_single_file(url, dest_dir, file.name.clone(), file.size, rid, "huggingface".into(), app).await;
            }
        });
        handles.push(handle);
    }

    for h in handles { let _ = h.await; }
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

#[tauri::command]
pub async fn persist_download_queue(
    queue: Vec<PersistedQueueEntry>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    save_download_state(&queue, &state);
    Ok(())
}

#[tauri::command]
pub async fn restore_download_queue(state: tauri::State<'_, AppState>) -> Result<Vec<PersistedQueueEntry>, String> {
    Ok(load_download_state(&state))
}
