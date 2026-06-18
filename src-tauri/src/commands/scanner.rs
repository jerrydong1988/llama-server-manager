use std::path::{Path, PathBuf};
use crate::models::{AppState, EngineInfo, ModelInfo};
use crate::utils;

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
pub async fn scan_models(paths: Vec<String>, state: tauri::State<'_, AppState>) -> Result<Vec<ModelInfo>, String> {
    let app_dir = utils::get_data_dir();
    let default_path = app_dir.join("models");

    let scan_paths: Vec<PathBuf> = if paths.is_empty() {
        vec![default_path]
    } else {
        paths.iter().map(|p| {
            let pb = PathBuf::from(p);
            if pb.is_relative() { app_dir.join(p) } else { pb }
        }).collect()
    };

    // Offload heavy disk I/O + GGUF parsing to a blocking thread
    let models = tokio::task::spawn_blocking(move || -> Result<Vec<ModelInfo>, Vec<String>> {
        let mut models: Vec<ModelInfo> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut errors = Vec::new();

        for scan_root in &scan_paths {
            let root_str = scan_root.display().to_string();
            if !scan_root.exists() { errors.push(format!("{} 不存在", root_str)); continue; }
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
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let ftype = utils::classify_gguf_file(path);
                let file_path = path.to_string_lossy().to_string();
                if !seen.insert(file_path.clone()) { continue; }

                let (architecture, context_length, quant_type, has_mtp_head) =
                    utils::parse_gguf_metadata(path).unwrap_or_else(|_| (None, None, None, false));

                file_count += 1;
                models.push(ModelInfo {
                    id: uuid::Uuid::new_v4().to_string(),
                    name, path: file_path, size, architecture, context_length, quant_type, has_mtp_head,
                    file_type: ftype.to_string(),
                    is_shard: false,
                });
            }
            if file_count == 0 { errors.push(format!("{} 中未找到 .gguf 模型文件", root_str)); }
        }

        if models.is_empty() && !errors.is_empty() {
            Err(errors)
        } else {
            Ok(models)
        }
    }).await.map_err(|e| format!("扫描线程失败: {}", e))?;

    let mut models = match models {
        Ok(m) => m,
        Err(errors) => return Err(errors.join("; ")),
    };

    // ── 分片模型检测：按基础名分组，验证文件数是否等于声明的 N ──
    {
        use regex_lite::Regex;
        let shard_re = Regex::new(r"(?i)^(.+)-(\d{5})-of-(\d{5})\.gguf$").unwrap();
        let mut groups: std::collections::HashMap<String, (u32, Vec<usize>)> = std::collections::HashMap::new();
        for (i, m) in models.iter().enumerate() {
            if let Some(caps) = shard_re.captures(&m.name) {
                let base = caps.get(1).unwrap().as_str().to_string();
                let total: u32 = caps.get(3).unwrap().as_str().parse().unwrap_or(0);
                groups.entry(base).or_insert_with(|| (total, Vec::new())).1.push(i);
            }
        }
        for (expected_total, indices) in groups.values() {
            if indices.len() as u32 == *expected_total && *expected_total > 1 {
                for &idx in indices { models[idx].is_shard = true; }
            }
        }
    }

    let mut state_models = state.models.lock().unwrap();
    *state_models = models.clone();
    Ok(models)
}

#[tauri::command]
pub async fn get_models(state: tauri::State<'_, AppState>) -> Result<Vec<ModelInfo>, String> {
    Ok(state.models.lock().unwrap().clone())
}

#[tauri::command]
pub async fn delete_model_file(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    std::fs::remove_file(&path).map_err(|e| format!("删除文件失败: {}", e))?;
    let mut models = state.models.lock().unwrap();
    models.retain(|m| m.path != path);
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
    // #9: 卸载到阻塞线程，避免阻塞 async runtime
    let mut engines = tokio::task::spawn_blocking(move || -> Result<Vec<EngineInfo>, String> {
    let mut engines: Vec<EngineInfo> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let app_dir = utils::get_data_dir();

    let engines_dir = app_dir.join("engines");
    if engines_dir.exists() {
        for entry in std::fs::read_dir(&engines_dir).map_err(|e| format!("{}", e))?.flatten() {
            let dir = entry.path();
            if !dir.is_dir() { continue; }
            let exe = dir.join(ENGINE_EXE_NAME);
            if exe.exists() {
                let norm = dir.to_string_lossy().to_lowercase();
                if seen.insert(norm) {
                        if let Some(info) = build_engine_info(&dir, &exe, "本地") {
                        engines.push(info);
                    }
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
                if let Some(info) = build_engine_info(&root, &direct_exe, "自定义") {
                    engines.push(info);
                }
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
                        if let Some(info) = build_engine_info(&sub, &exe, "自定义") {
                            engines.push(info);
                        }
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
                                if let Some(info) = build_engine_info(&sub2, &exe2, "自定义") {
                                    engines.push(info);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

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
    // Persist engine names immediately to survive crashes
    let config_dir = state.config_dir.lock().unwrap().clone();
    let path = config_dir.join("instances.json");
    if let Ok(json) = std::fs::read_to_string(&path) {
        if let Ok(mut global) = serde_json::from_str::<crate::models::GlobalConfig>(&json) {
            global.engine_names = state.engine_names.lock().unwrap().clone();
            let _ = std::fs::write(&path, serde_json::to_string_pretty(&global).unwrap_or_default());
        }
    }
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
