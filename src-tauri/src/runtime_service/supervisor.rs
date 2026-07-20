use super::protocol::{
    PersistedRuntimeState, RuntimeCommand, RuntimeLaunchSpec, RuntimeReply, RuntimeServiceStatus,
    RUNTIME_PROTOCOL_VERSION, RUNTIME_STATE_SCHEMA_VERSION,
};
use super::transport::runtime_state_path;
use crate::commands::proxy::{proxy_router_from_source, ProxyDataSource, ProxyRuntimeSnapshot};
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

fn validate_runtime_state(state: PersistedRuntimeState) -> Result<PersistedRuntimeState, String> {
    if state.schema_version != RUNTIME_STATE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported runtime state schema: expected {}, found {}",
            RUNTIME_STATE_SCHEMA_VERSION, state.schema_version
        ));
    }
    Ok(state)
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
                last_error: None,
            }),
            proxy_runtime: tokio::sync::Mutex::new(None),
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
        let (running, background_enabled, config_revision) = {
            let state = self.state.lock().unwrap();
            (
                state.running.clone(),
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
        RuntimeServiceStatus {
            protocol_version: RUNTIME_PROTOCOL_VERSION,
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            service_pid: std::process::id(),
            config_revision,
            background_enabled,
            registered_for_login,
            last_error: self.last_error.lock().unwrap().clone(),
            proxy: self.proxy_status.lock().unwrap().clone(),
            running,
            health: self.health.lock().unwrap().clone(),
            monitoring,
            performance,
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
        let _proxy_transition = self.proxy_runtime.lock().await;
        let _instance_transition = self.instance_lifecycle.lock().unwrap();
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
            );
            state.config_revision = revision;
            state.proxy_config = proxy_config.clone();
            state.instances = instances;
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
            state.background_enabled = enabled;
            previous
        };
        if let Err(error) = self.persist() {
            self.state.lock().unwrap().background_enabled = previous;
            return Err(error);
        }
        Ok(())
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
    ) -> Result<RunningInstance, String> {
        let _lifecycle = self.instance_lifecycle.lock().unwrap();
        self.start_instance_locked(spec)
    }

    fn start_instance_locked(
        self: &Arc<Self>,
        spec: RuntimeLaunchSpec,
    ) -> Result<RunningInstance, String> {
        if spec.command.is_empty() || spec.command[0].trim().is_empty() {
            return Err("runtime launch command is empty".into());
        }
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

    fn record_process_exit(&self, instance_id: &str, pid: u32, exit_code: Option<i32>) {
        let stop_intent = self.stop_intents.lock().unwrap().remove(instance_id);
        let expected_stop = stop_intent.is_some();
        let preserve_desired = stop_intent
            .as_ref()
            .is_some_and(|intent| intent.preserve_desired);
        let removed = {
            let mut state = self.state.lock().unwrap();
            if state
                .running
                .get(instance_id)
                .is_some_and(|running| running.pid == pid)
            {
                if !preserve_desired {
                    state.desired_instances.remove(instance_id);
                }
                state.running.remove(instance_id)
            } else {
                None
            }
        };
        if let Some(running) = removed {
            if !expected_stop {
                *self.last_error.lock().unwrap() = Some(format!(
                    "instance {instance_id} exited unexpectedly (code {})",
                    exit_code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "unknown".into())
                ));
            }
            let _ = crate::commands::telemetry::finish_run_session(
                running.telemetry_session_id.as_deref(),
                exit_code,
                stop_intent
                    .as_ref()
                    .map(|intent| intent.telemetry_reason.as_str())
                    .unwrap_or("process-exited"),
            );
            if let Err(error) = self.persist() {
                *self.last_error.lock().unwrap() = Some(format!(
                    "failed to persist exit of instance {instance_id}: {error}"
                ));
            }
        }
        self.health.lock().unwrap().remove(instance_id);
        self.perf_trackers.lock().unwrap().remove(instance_id);
        crate::commands::monitoring::remove_instance(instance_id);
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
                self.state
                    .lock()
                    .unwrap()
                    .desired_instances
                    .remove(instance_id);
            }
            return self.persist();
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
        let running = self.state.lock().unwrap().running.clone();
        let mut failures = Vec::new();
        for instance_id in running.keys() {
            if let Err(error) =
                self.stop_instance_locked(instance_id, preserve_desired, telemetry_reason)
            {
                failures.push(error);
            }
        }
        failures
    }

    fn restore_missing_desired_instances(self: &Arc<Self>) -> Vec<String> {
        let desired = self.state.lock().unwrap().desired_instances.clone();
        let mut failures = Vec::new();
        for (instance_id, spec) in desired {
            let is_running = self
                .state
                .lock()
                .unwrap()
                .running
                .get(&instance_id)
                .is_some_and(running_instance_matches_live_process);
            if !is_running {
                if let Err(error) = self.start_instance(spec) {
                    failures.push(format!("failed to restore instance {instance_id}: {error}"));
                }
            }
        }
        failures
    }

    fn record_proxy_runtime_error(&self, message: String) {
        self.proxy_status.lock().unwrap().last_error = Some(message.clone());
        *self.last_error.lock().unwrap() = Some(message);
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

    fn schedule_proxy_restart(self: &Arc<Self>) {
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

        let config = self.state.lock().unwrap().proxy_config.clone();
        let host = config.host.trim();
        let local = matches!(host, "" | "localhost" | "127.0.0.1" | "::1" | "[::1]");
        if !local && config.public_api_key.trim().is_empty() {
            return Err("代理监听非本机地址时必须设置公开 API Key".into());
        }
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
        let supervisor = self.clone();
        let task = tokio::spawn(async move {
            let result = axum::serve(listener, proxy_router_from_source(source))
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
                Ok(RuntimeReply::Status(Box::new(
                    self.status(registered_for_login),
                )))
            }
            RuntimeCommand::StartInstance { spec } => self
                .start_instance(*spec)
                .map(Box::new)
                .map(RuntimeReply::Instance),
            RuntimeCommand::StopInstance { instance_id } => {
                self.stop_instance(&instance_id)?;
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
        gui_owner_is_alive, should_stop_for_missing_gui, validate_runtime_state, GuiOwner,
    };
    use crate::commands::server::read_process_identity;
    use crate::runtime_service::protocol::{PersistedRuntimeState, RUNTIME_STATE_SCHEMA_VERSION};

    #[test]
    fn watchdog_only_stops_gui_bound_runtime_after_heartbeat_expiry() {
        assert!(!should_stop_for_missing_gui(false, false));
        assert!(should_stop_for_missing_gui(false, true));
        assert!(!should_stop_for_missing_gui(true, false));
        assert!(!should_stop_for_missing_gui(true, true));
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
}
