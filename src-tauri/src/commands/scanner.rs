use crate::commands::model_inventory::{self, InventoryEngineRecord, InventoryModelRecord};
use crate::models::{AppState, EngineInfo, ModelCapabilities, ModelInfo};
use crate::utils;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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
    for (_base, (expected_total, indices)) in &groups {
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
    })
}

// Model scanning.
#[tauri::command]
pub async fn scan_models(
    paths: Vec<String>,
    state: tauri::State<'_, AppState>,
    _app: tauri::AppHandle,
) -> Result<Vec<ModelInfo>, String> {
    let app_dir = utils::get_data_dir();
    let default_path = app_dir.join("models");
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

    let result = tokio::task::spawn_blocking(move || -> Result<Vec<ModelInfo>, Vec<String>> {
        let inventory = model_inventory::load_model_index().map_err(|err| vec![err])?;
        let mut models: Vec<ModelInfo> = Vec::new();
        let mut seen_display_paths = HashSet::new();
        let mut seen_inventory_paths = HashSet::new();
        let mut scan_root_keys = HashSet::new();
        let mut inventory_meta: HashMap<usize, (String, String, u64)> = HashMap::new();
        let mut errors = Vec::new();
        let mut fresh_files: Vec<(usize, PathBuf)> = Vec::new();

        for scan_root in &scan_paths {
            let root_str = scan_root.display().to_string();
            if !scan_root.exists() {
                if *scan_root == default_path_for_check {
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
            let mut file_count = 0;

            for entry in walkdir::WalkDir::new(scan_root)
                .max_depth(5)
                .into_iter()
                .flatten()
            {
                let path = entry.path();
                if entry.file_type().is_symlink() {
                    errors.push(format!("{} is a symlink and was skipped", path.display()));
                    continue;
                }
                if !path.is_file() {
                    continue;
                }
                let ext = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if ext != "gguf" {
                    continue;
                }
                let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if fname.starts_with('.') {
                    continue;
                }

                let name = fname.to_string();
                let mtime = file_mtime(path);
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let file_type = utils::classify_gguf_file(path).to_string();
                let file_path = path.to_string_lossy().to_string();
                if !seen_display_paths.insert(file_path.clone()) {
                    continue;
                }

                let cache_key = canonical_key(path);
                seen_inventory_paths.insert(cache_key.clone());
                file_count += 1;

                if let Some(record) = inventory.get(&cache_key) {
                    if record.mtime == mtime && record.size == size {
                        let idx = models.len();
                        let mut model = record.to_model_info();
                        model.name = name;
                        model.path = file_path;
                        model.size = size;
                        model.is_shard = false;
                        models.push(model);
                        inventory_meta.insert(idx, (cache_key, scan_root_key.clone(), mtime));
                        continue;
                    }
                }

                let idx = models.len();
                models.push(ModelInfo {
                    id: uuid::Uuid::new_v4().to_string(),
                    name,
                    path: file_path,
                    size,
                    architecture: None,
                    context_length: None,
                    quant_type: None,
                    has_mtp_head: false,
                    capabilities: ModelCapabilities::default(),
                    file_type,
                    is_shard: false,
                });
                inventory_meta.insert(idx, (cache_key, scan_root_key.clone(), mtime));
                fresh_files.push((idx, path.to_path_buf()));
            }

            if file_count == 0 {
                errors.push(format!("{} contains no .gguf model files", root_str));
            }
        }

        if !fresh_files.is_empty() {
            let chunk_size = (fresh_files.len()
                / std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4)
                    .max(1))
            .max(1);
            type MetadataParseResult = (
                usize,
                PathBuf,
                Result<crate::models::GgufMetadataSummary, String>,
            );
            let results: Vec<Vec<MetadataParseResult>> = std::thread::scope(|s| {
                let mut handles = Vec::new();
                for chunk in fresh_files.chunks(chunk_size) {
                    let chunk: Vec<_> = chunk.to_vec();
                    handles.push(s.spawn(move || {
                        chunk
                            .iter()
                            .map(|(model_idx, path)| {
                                (*model_idx, path.clone(), utils::parse_gguf_metadata(path))
                            })
                            .collect::<Vec<_>>()
                    }));
                }
                handles
                    .into_iter()
                    .map(|h| h.join().unwrap_or_default())
                    .collect()
            });

            for chunk_results in results {
                for (model_idx, path, summary_result) in chunk_results {
                    if model_idx < models.len() {
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
                }
            }
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
        model_inventory::upsert_model_records(&records).map_err(|err| vec![err])?;
        model_inventory::prune_absent_models(&scan_root_keys, &seen_inventory_paths)
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

    let mut state_models = state.models.lock().unwrap();
    *state_models = models.clone();
    Ok(models)
}

// Batch load: scan and restore downloads in one IPC call.
#[tauri::command]
pub async fn load_app_data(
    paths: Vec<String>,
    engine_paths: Vec<String>,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<
    (
        Vec<ModelInfo>,
        Vec<EngineInfo>,
        Vec<crate::models::PersistedQueueEntry>,
    ),
    String,
> {
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
#[tauri::command]
pub async fn get_cached_scan(
    state: tauri::State<'_, AppState>,
) -> Result<Option<(Vec<ModelInfo>, Vec<EngineInfo>)>, String> {
    let mut models = model_inventory::list_cached_models()?;
    mark_sharded_models(&mut models);

    let mut engines = model_inventory::list_cached_engines()?;
    {
        let saved_names = state.engine_names.lock().unwrap();
        for engine in &mut engines {
            if let Some(cn) = saved_names.get(&engine.id) {
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

#[tauri::command]
pub async fn get_models(state: tauri::State<'_, AppState>) -> Result<Vec<ModelInfo>, String> {
    Ok(state.models.lock().unwrap().clone())
}

#[tauri::command]
pub async fn delete_model_file(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if std::fs::symlink_metadata(p)
        .map_err(|e| format!("璺緞鏃犳晥: {}", e))?
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
    let canonical = std::fs::canonicalize(p).map_err(|e| format!("路径无效: {}", e))?;
    let state_models = state.models.lock().unwrap();
    let is_known = state_models
        .iter()
        .any(|m| std::fs::canonicalize(&m.path).ok().as_ref() == Some(&canonical));
    drop(state_models);
    if !is_known {
        return Err("文件不在已扫描的模型列表中".to_string());
    }
    std::fs::remove_file(&canonical).map_err(|e| format!("删除文件失败: {}", e))?;
    let _ = model_inventory::delete_model(&canonical.to_string_lossy());
    let mut models = state.models.lock().unwrap();
    models.retain(|m| std::fs::canonicalize(&m.path).ok().as_ref() != Some(&canonical));
    Ok(())
}

#[tauri::command]
pub async fn open_model_folder(path: String) -> Result<(), String> {
    let parent = Path::new(&path).parent().unwrap_or(Path::new("."));
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

#[tauri::command]
pub async fn read_gguf_metadata(
    path: String,
) -> Result<crate::models::GgufMetadataSummary, String> {
    utils::parse_gguf_metadata(Path::new(&path))
}

// Engine scanning.
#[tauri::command]
pub async fn scan_engines(
    paths: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<EngineInfo>, String> {
    let mut engines = tokio::task::spawn_blocking(move || -> Result<Vec<EngineInfo>, String> {
        let inventory = model_inventory::load_engine_index()?;
        let mut engines: Vec<EngineInfo> = Vec::new();
        let mut engine_records: Vec<InventoryEngineRecord> = Vec::new();
        let mut seen = HashSet::new();
        let mut seen_inventory_ids = HashSet::new();
        let mut scan_root_keys = HashSet::new();
        let app_dir = utils::get_data_dir();

        let mut try_indexed_engine = |dir: &Path, exe: &Path, scan_root_key: &str| {
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
        };

        let engines_dir = app_dir.join("engines");
        if engines_dir.exists() {
            let scan_root_key = canonical_key(&engines_dir);
            scan_root_keys.insert(scan_root_key.clone());
            for entry in std::fs::read_dir(&engines_dir)
                .map_err(|e| format!("{}", e))?
                .flatten()
            {
                let dir = entry.path();
                if !dir.is_dir() {
                    continue;
                }
                let exe = dir.join(ENGINE_EXE_NAME);
                if exe.exists() {
                    let norm = dir.to_string_lossy().to_lowercase();
                    if seen.insert(norm) {
                        try_indexed_engine(&dir, &exe, &scan_root_key);
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
            let direct_exe = root.join(ENGINE_EXE_NAME);
            if direct_exe.exists() {
                let norm = root.to_string_lossy().to_lowercase();
                if seen.insert(norm) {
                    try_indexed_engine(&root, &direct_exe, &scan_root_key);
                }
            }
            if let Ok(entries) = std::fs::read_dir(&root) {
                for entry in entries.flatten() {
                    let sub = entry.path();
                    if !sub.is_dir() {
                        continue;
                    }
                    let exe = sub.join(ENGINE_EXE_NAME);
                    if exe.exists() {
                        let norm = sub.to_string_lossy().to_lowercase();
                        if seen.insert(norm) {
                            try_indexed_engine(&sub, &exe, &scan_root_key);
                        }
                    }
                    if let Ok(sub_entries) = std::fs::read_dir(&sub) {
                        for se in sub_entries.flatten() {
                            let sub2 = se.path();
                            if !sub2.is_dir() {
                                continue;
                            }
                            let exe2 = sub2.join(ENGINE_EXE_NAME);
                            if exe2.exists() {
                                let norm = sub2.to_string_lossy().to_lowercase();
                                if seen.insert(norm) {
                                    try_indexed_engine(&sub2, &exe2, &scan_root_key);
                                }
                            }
                        }
                    }
                }
            }
        }

        model_inventory::upsert_engine_records(&engine_records)?;
        model_inventory::prune_absent_engines(&scan_root_keys, &seen_inventory_ids)?;
        Ok(engines)
    })
    .await
    .map_err(|e| format!("scan thread failed: {}", e))??;

    // Preserve custom engine names; state access stays outside spawn_blocking.
    {
        let saved_names = state.engine_names.lock().unwrap();
        for engine in &mut engines {
            if let Some(cn) = saved_names.get(&engine.id) {
                engine.custom_name = Some(cn.clone());
                engine.name = cn.clone();
            }
        }
    }

    let mut state_engines = state.engines.lock().unwrap();
    *state_engines = engines.clone();
    Ok(engines)
}

#[tauri::command]
pub async fn get_engines(state: tauri::State<'_, AppState>) -> Result<Vec<EngineInfo>, String> {
    Ok(state.engines.lock().unwrap().clone())
}

#[tauri::command]
pub async fn delete_engine(id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut engines = state.engines.lock().unwrap();
    engines.retain(|e| e.id != id);
    let _ = model_inventory::delete_engine(&id);
    Ok(())
}

#[tauri::command]
pub async fn rename_engine(
    id: String,
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut engines = state.engines.lock().unwrap();
    if let Some(engine) = engines.iter_mut().find(|e| e.id == id) {
        engine.custom_name = Some(name.clone());
        engine.name = name.clone();
    }
    state.engine_names.lock().unwrap().insert(id, name);
    // Persist engine names immediately using unified atomic writes to avoid race conditions.
    crate::commands::config::update_and_persist(&state, |global| {
        global.engine_names = state.engine_names.lock().unwrap().clone();
    })?;
    Ok(())
}

#[tauri::command]
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
