use crate::artifact_maintenance::metadata_is_link_like;
use crate::commands::telemetry::{telemetry_storage_info, TelemetryStorageInfo};
use crate::error::{AppError, AppResult};
use crate::external_artifacts::{inventory_external_artifacts, ExternalArtifactInventory};
use crate::models::AppState;
use crate::path_utils::path_is_within;
use crate::security::{authorized_directories_snapshot, AuthorizedDirectory};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const QUARANTINE_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const AUTOMATIC_UPDATER_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const QUARANTINE_FAMILY_LIMIT: usize = 10;
const WEBVIEW_CLEANUP_MARKER: &str = "webview-cache-cleanup.pending";

pub(crate) const GROUP_PRIVATE_SCRATCH: &str = "private-scratch";
pub(crate) const GROUP_PRIVATE_QUARANTINE: &str = "private-quarantine";
pub(crate) const GROUP_UPDATER_STAGING: &str = "updater-staging";
pub(crate) const GROUP_DEVELOPER_TEMP: &str = "developer-temp";
pub(crate) const GROUP_CRASH_MANAGER: &str = "crash-manager";
pub(crate) const GROUP_CRASH_ENGINE: &str = "crash-engine";
pub(crate) const GROUP_CRASH_WEBVIEW: &str = "crash-webview";
pub(crate) const GROUP_WEBVIEW_CACHE: &str = "webview-cache";

const WEBVIEW_CACHE_RELATIVES: &[&str] = &[
    "component_crx_cache",
    "extensions_crx_cache",
    "GPUPersistentCache",
    "GraphiteDawnCache",
    "GrShaderCache",
    "ShaderCache",
    "Default/AutofillAiModelCache",
    "Default/Cache",
    "Default/Code Cache",
    "Default/DawnGraphiteCache",
    "Default/DawnWebGPUCache",
    "Default/GPUCache",
    "Default/GPUPersistentCache",
    "Default/optimization_guide_hint_cache_store",
    "Default/Service Worker/CacheStorage",
    "Default/Shared Dictionary/cache",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageArtifactItem {
    pub path: String,
    pub bytes: u64,
    pub modified_at: Option<i64>,
    pub eligible: bool,
    pub safe: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageMaintenanceGroup {
    pub id: String,
    pub ownership: String,
    pub action: String,
    pub automatic: bool,
    pub item_count: usize,
    pub eligible_count: usize,
    pub total_bytes: u64,
    pub eligible_bytes: u64,
    pub items: Vec<StorageArtifactItem>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageMaintenanceInventory {
    pub generated_at: i64,
    pub app_data_root: String,
    pub temp_root: String,
    pub webview_root: Option<String>,
    pub scheduled_webview_cleanup: bool,
    pub running_instance_count: usize,
    pub groups: Vec<StorageMaintenanceGroup>,
    pub authorized_directories: Vec<AuthorizedDirectory>,
    pub external_artifacts: ExternalArtifactInventory,
    pub telemetry: TelemetryStorageInfo,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCleanupReport {
    pub group_id: String,
    pub removed_items: usize,
    pub removed_bytes: u64,
    pub skipped_items: usize,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone)]
struct MaintenanceRoots {
    app_data: PathBuf,
    temp: PathBuf,
    webview: Option<PathBuf>,
    crash_dumps: Option<PathBuf>,
}

impl MaintenanceRoots {
    fn current() -> Self {
        Self {
            app_data: crate::utils::get_data_dir(),
            temp: std::env::temp_dir(),
            webview: webview_root(),
            crash_dumps: crash_dump_root(),
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn modified_ms(metadata: &std::fs::Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as i64)
}

fn age_at_least(metadata: &std::fs::Metadata, now: SystemTime, minimum: Duration) -> bool {
    metadata
        .modified()
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age >= minimum)
}

#[cfg(windows)]
fn webview_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|root| root.join("com.llama.manager").join("EBWebView"))
}

#[cfg(not(windows))]
fn webview_root() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
fn crash_dump_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|root| root.join("CrashDumps"))
}

#[cfg(not(windows))]
fn crash_dump_root() -> Option<PathBuf> {
    None
}

fn cleanup_marker(app_data: &Path) -> PathBuf {
    app_data.join("maintenance").join(WEBVIEW_CLEANUP_MARKER)
}

fn ensure_safe_marker_parent(app_data: &Path) -> Result<(), String> {
    if root_metadata(app_data)?.is_none() {
        return Err(format!(
            "Application data root does not exist: {}",
            app_data.display()
        ));
    }
    let parent = cleanup_marker(app_data)
        .parent()
        .expect("cleanup marker always has a parent")
        .to_path_buf();
    match std::fs::symlink_metadata(&parent) {
        Ok(metadata) if metadata.is_dir() && !metadata_is_link_like(&metadata) => {}
        Ok(_) => return Err("Refusing unsafe WebView cleanup marker directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&parent).map_err(|error| {
                format!(
                    "Unable to create WebView cleanup marker directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        Err(error) => {
            return Err(format!(
                "Unable to inspect WebView cleanup marker directory {}: {error}",
                parent.display()
            ))
        }
    }
    let canonical_root = std::fs::canonicalize(app_data).map_err(|error| {
        format!(
            "Unable to resolve application data root {}: {error}",
            app_data.display()
        )
    })?;
    let canonical_parent = std::fs::canonicalize(&parent).map_err(|error| {
        format!(
            "Unable to resolve WebView cleanup marker directory {}: {error}",
            parent.display()
        )
    })?;
    if !path_is_within(&canonical_parent, &canonical_root) {
        return Err("WebView cleanup marker directory escapes application data".into());
    }
    Ok(())
}

fn marker_is_scheduled(app_data: &Path) -> bool {
    std::fs::symlink_metadata(cleanup_marker(app_data))
        .map(|metadata| metadata.is_file() && !metadata_is_link_like(&metadata))
        .unwrap_or(false)
}

fn root_metadata(root: &Path) -> Result<Option<std::fs::Metadata>, String> {
    match std::fs::symlink_metadata(root) {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata_is_link_like(&metadata) {
                Err(format!(
                    "Refusing unsafe maintenance root: {}",
                    root.display()
                ))
            } else {
                Ok(Some(metadata))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Unable to inspect maintenance root {}: {error}",
            root.display()
        )),
    }
}

fn validated_path_size(path: &Path, root: &Path) -> Result<u64, String> {
    if root_metadata(root)?.is_none() {
        return Err(format!(
            "Maintenance root does not exist: {}",
            root.display()
        ));
    }
    let canonical_root = std::fs::canonicalize(root).map_err(|error| {
        format!(
            "Unable to resolve maintenance root {}: {error}",
            root.display()
        )
    })?;
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        format!(
            "Unable to inspect maintenance item {}: {error}",
            path.display()
        )
    })?;
    if metadata_is_link_like(&metadata) || (!metadata.is_file() && !metadata.is_dir()) {
        return Err(format!(
            "Refusing linked or special maintenance item: {}",
            path.display()
        ));
    }
    let canonical_path = std::fs::canonicalize(path).map_err(|error| {
        format!(
            "Unable to resolve maintenance item {}: {error}",
            path.display()
        )
    })?;
    if canonical_path == canonical_root || !path_is_within(&canonical_path, &canonical_root) {
        return Err(format!(
            "Maintenance item escapes its fixed root: {}",
            path.display()
        ));
    }

    let mut ancestor = path.parent();
    while let Some(current) = ancestor {
        if current == root {
            break;
        }
        let metadata = std::fs::symlink_metadata(current).map_err(|error| {
            format!(
                "Unable to inspect maintenance ancestor {}: {error}",
                current.display()
            )
        })?;
        if metadata_is_link_like(&metadata) {
            return Err(format!(
                "Refusing maintenance path through a link: {}",
                path.display()
            ));
        }
        ancestor = current.parent();
    }

    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut bytes = 0_u64;
    for entry in WalkDir::new(path).follow_links(false) {
        let entry =
            entry.map_err(|error| format!("Unable to inspect {}: {error}", path.display()))?;
        let metadata = std::fs::symlink_metadata(entry.path())
            .map_err(|error| format!("Unable to inspect {}: {error}", entry.path().display()))?;
        if metadata_is_link_like(&metadata) {
            return Err(format!(
                "Refusing directory containing a link: {}",
                entry.path().display()
            ));
        }
        if metadata.is_file() {
            bytes = bytes.saturating_add(metadata.len());
        }
    }
    Ok(bytes)
}

fn item(path: PathBuf, root: &Path, eligible: bool, reason: Option<String>) -> StorageArtifactItem {
    let metadata = std::fs::symlink_metadata(&path).ok();
    match validated_path_size(&path, root) {
        Ok(bytes) => StorageArtifactItem {
            path: path.to_string_lossy().into_owned(),
            bytes,
            modified_at: metadata.as_ref().and_then(modified_ms),
            eligible,
            safe: true,
            reason,
        },
        Err(error) => StorageArtifactItem {
            path: path.to_string_lossy().into_owned(),
            bytes: 0,
            modified_at: metadata.as_ref().and_then(modified_ms),
            eligible,
            safe: false,
            reason: Some(error),
        },
    }
}

fn group(
    id: &str,
    ownership: &str,
    action: &str,
    automatic: bool,
    mut items: Vec<StorageArtifactItem>,
    warnings: Vec<String>,
) -> StorageMaintenanceGroup {
    items.sort_by(|left, right| left.path.cmp(&right.path));
    let eligible_count = items
        .iter()
        .filter(|entry| entry.eligible && entry.safe)
        .count();
    let total_bytes = items
        .iter()
        .fold(0_u64, |sum, entry| sum.saturating_add(entry.bytes));
    let eligible_bytes = items
        .iter()
        .filter(|entry| entry.eligible && entry.safe)
        .fold(0_u64, |sum, entry| sum.saturating_add(entry.bytes));
    StorageMaintenanceGroup {
        id: id.into(),
        ownership: ownership.into(),
        action: action.into(),
        automatic,
        item_count: items.len(),
        eligible_count,
        total_bytes,
        eligible_bytes,
        items,
        warnings,
    }
}

fn atomic_scratch_name(name: &str) -> bool {
    let Some(stem) = name
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return false;
    };
    stem.rsplit_once('.')
        .and_then(|(_, id)| uuid::Uuid::parse_str(id).ok())
        .is_some()
}

fn pending_checkpoint_name(name: &str) -> bool {
    name.strip_prefix(".pending-")
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .is_some()
}

fn collect_private_scratch(root: &Path, now: SystemTime) -> StorageMaintenanceGroup {
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    match root_metadata(root) {
        Ok(None) => {}
        Err(error) => warnings.push(error),
        Ok(Some(_)) => {
            let mut walker = WalkDir::new(root)
                .min_depth(1)
                .follow_links(false)
                .into_iter();
            while let Some(result) = walker.next() {
                let entry = match result {
                    Ok(entry) => entry,
                    Err(error) => {
                        warnings.push(error.to_string());
                        continue;
                    }
                };
                let name = entry.file_name().to_string_lossy();
                let metadata = match std::fs::symlink_metadata(entry.path()) {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        warnings.push(format!(
                            "Unable to inspect {}: {error}",
                            entry.path().display()
                        ));
                        continue;
                    }
                };
                let matches = (metadata.is_file() && atomic_scratch_name(&name))
                    || (metadata.is_dir() && pending_checkpoint_name(&name));
                if !matches {
                    continue;
                }
                if metadata.is_dir() {
                    walker.skip_current_dir();
                }
                items.push(item(
                    entry.path().to_path_buf(),
                    root,
                    age_at_least(&metadata, now, DAY),
                    Some("older-than-24-hours".into()),
                ));
            }
        }
    }
    group(
        GROUP_PRIVATE_SCRATCH,
        "manager",
        "confirm",
        true,
        items,
        warnings,
    )
}

fn quarantine_family(name: &str) -> Option<&'static str> {
    for (prefix, family) in [
        ("downloads.corrupt-", "downloads"),
        ("downloads_inflight.corrupt-", "downloads_inflight"),
    ] {
        let Some(id) = name
            .strip_prefix(prefix)
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        if uuid::Uuid::parse_str(id).is_ok() {
            return Some(family);
        }
    }
    None
}

fn collect_private_quarantine(root: &Path, now: SystemTime) -> StorageMaintenanceGroup {
    let mut families: HashMap<&'static str, Vec<(PathBuf, std::fs::Metadata)>> = HashMap::new();
    let mut warnings = Vec::new();
    match root_metadata(root) {
        Ok(None) => {}
        Err(error) => warnings.push(error),
        Ok(Some(_)) => {
            for result in WalkDir::new(root)
                .max_depth(4)
                .min_depth(1)
                .follow_links(false)
            {
                let entry = match result {
                    Ok(entry) => entry,
                    Err(error) => {
                        warnings.push(error.to_string());
                        continue;
                    }
                };
                let name = entry.file_name().to_string_lossy();
                let Some(family) = quarantine_family(&name) else {
                    continue;
                };
                let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
                    continue;
                };
                if metadata.is_file() {
                    families
                        .entry(family)
                        .or_default()
                        .push((entry.path().to_path_buf(), metadata));
                }
            }
        }
    }
    let mut items = Vec::new();
    for entries in families.values_mut() {
        entries.sort_by_key(|(_, metadata)| std::cmp::Reverse(modified_ms(metadata).unwrap_or(0)));
        for (index, (path, metadata)) in entries.drain(..).enumerate() {
            let expired = age_at_least(&metadata, now, QUARANTINE_RETENTION);
            let overflow = index >= QUARANTINE_FAMILY_LIMIT;
            let reason = if expired {
                "older-than-30-days"
            } else if overflow {
                "beyond-family-history-limit"
            } else {
                "retained-quarantine-history"
            };
            items.push(item(path, root, expired || overflow, Some(reason.into())));
        }
    }
    group(
        GROUP_PRIVATE_QUARANTINE,
        "manager",
        "confirm",
        true,
        items,
        warnings,
    )
}

fn updater_staging_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("LlamaServerManager-") else {
        return false;
    };
    let Some((version, suffix)) = rest.rsplit_once("-updater-") else {
        return false;
    };
    !version.is_empty()
        && (4..=64).contains(&suffix.len())
        && version
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
        && suffix.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn developer_temp_name(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix("lsm-") else {
        return false;
    };
    !suffix.is_empty()
        && suffix.len() <= 160
        && suffix
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
}

fn collect_temp_group(
    root: &Path,
    now: SystemTime,
    id: &str,
    matcher: fn(&str) -> bool,
    directories_only: bool,
    automatic: bool,
) -> StorageMaintenanceGroup {
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    match root_metadata(root) {
        Ok(None) => {}
        Err(error) => warnings.push(error),
        Ok(Some(_)) => match std::fs::read_dir(root) {
            Ok(entries) => {
                for result in entries {
                    let entry = match result {
                        Ok(entry) => entry,
                        Err(error) => {
                            warnings.push(error.to_string());
                            continue;
                        }
                    };
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !matcher(&name) {
                        continue;
                    }
                    let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
                        continue;
                    };
                    if directories_only && !metadata.is_dir() {
                        continue;
                    }
                    items.push(item(
                        entry.path(),
                        root,
                        age_at_least(&metadata, now, DAY),
                        Some("older-than-24-hours".into()),
                    ));
                }
            }
            Err(error) => warnings.push(format!("Unable to read {}: {error}", root.display())),
        },
    }
    group(id, "platform", "confirm", automatic, items, warnings)
}

fn crash_group(root: Option<&Path>, id: &str, prefix: &str) -> StorageMaintenanceGroup {
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    if let Some(root) = root {
        match root_metadata(root) {
            Ok(None) => {}
            Err(error) => warnings.push(error),
            Ok(Some(_)) => match std::fs::read_dir(root) {
                Ok(entries) => {
                    for result in entries {
                        let Ok(entry) = result else { continue };
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if !name.starts_with(prefix) || !name.ends_with(".dmp") {
                            continue;
                        }
                        let middle = &name[prefix.len()..name.len() - 4];
                        if middle.is_empty() || !middle.chars().all(|ch| ch.is_ascii_digit()) {
                            continue;
                        }
                        items.push(item(
                            entry.path(),
                            root,
                            true,
                            Some("explicit-confirmation".into()),
                        ));
                    }
                }
                Err(error) => warnings.push(format!("Unable to read {}: {error}", root.display())),
            },
        }
    }
    group(id, "platform", "confirm", false, items, warnings)
}

fn collect_webview_cache(root: Option<&Path>) -> StorageMaintenanceGroup {
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    if let Some(root) = root {
        match root_metadata(root) {
            Ok(None) => {}
            Err(error) => warnings.push(error),
            Ok(Some(_)) => {
                for relative in WEBVIEW_CACHE_RELATIVES {
                    let path = root.join(relative);
                    match std::fs::symlink_metadata(&path) {
                        Ok(_) => items.push(item(path, root, true, Some("next-restart".into()))),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => warnings.push(format!(
                            "Unable to inspect WebView cache {}: {error}",
                            path.display()
                        )),
                    }
                }
            }
        }
    }
    group(
        GROUP_WEBVIEW_CACHE,
        "platform",
        "restart",
        false,
        items,
        warnings,
    )
}

fn collect_groups(roots: &MaintenanceRoots, now: SystemTime) -> Vec<StorageMaintenanceGroup> {
    [
        GROUP_PRIVATE_SCRATCH,
        GROUP_PRIVATE_QUARANTINE,
        GROUP_UPDATER_STAGING,
        GROUP_DEVELOPER_TEMP,
        GROUP_CRASH_MANAGER,
        GROUP_CRASH_ENGINE,
        GROUP_CRASH_WEBVIEW,
        GROUP_WEBVIEW_CACHE,
    ]
    .into_iter()
    .filter_map(|group_id| collect_group(roots, now, group_id))
    .collect()
}

fn collect_group(
    roots: &MaintenanceRoots,
    now: SystemTime,
    group_id: &str,
) -> Option<StorageMaintenanceGroup> {
    match group_id {
        GROUP_PRIVATE_SCRATCH => Some(collect_private_scratch(&roots.app_data, now)),
        GROUP_PRIVATE_QUARANTINE => Some(collect_private_quarantine(&roots.app_data, now)),
        GROUP_UPDATER_STAGING => Some(collect_temp_group(
            &roots.temp,
            now,
            GROUP_UPDATER_STAGING,
            updater_staging_name,
            true,
            true,
        )),
        GROUP_DEVELOPER_TEMP => Some(collect_temp_group(
            &roots.temp,
            now,
            GROUP_DEVELOPER_TEMP,
            developer_temp_name,
            false,
            false,
        )),
        GROUP_CRASH_MANAGER => Some(crash_group(
            roots.crash_dumps.as_deref(),
            GROUP_CRASH_MANAGER,
            "llama-server-manager.exe.",
        )),
        GROUP_CRASH_ENGINE => Some(crash_group(
            roots.crash_dumps.as_deref(),
            GROUP_CRASH_ENGINE,
            "llama-server.exe.",
        )),
        GROUP_CRASH_WEBVIEW => Some(crash_group(
            roots.crash_dumps.as_deref(),
            GROUP_CRASH_WEBVIEW,
            "msedgewebview2.exe.",
        )),
        GROUP_WEBVIEW_CACHE => Some(collect_webview_cache(roots.webview.as_deref())),
        _ => None,
    }
}

fn inventory(
    roots: MaintenanceRoots,
    running_instance_count: usize,
    authorized_directories: Vec<AuthorizedDirectory>,
    external_artifacts: ExternalArtifactInventory,
) -> StorageMaintenanceInventory {
    StorageMaintenanceInventory {
        generated_at: now_ms(),
        app_data_root: roots.app_data.to_string_lossy().into_owned(),
        temp_root: roots.temp.to_string_lossy().into_owned(),
        webview_root: roots
            .webview
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        scheduled_webview_cleanup: marker_is_scheduled(&roots.app_data),
        running_instance_count,
        groups: collect_groups(&roots, SystemTime::now()),
        authorized_directories,
        external_artifacts,
        telemetry: telemetry_storage_info(),
    }
}

fn group_root<'a>(roots: &'a MaintenanceRoots, group_id: &str) -> Option<&'a Path> {
    match group_id {
        GROUP_PRIVATE_SCRATCH | GROUP_PRIVATE_QUARANTINE => Some(&roots.app_data),
        GROUP_UPDATER_STAGING | GROUP_DEVELOPER_TEMP => Some(&roots.temp),
        GROUP_CRASH_MANAGER | GROUP_CRASH_ENGINE | GROUP_CRASH_WEBVIEW => {
            roots.crash_dumps.as_deref()
        }
        GROUP_WEBVIEW_CACHE => roots.webview.as_deref(),
        _ => None,
    }
}

fn remove_validated(path: &Path, root: &Path) -> Result<u64, String> {
    let bytes = validated_path_size(path, root)?;
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        format!(
            "Unable to inspect {} before removal: {error}",
            path.display()
        )
    })?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
    .map_err(|error| format!("Unable to remove {}: {error}", path.display()))?;
    Ok(bytes)
}

fn cleanup_eligible(
    entry: &StorageArtifactItem,
    group_id: &str,
    automatic_run: bool,
    now: i64,
) -> bool {
    let auto_updater_eligible = entry.modified_at.is_some_and(|modified| {
        now.saturating_sub(modified) >= AUTOMATIC_UPDATER_RETENTION.as_millis() as i64
    });
    entry.eligible && (!automatic_run || group_id != GROUP_UPDATER_STAGING || auto_updater_eligible)
}

fn cleanup_group_with_roots(
    roots: &MaintenanceRoots,
    group_id: &str,
    running_instance_count: usize,
    automatic_run: bool,
) -> Result<StorageCleanupReport, String> {
    let known = [
        GROUP_PRIVATE_SCRATCH,
        GROUP_PRIVATE_QUARANTINE,
        GROUP_UPDATER_STAGING,
        GROUP_DEVELOPER_TEMP,
        GROUP_CRASH_MANAGER,
        GROUP_CRASH_ENGINE,
        GROUP_CRASH_WEBVIEW,
        GROUP_WEBVIEW_CACHE,
    ];
    if !known.contains(&group_id) {
        return Err("Unknown fixed storage-maintenance group".into());
    }
    if group_id == GROUP_WEBVIEW_CACHE && !automatic_run {
        return Err("WebView cache cleanup must be scheduled for the next restart".into());
    }
    if group_id == GROUP_PRIVATE_SCRATCH && running_instance_count > 0 {
        return Err("Managed scratch cleanup requires all instances to be stopped".into());
    }
    let Some(root) = group_root(roots, group_id) else {
        return Ok(StorageCleanupReport {
            group_id: group_id.into(),
            ..Default::default()
        });
    };
    let group = collect_group(roots, SystemTime::now(), group_id)
        .ok_or_else(|| "Storage-maintenance group could not be rescanned".to_string())?;
    let now = now_ms();
    let mut report = StorageCleanupReport {
        group_id: group_id.into(),
        ..Default::default()
    };
    for entry in group.items {
        if !cleanup_eligible(&entry, group_id, automatic_run, now) {
            report.skipped_items = report.skipped_items.saturating_add(1);
            continue;
        }
        if !entry.safe {
            report.skipped_items = report.skipped_items.saturating_add(1);
            report.failures.push(
                entry
                    .reason
                    .unwrap_or_else(|| format!("Refusing unsafe maintenance item: {}", entry.path)),
            );
            continue;
        }
        match remove_validated(Path::new(&entry.path), root) {
            Ok(bytes) => {
                report.removed_items = report.removed_items.saturating_add(1);
                report.removed_bytes = report.removed_bytes.saturating_add(bytes);
            }
            Err(error) => report.failures.push(error),
        }
    }
    Ok(report)
}

pub(crate) fn run_automatic_storage_maintenance(running_instance_count: usize) {
    let roots = MaintenanceRoots::current();
    for group_id in [
        GROUP_PRIVATE_QUARANTINE,
        GROUP_UPDATER_STAGING,
        GROUP_PRIVATE_SCRATCH,
    ] {
        if group_id == GROUP_PRIVATE_SCRATCH && running_instance_count > 0 {
            continue;
        }
        match cleanup_group_with_roots(&roots, group_id, running_instance_count, true) {
            Ok(report) if report.removed_items > 0 => eprintln!(
                "Storage maintenance removed {} item(s) and {} bytes from {}",
                report.removed_items, report.removed_bytes, group_id
            ),
            Ok(_) => {}
            Err(error) => eprintln!("Storage maintenance for {group_id} failed: {error}"),
        }
    }
}

pub(crate) fn process_scheduled_webview_cleanup() -> Result<Option<StorageCleanupReport>, String> {
    let roots = MaintenanceRoots::current();
    process_scheduled_webview_cleanup_with_roots(&roots)
}

fn process_scheduled_webview_cleanup_with_roots(
    roots: &MaintenanceRoots,
) -> Result<Option<StorageCleanupReport>, String> {
    let marker = cleanup_marker(&roots.app_data);
    let metadata = match std::fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Unable to inspect WebView cleanup marker: {error}")),
    };
    if !metadata.is_file() || metadata_is_link_like(&metadata) {
        return Err("Refusing unsafe WebView cleanup marker".into());
    }
    validated_path_size(&marker, &roots.app_data)?;
    let report = cleanup_group_with_roots(roots, GROUP_WEBVIEW_CACHE, 0, true)?;
    if !report.failures.is_empty() {
        return Err(format!(
            "WebView cache cleanup was incomplete: {}",
            report.failures.join("; ")
        ));
    }
    std::fs::remove_file(&marker)
        .map_err(|error| format!("Unable to clear WebView cleanup marker: {error}"))?;
    Ok(Some(report))
}

#[tauri::command]
pub async fn get_storage_maintenance_inventory(
    state: tauri::State<'_, AppState>,
) -> AppResult<StorageMaintenanceInventory> {
    let running_instance_count = state.running.lock().unwrap().len();
    let instances = state.instances.lock().unwrap().clone();
    let authorized_directories = authorized_directories_snapshot();
    tokio::task::spawn_blocking(move || {
        Ok(inventory(
            MaintenanceRoots::current(),
            running_instance_count,
            authorized_directories,
            inventory_external_artifacts(&instances),
        ))
    })
    .await
    .map_err(|error| {
        AppError::new(
            "INTERNAL",
            format!("Storage inventory task failed: {error}"),
            true,
        )
    })?
}

#[tauri::command]
pub async fn cleanup_storage_group(
    group_id: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<StorageCleanupReport> {
    let running_instance_count = state.running.lock().unwrap().len();
    tokio::task::spawn_blocking(move || {
        cleanup_group_with_roots(
            &MaintenanceRoots::current(),
            group_id.trim(),
            running_instance_count,
            false,
        )
        .map_err(AppError::from)
    })
    .await
    .map_err(|error| {
        AppError::new(
            "INTERNAL",
            format!("Storage cleanup task failed: {error}"),
            true,
        )
    })?
}

#[tauri::command]
pub async fn schedule_webview_cache_cleanup(enabled: bool) -> AppResult<bool> {
    tokio::task::spawn_blocking(move || {
        let roots = MaintenanceRoots::current();
        if roots.webview.is_none() {
            return Err(AppError::new(
                "VALIDATION",
                "WebView cache maintenance is available only on supported Windows installations",
                false,
            ));
        }
        let marker = cleanup_marker(&roots.app_data);
        if enabled {
            ensure_safe_marker_parent(&roots.app_data).map_err(AppError::from)?;
            match std::fs::symlink_metadata(&marker) {
                Ok(metadata) if !metadata.is_file() || metadata_is_link_like(&metadata) => {
                    return Err(AppError::new(
                        "VALIDATION",
                        "Refusing unsafe WebView cleanup marker",
                        false,
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(AppError::from(error)),
            }
            crate::persistence::atomic_write(&marker, b"scheduled\n", None)
                .map_err(AppError::from)?;
        } else {
            match std::fs::symlink_metadata(&marker) {
                Ok(metadata) => {
                    if !metadata.is_file() || metadata_is_link_like(&metadata) {
                        return Err(AppError::new(
                            "VALIDATION",
                            "Refusing unsafe WebView cleanup marker",
                            false,
                        ));
                    }
                    validated_path_size(&marker, &roots.app_data).map_err(AppError::from)?;
                    std::fs::remove_file(&marker).map_err(AppError::from)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(AppError::from(error)),
            }
        }
        Ok(enabled)
    })
    .await
    .map_err(|error| {
        AppError::new(
            "INTERNAL",
            format!("WebView cleanup scheduling failed: {error}"),
            true,
        )
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[cfg(unix)]
    fn create_dir_link(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_dir_link(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }

    fn sandbox(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "lsm-storage-maintenance-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn roots(root: &Path) -> MaintenanceRoots {
        let app_data = root.join("app-data");
        let temp = root.join("temp");
        let webview = root.join("webview");
        let crash_dumps = root.join("crash");
        for path in [&app_data, &temp, &webview, &crash_dumps] {
            fs::create_dir_all(path).unwrap();
        }
        MaintenanceRoots {
            app_data,
            temp,
            webview: Some(webview),
            crash_dumps: Some(crash_dumps),
        }
    }

    #[test]
    fn fixed_matchers_reject_lookalikes() {
        assert!(updater_staging_name(
            "LlamaServerManager-2.9.45-updater-vW8kF3"
        ));
        assert!(!updater_staging_name("Other-2.9.45-updater-vW8kF3"));
        assert!(!updater_staging_name(
            "LlamaServerManager-2.9.45-updater-../../x"
        ));
        assert!(developer_temp_name("lsm-cli-smoke-123"));
        assert!(!developer_temp_name("lsm-../outside"));
        let id = uuid::Uuid::new_v4();
        assert!(atomic_scratch_name(&format!(".config.json.{id}.tmp")));
        assert!(pending_checkpoint_name(&format!(".pending-{id}")));
    }

    #[test]
    fn automatic_updater_cleanup_uses_the_longer_retention_window() {
        let now = 20 * DAY.as_millis() as i64;
        let two_days_old = StorageArtifactItem {
            path: "updater-two-days-old".into(),
            bytes: 0,
            modified_at: Some(now - 2 * DAY.as_millis() as i64),
            eligible: true,
            safe: true,
            reason: None,
        };
        let eight_days_old = StorageArtifactItem {
            path: "updater-eight-days-old".into(),
            modified_at: Some(now - 8 * DAY.as_millis() as i64),
            ..two_days_old.clone()
        };

        assert!(cleanup_eligible(
            &two_days_old,
            GROUP_UPDATER_STAGING,
            false,
            now
        ));
        assert!(!cleanup_eligible(
            &two_days_old,
            GROUP_UPDATER_STAGING,
            true,
            now
        ));
        assert!(cleanup_eligible(
            &eight_days_old,
            GROUP_UPDATER_STAGING,
            true,
            now
        ));
    }

    #[test]
    fn updater_inventory_ignores_matching_regular_files() {
        let sandbox = sandbox("updater-files");
        let roots = roots(&sandbox);
        fs::write(
            roots
                .temp
                .join("LlamaServerManager-2.9.45-updater-regularfile"),
            b"not-a-staging-directory",
        )
        .unwrap();
        let group = collect_group(&roots, SystemTime::now(), GROUP_UPDATER_STAGING).unwrap();
        assert_eq!(group.item_count, 0);
        let _ = fs::remove_dir_all(sandbox);
    }

    #[test]
    fn inventory_and_cleanup_are_limited_to_fixed_groups() {
        let sandbox = sandbox("fixed-groups");
        let roots = roots(&sandbox);
        let crash = roots.crash_dumps.as_ref().unwrap();
        fs::write(crash.join("llama-server.exe.123.dmp"), b"engine").unwrap();
        fs::write(crash.join("ApplicationFrameHost.exe.123.dmp"), b"other").unwrap();

        let group = crash_group(Some(crash), GROUP_CRASH_ENGINE, "llama-server.exe.");
        assert_eq!(group.item_count, 1);
        let report = cleanup_group_with_roots(&roots, GROUP_CRASH_ENGINE, 0, false).unwrap();
        assert_eq!(report.removed_items, 1);
        assert!(crash.join("ApplicationFrameHost.exe.123.dmp").exists());
        assert!(cleanup_group_with_roots(&roots, "arbitrary", 0, false).is_err());
        let _ = fs::remove_dir_all(sandbox);
    }

    #[test]
    fn webview_cleanup_removes_only_fixed_cache_relatives() {
        let sandbox = sandbox("webview");
        let roots = roots(&sandbox);
        let webview = roots.webview.as_ref().unwrap();
        fs::create_dir_all(webview.join("Default/Cache")).unwrap();
        fs::create_dir_all(webview.join("Default/Local Storage")).unwrap();
        fs::write(webview.join("Default/Cache/cache.bin"), b"cache").unwrap();
        fs::write(webview.join("Default/Local Storage/state.bin"), b"state").unwrap();

        let report = cleanup_group_with_roots(&roots, GROUP_WEBVIEW_CACHE, 0, true).unwrap();
        assert_eq!(report.removed_items, 1);
        assert!(!webview.join("Default/Cache").exists());
        assert!(webview.join("Default/Local Storage/state.bin").exists());
        let _ = fs::remove_dir_all(sandbox);
    }

    #[test]
    fn scheduled_webview_cleanup_consumes_marker_only_after_success() {
        let sandbox = sandbox("webview-marker");
        let roots = roots(&sandbox);
        let webview = roots.webview.as_ref().unwrap();
        fs::create_dir_all(webview.join("Default/GPUCache")).unwrap();
        fs::write(webview.join("Default/GPUCache/cache.bin"), b"cache").unwrap();
        let marker = cleanup_marker(&roots.app_data);
        ensure_safe_marker_parent(&roots.app_data).unwrap();
        crate::persistence::atomic_write(&marker, b"scheduled\n", None).unwrap();

        let report = process_scheduled_webview_cleanup_with_roots(&roots)
            .unwrap()
            .unwrap();
        assert_eq!(report.removed_items, 1);
        assert!(!marker.exists());
        assert!(!webview.join("Default/GPUCache").exists());
        let _ = fs::remove_dir_all(sandbox);
    }

    #[test]
    fn unsafe_webview_link_preserves_restart_marker_and_external_data() {
        let sandbox = sandbox("webview-link");
        let roots = roots(&sandbox);
        let webview = roots.webview.as_ref().unwrap();
        let outside = sandbox.join("outside-cache");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("external.bin"), b"external").unwrap();
        fs::create_dir_all(webview.join("Default")).unwrap();
        if create_dir_link(&outside, &webview.join("Default/Cache")).is_err() {
            let _ = fs::remove_dir_all(sandbox);
            return;
        }
        let marker = cleanup_marker(&roots.app_data);
        ensure_safe_marker_parent(&roots.app_data).unwrap();
        crate::persistence::atomic_write(&marker, b"scheduled\n", None).unwrap();

        let error = process_scheduled_webview_cleanup_with_roots(&roots).unwrap_err();
        assert!(error.contains("incomplete"));
        assert!(marker.exists());
        assert!(outside.join("external.bin").exists());
        assert!(webview.join("Default/Cache").exists());
        let _ = fs::remove_dir_all(sandbox);
    }

    #[test]
    fn marker_parent_must_be_a_real_directory_inside_app_data() {
        let sandbox = sandbox("marker-parent");
        let roots = roots(&sandbox);
        fs::write(roots.app_data.join("maintenance"), b"not-a-directory").unwrap();
        assert!(ensure_safe_marker_parent(&roots.app_data).is_err());
        let _ = fs::remove_dir_all(sandbox);
    }

    #[test]
    fn running_instances_block_managed_scratch_cleanup() {
        let sandbox = sandbox("active-scratch");
        let roots = roots(&sandbox);
        let id = uuid::Uuid::new_v4();
        fs::write(roots.app_data.join(format!(".config.{id}.tmp")), b"scratch").unwrap();
        assert!(cleanup_group_with_roots(&roots, GROUP_PRIVATE_SCRATCH, 1, false).is_err());
        let _ = fs::remove_dir_all(sandbox);
    }
}
