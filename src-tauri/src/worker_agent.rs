use crate::models::WorkerDevice;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{BufReader as StdBufReader, Read as _, Write as _};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Once};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch, Semaphore};
use tokio_rustls::{TlsAcceptor, TlsConnector};

pub const AGENT_PROTOCOL_VERSION: u32 = 1;
pub const AGENT_CONFIG_SCHEMA_VERSION: u32 = 1;
const MAX_CONTROL_FRAME_BYTES: usize = 64 * 1024;
const MAX_AUDIT_RESULTS: usize = 500;
const MAX_AUDIT_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_IN_FLIGHT_CONNECTIONS: usize = 64;
const AUTH_FAILURE_AUDIT_INTERVAL: Duration = Duration::from_secs(60);
const CONTROL_TIMEOUT: Duration = Duration::from_secs(10);

const HELP: &str = "Llama Server Manager secure Worker Agent

Usage:
  lsm worker-agent init --config PATH --name NAME --control ADDRESS --tunnel ADDRESS
      --advertise-host HOST --tls-cert PATH --tls-key PATH --rpc-binary PATH
      [--rpc-port PORT] [--token-file PATH] [--audit-file PATH] [--rpc-log PATH]
  lsm worker-agent serve --config PATH
  lsm worker-agent rotate-token --config PATH
  lsm worker-agent inspect --config PATH
  lsm worker-agent help

Security model:
  The Agent accepts only status, rpc_start, rpc_stop, and audit actions over TLS.
  The configured rpc-server path, arguments, environment, and filesystem are never
  supplied by a remote request. The bearer credential is read from a private file
  and is never accepted as a CLI argument or emitted in output.";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerAgentConfig {
    pub schema_version: u32,
    pub agent_id: String,
    pub name: String,
    pub control_listen: SocketAddr,
    pub tunnel_listen: SocketAddr,
    pub advertise_host: String,
    pub tls_cert_path: PathBuf,
    pub tls_key_path: PathBuf,
    pub token_path: PathBuf,
    pub rpc_binary_path: PathBuf,
    pub rpc_port: u16,
    pub audit_path: PathBuf,
    pub rpc_log_path: PathBuf,
    #[serde(default)]
    pub devices: Vec<WorkerDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerAgentConnection {
    pub agent_id: String,
    pub control_host: String,
    pub control_port: u16,
    pub tunnel_host: String,
    pub tunnel_port: u16,
    pub tls_server_name: String,
    pub tls_cert_path: PathBuf,
    pub token_path: PathBuf,
    pub certificate_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerAgentEnrollment {
    pub name: String,
    pub control_host: String,
    pub control_port: u16,
    pub tunnel_host: String,
    pub tunnel_port: u16,
    pub tls_server_name: String,
    pub tls_cert_path: String,
    pub token_path: String,
    #[serde(default)]
    pub local_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerAgentStatus {
    pub protocol_version: u32,
    pub agent_id: String,
    pub name: String,
    pub rpc_running: bool,
    pub rpc_port: u16,
    pub tunnel_port: u16,
    pub certificate_sha256: String,
    pub devices: Vec<WorkerDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerAgentAuditEntry {
    pub sequence: u64,
    pub timestamp: String,
    pub agent_id: String,
    pub event: String,
    pub outcome: String,
    pub detail: String,
    pub previous_hash: String,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlRequest {
    protocol_version: u32,
    token: String,
    #[serde(default)]
    expected_agent_id: String,
    action: ControlAction,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ControlAction {
    Status,
    RpcStart,
    RpcStop,
    Audit,
}

impl ControlAction {
    fn name(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::RpcStart => "rpc_start",
            Self::RpcStop => "rpc_stop",
            Self::Audit => "audit",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlError {
    code: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlResponse {
    protocol_version: u32,
    agent_id: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<WorkerAgentStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    audit: Vec<WorkerAgentAuditEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ControlError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TunnelHello {
    protocol_version: u32,
    token: String,
    expected_agent_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TunnelResponse {
    protocol_version: u32,
    agent_id: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ControlError>,
}

struct AgentRuntime {
    config: WorkerAgentConfig,
    certificate_sha256: String,
    rpc_child: Mutex<Option<Child>>,
    audit_lock: Mutex<()>,
    auth_failure_audit: Mutex<AuthFailureAuditState>,
    #[cfg(test)]
    rpc_test_override: AtomicBool,
}

#[derive(Default)]
struct AuthFailureAuditState {
    last_recorded: Option<Instant>,
    suppressed: u64,
}

struct BridgeHandle {
    port: u16,
    connection: Arc<Mutex<WorkerAgentConnection>>,
    generation: uuid::Uuid,
    shutdown: watch::Sender<bool>,
    stopped: oneshot::Receiver<()>,
}

static AGENT_BRIDGES: LazyLock<Mutex<HashMap<String, BridgeHandle>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static TLS_PROVIDER_INIT: Once = Once::new();

fn worker_agent_error(message: impl Into<String>) -> String {
    format!("Worker Agent: {}", message.into())
}

fn ensure_tls_provider() -> Result<(), String> {
    TLS_PROVIDER_INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    if rustls::crypto::CryptoProvider::get_default().is_some() {
        Ok(())
    } else {
        Err(worker_agent_error(
            "failed to initialize TLS crypto provider",
        ))
    }
}

fn validate_host(value: &str, field: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 253
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(worker_agent_error(format!("{field} is invalid")));
    }
    Ok(())
}

fn validate_display_name(value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(worker_agent_error("name is invalid"));
    }
    Ok(())
}

fn require_absolute_path(path: &Path, field: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(worker_agent_error(format!(
            "{field} must be an absolute path"
        )));
    }
    Ok(())
}

fn expected_rpc_binary_name() -> &'static str {
    if cfg!(windows) {
        "rpc-server.exe"
    } else {
        "rpc-server"
    }
}

fn validate_rpc_binary(path: &Path) -> Result<PathBuf, String> {
    require_absolute_path(path, "rpc_binary_path")?;
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| worker_agent_error(format!("rpc-server path is unavailable: {error}")))?;
    if !canonical.is_file()
        || !canonical
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(expected_rpc_binary_name()))
    {
        return Err(worker_agent_error(format!(
            "rpc_binary_path must resolve to {}",
            expected_rpc_binary_name()
        )));
    }
    Ok(canonical)
}

fn validate_agent_config(config: &mut WorkerAgentConfig) -> Result<(), String> {
    if config.schema_version != AGENT_CONFIG_SCHEMA_VERSION {
        return Err(worker_agent_error(format!(
            "unsupported config schema version {}",
            config.schema_version
        )));
    }
    uuid::Uuid::parse_str(&config.agent_id)
        .map_err(|_| worker_agent_error("agent_id must be a UUID"))?;
    validate_display_name(&config.name)?;
    validate_host(&config.advertise_host, "advertise_host")?;
    if config.control_listen.port() == 0 || config.tunnel_listen.port() == 0 || config.rpc_port == 0
    {
        return Err(worker_agent_error("ports must be between 1 and 65535"));
    }
    if config.control_listen == config.tunnel_listen {
        return Err(worker_agent_error(
            "control and tunnel listeners must be distinct",
        ));
    }
    for (path, field) in [
        (&config.tls_cert_path, "tls_cert_path"),
        (&config.tls_key_path, "tls_key_path"),
        (&config.token_path, "token_path"),
        (&config.audit_path, "audit_path"),
        (&config.rpc_log_path, "rpc_log_path"),
    ] {
        require_absolute_path(path, field)?;
    }
    config.rpc_binary_path = validate_rpc_binary(&config.rpc_binary_path)?;
    let _ = server_tls_config(config)?;
    let _ = load_token(&config.token_path)?;
    Ok(())
}

pub fn load_agent_config(path: &Path) -> Result<WorkerAgentConfig, String> {
    require_absolute_path(path, "config")?;
    let bytes = std::fs::read(path)
        .map_err(|error| worker_agent_error(format!("failed to read config: {error}")))?;
    let mut config: WorkerAgentConfig = serde_json::from_slice(&bytes)
        .map_err(|error| worker_agent_error(format!("invalid config: {error}")))?;
    validate_agent_config(&mut config)?;
    Ok(config)
}

fn load_certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    require_absolute_path(path, "TLS certificate path")?;
    let file = File::open(path)
        .map_err(|error| worker_agent_error(format!("failed to read TLS certificate: {error}")))?;
    let mut reader = StdBufReader::new(file);
    let certificates = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| worker_agent_error(format!("invalid TLS certificate: {error}")))?;
    if certificates.is_empty() {
        return Err(worker_agent_error(
            "TLS certificate file contains no certificates",
        ));
    }
    Ok(certificates)
}

fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>, String> {
    require_absolute_path(path, "TLS private key path")?;
    let file = File::open(path)
        .map_err(|error| worker_agent_error(format!("failed to read TLS private key: {error}")))?;
    let mut reader = StdBufReader::new(file);
    rustls_pemfile::private_key(&mut reader)
        .map_err(|error| worker_agent_error(format!("invalid TLS private key: {error}")))?
        .ok_or_else(|| worker_agent_error("TLS private key file contains no supported key"))
}

pub fn certificate_sha256(path: &Path) -> Result<String, String> {
    let certificate = load_certificates(path)?
        .into_iter()
        .next()
        .ok_or_else(|| worker_agent_error("TLS certificate file is empty"))?;
    Ok(format!("{:x}", Sha256::digest(certificate.as_ref())))
}

fn load_token(path: &Path) -> Result<String, String> {
    require_absolute_path(path, "token path")?;
    let token = std::fs::read_to_string(path)
        .map_err(|error| worker_agent_error(format!("failed to read token file: {error}")))?;
    let token = token.trim();
    if token.len() != 64 || !token.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(worker_agent_error("token file is invalid"));
    }
    Ok(token.to_string())
}

pub fn validate_private_token(path: &Path) -> Result<(), String> {
    load_token(path).map(|_| ())
}

pub fn protect_private_token(path: &Path) -> Result<(), String> {
    require_absolute_path(path, "token path")?;
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| worker_agent_error(format!("token file is unavailable: {error}")))?;
    if !canonical.is_file() {
        return Err(worker_agent_error("token path is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&canonical, std::fs::Permissions::from_mode(0o600)).map_err(
            |error| worker_agent_error(format!("failed to protect token file: {error}")),
        )?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let identity_output = Command::new("whoami.exe")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|error| {
                worker_agent_error(format!("failed to identify token owner: {error}"))
            })?;
        if !identity_output.status.success() {
            return Err(worker_agent_error("failed to identify token owner"));
        }
        let identity = String::from_utf8(identity_output.stdout)
            .map_err(|_| worker_agent_error("token owner identity is not valid UTF-8"))?;
        let identity = identity.trim();
        if identity.is_empty() || identity.chars().any(char::is_control) {
            return Err(worker_agent_error("token owner identity is invalid"));
        }
        let grant = format!("{identity}:(F)");
        let reset_status = Command::new("icacls.exe")
            .arg(&canonical)
            .arg("/reset")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|error| worker_agent_error(format!("failed to reset token ACL: {error}")))?;
        if !reset_status.success() {
            return Err(worker_agent_error("failed to reset inherited token ACL"));
        }
        let status = Command::new("icacls.exe")
            .arg(&canonical)
            .args(["/inheritance:r", "/grant:r", grant.as_str()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|error| worker_agent_error(format!("failed to protect token ACL: {error}")))?;
        if !status.success() {
            return Err(worker_agent_error(
                "failed to restrict token ACL to the current user",
            ));
        }
    }
    Ok(())
}

fn tokens_equal(left: &str, right: &str) -> bool {
    let left_hash = Sha256::digest(left.as_bytes());
    let right_hash = Sha256::digest(right.as_bytes());
    left_hash
        .iter()
        .zip(right_hash.iter())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn server_tls_config(config: &WorkerAgentConfig) -> Result<rustls::ServerConfig, String> {
    ensure_tls_provider()?;
    rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            load_certificates(&config.tls_cert_path)?,
            load_private_key(&config.tls_key_path)?,
        )
        .map_err(|error| worker_agent_error(format!("TLS configuration failed: {error}")))
}

fn client_tls_config(connection: &WorkerAgentConnection) -> Result<rustls::ClientConfig, String> {
    ensure_tls_provider()?;
    let observed = certificate_sha256(&connection.tls_cert_path)?;
    if !tokens_equal(&observed, &connection.certificate_sha256) {
        return Err(worker_agent_error(
            "pinned TLS certificate fingerprint changed",
        ));
    }
    let roots = pinned_root_store(&connection.tls_cert_path)?;
    Ok(rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth())
}

fn pinned_root_store(path: &Path) -> Result<rustls::RootCertStore, String> {
    let certificate = load_certificates(path)?
        .into_iter()
        .next()
        .ok_or_else(|| worker_agent_error("TLS certificate file is empty"))?;
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(certificate)
        .map_err(|error| worker_agent_error(format!("failed to pin TLS certificate: {error}")))?;
    Ok(roots)
}

async fn tls_connect(
    host: &str,
    port: u16,
    connection: &WorkerAgentConnection,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, String> {
    validate_host(host, "Agent host")?;
    if port == 0 {
        return Err(worker_agent_error("Agent port is invalid"));
    }
    let stream = tokio::time::timeout(CONTROL_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| worker_agent_error("Agent connection timed out"))?
        .map_err(|error| worker_agent_error(format!("Agent connection failed: {error}")))?;
    let server_name = ServerName::try_from(connection.tls_server_name.clone())
        .map_err(|_| worker_agent_error("TLS server name is invalid"))?;
    tokio::time::timeout(
        CONTROL_TIMEOUT,
        TlsConnector::from(Arc::new(client_tls_config(connection)?)).connect(server_name, stream),
    )
    .await
    .map_err(|_| worker_agent_error("TLS handshake timed out"))?
    .map_err(|error| worker_agent_error(format!("TLS identity verification failed: {error}")))
}

async fn read_frame<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<Vec<u8>, String> {
    let mut frame = Vec::new();
    let mut limited = reader.take((MAX_CONTROL_FRAME_BYTES + 1) as u64);
    let count = limited
        .read_until(b'\n', &mut frame)
        .await
        .map_err(|error| worker_agent_error(format!("protocol read failed: {error}")))?;
    if count == 0 || count > MAX_CONTROL_FRAME_BYTES || !frame.ends_with(b"\n") {
        return Err(worker_agent_error("protocol frame is missing or too large"));
    }
    frame.pop();
    if frame.ends_with(b"\r") {
        frame.pop();
    }
    Ok(frame)
}

async fn write_frame<W: tokio::io::AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|error| worker_agent_error(format!("protocol serialization failed: {error}")))?;
    if bytes.len() >= MAX_CONTROL_FRAME_BYTES {
        return Err(worker_agent_error("protocol response is too large"));
    }
    bytes.push(b'\n');
    writer
        .write_all(&bytes)
        .await
        .map_err(|error| worker_agent_error(format!("protocol write failed: {error}")))?;
    writer
        .flush()
        .await
        .map_err(|error| worker_agent_error(format!("protocol flush failed: {error}")))
}

fn audit_hash(entry: &WorkerAgentAuditEntry) -> Result<String, String> {
    let bytes = serde_json::to_vec(&(
        entry.sequence,
        &entry.timestamp,
        &entry.agent_id,
        &entry.event,
        &entry.outcome,
        &entry.detail,
        &entry.previous_hash,
    ))
    .map_err(|error| worker_agent_error(format!("audit serialization failed: {error}")))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn read_verified_audit(path: &Path) -> Result<Vec<WorkerAgentAuditEntry>, String> {
    match File::open(path) {
        Ok(file) => {
            let length = file
                .metadata()
                .map_err(|error| {
                    worker_agent_error(format!("failed to inspect audit log: {error}"))
                })?
                .len();
            if length > MAX_AUDIT_FILE_BYTES {
                return Err(worker_agent_error(
                    "audit log reached its size limit; archive it before restarting the Agent",
                ));
            }
            let mut contents = String::new();
            file.take(MAX_AUDIT_FILE_BYTES + 1)
                .read_to_string(&mut contents)
                .map_err(|error| {
                    worker_agent_error(format!("failed to read audit log: {error}"))
                })?;
            if contents.len() as u64 > MAX_AUDIT_FILE_BYTES {
                return Err(worker_agent_error(
                    "audit log reached its size limit; archive it before restarting the Agent",
                ));
            }
            let mut entries = Vec::new();
            let mut previous_hash = String::new();
            for (index, line) in contents.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let entry: WorkerAgentAuditEntry = serde_json::from_str(line).map_err(|error| {
                    worker_agent_error(format!("audit line {} is invalid: {error}", index + 1))
                })?;
                if entry.sequence != entries.len() as u64 + 1
                    || entry.previous_hash != previous_hash
                    || audit_hash(&entry)? != entry.hash
                {
                    return Err(worker_agent_error(format!(
                        "audit integrity verification failed at line {}",
                        index + 1
                    )));
                }
                previous_hash = entry.hash.clone();
                entries.push(entry);
            }
            Ok(entries)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(worker_agent_error(format!(
            "failed to read audit log: {error}"
        ))),
    }
}

fn append_audit(
    runtime: &AgentRuntime,
    event: &str,
    outcome: &str,
    detail: impl Into<String>,
) -> Result<WorkerAgentAuditEntry, String> {
    let _guard = runtime
        .audit_lock
        .lock()
        .map_err(|_| worker_agent_error("audit lock is poisoned"))?;
    let entries = read_verified_audit(&runtime.config.audit_path)?;
    if !runtime.config.audit_path.exists() {
        crate::persistence::atomic_write(&runtime.config.audit_path, b"", None)?;
    }
    let mut entry = WorkerAgentAuditEntry {
        sequence: entries.len() as u64 + 1,
        timestamp: chrono::Utc::now().to_rfc3339(),
        agent_id: runtime.config.agent_id.clone(),
        event: event.to_string(),
        outcome: outcome.to_string(),
        detail: detail.into(),
        previous_hash: entries
            .last()
            .map(|entry| entry.hash.clone())
            .unwrap_or_default(),
        hash: String::new(),
    };
    entry.hash = audit_hash(&entry)?;
    let mut line = serde_json::to_vec(&entry)
        .map_err(|error| worker_agent_error(format!("audit serialization failed: {error}")))?;
    line.push(b'\n');
    let current_length = runtime
        .config
        .audit_path
        .metadata()
        .map_err(|error| worker_agent_error(format!("failed to inspect audit log: {error}")))?
        .len();
    if current_length.saturating_add(line.len() as u64) > MAX_AUDIT_FILE_BYTES {
        return Err(worker_agent_error(
            "audit log reached its size limit; archive it before restarting the Agent",
        ));
    }
    let mut file = OpenOptions::new()
        .append(true)
        .open(&runtime.config.audit_path)
        .map_err(|error| worker_agent_error(format!("failed to open audit log: {error}")))?;
    file.write_all(&line)
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_data())
        .map_err(|error| worker_agent_error(format!("failed to persist audit event: {error}")))?;
    Ok(entry)
}

fn audit_auth_failure(runtime: &AgentRuntime, event: &str) {
    let Ok(mut state) = runtime.auth_failure_audit.lock() else {
        return;
    };
    let now = Instant::now();
    if state
        .last_recorded
        .is_some_and(|last| now.duration_since(last) < AUTH_FAILURE_AUDIT_INTERVAL)
    {
        state.suppressed = state.suppressed.saturating_add(1);
        return;
    }
    let suppressed = std::mem::take(&mut state.suppressed);
    state.last_recorded = Some(now);
    drop(state);
    let detail = if suppressed == 0 {
        "authentication failed".to_string()
    } else {
        format!("authentication failed; {suppressed} repeated failures were suppressed")
    };
    let _ = append_audit(runtime, event, "denied", detail);
}

fn process_is_running(runtime: &AgentRuntime) -> Result<bool, String> {
    #[cfg(test)]
    if runtime.rpc_test_override.load(Ordering::Acquire) {
        return Ok(true);
    }
    let mut child = runtime
        .rpc_child
        .lock()
        .map_err(|_| worker_agent_error("rpc process lock is poisoned"))?;
    let exited = match child.as_mut() {
        Some(process) => process
            .try_wait()
            .map_err(|error| worker_agent_error(format!("failed to inspect rpc-server: {error}")))?
            .is_some(),
        None => return Ok(false),
    };
    if exited {
        *child = None;
        Ok(false)
    } else {
        Ok(true)
    }
}

fn status(runtime: &AgentRuntime) -> Result<WorkerAgentStatus, String> {
    Ok(WorkerAgentStatus {
        protocol_version: AGENT_PROTOCOL_VERSION,
        agent_id: runtime.config.agent_id.clone(),
        name: runtime.config.name.clone(),
        rpc_running: process_is_running(runtime)?,
        rpc_port: runtime.config.rpc_port,
        tunnel_port: runtime.config.tunnel_listen.port(),
        certificate_sha256: runtime.certificate_sha256.clone(),
        devices: runtime.config.devices.clone(),
    })
}

fn spawn_rpc_process(runtime: &AgentRuntime) -> Result<(), String> {
    let mut child_slot = runtime
        .rpc_child
        .lock()
        .map_err(|_| worker_agent_error("rpc process lock is poisoned"))?;
    if let Some(child) = child_slot.as_mut() {
        if child
            .try_wait()
            .map_err(|error| worker_agent_error(format!("failed to inspect rpc-server: {error}")))?
            .is_none()
        {
            return Ok(());
        }
        *child_slot = None;
    }
    let availability =
        std::net::TcpListener::bind((IpAddr::from([127, 0, 0, 1]), runtime.config.rpc_port))
            .map_err(|error| worker_agent_error(format!("rpc port is unavailable: {error}")))?;
    drop(availability);
    if let Some(parent) = runtime.config.rpc_log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            worker_agent_error(format!("failed to create rpc log directory: {error}"))
        })?;
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&runtime.config.rpc_log_path)
        .map_err(|error| worker_agent_error(format!("failed to open rpc log: {error}")))?;
    let stderr = log
        .try_clone()
        .map_err(|error| worker_agent_error(format!("failed to prepare rpc log: {error}")))?;
    let mut command = Command::new(&runtime.config.rpc_binary_path);
    command
        .args([
            "--host",
            "127.0.0.1",
            "--port",
            &runtime.config.rpc_port.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let child = command.spawn().map_err(|error| {
        worker_agent_error(format!("failed to start fixed rpc-server: {error}"))
    })?;
    *child_slot = Some(child);
    Ok(())
}

async fn start_rpc(runtime: Arc<AgentRuntime>) -> Result<WorkerAgentStatus, String> {
    spawn_rpc_process(&runtime)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if TcpStream::connect(("127.0.0.1", runtime.config.rpc_port))
            .await
            .is_ok()
        {
            return status(&runtime);
        }
        if !process_is_running(&runtime)? {
            return Err(worker_agent_error(
                "rpc-server exited before becoming ready",
            ));
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = stop_rpc(&runtime);
            return Err(worker_agent_error("rpc-server readiness timed out"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn stop_rpc(runtime: &AgentRuntime) -> Result<bool, String> {
    let mut child_slot = runtime
        .rpc_child
        .lock()
        .map_err(|_| worker_agent_error("rpc process lock is poisoned"))?;
    let Some(child) = child_slot.as_mut() else {
        return Ok(false);
    };
    child
        .kill()
        .map_err(|error| worker_agent_error(format!("failed to stop rpc-server: {error}")))?;
    child
        .wait()
        .map_err(|error| worker_agent_error(format!("failed to reap rpc-server: {error}")))?;
    *child_slot = None;
    Ok(true)
}

fn ok_response(runtime: &AgentRuntime) -> ControlResponse {
    ControlResponse {
        protocol_version: AGENT_PROTOCOL_VERSION,
        agent_id: runtime.config.agent_id.clone(),
        ok: true,
        status: None,
        audit: Vec::new(),
        error: None,
    }
}

fn error_response(runtime: Option<&AgentRuntime>, code: &str, message: &str) -> ControlResponse {
    ControlResponse {
        protocol_version: AGENT_PROTOCOL_VERSION,
        agent_id: runtime
            .map(|runtime| runtime.config.agent_id.clone())
            .unwrap_or_default(),
        ok: false,
        status: None,
        audit: Vec::new(),
        error: Some(ControlError {
            code: code.to_string(),
            message: message.to_string(),
        }),
    }
}

fn request_authenticated(runtime: &AgentRuntime, request: &ControlRequest) -> bool {
    request.protocol_version == AGENT_PROTOCOL_VERSION
        && (request.expected_agent_id.is_empty()
            || request.expected_agent_id == runtime.config.agent_id)
        && load_token(&runtime.config.token_path)
            .is_ok_and(|token| tokens_equal(&token, &request.token))
}

async fn dispatch_control(runtime: Arc<AgentRuntime>, request: ControlRequest) -> ControlResponse {
    if !request_authenticated(&runtime, &request) {
        audit_auth_failure(&runtime, "control_auth");
        return error_response(None, "UNAUTHORIZED", "authentication failed");
    }
    let action = request.action;
    if let Err(error) = append_audit(
        &runtime,
        action.name(),
        "requested",
        "authenticated request accepted",
    ) {
        return error_response(Some(&runtime), "AUDIT_UNAVAILABLE", &error);
    }
    let result: Result<ControlResponse, String> = match action {
        ControlAction::Status if request.limit.is_none() => status(&runtime).map(|status| {
            let mut response = ok_response(&runtime);
            response.status = Some(status);
            response
        }),
        ControlAction::RpcStart if request.limit.is_none() => {
            start_rpc(runtime.clone()).await.map(|status| {
                let mut response = ok_response(&runtime);
                response.status = Some(status);
                response
            })
        }
        ControlAction::RpcStop if request.limit.is_none() => stop_rpc(&runtime)
            .and_then(|_| status(&runtime))
            .map(|status| {
                let mut response = ok_response(&runtime);
                response.status = Some(status);
                response
            }),
        ControlAction::Audit => {
            let limit = request.limit.unwrap_or(100).clamp(1, MAX_AUDIT_RESULTS);
            read_verified_audit(&runtime.config.audit_path).map(|entries| {
                let start = entries.len().saturating_sub(limit);
                let mut response = ok_response(&runtime);
                response.audit = entries[start..].to_vec();
                response
            })
        }
        _ => Err(worker_agent_error(
            "action is not allowed or contains invalid fields",
        )),
    };
    match result {
        Ok(response) => match append_audit(&runtime, action.name(), "allowed", "request completed")
        {
            Ok(_) => response,
            Err(error) => error_response(Some(&runtime), "AUDIT_UNAVAILABLE", &error),
        },
        Err(error) => {
            if let Err(audit_error) = append_audit(&runtime, action.name(), "failed", &error) {
                error_response(Some(&runtime), "AUDIT_UNAVAILABLE", &audit_error)
            } else {
                error_response(Some(&runtime), "ACTION_FAILED", &error)
            }
        }
    }
}

async fn handle_control_connection(
    runtime: Arc<AgentRuntime>,
    acceptor: TlsAcceptor,
    stream: TcpStream,
) -> Result<(), String> {
    let mut tls = tokio::time::timeout(CONTROL_TIMEOUT, acceptor.accept(stream))
        .await
        .map_err(|_| worker_agent_error("TLS handshake timed out"))?
        .map_err(|error| worker_agent_error(format!("TLS handshake failed: {error}")))?;
    let mut reader = BufReader::new(&mut tls);
    let frame = tokio::time::timeout(CONTROL_TIMEOUT, read_frame(&mut reader))
        .await
        .map_err(|_| worker_agent_error("control request timed out"))??;
    drop(reader);
    let request: ControlRequest = serde_json::from_slice(&frame)
        .map_err(|_| worker_agent_error("invalid control request"))?;
    let response = dispatch_control(runtime, request).await;
    write_frame(&mut tls, &response).await?;
    let _ = tls.shutdown().await;
    Ok(())
}

async fn handle_tunnel_connection(
    runtime: Arc<AgentRuntime>,
    acceptor: TlsAcceptor,
    stream: TcpStream,
) -> Result<(), String> {
    let tls = tokio::time::timeout(CONTROL_TIMEOUT, acceptor.accept(stream))
        .await
        .map_err(|_| worker_agent_error("TLS tunnel handshake timed out"))?
        .map_err(|error| worker_agent_error(format!("TLS tunnel handshake failed: {error}")))?;
    let mut reader = BufReader::new(tls);
    let frame = tokio::time::timeout(CONTROL_TIMEOUT, read_frame(&mut reader))
        .await
        .map_err(|_| worker_agent_error("tunnel authentication timed out"))??;
    let hello: TunnelHello = serde_json::from_slice(&frame)
        .map_err(|_| worker_agent_error("invalid tunnel handshake"))?;
    let authenticated = hello.protocol_version == AGENT_PROTOCOL_VERSION
        && hello.expected_agent_id == runtime.config.agent_id
        && load_token(&runtime.config.token_path)
            .is_ok_and(|token| tokens_equal(&token, &hello.token));
    if !authenticated {
        let mut tls = reader.into_inner();
        let _ = write_frame(
            &mut tls,
            &TunnelResponse {
                protocol_version: AGENT_PROTOCOL_VERSION,
                agent_id: String::new(),
                ok: false,
                error: Some(ControlError {
                    code: "UNAUTHORIZED".into(),
                    message: "authentication failed".into(),
                }),
            },
        )
        .await;
        audit_auth_failure(&runtime, "tunnel_auth");
        return Ok(());
    }
    if let Err(error) = append_audit(
        &runtime,
        "tunnel",
        "requested",
        "authenticated tunnel requested",
    ) {
        let mut tls = reader.into_inner();
        write_frame(
            &mut tls,
            &TunnelResponse {
                protocol_version: AGENT_PROTOCOL_VERSION,
                agent_id: runtime.config.agent_id.clone(),
                ok: false,
                error: Some(ControlError {
                    code: "AUDIT_UNAVAILABLE".into(),
                    message: error,
                }),
            },
        )
        .await?;
        return Ok(());
    }
    if !process_is_running(&runtime)? {
        append_audit(&runtime, "tunnel", "failed", "rpc-server is not running")?;
        let mut tls = reader.into_inner();
        write_frame(
            &mut tls,
            &TunnelResponse {
                protocol_version: AGENT_PROTOCOL_VERSION,
                agent_id: runtime.config.agent_id.clone(),
                ok: false,
                error: Some(ControlError {
                    code: "RPC_NOT_RUNNING".into(),
                    message: "rpc-server is not running".into(),
                }),
            },
        )
        .await?;
        return Ok(());
    }
    let mut rpc = TcpStream::connect(("127.0.0.1", runtime.config.rpc_port))
        .await
        .map_err(|error| worker_agent_error(format!("rpc-server connection failed: {error}")))?;
    append_audit(&runtime, "tunnel", "allowed", "encrypted rpc tunnel opened")?;
    let mut tls = reader.into_inner();
    write_frame(
        &mut tls,
        &TunnelResponse {
            protocol_version: AGENT_PROTOCOL_VERSION,
            agent_id: runtime.config.agent_id.clone(),
            ok: true,
            error: None,
        },
    )
    .await?;
    let result = tokio::io::copy_bidirectional(&mut tls, &mut rpc).await;
    let _ = tls.shutdown().await;
    let _ = rpc.shutdown().await;
    let outcome = if result.is_ok() { "closed" } else { "failed" };
    let audit_result = append_audit(&runtime, "tunnel", outcome, "encrypted rpc tunnel closed");
    result
        .map(|_| ())
        .map_err(|error| worker_agent_error(format!("tunnel forwarding failed: {error}")))?;
    audit_result.map(|_| ())
}

async fn serve_listener(
    listener: TcpListener,
    runtime: Arc<AgentRuntime>,
    acceptor: TlsAcceptor,
    mut shutdown: watch::Receiver<bool>,
    connection_limit: Arc<Semaphore>,
    tunnel: bool,
) -> Result<(), String> {
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted
                    .map_err(|error| worker_agent_error(format!("listener failed: {error}")))?;
                let Ok(permit) = connection_limit.clone().try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let runtime = runtime.clone();
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let result = if tunnel {
                        handle_tunnel_connection(runtime, acceptor, stream).await
                    } else {
                        handle_control_connection(runtime, acceptor, stream).await
                    };
                    if let Err(error) = result {
                        eprintln!("{error}");
                    }
                });
            }
        }
    }
}

pub async fn serve(config_path: &Path) -> Result<(), String> {
    let config = load_agent_config(config_path)?;
    protect_private_token(&config.token_path)?;
    let _ = read_verified_audit(&config.audit_path)?;
    let certificate_sha256 = certificate_sha256(&config.tls_cert_path)?;
    let control = TcpListener::bind(config.control_listen)
        .await
        .map_err(|error| worker_agent_error(format!("control listener failed: {error}")))?;
    let tunnel = TcpListener::bind(config.tunnel_listen)
        .await
        .map_err(|error| worker_agent_error(format!("tunnel listener failed: {error}")))?;
    let acceptor = TlsAcceptor::from(Arc::new(server_tls_config(&config)?));
    let runtime = Arc::new(AgentRuntime {
        config,
        certificate_sha256,
        rpc_child: Mutex::new(None),
        audit_lock: Mutex::new(()),
        auth_failure_audit: Mutex::new(AuthFailureAuditState::default()),
        #[cfg(test)]
        rpc_test_override: AtomicBool::new(false),
    });
    append_audit(&runtime, "agent", "started", "secure Worker Agent started")?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connection_limit = Arc::new(Semaphore::new(MAX_IN_FLIGHT_CONNECTIONS));
    let control_task = tokio::spawn(serve_listener(
        control,
        runtime.clone(),
        acceptor.clone(),
        shutdown_rx.clone(),
        connection_limit.clone(),
        false,
    ));
    let tunnel_task = tokio::spawn(serve_listener(
        tunnel,
        runtime.clone(),
        acceptor,
        shutdown_rx,
        connection_limit,
        true,
    ));
    tokio::signal::ctrl_c()
        .await
        .map_err(|error| worker_agent_error(format!("shutdown signal failed: {error}")))?;
    let _ = shutdown_tx.send(true);
    let _ = control_task.await;
    let _ = tunnel_task.await;
    let _ = stop_rpc(&runtime);
    append_audit(&runtime, "agent", "stopped", "secure Worker Agent stopped")?;
    Ok(())
}

async fn send_control(
    connection: &WorkerAgentConnection,
    action: ControlAction,
    limit: Option<usize>,
) -> Result<ControlResponse, String> {
    let mut tls = tls_connect(
        &connection.control_host,
        connection.control_port,
        connection,
    )
    .await?;
    let request = ControlRequest {
        protocol_version: AGENT_PROTOCOL_VERSION,
        token: load_token(&connection.token_path)?,
        expected_agent_id: connection.agent_id.clone(),
        action,
        limit,
    };
    write_frame(&mut tls, &request).await?;
    let mut reader = BufReader::new(tls);
    let frame = tokio::time::timeout(CONTROL_TIMEOUT, read_frame(&mut reader))
        .await
        .map_err(|_| worker_agent_error("Agent response timed out"))??;
    let response: ControlResponse = serde_json::from_slice(&frame)
        .map_err(|error| worker_agent_error(format!("invalid Agent response: {error}")))?;
    if response.protocol_version != AGENT_PROTOCOL_VERSION {
        return Err(worker_agent_error("Agent protocol version mismatch"));
    }
    if !connection.agent_id.is_empty() && response.agent_id != connection.agent_id {
        return Err(worker_agent_error("Agent identity changed"));
    }
    if !response.ok {
        let error = response
            .error
            .map(|error| format!("{}: {}", error.code, error.message))
            .unwrap_or_else(|| "request failed".to_string());
        return Err(worker_agent_error(error));
    }
    Ok(response)
}

pub async fn get_remote_status(
    connection: &WorkerAgentConnection,
) -> Result<WorkerAgentStatus, String> {
    send_control(connection, ControlAction::Status, None)
        .await?
        .status
        .ok_or_else(|| worker_agent_error("Agent status response is missing"))
}

pub async fn start_remote_rpc(
    connection: &WorkerAgentConnection,
) -> Result<WorkerAgentStatus, String> {
    send_control(connection, ControlAction::RpcStart, None)
        .await?
        .status
        .ok_or_else(|| worker_agent_error("Agent start response is missing"))
}

pub async fn stop_remote_rpc(
    connection: &WorkerAgentConnection,
) -> Result<WorkerAgentStatus, String> {
    send_control(connection, ControlAction::RpcStop, None)
        .await?
        .status
        .ok_or_else(|| worker_agent_error("Agent stop response is missing"))
}

pub async fn get_remote_audit(
    connection: &WorkerAgentConnection,
    limit: usize,
) -> Result<Vec<WorkerAgentAuditEntry>, String> {
    Ok(send_control(
        connection,
        ControlAction::Audit,
        Some(limit.clamp(1, MAX_AUDIT_RESULTS)),
    )
    .await?
    .audit)
}

async fn forward_to_agent(
    mut local: TcpStream,
    connection: WorkerAgentConnection,
) -> Result<(), String> {
    let mut tls = tls_connect(&connection.tunnel_host, connection.tunnel_port, &connection).await?;
    write_frame(
        &mut tls,
        &TunnelHello {
            protocol_version: AGENT_PROTOCOL_VERSION,
            token: load_token(&connection.token_path)?,
            expected_agent_id: connection.agent_id.clone(),
        },
    )
    .await?;
    let mut reader = BufReader::new(tls);
    let frame = tokio::time::timeout(CONTROL_TIMEOUT, read_frame(&mut reader))
        .await
        .map_err(|_| worker_agent_error("tunnel handshake timed out"))??;
    let response: TunnelResponse = serde_json::from_slice(&frame)
        .map_err(|error| worker_agent_error(format!("invalid tunnel response: {error}")))?;
    if !response.ok
        || response.protocol_version != AGENT_PROTOCOL_VERSION
        || response.agent_id != connection.agent_id
    {
        return Err(worker_agent_error(
            response
                .error
                .map(|error| error.message)
                .unwrap_or_else(|| "tunnel identity verification failed".to_string()),
        ));
    }
    let mut tls = reader.into_inner();
    let result = tokio::io::copy_bidirectional(&mut local, &mut tls).await;
    let _ = local.shutdown().await;
    let _ = tls.shutdown().await;
    result
        .map(|_| ())
        .map_err(|error| worker_agent_error(format!("local tunnel bridge failed: {error}")))
}

pub async fn ensure_manager_bridge(
    worker_id: &str,
    connection: WorkerAgentConnection,
    local_port: u16,
) -> Result<u16, String> {
    if let Some(existing) = AGENT_BRIDGES
        .lock()
        .map_err(|_| worker_agent_error("bridge registry is poisoned"))?
        .get(worker_id)
    {
        if *existing
            .connection
            .lock()
            .map_err(|_| worker_agent_error("bridge connection lock is poisoned"))?
            != connection
        {
            return Err(worker_agent_error(
                "Worker bridge already uses a different Agent identity",
            ));
        }
        if local_port == 0 || existing.port == local_port {
            return Ok(existing.port);
        }
        return Err(worker_agent_error(
            "Worker bridge already uses a different port",
        ));
    }
    let listener = TcpListener::bind(("127.0.0.1", local_port))
        .await
        .map_err(|error| {
            worker_agent_error(format!("local bridge port is unavailable: {error}"))
        })?;
    let port = listener
        .local_addr()
        .map_err(|error| worker_agent_error(format!("failed to inspect local bridge: {error}")))?
        .port();
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let (stopped_tx, stopped_rx) = oneshot::channel();
    let generation = uuid::Uuid::new_v4();
    let connection = Arc::new(Mutex::new(connection));
    AGENT_BRIDGES
        .lock()
        .map_err(|_| worker_agent_error("bridge registry is poisoned"))?
        .insert(
            worker_id.to_string(),
            BridgeHandle {
                port,
                connection: connection.clone(),
                generation,
                shutdown: shutdown_tx,
                stopped: stopped_rx,
            },
        );
    let worker_id = worker_id.to_string();
    tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        break;
                    }
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _)) => {
                            let connection = match connection.lock() {
                                Ok(connection) => connection.clone(),
                                Err(_) => {
                                    eprintln!("{}", worker_agent_error("bridge connection lock is poisoned"));
                                    continue;
                                }
                            };
                            connections.spawn(async move {
                                if let Err(error) = forward_to_agent(stream, connection).await {
                                    eprintln!("{error}");
                                }
                            });
                        }
                        Err(error) => {
                            eprintln!("{}", worker_agent_error(format!("bridge listener failed: {error}")));
                            break;
                        }
                    }
                }
                completed = connections.join_next(), if !connections.is_empty() => {
                    if let Some(Err(error)) = completed {
                        eprintln!("{}", worker_agent_error(format!("bridge task failed: {error}")));
                    }
                }
            }
        }
        drop(listener);
        connections.abort_all();
        while connections.join_next().await.is_some() {}
        if let Ok(mut bridges) = AGENT_BRIDGES.lock() {
            if bridges
                .get(&worker_id)
                .is_some_and(|bridge| bridge.generation == generation)
            {
                bridges.remove(&worker_id);
            }
        }
        let _ = stopped_tx.send(());
    });
    Ok(port)
}

pub fn replace_manager_bridge_connection(
    worker_id: &str,
    connection: WorkerAgentConnection,
    expected_port: u16,
) -> Result<Option<WorkerAgentConnection>, String> {
    let bridges = AGENT_BRIDGES
        .lock()
        .map_err(|_| worker_agent_error("bridge registry is poisoned"))?;
    let Some(bridge) = bridges.get(worker_id) else {
        return Ok(None);
    };
    if bridge.port != expected_port {
        return Err(worker_agent_error(
            "Worker bridge already uses a different port",
        ));
    }
    let mut current = bridge
        .connection
        .lock()
        .map_err(|_| worker_agent_error("bridge connection lock is poisoned"))?;
    Ok(Some(std::mem::replace(&mut *current, connection)))
}

pub async fn stop_manager_bridge(worker_id: &str) -> bool {
    let bridge = AGENT_BRIDGES
        .lock()
        .ok()
        .and_then(|mut bridges| bridges.remove(worker_id));
    let Some(bridge) = bridge else {
        return false;
    };
    if bridge.shutdown.send(true).is_err() {
        return false;
    }
    tokio::time::timeout(CONTROL_TIMEOUT, bridge.stopped)
        .await
        .is_ok()
}

pub fn stop_all_manager_bridges() {
    let bridges = AGENT_BRIDGES
        .lock()
        .map(|mut bridges| std::mem::take(&mut *bridges))
        .unwrap_or_default();
    for (_, bridge) in bridges {
        let _ = bridge.shutdown.send(true);
    }
}

fn parse_socket(value: &str, field: &str) -> Result<SocketAddr, String> {
    value
        .parse::<SocketAddr>()
        .map_err(|_| worker_agent_error(format!("{field} must be an IP address and port")))
}

fn parse_port(value: &str, field: &str) -> Result<u16, String> {
    let port = value
        .parse::<u16>()
        .map_err(|_| worker_agent_error(format!("{field} is invalid")))?;
    if port == 0 {
        return Err(worker_agent_error(format!("{field} must not be zero")));
    }
    Ok(port)
}

fn parse_options(arguments: &[String]) -> Result<HashMap<String, String>, String> {
    let mut options = HashMap::new();
    let mut index = 0;
    while index < arguments.len() {
        let key = arguments[index].as_str();
        if !key.starts_with("--") {
            return Err(worker_agent_error(format!("unexpected argument: {key}")));
        }
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| worker_agent_error(format!("{key} requires a value")))?;
        if options.insert(key.to_string(), value.clone()).is_some() {
            return Err(worker_agent_error(format!("duplicate option: {key}")));
        }
        index += 2;
    }
    Ok(options)
}

fn take_required(options: &mut HashMap<String, String>, key: &str) -> Result<String, String> {
    options
        .remove(key)
        .ok_or_else(|| worker_agent_error(format!("{key} is required")))
}

fn init_agent(arguments: &[String]) -> Result<(), String> {
    let mut options = parse_options(arguments)?;
    let config_path = PathBuf::from(take_required(&mut options, "--config")?);
    let name = take_required(&mut options, "--name")?;
    let control_listen = parse_socket(&take_required(&mut options, "--control")?, "--control")?;
    let tunnel_listen = parse_socket(&take_required(&mut options, "--tunnel")?, "--tunnel")?;
    let advertise_host = take_required(&mut options, "--advertise-host")?;
    let tls_cert_path = PathBuf::from(take_required(&mut options, "--tls-cert")?);
    let tls_key_path = PathBuf::from(take_required(&mut options, "--tls-key")?);
    let rpc_binary_path = PathBuf::from(take_required(&mut options, "--rpc-binary")?);
    require_absolute_path(&config_path, "--config")?;
    let parent = config_path
        .parent()
        .ok_or_else(|| worker_agent_error("config path has no parent"))?;
    let token_path = options
        .remove("--token-file")
        .map(PathBuf::from)
        .unwrap_or_else(|| parent.join("worker-agent.token"));
    let audit_path = options
        .remove("--audit-file")
        .map(PathBuf::from)
        .unwrap_or_else(|| parent.join("worker-agent-audit.jsonl"));
    let rpc_log_path = options
        .remove("--rpc-log")
        .map(PathBuf::from)
        .unwrap_or_else(|| parent.join("worker-agent-rpc.log"));
    let rpc_port = options
        .remove("--rpc-port")
        .map(|value| parse_port(&value, "--rpc-port"))
        .transpose()?
        .unwrap_or(50052);
    if !options.is_empty() {
        return Err(worker_agent_error(format!(
            "unknown option: {}",
            options.keys().next().unwrap()
        )));
    }
    if config_path.exists() || token_path.exists() {
        return Err(worker_agent_error(
            "refusing to overwrite existing config or token file",
        ));
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    crate::persistence::atomic_write(&token_path, token.as_bytes(), None)?;
    protect_private_token(&token_path)?;
    let mut config = WorkerAgentConfig {
        schema_version: AGENT_CONFIG_SCHEMA_VERSION,
        agent_id: uuid::Uuid::new_v4().to_string(),
        name,
        control_listen,
        tunnel_listen,
        advertise_host,
        tls_cert_path,
        tls_key_path,
        token_path,
        rpc_binary_path,
        rpc_port,
        audit_path,
        rpc_log_path,
        devices: Vec::new(),
    };
    if let Err(error) = validate_agent_config(&mut config) {
        let _ = std::fs::remove_file(&config.token_path);
        return Err(error);
    }
    let bytes = serde_json::to_vec_pretty(&config)
        .map_err(|error| worker_agent_error(format!("config serialization failed: {error}")))?;
    if let Err(error) = crate::persistence::atomic_write(&config_path, &bytes, None) {
        let _ = std::fs::remove_file(&config.token_path);
        return Err(error);
    }
    let output = serde_json::json!({
        "schemaVersion": AGENT_CONFIG_SCHEMA_VERSION,
        "protocolVersion": AGENT_PROTOCOL_VERSION,
        "agentId": config.agent_id,
        "name": config.name,
        "control": format!("{}:{}", config.advertise_host, config.control_listen.port()),
        "tunnel": format!("{}:{}", config.advertise_host, config.tunnel_listen.port()),
        "configPath": config_path,
        "tokenPath": config.token_path,
        "tlsCertificatePath": config.tls_cert_path,
        "certificateSha256": certificate_sha256(&config.tls_cert_path)?,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output).unwrap_or_default()
    );
    Ok(())
}

fn config_argument(arguments: &[String]) -> Result<PathBuf, String> {
    let mut options = parse_options(arguments)?;
    let path = PathBuf::from(take_required(&mut options, "--config")?);
    if !options.is_empty() {
        return Err(worker_agent_error(format!(
            "unknown option: {}",
            options.keys().next().unwrap()
        )));
    }
    require_absolute_path(&path, "--config")?;
    Ok(path)
}

fn rotate_token(config_path: &Path) -> Result<(), String> {
    let config = load_agent_config(config_path)?;
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    crate::persistence::atomic_write(&config.token_path, token.as_bytes(), None)?;
    protect_private_token(&config.token_path)?;
    println!(
        "{}",
        serde_json::json!({
            "rotated": true,
            "agentId": config.agent_id,
            "tokenPath": config.token_path,
        })
    );
    Ok(())
}

fn inspect_agent(config_path: &Path) -> Result<(), String> {
    let config = load_agent_config(config_path)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schemaVersion": config.schema_version,
            "protocolVersion": AGENT_PROTOCOL_VERSION,
            "agentId": config.agent_id,
            "name": config.name,
            "control": format!("{}:{}", config.advertise_host, config.control_listen.port()),
            "tunnel": format!("{}:{}", config.advertise_host, config.tunnel_listen.port()),
            "rpcPort": config.rpc_port,
            "tokenPath": config.token_path,
            "tlsCertificatePath": config.tls_cert_path,
            "certificateSha256": certificate_sha256(&config.tls_cert_path)?,
            "auditPath": config.audit_path,
        }))
        .unwrap_or_default()
    );
    Ok(())
}

pub fn run_cli(arguments: impl IntoIterator<Item = OsString>) -> i32 {
    let arguments = match arguments
        .into_iter()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| worker_agent_error("arguments must be valid Unicode"))
        })
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    let result = match arguments.first().map(String::as_str) {
        None | Some("help" | "--help" | "-h") if arguments.len() <= 1 => {
            println!("{HELP}");
            Ok(())
        }
        Some("init") => init_agent(&arguments[1..]),
        Some("rotate-token") => {
            config_argument(&arguments[1..]).and_then(|path| rotate_token(&path))
        }
        Some("inspect") => config_argument(&arguments[1..]).and_then(|path| inspect_agent(&path)),
        Some("serve") => config_argument(&arguments[1..]).and_then(|path| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    worker_agent_error(format!("runtime initialization failed: {error}"))
                })?
                .block_on(serve(&path))
        }),
        _ => Err(worker_agent_error(
            "unknown or malformed worker-agent command",
        )),
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("lsm-worker-agent-{name}-{}", uuid::Uuid::new_v4()))
    }

    fn test_runtime(directory: &Path) -> Arc<AgentRuntime> {
        let token_path = directory.join("agent.token");
        std::fs::write(&token_path, "a".repeat(64)).unwrap();
        Arc::new(AgentRuntime {
            config: WorkerAgentConfig {
                schema_version: AGENT_CONFIG_SCHEMA_VERSION,
                agent_id: uuid::Uuid::new_v4().to_string(),
                name: "Test Agent".to_string(),
                control_listen: "127.0.0.1:17443".parse().unwrap(),
                tunnel_listen: "127.0.0.1:17444".parse().unwrap(),
                advertise_host: "localhost".to_string(),
                tls_cert_path: directory.join("agent.crt"),
                tls_key_path: directory.join("agent.key"),
                token_path,
                rpc_binary_path: directory.join(expected_rpc_binary_name()),
                rpc_port: 50052,
                audit_path: directory.join("audit.jsonl"),
                rpc_log_path: directory.join("rpc.log"),
                devices: Vec::new(),
            },
            certificate_sha256: "a".repeat(64),
            rpc_child: Mutex::new(None),
            audit_lock: Mutex::new(()),
            auth_failure_audit: Mutex::new(AuthFailureAuditState::default()),
            rpc_test_override: AtomicBool::new(false),
        })
    }

    #[test]
    fn token_comparison_is_content_sensitive() {
        let token = "a".repeat(64);
        assert!(tokens_equal(&token, &token));
        assert!(!tokens_equal(&token, &"b".repeat(64)));
    }

    #[test]
    fn token_file_requires_exactly_256_bits_of_hex() {
        let directory = test_directory("token-format");
        std::fs::create_dir_all(&directory).unwrap();
        let token_path = directory.join("agent.token");
        std::fs::write(&token_path, "A5".repeat(32)).unwrap();
        assert!(validate_private_token(&token_path).is_ok());
        for invalid in ["a".repeat(63), "a".repeat(65), "z".repeat(64)] {
            std::fs::write(&token_path, invalid).unwrap();
            assert!(validate_private_token(&token_path).is_err());
        }
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn protocol_frame_is_bounded_before_a_delimiter_arrives() {
        let (mut writer, reader) = tokio::io::duplex(MAX_CONTROL_FRAME_BYTES + 2);
        let send = tokio::spawn(async move {
            writer
                .write_all(&vec![b'x'; MAX_CONTROL_FRAME_BYTES + 1])
                .await
                .unwrap();
            writer.shutdown().await.unwrap();
        });
        let mut reader = BufReader::new(reader);
        assert!(read_frame(&mut reader).await.is_err());
        send.await.unwrap();
    }

    #[test]
    fn client_trust_store_contains_only_the_pinned_leaf() {
        let directory = test_directory("leaf-pin");
        std::fs::create_dir_all(&directory).unwrap();
        let first = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let second = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_path = directory.join("agent.crt");
        std::fs::write(
            &cert_path,
            format!("{}{}", first.cert.pem(), second.cert.pem()),
        )
        .unwrap();
        assert_eq!(load_certificates(&cert_path).unwrap().len(), 2);
        assert_eq!(pinned_root_store(&cert_path).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn unavailable_audit_blocks_authenticated_lifecycle_action() {
        let directory = test_directory("audit-fail-closed");
        std::fs::create_dir_all(&directory).unwrap();
        let runtime = test_runtime(&directory);
        std::fs::write(
            &runtime.config.audit_path,
            vec![b'x'; MAX_AUDIT_FILE_BYTES as usize + 1],
        )
        .unwrap();
        let response = dispatch_control(
            runtime.clone(),
            ControlRequest {
                protocol_version: AGENT_PROTOCOL_VERSION,
                token: "a".repeat(64),
                expected_agent_id: runtime.config.agent_id.clone(),
                action: ControlAction::RpcStart,
                limit: None,
            },
        )
        .await;
        assert!(!response.ok);
        assert_eq!(
            response.error.as_ref().map(|error| error.code.as_str()),
            Some("AUDIT_UNAVAILABLE")
        );
        assert!(runtime.rpc_child.lock().unwrap().is_none());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn repeated_authentication_failures_are_aggregated() {
        let directory = test_directory("auth-throttle");
        std::fs::create_dir_all(&directory).unwrap();
        let runtime = test_runtime(&directory);
        audit_auth_failure(&runtime, "control_auth");
        audit_auth_failure(&runtime, "control_auth");
        let entries = read_verified_audit(&runtime.config.audit_path).unwrap();
        assert_eq!(entries.len(), 1);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn stop_rpc_terminates_and_reaps_the_owned_child() {
        let directory = test_directory("process-cleanup");
        std::fs::create_dir_all(&directory).unwrap();
        let runtime = test_runtime(&directory);
        #[cfg(windows)]
        let child = {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            let mut command = Command::new("powershell.exe");
            command
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 30",
                ])
                .creation_flags(CREATE_NO_WINDOW);
            command.spawn().unwrap()
        };
        #[cfg(unix)]
        let child = Command::new("sh").args(["-c", "sleep 30"]).spawn().unwrap();
        *runtime.rpc_child.lock().unwrap() = Some(child);
        assert!(stop_rpc(&runtime).unwrap());
        assert!(runtime.rpc_child.lock().unwrap().is_none());
        assert!(!stop_rpc(&runtime).unwrap());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    async fn existing_bridge_rejects_a_different_agent_identity() {
        let directory = test_directory("bridge-identity");
        std::fs::create_dir_all(&directory).unwrap();
        let token_path = directory.join("agent.token");
        let cert_path = directory.join("agent.crt");
        std::fs::write(&token_path, "a".repeat(64)).unwrap();
        std::fs::write(&cert_path, "fixture").unwrap();
        let connection = WorkerAgentConnection {
            agent_id: uuid::Uuid::new_v4().to_string(),
            control_host: "127.0.0.1".to_string(),
            control_port: 17443,
            tunnel_host: "127.0.0.1".to_string(),
            tunnel_port: 17444,
            tls_server_name: "localhost".to_string(),
            tls_cert_path: cert_path,
            token_path,
            certificate_sha256: "a".repeat(64),
        };
        let worker_id = format!("bridge-{}", uuid::Uuid::new_v4());
        let port = ensure_manager_bridge(&worker_id, connection.clone(), 0)
            .await
            .unwrap();
        let mut replacement = connection;
        replacement.agent_id = uuid::Uuid::new_v4().to_string();
        assert!(ensure_manager_bridge(&worker_id, replacement.clone(), 0)
            .await
            .is_err());
        let previous = replace_manager_bridge_connection(&worker_id, replacement.clone(), port)
            .unwrap()
            .unwrap();
        assert_ne!(previous.agent_id, replacement.agent_id);
        assert_eq!(
            ensure_manager_bridge(&worker_id, replacement.clone(), port)
                .await
                .unwrap(),
            port
        );
        assert!(stop_manager_bridge(&worker_id).await);
        assert_eq!(
            ensure_manager_bridge(&worker_id, replacement, port)
                .await
                .unwrap(),
            port
        );
        assert!(stop_manager_bridge(&worker_id).await);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn protocol_rejects_unknown_fields_and_actions() {
        let extra = br#"{"protocol_version":1,"token":"x","expected_agent_id":"","action":"status","extra":true}"#;
        assert!(serde_json::from_slice::<ControlRequest>(extra).is_err());
        let unknown_action =
            br#"{"protocol_version":1,"token":"x","expected_agent_id":"","action":"shell"}"#;
        assert!(serde_json::from_slice::<ControlRequest>(unknown_action).is_err());
    }

    #[test]
    fn audit_hash_changes_when_an_entry_is_tampered() {
        let mut entry = WorkerAgentAuditEntry {
            sequence: 1,
            timestamp: "2026-08-21T00:00:00Z".into(),
            agent_id: uuid::Uuid::nil().to_string(),
            event: "rpc_start".into(),
            outcome: "allowed".into(),
            detail: "request completed".into(),
            previous_hash: String::new(),
            hash: String::new(),
        };
        entry.hash = audit_hash(&entry).unwrap();
        let original = entry.hash.clone();
        entry.outcome = "denied".into();
        assert_ne!(audit_hash(&entry).unwrap(), original);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authenticated_tls_control_and_rpc_tunnel_round_trip() {
        let directory = test_directory("e2e");
        std::fs::create_dir_all(&directory).unwrap();
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_path = directory.join("agent.crt");
        let key_path = directory.join("agent.key");
        let token_path = directory.join("agent.token");
        std::fs::write(&cert_path, certified.cert.pem()).unwrap();
        std::fs::write(&key_path, certified.key_pair.serialize_pem()).unwrap();
        std::fs::write(&token_path, "a".repeat(64)).unwrap();

        let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tunnel_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rpc_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let control_port = control_listener.local_addr().unwrap().port();
        let tunnel_port = tunnel_listener.local_addr().unwrap().port();
        let rpc_port = rpc_listener.local_addr().unwrap().port();
        let config = WorkerAgentConfig {
            schema_version: AGENT_CONFIG_SCHEMA_VERSION,
            agent_id: uuid::Uuid::new_v4().to_string(),
            name: "E2E Agent".to_string(),
            control_listen: control_listener.local_addr().unwrap(),
            tunnel_listen: tunnel_listener.local_addr().unwrap(),
            advertise_host: "localhost".to_string(),
            tls_cert_path: cert_path.clone(),
            tls_key_path: key_path,
            token_path: token_path.clone(),
            rpc_binary_path: directory.join(expected_rpc_binary_name()),
            rpc_port,
            audit_path: directory.join("audit.jsonl"),
            rpc_log_path: directory.join("rpc.log"),
            devices: Vec::new(),
        };
        let fingerprint = certificate_sha256(&cert_path).unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(server_tls_config(&config).unwrap()));
        let runtime = Arc::new(AgentRuntime {
            config: config.clone(),
            certificate_sha256: fingerprint.clone(),
            rpc_child: Mutex::new(None),
            audit_lock: Mutex::new(()),
            auth_failure_audit: Mutex::new(AuthFailureAuditState::default()),
            rpc_test_override: AtomicBool::new(true),
        });
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connection_limit = Arc::new(Semaphore::new(MAX_IN_FLIGHT_CONNECTIONS));
        let control_task = tokio::spawn(serve_listener(
            control_listener,
            runtime.clone(),
            acceptor.clone(),
            shutdown_rx.clone(),
            connection_limit.clone(),
            false,
        ));
        let tunnel_task = tokio::spawn(serve_listener(
            tunnel_listener,
            runtime,
            acceptor,
            shutdown_rx,
            connection_limit,
            true,
        ));
        let rpc_task = tokio::spawn(async move {
            let (mut stream, _) = rpc_listener.accept().await.unwrap();
            let mut bytes = [0_u8; 4];
            stream.read_exact(&mut bytes).await.unwrap();
            stream.write_all(&bytes).await.unwrap();
        });

        let connection = WorkerAgentConnection {
            agent_id: config.agent_id.clone(),
            control_host: "127.0.0.1".to_string(),
            control_port,
            tunnel_host: "127.0.0.1".to_string(),
            tunnel_port,
            tls_server_name: "localhost".to_string(),
            tls_cert_path: cert_path.clone(),
            token_path: token_path.clone(),
            certificate_sha256: fingerprint,
        };
        let remote = get_remote_status(&connection).await.unwrap();
        assert_eq!(remote.agent_id, config.agent_id);
        assert!(remote.rpc_running);

        let local_port = ensure_manager_bridge("e2e-worker", connection.clone(), 0)
            .await
            .unwrap();
        let mut local = TcpStream::connect(("127.0.0.1", local_port)).await.unwrap();
        local.write_all(b"ping").await.unwrap();
        let mut response = [0_u8; 4];
        local.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"ping");
        drop(local);

        let wrong_token = directory.join("wrong.token");
        std::fs::write(&wrong_token, "b".repeat(64)).unwrap();
        let mut rejected = connection.clone();
        rejected.token_path = wrong_token;
        assert!(get_remote_status(&rejected).await.is_err());

        let untrusted = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let untrusted_path = directory.join("untrusted.crt");
        std::fs::write(&untrusted_path, untrusted.cert.pem()).unwrap();
        let mut wrong_ca = connection.clone();
        wrong_ca.tls_cert_path = untrusted_path.clone();
        wrong_ca.certificate_sha256 = certificate_sha256(&untrusted_path).unwrap();
        assert!(get_remote_status(&wrong_ca).await.is_err());

        let mut wrong_identity = connection.clone();
        wrong_identity.tls_server_name = "other.example.test".to_string();
        assert!(get_remote_status(&wrong_identity).await.is_err());

        let mut forbidden = tls_connect(
            &connection.control_host,
            connection.control_port,
            &connection,
        )
        .await
        .unwrap();
        let forbidden_frame = format!(
            "{{\"protocol_version\":1,\"token\":\"{}\",\"expected_agent_id\":\"{}\",\"action\":\"shell\"}}\n",
            "a".repeat(64),
            connection.agent_id,
        );
        forbidden
            .write_all(forbidden_frame.as_bytes())
            .await
            .unwrap();
        forbidden.flush().await.unwrap();
        let mut forbidden = BufReader::new(forbidden);
        assert!(read_frame(&mut forbidden).await.is_err());

        assert!(stop_manager_bridge("e2e-worker").await);
        let _ = shutdown_tx.send(true);
        rpc_task.await.unwrap();
        control_task.await.unwrap().unwrap();
        tunnel_task.await.unwrap().unwrap();
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn init_and_rotation_keep_token_contents_out_of_configuration() {
        let directory = test_directory("rotation");
        std::fs::create_dir_all(&directory).unwrap();
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_path = directory.join("agent.crt");
        let key_path = directory.join("agent.key");
        let config_path = directory.join("agent.json");
        let rpc_path = directory.join(expected_rpc_binary_name());
        std::fs::write(&cert_path, certified.cert.pem()).unwrap();
        std::fs::write(&key_path, certified.key_pair.serialize_pem()).unwrap();
        std::fs::write(&rpc_path, b"test fixture").unwrap();
        let arguments = vec![
            "--config".to_string(),
            config_path.to_string_lossy().to_string(),
            "--name".to_string(),
            "Rotation Agent".to_string(),
            "--control".to_string(),
            "127.0.0.1:17443".to_string(),
            "--tunnel".to_string(),
            "127.0.0.1:17444".to_string(),
            "--advertise-host".to_string(),
            "localhost".to_string(),
            "--tls-cert".to_string(),
            cert_path.to_string_lossy().to_string(),
            "--tls-key".to_string(),
            key_path.to_string_lossy().to_string(),
            "--rpc-binary".to_string(),
            rpc_path.to_string_lossy().to_string(),
        ];
        init_agent(&arguments).unwrap();
        let config = load_agent_config(&config_path).unwrap();
        let original = load_token(&config.token_path).unwrap();
        let persisted = std::fs::read_to_string(&config_path).unwrap();
        assert!(!persisted.contains(&original));
        rotate_token(&config_path).unwrap();
        let rotated = load_token(&config.token_path).unwrap();
        assert_ne!(original, rotated);
        assert_eq!(rotated.len(), 64);
        let _ = std::fs::remove_dir_all(directory);
    }
}
