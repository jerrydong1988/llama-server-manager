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
        .build()
        .unwrap_or_default()
});

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
    let save_path = if Path::new(&save_dir).is_absolute() {
        PathBuf::from(&save_dir)
    } else {
        let managed = app.state::<AppState>();
        let config_dir = managed.config_dir.lock().unwrap();
        config_dir.parent().unwrap_or(Path::new(".")).to_path_buf().join(&save_dir)
    };
    let save_path = save_path.join(repo_id.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
    std::fs::create_dir_all(&save_path).map_err(|e| format!("创建目录失败: {}", e))?;
    app.state::<AppState>().cancel_flags.lock().unwrap().clear();

    let mut handles = Vec::new();

    // 并发控制：使用 semaphore 限制最多 3 个同时下载
    use std::sync::Arc;
    use tokio::sync::Semaphore;
    let semaphore = Arc::new(Semaphore::new(3));

    for file in files {

        let url = format!(
            "https://modelscope.cn/models/{}/resolve/master/{}",
            repo_id, file.path
        );
        let save_path = save_path.clone();
        let app = app.clone();
        let file_name = file.name.clone();
        let file_size = file.size;

        let handle = tokio::spawn({
            let app = app.clone();
            let permit = semaphore.clone().acquire_owned();
            async move {
            let _permit = permit.await;
            let shared = app.state::<AppState>();
            if shared.cancel_flags.lock().unwrap().get(&file_name).copied().unwrap_or(false) {
                if !shared.pause_flags.lock().unwrap().get(&file_name).copied().unwrap_or(false) {
                    let _ = app.emit("download-cancelled", serde_json::json!({ "fileName": file_name }));
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
                        "fileName": file_name, "error": e.to_string()
                    }));
                    return;
                }
            };
            if !resp.status().is_success() && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                let _ = app.emit("download-error", serde_json::json!({
                    "fileName": file_name, "error": format!("HTTP {}", resp.status())
                }));
                return;
            }

            let total = if resume_from > 0 {
                resp.content_length().unwrap_or(0) + resume_from
            } else {
                resp.content_length().unwrap_or(file_size)
            };
            let mut downloaded = resume_from;
            let dl_start = std::time::Instant::now();
            let mut win_start = dl_start;
            let mut win_bytes: u64 = 0;

            use std::io::Write;
            let mut file = match std::fs::OpenOptions::new()
                .create(true).append(resume_from == 0).write(true)
                .open(&dest) {
                Ok(f) => f,
                Err(e) => {
                    let _ = app.emit("download-error", serde_json::json!({
                        "fileName": file_name, "error": format!("文件创建失败: {}", e)
                    }));
                    return;
                }
            };

            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                if shared.cancel_flags.lock().unwrap().get(&file_name).copied().unwrap_or(false) {
                    let _ = app.emit("download-cancelled", serde_json::json!({ "fileName": file_name }));
                    return;
                }
                match chunk {
                    Ok(bytes) => {
                        let len = bytes.len() as u64;
                        if let Err(e) = file.write_all(&bytes) {
                            let _ = app.emit("download-error", serde_json::json!({
                                "fileName": file_name, "error": format!("写入失败: {}", e)
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
                        let _ = app.emit("download-progress", serde_json::json!({
                            "fileName": file_name, "downloaded": downloaded,
                            "total": total, "speed": speed,
                        }));
                    }
                    Err(e) => {
                        let _ = app.emit("download-error", serde_json::json!({
                            "fileName": file_name, "error": e.to_string()
                        }));
                        return;
                    }
                }
            }
            let _ = app.emit("download-complete", serde_json::json!({
                "fileName": file_name, "path": dest.to_string_lossy(),
            }));
            }
        });
        handles.push(handle);
    }

    for h in handles {
        let _ = h.await;
    }
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
    state.pause_flags.lock().unwrap().insert(file_name.clone(), true);
    state.cancel_flags.lock().unwrap().insert(file_name, true);
    Ok(())
}

#[tauri::command]
pub async fn cancel_and_cleanup_download(file_name: String, file_path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.cancel_flags.lock().unwrap().insert(file_name.clone(), true);
    state.pause_flags.lock().unwrap().remove(&file_name);
    let _ = std::fs::remove_file(&file_path);
    Ok(())
}

// ── HuggingFace 数据结构和浏览 ──────────────────────────────────
#[derive(serde::Deserialize)]
struct HfRepoInfo {
    siblings: Vec<HfSibling>,
}

#[derive(serde::Deserialize)]
struct HfSibling {
    rfilename: String,
    size: u64,
}

#[tauri::command]
pub async fn browse_huggingface(repo_id: String) -> Result<Vec<MsFileEntry>, String> {
    let url = format!("https://huggingface.co/api/models/{}", repo_id);
    let resp = HTTP_CLIENT.get(&url).send().await.map_err(|e| format!("网络错误: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("仓库未找到 (HTTP {})", resp.status()));
    }
    let info: HfRepoInfo = resp.json().await.map_err(|e| format!("解析失败: {}", e))?;

    let mut result: Vec<MsFileEntry> = info.siblings.iter().filter_map(|s| {
        let name = s.rfilename.split('/').last()?.to_string();
        if !name.ends_with(".gguf") && !name.ends_with(".txt") { return None; }
        Some(MsFileEntry {
            file_type: utils::classify_gguf_file(Path::new(&name)).to_string(),
            name,
            path: s.rfilename.clone(),
            size: s.size,
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

    let save_path = if Path::new(&save_dir).is_absolute() {
        PathBuf::from(&save_dir)
    } else {
        let managed = app.state::<AppState>();
        let config_dir = managed.config_dir.lock().unwrap();
        config_dir.parent().unwrap_or(Path::new(".")).to_path_buf().join(&save_dir)
    };
    let save_path = save_path.join(repo_id.replace('/', &std::path::MAIN_SEPARATOR.to_string()));
    std::fs::create_dir_all(&save_path).map_err(|e| format!("创建目录失败: {}", e))?;
    app.state::<AppState>().cancel_flags.lock().unwrap().clear();

    let semaphore = Arc::new(Semaphore::new(3));
    let mut handles = Vec::new();

    for file in files {
        let url = format!("https://huggingface.co/{}/resolve/main/{}", repo_id, file.path);
        let save_path = save_path.clone();
        let app = app.clone();
        let file_name = file.name.clone();
        let file_size = file.size;

        let handle = tokio::spawn({
            let app = app.clone();
            let permit = semaphore.clone().acquire_owned();
            async move {
                let _permit = permit.await;
                let shared = app.state::<AppState>();
                if shared.cancel_flags.lock().unwrap().get(&file_name).copied().unwrap_or(false) {
                    if !shared.pause_flags.lock().unwrap().get(&file_name).copied().unwrap_or(false) {
                        let _ = app.emit("download-cancelled", serde_json::json!({ "fileName": file_name }));
                    }
                    return;
                }

                let dest = save_path.join(&file_name);
                let resume_from = dest.metadata().map(|m| m.len()).unwrap_or(0);
                let mut req = HTTP_CLIENT.get(&url);
                if resume_from > 0 {
                    req = req.header("Range", format!("bytes={}-", resume_from));
                }

                let resp = match req.send().await {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = app.emit("download-error", serde_json::json!({ "fileName": file_name, "error": e.to_string() }));
                        return;
                    }
                };
                if !resp.status().is_success() && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                    let _ = app.emit("download-error", serde_json::json!({ "fileName": file_name, "error": format!("HTTP {}", resp.status()) }));
                    return;
                }

                let total = if resume_from > 0 {
                    resp.content_length().unwrap_or(0) + resume_from
                } else {
                    resp.content_length().unwrap_or(file_size)
                };
                let mut downloaded = resume_from;
                let mut win_start = std::time::Instant::now();
                let mut win_bytes: u64 = 0;

                use std::io::Write;
                let mut file = match std::fs::OpenOptions::new()
                    .create(true).append(resume_from == 0).write(true)
                    .open(&dest) {
                    Ok(f) => f,
                    Err(e) => {
                        let _ = app.emit("download-error", serde_json::json!({ "fileName": file_name, "error": format!("文件创建失败: {}", e) }));
                        return;
                    }
                };

                let mut stream = resp.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    if shared.cancel_flags.lock().unwrap().get(&file_name).copied().unwrap_or(false) {
                        let _ = app.emit("download-cancelled", serde_json::json!({ "fileName": file_name }));
                        return;
                    }
                    match chunk {
                        Ok(bytes) => {
                            if let Err(e) = file.write_all(&bytes) {
                                let _ = app.emit("download-error", serde_json::json!({ "fileName": file_name, "error": format!("写入失败: {}", e) }));
                                return;
                            }
                            downloaded += bytes.len() as u64;
                            let now = std::time::Instant::now();
                            win_bytes += bytes.len() as u64;
                            let win_elapsed = now.duration_since(win_start).as_secs_f64();
                            let speed = if win_elapsed >= 1.0 {
                                let s = win_bytes as f64 / win_elapsed;
                                win_start = now; win_bytes = 0; s
                            } else if win_elapsed > 0.0 { win_bytes as f64 / win_elapsed } else { 0.0 };
                            let _ = app.emit("download-progress", serde_json::json!({ "fileName": file_name, "downloaded": downloaded, "total": total, "speed": speed }));
                        }
                        Err(e) => {
                            let _ = app.emit("download-error", serde_json::json!({ "fileName": file_name, "error": e.to_string() }));
                            return;
                        }
                    }
                }
                let _ = app.emit("download-complete", serde_json::json!({ "fileName": file_name, "path": dest.to_string_lossy() }));
            }
        });
        handles.push(handle);
    }

    for h in handles { let _ = h.await; }
    Ok(())
}
