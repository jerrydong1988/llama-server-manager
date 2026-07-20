use crate::error::{AppError, AppResult};
use crate::models::{
    ensure_managed_public_model_alias, AppState, GlobalConfig, InstanceConfig, ProxyConfig,
    WindowState,
};
use crate::vector_policy::normalize_for_launch;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use tauri::Emitter;

// Unified config write helpers.

static CONFIG_WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn normalize_model_dirs(mut model_dirs: Vec<String>) -> Vec<String> {
    model_dirs.retain(|directory| !directory.trim().is_empty());
    if model_dirs.is_empty() {
        model_dirs.push(crate::utils::DEFAULT_MODELS_DIR_NAME.to_string());
    }
    model_dirs
}

fn default_global_config() -> GlobalConfig {
    GlobalConfig {
        config_load_warning: None,
        instances: HashMap::new(),
        model_dirs: normalize_model_dirs(Vec::new()),
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
) -> Result<bool, String> {
    let path = config_dir.join("instances.json");
    let mut persisted_value =
        serde_json::to_value(global).map_err(|e| format!("序列化失败: {}", e))?;
    if let Some(object) = persisted_value.as_object_mut() {
        object.remove("config_load_warning");
    }
    let json =
        serde_json::to_string_pretty(&persisted_value).map_err(|e| format!("序列化失败: {}", e))?;
    if std::fs::read_to_string(&path).is_ok_and(|current| current == json) {
        let backup = config_dir.join("instances.json.bak");
        if !backup.exists() {
            let _ = std::fs::copy(&path, backup);
        }
        return Ok(false);
    }
    let primary_is_valid = std::fs::read_to_string(&path)
        .ok()
        .and_then(|contents| serde_json::from_str::<GlobalConfig>(&contents).ok())
        .is_some();
    let backup_path = config_dir.join("instances.json.bak");
    crate::persistence::atomic_write(
        &path,
        json.as_bytes(),
        primary_is_valid.then_some(backup_path.as_path()),
    )
    .map_err(|error| format!("保存失败: {error}"))?;
    Ok(true)
}

/// Atomically writes instances.json; all config writes should go through this helper to avoid races.
pub fn persist_global_config(
    config_dir: &std::path::Path,
    global: &GlobalConfig,
) -> Result<(), String> {
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap();
    persist_global_config_unlocked(config_dir, global).map(|_| ())
}

/// Reads existing config, mutates it, then writes atomically for non-save_config paths.
pub fn update_and_persist<F>(state: &AppState, update_fn: F) -> Result<(), String>
where
    F: FnOnce(&mut GlobalConfig),
{
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap();
    let config_dir = state.config_dir.lock().unwrap().clone();
    let mut global = load_global_config_for_update_unlocked(&config_dir)?;
    update_fn(&mut global);
    persist_global_config_unlocked(&config_dir, &global).map(|_| ())
}

// Config persistence.

fn load_global_config_file(config_dir: &std::path::Path) -> GlobalConfig {
    let path = config_dir.join("instances.json");
    let backup_path = config_dir.join("instances.json.bak");
    let mut config = match std::fs::read_to_string(&path) {
        Ok(json) => match serde_json::from_str::<GlobalConfig>(&json) {
            Ok(config) => config,
            Err(primary_error) => match std::fs::read_to_string(&backup_path)
                .ok()
                .and_then(|backup| serde_json::from_str::<GlobalConfig>(&backup).ok())
            {
                Some(mut config) => {
                    config.config_load_warning =
                        Some(format!("主配置损坏，已从备份恢复：{primary_error}"));
                    config
                }
                None => {
                    let mut config = default_global_config();
                    config.config_load_warning = Some(format!(
                        "主配置与备份均损坏，已进入只读恢复状态：{primary_error}"
                    ));
                    config
                }
            },
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::read_to_string(&backup_path)
                .ok()
                .and_then(|backup| serde_json::from_str::<GlobalConfig>(&backup).ok())
            {
                Some(mut config) => {
                    config.config_load_warning = Some("主配置缺失，已从备份恢复".to_string());
                    config
                }
                None => default_global_config(),
            }
        }
        Err(error) => {
            let mut config = default_global_config();
            config.config_load_warning = Some(format!("读取主配置失败：{error}"));
            config
        }
    };
    config.model_dirs = normalize_model_dirs(config.model_dirs);
    config
}

fn load_global_config_for_update_unlocked(
    config_dir: &std::path::Path,
) -> Result<GlobalConfig, String> {
    let primary_path = config_dir.join("instances.json");
    let backup_path = config_dir.join("instances.json.bak");
    let primary = std::fs::read_to_string(&primary_path);
    if let Ok(contents) = &primary {
        if let Ok(mut config) = serde_json::from_str::<GlobalConfig>(contents) {
            config.model_dirs = normalize_model_dirs(config.model_dirs);
            return Ok(config);
        }
    }

    if let Ok(contents) = std::fs::read_to_string(&backup_path) {
        let mut config = serde_json::from_str::<GlobalConfig>(&contents)
            .map_err(|error| format!("解析配置备份失败: {error}"))?;
        config.model_dirs = normalize_model_dirs(config.model_dirs);
        crate::persistence::atomic_write(&primary_path, contents.as_bytes(), None)
            .map_err(|error| format!("修复主配置失败: {error}"))?;
        return Ok(config);
    }

    match primary {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(default_global_config()),
        Err(error) => Err(format!("读取配置失败: {error}")),
        Ok(_) => Err("主配置损坏且没有有效备份".into()),
    }
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
        if let Err(error) = persist_global_config(config_dir, &global) {
            eprintln!("Failed to persist stale runtime cleanup: {error}");
        }
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
    global.model_dirs = normalize_model_dirs(snapshot.model_dirs);
    global.engine_dirs = snapshot.engine_dirs;
    global.default_engine_id = snapshot.default_engine_id;
    global.running = running;
    global.instance_order = snapshot.instance_order;
    global.last_tab = snapshot.last_tab;
    global.dark_mode = snapshot.dark_mode;
    global.engine_names = engine_names;
}

struct NormalizedInstances {
    all: HashMap<String, InstanceConfig>,
    changed: HashMap<String, InstanceConfig>,
}

fn normalize_instances_for_save(instances: HashMap<String, InstanceConfig>) -> NormalizedInstances {
    let mut all = HashMap::with_capacity(instances.len());
    let mut changed = HashMap::new();
    for (id, config) in instances {
        let mut public_config = config.clone();
        ensure_managed_public_model_alias(&mut public_config);
        let normalized = if public_config.launch_mode.eq_ignore_ascii_case("manual") {
            public_config
        } else {
            normalize_for_launch(public_config).into_config()
        };
        if normalized != config {
            changed.insert(id.clone(), normalized.clone());
        }
        all.insert(id, normalized);
    }
    NormalizedInstances { all, changed }
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
) -> AppResult<HashMap<String, InstanceConfig>> {
    let normalized = normalize_instances_for_save(instances);
    let instances = normalized.all;
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
    std::fs::create_dir_all(&config_dir).map_err(|error| {
        AppError::new("CONFIG_DIRECTORY_WRITE", error.to_string(), true)
            .with_context("path", config_dir.display().to_string())
    })?;
    {
        let _guard = CONFIG_WRITE_LOCK.lock().unwrap();
        // Runtime-owned fields must be sampled after taking the write lock. Otherwise a
        // concurrent start/stop can persist newer state and then be overwritten here.
        let running_snapshot = state.running.lock().unwrap().clone();
        let engine_names = state.engine_names.lock().unwrap().clone();
        let mut global = load_global_config_for_update_unlocked(&config_dir).map_err(|error| {
            AppError::new("CONFIG_RECOVERY_FAILED", error, true).with_context(
                "path",
                config_dir.join("instances.json").display().to_string(),
            )
        })?;
        apply_frontend_config(&mut global, snapshot, running_snapshot, engine_names);
        persist_global_config_unlocked(&config_dir, &global).map_err(|error| {
            AppError::new("CONFIG_PERSIST_FAILED", error, true).with_context(
                "path",
                config_dir.join("instances.json").display().to_string(),
            )
        })?;
    }
    {
        let mut stored = state.instances.lock().unwrap();
        *stored = instances.clone();
    }
    if crate::runtime_service::manages_instances() {
        let sync_generation = crate::runtime_service::mark_config_sync_pending();
        crate::runtime_service::sync_app_config(&state)
            .await
            .map_err(|error| AppError::new("RUNTIME_CONFIG_SYNC_FAILED", error, true))?;
        crate::runtime_service::mark_config_sync_complete(sync_generation);
    }
    Ok(normalized.changed)
}

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

    // Restore log capture, metrics, and the single authoritative health monitor.
    let runtime_managed = crate::runtime_service::persisted_managed_instance_ids();
    for (id, ri) in &global.running {
        if !crate::commands::server::register_restored_runtime_instance(&app, id, ri.pid) {
            continue;
        }
        let pid = ri.pid;
        let app_reconnect = app.clone();
        let config_dir = config_dir.clone();

        let launch_config = ri
            .launch_config
            .clone()
            .or_else(|| global.instances.get(id).cloned())
            .unwrap_or_else(|| InstanceConfig {
                host: ri.host.clone(),
                port: ri.port,
                ..InstanceConfig::default()
            });
        if runtime_managed.contains(id) {
            state
                .runtime_managed_instances
                .lock()
                .unwrap()
                .insert(id.clone());
            crate::commands::server::reconnect_runtime_instance_logs(
                id,
                pid,
                &config_dir,
                app_reconnect,
            );
        } else {
            crate::commands::server::reconnect_running_instance(
                id,
                pid,
                &launch_config,
                &config_dir,
                app_reconnect,
            );
        }
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

pub fn persist_window_state(
    config_dir: &std::path::Path,
    window_state: &WindowState,
) -> AppResult<()> {
    let json = serde_json::to_vec(window_state)
        .map_err(|error| AppError::new("WINDOW_STATE_SERIALIZE", error.to_string(), false))?;
    let path = config_dir.join("window_state.json");
    crate::persistence::atomic_write(&path, &json, None).map_err(|error| {
        AppError::new("WINDOW_STATE_PERSIST_FAILED", error, true)
            .with_context("path", path.display().to_string())
    })
}

#[tauri::command]
pub fn save_window_state(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    state: tauri::State<'_, AppState>,
) -> AppResult<()> {
    let config_dir = state.config_dir.lock().unwrap().clone();
    let ws = WindowState {
        x,
        y,
        width,
        height,
    };
    persist_window_state(&config_dir, &ws)
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
        let config = &normalized.all["embedding"];
        assert!(config.embedding);
        assert!(config.spec_type.is_empty());
        assert_eq!(config.custom_args, vec!["--temp 1.5"]);
        assert_eq!(normalized.changed["embedding"], *config);
    }

    #[test]
    fn save_config_returns_only_backend_normalization_changes() {
        let config = normalize_for_launch(InstanceConfig::default()).into_config();
        let mut instances = HashMap::new();
        instances.insert("clean".into(), config.clone());

        let normalized = normalize_instances_for_save(instances);

        assert_eq!(normalized.all["clean"], config);
        assert!(normalized.changed.is_empty());
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
            config_load_warning: None,
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
    fn fresh_config_exposes_the_default_model_scan_root() {
        let dir = temp_config_dir("fresh-default-model-root");

        let loaded = read_config_from_disk(&dir);

        assert_eq!(
            loaded.model_dirs,
            vec![crate::utils::get_default_models_dir()
                .to_string_lossy()
                .to_string()]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn legacy_empty_model_roots_migrate_to_the_default_directory() {
        let dir = temp_config_dir("legacy-empty-model-root");
        let mut legacy = sample_config();
        legacy.model_dirs.clear();
        std::fs::write(
            dir.join("instances.json"),
            serde_json::to_string_pretty(&legacy).unwrap(),
        )
        .unwrap();

        let loaded = read_config_from_disk(&dir);

        assert_eq!(
            loaded.model_dirs,
            vec![crate::utils::get_default_models_dir()
                .to_string_lossy()
                .to_string()]
        );
        let _ = std::fs::remove_dir_all(dir);
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
        assert!(loaded
            .config_load_warning
            .as_deref()
            .is_some_and(|warning| warning.contains("已从备份恢复")));
        assert_eq!(
            loaded.download_max_concurrent,
            expected.download_max_concurrent
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn double_corruption_is_reported_and_cannot_be_silently_overwritten() {
        let dir = temp_config_dir("double-corruption");
        std::fs::write(dir.join("instances.json"), "{broken-primary").unwrap();
        std::fs::write(dir.join("instances.json.bak"), "{broken-backup").unwrap();

        let loaded = read_config_from_disk(&dir);

        assert!(loaded.instances.is_empty());
        assert!(loaded
            .config_load_warning
            .as_deref()
            .is_some_and(|warning| warning.contains("主配置与备份均损坏")));
        assert!(load_global_config_for_update_unlocked(&dir).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("instances.json")).unwrap(),
            "{broken-primary"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn transient_recovery_warning_is_never_persisted() {
        let dir = temp_config_dir("transient-warning");
        let mut config = sample_config();
        config.config_load_warning = Some("do not persist".into());

        persist_global_config_unlocked(&dir, &config).unwrap();

        let json = std::fs::read_to_string(dir.join("instances.json")).unwrap();
        assert!(!json.contains("config_load_warning"));
        assert!(!json.contains("do not persist"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn update_load_repairs_corrupt_primary_without_destroying_valid_backup() {
        let dir = temp_config_dir("backup-repair-for-update");
        let expected = sample_config();
        let backup_json = serde_json::to_string_pretty(&expected).unwrap();
        std::fs::write(dir.join("instances.json"), "{not-json").unwrap();
        std::fs::write(dir.join("instances.json.bak"), &backup_json).unwrap();

        let mut loaded = load_global_config_for_update_unlocked(&dir).unwrap();

        assert_eq!(loaded.default_engine_id, expected.default_engine_id);
        assert_eq!(
            serde_json::from_str::<GlobalConfig>(
                &std::fs::read_to_string(dir.join("instances.json")).unwrap()
            )
            .unwrap()
            .last_tab,
            expected.last_tab
        );
        assert_eq!(
            serde_json::from_str::<GlobalConfig>(
                &std::fs::read_to_string(dir.join("instances.json.bak")).unwrap()
            )
            .unwrap()
            .last_tab,
            expected.last_tab
        );

        loaded.dark_mode = true;
        assert!(persist_global_config_unlocked(&dir, &loaded).unwrap());
        assert!(
            serde_json::from_str::<GlobalConfig>(
                &std::fs::read_to_string(dir.join("instances.json.bak")).unwrap()
            )
            .is_ok(),
            "a repaired update must never rotate corrupt JSON into the backup"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn identical_config_skips_redundant_atomic_write() {
        let dir = temp_config_dir("skip-identical-write");
        let mut config = sample_config();

        assert!(persist_global_config_unlocked(&dir, &config).unwrap());
        assert!(!persist_global_config_unlocked(&dir, &config).unwrap());

        config.dark_mode = !config.dark_mode;
        assert!(persist_global_config_unlocked(&dir, &config).unwrap());
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

    #[test]
    fn frontend_config_cannot_persist_an_implicit_empty_model_root() {
        let mut config = sample_config();
        let snapshot = FrontendConfigSnapshot {
            instances: HashMap::new(),
            model_dirs: Vec::new(),
            engine_dirs: vec!["engines-new".into()],
            default_engine_id: String::new(),
            instance_order: Vec::new(),
            last_tab: "model-repo".into(),
            dark_mode: true,
        };

        apply_frontend_config(&mut config, snapshot, HashMap::new(), HashMap::new());

        assert_eq!(
            config.model_dirs,
            vec![crate::utils::DEFAULT_MODELS_DIR_NAME.to_string()]
        );
    }
}

// IPC compatibility boundary: legacy command internals keep their existing error flow,
// while every registered command serializes a stable AppError object.
#[allow(dead_code, unused_imports, unused_mut)] // Tauri references adapters through generated macros.
pub mod ipc {
    use super::*;

    #[tauri::command]
    pub async fn load_config(
        state: tauri::State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<GlobalConfig> {
        super::load_config(state, app)
            .await
            .map_err(crate::error::AppError::from)
    }
}
