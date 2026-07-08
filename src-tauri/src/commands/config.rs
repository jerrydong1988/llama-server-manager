use crate::models::{AppState, GlobalConfig, InstanceConfig, ProxyConfig, WindowState};
use std::collections::HashMap;
use tauri::Emitter;

// ── 统一配置写入工具 ────────────────────────────────────────────

/// 原子写入 instances.json — 所有配置写入操作都应走此函数以避免竞态
pub fn persist_global_config(
    config_dir: &std::path::Path,
    global: &GlobalConfig,
) -> Result<(), String> {
    let path = config_dir.join("instances.json");
    let json = serde_json::to_string_pretty(global).map_err(|e| format!("序列化失败: {}", e))?;
    let tmp = config_dir.join("instances.json.tmp");
    std::fs::write(&tmp, &json).map_err(|e| format!("保存失败: {}", e))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("保存失败: {}", e))?;
    let _ = std::fs::copy(&path, config_dir.join("instances.json.bak"));
    Ok(())
}

/// 读取现有配置，修改后原子写入 — 用于非 save_config 路径的写入
pub fn update_and_persist<F>(state: &AppState, update_fn: F) -> Result<(), String>
where
    F: FnOnce(&mut GlobalConfig),
{
    let config_dir = state.config_dir.lock().unwrap().clone();
    let path = config_dir.join("instances.json");
    let json = std::fs::read_to_string(&path).map_err(|e| format!("读取配置失败: {}", e))?;
    let mut global: GlobalConfig =
        serde_json::from_str(&json).map_err(|e| format!("解析配置失败: {}", e))?;
    update_fn(&mut global);
    persist_global_config(&config_dir, &global)
}

/// 轻量级进程存活检查 — 使用 Windows OpenProcess / Unix kill(0) 避免 sysinfo 全量刷新
fn is_process_alive(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            CloseHandle(handle);
            true
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        // kill(pid, 0) returns 0 if process exists, -1 with ESRCH if not
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
}

// ── 配置持久化 ────────────────────────────────────────────────────

/// 从磁盘读取配置并 resolve 路径 — 不依赖 AppState，供 main.rs setup() 提前调用
pub fn read_config_from_disk(config_dir: &std::path::Path) -> GlobalConfig {
    let path = config_dir.join("instances.json");

    let load_json = || -> Result<String, String> {
        let primary_result = std::fs::read_to_string(&path);
        if let Ok(json) = primary_result {
            return Ok(json);
        }
        // 主文件读取失败（可能被锁定），尝试 .bak 回退
        let bak = config_dir.join("instances.json.bak");
        if bak.exists() {
            let json = std::fs::read_to_string(&bak).map_err(|e| format!("{}", e))?;
            return Ok(json);
        }
        // 如果主文件存在但读不了且 .bak 也不存在，返回主文件的错误
        if path.exists() {
            primary_result.map_err(|e| format!("{}", e))
        } else {
            Err("no config".into())
        }
    };

    let json = match load_json() {
        Ok(j) => j,
        Err(_) => {
            return GlobalConfig {
                instances: HashMap::new(),
                model_dirs: vec![],
                engine_dirs: vec![],
                default_engine_id: String::new(),
                running: HashMap::new(),
                instance_order: vec![],
                last_tab: "model-repo".into(),
                dark_mode: true,
                engine_names: HashMap::new(),
                download_resume_policy: "manual".into(),
                download_max_concurrent: 1,
                download_bandwidth_limit_bytes_per_sec: 0,
                download_low_priority_throttle: false,
                proxy_config: ProxyConfig::default(),
            }
        }
    };

    let mut global: GlobalConfig = match serde_json::from_str(&json) {
        Ok(g) => g,
        Err(_) => {
            return GlobalConfig {
                instances: HashMap::new(),
                model_dirs: vec![],
                engine_dirs: vec![],
                default_engine_id: String::new(),
                running: HashMap::new(),
                instance_order: vec![],
                last_tab: "model-repo".into(),
                dark_mode: true,
                engine_names: HashMap::new(),
                download_resume_policy: "manual".into(),
                download_max_concurrent: 1,
                download_bandwidth_limit_bytes_per_sec: 0,
                download_low_priority_throttle: false,
                proxy_config: ProxyConfig::default(),
            }
        }
    };

    // resolve 相对路径
    let app_dir = crate::utils::get_data_dir();
    global.model_dirs = global
        .model_dirs
        .iter()
        .map(|d| {
            let pb = std::path::PathBuf::from(d);
            if pb.is_relative() {
                app_dir.join(d).to_string_lossy().to_string()
            } else {
                d.clone()
            }
        })
        .collect();
    global.engine_dirs = global
        .engine_dirs
        .iter()
        .map(|d| {
            let pb = std::path::PathBuf::from(d);
            if pb.is_relative() {
                app_dir.join(d).to_string_lossy().to_string()
            } else {
                d.clone()
            }
        })
        .collect();

    // 过滤已死的进程
    let mut restored = HashMap::new();
    for (id, ri) in &global.running {
        if is_process_alive(ri.pid) {
            restored.insert(id.clone(), ri.clone());
        }
    }
    global.running = restored;

    global
}

#[tauri::command]
pub async fn save_config(
    instances: HashMap<String, InstanceConfig>,
    model_dirs: Vec<String>,
    engine_dirs: Vec<String>,
    default_engine_id: String,
    instance_order: Vec<String>,
    last_tab: String,
    dark_mode: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let running_snapshot = state.running.lock().unwrap().clone();
    let engine_names = state.engine_names.lock().unwrap().clone();
    let config_dir = state.config_dir.lock().unwrap().clone();
    let existing = read_config_from_disk(&config_dir);
    let global = GlobalConfig {
        instances: instances.clone(),
        model_dirs,
        engine_dirs,
        default_engine_id,
        running: running_snapshot,
        instance_order,
        last_tab,
        dark_mode,
        engine_names,
        download_resume_policy: existing.download_resume_policy,
        download_max_concurrent: existing.download_max_concurrent,
        download_bandwidth_limit_bytes_per_sec: existing.download_bandwidth_limit_bytes_per_sec,
        download_low_priority_throttle: existing.download_low_priority_throttle,
        proxy_config: existing.proxy_config,
    };
    std::fs::create_dir_all(&config_dir).map_err(|e| format!("{}", e))?;
    persist_global_config(&config_dir, &global)?;
    let mut stored = state.instances.lock().unwrap();
    *stored = instances;
    Ok(())
}

#[tauri::command]
pub async fn load_config(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<GlobalConfig, String> {
    let t0 = std::time::Instant::now();
    let config_dir = state.config_dir.lock().unwrap().clone();

    // 读取配置（复用 read_config_from_disk，已含进程存活检查）
    let global = read_config_from_disk(&config_dir);

    // 更新 AppState
    {
        let mut stored = state.instances.lock().unwrap();
        *stored = global.instances.clone();
    }
    *state.engine_names.lock().unwrap() = global.engine_names.clone();
    *state.running.lock().unwrap() = global.running.clone();
    *state.download_max_concurrent.lock().unwrap() = global.download_max_concurrent.max(1);
    *state.download_bandwidth_limit_bytes_per_sec.lock().unwrap() =
        global.download_bandwidth_limit_bytes_per_sec;
    *state.download_low_priority_throttle.lock().unwrap() = global.download_low_priority_throttle;
    *state.proxy_config.lock().unwrap() = global.proxy_config.clone();

    // 为恢复的运行中实例启动健康检查 + 日志恢复 + 指标监控
    for (id, ri) in &global.running {
        if !crate::commands::server::register_restored_runtime_instance(&app, id, ri.pid) {
            continue;
        }
        let id_hc = id.clone();
        let host = if ri.host == "0.0.0.0" {
            "localhost".to_string()
        } else {
            ri.host.clone()
        };
        let host2 = host.clone();
        let port = ri.port;
        let pid = ri.pid;
        let app = app.clone();
        let app2 = app.clone();
        let config_dir = config_dir.clone();

        let api_key_health = {
            let stored = state.instances.lock().unwrap();
            stored
                .get(&id_hc)
                .and_then(|c| {
                    if c.api_key.is_empty() {
                        None
                    } else {
                        Some(c.api_key.clone())
                    }
                })
                .unwrap_or_default()
        };
        let api_key_reconnect = api_key_health.clone();
        std::thread::spawn(move || {
            crate::commands::server::health_check_loop(
                &id_hc,
                &host,
                port,
                pid,
                &api_key_health,
                app,
            );
        });

        crate::commands::server::reconnect_running_instance(
            &id,
            pid,
            &host2,
            port,
            &config_dir,
            &api_key_reconnect,
            app2,
        );
    }

    let t_total = t0.elapsed().as_millis();
    let _ = app.emit(
        "startup-timing",
        serde_json::json!({
            "name": "load-config-rust", "ms": t_total
        }),
    );
    Ok(global)
}

// ── 窗口状态 ─────────────────────────────────────────────────────

/// 从磁盘读取窗口状态 — 供 main.rs setup() 直接调用
pub fn read_window_state_from_disk(config_dir: &std::path::Path) -> Option<WindowState> {
    let path = config_dir.join("window_state.json");
    if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<WindowState>(&s).ok())
    } else {
        None
    }
}

#[tauri::command]
pub fn save_window_state(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    state: tauri::State<'_, AppState>,
) {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let _ = std::fs::create_dir_all(&config_dir);
    let ws = WindowState {
        x,
        y,
        width,
        height,
    };
    if let Ok(json) = serde_json::to_string(&ws) {
        let _ = std::fs::write(config_dir.join("window_state.json"), json);
    }
}

#[tauri::command]
pub fn load_window_state(state: tauri::State<'_, AppState>) -> Option<WindowState> {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let path = config_dir.join("window_state.json");
    if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<WindowState>(&s).ok())
    } else {
        None
    }
}

// ── 路径 resolve（相对 → 绝对） ──────────────────────────────────
#[tauri::command]
pub fn resolve_path(path: String) -> String {
    let pb = std::path::PathBuf::from(&path);
    if pb.is_relative() {
        crate::utils::get_data_dir()
            .join(&path)
            .to_string_lossy()
            .to_string()
    } else {
        path
    }
}
