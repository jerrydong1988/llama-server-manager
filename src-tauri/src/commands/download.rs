use crate::bounded_http;
use crate::models::{
    AppState, DownloadArtifactState, MsFileEntry, NativeDownloadArtifactRecord,
    NativeDownloadArtifactRegistry, NativeDownloadBrowseGrant, NativeDownloadPartialRecord,
    PersistedQueueEntry,
};
#[cfg(test)]
use crate::path_utils::paths_equal;
use crate::path_utils::{path_identity_key, path_is_within};
use crate::utils;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, CONTENT_LENGTH, CONTENT_RANGE, IF_RANGE};
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

// Shared HTTP client for all download operations
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_DOWNLOAD_REDIRECTS {
                return attempt.error("download redirect limit exceeded");
            }
            if download_redirect_destination_allowed(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("download redirect destination is not trusted")
            }
        }))
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default()
});
static REPOSITORY_BROWSE_SLOTS: LazyLock<tokio::sync::Semaphore> =
    LazyLock::new(|| tokio::sync::Semaphore::new(8));

const DOWNLOAD_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const DOWNLOAD_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const DOWNLOAD_MAX_TRANSFER_LIFETIME: std::time::Duration =
    std::time::Duration::from_secs(24 * 60 * 60);
const MAX_DOWNLOAD_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024 * 1024;
const MAX_DOWNLOAD_BATCH_BYTES: u64 = 8 * 1024 * 1024 * 1024 * 1024;
const MAX_DOWNLOAD_BATCH_FILES: usize = 256;
const DOWNLOAD_DISK_RESERVE_BYTES: u64 = 512 * 1024 * 1024;
const DOWNLOAD_DISK_RECHECK_BYTES: u64 = 64 * 1024 * 1024;
const MAX_REPOSITORY_METADATA_BYTES: usize = 8 * 1024 * 1024;
const MAX_REPOSITORY_ENTRIES: usize = 20_000;
const MAX_REPOSITORY_FIELD_BYTES: usize = 4 * 1024;
const DOWNLOAD_BATCH_WORKERS: usize = 8;
const MAX_DOWNLOAD_REDIRECTS: usize = 5;
const MAX_ARTIFACT_STATE_BYTES: u64 = 64 * 1024;
const MAX_DOWNLOAD_STATE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_DOWNLOAD_STATE_ENTRIES: usize = 1_024;
const MAX_DOWNLOAD_STATE_FILES: usize = 4_096;
const MAX_DOWNLOAD_ARTIFACT_REGISTRY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_DOWNLOAD_ARTIFACT_REGISTRY_ENTRIES: usize = 8_192;
const MAX_BROWSE_GRANTS_PER_RESPONSE: usize = 4_096;
const DOWNLOAD_BROWSE_GRANT_LIFETIME_SECS: u64 = 30 * 24 * 60 * 60;

static DOWNLOAD_STATE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static RESERVED_DOWNLOAD_BYTES: LazyLock<Mutex<u64>> = LazyLock::new(|| Mutex::new(0));

struct DownloadDiskReservation {
    bytes: u64,
}

impl Drop for DownloadDiskReservation {
    fn drop(&mut self) {
        let mut reserved = RESERVED_DOWNLOAD_BYTES.lock().unwrap();
        *reserved = reserved.saturating_sub(self.bytes);
    }
}

fn reserve_download_disk_budget(
    path: &Path,
    remaining: u64,
) -> Result<DownloadDiskReservation, String> {
    let mut reserved = RESERVED_DOWNLOAD_BYTES
        .lock()
        .map_err(|_| "download disk reservation lock is poisoned".to_string())?;
    let required = reserved
        .checked_add(remaining)
        .and_then(|bytes| bytes.checked_add(DOWNLOAD_DISK_RESERVE_BYTES))
        .ok_or_else(|| "download disk reservation overflow".to_string())?;
    let available = fs2::available_space(path)
        .map_err(|error| format!("Failed to inspect available download space: {error}"))?;
    if available < required {
        return Err(format!(
            "insufficient disk space: {available} bytes available, {required} atomically reserved"
        ));
    }
    *reserved = reserved
        .checked_add(remaining)
        .ok_or_else(|| "download disk reservation overflow".to_string())?;
    Ok(DownloadDiskReservation { bytes: remaining })
}

fn download_redirect_destination_allowed(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let Some(host) = url.host_str().map(|host| host.to_ascii_lowercase()) else {
        return false;
    };
    const TRUSTED_DOWNLOAD_HOST_SUFFIXES: &[&str] = &[
        "huggingface.co",
        "hf.co",
        "xethub.hf.co",
        "xetcontent.com",
        "modelscope.cn",
        "aliyuncs.com",
    ];
    TRUSTED_DOWNLOAD_HOST_SUFFIXES.iter().any(|suffix| {
        host == *suffix
            || host
                .strip_suffix(suffix)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

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
    if remote_path.is_empty()
        || remote_path.starts_with('/')
        || remote_path.contains('\\')
        || Path::new(remote_path).is_absolute()
        || Path::new(remote_path).has_root()
    {
        return Err("Remote file path is invalid".into());
    }
    let mut destination = root.to_path_buf();
    let mut segments = remote_path.split('/').peekable();
    while let Some(segment) = segments.next() {
        if segment.is_empty() || segment == "." || segment == ".." || segment.contains(':') {
            return Err("Remote file path contains an unsafe segment".into());
        }
        if segments.peek().is_some() {
            destination.push(segment);
        }
    }
    crate::security::ensure_download_path_within_root(&destination, root)?;
    crate::security::ensure_existing_download_ancestors_within_root(&destination, root)?;
    Ok(destination)
}

fn validate_managed_file(file: &MsFileEntry) -> Result<(), String> {
    let name = sanitize_file_name(&file.name)?;
    let remote_name = file
        .path
        .split('/')
        .next_back()
        .ok_or_else(|| "Remote file path has no file name".to_string())?;
    if remote_name != name {
        return Err("Download file name does not match its remote path".to_string());
    }
    let allowed_data_file = std::path::Path::new(&name)
        .extension()
        .is_some_and(|extension| {
            extension.to_string_lossy().eq_ignore_ascii_case("gguf")
                || extension.to_string_lossy().eq_ignore_ascii_case("txt")
        });
    if !allowed_data_file {
        return Err(
            "Native downloads allow only model data (.gguf) and plain text metadata (.txt)"
                .to_string(),
        );
    }
    let _ = remote_parent_dir(Path::new("."), &file.path)?;
    Ok(())
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
    path_identity_key(&absolute)
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

fn strong_resume_validator_missing_or_changed(
    stored_etag: Option<&str>,
    response_etag: Option<&str>,
) -> bool {
    stored_etag
        .filter(|etag| !etag.trim_start().starts_with("W/"))
        .is_some_and(|expected| response_etag != Some(expected))
}

fn response_total_size(
    headers: &HeaderMap,
    resume_from: u64,
    fallback_size: u64,
) -> Result<u64, String> {
    if resume_from > 0 {
        if let Some(total) = response_content_range(headers).and_then(|range| range.total) {
            if fallback_size > 0 && total != fallback_size {
                return Err(format!(
                    "remote object size changed from {fallback_size} to {total} bytes"
                ));
            }
            return validate_accepted_download_size(total);
        }
    }
    let header_total = headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|content_length| {
            content_length
                .checked_add(resume_from)
                .ok_or_else(|| "download response size overflow".to_string())
        })
        .transpose()?;
    if let Some(header_total) = header_total {
        if fallback_size > 0 && header_total != fallback_size {
            return Err(format!(
                "remote object size changed from {fallback_size} to {header_total} bytes"
            ));
        }
        return validate_accepted_download_size(header_total);
    }
    validate_accepted_download_size(fallback_size)
}

fn validate_accepted_download_size(size: u64) -> Result<u64, String> {
    if size == 0 {
        return Err("download response does not declare an accepted artifact size".into());
    }
    if size > MAX_DOWNLOAD_FILE_BYTES {
        return Err(format!(
            "download artifact exceeds the {MAX_DOWNLOAD_FILE_BYTES}-byte application limit"
        ));
    }
    Ok(size)
}

fn ensure_download_disk_budget(path: &Path, remaining: u64) -> Result<(), String> {
    let required = remaining
        .checked_add(DOWNLOAD_DISK_RESERVE_BYTES)
        .ok_or_else(|| "download disk budget overflow".to_string())?;
    let available = fs2::available_space(path)
        .map_err(|error| format!("Failed to inspect available download space: {error}"))?;
    if available < required {
        return Err(format!(
            "insufficient disk space: {available} bytes available, {required} required including reserve"
        ));
    }
    Ok(())
}

fn validate_download_batch(files: &[MsFileEntry]) -> Result<u64, String> {
    if files.is_empty() {
        return Err("Download batch is empty".into());
    }
    if files.len() > MAX_DOWNLOAD_BATCH_FILES {
        return Err(format!(
            "Download batch exceeds the {MAX_DOWNLOAD_BATCH_FILES}-file limit"
        ));
    }
    let mut total = 0_u64;
    for file in files {
        if file.size == 0 {
            return Err(format!(
                "{} has an unknown size; refresh repository metadata before downloading",
                file.name
            ));
        }
        if file.size > MAX_DOWNLOAD_FILE_BYTES {
            return Err(format!("{} exceeds the per-file download limit", file.name));
        }
        total = total
            .checked_add(file.size)
            .ok_or_else(|| "Download batch size overflow".to_string())?;
        if total > MAX_DOWNLOAD_BATCH_BYTES {
            return Err(format!(
                "Download batch exceeds the {MAX_DOWNLOAD_BATCH_BYTES}-byte limit"
            ));
        }
    }
    Ok(total)
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

fn metadata_is_link_like(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DownloadFileObjectId(String);

fn download_file_object_id(file: &std::fs::File) -> Result<DownloadFileObjectId, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file
            .metadata()
            .map_err(|error| format!("Failed to inspect download artifact: {error}"))?;
        return Ok(DownloadFileObjectId(format!(
            "unix:{}:{}",
            metadata.dev(),
            metadata.ino()
        )));
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        let result =
            unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info as *mut _) };
        if result == 0 {
            return Err(format!(
                "Failed to identify download artifact: {}",
                std::io::Error::last_os_error()
            ));
        }
        return Ok(DownloadFileObjectId(format!(
            "windows:{}:{}",
            info.dwVolumeSerialNumber,
            ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64
        )));
    }
    #[allow(unreachable_code)]
    Err("Download artifact identity is unsupported on this platform".to_string())
}

fn reject_download_hardlink(file: &std::fs::File) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let links = file
            .metadata()
            .map_err(|error| format!("Failed to inspect download artifact links: {error}"))?
            .nlink();
        if links != 1 {
            return Err(format!(
                "Download artifact has {links} hard links; exactly one is required"
            ));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        let result =
            unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info as *mut _) };
        if result == 0 {
            return Err(format!(
                "Failed to inspect download artifact links: {}",
                std::io::Error::last_os_error()
            ));
        }
        if info.nNumberOfLinks != 1 {
            return Err(format!(
                "Download artifact has {} hard links; exactly one is required",
                info.nNumberOfLinks
            ));
        }
    }
    Ok(())
}

struct DownloadDirectoryLease {
    requested_dir_key: String,
    dir: cap_std::fs::Dir,
}

impl DownloadDirectoryLease {
    fn open_within(path: &Path, managed_root: &Path) -> Result<Self, String> {
        let canonical_root_before = std::fs::canonicalize(managed_root)
            .map_err(|error| format!("Failed to resolve managed download root: {error}"))?;
        let canonical_before = std::fs::canonicalize(path)
            .map_err(|error| format!("Failed to resolve download directory: {error}"))?;
        if !path_is_within(&canonical_before, &canonical_root_before) {
            return Err("Download directory escaped its managed root".to_string());
        }
        let root_dir = cap_std::fs::Dir::open_ambient_dir(
            &canonical_root_before,
            cap_std::ambient_authority(),
        )
        .map_err(|error| format!("Failed to bind managed download root: {error}"))?;
        let relative = canonical_before
            .strip_prefix(&canonical_root_before)
            .map_err(|_| "Download directory escaped its managed root".to_string())?;
        let open_relative = || {
            if relative.as_os_str().is_empty() {
                root_dir.try_clone()
            } else {
                root_dir.open_dir(relative)
            }
        };
        let dir = open_relative()
            .map_err(|error| format!("Failed to bind download directory: {error}"))?;
        let verification = open_relative()
            .map_err(|error| format!("Failed to verify download directory: {error}"))?;
        let first = download_file_object_id(
            &dir.try_clone()
                .map_err(|error| format!("Failed to clone download directory handle: {error}"))?
                .into_std_file(),
        )?;
        let second = download_file_object_id(&verification.into_std_file())?;
        let canonical_after = std::fs::canonicalize(path)
            .map_err(|error| format!("Failed to re-resolve download directory: {error}"))?;
        let canonical_root_after = std::fs::canonicalize(managed_root)
            .map_err(|error| format!("Failed to re-resolve managed download root: {error}"))?;
        if first != second
            || path_identity_key(&canonical_before) != path_identity_key(&canonical_after)
            || path_identity_key(&canonical_root_before) != path_identity_key(&canonical_root_after)
        {
            return Err("Download directory changed while it was being bound".to_string());
        }
        Ok(Self {
            requested_dir_key: path_identity_key(path),
            dir,
        })
    }

    fn artifact_name<'a>(&self, path: &'a Path) -> Result<&'a std::ffi::OsStr, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "Download artifact has no parent directory".to_string())?;
        if path_identity_key(parent) != self.requested_dir_key {
            return Err(format!(
                "Download artifact {} is not in the bound directory",
                path.display()
            ));
        }
        let name_lossy = path
            .file_name()
            .ok_or_else(|| "Download artifact has no file name".to_string())?
            .to_string_lossy();
        let mut components = name_lossy
            .split(['/', '\\'])
            .filter(|component| !component.is_empty());
        let Some(name) = components.next() else {
            return Err("Download artifact has an empty file name".to_string());
        };
        if components.next().is_some() || matches!(name, "." | "..") {
            return Err("Download artifact name is unsafe".to_string());
        }
        Ok(path.file_name().unwrap())
    }

    fn open_nofollow(
        &self,
        name: &std::ffi::OsStr,
        options: &cap_std::fs::OpenOptions,
    ) -> Result<std::fs::File, String> {
        self.dir
            .open_with(name, options)
            .map(cap_std::fs::File::into_std)
            .map_err(|error| format!("Failed to open bound download artifact: {error}"))
    }

    fn validate_candidate(&self, path: &Path) -> Result<(), String> {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        let name = self.artifact_name(path)?;
        match self.dir.symlink_metadata(name) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(format!(
                        "Download artifact is not a regular file: {}",
                        path.display()
                    ));
                }
                let mut options = cap_std::fs::OpenOptions::new();
                options.read(true).follow(FollowSymlinks::No);
                let file = self.open_nofollow(name, &options)?;
                reject_download_hardlink(&file)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "Failed to inspect bound download artifact {}: {error}",
                path.display()
            )),
        }
    }

    fn open_temp(
        &self,
        final_path: &Path,
        temp_path: &Path,
        metadata_path: &Path,
        append: bool,
        existing_owned: bool,
    ) -> Result<std::fs::File, String> {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        for path in [final_path, temp_path, metadata_path] {
            self.validate_candidate(path)?;
        }
        let name = self.artifact_name(temp_path)?;
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).follow(FollowSymlinks::No);
        if existing_owned {
            options.create(false);
        } else {
            options.create_new(true);
        }
        if append {
            options.append(true);
        }
        // Never request truncation during the pathname open. A raced hardlink
        // must be rejected from the opened handle before any destructive
        // operation is applied to that object.
        let mut file = self.open_nofollow(name, &options)?;
        if !file
            .metadata()
            .map_err(|error| format!("Failed to inspect opened download artifact: {error}"))?
            .is_file()
        {
            return Err("Opened download artifact is not a regular file".to_string());
        }
        reject_download_hardlink(&file)?;
        if !append {
            file.set_len(0)
                .map_err(|error| format!("Failed to reset verified download artifact: {error}"))?;
            file.seek(SeekFrom::Start(0))
                .map_err(|error| format!("Failed to rewind verified download artifact: {error}"))?;
        }
        Ok(file)
    }

    fn metadata_len(&self, path: &Path) -> Result<u64, String> {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        let name = self.artifact_name(path)?;
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = self.open_nofollow(name, &options)?;
        reject_download_hardlink(&file)?;
        file.metadata()
            .map(|metadata| metadata.len())
            .map_err(|error| format!("Failed to inspect download artifact size: {error}"))
    }

    fn inspect_file(&self, path: &Path) -> Result<(DownloadFileObjectId, u64), String> {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        let name = self.artifact_name(path)?;
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = self.open_nofollow(name, &options)?;
        reject_download_hardlink(&file)?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("Failed to inspect download artifact: {error}"))?;
        if !metadata.is_file() {
            return Err("Download artifact is not a regular file".to_string());
        }
        Ok((download_file_object_id(&file)?, metadata.len()))
    }

    fn sha256_file(&self, path: &Path) -> Result<String, String> {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        let name = self.artifact_name(path)?;
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = self.open_nofollow(name, &options)?;
        reject_download_hardlink(&file)?;
        let mut digest = Sha256::new();
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|error| format!("Failed to hash completed download: {error}"))?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        Ok(format!("{:x}", digest.finalize()))
    }

    fn exists(&self, path: &Path) -> bool {
        self.artifact_name(path)
            .ok()
            .and_then(|name| self.dir.symlink_metadata(name).ok())
            .is_some()
    }

    fn read_to_string_bounded(&self, path: &Path, max_bytes: u64) -> Result<String, String> {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        let name = self.artifact_name(path)?;
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = self.open_nofollow(name, &options)?;
        reject_download_hardlink(&file)?;
        let length = file
            .metadata()
            .map_err(|error| format!("Failed to inspect download artifact state: {error}"))?
            .len();
        if length > max_bytes {
            return Err(format!(
                "Download artifact state exceeds the {max_bytes}-byte limit"
            ));
        }
        let mut contents = String::new();
        std::io::Read::by_ref(&mut file)
            .take(max_bytes.saturating_add(1))
            .read_to_string(&mut contents)
            .map_err(|error| format!("Failed to read download artifact state: {error}"))?;
        if contents.len() as u64 > max_bytes {
            return Err(format!(
                "Download artifact state exceeds the {max_bytes}-byte limit"
            ));
        }
        Ok(contents)
    }

    #[cfg(test)]
    fn write_atomic(&self, path: &Path, contents: &[u8]) -> Result<(), String> {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        self.validate_candidate(path)?;
        let destination = self.artifact_name(path)?;
        let temp_name = format!(
            ".{}.{}.tmp",
            destination.to_string_lossy(),
            uuid::Uuid::new_v4()
        );
        let mut options = cap_std::fs::OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let mut file = self.open_nofollow(std::ffi::OsStr::new(&temp_name), &options)?;
        reject_download_hardlink(&file)?;
        let result = (|| {
            file.write_all(contents)
                .map_err(|error| format!("Failed to write download artifact state: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("Failed to sync download artifact state: {error}"))?;
            drop(file);
            #[cfg(windows)]
            match self.dir.remove_file(destination) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "Failed to replace existing download artifact state: {error}"
                    ));
                }
            }
            self.dir
                .rename(&temp_name, &self.dir, destination)
                .map_err(|error| format!("Failed to replace download artifact state: {error}"))
        })();
        if result.is_err() {
            let _ = self.dir.remove_file(&temp_name);
        }
        result
    }

    fn replace(&self, source: &Path, destination: &Path) -> Result<DownloadFileObjectId, String> {
        self.validate_candidate(source)?;
        self.validate_candidate(destination)?;
        let source_name = self.artifact_name(source)?;
        let destination_name = self.artifact_name(destination)?;
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt as _;
            use windows_sys::Win32::Storage::FileSystem::{
                DELETE, FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            };
            options
                .access_mode(FILE_GENERIC_READ | DELETE)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
        }
        let source_file = self.open_nofollow(source_name, &options)?;
        reject_download_hardlink(&source_file)?;
        let expected = download_file_object_id(&source_file)?;
        #[cfg(unix)]
        rustix::fs::renameat_with(
            &self.dir,
            source_name,
            &self.dir,
            destination_name,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|error| {
            format!("Failed to finalize download artifact without replacement: {error}")
        })?;
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                FileRenameInfo, SetFileInformationByHandle, FILE_RENAME_INFO,
            };
            let destination_utf16 = destination_name.encode_wide().collect::<Vec<_>>();
            let header_bytes = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
            let buffer_bytes = header_bytes + destination_utf16.len() * std::mem::size_of::<u16>();
            let mut buffer = vec![0usize; buffer_bytes.div_ceil(std::mem::size_of::<usize>())];
            let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
            // SAFETY: `buffer` is aligned for FILE_RENAME_INFO and sized for
            // the header plus the complete UTF-16 destination name.
            unsafe {
                (*info).Anonymous.ReplaceIfExists = false;
                (*info).RootDirectory = self.dir.as_raw_handle() as _;
                (*info).FileNameLength =
                    (destination_utf16.len() * std::mem::size_of::<u16>()) as u32;
                std::ptr::copy_nonoverlapping(
                    destination_utf16.as_ptr(),
                    (*info).FileName.as_mut_ptr(),
                    destination_utf16.len(),
                );
                if SetFileInformationByHandle(
                    source_file.as_raw_handle() as _,
                    FileRenameInfo,
                    info.cast(),
                    buffer_bytes as u32,
                ) == 0
                {
                    return Err(format!(
                        "Failed to finalize download artifact without replacement: {}",
                        std::io::Error::last_os_error()
                    ));
                }
            }
        }
        let final_file = self.open_nofollow(destination_name, &options)?;
        if download_file_object_id(&final_file)? != expected {
            let _ = self.dir.remove_file(destination_name);
            return Err("Download artifact changed during final replacement".to_string());
        }
        reject_download_hardlink(&final_file)?;
        Ok(expected)
    }

    fn remove_if_identity(
        &self,
        path: &Path,
        expected: &DownloadFileObjectId,
    ) -> Result<bool, String> {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
        self.validate_candidate(path)?;
        let name = self.artifact_name(path)?;
        match self.dir.symlink_metadata(name) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "Failed to inspect managed download artifact before cleanup: {error}"
                ));
            }
        }
        let quarantine = format!(
            ".{}.{}.delete",
            name.to_string_lossy(),
            uuid::Uuid::new_v4().simple()
        );
        self.dir
            .rename(name, &self.dir, &quarantine)
            .map_err(|error| format!("Failed to quarantine managed download artifact: {error}"))?;
        let result = (|| {
            let mut options = cap_std::fs::OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            let moved = self.open_nofollow(std::ffi::OsStr::new(&quarantine), &options)?;
            reject_download_hardlink(&moved)?;
            if download_file_object_id(&moved)? != *expected {
                return Err("Managed download artifact changed before cleanup".to_string());
            }
            drop(moved);
            self.dir
                .remove_file(&quarantine)
                .map_err(|error| format!("Failed to remove managed download artifact: {error}"))
        })();
        if result.is_err()
            && matches!(
                self.dir.symlink_metadata(name),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound
            )
        {
            let _ = self.dir.rename(&quarantine, &self.dir, name);
        }
        result.map(|_| true)
    }
}

fn validate_download_file_candidate(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata_is_link_like(&metadata) {
                return Err(format!(
                    "Download artifact must not be a symbolic link or reparse point: {}",
                    path.display()
                ));
            }
            if !metadata.is_file() {
                return Err(format!(
                    "Download artifact is not a file: {}",
                    path.display()
                ));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to inspect download artifact {}: {error}",
            path.display()
        )),
    }
}

fn validate_download_artifact_paths(
    save_dir: &Path,
    final_path: &Path,
    temp_path: &Path,
    metadata_path: &Path,
) -> Result<(), String> {
    let canonical_save_dir = std::fs::canonicalize(save_dir)
        .map_err(|error| format!("Failed to resolve download directory: {error}"))?;
    for path in [final_path, temp_path, metadata_path] {
        let parent = path
            .parent()
            .ok_or_else(|| "Download artifact has no parent directory".to_string())?;
        let canonical_parent = std::fs::canonicalize(parent).map_err(|error| {
            format!(
                "Failed to resolve download artifact parent {}: {error}",
                parent.display()
            )
        })?;
        if path_identity_key(&canonical_parent) != path_identity_key(&canonical_save_dir) {
            return Err(format!(
                "Download artifact parent {} escaped its managed directory {}",
                canonical_parent.display(),
                canonical_save_dir.display()
            ));
        }
        validate_download_file_candidate(path)?;
    }
    Ok(())
}

fn open_download_temp_file(
    directory: &DownloadDirectoryLease,
    final_path: &Path,
    temp_path: &Path,
    metadata_path: &Path,
    append: bool,
    existing_owned: bool,
) -> Result<std::fs::File, String> {
    directory.open_temp(final_path, temp_path, metadata_path, append, existing_owned)
}

fn replace_download_artifact(
    directory: &DownloadDirectoryLease,
    source: &Path,
    destination: &Path,
) -> Result<DownloadFileObjectId, String> {
    directory.replace(source, destination)
}

fn artifact_state_path(temp_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.json", temp_path.display()))
}

fn write_string_atomic(path: &Path, contents: &str) -> Result<(), String> {
    crate::persistence::atomic_write(path, contents.as_bytes(), None)
}

fn load_artifact_state(
    state: &AppState,
    task_id: &str,
) -> Result<Option<DownloadArtifactState>, String> {
    with_native_artifact_registry(state, |registry| {
        Ok(registry
            .partials
            .get(task_id)
            .and_then(|record| record.artifact_state.clone()))
    })
}

async fn save_artifact_state(
    state: &AppState,
    task_id: &str,
    artifact_state: &DownloadArtifactState,
) {
    let result = with_native_artifact_registry(state, |registry| {
        let record = registry
            .partials
            .get_mut(task_id)
            .ok_or_else(|| "Partial download ownership is not registered".to_string())?;
        if artifact_state.task_id != record.task_id
            || artifact_state.run_id != record.run_id
            || path_identity_key(Path::new(&artifact_state.temp_path))
                != path_identity_key(Path::new(&record.temp_path))
        {
            return Err("Partial download state does not match native ownership".to_string());
        }
        let previous = record.artifact_state.replace(artifact_state.clone());
        if let Err(error) = save_native_artifact_registry(state, registry) {
            registry
                .partials
                .get_mut(task_id)
                .expect("partial ownership still present")
                .artifact_state = previous;
            return Err(error);
        }
        Ok(())
    });
    if let Err(error) = result {
        eprintln!("Failed to persist native-owned download state: {error}");
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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
    crate::security::resolve_authorized_download_root(base_dir, &entry.save_dir)
}

fn validate_queue_entry(base_dir: &Path, entry: &PersistedQueueEntry) -> Result<(), String> {
    let _ = sanitize_repo_id(&entry.repo_id)?;
    let managed_root = queue_entry_managed_root(base_dir, entry)?;
    let repo_dir = queue_entry_download_dir(base_dir, entry)?;
    crate::security::ensure_download_path_within_root(&repo_dir, &managed_root)?;
    if entry.files.is_empty() {
        return Err("Download queue entry has no files".to_string());
    }
    for file in &entry.files {
        validate_managed_file(file)?;
        let destination = remote_parent_dir(&repo_dir, &file.path)?;
        crate::security::ensure_download_path_within_root(&destination, &managed_root)?;
    }
    Ok(())
}

fn validate_download_queue_budget(queue: &[PersistedQueueEntry]) -> Result<(), String> {
    if queue.len() > MAX_DOWNLOAD_STATE_ENTRIES {
        return Err(format!(
            "Download queue exceeds the {MAX_DOWNLOAD_STATE_ENTRIES}-entry limit"
        ));
    }
    let mut file_count = 0usize;
    for entry in queue {
        if entry.files.len() > MAX_DOWNLOAD_BATCH_FILES {
            return Err(format!(
                "Download queue entry exceeds the {MAX_DOWNLOAD_BATCH_FILES}-file limit"
            ));
        }
        file_count = file_count
            .checked_add(entry.files.len())
            .ok_or_else(|| "Download queue file count overflowed".to_string())?;
        if file_count > MAX_DOWNLOAD_STATE_FILES {
            return Err(format!(
                "Download queue exceeds the {MAX_DOWNLOAD_STATE_FILES}-file limit"
            ));
        }
        for (value, label) in [
            (&entry.id, "entry id"),
            (&entry.repo_id, "repository id"),
            (&entry.source, "source"),
            (&entry.save_dir, "save directory"),
            (&entry.status, "entry status"),
        ] {
            validate_repository_field(value, label)?;
        }
        if let Some(error) = &entry.last_error {
            validate_repository_field(error, "entry error")?;
        }
        for file in &entry.files {
            for (value, label) in [
                (&file.name, "file name"),
                (&file.path, "remote path"),
                (&file.file_type, "file type"),
            ] {
                validate_repository_field(value, label)?;
            }
            for (value, label) in [
                (file.task_id.as_deref(), "task id"),
                (file.run_id.as_deref(), "run id"),
                (file.status.as_deref(), "file status"),
                (file.error.as_deref(), "file error"),
            ] {
                if let Some(value) = value {
                    validate_repository_field(value, label)?;
                }
            }
        }
    }
    let encoded = serde_json::to_vec(&DownloadState {
        queue: queue.to_vec(),
    })
    .map_err(|error| format!("failed to validate download state size: {error}"))?;
    if encoded.len() as u64 > MAX_DOWNLOAD_STATE_BYTES {
        return Err(format!(
            "Download queue exceeds the {MAX_DOWNLOAD_STATE_BYTES}-byte persistence limit"
        ));
    }
    Ok(())
}

fn read_bounded_state_file(path: &Path, max_bytes: u64) -> Result<Option<String>, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to open download state: {error}")),
    };
    let metadata = file
        .metadata()
        .map_err(|error| format!("failed to inspect download state: {error}"))?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Err(format!(
            "download state is not a regular file within the {max_bytes}-byte limit"
        ));
    }
    reject_download_hardlink(&file)?;
    let mut json = String::new();
    std::io::Read::by_ref(&mut file)
        .take(max_bytes.saturating_add(1))
        .read_to_string(&mut json)
        .map_err(|error| format!("failed to read download state: {error}"))?;
    if json.len() as u64 > max_bytes {
        return Err(format!("download state exceeds the {max_bytes}-byte limit"));
    }
    Ok(Some(json))
}

fn download_artifact_registry_path(state: &AppState) -> PathBuf {
    state
        .config_dir
        .lock()
        .unwrap()
        .join("download_artifacts.json")
}

fn validate_native_artifact_registry(
    registry: &NativeDownloadArtifactRegistry,
) -> Result<(), String> {
    if registry
        .artifacts
        .len()
        .saturating_add(registry.partials.len())
        .saturating_add(registry.browse_grants.len())
        > MAX_DOWNLOAD_ARTIFACT_REGISTRY_ENTRIES
    {
        return Err(format!(
            "Download artifact registry exceeds the {MAX_DOWNLOAD_ARTIFACT_REGISTRY_ENTRIES}-entry limit"
        ));
    }
    for (task_id, record) in &registry.artifacts {
        if task_id != &record.task_id {
            return Err("Download artifact registry key does not match its task id".to_string());
        }
        for (value, label) in [
            (task_id.as_str(), "artifact task id"),
            (record.managed_root.as_str(), "artifact managed root"),
            (record.final_path.as_str(), "artifact final path"),
            (record.file_object_id.as_str(), "artifact file identity"),
        ] {
            validate_repository_field(value, label)?;
        }
        let root = Path::new(&record.managed_root);
        let path = Path::new(&record.final_path);
        if !root.is_absolute() || !path.is_absolute() || !path_is_within(path, root) {
            return Err("Download artifact registry contains an unsafe path".to_string());
        }
    }
    for (task_id, record) in &registry.partials {
        if task_id != &record.task_id {
            return Err("Download partial registry key does not match its task id".to_string());
        }
        for (value, label) in [
            (task_id.as_str(), "partial task id"),
            (record.run_id.as_str(), "partial run id"),
            (record.managed_root.as_str(), "partial managed root"),
            (record.temp_path.as_str(), "partial temp path"),
            (record.file_object_id.as_str(), "partial file identity"),
        ] {
            validate_repository_field(value, label)?;
        }
        let root = Path::new(&record.managed_root);
        let temp = Path::new(&record.temp_path);
        if !root.is_absolute() || !temp.is_absolute() || !path_is_within(temp, root) {
            return Err("Download partial registry contains an unsafe path".to_string());
        }
        if let Some(artifact) = &record.artifact_state {
            for (value, label) in [
                (artifact.task_id.as_str(), "partial state task id"),
                (artifact.run_id.as_str(), "partial state run id"),
                (artifact.repo_id.as_str(), "partial state repository id"),
                (artifact.source.as_str(), "partial state source"),
                (artifact.remote_path.as_str(), "partial state remote path"),
                (artifact.final_path.as_str(), "partial state final path"),
                (artifact.temp_path.as_str(), "partial state temp path"),
            ] {
                validate_repository_field(value, label)?;
            }
            if artifact.task_id != record.task_id
                || path_identity_key(Path::new(&artifact.temp_path))
                    != path_identity_key(Path::new(&record.temp_path))
            {
                return Err(
                    "Download partial state does not match its ownership record".to_string()
                );
            }
        }
    }
    for (grant_id, grant) in &registry.browse_grants {
        if grant_id != &grant.grant_id {
            return Err("Download browse grant key does not match its grant id".to_string());
        }
        for (value, label) in [
            (grant_id.as_str(), "browse grant id"),
            (grant.repo_id.as_str(), "browse grant repository id"),
            (grant.source.as_str(), "browse grant source"),
            (grant.remote_path.as_str(), "browse grant remote path"),
            (
                grant.immutable_revision.as_str(),
                "browse grant immutable revision",
            ),
        ] {
            validate_repository_field(value, label)?;
        }
        validate_immutable_revision(&grant.immutable_revision)?;
        if let Some(digest) = grant.expected_sha256.as_deref() {
            validate_expected_sha256(digest)?;
        }
        if grant.size > MAX_DOWNLOAD_FILE_BYTES {
            return Err("Download browse grant exceeds the maximum file size".to_string());
        }
    }
    Ok(())
}

fn load_native_artifact_registry(
    state: &AppState,
) -> Result<NativeDownloadArtifactRegistry, String> {
    let path = download_artifact_registry_path(state);
    let Some(json) = read_bounded_state_file(&path, MAX_DOWNLOAD_ARTIFACT_REGISTRY_BYTES)? else {
        return Ok(NativeDownloadArtifactRegistry::default());
    };
    let registry: NativeDownloadArtifactRegistry = serde_json::from_str(&json)
        .map_err(|error| format!("failed to parse download artifact registry: {error}"))?;
    validate_native_artifact_registry(&registry)?;
    Ok(registry)
}

fn save_native_artifact_registry(
    state: &AppState,
    registry: &NativeDownloadArtifactRegistry,
) -> Result<(), String> {
    validate_native_artifact_registry(registry)?;
    let json = serde_json::to_vec_pretty(registry)
        .map_err(|error| format!("failed to serialize download artifact registry: {error}"))?;
    if json.len() as u64 > MAX_DOWNLOAD_ARTIFACT_REGISTRY_BYTES {
        return Err(format!(
            "Download artifact registry exceeds the {MAX_DOWNLOAD_ARTIFACT_REGISTRY_BYTES}-byte limit"
        ));
    }
    crate::persistence::atomic_write(&download_artifact_registry_path(state), &json, None)
}

fn with_native_artifact_registry<R>(
    state: &AppState,
    operation: impl FnOnce(&mut NativeDownloadArtifactRegistry) -> Result<R, String>,
) -> Result<R, String> {
    let mut guard = state
        .download_artifact_registry
        .lock()
        .map_err(|_| "download artifact registry lock is poisoned".to_string())?;
    if guard.is_none() {
        *guard = Some(load_native_artifact_registry(state)?);
    }
    operation(guard.as_mut().expect("registry initialized"))
}

fn validate_immutable_revision(revision: &str) -> Result<(), String> {
    if !matches!(revision.len(), 40 | 64) || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Repository did not provide a full immutable commit revision".to_string());
    }
    Ok(())
}

fn normalize_expected_sha256(value: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let value = value.strip_prefix("sha256:").unwrap_or(value);
    validate_expected_sha256(value)?;
    Ok(Some(value.to_ascii_lowercase()))
}

fn validate_expected_sha256(value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Repository provided an invalid SHA-256 artifact digest".to_string());
    }
    Ok(())
}

fn issue_download_browse_grants(
    state: &AppState,
    source: &str,
    repo_id: &str,
    entries: Vec<(MsFileEntry, String, Option<String>)>,
) -> Result<Vec<MsFileEntry>, String> {
    if entries.len() > MAX_BROWSE_GRANTS_PER_RESPONSE {
        return Err(format!(
            "Repository exposes more than {MAX_BROWSE_GRANTS_PER_RESPONSE} downloadable artifacts"
        ));
    }
    let now = now_secs();
    let mut issued = Vec::with_capacity(entries.len());
    let mut grants = Vec::with_capacity(entries.len());
    for (mut entry, revision, digest) in entries {
        validate_immutable_revision(&revision)?;
        let digest = normalize_expected_sha256(digest.as_deref())?;
        let grant_id = uuid::Uuid::new_v4().to_string();
        entry.artifact_grant = Some(grant_id.clone());
        grants.push(NativeDownloadBrowseGrant {
            grant_id,
            repo_id: repo_id.to_string(),
            source: source.to_string(),
            remote_path: entry.path.clone(),
            immutable_revision: revision.to_ascii_lowercase(),
            expected_sha256: digest,
            size: entry.size,
            issued_at: now,
        });
        issued.push(entry);
    }
    with_native_artifact_registry(state, |registry| {
        let previous = registry.browse_grants.clone();
        registry.browse_grants.retain(|_, grant| {
            now.saturating_sub(grant.issued_at) <= DOWNLOAD_BROWSE_GRANT_LIFETIME_SECS
        });
        let non_grant_entries = registry
            .artifacts
            .len()
            .saturating_add(registry.partials.len());
        let available_grants = MAX_DOWNLOAD_ARTIFACT_REGISTRY_ENTRIES
            .checked_sub(non_grant_entries)
            .ok_or_else(|| "Download artifact registry has no browse-grant capacity".to_string())?;
        if grants.len() > available_grants {
            registry.browse_grants = previous;
            return Err(
                "Download artifact registry has insufficient browse-grant capacity".to_string(),
            );
        }
        let new_grant_ids = grants
            .iter()
            .map(|grant| grant.grant_id.clone())
            .collect::<std::collections::HashSet<_>>();
        for grant in grants {
            registry.browse_grants.insert(grant.grant_id.clone(), grant);
        }
        if registry.browse_grants.len() > available_grants {
            let mut oldest = registry
                .browse_grants
                .values()
                .filter(|grant| !new_grant_ids.contains(&grant.grant_id))
                .map(|grant| (grant.issued_at, grant.grant_id.clone()))
                .collect::<Vec<_>>();
            oldest.sort_unstable();
            let excess = registry.browse_grants.len() - available_grants;
            for (_, grant_id) in oldest.into_iter().take(excess) {
                registry.browse_grants.remove(&grant_id);
            }
        }
        if let Err(error) = save_native_artifact_registry(state, registry) {
            registry.browse_grants = previous;
            return Err(error);
        }
        Ok(())
    })?;
    Ok(issued)
}

#[derive(Debug, Clone)]
struct DownloadArtifactExpectation {
    immutable_revision: String,
    expected_sha256: Option<String>,
}

fn resolve_download_browse_grant(
    state: &AppState,
    source: &str,
    repo_id: &str,
    file: &MsFileEntry,
) -> Result<DownloadArtifactExpectation, String> {
    let grant_id = file
        .artifact_grant
        .as_deref()
        .ok_or_else(|| "Download selection is missing its native browse grant".to_string())?;
    let grant = with_native_artifact_registry(state, |registry| {
        Ok(registry.browse_grants.get(grant_id).cloned())
    })?
    .ok_or_else(|| "Download browse grant is unknown or no longer available".to_string())?;
    if now_secs().saturating_sub(grant.issued_at) > DOWNLOAD_BROWSE_GRANT_LIFETIME_SECS {
        return Err("Download browse grant expired; browse the repository again".to_string());
    }
    if grant.source != source
        || grant.repo_id != repo_id
        || grant.remote_path != file.path
        || grant.size != file.size
    {
        return Err("Download selection does not match its native browse grant".to_string());
    }
    validate_immutable_revision(&grant.immutable_revision)?;
    if let Some(digest) = grant.expected_sha256.as_deref() {
        validate_expected_sha256(digest)?;
    }
    Ok(DownloadArtifactExpectation {
        immutable_revision: grant.immutable_revision,
        expected_sha256: grant.expected_sha256,
    })
}

fn register_owned_download_artifact(
    state: &AppState,
    task_id: &str,
    managed_root: &Path,
    final_path: &Path,
    directory: &DownloadDirectoryLease,
    expected_identity: &DownloadFileObjectId,
) -> Result<NativeDownloadArtifactRecord, String> {
    validate_repository_field(task_id, "artifact task id")?;
    let canonical_root = std::fs::canonicalize(managed_root)
        .map_err(|error| format!("Failed to resolve managed download root: {error}"))?;
    let canonical_path = std::fs::canonicalize(final_path)
        .map_err(|error| format!("Failed to resolve completed download artifact: {error}"))?;
    if !path_is_within(&canonical_path, &canonical_root) {
        return Err("Completed download artifact escaped its managed root".to_string());
    }
    let (file_object_id, size) = directory.inspect_file(final_path)?;
    if &file_object_id != expected_identity {
        return Err("Completed download changed before ownership registration".to_string());
    }
    let record = NativeDownloadArtifactRecord {
        task_id: task_id.to_string(),
        managed_root: canonical_root.to_string_lossy().to_string(),
        final_path: canonical_path.to_string_lossy().to_string(),
        file_object_id: file_object_id.0,
        size,
    };
    with_native_artifact_registry(state, |registry| {
        let previous = registry
            .artifacts
            .insert(task_id.to_string(), record.clone());
        let previous_partial = registry.partials.remove(task_id);
        if let Err(error) = save_native_artifact_registry(state, registry) {
            match previous {
                Some(previous) => {
                    registry.artifacts.insert(task_id.to_string(), previous);
                }
                None => {
                    registry.artifacts.remove(task_id);
                }
            }
            if let Some(previous_partial) = previous_partial {
                registry
                    .partials
                    .insert(task_id.to_string(), previous_partial);
            }
            return Err(error);
        }
        Ok(record.clone())
    })
}

fn finalize_owned_download_artifact(
    state: &AppState,
    task_id: &str,
    managed_root: &Path,
    temp_path: &Path,
    final_path: &Path,
    directory: &DownloadDirectoryLease,
    expected_sha256: Option<&str>,
) -> Result<(), String> {
    if let Some(expected_sha256) = expected_sha256 {
        validate_expected_sha256(expected_sha256)?;
        let actual_sha256 = directory.sha256_file(temp_path)?;
        if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
            return Err(format!(
                "Downloaded artifact SHA-256 mismatch: expected {expected_sha256}, received {actual_sha256}"
            ));
        }
    }
    let identity = replace_download_artifact(directory, temp_path, final_path)?;
    if let Err(error) = register_owned_download_artifact(
        state,
        task_id,
        managed_root,
        final_path,
        directory,
        &identity,
    ) {
        let rollback = replace_download_artifact(directory, final_path, temp_path);
        return match rollback {
            Ok(_) => Err(error),
            Err(rollback_error) => Err(format!(
                "{error}; failed to restore native-owned partial after registration failure: {rollback_error}"
            )),
        };
    }
    Ok(())
}

fn find_owned_artifact_by_file(
    state: &AppState,
    final_path: &Path,
    file_object_id: &DownloadFileObjectId,
) -> Result<Option<NativeDownloadArtifactRecord>, String> {
    let path_key = path_identity_key(final_path);
    with_native_artifact_registry(state, |registry| {
        Ok(registry
            .artifacts
            .values()
            .find(|record| {
                path_identity_key(Path::new(&record.final_path)) == path_key
                    && record.file_object_id == file_object_id.0
            })
            .cloned())
    })
}

fn authorize_existing_download_partial(
    state: &AppState,
    task_id: &str,
    managed_root: &Path,
    temp_path: &Path,
    directory: &DownloadDirectoryLease,
) -> Result<bool, String> {
    let temp_exists = directory.exists(temp_path);
    if !temp_exists {
        return Ok(false);
    }
    let (identity, _) = directory.inspect_file(temp_path)?;
    let path_key = path_identity_key(temp_path);
    let root_key = path_identity_key(managed_root);
    with_native_artifact_registry(state, |registry| {
        let record = registry
            .partials
            .get(task_id)
            .ok_or_else(|| "Existing partial download is not native-owned".to_string())?;
        if path_identity_key(Path::new(&record.managed_root)) != root_key
            || path_identity_key(Path::new(&record.temp_path)) != path_key
            || record.file_object_id != identity.0
        {
            return Err(
                "Existing partial download no longer matches its native ownership record"
                    .to_string(),
            );
        }
        Ok(true)
    })
}

fn register_owned_download_partial(
    state: &AppState,
    task_id: &str,
    run_id: &str,
    managed_root: &Path,
    temp_path: &Path,
    directory: &DownloadDirectoryLease,
) -> Result<(), String> {
    validate_repository_field(task_id, "partial task id")?;
    validate_repository_field(run_id, "partial run id")?;
    let canonical_root = std::fs::canonicalize(managed_root)
        .map_err(|error| format!("Failed to resolve managed download root: {error}"))?;
    let canonical_temp = std::fs::canonicalize(temp_path)
        .map_err(|error| format!("Failed to resolve partial download: {error}"))?;
    if !path_is_within(&canonical_temp, &canonical_root) {
        return Err("Partial download escaped its managed root".to_string());
    }
    let (identity, _) = directory.inspect_file(temp_path)?;
    let mut record = NativeDownloadPartialRecord {
        task_id: task_id.to_string(),
        run_id: run_id.to_string(),
        managed_root: canonical_root.to_string_lossy().to_string(),
        temp_path: canonical_temp.to_string_lossy().to_string(),
        file_object_id: identity.0,
        artifact_state: None,
    };
    with_native_artifact_registry(state, |registry| {
        if registry.partials.iter().any(|(candidate_task, candidate)| {
            candidate_task != task_id
                && path_identity_key(Path::new(&candidate.temp_path))
                    == path_identity_key(&canonical_temp)
        }) {
            return Err(
                "Partial download destination is already owned by another task".to_string(),
            );
        }
        if let Some(previous) = registry.partials.get(task_id) {
            if previous.file_object_id != record.file_object_id
                || path_identity_key(Path::new(&previous.temp_path))
                    != path_identity_key(Path::new(&record.temp_path))
            {
                return Err("Partial download ownership changed before resume".to_string());
            }
            record.artifact_state = previous.artifact_state.clone();
        }
        let previous = registry.partials.insert(task_id.to_string(), record);
        if let Err(error) = save_native_artifact_registry(state, registry) {
            match previous {
                Some(previous) => {
                    registry.partials.insert(task_id.to_string(), previous);
                }
                None => {
                    registry.partials.remove(task_id);
                }
            }
            return Err(error);
        }
        Ok(())
    })
}

fn remove_owned_download_partial(
    state: &AppState,
    task_id: &str,
    expected_run_id: Option<&str>,
) -> Result<bool, String> {
    let record = with_native_artifact_registry(state, |registry| {
        Ok(registry.partials.get(task_id).cloned())
    })?;
    let Some(record) = record else {
        return Ok(false);
    };
    if expected_run_id.is_some_and(|run_id| run_id != record.run_id) {
        return Err("Partial download run identity does not match native ownership".to_string());
    }
    let root = PathBuf::from(&record.managed_root);
    let temp_path = PathBuf::from(&record.temp_path);
    let parent = temp_path
        .parent()
        .ok_or_else(|| "Registered partial download has no parent directory".to_string())?;
    let directory = DownloadDirectoryLease::open_within(parent, &root)?;
    if directory.exists(&temp_path) {
        let (identity, _) = directory.inspect_file(&temp_path)?;
        if identity.0 != record.file_object_id {
            return Err("Registered partial download changed and will not be deleted".to_string());
        }
        directory.remove_if_identity(&temp_path, &identity)?;
    }
    with_native_artifact_registry(state, |registry| {
        let removed = registry.partials.remove(task_id);
        if let Err(error) = save_native_artifact_registry(state, registry) {
            if let Some(removed) = removed {
                registry.partials.insert(task_id.to_string(), removed);
            }
            return Err(error);
        }
        Ok(true)
    })
}

fn remove_owned_download_artifact(state: &AppState, task_id: &str) -> Result<bool, String> {
    let record = with_native_artifact_registry(state, |registry| {
        Ok(registry.artifacts.get(task_id).cloned())
    })?;
    let Some(record) = record else {
        return Ok(false);
    };
    let root = PathBuf::from(&record.managed_root);
    let final_path = PathBuf::from(&record.final_path);
    let parent = final_path
        .parent()
        .ok_or_else(|| "Registered download artifact has no parent directory".to_string())?;
    let directory = DownloadDirectoryLease::open_within(parent, &root)?;
    if directory.exists(&final_path) {
        let (identity, size) = directory.inspect_file(&final_path)?;
        if identity.0 != record.file_object_id || size != record.size {
            return Err("Registered download artifact changed and will not be deleted".to_string());
        }
        directory.remove_if_identity(&final_path, &identity)?;
    }
    with_native_artifact_registry(state, |registry| {
        let removed = registry.artifacts.remove(task_id);
        if let Err(error) = save_native_artifact_registry(state, registry) {
            if let Some(removed) = removed {
                registry.artifacts.insert(task_id.to_string(), removed);
            }
            return Err(error);
        }
        Ok(true)
    })
}

#[cfg(test)]
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
            if path_is_within(path, root) {
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
    if !path_is_within(&canonical_ancestor, &canonical_root) {
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
) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), String> {
    let sanitized = sanitize_file_name(file_name)?;
    let mut trusted = None;
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
            let candidate = (managed_root, final_path, temp_path, metadata_path);
            if trusted
                .as_ref()
                .is_some_and(|current| current != &candidate)
            {
                return Err("Download task maps to more than one managed artifact".to_string());
            }
            trusted = Some(candidate);
        }
    }

    trusted.ok_or_else(|| "Download task not found".to_string())
}

fn registered_download_paths_for_task(
    entries: &[PersistedQueueEntry],
    base_dir: &Path,
    task_id: &str,
) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), String> {
    let mut trusted = None;
    for entry in entries {
        validate_queue_entry(base_dir, entry)?;
        for file in &entry.files {
            if file.task_id.as_deref() != Some(task_id) {
                continue;
            }
            let managed_root = queue_entry_managed_root(base_dir, entry)?;
            let dir = remote_parent_dir(&queue_entry_download_dir(base_dir, entry)?, &file.path)?;
            let paths = build_download_paths(&dir, &file.name);
            let candidate = (managed_root, paths.0, paths.1, paths.2);
            if trusted
                .as_ref()
                .is_some_and(|current| current != &candidate)
            {
                return Err("Download task maps to more than one managed artifact".to_string());
            }
            trusted = Some(candidate);
        }
    }
    trusted.ok_or_else(|| "Download task not found".to_string())
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
    immutable_revision: String,
    expected_sha256: Option<String>,
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

fn build_task_context(
    file: &MsFileEntry,
    repo_id: &str,
    source: &str,
    expectation: DownloadArtifactExpectation,
) -> DownloadTaskContext {
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
        immutable_revision: expectation.immutable_revision,
        expected_sha256: expectation.expected_sha256,
    }
}

fn resolve_repo_save_path(
    app: &tauri::AppHandle,
    save_dir: &str,
    repo_id: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let managed = app.state::<AppState>();
    let config_dir = managed.config_dir.lock().unwrap();
    let app_data_root = config_dir.parent().unwrap_or(Path::new("."));
    let base_path = crate::security::resolve_authorized_download_root(app_data_root, save_dir)?;
    let base_path = if Path::new(save_dir.trim()).is_relative() {
        let canonical_app_data = std::fs::canonicalize(app_data_root)
            .map_err(|error| format!("Failed to resolve app data directory: {error}"))?;
        let canonical_base = canonical_app_data.join(save_dir.trim());
        crate::security::create_download_directory_within_root(
            &canonical_app_data,
            &canonical_base,
        )?
    } else {
        base_path
    };
    let save_path = base_path.join(repo_id.replace('/', std::path::MAIN_SEPARATOR_STR));
    let save_path = crate::security::create_download_directory_within_root(&base_path, &save_path)?;
    Ok((base_path, save_path))
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
    managed_root: PathBuf,
    file_size: u64,
    app: tauri::AppHandle,
    error_flags: (Arc<AtomicBool>, Arc<AtomicBool>),
) {
    let (has_error, has_non_retryable_error) = error_flags;
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
    if file_size > MAX_DOWNLOAD_FILE_BYTES {
        has_error.store(true, Ordering::SeqCst);
        has_non_retryable_error.store(true, Ordering::SeqCst);
        ctx.emit(
            &app,
            "download-error",
            serde_json::json!({
                "error": format!("Download exceeds the {MAX_DOWNLOAD_FILE_BYTES}-byte application limit"),
                "retryable": false,
            }),
        );
        return;
    }
    let (final_path, temp_path, _metadata_path) = build_download_paths(&save_path, &file_name);
    let metadata_path = artifact_state_path(&temp_path);
    if let Err(error) =
        validate_download_artifact_paths(&save_path, &final_path, &temp_path, &metadata_path)
    {
        has_error.store(true, Ordering::SeqCst);
        has_non_retryable_error.store(true, Ordering::SeqCst);
        ctx.emit(
            &app,
            "download-error",
            serde_json::json!({ "error": error, "retryable": false }),
        );
        return;
    }
    let directory = match DownloadDirectoryLease::open_within(&save_path, &managed_root) {
        Ok(directory) => directory,
        Err(error) => {
            has_error.store(true, Ordering::SeqCst);
            has_non_retryable_error.store(true, Ordering::SeqCst);
            ctx.emit(
                &app,
                "download-error",
                serde_json::json!({ "error": error, "retryable": false }),
            );
            return;
        }
    };
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

    if directory.exists(&final_path) {
        has_error.store(true, Ordering::SeqCst);
        has_non_retryable_error.store(true, Ordering::SeqCst);
        ctx.emit(
            &app,
            "download-error",
            serde_json::json!({
                "error": "The final download destination already exists and will not be replaced",
                "retryable": false,
            }),
        );
        return;
    }
    let existing_owned = match authorize_existing_download_partial(
        &shared,
        &task_id,
        &managed_root,
        &temp_path,
        &directory,
    ) {
        Ok(owned) => owned,
        Err(error) => {
            has_error.store(true, Ordering::SeqCst);
            has_non_retryable_error.store(true, Ordering::SeqCst);
            ctx.emit(
                &app,
                "download-error",
                serde_json::json!({ "error": error, "retryable": false }),
            );
            return;
        }
    };
    let mut file = match open_download_temp_file(
        &directory,
        &final_path,
        &temp_path,
        &metadata_path,
        existing_owned,
        existing_owned,
    ) {
        Ok(file) => file,
        Err(error) => {
            has_error.store(true, Ordering::SeqCst);
            has_non_retryable_error.store(true, Ordering::SeqCst);
            ctx.emit(
                &app,
                "download-error",
                serde_json::json!({ "error": error, "retryable": false }),
            );
            return;
        }
    };
    if let Err(error) = register_owned_download_partial(
        &shared,
        &task_id,
        &run_id,
        &managed_root,
        &temp_path,
        &directory,
    ) {
        if !existing_owned {
            if let Ok(identity) = download_file_object_id(&file) {
                drop(file);
                let _ = directory.remove_if_identity(&temp_path, &identity);
            }
        }
        has_error.store(true, Ordering::SeqCst);
        has_non_retryable_error.store(true, Ordering::SeqCst);
        ctx.emit(
            &app,
            "download-error",
            serde_json::json!({ "error": error, "retryable": false }),
        );
        return;
    }

    let artifact = match load_artifact_state(&shared, &task_id) {
        Ok(artifact) => artifact,
        Err(error) => {
            drop(file);
            let _ = remove_owned_download_partial(&shared, &task_id, Some(&run_id));
            has_error.store(true, Ordering::SeqCst);
            has_non_retryable_error.store(true, Ordering::SeqCst);
            ctx.emit(
                &app,
                "download-error",
                serde_json::json!({ "error": error, "retryable": false }),
            );
            return;
        }
    };
    if artifact.as_ref().is_some_and(|artifact| {
        artifact.task_id != task_id
            || artifact.repo_id != repo_id
            || artifact.source != source
            || artifact.remote_path != remote_path
            || artifact.immutable_revision != ctx.immutable_revision
            || artifact.expected_sha256 != ctx.expected_sha256
            || path_identity_key(Path::new(&artifact.final_path)) != path_identity_key(&final_path)
            || path_identity_key(Path::new(&artifact.temp_path)) != path_identity_key(&temp_path)
    }) {
        drop(file);
        let _ = remove_owned_download_partial(&shared, &task_id, Some(&run_id));
        has_error.store(true, Ordering::SeqCst);
        has_non_retryable_error.store(true, Ordering::SeqCst);
        ctx.emit(
            &app,
            "download-error",
            serde_json::json!({
                "error": "Partial download metadata does not match the requested artifact",
                "retryable": false,
            }),
        );
        return;
    }
    let mut save_etag = artifact.as_ref().and_then(|a| a.etag.clone());
    let mut save_lm = artifact.as_ref().and_then(|a| a.last_modified.clone());
    let resume_from = artifact
        .as_ref()
        .map(|a| a.downloaded_size)
        .unwrap_or_else(|| directory.metadata_len(&temp_path).unwrap_or(0));

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
            drop(file);
            let _ = remove_owned_download_partial(&shared, &task_id, Some(&run_id));
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
            save_artifact_state(
                &shared,
                &task_id,
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
                    immutable_revision: ctx.immutable_revision.clone(),
                    expected_sha256: ctx.expected_sha256.clone(),
                    updated_at: now_secs(),
                },
            )
            .await;
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
        });
        if let Some(validator) = if_range {
            req = req.header(IF_RANGE, validator);
        }
    }

    let resp = match tokio::time::timeout(DOWNLOAD_RESPONSE_TIMEOUT, req.send()).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            save_artifact_state(
                &shared,
                &task_id,
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
                    immutable_revision: ctx.immutable_revision.clone(),
                    expected_sha256: ctx.expected_sha256.clone(),
                    updated_at: now_secs(),
                },
            )
            .await;
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
        &shared,
        &task_id,
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
            immutable_revision: ctx.immutable_revision.clone(),
            expected_sha256: ctx.expected_sha256.clone(),
            updated_at: now_secs(),
        },
    )
    .await;

    // A 416 is only a completion signal when local and remote sizes agree exactly.
    if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        let part_size = directory.metadata_len(&temp_path).unwrap_or(0);
        let remote_size = response_content_range(resp.headers()).and_then(|range| range.total);
        let exact_size = unsatisfied_range_is_complete(part_size, file_size, remote_size);
        if exact_size {
            drop(file);
            if let Err(error) = finalize_owned_download_artifact(
                &shared,
                &task_id,
                &managed_root,
                &temp_path,
                &final_path,
                &directory,
                ctx.expected_sha256.as_deref(),
            ) {
                has_error.store(true, Ordering::SeqCst);
                update_manager_file_state(
                    &shared,
                    &task_id,
                    FileStatePatch {
                        downloaded: Some(part_size),
                        size: Some(file_size),
                        version: Some(ctx.version),
                        status: Some("error".into()),
                        error: Some(Some(format!("Failed to finalize download: {error}"))),
                        ..Default::default()
                    },
                );
                ctx.emit(
                    &app,
                    "download-error",
                    serde_json::json!({ "error": error, "retryable": false }),
                );
                return;
            }
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
            drop(file);
            let _ = remove_owned_download_partial(&shared, &task_id, Some(&run_id));
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
            &shared,
            &task_id,
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
                immutable_revision: ctx.immutable_revision.clone(),
                expected_sha256: ctx.expected_sha256.clone(),
                updated_at: now_secs(),
            },
        )
        .await;
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
            &shared,
            &task_id,
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
                immutable_revision: ctx.immutable_revision.clone(),
                expected_sha256: ctx.expected_sha256.clone(),
                updated_at: now_secs(),
            },
        )
        .await;
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
            drop(file);
            let _ = remove_owned_download_partial(&shared, &task_id, Some(&run_id));
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
    if !is_partial && directory.exists(&temp_path) {
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
        let strong_etag_missing_or_changed = strong_resume_validator_missing_or_changed(
            old_state.etag.as_deref(),
            resp_etag.as_deref(),
        );
        let last_modified_changed = old_state.last_modified.is_some()
            && resp_last_modified.is_some()
            && resp_last_modified != old_state.last_modified;
        if (strong_etag_missing_or_changed || last_modified_changed) && directory.exists(&temp_path)
        {
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
            drop(file);
            let _ = remove_owned_download_partial(&shared, &task_id, Some(&run_id));
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
    let total = match response_total_size(resp.headers(), resume_from, file_size) {
        Ok(total) => total,
        Err(error) => {
            drop(file);
            let _ = remove_owned_download_partial(&shared, &task_id, Some(&run_id));
            has_error.store(true, Ordering::SeqCst);
            has_non_retryable_error.store(true, Ordering::SeqCst);
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
                serde_json::json!({ "error": error, "retryable": false }),
            );
            return;
        }
    };
    if resume_from > total {
        drop(file);
        let _ = remove_owned_download_partial(&shared, &task_id, Some(&run_id));
        has_error.store(true, Ordering::SeqCst);
        has_non_retryable_error.store(true, Ordering::SeqCst);
        ctx.emit(
            &app,
            "download-error",
            serde_json::json!({
                "error": "Existing partial download is larger than the accepted artifact size",
                "retryable": false,
            }),
        );
        return;
    }
    if let Err(error) = ensure_download_disk_budget(&save_path, total - resume_from) {
        has_error.store(true, Ordering::SeqCst);
        has_non_retryable_error.store(true, Ordering::SeqCst);
        ctx.emit(
            &app,
            "download-error",
            serde_json::json!({ "error": error, "retryable": false }),
        );
        return;
    }
    let mut downloaded = resume_from;
    let mut disk_budget_checkpoint = downloaded;
    let mut win_start = std::time::Instant::now();
    let mut win_bytes: u64 = 0;
    let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(1); // Emit immediately the first time.
    let mut last_artifact_save = std::time::Instant::now() - std::time::Duration::from_secs(2);

    if !is_partial {
        if let Err(error) = file
            .set_len(0)
            .and_then(|_| file.seek(SeekFrom::Start(0)).map(|_| ()))
        {
            has_error.store(true, Ordering::SeqCst);
            has_non_retryable_error.store(true, Ordering::SeqCst);
            ctx.emit(
                &app,
                "download-error",
                serde_json::json!({ "error": error.to_string(), "retryable": false }),
            );
            return;
        }
    }

    let mut stream = resp.bytes_stream();
    let mut last_received = std::time::Instant::now();
    let transfer_started = std::time::Instant::now();
    loop {
        if transfer_started.elapsed() >= DOWNLOAD_MAX_TRANSFER_LIFETIME {
            let message = "Download exceeded the 24-hour transfer lifetime".to_string();
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
                let _ = remove_owned_download_partial(&shared, &task_id, Some(&run_id));
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
                    &shared,
                    &task_id,
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
                        immutable_revision: ctx.immutable_revision.clone(),
                        expected_sha256: ctx.expected_sha256.clone(),
                        updated_at: now_secs(),
                    },
                )
                .await;
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
                let next_downloaded = match downloaded.checked_add(len) {
                    Some(next) if next <= total && next <= MAX_DOWNLOAD_FILE_BYTES => next,
                    _ => {
                        drop(file);
                        let _ = remove_owned_download_partial(&shared, &task_id, Some(&run_id));
                        has_error.store(true, Ordering::SeqCst);
                        has_non_retryable_error.store(true, Ordering::SeqCst);
                        let error = "Download response exceeded the accepted artifact size";
                        update_manager_file_state(
                            &shared,
                            &task_id,
                            FileStatePatch {
                                downloaded: Some(0),
                                size: Some(total),
                                version: Some(ctx.version),
                                status: Some("error".into()),
                                error: Some(Some(error.into())),
                                ..Default::default()
                            },
                        );
                        ctx.emit(
                            &app,
                            "download-error",
                            serde_json::json!({ "error": error, "retryable": false }),
                        );
                        return;
                    }
                };
                if next_downloaded.saturating_sub(disk_budget_checkpoint)
                    >= DOWNLOAD_DISK_RECHECK_BYTES
                {
                    if let Err(error) =
                        ensure_download_disk_budget(&save_path, total - next_downloaded)
                    {
                        has_error.store(true, Ordering::SeqCst);
                        has_non_retryable_error.store(true, Ordering::SeqCst);
                        update_manager_file_state(
                            &shared,
                            &task_id,
                            FileStatePatch {
                                downloaded: Some(downloaded),
                                size: Some(total),
                                version: Some(ctx.version),
                                status: Some("error".into()),
                                error: Some(Some(error.clone())),
                                ..Default::default()
                            },
                        );
                        ctx.emit(
                            &app,
                            "download-error",
                            serde_json::json!({ "error": error, "retryable": false }),
                        );
                        return;
                    }
                    disk_budget_checkpoint = next_downloaded;
                }
                throttle_download_bytes(&shared, len).await;
                if let Err(e) = file.write_all(&bytes) {
                    save_artifact_state(
                        &shared,
                        &task_id,
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
                            immutable_revision: ctx.immutable_revision.clone(),
                            expected_sha256: ctx.expected_sha256.clone(),
                            updated_at: now_secs(),
                        },
                    )
                    .await;
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
                downloaded = next_downloaded;
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
                            &shared,
                            &task_id,
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
                                immutable_revision: ctx.immutable_revision.clone(),
                                expected_sha256: ctx.expected_sha256.clone(),
                                updated_at: now_secs(),
                            },
                        )
                        .await;
                    }
                }
            }
            Err(e) => {
                save_artifact_state(
                    &shared,
                    &task_id,
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
                        immutable_revision: ctx.immutable_revision.clone(),
                        expected_sha256: ctx.expected_sha256.clone(),
                        updated_at: now_secs(),
                    },
                )
                .await;
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

    let actual_size = directory.metadata_len(&temp_path).unwrap_or(0);
    let authoritative_size = if file_size > 0 { file_size } else { total };
    if authoritative_size > 0 && actual_size != authoritative_size {
        let message =
            format!("Download ended at {actual_size} bytes, expected {authoritative_size} bytes");
        has_error.store(true, Ordering::SeqCst);
        save_artifact_state(
            &shared,
            &task_id,
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
                immutable_revision: ctx.immutable_revision.clone(),
                expected_sha256: ctx.expected_sha256.clone(),
                updated_at: now_secs(),
            },
        )
        .await;
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

    if let Err(e) = finalize_owned_download_artifact(
        &shared,
        &task_id,
        &managed_root,
        &temp_path,
        &final_path,
        &directory,
        ctx.expected_sha256.as_deref(),
    ) {
        save_artifact_state(
            &shared,
            &task_id,
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
                immutable_revision: ctx.immutable_revision.clone(),
                expected_sha256: ctx.expected_sha256.clone(),
                updated_at: now_secs(),
            },
        )
        .await;
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

fn validate_repository_field(value: &str, label: &str) -> Result<(), String> {
    if value.len() > MAX_REPOSITORY_FIELD_BYTES {
        return Err(format!("Repository {label} exceeds the field-size limit"));
    }
    Ok(())
}

async fn confirm_download_destruction(
    app: tauri::AppHandle,
    title: &'static str,
    message: String,
    approve_label: &'static str,
) -> Result<(), String> {
    let approved = tokio::task::spawn_blocking(move || {
        app.dialog()
            .message(message)
            .title(title)
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                approve_label.to_string(),
                "取消".to_string(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|error| format!("download deletion confirmation failed: {error}"))?;
    if approved {
        Ok(())
    } else {
        Err("Download deletion was not approved".to_string())
    }
}

async fn send_repository_metadata(url: &str) -> Result<reqwest::Response, String> {
    let _slot = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        REPOSITORY_BROWSE_SLOTS.acquire(),
    )
    .await
    .map_err(|_| "Repository metadata admission timed out".to_string())?
    .map_err(|_| "Repository metadata admission is unavailable".to_string())?;
    tokio::time::timeout(DOWNLOAD_RESPONSE_TIMEOUT, HTTP_CLIENT.get(url).send())
        .await
        .map_err(|_| "Timed out waiting for repository response headers".to_string())?
        .map_err(|error| format!("网络错误: {error}"))
}

pub async fn browse_modelscope(
    repo_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<MsFileEntry>, String> {
    let repo_id = sanitize_repo_id(&repo_id)?;
    let url = format!(
        "https://www.modelscope.cn/api/v1/models/{}/repo/files?Recursive=true&Revision=master",
        repo_id
    );
    let resp = send_repository_metadata(&url).await?;
    if !resp.status().is_success() {
        return Err(format!("仓库请求失败 (HTTP {})", resp.status()));
    }
    let (body, _) = bounded_http::collect_response(
        resp,
        MAX_REPOSITORY_METADATA_BYTES,
        std::time::Duration::from_secs(15),
        std::time::Duration::from_secs(60),
    )
    .await?;
    let body: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| format!("解析失败: {e}"))?;

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
    let listing_revision = body["Data"]["Revision"].as_str().map(str::to_string);
    if files.len() > MAX_REPOSITORY_ENTRIES {
        return Err(format!(
            "Repository listing exceeds the {MAX_REPOSITORY_ENTRIES}-entry limit"
        ));
    }
    let mut grant_entries = Vec::new();
    for file in files {
        if file.get("Type").and_then(|value| value.as_str()) != Some("blob") {
            continue;
        }
        let Some(name) = file.get("Name").and_then(|value| value.as_str()) else {
            continue;
        };
        let Some(path) = file.get("Path").and_then(|value| value.as_str()) else {
            continue;
        };
        validate_repository_field(name, "name")?;
        validate_repository_field(path, "path")?;
        if !name.ends_with(".gguf") && !name.ends_with(".txt") {
            continue;
        }
        let revision = file
            .get("Revision")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| listing_revision.clone())
            .ok_or_else(|| {
                format!("ModelScope did not provide an immutable revision for {path}")
            })?;
        let digest = ["Sha256", "SHA256", "sha256"]
            .iter()
            .find_map(|key| file.get(*key).and_then(|value| value.as_str()))
            .map(str::to_string);
        grant_entries.push((
            MsFileEntry {
                file_type: utils::classify_gguf_file(Path::new(name)).to_string(),
                name: name.to_string(),
                path: path.to_string(),
                size: file
                    .get("Size")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0),
                task_id: None,
                run_id: None,
                downloaded: None,
                version: None,
                status: None,
                error: None,
                artifact_grant: None,
            },
            revision,
            digest,
        ));
    }
    let mut result = issue_download_browse_grants(&state, "modelscope", &repo_id, grant_entries)?;

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
    let batch_size = validate_download_batch(&files)?;
    let (managed_root, save_path) = resolve_repo_save_path(&app, &save_dir, &repo_id)?;
    let _disk_reservation = reserve_download_disk_budget(&save_path, batch_size)?;

    let has_error = Arc::new(AtomicBool::new(false));
    let has_non_retryable_error = Arc::new(AtomicBool::new(false));
    let state = app.state::<AppState>();
    let mut jobs = Vec::with_capacity(files.len());
    for file in files {
        validate_managed_file(&file)?;
        let expectation = resolve_download_browse_grant(&state, "modelscope", &repo_id, &file)?;
        let mut url = reqwest::Url::parse(&format!(
            "https://modelscope.cn/api/v1/models/{repo_id}/repo"
        ))
        .map_err(|error| format!("Failed to build ModelScope download URL: {error}"))?;
        url.query_pairs_mut()
            .append_pair("Revision", &expectation.immutable_revision)
            .append_pair("FilePath", &file.path);
        let dest_dir = remote_parent_dir(&save_path, &file.path)?;
        let dest_dir =
            crate::security::create_download_directory_within_root(&save_path, &dest_dir)?;
        let ctx = build_task_context(&file, &repo_id, "modelscope", expectation);
        jobs.push((ctx, url.to_string(), dest_dir, file.size));
    }
    futures_util::stream::iter(jobs)
        .for_each_concurrent(DOWNLOAD_BATCH_WORKERS, |(ctx, url, dest_dir, file_size)| {
            let app = app.clone();
            let managed_root = managed_root.clone();
            let has_error = Arc::clone(&has_error);
            let has_non_retryable_error = Arc::clone(&has_non_retryable_error);
            async move {
                let _slot = acquire_global_download_slot(&app).await;
                download_single_file(
                    ctx,
                    url,
                    dest_dir,
                    managed_root,
                    file_size,
                    app,
                    (has_error, has_non_retryable_error),
                )
                .await;
            }
        })
        .await;
    if has_error.load(Ordering::SeqCst) {
        if has_non_retryable_error.load(Ordering::SeqCst) {
            return Err("Non-retryable download error".into());
        }
        return Err("Download completed with errors".into());
    }
    Ok(())
}

// Download controls.

fn resolve_registered_download_control_key(
    state: &AppState,
    task_id: &str,
    run_id: Option<&str>,
) -> Result<String, String> {
    const MAX_DOWNLOAD_ID_BYTES: usize = 128;
    if task_id.is_empty() || task_id.len() > MAX_DOWNLOAD_ID_BYTES {
        return Err("Download task identity is invalid".to_string());
    }
    if run_id.is_some_and(|value| value.is_empty() || value.len() > MAX_DOWNLOAD_ID_BYTES) {
        return Err("Download run identity is invalid".to_string());
    }
    let mut entries = load_download_state(state)?;
    entries.extend(state.download_queue.lock().unwrap().clone());
    entries.extend(
        state
            .download_active_entries
            .lock()
            .unwrap()
            .values()
            .cloned(),
    );
    let mut resolved = None;
    for file in entries.iter().flat_map(|entry| entry.files.iter()) {
        if file.task_id.as_deref() != Some(task_id) {
            continue;
        }
        if let Some(expected) = run_id {
            if file.run_id.as_deref() != Some(expected) {
                continue;
            }
        }
        let candidate = run_id
            .or(file.run_id.as_deref())
            .unwrap_or(task_id)
            .to_string();
        if resolved
            .as_ref()
            .is_some_and(|current: &String| current != &candidate)
        {
            return Err("Download task maps to multiple active runs".to_string());
        }
        resolved = Some(candidate);
    }
    resolved.ok_or_else(|| "Download task or run identity is unknown".to_string())
}

pub async fn cancel_file_download(
    task_id: String,
    run_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _scheduler = state
        .download_scheduler_lock
        .lock()
        .map_err(|_| "download scheduler lock is poisoned".to_string())?;
    let key = resolve_registered_download_control_key(&state, &task_id, run_id.as_deref())?;
    state.cancel_flags.lock().unwrap().insert(key, true);
    Ok(())
}

pub async fn pause_file_download(
    task_id: String,
    run_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let _scheduler = state
        .download_scheduler_lock
        .lock()
        .map_err(|_| "download scheduler lock is poisoned".to_string())?;
    let key = resolve_registered_download_control_key(&state, &task_id, run_id.as_deref())?;
    let mut cancel = state.cancel_flags.lock().unwrap();
    let mut pause = state.pause_flags.lock().unwrap();
    pause.insert(key.clone(), true);
    cancel.insert(key, true);
    Ok(())
}

pub async fn cancel_and_cleanup_download(
    task_id: String,
    file_name: String,
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
    let _ = trusted_download_cleanup_paths(
        &entries,
        &base_dir,
        &task_id,
        &file_name,
        run_id_for_match,
    )?;
    drop(scheduler);
    let confirmation_name = file_name.clone();
    let confirmation_task = task_id.clone();
    let dialog_app = app.clone();
    let approved = tokio::task::spawn_blocking(move || {
        dialog_app
            .dialog()
            .message(format!(
                "确认取消并永久清理下载制品？\n\n文件: {confirmation_name}\n任务: {confirmation_task}\n\n已下载的部分或完整文件将被删除。"
            ))
            .title("确认清理下载")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "取消并清理".to_string(),
                "返回".to_string(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|error| format!("download cleanup confirmation failed: {error}"))?;
    if !approved {
        return Err("Download cleanup was not approved".to_string());
    }
    let scheduler = state
        .download_scheduler_lock
        .lock()
        .map_err(|_| "download scheduler lock is poisoned".to_string())?;
    let key = run_id.clone().unwrap_or_else(|| task_id.clone());
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
    remove_owned_download_partial(&state, &task_id, run_id_for_match)?;
    if let Err(error) = remove_owned_download_artifact(&state, &task_id) {
        return Err(format!(
            "Failed to remove the registered completed artifact: {error}"
        ));
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
    #[serde(default)]
    oid: Option<String>,
}

#[derive(serde::Deserialize)]
struct HfRepoInfo {
    sha: String,
}

pub async fn browse_huggingface(
    repo_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<MsFileEntry>, String> {
    let repo_id = sanitize_repo_id(&repo_id)?;
    let info_url = format!("https://huggingface.co/api/models/{repo_id}");
    let info_response = send_repository_metadata(&info_url).await?;
    if !info_response.status().is_success() {
        return Err(format!("仓库未找到 (HTTP {})", info_response.status()));
    }
    let (info_body, _) = bounded_http::collect_response(
        info_response,
        MAX_REPOSITORY_METADATA_BYTES,
        std::time::Duration::from_secs(15),
        std::time::Duration::from_secs(60),
    )
    .await?;
    let info: HfRepoInfo =
        serde_json::from_slice(&info_body).map_err(|e| format!("解析仓库版本失败: {e}"))?;
    validate_immutable_revision(&info.sha)?;
    let immutable_revision = info.sha.to_ascii_lowercase();
    let url = format!(
        "https://huggingface.co/api/models/{}/tree/{}?recursive=true&expand=true",
        repo_id, immutable_revision
    );
    let resp = send_repository_metadata(&url).await?;
    if !resp.status().is_success() {
        return Err(format!("仓库未找到 (HTTP {})", resp.status()));
    }
    let (body, _) = bounded_http::collect_response(
        resp,
        MAX_REPOSITORY_METADATA_BYTES,
        std::time::Duration::from_secs(15),
        std::time::Duration::from_secs(60),
    )
    .await?;
    let entries: Vec<HfFileEntry> =
        serde_json::from_slice(&body).map_err(|e| format!("解析失败: {e}"))?;
    if entries.len() > MAX_REPOSITORY_ENTRIES {
        return Err(format!(
            "Repository listing exceeds the {MAX_REPOSITORY_ENTRIES}-entry limit"
        ));
    }

    let grant_entries: Vec<(MsFileEntry, String, Option<String>)> = entries
        .iter()
        .filter_map(|e| {
            if e.entry_type != "file" {
                return None;
            }
            let name = e.path.split('/').next_back()?.to_string();
            if validate_repository_field(&name, "name").is_err()
                || validate_repository_field(&e.path, "path").is_err()
            {
                return None;
            }
            if !name.ends_with(".gguf") && !name.ends_with(".txt") {
                return None;
            }
            let size = e.lfs.as_ref().map(|l| l.size).or(e.size).unwrap_or(0);
            Some((
                MsFileEntry {
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
                    artifact_grant: None,
                },
                immutable_revision.clone(),
                e.lfs.as_ref().and_then(|lfs| lfs.oid.clone()),
            ))
        })
        .collect();
    let mut result = issue_download_browse_grants(&state, "huggingface", &repo_id, grant_entries)?;

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
    let batch_size = validate_download_batch(&files)?;
    let (managed_root, save_path) = resolve_repo_save_path(&app, &save_dir, &repo_id)?;
    let _disk_reservation = reserve_download_disk_budget(&save_path, batch_size)?;

    let has_error = Arc::new(AtomicBool::new(false));
    let has_non_retryable_error = Arc::new(AtomicBool::new(false));
    let state = app.state::<AppState>();
    let mut jobs = Vec::with_capacity(files.len());
    for file in files {
        validate_managed_file(&file)?;
        let expectation = resolve_download_browse_grant(&state, "huggingface", &repo_id, &file)?;
        let encoded_path = percent_encode_path(&file.path)?;
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            repo_id, expectation.immutable_revision, encoded_path
        );
        let dest_dir = remote_parent_dir(&save_path, &file.path)?;
        let dest_dir =
            crate::security::create_download_directory_within_root(&save_path, &dest_dir)?;
        let ctx = build_task_context(&file, &repo_id, "huggingface", expectation);
        jobs.push((ctx, url, dest_dir, file.size));
    }
    futures_util::stream::iter(jobs)
        .for_each_concurrent(DOWNLOAD_BATCH_WORKERS, |(ctx, url, dest_dir, file_size)| {
            let app = app.clone();
            let managed_root = managed_root.clone();
            let has_error = Arc::clone(&has_error);
            let has_non_retryable_error = Arc::clone(&has_non_retryable_error);
            async move {
                let _slot = acquire_global_download_slot(&app).await;
                download_single_file(
                    ctx,
                    url,
                    dest_dir,
                    managed_root,
                    file_size,
                    app,
                    (has_error, has_non_retryable_error),
                )
                .await;
            }
        })
        .await;
    if has_error.load(Ordering::SeqCst) {
        if has_non_retryable_error.load(Ordering::SeqCst) {
            return Err("Non-retryable download error".into());
        }
        return Err("Download completed with errors".into());
    }
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedLocalFileResult {
    pub task_id: Option<String>,
    pub size: u64,
    pub manager_owned: bool,
}

/// Resolve and inspect a file only through native-managed repository operands.
/// The renderer never supplies a raw filesystem path to this command.
pub async fn check_local_file(
    save_dir: String,
    repo_id: String,
    remote_path: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<ManagedLocalFileResult>, String> {
    let repo_id = sanitize_repo_id(&repo_id)?;
    let file_name = remote_path
        .split('/')
        .next_back()
        .ok_or_else(|| "Remote file path has no file name".to_string())?;
    let file_name = sanitize_file_name(file_name)?;
    let base_dir = state
        .config_dir
        .lock()
        .unwrap()
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let managed_root = crate::security::resolve_authorized_download_root(&base_dir, &save_dir)?;
    if !managed_root.exists() {
        return Ok(None);
    }
    let repo_dir = managed_root.join(repo_id.replace('/', std::path::MAIN_SEPARATOR_STR));
    crate::security::ensure_download_path_within_root(&repo_dir, &managed_root)?;
    let parent = remote_parent_dir(&repo_dir, &remote_path)?;
    if !parent.exists() {
        return Ok(None);
    }
    let final_path = parent.join(file_name);
    let directory = DownloadDirectoryLease::open_within(&parent, &managed_root)?;
    if !directory.exists(&final_path) {
        return Ok(None);
    }
    let (identity, size) = directory.inspect_file(&final_path)?;
    let record = find_owned_artifact_by_file(&state, &final_path, &identity)?;
    Ok(Some(ManagedLocalFileResult {
        task_id: record.as_ref().map(|record| record.task_id.clone()),
        size,
        manager_owned: record.is_some(),
    }))
}

pub async fn delete_managed_local_file(
    task_id: String,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let record = with_native_artifact_registry(&state, |registry| {
        Ok(registry.artifacts.get(&task_id).cloned())
    })?
    .ok_or_else(|| "No native-owned completed download is registered for this task".to_string())?;
    confirm_download_destruction(
        app,
        "确认删除下载制品",
        format!(
            "确认永久删除已验证下载制品？\n\n文件: {}\n任务: {}\n大小: {} 字节\n\n此操作无法撤销。",
            record.final_path, record.task_id, record.size
        ),
        "永久删除",
    )
    .await?;
    if !remove_owned_download_artifact(&state, &task_id)? {
        return Err("No native-owned completed download is registered for this task".to_string());
    }
    remove_manager_file(&state, &task_id)?;
    Ok(())
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

fn download_file_identity(
    base_dir: &Path,
    entry: &PersistedQueueEntry,
    file: &MsFileEntry,
) -> Result<String, String> {
    validate_managed_file(file)?;
    let repo_dir = queue_entry_download_dir(base_dir, entry)?;
    let destination = remote_parent_dir(&repo_dir, &file.path)?.join(&file.name);
    Ok(path_identity_key(&destination))
}

fn file_can_write(entry: &PersistedQueueEntry, file: &MsFileEntry) -> bool {
    matches!(
        file.status.as_deref().unwrap_or(entry.status.as_str()),
        "" | "queued" | "active" | "paused" | "pausing"
    )
}

fn conflicting_download_identity(
    base_dir: &Path,
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
                .filter_map(|file| download_file_identity(base_dir, entry, file).ok())
        })
        .collect::<std::collections::HashSet<_>>();
    candidate
        .files
        .iter()
        .filter_map(|file| download_file_identity(base_dir, candidate, file).ok())
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
        && entry.added_at > 0
        && now_secs().saturating_sub(entry.added_at) <= MAX_DOWNLOAD_RETRY_LIFETIME_SECS
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
                    retry_entry.retries = retry_entry
                        .retries
                        .checked_add(1)
                        .unwrap_or(MAX_NATIVE_DOWNLOAD_RETRIES)
                        .min(MAX_NATIVE_DOWNLOAD_RETRIES);
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
    let mut queue = queue.to_vec();
    for entry in &mut queue {
        normalize_download_retry_state(entry);
    }
    validate_download_queue_budget(&queue)?;
    let path = download_state_path(state);
    let ds = DownloadState { queue };
    let json = serde_json::to_vec_pretty(&ds)
        .map_err(|error| format!("failed to serialize download state: {error}"))?;
    crate::persistence::atomic_write(&path, &json, None)
}

fn load_download_state_unlocked(state: &AppState) -> Result<Vec<PersistedQueueEntry>, String> {
    let path = download_state_path(state);
    let Some(json) = read_bounded_state_file(&path, MAX_DOWNLOAD_STATE_BYTES)? else {
        return Ok(Vec::new());
    };
    let mut queue = serde_json::from_str::<DownloadState>(&json)
        .map(|state| state.queue)
        .map_err(|error| format!("failed to parse download state: {error}"))?;
    for entry in &mut queue {
        normalize_download_retry_state(entry);
    }
    validate_download_queue_budget(&queue)?;
    Ok(queue)
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
    validate_download_queue_budget(inflight)?;
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
    let Some(json) = read_bounded_state_file(&path, MAX_DOWNLOAD_STATE_BYTES)? else {
        return Ok(Vec::new());
    };
    let queue = serde_json::from_str::<DownloadState>(&json)
        .map(|state| state.queue)
        .map_err(|error| format!("failed to parse inflight download state: {error}"))?;
    validate_download_queue_budget(&queue)?;
    Ok(queue)
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
    queue.retain(|entry| match validate_queue_entry(&save_dir_base, entry) {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "Discarding unsafe persisted download entry {}: {error}",
                entry.id
            );
            false
        }
    });

    for entry in queue.iter_mut() {
        let managed_root = match queue_entry_managed_root(&save_dir_base, entry) {
            Ok(root) => root,
            Err(_) => continue,
        };
        prepare_restored_entry(entry, auto_resume);
        let repo_dir = save_dir_base
            .join(&entry.save_dir)
            .join(entry.repo_id.replace('/', std::path::MAIN_SEPARATOR_STR));
        for file in entry.files.iter_mut() {
            let file_dir =
                remote_parent_dir(&repo_dir, &file.path).unwrap_or_else(|_| repo_dir.clone());
            let (final_path, temp_path, _) = build_download_paths(&file_dir, &file.name);
            if temp_path.exists() {
                let artifact = DownloadDirectoryLease::open_within(&file_dir, &managed_root)
                    .ok()
                    .and_then(|directory| {
                        let metadata_path = artifact_state_path(&temp_path);
                        directory
                            .read_to_string_bounded(&metadata_path, MAX_ARTIFACT_STATE_BYTES)
                            .ok()
                    })
                    .and_then(|contents| {
                        serde_json::from_str::<DownloadArtifactState>(&contents).ok()
                    });
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
    mut queue: Vec<PersistedQueueEntry>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    for entry in &mut queue {
        initialize_download_retry_state(entry);
    }
    validate_download_queue_budget(&queue)?;
    let base_dir = state
        .config_dir
        .lock()
        .unwrap()
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    for entry in &queue {
        validate_queue_entry(&base_dir, entry)?;
    }
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
    initialize_download_retry_state(&mut entry);
    entry.status = "queued".into();
    for file in entry.files.iter_mut() {
        if file.version.is_none() {
            file.version = Some(0);
        }
        file.status = Some("queued".into());
        file.error = None;
    }
    let base_dir = state
        .config_dir
        .lock()
        .unwrap()
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    validate_queue_entry(&base_dir, &entry)?;
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
    if let Some(identity) = conflicting_download_identity(&base_dir, &entry, &existing) {
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

    let (repo_id, source, save_dir, _retries, _max_retries) = target_meta;
    let identity = refresh_download_file_identity(&mut file);

    let runtime_entry = PersistedQueueEntry {
        id: uuid::Uuid::new_v4().to_string(),
        repo_id,
        source,
        files: vec![file],
        save_dir,
        added_at: now_secs(),
        status: "queued".into(),
        retries: 0,
        max_retries: MAX_NATIVE_DOWNLOAD_RETRIES,
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

    fn download_test_state(directory: &Path) -> AppState {
        let config = crate::commands::config::default_global_config();
        AppState::from_global_config(
            directory.to_path_buf(),
            &config,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    fn browsed_test_file(path: &str, size: u64) -> MsFileEntry {
        MsFileEntry {
            name: path.split('/').next_back().unwrap().to_string(),
            path: path.to_string(),
            size,
            file_type: "model".into(),
            task_id: None,
            run_id: None,
            downloaded: None,
            version: None,
            status: None,
            error: None,
            artifact_grant: None,
        }
    }

    #[test]
    fn native_browse_grant_binds_provider_revision_digest_path_and_size() {
        let directory =
            std::env::temp_dir().join(format!("lsm-download-grant-{}", uuid::Uuid::new_v4()));
        crate::persistence::enforce_private_directory(&directory).unwrap();
        let state = download_test_state(&directory);
        let digest = "ab".repeat(32);
        let mut issued = issue_download_browse_grants(
            &state,
            "huggingface",
            "owner/repo",
            vec![(
                browsed_test_file("models/model.gguf", 42),
                "1".repeat(40),
                Some(format!("sha256:{digest}")),
            )],
        )
        .unwrap();
        let file = issued.pop().unwrap();
        let expectation =
            resolve_download_browse_grant(&state, "huggingface", "owner/repo", &file).unwrap();
        assert_eq!(expectation.immutable_revision, "1".repeat(40));
        assert_eq!(
            expectation.expected_sha256.as_deref(),
            Some(digest.as_str())
        );

        let mut tampered = file.clone();
        tampered.path = "models/other.gguf".into();
        assert!(
            resolve_download_browse_grant(&state, "huggingface", "owner/repo", &tampered).is_err()
        );
        let mut forged = file;
        forged.artifact_grant = Some(uuid::Uuid::new_v4().to_string());
        assert!(
            resolve_download_browse_grant(&state, "huggingface", "owner/repo", &forged).is_err()
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn digest_mismatch_blocks_download_finalization() {
        let directory =
            std::env::temp_dir().join(format!("lsm-download-digest-{}", uuid::Uuid::new_v4()));
        crate::persistence::enforce_private_directory(&directory).unwrap();
        let state_dir = directory.join("state");
        crate::persistence::enforce_private_directory(&state_dir).unwrap();
        let state = download_test_state(&state_dir);
        let temp_path = directory.join("model.gguf.part");
        let final_path = directory.join("model.gguf");
        std::fs::write(&temp_path, b"abc").unwrap();
        crate::persistence::enforce_private_file(&temp_path).unwrap();
        let lease = DownloadDirectoryLease::open_within(&directory, &directory).unwrap();
        let error = finalize_owned_download_artifact(
            &state,
            "digest-test",
            &directory,
            &temp_path,
            &final_path,
            &lease,
            Some(&"00".repeat(32)),
        )
        .unwrap_err();
        assert!(error.contains("SHA-256 mismatch"));
        assert!(temp_path.exists());
        assert!(!final_path.exists());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn resumed_transfer_requires_a_persisted_strong_etag_to_reappear() {
        assert!(strong_resume_validator_missing_or_changed(
            Some("\"v1\""),
            None
        ));
        assert!(strong_resume_validator_missing_or_changed(
            Some("\"v1\""),
            Some("\"v2\"")
        ));
        assert!(!strong_resume_validator_missing_or_changed(
            Some("\"v1\""),
            Some("\"v1\"")
        ));
        assert!(!strong_resume_validator_missing_or_changed(
            Some("W/\"v1\""),
            None
        ));
    }

    #[test]
    fn download_batch_rejects_unknown_file_sizes_before_admission() {
        let file = MsFileEntry {
            name: "unknown.gguf".into(),
            path: "unknown.gguf".into(),
            size: 0,
            file_type: "file".into(),
            downloaded: None,
            status: None,
            error: None,
            artifact_grant: None,
            task_id: None,
            run_id: None,
            version: None,
        };
        assert!(validate_download_batch(&[file])
            .unwrap_err()
            .contains("unknown size"));
    }

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
                artifact_grant: None,
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
                artifact_grant: None,
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
            added_at: now_secs(),
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
                artifact_grant: None,
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
                artifact_grant: None,
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
            added_at: now_secs(),
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
                artifact_grant: None,
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
            artifact_grant: None,
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
    fn trusted_download_cleanup_path_is_derived_from_registered_task() {
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
                artifact_grant: None,
                task_id: Some("task-1".into()),
                run_id: Some("run-1".into()),
                version: Some(1),
            }],
        };
        let base = Path::new("/app-data");

        let (_, final_path, temp_path, metadata_path) =
            trusted_download_cleanup_paths(&[entry], base, "task-1", "model.gguf", Some("run-1"))
                .unwrap();

        assert!(paths_equal(
            &final_path,
            Path::new("/app-data/models/repo/model/model.gguf")
        ));
        assert_eq!(
            temp_path,
            Path::new("/app-data/models/repo/model/model.gguf.part")
        );
        assert_eq!(
            metadata_path,
            Path::new("/app-data/models/repo/model/model.gguf.part.json")
        );
    }

    #[test]
    fn remote_paths_reject_windows_drive_prefixes_on_every_platform() {
        let root = Path::new("/managed/org/model");
        assert!(remote_parent_dir(root, "C:/outside/model.gguf").is_err());
        assert!(remote_parent_dir(root, "nested/D:/outside/model.gguf").is_err());
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
    fn security_regression_remote_parent_rejects_existing_directory_link_escape() {
        let nonce = uuid::Uuid::new_v4();
        let base = std::env::temp_dir().join(format!("lsm-download-link-{nonce}"));
        let root = base.join("managed");
        let outside = base.join("outside");
        let linked = root.join("linked");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &linked).unwrap();
        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&linked)
                .arg(&outside)
                .status()
                .unwrap();
            assert!(status.success(), "failed to create test directory junction");
        }

        let result = remote_parent_dir(&root, "linked/model.gguf");

        #[cfg(unix)]
        let _ = std::fs::remove_file(&linked);
        #[cfg(windows)]
        let _ = std::fs::remove_dir(&linked);
        let _ = std::fs::remove_dir_all(&base);

        assert!(
            result.is_err(),
            "an existing directory link must not redirect a managed download outside its root"
        );
    }

    #[test]
    fn security_regression_part_and_final_artifact_links_are_rejected() {
        let base = std::env::temp_dir().join(format!(
            "lsm-download-artifact-link-{}",
            uuid::Uuid::new_v4()
        ));
        let save_dir = base.join("managed");
        let outside = base.join("outside");
        std::fs::create_dir_all(&save_dir).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let (final_path, temp_path, metadata_path) = build_download_paths(&save_dir, "model.gguf");

        #[cfg(unix)]
        {
            let outside_file = outside.join("target");
            std::fs::write(&outside_file, b"outside").unwrap();
            std::os::unix::fs::symlink(&outside_file, &temp_path).unwrap();
        }
        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&temp_path)
                .arg(&outside)
                .status()
                .unwrap();
            assert!(status.success(), "failed to create test artifact junction");
        }

        let part_result =
            validate_download_artifact_paths(&save_dir, &final_path, &temp_path, &metadata_path);

        #[cfg(unix)]
        let _ = std::fs::remove_file(&temp_path);
        #[cfg(windows)]
        let _ = std::fs::remove_dir(&temp_path);

        #[cfg(unix)]
        {
            let outside_file = outside.join("target");
            std::os::unix::fs::symlink(&outside_file, &final_path).unwrap();
        }
        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&final_path)
                .arg(&outside)
                .status()
                .unwrap();
            assert!(status.success(), "failed to create test final junction");
        }
        let final_result =
            validate_download_artifact_paths(&save_dir, &final_path, &temp_path, &metadata_path);

        #[cfg(unix)]
        let _ = std::fs::remove_file(&final_path);
        #[cfg(windows)]
        let _ = std::fs::remove_dir(&final_path);
        let _ = std::fs::remove_dir_all(&base);

        assert!(part_result.is_err());
        assert!(final_result.is_err());
    }

    #[test]
    fn bound_download_directory_cannot_be_redirected_after_authorization() {
        let base = std::env::temp_dir().join(format!(
            "lsm-download-bound-directory-{}",
            uuid::Uuid::new_v4()
        ));
        let root = base.join("managed");
        let download_dir = root.join("repo");
        let anchored_dir = root.join("repo-anchored");
        std::fs::create_dir_all(&download_dir).unwrap();
        let directory = DownloadDirectoryLease::open_within(&download_dir, &root).unwrap();
        let metadata_path = download_dir.join("model.gguf.part.json");

        let renamed = std::fs::rename(&download_dir, &anchored_dir).is_ok();
        if renamed {
            std::fs::create_dir_all(&download_dir).unwrap();
        }
        directory.write_atomic(&metadata_path, b"bound").unwrap();

        if renamed {
            assert_eq!(
                std::fs::read(anchored_dir.join("model.gguf.part.json")).unwrap(),
                b"bound"
            );
            assert!(!download_dir.join("model.gguf.part.json").exists());
        } else {
            assert_eq!(std::fs::read(&metadata_path).unwrap(), b"bound");
        }
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn bound_download_rejects_existing_hardlinked_part_file() {
        let base =
            std::env::temp_dir().join(format!("lsm-download-hardlink-{}", uuid::Uuid::new_v4()));
        let root = base.join("managed");
        let download_dir = root.join("repo");
        std::fs::create_dir_all(&download_dir).unwrap();
        let outside = base.join("outside-target");
        std::fs::write(&outside, b"outside").unwrap();
        let (final_path, temp_path, metadata_path) =
            build_download_paths(&download_dir, "model.gguf");
        std::fs::hard_link(&outside, &temp_path).unwrap();
        let directory = DownloadDirectoryLease::open_within(&download_dir, &root).unwrap();

        let result = directory.open_temp(&final_path, &temp_path, &metadata_path, false, false);

        assert!(result.is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn unregistered_partial_file_is_never_adopted() {
        let base =
            std::env::temp_dir().join(format!("lsm-download-unowned-{}", uuid::Uuid::new_v4()));
        let root = base.join("managed");
        let download_dir = root.join("repo");
        std::fs::create_dir_all(&download_dir).unwrap();
        let (final_path, temp_path, metadata_path) =
            build_download_paths(&download_dir, "model.gguf");
        std::fs::write(&temp_path, b"unowned").unwrap();
        let directory = DownloadDirectoryLease::open_within(&download_dir, &root).unwrap();

        let result = directory.open_temp(&final_path, &temp_path, &metadata_path, false, false);

        assert!(result.is_err());
        assert_eq!(std::fs::read(&temp_path).unwrap(), b"unowned");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn download_finalization_never_replaces_an_existing_destination() {
        let base = std::env::temp_dir().join(format!(
            "lsm-download-final-collision-{}",
            uuid::Uuid::new_v4()
        ));
        let root = base.join("managed");
        let download_dir = root.join("repo");
        std::fs::create_dir_all(&download_dir).unwrap();
        let (final_path, temp_path, _) = build_download_paths(&download_dir, "model.gguf");
        std::fs::write(&temp_path, b"downloaded").unwrap();
        std::fs::write(&final_path, b"preexisting").unwrap();
        let directory = DownloadDirectoryLease::open_within(&download_dir, &root).unwrap();

        let result = directory.replace(&temp_path, &final_path);

        assert!(result.is_err());
        assert_eq!(std::fs::read(&final_path).unwrap(), b"preexisting");
        assert_eq!(std::fs::read(&temp_path).unwrap(), b"downloaded");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn relative_registered_cleanup_path_is_resolved_from_the_managed_base() {
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
                artifact_grant: None,
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
        )
        .unwrap();

        assert!(paths_equal(
            &final_path,
            Path::new("/app-data/models/org/model/weights/model.gguf")
        ));
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
    fn absolute_save_directory_requires_an_explicit_native_grant() {
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
                artifact_grant: None,
                task_id: Some("task-absolute".into()),
                run_id: Some("run-absolute".into()),
                version: Some(1),
            }],
        };

        let result = trusted_download_cleanup_paths(
            &[entry],
            Path::new("/ignored-base"),
            "task-absolute",
            "model.gguf",
            Some("run-absolute"),
        );

        assert!(result.unwrap_err().contains("绝对下载目录未获授权"));
        let _ = std::fs::remove_dir_all(root);
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
    fn managed_cleanup_rejects_missing_sibling_with_shared_prefix() {
        let base = std::env::temp_dir().join(format!(
            "lsm-download-cleanup-boundary-{}",
            uuid::Uuid::new_v4()
        ));
        let root = base.join("models");
        let sibling = base.join("models-old").join("model.gguf");
        let _ = std::fs::remove_dir_all(&base);

        assert!(verified_managed_cleanup_path(&root, &sibling).is_err());
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
            artifact_grant: None,
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
            artifact_grant: None,
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
                artifact_grant: None,
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
            artifact_grant: None,
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
                artifact_grant: None,
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

        assert_eq!(response_total_size(&headers, 10, 100), Ok(100));
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
            artifact_grant: None,
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
            download_file_identity(Path::new("/app-data"), &entry, &file).unwrap(),
            download_file_identity(Path::new("/app-data"), &duplicate, &duplicate.files[0])
                .unwrap()
        );
        assert!(conflicting_download_identity(
            Path::new("/app-data"),
            &duplicate,
            std::slice::from_ref(&entry),
        )
        .is_some());
    }

    #[cfg(windows)]
    #[test]
    fn artifact_identity_tracks_the_resolved_windows_destination() {
        let file = MsFileEntry {
            name: "Model.gguf".into(),
            path: "Weights/Model.gguf".into(),
            size: 100,
            file_type: "file".into(),
            downloaded: None,
            status: Some("queued".into()),
            error: None,
            artifact_grant: None,
            task_id: None,
            run_id: None,
            version: None,
        };
        let entry = PersistedQueueEntry {
            id: "entry-1".into(),
            repo_id: "Org/Model".into(),
            source: "huggingface".into(),
            save_dir: "models".into(),
            added_at: 1,
            status: "queued".into(),
            retries: 0,
            max_retries: 3,
            last_error: None,
            files: vec![file.clone()],
        };
        let mut save_dir_alias = entry.clone();
        save_dir_alias.save_dir = "Models".into();
        assert_eq!(
            download_file_identity(Path::new(r"C:\AppData"), &entry, &file).unwrap(),
            download_file_identity(Path::new(r"c:\appdata"), &save_dir_alias, &file).unwrap()
        );

        let mut remote_case_variant = entry.clone();
        remote_case_variant.files[0].path = "weights/Model.gguf".into();
        assert_eq!(
            download_file_identity(Path::new(r"C:\AppData"), &entry, &file).unwrap(),
            download_file_identity(
                Path::new(r"C:\AppData"),
                &remote_case_variant,
                &remote_case_variant.files[0]
            )
            .unwrap()
        );
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
            artifact_grant: None,
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
            download_file_identity(Path::new("/app-data"), &first, &first.files[0]).unwrap(),
            download_file_identity(Path::new("/app-data"), &second, &second.files[0]).unwrap()
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
            artifact_grant: None,
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
            artifact_grant: None,
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

    for entry in &affected_entries {
        for file in &entry.files {
            if matches!(file.status.as_deref(), Some("completed" | "error")) {
                continue;
            }
            if !file
                .run_id
                .as_ref()
                .is_some_and(|run_id| active_run_ids.contains(run_id))
            {
                if let Some(task_id) = file.task_id.as_deref() {
                    remove_owned_download_partial(&state, task_id, file.run_id.as_deref())?;
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
    task_id: String,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
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
    let _ = registered_download_paths_for_task(&entries, &base_dir, &task_id)?;
    confirm_download_destruction(
        app,
        "确认重新下载",
        format!(
            "确认删除当前的部分下载并从头开始？\n\n任务: {task_id}\n\n已下载的部分文件将被永久删除。"
        ),
        "删除并重新下载",
    )
    .await?;
    remove_owned_download_partial(&state, &task_id, None)?;
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
    pub async fn browse_modelscope(
        repo_id: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<Vec<MsFileEntry>> {
        super::browse_modelscope(repo_id, state)
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
        run_id: Option<String>,
        version: Option<u32>,
        state: tauri::State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<()> {
        super::cancel_and_cleanup_download(task_id, file_name, run_id, version, state, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn browse_huggingface(
        repo_id: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<Vec<MsFileEntry>> {
        super::browse_huggingface(repo_id, state)
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
    pub async fn check_local_file(
        save_dir: String,
        repo_id: String,
        remote_path: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<Option<super::ManagedLocalFileResult>> {
        super::check_local_file(save_dir, repo_id, remote_path, state)
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
        task_id: String,
        state: tauri::State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<()> {
        super::reset_download_for_redownload(task_id, state, app)
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
        task_id: String,
        state: tauri::State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<()> {
        super::delete_managed_local_file(task_id, state, app)
            .await
            .map_err(crate::error::AppError::from)
    }
}
const MAX_NATIVE_DOWNLOAD_RETRIES: u32 = 3;
const MAX_DOWNLOAD_RETRY_LIFETIME_SECS: u64 = 24 * 60 * 60;

fn normalize_download_retry_state(entry: &mut PersistedQueueEntry) {
    entry.max_retries = MAX_NATIVE_DOWNLOAD_RETRIES;
    entry.retries = entry.retries.min(MAX_NATIVE_DOWNLOAD_RETRIES);
}

fn initialize_download_retry_state(entry: &mut PersistedQueueEntry) {
    entry.retries = 0;
    entry.max_retries = MAX_NATIVE_DOWNLOAD_RETRIES;
    entry.added_at = now_secs();
}
