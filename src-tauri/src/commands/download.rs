use crate::models::{AppState, DownloadArtifactState, MsFileEntry, PersistedQueueEntry};
use crate::utils;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, CONTENT_LENGTH, CONTENT_RANGE, IF_RANGE};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use tauri::{Emitter, Manager};

// Shared HTTP client for all download operations
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default()
});

const DOWNLOAD_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const DOWNLOAD_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

static DOWNLOAD_STATE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

// #9: Shared download core.

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
    apply_download_priority_concurrency(
        configured,
        *state.download_low_priority_throttle.lock().unwrap(),
    )
}

fn apply_download_priority_concurrency(configured: usize, low_priority: bool) -> usize {
    if low_priority {
        1
    } else {
        configured.max(1)
    }
}

struct GlobalDownloadSlot {
    app: tauri::AppHandle,
}

impl Drop for GlobalDownloadSlot {
    fn drop(&mut self) {
        let state = self.app.state::<AppState>();
        state
            .download_active_file_slots
            .fetch_sub(1, Ordering::AcqRel);
        state.download_slot_notify.notify_waiters();
    }
}

async fn acquire_global_download_slot(app: &tauri::AppHandle) -> GlobalDownloadSlot {
    let notify = app.state::<AppState>().download_slot_notify.clone();
    loop {
        let state = app.state::<AppState>();
        let limit = effective_download_concurrency(&state);
        let mut active = state.download_active_file_slots.load(Ordering::Acquire);
        while active < limit {
            match state.download_active_file_slots.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return GlobalDownloadSlot { app: app.clone() },
                Err(current) => active = current,
            }
        }
        let notified = notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let should_wait = {
            let state = app.state::<AppState>();
            state.download_active_file_slots.load(Ordering::Acquire)
                >= effective_download_concurrency(&state)
        };
        if should_wait {
            notified.as_mut().await;
        }
    }
}

fn active_download_slot_count(state: &AppState) -> usize {
    let file_slots: usize = {
        let active_entries = state.download_active_entries.lock().unwrap();
        active_entries
            .values()
            .map(|entry| entry.files.len().max(1))
            .sum()
    };
    if file_slots > 0 {
        file_slots
    } else {
        state.download_active_batches.lock().unwrap().len()
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
    if repo_id.starts_with('/')
        || repo_id.ends_with('/')
        || repo_id.contains("//")
        || repo_id.contains("..")
        || repo_id.contains('\\')
        || Path::new(repo_id).is_absolute()
        || Path::new(repo_id).has_root()
    {
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
    if name.is_empty() {
        return Err("文件名不能为空".into());
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(format!("文件名包含非法路径字符: {}", name));
    }
    #[cfg(target_os = "windows")]
    {
        if name.len() >= 2 && name.as_bytes()[1] == b':' {
            return Err(format!("文件名包含非法路径字符: {}", name));
        }
    }
    // Ensure name is only a file name without path separators.
    let path = Path::new(name);
    if path.file_name().and_then(|s| s.to_str()) != Some(name) {
        return Err(format!("文件名包含路径分隔符: {}", name));
    }
    Ok(name.to_string())
}

fn remote_parent_dir(root: &Path, remote_path: &str) -> Result<PathBuf, String> {
    if remote_path.is_empty() || remote_path.starts_with('/') || remote_path.contains('\\') {
        return Err("Remote file path is invalid".into());
    }
    let mut destination = root.to_path_buf();
    let mut segments = remote_path.split('/').peekable();
    while let Some(segment) = segments.next() {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err("Remote file path contains an unsafe segment".into());
        }
        if segments.peek().is_some() {
            destination.push(segment);
        }
    }
    Ok(destination)
}

fn percent_encode_path(remote_path: &str) -> Result<String, String> {
    let _ = remote_parent_dir(Path::new("."), remote_path)?;
    let mut encoded = String::new();
    for byte in remote_path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    Ok(encoded)
}

/// RAII guard: removes run_id from active_downloads on drop (all exit paths including panic)
struct ActiveDownloadGuard {
    app: tauri::AppHandle,
    run_id: String,
    path_key: String,
}
impl Drop for ActiveDownloadGuard {
    fn drop(&mut self) {
        let state = self.app.state::<AppState>();
        state.active_downloads.lock().unwrap().remove(&self.run_id);
        state
            .active_download_paths
            .lock()
            .unwrap()
            .remove(&self.path_key);
    }
}

fn normalized_destination_key(path: &Path) -> String {
    let absolute = path
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .and_then(|parent| path.file_name().map(|name| parent.join(name)))
        .unwrap_or_else(|| path.to_path_buf());
    let key = absolute.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParsedContentRange {
    start: Option<u64>,
    end: Option<u64>,
    total: Option<u64>,
}

fn parse_content_range(value: &str) -> Option<ParsedContentRange> {
    let value = value.trim();
    let rest = value.strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let total = if total == "*" {
        None
    } else {
        Some(total.parse().ok()?)
    };
    if range == "*" {
        return Some(ParsedContentRange {
            start: None,
            end: None,
            total,
        });
    }
    let (start, end) = range.split_once('-')?;
    let start = start.parse().ok()?;
    let end = end.parse().ok()?;
    if end < start {
        return None;
    }
    Some(ParsedContentRange {
        start: Some(start),
        end: Some(end),
        total,
    })
}

fn response_content_range(headers: &HeaderMap) -> Option<ParsedContentRange> {
    headers
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_content_range)
}

fn response_total_size(headers: &HeaderMap, resume_from: u64, fallback_size: u64) -> u64 {
    if resume_from > 0 {
        if let Some(total) = response_content_range(headers).and_then(|range| range.total) {
            return total;
        }
    }
    headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|content_length| content_length.saturating_add(resume_from))
        .unwrap_or(fallback_size)
}

fn validate_partial_response(
    headers: &HeaderMap,
    expected_start: u64,
    expected_total: u64,
) -> Result<(), String> {
    let range = response_content_range(headers)
        .ok_or_else(|| "206 response is missing a valid Content-Range header".to_string())?;
    let (Some(start), Some(end)) = (range.start, range.end) else {
        return Err("206 response contains an unsatisfied Content-Range".into());
    };
    if start != expected_start {
        return Err(format!(
            "206 response starts at byte {start}, expected {expected_start}"
        ));
    }
    if let Some(total) = range.total {
        if expected_total > 0 && total != expected_total {
            return Err(format!(
                "remote object size changed from {expected_total} to {total} bytes"
            ));
        }
        if end >= total {
            return Err(format!(
                "206 response ends at byte {end}, outside total size {total}"
            ));
        }
    }
    if let Some(content_length) = headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        let range_length = end.saturating_sub(start).saturating_add(1);
        if content_length != range_length {
            return Err(format!(
                "206 response length is {content_length}, expected {range_length}"
            ));
        }
    }
    Ok(())
}

fn unsatisfied_range_is_complete(
    part_size: u64,
    expected_size: u64,
    remote_size: Option<u64>,
) -> bool {
    expected_size > 0 && part_size == expected_size && remote_size == Some(expected_size)
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

fn write_string_atomic(path: &Path, contents: &str) -> Result<(), String> {
    crate::persistence::atomic_write(path, contents.as_bytes(), None)
}

fn load_artifact_state(temp_path: &Path) -> Option<DownloadArtifactState> {
    let metadata_path = artifact_state_path(temp_path);
    std::fs::read_to_string(&metadata_path)
        .ok()
        .and_then(|s| serde_json::from_str::<DownloadArtifactState>(&s).ok())
}

fn save_artifact_state(state: &DownloadArtifactState, temp_path: &Path) {
    let metadata_path = artifact_state_path(temp_path);
    let result = serde_json::to_string(state)
        .map_err(|error| format!("failed to serialize download artifact state: {error}"))
        .and_then(|json| write_string_atomic(&metadata_path, &json));
    if let Err(error) = result {
        eprintln!("Failed to persist download artifact state: {error}");
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

fn normalize_path_for_compare(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn queue_entry_download_dir(
    base_dir: &Path,
    entry: &PersistedQueueEntry,
) -> Result<PathBuf, String> {
    let repo_id = sanitize_repo_id(&entry.repo_id)?;
    let managed_root = queue_entry_managed_root(base_dir, entry)?;
    Ok(managed_root.join(repo_id.replace('/', std::path::MAIN_SEPARATOR_STR)))
}

fn queue_entry_managed_root(
    base_dir: &Path,
    entry: &PersistedQueueEntry,
) -> Result<PathBuf, String> {
    let save_dir = Path::new(&entry.save_dir);
    if save_dir
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("Download save directory cannot contain parent traversal".into());
    }
    Ok(if save_dir.is_absolute() {
        save_dir.to_path_buf()
    } else {
        base_dir.join(save_dir)
    })
}

fn verified_managed_cleanup_path(root: &Path, path: &Path) -> Result<PathBuf, String> {
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("Download cleanup path contains parent traversal".into());
    }
    let canonical_root = match std::fs::canonicalize(root) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let normalized_root = normalize_path_for_compare(root);
            let normalized_path = normalize_path_for_compare(path);
            if normalized_path.starts_with(&normalized_root) {
                return Ok(path.to_path_buf());
            }
            return Err("文件不在受管目录内".into());
        }
        Err(error) => {
            return Err(format!("Failed to resolve managed download root: {error}"));
        }
    };
    let parent = path
        .parent()
        .ok_or_else(|| "Download cleanup path has no parent directory".to_string())?;
    let mut existing_ancestor = parent;
    let canonical_ancestor = loop {
        match std::fs::canonicalize(existing_ancestor) {
            Ok(path) => break path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing_ancestor = existing_ancestor.parent().ok_or_else(|| {
                    "Download cleanup path has no existing managed ancestor".to_string()
                })?;
            }
            Err(error) => {
                return Err(format!("Failed to resolve download directory: {error}"));
            }
        }
    };
    if !canonical_ancestor.starts_with(&canonical_root) {
        return Err("文件不在受管目录内".into());
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| "Download cleanup path has no file name".to_string())?;
    if parent.exists() {
        Ok(std::fs::canonicalize(parent)
            .map_err(|error| format!("Failed to resolve download directory: {error}"))?
            .join(file_name))
    } else {
        // No deletion can occur below a missing parent, but cleanup may still remove the task.
        Ok(path.to_path_buf())
    }
}

fn refresh_download_file_identity(file: &mut MsFileEntry) -> ResumeDownloadTaskResult {
    let task_id = file
        .task_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let run_id = uuid::Uuid::new_v4().to_string();
    let version = file.version.unwrap_or(0) + 1;
    file.task_id = Some(task_id.clone());
    file.run_id = Some(run_id.clone());
    file.version = Some(version);
    file.status = Some("queued".into());
    file.error = None;
    ResumeDownloadTaskResult {
        task_id,
        run_id,
        version,
    }
}

fn normalize_crash_recovered_entry(entry: &mut PersistedQueueEntry) {
    if matches!(entry.status.as_str(), "active" | "pausing") {
        entry.status = "paused".into();
    }
    for file in entry.files.iter_mut() {
        if matches!(file.status.as_deref(), Some("active" | "pausing")) {
            file.status = Some("paused".into());
        } else if file.status.is_none() {
            file.status = Some(entry.status.clone());
        }
    }
}

fn trusted_download_cleanup_paths(
    entries: &[PersistedQueueEntry],
    base_dir: &Path,
    task_id: &str,
    file_name: &str,
    run_id: Option<&str>,
    frontend_path: Option<&Path>,
) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), String> {
    let sanitized = sanitize_file_name(file_name)?;
    let mut candidates = Vec::new();
    for entry in entries {
        for file in &entry.files {
            if file.task_id.as_deref() != Some(task_id) {
                continue;
            }
            if let Some(expected_run_id) = run_id {
                if file.run_id.as_deref() != Some(expected_run_id) {
                    continue;
                }
            }
            if file.name != sanitized {
                return Err("Download task file name does not match registered file".into());
            }
            let managed_root = queue_entry_managed_root(base_dir, entry)?;
            let dir = remote_parent_dir(&queue_entry_download_dir(base_dir, entry)?, &file.path)?;
            let (final_path, temp_path, metadata_path) = build_download_paths(&dir, &file.name);
            candidates.push((managed_root, final_path, temp_path, metadata_path));
        }
    }

    let (managed_root, final_path, temp_path, metadata_path) = candidates
        .into_iter()
        .next()
        .ok_or_else(|| "Download task not found".to_string())?;

    if let Some(frontend_path) = frontend_path {
        let expected = normalize_path_for_compare(&final_path);
        let provided = normalize_path_for_compare(
            if frontend_path.is_absolute() {
                frontend_path.to_path_buf()
            } else {
                base_dir.join(frontend_path)
            }
            .as_path(),
        );
        if provided != expected {
            return Err("Frontend file path does not match registered download task".into());
        }
    }

    Ok((managed_root, final_path, temp_path, metadata_path))
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
        payload.insert(
            "taskId".into(),
            serde_json::Value::String(self.task_id.clone()),
        );
        payload.insert(
            "runId".into(),
            serde_json::Value::String(self.run_id.clone()),
        );
        payload.insert(
            "version".into(),
            serde_json::Value::Number(self.version.into()),
        );
        payload.insert(
            "fileName".into(),
            serde_json::Value::String(self.file_name.clone()),
        );
        payload.insert(
            "repoId".into(),
            serde_json::Value::String(self.repo_id.clone()),
        );
        payload.insert(
            "source".into(),
            serde_json::Value::String(self.source.clone()),
        );
        payload.insert(
            "remotePath".into(),
            serde_json::Value::String(self.remote_path.clone()),
        );

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
        task_id: file
            .task_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        run_id: file
            .run_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        version: file.version.unwrap_or(0),
        file_name: file.name.clone(),
        repo_id: repo_id.to_string(),
        source: source.to_string(),
        remote_path: file.path.clone(),
    }
}

fn resolve_repo_save_path(
    app: &tauri::AppHandle,
    save_dir: &str,
    repo_id: &str,
) -> Result<PathBuf, String> {
    let base_path = if Path::new(save_dir).is_absolute() {
        PathBuf::from(save_dir)
    } else {
        let managed = app.state::<AppState>();
        let config_dir = managed.config_dir.lock().unwrap();
        config_dir
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf()
            .join(save_dir)
    };
    let save_path = base_path.join(repo_id.replace('/', std::path::MAIN_SEPARATOR_STR));
    std::fs::create_dir_all(&save_path)
        .map_err(|e| format!("Failed to create download directory: {}", e))?;
    Ok(save_path)
}

fn resolve_redownload_save_path(
    config_dir: &Path,
    save_dir: &str,
    repo_id: &str,
) -> Result<PathBuf, String> {
    let repo_id = sanitize_repo_id(repo_id)?;
    let base_path = if Path::new(save_dir).is_absolute() {
        PathBuf::from(save_dir)
    } else {
        config_dir.parent().unwrap_or(Path::new(".")).join(save_dir)
    };
    Ok(base_path.join(repo_id.replace('/', std::path::MAIN_SEPARATOR_STR)))
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

/// Shared logic for downloading a single file.
async fn download_single_file(
    ctx: DownloadTaskContext,
    url: String,
    save_path: PathBuf,
    file_size: u64,
    app: tauri::AppHandle,
    has_error: Arc<AtomicBool>,
    has_non_retryable_error: Arc<AtomicBool>,
) {
    let file_name = match sanitize_file_name(&ctx.file_name) {
        Ok(n) => n,
        Err(e) => {
            has_error.store(true, Ordering::SeqCst);
            has_non_retryable_error.store(true, Ordering::SeqCst);
            ctx.emit(
                &app,
                "download-error",
                serde_json::json!({
                    "error": e,
                    "retryable": false,
                }),
            );
            return;
        }
    };
    let (final_path, temp_path, _metadata_path) = build_download_paths(&save_path, &file_name);
    let shared = app.state::<AppState>();
    let path_key = normalized_destination_key(&final_path);

    {
        let mut active_paths = shared.active_download_paths.lock().unwrap();
        if !active_paths.insert(path_key.clone()) {
            has_error.store(true, Ordering::SeqCst);
            has_non_retryable_error.store(true, Ordering::SeqCst);
            ctx.emit(
                &app,
                "download-error",
                serde_json::json!({
                    "error": "Another download is already writing this destination",
                    "retryable": false,
                }),
            );
            return;
        }
    }
    shared
        .active_downloads
        .lock()
        .unwrap()
        .insert(ctx.run_id.clone());
    let _guard = ActiveDownloadGuard {
        app: app.clone(),
        run_id: ctx.run_id.clone(),
        path_key,
    };
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

    if shared
        .cancel_flags
        .lock()
        .unwrap()
        .get(&run_id)
        .copied()
        .unwrap_or(false)
    {
        let paused = shared
            .pause_flags
            .lock()
            .unwrap()
            .get(&run_id)
            .copied()
            .unwrap_or(false);
        if !paused {
            cleanup_artifact_state(&temp_path);
            save_artifact_state(
                &DownloadArtifactState {
                    task_id: task_id.clone(),
                    run_id: run_id.clone(),
                    repo_id: repo_id.clone(),
                    source: source.clone(),
                    remote_path: remote_path.clone(),
                    final_path: final_path.to_string_lossy().to_string(),
                    temp_path: temp_path.to_string_lossy().to_string(),
                    expected_size: file_size,
                    downloaded_size: resume_from,
                    etag: save_etag.clone(),
                    last_modified: save_lm.clone(),
                    updated_at: now_secs(),
                },
                &temp_path,
            );
            cleanup_artifact_state(&temp_path);
            update_manager_file_state(
                &shared,
                &task_id,
                FileStatePatch {
                    downloaded: Some(resume_from),
                    version: Some(ctx.version),
                    status: Some("cancelled".into()),
                    ..Default::default()
                },
            );
            let _ = app.emit("download-cancelled", serde_json::json!({
                    "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
                    "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                }));
        } else {
            update_manager_file_state(
                &shared,
                &task_id,
                FileStatePatch {
                    downloaded: Some(resume_from),
                    size: Some(file_size),
                    version: Some(ctx.version),
                    status: Some("paused".into()),
                    ..Default::default()
                },
            );
            ctx.emit(
                &app,
                "download-paused",
                serde_json::json!({ "downloaded": resume_from, "total": file_size }),
            );
        }
        return;
    }

    let mut req = HTTP_CLIENT.get(&url).header("User-Agent", "Mozilla/5.0");
    if resume_from > 0 {
        req = req.header("Range", format!("bytes={}-", resume_from));
        let if_range = artifact.as_ref().and_then(|state| {
            state
                .etag
                .as_deref()
                .filter(|etag| !etag.trim_start().starts_with("W/"))
                .or(state.last_modified.as_deref())
        });
        if let Some(validator) = if_range {
            req = req.header(IF_RANGE, validator);
        }
    }

    let resp = match tokio::time::timeout(DOWNLOAD_RESPONSE_TIMEOUT, req.send()).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            save_artifact_state(
                &DownloadArtifactState {
                    task_id: task_id.clone(),
                    run_id: run_id.clone(),
                    repo_id: repo_id.clone(),
                    source: source.clone(),
                    remote_path: remote_path.clone(),
                    final_path: final_path.to_string_lossy().to_string(),
                    temp_path: temp_path.to_string_lossy().to_string(),
                    expected_size: file_size,
                    downloaded_size: resume_from,
                    etag: save_etag.clone(),
                    last_modified: save_lm.clone(),
                    updated_at: now_secs(),
                },
                &temp_path,
            );
            has_error.store(true, Ordering::SeqCst);
            update_manager_file_state(
                &shared,
                &task_id,
                FileStatePatch {
                    downloaded: Some(resume_from),
                    version: Some(ctx.version),
                    status: Some("error".into()),
                    error: Some(Some(e.to_string())),
                    ..Default::default()
                },
            );
            let _ = app.emit("download-error", serde_json::json!({
                "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name, "error": e.to_string(),
                "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                "retryable": true,
            }));
            return;
        }
        Err(_) => {
            has_error.store(true, Ordering::SeqCst);
            let message = "Timed out waiting for download response headers".to_string();
            update_manager_file_state(
                &shared,
                &task_id,
                FileStatePatch {
                    downloaded: Some(resume_from),
                    version: Some(ctx.version),
                    status: Some("error".into()),
                    error: Some(Some(message.clone())),
                    ..Default::default()
                },
            );
            ctx.emit(
                &app,
                "download-error",
                serde_json::json!({ "error": message, "retryable": true }),
            );
            return;
        }
    };
    // A1-06: Read ETag and Last-Modified from response headers
    let resp_etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let resp_last_modified = resp
        .headers()
        .get("last-modified")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    save_etag = resp_etag.clone();
    save_lm = resp_last_modified.clone();
    // A1-06: Persist updated artifact state immediately after reading headers
    save_artifact_state(
        &DownloadArtifactState {
            task_id: task_id.clone(),
            run_id: run_id.clone(),
            repo_id: repo_id.clone(),
            source: source.clone(),
            remote_path: remote_path.clone(),
            final_path: final_path.to_string_lossy().to_string(),
            temp_path: temp_path.to_string_lossy().to_string(),
            expected_size: file_size,
            downloaded_size: resume_from,
            etag: save_etag.clone(),
            last_modified: save_lm.clone(),
            updated_at: now_secs(),
        },
        &temp_path,
    );

    // A 416 is only a completion signal when local and remote sizes agree exactly.
    if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        let part_size = temp_path.metadata().map(|m| m.len()).unwrap_or(0);
        let remote_size = response_content_range(resp.headers()).and_then(|range| range.total);
        let exact_size = unsatisfied_range_is_complete(part_size, file_size, remote_size);
        if exact_size {
            if let Err(error) = crate::persistence::replace_file(&temp_path, &final_path, None) {
                has_error.store(true, Ordering::SeqCst);
                update_manager_file_state(
                    &shared,
                    &task_id,
                    FileStatePatch {
                        downloaded: Some(part_size),
                        version: Some(ctx.version),
                        status: Some("error".into()),
                        error: Some(Some(error.clone())),
                        ..Default::default()
                    },
                );
                ctx.emit(
                    &app,
                    "download-error",
                    serde_json::json!({ "error": error, "retryable": true }),
                );
                return;
            }
            cleanup_artifact_state(&temp_path);
            update_manager_file_state(
                &shared,
                &task_id,
                FileStatePatch {
                    downloaded: Some(file_size),
                    size: Some(file_size),
                    version: Some(ctx.version),
                    status: Some("completed".into()),
                    error: Some(None),
                    ..Default::default()
                },
            );
            ctx.emit(
                &app,
                "download-complete",
                serde_json::json!({ "path": final_path.to_string_lossy() }),
            );
            return;
        }

        let remote_changed = remote_size
            .map(|remote| remote != file_size)
            .unwrap_or(false);
        if part_size > file_size || remote_changed {
            let _ = std::fs::remove_file(&temp_path);
            cleanup_artifact_state(&temp_path);
            has_error.store(true, Ordering::SeqCst);
            let message = if let Some(remote) = remote_size.filter(|remote| *remote != file_size) {
                format!("Remote object size changed from {file_size} to {remote} bytes")
            } else {
                "Download corrupted: part file is larger than expected".to_string()
            };
            update_manager_file_state(
                &shared,
                &task_id,
                FileStatePatch {
                    downloaded: Some(0),
                    version: Some(ctx.version),
                    status: Some("error".into()),
                    error: Some(Some(message.clone())),
                    ..Default::default()
                },
            );
            ctx.emit(
                &app,
                "download-error",
                serde_json::json!({ "error": message, "retryable": true }),
            );
            return;
        }
        save_artifact_state(
            &DownloadArtifactState {
                task_id: task_id.clone(),
                run_id: run_id.clone(),
                repo_id: repo_id.clone(),
                source: source.clone(),
                remote_path: remote_path.clone(),
                final_path: final_path.to_string_lossy().to_string(),
                temp_path: temp_path.to_string_lossy().to_string(),
                expected_size: file_size,
                downloaded_size: 0,
                etag: save_etag.clone(),
                last_modified: save_lm.clone(),
                updated_at: now_secs(),
            },
            &temp_path,
        );
        has_error.store(true, Ordering::SeqCst);
        update_manager_file_state(
            &shared,
            &task_id,
            FileStatePatch {
                downloaded: Some(0),
                version: Some(ctx.version),
                status: Some("error".into()),
                error: Some(Some(
                    "Server does not support resume, please restart download".into(),
                )),
                ..Default::default()
            },
        );
        let _ = app.emit("download-error", serde_json::json!({
            "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
            "error": "Resume offset is outside the remote object; restart the download",
            "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
            "retryable": true,
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
        if !retryable {
            has_non_retryable_error.store(true, Ordering::SeqCst);
        }
        save_artifact_state(
            &DownloadArtifactState {
                task_id: task_id.clone(),
                run_id: run_id.clone(),
                repo_id: repo_id.clone(),
                source: source.clone(),
                remote_path: remote_path.clone(),
                final_path: final_path.to_string_lossy().to_string(),
                temp_path: temp_path.to_string_lossy().to_string(),
                expected_size: file_size,
                downloaded_size: resume_from,
                etag: save_etag.clone(),
                last_modified: save_lm.clone(),
                updated_at: now_secs(),
            },
            &temp_path,
        );
        has_error.store(true, Ordering::SeqCst);
        update_manager_file_state(
            &shared,
            &task_id,
            FileStatePatch {
                downloaded: Some(resume_from),
                version: Some(ctx.version),
                status: Some("error".into()),
                error: Some(Some(msg.to_string())),
                ..Default::default()
            },
        );
        let _ = app.emit("download-error", serde_json::json!({
            "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name, "error": msg,
            "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
            "retryable": retryable,
        }));
        return;
    }

    let is_partial = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if is_partial {
        if let Err(error) = validate_partial_response(resp.headers(), resume_from, file_size) {
            let _ = std::fs::remove_file(&temp_path);
            cleanup_artifact_state(&temp_path);
            has_error.store(true, Ordering::SeqCst);
            update_manager_file_state(
                &shared,
                &task_id,
                FileStatePatch {
                    downloaded: Some(0),
                    version: Some(ctx.version),
                    status: Some("error".into()),
                    error: Some(Some(error.clone())),
                    ..Default::default()
                },
            );
            ctx.emit(
                &app,
                "download-error",
                serde_json::json!({ "error": error, "retryable": true }),
            );
            return;
        }
    }
    let mut resume_from = if is_partial { resume_from } else { 0 };

    // A1-05: 200 OK with .part file means server ignored Range header, so restart.
    if !is_partial && temp_path.exists() {
        update_manager_file_state(
            &shared,
            &task_id,
            FileStatePatch {
                downloaded: Some(0),
                version: Some(ctx.version),
                status: Some("active".into()),
                error: Some(None),
                ..Default::default()
            },
        );
        let _ = app.emit("download-restarted", serde_json::json!({
            "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
            "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
        }));
        resume_from = 0;
    }

    // A1-06: Check if remote file changed (ETag/Last-Modified mismatch on resume)
    if let Some(ref old_state) = artifact {
        let etag_changed =
            resp_etag.is_some() && old_state.etag.is_some() && resp_etag != old_state.etag;
        let lm_changed = resp_last_modified.is_some()
            && old_state.last_modified.is_some()
            && resp_last_modified != old_state.last_modified;
        if (etag_changed || lm_changed) && temp_path.exists() {
            update_manager_file_state(
                &shared,
                &task_id,
                FileStatePatch {
                    downloaded: Some(0),
                    version: Some(ctx.version),
                    status: Some("active".into()),
                    error: Some(None),
                    ..Default::default()
                },
            );
            let _ = app.emit("download-remote-changed", serde_json::json!({
                "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
                "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
            }));
            cleanup_artifact_state(&temp_path);
            has_error.store(true, Ordering::SeqCst);
            let error = "Remote object changed during resume; restarting from byte zero";
            update_manager_file_state(
                &shared,
                &task_id,
                FileStatePatch {
                    downloaded: Some(0),
                    version: Some(ctx.version),
                    status: Some("error".into()),
                    error: Some(Some(error.into())),
                    ..Default::default()
                },
            );
            ctx.emit(
                &app,
                "download-error",
                serde_json::json!({ "error": error, "retryable": true }),
            );
            return;
        }
    }
    let total = response_total_size(resp.headers(), resume_from, file_size);
    let mut downloaded = resume_from;
    let mut win_start = std::time::Instant::now();
    let mut win_bytes: u64 = 0;
    let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(1); // Emit immediately the first time.
    let mut last_artifact_save = std::time::Instant::now() - std::time::Duration::from_secs(2);

    use std::io::Write;
    let mut file = match std::fs::OpenOptions::new()
        .create(true)
        .append(is_partial)
        .truncate(!is_partial)
        .write(true)
        .open(&temp_path)
    {
        Ok(f) => f,
        Err(e) => {
            save_artifact_state(
                &DownloadArtifactState {
                    task_id: task_id.clone(),
                    run_id: run_id.clone(),
                    repo_id: repo_id.clone(),
                    source: source.clone(),
                    remote_path: remote_path.clone(),
                    final_path: final_path.to_string_lossy().to_string(),
                    temp_path: temp_path.to_string_lossy().to_string(),
                    expected_size: total,
                    downloaded_size: downloaded,
                    etag: save_etag.clone(),
                    last_modified: save_lm.clone(),
                    updated_at: now_secs(),
                },
                &temp_path,
            );
            update_manager_file_state(
                &shared,
                &task_id,
                FileStatePatch {
                    downloaded: Some(downloaded),
                    size: Some(total),
                    version: Some(ctx.version),
                    status: Some("error".into()),
                    error: Some(Some(format!("File create/write failed: {}", e))),
                    ..Default::default()
                },
            );
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
    let mut last_received = std::time::Instant::now();
    loop {
        if shared
            .cancel_flags
            .lock()
            .unwrap()
            .get(&run_id)
            .copied()
            .unwrap_or(false)
        {
            if !shared
                .pause_flags
                .lock()
                .unwrap()
                .get(&run_id)
                .copied()
                .unwrap_or(false)
            {
                drop(file);
                cleanup_artifact_state(&temp_path);
                update_manager_file_state(
                    &shared,
                    &task_id,
                    FileStatePatch {
                        downloaded: Some(downloaded),
                        size: Some(total),
                        version: Some(ctx.version),
                        status: Some("cancelled".into()),
                        ..Default::default()
                    },
                );
                let _ = app.emit("download-cancelled", serde_json::json!({
                    "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
                    "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                }));
            } else {
                save_artifact_state(
                    &DownloadArtifactState {
                        task_id: task_id.clone(),
                        run_id: run_id.clone(),
                        repo_id: repo_id.clone(),
                        source: source.clone(),
                        remote_path: remote_path.clone(),
                        final_path: final_path.to_string_lossy().to_string(),
                        temp_path: temp_path.to_string_lossy().to_string(),
                        expected_size: total,
                        downloaded_size: downloaded,
                        etag: save_etag.clone(),
                        last_modified: save_lm.clone(),
                        updated_at: now_secs(),
                    },
                    &temp_path,
                );
                update_manager_file_state(
                    &shared,
                    &task_id,
                    FileStatePatch {
                        downloaded: Some(downloaded),
                        size: Some(total),
                        version: Some(ctx.version),
                        status: Some("paused".into()),
                        ..Default::default()
                    },
                );
                let _ = app.emit("download-paused", serde_json::json!({
                    "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
                    "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                    "downloaded": downloaded, "total": total,
                }));
            }
            return;
        }
        let chunk = match tokio::time::timeout(std::time::Duration::from_millis(500), stream.next())
            .await
        {
            Ok(Some(chunk)) => {
                last_received = std::time::Instant::now();
                chunk
            }
            Ok(None) => break,
            Err(_) if last_received.elapsed() < DOWNLOAD_IDLE_TIMEOUT => continue,
            Err(_) => {
                let message = "Download connection was idle for 60 seconds".to_string();
                has_error.store(true, Ordering::SeqCst);
                update_manager_file_state(
                    &shared,
                    &task_id,
                    FileStatePatch {
                        downloaded: Some(downloaded),
                        size: Some(total),
                        version: Some(ctx.version),
                        status: Some("error".into()),
                        error: Some(Some(message.clone())),
                        ..Default::default()
                    },
                );
                ctx.emit(
                    &app,
                    "download-error",
                    serde_json::json!({ "error": message, "retryable": true }),
                );
                return;
            }
        };
        match chunk {
            Ok(bytes) => {
                let len = bytes.len() as u64;
                throttle_download_bytes(&shared, len).await;
                if let Err(e) = file.write_all(&bytes) {
                    save_artifact_state(
                        &DownloadArtifactState {
                            task_id: task_id.clone(),
                            run_id: run_id.clone(),
                            repo_id: repo_id.clone(),
                            source: source.clone(),
                            remote_path: remote_path.clone(),
                            final_path: final_path.to_string_lossy().to_string(),
                            temp_path: temp_path.to_string_lossy().to_string(),
                            expected_size: total,
                            downloaded_size: downloaded,
                            etag: save_etag.clone(),
                            last_modified: save_lm.clone(),
                            updated_at: now_secs(),
                        },
                        &temp_path,
                    );
                    update_manager_file_state(
                        &shared,
                        &task_id,
                        FileStatePatch {
                            downloaded: Some(downloaded),
                            size: Some(total),
                            version: Some(ctx.version),
                            status: Some("error".into()),
                            error: Some(Some(format!("File create/write failed: {}", e))),
                            ..Default::default()
                        },
                    );
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
                } else {
                    0.0
                };
                if last_emit.elapsed().as_millis() >= 500 {
                    last_emit = now;
                    update_manager_file_state(
                        &shared,
                        &task_id,
                        FileStatePatch {
                            run_id: Some(run_id.clone()),
                            downloaded: Some(downloaded),
                            size: Some(total),
                            version: Some(ctx.version),
                            status: Some("active".into()),
                            error: Some(None),
                        },
                    );
                    let _ = app.emit("download-progress", serde_json::json!({
                        "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name, "downloaded": downloaded,
                        "total": total, "speed": speed, "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
                    }));
                    if last_artifact_save.elapsed() >= std::time::Duration::from_secs(2) {
                        last_artifact_save = now;
                        save_artifact_state(
                            &DownloadArtifactState {
                                task_id: task_id.clone(),
                                run_id: run_id.clone(),
                                repo_id: repo_id.clone(),
                                source: source.clone(),
                                remote_path: remote_path.clone(),
                                final_path: final_path.to_string_lossy().to_string(),
                                temp_path: temp_path.to_string_lossy().to_string(),
                                expected_size: total,
                                downloaded_size: downloaded,
                                etag: save_etag.clone(),
                                last_modified: save_lm.clone(),
                                updated_at: now_secs(),
                            },
                            &temp_path,
                        );
                    }
                }
            }
            Err(e) => {
                save_artifact_state(
                    &DownloadArtifactState {
                        task_id: task_id.clone(),
                        run_id: run_id.clone(),
                        repo_id: repo_id.clone(),
                        source: source.clone(),
                        remote_path: remote_path.clone(),
                        final_path: final_path.to_string_lossy().to_string(),
                        temp_path: temp_path.to_string_lossy().to_string(),
                        expected_size: total,
                        downloaded_size: downloaded,
                        etag: save_etag.clone(),
                        last_modified: save_lm.clone(),
                        updated_at: now_secs(),
                    },
                    &temp_path,
                );
                update_manager_file_state(
                    &shared,
                    &task_id,
                    FileStatePatch {
                        downloaded: Some(downloaded),
                        size: Some(total),
                        version: Some(ctx.version),
                        status: Some("error".into()),
                        error: Some(Some(e.to_string())),
                        ..Default::default()
                    },
                );
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
    if let Err(error) = file.flush().and_then(|_| file.sync_all()) {
        let message = format!("Failed to flush completed download: {error}");
        has_error.store(true, Ordering::SeqCst);
        update_manager_file_state(
            &shared,
            &task_id,
            FileStatePatch {
                downloaded: Some(downloaded),
                size: Some(total),
                version: Some(ctx.version),
                status: Some("error".into()),
                error: Some(Some(message.clone())),
                ..Default::default()
            },
        );
        ctx.emit(
            &app,
            "download-error",
            serde_json::json!({ "error": message, "retryable": true }),
        );
        return;
    }
    drop(file);

    let actual_size = temp_path
        .metadata()
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let authoritative_size = if file_size > 0 { file_size } else { total };
    if authoritative_size > 0 && actual_size != authoritative_size {
        let message =
            format!("Download ended at {actual_size} bytes, expected {authoritative_size} bytes");
        has_error.store(true, Ordering::SeqCst);
        save_artifact_state(
            &DownloadArtifactState {
                task_id: task_id.clone(),
                run_id: run_id.clone(),
                repo_id: repo_id.clone(),
                source: source.clone(),
                remote_path: remote_path.clone(),
                final_path: final_path.to_string_lossy().to_string(),
                temp_path: temp_path.to_string_lossy().to_string(),
                expected_size: authoritative_size,
                downloaded_size: actual_size,
                etag: save_etag.clone(),
                last_modified: save_lm.clone(),
                updated_at: now_secs(),
            },
            &temp_path,
        );
        update_manager_file_state(
            &shared,
            &task_id,
            FileStatePatch {
                downloaded: Some(actual_size),
                size: Some(authoritative_size),
                version: Some(ctx.version),
                status: Some("error".into()),
                error: Some(Some(message.clone())),
                ..Default::default()
            },
        );
        ctx.emit(
            &app,
            "download-error",
            serde_json::json!({ "error": message, "retryable": true }),
        );
        return;
    }

    if let Err(e) = crate::persistence::replace_file(&temp_path, &final_path, None) {
        save_artifact_state(
            &DownloadArtifactState {
                task_id: task_id.clone(),
                run_id: run_id.clone(),
                repo_id: repo_id.clone(),
                source: source.clone(),
                remote_path: remote_path.clone(),
                final_path: final_path.to_string_lossy().to_string(),
                temp_path: temp_path.to_string_lossy().to_string(),
                expected_size: total,
                downloaded_size: downloaded,
                etag: save_etag.clone(),
                last_modified: save_lm.clone(),
                updated_at: now_secs(),
            },
            &temp_path,
        );
        has_error.store(true, Ordering::SeqCst);
        update_manager_file_state(
            &shared,
            &task_id,
            FileStatePatch {
                downloaded: Some(downloaded),
                size: Some(total),
                version: Some(ctx.version),
                status: Some("error".into()),
                error: Some(Some(format!("Failed to finalize download: {}", e))),
                ..Default::default()
            },
        );
        let _ = app.emit("download-error", serde_json::json!({
            "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name,
            "error": format!("Failed to finalize download: {}", e),
            "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
            "retryable": false,
        }));
        return;
    }
    cleanup_artifact_state(&temp_path);
    update_manager_file_state(
        &shared,
        &task_id,
        FileStatePatch {
            downloaded: Some(total),
            size: Some(total),
            version: Some(ctx.version),
            status: Some("completed".into()),
            error: Some(None),
            ..Default::default()
        },
    );
    let _ = app.emit("download-complete", serde_json::json!({
        "taskId": &task_id, "runId": &run_id, "version": ctx.version, "fileName": &file_name, "path": final_path.to_string_lossy(),
        "repoId": &repo_id, "source": &source, "remotePath": &remote_path,
    }));
}

// ModelScope browse.

pub async fn browse_modelscope(repo_id: String) -> Result<Vec<MsFileEntry>, String> {
    let url = format!(
        "https://www.modelscope.cn/api/v1/models/{}/repo/files?Recursive=true",
        repo_id
    );
    let resp = HTTP_CLIENT
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("{}", e))?;

    if !body
        .get("Success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let msg = body
            .get("Message")
            .and_then(|v| v.as_str())
            .unwrap_or("未知错误");
        return Err(msg.to_string());
    }

    let empty_vec = vec![];
    let files = body["Data"]["Files"].as_array().unwrap_or(&empty_vec);
    let mut result: Vec<MsFileEntry> = files
        .iter()
        .filter_map(|f| {
            if f.get("Type")?.as_str()? != "blob" {
                return None;
            }
            let name = f.get("Name")?.as_str()?.to_string();
            if !name.ends_with(".gguf") && !name.ends_with(".txt") {
                return None;
            }
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
        })
        .collect();

    result.sort_by_key(|e| match e.file_type.as_str() {
        "mmproj" => 0,
        "model" => 1,
        "imatrix" => 2,
        _ => 9,
    });
    Ok(result)
}

// ModelScope parallel download.

pub async fn download_modelscope_files(
    repo_id: String,
    files: Vec<MsFileEntry>,
    save_dir: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let repo_id = sanitize_repo_id(&repo_id)?;
    let save_path = resolve_repo_save_path(&app, &save_dir, &repo_id)?;

    let mut handles = Vec::new();
    let has_error = Arc::new(AtomicBool::new(false));
    let has_non_retryable_error = Arc::new(AtomicBool::new(false));

    for file in files {
        let encoded_path = percent_encode_path(&file.path)?;
        let url = format!(
            "https://modelscope.cn/models/{}/resolve/master/{}",
            repo_id, encoded_path
        );
        let dest_dir = remote_parent_dir(&save_path, &file.path)?;
        std::fs::create_dir_all(&dest_dir)
            .map_err(|error| format!("Failed to create remote file directory: {error}"))?;
        let ctx = build_task_context(&file, &repo_id, "modelscope");
        let has_error = Arc::clone(&has_error);
        let has_non_retryable_error = Arc::clone(&has_non_retryable_error);
        let handle = tokio::spawn({
            let app = app.clone();
            async move {
                let _slot = acquire_global_download_slot(&app).await;
                download_single_file(
                    ctx,
                    url,
                    dest_dir,
                    file.size,
                    app,
                    has_error,
                    has_non_retryable_error,
                )
                .await;
            }
        });
        handles.push(handle);
    }

    for h in handles {
        let _ = h.await;
    }
    if has_error.load(Ordering::SeqCst) {
        if has_non_retryable_error.load(Ordering::SeqCst) {
            return Err("Non-retryable download error".into());
        }
        return Err("Download completed with errors".into());
    }
    Ok(())
}

// Download controls.

pub async fn cancel_file_download(
    task_id: String,
    run_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let key = run_id.unwrap_or(task_id);
    state.cancel_flags.lock().unwrap().insert(key, true);
    Ok(())
}

pub async fn pause_file_download(
    task_id: String,
    run_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let key = run_id.unwrap_or(task_id);
    let mut cancel = state.cancel_flags.lock().unwrap();
    let mut pause = state.pause_flags.lock().unwrap();
    pause.insert(key.clone(), true);
    cancel.insert(key, true);
    Ok(())
}

pub async fn cancel_and_cleanup_download(
    task_id: String,
    file_name: String,
    file_path: String,
    run_id: Option<String>,
    version: Option<u32>,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let _ = sanitize_file_name(&file_name)?;
    let scheduler = state
        .download_scheduler_lock
        .lock()
        .map_err(|_| "download scheduler lock is poisoned".to_string())?;
    let run_id_for_match = run_id.as_deref();
    let mut entries = load_download_state(&state)?;
    entries.extend(state.download_queue.lock().unwrap().clone());
    entries.extend(
        state
            .download_active_entries
            .lock()
            .unwrap()
            .values()
            .cloned(),
    );
    let base_dir = state
        .config_dir
        .lock()
        .unwrap()
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let (root, trusted_final_path, trusted_temp_path, trusted_metadata_path) =
        trusted_download_cleanup_paths(
            &entries,
            &base_dir,
            &task_id,
            &file_name,
            run_id_for_match,
            Some(Path::new(&file_path)),
        )?;
    // Resolve the parent directory so symlinks and traversal cannot escape the managed root.
    let cpath = verified_managed_cleanup_path(&root, &trusted_final_path)?;
    let ctemp = verified_managed_cleanup_path(&root, &trusted_temp_path)?;
    let cmetadata = verified_managed_cleanup_path(&root, &trusted_metadata_path)?;
    let key = run_id.unwrap_or_else(|| task_id.clone());
    state.cancel_flags.lock().unwrap().insert(key.clone(), true);
    state.pause_flags.lock().unwrap().remove(&key);
    drop(scheduler);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while state.active_downloads.lock().unwrap().contains(&key) {
        if std::time::Instant::now() >= deadline {
            return Err("Timed out waiting for the download worker to stop before cleanup".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let _scheduler = state
        .download_scheduler_lock
        .lock()
        .map_err(|_| "download scheduler lock is poisoned".to_string())?;
    for path in [&cpath, &ctemp, &cmetadata] {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Failed to remove {}: {error}", path.display())),
        }
    }
    remove_manager_file(&state, &task_id)?;
    let _ = app.emit("download-removed", serde_json::json!({ "taskId": task_id, "fileName": file_name, "version": version.unwrap_or(0) }));
    Ok(())
}

// HuggingFace data structures and browse.

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

pub async fn browse_huggingface(repo_id: String) -> Result<Vec<MsFileEntry>, String> {
    let url = format!(
        "https://huggingface.co/api/models/{}/tree/main?recursive=true",
        repo_id
    );
    let resp = HTTP_CLIENT
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("仓库未找到 (HTTP {})", resp.status()));
    }
    let entries: Vec<HfFileEntry> = resp.json().await.map_err(|e| format!("解析失败: {}", e))?;

    let mut result: Vec<MsFileEntry> = entries
        .iter()
        .filter_map(|e| {
            if e.entry_type != "file" {
                return None;
            }
            let name = e.path.split('/').next_back()?.to_string();
            if !name.ends_with(".gguf") && !name.ends_with(".txt") {
                return None;
            }
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
        })
        .collect();

    result.sort_by_key(|e| match e.file_type.as_str() {
        "mmproj" => 0,
        "model" => 1,
        "imatrix" => 2,
        _ => 9,
    });
    Ok(result)
}

// HuggingFace download.

pub async fn download_huggingface_files(
    repo_id: String,
    files: Vec<MsFileEntry>,
    save_dir: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let repo_id = sanitize_repo_id(&repo_id)?;
    let save_path = resolve_repo_save_path(&app, &save_dir, &repo_id)?;

    let has_error = Arc::new(AtomicBool::new(false));
    let has_non_retryable_error = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::new();

    for file in files {
        let encoded_path = percent_encode_path(&file.path)?;
        let url = format!(
            "https://huggingface.co/{}/resolve/main/{}",
            repo_id, encoded_path
        );
        let dest_dir = remote_parent_dir(&save_path, &file.path)?;
        std::fs::create_dir_all(&dest_dir)
            .map_err(|error| format!("Failed to create remote file directory: {error}"))?;
        let ctx = build_task_context(&file, &repo_id, "huggingface");
        let has_error = Arc::clone(&has_error);
        let has_non_retryable_error = Arc::clone(&has_non_retryable_error);
        let handle = tokio::spawn({
            let app = app.clone();
            async move {
                let _slot = acquire_global_download_slot(&app).await;
                download_single_file(
                    ctx,
                    url,
                    dest_dir,
                    file.size,
                    app,
                    has_error,
                    has_non_retryable_error,
                )
                .await;
            }
        });
        handles.push(handle);
    }

    for h in handles {
        let _ = h.await;
    }
    if has_error.load(Ordering::SeqCst) {
        if has_non_retryable_error.load(Ordering::SeqCst) {
            return Err("Non-retryable download error".into());
        }
        return Err("Download completed with errors".into());
    }
    Ok(())
}

/// Check if local file exists and return its size.
pub async fn check_local_file(path: String) -> Result<Option<u64>, String> {
    let p = std::path::Path::new(&path);
    match std::fs::metadata(p) {
        Ok(m) if m.is_file() => Ok(Some(m.len())),
        Ok(_) => Ok(None),
        Err(_) => Ok(None),
    }
}

pub async fn delete_managed_local_file(
    file_path: String,
    save_dir: String,
    repo_id: String,
    remote_path: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let repo_id = sanitize_repo_id(&repo_id)?;
    let base_dir = state
        .config_dir
        .lock()
        .unwrap()
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let root = if Path::new(&save_dir).is_absolute() {
        PathBuf::from(&save_dir)
    } else {
        base_dir.join(&save_dir)
    };
    let repo_root = root.join(repo_id.replace('/', std::path::MAIN_SEPARATOR_STR));
    let parent = remote_parent_dir(&repo_root, &remote_path)?;
    let name = remote_path
        .split('/')
        .next_back()
        .ok_or_else(|| "Remote file path has no file name".to_string())?;
    let expected = parent.join(sanitize_file_name(name)?);
    let provided = if Path::new(&file_path).is_absolute() {
        PathBuf::from(&file_path)
    } else {
        base_dir.join(&file_path)
    };
    if normalize_path_for_compare(&expected) != normalize_path_for_compare(&provided) {
        return Err("Local file path does not match the managed repository artifact".into());
    }
    let verified = verified_managed_cleanup_path(&root, &expected)?;
    match std::fs::remove_file(&verified) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Failed to remove {}: {error}", verified.display())),
    }
}

// Download queue persistence.

use crate::models::DownloadState;

fn persist_manager_queue(state: &AppState) -> Result<(), String> {
    let queue = state.download_queue.lock().unwrap().clone();
    let queued_ids = queue
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<std::collections::HashSet<_>>();
    let queued_task_ids = queue
        .iter()
        .flat_map(|entry| entry.files.iter().filter_map(|file| file.task_id.clone()))
        .collect::<std::collections::HashSet<_>>();
    update_download_state(state, move |persisted| {
        persisted.retain(|entry| !is_runtime_queued(entry));
        persisted.retain(|entry| !queued_ids.contains(&entry.id));
        for entry in persisted.iter_mut() {
            entry.files.retain(|file| {
                !file
                    .task_id
                    .as_ref()
                    .is_some_and(|task_id| queued_task_ids.contains(task_id))
            });
            entry.status = derive_entry_status(entry);
        }
        persisted.retain(|entry| !entry.files.is_empty());
        persisted.extend(queue);
        Ok(())
    })
}

fn collect_manager_entries(state: &AppState) -> Vec<PersistedQueueEntry> {
    let mut entries = state.download_queue.lock().unwrap().clone();
    let mut positions = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.id.clone(), index))
        .collect::<std::collections::HashMap<_, _>>();
    let active_entries = state.download_active_entries.lock().unwrap();

    for entry in active_entries.values() {
        let mut active_entry = entry.clone();
        active_entry.status = "active".into();
        if let Some(index) = positions.get(&active_entry.id).copied() {
            entries[index] = active_entry;
        } else {
            positions.insert(active_entry.id.clone(), entries.len());
            entries.push(active_entry);
        }
    }

    entries
}

fn derive_entry_status(entry: &PersistedQueueEntry) -> String {
    if entry
        .files
        .iter()
        .any(|file| matches!(file.status.as_deref(), Some("active") | Some("pausing")))
    {
        return "active".into();
    }
    if entry
        .files
        .iter()
        .any(|file| matches!(file.status.as_deref(), Some("error")))
    {
        return "error".into();
    }
    if entry
        .files
        .iter()
        .any(|file| matches!(file.status.as_deref(), Some("paused")))
    {
        return "paused".into();
    }
    if !entry.files.is_empty() && entry.files.iter().all(is_terminal_download_file) {
        return if entry
            .files
            .iter()
            .all(|file| matches!(file.status.as_deref(), Some("completed")))
        {
            "completed".into()
        } else {
            "cancelled".into()
        };
    }
    entry.status.clone()
}

fn download_file_identity(entry: &PersistedQueueEntry, file: &MsFileEntry) -> String {
    let identity = format!(
        "{}|{}|{}|{}|{}",
        entry.source.trim(),
        entry.repo_id.trim(),
        entry.save_dir.trim(),
        file.path.trim(),
        file.name.trim()
    )
    .replace('\\', "/");
    if cfg!(windows) {
        identity.to_lowercase()
    } else {
        identity
    }
}

fn file_can_write(entry: &PersistedQueueEntry, file: &MsFileEntry) -> bool {
    matches!(
        file.status.as_deref().unwrap_or(entry.status.as_str()),
        "" | "queued" | "active" | "paused" | "pausing"
    )
}

fn conflicting_download_identity(
    candidate: &PersistedQueueEntry,
    existing: &[PersistedQueueEntry],
) -> Option<String> {
    let existing_identities = existing
        .iter()
        .flat_map(|entry| {
            entry
                .files
                .iter()
                .filter(|file| file_can_write(entry, file))
                .map(|file| download_file_identity(entry, file))
        })
        .collect::<std::collections::HashSet<_>>();
    candidate
        .files
        .iter()
        .map(|file| download_file_identity(candidate, file))
        .find(|identity| existing_identities.contains(identity))
}

fn persist_active_entries_snapshot(state: &AppState, force: bool) {
    if !force {
        let mut last_persist = state.download_last_inflight_persist.lock().unwrap();
        if last_persist.elapsed() < std::time::Duration::from_secs(2) {
            return;
        }
        *last_persist = std::time::Instant::now();
    }
    let _guard = state.download_inflight_lock.lock().unwrap();
    let inflight: Vec<PersistedQueueEntry> = {
        let entries = state.download_active_entries.lock().unwrap();
        entries.values().cloned().collect()
    };
    if let Err(error) = save_inflight_state_unlocked(&inflight, state) {
        eprintln!("Failed to persist active download snapshot: {error}");
    }
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
    let force_persist = !matches!(patch.status.as_deref(), Some("active"));
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
            persist_active_entries_snapshot(state, force_persist);
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
        if let Err(error) = persist_manager_queue(state) {
            eprintln!("Failed to persist download queue update: {error}");
        }
    }
    changed
}

fn remove_task_from_entries(entries: &mut Vec<PersistedQueueEntry>, task_id: &str) -> bool {
    let mut changed = false;
    entries.retain_mut(|entry| {
        let old_file_len = entry.files.len();
        entry
            .files
            .retain(|file| file.task_id.as_deref() != Some(task_id));
        if entry.files.len() != old_file_len {
            changed = true;
        }
        entry.status = derive_entry_status(entry);
        !entry.files.is_empty()
    });
    changed
}

fn remove_manager_file(state: &AppState, task_id: &str) -> Result<bool, String> {
    let (previous_persisted, persisted_changed) = update_download_state(state, |persisted| {
        let previous = persisted.clone();
        let changed = remove_task_from_entries(persisted, task_id);
        Ok((previous, changed))
    })?;

    let inflight_result = update_inflight_state(state, |inflight| {
        remove_task_from_entries(inflight, task_id)
    });
    let inflight_changed = match inflight_result {
        Ok(changed) => changed,
        Err(error) => {
            let rollback = save_download_state(&previous_persisted, state);
            return Err(match rollback {
                Ok(()) => format!("failed to remove task from inflight state: {error}"),
                Err(rollback_error) => format!(
                    "failed to remove task from inflight state: {error}; download state rollback failed: {rollback_error}"
                ),
            });
        }
    };

    let active_changed = {
        let mut active_entries = state.download_active_entries.lock().unwrap();
        let mut changed = false;
        active_entries.retain(|_, entry| {
            let old_len = entry.files.len();
            entry
                .files
                .retain(|file| file.task_id.as_deref() != Some(task_id));
            changed |= entry.files.len() != old_len;
            entry.status = derive_entry_status(entry);
            !entry.files.is_empty()
        });
        changed
    };
    let runtime_changed =
        remove_task_from_entries(&mut state.download_queue.lock().unwrap(), task_id);

    Ok(persisted_changed || inflight_changed || active_changed || runtime_changed)
}

fn cleanup_requested(
    file: &MsFileEntry,
    cancel_flags: &HashMap<String, bool>,
    pause_flags: &HashMap<String, bool>,
) -> bool {
    let key = file.run_id.as_deref().or(file.task_id.as_deref());
    key.is_some_and(|key| {
        cancel_flags.get(key).copied().unwrap_or(false)
            && !pause_flags.get(key).copied().unwrap_or(false)
    })
}

fn persist_terminal_entry(state: &AppState, mut entry: PersistedQueueEntry) -> Result<(), String> {
    {
        let cancel_flags = state.cancel_flags.lock().unwrap().clone();
        let pause_flags = state.pause_flags.lock().unwrap().clone();
        entry.files.retain(|file| {
            !matches!(
                file.status.as_deref(),
                Some("completed") | Some("cancelled")
            ) && !cleanup_requested(file, &cancel_flags, &pause_flags)
        });
    }

    let runtime_queue = state.download_queue.lock().unwrap().clone();
    update_download_state(state, move |persisted| {
        persisted.retain(|saved| {
            saved.id != entry.id && !runtime_queue.iter().any(|queued| queued.id == saved.id)
        });
        persisted.extend(runtime_queue);

        if !entry.files.is_empty() {
            entry.status = derive_entry_status(&entry);
            persisted.push(entry);
        }
        Ok(())
    })
}

fn is_runtime_queued(entry: &PersistedQueueEntry) -> bool {
    entry.status.is_empty() || entry.status == "queued"
}

fn is_restore_runnable(entry: &PersistedQueueEntry) -> bool {
    (entry.status.is_empty() || entry.status == "queued" || entry.status == "active")
        && entry.retries < entry.max_retries
}

fn is_terminal_download_file(file: &MsFileEntry) -> bool {
    matches!(file.status.as_deref(), Some("completed" | "cancelled"))
}

fn pending_download_files(files: Vec<MsFileEntry>) -> Vec<MsFileEntry> {
    files
        .into_iter()
        .filter(|file| !is_terminal_download_file(file))
        .collect()
}

fn refresh_paused_entry_for_resume(
    entry: &mut PersistedQueueEntry,
) -> Vec<ResumeDownloadTaskResult> {
    let mut identities = Vec::new();
    for file in entry.files.iter_mut() {
        if matches!(file.status.as_deref(), Some("paused" | "pausing")) {
            identities.push(refresh_download_file_identity(file));
        }
    }
    if !identities.is_empty() {
        entry.status = "queued".into();
    }
    identities
}

fn prepare_restored_entry(entry: &mut PersistedQueueEntry, auto_resume: bool) {
    normalize_crash_recovered_entry(entry);
    if auto_resume && entry.status == "paused" && entry.retries < entry.max_retries {
        let identities = refresh_paused_entry_for_resume(entry);
        if identities.is_empty()
            && entry
                .files
                .iter()
                .any(|file| file.status.as_deref() == Some("queued"))
        {
            entry.status = "queued".into();
        }
    }
}

fn retain_cancel_all_terminal_entries(entries: &mut Vec<PersistedQueueEntry>) {
    entries.retain_mut(|entry| {
        entry
            .files
            .retain(|file| matches!(file.status.as_deref(), Some("completed" | "error")));
        entry.status = derive_entry_status(entry);
        !entry.files.is_empty()
    });
}

async fn run_persisted_entry(
    entry: PersistedQueueEntry,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let files = pending_download_files(entry.files);
    if files.is_empty() {
        return Ok(());
    }
    if entry.source == "modelscope" {
        download_modelscope_files(entry.repo_id, files, entry.save_dir, app).await
    } else {
        download_huggingface_files(entry.repo_id, files, entry.save_dir, app).await
    }
}

fn process_download_queue_inner(app: tauri::AppHandle) -> bool {
    let state = app.state::<AppState>();
    if state.download_shutting_down.load(Ordering::SeqCst) {
        return false;
    }
    {
        let max = effective_download_concurrency(&state);
        if active_download_slot_count(&state) >= max {
            return false;
        }
    }

    let entry = {
        let mut queue = state.download_queue.lock().unwrap();
        // Drop non-runnable entries once so they cannot block later queue items.
        let old_len = queue.len();
        queue.retain(is_restore_runnable);
        if queue.is_empty() {
            if queue.len() != old_len {
                drop(queue);
                if let Err(error) = persist_manager_queue(&state) {
                    eprintln!("Failed to persist empty download queue: {error}");
                }
            }
            return false;
        }
        let queued_entry = queue.remove(0);
        let mut entry = queued_entry.clone();
        for file in entry.files.iter_mut() {
            if !is_terminal_download_file(file) {
                file.status = Some("active".into());
                file.error = None;
            }
        }
        entry.status = "active".into();
        let inflight_entry = entry.clone();
        if let Err(error) = update_inflight_state(&state, |inflight| {
            inflight.retain(|saved| saved.id != entry.id);
            inflight.push(inflight_entry.clone());
        }) {
            queue.insert(0, queued_entry);
            eprintln!("Failed to hand off download entry to inflight state: {error}");
            return false;
        }
        state
            .download_active_entries
            .lock()
            .unwrap()
            .insert(entry.id.clone(), inflight_entry);
        drop(queue);
        if let Err(error) = persist_manager_queue(&state) {
            state
                .download_active_entries
                .lock()
                .unwrap()
                .remove(&entry.id);
            state.download_queue.lock().unwrap().insert(0, queued_entry);
            if let Err(rollback_error) = update_inflight_state(&state, |inflight| {
                inflight.retain(|saved| saved.id != entry.id);
            }) {
                eprintln!("Failed to roll back inflight download state: {rollback_error}");
            }
            eprintln!("Failed to persist dequeued download entry: {error}");
            return false;
        }
        entry
    };

    {
        let mut active = state.download_active_batches.lock().unwrap();
        active.insert(entry.id.clone());
    }

    for file in entry
        .files
        .iter()
        .filter(|file| !is_terminal_download_file(file))
    {
        let _ = app.emit(
            "download-started",
            serde_json::json!({
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
            }),
        );
    }

    tauri::async_runtime::spawn(async move {
        let batch_id = entry.id.clone();
        let entry_for_retry = entry.clone();
        let result = run_persisted_entry(entry, app.clone()).await;
        let retryable_result = result
            .as_ref()
            .err()
            .is_some_and(|error| !error.starts_with("Non-retryable"));
        if result.is_err() {
            let shutting_down = app
                .state::<AppState>()
                .download_shutting_down
                .load(Ordering::SeqCst);
            if retryable_result
                && !shutting_down
                && entry_for_retry.retries < entry_for_retry.max_retries
            {
                let delay_ms = 2000u64 * 2u64.pow(entry_for_retry.retries.min(5));
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

                let state = app.state::<AppState>();
                if state.download_shutting_down.load(Ordering::SeqCst) {
                    // Shutdown persists the latest active snapshot below instead of requeueing it.
                } else {
                    let scheduler = state.download_scheduler_lock.lock().unwrap();
                    let live_entry = state
                        .download_active_entries
                        .lock()
                        .unwrap()
                        .get(&batch_id)
                        .cloned();
                    let Some(mut retry_entry) = live_entry else {
                        state
                            .download_active_batches
                            .lock()
                            .unwrap()
                            .remove(&batch_id);
                        drop(scheduler);
                        fill_download_queue_slots(app);
                        return;
                    };
                    retry_entry.retries += 1;
                    retry_entry.status = "queued".into();
                    for file in retry_entry.files.iter_mut() {
                        if matches!(
                            file.status.as_deref(),
                            Some("error") | Some("paused") | Some("active")
                        ) {
                            file.status = Some("queued".into());
                            file.error = None;
                        }
                    }

                    if retry_entry
                        .files
                        .iter()
                        .any(|file| !is_terminal_download_file(file))
                    {
                        state
                            .download_active_batches
                            .lock()
                            .unwrap()
                            .remove(&batch_id);
                        state
                            .download_active_entries
                            .lock()
                            .unwrap()
                            .remove(&batch_id);
                        state.download_queue.lock().unwrap().insert(0, retry_entry);
                        match persist_manager_queue(&state) {
                            Ok(()) => {
                                if let Err(error) = update_inflight_state(&state, |inflight| {
                                    inflight.retain(|entry| entry.id != batch_id);
                                }) {
                                    eprintln!(
                                        "Failed to clear inflight state before retry: {error}"
                                    );
                                }
                                drop(scheduler);
                                fill_download_queue_slots(app);
                                return;
                            }
                            Err(error) => {
                                state
                                    .download_queue
                                    .lock()
                                    .unwrap()
                                    .retain(|entry| entry.id != batch_id);
                                eprintln!("Failed to persist download retry: {error}");
                            }
                        }
                    }
                    drop(scheduler);
                }
            }
        }

        {
            let state = app.state::<AppState>();
            let latest_entry = state
                .download_active_entries
                .lock()
                .unwrap()
                .get(&batch_id)
                .cloned();
            let terminal_persisted = if let Some(entry) = latest_entry {
                match persist_terminal_entry(&state, entry) {
                    Ok(()) => true,
                    Err(error) => {
                        // Keep the inflight snapshot when the terminal state is not durable.
                        eprintln!("Failed to persist terminal download entry: {error}");
                        false
                    }
                }
            } else {
                false
            };
            state
                .download_active_batches
                .lock()
                .unwrap()
                .remove(&batch_id);
            state
                .download_active_entries
                .lock()
                .unwrap()
                .remove(&batch_id);
            if terminal_persisted {
                if let Err(error) = update_inflight_state(&state, |inflight| {
                    inflight.retain(|entry| entry.id != batch_id);
                }) {
                    eprintln!("Failed to clear completed inflight download state: {error}");
                }
            }
        }
        fill_download_queue_slots(app);
    });
    true
}

fn fill_download_queue_slots(app: tauri::AppHandle) {
    let scheduler_app = app.clone();
    let scheduler_state = scheduler_app.state::<AppState>();
    let _scheduler = scheduler_state.download_scheduler_lock.lock().unwrap();
    if scheduler_state
        .download_shutting_down
        .load(Ordering::SeqCst)
    {
        return;
    }
    while process_download_queue_inner(app.clone()) {}
}

fn save_download_state_unlocked(
    queue: &[PersistedQueueEntry],
    state: &AppState,
) -> Result<(), String> {
    let path = download_state_path(state);
    let ds = DownloadState {
        queue: queue.to_vec(),
    };
    let json = serde_json::to_vec_pretty(&ds)
        .map_err(|error| format!("failed to serialize download state: {error}"))?;
    crate::persistence::atomic_write(&path, &json, None)
}

fn load_download_state_unlocked(state: &AppState) -> Result<Vec<PersistedQueueEntry>, String> {
    let path = download_state_path(state);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let json = std::fs::read_to_string(&path)
        .map_err(|error| format!("failed to read download state: {error}"))?;
    serde_json::from_str::<DownloadState>(&json)
        .map(|state| state.queue)
        .map_err(|error| format!("failed to parse download state: {error}"))
}

fn download_state_path(state: &AppState) -> PathBuf {
    state.config_dir.lock().unwrap().join("downloads.json")
}

fn quarantine_corrupt_state(path: &Path, file_prefix: &str) -> Result<PathBuf, String> {
    let quarantine = path.with_file_name(format!(
        "{file_prefix}.corrupt-{}.json",
        uuid::Uuid::new_v4()
    ));
    std::fs::rename(path, &quarantine)
        .map_err(|error| format!("failed to preserve corrupt download state: {error}"))?;
    Ok(quarantine)
}

pub(crate) fn save_download_state(
    queue: &[PersistedQueueEntry],
    state: &AppState,
) -> Result<(), String> {
    let _guard = DOWNLOAD_STATE_LOCK
        .lock()
        .map_err(|_| "download state lock is poisoned".to_string())?;
    save_download_state_unlocked(queue, state)
}

pub(crate) fn load_download_state(state: &AppState) -> Result<Vec<PersistedQueueEntry>, String> {
    let _guard = DOWNLOAD_STATE_LOCK
        .lock()
        .map_err(|_| "download state lock is poisoned".to_string())?;
    load_download_state_unlocked(state)
}

fn update_download_state<R, F>(state: &AppState, update: F) -> Result<R, String>
where
    F: FnOnce(&mut Vec<PersistedQueueEntry>) -> Result<R, String>,
{
    let _guard = DOWNLOAD_STATE_LOCK
        .lock()
        .map_err(|_| "download state lock is poisoned".to_string())?;
    let mut queue = load_download_state_unlocked(state)?;
    let result = update(&mut queue)?;
    save_download_state_unlocked(&queue, state)?;
    Ok(result)
}

fn inflight_path(state: &AppState) -> PathBuf {
    state
        .config_dir
        .lock()
        .unwrap()
        .join("downloads_inflight.json")
}

fn save_inflight_state_unlocked(
    inflight: &[PersistedQueueEntry],
    state: &AppState,
) -> Result<(), String> {
    let path = inflight_path(state);
    if inflight.is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("failed to remove inflight download state: {error}")),
        };
    }
    let ds = DownloadState {
        queue: inflight.to_vec(),
    };
    serde_json::to_string_pretty(&ds)
        .map_err(|error| format!("failed to serialize inflight download state: {error}"))
        .and_then(|json| write_string_atomic(&path, &json))
}

fn load_inflight_state_unlocked(state: &AppState) -> Result<Vec<PersistedQueueEntry>, String> {
    let path = inflight_path(state);
    let json = match std::fs::read_to_string(&path) {
        Ok(json) => json,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("failed to read inflight download state: {error}")),
    };
    serde_json::from_str::<DownloadState>(&json)
        .map(|state| state.queue)
        .map_err(|error| format!("failed to parse inflight download state: {error}"))
}

fn update_inflight_state<R, F>(state: &AppState, update: F) -> Result<R, String>
where
    F: FnOnce(&mut Vec<PersistedQueueEntry>) -> R,
{
    let _guard = state.download_inflight_lock.lock().unwrap();
    let mut inflight = load_inflight_state_unlocked(state)?;
    let result = update(&mut inflight);
    save_inflight_state_unlocked(&inflight, state)?;
    Ok(result)
}

fn clear_inflight_state(state: &AppState) -> Result<(), String> {
    let _guard = state.download_inflight_lock.lock().unwrap();
    save_inflight_state_unlocked(&[], state)
}

fn merge_crash_recovered_inflight(
    queue: &mut Vec<PersistedQueueEntry>,
    inflight: Vec<PersistedQueueEntry>,
) {
    let mut positions = queue
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.id.clone(), index))
        .collect::<std::collections::HashMap<_, _>>();
    for mut entry in inflight {
        normalize_crash_recovered_entry(&mut entry);
        if let Some(index) = positions.get(&entry.id).copied() {
            queue[index] = entry;
        } else {
            positions.insert(entry.id.clone(), queue.len());
            queue.push(entry);
        }
    }
}

pub(crate) fn restore_runtime_queue_from_disk(
    state: &AppState,
    app: &tauri::AppHandle,
) -> Vec<PersistedQueueEntry> {
    let (mut queue, can_persist_restored_state) = {
        let state_lock = DOWNLOAD_STATE_LOCK.lock();
        match state_lock {
            Ok(_guard) => match load_download_state_unlocked(state) {
                Ok(queue) => (queue, true),
                Err(error) => {
                    eprintln!("Failed to restore download queue: {error}");
                    match quarantine_corrupt_state(&download_state_path(state), "downloads") {
                        Ok(path) => {
                            eprintln!("Preserved corrupt download queue at {}", path.display());
                            (Vec::new(), true)
                        }
                        Err(quarantine_error) => {
                            eprintln!(
                                "Failed to quarantine corrupt download queue: {quarantine_error}"
                            );
                            (Vec::new(), false)
                        }
                    }
                }
            },
            Err(_) => {
                eprintln!("Failed to restore download queue: download state lock is poisoned");
                (Vec::new(), false)
            }
        }
    };

    state.download_shutting_down.store(false, Ordering::SeqCst);
    let inflight = {
        let _guard = state.download_inflight_lock.lock().unwrap();
        match load_inflight_state_unlocked(state) {
            Ok(inflight) => inflight,
            Err(error) => {
                eprintln!("Failed to restore inflight download queue: {error}");
                match quarantine_corrupt_state(&inflight_path(state), "downloads_inflight") {
                    Ok(path) => {
                        eprintln!("Preserved corrupt inflight queue at {}", path.display())
                    }
                    Err(quarantine_error) => {
                        eprintln!("Failed to quarantine corrupt inflight queue: {quarantine_error}")
                    }
                }
                Vec::new()
            }
        }
    };
    let had_inflight = !inflight.is_empty();
    if had_inflight {
        merge_crash_recovered_inflight(&mut queue, inflight);
    }

    let config_dir = state.config_dir.lock().unwrap().clone();
    let config = crate::commands::config::read_config_from_disk(&config_dir);
    let save_dir_base = config_dir.parent().unwrap_or(Path::new(".")).to_path_buf();
    let auto_resume = config.download_resume_policy == "auto_on_launch";

    for entry in queue.iter_mut() {
        prepare_restored_entry(entry, auto_resume);
        let repo_dir = save_dir_base
            .join(&entry.save_dir)
            .join(entry.repo_id.replace('/', std::path::MAIN_SEPARATOR_STR));
        for file in entry.files.iter_mut() {
            let file_dir =
                remote_parent_dir(&repo_dir, &file.path).unwrap_or_else(|_| repo_dir.clone());
            let (final_path, temp_path, _) = build_download_paths(&file_dir, &file.name);
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

    let runnable: Vec<_> = queue
        .iter()
        .filter(|e| is_restore_runnable(e))
        .cloned()
        .collect();
    *state.download_queue.lock().unwrap() = runnable;
    match can_persist_restored_state
        .then(|| save_download_state(&queue, state))
        .transpose()
    {
        Ok(Some(())) if had_inflight => {
            if let Err(error) = clear_inflight_state(state) {
                eprintln!("Failed to clear restored inflight download queue: {error}");
            }
        }
        Ok(Some(())) => {}
        Ok(None) => {}
        Err(error) => {
            eprintln!("Failed to persist restored download queue: {error}");
        }
    }

    if auto_resume {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            fill_download_queue_slots(app);
        });
    }

    queue
}

pub async fn persist_download_queue(
    queue: Vec<PersistedQueueEntry>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _scheduler = state
        .download_scheduler_lock
        .lock()
        .map_err(|_| "download scheduler lock is poisoned".to_string())?;
    let runtime_queue: Vec<PersistedQueueEntry> = queue
        .iter()
        .filter(|e| is_runtime_queued(e))
        .cloned()
        .collect();
    let previous_runtime = {
        let mut stored = state.download_queue.lock().unwrap();
        std::mem::replace(&mut *stored, runtime_queue.clone())
    };

    let manager_entries = collect_manager_entries(&state);
    let runtime_ids = runtime_queue
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<std::collections::HashSet<_>>();
    let result = update_download_state(&state, move |persisted| {
        persisted.retain(|entry| !is_runtime_queued(entry));
        let mut persisted_positions = persisted
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.id.clone(), index))
            .collect::<std::collections::HashMap<_, _>>();

        for entry in manager_entries
            .into_iter()
            .filter(|entry| !is_runtime_queued(entry))
        {
            if let Some(index) = persisted_positions.get(&entry.id).copied() {
                persisted[index] = entry;
            } else {
                persisted_positions.insert(entry.id.clone(), persisted.len());
                persisted.push(entry);
            }
        }
        persisted.retain(|entry| !runtime_ids.contains(&entry.id));
        persisted.extend(runtime_queue);
        Ok(())
    });
    if let Err(error) = result {
        *state.download_queue.lock().unwrap() = previous_runtime;
        return Err(error);
    }
    Ok(())
}

pub async fn restore_download_queue(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Vec<PersistedQueueEntry>, String> {
    Ok(restore_runtime_queue_from_disk(&state, &app))
}

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
    let scheduler = state
        .download_scheduler_lock
        .lock()
        .map_err(|_| "download scheduler lock is poisoned".to_string())?;
    let mut existing = load_download_state(&state)?;
    existing.extend(state.download_queue.lock().unwrap().clone());
    existing.extend(
        state
            .download_active_entries
            .lock()
            .unwrap()
            .values()
            .cloned(),
    );
    if let Some(identity) = conflicting_download_identity(&entry, &existing) {
        return Err(format!(
            "A queued or active download already owns this destination: {identity}"
        ));
    }
    let entry_id = entry.id.clone();
    {
        let mut queue = state.download_queue.lock().unwrap();
        if queue.iter().any(|queued| queued.id == entry_id) {
            return Ok(());
        }
        if is_runtime_queued(&entry) {
            queue.push(entry);
        }
    }
    if let Err(error) = persist_manager_queue(&state) {
        state
            .download_queue
            .lock()
            .unwrap()
            .retain(|queued| queued.id != entry_id);
        return Err(error);
    }
    drop(scheduler);
    fill_download_queue_slots(app);
    Ok(())
}

pub async fn remove_download_queue_entry(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _scheduler = state
        .download_scheduler_lock
        .lock()
        .map_err(|_| "download scheduler lock is poisoned".to_string())?;
    let previous_runtime = {
        let mut queue = state.download_queue.lock().unwrap();
        let previous = queue.clone();
        queue.retain(|entry| entry.id != id);
        previous
    };
    if let Err(error) = persist_manager_queue(&state) {
        *state.download_queue.lock().unwrap() = previous_runtime;
        return Err(error);
    }
    Ok(())
}

pub async fn clear_download_tasks_by_status(
    statuses: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _scheduler = state
        .download_scheduler_lock
        .lock()
        .map_err(|_| "download scheduler lock is poisoned".to_string())?;
    let status_set: std::collections::HashSet<String> = statuses.into_iter().collect();
    let previous_persisted = load_download_state(&state)?;
    let previous_runtime = state.download_queue.lock().unwrap().clone();
    {
        let mut queue = state.download_queue.lock().unwrap();
        queue.retain_mut(|entry| {
            entry.files.retain(|file| {
                !file
                    .status
                    .as_ref()
                    .map(|s| status_set.contains(s))
                    .unwrap_or(false)
            });
            entry.status = derive_entry_status(entry);
            !entry.files.is_empty()
        });
    }

    let runtime_queue = state.download_queue.lock().unwrap().clone();
    let persisted_statuses = status_set.clone();
    let result = update_download_state(&state, move |persisted| {
        persisted.retain_mut(|entry| {
            entry.files.retain(|file| {
                !file
                    .status
                    .as_ref()
                    .map(|status| persisted_statuses.contains(status))
                    .unwrap_or(false)
            });
            entry.status = derive_entry_status(entry);
            !entry.files.is_empty()
        });
        persisted.retain(|entry| !runtime_queue.iter().any(|queued| queued.id == entry.id));
        persisted.extend(runtime_queue);
        Ok(())
    });
    if let Err(error) = result {
        *state.download_queue.lock().unwrap() = previous_runtime;
        return Err(error);
    }

    let inflight_statuses = status_set.clone();
    if let Err(error) = update_inflight_state(&state, |inflight| {
        inflight.retain_mut(|entry| {
            entry.files.retain(|file| {
                !file
                    .status
                    .as_ref()
                    .is_some_and(|status| inflight_statuses.contains(status))
            });
            entry.status = derive_entry_status(entry);
            !entry.files.is_empty()
        });
    }) {
        let rollback = save_download_state(&previous_persisted, &state);
        *state.download_queue.lock().unwrap() = previous_runtime;
        return Err(match rollback {
            Ok(()) => format!("failed to clear inflight download tasks: {error}"),
            Err(rollback_error) => format!(
                "failed to clear inflight download tasks: {error}; download state rollback failed: {rollback_error}"
            ),
        });
    }

    let removed_active_entries = {
        let mut active_entries = state.download_active_entries.lock().unwrap();
        let mut removed = Vec::new();
        active_entries.retain(|entry_id, entry| {
            entry.files.retain(|file| {
                !file
                    .status
                    .as_ref()
                    .is_some_and(|status| status_set.contains(status))
            });
            entry.status = derive_entry_status(entry);
            if entry.files.is_empty() {
                removed.push(entry_id.clone());
                false
            } else {
                true
            }
        });
        removed
    };
    if !removed_active_entries.is_empty() {
        let mut active_batches = state.download_active_batches.lock().unwrap();
        for entry_id in removed_active_entries {
            active_batches.remove(&entry_id);
        }
    }
    Ok(())
}

pub async fn process_download_queue(app: tauri::AppHandle) -> Result<(), String> {
    fill_download_queue_slots(app);
    Ok(())
}

pub async fn get_download_resume_policy(
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let config = crate::commands::config::read_config_from_disk(&config_dir);
    Ok(config.download_resume_policy)
}

pub async fn set_download_resume_policy(
    policy: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if policy != "manual" && policy != "auto_on_launch" {
        return Err("Invalid policy. Must be 'manual' or 'auto_on_launch'".into());
    }
    crate::commands::config::update_and_persist(&state, |global| {
        global.download_resume_policy = policy;
    })?;
    Ok(())
}

pub async fn resume_download_task(
    task_id: String,
    app: tauri::AppHandle,
) -> Result<ResumeDownloadTaskResult, String> {
    let state = app.state::<AppState>();
    let scheduler = state
        .download_scheduler_lock
        .lock()
        .map_err(|_| "download scheduler lock is poisoned".to_string())?;

    if state
        .download_active_entries
        .lock()
        .unwrap()
        .values()
        .any(|entry| {
            entry
                .files
                .iter()
                .any(|file| file.task_id.as_deref() == Some(task_id.as_str()))
        })
    {
        return Err("Download task is already active".into());
    }

    if let Some(existing) = state
        .download_queue
        .lock()
        .unwrap()
        .iter()
        .find_map(|entry| {
            entry
                .files
                .iter()
                .find(|file| file.task_id.as_deref() == Some(task_id.as_str()))
        })
    {
        return Ok(ResumeDownloadTaskResult {
            task_id: task_id.clone(),
            run_id: existing.run_id.clone().unwrap_or_default(),
            version: existing.version.unwrap_or(0),
        });
    }

    let previous_persisted = load_download_state(&state)?;
    let task_id_for_lookup = task_id.clone();
    let (mut file, target_meta) = update_download_state(&state, move |persisted| {
        let mut target_file = None;
        let mut target_meta = None;
        persisted.retain_mut(|entry| {
            if let Some(position) = entry
                .files
                .iter()
                .position(|file| file.task_id.as_deref() == Some(task_id_for_lookup.as_str()))
            {
                if target_file.is_none() {
                    target_file = Some(entry.files.remove(position));
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
        Ok((
            target_file.ok_or_else(|| "Download task not found".to_string())?,
            target_meta.ok_or_else(|| "Download task metadata is missing".to_string())?,
        ))
    })?;
    clear_control_flags_for_files(&state, &[file.clone()]);

    let (repo_id, source, save_dir, retries, max_retries) = target_meta;
    let identity = refresh_download_file_identity(&mut file);

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
    let runtime_entry_id = runtime_entry.id.clone();

    {
        let mut runtime = state.download_queue.lock().unwrap();
        runtime.push(runtime_entry);
    }
    if let Err(error) = persist_manager_queue(&state) {
        state
            .download_queue
            .lock()
            .unwrap()
            .retain(|entry| entry.id != runtime_entry_id);
        let restore_result = save_download_state(&previous_persisted, &state);
        return Err(match restore_result {
            Ok(()) => error,
            Err(restore_error) => {
                format!("{error}; failed to restore paused download state: {restore_error}")
            }
        });
    }

    drop(scheduler);
    fill_download_queue_slots(app);

    Ok(ResumeDownloadTaskResult {
        task_id,
        run_id: identity.run_id,
        version: identity.version,
    })
}

pub async fn resume_all_downloads(
    app: tauri::AppHandle,
) -> Result<Vec<ResumeDownloadTaskResult>, String> {
    let state = app.state::<AppState>();
    let scheduler = state
        .download_scheduler_lock
        .lock()
        .map_err(|_| "download scheduler lock is poisoned".to_string())?;
    let queue = load_download_state(&state)?;
    let mut runtime = state.download_queue.lock().unwrap();
    let previous_runtime = runtime.clone();
    let mut identities = Vec::new();
    for entry in &queue {
        if entry.status == "paused" && !runtime.iter().any(|e| e.id == entry.id) {
            let mut entry = entry.clone();
            let resumed = refresh_paused_entry_for_resume(&mut entry);
            if !resumed.is_empty() {
                identities.extend(resumed);
                runtime.push(entry);
            }
        }
    }
    drop(runtime);
    if let Err(error) = persist_manager_queue(&state) {
        *state.download_queue.lock().unwrap() = previous_runtime;
        return Err(error);
    }
    drop(scheduler);
    fill_download_queue_slots(app);
    Ok(identities)
}

pub fn flush_download_manager_state(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    state.download_shutting_down.store(true, Ordering::SeqCst);
    let scheduler = state.download_scheduler_lock.lock().unwrap();

    {
        let mut cancel = state.cancel_flags.lock().unwrap();
        let mut pause = state.pause_flags.lock().unwrap();
        let active = state.active_downloads.lock().unwrap();
        for run_id in active.iter() {
            cancel.insert(run_id.clone(), true);
            pause.insert(run_id.clone(), true);
        }
    }

    drop(scheduler);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !state.download_active_batches.lock().unwrap().is_empty()
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let _scheduler = state.download_scheduler_lock.lock().unwrap();

    let active_entries: Vec<PersistedQueueEntry> = state
        .download_active_entries
        .lock()
        .unwrap()
        .values()
        .cloned()
        .collect();
    let mut queue = state.download_queue.lock().unwrap().clone();
    for mut entry in active_entries {
        entry.status = "paused".to_string();
        for file in entry.files.iter_mut() {
            if !matches!(file.status.as_deref(), Some("completed" | "cancelled")) {
                file.status = Some("paused".into());
            }
        }
        if let Some(existing) = queue.iter_mut().find(|saved| saved.id == entry.id) {
            *existing = entry;
        } else {
            queue.push(entry);
        }
    }
    match update_download_state(&state, move |persisted| {
        persisted.retain(|entry| !queue.iter().any(|queued| queued.id == entry.id));
        persisted.extend(queue);
        Ok(())
    }) {
        Ok(()) => {
            if let Err(error) = clear_inflight_state(&state) {
                eprintln!("Failed to clear inflight state during shutdown: {error}");
            }
        }
        Err(error) => eprintln!("Failed to flush download manager state: {error}"),
    }
}

#[cfg(test)]
mod audit_remediation_tests {
    use super::*;

    #[test]
    fn crash_recovered_inflight_entry_normalizes_active_file_statuses() {
        let mut entry = PersistedQueueEntry {
            id: "entry-1".into(),
            repo_id: "repo/model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: "active".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![MsFileEntry {
                name: "model.gguf".into(),
                path: "model.gguf".into(),
                size: 100,
                file_type: "file".into(),
                downloaded: Some(12),
                status: Some("active".into()),
                error: None,
                task_id: Some("task-1".into()),
                run_id: Some("run-1".into()),
                version: Some(1),
            }],
        };

        normalize_crash_recovered_entry(&mut entry);

        assert_eq!(entry.status, "paused");
        assert_eq!(entry.files[0].status.as_deref(), Some("paused"));
        assert_eq!(entry.files[0].error, None);
    }

    #[test]
    fn manual_restore_pauses_active_entries_and_excludes_them_from_runtime_queue() {
        let mut entry = PersistedQueueEntry {
            id: "manual-active".into(),
            repo_id: "repo/model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: "active".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![MsFileEntry {
                name: "model.gguf".into(),
                path: "model.gguf".into(),
                size: 100,
                file_type: "file".into(),
                downloaded: Some(12),
                status: None,
                error: None,
                task_id: Some("task-manual".into()),
                run_id: Some("run-manual".into()),
                version: Some(3),
            }],
        };

        prepare_restored_entry(&mut entry, false);

        assert_eq!(entry.status, "paused");
        assert_eq!(entry.files[0].status.as_deref(), Some("paused"));
        assert!(!is_restore_runnable(&entry));
        assert_eq!(entry.files[0].run_id.as_deref(), Some("run-manual"));
        assert_eq!(entry.files[0].version, Some(3));
    }

    #[test]
    fn auto_on_launch_requeues_paused_entries_with_a_fresh_run_identity() {
        let mut entry = PersistedQueueEntry {
            id: "auto-paused".into(),
            repo_id: "repo/model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: "paused".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![MsFileEntry {
                name: "model.gguf".into(),
                path: "model.gguf".into(),
                size: 100,
                file_type: "file".into(),
                downloaded: Some(12),
                status: Some("paused".into()),
                error: Some("interrupted".into()),
                task_id: Some("task-auto".into()),
                run_id: Some("run-auto".into()),
                version: Some(3),
            }],
        };

        prepare_restored_entry(&mut entry, true);

        assert_eq!(entry.status, "queued");
        assert_eq!(entry.files[0].status.as_deref(), Some("queued"));
        assert!(is_restore_runnable(&entry));
        assert_ne!(entry.files[0].run_id.as_deref(), Some("run-auto"));
        assert_eq!(entry.files[0].version, Some(4));
        assert_eq!(entry.files[0].error, None);
    }

    #[test]
    fn auto_on_launch_leaves_retry_exhausted_entries_paused() {
        let mut entry = PersistedQueueEntry {
            id: "auto-exhausted".into(),
            repo_id: "repo/model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: "active".into(),
            retries: 3,
            max_retries: 3,
            last_error: Some("retry limit reached".into()),
            files: vec![MsFileEntry {
                name: "model.gguf".into(),
                path: "model.gguf".into(),
                size: 100,
                file_type: "file".into(),
                downloaded: Some(12),
                status: Some("active".into()),
                error: Some("retry limit reached".into()),
                task_id: Some("task-exhausted".into()),
                run_id: Some("run-exhausted".into()),
                version: Some(3),
            }],
        };

        prepare_restored_entry(&mut entry, true);

        assert_eq!(entry.status, "paused");
        assert_eq!(entry.files[0].status.as_deref(), Some("paused"));
        assert!(!is_restore_runnable(&entry));
        assert_eq!(entry.files[0].run_id.as_deref(), Some("run-exhausted"));
        assert_eq!(entry.files[0].version, Some(3));
    }

    #[test]
    fn auto_on_launch_requeues_legacy_active_entry_with_a_queued_file() {
        let mut entry = PersistedQueueEntry {
            id: "auto-legacy-queued".into(),
            repo_id: "repo/model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: "active".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![MsFileEntry {
                name: "model.gguf".into(),
                path: "model.gguf".into(),
                size: 100,
                file_type: "file".into(),
                downloaded: None,
                status: Some("queued".into()),
                error: None,
                task_id: Some("task-legacy".into()),
                run_id: Some("run-legacy".into()),
                version: Some(1),
            }],
        };

        prepare_restored_entry(&mut entry, true);

        assert_eq!(entry.status, "queued");
        assert_eq!(entry.files[0].status.as_deref(), Some("queued"));
        assert!(is_restore_runnable(&entry));
        assert_eq!(entry.files[0].run_id.as_deref(), Some("run-legacy"));
        assert_eq!(entry.files[0].version, Some(1));
    }

    #[test]
    fn refresh_download_file_identity_increments_version_and_clears_error() {
        let mut file = MsFileEntry {
            name: "model.gguf".into(),
            path: "model.gguf".into(),
            size: 100,
            file_type: "file".into(),
            downloaded: Some(12),
            status: Some("paused".into()),
            error: Some("old".into()),
            task_id: Some("task-1".into()),
            run_id: Some("run-1".into()),
            version: Some(7),
        };

        let result = refresh_download_file_identity(&mut file);

        assert_eq!(result.task_id, "task-1");
        assert_eq!(result.version, 8);
        assert_ne!(result.run_id, "run-1");
        assert_eq!(file.status.as_deref(), Some("queued"));
        assert_eq!(file.error, None);
        assert_eq!(file.version, Some(8));
        assert_eq!(file.run_id.as_deref(), Some(result.run_id.as_str()));
    }

    #[test]
    fn trusted_download_cleanup_path_rejects_frontend_path_outside_entry_directory() {
        let entry = PersistedQueueEntry {
            id: "entry-1".into(),
            repo_id: "repo/model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: "paused".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![MsFileEntry {
                name: "model.gguf".into(),
                path: "model.gguf".into(),
                size: 100,
                file_type: "file".into(),
                downloaded: None,
                status: Some("paused".into()),
                error: None,
                task_id: Some("task-1".into()),
                run_id: Some("run-1".into()),
                version: Some(1),
            }],
        };
        let base = Path::new("/app-data");
        let forged = Path::new("/app-data/configs/instances.json");

        let err = trusted_download_cleanup_paths(
            &[entry],
            base,
            "task-1",
            "model.gguf",
            Some("run-1"),
            Some(forged),
        )
        .unwrap_err();

        assert!(err.contains("does not match"));
    }

    #[test]
    fn redownload_cleanup_path_includes_repository_directory() {
        let config_dir = Path::new("/app-data/configs");

        let path = resolve_redownload_save_path(config_dir, "models", "org/model").unwrap();

        assert_eq!(
            path,
            Path::new("/app-data")
                .join("models")
                .join("org")
                .join("model")
        );
    }

    #[test]
    fn repository_id_rejects_rooted_and_empty_path_segments() {
        assert!(sanitize_repo_id("/org/model").is_err());
        assert!(sanitize_repo_id("org/model/").is_err());
        assert!(sanitize_repo_id("org//model").is_err());
        assert!(sanitize_repo_id("org/model").is_ok());
    }

    #[test]
    fn remote_paths_preserve_safe_directories_and_encode_url_segments() {
        let root = Path::new("/managed/org/model");
        assert_eq!(
            remote_parent_dir(root, "weights/sub dir/model #1.gguf").unwrap(),
            root.join("weights").join("sub dir")
        );
        assert_eq!(
            percent_encode_path("weights/sub dir/model #1?.gguf").unwrap(),
            "weights/sub%20dir/model%20%231%3F.gguf"
        );
        for unsafe_path in [
            "../model.gguf",
            "weights//model.gguf",
            "/rooted.gguf",
            "a\\b.gguf",
        ] {
            assert!(remote_parent_dir(root, unsafe_path).is_err());
        }
    }

    #[test]
    fn relative_frontend_cleanup_path_is_resolved_from_the_managed_base() {
        let entry = PersistedQueueEntry {
            id: "entry-relative".into(),
            repo_id: "org/model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: "paused".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![MsFileEntry {
                name: "model.gguf".into(),
                path: "weights/model.gguf".into(),
                size: 100,
                file_type: "model".into(),
                downloaded: Some(25),
                status: Some("paused".into()),
                error: None,
                task_id: Some("relative-task".into()),
                run_id: Some("relative-run".into()),
                version: Some(1),
            }],
        };

        let (_, final_path, _, _) = trusted_download_cleanup_paths(
            &[entry],
            Path::new("/app-data"),
            "relative-task",
            "model.gguf",
            Some("relative-run"),
            Some(Path::new("models/org/model/weights/model.gguf")),
        )
        .unwrap();

        assert_eq!(
            normalize_path_for_compare(&final_path),
            normalize_path_for_compare(Path::new("/app-data/models/org/model/weights/model.gguf"))
        );
    }

    #[test]
    fn queue_entry_directory_rejects_save_directory_parent_traversal() {
        let entry = PersistedQueueEntry {
            id: "entry-1".into(),
            repo_id: "org/model".into(),
            source: "huggingface".into(),
            save_dir: "../outside".into(),
            added_at: 1,
            status: "paused".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: Vec::new(),
        };

        assert!(queue_entry_download_dir(Path::new("/app-data"), &entry).is_err());
    }

    #[test]
    fn absolute_save_directory_is_its_own_managed_cleanup_root() {
        let root = std::env::temp_dir().join(format!(
            "lsm-absolute-download-root-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let entry = PersistedQueueEntry {
            id: "entry-absolute".into(),
            repo_id: "org/model".into(),
            source: "huggingface".into(),
            save_dir: root.to_string_lossy().into_owned(),
            added_at: 1,
            status: "paused".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![MsFileEntry {
                name: "model.gguf".into(),
                path: "model.gguf".into(),
                size: 100,
                file_type: "file".into(),
                downloaded: None,
                status: Some("paused".into()),
                error: None,
                task_id: Some("task-absolute".into()),
                run_id: Some("run-absolute".into()),
                version: Some(1),
            }],
        };

        let (managed_root, final_path, _, _) = trusted_download_cleanup_paths(
            &[entry],
            Path::new("/ignored-base"),
            "task-absolute",
            "model.gguf",
            Some("run-absolute"),
            None,
        )
        .unwrap();

        assert_eq!(managed_root, root);
        assert!(verified_managed_cleanup_path(&managed_root, &final_path).is_ok());
        let _ = std::fs::remove_dir_all(managed_root);
    }

    #[test]
    fn managed_cleanup_allows_already_missing_nested_directory() {
        let root =
            std::env::temp_dir().join(format!("lsm-download-cleanup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("missing").join("nested").join("model.gguf");

        assert_eq!(
            verified_managed_cleanup_path(&root, &target).unwrap(),
            target
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn retry_filters_files_that_already_completed() {
        let file = |name: &str, status: &str| MsFileEntry {
            name: name.into(),
            path: name.into(),
            size: 100,
            file_type: "file".into(),
            downloaded: None,
            status: Some(status.into()),
            error: None,
            task_id: None,
            run_id: None,
            version: None,
        };

        let pending = pending_download_files(vec![
            file("finished.gguf", "completed"),
            file("cancelled.gguf", "cancelled"),
            file("retry.gguf", "error"),
            file("queued.gguf", "queued"),
        ]);

        assert_eq!(
            pending
                .iter()
                .map(|file| file.name.as_str())
                .collect::<Vec<_>>(),
            vec!["retry.gguf", "queued.gguf"]
        );
    }

    #[test]
    fn terminal_entry_status_preserves_cancellation() {
        let file = |status: &str| MsFileEntry {
            name: format!("{status}.gguf"),
            path: format!("{status}.gguf"),
            size: 100,
            file_type: "file".into(),
            downloaded: None,
            status: Some(status.into()),
            error: None,
            task_id: None,
            run_id: None,
            version: None,
        };
        let entry = PersistedQueueEntry {
            id: "terminal-entry".into(),
            repo_id: "org/model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: "active".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![file("completed"), file("cancelled")],
        };

        assert_eq!(derive_entry_status(&entry), "cancelled");
    }

    #[test]
    fn persisted_only_paused_task_is_removed_by_task_id() {
        let mut entries = vec![PersistedQueueEntry {
            id: "paused-entry".into(),
            repo_id: "org/model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: "paused".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![MsFileEntry {
                name: "model.gguf".into(),
                path: "model.gguf".into(),
                size: 100,
                file_type: "model".into(),
                downloaded: Some(25),
                status: Some("paused".into()),
                error: None,
                task_id: Some("paused-task".into()),
                run_id: Some("paused-run".into()),
                version: Some(1),
            }],
        }];

        assert!(remove_task_from_entries(&mut entries, "paused-task"));
        assert!(entries.is_empty());
    }

    #[test]
    fn cleanup_tombstone_does_not_mistake_a_pause_for_cancellation() {
        let file = MsFileEntry {
            name: "model.gguf".into(),
            path: "model.gguf".into(),
            size: 100,
            file_type: "model".into(),
            downloaded: Some(25),
            status: Some("paused".into()),
            error: None,
            task_id: Some("paused-task".into()),
            run_id: Some("paused-run".into()),
            version: Some(1),
        };
        let cancel_flags = HashMap::from([("paused-run".into(), true)]);
        let pause_flags = HashMap::from([("paused-run".into(), true)]);

        assert!(!cleanup_requested(&file, &cancel_flags, &pause_flags));
        assert!(cleanup_requested(&file, &cancel_flags, &HashMap::new()));
    }

    #[test]
    fn corrupt_download_state_is_quarantined_without_overwrite() {
        let dir = std::env::temp_dir().join(format!(
            "lsm-corrupt-download-state-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("downloads.json");
        std::fs::write(&path, "{broken-json").unwrap();

        let quarantine = quarantine_corrupt_state(&path, "downloads").unwrap();

        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(quarantine).unwrap(), "{broken-json");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn inflight_restore_replaces_same_queue_entry_instead_of_duplicating_it() {
        let entry = |status: &str| PersistedQueueEntry {
            id: "entry-1".into(),
            repo_id: "org/model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: status.into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![MsFileEntry {
                name: "model.gguf".into(),
                path: "model.gguf".into(),
                size: 100,
                file_type: "file".into(),
                downloaded: Some(50),
                status: Some(status.into()),
                error: None,
                task_id: Some("task-1".into()),
                run_id: Some("run-1".into()),
                version: Some(1),
            }],
        };
        let mut queue = vec![entry("queued")];

        merge_crash_recovered_inflight(&mut queue, vec![entry("active")]);

        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].status, "paused");
        assert_eq!(queue[0].files[0].status.as_deref(), Some("paused"));
    }

    #[test]
    fn low_priority_mode_limits_every_download_source_to_one_slot() {
        assert_eq!(apply_download_priority_concurrency(8, true), 1);
        assert_eq!(apply_download_priority_concurrency(4, false), 4);
        assert_eq!(apply_download_priority_concurrency(0, false), 1);
    }

    #[test]
    fn content_range_parser_distinguishes_partial_and_unsatisfied_ranges() {
        assert_eq!(
            parse_content_range("bytes 100-199/1000"),
            Some(ParsedContentRange {
                start: Some(100),
                end: Some(199),
                total: Some(1000),
            })
        );
        assert_eq!(
            parse_content_range("bytes */1000"),
            Some(ParsedContentRange {
                start: None,
                end: None,
                total: Some(1000),
            })
        );
        assert_eq!(parse_content_range("bytes 200-100/1000"), None);
        assert_eq!(parse_content_range("items 0-1/2"), None);
    }

    #[test]
    fn resumed_response_must_start_at_the_local_offset() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_RANGE, "bytes 10-19/100".parse().unwrap());
        headers.insert(CONTENT_LENGTH, "10".parse().unwrap());

        assert!(validate_partial_response(&headers, 10, 100).is_ok());
        assert!(validate_partial_response(&headers, 11, 100)
            .unwrap_err()
            .contains("starts at byte"));
        assert!(validate_partial_response(&headers, 10, 99)
            .unwrap_err()
            .contains("object size changed"));
    }

    #[test]
    fn resumed_total_prefers_content_range_over_chunk_length() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_RANGE, "bytes 10-19/100".parse().unwrap());
        headers.insert(CONTENT_LENGTH, "10".parse().unwrap());

        assert_eq!(response_total_size(&headers, 10, 100), 100);
    }

    #[test]
    fn range_416_requires_an_authoritative_exact_remote_size() {
        assert!(unsatisfied_range_is_complete(100, 100, Some(100)));
        assert!(!unsatisfied_range_is_complete(100, 100, None));
        assert!(!unsatisfied_range_is_complete(101, 100, Some(100)));
        assert!(!unsatisfied_range_is_complete(0, 0, Some(0)));
    }

    #[test]
    fn artifact_identity_detects_two_entries_targeting_the_same_file() {
        let file = MsFileEntry {
            name: "model.gguf".into(),
            path: "weights/model.gguf".into(),
            size: 100,
            file_type: "file".into(),
            downloaded: None,
            status: Some("queued".into()),
            error: None,
            task_id: Some("task-1".into()),
            run_id: Some("run-1".into()),
            version: Some(1),
        };
        let entry = PersistedQueueEntry {
            id: "entry-1".into(),
            repo_id: "org/model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: "queued".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![file.clone()],
        };
        let mut duplicate = entry.clone();
        duplicate.id = "entry-2".into();
        duplicate.files[0].task_id = Some("task-2".into());
        duplicate.files[0].run_id = Some("run-2".into());

        assert_eq!(
            download_file_identity(&entry, &file),
            download_file_identity(&duplicate, &duplicate.files[0])
        );
        assert!(conflicting_download_identity(&duplicate, std::slice::from_ref(&entry),).is_some());
    }

    #[test]
    fn same_basename_in_different_remote_directories_has_distinct_targets() {
        let file = |path: &str| MsFileEntry {
            name: "model.gguf".into(),
            path: path.into(),
            size: 100,
            file_type: "model".into(),
            downloaded: None,
            status: Some("queued".into()),
            error: None,
            task_id: None,
            run_id: None,
            version: None,
        };
        let entry = |id: &str, file: MsFileEntry| PersistedQueueEntry {
            id: id.into(),
            repo_id: "org/model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: "queued".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![file],
        };
        let first = entry("a", file("a/model.gguf"));
        let second = entry("b", file("b/model.gguf"));

        assert_ne!(
            download_file_identity(&first, &first.files[0]),
            download_file_identity(&second, &second.files[0])
        );
        assert_ne!(
            remote_parent_dir(Path::new("/managed"), &first.files[0].path).unwrap(),
            remote_parent_dir(Path::new("/managed"), &second.files[0].path).unwrap()
        );
    }

    #[test]
    fn resume_all_only_refreshes_paused_files_in_mixed_entries() {
        let file = |name: &str, status: &str, version: u32| MsFileEntry {
            name: name.into(),
            path: name.into(),
            size: 100,
            file_type: "model".into(),
            downloaded: Some(100),
            status: Some(status.into()),
            error: None,
            task_id: Some(format!("task-{name}")),
            run_id: Some(format!("run-{name}")),
            version: Some(version),
        };
        let mut entry = PersistedQueueEntry {
            id: "mixed".into(),
            repo_id: "org/model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: "paused".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![
                file("done.gguf", "completed", 4),
                file("paused.gguf", "paused", 7),
            ],
        };

        let identities = refresh_paused_entry_for_resume(&mut entry);

        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].task_id, "task-paused.gguf");
        assert_eq!(entry.files[0].status.as_deref(), Some("completed"));
        assert_eq!(entry.files[0].version, Some(4));
        assert_eq!(entry.files[1].status.as_deref(), Some("queued"));
        assert_eq!(entry.files[1].version, Some(8));
    }

    #[test]
    fn cancel_all_durably_removes_paused_and_queued_files() {
        let file = |name: &str, status: &str| MsFileEntry {
            name: name.into(),
            path: name.into(),
            size: 100,
            file_type: "model".into(),
            downloaded: None,
            status: Some(status.into()),
            error: None,
            task_id: Some(format!("task-{name}")),
            run_id: Some(format!("run-{name}")),
            version: Some(1),
        };
        let mut entries = vec![PersistedQueueEntry {
            id: "mixed".into(),
            repo_id: "org/model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: "paused".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![
                file("paused.gguf", "paused"),
                file("queued.gguf", "queued"),
                file("done.gguf", "completed"),
                file("failed.gguf", "error"),
            ],
        }];

        retain_cancel_all_terminal_entries(&mut entries);

        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0]
                .files
                .iter()
                .map(|file| file.status.as_deref().unwrap())
                .collect::<Vec<_>>(),
            vec!["completed", "error"]
        );
        assert_eq!(entries[0].status, "error");
    }

    #[test]
    fn deterministic_http_failures_are_not_retried() {
        assert!(!is_retryable_error(Some(400)));
        assert!(!is_retryable_error(Some(403)));
        assert!(!is_retryable_error(Some(404)));
        assert!(is_retryable_error(Some(429)));
        assert!(is_retryable_error(Some(500)));
        assert!(!is_retryable_error(None));
    }
}

// Batch control commands.

pub async fn pause_all_downloads(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    let _scheduler = state
        .download_scheduler_lock
        .lock()
        .map_err(|_| "download scheduler lock is poisoned".to_string())?;
    let active_run_ids = state.active_downloads.lock().unwrap().clone();
    {
        let mut cancel = state.cancel_flags.lock().unwrap();
        let mut pause = state.pause_flags.lock().unwrap();
        for run_id in &active_run_ids {
            cancel.insert(run_id.clone(), true);
            pause.insert(run_id.clone(), true);
        }
    }

    let mut affected = Vec::new();
    {
        let mut active_entries = state.download_active_entries.lock().unwrap();
        for entry in active_entries.values_mut() {
            for file in &mut entry.files {
                if file
                    .run_id
                    .as_ref()
                    .is_some_and(|run_id| active_run_ids.contains(run_id))
                {
                    file.status = Some("pausing".into());
                    if let Some(task_id) = &file.task_id {
                        affected.push(task_id.clone());
                    }
                }
            }
            entry.status = derive_entry_status(entry);
        }
    }
    persist_active_entries_snapshot(&state, true);

    {
        let mut queue = state.download_queue.lock().unwrap();
        for entry in queue.iter_mut() {
            entry.status = "paused".into();
            for file in &mut entry.files {
                if !is_terminal_download_file(file) {
                    file.status = Some("paused".into());
                    if let Some(task_id) = &file.task_id {
                        affected.push(task_id.clone());
                    }
                }
            }
        }
    }
    persist_manager_queue(&state)?;
    state
        .download_queue
        .lock()
        .unwrap()
        .retain(|entry| entry.status != "paused");
    affected.sort();
    affected.dedup();
    Ok(affected)
}

pub async fn cancel_all_downloads(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let mut affected_entries: Vec<PersistedQueueEntry> = load_download_state(&state)?;
    let active_entries: Vec<PersistedQueueEntry> = {
        let entries = state.download_active_entries.lock().unwrap();
        entries.values().cloned().collect()
    };
    affected_entries.extend(active_entries.clone());
    affected_entries.extend(state.download_queue.lock().unwrap().clone());
    let active_run_ids = state
        .active_downloads
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    {
        let mut cancel = state.cancel_flags.lock().unwrap();
        for run_id in &active_run_ids {
            cancel.insert(run_id.clone(), true);
        }
    }
    let _scheduler = state
        .download_scheduler_lock
        .lock()
        .map_err(|_| "download scheduler lock is poisoned".to_string())?;
    let previous_runtime = {
        let mut queue = state.download_queue.lock().unwrap();
        let previous = queue.clone();
        queue.clear();
        previous
    };
    if let Err(error) = persist_manager_queue(&state) {
        *state.download_queue.lock().unwrap() = previous_runtime;
        return Err(error);
    }
    update_download_state(&state, |persisted| {
        retain_cancel_all_terminal_entries(persisted);
        Ok(())
    })?;
    update_inflight_state(&state, |inflight| {
        inflight.clear();
    })?;

    let base_dir = state
        .config_dir
        .lock()
        .unwrap()
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    for entry in &affected_entries {
        let repo_dir = match queue_entry_download_dir(&base_dir, entry) {
            Ok(path) => path,
            Err(_) => continue,
        };
        for file in &entry.files {
            if matches!(file.status.as_deref(), Some("completed" | "error")) {
                continue;
            }
            let Ok(dir) = remote_parent_dir(&repo_dir, &file.path) else {
                continue;
            };
            let (_, temp_path, metadata_path) = build_download_paths(&dir, &file.name);
            if !file
                .run_id
                .as_ref()
                .is_some_and(|run_id| active_run_ids.contains(run_id))
            {
                for path in [&temp_path, &metadata_path] {
                    match std::fs::remove_file(path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(format!("Failed to remove {}: {error}", path.display()))
                        }
                    }
                }
            }
        }
    }
    for entry in active_entries {
        for file in entry.files {
            let _ = app.emit(
                "download-cancelled",
                serde_json::json!({
                    "taskId": file.task_id.as_deref().unwrap_or(""),
                    "runId": file.run_id.as_deref().unwrap_or(""),
                    "version": file.version.unwrap_or(0),
                    "fileName": &file.name,
                    "repoId": &entry.repo_id,
                    "source": &entry.source,
                    "remotePath": &file.path,
                }),
            );
        }
    }
    Ok(())
}

// Concurrency control commands.

pub async fn set_download_concurrency(
    n: usize,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    if !(1..=8).contains(&n) {
        return Err("concurrency must be 1-8".into());
    }
    crate::commands::config::update_and_persist(&state, |global| {
        global.download_max_concurrent = n;
    })?;
    *state.download_max_concurrent.lock().unwrap() = n;
    state.download_slot_notify.notify_waiters();
    fill_download_queue_slots(app);
    Ok(())
}

pub async fn get_download_concurrency(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    Ok(*state.download_max_concurrent.lock().unwrap())
}

// Reset download state for redownload.

pub async fn set_download_bandwidth_limit(
    bytes_per_sec: u64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    const MAX_LIMIT_BYTES_PER_SEC: u64 = 10 * 1024 * 1024 * 1024;
    if bytes_per_sec > MAX_LIMIT_BYTES_PER_SEC {
        return Err("bandwidth limit must be 0-10 GiB/s".into());
    }
    crate::commands::config::update_and_persist(&state, |global| {
        global.download_bandwidth_limit_bytes_per_sec = bytes_per_sec;
    })?;
    *state.download_bandwidth_limit_bytes_per_sec.lock().unwrap() = bytes_per_sec;
    {
        let mut limiter = state.download_bandwidth_limiter.lock().unwrap();
        limiter.available_bytes = 0.0;
        limiter.last_refill = std::time::Instant::now();
    }
    Ok(())
}

pub async fn get_download_bandwidth_limit(
    state: tauri::State<'_, AppState>,
) -> Result<u64, String> {
    Ok(*state.download_bandwidth_limit_bytes_per_sec.lock().unwrap())
}

pub async fn set_download_low_priority_throttle(
    enabled: bool,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    crate::commands::config::update_and_persist(&state, |global| {
        global.download_low_priority_throttle = enabled;
    })?;
    *state.download_low_priority_throttle.lock().unwrap() = enabled;
    {
        let mut limiter = state.download_bandwidth_limiter.lock().unwrap();
        limiter.available_bytes = 0.0;
        limiter.last_refill = std::time::Instant::now();
    }
    state.download_slot_notify.notify_waiters();
    if !enabled {
        fill_download_queue_slots(app);
    }
    Ok(())
}

pub async fn get_download_low_priority_throttle(
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    Ok(*state.download_low_priority_throttle.lock().unwrap())
}

pub async fn reset_download_for_redownload(
    _task_id: String,
    file_name: String,
    repo_id: String,
    save_dir: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _ = sanitize_file_name(&file_name)?;
    let config_dir = state.config_dir.lock().unwrap().clone();
    let save_path = resolve_redownload_save_path(&config_dir, &save_dir, &repo_id)?;
    let (_final_path, temp_path, _meta_path) = build_download_paths(&save_path, &file_name);
    cleanup_artifact_state(&temp_path);
    Ok(())
}

// Download manager snapshot.

#[derive(serde::Serialize)]
pub struct DownloadManagerSnapshot {
    pub queue: Vec<PersistedQueueEntry>,
    pub active_count: usize,
    pub max_concurrent: usize,
    pub resume_policy: String,
    pub bandwidth_limit_bytes_per_sec: u64,
    pub low_priority_throttle: bool,
}

pub async fn get_download_manager_snapshot(
    state: tauri::State<'_, AppState>,
) -> Result<DownloadManagerSnapshot, String> {
    let queue = collect_manager_entries(&state);
    let active_count = state.download_active_batches.lock().unwrap().len();
    let max_concurrent = *state.download_max_concurrent.lock().unwrap();
    let bandwidth_limit_bytes_per_sec =
        *state.download_bandwidth_limit_bytes_per_sec.lock().unwrap();
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

// IPC compatibility boundary: legacy command internals keep their existing error flow,
// while every registered command serializes a stable AppError object.
#[allow(dead_code, unused_imports, unused_mut)] // Tauri references adapters through generated macros.
pub mod ipc {
    use super::*;

    #[tauri::command]
    pub async fn browse_modelscope(repo_id: String) -> crate::error::AppResult<Vec<MsFileEntry>> {
        super::browse_modelscope(repo_id)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn download_modelscope_files(
        repo_id: String,
        files: Vec<MsFileEntry>,
        save_dir: String,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<()> {
        super::download_modelscope_files(repo_id, files, save_dir, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn cancel_file_download(
        task_id: String,
        run_id: Option<String>,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::cancel_file_download(task_id, run_id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn pause_file_download(
        task_id: String,
        run_id: Option<String>,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::pause_file_download(task_id, run_id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn cancel_and_cleanup_download(
        task_id: String,
        file_name: String,
        file_path: String,
        run_id: Option<String>,
        version: Option<u32>,
        state: tauri::State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<()> {
        super::cancel_and_cleanup_download(
            task_id, file_name, file_path, run_id, version, state, app,
        )
        .await
        .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn browse_huggingface(repo_id: String) -> crate::error::AppResult<Vec<MsFileEntry>> {
        super::browse_huggingface(repo_id)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn download_huggingface_files(
        repo_id: String,
        files: Vec<MsFileEntry>,
        save_dir: String,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<()> {
        super::download_huggingface_files(repo_id, files, save_dir, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn check_local_file(path: String) -> crate::error::AppResult<Option<u64>> {
        super::check_local_file(path)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn persist_download_queue(
        queue: Vec<PersistedQueueEntry>,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::persist_download_queue(queue, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn restore_download_queue(
        state: tauri::State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<Vec<PersistedQueueEntry>> {
        super::restore_download_queue(state, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn enqueue_download_queue(
        mut entry: PersistedQueueEntry,
        state: tauri::State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<()> {
        super::enqueue_download_queue(entry, state, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn remove_download_queue_entry(
        id: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::remove_download_queue_entry(id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn clear_download_tasks_by_status(
        statuses: Vec<String>,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::clear_download_tasks_by_status(statuses, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn process_download_queue(app: tauri::AppHandle) -> crate::error::AppResult<()> {
        super::process_download_queue(app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_download_resume_policy(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<String> {
        super::get_download_resume_policy(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn set_download_resume_policy(
        policy: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::set_download_resume_policy(policy, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn resume_download_task(
        task_id: String,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<ResumeDownloadTaskResult> {
        super::resume_download_task(task_id, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn resume_all_downloads(
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<Vec<ResumeDownloadTaskResult>> {
        super::resume_all_downloads(app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn pause_all_downloads(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<Vec<String>> {
        super::pause_all_downloads(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn cancel_all_downloads(
        state: tauri::State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<()> {
        super::cancel_all_downloads(state, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn set_download_concurrency(
        n: usize,
        state: tauri::State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<()> {
        super::set_download_concurrency(n, state, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_download_concurrency(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<usize> {
        super::get_download_concurrency(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn set_download_bandwidth_limit(
        bytes_per_sec: u64,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::set_download_bandwidth_limit(bytes_per_sec, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_download_bandwidth_limit(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<u64> {
        super::get_download_bandwidth_limit(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn set_download_low_priority_throttle(
        enabled: bool,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<()> {
        super::set_download_low_priority_throttle(enabled, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_download_low_priority_throttle(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<bool> {
        super::get_download_low_priority_throttle(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn reset_download_for_redownload(
        _task_id: String,
        file_name: String,
        repo_id: String,
        save_dir: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::reset_download_for_redownload(_task_id, file_name, repo_id, save_dir, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_download_manager_snapshot(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<DownloadManagerSnapshot> {
        super::get_download_manager_snapshot(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn delete_managed_local_file(
        file_path: String,
        save_dir: String,
        repo_id: String,
        remote_path: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::delete_managed_local_file(file_path, save_dir, repo_id, remote_path, state)
            .await
            .map_err(crate::error::AppError::from)
    }
}
