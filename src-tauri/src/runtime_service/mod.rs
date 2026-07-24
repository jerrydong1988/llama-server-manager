pub mod autostart;
pub mod protocol;
mod supervisor;
mod transport;

use fs2::FileExt;
use protocol::{
    RuntimeCommand, RuntimeReply, RuntimeRequest, RuntimeResponse, RuntimeServiceStatus,
    BACKGROUND_DETACH_CAPABILITY, RUNTIME_ERROR_ACK_CAPABILITY, RUNTIME_PROTOCOL_VERSION,
};
use std::fs::OpenOptions;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use supervisor::{start_watchdog, RuntimeSupervisor};

pub use protocol::RuntimeLaunchSpec;

static CONFIG_REVISION: AtomicU64 = AtomicU64::new(0);
static CONFIG_CHANGE_GENERATION: AtomicU64 = AtomicU64::new(1);
static CONFIG_SYNCED_GENERATION: AtomicU64 = AtomicU64::new(0);
static RUNTIME_START_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static APP_CONFIG_SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const MAX_RUNTIME_SERVICE_LOG_BYTES: u64 = 4 * 1024 * 1024;

fn has_required_runtime_capabilities(status: &RuntimeServiceStatus) -> bool {
    [BACKGROUND_DETACH_CAPABILITY, RUNTIME_ERROR_ACK_CAPABILITY]
        .iter()
        .all(|required| {
            status
                .capabilities
                .iter()
                .any(|capability| capability.as_str() == *required)
        })
}

pub fn next_config_revision() -> u64 {
    let wall_clock = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(1);
    let mut current = CONFIG_REVISION.load(Ordering::Acquire);
    loop {
        let next = wall_clock.max(current.saturating_add(1));
        match CONFIG_REVISION.compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return next,
            Err(actual) => current = actual,
        }
    }
}

pub fn mark_config_sync_pending() -> u64 {
    CONFIG_CHANGE_GENERATION.fetch_add(1, Ordering::AcqRel) + 1
}

pub fn mark_config_sync_complete(generation: u64) {
    CONFIG_SYNCED_GENERATION.fetch_max(generation, Ordering::AcqRel);
}

fn config_sync_pending() -> bool {
    CONFIG_CHANGE_GENERATION.load(Ordering::Acquire)
        > CONFIG_SYNCED_GENERATION.load(Ordering::Acquire)
}

pub fn manages_instances() -> bool {
    true
}

pub fn persisted_managed_instance_ids() -> std::collections::HashSet<String> {
    let primary = transport::runtime_state_path();
    [primary.clone(), primary.with_extension("json.bak")]
        .into_iter()
        .find_map(|path| {
            std::fs::read_to_string(path).ok().and_then(|json| {
                serde_json::from_str::<protocol::PersistedRuntimeState>(&json).ok()
            })
        })
        .map(|state| {
            state
                .running
                .into_keys()
                .chain(state.desired_instances.into_keys())
                .collect()
        })
        .unwrap_or_default()
}

pub async fn start_instance(
    spec: RuntimeLaunchSpec,
) -> Result<crate::models::RunningInstance, String> {
    ensure_runtime_service().await?;
    match call(RuntimeCommand::StartInstance {
        spec: Box::new(spec),
    })
    .await?
    {
        RuntimeReply::Instance(running) => Ok(*running),
        _ => Err("runtime service returned an unexpected instance response".into()),
    }
}

pub async fn stop_instance(instance_id: String) -> Result<(), String> {
    ensure_runtime_service().await?;
    match call(RuntimeCommand::StopInstance { instance_id }).await? {
        RuntimeReply::Ack => Ok(()),
        _ => Err("runtime service returned an unexpected stop response".into()),
    }
}

pub async fn clear_last_error() -> Result<(), String> {
    ensure_runtime_service().await?;
    match call(RuntimeCommand::ClearLastError).await? {
        RuntimeReply::Ack => Ok(()),
        _ => Err("runtime service returned an unexpected error acknowledgement response".into()),
    }
}

pub async fn is_instance_managed(instance_id: &str) -> Result<bool, String> {
    ensure_runtime_service()
        .await
        .map(|status| status.running.contains_key(instance_id))
}

pub async fn start_proxy() -> Result<crate::models::ProxyStatus, String> {
    ensure_runtime_service().await?;
    match call(RuntimeCommand::StartProxy).await? {
        RuntimeReply::ProxyStatus(status) => Ok(status),
        _ => Err("runtime service returned an unexpected proxy response".into()),
    }
}

pub async fn stop_proxy() -> Result<crate::models::ProxyStatus, String> {
    ensure_runtime_service().await?;
    match call(RuntimeCommand::StopProxy).await? {
        RuntimeReply::ProxyStatus(status) => Ok(status),
        _ => Err("runtime service returned an unexpected proxy response".into()),
    }
}

pub async fn set_background_enabled(enabled: bool) -> Result<RuntimeServiceStatus, String> {
    ensure_runtime_service().await?;
    match call(RuntimeCommand::SetBackgroundEnabled { enabled }).await? {
        RuntimeReply::Status(status) => Ok(*status),
        _ => Err("runtime service returned an unexpected background response".into()),
    }
}

pub async fn sync_config(
    revision: u64,
    proxy_config: crate::models::ProxyConfig,
    instances: std::collections::HashMap<String, crate::models::InstanceConfig>,
) -> Result<RuntimeServiceStatus, String> {
    let current = ensure_runtime_service().await?;
    let revision = revision.max(current.config_revision.saturating_add(1));
    match call(RuntimeCommand::SyncConfig {
        revision,
        proxy_config,
        instances,
    })
    .await?
    {
        RuntimeReply::Status(status) => Ok(*status),
        _ => Err("runtime service returned an unexpected configuration response".into()),
    }
}

pub async fn sync_app_config(
    state: &crate::models::AppState,
) -> Result<RuntimeServiceStatus, String> {
    let _sync = APP_CONFIG_SYNC_LOCK.lock().await;
    let proxy_config = state.proxy_config.lock().unwrap().clone();
    let instances = state.instances.lock().unwrap().clone();
    sync_config(next_config_revision(), proxy_config, instances).await
}

pub async fn prepare_background_detach(
    state: &crate::models::AppState,
) -> Result<RuntimeServiceStatus, String> {
    let _sync = APP_CONFIG_SYNC_LOCK.lock().await;
    let current = ensure_runtime_service().await?;
    let gui_running = state.running.lock().unwrap().clone();
    let mut gui_ids = gui_running.keys().cloned().collect::<Vec<_>>();
    let mut runtime_ids = current.running.keys().cloned().collect::<Vec<_>>();
    gui_ids.sort();
    runtime_ids.sort();
    if gui_ids != runtime_ids {
        return Err(format!(
            "后台接管前运行状态尚未同步：界面 [{}]，后台 [{}]；请等待状态刷新后重试",
            gui_ids.join(", "),
            runtime_ids.join(", ")
        ));
    }
    let proxy_config = state.proxy_config.lock().unwrap().clone();
    let instances = state.instances.lock().unwrap().clone();
    let revision = next_config_revision().max(current.config_revision.saturating_add(1));
    match call(RuntimeCommand::PrepareBackgroundDetach {
        revision,
        proxy_config,
        instances,
        // The daemon may have been upgraded and restarted by ensure_runtime_service,
        // so its freshly authenticated process identities are authoritative here.
        expected_running: current.running,
    })
    .await?
    {
        RuntimeReply::Status(status) => Ok(*status),
        _ => Err("runtime service returned an unexpected background detach response".into()),
    }
}

pub async fn heartbeat() -> Result<RuntimeServiceStatus, String> {
    match call(RuntimeCommand::Heartbeat {
        gui_pid: std::process::id(),
    })
    .await?
    {
        RuntimeReply::Status(status) => Ok(*status),
        _ => Err("runtime service returned an unexpected heartbeat response".into()),
    }
}

pub async fn shutdown(stop_instances: bool) -> Result<(), String> {
    match call(RuntimeCommand::Shutdown { stop_instances }).await? {
        RuntimeReply::Ack => Ok(()),
        _ => Err("runtime service returned an unexpected shutdown response".into()),
    }
}

pub fn is_runtime_service_invocation() -> bool {
    std::env::args_os().any(|argument| argument == "--runtime-service")
}

pub fn configure_runtime_data_dir_from_args() -> Result<(), String> {
    let mut arguments = std::env::args_os();
    while let Some(argument) = arguments.next() {
        if argument == "--runtime-data-dir" {
            let path = arguments
                .next()
                .ok_or_else(|| "--runtime-data-dir requires a path".to_string())?;
            return crate::utils::set_data_dir_override(std::path::PathBuf::from(path));
        }
    }
    Ok(())
}

fn is_runtime_autostart_invocation() -> bool {
    std::env::args_os().any(|argument| argument == "--autostart")
}

fn runtime_registration_enabled() -> bool {
    // The cross-process smoke test uses an isolated data directory and must not
    // write a real login item into the developer's OS account.
    #[cfg(debug_assertions)]
    if std::env::var_os("LSM_RUNTIME_TEST_LOGIN_REGISTERED").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        return true;
    }
    autostart::is_runtime_autostart_enabled().unwrap_or(false)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}

fn load_or_create_control_token() -> Result<String, String> {
    let path = transport::control_token_path();
    let parent = path
        .parent()
        .ok_or_else(|| "runtime token path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create runtime directory: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("failed to protect runtime directory: {error}"))?;
    }
    let lock_path = parent.join("control-token.lock");
    let token_lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| format!("failed to open runtime token lock: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("failed to protect runtime token lock: {error}"))?;
    }
    let lock_deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        match FileExt::try_lock_exclusive(&token_lock) {
            Ok(()) => break,
            Err(error)
                if error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
                    && std::time::Instant::now() < lock_deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                return Err("timed out acquiring runtime token lock".into());
            }
            Err(error) => {
                return Err(format!("failed to acquire runtime token lock: {error}"));
            }
        }
    }

    match std::fs::read_to_string(&path) {
        Ok(token) if token.trim().len() >= 32 => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                    .map_err(|error| format!("failed to protect runtime control token: {error}"))?;
            }
            return Ok(token.trim().to_string());
        }
        Ok(_) => std::fs::remove_file(&path)
            .map_err(|error| format!("failed to replace invalid runtime control token: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("failed to read runtime control token: {error}")),
    }
    let token = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|error| format!("failed to create runtime control token: {error}"))?;
    file.write_all(token.as_bytes())
        .map_err(|error| format!("failed to write runtime control token: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync runtime control token: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("failed to protect runtime control token: {error}"))?;
    }
    Ok(token)
}

fn load_control_token() -> Result<String, String> {
    let token = std::fs::read_to_string(transport::control_token_path())
        .map_err(|error| format!("failed to read runtime control token: {error}"))?;
    let token = token.trim();
    if token.len() < 32 {
        return Err("runtime control token is invalid".into());
    }
    Ok(token.to_string())
}

fn rotate_control_token() -> Result<String, String> {
    let token = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    crate::persistence::atomic_write(&transport::control_token_path(), token.as_bytes(), None)
        .map_err(|error| format!("failed to rotate runtime control token: {error}"))?;
    Ok(token)
}

async fn call_with_token(token: String, command: RuntimeCommand) -> Result<RuntimeReply, String> {
    let request = RuntimeRequest {
        protocol_version: RUNTIME_PROTOCOL_VERSION,
        request_id: uuid::Uuid::new_v4().to_string(),
        token,
        command,
    };
    let response = transport::send_request(&request).await?;
    if let Some(error) = response.error {
        return Err(error);
    }
    response
        .reply
        .ok_or_else(|| "runtime service returned an empty response".to_string())
}

pub async fn call(command: RuntimeCommand) -> Result<RuntimeReply, String> {
    call_with_token(load_control_token()?, command).await
}

pub async fn runtime_status() -> Result<RuntimeServiceStatus, String> {
    match call(RuntimeCommand::GetStatus).await? {
        RuntimeReply::Status(status) => Ok(*status),
        _ => Err("runtime service returned an unexpected status response".into()),
    }
}

#[tauri::command]
pub async fn get_runtime_service_status() -> Result<serde_json::Value, String> {
    ensure_runtime_service()
        .await
        .map(|status| runtime_status_payload(&status, None))
}

#[tauri::command]
pub async fn clear_runtime_service_error() -> Result<(), String> {
    clear_last_error().await
}

async fn ping_with_token(token: String) -> bool {
    matches!(
        call_with_token(token, RuntimeCommand::Ping).await,
        Ok(RuntimeReply::Pong)
    )
}

fn spawn_runtime_process() -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to locate runtime executable: {error}"))?;
    let log_path = transport::service_log_path();
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create runtime log directory: {error}"))?;
    }
    let truncate_log = std::fs::metadata(&log_path)
        .map(|metadata| metadata.len() > MAX_RUNTIME_SERVICE_LOG_BYTES)
        .unwrap_or(false);
    let mut log_options = OpenOptions::new();
    log_options.create(true).write(true);
    if truncate_log {
        log_options.truncate(true);
    } else {
        log_options.append(true);
    }
    let stdout = log_options
        .open(&log_path)
        .map_err(|error| format!("failed to open runtime service log: {error}"))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("failed to clone runtime service log: {error}"))?;
    let mut command = Command::new(executable);
    command
        .arg("--runtime-service")
        .arg("--runtime-data-dir")
        .arg(crate::utils::get_data_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00000200);
    }
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("failed to start runtime service: {error}"))
}

fn append_runtime_startup_failure(error: &str) {
    let log_path = transport::service_log_path();
    let Some(parent) = log_path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let truncate = std::fs::metadata(&log_path)
        .map(|metadata| metadata.len() > MAX_RUNTIME_SERVICE_LOG_BYTES)
        .unwrap_or(false);
    let mut options = OpenOptions::new();
    options.create(true).write(true);
    if truncate {
        options.truncate(true);
    } else {
        options.append(true);
    }
    let Ok(mut log) = options.open(log_path) else {
        return;
    };
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let _ = writeln!(log, "[{timestamp}] runtime service startup failed: {error}");
    let _ = log.sync_all();
}

pub async fn ensure_runtime_service() -> Result<RuntimeServiceStatus, String> {
    let _transition = RUNTIME_START_LOCK.lock().await;
    let mut token = load_or_create_control_token()?;
    let runtime_lock_probe = transport::acquire_runtime_lock()?;
    if runtime_lock_probe.is_none() {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        loop {
            if ping_with_token(token.clone()).await {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(
                    "runtime lock is held but the authenticated service is unavailable".into(),
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            token = load_control_token()?;
        }
        let status = runtime_status().await?;
        if status.service_version == env!("CARGO_PKG_VERSION")
            && has_required_runtime_capabilities(&status)
        {
            return Ok(status);
        }
        shutdown(false).await?;
        if !transport::wait_until_stopped(&token, Duration::from_secs(6)).await {
            return Err(format!(
                "runtime service {} did not stop for upgrade to {}",
                status.service_version,
                env!("CARGO_PKG_VERSION")
            ));
        }
    } else {
        drop(runtime_lock_probe);
    }
    token = rotate_control_token()?;
    spawn_runtime_process()?;
    if !transport::wait_until_ready(&token, Duration::from_secs(8)).await {
        return Err(format!(
            "runtime service did not become ready; inspect {}",
            transport::service_log_path().display()
        ));
    }
    if !ping_with_token(token).await {
        return Err("runtime service rejected the authenticated startup probe".into());
    }
    let status = runtime_status().await?;
    if !has_required_runtime_capabilities(&status) {
        return Err("runtime service started without required capabilities".into());
    }
    Ok(status)
}

pub fn run_runtime_service() -> Result<(), String> {
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("llama-runtime")
        .build()
        .map_err(|error| format!("failed to create runtime service executor: {error}"))
        .and_then(|runtime| {
            runtime.block_on(async {
                let mut token = load_or_create_control_token()?;
                let Some(runtime_lock) = transport::acquire_runtime_lock()? else {
                    return Ok(());
                };
                let supervisor = RuntimeSupervisor::load()?;
                if is_runtime_autostart_invocation() && !supervisor.background_enabled() {
                    return Ok(());
                }
                if is_runtime_autostart_invocation() {
                    token = rotate_control_token()?;
                }
                let registered_for_login =
                    std::sync::Arc::new(AtomicBool::new(runtime_registration_enabled()));
                let supervisor_for_initialization = supervisor.clone();
                transport::run_server(
                    runtime_lock,
                    token.clone(),
                    move || {
                        let supervisor = supervisor_for_initialization.clone();
                        async move {
                            supervisor.restore().await?;
                            start_watchdog(supervisor);
                            Ok(())
                        }
                    },
                    move |request| {
                        let token = token.clone();
                        let supervisor = supervisor.clone();
                        let registered_for_login = registered_for_login.clone();
                        async move {
                            if request.protocol_version != RUNTIME_PROTOCOL_VERSION {
                                return RuntimeResponse::failure(
                                    request.request_id,
                                    format!(
                                        "runtime protocol mismatch: service {}, client {}",
                                        RUNTIME_PROTOCOL_VERSION, request.protocol_version
                                    ),
                                );
                            }
                            if !constant_time_eq(request.token.as_bytes(), token.as_bytes()) {
                                return RuntimeResponse::failure(
                                    request.request_id,
                                    "unauthorized",
                                );
                            }
                            let refresh_registration = matches!(
                                &request.command,
                                RuntimeCommand::SyncConfig { .. }
                                    | RuntimeCommand::PrepareBackgroundDetach { .. }
                                    | RuntimeCommand::SetBackgroundEnabled { .. }
                            );
                            let registered = if refresh_registration {
                                let registered = runtime_registration_enabled();
                                registered_for_login.store(registered, Ordering::Release);
                                registered
                            } else {
                                registered_for_login.load(Ordering::Acquire)
                            };
                            match supervisor.handle_command(request.command, registered).await {
                                Ok(reply) => RuntimeResponse::success(request.request_id, reply),
                                Err(error) => RuntimeResponse::failure(request.request_id, error),
                            }
                        }
                    },
                )
                .await
            })
        });
    if let Err(error) = &result {
        append_runtime_startup_failure(error);
    }
    result
}

fn runtime_status_payload(
    status: &RuntimeServiceStatus,
    previously_managed: Option<&std::collections::HashSet<String>>,
) -> serde_json::Value {
    let running = status
        .running
        .iter()
        .map(|(instance_id, running)| {
            (
                instance_id.clone(),
                serde_json::json!({
                    "pid": running.pid,
                    "host": running.host,
                    "port": running.port,
                    "startTime": running.start_time,
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    serde_json::json!({
        "protocolVersion": status.protocol_version,
        "serviceVersion": status.service_version,
        "servicePid": status.service_pid,
        "capabilities": status.capabilities,
        "configRevision": status.config_revision,
        "backgroundEnabled": status.background_enabled,
        "registeredForLogin": status.registered_for_login,
        "lastError": status.last_error,
        "proxy": status.proxy,
        "running": running,
        "health": status.health,
        "previouslyManaged": previously_managed
            .map(|ids| ids.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default(),
    })
}

pub(crate) fn reconcile_app_runtime(app: &tauri::AppHandle, status: &RuntimeServiceStatus) {
    use tauri::{Emitter, Manager};

    let state = app.state::<crate::models::AppState>();
    let previous_managed = state.runtime_managed_instances.lock().unwrap().clone();
    let next_managed = status
        .running
        .keys()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let mut changed = false;
    let mut reconnect = Vec::new();
    {
        let mut running = state.running.lock().unwrap();
        for instance_id in previous_managed.difference(&next_managed) {
            changed |= running.remove(instance_id).is_some();
            crate::commands::monitoring::remove_instance(instance_id);
        }
        for (instance_id, runtime_instance) in &status.running {
            let differs = running
                .get(instance_id)
                .map_or(true, |current| current.pid != runtime_instance.pid);
            if differs {
                running.insert(instance_id.clone(), runtime_instance.clone());
                changed = true;
            }
            if differs || !previous_managed.contains(instance_id) {
                reconnect.push((instance_id.clone(), runtime_instance.pid));
            }
        }
    }
    // Keep IDs that were managed during this GUI session. The frontend may not
    // have attached its event listener when the first reconciliation arrives;
    // retaining the IDs lets later heartbeats still correct a missed stop event.
    state
        .runtime_managed_instances
        .lock()
        .unwrap()
        .extend(next_managed.iter().cloned());
    *state.proxy_bound_addr.lock().unwrap() = status
        .proxy
        .running
        .then(|| status.proxy.bound_addr.clone());
    *state.proxy_last_error.lock().unwrap() = status.proxy.last_error.clone();

    for (instance_id, pid) in reconnect {
        if crate::commands::server::register_restored_runtime_instance(app, &instance_id, pid) {
            let config_dir = state.config_dir.lock().unwrap().clone();
            crate::commands::server::reconnect_runtime_instance_logs(
                &instance_id,
                pid,
                &config_dir,
                app.clone(),
            );
        }
    }

    for (instance_id, health) in &status.health {
        let _ = app.emit(
            "health-status",
            serde_json::json!({ "instanceId": instance_id, "status": health }),
        );
    }
    for frame in status.monitoring.values() {
        if crate::commands::monitoring::ingest_external_frame(frame.clone()) {
            let _ = app.emit("monitoring-frame", frame);
        }
    }
    for performance in status.performance.values() {
        let _ = app.emit("perf-update", performance);
    }

    if changed {
        let snapshot = state.running.lock().unwrap().clone();
        let _ = crate::commands::config::update_and_persist(&state, |global| {
            global.running = snapshot;
        });
    }
    let _ = app.emit(
        "runtime-service-status",
        runtime_status_payload(status, Some(&previous_managed)),
    );
}

async fn synchronize_from_app(app: &tauri::AppHandle) -> Result<RuntimeServiceStatus, String> {
    use tauri::Manager;

    let state = app.state::<crate::models::AppState>();
    let _proxy_transition = state.proxy_lifecycle_lock.lock().await;
    let sync_generation = CONFIG_CHANGE_GENERATION.load(Ordering::Acquire);
    let proxy_config = state.proxy_config.lock().unwrap().clone();
    sync_app_config(&state).await?;
    let status = if proxy_config.runtime_service_enabled {
        if !autostart::is_runtime_autostart_enabled().unwrap_or(false) {
            autostart::enable_runtime_autostart()?;
        }
        set_background_enabled(true).await?
    } else if autostart::is_runtime_autostart_enabled().unwrap_or(false) {
        set_background_enabled(false).await?;
        autostart::disable_runtime_autostart()?;
        set_background_enabled(false).await?
    } else {
        set_background_enabled(false).await?
    };
    let status = if proxy_config.enabled && !status.proxy.running {
        start_proxy().await?;
        runtime_status().await
    } else if !proxy_config.enabled && status.proxy.running {
        stop_proxy().await?;
        runtime_status().await
    } else {
        Ok(status)
    }?;
    mark_config_sync_complete(sync_generation);
    Ok(status)
}

pub fn start_app_bridge(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut first = true;
        loop {
            let result = if first {
                first = false;
                synchronize_from_app(&app).await
            } else if config_sync_pending() {
                synchronize_from_app(&app).await
            } else {
                match heartbeat().await {
                    Ok(status) => Ok(status),
                    Err(_) => match ensure_runtime_service().await {
                        Ok(_) => synchronize_from_app(&app).await,
                        Err(error) => Err(error),
                    },
                }
            };
            match result {
                Ok(status) => reconcile_app_runtime(&app, &status),
                Err(error) => {
                    use tauri::Emitter;
                    let _ = app.emit(
                        "runtime-service-error",
                        serde_json::json!({ "error": error }),
                    );
                }
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_mode_detection_ignores_normal_test_arguments() {
        assert!(!is_runtime_service_invocation());
    }

    #[test]
    fn token_comparison_rejects_length_and_content_changes() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"different"));
        assert!(!constant_time_eq(b"same", b"sane"));
    }

    #[test]
    fn runtime_event_payload_carries_previously_managed_instances() {
        let status = RuntimeServiceStatus {
            protocol_version: RUNTIME_PROTOCOL_VERSION,
            service_version: "test".into(),
            service_pid: 1,
            capabilities: vec![
                BACKGROUND_DETACH_CAPABILITY.into(),
                RUNTIME_ERROR_ACK_CAPABILITY.into(),
            ],
            config_revision: 1,
            background_enabled: false,
            registered_for_login: false,
            last_error: None,
            proxy: crate::models::ProxyStatus {
                running: false,
                bound_addr: "127.0.0.1:11435".into(),
                active_routes: 0,
                last_error: None,
            },
            running: Default::default(),
            health: Default::default(),
            monitoring: Default::default(),
            performance: Default::default(),
        };
        let previously_managed = ["stale-instance".to_string()].into_iter().collect();
        let payload = runtime_status_payload(&status, Some(&previously_managed));
        assert_eq!(payload["previouslyManaged"][0], "stale-instance");
        assert!(has_required_runtime_capabilities(&status));

        let mut legacy_status = status;
        legacy_status
            .capabilities
            .retain(|capability| capability != RUNTIME_ERROR_ACK_CAPABILITY);
        assert!(!has_required_runtime_capabilities(&legacy_status));
    }
}
