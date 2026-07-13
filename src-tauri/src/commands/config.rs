use crate::models::{AppState, GlobalConfig, InstanceConfig, ProxyConfig, WindowState};
use crate::vector_policy::normalize_for_launch;
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

// Config persistence.

fn load_global_config_file(config_dir: &std::path::Path) -> GlobalConfig {
    let path = config_dir.join("instances.json");
    let primary_result = std::fs::read_to_string(&path);
    let json = match primary_result {
        Ok(json) => Some(json),
        Err(_) => {
            let backup_path = config_dir.join("instances.json.bak");
            std::fs::read_to_string(backup_path).ok()
        }
    };
    let Some(json) = json else {
        return default_global_config();
    };
    serde_json::from_str(&json).unwrap_or_else(|_| {
        let backup_path = config_dir.join("instances.json.bak");
        std::fs::read_to_string(backup_path)
            .ok()
            .and_then(|backup| serde_json::from_str(&backup).ok())
            .unwrap_or_else(default_global_config)
    })
}

/// Reads config from disk and resolves paths without AppState so main.rs setup() can call it early.
pub fn read_config_from_disk(config_dir: &std::path::Path) -> GlobalConfig {
    let mut global = load_global_config_file(config_dir);

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
        if crate::commands::server::running_instance_matches_live_process(ri) {
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

struct FrontendConfigSnapshot {
    instances: HashMap<String, InstanceConfig>,
    model_dirs: Vec<String>,
    engine_dirs: Vec<String>,
    default_engine_id: String,
    instance_order: Vec<String>,
    last_tab: String,
    dark_mode: bool,
}

fn apply_frontend_config(
    global: &mut GlobalConfig,
    snapshot: FrontendConfigSnapshot,
    running: HashMap<String, crate::models::RunningInstance>,
    engine_names: HashMap<String, String>,
) {
    global.instances = snapshot.instances;
    global.model_dirs = snapshot.model_dirs;
    global.engine_dirs = snapshot.engine_dirs;
    global.default_engine_id = snapshot.default_engine_id;
    global.running = running;
    global.instance_order = snapshot.instance_order;
    global.last_tab = snapshot.last_tab;
    global.dark_mode = snapshot.dark_mode;
    global.engine_names = engine_names;
}

fn normalize_instances_for_save(
    instances: HashMap<String, InstanceConfig>,
) -> HashMap<String, InstanceConfig> {
    instances
        .into_iter()
        .map(|(id, config)| (id, normalize_for_launch(config).into_config()))
        .collect()
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri expands IPC fields into command parameters.
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
    let instances = normalize_instances_for_save(instances);
    let running_snapshot = state.running.lock().unwrap().clone();
    let engine_names = state.engine_names.lock().unwrap().clone();
    let config_dir = state.config_dir.lock().unwrap().clone();
    let snapshot = FrontendConfigSnapshot {
        instances: instances.clone(),
        model_dirs,
        engine_dirs,
        default_engine_id,
        instance_order,
        last_tab,
        dark_mode,
    };
    std::fs::create_dir_all(&config_dir).map_err(|e| format!("{}", e))?;
    {
        let _guard = CONFIG_WRITE_LOCK.lock().unwrap();
        let mut global = load_global_config_file(&config_dir);
        apply_frontend_config(&mut global, snapshot, running_snapshot, engine_names);
        persist_global_config_unlocked(&config_dir, &global)?;
    }
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
            id,
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

    #[test]
    fn save_config_normalizes_vector_instances_before_storage() {
        let mut instances = HashMap::new();
        instances.insert(
            "embedding".into(),
            InstanceConfig {
                id: "embedding".into(),
                model_path: "C:/models/bge-small.gguf".into(),
                spec_type: "draft-mtp".into(),
                custom_args: vec!["--temp 1.5".into()],
                ..InstanceConfig::default()
            },
        );

        let normalized = normalize_instances_for_save(instances);
        let config = &normalized["embedding"];
        assert!(config.embedding);
        assert!(config.spec_type.is_empty());
        assert!(config.custom_args.is_empty());
    }

    fn temp_config_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lsm-config-test-{}-{}", name, std::process::id()));
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

    #[test]
    fn frontend_config_merge_preserves_backend_owned_fields() {
        let mut config = sample_config();
        config.download_max_concurrent = 7;
        config.proxy_config.public_api_key = "proxy-secret".into();
        let snapshot = FrontendConfigSnapshot {
            instances: HashMap::new(),
            model_dirs: vec!["models-new".into()],
            engine_dirs: vec!["engines-new".into()],
            default_engine_id: "engine-new".into(),
            instance_order: vec!["instance-new".into()],
            last_tab: "dashboard".into(),
            dark_mode: true,
        };

        apply_frontend_config(&mut config, snapshot, HashMap::new(), HashMap::new());

        assert_eq!(config.download_max_concurrent, 7);
        assert_eq!(config.proxy_config.public_api_key, "proxy-secret");
        assert_eq!(config.default_engine_id, "engine-new");
        assert_eq!(config.last_tab, "dashboard");
    }
}
