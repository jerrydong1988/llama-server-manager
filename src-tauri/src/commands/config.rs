use crate::error::{AppError, AppResult};
use crate::models::{
    ensure_managed_public_model_alias, migrate_legacy_load_mode, AppState, FrontendGlobalConfig,
    GlobalConfig, InstanceConfig, ProxyConfig, WindowState,
};
use crate::vector_policy::normalize_for_launch;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use tauri::Emitter;

// Unified config write helpers.

pub(crate) static CONFIG_WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn dedupe_path_dirs(directories: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    directories
        .into_iter()
        .filter(|directory| !directory.trim().is_empty())
        .filter(|directory| seen.insert(crate::path_utils::path_identity_key(Path::new(directory))))
        .collect()
}

fn normalize_model_dirs(model_dirs: Vec<String>) -> Vec<String> {
    let mut model_dirs = dedupe_path_dirs(model_dirs);
    if model_dirs.is_empty() {
        model_dirs.push(crate::utils::DEFAULT_MODELS_DIR_NAME.to_string());
    }
    model_dirs
}

fn migrate_global_load_modes(global: &mut GlobalConfig) -> bool {
    let mut changed = false;
    for config in global.instances.values_mut() {
        changed |= migrate_legacy_load_mode(config);
    }
    for running in global.running.values_mut() {
        if let Some(config) = running.launch_config.as_mut() {
            changed |= migrate_legacy_load_mode(config);
        }
    }
    changed
}

pub(crate) fn default_global_config() -> GlobalConfig {
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
        config_revision_schema_version: crate::config_revision::CONFIG_REVISION_SCHEMA_VERSION,
        config_revisions: HashMap::new(),
        known_good_config_revisions: HashMap::new(),
        config_revision_audit: Vec::new(),
        deployment_schema_version: crate::deployment::DEPLOYMENT_SCHEMA_VERSION,
        deployments: HashMap::new(),
        canary_schema_version: crate::canary::CANARY_SCHEMA_VERSION,
        canary_rollouts: Vec::new(),
        residency_schema_version: crate::residency::RESIDENCY_SCHEMA_VERSION,
        residency_policy: crate::residency::ResidencyPolicy::default(),
        residency_placements: Vec::new(),
        residency_audit: Vec::new(),
    }
}

pub(crate) fn persist_global_config_unlocked(
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
            if let Ok(contents) = std::fs::read(&path) {
                crate::persistence::atomic_write(&backup, &contents, None)?;
            }
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
    crate::config_revision::ensure_current_config_revisions(&mut global)?;
    crate::deployment::ensure_deployments(&mut global)?;
    crate::canary::ensure_canary_catalog(&mut global)?;
    crate::residency::ensure_residency_catalog(&mut global)?;
    update_fn(&mut global);
    persist_global_config_unlocked(&config_dir, &global).map(|_| ())
}

/// Persists proxy routing only after rechecking instance start reservations under
/// the serialized config-write boundary. This closes the check-to-write window
/// between a proxy edit and deployment revision materialization.
pub fn update_proxy_config_and_persist(
    state: &AppState,
    proxy_config: &ProxyConfig,
) -> Result<(), String> {
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap();
    let config_dir = state.config_dir.lock().unwrap().clone();
    let mut global = load_global_config_for_update_unlocked(&config_dir)?;
    crate::config_revision::ensure_current_config_revisions(&mut global)?;
    crate::deployment::ensure_deployments(&mut global)?;
    crate::canary::ensure_canary_catalog(&mut global)?;
    crate::residency::ensure_residency_catalog(&mut global)?;
    let routing_changes = crate::deployment::routing_changed_instance_ids(
        &global.proxy_config,
        proxy_config,
        global.instances.keys().cloned(),
    );
    let canary_instances = crate::canary::active_instance_ids(&global);
    if let Some(instance_id) = routing_changes
        .iter()
        .find(|instance_id| canary_instances.contains(*instance_id))
    {
        return Err(format!(
            "instance {instance_id} is bound to an unresolved canary rollout; abort or roll back the rollout before changing its deployment routing"
        ));
    }
    let lifecycle_conflict = {
        let starting = state.starting.lock().unwrap();
        routing_changes
            .iter()
            .find(|instance_id| starting.contains(instance_id.as_str()))
            .cloned()
    };
    if let Some(instance_id) = lifecycle_conflict {
        return Err(format!(
            "实例 {instance_id} 正在启动，部署路由状态暂时不能修改；请等待启动完成后重试"
        ));
    }
    global.proxy_config = proxy_config.clone();
    persist_global_config_unlocked(&config_dir, &global).map(|_| ())
}

/// Materializes a deployment revision under the same serialized boundary used by
/// configuration saves. The caller must already hold the instance start reservation.
pub fn materialize_deployment_revision(
    state: &AppState,
    instance_id: &str,
    identity: &crate::deployment_identity::DeploymentIdentity,
) -> Result<crate::deployment::DeploymentRevision, String> {
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap();
    let config_dir = state.config_dir.lock().unwrap().clone();
    let mut global = load_global_config_for_update_unlocked(&config_dir)?;
    crate::config_revision::ensure_current_config_revisions(&mut global)?;
    crate::deployment::ensure_deployments(&mut global)?;
    crate::canary::ensure_canary_catalog(&mut global)?;
    crate::residency::ensure_residency_catalog(&mut global)?;
    let bound_revision =
        crate::canary::active_revision_for_instance(&global, instance_id).map(str::to_owned);
    let revision = crate::deployment::materialize_revision(&mut global, instance_id, identity)?;
    if bound_revision
        .as_deref()
        .is_some_and(|expected| expected != revision.id)
    {
        return Err(format!(
            "instance {instance_id} is bound to another revision by an unresolved canary rollout; abort or roll back the rollout before starting a new revision"
        ));
    }
    persist_global_config_unlocked(&config_dir, &global)?;
    Ok(revision)
}

pub fn inspect_deployment_catalog(
    state: &AppState,
    instance_id: &str,
    expected_identity: Option<&crate::deployment_identity::DeploymentIdentity>,
    preflight_error: Option<String>,
) -> Result<crate::deployment::DeploymentInspection, String> {
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap();
    let config_dir = state.config_dir.lock().unwrap().clone();
    let mut global = load_global_config_for_update_unlocked(&config_dir)?;
    let config_changed = crate::config_revision::ensure_current_config_revisions(&mut global)?;
    let deployment_changed = crate::deployment::ensure_deployments(&mut global)?;
    let canary_changed = crate::canary::ensure_canary_catalog(&mut global)?;
    let residency_changed = crate::residency::ensure_residency_catalog(&mut global)?;
    if config_changed || deployment_changed || canary_changed || residency_changed {
        persist_global_config_unlocked(&config_dir, &global)?;
    }
    let running_revision_id = state
        .running
        .lock()
        .unwrap()
        .get(instance_id)
        .map(|running| running.deployment_revision_id.clone());
    Ok(crate::deployment::inspect(
        &mut global,
        instance_id,
        expected_identity,
        preflight_error,
        running_revision_id,
    ))
}

/// Persists an engine-name snapshot before publishing it to shared memory. Keeping both
/// operations under the config write lock prevents a concurrent save_config from restoring
/// the previous names between the disk write and the in-memory commit.
fn publish_engine_names_after_persist<F>(
    shared_engine_names: &Mutex<HashMap<String, String>>,
    engine_names: HashMap<String, String>,
    persist: F,
) -> Result<(), String>
where
    F: FnOnce(&HashMap<String, String>) -> Result<(), String>,
{
    persist(&engine_names)?;
    *shared_engine_names.lock().unwrap() = engine_names;
    Ok(())
}

pub fn replace_engine_names_and_persist(
    state: &AppState,
    engine_names: HashMap<String, String>,
) -> Result<(), String> {
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap();
    let config_dir = state.config_dir.lock().unwrap().clone();
    let mut global = load_global_config_for_update_unlocked(&config_dir)?;
    crate::config_revision::ensure_current_config_revisions(&mut global)?;
    crate::deployment::ensure_deployments(&mut global)?;
    crate::canary::ensure_canary_catalog(&mut global)?;
    crate::residency::ensure_residency_catalog(&mut global)?;
    publish_engine_names_after_persist(&state.engine_names, engine_names, |names| {
        global.engine_names = names.clone();
        persist_global_config_unlocked(&config_dir, &global).map(|_| ())
    })
}

// Config persistence.

fn load_global_config_file(config_dir: &std::path::Path) -> GlobalConfig {
    let path = config_dir.join("instances.json");
    let backup_path = config_dir.join("instances.json.bak");
    for private_path in [&path, &backup_path] {
        if let Err(error) = crate::persistence::enforce_private_file(private_path) {
            eprintln!("Failed to enforce private config permissions: {error}");
        }
    }
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
    config.engine_dirs = dedupe_path_dirs(config.engine_dirs);
    migrate_global_load_modes(&mut config);
    let migration = crate::config_revision::ensure_current_config_revisions(&mut config)
        .and_then(|config_changed| {
            crate::deployment::ensure_deployments(&mut config)
                .map(|deployment_changed| config_changed || deployment_changed)
        })
        .and_then(|changed| {
            crate::canary::ensure_canary_catalog(&mut config)
                .map(|canary_changed| changed || canary_changed)
        })
        .and_then(|changed| {
            crate::residency::ensure_residency_catalog(&mut config)
                .map(|residency_changed| changed || residency_changed)
        });
    match migration {
        Ok(true) => {
            if let Err(error) = persist_global_config(config_dir, &config) {
                let warning = format!("配置、部署修订或模型驻留记录迁移未能持久化：{error}");
                config.config_load_warning = Some(match config.config_load_warning.take() {
                    Some(existing) => format!("{existing}；{warning}"),
                    None => warning,
                });
            }
        }
        Ok(false) => {}
        Err(error) => {
            // A rollout whose integrity cannot be proven must never resume
            // traffic automatically. Keep its evidence intact but disable the
            // proxy in the recovered in-memory snapshot.
            config.proxy_config.enabled = false;
            let warning = format!("配置、部署、金丝雀发布或模型驻留记录迁移失败：{error}");
            config.config_load_warning = Some(match config.config_load_warning.take() {
                Some(existing) => format!("{existing}；{warning}"),
                None => warning,
            });
        }
    }
    config
}

pub(crate) fn load_global_config_for_update_unlocked(
    config_dir: &std::path::Path,
) -> Result<GlobalConfig, String> {
    let primary_path = config_dir.join("instances.json");
    let backup_path = config_dir.join("instances.json.bak");
    let primary = std::fs::read_to_string(&primary_path);
    if let Ok(contents) = &primary {
        if let Ok(mut config) = serde_json::from_str::<GlobalConfig>(contents) {
            config.model_dirs = normalize_model_dirs(config.model_dirs);
            config.engine_dirs = dedupe_path_dirs(config.engine_dirs);
            migrate_global_load_modes(&mut config);
            return Ok(config);
        }
    }

    if let Ok(contents) = std::fs::read_to_string(&backup_path) {
        let mut config = serde_json::from_str::<GlobalConfig>(&contents)
            .map_err(|error| format!("解析配置备份失败: {error}"))?;
        config.model_dirs = normalize_model_dirs(config.model_dirs);
        config.engine_dirs = dedupe_path_dirs(config.engine_dirs);
        migrate_global_load_modes(&mut config);
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
    global.model_dirs = normalize_model_dirs(
        global
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
            .collect(),
    );
    global.engine_dirs = dedupe_path_dirs(
        global
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
            .collect(),
    );

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
    global.engine_dirs = dedupe_path_dirs(snapshot.engine_dirs);
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
    for (id, mut config) in instances {
        migrate_legacy_load_mode(&mut config);
        config.restart_policy = if config
            .restart_policy
            .trim()
            .eq_ignore_ascii_case("on-failure")
        {
            "on-failure".into()
        } else {
            "never".into()
        };
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
    let mut timing = crate::operation_timing::OperationTiming::new("save_config");
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
    timing.mark("normalize");
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
        crate::config_revision::ensure_current_config_revisions(&mut global)
            .map_err(|error| AppError::new("CONFIG_REVISION_MIGRATION_FAILED", error, false))?;
        crate::deployment::ensure_deployments(&mut global)
            .map_err(|error| AppError::new("DEPLOYMENT_MIGRATION_FAILED", error, false))?;
        crate::canary::ensure_canary_catalog(&mut global)
            .map_err(|error| AppError::new("CANARY_MIGRATION_FAILED", error, false))?;
        crate::residency::ensure_residency_catalog(&mut global)
            .map_err(|error| AppError::new("RESIDENCY_MIGRATION_FAILED", error, false))?;
        let previous_instances = global.instances.clone();
        let changed_instance_ids = crate::config_revision::changed_deployment_instance_ids(
            &previous_instances,
            &instances,
        )
        .map_err(|error| AppError::new("CONFIG_REVISION_FINGERPRINT_FAILED", error, false))?;
        let canary_instances = crate::canary::active_instance_ids(&global);
        if let Some(conflict) = changed_instance_ids
            .iter()
            .find(|instance_id| canary_instances.contains(*instance_id))
        {
            return Err(AppError::new(
                "CANARY_LIFECYCLE_CONFLICT",
                "deployment configuration cannot change while the instance is bound to an unresolved canary rollout",
                true,
            )
            .with_context("instanceId", conflict.clone()));
        }
        let lifecycle_conflict = {
            let reserved_instance_ids = state.starting.lock().unwrap();
            crate::config_revision::first_reserved_deployment_change(
                &changed_instance_ids,
                &reserved_instance_ids,
            )
        };
        if let Some(conflict) = lifecycle_conflict {
            return Err(AppError::new(
                "CONFIG_REVISION_LIFECYCLE_CONFLICT",
                "deployment configuration cannot change while the instance is starting or rolling back",
                true,
            )
            .with_context("instanceId", conflict));
        }
        apply_frontend_config(&mut global, snapshot, running_snapshot, engine_names);
        crate::config_revision::record_saved_config_revisions(&mut global, &previous_instances)
            .map_err(|error| AppError::new("CONFIG_REVISION_CREATE_FAILED", error, false))?;
        crate::deployment::ensure_deployments(&mut global)
            .map_err(|error| AppError::new("DEPLOYMENT_MIGRATION_FAILED", error, false))?;
        crate::residency::ensure_residency_catalog(&mut global)
            .map_err(|error| AppError::new("RESIDENCY_MIGRATION_FAILED", error, false))?;
        persist_global_config_unlocked(&config_dir, &global).map_err(|error| {
            AppError::new("CONFIG_PERSIST_FAILED", error, true).with_context(
                "path",
                config_dir.join("instances.json").display().to_string(),
            )
        })?;
        let mut stored = state.instances.lock().unwrap();
        *stored = instances.clone();
    }
    timing.mark("persist-main");
    if crate::runtime_service::manages_instances() {
        // instances.json is the durable source of truth for this operation. The app bridge
        // coalesces rapid edits and reliably retries delivery to the runtime service, so a
        // routine parameter save does not wait for daemon discovery or startup.
        crate::runtime_service::mark_config_sync_pending();
    }
    timing.mark("queue-runtime-sync");
    timing.finish("success");
    Ok(normalized.changed)
}

pub async fn load_config(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<FrontendGlobalConfig, String> {
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
    *state.residency_draining.lock().unwrap() = crate::residency::draining_instance_ids(&global);

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
    Ok(FrontendGlobalConfig::from(&global))
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
    fn engine_names_are_not_published_when_persistence_fails() {
        let shared = Mutex::new(HashMap::from([("engine".to_string(), "Old".to_string())]));
        let next = HashMap::from([("engine".to_string(), "New".to_string())]);

        let result = publish_engine_names_after_persist(&shared, next, |_| {
            Err("disk unavailable".to_string())
        });

        assert_eq!(result, Err("disk unavailable".to_string()));
        assert_eq!(
            shared.lock().unwrap().get("engine").map(String::as_str),
            Some("Old")
        );
    }

    #[test]
    fn engine_names_publish_after_persistence_succeeds() {
        let shared = Mutex::new(HashMap::from([("engine".to_string(), "Old".to_string())]));
        let next = HashMap::from([("engine".to_string(), "New".to_string())]);

        publish_engine_names_after_persist(&shared, next, |_| Ok(())).unwrap();

        assert_eq!(
            shared.lock().unwrap().get("engine").map(String::as_str),
            Some("New")
        );
    }

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

    #[test]
    fn save_config_canonicalizes_the_manager_restart_policy() {
        let enabled = InstanceConfig {
            restart_policy: " ON-FAILURE ".into(),
            ..InstanceConfig::default()
        };
        let invalid = InstanceConfig {
            restart_policy: "always".into(),
            ..InstanceConfig::default()
        };

        let normalized = normalize_instances_for_save(HashMap::from([
            ("enabled".into(), enabled),
            ("invalid".into(), invalid),
        ]));

        assert_eq!(normalized.all["enabled"].restart_policy, "on-failure");
        assert_eq!(normalized.all["invalid"].restart_policy, "never");
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
            config_revision_schema_version: crate::config_revision::CONFIG_REVISION_SCHEMA_VERSION,
            config_revisions: HashMap::new(),
            known_good_config_revisions: HashMap::new(),
            config_revision_audit: Vec::new(),
            deployment_schema_version: crate::deployment::DEPLOYMENT_SCHEMA_VERSION,
            deployments: HashMap::new(),
            canary_schema_version: crate::canary::CANARY_SCHEMA_VERSION,
            canary_rollouts: Vec::new(),
            residency_schema_version: crate::residency::RESIDENCY_SCHEMA_VERSION,
            residency_policy: crate::residency::ResidencyPolicy::default(),
            residency_placements: Vec::new(),
            residency_audit: Vec::new(),
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
    fn legacy_config_migrates_the_residency_catalog_without_frontend_injection() {
        let dir = temp_config_dir("legacy-residency-catalog");
        let legacy = sample_config();
        let mut value = serde_json::to_value(legacy).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("residency_schema_version");
        object.remove("residency_policy");
        object.remove("residency_placements");
        object.remove("residency_audit");
        std::fs::write(
            dir.join("instances.json"),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();

        let loaded = read_config_from_disk(&dir);
        assert_eq!(
            loaded.residency_schema_version,
            crate::residency::RESIDENCY_SCHEMA_VERSION
        );
        assert_eq!(
            loaded.residency_policy,
            crate::residency::ResidencyPolicy::default()
        );
        assert!(loaded.residency_placements.is_empty());
        assert!(loaded.residency_audit.is_empty());
        let frontend = FrontendGlobalConfig::from(&loaded);
        let frontend_value = serde_json::to_value(frontend).unwrap();
        assert!(frontend_value.get("residency_policy").is_none());
        assert!(frontend_value.get("residency_placements").is_none());
        assert!(frontend_value.get("residency_audit").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn existing_instance_migrates_to_a_durable_revision_baseline() {
        let dir = temp_config_dir("revision-baseline");
        let mut config = sample_config();
        let instance = InstanceConfig {
            id: "baseline".into(),
            name: "Baseline".into(),
            port: 8123,
            ..InstanceConfig::default()
        };
        config
            .instances
            .insert(instance.id.clone(), instance.clone());
        std::fs::write(
            dir.join("instances.json"),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();

        let loaded = read_config_from_disk(&dir);
        let persisted: GlobalConfig =
            serde_json::from_str(&std::fs::read_to_string(dir.join("instances.json")).unwrap())
                .unwrap();

        assert_eq!(loaded.instances["baseline"], instance);
        assert_eq!(persisted.config_revisions["baseline"].len(), 1);
        assert_eq!(
            persisted.config_revisions["baseline"][0].fingerprint,
            crate::config_revision::deployment_config_fingerprint(&instance).unwrap()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn frontend_global_config_never_serializes_revision_snapshots() {
        let mut config = sample_config();
        let instance = InstanceConfig {
            id: "private-history".into(),
            name: "Private history".into(),
            api_key: "historical-api-key-must-not-cross-ipc".into(),
            ..InstanceConfig::default()
        };
        config.instances.insert(instance.id.clone(), instance);
        crate::config_revision::ensure_current_config_revisions(&mut config).unwrap();
        let identity = crate::deployment_identity::DeploymentIdentity::new(
            "urn:lsm:engine:v1:sha256:test".into(),
            "urn:lsm:model:v1:sha256:test".into(),
            "revision-test".into(),
            "urn:lsm:configuration:v1:sha256:test".into(),
            "urn:lsm:qualification:v2:sha256:test".into(),
        )
        .unwrap();
        crate::deployment::materialize_revision(&mut config, "private-history", &identity).unwrap();
        config
            .instances
            .get_mut("private-history")
            .unwrap()
            .api_key
            .clear();

        let public = FrontendGlobalConfig::from(&config);
        let json = serde_json::to_string(&public).unwrap();

        assert!(!json.contains("config_revisions"));
        assert!(!json.contains("known_good_config_revisions"));
        assert!(!json.contains("config_revision_audit"));
        assert!(!json.contains("deployment_schema_version"));
        assert!(!json.contains("managed-deployment"));
        assert!(!json.contains("historical-api-key-must-not-cross-ipc"));
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
    fn backup_recovery_preserves_revision_history_and_known_good_pointer() {
        let dir = temp_config_dir("revision-backup-fallback");
        let mut expected = sample_config();
        let baseline = InstanceConfig {
            id: "revision-backup".into(),
            name: "Revision backup".into(),
            port: 8111,
            ..InstanceConfig::default()
        };
        expected
            .instances
            .insert(baseline.id.clone(), baseline.clone());
        crate::config_revision::ensure_current_config_revisions(&mut expected).unwrap();
        let baseline_revision_id = expected.config_revisions["revision-backup"][0].id.clone();
        expected
            .known_good_config_revisions
            .insert("revision-backup".into(), baseline_revision_id.clone());
        let previous = expected.instances.clone();
        expected.instances.get_mut("revision-backup").unwrap().port = 8222;
        crate::config_revision::record_saved_config_revisions(&mut expected, &previous).unwrap();
        std::fs::write(dir.join("instances.json"), "{not-json").unwrap();
        std::fs::write(
            dir.join("instances.json.bak"),
            serde_json::to_string_pretty(&expected).unwrap(),
        )
        .unwrap();

        let loaded = read_config_from_disk(&dir);

        assert_eq!(loaded.instances["revision-backup"].port, 8222);
        assert_eq!(loaded.config_revisions["revision-backup"].len(), 2);
        assert_eq!(
            loaded
                .known_good_config_revisions
                .get("revision-backup")
                .map(String::as_str),
            Some(baseline_revision_id.as_str())
        );
        assert!(loaded
            .config_load_warning
            .as_deref()
            .is_some_and(|warning| warning.contains("已从备份恢复")));
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

    #[cfg(windows)]
    #[test]
    fn configured_scan_roots_deduplicate_windows_path_aliases() {
        let directories = dedupe_path_dirs(vec![
            r"C:\Models".into(),
            r"\\?\c:\models\".into(),
            r"\\Server\Share\Models".into(),
            r"\\?\UNC\server\share\models".into(),
        ]);

        assert_eq!(directories, vec![r"C:\Models", r"\\Server\Share\Models"]);
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
    ) -> crate::error::AppResult<FrontendGlobalConfig> {
        super::load_config(state, app)
            .await
            .map_err(crate::error::AppError::from)
    }
}
