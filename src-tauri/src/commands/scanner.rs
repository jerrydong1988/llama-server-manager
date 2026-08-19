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
    mtime: u64,
    mtime_ns: u128,
}

#[derive(Debug, Clone)]
struct DirectoryFingerprint {
    signature: String,
    entries: Vec<DirectoryEntryFingerprint>,
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

fn read_directory_fingerprint(path: &Path) -> Result<DirectoryFingerprint, String> {
    let mut entries = Vec::new();
    for entry in
        std::fs::read_dir(path).map_err(|e| format!("failed to read {}: {}", path.display(), e))?
    {
        let entry = entry.map_err(|e| format!("failed to read {} entry: {}", path.display(), e))?;
        let file_type = entry
            .file_type()
            .map_err(|e| format!("failed to read {} file type: {}", entry.path().display(), e))?;
        let metadata = entry.metadata().ok();
        let modified = metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok());
        entries.push(DirectoryEntryFingerprint {
            path: entry.path(),
            name: entry.file_name().to_string_lossy().to_string(),
            is_dir: file_type.is_dir(),
            is_file: file_type.is_file(),
            is_symlink: file_type.is_symlink(),
            size: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
            mtime: modified.map(|duration| duration.as_secs()).unwrap_or(0),
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

fn read_directory_tree_signature(path: &Path, max_depth: usize) -> Result<String, String> {
    fn collect(
        path: &Path,
        depth: usize,
        max_depth: usize,
        parts: &mut Vec<String>,
    ) -> Result<(), String> {
        let fingerprint = read_directory_fingerprint(path)?;
        let dir_key = canonical_key(path);
        parts.push(format!("dir|{}|{}", dir_key, fingerprint.signature));
        if depth >= max_depth {
            return Ok(());
        }
        for entry in fingerprint.entries {
            if entry.is_dir && !entry.is_symlink {
                collect(&entry.path, depth + 1, max_depth, parts)?;
            }
        }
        Ok(())
    }

    let mut parts = Vec::new();
    collect(path, 0, max_depth, &mut parts)?;
    parts.sort();
    Ok(stable_hash(&parts))
}

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

fn cached_models_under_directory(
    dir_key: &str,
    inventory: &HashMap<String, InventoryModelRecord>,
) -> Vec<InventoryModelRecord> {
    let dir = Path::new(dir_key);
    inventory
        .values()
        .filter(|record| path_is_under_directory(Path::new(&record.path), dir))
        .cloned()
        .collect()
}

const MAX_MODEL_SCAN_DEPTH: usize = 32;

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
        .unwrap_or(false);
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
    engines: &mut Vec<EngineInfo>,
    engine_records: &mut Vec<InventoryEngineRecord>,
) {
    let cache_key = canonical_key(dir);
    let exe_mtime = file_mtime(exe);
    seen_inventory_ids.insert(cache_key.clone());

    if let Some(record) = inventory.get(&cache_key) {
        if record.exe_mtime == exe_mtime {
            let info = record.to_engine_info();
            engine_records.push(InventoryEngineRecord::from_engine(
                &info,
                exe_mtime,
                scan_root_key.to_string(),
            ));
            engines.push(info);
            return;
        }
    }

    if let Some(info) = build_engine_info(dir, exe, "") {
        engine_records.push(InventoryEngineRecord::from_engine(
            &info,
            exe_mtime,
            scan_root_key.to_string(),
        ));
        engines.push(info);
    }
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
    scan_root_key: &str,
    depth: usize,
    inventory: &HashMap<String, InventoryModelRecord>,
    directory_inventory: &HashMap<String, InventoryDirectoryRecord>,
    models: &mut Vec<ModelInfo>,
    seen_display_paths: &mut HashSet<String>,
    seen_inventory_paths: &mut HashSet<String>,
    seen_directory_keys: &mut HashSet<String>,
    inventory_meta: &mut HashMap<usize, (String, String, u64)>,
    fresh_files: &mut Vec<(usize, PathBuf)>,
    directory_records: &mut Vec<InventoryDirectoryRecord>,
    errors: &mut Vec<String>,
) -> usize {
    let dir_key = canonical_key(dir);
    seen_directory_keys.insert(dir_key.clone());

    let fingerprint = match read_directory_fingerprint(dir) {
        Ok(fingerprint) => fingerprint,
        Err(err) => {
            errors.push(err);
            return 0;
        }
    };
    let tree_signature =
        match read_directory_tree_signature(dir, MAX_MODEL_SCAN_DEPTH.saturating_sub(depth)) {
            Ok(signature) => signature,
            Err(error) => {
                errors.push(error);
                return 0;
            }
        };

    if directory_inventory
        .get(&dir_key)
        .map(|record| record.signature == tree_signature)
        .unwrap_or(false)
    {
        let mut reused = 0;
        for record in cached_models_under_directory(&dir_key, inventory) {
            let mut model = record.to_model_info();
            if !seen_display_paths.insert(path_identity_key(Path::new(&model.path))) {
                continue;
            }
            model.is_shard = false;
            let idx = models.len();
            seen_inventory_paths.insert(record.path.clone());
            models.push(model);
            inventory_meta.insert(
                idx,
                (record.path.clone(), record.scan_root.clone(), record.mtime),
            );
            reused += 1;
        }
        directory_records.push(InventoryDirectoryRecord::new(
            "model",
            dir_key,
            scan_root_key.to_string(),
            tree_signature,
        ));
        return reused;
    }

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
                    scan_root_key,
                    depth + 1,
                    inventory,
                    directory_inventory,
                    models,
                    seen_display_paths,
                    seen_inventory_paths,
                    seen_directory_keys,
                    inventory_meta,
                    fresh_files,
                    directory_records,
                    errors,
                );
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

        let file_path = entry.path.to_string_lossy().to_string();
        if !seen_display_paths.insert(path_identity_key(Path::new(&file_path))) {
            continue;
        }

        let cache_key = canonical_key(&entry.path);
        seen_inventory_paths.insert(cache_key.clone());
        file_count += 1;

        if let Some(record) = inventory.get(&cache_key) {
            if record.mtime == entry.mtime && record.size == entry.size {
                let idx = models.len();
                let mut model = record.to_model_info();
                model.name = entry.name.clone();
                model.path = file_path;
                model.size = entry.size;
                model.is_shard = false;
                models.push(model);
                inventory_meta.insert(idx, (cache_key, scan_root_key.to_string(), entry.mtime));
                continue;
            }
        }

        let idx = models.len();
        models.push(ModelInfo {
            id: uuid::Uuid::new_v4().to_string(),
            name: entry.name,
            path: file_path,
            size: entry.size,
            architecture: None,
            context_length: None,
            quant_type: None,
            has_mtp_head: false,
            capabilities: ModelCapabilities::default(),
            file_type: utils::classify_gguf_file(&entry.path).to_string(),
            is_shard: false,
        });
        inventory_meta.insert(idx, (cache_key, scan_root_key.to_string(), entry.mtime));
        fresh_files.push((idx, entry.path));
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

fn inspect_engine_tree(root: &Path, max_depth: usize) -> Result<EngineTreeInspection, String> {
    // The marker guarantees that inventory created by the former two-level algorithm is not
    // reused after upgrading to recursive engine discovery.
    let mut signature_parts = vec!["engine-tree-signature-v2".to_string()];
    let mut executables = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut visited = HashSet::new();
    let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());

    while let Some((dir, depth)) = pending.pop() {
        let canonical_dir = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if !paths_equal(&canonical_dir, &canonical_root)
            && !path_is_within(&canonical_dir, &canonical_root)
        {
            continue;
        }
        let identity = path_identity_key(&canonical_dir);
        if !visited.insert(identity.clone()) {
            continue;
        }
        signature_parts.push(format!("dir|{identity}"));

        let exe = dir.join(ENGINE_EXE_NAME);
        if let Ok(metadata) = exe.metadata() {
            if metadata.is_file() {
                let canonical_exe = std::fs::canonicalize(&exe).unwrap_or_else(|_| exe.clone());
                if path_is_within(&canonical_exe, &canonical_root) {
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
            let Ok(file_type) = entry.file_type() else {
                signature_parts.push(format!("type-error|{}", path_identity_key(&entry.path())));
                continue;
            };
            if file_type.is_dir() && !file_type.is_symlink() {
                let child = entry.path();
                signature_parts.push(format!("child|{}", engine_path_identity(&child)));
                children.push(child);
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
pub fn build_engine_info(dir: &Path, exe: &Path, _source: &str) -> Option<EngineInfo> {
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
    Some(EngineInfo {
        id,
        name: format!("{} ({})", name, backend),
        dir: dir.to_string_lossy().to_string(),
        exe: exe.to_string_lossy().to_string(),
        version,
        backend,
        custom_name: None,
        capabilities: Default::default(),
    })
}

// Model scanning.
pub async fn scan_models(
    paths: Vec<String>,
    state: tauri::State<'_, AppState>,
    _app: tauri::AppHandle,
) -> Result<Vec<ModelInfo>, String> {
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

    let result = tokio::task::spawn_blocking(move || -> Result<Vec<ModelInfo>, Vec<String>> {
        let (inventory, directory_inventory) =
            model_inventory::load_model_scan_indexes().map_err(|err| vec![err])?;
        let mut models: Vec<ModelInfo> = Vec::new();
        let mut seen_display_paths = HashSet::new();
        let mut seen_inventory_paths = HashSet::new();
        let mut seen_directory_keys = HashSet::new();
        let mut scan_root_keys = HashSet::new();
        let mut inventory_meta: HashMap<usize, (String, String, u64)> = HashMap::new();
        let mut errors = Vec::new();
        let mut fresh_files: Vec<(usize, PathBuf)> = Vec::new();
        let mut directory_records: Vec<InventoryDirectoryRecord> = Vec::new();

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
            );

            if file_count == 0 {
                errors.push(format!("{} contains no .gguf model files", root_str));
            }
        }

        if !fresh_files.is_empty() {
            const MAX_GGUF_PARSE_WORKERS: usize = 4;
            let worker_count = std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(MAX_GGUF_PARSE_WORKERS)
                .clamp(1, MAX_GGUF_PARSE_WORKERS)
                .min(fresh_files.len());
            let chunk_size = fresh_files.len().div_ceil(worker_count);
            type MetadataParseResult = (
                usize,
                PathBuf,
                Result<crate::models::GgufMetadataSummary, String>,
            );
            std::thread::scope(|scope| {
                let (sender, receiver) =
                    std::sync::mpsc::sync_channel::<MetadataParseResult>(worker_count);
                for chunk in fresh_files.chunks(chunk_size) {
                    let chunk: Vec<_> = chunk.to_vec();
                    let sender = sender.clone();
                    scope.spawn(move || {
                        for (model_idx, path) in chunk {
                            if sender
                                .send((model_idx, path.clone(), utils::parse_gguf_metadata(&path)))
                                .is_err()
                            {
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
                    let model = &mut models[model_idx];
                    model.architecture = summary.architecture;
                    model.context_length = summary.context_length;
                    model.quant_type = summary.quant_type;
                    model.has_mtp_head = summary.capabilities.has_builtin_mtp;
                    model.capabilities = summary.capabilities;
                }
            });
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
    let canonical = crate::security::require_authorized_model_path(p)?;
    let state_models = state.models.lock().unwrap();
    let is_known = state_models
        .iter()
        .filter_map(|model| std::fs::canonicalize(&model.path).ok())
        .any(|model_path| paths_equal(&model_path, &canonical));
    drop(state_models);
    if !is_known {
        return Err("文件不在已扫描的模型列表中".to_string());
    }
    let referenced_by = instances_referencing_model(&state.instances.lock().unwrap(), &canonical);
    if !referenced_by.is_empty() {
        return Err(format!(
            "模型文件正被实例引用，无法删除: {}",
            referenced_by.join(", ")
        ));
    }
    std::fs::remove_file(&canonical).map_err(|e| format!("删除文件失败: {}", e))?;
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
    let canonical = crate::security::require_authorized_model_path(Path::new(&path))?;
    tokio::task::spawn_blocking(move || utils::parse_gguf_metadata(&canonical))
        .await
        .map_err(|error| format!("GGUF metadata worker failed: {error}"))?
}

// Engine scanning.
pub async fn scan_engines(
    paths: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<EngineInfo>, String> {
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

        let engines_dir = app_dir.join("engines");
        if engines_dir.exists() {
            let scan_root_key = canonical_key(&engines_dir);
            scan_root_keys.insert(scan_root_key.clone());
            let inspection = inspect_engine_tree(&engines_dir, MAX_ENGINE_SCAN_DEPTH)?;
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
                            &mut engines,
                            &mut engine_records,
                        );
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
            let inspection = inspect_engine_tree(&root, MAX_ENGINE_SCAN_DEPTH)?;
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
                        &mut engines,
                        &mut engine_records,
                    );
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
        let initial = read_directory_fingerprint(&dir).unwrap();

        std::fs::write(dir.join("model.gguf"), b"test").unwrap();

        let updated = read_directory_fingerprint(&dir).unwrap();
        assert_ne!(initial.signature, updated.signature);

        let _ = std::fs::remove_dir_all(dir);
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
        let initial = inspect_engine_tree(&dir, MAX_ENGINE_SCAN_DEPTH)
            .unwrap()
            .signature;

        std::fs::write(nested.join(ENGINE_EXE_NAME), b"exe").unwrap();

        let updated = inspect_engine_tree(&dir, MAX_ENGINE_SCAN_DEPTH)
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

        let discovered = inspect_engine_tree(&dir, MAX_ENGINE_SCAN_DEPTH)
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

        assert!(inspect_engine_tree(&dir, MAX_ENGINE_SCAN_DEPTH)
            .unwrap()
            .executables
            .is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tree_signature_changes_when_a_deep_model_is_rewritten() {
        let dir = temp_test_dir("model-tree");
        let nested = dir.join("vendor").join("family").join("quant");
        std::fs::create_dir_all(&nested).unwrap();
        let model = nested.join("model.gguf");
        std::fs::write(&model, b"first-model-payload").unwrap();
        let initial = read_directory_tree_signature(&dir, MAX_MODEL_SCAN_DEPTH).unwrap();

        // Use a different size so the assertion stays valid on filesystems whose
        // modification timestamps are coarser than the test's execution time.
        std::fs::write(&model, b"other-longer-model-payload").unwrap();

        let updated = read_directory_tree_signature(&dir, MAX_MODEL_SCAN_DEPTH).unwrap();
        assert_ne!(initial, updated);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unchanged_failed_metadata_record_is_reused_without_reparsing() {
        let dir = temp_test_dir("malformed-model-cache");
        std::fs::write(dir.join("malformed.gguf"), b"not-a-gguf").unwrap();
        let scan_root_key = canonical_key(&dir);
        let mut models = Vec::new();
        let mut seen_display_paths = HashSet::new();
        let mut seen_inventory_paths = HashSet::new();
        let mut seen_directory_keys = HashSet::new();
        let mut inventory_meta = HashMap::new();
        let mut fresh_files = Vec::new();
        let mut directory_records = Vec::new();
        let mut errors = Vec::new();

        scan_model_directory_incremental(
            &dir,
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
        );
        assert_eq!(fresh_files.len(), 1);
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
        scan_model_directory_incremental(
            &dir,
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
        );

        assert_eq!(models.len(), 1);
        assert!(fresh_files.is_empty());
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

pub async fn open_engine_folder(dir: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("{}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&dir)
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
    ) -> crate::error::AppResult<()> {
        super::delete_model_file(path, state)
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
    pub async fn open_engine_folder(dir: String) -> crate::error::AppResult<()> {
        super::open_engine_folder(dir)
            .await
            .map_err(crate::error::AppError::from)
    }
}
