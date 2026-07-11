use crate::models::{AppState, GlobalConfig, InstanceConfig, ProxyConfig, WindowState};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use tauri::Emitter;

// Unified config write helpers.

static CONFIG_WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn default_global_config() -> GlobalConfig {
    GlobalConfig {
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

fn persist_global_config_unlocked(
    config_dir: &std::path::Path,
    global: &GlobalConfig,
) -> Result<(), String> {
    let path = config_dir.join("instances.json");
    let json = serde_json::to_string_pretty(global).map_err(|e| format!("序列化失败: {}", e))?;
    let tmp = config_dir.join("instances.json.tmp");
    std::fs::write(&tmp, &json).map_err(|e| format!("保存失败: {}", e))?;
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("保存失败: {}", e))?;
    let _ = std::fs::copy(&path, config_dir.join("instances.json.bak"));
    Ok(())
}

/// Atomically writes instances.json; all config writes should go through this helper to avoid races.
pub fn persist_global_config(
    config_dir: &std::path::Path,
    global: &GlobalConfig,
) -> Result<(), String> {
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap();
    persist_global_config_unlocked(config_dir, global)
}

/// Reads existing config, mutates it, then writes atomically for non-save_config paths.
pub fn update_and_persist<F>(state: &AppState, update_fn: F) -> Result<(), String>
where
    F: FnOnce(&mut GlobalConfig),
{
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap();
    let config_dir = state.config_dir.lock().unwrap().clone();
    let path = config_dir.join("instances.json");
    let json = std::fs::read_to_string(&path).map_err(|e| format!("读取配置失败: {}", e))?;
    let mut global: GlobalConfig =
        serde_json::from_str(&json).map_err(|e| format!("解析配置失败: {}", e))?;
    update_fn(&mut global);
    persist_global_config_unlocked(&config_dir, &global)
}

/// Lightweight process liveness check using Windows OpenProcess or Unix kill(0), avoiding a full sysinfo refresh.
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

// Config persistence.

/// Reads config from disk and resolves paths without AppState so main.rs setup() can call it early.
pub fn read_config_from_disk(config_dir: &std::path::Path) -> GlobalConfig {
    let path = config_dir.join("instances.json");

    let load_json = || -> Result<String, String> {
        let primary_result = std::fs::read_to_string(&path);
        if let Ok(json) = primary_result {
            return Ok(json);
        }
        // If the primary file cannot be read, possibly because it is locked, fall back to .bak.
        let bak = config_dir.join("instances.json.bak");
        if bak.exists() {
            let json = std::fs::read_to_string(&bak).map_err(|e| format!("{}", e))?;
            return Ok(json);
        }
        // If the primary file exists but cannot be read and .bak is missing, return the primary error.
        if path.exists() {
            primary_result.map_err(|e| format!("{}", e))
        } else {
            Err("no config".into())
        }
    };

    let json = match load_json() {
        Ok(j) => j,
        Err(_) => {
            return default_global_config()
        }
    };

    let mut global: GlobalConfig = match serde_json::from_str(&json) {
        Ok(g) => g,
        Err(_) => {
            let bak = config_dir.join("instances.json.bak");
            let Ok(backup_json) = std::fs::read_to_string(&bak) else {
                return default_global_config();
            };
            match serde_json::from_str(&backup_json) {
                Ok(g) => g,
                Err(_) => return default_global_config(),
            }
        }
    };

    // Resolve relative paths.
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

    // Filter dead processes.
    let mut restored = HashMap::new();
    let mut stale_sessions = Vec::new();
    for (id, ri) in &global.running {
        if is_process_alive(ri.pid) {
            restored.insert(id.clone(), ri.clone());
        } else if let Some(session_id) = &ri.telemetry_session_id {
            stale_sessions.push(session_id.clone());
        }
    }
    let removed_running = restored.len() != global.running.len();
    global.running = restored;
    if removed_running {
        for session_id in stale_sessions {
            let _ = crate::commands::telemetry::finish_run_session(
                Some(session_id.as_str()),
                None,
                "restore-cleanup",
            );
        }
        let _ = persist_global_config(config_dir, &global);
    }

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

    // Read config through read_config_from_disk, including process liveness checks.
    let global = read_config_from_disk(&config_dir);

    // Update AppState.
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

    // Start health checks, log recovery, and metrics monitoring for restored running instances.
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
                .map(crate::commands::server::effective_api_key)
                .filter(|key| !key.is_empty())
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

// Window state.

/// Reads window state from disk for direct use by main.rs setup().
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

// Path resolution from relative to absolute paths.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lsm-config-test-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_config() -> GlobalConfig {
        GlobalConfig {
            instances: HashMap::new(),
            model_dirs: vec!["models-a".into()],
            engine_dirs: vec!["engines-a".into()],
            default_engine_id: "engine-a".into(),
            running: HashMap::new(),
            instance_order: vec![],
            last_tab: "proxy".into(),
            dark_mode: false,
            engine_names: HashMap::new(),
            download_resume_policy: "manual".into(),
            download_max_concurrent: 2,
            download_bandwidth_limit_bytes_per_sec: 0,
            download_low_priority_throttle: false,
            proxy_config: ProxyConfig::default(),
        }
    }

    #[test]
    fn read_config_falls_back_to_backup_when_primary_json_is_corrupt() {
        let dir = temp_config_dir("backup-fallback");
        let expected = sample_config();
        std::fs::write(dir.join("instances.json"), "{not-json").unwrap();
        std::fs::write(
            dir.join("instances.json.bak"),
            serde_json::to_string_pretty(&expected).unwrap(),
        )
        .unwrap();

        let loaded = read_config_from_disk(&dir);

        assert_eq!(loaded.default_engine_id, expected.default_engine_id);
        assert_eq!(loaded.last_tab, expected.last_tab);
        assert_eq!(
            loaded.download_max_concurrent,
            expected.download_max_concurrent
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
