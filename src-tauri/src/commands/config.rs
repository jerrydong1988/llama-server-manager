use std::collections::HashMap;
use crate::models::{AppState, GlobalConfig, InstanceConfig, WindowState};

// ── 配置持久化 ────────────────────────────────────────────────────
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
    let mut stored = state.instances.lock().unwrap();
    *stored = instances.clone();
    let running_snapshot = state.running.lock().unwrap().clone();
    let engine_names = state.engine_names.lock().unwrap().clone();
    let global = GlobalConfig { instances, model_dirs, engine_dirs, default_engine_id, running: running_snapshot, instance_order, last_tab, dark_mode, engine_names };
    let config_dir = state.config_dir.lock().unwrap().clone();
    std::fs::create_dir_all(&config_dir).map_err(|e| format!("{}", e))?;
    let path = config_dir.join("instances.json");
    let json = serde_json::to_string_pretty(&global).map_err(|e| format!("{}", e))?;
    // #2: 原子写入 — 先写临时文件再 rename，避免崩溃时损坏配置
    let tmp = config_dir.join("instances.json.tmp");
    std::fs::write(&tmp, &json).map_err(|e| format!("保存失败: {}", e))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("保存失败: {}", e))?;
    // 保留备份
    let _ = std::fs::copy(&path, config_dir.join("instances.json.bak"));
    Ok(())
}

#[tauri::command]
pub async fn load_config(state: tauri::State<'_, AppState>, app: tauri::AppHandle) -> Result<GlobalConfig, String> {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let path = config_dir.join("instances.json");

    // 尝试加载主配置，失败则尝试备份
    let load_json = || -> Result<String, String> {
        if path.exists() {
            return std::fs::read_to_string(&path).map_err(|e| format!("{}", e));
        }
        let bak = config_dir.join("instances.json.bak");
        if bak.exists() {
            let json = std::fs::read_to_string(&bak).map_err(|e| format!("{}", e))?;
            // 恢复备份
            let _ = std::fs::write(&path, &json);
            return Ok(json);
        }
        Err("no config".into())
    };

    let json = match load_json() {
        Ok(j) => j,
        Err(_) => return Ok(GlobalConfig {
            instances: HashMap::new(), model_dirs: vec![], engine_dirs: vec![],
            default_engine_id: String::new(), running: HashMap::new(),
            instance_order: vec![], last_tab: "model-repo".into(), dark_mode: true,
            engine_names: HashMap::new(),
        }),
    };

    let mut global: GlobalConfig = serde_json::from_str(&json).map_err(|e| format!("解析配置失败: {}", e))?;

    // 将相对路径的模型/引擎目录 resolve 为绝对路径
    let app_dir = crate::utils::get_data_dir();
    global.model_dirs = global.model_dirs.iter().map(|d| {
        let pb = std::path::PathBuf::from(d);
        if pb.is_relative() { app_dir.join(d).to_string_lossy().to_string() } else { d.clone() }
    }).collect();
    global.engine_dirs = global.engine_dirs.iter().map(|d| {
        let pb = std::path::PathBuf::from(d);
        if pb.is_relative() { app_dir.join(d).to_string_lossy().to_string() } else { d.clone() }
    }).collect();

    let mut stored = state.instances.lock().unwrap();
    *stored = global.instances.clone();
    *state.engine_names.lock().unwrap() = global.engine_names.clone();

    // 恢复仍在运行的后台进程
    let mut restored = HashMap::new();
    for (id, ri) in &global.running {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let alive = std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {}", ri.pid), "/NH"])
                .creation_flags(0x08000000)
                .output()
                .map(|o| {
                    let out = String::from_utf8_lossy(&o.stdout);
                    // Check PID with word boundaries to avoid substring false match
                    // (e.g., PID=12 must not match PID=123)
                    let pid_str = ri.pid.to_string();
                    out.contains(&format!(" {pid_str} "))
                        || out.ends_with(&format!(" {pid_str}\r\n"))
                        || out.ends_with(&format!(" {pid_str}\n"))
                })
                .unwrap_or(false);
            if alive { restored.insert(id.clone(), ri.clone()); }
        }
        #[cfg(not(windows))]
        {
            let alive = std::process::Command::new("kill").arg("-0").arg(ri.pid.to_string()).status().map(|s| s.success()).unwrap_or(false);
            if alive { restored.insert(id.clone(), ri.clone()); }
        }
    }
    *state.running.lock().unwrap() = restored.clone();
    global.running = restored.clone();

    // 为恢复的运行中实例启动健康检查 + 日志恢复 + 指标监控
    for (id, ri) in &global.running {
        let id_hc = id.clone();
        let host = if ri.host == "0.0.0.0" { "localhost".to_string() } else { ri.host.clone() };
        let host2 = host.clone();
        let port = ri.port;
        let pid = ri.pid;
        let app = app.clone();
        let app2 = app.clone();
        let config_dir = config_dir.clone();

        // 健康检查
        let api_key_health = stored.get(&id_hc).and_then(|c| if c.api_key.is_empty() { None } else { Some(c.api_key.clone()) }).unwrap_or_default();
        let api_key_reconnect = api_key_health.clone();
        std::thread::spawn(move || {
            crate::commands::server::health_check_loop(&id_hc, &host, port, pid, &api_key_health, app);
        });

        // 日志 tail + 指标推送
        crate::commands::server::reconnect_running_instance(&id, pid, &host2, port, &config_dir, &api_key_reconnect, app2);
    }
    Ok(global)
}

// ── 窗口状态 ─────────────────────────────────────────────────────
#[tauri::command]
pub fn save_window_state(x: i32, y: i32, width: u32, height: u32, state: tauri::State<'_, AppState>) {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let _ = std::fs::create_dir_all(&config_dir);
    let ws = WindowState { x, y, width, height };
    if let Ok(json) = serde_json::to_string(&ws) {
        let _ = std::fs::write(config_dir.join("window_state.json"), json);
    }
}

#[tauri::command]
pub fn load_window_state(state: tauri::State<'_, AppState>) -> Option<WindowState> {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let path = config_dir.join("window_state.json");
    if path.exists() {
        std::fs::read_to_string(&path).ok()
            .and_then(|s| serde_json::from_str::<WindowState>(&s).ok())
    } else { None }
}

// ── 路径 resolve（相对 → 绝对） ──────────────────────────────────
#[tauri::command]
pub fn resolve_path(path: String) -> String {
    let pb = std::path::PathBuf::from(&path);
    if pb.is_relative() {
        crate::utils::get_data_dir().join(&path).to_string_lossy().to_string()
    } else {
        path
    }
}
