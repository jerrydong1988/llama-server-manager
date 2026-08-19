use super::protocol::{
    InstanceRecoveryPhase, InstanceRecoveryStatus, PersistedRuntimeState, RuntimeCommand,
    RuntimeFailure, RuntimeFailureKind, RuntimeLaunchSpec, RuntimeReply, RuntimeServiceStatus,
    BACKGROUND_DETACH_CAPABILITY, CONFIG_SYNC_ACK_CAPABILITY, DEPLOYMENT_REVISION_CAPABILITY,
    INSTANCE_RECOVERY_BACKOFF_SECS, INSTANCE_RECOVERY_CAPABILITY, INSTANCE_RECOVERY_MAX_ATTEMPTS,
    INSTANCE_RECOVERY_STABLE_SECS, RUNTIME_ERROR_ACK_CAPABILITY, RUNTIME_PROTOCOL_VERSION,
    RUNTIME_STATE_SCHEMA_VERSION,
};
use super::transport::runtime_state_path;
use crate::commands::engine_capabilities::{executable_fingerprint, QUALIFICATION_PROFILE_VERSION};
use crate::commands::proxy::{
    normalize_and_validate_proxy_config, proxy_request_resolution_from,
    proxy_router_from_source_with_runtime, status_with_runtime, ProxyDataSource,
    ProxyRequestResolution, ProxyRuntimeSnapshot,
};
use crate::commands::proxy_runtime::RouterRuntime;
use crate::commands::server::{
    advance_health_state, collect_instance_monitor_sample, effective_api_key,
    effective_server_scheme, read_process_identity, running_instance_matches_live_process,
    spawn_runtime_log_pump, telemetry_config_hash, terminate_running_instance, CappedLogWriter,
    HealthTransition, RuntimePerfTracker, INITIAL_HEALTH_GRACE, MAX_SERVER_LOG_BYTES,
    RETAINED_SERVER_LOG_BYTES,
};
use crate::models::{ProxyStatus, RunningInstance};
use crate::vector_policy::ModelWorkload;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const GUI_HEARTBEAT_TIMEOUT_MS: u64 = 20_000;

fn sorted_ids<'a>(ids: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut ids = ids.cloned().collect::<Vec<_>>();
    ids.sort();
    ids
}

fn is_instance_recovery_error(error: &str, instance_id: &str) -> bool {
    error.starts_with(&format!(
        "instance {instance_id} exited unexpectedly (code "
    )) || error.starts_with(&format!("instance {instance_id} failed to start: "))
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn validate_runtime_engine_qualification(spec: &RuntimeLaunchSpec) -> Result<(), String> {
    if spec.engine_qualification_fingerprint.is_empty()
        || spec.engine_qualification_profile_version == 0
    {
        return Err(
            "ENGINE_QUALIFICATION_REQUIRED: runtime launch snapshot has no qualification binding"
                .to_string(),
        );
    }
    if spec.engine_qualification_profile_version != QUALIFICATION_PROFILE_VERSION {
        return Err(format!(
            "ENGINE_QUALIFICATION_INCOMPLETE: runtime launch snapshot uses qualification profile {} but profile {} is required",
            spec.engine_qualification_profile_version, QUALIFICATION_PROFILE_VERSION
        ));
    }
    let executable = spec.command.first().map(String::as_str).unwrap_or_default();
    if executable_fingerprint(executable) != spec.engine_qualification_fingerprint {
        return Err(
            "ENGINE_QUALIFICATION_STALE: runtime engine artifact no longer matches qualification evidence"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_runtime_deployment_identity(spec: &RuntimeLaunchSpec) -> Result<(), String> {
    if !spec.deployment_identity.is_valid() {
        return Err(
            "DEPLOYMENT_IDENTITY_INVALID: runtime launch snapshot has no valid deployment identity"
                .to_string(),
        );
    }
    let executable = spec.command.first().map(String::as_str).unwrap_or_default();
    let engine = crate::deployment_identity::artifact_identity_for_path(
        "engine",
        std::path::Path::new(executable),
    )
    .map_err(|error| format!("DEPLOYMENT_ENGINE_IDENTITY_FAILED: {error}"))?;
    if engine.artifact_id != spec.deployment_identity.engine_artifact_id {
        return Err(
            "DEPLOYMENT_ENGINE_IDENTITY_STALE: runtime engine artifact identity changed"
                .to_string(),
        );
    }
    let model = crate::deployment_identity::artifact_identity_for_path(
        "model",
        std::path::Path::new(&spec.config.model_path),
    )
    .map_err(|error| format!("DEPLOYMENT_MODEL_IDENTITY_FAILED: {error}"))?;
    if model.artifact_id != spec.deployment_identity.model_artifact_id {
        return Err(
            "DEPLOYMENT_MODEL_IDENTITY_STALE: runtime model artifact identity changed".to_string(),
        );
    }
    let fingerprint = crate::config_revision::deployment_config_fingerprint(&spec.config)
        .map_err(|error| format!("DEPLOYMENT_CONFIG_IDENTITY_FAILED: {error}"))?;
    let configuration_id = crate::config_revision::configuration_id_from_fingerprint(&fingerprint)
        .map_err(|error| format!("DEPLOYMENT_CONFIG_IDENTITY_FAILED: {error}"))?;
    if configuration_id != spec.deployment_identity.configuration_id {
        return Err(format!(
            "DEPLOYMENT_CONFIG_IDENTITY_STALE: runtime configuration identity changed (expected {}, found {})",
            spec.deployment_identity.configuration_id, configuration_id
        ));
    }
    Ok(())
}

fn validate_runtime_deployment_revision(
    spec: &RuntimeLaunchSpec,
    proxy_config: &crate::models::ProxyConfig,
) -> Result<(), String> {
    crate::deployment::validate_runtime_revision(
        &spec.deployment_revision,
        &spec.instance_id,
        &spec.config,
        &spec.deployment_identity,
        proxy_config,
    )
}

fn instance_recovery_enabled(spec: &RuntimeLaunchSpec) -> bool {
    !spec.launch_config_stale
        && spec
            .config
            .restart_policy
            .trim()
            .eq_ignore_ascii_case("on-failure")
}

fn runtime_launch_config_matches(
    launch_config: &crate::models::InstanceConfig,
    current_config: &crate::models::InstanceConfig,
) -> bool {
    let mut comparable = launch_config.clone();
    comparable.id = current_config.id.clone();
    comparable.name = current_config.name.clone();
    comparable == *current_config
}

fn sync_desired_launch_config(
    spec: &mut RuntimeLaunchSpec,
    current_config: &crate::models::InstanceConfig,
    proxy_config: &crate::models::ProxyConfig,
) {
    spec.launch_config_stale = !runtime_launch_config_matches(&spec.config, current_config)
        || validate_runtime_deployment_revision(spec, proxy_config).is_err();
}

fn next_recovery_delay(restart_attempts: u32) -> Option<u64> {
    INSTANCE_RECOVERY_BACKOFF_SECS
        .get(restart_attempts as usize)
        .copied()
}

fn recovery_status_after_failure(
    recovery_enabled: bool,
    active: Option<&InstanceRecoveryStatus>,
    failure: RuntimeFailure,
    now: u64,
) -> InstanceRecoveryStatus {
    let active = active.filter(|status| {
        matches!(
            status.phase,
            InstanceRecoveryPhase::Waiting
                | InstanceRecoveryPhase::Monitoring
                | InstanceRecoveryPhase::Restoring
        )
    });
    let restart_attempts = active.map(|status| status.restart_attempts).unwrap_or(0);
    let origin_failure = active
        .map(|status| status.origin_failure.clone())
        .unwrap_or_else(|| failure.clone());
    let next_retry_at = recovery_enabled
        .then(|| next_recovery_delay(restart_attempts))
        .flatten()
        .map(|delay| now.saturating_add(delay));
    let phase = if !recovery_enabled {
        InstanceRecoveryPhase::Failed
    } else if next_retry_at.is_some() {
        InstanceRecoveryPhase::Waiting
    } else {
        InstanceRecoveryPhase::CrashLoop
    };
    InstanceRecoveryStatus {
        phase,
        restart_attempts,
        max_restart_attempts: INSTANCE_RECOVERY_MAX_ATTEMPTS,
        next_retry_at,
        origin_failure,
        last_failure: failure,
    }
}

fn recovery_budget_is_stable(start_time: u64, now: u64) -> bool {
    start_time > 0 && now.saturating_sub(start_time) >= INSTANCE_RECOVERY_STABLE_SECS
}

fn scheduled_recovery_matches(
    status: &InstanceRecoveryStatus,
    expected_restart_attempts: u32,
    expected_retry_at: u64,
) -> bool {
    status.phase == InstanceRecoveryPhase::Waiting
        && status.restart_attempts == expected_restart_attempts
        && status.next_retry_at == Some(expected_retry_at)
}

fn validate_background_detach_inventory(
    expected: &HashMap<String, RunningInstance>,
    actual: &HashMap<String, RunningInstance>,
    desired: &HashMap<String, RuntimeLaunchSpec>,
) -> Result<(), String> {
    let expected_ids = sorted_ids(expected.keys());
    let actual_ids = sorted_ids(actual.keys());
    if expected_ids != actual_ids {
        return Err(format!(
            "后台接管前实例清单不一致：界面 [{}]，后台 [{}]",
            expected_ids.join(", "),
            actual_ids.join(", ")
        ));
    }
    let desired_ids = sorted_ids(desired.keys());
    if actual_ids != desired_ids {
        return Err(format!(
            "后台恢复清单与运行实例不一致：运行 [{}]，恢复 [{}]",
            actual_ids.join(", "),
            desired_ids.join(", ")
        ));
    }
    for (instance_id, expected_instance) in expected {
        let actual_instance = actual
            .get(instance_id)
            .ok_or_else(|| format!("后台未接管实例 {instance_id}"))?;
        if actual_instance.pid != expected_instance.pid
            || actual_instance.start_time != expected_instance.start_time
            || actual_instance.executable_path != expected_instance.executable_path
            || actual_instance.host != expected_instance.host
            || actual_instance.port != expected_instance.port
        {
            return Err(format!(
                "后台实例 {instance_id} 的进程身份或监听端点已变化，请等待状态刷新后重试"
            ));
        }
    }
    Ok(())
}

fn validate_runtime_state(
    mut state: PersistedRuntimeState,
) -> Result<PersistedRuntimeState, String> {
    match state.schema_version {
        RUNTIME_STATE_SCHEMA_VERSION => Ok(state),
        1..=3 => {
            state.schema_version = RUNTIME_STATE_SCHEMA_VERSION;
            Ok(state)
        }
        found => Err(format!(
            "unsupported runtime state schema: expected {} or migratable schema 1, 2, or 3, found {found}",
            RUNTIME_STATE_SCHEMA_VERSION
        )),
    }
}

enum RuntimeStateReadError {
    Missing,
    Unsupported(String),
    Invalid(String),
}

impl std::fmt::Display for RuntimeStateReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => formatter.write_str("file does not exist"),
            Self::Unsupported(error) | Self::Invalid(error) => formatter.write_str(error),
        }
    }
}

fn parse_runtime_state(
    path: &std::path::Path,
) -> Result<PersistedRuntimeState, RuntimeStateReadError> {
    let json = std::fs::read_to_string(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            RuntimeStateReadError::Missing
        } else {
            RuntimeStateReadError::Invalid(format!("failed to read {}: {error}", path.display()))
        }
    })?;
    let state = serde_json::from_str(&json).map_err(|error| {
        RuntimeStateReadError::Invalid(format!("failed to parse {}: {error}", path.display()))
    })?;
    validate_runtime_state(state).map_err(RuntimeStateReadError::Unsupported)
}

fn load_persisted_state() -> Result<PersistedRuntimeState, String> {
    let path = runtime_state_path();
    let backup = path.with_extension("json.bak");
    match parse_runtime_state(&path) {
        Ok(state) => Ok(state),
        Err(RuntimeStateReadError::Missing) if !backup.exists() => Ok(PersistedRuntimeState::default()),
        Err(RuntimeStateReadError::Unsupported(error)) => Err(error),
        Err(primary_error) => match parse_runtime_state(&backup) {
            Ok(state) => {
                let json = serde_json::to_vec_pretty(&state)
                    .map_err(|error| format!("failed to serialize recovered runtime state: {error}"))?;
                crate::persistence::atomic_write(&path, &json, None).map_err(|error| {
                    format!(
                        "recovered runtime state from {}, but failed to restore {}: {error}",
                        backup.display(),
                        path.display()
                    )
                })?;
                protect_runtime_state_file(&path)?;
                Ok(state)
            }
            Err(backup_error) => Err(format!(
                "runtime state is unavailable; primary error: {primary_error}; backup error: {backup_error}"
            )),
        },
    }
}

fn protect_runtime_state_file(path: &std::path::Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("failed to protect runtime state: {error}"))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn persist_state(state: &PersistedRuntimeState) -> Result<(), String> {
    let path = runtime_state_path();
    let json = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("failed to serialize runtime state: {error}"))?;
    crate::persistence::atomic_write(&path, &json, Some(&path.with_extension("json.bak")))?;
    protect_runtime_state_file(&path)
}

pub struct RuntimeSupervisor {
    state: Mutex<PersistedRuntimeState>,
    proxy_status: Mutex<ProxyStatus>,
    proxy_runtime: tokio::sync::Mutex<Option<RuntimeProxy>>,
    proxy_router_runtime: Mutex<Option<Arc<RouterRuntime>>>,
    health: Mutex<HashMap<String, String>>,
    perf_trackers: Mutex<HashMap<String, Arc<Mutex<RuntimePerfTracker>>>>,
    last_error: Mutex<Option<String>>,
    stop_intents: Mutex<HashMap<String, StopIntent>>,
    instance_lifecycle: Mutex<()>,
    gui_owner: Mutex<Option<GuiOwner>>,
    last_gui_heartbeat: Mutex<std::time::Instant>,
}

#[derive(Clone)]
struct GuiOwner {
    pid: u32,
    start_time: u64,
    executable_path: std::path::PathBuf,
}

fn gui_owner_is_alive(owner: &GuiOwner) -> bool {
    read_process_identity(owner.pid).is_some_and(|(start_time, executable_path)| {
        start_time == owner.start_time && executable_path == owner.executable_path
    })
}

fn runtime_config_matches(
    state: &PersistedRuntimeState,
    proxy_config: &crate::models::ProxyConfig,
    instances: &HashMap<String, crate::models::InstanceConfig>,
) -> bool {
    state.proxy_config == *proxy_config && state.instances == *instances
}

struct RuntimeProxy {
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
struct StopIntent {
    preserve_desired: bool,
    telemetry_reason: String,
}

impl RuntimeSupervisor {
    pub fn load() -> Result<Arc<Self>, String> {
        let state = load_persisted_state()?;
        let bound_addr =
            crate::utils::format_host_port(state.proxy_config.host.trim(), state.proxy_config.port);
        let active_routes = state
            .proxy_config
            .routes
            .iter()
            .filter(|route| route.enabled)
            .count();
        let supervisor = Arc::new(Self {
            state: Mutex::new(state),
            proxy_status: Mutex::new(ProxyStatus {
                running: false,
                bound_addr,
                active_routes,
                healthy_routes: 0,
                unhealthy_routes: active_routes,
                in_flight_requests: 0,
                total_requests: 0,
                last_error: None,
            }),
            proxy_runtime: tokio::sync::Mutex::new(None),
            proxy_router_runtime: Mutex::new(None),
            health: Mutex::new(HashMap::new()),
            perf_trackers: Mutex::new(HashMap::new()),
            last_error: Mutex::new(None),
            stop_intents: Mutex::new(HashMap::new()),
            instance_lifecycle: Mutex::new(()),
            gui_owner: Mutex::new(None),
            last_gui_heartbeat: Mutex::new(std::time::Instant::now()),
        });
        Ok(supervisor)
    }

    pub fn status(&self, registered_for_login: bool) -> RuntimeServiceStatus {
        let (running, recovery, background_enabled, config_revision) = {
            let state = self.state.lock().unwrap();
            (
                state.running.clone(),
                state.recovery.clone(),
                state.background_enabled,
                state.config_revision,
            )
        };
        let monitoring = running
            .keys()
            .filter_map(|instance_id| {
                crate::commands::monitoring::capture_frame(instance_id)
                    .map(|frame| (instance_id.clone(), frame))
            })
            .collect();
        let performance = self
            .perf_trackers
            .lock()
            .unwrap()
            .iter()
            .map(|(instance_id, tracker)| (instance_id.clone(), tracker.lock().unwrap().snapshot()))
            .collect();
        let proxy = self
            .proxy_router_runtime
            .lock()
            .unwrap()
            .clone()
            .map(|runtime| status_with_runtime(&self.proxy_snapshot(), &runtime))
            .unwrap_or_else(|| self.proxy_status.lock().unwrap().clone());
        RuntimeServiceStatus {
            protocol_version: RUNTIME_PROTOCOL_VERSION,
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            service_pid: std::process::id(),
            capabilities: vec![
                BACKGROUND_DETACH_CAPABILITY.to_string(),
                CONFIG_SYNC_ACK_CAPABILITY.to_string(),
                DEPLOYMENT_REVISION_CAPABILITY.to_string(),
                INSTANCE_RECOVERY_CAPABILITY.to_string(),
                RUNTIME_ERROR_ACK_CAPABILITY.to_string(),
            ],
            config_revision,
            background_enabled,
            registered_for_login,
            last_error: self.last_error.lock().unwrap().clone(),
            proxy,
            running,
            health: self.health.lock().unwrap().clone(),
            monitoring,
            performance,
            recovery,
        }
    }

    pub fn heartbeat(&self, gui_pid: u32) -> Result<(), String> {
        let (start_time, executable_path) = read_process_identity(gui_pid)
            .ok_or_else(|| format!("unable to verify GUI process identity for PID {gui_pid}"))?;
        *self.gui_owner.lock().unwrap() = Some(GuiOwner {
            pid: gui_pid,
            start_time,
            executable_path,
        });
        *self.last_gui_heartbeat.lock().unwrap() = std::time::Instant::now();
        Ok(())
    }

    pub fn background_enabled(&self) -> bool {
        self.state.lock().unwrap().background_enabled
    }

    pub fn heartbeat_expired(&self) -> bool {
        let stale = self.last_gui_heartbeat.lock().unwrap().elapsed()
            > std::time::Duration::from_millis(GUI_HEARTBEAT_TIMEOUT_MS);
        if !stale {
            return false;
        }
        let owner = self.gui_owner.lock().unwrap().clone();
        !owner.as_ref().is_some_and(gui_owner_is_alive)
    }

    fn persist(&self) -> Result<(), String> {
        persist_state(&self.state.lock().unwrap())
    }

    pub async fn sync_config(
        &self,
        revision: u64,
        proxy_config: crate::models::ProxyConfig,
        instances: HashMap<String, crate::models::InstanceConfig>,
    ) -> Result<(), String> {
        let proxy_config = normalize_and_validate_proxy_config(proxy_config, &instances)?;
        let _proxy_transition = self.proxy_runtime.lock().await;
        let _instance_transition = self.instance_lifecycle.lock().unwrap();
        {
            let state = self.state.lock().unwrap();
            if runtime_config_matches(&state, &proxy_config, &instances) {
                return Ok(());
            }
        }
        let requested_addr =
            crate::utils::format_host_port(proxy_config.host.trim(), proxy_config.port);
        {
            let proxy_status = self.proxy_status.lock().unwrap();
            if proxy_status.running && proxy_status.bound_addr != requested_addr {
                return Err(format!(
                    "代理正在监听 {}；修改监听地址或端口前请先停止路由服务",
                    proxy_status.bound_addr
                ));
            }
        }
        let previous_config = {
            let mut state = self.state.lock().unwrap();
            if revision <= state.config_revision {
                return Err(format!(
                    "stale runtime configuration revision: current {}, received {}",
                    state.config_revision, revision
                ));
            }
            let previous = (
                state.config_revision,
                state.proxy_config.clone(),
                state.instances.clone(),
                state.desired_instances.clone(),
                state.recovery.clone(),
            );
            state.config_revision = revision;
            state.proxy_config = proxy_config.clone();
            state.instances = instances;
            let current_configs = state.instances.clone();
            let current_proxy = state.proxy_config.clone();
            for (instance_id, spec) in &mut state.desired_instances {
                if let Some(config) = current_configs.get(instance_id) {
                    sync_desired_launch_config(spec, config, &current_proxy);
                }
            }
            let disabled = state
                .desired_instances
                .iter()
                .filter_map(|(instance_id, spec)| {
                    (!instance_recovery_enabled(spec)).then_some(instance_id.clone())
                })
                .collect::<Vec<_>>();
            for instance_id in disabled {
                if let Some(recovery) = state.recovery.get_mut(&instance_id) {
                    if recovery.phase == InstanceRecoveryPhase::Waiting {
                        recovery.phase = InstanceRecoveryPhase::Failed;
                        recovery.next_retry_at = None;
                    }
                }
            }
            previous
        };
        let previous_proxy_status = {
            let mut proxy_status = self.proxy_status.lock().unwrap();
            let previous = proxy_status.clone();
            proxy_status.active_routes = proxy_config
                .routes
                .iter()
                .filter(|route| route.enabled)
                .count();
            if !proxy_status.running {
                proxy_status.bound_addr =
                    crate::utils::format_host_port(proxy_config.host.trim(), proxy_config.port);
            }
            previous
        };
        if let Err(error) = self.persist() {
            let mut state = self.state.lock().unwrap();
            state.config_revision = previous_config.0;
            state.proxy_config = previous_config.1;
            state.instances = previous_config.2;
            state.desired_instances = previous_config.3;
            state.recovery = previous_config.4;
            drop(state);
            *self.proxy_status.lock().unwrap() = previous_proxy_status;
            return Err(error);
        }
        Ok(())
    }

    pub fn set_background_enabled(&self, enabled: bool) -> Result<(), String> {
        let previous = {
            let mut state = self.state.lock().unwrap();
            let previous = state.background_enabled;
            if previous == enabled {
                return Ok(());
            }
            state.background_enabled = enabled;
            previous
        };
        if let Err(error) = self.persist() {
            self.state.lock().unwrap().background_enabled = previous;
            return Err(error);
        }
        Ok(())
    }

    fn validate_background_detach(
        &self,
        expected_running: &HashMap<String, RunningInstance>,
    ) -> Result<(), String> {
        let (actual, desired) = {
            let state = self.state.lock().unwrap();
            (state.running.clone(), state.desired_instances.clone())
        };
        validate_background_detach_inventory(expected_running, &actual, &desired)?;
        for (instance_id, running) in &actual {
            if !running_instance_matches_live_process(running) {
                return Err(format!(
                    "后台实例 {instance_id} (PID {}) 未通过进程身份校验",
                    running.pid
                ));
            }
        }
        Ok(())
    }

    pub async fn prepare_background_detach(
        self: &Arc<Self>,
        revision: u64,
        proxy_config: crate::models::ProxyConfig,
        instances: HashMap<String, crate::models::InstanceConfig>,
        expected_running: HashMap<String, RunningInstance>,
        registered_for_login: bool,
    ) -> Result<RuntimeServiceStatus, String> {
        if !registered_for_login {
            return Err("后台运行时尚未注册当前用户登录自启动".into());
        }
        let was_background_enabled = self.background_enabled();
        self.sync_config(revision, proxy_config.clone(), instances)
            .await?;
        self.validate_background_detach(&expected_running)?;

        let proxy_status = self.proxy_status.lock().unwrap().clone();
        if proxy_config.enabled && !proxy_status.running {
            self.start_proxy().await?;
        } else if !proxy_config.enabled && proxy_status.running {
            self.stop_proxy().await?;
        }

        let expected_addr =
            crate::utils::format_host_port(proxy_config.host.trim(), proxy_config.port);
        let proxy_status = self.proxy_status.lock().unwrap().clone();
        if proxy_status.running != proxy_config.enabled {
            return Err("统一 API 路由未达到保存配置要求的运行状态".into());
        }
        if proxy_config.enabled && proxy_status.bound_addr != expected_addr {
            return Err(format!(
                "统一 API 路由监听地址不一致：期望 {expected_addr}，实际 {}",
                proxy_status.bound_addr
            ));
        }

        self.set_background_enabled(true)?;
        let verification = self.validate_background_detach(&expected_running);
        if let Err(error) = verification {
            if !was_background_enabled {
                let _ = self.set_background_enabled(false);
            }
            return Err(error);
        }
        let status = self.status(registered_for_login);
        if !status.background_enabled || !status.registered_for_login {
            if !was_background_enabled {
                let _ = self.set_background_enabled(false);
            }
            return Err("后台接管回执未确认持久化与登录恢复状态".into());
        }
        Ok(status)
    }

    fn spawn_instance_monitor(
        self: &Arc<Self>,
        running: RunningInstance,
        config: crate::models::InstanceConfig,
    ) -> Result<(), String> {
        let instance_id = running.instance_id.clone();
        let expected_pid = running.pid;
        let telemetry_session_id = running.telemetry_session_id.clone();
        let workload = ModelWorkload::from_storage(&running.workload);
        let endpoint_host = if matches!(config.host.as_str(), "0.0.0.0" | "::") {
            "localhost"
        } else {
            config.host.as_str()
        };
        let endpoint_base = crate::utils::service_url(
            effective_server_scheme(&config),
            endpoint_host,
            config.port,
            &config.api_prefix,
            "",
        );
        let api_key = effective_api_key(&config);
        self.health
            .lock()
            .unwrap()
            .insert(instance_id.clone(), "pending".into());
        let supervisor = Arc::downgrade(self);
        std::thread::Builder::new()
            .name(format!("runtime-metrics-{instance_id}"))
            .spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(3));
                let client = reqwest::blocking::Client::new();
                let mut process_system = sysinfo::System::new_all();
                let started = std::time::Instant::now();
                let initial_uptime = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs().saturating_sub(running.start_time))
                    .unwrap_or(0);
                let mut health_failures = 0_u32;
                let mut last_health_ready = None;
                loop {
                    let iteration_started = std::time::Instant::now();
                    let Some(supervisor) = supervisor.upgrade() else {
                        break;
                    };
                    let is_current = supervisor
                        .state
                        .lock()
                        .unwrap()
                        .running
                        .get(&instance_id)
                        .is_some_and(|current| current.pid == expected_pid);
                    if !is_current || !running_instance_matches_live_process(&running) {
                        break;
                    }
                    supervisor.clear_stable_instance_recovery(&instance_id, expected_pid);

                    let sample = collect_instance_monitor_sample(
                        &client,
                        &endpoint_base,
                        &api_key,
                        &mut process_system,
                        expected_pid,
                        initial_uptime.saturating_add(started.elapsed().as_secs()),
                    );
                    match advance_health_state(
                        sample.ready,
                        started.elapsed() >= INITIAL_HEALTH_GRACE,
                        &mut health_failures,
                        &mut last_health_ready,
                    ) {
                        HealthTransition::Ready => {
                            supervisor
                                .health
                                .lock()
                                .unwrap()
                                .insert(instance_id.clone(), "ok".into());
                        }
                        HealthTransition::Failed => {
                            supervisor
                                .health
                                .lock()
                                .unwrap()
                                .insert(instance_id.clone(), "fail".into());
                        }
                        HealthTransition::None => {}
                    }

                    let _ = crate::commands::telemetry::record_metric_sample(
                        telemetry_session_id.as_deref(),
                        &instance_id,
                        &sample.system,
                        sample.llama.as_ref(),
                    );
                    crate::commands::monitoring::update_metrics(
                        &instance_id,
                        telemetry_session_id.as_deref(),
                        workload,
                        sample.system,
                        sample.llama,
                    );
                    if let Some(slots) = sample.slots {
                        crate::commands::monitoring::update_slots(
                            &instance_id,
                            telemetry_session_id.as_deref(),
                            workload,
                            slots.len() as u64,
                            slots.iter().filter(|slot| slot.is_processing).count() as u64,
                        );
                        let _ = crate::commands::telemetry::record_slot_snapshots(
                            telemetry_session_id.as_deref(),
                            &instance_id,
                            &slots,
                        );
                    }
                    let _ = crate::commands::monitoring::capture_frame(&instance_id);
                    std::thread::sleep(
                        std::time::Duration::from_secs(5)
                            .saturating_sub(iteration_started.elapsed()),
                    );
                }
            })
            .map(|_| ())
            .map_err(|error| format!("failed to start runtime metrics monitor: {error}"))
    }

    pub fn start_instance(
        self: &Arc<Self>,
        spec: RuntimeLaunchSpec,
        manual_recovery: bool,
    ) -> Result<RunningInstance, String> {
        let _lifecycle = self.instance_lifecycle.lock().unwrap();
        if self
            .state
            .lock()
            .unwrap()
            .running
            .get(&spec.instance_id)
            .is_some_and(running_instance_matches_live_process)
        {
            return Err("该实例已在运行中".into());
        }
        if !manual_recovery
            && self
                .state
                .lock()
                .unwrap()
                .recovery
                .contains_key(&spec.instance_id)
        {
            return Err(format!(
                "automatic start skipped for instance {} because a recovery incident requires operator action or its scheduled retry",
                spec.instance_id
            ));
        }
        if manual_recovery {
            let mut state = self.state.lock().unwrap();
            if let Some(recovery) = state.recovery.get_mut(&spec.instance_id) {
                recovery.phase = InstanceRecoveryPhase::Monitoring;
                recovery.restart_attempts = 0;
                recovery.next_retry_at = None;
            }
            state.desired_instances.remove(&spec.instance_id);
        }
        match self.start_instance_locked(spec.clone(), manual_recovery) {
            Ok(running) => Ok(running),
            Err(error) => {
                self.record_instance_failure_locked(
                    spec,
                    RuntimeFailureKind::StartupFailure,
                    error.clone(),
                    None,
                );
                Err(error)
            }
        }
    }

    fn start_instance_locked(
        self: &Arc<Self>,
        spec: RuntimeLaunchSpec,
        clear_runtime_error: bool,
    ) -> Result<RunningInstance, String> {
        crate::commands::server::validate_instance_id(&spec.instance_id)
            .map_err(|error| error.to_string())?;
        crate::commands::server::validate_public_bind_auth(&spec.config)
            .map_err(|error| error.to_string())?;
        if spec.command.is_empty() || spec.command[0].trim().is_empty() {
            return Err("runtime launch command is empty".into());
        }
        validate_runtime_engine_qualification(&spec)?;
        validate_runtime_deployment_identity(&spec)?;
        let proxy_config = self.state.lock().unwrap().proxy_config.clone();
        validate_runtime_deployment_revision(&spec, &proxy_config)?;
        crate::commands::server::validate_effective_launch_security(&spec.config, &spec.command)
            .map_err(|error| error.to_string())?;
        {
            let state = self.state.lock().unwrap();
            if state
                .running
                .get(&spec.instance_id)
                .is_some_and(running_instance_matches_live_process)
            {
                return Err("该实例已在运行中".into());
            }
        }
        if clear_runtime_error {
            self.clear_retried_instance_error(&spec.instance_id);
        }

        let log_dir = crate::utils::get_data_dir().join("configs").join("logs");
        std::fs::create_dir_all(&log_dir).map_err(|error| format!("无法创建日志目录: {error}"))?;
        let log_path = log_dir.join(format!("{}.log", spec.instance_id));
        let log_writer = Arc::new(
            CappedLogWriter::new(log_path, MAX_SERVER_LOG_BYTES, RETAINED_SERVER_LOG_BYTES)
                .map_err(|error| format!("无法创建日志文件: {error}"))?,
        );

        let mut command = Command::new(&spec.command[0]);
        command
            .args(&spec.command[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(working_directory) = spec
            .working_directory
            .as_deref()
            .filter(|directory| !directory.trim().is_empty())
        {
            command.current_dir(working_directory);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("启动服务器失败: {error}\n命令: {}", spec.command_display))?;
        let pid = child.id();
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Unable to capture server stdout".into());
        };
        let Some(stderr) = child.stderr.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Unable to capture server stderr".into());
        };
        let (start_time, executable_path) = match read_process_identity(pid) {
            Some(identity) => identity,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Unable to verify the started server process identity".into());
            }
        };
        let workload = ModelWorkload::from_storage(&spec.workload);
        let telemetry_session_id = crate::commands::telemetry::begin_run_session(
            &spec.instance_id,
            &spec.config,
            &spec.engine_backend,
            &telemetry_config_hash(&spec.config),
            &spec.command_display,
            workload,
        )
        .ok();
        let running = RunningInstance {
            instance_id: spec.instance_id.clone(),
            pid,
            port: spec.config.port,
            host: spec.config.host.clone(),
            start_time,
            executable_path: executable_path.to_string_lossy().to_string(),
            telemetry_session_id,
            workload: spec.workload.clone(),
            launch_config: Some(spec.config.clone()),
            deployment_identity: spec.deployment_identity.clone(),
            deployment_id: spec.deployment_revision.deployment_id.clone(),
            deployment_revision_id: spec.deployment_revision.id.clone(),
        };
        let perf_tracker = Arc::new(Mutex::new(RuntimePerfTracker::new(
            spec.instance_id.clone(),
            running.telemetry_session_id.clone(),
            workload,
        )));
        let stdout_pump = spawn_runtime_log_pump(stdout, log_writer.clone(), perf_tracker.clone());
        let stderr_pump = spawn_runtime_log_pump(stderr, log_writer, perf_tracker.clone());
        self.perf_trackers
            .lock()
            .unwrap()
            .insert(spec.instance_id.clone(), perf_tracker.clone());
        {
            let mut state = self.state.lock().unwrap();
            state
                .desired_instances
                .insert(spec.instance_id.clone(), spec.clone());
            state
                .instances
                .insert(spec.instance_id.clone(), spec.config.clone());
            state
                .running
                .insert(spec.instance_id.clone(), running.clone());
        }
        if let Err(error) = self.persist() {
            let _ = terminate_running_instance(&running);
            let _ = child.wait();
            let _ = stdout_pump.join();
            let _ = stderr_pump.join();
            perf_tracker.lock().unwrap().finish();
            let mut state = self.state.lock().unwrap();
            state.running.remove(&spec.instance_id);
            state.desired_instances.remove(&spec.instance_id);
            drop(state);
            self.perf_trackers.lock().unwrap().remove(&spec.instance_id);
            crate::commands::monitoring::remove_instance(&spec.instance_id);
            let _ = crate::commands::telemetry::finish_run_session(
                running.telemetry_session_id.as_deref(),
                None,
                "runtime-state-persist-failed",
            );
            return Err(format!(
                "Server start was rolled back because runtime state could not be persisted: {error}"
            ));
        }

        let supervisor = Arc::downgrade(self);
        let instance_id = spec.instance_id.clone();
        let process_monitor = std::thread::Builder::new()
            .name(format!("runtime-instance-{instance_id}"))
            .spawn(move || {
                let exit_code = child.wait().ok().and_then(|status| status.code());
                let _ = stdout_pump.join();
                let _ = stderr_pump.join();
                perf_tracker.lock().unwrap().finish();
                if let Some(supervisor) = supervisor.upgrade() {
                    supervisor.record_process_exit(&instance_id, pid, exit_code);
                }
            });
        if let Err(error) = process_monitor {
            let _ = terminate_running_instance(&running);
            let mut state = self.state.lock().unwrap();
            state.running.remove(&spec.instance_id);
            state.desired_instances.remove(&spec.instance_id);
            drop(state);
            self.perf_trackers.lock().unwrap().remove(&spec.instance_id);
            let _ = crate::commands::telemetry::finish_run_session(
                running.telemetry_session_id.as_deref(),
                None,
                "runtime-monitor-start-failed",
            );
            let _ = self.persist();
            return Err(format!("failed to start runtime process monitor: {error}"));
        }

        if let Err(error) = self.spawn_instance_monitor(running.clone(), spec.config.clone()) {
            let _ = terminate_running_instance(&running);
            let mut state = self.state.lock().unwrap();
            state.running.remove(&spec.instance_id);
            state.desired_instances.remove(&spec.instance_id);
            drop(state);
            self.perf_trackers.lock().unwrap().remove(&spec.instance_id);
            let _ = crate::commands::telemetry::finish_run_session(
                running.telemetry_session_id.as_deref(),
                None,
                "runtime-metrics-start-failed",
            );
            let _ = self.persist();
            return Err(error);
        }

        Ok(running)
    }

    fn record_instance_failure_locked(
        self: &Arc<Self>,
        spec: RuntimeLaunchSpec,
        kind: RuntimeFailureKind,
        message: String,
        exit_code: Option<i32>,
    ) {
        let now = unix_timestamp();
        let failure = RuntimeFailure {
            kind,
            message: message.clone(),
            exit_code,
            occurred_at: now,
        };
        let runtime_error = match kind {
            RuntimeFailureKind::StartupFailure => {
                format!("instance {} failed to start: {message}", spec.instance_id)
            }
            RuntimeFailureKind::UnexpectedExit => message.clone(),
        };
        let (restart_attempts, next_retry_at) = {
            let mut state = self.state.lock().unwrap();
            let recovery = recovery_status_after_failure(
                instance_recovery_enabled(&spec),
                state.recovery.get(&spec.instance_id),
                failure,
                now,
            );
            let restart_attempts = recovery.restart_attempts;
            let next_retry_at = recovery.next_retry_at;
            state
                .desired_instances
                .insert(spec.instance_id.clone(), spec.clone());
            state.recovery.insert(spec.instance_id.clone(), recovery);
            (restart_attempts, next_retry_at)
        };
        *self.last_error.lock().unwrap() = Some(runtime_error);
        if let Err(error) = self.persist() {
            if let Some(recovery) = self
                .state
                .lock()
                .unwrap()
                .recovery
                .get_mut(&spec.instance_id)
            {
                recovery.phase = InstanceRecoveryPhase::CrashLoop;
                recovery.next_retry_at = None;
            }
            *self.last_error.lock().unwrap() = Some(format!(
                "failed to persist recovery state for instance {}: {error}",
                spec.instance_id
            ));
            return;
        }
        if let Some(next_retry_at) = next_retry_at {
            self.schedule_instance_recovery(&spec.instance_id, restart_attempts, next_retry_at);
        }
    }

    fn schedule_instance_recovery(
        self: &Arc<Self>,
        instance_id: &str,
        expected_restart_attempts: u32,
        next_retry_at: u64,
    ) {
        let supervisor = Arc::downgrade(self);
        let scheduled_instance_id = instance_id.to_string();
        let thread_instance_id = scheduled_instance_id.clone();
        let result = std::thread::Builder::new()
            .name(format!("runtime-recovery-{thread_instance_id}"))
            .spawn(move || {
                let delay = next_retry_at.saturating_sub(unix_timestamp());
                if delay > 0 {
                    std::thread::sleep(std::time::Duration::from_secs(delay));
                }
                if let Some(supervisor) = supervisor.upgrade() {
                    supervisor.run_scheduled_instance_recovery(
                        &thread_instance_id,
                        expected_restart_attempts,
                        next_retry_at,
                    );
                }
            });
        if let Err(error) = result {
            let mut state = self.state.lock().unwrap();
            if let Some(recovery) = state.recovery.get_mut(&scheduled_instance_id) {
                if scheduled_recovery_matches(recovery, expected_restart_attempts, next_retry_at) {
                    recovery.phase = InstanceRecoveryPhase::CrashLoop;
                    recovery.next_retry_at = None;
                    recovery.last_failure.message = format!(
                        "{}; failed to schedule recovery: {error}",
                        recovery.last_failure.message
                    );
                }
            }
            drop(state);
            *self.last_error.lock().unwrap() = Some(format!(
                "failed to schedule recovery for instance {scheduled_instance_id}: {error}"
            ));
            let _ = self.persist();
        }
    }

    fn run_scheduled_instance_recovery(
        self: &Arc<Self>,
        instance_id: &str,
        expected_restart_attempts: u32,
        expected_retry_at: u64,
    ) {
        let _lifecycle = self.instance_lifecycle.lock().unwrap();
        let spec = {
            let mut state = self.state.lock().unwrap();
            if state.running.contains_key(instance_id) {
                return;
            }
            let Some(recovery) = state.recovery.get_mut(instance_id) else {
                return;
            };
            if !scheduled_recovery_matches(recovery, expected_restart_attempts, expected_retry_at) {
                return;
            }
            recovery.restart_attempts = recovery.restart_attempts.saturating_add(1);
            recovery.phase = InstanceRecoveryPhase::Monitoring;
            recovery.next_retry_at = None;
            state.desired_instances.get(instance_id).cloned()
        };
        let Some(spec) = spec else {
            return;
        };
        if let Err(error) = self.persist() {
            if let Some(recovery) = self.state.lock().unwrap().recovery.get_mut(instance_id) {
                recovery.phase = InstanceRecoveryPhase::CrashLoop;
            }
            *self.last_error.lock().unwrap() = Some(format!(
                "recovery of instance {instance_id} stopped because its attempt could not be persisted: {error}"
            ));
            return;
        }
        if let Err(error) = self.start_instance_locked(spec.clone(), false) {
            self.record_instance_failure_locked(
                spec,
                RuntimeFailureKind::StartupFailure,
                error,
                None,
            );
        }
    }

    fn record_process_exit(self: &Arc<Self>, instance_id: &str, pid: u32, exit_code: Option<i32>) {
        let _lifecycle = self.instance_lifecycle.lock().unwrap();
        let stop_intent = self.stop_intents.lock().unwrap().remove(instance_id);
        let expected_stop = stop_intent.is_some();
        let preserve_desired = stop_intent
            .as_ref()
            .is_some_and(|intent| intent.preserve_desired);
        let (removed, desired) = {
            let mut state = self.state.lock().unwrap();
            let desired = state.desired_instances.get(instance_id).cloned();
            if state
                .running
                .get(instance_id)
                .is_some_and(|running| running.pid == pid)
            {
                if expected_stop && !preserve_desired {
                    state.desired_instances.remove(instance_id);
                    state.recovery.remove(instance_id);
                }
                (state.running.remove(instance_id), desired)
            } else {
                (None, desired)
            }
        };
        if let Some(running) = removed {
            let failure_message = (!expected_stop).then(|| {
                format!(
                    "instance {instance_id} exited unexpectedly (code {})",
                    exit_code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "unknown".into())
                )
            });
            let _ = crate::commands::telemetry::finish_run_session(
                running.telemetry_session_id.as_deref(),
                exit_code,
                stop_intent
                    .as_ref()
                    .map(|intent| intent.telemetry_reason.as_str())
                    .unwrap_or("process-exited"),
            );
            if expected_stop {
                if let Err(error) = self.persist() {
                    *self.last_error.lock().unwrap() = Some(format!(
                        "failed to persist exit of instance {instance_id}: {error}"
                    ));
                }
            } else if let Some(message) = failure_message {
                if recovery_budget_is_stable(running.start_time, unix_timestamp()) {
                    self.state.lock().unwrap().recovery.remove(instance_id);
                }
                if let Some(spec) = desired {
                    self.record_instance_failure_locked(
                        spec,
                        RuntimeFailureKind::UnexpectedExit,
                        message,
                        exit_code,
                    );
                } else {
                    let failure = RuntimeFailure {
                        kind: RuntimeFailureKind::UnexpectedExit,
                        message: message.clone(),
                        exit_code,
                        occurred_at: unix_timestamp(),
                    };
                    self.state.lock().unwrap().recovery.insert(
                        instance_id.to_string(),
                        InstanceRecoveryStatus {
                            phase: InstanceRecoveryPhase::Failed,
                            restart_attempts: 0,
                            max_restart_attempts: INSTANCE_RECOVERY_MAX_ATTEMPTS,
                            next_retry_at: None,
                            origin_failure: failure.clone(),
                            last_failure: failure,
                        },
                    );
                    *self.last_error.lock().unwrap() = Some(message);
                    if let Err(error) = self.persist() {
                        *self.last_error.lock().unwrap() = Some(format!(
                            "failed to persist unexpected exit of instance {instance_id}: {error}"
                        ));
                    }
                }
            }
        }
        self.health.lock().unwrap().remove(instance_id);
        self.perf_trackers.lock().unwrap().remove(instance_id);
        crate::commands::monitoring::remove_instance(instance_id);
    }

    fn clear_stable_instance_recovery(&self, instance_id: &str, expected_pid: u32) {
        let _lifecycle = self.instance_lifecycle.lock().unwrap();
        let removed = {
            let mut state = self.state.lock().unwrap();
            let stable_current_process = state.running.get(instance_id).is_some_and(|running| {
                running.pid == expected_pid
                    && recovery_budget_is_stable(running.start_time, unix_timestamp())
            });
            let monitored_incident = state
                .recovery
                .get(instance_id)
                .is_some_and(|recovery| recovery.phase == InstanceRecoveryPhase::Monitoring);
            (stable_current_process && monitored_incident)
                .then(|| state.recovery.remove(instance_id))
                .flatten()
        };
        let Some(recovery) = removed else {
            return;
        };
        if let Err(error) = self.persist() {
            let mut state = self.state.lock().unwrap();
            let still_current = state
                .running
                .get(instance_id)
                .is_some_and(|running| running.pid == expected_pid);
            if still_current && !state.recovery.contains_key(instance_id) {
                state.recovery.insert(instance_id.to_string(), recovery);
            }
            drop(state);
            *self.last_error.lock().unwrap() = Some(format!(
                "failed to persist stable recovery completion for instance {instance_id}: {error}"
            ));
            return;
        }
        self.clear_retried_instance_error(instance_id);
    }

    pub fn stop_instance(&self, instance_id: &str) -> Result<(), String> {
        self.stop_instance_with_mode(instance_id, false, "manual-stop")
    }

    fn stop_instance_with_mode(
        &self,
        instance_id: &str,
        preserve_desired: bool,
        telemetry_reason: &str,
    ) -> Result<(), String> {
        let _lifecycle = self.instance_lifecycle.lock().unwrap();
        self.stop_instance_locked(instance_id, preserve_desired, telemetry_reason)
    }

    fn stop_instance_locked(
        &self,
        instance_id: &str,
        preserve_desired: bool,
        telemetry_reason: &str,
    ) -> Result<(), String> {
        let running = self.state.lock().unwrap().running.get(instance_id).cloned();
        let Some(running) = running else {
            if !preserve_desired {
                let previous = {
                    let mut state = self.state.lock().unwrap();
                    (
                        state.desired_instances.remove(instance_id),
                        state.recovery.remove(instance_id),
                    )
                };
                if previous.0.is_none() && previous.1.is_none() {
                    return Ok(());
                }
                if let Err(error) = self.persist() {
                    let mut state = self.state.lock().unwrap();
                    if let Some(desired) = previous.0 {
                        state
                            .desired_instances
                            .insert(instance_id.to_string(), desired);
                    }
                    if let Some(recovery) = previous.1 {
                        state.recovery.insert(instance_id.to_string(), recovery);
                    }
                    return Err(error);
                }
            }
            return Ok(());
        };
        self.stop_intents.lock().unwrap().insert(
            instance_id.to_string(),
            StopIntent {
                preserve_desired,
                telemetry_reason: telemetry_reason.to_string(),
            },
        );
        if running_instance_matches_live_process(&running) && !terminate_running_instance(&running)
        {
            self.stop_intents.lock().unwrap().remove(instance_id);
            return Err(format!(
                "无法终止后台实例 {} (PID {})",
                instance_id, running.pid
            ));
        }
        let removed = {
            let mut state = self.state.lock().unwrap();
            if !preserve_desired {
                state.desired_instances.remove(instance_id);
                state.recovery.remove(instance_id);
            } else if let Some(recovery) = state.recovery.get_mut(instance_id) {
                recovery.phase = InstanceRecoveryPhase::Restoring;
                recovery.next_retry_at = None;
            }
            state.running.remove(instance_id)
        };
        self.stop_intents.lock().unwrap().remove(instance_id);
        if removed.is_some() {
            self.health.lock().unwrap().remove(instance_id);
            self.perf_trackers.lock().unwrap().remove(instance_id);
            crate::commands::monitoring::remove_instance(instance_id);
            let _ = crate::commands::telemetry::finish_run_session(
                running.telemetry_session_id.as_deref(),
                None,
                telemetry_reason,
            );
        }
        self.persist()
    }

    pub fn stop_all_instances(&self) -> Vec<String> {
        self.stop_all_instances_internal(false, "manual-stop")
    }

    fn stop_all_instances_internal(
        &self,
        preserve_desired: bool,
        telemetry_reason: &str,
    ) -> Vec<String> {
        let _lifecycle = self.instance_lifecycle.lock().unwrap();
        let instance_ids = {
            let state = self.state.lock().unwrap();
            let mut ids = state
                .running
                .keys()
                .chain(state.desired_instances.keys())
                .cloned()
                .collect::<Vec<_>>();
            ids.sort();
            ids.dedup();
            ids
        };
        let mut failures = Vec::new();
        for instance_id in instance_ids {
            if let Err(error) =
                self.stop_instance_locked(&instance_id, preserve_desired, telemetry_reason)
            {
                failures.push(error);
            }
        }
        failures
    }

    fn restore_missing_desired_instances(self: &Arc<Self>) -> Vec<String> {
        let (desired, recovery) = {
            let state = self.state.lock().unwrap();
            (state.desired_instances.clone(), state.recovery.clone())
        };
        let mut failures = Vec::new();
        for (instance_id, spec) in desired {
            let is_running = self
                .state
                .lock()
                .unwrap()
                .running
                .get(&instance_id)
                .is_some_and(running_instance_matches_live_process);
            if is_running {
                continue;
            }
            match recovery.get(&instance_id).map(|status| status.phase) {
                Some(InstanceRecoveryPhase::CrashLoop | InstanceRecoveryPhase::Failed) => {}
                Some(InstanceRecoveryPhase::Waiting) => {
                    if let Some(status) = recovery.get(&instance_id) {
                        if let Some(next_retry_at) = status.next_retry_at {
                            self.schedule_instance_recovery(
                                &instance_id,
                                status.restart_attempts,
                                next_retry_at,
                            );
                        }
                    }
                }
                Some(InstanceRecoveryPhase::Monitoring) => {
                    let _lifecycle = self.instance_lifecycle.lock().unwrap();
                    self.record_instance_failure_locked(
                        spec,
                        RuntimeFailureKind::UnexpectedExit,
                        format!("instance {instance_id} was absent while restoring runtime state"),
                        None,
                    );
                }
                Some(InstanceRecoveryPhase::Restoring) => {
                    let _lifecycle = self.instance_lifecycle.lock().unwrap();
                    if let Err(error) = self.start_instance_locked(spec.clone(), false) {
                        self.record_instance_failure_locked(
                            spec,
                            RuntimeFailureKind::StartupFailure,
                            error.clone(),
                            None,
                        );
                        failures.push(format!("failed to restore instance {instance_id}: {error}"));
                    } else {
                        if let Some(recovery) =
                            self.state.lock().unwrap().recovery.get_mut(&instance_id)
                        {
                            recovery.phase = InstanceRecoveryPhase::Monitoring;
                        }
                        let _ = self.persist();
                    }
                }
                None => {
                    let _lifecycle = self.instance_lifecycle.lock().unwrap();
                    if let Err(error) = self.start_instance_locked(spec.clone(), false) {
                        self.record_instance_failure_locked(
                            spec,
                            RuntimeFailureKind::StartupFailure,
                            error.clone(),
                            None,
                        );
                        failures.push(format!("failed to restore instance {instance_id}: {error}"));
                    }
                }
            }
        }
        failures
    }

    fn record_proxy_runtime_error(&self, message: String) {
        self.proxy_status.lock().unwrap().last_error = Some(message.clone());
        *self.last_error.lock().unwrap() = Some(message);
    }

    pub fn clear_last_error(&self) {
        *self.last_error.lock().unwrap() = None;
    }

    fn clear_retried_instance_error(&self, instance_id: &str) {
        let mut last_error = self.last_error.lock().unwrap();
        if last_error
            .as_deref()
            .is_some_and(|error| is_instance_recovery_error(error, instance_id))
        {
            *last_error = None;
        }
    }

    fn clear_recovered_proxy_error(&self) {
        let mut last_error = self.last_error.lock().unwrap();
        if last_error.as_deref().is_some_and(|error| {
            error.starts_with("proxy server error:")
                || error.starts_with("failed to restart routing service:")
                || error.starts_with("failed to restore routing service:")
        }) {
            *last_error = None;
        }
    }

    fn proxy_configuration_is_valid(&self) -> bool {
        let state = self.state.lock().unwrap();
        normalize_and_validate_proxy_config(state.proxy_config.clone(), &state.instances).is_ok()
    }

    fn schedule_proxy_restart(self: &Arc<Self>) {
        if !self.proxy_configuration_is_valid() {
            return;
        }
        let supervisor = self.clone();
        tokio::spawn(async move {
            let mut retry_delay = std::time::Duration::from_secs(2);
            loop {
                tokio::time::sleep(retry_delay).await;
                if !supervisor.state.lock().unwrap().proxy_config.enabled {
                    break;
                }
                match supervisor.start_proxy().await {
                    Ok(_) => {
                        supervisor.clear_recovered_proxy_error();
                        break;
                    }
                    Err(error) => {
                        supervisor.record_proxy_runtime_error(format!(
                            "failed to restart routing service: {error}"
                        ));
                        retry_delay = std::time::Duration::from_secs(10);
                    }
                }
            }
        });
    }

    pub async fn start_proxy(self: &Arc<Self>) -> Result<ProxyStatus, String> {
        let mut runtime = self.proxy_runtime.lock().await;
        if runtime
            .as_ref()
            .is_some_and(|current| !current.task.is_finished())
        {
            return Ok(self.proxy_status.lock().unwrap().clone());
        }
        if let Some(finished) = runtime.take() {
            let _ = finished.task.await;
        }

        let (persisted_config, instances) = {
            let state = self.state.lock().unwrap();
            (state.proxy_config.clone(), state.instances.clone())
        };
        let config = match normalize_and_validate_proxy_config(persisted_config.clone(), &instances)
        {
            Ok(config) => config,
            Err(error) => {
                self.record_proxy_runtime_error(error.clone());
                return Err(error);
            }
        };
        self.state.lock().unwrap().proxy_config = config.clone();
        if let Err(error) = self.persist() {
            self.state.lock().unwrap().proxy_config = persisted_config;
            return Err(error);
        }
        let host = config.host.trim();
        let bound_addr = crate::utils::format_host_port(host, config.port);
        let listener = tokio::net::TcpListener::bind(&bound_addr)
            .await
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AddrInUse {
                    format!("failed to bind proxy {bound_addr}: address is already in use")
                } else {
                    format!("failed to bind proxy {bound_addr}: {error}")
                }
            })?;
        let previous_enabled = {
            let mut state = self.state.lock().unwrap();
            let previous = state.proxy_config.enabled;
            state.proxy_config.enabled = true;
            previous
        };
        if let Err(error) = self.persist() {
            self.state.lock().unwrap().proxy_config.enabled = previous_enabled;
            return Err(error);
        }
        {
            let mut status = self.proxy_status.lock().unwrap();
            status.running = true;
            status.bound_addr = bound_addr;
            status.last_error = None;
        }
        let (shutdown, receiver) = tokio::sync::oneshot::channel();
        let source: Arc<dyn ProxyDataSource> = self.clone();
        let (router, router_runtime) = proxy_router_from_source_with_runtime(source);
        *self.proxy_router_runtime.lock().unwrap() = Some(router_runtime);
        let supervisor = self.clone();
        let task = tokio::spawn(async move {
            let result = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = receiver.await;
                })
                .await;
            let restart = result.is_err();
            let runtime_error = result
                .as_ref()
                .err()
                .map(|error| format!("proxy server error: {error}"));
            {
                let mut status = supervisor.proxy_status.lock().unwrap();
                status.running = false;
                if let Some(error) = runtime_error.as_ref() {
                    status.last_error = Some(error.clone());
                }
            }
            *supervisor.proxy_router_runtime.lock().unwrap() = None;
            if let Some(error) = runtime_error {
                *supervisor.last_error.lock().unwrap() = Some(error);
            }
            if restart {
                supervisor.schedule_proxy_restart();
            }
        });
        *runtime = Some(RuntimeProxy { shutdown, task });
        Ok(self.proxy_status.lock().unwrap().clone())
    }

    async fn stop_proxy_runtime(&self, clear_desired_state: bool) -> Result<ProxyStatus, String> {
        let mut proxy_runtime = self.proxy_runtime.lock().await;
        let runtime = proxy_runtime.take();
        let mut task_failure = None;
        if let Some(runtime) = runtime {
            let _ = runtime.shutdown.send(());
            let mut task = runtime.task;
            tokio::select! {
                result = &mut task => {
                    if let Err(error) = result {
                        task_failure = Some(format!("proxy runtime task failed: {error}"));
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {
                    task.abort();
                    let _ = task.await;
                }
            }
        }
        *self.proxy_router_runtime.lock().unwrap() = None;
        {
            let mut status = self.proxy_status.lock().unwrap();
            status.running = false;
            if let Some(error) = task_failure.as_ref() {
                status.last_error = Some(error.clone());
            }
        }
        let mut persist_failure = None;
        if clear_desired_state {
            let previous_enabled = {
                let mut state = self.state.lock().unwrap();
                let previous = state.proxy_config.enabled;
                state.proxy_config.enabled = false;
                previous
            };
            if let Err(error) = self.persist() {
                self.state.lock().unwrap().proxy_config.enabled = previous_enabled;
                persist_failure = Some(error);
            }
        }
        if task_failure.is_some() || persist_failure.is_some() {
            return Err([task_failure, persist_failure]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("; "));
        }
        Ok(self.proxy_status.lock().unwrap().clone())
    }

    pub async fn stop_proxy(&self) -> Result<ProxyStatus, String> {
        self.stop_proxy_runtime(true).await
    }

    pub async fn restore(self: &Arc<Self>) -> Result<(), String> {
        let stale_running = {
            let mut state = self.state.lock().unwrap();
            let stale = state
                .running
                .values()
                .filter(|running| !running_instance_matches_live_process(running))
                .cloned()
                .collect::<Vec<_>>();
            state
                .running
                .retain(|_, running| running_instance_matches_live_process(running));
            stale
        };
        for running in stale_running {
            let _ = crate::commands::telemetry::finish_run_session(
                running.telemetry_session_id.as_deref(),
                None,
                "runtime-supervisor-recovery",
            );
        }
        self.persist()?;
        let (running, desired, proxy_enabled) = {
            let state = self.state.lock().unwrap();
            (
                state.running.clone(),
                state.desired_instances.clone(),
                state.proxy_config.enabled,
            )
        };
        for (instance_id, running) in running {
            if desired.contains_key(&instance_id) {
                if let Err(error) =
                    self.stop_instance_with_mode(&instance_id, true, "runtime-supervisor-recovery")
                {
                    *self.last_error.lock().unwrap() = Some(format!(
                        "failed to restart incompletely supervised instance {instance_id}: {error}"
                    ));
                } else {
                    continue;
                }
            }
            if let Some(config) = desired
                .get(&instance_id)
                .map(|spec| spec.config.clone())
                .or_else(|| running.launch_config.clone())
            {
                if let Err(error) = self.spawn_instance_monitor(running.clone(), config) {
                    *self.last_error.lock().unwrap() = Some(format!(
                        "failed to monitor adopted instance {instance_id}: {error}"
                    ));
                    let preserve_desired = desired.contains_key(&instance_id);
                    if let Err(stop_error) = self.stop_instance_with_mode(
                        &instance_id,
                        preserve_desired,
                        "runtime-metrics-start-failed",
                    ) {
                        *self.last_error.lock().unwrap() = Some(format!(
                            "failed to stop unmonitored instance {instance_id}: {stop_error}"
                        ));
                    }
                    continue;
                }
            }
            let supervisor = Arc::downgrade(self);
            let monitored_instance_id = instance_id.clone();
            let monitored_running = running.clone();
            if let Err(error) = std::thread::Builder::new()
                .name(format!("runtime-adopted-{instance_id}"))
                .spawn(move || {
                    while running_instance_matches_live_process(&monitored_running) {
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    }
                    if let Some(supervisor) = supervisor.upgrade() {
                        supervisor.record_process_exit(
                            &monitored_instance_id,
                            monitored_running.pid,
                            None,
                        );
                    }
                })
            {
                *self.last_error.lock().unwrap() = Some(format!(
                    "failed to monitor adopted instance {instance_id}: {error}"
                ));
                let preserve_desired = desired.contains_key(&instance_id);
                if let Err(stop_error) = self.stop_instance_with_mode(
                    &instance_id,
                    preserve_desired,
                    "runtime-monitor-start-failed",
                ) {
                    *self.last_error.lock().unwrap() = Some(format!(
                        "failed to stop unmonitored instance {instance_id}: {stop_error}"
                    ));
                }
            }
        }
        let restore_failures = self.restore_missing_desired_instances();
        if !restore_failures.is_empty() {
            *self.last_error.lock().unwrap() = Some(restore_failures.join("; "));
        }
        if proxy_enabled {
            if let Err(error) = self.start_proxy().await {
                self.record_proxy_runtime_error(format!(
                    "failed to restore routing service: {error}"
                ));
                // Login recovery can race with a temporarily occupied port or a
                // network stack that is not ready yet. Keep the persisted intent
                // and retry in the runtime instead of requiring the GUI to reopen.
                self.schedule_proxy_restart();
            }
        }
        Ok(())
    }

    pub async fn handle_command(
        self: &Arc<Self>,
        command: RuntimeCommand,
        registered_for_login: bool,
    ) -> Result<RuntimeReply, String> {
        match command {
            RuntimeCommand::Ping => Ok(RuntimeReply::Pong),
            RuntimeCommand::Heartbeat { gui_pid } => {
                self.heartbeat(gui_pid)?;
                Ok(RuntimeReply::Status(Box::new(
                    self.status(registered_for_login),
                )))
            }
            RuntimeCommand::GetStatus => Ok(RuntimeReply::Status(Box::new(
                self.status(registered_for_login),
            ))),
            RuntimeCommand::SyncConfig {
                revision,
                proxy_config,
                instances,
            } => {
                self.sync_config(revision, proxy_config, instances).await?;
                Ok(RuntimeReply::Ack)
            }
            RuntimeCommand::PrepareBackgroundDetach {
                revision,
                proxy_config,
                instances,
                expected_running,
            } => self
                .prepare_background_detach(
                    revision,
                    proxy_config,
                    instances,
                    expected_running,
                    registered_for_login,
                )
                .await
                .map(Box::new)
                .map(RuntimeReply::Status),
            RuntimeCommand::StartInstance {
                spec,
                manual_recovery,
            } => self
                .start_instance(*spec, manual_recovery)
                .map(Box::new)
                .map(RuntimeReply::Instance),
            RuntimeCommand::StopInstance { instance_id } => {
                self.stop_instance(&instance_id)?;
                Ok(RuntimeReply::Ack)
            }
            RuntimeCommand::ClearLastError => {
                self.clear_last_error();
                Ok(RuntimeReply::Ack)
            }
            RuntimeCommand::SetBackgroundEnabled { enabled } => {
                self.set_background_enabled(enabled)?;
                Ok(RuntimeReply::Status(Box::new(
                    self.status(registered_for_login),
                )))
            }
            RuntimeCommand::StartProxy => self.start_proxy().await.map(RuntimeReply::ProxyStatus),
            RuntimeCommand::StopProxy => self.stop_proxy().await.map(RuntimeReply::ProxyStatus),
            RuntimeCommand::Shutdown { stop_instances } => {
                if stop_instances {
                    let mut failures = Vec::new();
                    if let Err(error) = self.stop_proxy().await {
                        failures.push(format!("failed to stop routing service: {error}"));
                    }
                    failures.extend(self.stop_all_instances());
                    if !failures.is_empty() {
                        return Err(failures.join("; "));
                    }
                } else {
                    let _ = self.stop_proxy_runtime(false).await;
                    let failures = self.stop_all_instances_internal(true, "runtime-upgrade");
                    if !failures.is_empty() {
                        let restore_failures = self.restore_missing_desired_instances();
                        let mut errors = failures;
                        errors.extend(restore_failures);
                        if self.state.lock().unwrap().proxy_config.enabled {
                            if let Err(error) = self.start_proxy().await {
                                errors.push(format!(
                                    "failed to restore routing service after aborted upgrade: {error}"
                                ));
                            }
                        }
                        return Err(errors.join("; "));
                    }
                }
                let _ = crate::commands::telemetry::flush_telemetry_writer();
                tokio::spawn(async {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    std::process::exit(0);
                });
                Ok(RuntimeReply::Ack)
            }
        }
    }
}

impl ProxyDataSource for RuntimeSupervisor {
    fn proxy_snapshot(&self) -> ProxyRuntimeSnapshot {
        let state = self.state.lock().unwrap();
        let proxy_status = self.proxy_status.lock().unwrap();
        ProxyRuntimeSnapshot {
            config: state.proxy_config.clone(),
            instances: state.instances.clone(),
            running: state.running.clone(),
            bound_addr: proxy_status.bound_addr.clone(),
            last_error: proxy_status.last_error.clone(),
        }
    }

    fn resolve_proxy_request(
        &self,
        requested_model: Option<&str>,
        endpoint_workload: Option<ModelWorkload>,
    ) -> ProxyRequestResolution {
        let state = self.state.lock().unwrap();
        proxy_request_resolution_from(
            state.proxy_config.clone(),
            &state.instances,
            &state.running,
            requested_model,
            endpoint_workload,
        )
    }
}

pub fn start_watchdog(supervisor: Arc<RuntimeSupervisor>) {
    std::thread::Builder::new()
        .name("runtime-gui-watchdog".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            if !should_stop_for_missing_gui(
                supervisor.background_enabled(),
                supervisor.heartbeat_expired(),
            ) {
                continue;
            }
            let failures = supervisor.stop_all_instances();
            if failures.is_empty() {
                let _ = crate::commands::telemetry::flush_telemetry_writer();
                std::process::exit(0);
            }
        })
        .expect("runtime watchdog thread must start");
}

fn should_stop_for_missing_gui(background_enabled: bool, heartbeat_expired: bool) -> bool {
    !background_enabled && heartbeat_expired
}

#[cfg(test)]
mod tests {
    use super::{
        gui_owner_is_alive, is_instance_recovery_error, recovery_budget_is_stable,
        recovery_status_after_failure, runtime_config_matches, runtime_launch_config_matches,
        scheduled_recovery_matches, should_stop_for_missing_gui, sync_desired_launch_config,
        validate_background_detach_inventory, validate_runtime_deployment_identity,
        validate_runtime_engine_qualification, validate_runtime_state, GuiOwner,
        QUALIFICATION_PROFILE_VERSION,
    };
    use crate::commands::engine_capabilities::executable_fingerprint;
    use crate::commands::server::read_process_identity;
    use crate::models::{InstanceConfig, RunningInstance};
    use crate::runtime_service::protocol::{
        InstanceRecoveryPhase, PersistedRuntimeState, RuntimeFailure, RuntimeFailureKind,
        RuntimeLaunchSpec, INSTANCE_RECOVERY_MAX_ATTEMPTS, INSTANCE_RECOVERY_STABLE_SECS,
        RUNTIME_STATE_SCHEMA_VERSION,
    };
    use std::collections::HashMap;

    fn test_deployment_identity() -> crate::deployment_identity::DeploymentIdentity {
        crate::deployment_identity::DeploymentIdentity::new(
            "urn:lsm:engine:v1:sha256:test".into(),
            "urn:lsm:model:v1:sha256:test".into(),
            "revision-test".into(),
            "urn:lsm:configuration:v1:sha256:test".into(),
            "urn:lsm:qualification:v2:sha256:test".into(),
        )
        .unwrap()
    }

    fn detach_instance(pid: u32) -> RunningInstance {
        RunningInstance {
            instance_id: "instance-1".into(),
            pid,
            port: 8080,
            host: "127.0.0.1".into(),
            start_time: 123,
            executable_path: "/tmp/llama-server".into(),
            telemetry_session_id: None,
            workload: "inference".into(),
            launch_config: None,
            deployment_identity: Default::default(),
            deployment_id: String::new(),
            deployment_revision_id: String::new(),
        }
    }

    fn detach_spec() -> RuntimeLaunchSpec {
        let config = InstanceConfig {
            id: "instance-1".into(),
            ..InstanceConfig::default()
        };
        let deployment_identity = test_deployment_identity();
        let deployment_revision = crate::deployment::test_revision(
            "instance-1",
            &config,
            &deployment_identity,
            &crate::models::ProxyConfig::default(),
        );
        RuntimeLaunchSpec {
            instance_id: "instance-1".into(),
            config,
            launch_config_stale: false,
            engine_qualification_fingerprint: "test-fingerprint".into(),
            engine_qualification_profile_version: QUALIFICATION_PROFILE_VERSION,
            deployment_identity,
            deployment_revision,
            engine_backend: "test".into(),
            command: vec!["llama-server".into()],
            command_display: "llama-server".into(),
            workload: "inference".into(),
            working_directory: None,
        }
    }

    #[test]
    fn runtime_recovery_revalidates_the_complete_deployment_identity() {
        let dir = std::env::temp_dir().join(format!(
            "lsm-runtime-deployment-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let engine_path = dir.join("llama-server");
        let model_path = dir.join("model.gguf");
        std::fs::write(&engine_path, vec![b'e'; 128 * 1024]).unwrap();
        std::fs::write(&model_path, vec![b'm'; 128 * 1024]).unwrap();
        let engine =
            crate::deployment_identity::artifact_identity_for_path("engine", &engine_path).unwrap();
        let model =
            crate::deployment_identity::artifact_identity_for_path("model", &model_path).unwrap();
        let config = InstanceConfig {
            id: "instance-1".into(),
            model_path: model_path.to_string_lossy().to_string(),
            ..InstanceConfig::default()
        };
        let fingerprint = crate::config_revision::deployment_config_fingerprint(&config).unwrap();
        let configuration_id =
            crate::config_revision::configuration_id_from_fingerprint(&fingerprint).unwrap();
        let deployment_identity = crate::deployment_identity::DeploymentIdentity::new(
            engine.artifact_id,
            model.artifact_id,
            "revision-1".into(),
            configuration_id,
            "urn:lsm:qualification:v2:sha256:qualification-1".into(),
        )
        .unwrap();
        let deployment_revision = crate::deployment::test_revision(
            "instance-1",
            &config,
            &deployment_identity,
            &crate::models::ProxyConfig::default(),
        );
        let spec = RuntimeLaunchSpec {
            instance_id: "instance-1".into(),
            config,
            launch_config_stale: false,
            engine_qualification_fingerprint: executable_fingerprint(
                &engine_path.to_string_lossy(),
            ),
            engine_qualification_profile_version: QUALIFICATION_PROFILE_VERSION,
            deployment_identity,
            deployment_revision,
            engine_backend: "test".into(),
            command: vec![engine_path.to_string_lossy().to_string()],
            command_display: "llama-server".into(),
            workload: "inference".into(),
            working_directory: None,
        };
        validate_runtime_deployment_identity(&spec).unwrap();
        std::fs::write(&model_path, vec![b'x'; 128 * 1024]).unwrap();
        assert!(validate_runtime_deployment_identity(&spec)
            .unwrap_err()
            .starts_with("DEPLOYMENT_MODEL_IDENTITY_STALE"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn failure(message: &str, occurred_at: u64) -> RuntimeFailure {
        RuntimeFailure {
            kind: RuntimeFailureKind::UnexpectedExit,
            message: message.into(),
            exit_code: Some(1),
            occurred_at,
        }
    }

    #[test]
    fn disabled_recovery_records_failure_without_scheduling() {
        let status = recovery_status_after_failure(false, None, failure("origin", 100), 100);
        assert_eq!(status.phase, InstanceRecoveryPhase::Failed);
        assert_eq!(status.restart_attempts, 0);
        assert_eq!(status.max_restart_attempts, INSTANCE_RECOVERY_MAX_ATTEMPTS);
        assert_eq!(status.next_retry_at, None);
        assert_eq!(status.origin_failure.message, "origin");
    }

    #[test]
    fn runtime_recovery_is_bound_to_the_qualified_engine_artifact() {
        let mut spec = detach_spec();
        spec.engine_qualification_fingerprint.clear();
        spec.engine_qualification_profile_version = 0;
        assert!(validate_runtime_engine_qualification(&spec)
            .unwrap_err()
            .starts_with("ENGINE_QUALIFICATION_REQUIRED:"));

        let executable = std::env::temp_dir().join(format!(
            "lsm-runtime-qualified-engine-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&executable, vec![b'a'; 128 * 1024]).unwrap();
        spec.command = vec![executable.to_string_lossy().to_string()];
        spec.engine_qualification_fingerprint =
            executable_fingerprint(&executable.to_string_lossy());
        spec.engine_qualification_profile_version = QUALIFICATION_PROFILE_VERSION;
        validate_runtime_engine_qualification(&spec).unwrap();

        std::fs::write(&executable, vec![b'b'; 128 * 1024]).unwrap();
        assert!(validate_runtime_engine_qualification(&spec)
            .unwrap_err()
            .starts_with("ENGINE_QUALIFICATION_STALE:"));
        let _ = std::fs::remove_file(executable);
    }

    #[test]
    fn launch_snapshot_staleness_allows_display_renames_but_blocks_policy_and_command_drift() {
        let mut spec = detach_spec();
        spec.config.restart_policy = "on-failure".into();
        spec.deployment_revision = crate::deployment::test_revision(
            "instance-1",
            &spec.config,
            &spec.deployment_identity,
            &crate::models::ProxyConfig::default(),
        );
        let mut current = spec.config.clone();
        current.name = "renamed".into();

        assert!(runtime_launch_config_matches(&spec.config, &current));
        sync_desired_launch_config(&mut spec, &current, &crate::models::ProxyConfig::default());
        assert!(!spec.launch_config_stale);
        assert!(super::instance_recovery_enabled(&spec));

        current.auto_start = true;
        sync_desired_launch_config(&mut spec, &current, &crate::models::ProxyConfig::default());
        assert!(spec.launch_config_stale);
        assert!(!super::instance_recovery_enabled(&spec));
        current.auto_start = spec.config.auto_start;

        current.port = current.port.saturating_add(1);
        sync_desired_launch_config(&mut spec, &current, &crate::models::ProxyConfig::default());
        assert!(spec.launch_config_stale);
        assert!(!super::instance_recovery_enabled(&spec));
        assert_ne!(spec.config.port, current.port);

        current.port = spec.config.port;
        sync_desired_launch_config(&mut spec, &current, &crate::models::ProxyConfig::default());
        assert!(!spec.launch_config_stale);
        assert!(super::instance_recovery_enabled(&spec));
    }

    #[test]
    fn recovery_backoff_is_bounded_and_preserves_the_originating_failure() {
        let first = recovery_status_after_failure(true, None, failure("origin", 100), 100);
        assert_eq!(first.phase, InstanceRecoveryPhase::Waiting);
        assert_eq!(first.next_retry_at, Some(102));
        assert!(scheduled_recovery_matches(&first, 0, 102));

        let mut after_first_attempt = first.clone();
        after_first_attempt.phase = InstanceRecoveryPhase::Monitoring;
        after_first_attempt.restart_attempts = 1;
        after_first_attempt.next_retry_at = None;
        let second = recovery_status_after_failure(
            true,
            Some(&after_first_attempt),
            failure("retry one failed", 110),
            110,
        );
        assert_eq!(second.next_retry_at, Some(120));
        assert_eq!(second.origin_failure.message, "origin");
        assert_eq!(second.last_failure.message, "retry one failed");

        let mut exhausted = second;
        exhausted.phase = InstanceRecoveryPhase::Monitoring;
        exhausted.restart_attempts = INSTANCE_RECOVERY_MAX_ATTEMPTS;
        exhausted.next_retry_at = None;
        let crash_loop = recovery_status_after_failure(
            true,
            Some(&exhausted),
            failure("retry three failed", 140),
            140,
        );
        assert_eq!(crash_loop.phase, InstanceRecoveryPhase::CrashLoop);
        assert_eq!(crash_loop.next_retry_at, None);
        assert_eq!(crash_loop.origin_failure.message, "origin");
        assert!(!scheduled_recovery_matches(&crash_loop, 3, 170));
    }

    #[test]
    fn stable_runtime_resets_only_after_the_full_stability_window() {
        assert!(!recovery_budget_is_stable(
            100,
            100 + INSTANCE_RECOVERY_STABLE_SECS - 1
        ));
        assert!(recovery_budget_is_stable(
            100,
            100 + INSTANCE_RECOVERY_STABLE_SECS
        ));
        assert!(!recovery_budget_is_stable(0, u64::MAX));
    }

    #[test]
    fn watchdog_only_stops_gui_bound_runtime_after_heartbeat_expiry() {
        assert!(!should_stop_for_missing_gui(false, false));
        assert!(should_stop_for_missing_gui(false, true));
        assert!(!should_stop_for_missing_gui(true, false));
        assert!(!should_stop_for_missing_gui(true, true));
    }

    #[test]
    fn identical_runtime_config_is_recognized_as_a_noop() {
        let state = PersistedRuntimeState::default();
        let instances = HashMap::new();
        assert!(runtime_config_matches(
            &state,
            &state.proxy_config,
            &instances
        ));

        let mut changed = state.proxy_config.clone();
        changed.port = changed.port.saturating_add(1);
        assert!(!runtime_config_matches(&state, &changed, &instances));
    }

    #[test]
    fn retry_only_recognizes_the_same_instances_recovery_error() {
        let error = "instance instance-1 exited unexpectedly (code 1)";
        assert!(is_instance_recovery_error(error, "instance-1"));
        assert!(is_instance_recovery_error(
            "instance instance-1 failed to start: missing executable",
            "instance-1"
        ));
        assert!(!is_instance_recovery_error(error, "instance-2"));
        assert!(!is_instance_recovery_error(
            "failed to persist exit of instance instance-1",
            "instance-1"
        ));
    }

    #[test]
    fn future_runtime_state_schema_is_rejected_instead_of_downgraded() {
        let state = PersistedRuntimeState {
            schema_version: RUNTIME_STATE_SCHEMA_VERSION + 1,
            ..PersistedRuntimeState::default()
        };
        assert!(validate_runtime_state(state)
            .unwrap_err()
            .contains("unsupported runtime state schema"));
    }

    #[test]
    fn first_runtime_state_schema_migrates_without_recovery_incidents() {
        let state = PersistedRuntimeState {
            schema_version: 1,
            ..PersistedRuntimeState::default()
        };
        let migrated = validate_runtime_state(state).unwrap();
        assert_eq!(migrated.schema_version, RUNTIME_STATE_SCHEMA_VERSION);
        assert!(migrated.recovery.is_empty());
    }

    #[test]
    fn schema_three_runtime_state_migrates_but_legacy_specs_remain_unbound() {
        let mut state = PersistedRuntimeState {
            schema_version: 3,
            ..PersistedRuntimeState::default()
        };
        state
            .desired_instances
            .insert("instance-1".into(), detach_spec());
        state
            .desired_instances
            .get_mut("instance-1")
            .unwrap()
            .deployment_revision = Default::default();
        let migrated = validate_runtime_state(state).unwrap();
        assert_eq!(migrated.schema_version, RUNTIME_STATE_SCHEMA_VERSION);
        let legacy = &migrated.desired_instances["instance-1"];
        assert!(
            super::validate_runtime_deployment_revision(legacy, &migrated.proxy_config).is_err()
        );
    }

    #[test]
    fn live_gui_identity_keeps_a_stale_wall_clock_heartbeat_from_expiring() {
        let pid = std::process::id();
        let (start_time, executable_path) = read_process_identity(pid).unwrap();
        let mut owner = GuiOwner {
            pid,
            start_time,
            executable_path,
        };
        assert!(gui_owner_is_alive(&owner));
        owner.start_time = owner.start_time.saturating_add(1);
        assert!(!gui_owner_is_alive(&owner));
    }

    #[test]
    fn detach_inventory_requires_the_same_running_and_recovery_sets() {
        let expected = [("instance-1".to_string(), detach_instance(42))]
            .into_iter()
            .collect::<HashMap<_, _>>();
        let actual = expected.clone();
        let desired = [("instance-1".to_string(), detach_spec())]
            .into_iter()
            .collect::<HashMap<_, _>>();
        validate_background_detach_inventory(&expected, &actual, &desired).unwrap();

        let mut unexpected = actual.clone();
        unexpected.insert("instance-2".into(), detach_instance(43));
        assert!(
            validate_background_detach_inventory(&expected, &unexpected, &desired)
                .unwrap_err()
                .contains("实例清单不一致")
        );
    }

    #[test]
    fn detach_inventory_rejects_changed_process_identity() {
        let expected = [("instance-1".to_string(), detach_instance(42))]
            .into_iter()
            .collect::<HashMap<_, _>>();
        let actual = [("instance-1".to_string(), detach_instance(99))]
            .into_iter()
            .collect::<HashMap<_, _>>();
        let desired = [("instance-1".to_string(), detach_spec())]
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert!(
            validate_background_detach_inventory(&expected, &actual, &desired)
                .unwrap_err()
                .contains("进程身份")
        );
    }
}
