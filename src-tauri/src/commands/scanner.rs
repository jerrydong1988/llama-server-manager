use std::collections::HashMap;
use std::path::{Path, PathBuf};
use crate::models::{AppState, EngineInfo, ModelInfo};
use crate::utils;

// ── 扫描缓存 ──────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CachedModel {
    mtime: u64,
    size: u64,
    architecture: Option<String>,
    context_length: Option<u32>,
    quant_type: Option<String>,
    has_mtp_head: bool,
    file_type: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CachedEngine {
    exe_mtime: u64,
    name: String,
    version: String,
    backend: String,
    dir: String,
    exe: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ScanCache {
    models: HashMap<String, (String, CachedModel)>,     // canonical_path -> (id, cached)
    engines: HashMap<String, CachedEngine>,              // canonical_dir -> cached
}

fn cache_path() -> PathBuf {
    utils::get_data_dir().join("scan_cache.json")
}

fn load_scan_cache() -> ScanCache {
    let p = cache_path();
    if p.exists() {
        if let Ok(data) = std::fs::read_to_string(&p) {
            if let Ok(c) = serde_json::from_str::<ScanCache>(&data) {
                return c;
            }
        }
    }
    ScanCache { models: HashMap::new(), engines: HashMap::new() }
}

fn save_scan_cache(cache: &ScanCache) {
    let p = cache_path();
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(cache) {
        let _ = std::fs::write(&p, json);
    }
}

fn file_mtime(path: &Path) -> u64 {
    path.metadata().and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── 跨平台可执行文件名 ────────────────────────────────────────────

// ── 跨平台可执行文件名 ────────────────────────────────────────────
#[cfg(target_os = "windows")]
const ENGINE_EXE_NAME: &str = "llama-server.exe";
#[cfg(not(target_os = "windows"))]
const ENGINE_EXE_NAME: &str = "llama-server";

// ── 引擎信息构建 ──────────────────────────────────────────────────
pub fn build_engine_info(dir: &Path, exe: &Path, _source: &str) -> Option<EngineInfo> {
    let name = dir.file_name().and_then(|s| s.to_str()).unwrap_or("llama-server").to_string();
    let version = name.clone();
    let backend = utils::detect_backend(dir);
    // #10: 使用目录规范化路径作为稳定 ID，避免移动目录导致引用断裂
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

// ── 模型扫描 ──────────────────────────────────────────────────────
#[tauri::command]
pub async fn scan_models(paths: Vec<String>, state: tauri::State<'_, AppState>, _app: tauri::AppHandle) -> Result<Vec<ModelInfo>, String> {
    let app_dir = utils::get_data_dir();
    let default_path = app_dir.join("models");
    let default_path_for_check = default_path.clone();

    let scan_paths: Vec<PathBuf> = if paths.is_empty() {
        vec![default_path]
    } else {
        paths.iter().map(|p| {
            let pb = PathBuf::from(p);
            if pb.is_relative() { app_dir.join(p) } else { pb }
        }).collect()
    };

    // Offload heavy disk I/O + GGUF parsing to a blocking thread
    let result = tokio::task::spawn_blocking(move || -> Result<(Vec<ModelInfo>, ScanCache), Vec<String>> {
        let mut cache = load_scan_cache();
        let mut models: Vec<ModelInfo> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut errors = Vec::new();
        // 收集需要新鲜解析的文件路径（mtime 变更或缓存未命中）
        let mut fresh_files: Vec<(PathBuf, String, u64)> = Vec::new(); // (path, canonical_key, size)

        for scan_root in &scan_paths {
            let root_str = scan_root.display().to_string();
            if !scan_root.exists() {
                if *scan_root == default_path_for_check { continue; }
                errors.push(format!("{} 不存在", root_str)); continue;
            }
            if !scan_root.is_dir() { errors.push(format!("{} 不是目录", root_str)); continue; }

            let mut file_count = 0;
            for entry in walkdir::WalkDir::new(scan_root).max_depth(5).into_iter().flatten() {
                let path = entry.path();
                if !path.is_file() { continue; }
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                if ext != "gguf" { continue; }
                let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if fname.starts_with('.') { continue; }

                let name = fname.to_string();
                let mtime = file_mtime(path);
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let ftype = utils::classify_gguf_file(path);
                let file_path = path.to_string_lossy().to_string();
                if !seen.insert(file_path.clone()) { continue; }

                let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
                let cache_key = canonical.to_string_lossy().to_string();

                // mtime 匹配 → 缓存命中，跳过 GGUF 解析
                if let Some((cached_id, cm)) = cache.models.get(&cache_key) {
                    if cm.mtime == mtime && cm.size == size {
                        file_count += 1;
                        models.push(ModelInfo {
                            id: cached_id.clone(),
                            name, path: file_path, size,
                            architecture: cm.architecture.clone(),
                            context_length: cm.context_length,
                            quant_type: cm.quant_type.clone(),
                            has_mtp_head: cm.has_mtp_head,
                            file_type: cm.file_type.clone(),
                            is_shard: false,
                        });
                        continue;
                    }
                }

                file_count += 1;
                fresh_files.push((path.to_path_buf(), cache_key, size));
                // 占位先入 models，之后用解析结果回填
                let _idx = models.len();
                models.push(ModelInfo {
                    id: uuid::Uuid::new_v4().to_string(),
                    name, path: file_path, size,
                    architecture: None, context_length: None, quant_type: None, has_mtp_head: false,
                    file_type: ftype.to_string(),
                    is_shard: false,
                });
            }
            if file_count == 0 { errors.push(format!("{} 中未找到 .gguf 模型文件", root_str)); }
        }

        // ── 并行 GGUF 解析：对需要新鲜解析的文件分线程批量处理 ──
        if !fresh_files.is_empty() {
            let chunk_size = (fresh_files.len() / std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).max(1)).max(1);
            let results: Vec<Vec<(usize, Option<String>, Option<u32>, Option<String>, bool)>> =
                std::thread::scope(|s| {
                    let mut handles = Vec::new();
                    for chunk in fresh_files.chunks(chunk_size) {
                        let chunk: Vec<_> = chunk.to_vec();
                        handles.push(s.spawn(move || {
                            chunk.iter().enumerate().map(|(i, (p, _key, _sz))| {
                                let (arch, ctx, qt, mtp) =
                                    utils::parse_gguf_metadata(p).unwrap_or_else(|_| (None, None, None, false));
                                (i, arch, ctx, qt, mtp)
                            }).collect::<Vec<_>>()
                        }));
                    }
                    handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
                });

            // 回填解析结果 + 更新缓存
            let base_idx = models.len() - fresh_files.len();
            for (chunk_idx, chunk_results) in results.iter().enumerate() {
                for (i, arch, ctx, qt, mtp) in chunk_results {
                    let global_i = base_idx + chunk_idx * chunk_size + i;
                    if global_i < models.len() {
                        let m = &mut models[global_i];
                        m.architecture = arch.clone();
                        m.context_length = *ctx;
                        m.quant_type = qt.clone();
                        m.has_mtp_head = *mtp;
                    }
                    if chunk_idx * chunk_size + i < fresh_files.len() {
                        let (_, cache_key, size) = &fresh_files[chunk_idx * chunk_size + i];
                        let mpath = std::path::Path::new(cache_key);
                        let mtime = file_mtime(mpath);
                        if global_i < models.len() {
                            let cached_id = models[global_i].id.clone();
                            cache.models.insert(cache_key.clone(), (cached_id, CachedModel {
                                mtime, size: *size,
                                architecture: arch.clone(),
                                context_length: *ctx,
                                quant_type: qt.clone(),
                                has_mtp_head: *mtp,
                                file_type: models[global_i].file_type.clone(),
                            }));
                        }
                    }
                }
            }
        }

        save_scan_cache(&cache);

        if models.is_empty() && !errors.is_empty() {
            Err(errors)
        } else {
            Ok((models, cache))
        }
    }).await.map_err(|e| format!("扫描线程失败: {:?}", e))?;

    let mut models = match result {
        Ok((m, _cache)) => m,
        Err(errors) => return Err(errors.join("; ")),
    };

    // ── 分片模型检测：按基础名分组，验证文件数是否等于声明的 N ──
    {
        use regex_lite::Regex;
        // [0-9] for regex_lite compat; .+? (lazy) avoids backtracking issues with names containing hyphens
        let shard_re = Regex::new(r"(?i)^(.+?)-([0-9]{5})-of-([0-9]{5})\.gguf$").unwrap();
        let mut groups: std::collections::HashMap<String, (u32, Vec<usize>)> = std::collections::HashMap::new();
        let mut _matched_count = 0u32;
        for (i, m) in models.iter().enumerate() {
            if let Some(caps) = shard_re.captures(&m.name) {
                let base = caps.get(1).unwrap().as_str().to_string();
                let total: u32 = caps.get(3).unwrap().as_str().parse().unwrap_or(0);
                groups.entry(base).or_insert_with(|| (total, Vec::new())).1.push(i);
                _matched_count += 1;
            }
        }
        let mut _marked_count = 0u32;
        for (_base, (expected_total, indices)) in &groups {
            if indices.len() as u32 == *expected_total && *expected_total > 1 {
                for &idx in indices { models[idx].is_shard = true; _marked_count += 1; }
            }
        }

    }

    let mut state_models = state.models.lock().unwrap();
    *state_models = models.clone();
    Ok(models)
}

// ── 批量加载 — 一次 IPC 完成扫描+下载恢复 ──
#[tauri::command]
pub async fn load_app_data(
    paths: Vec<String>,
    engine_paths: Vec<String>,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(Vec<ModelInfo>, Vec<EngineInfo>, Vec<crate::models::PersistedQueueEntry>), String> {
    let (models_result, engines_result) = tokio::join!(
        scan_models(paths, state.clone(), app.clone()),
        scan_engines(engine_paths, state.clone())
    );
    let models = models_result.unwrap_or_else(|_| Vec::new());
    let engines = engines_result.unwrap_or_else(|_| Vec::new());
    let queue = crate::commands::download::load_download_state(&state);
    Ok((models, engines, queue))
}

/// 从磁盘缓存读取扫描结果，用于启动时立即展示数据（无需等待全量扫描）
#[tauri::command]
pub async fn get_cached_scan(
    state: tauri::State<'_, AppState>,
) -> Result<Option<(Vec<ModelInfo>, Vec<EngineInfo>)>, String> {
    let cache = load_scan_cache();
    if cache.models.is_empty() && cache.engines.is_empty() {
        return Ok(None);
    }

    let mut models: Vec<ModelInfo> = Vec::new();
    for (path, (id, cm)) in &cache.models {
        models.push(ModelInfo {
            id: id.clone(),
            name: std::path::Path::new(path).file_name()
                .and_then(|s| s.to_str()).unwrap_or("").to_string(),
            path: path.clone(),
            size: cm.size,
            architecture: cm.architecture.clone(),
            context_length: cm.context_length,
            quant_type: cm.quant_type.clone(),
            has_mtp_head: cm.has_mtp_head,
            file_type: cm.file_type.clone(),
            is_shard: false,
        });
    }

    let mut engines: Vec<EngineInfo> = Vec::new();
    for (dir, ce) in &cache.engines {
        let saved_names = state.engine_names.lock().unwrap();
        let mut info = EngineInfo {
            id: dir.clone(),
            name: ce.name.clone(),
            dir: ce.dir.clone(),
            exe: ce.exe.clone(),
            version: ce.version.clone(),
            backend: ce.backend.clone(),
            custom_name: None,
        };
        if let Some(cn) = saved_names.get(dir) {
            info.custom_name = Some(cn.clone());
            info.name = cn.clone();
        }
        engines.push(info);
    }

    // 写入 state 以便其他组件立即可用
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
pub async fn delete_model_file(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if p.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase()) != Some("gguf".to_string()) {
        return Err("只能删除 .gguf 文件".to_string());
    }
    let canonical = std::fs::canonicalize(p).map_err(|e| format!("路径无效: {}", e))?;
    let state_models = state.models.lock().unwrap();
    let is_known = state_models.iter().any(|m| {
        std::fs::canonicalize(&m.path).ok().as_ref() == Some(&canonical)
    });
    drop(state_models);
    if !is_known {
        return Err("文件不在已扫描的模型列表中".to_string());
    }
    std::fs::remove_file(&canonical).map_err(|e| format!("删除文件失败: {}", e))?;
    let mut models = state.models.lock().unwrap();
    models.retain(|m| {
        std::fs::canonicalize(&m.path).ok().as_ref() != Some(&canonical)
    });
    Ok(())
}

#[tauri::command]
pub async fn open_model_folder(path: String) -> Result<(), String> {
    let parent = Path::new(&path).parent().unwrap_or(Path::new("."));
    #[cfg(target_os = "windows")]
    { std::process::Command::new("explorer").arg(parent).spawn().map_err(|e| format!("{}", e))?; }
    #[cfg(target_os = "macos")]
    { std::process::Command::new("open").arg(parent).spawn().map_err(|e| format!("{}", e))?; }
    #[cfg(target_os = "linux")]
    { std::process::Command::new("xdg-open").arg(parent).spawn().map_err(|e| format!("{}", e))?; }
    Ok(())
}

#[tauri::command]
pub async fn read_gguf_metadata(path: String) -> Result<(Option<String>, Option<u32>, Option<String>, bool), String> {
    utils::parse_gguf_metadata(Path::new(&path))
}

// ── 引擎扫描 ──────────────────────────────────────────────────────
#[tauri::command]
pub async fn scan_engines(paths: Vec<String>, state: tauri::State<'_, AppState>) -> Result<Vec<EngineInfo>, String> {
    let mut engines = tokio::task::spawn_blocking(move || -> Result<Vec<EngineInfo>, String> {
    let mut cache = load_scan_cache();
    let mut engines: Vec<EngineInfo> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let app_dir = utils::get_data_dir();

    // 辅助：尝试从缓存还原引擎信息
    let mut try_cache = |dir: &Path, exe: &Path, source: &str| {
        let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        let cache_key = canonical.to_string_lossy().to_string();
        let exe_mtime = file_mtime(exe);

        if let Some(ce) = cache.engines.get(&cache_key) {
            if ce.exe_mtime == exe_mtime {
                let info = EngineInfo {
                    id: cache_key.clone(),
                    name: ce.name.clone(),
                    dir: ce.dir.clone(),
                    exe: ce.exe.clone(),
                    version: ce.version.clone(),
                    backend: ce.backend.clone(),
                    custom_name: None,
                };
                engines.push(info);
                return;
            }
        }

        if let Some(info) = build_engine_info(dir, exe, source) {
            let canonical2 = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
            let ck = canonical2.to_string_lossy().to_string();
            cache.engines.insert(ck, CachedEngine {
                exe_mtime,
                name: info.name.clone(),
                version: info.version.clone(),
                backend: info.backend.clone(),
                dir: info.dir.clone(),
                exe: info.exe.clone(),
            });
            engines.push(info);
        }
    };

    let engines_dir = app_dir.join("engines");
    if engines_dir.exists() {
        for entry in std::fs::read_dir(&engines_dir).map_err(|e| format!("{}", e))?.flatten() {
            let dir = entry.path();
            if !dir.is_dir() { continue; }
            let exe = dir.join(ENGINE_EXE_NAME);
            if exe.exists() {
                let norm = dir.to_string_lossy().to_lowercase();
                if seen.insert(norm) {
                    try_cache(&dir, &exe, "本地");
                }
            }
        }
    }

    for p in &paths {
        let root = PathBuf::from(p);
        if !root.exists() || !root.is_dir() { continue; }
        let direct_exe = root.join(ENGINE_EXE_NAME);
        if direct_exe.exists() {
            let norm = root.to_string_lossy().to_lowercase();
            if seen.insert(norm) {
                try_cache(&root, &direct_exe, "自定义");
            }
        }
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let sub = entry.path();
                if !sub.is_dir() { continue; }
                let exe = sub.join(ENGINE_EXE_NAME);
                if exe.exists() {
                    let norm = sub.to_string_lossy().to_lowercase();
                    if seen.insert(norm) {
                        try_cache(&sub, &exe, "自定义");
                    }
                }
                if let Ok(sub_entries) = std::fs::read_dir(&sub) {
                    for se in sub_entries.flatten() {
                        let sub2 = se.path();
                        if !sub2.is_dir() { continue; }
                        let exe2 = sub2.join(ENGINE_EXE_NAME);
                        if exe2.exists() {
                            let norm = sub2.to_string_lossy().to_lowercase();
                            if seen.insert(norm) {
                                try_cache(&sub2, &exe2, "自定义");
                            }
                        }
                    }
                }
            }
        }
    }

    save_scan_cache(&cache);
    Ok(engines)
    }).await.map_err(|e| format!("扫描线程失败: {}", e))??;

    // 保留已改名的引擎自定义名称（state 访问在 spawn_blocking 外部）
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
    Ok(())
}

#[tauri::command]
pub async fn rename_engine(id: String, name: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut engines = state.engines.lock().unwrap();
    if let Some(engine) = engines.iter_mut().find(|e| e.id == id) {
        engine.custom_name = Some(name.clone());
        engine.name = name.clone();
    }
    state.engine_names.lock().unwrap().insert(id, name);
    // Persist engine names immediately — use unified atomic write to avoid race conditions
    crate::commands::config::update_and_persist(&state, |global| {
        global.engine_names = state.engine_names.lock().unwrap().clone();
    })?;
    Ok(())
}

#[tauri::command]
pub async fn open_engine_folder(dir: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    { std::process::Command::new("explorer").arg(&dir).spawn().map_err(|e| format!("{}", e))?; }
    #[cfg(target_os = "macos")]
    { std::process::Command::new("open").arg(&dir).spawn().map_err(|e| format!("{}", e))?; }
    #[cfg(target_os = "linux")]
    { std::process::Command::new("xdg-open").arg(&dir).spawn().map_err(|e| format!("{}", e))?; }
    Ok(())
}
