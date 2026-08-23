use crate::commands::cluster::update_workers;
use crate::models::{AppState, WorkerInfo, WorkerOrigin, WorkerStatus};
use crate::worker_agent::{
    certificate_sha256, certificate_sha256_from_pem, get_remote_audit, get_remote_status,
    stop_manager_bridge, stop_remote_rpc, validate_private_token, verify_remote_audit_extension,
    WorkerAgentAuditEntry, WorkerAgentConnection, WorkerAgentEnrollment, WorkerAgentStatus,
    AGENT_PROTOCOL_VERSION,
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

fn read_enrollment_file(
    value: &str,
    field: &str,
    max_bytes: u64,
) -> Result<(std::path::PathBuf, Vec<u8>), String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 32_768 || value.chars().any(char::is_control) {
        return Err(format!("{field} path is invalid"));
    }
    let path = std::path::PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("{field} path must be absolute"));
    }
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|error| format!("{field} path is unavailable: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{field} path is not a regular non-link file"));
    }
    let bytes = crate::persistence::read_regular_file_nofollow_bounded(&path, max_bytes)?
        .ok_or_else(|| format!("{field} path is unavailable"))?;
    let canonical = std::fs::canonicalize(&path)
        .map_err(|error| format!("{field} path is unavailable: {error}"))?;
    Ok((canonical, bytes))
}

struct ImportedEnrollmentCredentials {
    directory: std::path::PathBuf,
    certificate_path: std::path::PathBuf,
    token_path: std::path::PathBuf,
    retain: bool,
}

impl ImportedEnrollmentCredentials {
    fn import(state: &AppState, certificate: &[u8], token: &[u8]) -> Result<Self, String> {
        let directory = state
            .config_dir
            .lock()
            .map_err(|_| "config directory lock is poisoned".to_string())?
            .join("worker-agent-credentials")
            .join(uuid::Uuid::new_v4().simple().to_string());
        crate::persistence::enforce_private_directory(&directory)?;
        let certificate_path = directory.join("agent-cert.pem");
        let token_path = directory.join("agent.token");
        crate::persistence::atomic_write(&certificate_path, certificate, None)?;
        crate::persistence::atomic_write(&token_path, token, None)?;
        crate::persistence::enforce_private_file(&certificate_path)?;
        crate::persistence::enforce_private_file(&token_path)?;
        Ok(Self {
            directory,
            certificate_path,
            token_path,
            retain: false,
        })
    }

    fn retain(&mut self) {
        self.retain = true;
    }
}

impl Drop for ImportedEnrollmentCredentials {
    fn drop(&mut self) {
        if !self.retain {
            let _ = std::fs::remove_file(&self.certificate_path);
            let _ = std::fs::remove_file(&self.token_path);
            let _ = std::fs::remove_dir(&self.directory);
        }
    }
}

async fn confirm_agent_enrollment(
    app: &tauri::AppHandle,
    enrollment: &WorkerAgentEnrollment,
    tls_cert_path: &std::path::Path,
    token_path: &std::path::Path,
    certificate_fingerprint: &str,
) -> Result<(), String> {
    let local_bridge = if enrollment.local_port == 0 {
        "127.0.0.1:<automatic>".to_string()
    } else {
        format!("127.0.0.1:{}", enrollment.local_port)
    };
    let message = format!(
        "确认登记 Secure Worker Agent？ / Enroll this Secure Worker Agent?\n\nControl: {}:{}\nTunnel: {}:{}\nTLS server name: {}\nCertificate: {}\nCertificate SHA-256: {}\nPrivate token: {}\nLocal RPC bridge: {}\n\n继续后，后端会把已核验的证书与令牌内容导入应用私有目录，并仅通过上述 TLS 端点发送令牌。本机桥接只接受由管理器启动且 PID、启动时间和可执行文件身份仍匹配的 llama-server 进程。\n\nContinuing imports the verified certificate and token bytes into application-owned private storage and sends the token only to the TLS endpoint above. The local bridge accepts only manager-launched llama-server processes whose PID, start time, and executable identity still match.",
        enrollment.control_host,
        enrollment.control_port,
        enrollment.tunnel_host,
        enrollment.tunnel_port,
        enrollment.tls_server_name,
        tls_cert_path.display(),
        certificate_fingerprint,
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
        Ok::<(), String>(())
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
    let (selected_tls_cert_path, certificate_bytes) =
        read_enrollment_file(&enrollment.tls_cert_path, "Agent certificate", 256 * 1024)?;
    let (selected_token_path, token_bytes) =
        read_enrollment_file(&enrollment.token_path, "Agent token", 256)?;
    let token_text = std::str::from_utf8(&token_bytes)
        .map_err(|_| "Agent token is not UTF-8".to_string())?
        .trim();
    if token_text.len() != 64 || !token_text.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err("Agent token must contain exactly 64 hexadecimal characters".to_string());
    }
    let fingerprint = certificate_sha256_from_pem(&certificate_bytes)?;
    if enrollment.control_port == 0 || enrollment.tunnel_port == 0 {
        return Err("Agent control and tunnel ports must be non-zero".to_string());
    }
    if enrollment.control_host == enrollment.tunnel_host
        && enrollment.control_port == enrollment.tunnel_port
    {
        return Err("Agent control and tunnel endpoints must be distinct".to_string());
    }
    let _enrollment_guard = AGENT_ENROLLMENT_LOCK.lock().await;
    confirm_agent_enrollment(
        &app,
        &enrollment,
        &selected_tls_cert_path,
        &selected_token_path,
        &fingerprint,
    )
    .await?;
    let mut imported =
        ImportedEnrollmentCredentials::import(&state, &certificate_bytes, token_text.as_bytes())?;
    let tls_cert_path = imported.certificate_path.clone();
    let token_path = imported.token_path.clone();
    validate_private_token(&token_path)?;
    if certificate_sha256(&tls_cert_path)? != fingerprint {
        return Err("Imported Agent certificate identity changed".to_string());
    }
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
        audit_sequence: 0,
        audit_hash: String::new(),
    };
    let status = verified_status(&connection).await?;
    let parsed_agent_id = uuid::Uuid::parse_str(&status.agent_id)
        .map_err(|_| "Secure Worker Agent returned a non-UUID identity".to_string())?;
    if parsed_agent_id.is_nil() {
        return Err("Secure Worker Agent returned an empty identity".to_string());
    }
    connection.agent_id = status.agent_id.clone();
    let worker_id = format!("agent-{}", status.agent_id);
    let existing = state
        .workers
        .lock()
        .map_err(|_| "worker state lock is poisoned".to_string())?
        .iter()
        .find(|worker| worker.id == worker_id)
        .cloned();
    let (checkpoint_sequence, checkpoint_hash) = existing
        .as_ref()
        .and_then(|worker| worker.agent.as_ref())
        .map(|agent| (agent.audit_sequence, agent.audit_hash.clone()))
        .unwrap_or((0, String::new()));
    connection.audit_sequence = checkpoint_sequence;
    connection.audit_hash = checkpoint_hash.clone();
    let enrollment_audit = get_remote_audit(&connection, 500).await?;
    let (audit_sequence, audit_hash) = verify_remote_audit_extension(
        &connection.agent_id,
        checkpoint_sequence,
        &checkpoint_hash,
        &enrollment_audit,
    )?;
    connection.audit_sequence = audit_sequence;
    connection.audit_hash = audit_hash;
    if let Some(existing) = existing.as_ref() {
        let same_connection = existing.agent.as_ref().is_some_and(|previous| {
            let same_token =
                crate::persistence::read_private_file_bounded(&previous.token_path, 256)
                    .ok()
                    .flatten()
                    .is_some_and(|bytes| bytes.as_slice() == token_text.as_bytes());
            previous.agent_id == connection.agent_id
                && previous.control_host == connection.control_host
                && previous.control_port == connection.control_port
                && previous.tunnel_host == connection.tunnel_host
                && previous.tunnel_port == connection.tunnel_port
                && previous.tls_server_name == connection.tls_server_name
                && previous.certificate_sha256 == connection.certificate_sha256
                && same_token
        });
        let same_requested_port =
            enrollment.local_port == 0 || enrollment.local_port == existing.port;
        if !same_connection || !same_requested_port {
            return Err(
                "Secure Worker Agent identity is already enrolled with different endpoint, certificate, token, or bridge metadata; use an explicit replacement workflow"
                    .to_string(),
            );
        }
    }
    let local_port = if enrollment.local_port == 0 {
        existing.as_ref().map(|worker| worker.port).unwrap_or(0)
    } else {
        enrollment.local_port
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
        Ok(result) => match result {
            Ok(worker) => {
                let _ = stop_manager_bridge(&worker_id).await;
                imported.retain();
                Ok(worker)
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
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
    let _ = stop_manager_bridge(&worker.id).await;
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
    let _connection = agent_connection(&worker)?;
    let _ = stop_manager_bridge(&worker.id).await;
    Err(
        "Secure Worker Agent compute startup is disabled because current upstream rpc-server cannot expose an authenticated or OS-private child transport"
            .to_string(),
    )
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
    let connection = agent_connection(&worker)?;
    let entries = get_remote_audit(&connection, 500).await?;
    let (audit_sequence, audit_hash) = verify_remote_audit_extension(
        &connection.agent_id,
        connection.audit_sequence,
        &connection.audit_hash,
        &entries,
    )?;
    update_workers(&state, |workers| {
        let worker = workers
            .iter_mut()
            .find(|worker| worker.id == id)
            .ok_or_else(|| "Worker not found".to_string())?;
        let agent = worker
            .agent
            .as_mut()
            .ok_or_else(|| "Secure Worker Agent metadata is missing".to_string())?;
        agent.audit_sequence = audit_sequence;
        agent.audit_hash = audit_hash.clone();
        Ok::<(), String>(())
    })??;
    let start = entries.len().saturating_sub(limit.clamp(1, 500));
    Ok(entries[start..].to_vec())
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
        let _ = stop_manager_bridge(&worker.id).await;
        let result = async {
            let connection = agent_connection(&worker)?;
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
    ) -> crate::error::AppResult<crate::models::FrontendWorkerInfo> {
        super::enroll_worker_agent(enrollment, state, app)
            .await
            .map(|worker| crate::models::FrontendWorkerInfo::from(&worker))
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
