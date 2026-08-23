use crate::commands::engine_capabilities::{
    capabilities_match_executable, invalidate_engine_evidence,
};
use crate::commands::model_inventory::{
    self, InventoryDirectoryRecord, InventoryEngineRecord, InventoryModelRecord,
};
use crate::models::{AppState, EngineInfo, InstanceConfig, ModelCapabilities, ModelInfo};
use crate::path_utils::{path_identity_key, path_is_within, paths_equal};
use crate::utils;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, LazyLock};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

static GGUF_WORK_SLOTS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(4)));
const GGUF_WORK_ADMISSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn file_mtime(path: &Path) -> u64 {
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn canonical_key(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn engine_path_identity(path: &Path) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    path_identity_key(&canonical)
}

fn instances_referencing_model(
    instances: &HashMap<String, InstanceConfig>,
    target: &Path,
) -> Vec<String> {
    let target_identity = engine_path_identity(target);
    instances
        .values()
        .filter(|instance| {
            [
                instance.model_path.as_str(),
                instance.draft_model_path.as_str(),
                instance.mmproj_path.as_str(),
            ]
            .into_iter()
            .filter(|candidate| !candidate.trim().is_empty())
            .any(|candidate| engine_path_identity(Path::new(candidate)) == target_identity)
        })
        .map(|instance| instance.name.clone())
        .collect()
}

fn instances_referencing_engine(
    instances: &HashMap<String, InstanceConfig>,
    engine_id: &str,
) -> Vec<String> {
    instances
        .values()
        .filter(|instance| paths_equal(Path::new(&instance.engine_id), Path::new(engine_id)))
        .map(|instance| instance.name.clone())
        .collect()
}

#[derive(Debug, Clone)]
struct DirectoryEntryFingerprint {
    path: PathBuf,
    name: String,
    is_dir: bool,
    is_file: bool,
    is_symlink: bool,
    size: u64,
    mtime_ns: u128,
}

#[derive(Debug, Clone)]
struct DirectoryFingerprint {
    signature: String,
    entries: Vec<DirectoryEntryFingerprint>,
}

const MAX_SCAN_DIRECTORIES: usize = 20_000;
const MAX_SCAN_ROOTS: usize = 256;
const MAX_SCAN_ENTRIES: usize = 200_000;
const MAX_SCAN_FILES: usize = 100_000;
const MAX_SCAN_RESULTS: usize = 10_000;
const MAX_SCAN_PARSES: usize = 2_048;
const MAX_SCAN_METADATA_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SCAN_RETAINED_METADATA_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SCAN_DURATION: std::time::Duration = std::time::Duration::from_secs(120);

struct ScanBudget {
    started: std::time::Instant,
    directories: usize,
    entries: usize,
    files: usize,
    results: usize,
    parses: usize,
    metadata_bytes: u64,
    retained_metadata_bytes: u64,
    visited_directories: HashSet<String>,
}

impl Default for ScanBudget {
    fn default() -> Self {
        Self {
            started: std::time::Instant::now(),
            directories: 0,
            entries: 0,
            files: 0,
            results: 0,
            parses: 0,
            metadata_bytes: 0,
            retained_metadata_bytes: 0,
            visited_directories: HashSet::new(),
        }
    }
}

impl ScanBudget {
    fn deadline(&self) -> std::time::Instant {
        self.started + MAX_SCAN_DURATION
    }

    fn check_time(&self) -> Result<(), String> {
        if self.started.elapsed() > MAX_SCAN_DURATION {
            return Err("scan exceeded its 120-second work budget".into());
        }
        Ok(())
    }

    fn enter_directory(&mut self, path: &Path, root: &Path) -> Result<PathBuf, String> {
        self.check_time()?;
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if metadata_is_link_like(&metadata) {
            return Err(format!("{} is link-like and was skipped", path.display()));
        }
        let canonical = std::fs::canonicalize(path)
            .map_err(|error| format!("failed to resolve {}: {error}", path.display()))?;
        let canonical_root = std::fs::canonicalize(root)
            .map_err(|error| format!("failed to resolve scan root {}: {error}", root.display()))?;
        if !paths_equal(&canonical, &canonical_root) && !path_is_within(&canonical, &canonical_root)
        {
            return Err(format!(
                "{} escaped the authorized scan root",
                path.display()
            ));
        }
        let identity = path_identity_key(&canonical);
        if !self.visited_directories.insert(identity) {
            return Err(format!("{} was already visited", path.display()));
        }
        self.directories = self
            .directories
            .checked_add(1)
            .ok_or_else(|| "scan directory counter overflow".to_string())?;
        if self.directories > MAX_SCAN_DIRECTORIES {
            return Err(format!(
                "scan exceeded the {MAX_SCAN_DIRECTORIES}-directory budget"
            ));
        }
        Ok(canonical)
    }

    fn observe_entry(&mut self, metadata: &std::fs::Metadata) -> Result<(), String> {
        self.check_time()?;
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or_else(|| "scan entry counter overflow".to_string())?;
        if self.entries > MAX_SCAN_ENTRIES {
            return Err(format!("scan exceeded the {MAX_SCAN_ENTRIES}-entry budget"));
        }
        if metadata.is_file() {
            self.files = self
                .files
                .checked_add(1)
                .ok_or_else(|| "scan file counter overflow".to_string())?;
            if self.files > MAX_SCAN_FILES {
                return Err(format!("scan exceeded the {MAX_SCAN_FILES}-file budget"));
            }
        }
        self.metadata_bytes = self
            .metadata_bytes
            .checked_add(std::mem::size_of_val(metadata) as u64)
            .ok_or_else(|| "scan metadata budget overflow".to_string())?;
        if self.metadata_bytes > MAX_SCAN_METADATA_BYTES {
            return Err("scan exceeded its metadata allocation budget".into());
        }
        Ok(())
    }

    fn add_result(&mut self) -> Result<(), String> {
        self.results = self
            .results
            .checked_add(1)
            .ok_or_else(|| "scan result counter overflow".to_string())?;
        if self.results > MAX_SCAN_RESULTS {
            return Err(format!(
                "scan exceeded the {MAX_SCAN_RESULTS}-result budget"
            ));
        }
        Ok(())
    }

    fn retain_metadata(&mut self, bytes: usize) -> Result<(), String> {
        self.retained_metadata_bytes = self
            .retained_metadata_bytes
            .checked_add(bytes as u64)
            .ok_or_else(|| "scan retained-metadata budget overflow".to_string())?;
        if self.retained_metadata_bytes > MAX_SCAN_RETAINED_METADATA_BYTES {
            return Err(format!(
                "scan exceeded the {MAX_SCAN_RETAINED_METADATA_BYTES}-byte retained metadata budget"
            ));
        }
        Ok(())
    }

    fn add_parse(&mut self) -> Result<(), String> {
        self.parses = self
            .parses
            .checked_add(1)
            .ok_or_else(|| "scan parse counter overflow".to_string())?;
        if self.parses > MAX_SCAN_PARSES {
            return Err(format!("scan exceeded the {MAX_SCAN_PARSES}-parse budget"));
        }
        Ok(())
    }
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

fn stable_hash(parts: &[String]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn read_directory_fingerprint(
    path: &Path,
    root: &Path,
    budget: &mut ScanBudget,
) -> Result<DirectoryFingerprint, String> {
    let _canonical = budget.enter_directory(path, root)?;
    let mut entries = Vec::new();
    for entry in
        std::fs::read_dir(path).map_err(|e| format!("failed to read {}: {}", path.display(), e))?
    {
        let entry = entry.map_err(|e| format!("failed to read {} entry: {}", path.display(), e))?;
        let metadata = std::fs::symlink_metadata(entry.path())
            .map_err(|e| format!("failed to inspect {} metadata: {e}", entry.path().display()))?;
        budget.observe_entry(&metadata)?;
        let link_like = metadata_is_link_like(&metadata);
        let file_type = metadata.file_type();
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok());
        entries.push(DirectoryEntryFingerprint {
            path: entry.path(),
            name: entry.file_name().to_string_lossy().to_string(),
            is_dir: file_type.is_dir() && !link_like,
            is_file: file_type.is_file() && !link_like,
            is_symlink: link_like,
            size: metadata.len(),
            mtime_ns: modified.map(|duration| duration.as_nanos()).unwrap_or(0),
        });
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    let parts = entries
        .iter()
        .map(|entry| {
            format!(
                "{}|{}|{}|{}|{}|{}",
                entry.name,
                if entry.is_dir {
                    "d"
                } else if entry.is_file {
                    "f"
                } else {
                    "o"
                },
                if entry.is_symlink { "l" } else { "-" },
                entry.size,
                entry.mtime_ns,
                entry.path.extension().and_then(OsStr::to_str).unwrap_or("")
            )
        })
        .collect::<Vec<_>>();
    Ok(DirectoryFingerprint {
        signature: stable_hash(&parts),
        entries,
    })
}

#[cfg(test)]
fn path_is_under_directory(path: &Path, directory: &Path) -> bool {
    !paths_equal(path, directory) && path_is_within(path, directory)
}

fn saved_engine_name<'a>(names: &'a HashMap<String, String>, id: &str) -> Option<&'a String> {
    names.get(id).or_else(|| {
        names
            .iter()
            .find(|(saved_id, _)| paths_equal(Path::new(saved_id), Path::new(id)))
            .map(|(_, name)| name)
    })
}

const MAX_MODEL_SCAN_DEPTH: usize = 32;

fn cached_artifact_identity_is_reusable(
    identity: &crate::deployment_identity::ArtifactIdentity,
) -> bool {
    identity.is_verified()
}

fn push_scan_error_once(errors: &mut Vec<String>, error: String) {
    if !errors.iter().any(|existing| existing == &error) {
        errors.push(error);
    }
}

fn reuse_cached_engines_for_root(
    scan_root_key: &str,
    inventory: &HashMap<String, InventoryEngineRecord>,
    seen_ids: &mut HashSet<String>,
    engines: &mut Vec<EngineInfo>,
    engine_records: &mut Vec<InventoryEngineRecord>,
) -> usize {
    let mut reused = 0;
    for record in inventory
        .values()
        .filter(|record| record.scan_root == scan_root_key)
    {
        if !seen_ids.insert(record.id.clone()) {
            continue;
        }
        let info = record.to_engine_info();
        engine_records.push(InventoryEngineRecord::from_engine(
            &info,
            record.exe_mtime,
            scan_root_key.to_string(),
        ));
        engines.push(info);
        reused += 1;
    }
    reused
}

#[allow(clippy::too_many_arguments)]
fn try_reuse_engine_root(
    scan_root_key: &str,
    signature: &str,
    inventory: &HashMap<String, InventoryEngineRecord>,
    directory_inventory: &HashMap<String, InventoryDirectoryRecord>,
    seen_directory_keys: &mut HashSet<String>,
    directory_records: &mut Vec<InventoryDirectoryRecord>,
    seen_inventory_ids: &mut HashSet<String>,
    engines: &mut Vec<EngineInfo>,
    engine_records: &mut Vec<InventoryEngineRecord>,
) -> Result<bool, String> {
    seen_directory_keys.insert(scan_root_key.to_string());
    directory_records.push(InventoryDirectoryRecord::new(
        "engine",
        scan_root_key.to_string(),
        scan_root_key.to_string(),
        signature.to_string(),
    ));

    let reusable = directory_inventory
        .get(scan_root_key)
        .map(|record| record.signature == signature)
        .unwrap_or(false)
        && inventory
            .values()
            .filter(|record| record.scan_root == scan_root_key)
            .all(|record| cached_artifact_identity_is_reusable(&record.artifact_identity));
    if reusable {
        reuse_cached_engines_for_root(
            scan_root_key,
            inventory,
            seen_inventory_ids,
            engines,
            engine_records,
        );
    }
    Ok(reusable)
}

fn push_indexed_engine(
    dir: &Path,
    exe: &Path,
    scan_root_key: &str,
    inventory: &HashMap<String, InventoryEngineRecord>,
    seen_inventory_ids: &mut HashSet<String>,
    output: (&mut Vec<EngineInfo>, &mut Vec<InventoryEngineRecord>),
    budget: &mut ScanBudget,
) -> Result<(), String> {
    let (engines, engine_records) = output;
    let cache_key = canonical_key(dir);
    let exe_mtime = file_mtime(exe);
    seen_inventory_ids.insert(cache_key.clone());

    if let Some(record) = inventory.get(&cache_key) {
        if record.exe_mtime == exe_mtime
            && cached_artifact_identity_is_reusable(&record.artifact_identity)
        {
            let info = record.to_engine_info();
            engine_records.push(InventoryEngineRecord::from_engine(
                &info,
                exe_mtime,
                scan_root_key.to_string(),
            ));
            engines.push(info);
            return Ok(());
        }
    }

    if let Some(info) = build_engine_info(dir, exe, "", budget.deadline()) {
        engine_records.push(InventoryEngineRecord::from_engine(
            &info,
            exe_mtime,
            scan_root_key.to_string(),
        ));
        engines.push(info);
    }
    budget.check_time()
}

fn merge_scanned_engine_capabilities(scanned: &mut [EngineInfo], current: &[EngineInfo]) {
    for engine in scanned {
        if !engine.capabilities.executable_fingerprint.is_empty()
            && !capabilities_match_executable(&engine.exe, &engine.capabilities)
        {
            invalidate_engine_evidence(
                engine,
                "engine executable changed; compatibility probe and qualification required",
            );
        }

        let Some(active) = current.iter().find(|candidate| {
            paths_equal(Path::new(&candidate.id), Path::new(&engine.id))
                && paths_equal(Path::new(&candidate.exe), Path::new(&engine.exe))
        }) else {
            continue;
        };
        if capabilities_match_executable(&engine.exe, &active.capabilities)
            && active.capabilities.probed_at.unwrap_or(0)
                >= engine.capabilities.probed_at.unwrap_or(0)
        {
            engine.version = active.version.clone();
            engine.capabilities = active.capabilities.clone();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_model_directory_incremental(
    dir: &Path,
    scan_root: &Path,
    scan_root_key: &str,
    depth: usize,
    inventory: &HashMap<String, InventoryModelRecord>,
    _directory_inventory: &HashMap<String, InventoryDirectoryRecord>,
    models: &mut Vec<ModelInfo>,
    seen_display_paths: &mut HashSet<String>,
    seen_inventory_paths: &mut HashSet<String>,
    seen_directory_keys: &mut HashSet<String>,
    inventory_meta: &mut HashMap<usize, (String, String, u64)>,
    fresh_files: &mut Vec<(usize, crate::deployment_identity::ArtifactInspectionLease)>,
    directory_records: &mut Vec<InventoryDirectoryRecord>,
    errors: &mut Vec<String>,
    budget: &mut ScanBudget,
) -> usize {
    let dir_key = canonical_key(dir);
    seen_directory_keys.insert(dir_key.clone());

    let fingerprint = match read_directory_fingerprint(dir, scan_root, budget) {
        Ok(fingerprint) => fingerprint,
        Err(err) => {
            push_scan_error_once(errors, err);
            return 0;
        }
    };
    // Directory signatures are intentionally local. Reusing a complete subtree from a
    // parent-only signature allowed descendant changes to be missed and required repeated
    // full-subtree walks. Individual unchanged files are still reused below.
    let tree_signature = fingerprint.signature.clone();

    let mut file_count = 0;
    for entry in fingerprint.entries {
        if entry.is_symlink {
            errors.push(format!(
                "{} is a symlink and was skipped",
                entry.path.display()
            ));
            continue;
        }

        if entry.is_dir {
            if depth < MAX_MODEL_SCAN_DEPTH {
                file_count += scan_model_directory_incremental(
                    &entry.path,
                    scan_root,
                    scan_root_key,
                    depth + 1,
                    inventory,
                    _directory_inventory,
                    models,
                    seen_display_paths,
                    seen_inventory_paths,
                    seen_directory_keys,
                    inventory_meta,
                    fresh_files,
                    directory_records,
                    errors,
                    budget,
                );
                if let Err(error) = budget.check_time() {
                    push_scan_error_once(errors, error);
                    break;
                }
            } else {
                errors.push(format!(
                    "{} exceeded the model scan depth limit of {} and was skipped",
                    entry.path.display(),
                    MAX_MODEL_SCAN_DEPTH
                ));
            }
            continue;
        }

        if !entry.is_file {
            continue;
        }

        let ext = entry
            .path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext != "gguf" || entry.name.starts_with('.') {
            continue;
        }

        let candidate_metadata = match std::fs::symlink_metadata(&entry.path) {
            Ok(metadata) if metadata.is_file() && !metadata_is_link_like(&metadata) => metadata,
            Ok(_) => {
                errors.push(format!(
                    "{} became link-like and was skipped",
                    entry.path.display()
                ));
                continue;
            }
            Err(error) => {
                errors.push(format!(
                    "{} could not be revalidated: {error}",
                    entry.path.display()
                ));
                continue;
            }
        };
        let canonical_entry = match std::fs::canonicalize(&entry.path) {
            Ok(path) if path_is_within(&path, scan_root) => path,
            Ok(_) => {
                errors.push(format!(
                    "{} escaped the authorized scan root",
                    entry.path.display()
                ));
                continue;
            }
            Err(error) => {
                errors.push(format!(
                    "{} could not be resolved: {error}",
                    entry.path.display()
                ));
                continue;
            }
        };
        let candidate_mtime = candidate_metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or(0);

        let file_path = canonical_entry.to_string_lossy().to_string();
        if !seen_display_paths.insert(path_identity_key(Path::new(&file_path))) {
            continue;
        }

        let cache_key = canonical_key(&canonical_entry);
        seen_inventory_paths.insert(cache_key.clone());
        file_count += 1;

        if let Some(record) = inventory.get(&cache_key) {
            if record.mtime == candidate_mtime && record.size == candidate_metadata.len() {
                if let Err(error) = budget.add_result() {
                    push_scan_error_once(errors, error);
                    break;
                }
                let idx = models.len();
                let mut model = record.to_model_info();
                model.name = entry.name.clone();
                model.path = file_path;
                model.size = candidate_metadata.len();
                model.is_shard = false;
                models.push(model);
                inventory_meta.insert(idx, (cache_key, scan_root_key.to_string(), candidate_mtime));
                continue;
            }
        }

        if let Err(error) = budget.add_result().and_then(|_| budget.add_parse()) {
            push_scan_error_once(errors, error);
            break;
        }
        let idx = models.len();
        let inspection_lease =
            match crate::deployment_identity::ArtifactInspectionLease::open_model_beneath_authorized_root(
                &canonical_entry,
                scan_root,
            ) {
                Ok(lease) => Some(lease),
                Err(error) => {
                    if let Err(budget_error) = budget.check_time() {
                        push_scan_error_once(errors, budget_error);
                        break;
                    }
                    errors.push(format!(
                        "{} metadata inspection failed: {error}",
                        canonical_entry.display()
                    ));
                    None
                }
            };
        models.push(ModelInfo {
            id: uuid::Uuid::new_v4().to_string(),
            name: entry.name,
            path: file_path,
            size: candidate_metadata.len(),
            architecture: None,
            context_length: None,
            quant_type: None,
            has_mtp_head: false,
            capabilities: ModelCapabilities::default(),
            file_type: utils::classify_gguf_file(&canonical_entry).to_string(),
            is_shard: false,
            // Repository scans discover models and read bounded metadata. Full
            // content identity is established only for a model selected for
            // qualification or launch, avoiding a terabyte-scale scan pass.
            artifact_identity: Default::default(),
        });
        inventory_meta.insert(idx, (cache_key, scan_root_key.to_string(), candidate_mtime));
        if let Some(lease) = inspection_lease {
            fresh_files.push((idx, lease));
        }
    }

    directory_records.push(InventoryDirectoryRecord::new(
        "model",
        dir_key,
        scan_root_key.to_string(),
        tree_signature,
    ));
    file_count
}

fn mark_sharded_models(models: &mut [ModelInfo]) {
    use regex_lite::Regex;
    let shard_re = Regex::new(r"(?i)^(.+?)-([0-9]{5})-of-([0-9]{5})\.gguf$").unwrap();
    let mut groups: HashMap<String, (u32, Vec<usize>)> = HashMap::new();
    for (i, model) in models.iter().enumerate() {
        if let Some(caps) = shard_re.captures(&model.name) {
            let base = caps.get(1).unwrap().as_str().to_string();
            let total: u32 = caps.get(3).unwrap().as_str().parse().unwrap_or(0);
            groups
                .entry(base)
                .or_insert_with(|| (total, Vec::new()))
                .1
                .push(i);
        }
    }
    for (expected_total, indices) in groups.values() {
        if indices.len() as u32 == *expected_total && *expected_total > 1 {
            for &idx in indices {
                models[idx].is_shard = true;
            }
        }
    }
}

// Cross-platform executable names.

// Cross-platform executable names.
#[cfg(target_os = "windows")]
const ENGINE_EXE_NAME: &str = "llama-server.exe";
#[cfg(not(target_os = "windows"))]
const ENGINE_EXE_NAME: &str = "llama-server";

// Engine packages commonly add vendor/backend/bin directories below the selected root.
// Keep traversal bounded so accidentally selecting a broad directory cannot recurse forever.
const MAX_ENGINE_SCAN_DEPTH: usize = 8;

#[derive(Debug)]
struct EngineTreeInspection {
    signature: String,
    executables: Vec<(PathBuf, PathBuf)>,
}

fn inspect_engine_tree(
    root: &Path,
    max_depth: usize,
    budget: &mut ScanBudget,
) -> Result<EngineTreeInspection, String> {
    // The marker guarantees that inventory created by the former two-level algorithm is not
    // reused after upgrading to recursive engine discovery.
    let mut signature_parts = vec!["engine-tree-signature-v2".to_string()];
    let mut executables = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|error| format!("failed to resolve engine root {}: {error}", root.display()))?;

    while let Some((dir, depth)) = pending.pop() {
        let canonical_dir = match budget.enter_directory(&dir, &canonical_root) {
            Ok(path) => path,
            Err(error) if depth == 0 => return Err(error),
            Err(error) => {
                signature_parts.push(format!("skipped|{}|{error}", path_identity_key(&dir)));
                continue;
            }
        };
        let identity = path_identity_key(&canonical_dir);
        signature_parts.push(format!("dir|{identity}"));

        let exe = dir.join(ENGINE_EXE_NAME);
        if let Ok(metadata) = std::fs::symlink_metadata(&exe) {
            if metadata.is_file() && !metadata_is_link_like(&metadata) {
                if let Ok(canonical_exe) = std::fs::canonicalize(&exe) {
                    if path_is_within(&canonical_exe, &canonical_root) {
                        budget.add_result()?;
                        let modified_ns = metadata
                            .modified()
                            .ok()
                            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|duration| duration.as_nanos())
                            .unwrap_or(0);
                        signature_parts.push(format!(
                            "exe|{}|{}|{}",
                            canonical_key(&canonical_exe),
                            metadata.len(),
                            modified_ns
                        ));
                        executables.push((dir.clone(), exe));
                    }
                }
            }
        }

        if depth >= max_depth {
            continue;
        }

        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if depth == 0 => {
                return Err(format!("failed to read {}: {}", dir.display(), error));
            }
            Err(_) => {
                signature_parts.push(format!("unreadable|{identity}"));
                continue;
            }
        };
        let mut children = Vec::new();
        for entry in entries {
            let Ok(entry) = entry else {
                signature_parts.push(format!("entry-error|{identity}"));
                continue;
            };
            let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
                signature_parts.push(format!("type-error|{}", path_identity_key(&entry.path())));
                continue;
            };
            budget.observe_entry(&metadata)?;
            if metadata.is_dir() && !metadata_is_link_like(&metadata) {
                let child = entry.path();
                if let Ok(canonical_child) = std::fs::canonicalize(&child) {
                    if path_is_within(&canonical_child, &canonical_root) {
                        signature_parts
                            .push(format!("child|{}", path_identity_key(&canonical_child)));
                        children.push(child);
                    }
                }
            }
        }
        children.sort();
        pending.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
    }

    signature_parts.sort();
    Ok(EngineTreeInspection {
        signature: stable_hash(&signature_parts),
        executables,
    })
}

// Engine info construction.
pub fn build_engine_info(
    dir: &Path,
    exe: &Path,
    _source: &str,
    deadline: std::time::Instant,
) -> Option<EngineInfo> {
    let name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("llama-server")
        .to_string();
    let version = name.clone();
    let backend = utils::detect_backend(dir);
    // #10: Use the canonical directory path as a stable ID so moved directories do not break references.
    let id = std::fs::canonicalize(dir)
        .unwrap_or_else(|_| dir.to_path_buf())
        .to_string_lossy()
        .to_string();
    let artifact_identity = crate::deployment_identity::artifact_identity_for_path_with_deadline(
        "engine", exe, deadline,
    )
    .ok()?;
    Some(EngineInfo {
        id,
        name: format!("{} ({})", name, backend),
        dir: dir.to_string_lossy().to_string(),
        exe: exe.to_string_lossy().to_string(),
        version,
        backend,
        custom_name: None,
        capabilities: Default::default(),
        artifact_identity,
    })
}

// Model scanning.
pub async fn scan_models(
    paths: Vec<String>,
    state: tauri::State<'_, AppState>,
    _app: tauri::AppHandle,
) -> Result<Vec<ModelInfo>, String> {
    if paths.len() > MAX_SCAN_ROOTS {
        return Err(format!("scan exceeds the {MAX_SCAN_ROOTS}-root budget"));
    }
    let generation = state.model_scan_generation.fetch_add(1, Ordering::AcqRel) + 1;
    let app_dir = utils::get_data_dir();
    let default_path = utils::get_default_models_dir();
    let default_path_for_check = default_path.clone();

    let scan_paths: Vec<PathBuf> = if paths.is_empty() {
        vec![default_path]
    } else {
        paths
            .iter()
            .map(|p| {
                let pb = PathBuf::from(p);
                if pb.is_relative() {
                    app_dir.join(p)
                } else {
                    pb
                }
            })
            .collect()
    };
    let scan_paths = scan_paths
        .into_iter()
        .map(|path| {
            if paths_equal(&path, &default_path_for_check) && !path.exists() {
                Ok(path)
            } else {
                crate::security::require_authorized_model_root(&path)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut seen_scan_roots = HashSet::new();
    let scan_paths = scan_paths
        .into_iter()
        .filter(|path| seen_scan_roots.insert(path_identity_key(path)))
        .collect::<Vec<_>>();

    let work_permit = tokio::time::timeout(
        GGUF_WORK_ADMISSION_TIMEOUT,
        GGUF_WORK_SLOTS.clone().acquire_owned(),
    )
    .await
    .map_err(|_| "GGUF inspection capacity is busy; retry later".to_string())?
    .map_err(|_| "GGUF inspection capacity is unavailable".to_string())?;

    let result = tokio::task::spawn_blocking(move || -> Result<Vec<ModelInfo>, Vec<String>> {
        let _work_permit = work_permit;
        let (inventory, directory_inventory) =
            model_inventory::load_model_scan_indexes().map_err(|err| vec![err])?;
        let mut models: Vec<ModelInfo> = Vec::new();
        let mut seen_display_paths = HashSet::new();
        let mut seen_inventory_paths = HashSet::new();
        let mut seen_directory_keys = HashSet::new();
        let mut scan_root_keys = HashSet::new();
        let mut inventory_meta: HashMap<usize, (String, String, u64)> = HashMap::new();
        let mut errors = Vec::new();
        let mut fresh_files: Vec<(usize, crate::deployment_identity::ArtifactInspectionLease)> =
            Vec::new();
        let mut directory_records: Vec<InventoryDirectoryRecord> = Vec::new();
        let mut budget = ScanBudget::default();

        for scan_root in &scan_paths {
            let root_str = scan_root.display().to_string();
            if !scan_root.exists() {
                if paths_equal(scan_root, &default_path_for_check) {
                    continue;
                }
                errors.push(format!("{} does not exist", root_str));
                continue;
            }
            if !scan_root.is_dir() {
                errors.push(format!("{} is not a directory", root_str));
                continue;
            }

            let scan_root_key = canonical_key(scan_root);
            scan_root_keys.insert(scan_root_key.clone());
            let file_count = scan_model_directory_incremental(
                scan_root,
                scan_root,
                &scan_root_key,
                0,
                &inventory,
                &directory_inventory,
                &mut models,
                &mut seen_display_paths,
                &mut seen_inventory_paths,
                &mut seen_directory_keys,
                &mut inventory_meta,
                &mut fresh_files,
                &mut directory_records,
                &mut errors,
                &mut budget,
            );

            if let Err(error) = budget.check_time() {
                push_scan_error_once(&mut errors, error);
                break;
            }

            if file_count == 0 {
                errors.push(format!("{} contains no .gguf model files", root_str));
            }
        }

        if errors.iter().any(|error| error.contains("scan exceeded")) {
            return Err(errors);
        }

        if !fresh_files.is_empty() {
            budget.check_time().map_err(|error| vec![error])?;
            let parse_deadline = budget.deadline();
            const MAX_GGUF_PARSE_WORKERS: usize = 4;
            let worker_count = std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(MAX_GGUF_PARSE_WORKERS)
                .clamp(1, MAX_GGUF_PARSE_WORKERS)
                .min(fresh_files.len());
            type MetadataParseResult = (
                usize,
                PathBuf,
                Result<crate::models::GgufMetadataSummary, String>,
            );
            let mut metadata_budget_error = None;
            std::thread::scope(|scope| {
                let (sender, receiver) =
                    std::sync::mpsc::sync_channel::<MetadataParseResult>(worker_count);
                let mut chunks = (0..worker_count).map(|_| Vec::new()).collect::<Vec<_>>();
                for (index, item) in fresh_files.drain(..).enumerate() {
                    chunks[index % worker_count].push(item);
                }
                for chunk in chunks {
                    let sender = sender.clone();
                    scope.spawn(move || {
                        for (model_idx, lease) in chunk {
                            let path = lease.canonical_path().to_path_buf();
                            let result = lease.try_clone_file().and_then(|file| {
                                utils::parse_gguf_metadata_from_open_file_with_deadline(
                                    file,
                                    &path,
                                    parse_deadline,
                                )
                            });
                            let result = result
                                .and_then(|summary| lease.verify_unchanged().map(|_| summary));
                            if sender.send((model_idx, path, result)).is_err() {
                                break;
                            }
                        }
                    });
                }
                drop(sender);

                for (model_idx, path, summary_result) in receiver {
                    if model_idx >= models.len() {
                        continue;
                    }
                    let summary = match summary_result {
                        Ok(summary) => summary,
                        Err(err) => {
                            errors.push(format!(
                                "{} metadata parse failed: {}",
                                path.display(),
                                err
                            ));
                            continue;
                        }
                    };
                    let retained = match serde_json::to_vec(&summary) {
                        Ok(retained) => retained,
                        Err(error) => {
                            metadata_budget_error =
                                Some(format!("cannot size retained GGUF metadata: {error}"));
                            break;
                        }
                    };
                    if let Err(error) = budget.retain_metadata(retained.len()) {
                        metadata_budget_error = Some(error);
                        break;
                    }
                    let model = &mut models[model_idx];
                    model.architecture = summary.architecture;
                    model.context_length = summary.context_length;
                    model.quant_type = summary.quant_type;
                    model.has_mtp_head = summary.capabilities.has_builtin_mtp;
                    model.capabilities = summary.capabilities;
                }
            });
            if let Some(error) = metadata_budget_error {
                return Err(vec![error]);
            }
            budget.check_time().map_err(|error| vec![error])?;
        }

        mark_sharded_models(&mut models);

        let records = models
            .iter()
            .enumerate()
            .filter_map(|(idx, model)| {
                inventory_meta
                    .get(&idx)
                    .map(|(cache_key, scan_root, mtime)| {
                        InventoryModelRecord::from_model(
                            model,
                            cache_key.clone(),
                            scan_root.clone(),
                            *mtime,
                        )
                    })
            })
            .collect::<Vec<_>>();
        model_inventory::apply_model_scan(
            &records,
            &directory_records,
            &scan_root_keys,
            &seen_inventory_paths,
            &seen_directory_keys,
        )
        .map_err(|err| vec![err])?;

        if models.is_empty() && !errors.is_empty() {
            Err(errors)
        } else {
            Ok(models)
        }
    })
    .await
    .map_err(|e| format!("scan thread failed: {:?}", e))?;

    let models = match result {
        Ok(models) => models,
        Err(errors) => return Err(errors.join("; ")),
    };

    if state.model_scan_generation.load(Ordering::Acquire) != generation {
        return Ok(state.models.lock().unwrap().clone());
    }
    let mut state_models = state.models.lock().unwrap();
    *state_models = models.clone();
    Ok(models)
}

// Batch load: scan and restore downloads in one IPC call.
type AppDataSnapshot = (
    Vec<ModelInfo>,
    Vec<EngineInfo>,
    Vec<crate::models::PersistedQueueEntry>,
);
type CachedScan = (Vec<ModelInfo>, Vec<EngineInfo>);

pub async fn load_app_data(
    paths: Vec<String>,
    engine_paths: Vec<String>,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<AppDataSnapshot, String> {
    let (models_result, engines_result) = tokio::join!(
        scan_models(paths, state.clone(), app.clone()),
        scan_engines(engine_paths, state.clone())
    );
    let models = models_result.unwrap_or_else(|_| Vec::new());
    let engines = engines_result.unwrap_or_else(|_| Vec::new());
    let queue = crate::commands::download::restore_runtime_queue_from_disk(&state, &app);
    Ok((models, engines, queue))
}

/// Reads cached scan results from disk so startup can show data before a full scan finishes.
pub async fn get_cached_scan(
    state: tauri::State<'_, AppState>,
) -> Result<Option<CachedScan>, String> {
    let mut models = model_inventory::list_cached_models()?;
    mark_sharded_models(&mut models);

    let mut engines = model_inventory::list_cached_engines()?;
    {
        let saved_names = state.engine_names.lock().unwrap();
        for engine in &mut engines {
            if let Some(cn) = saved_engine_name(&saved_names, &engine.id) {
                engine.custom_name = Some(cn.clone());
                engine.name = cn.clone();
            }
        }
    }

    if models.is_empty() && engines.is_empty() {
        return Ok(None);
    }

    // Write into state so other components can use it immediately.
    {
        let mut state_models = state.models.lock().unwrap();
        *state_models = models.clone();
        let mut state_engines = state.engines.lock().unwrap();
        *state_engines = engines.clone();
    }

    Ok(Some((models, engines)))
}

pub async fn get_models(state: tauri::State<'_, AppState>) -> Result<Vec<ModelInfo>, String> {
    Ok(state.models.lock().unwrap().clone())
}

pub async fn delete_model_file(
    path: String,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if std::fs::symlink_metadata(p)
        .map_err(|e| format!("路径无效: {}", e))?
        .file_type()
        .is_symlink()
    {
        return Err("Cannot delete symlinked model files".to_string());
    }
    if p.extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
        != Some("gguf".to_string())
    {
        return Err("只能删除 .gguf 文件".to_string());
    }
    let work_permit = tokio::time::timeout(
        GGUF_WORK_ADMISSION_TIMEOUT,
        GGUF_WORK_SLOTS.clone().acquire_owned(),
    )
    .await
    .map_err(|_| "GGUF deletion capacity is busy; retry later".to_string())?
    .map_err(|_| "GGUF deletion capacity is unavailable".to_string())?;
    let deadline = std::time::Instant::now() + MAX_SCAN_DURATION;
    let lease_path = path.clone();
    let mut lease = tokio::task::spawn_blocking(move || {
        crate::deployment_identity::ArtifactLease::open_authorized_for_removal_with_deadline(
            "model",
            Path::new(&lease_path),
            deadline,
        )
    })
    .await
    .map_err(|error| format!("GGUF deletion worker failed: {error}"))??;
    let canonical = lease.canonical_path().to_path_buf();
    let is_known = {
        let state_models = state.models.lock().unwrap();
        state_models
            .iter()
            .filter_map(|model| std::fs::canonicalize(&model.path).ok())
            .any(|model_path| paths_equal(&model_path, &canonical))
    };
    if !is_known {
        return Err("文件不在已扫描的模型列表中".to_string());
    }
    let referenced_by = {
        let instances = state.instances.lock().unwrap();
        instances_referencing_model(&instances, &canonical)
    };
    if !referenced_by.is_empty() {
        return Err(format!(
            "模型文件正被实例引用，无法删除: {}",
            referenced_by.join(", ")
        ));
    }
    let confirmation_path = canonical.display().to_string();
    let approved = tokio::task::spawn_blocking(move || {
        app.dialog()
            .message(format!(
                "确认永久删除已验证模型？\n\n{confirmation_path}\n\n此操作无法撤销。"
            ))
            .title("确认删除模型")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "永久删除".to_string(),
                "取消".to_string(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|error| format!("model deletion confirmation failed: {error}"))?;
    if !approved {
        return Err("Model deletion was not approved".to_string());
    }
    tokio::task::spawn_blocking(move || lease.remove_verified_with_deadline(deadline))
        .await
        .map_err(|error| format!("GGUF deletion worker failed: {error}"))?
        .map_err(|error| format!("删除文件失败: {error}"))?;
    drop(work_permit);
    let _ = model_inventory::delete_model(&canonical.to_string_lossy());
    let mut models = state.models.lock().unwrap();
    models.retain(|model| {
        std::fs::canonicalize(&model.path)
            .map(|model_path| !paths_equal(&model_path, &canonical))
            .unwrap_or(true)
    });
    Ok(())
}

pub async fn open_model_folder(path: String) -> Result<(), String> {
    let canonical = crate::security::require_authorized_model_path(Path::new(&path))?;
    let parent = canonical.parent().unwrap_or(Path::new("."));
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(parent)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(parent)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    Ok(())
}

pub async fn read_gguf_metadata(
    path: String,
) -> Result<crate::models::GgufMetadataSummary, String> {
    let work_permit = tokio::time::timeout(
        GGUF_WORK_ADMISSION_TIMEOUT,
        GGUF_WORK_SLOTS.clone().acquire_owned(),
    )
    .await
    .map_err(|_| "GGUF inspection capacity is busy; retry later".to_string())?
    .map_err(|_| "GGUF inspection capacity is unavailable".to_string())?;
    let deadline = std::time::Instant::now() + MAX_SCAN_DURATION;
    tokio::task::spawn_blocking(move || {
        let _work_permit = work_permit;
        let mut lease = crate::deployment_identity::ArtifactLease::open_authorized_with_deadline(
            "model",
            Path::new(&path),
            deadline,
        )?;
        let file = lease.try_clone_file()?;
        let result = utils::parse_gguf_metadata_from_open_file_with_deadline(
            file,
            lease.canonical_path(),
            deadline,
        )?;
        lease.verify_unchanged_with_deadline(deadline)?;
        Ok(result)
    })
    .await
    .map_err(|error| format!("GGUF metadata worker failed: {error}"))?
}

// Engine scanning.
pub async fn scan_engines(
    paths: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<EngineInfo>, String> {
    if paths.len() > MAX_SCAN_ROOTS {
        return Err(format!("scan exceeds the {MAX_SCAN_ROOTS}-root budget"));
    }
    let generation = state.engine_scan_generation.fetch_add(1, Ordering::AcqRel) + 1;
    let paths = paths
        .into_iter()
        .map(|path| {
            crate::security::require_authorized_engine_root(Path::new(&path))
                .map(|canonical| canonical.to_string_lossy().to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut seen_scan_roots = HashSet::new();
    let paths = paths
        .into_iter()
        .filter(|path| seen_scan_roots.insert(path_identity_key(Path::new(path))))
        .collect::<Vec<_>>();
    let mut engines = tokio::task::spawn_blocking(move || -> Result<Vec<EngineInfo>, String> {
        let (inventory, directory_inventory) = model_inventory::load_engine_scan_indexes()?;
        let mut engines: Vec<EngineInfo> = Vec::new();
        let mut engine_records: Vec<InventoryEngineRecord> = Vec::new();
        let mut directory_records: Vec<InventoryDirectoryRecord> = Vec::new();
        let mut seen = HashSet::new();
        let mut seen_inventory_ids = HashSet::new();
        let mut seen_directory_keys = HashSet::new();
        let mut scan_root_keys = HashSet::new();
        let app_dir = utils::get_data_dir();
        let mut budget = ScanBudget::default();

        let engines_dir = app_dir.join("engines");
        if engines_dir.exists() {
            let scan_root_key = canonical_key(&engines_dir);
            scan_root_keys.insert(scan_root_key.clone());
            let inspection = inspect_engine_tree(&engines_dir, MAX_ENGINE_SCAN_DEPTH, &mut budget)?;
            if try_reuse_engine_root(
                &scan_root_key,
                &inspection.signature,
                &inventory,
                &directory_inventory,
                &mut seen_directory_keys,
                &mut directory_records,
                &mut seen_inventory_ids,
                &mut engines,
                &mut engine_records,
            )? {
                // Cached entries for this unchanged root have already been appended.
            } else {
                for (dir, exe) in inspection.executables {
                    let norm = engine_path_identity(&dir);
                    if seen.insert(norm) {
                        push_indexed_engine(
                            &dir,
                            &exe,
                            &scan_root_key,
                            &inventory,
                            &mut seen_inventory_ids,
                            (&mut engines, &mut engine_records),
                            &mut budget,
                        )?;
                    }
                }
            }
        }

        for p in &paths {
            let root = PathBuf::from(p);
            if !root.exists() || !root.is_dir() {
                continue;
            }
            let scan_root_key = canonical_key(&root);
            scan_root_keys.insert(scan_root_key.clone());
            let inspection = inspect_engine_tree(&root, MAX_ENGINE_SCAN_DEPTH, &mut budget)?;
            if try_reuse_engine_root(
                &scan_root_key,
                &inspection.signature,
                &inventory,
                &directory_inventory,
                &mut seen_directory_keys,
                &mut directory_records,
                &mut seen_inventory_ids,
                &mut engines,
                &mut engine_records,
            )? {
                continue;
            }
            for (dir, exe) in inspection.executables {
                let norm = engine_path_identity(&dir);
                if seen.insert(norm) {
                    push_indexed_engine(
                        &dir,
                        &exe,
                        &scan_root_key,
                        &inventory,
                        &mut seen_inventory_ids,
                        (&mut engines, &mut engine_records),
                        &mut budget,
                    )?;
                }
            }
        }

        model_inventory::apply_engine_scan(
            &engine_records,
            &directory_records,
            &scan_root_keys,
            &seen_inventory_ids,
            &seen_directory_keys,
        )?;
        Ok(engines)
    })
    .await
    .map_err(|e| format!("scan thread failed: {}", e))??;

    if state.engine_scan_generation.load(Ordering::Acquire) != generation {
        return Ok(state.engines.lock().unwrap().clone());
    }

    // Preserve custom engine names; state access stays outside spawn_blocking.
    {
        let saved_names = state.engine_names.lock().unwrap();
        for engine in &mut engines {
            if let Some(cn) = saved_engine_name(&saved_names, &engine.id) {
                engine.custom_name = Some(cn.clone());
                engine.name = cn.clone();
            }
        }
    }

    let finalized = {
        let mut state_engines = state.engines.lock().unwrap();
        merge_scanned_engine_capabilities(&mut engines, &state_engines);
        *state_engines = engines.clone();
        // Keep the state lock through persistence. A concurrent probe uses the same lock for its
        // state and cache update, so an older scan snapshot cannot be written after a newer probe.
        for engine in state_engines.iter() {
            let _ = model_inventory::update_engine_probe(engine);
        }
        state_engines.clone()
    };
    Ok(finalized)
}

pub async fn get_engines(state: tauri::State<'_, AppState>) -> Result<Vec<EngineInfo>, String> {
    Ok(state.engines.lock().unwrap().clone())
}

#[cfg(test)]
mod incremental_scan_tests {
    use super::*;

    fn temp_test_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lsm-incremental-{}-{}", name, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn directory_fingerprint_changes_when_model_file_is_added() {
        let dir = temp_test_dir("fingerprint");
        let initial = read_directory_fingerprint(&dir, &dir, &mut ScanBudget::default()).unwrap();

        std::fs::write(dir.join("model.gguf"), b"test").unwrap();

        let updated = read_directory_fingerprint(&dir, &dir, &mut ScanBudget::default()).unwrap();
        assert_ne!(initial.signature, updated.signature);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn retired_sampled_identities_are_never_reused() {
        let retired = crate::deployment_identity::ArtifactIdentity {
            schema_version: crate::deployment_identity::ARTIFACT_IDENTITY_SCHEMA_VERSION,
            kind: "engine".to_string(),
            artifact_id: "urn:lsm:engine:v1:sha256:retired".to_string(),
            algorithm: "sha256-sampled-v1".to_string(),
            file_size: 1024,
            sample_size: 64,
            sample_count: 3,
        };
        let verified = crate::deployment_identity::ArtifactIdentity {
            algorithm: "sha256-full-v1".to_string(),
            sample_size: 0,
            sample_count: 0,
            ..retired.clone()
        };

        assert!(!cached_artifact_identity_is_reusable(&retired));
        assert!(cached_artifact_identity_is_reusable(&verified));
    }

    #[test]
    fn path_under_directory_uses_component_boundaries() {
        let parent = PathBuf::from("models").join("foo");
        let child = parent.join("bar").join("model.gguf");
        let sibling = PathBuf::from("models").join("foo2").join("model.gguf");

        assert!(path_is_under_directory(&child, &parent));
        assert!(!path_is_under_directory(&sibling, &parent));
    }

    #[test]
    fn scan_merge_preserves_a_newer_probe_and_invalidates_changed_executables() {
        let dir = temp_test_dir("engine-capability-merge");
        let exe = dir.join("llama-server-test");
        std::fs::write(&exe, vec![b'a'; 128 * 1024]).unwrap();
        let fingerprint =
            crate::commands::engine_capabilities::executable_fingerprint(&exe.to_string_lossy());
        let mut current = EngineInfo {
            id: "engine-1".to_string(),
            name: "engine".to_string(),
            dir: dir.to_string_lossy().to_string(),
            exe: exe.to_string_lossy().to_string(),
            version: "version: 100".to_string(),
            backend: "CPU".to_string(),
            custom_name: None,
            capabilities: crate::models::EngineCapabilities {
                status: "detected".to_string(),
                version_status: "detected".to_string(),
                executable_fingerprint: fingerprint,
                probed_at: Some(100),
                qualification: crate::models::EngineQualificationReport {
                    status: "passed".to_string(),
                    executable_fingerprint:
                        crate::commands::engine_capabilities::executable_fingerprint(
                            &exe.to_string_lossy(),
                        ),
                    checks: vec![crate::models::EngineQualificationCheck {
                        name: "inference".to_string(),
                        status: "passed".to_string(),
                        duration_ms: 10,
                        detail: None,
                    }],
                    completed_at: Some(100),
                    ..crate::models::EngineQualificationReport::default()
                },
                ..crate::models::EngineCapabilities::default()
            },
            artifact_identity: crate::deployment_identity::artifact_identity_for_path(
                "engine", &exe,
            )
            .unwrap(),
        };
        let mut scanned = vec![EngineInfo {
            capabilities: crate::models::EngineCapabilities::default(),
            version: String::new(),
            ..current.clone()
        }];
        merge_scanned_engine_capabilities(&mut scanned, &[current.clone()]);
        assert_eq!(scanned[0].version, "version: 100");
        assert_eq!(scanned[0].capabilities.status, "detected");
        assert_eq!(scanned[0].capabilities.qualification.status, "passed");

        std::fs::write(&exe, vec![b'b'; 128 * 1024]).unwrap();
        current.capabilities.probed_at = Some(200);
        merge_scanned_engine_capabilities(&mut scanned, &[current]);
        assert_eq!(scanned[0].capabilities.status, "unprobed");
        assert_eq!(scanned[0].capabilities.qualification.status, "stale");
        assert_eq!(scanned[0].capabilities.qualification.checks.len(), 1);
        assert!(scanned[0].version.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tree_signature_changes_for_nested_engine_file() {
        let dir = temp_test_dir("engine-tree");
        let nested = dir.join("vendor").join("backend").join("bin");
        std::fs::create_dir_all(&nested).unwrap();
        let initial = inspect_engine_tree(&dir, MAX_ENGINE_SCAN_DEPTH, &mut ScanBudget::default())
            .unwrap()
            .signature;

        std::fs::write(nested.join(ENGINE_EXE_NAME), b"exe").unwrap();

        let updated = inspect_engine_tree(&dir, MAX_ENGINE_SCAN_DEPTH, &mut ScanBudget::default())
            .unwrap()
            .signature;
        assert_ne!(initial, updated);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn engine_scan_discovers_nested_vendor_backend_layouts() {
        let dir = temp_test_dir("engine-depth");
        let atomic = dir.join("atomic");
        let hip = dir.join("poolside").join("hip-rocm714").join("bin");
        let vulkan = dir.join("poolside").join("vulkan").join("bin");
        for engine_dir in [&atomic, &hip, &vulkan] {
            std::fs::create_dir_all(engine_dir).unwrap();
            std::fs::write(engine_dir.join(ENGINE_EXE_NAME), b"exe").unwrap();
        }

        let discovered =
            inspect_engine_tree(&dir, MAX_ENGINE_SCAN_DEPTH, &mut ScanBudget::default())
                .unwrap()
                .executables;
        let discovered_dirs = discovered
            .iter()
            .map(|(engine_dir, _)| engine_path_identity(engine_dir))
            .collect::<HashSet<_>>();

        assert_eq!(discovered.len(), 3);
        assert!(discovered_dirs.contains(&engine_path_identity(&atomic)));
        assert!(discovered_dirs.contains(&engine_path_identity(&hip)));
        assert!(discovered_dirs.contains(&engine_path_identity(&vulkan)));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn engine_scan_stops_at_the_configured_depth_limit() {
        let dir = temp_test_dir("engine-depth-limit");
        let mut too_deep = dir.clone();
        for depth in 0..=MAX_ENGINE_SCAN_DEPTH {
            too_deep = too_deep.join(format!("level-{depth}"));
        }
        std::fs::create_dir_all(&too_deep).unwrap();
        std::fs::write(too_deep.join(ENGINE_EXE_NAME), b"exe").unwrap();

        assert!(
            inspect_engine_tree(&dir, MAX_ENGINE_SCAN_DEPTH, &mut ScanBudget::default())
                .unwrap()
                .executables
                .is_empty()
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn local_signature_changes_when_a_deep_model_is_rewritten() {
        let dir = temp_test_dir("model-tree");
        let nested = dir.join("vendor").join("family").join("quant");
        std::fs::create_dir_all(&nested).unwrap();
        let model = nested.join("model.gguf");
        std::fs::write(&model, b"first-model-payload").unwrap();
        let initial = read_directory_fingerprint(&nested, &dir, &mut ScanBudget::default())
            .unwrap()
            .signature;

        // Use a different size so the assertion stays valid on filesystems whose
        // modification timestamps are coarser than the test's execution time.
        std::fs::write(&model, b"other-longer-model-payload").unwrap();

        let updated = read_directory_fingerprint(&nested, &dir, &mut ScanBudget::default())
            .unwrap()
            .signature;
        assert_ne!(initial, updated);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unchanged_discovery_record_is_reused_without_reparsing_or_rehashing() {
        let dir = temp_test_dir("malformed-model-cache");
        crate::persistence::enforce_private_directory(&dir).unwrap();
        let malformed = dir.join("malformed.gguf");
        std::fs::write(&malformed, b"not-a-gguf").unwrap();
        crate::persistence::enforce_private_file(&malformed).unwrap();
        // Windows runners can expose the temporary directory through an 8.3 alias while
        // canonicalizing its children to the long path. Mirror the production scan entry
        // point by passing the canonical root to the incremental scanner.
        let canonical_dir = std::fs::canonicalize(&dir).unwrap();
        let canonical_malformed = std::fs::canonicalize(&malformed).unwrap();
        assert!(path_is_within(&canonical_malformed, &canonical_dir));
        let scan_root_key = canonical_key(&canonical_dir);
        let mut models = Vec::new();
        let mut seen_display_paths = HashSet::new();
        let mut seen_inventory_paths = HashSet::new();
        let mut seen_directory_keys = HashSet::new();
        let mut inventory_meta = HashMap::new();
        let mut fresh_files = Vec::new();
        let mut directory_records = Vec::new();
        let mut errors = Vec::new();
        let mut budget = ScanBudget::default();

        scan_model_directory_incremental(
            &canonical_dir,
            &canonical_dir,
            &scan_root_key,
            0,
            &HashMap::new(),
            &HashMap::new(),
            &mut models,
            &mut seen_display_paths,
            &mut seen_inventory_paths,
            &mut seen_directory_keys,
            &mut inventory_meta,
            &mut fresh_files,
            &mut directory_records,
            &mut errors,
            &mut budget,
        );
        assert_eq!(fresh_files.len(), 1, "{errors:?}");
        assert!(!models[0].artifact_identity.is_verified());
        let (cache_key, stored_root, mtime) = inventory_meta.get(&0).unwrap().clone();
        let cached =
            InventoryModelRecord::from_model(&models[0], cache_key.clone(), stored_root, mtime);
        let inventory = HashMap::from([(cache_key, cached)]);

        models.clear();
        seen_display_paths.clear();
        seen_inventory_paths.clear();
        seen_directory_keys.clear();
        inventory_meta.clear();
        fresh_files.clear();
        directory_records.clear();
        errors.clear();
        let mut budget = ScanBudget::default();
        scan_model_directory_incremental(
            &canonical_dir,
            &canonical_dir,
            &scan_root_key,
            0,
            &inventory,
            &HashMap::new(),
            &mut models,
            &mut seen_display_paths,
            &mut seen_inventory_paths,
            &mut seen_directory_keys,
            &mut inventory_meta,
            &mut fresh_files,
            &mut directory_records,
            &mut errors,
            &mut budget,
        );

        assert_eq!(models.len(), 1);
        assert!(fresh_files.is_empty());

        let mut retired_inventory = inventory.clone();
        let retired_identity = &mut retired_inventory
            .values_mut()
            .next()
            .unwrap()
            .artifact_identity;
        retired_identity.algorithm = "sha256-sampled-v1".to_string();
        retired_identity.sample_size = 64;
        retired_identity.sample_count = 3;
        models.clear();
        seen_display_paths.clear();
        seen_inventory_paths.clear();
        seen_directory_keys.clear();
        inventory_meta.clear();
        fresh_files.clear();
        directory_records.clear();
        errors.clear();
        let mut budget = ScanBudget::default();
        scan_model_directory_incremental(
            &canonical_dir,
            &canonical_dir,
            &scan_root_key,
            0,
            &retired_inventory,
            &HashMap::new(),
            &mut models,
            &mut seen_display_paths,
            &mut seen_inventory_paths,
            &mut seen_directory_keys,
            &mut inventory_meta,
            &mut fresh_files,
            &mut directory_records,
            &mut errors,
            &mut budget,
        );
        assert!(fresh_files.is_empty(), "{errors:?}");
        assert!(!models[0].artifact_identity.is_verified());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn engine_path_identity_respects_platform_case_rules() {
        let upper = Path::new("/opt/llama/CUDA");
        let lower = Path::new("/opt/llama/cuda");
        #[cfg(target_os = "windows")]
        assert_eq!(engine_path_identity(upper), engine_path_identity(lower));
        #[cfg(not(target_os = "windows"))]
        assert_ne!(engine_path_identity(upper), engine_path_identity(lower));
    }

    #[test]
    fn referenced_models_and_engines_are_identified_before_deletion() {
        let mut instances = HashMap::new();
        instances.insert(
            "primary".into(),
            InstanceConfig {
                name: "Primary".into(),
                model_path: "/models/chat.gguf".into(),
                engine_id: "engine-1".into(),
                ..InstanceConfig::default()
            },
        );

        assert_eq!(
            instances_referencing_model(&instances, Path::new("/models/chat.gguf")),
            vec!["Primary"]
        );
        assert_eq!(
            instances_referencing_engine(&instances, "engine-1"),
            vec!["Primary"]
        );
        assert!(instances_referencing_engine(&instances, "engine-2").is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn engine_references_and_names_accept_windows_namespace_aliases() {
        let stored = r"\\?\C:\Engines\llama-server";
        let configured = r"c:\engines\llama-server\";
        let mut instances = HashMap::new();
        instances.insert(
            "primary".into(),
            InstanceConfig {
                name: "Primary".into(),
                engine_id: configured.into(),
                ..InstanceConfig::default()
            },
        );
        assert_eq!(
            instances_referencing_engine(&instances, stored),
            vec!["Primary"]
        );

        let names = HashMap::from([(configured.to_string(), "Custom".to_string())]);
        assert_eq!(
            saved_engine_name(&names, stored).map(String::as_str),
            Some("Custom")
        );
    }
}

pub async fn delete_engine(id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let referenced_by = instances_referencing_engine(&state.instances.lock().unwrap(), &id);
    if !referenced_by.is_empty() {
        return Err(format!(
            "引擎正被实例引用，无法删除: {}",
            referenced_by.join(", ")
        ));
    }
    let mut engines = state.engines.lock().unwrap();
    let stored_id = engines
        .iter()
        .find(|engine| paths_equal(Path::new(&engine.id), Path::new(&id)))
        .map(|engine| engine.id.clone())
        .unwrap_or_else(|| id.clone());
    engines.retain(|engine| !paths_equal(Path::new(&engine.id), Path::new(&id)));
    let _ = model_inventory::delete_engine(&stored_id);
    Ok(())
}

pub async fn rename_engine(
    id: String,
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut engines = state.engines.lock().unwrap();
    let Some(engine_index) = engines
        .iter()
        .position(|engine| paths_equal(Path::new(&engine.id), Path::new(&id)))
    else {
        return Err("未找到要重命名的引擎".to_string());
    };
    let stored_id = engines[engine_index].id.clone();
    let mut next_engine_names = state.engine_names.lock().unwrap().clone();
    next_engine_names
        .retain(|saved_id, _| !paths_equal(Path::new(saved_id), Path::new(&stored_id)));
    next_engine_names.insert(stored_id, name.clone());

    // Do not publish either in-memory name until the atomic config write succeeds.
    crate::commands::config::replace_engine_names_and_persist(&state, next_engine_names)?;
    let engine = engines
        .get_mut(engine_index)
        .ok_or_else(|| "未找到要重命名的引擎".to_string())?;
    engine.custom_name = Some(name.clone());
    engine.name = name;
    Ok(())
}

pub async fn open_engine_folder(
    engine_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let executable = state
        .engines
        .lock()
        .map_err(|_| "engine state lock is poisoned".to_string())?
        .iter()
        .find(|engine| paths_equal(Path::new(&engine.id), Path::new(&engine_id)))
        .map(|engine| PathBuf::from(&engine.exe))
        .ok_or_else(|| "未找到受管理的引擎".to_string())?;
    let (canonical_executable, _) =
        crate::security::require_authorized_artifact_path("engine", &executable)?;
    let dir = canonical_executable
        .parent()
        .ok_or_else(|| "引擎可执行文件没有父目录".to_string())?;
    let metadata =
        std::fs::symlink_metadata(dir).map_err(|error| format!("无法检查引擎目录: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("引擎目录必须是本地非链接目录".to_string());
    }
    #[cfg(windows)]
    if dir.to_string_lossy().starts_with(r"\\") {
        return Err("拒绝打开 UNC 或网络引擎目录".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(dir)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(dir)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    Ok(())
}

// IPC compatibility boundary: legacy command internals keep their existing error flow,
// while every registered command serializes a stable AppError object.
#[allow(dead_code, unused_imports, unused_mut)] // Tauri references adapters through generated macros.
pub mod ipc {
    use super::*;

    #[tauri::command]
    pub async fn scan_models(
        paths: Vec<String>,
        state: tauri::State<'_, AppState>,
        _app: tauri::AppHandle,
    ) -> crate::error::AppResult<Vec<ModelInfo>> {
        super::scan_models(paths, state, _app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn load_app_data(
        paths: Vec<String>,
        engine_paths: Vec<String>,
        state: tauri::State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<AppDataSnapshot> {
        super::load_app_data(paths, engine_paths, state, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_cached_scan(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<Option<CachedScan>> {
        super::get_cached_scan(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_models(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<Vec<ModelInfo>> {
        super::get_models(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn delete_model_file(
        path: String,
        state: tauri::State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<()> {
        super::delete_model_file(path, state, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn open_model_folder(path: String) -> crate::error::AppResult<()> {
        super::open_model_folder(path)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn read_gguf_metadata(
        path: String,
    ) -> crate::error::AppResult<crate::models::GgufMetadataSummary> {
        super::read_gguf_metadata(path)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn scan_engines(
        paths: Vec<String>,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<Vec<EngineInfo>> {
        super::scan_engines(paths, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_engines(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<Vec<EngineInfo>> {
        super::get_engines(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn delete_engine(
        id: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::delete_engine(id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn rename_engine(
        id: String,
        name: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::rename_engine(id, name, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn open_engine_folder(
        engine_id: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::open_engine_folder(engine_id, state)
            .await
            .map_err(crate::error::AppError::from)
    }
}
