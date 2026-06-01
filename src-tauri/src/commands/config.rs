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
    let global = GlobalConfig { instances, model_dirs, engine_dirs, default_engine_id, running: running_snapshot, instance_order, last_tab, dark_mode };
    let config_dir = state.config_dir.lock().unwrap().clone();
    std::fs::create_dir_all(&config_dir).map_err(|e| format!("{}", e))?;
    let path = config_dir.join("instances.json");
    let json = serde_json::to_string_pretty(&global).map_err(|e| format!("{}", e))?;
    // 备份旧配置（如果存在）
    if path.exists() {
        let bak = config_dir.join("instances.json.bak");
        let _ = std::fs::copy(&path, &bak);
    }
    std::fs::write(&path, json).map_err(|e| format!("保存失败: {}", e))?;
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
        }),
    };

    let mut global: GlobalConfig = serde_json::from_str(&json).map_err(|e| format!("解析配置失败: {}", e))?;
    let mut stored = state.instances.lock().unwrap();
    *stored = global.instances.clone();

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
                .map(|o| String::from_utf8_lossy(&o.stdout).contains(&ri.pid.to_string()))
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

    // 为恢复的运行中实例启动健康检查
    for (id, ri) in &global.running {
        let id = id.clone();
        let host = if ri.host == "0.0.0.0" { "localhost".to_string() } else { ri.host.clone() };
        let port = ri.port;
        let pid = ri.pid;
        let app = app.clone();
        std::thread::spawn(move || {
            crate::commands::server::health_check_loop(&id, &host, port, pid, app);
        });
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
