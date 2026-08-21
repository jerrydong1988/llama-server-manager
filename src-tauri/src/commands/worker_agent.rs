use crate::commands::cluster::update_workers;
use crate::models::{AppState, WorkerInfo, WorkerOrigin, WorkerStatus};
use crate::worker_agent::{
    certificate_sha256, ensure_manager_bridge, get_remote_audit, get_remote_status,
    protect_private_token, replace_manager_bridge_connection, start_remote_rpc,
    stop_manager_bridge, stop_remote_rpc, validate_private_token, WorkerAgentAuditEntry,
    WorkerAgentConnection, WorkerAgentEnrollment, WorkerAgentStatus, AGENT_PROTOCOL_VERSION,
};
use tauri::{Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

static AGENT_ENROLLMENT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn validate_endpoint(value: &str, field: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 253
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(format!("{field} is invalid"));
    }
    Ok(value.to_string())
}

fn canonical_enrollment_file(value: &str, field: &str) -> Result<std::path::PathBuf, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 32_768 || value.chars().any(char::is_control) {
        return Err(format!("{field} path is invalid"));
    }
    let path = std::path::PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("{field} path must be absolute"));
    }
    let canonical = std::fs::canonicalize(&path)
        .map_err(|error| format!("{field} path is unavailable: {error}"))?;
    if !canonical.is_file() {
        return Err(format!("{field} path is not a regular file"));
    }
    Ok(canonical)
}

async fn confirm_agent_enrollment(
    app: &tauri::AppHandle,
    enrollment: &WorkerAgentEnrollment,
    tls_cert_path: &std::path::Path,
    token_path: &std::path::Path,
) -> Result<(), String> {
    let local_bridge = if enrollment.local_port == 0 {
        "127.0.0.1:<automatic>".to_string()
    } else {
        format!("127.0.0.1:{}", enrollment.local_port)
    };
    let message = format!(
        "确认登记 Secure Worker Agent？ / Enroll this Secure Worker Agent?\n\nControl: {}:{}\nTunnel: {}:{}\nTLS server name: {}\nCertificate: {}\nPrivate token: {}\nLocal raw RPC bridge: {}\n\n继续后，后端会读取私有令牌、把其权限收紧到当前用户，并仅通过上述 TLS 端点发送。原始 RPC 桥接可被本机所有进程访问，因此仅可在没有不受信任本地用户的专用管理账户/主机上使用。\n\nContinuing lets the backend read the private token, restrict it to the current user, and send it only to the TLS endpoint above. Every local process can access the raw RPC bridge, so use this only from a dedicated manager account/host without untrusted local users.",
        enrollment.control_host,
        enrollment.control_port,
        enrollment.tunnel_host,
        enrollment.tunnel_port,
        enrollment.tls_server_name,
        tls_cert_path.display(),
        token_path.display(),
        local_bridge,
    );
    let app = app.clone();
    let approved = tokio::task::spawn_blocking(move || {
        app.dialog()
            .message(message)
            .title("确认 Secure Worker Agent / Confirm enrollment")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "登记 / Enroll".to_string(),
                "取消 / Cancel".to_string(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|error| format!("Agent approval dialog failed: {error}"))?;
    if approved {
        Ok(())
    } else {
        Err("Secure Worker Agent enrollment was not approved".to_string())
    }
}

fn agent_connection(worker: &WorkerInfo) -> Result<WorkerAgentConnection, String> {
    if worker.origin != WorkerOrigin::Agent {
        return Err("Worker is not managed by a secure Agent".to_string());
    }
    worker
        .agent
        .clone()
        .ok_or_else(|| "Secure Worker Agent metadata is missing".to_string())
}

fn worker_by_id(state: &AppState, id: &str) -> Result<WorkerInfo, String> {
    state
        .workers
        .lock()
        .map_err(|_| "worker state lock is poisoned".to_string())?
        .iter()
        .find(|worker| worker.id == id)
        .cloned()
        .ok_or_else(|| "Worker not found".to_string())
}

fn apply_agent_status(worker: &mut WorkerInfo, status: &WorkerAgentStatus) {
    worker.devices = status.devices.clone();
    worker.status = if status.rpc_running {
        WorkerStatus::Online
    } else {
        WorkerStatus::Offline
    };
    worker.last_seen = Some(chrono::Utc::now().to_rfc3339());
}

async fn verified_status(connection: &WorkerAgentConnection) -> Result<WorkerAgentStatus, String> {
    let status = get_remote_status(connection).await?;
    if status.protocol_version != AGENT_PROTOCOL_VERSION {
        return Err("Secure Worker Agent protocol version mismatch".to_string());
    }
    if !connection.agent_id.is_empty() && status.agent_id != connection.agent_id {
        return Err("Secure Worker Agent identity changed".to_string());
    }
    if status.certificate_sha256 != connection.certificate_sha256 {
        return Err("Secure Worker Agent certificate identity changed".to_string());
    }
    Ok(status)
}

pub async fn enroll_worker_agent(
    mut enrollment: WorkerAgentEnrollment,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<WorkerInfo, String> {
    enrollment.control_host = validate_endpoint(&enrollment.control_host, "Agent control host")?;
    enrollment.tunnel_host = validate_endpoint(&enrollment.tunnel_host, "Agent tunnel host")?;
    enrollment.tls_server_name =
        validate_endpoint(&enrollment.tls_server_name, "Agent TLS server name")?;
    let name = enrollment.name.trim();
    if name.len() > 128 || name.chars().any(char::is_control) {
        return Err("Agent name is invalid".to_string());
    }
    enrollment.name = name.to_string();
    let tls_cert_path = canonical_enrollment_file(&enrollment.tls_cert_path, "Agent certificate")?;
    let token_path = canonical_enrollment_file(&enrollment.token_path, "Agent token")?;
    if enrollment.control_port == 0 || enrollment.tunnel_port == 0 {
        return Err("Agent control and tunnel ports must be non-zero".to_string());
    }
    if enrollment.control_host == enrollment.tunnel_host
        && enrollment.control_port == enrollment.tunnel_port
    {
        return Err("Agent control and tunnel endpoints must be distinct".to_string());
    }
    let _enrollment_guard = AGENT_ENROLLMENT_LOCK.lock().await;
    confirm_agent_enrollment(&app, &enrollment, &tls_cert_path, &token_path).await?;
    validate_private_token(&token_path)?;
    protect_private_token(&token_path)?;
    let fingerprint = certificate_sha256(&tls_cert_path)?;
    let mut connection = WorkerAgentConnection {
        agent_id: String::new(),
        control_host: enrollment.control_host.trim().to_string(),
        control_port: enrollment.control_port,
        tunnel_host: enrollment.tunnel_host.trim().to_string(),
        tunnel_port: enrollment.tunnel_port,
        tls_server_name: enrollment.tls_server_name.trim().to_string(),
        tls_cert_path,
        token_path,
        certificate_sha256: fingerprint,
    };
    let status = verified_status(&connection).await?;
    connection.agent_id = status.agent_id.clone();
    let worker_id = format!("agent-{}", status.agent_id);
    let existing = state
        .workers
        .lock()
        .map_err(|_| "worker state lock is poisoned".to_string())?
        .iter()
        .find(|worker| worker.id == worker_id)
        .cloned();
    let requested_port = if enrollment.local_port == 0 {
        existing.as_ref().map(|worker| worker.port).unwrap_or(0)
    } else {
        enrollment.local_port
    };
    let mut previous_bridge_connection = None;
    let mut bridge_to_restore = None;
    let local_port = if existing
        .as_ref()
        .is_some_and(|worker| worker.port == requested_port)
    {
        match replace_manager_bridge_connection(&worker_id, connection.clone(), requested_port)? {
            Some(previous) => {
                previous_bridge_connection = Some(previous);
                requested_port
            }
            None => ensure_manager_bridge(&worker_id, connection.clone(), requested_port).await?,
        }
    } else {
        if let Some(existing) = &existing {
            bridge_to_restore = existing
                .agent
                .clone()
                .map(|connection| (connection, existing.port));
            let _ = stop_manager_bridge(&existing.id).await;
        }
        ensure_manager_bridge(&worker_id, connection.clone(), requested_port).await?
    };
    let mut worker = WorkerInfo {
        id: worker_id.clone(),
        host: "127.0.0.1".to_string(),
        port: local_port,
        name: if enrollment.name.trim().is_empty() {
            status.name.clone()
        } else {
            enrollment.name.trim().to_string()
        },
        origin: WorkerOrigin::Agent,
        devices: status.devices.clone(),
        status: if status.rpc_running {
            WorkerStatus::Online
        } else {
            WorkerStatus::Offline
        },
        last_seen: Some(chrono::Utc::now().to_rfc3339()),
        auto_discovered: false,
        agent: Some(connection),
    };
    if !enrollment.name.trim().is_empty() {
        worker.name = enrollment.name.trim().to_string();
    }
    let persisted = update_workers(&state, |workers| {
        if workers.iter().any(|candidate| {
            candidate.id != worker_id
                && candidate
                    .agent
                    .as_ref()
                    .is_some_and(|agent| agent.agent_id == status.agent_id)
        }) {
            return Err("Secure Worker Agent is already enrolled".to_string());
        }
        if let Some(existing) = workers
            .iter_mut()
            .find(|candidate| candidate.id == worker_id)
        {
            *existing = worker.clone();
        } else {
            workers.push(worker.clone());
        }
        Ok(worker.clone())
    });
    match persisted {
        Ok(result) => result,
        Err(error) => {
            if let Some(previous) = previous_bridge_connection {
                let _ = replace_manager_bridge_connection(&worker_id, previous, local_port);
            } else {
                let _ = stop_manager_bridge(&worker_id).await;
                if let Some((previous, previous_port)) = bridge_to_restore {
                    if let Err(restore_error) =
                        ensure_manager_bridge(&worker_id, previous, previous_port).await
                    {
                        eprintln!(
                            "Failed to restore Secure Worker Agent bridge after persistence error: {restore_error}"
                        );
                    }
                }
            }
            Err(error)
        }
    }
}

pub async fn test_worker_agent(
    id: String,
    state: State<'_, AppState>,
) -> Result<WorkerAgentStatus, String> {
    let worker = worker_by_id(&state, &id)?;
    let connection = agent_connection(&worker)?;
    let status = match verified_status(&connection).await {
        Ok(status) => status,
        Err(error) => {
            update_workers(&state, |workers| {
                if let Some(worker) = workers.iter_mut().find(|worker| worker.id == id) {
                    worker.status = WorkerStatus::Offline;
                }
            })?;
            return Err(error);
        }
    };
    ensure_manager_bridge(&worker.id, connection, worker.port).await?;
    update_workers(&state, |workers| {
        if let Some(worker) = workers.iter_mut().find(|worker| worker.id == id) {
            apply_agent_status(worker, &status);
        }
    })?;
    Ok(status)
}

pub async fn start_worker_agent(
    id: String,
    state: State<'_, AppState>,
) -> Result<WorkerAgentStatus, String> {
    let worker = worker_by_id(&state, &id)?;
    let connection = agent_connection(&worker)?;
    let status = start_remote_rpc(&connection).await?;
    if status.agent_id != connection.agent_id
        || status.certificate_sha256 != connection.certificate_sha256
    {
        return Err("Secure Worker Agent identity changed during start".to_string());
    }
    ensure_manager_bridge(&worker.id, connection, worker.port).await?;
    update_workers(&state, |workers| {
        if let Some(worker) = workers.iter_mut().find(|worker| worker.id == id) {
            apply_agent_status(worker, &status);
        }
    })?;
    Ok(status)
}

pub async fn stop_worker_agent(
    id: String,
    state: State<'_, AppState>,
) -> Result<WorkerAgentStatus, String> {
    let worker = worker_by_id(&state, &id)?;
    let connection = agent_connection(&worker)?;
    let status = stop_remote_rpc(&connection).await?;
    update_workers(&state, |workers| {
        if let Some(worker) = workers.iter_mut().find(|worker| worker.id == id) {
            apply_agent_status(worker, &status);
        }
    })?;
    Ok(status)
}

pub async fn list_worker_agent_audit(
    id: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<WorkerAgentAuditEntry>, String> {
    let worker = worker_by_id(&state, &id)?;
    get_remote_audit(&agent_connection(&worker)?, limit).await
}

pub async fn restore_worker_agent_bridges(app: tauri::AppHandle) {
    let state = app.state::<AppState>();
    let workers = state
        .workers
        .lock()
        .map(|workers| {
            workers
                .iter()
                .filter(|worker| worker.origin == WorkerOrigin::Agent)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for worker in workers {
        let result = async {
            let connection = agent_connection(&worker)?;
            ensure_manager_bridge(&worker.id, connection.clone(), worker.port).await?;
            verified_status(&connection).await
        }
        .await;
        match result {
            Ok(status) => {
                let _ = update_workers(&state, |workers| {
                    if let Some(candidate) = workers.iter_mut().find(|item| item.id == worker.id) {
                        apply_agent_status(candidate, &status);
                    }
                });
            }
            Err(error) => {
                eprintln!(
                    "Secure Worker Agent restoration failed for {}: {error}",
                    worker.id
                );
                let _ = update_workers(&state, |workers| {
                    if let Some(candidate) = workers.iter_mut().find(|item| item.id == worker.id) {
                        candidate.status = WorkerStatus::Offline;
                    }
                });
            }
        }
    }
}

pub mod ipc {
    use super::*;

    #[tauri::command]
    pub async fn enroll_worker_agent(
        enrollment: WorkerAgentEnrollment,
        state: State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<WorkerInfo> {
        super::enroll_worker_agent(enrollment, state, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn test_worker_agent(
        id: String,
        state: State<'_, AppState>,
    ) -> crate::error::AppResult<WorkerAgentStatus> {
        super::test_worker_agent(id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn start_worker_agent(
        id: String,
        state: State<'_, AppState>,
    ) -> crate::error::AppResult<WorkerAgentStatus> {
        super::start_worker_agent(id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn stop_worker_agent(
        id: String,
        state: State<'_, AppState>,
    ) -> crate::error::AppResult<WorkerAgentStatus> {
        super::stop_worker_agent(id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn list_worker_agent_audit(
        id: String,
        limit: usize,
        state: State<'_, AppState>,
    ) -> crate::error::AppResult<Vec<WorkerAgentAuditEntry>> {
        super::list_worker_agent_audit(id, limit, state)
            .await
            .map_err(crate::error::AppError::from)
    }
}
