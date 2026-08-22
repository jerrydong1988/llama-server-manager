use crate::models::WorkerDevice;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{BufReader as StdBufReader, Write as _};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Child;
#[cfg(test)]
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Once};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch, OwnedSemaphorePermit, Semaphore};
use tokio_rustls::{TlsAcceptor, TlsConnector};

pub const AGENT_PROTOCOL_VERSION: u32 = 1;
pub const AGENT_CONFIG_SCHEMA_VERSION: u32 = 1;
const MAX_CONTROL_FRAME_BYTES: usize = 64 * 1024;
const MAX_AUDIT_RESULTS: usize = 500;
const MAX_AUDIT_FILE_BYTES: u64 = 8 * 1024 * 1024;
const AUDIT_ROTATE_AT_BYTES: u64 = 6 * 1024 * 1024;
const LIFECYCLE_AUDIT_RESERVATION_BYTES: u64 = ((MAX_CONTROL_FRAME_BYTES + 4 * 1024) as u64) * 2;
const MAX_PREAUTH_CONTROL_CONNECTIONS: usize = 16;
const MAX_PREAUTH_TUNNEL_CONNECTIONS: usize = 16;
const MAX_AUTHENTICATED_CONTROL_CONNECTIONS: usize = 32;
const MAX_AUTHENTICATED_TUNNELS: usize = 64;
const MAX_PREAUTH_CONNECTIONS_PER_SOURCE: usize = 4;
const AUTH_FAILURE_AUDIT_INTERVAL: Duration = Duration::from_secs(60);
const MAX_AUTH_DENIAL_FILE_BYTES: u64 = 64 * 1024;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_TUNNEL_LIFETIME: Duration = Duration::from_secs(60 * 60);
const MAX_LOCAL_BRIDGE_CONNECTIONS: usize = 16;

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
  The Agent accepts status, rpc_stop, and audit actions over TLS.
  rpc_start fails closed until rpc-server supports an authenticated or OS-private endpoint.
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
    #[serde(default)]
    pub rpc_artifact_identity: crate::deployment_identity::ArtifactIdentity,
    pub rpc_port: u16,
    pub audit_path: PathBuf,
    pub rpc_log_path: PathBuf,
    #[serde(default)]
    pub devices: Vec<WorkerDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default)]
    pub audit_sequence: u64,
    #[serde(default)]
    pub audit_hash: String,
}

impl PartialEq for WorkerAgentConnection {
    fn eq(&self, other: &Self) -> bool {
        self.agent_id == other.agent_id
            && self.control_host == other.control_host
            && self.control_port == other.control_port
            && self.tunnel_host == other.tunnel_host
            && self.tunnel_port == other.tunnel_port
            && self.tls_server_name == other.tls_server_name
            && self.tls_cert_path == other.tls_cert_path
            && self.token_path == other.token_path
            && self.certificate_sha256 == other.certificate_sha256
    }
}

impl Eq for WorkerAgentConnection {}

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
    #[serde(default)]
    from_sequence: Option<u64>,
    #[serde(default)]
    checkpoint_hash: Option<String>,
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
    next_sequence: Option<u64>,
    #[serde(default)]
    has_more: bool,
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
    lifecycle_lock: Mutex<()>,
    closing: AtomicBool,
    audit_lock: Mutex<()>,
    audit_reserved_bytes: Mutex<u64>,
    auth_failure_audit: Mutex<AuthFailureAuditState>,
    #[cfg(test)]
    rpc_test_override: AtomicBool,
}

#[derive(Default)]
struct AuthFailureAuditState {
    last_recorded: Option<Instant>,
    suppressed: u64,
}

struct AuditReservation {
    runtime: Arc<AgentRuntime>,
    remaining: u64,
}

impl AuditReservation {
    fn consume(&mut self, bytes: u64) -> Result<(), String> {
        if bytes > self.remaining {
            return Err(worker_agent_error(
                "lifecycle audit record exceeded its reserved capacity",
            ));
        }
        let mut reserved = self
            .runtime
            .audit_reserved_bytes
            .lock()
            .map_err(|_| worker_agent_error("audit reservation lock is poisoned"))?;
        if *reserved < bytes {
            return Err(worker_agent_error(
                "audit reservation accounting is invalid",
            ));
        }
        *reserved -= bytes;
        self.remaining -= bytes;
        Ok(())
    }

    fn restore(&mut self, bytes: u64) {
        if let Ok(mut reserved) = self.runtime.audit_reserved_bytes.lock() {
            *reserved = reserved.saturating_add(bytes);
            self.remaining = self.remaining.saturating_add(bytes);
        }
    }
}

impl Drop for AuditReservation {
    fn drop(&mut self) {
        if let Ok(mut reserved) = self.runtime.audit_reserved_bytes.lock() {
            *reserved = reserved.saturating_sub(self.remaining);
        }
    }
}

fn reserve_lifecycle_audit(runtime: &Arc<AgentRuntime>) -> Result<AuditReservation, String> {
    let _audit_guard = runtime
        .audit_lock
        .lock()
        .map_err(|_| worker_agent_error("audit lock is poisoned"))?;
    let mut current = runtime
        .config
        .audit_path
        .metadata()
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if current >= AUDIT_ROTATE_AT_BYTES {
        let _ = read_verified_audit(&runtime.config.audit_path)?;
        rotate_audit_log(&runtime.config.audit_path)?;
        current = 0;
    }
    let mut reserved = runtime
        .audit_reserved_bytes
        .lock()
        .map_err(|_| worker_agent_error("audit reservation lock is poisoned"))?;
    let required = current
        .checked_add(*reserved)
        .and_then(|value| value.checked_add(LIFECYCLE_AUDIT_RESERVATION_BYTES))
        .ok_or_else(|| worker_agent_error("audit capacity calculation overflow"))?;
    if required > MAX_AUDIT_FILE_BYTES {
        return Err(worker_agent_error(
            "audit log cannot reserve a durable lifecycle outcome; archive it first",
        ));
    }
    *reserved += LIFECYCLE_AUDIT_RESERVATION_BYTES;
    Ok(AuditReservation {
        runtime: runtime.clone(),
        remaining: LIFECYCLE_AUDIT_RESERVATION_BYTES,
    })
}

struct SourceAdmission {
    source: IpAddr,
    counts: Arc<Mutex<HashMap<IpAddr, usize>>>,
}

impl Drop for SourceAdmission {
    fn drop(&mut self) {
        if let Ok(mut counts) = self.counts.lock() {
            if let Some(count) = counts.get_mut(&self.source) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    counts.remove(&self.source);
                }
            }
        }
    }
}

fn try_admit_source(
    counts: &Arc<Mutex<HashMap<IpAddr, usize>>>,
    source: IpAddr,
) -> Option<SourceAdmission> {
    let mut guard = counts.lock().ok()?;
    let count = guard.entry(source).or_default();
    if *count >= MAX_PREAUTH_CONNECTIONS_PER_SOURCE {
        return None;
    }
    *count += 1;
    drop(guard);
    Some(SourceAdmission {
        source,
        counts: counts.clone(),
    })
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
    config.tls_key_path = protect_private_file(&config.tls_key_path, "TLS private key")?;
    config.token_path = protect_private_file(&config.token_path, "token file")?;
    config.rpc_binary_path = validate_rpc_binary(&config.rpc_binary_path)?;
    let rpc_executable =
        crate::deployment_identity::ArtifactLease::open_owner_protected_executable(
            &config.rpc_binary_path,
        )
        .map_err(worker_agent_error)?;
    if !config.rpc_artifact_identity.is_verified()
        || rpc_executable.identity() != &config.rpc_artifact_identity
    {
        return Err(worker_agent_error(
            "rpc-server identity is missing or changed; explicitly reinitialize the Agent to approve the new executable",
        ));
    }
    config.rpc_binary_path = rpc_executable.canonical_path().to_path_buf();
    let _ = server_tls_config(config)?;
    let _ = load_token(&config.token_path)?;
    Ok(())
}

pub fn load_agent_config(path: &Path) -> Result<WorkerAgentConfig, String> {
    require_absolute_path(path, "config")?;
    let config_root = path
        .parent()
        .ok_or_else(|| worker_agent_error("config path has no parent directory"))?;
    crate::persistence::enforce_private_directory(config_root).map_err(worker_agent_error)?;
    let bytes = crate::persistence::read_private_file_bounded(path, MAX_CONTROL_FRAME_BYTES as u64)
        .map_err(worker_agent_error)?
        .ok_or_else(|| worker_agent_error("Agent config is unavailable"))?;
    let mut config: WorkerAgentConfig = serde_json::from_slice(&bytes)
        .map_err(|error| worker_agent_error(format!("invalid config: {error}")))?;
    validate_agent_config(&mut config)?;
    let config_root = std::fs::canonicalize(config_root)
        .map_err(|error| worker_agent_error(format!("config directory is unavailable: {error}")))?;
    for (path, label) in [
        (&config.token_path, "token file"),
        (&config.audit_path, "audit log"),
        (&config.rpc_log_path, "RPC log"),
    ] {
        let parent = path
            .parent()
            .ok_or_else(|| worker_agent_error(format!("{label} has no parent directory")))?;
        let canonical_parent = std::fs::canonicalize(parent).map_err(|error| {
            worker_agent_error(format!("{label} directory is unavailable: {error}"))
        })?;
        if !crate::path_utils::paths_equal(&canonical_parent, &config_root) {
            return Err(worker_agent_error(format!(
                "{label} must reside directly in the private Agent config directory"
            )));
        }
        if path.exists() {
            let metadata = std::fs::symlink_metadata(path).map_err(|error| {
                worker_agent_error(format!("failed to inspect {label}: {error}"))
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(worker_agent_error(format!(
                    "{label} must be a regular non-link file"
                )));
            }
        }
    }
    Ok(config)
}

fn load_certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    require_absolute_path(path, "TLS certificate path")?;
    const MAX_CERTIFICATE_PEM_BYTES: u64 = 256 * 1024;
    let bytes =
        crate::persistence::read_regular_file_nofollow_bounded(path, MAX_CERTIFICATE_PEM_BYTES)
            .map_err(worker_agent_error)?
            .ok_or_else(|| worker_agent_error("TLS certificate file is unavailable"))?;
    parse_certificates(&bytes)
}

fn parse_certificates(bytes: &[u8]) -> Result<Vec<CertificateDer<'static>>, String> {
    const MAX_CERTIFICATES: usize = 8;
    const MAX_CERTIFICATE_DER_BYTES: usize = 512 * 1024;
    let mut reader = std::io::Cursor::new(bytes);
    let certificates = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| worker_agent_error(format!("invalid TLS certificate: {error}")))?;
    if certificates.is_empty() {
        return Err(worker_agent_error(
            "TLS certificate file contains no certificates",
        ));
    }
    if certificates.len() > MAX_CERTIFICATES {
        return Err(worker_agent_error(format!(
            "TLS certificate bundle exceeds {MAX_CERTIFICATES} certificates"
        )));
    }
    let der_bytes = certificates.iter().try_fold(0_usize, |total, certificate| {
        total.checked_add(certificate.as_ref().len())
    });
    if !matches!(der_bytes, Some(bytes) if bytes <= MAX_CERTIFICATE_DER_BYTES) {
        return Err(worker_agent_error(format!(
            "TLS certificate bundle exceeds {MAX_CERTIFICATE_DER_BYTES} decoded bytes"
        )));
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

fn protect_private_file(path: &Path, label: &str) -> Result<PathBuf, String> {
    require_absolute_path(path, label)?;
    let original = std::fs::symlink_metadata(path)
        .map_err(|error| worker_agent_error(format!("failed to inspect {label}: {error}")))?;
    if !original.is_file() || original.file_type().is_symlink() {
        return Err(worker_agent_error(format!(
            "{label} must be a regular non-link file"
        )));
    }
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| worker_agent_error(format!("{label} is unavailable: {error}")))?;
    let metadata = std::fs::symlink_metadata(&canonical)
        .map_err(|error| worker_agent_error(format!("failed to inspect {label}: {error}")))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(worker_agent_error(format!(
            "{label} must be a regular non-link file"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&canonical, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| worker_agent_error(format!("failed to protect {label}: {error}")))?;
    }
    #[cfg(windows)]
    {
        crate::persistence::enforce_private_file(&canonical)
            .map_err(|error| worker_agent_error(format!("failed to protect {label}: {error}")))?;
    }
    Ok(canonical)
}

pub fn certificate_sha256(path: &Path) -> Result<String, String> {
    let certificates = load_certificates(path)?;
    if certificates.len() != 1 {
        return Err(worker_agent_error(
            "pinned TLS certificate enrollment requires exactly one certificate",
        ));
    }
    let certificate = &certificates[0];
    Ok(format!("{:x}", Sha256::digest(certificate.as_ref())))
}

pub fn certificate_sha256_from_pem(bytes: &[u8]) -> Result<String, String> {
    let certificates = parse_certificates(bytes)?;
    if certificates.len() != 1 {
        return Err(worker_agent_error(
            "pinned TLS certificate enrollment requires exactly one certificate",
        ));
    }
    Ok(format!("{:x}", Sha256::digest(certificates[0].as_ref())))
}

fn load_token(path: &Path) -> Result<String, String> {
    require_absolute_path(path, "token path")?;
    let bytes = crate::persistence::read_private_file_bounded(path, 256)
        .map_err(worker_agent_error)?
        .ok_or_else(|| worker_agent_error("token file is unavailable"))?;
    let token = std::str::from_utf8(&bytes)
        .map_err(|_| worker_agent_error("token file is invalid"))?
        .trim();
    if token.len() != 64 || !token.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(worker_agent_error("token file is invalid"));
    }
    Ok(token.to_string())
}

pub fn validate_private_token(path: &Path) -> Result<(), String> {
    load_token(path).map(|_| ())
}

pub fn protect_private_token(path: &Path) -> Result<(), String> {
    protect_private_file(path, "token file").map(|_| ())
}

fn open_private_append_file(path: &Path, label: &str) -> Result<File, String> {
    if !path.exists() {
        crate::persistence::atomic_write(path, b"", None).map_err(worker_agent_error)?;
    }
    let canonical = protect_private_file(path, label)?;
    let mut options = OpenOptions::new();
    options.read(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        options
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(&canonical)
        .map_err(|error| worker_agent_error(format!("failed to open {label}: {error}")))?;
    let metadata = file
        .metadata()
        .map_err(|error| worker_agent_error(format!("failed to inspect {label}: {error}")))?;
    if !metadata.is_file() {
        return Err(worker_agent_error(format!(
            "{label} must be a regular file"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(worker_agent_error(format!(
                "{label} must have exactly one hard link"
            )));
        }
    }
    Ok(file)
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
    let tls = tokio::time::timeout(
        CONTROL_TIMEOUT,
        TlsConnector::from(Arc::new(client_tls_config(connection)?)).connect(server_name, stream),
    )
    .await
    .map_err(|_| worker_agent_error("TLS handshake timed out"))?
    .map_err(|error| worker_agent_error(format!("TLS identity verification failed: {error}")))?;
    let negotiated = tls
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .ok_or_else(|| worker_agent_error("TLS peer did not present a leaf certificate"))?;
    let negotiated_sha256 = format!("{:x}", Sha256::digest(negotiated.as_ref()));
    if !tokens_equal(&negotiated_sha256, &connection.certificate_sha256) {
        return Err(worker_agent_error(
            "negotiated TLS leaf certificate does not match the enrolled fingerprint",
        ));
    }
    Ok(tls)
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

pub fn verify_remote_audit_extension(
    agent_id: &str,
    checkpoint_sequence: u64,
    checkpoint_hash: &str,
    entries: &[WorkerAgentAuditEntry],
) -> Result<(u64, String), String> {
    if entries.is_empty() {
        return Ok((checkpoint_sequence, checkpoint_hash.to_string()));
    }
    for (index, entry) in entries.iter().enumerate() {
        if entry.agent_id != agent_id || audit_hash(entry)? != entry.hash {
            return Err(worker_agent_error(format!(
                "remote audit integrity verification failed at result {}",
                index + 1
            )));
        }
        if let Some(previous) = index.checked_sub(1).and_then(|value| entries.get(value)) {
            if entry.sequence != previous.sequence.saturating_add(1)
                || entry.previous_hash != previous.hash
            {
                return Err(worker_agent_error(format!(
                    "remote audit chain forked at result {}",
                    index + 1
                )));
            }
        }
    }

    let extension_start = if checkpoint_sequence == 0 {
        let first = &entries[0];
        if first.sequence != 1 || !first.previous_hash.is_empty() {
            return Err(worker_agent_error(
                "remote audit does not provide a valid genesis checkpoint",
            ));
        }
        0
    } else if let Some(index) = entries
        .iter()
        .position(|entry| entry.sequence == checkpoint_sequence && entry.hash == checkpoint_hash)
    {
        index.saturating_add(1)
    } else {
        let first = &entries[0];
        if first.sequence != checkpoint_sequence.saturating_add(1)
            || first.previous_hash != checkpoint_hash
        {
            return Err(worker_agent_error(
                "remote audit history was truncated, rolled back, or forked from the manager checkpoint",
            ));
        }
        0
    };
    let terminal = entries
        .get(extension_start)
        .and_then(|_| entries.last())
        .or_else(|| {
            entries.iter().find(|entry| {
                entry.sequence == checkpoint_sequence && entry.hash == checkpoint_hash
            })
        })
        .ok_or_else(|| worker_agent_error("remote audit checkpoint is unavailable"))?;
    if terminal.sequence < checkpoint_sequence {
        return Err(worker_agent_error("remote audit history rolled back"));
    }
    Ok((terminal.sequence, terminal.hash.clone()))
}

fn read_audit_segment(path: &Path) -> Result<Vec<WorkerAgentAuditEntry>, String> {
    let Some(bytes) = crate::persistence::read_private_file_bounded(path, MAX_AUDIT_FILE_BYTES + 1)
        .map_err(worker_agent_error)?
    else {
        return Ok(Vec::new());
    };
    if bytes.len() as u64 > MAX_AUDIT_FILE_BYTES {
        return Err(worker_agent_error("audit segment exceeds its size limit"));
    }
    let contents =
        String::from_utf8(bytes).map_err(|_| worker_agent_error("audit log is not valid UTF-8"))?;
    let mut entries = Vec::new();
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        if entries.len() >= 200_000 {
            return Err(worker_agent_error(
                "audit history exceeds its record budget",
            ));
        }
        entries.push(serde_json::from_str(line).map_err(|error| {
            worker_agent_error(format!("audit line {} is invalid: {error}", index + 1))
        })?);
    }
    Ok(entries)
}

fn audit_segment_paths(path: &Path) -> Result<Vec<PathBuf>, String> {
    let parent = path
        .parent()
        .ok_or_else(|| worker_agent_error("audit log has no parent directory"))?;
    let prefix = format!(
        "{}.segment-",
        path.file_name()
            .ok_or_else(|| worker_agent_error("audit log has no file name"))?
            .to_string_lossy()
    );
    let mut segments = std::fs::read_dir(parent)
        .map_err(|error| {
            worker_agent_error(format!("failed to enumerate audit segments: {error}"))
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
        })
        .collect::<Vec<_>>();
    segments.sort();
    if segments.len() > 16 {
        return Err(worker_agent_error(
            "audit history exceeds 16 retained segments; export and explicitly reset it",
        ));
    }
    Ok(segments)
}

fn read_verified_audit(path: &Path) -> Result<Vec<WorkerAgentAuditEntry>, String> {
    let mut entries = Vec::new();
    for segment in audit_segment_paths(path)?
        .into_iter()
        .chain(std::iter::once(path.to_path_buf()))
    {
        entries.extend(read_audit_segment(&segment)?);
        if entries.len() > 200_000 {
            return Err(worker_agent_error(
                "audit history exceeds its record budget",
            ));
        }
    }
    let mut expected_sequence = 1_u64;
    let mut previous_hash = String::new();
    for (index, entry) in entries.iter().enumerate() {
        if entry.sequence != expected_sequence
            || entry.previous_hash != previous_hash
            || audit_hash(entry)? != entry.hash
        {
            return Err(worker_agent_error(format!(
                "audit integrity verification failed at logical record {}",
                index + 1
            )));
        }
        expected_sequence = entry
            .sequence
            .checked_add(1)
            .ok_or_else(|| worker_agent_error("audit sequence overflow"))?;
        previous_hash = entry.hash.clone();
    }
    Ok(entries)
}

fn rotate_audit_log(path: &Path) -> Result<(), String> {
    let active = read_audit_segment(path)?;
    if active.is_empty() {
        return Ok(());
    }
    let first = active.first().map(|entry| entry.sequence).unwrap_or(0);
    let last = active.last().map(|entry| entry.sequence).unwrap_or(0);
    if audit_segment_paths(path)?.len() >= 16 {
        return Err(worker_agent_error(
            "audit segment retention is full; export and explicitly reset it",
        ));
    }
    let archive = path.with_file_name(format!(
        "{}.segment-{first:020}-{last:020}",
        path.file_name()
            .ok_or_else(|| worker_agent_error("audit log has no file name"))?
            .to_string_lossy()
    ));
    if archive.exists() {
        return Err(worker_agent_error("audit segment archive already exists"));
    }
    std::fs::rename(path, &archive)
        .map_err(|error| worker_agent_error(format!("failed to rotate audit log: {error}")))?;
    protect_private_file(&archive, "audit archive")?;
    crate::persistence::atomic_write(path, b"", None).map_err(worker_agent_error)
}

fn append_audit(
    runtime: &AgentRuntime,
    event: &str,
    outcome: &str,
    detail: impl Into<String>,
) -> Result<WorkerAgentAuditEntry, String> {
    append_audit_internal(runtime, event, outcome, detail.into(), None)
}

fn append_reserved_audit(
    runtime: &AgentRuntime,
    event: &str,
    outcome: &str,
    detail: impl Into<String>,
    reservation: &mut AuditReservation,
) -> Result<WorkerAgentAuditEntry, String> {
    append_audit_internal(runtime, event, outcome, detail.into(), Some(reservation))
}

fn append_audit_internal(
    runtime: &AgentRuntime,
    event: &str,
    outcome: &str,
    detail: String,
    mut reservation: Option<&mut AuditReservation>,
) -> Result<WorkerAgentAuditEntry, String> {
    let _guard = runtime
        .audit_lock
        .lock()
        .map_err(|_| worker_agent_error("audit lock is poisoned"))?;
    let entries = read_verified_audit(&runtime.config.audit_path)?;
    if !runtime.config.audit_path.exists() {
        crate::persistence::atomic_write(&runtime.config.audit_path, b"", None)?;
    }
    let current_length = runtime
        .config
        .audit_path
        .metadata()
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if current_length >= AUDIT_ROTATE_AT_BYTES {
        rotate_audit_log(&runtime.config.audit_path)?;
    }
    let mut entry = WorkerAgentAuditEntry {
        sequence: entries.len() as u64 + 1,
        timestamp: chrono::Utc::now().to_rfc3339(),
        agent_id: runtime.config.agent_id.clone(),
        event: event.to_string(),
        outcome: outcome.to_string(),
        detail,
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
    let consumed = line.len() as u64;
    if let Some(reservation) = reservation.as_deref_mut() {
        reservation.consume(consumed)?;
    }
    let result = (|| {
        let current_length = runtime
            .config
            .audit_path
            .metadata()
            .map_err(|error| worker_agent_error(format!("failed to inspect audit log: {error}")))?
            .len();
        let reserved = *runtime
            .audit_reserved_bytes
            .lock()
            .map_err(|_| worker_agent_error("audit reservation lock is poisoned"))?;
        let required = current_length
            .checked_add(line.len() as u64)
            .and_then(|value| value.checked_add(reserved))
            .ok_or_else(|| worker_agent_error("audit capacity calculation overflow"))?;
        if required > MAX_AUDIT_FILE_BYTES {
            return Err(worker_agent_error(
                "audit log reached its size limit; archive it before restarting the Agent",
            ));
        }
        let mut file = open_private_append_file(&runtime.config.audit_path, "audit log")?;
        file.write_all(&line)
            .and_then(|_| file.flush())
            .and_then(|_| file.sync_data())
            .map_err(|error| {
                worker_agent_error(format!("failed to persist audit event: {error}"))
            })?;
        Ok(entry)
    })();
    if result.is_err() {
        if let Some(reservation) = reservation {
            reservation.restore(consumed);
        }
    }
    result
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
    let path = runtime.config.audit_path.with_extension("auth-denials.log");
    let mut line = serde_json::to_vec(&serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "agentId": runtime.config.agent_id,
        "event": event,
        "outcome": "denied",
        "detail": detail,
    }))
    .unwrap_or_default();
    line.push(b'\n');
    let current = path.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let result = if current.saturating_add(line.len() as u64) > MAX_AUTH_DENIAL_FILE_BYTES {
        crate::persistence::atomic_write(&path, &line, None)
    } else {
        open_private_append_file(&path, "authentication denial log").and_then(|mut file| {
            file.write_all(&line)
                .and_then(|_| file.sync_data())
                .map_err(|error| {
                    worker_agent_error(format!("failed to persist auth denial: {error}"))
                })
        })
    };
    if result.is_ok() {
        let _ = protect_private_file(&path, "authentication denial log");
    }
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

fn stop_rpc(runtime: &AgentRuntime) -> Result<bool, String> {
    let _lifecycle_guard = runtime
        .lifecycle_lock
        .lock()
        .map_err(|_| worker_agent_error("rpc lifecycle lock is poisoned"))?;
    stop_rpc_locked(runtime)
}

fn stop_rpc_locked(runtime: &AgentRuntime) -> Result<bool, String> {
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

impl Drop for AgentRuntime {
    fn drop(&mut self) {
        if let Ok(child_slot) = self.rpc_child.get_mut() {
            if let Some(child) = child_slot.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
            *child_slot = None;
        }
    }
}

fn ok_response(runtime: &AgentRuntime) -> ControlResponse {
    ControlResponse {
        protocol_version: AGENT_PROTOCOL_VERSION,
        agent_id: runtime.config.agent_id.clone(),
        ok: true,
        status: None,
        audit: Vec::new(),
        next_sequence: None,
        has_more: false,
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
        next_sequence: None,
        has_more: false,
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

fn audit_page(runtime: &AgentRuntime, request: &ControlRequest) -> Result<ControlResponse, String> {
    let entries = read_verified_audit(&runtime.config.audit_path)?;
    let from_sequence = request.from_sequence.unwrap_or(0);
    let checkpoint_hash = request.checkpoint_hash.as_deref().unwrap_or_default();
    if from_sequence > 0
        && (checkpoint_hash.len() != 64 || !checkpoint_hash.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(worker_agent_error("audit checkpoint hash is invalid"));
    }
    let start = if from_sequence == 0 {
        0
    } else {
        entries
            .iter()
            .position(|entry| entry.sequence == from_sequence && entry.hash == checkpoint_hash)
            .map(|index| index + 1)
            .ok_or_else(|| {
                worker_agent_error("audit checkpoint is unavailable or does not match")
            })?
    };
    let limit = request.limit.unwrap_or(100).clamp(1, MAX_AUDIT_RESULTS);
    let mut response = ok_response(runtime);
    response.next_sequence = Some(from_sequence);
    for entry in entries.iter().skip(start).take(limit) {
        response.audit.push(entry.clone());
        response.next_sequence = Some(entry.sequence);
        response.has_more = start + response.audit.len() < entries.len();
        let encoded = serde_json::to_vec(&response).map_err(|error| {
            worker_agent_error(format!("audit page serialization failed: {error}"))
        })?;
        if encoded.len() >= MAX_CONTROL_FRAME_BYTES {
            response.audit.pop();
            response.next_sequence = response
                .audit
                .last()
                .map(|entry| entry.sequence)
                .or(Some(from_sequence));
            response.has_more = true;
            if response.audit.is_empty() {
                return Err(worker_agent_error(
                    "one audit record exceeds the control frame byte budget",
                ));
            }
            break;
        }
    }
    response.has_more = start + response.audit.len() < entries.len();
    Ok(response)
}

async fn dispatch_control(runtime: Arc<AgentRuntime>, request: ControlRequest) -> ControlResponse {
    if !request_authenticated(&runtime, &request) {
        audit_auth_failure(&runtime, "control_auth");
        return error_response(None, "UNAUTHORIZED", "authentication failed");
    }
    if runtime.closing.load(Ordering::Acquire) {
        return error_response(
            Some(&runtime),
            "AGENT_SHUTTING_DOWN",
            "Worker Agent is shutting down",
        );
    }
    let action = request.action;
    let mut lifecycle_reservation =
        if matches!(action, ControlAction::RpcStart | ControlAction::RpcStop) {
            match reserve_lifecycle_audit(&runtime) {
                Ok(reservation) => Some(reservation),
                Err(error) => {
                    return error_response(Some(&runtime), "AUDIT_UNAVAILABLE", &error);
                }
            }
        } else {
            None
        };
    let rpc_was_running = if lifecycle_reservation.is_some() {
        process_is_running(&runtime).unwrap_or(false)
    } else {
        false
    };
    let requested_audit = if let Some(reservation) = lifecycle_reservation.as_mut() {
        append_reserved_audit(
            &runtime,
            action.name(),
            "requested",
            "authenticated request accepted",
            reservation,
        )
    } else {
        append_audit(
            &runtime,
            action.name(),
            "requested",
            "authenticated request accepted",
        )
    };
    if let Err(error) = requested_audit {
        return error_response(Some(&runtime), "AUDIT_UNAVAILABLE", &error);
    }
    let result: Result<ControlResponse, String> = match action {
        ControlAction::Status if request.limit.is_none() => status(&runtime).map(|status| {
            let mut response = ok_response(&runtime);
            response.status = Some(status);
            response
        }),
        ControlAction::RpcStart if request.limit.is_none() => Err(worker_agent_error(
            "rpc-server startup is disabled because the upstream child exposes an unauthenticated loopback TCP endpoint; a private inherited socket, named pipe, or authenticated upstream transport is required",
        )),
        ControlAction::RpcStop if request.limit.is_none() => stop_rpc(&runtime)
            .and_then(|_| status(&runtime))
            .map(|status| {
                let mut response = ok_response(&runtime);
                response.status = Some(status);
                response
            }),
        ControlAction::Audit => {
            audit_page(&runtime, &request)
        }
        _ => Err(worker_agent_error(
            "action is not allowed or contains invalid fields",
        )),
    };
    let response = match result {
        Ok(response) => match if let Some(reservation) = lifecycle_reservation.as_mut() {
            append_reserved_audit(
                &runtime,
                action.name(),
                "allowed",
                "request completed",
                reservation,
            )
        } else {
            append_audit(&runtime, action.name(), "allowed", "request completed")
        } {
            Ok(_) => response,
            Err(error) => {
                let rollback = match action {
                    ControlAction::RpcStart if !rpc_was_running => stop_rpc(&runtime).map(|_| ()),
                    ControlAction::RpcStop if rpc_was_running => Err(worker_agent_error(
                        "rpc-server stop could not be rolled back without recreating an unauthenticated loopback endpoint",
                    )),
                    _ => Ok(()),
                };
                let message = match rollback {
                    Ok(()) => error,
                    Err(rollback_error) => format!(
                        "{error}; lifecycle rollback also failed: {rollback_error}; the durable requested record marks this action as indeterminate"
                    ),
                };
                error_response(Some(&runtime), "AUDIT_UNAVAILABLE", &message)
            }
        },
        Err(error) => {
            let failure_audit = if let Some(reservation) = lifecycle_reservation.as_mut() {
                append_reserved_audit(&runtime, action.name(), "failed", &error, reservation)
            } else {
                append_audit(&runtime, action.name(), "failed", &error)
            };
            if let Err(audit_error) = failure_audit {
                error_response(Some(&runtime), "AUDIT_UNAVAILABLE", &audit_error)
            } else {
                error_response(Some(&runtime), "ACTION_FAILED", &error)
            }
        }
    };
    drop(lifecycle_reservation);
    response
}

async fn handle_control_connection(
    runtime: Arc<AgentRuntime>,
    acceptor: TlsAcceptor,
    stream: TcpStream,
    preauth_permit: OwnedSemaphorePermit,
    source_admission: SourceAdmission,
    authenticated_limit: Arc<Semaphore>,
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
    if request_authenticated(&runtime, &request) {
        drop(preauth_permit);
        drop(source_admission);
        let _authenticated_permit = authenticated_limit
            .try_acquire_owned()
            .map_err(|_| worker_agent_error("authenticated control capacity is saturated"))?;
        let response = dispatch_control(runtime, request).await;
        write_frame(&mut tls, &response).await?;
        let _ = tls.shutdown().await;
        return Ok(());
    }
    let response = dispatch_control(runtime, request).await;
    write_frame(&mut tls, &response).await?;
    let _ = tls.shutdown().await;
    Ok(())
}

async fn handle_tunnel_connection(
    runtime: Arc<AgentRuntime>,
    acceptor: TlsAcceptor,
    stream: TcpStream,
    preauth_permit: OwnedSemaphorePermit,
    source_admission: SourceAdmission,
    authenticated_limit: Arc<Semaphore>,
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
    let session_token = load_token(&runtime.config.token_path).ok();
    let authenticated = hello.protocol_version == AGENT_PROTOCOL_VERSION
        && hello.expected_agent_id == runtime.config.agent_id
        && session_token
            .as_ref()
            .is_some_and(|token| tokens_equal(token, &hello.token));
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
    if runtime.closing.load(Ordering::Acquire) {
        let mut tls = reader.into_inner();
        write_frame(
            &mut tls,
            &TunnelResponse {
                protocol_version: AGENT_PROTOCOL_VERSION,
                agent_id: runtime.config.agent_id.clone(),
                ok: false,
                error: Some(ControlError {
                    code: "AGENT_SHUTTING_DOWN".into(),
                    message: "Worker Agent is shutting down".into(),
                }),
            },
        )
        .await?;
        return Ok(());
    }
    let session_token = session_token.expect("authenticated tunnel must have a loaded token");
    drop(preauth_permit);
    drop(source_admission);
    let _authenticated_permit = match authenticated_limit.try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            let mut tls = reader.into_inner();
            write_frame(
                &mut tls,
                &TunnelResponse {
                    protocol_version: AGENT_PROTOCOL_VERSION,
                    agent_id: runtime.config.agent_id.clone(),
                    ok: false,
                    error: Some(ControlError {
                        code: "CAPACITY_EXHAUSTED".into(),
                        message: "authenticated tunnel capacity is saturated".into(),
                    }),
                },
            )
            .await?;
            return Ok(());
        }
    };
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
    verify_rpc_socket_owner(&runtime, &rpc)?;
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
    let mut forwarding = Box::pin(tokio::io::copy_bidirectional(&mut tls, &mut rpc));
    let lifetime = tokio::time::sleep(MAX_TUNNEL_LIFETIME);
    tokio::pin!(lifetime);
    let mut token_check = tokio::time::interval(Duration::from_millis(100));
    token_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let result = loop {
        tokio::select! {
            result = &mut forwarding => break result.map_err(|error| worker_agent_error(format!("tunnel forwarding failed: {error}"))),
            _ = &mut lifetime => break Err(worker_agent_error("encrypted rpc tunnel exceeded its maximum lifetime")),
            _ = token_check.tick() => {
                let current = load_token(&runtime.config.token_path);
                let revoked = match current {
                    Ok(token) => !tokens_equal(&token, &session_token),
                    Err(_) => true,
                };
                if revoked {
                    break Err(worker_agent_error("encrypted rpc tunnel revoked after token rotation"));
                }
            }
        }
    };
    drop(forwarding);
    let _ = tls.shutdown().await;
    let _ = rpc.shutdown().await;
    let outcome = if result.is_ok() { "closed" } else { "failed" };
    let audit_result = append_audit(&runtime, "tunnel", outcome, "encrypted rpc tunnel closed");
    result.map(|_| ())?;
    audit_result.map(|_| ())
}

struct ListenerAdmission {
    preauth_limit: Arc<Semaphore>,
    authenticated_limit: Arc<Semaphore>,
    source_counts: Arc<Mutex<HashMap<IpAddr, usize>>>,
    tunnel: bool,
}

async fn serve_listener(
    listener: TcpListener,
    runtime: Arc<AgentRuntime>,
    acceptor: TlsAcceptor,
    mut shutdown: watch::Receiver<bool>,
    admission: ListenerAdmission,
) -> Result<(), String> {
    let mut handlers = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            completed = handlers.join_next(), if !handlers.is_empty() => {
                let _ = completed;
            }
            accepted = listener.accept() => {
                let (stream, peer) = accepted
                    .map_err(|error| worker_agent_error(format!("listener failed: {error}")))?;
                let Some(source_admission) = try_admit_source(&admission.source_counts, peer.ip()) else {
                    drop(stream);
                    continue;
                };
                let Ok(permit) = admission.preauth_limit.clone().try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let runtime = runtime.clone();
                let acceptor = acceptor.clone();
                let authenticated_limit = admission.authenticated_limit.clone();
                let tunnel = admission.tunnel;
                handlers.spawn(async move {
                    let result = if tunnel {
                        handle_tunnel_connection(runtime, acceptor, stream, permit, source_admission, authenticated_limit).await
                    } else {
                        handle_control_connection(runtime, acceptor, stream, permit, source_admission, authenticated_limit).await
                    };
                    let _ = result;
                });
            }
        }
    }
    let drained = tokio::time::timeout(Duration::from_secs(5), async {
        while handlers.join_next().await.is_some() {}
    })
    .await;
    if drained.is_err() {
        handlers.shutdown().await;
    }
    Ok(())
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
        lifecycle_lock: Mutex::new(()),
        closing: AtomicBool::new(false),
        audit_lock: Mutex::new(()),
        audit_reserved_bytes: Mutex::new(0),
        auth_failure_audit: Mutex::new(AuthFailureAuditState::default()),
        #[cfg(test)]
        rpc_test_override: AtomicBool::new(false),
    });
    append_audit(&runtime, "agent", "started", "secure Worker Agent started")?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let control_preauth = Arc::new(Semaphore::new(MAX_PREAUTH_CONTROL_CONNECTIONS));
    let tunnel_preauth = Arc::new(Semaphore::new(MAX_PREAUTH_TUNNEL_CONNECTIONS));
    let authenticated_control = Arc::new(Semaphore::new(MAX_AUTHENTICATED_CONTROL_CONNECTIONS));
    let authenticated_tunnels = Arc::new(Semaphore::new(MAX_AUTHENTICATED_TUNNELS));
    let source_counts = Arc::new(Mutex::new(HashMap::new()));
    let control_task = tokio::spawn(serve_listener(
        control,
        runtime.clone(),
        acceptor.clone(),
        shutdown_rx.clone(),
        ListenerAdmission {
            preauth_limit: control_preauth,
            authenticated_limit: authenticated_control,
            source_counts: source_counts.clone(),
            tunnel: false,
        },
    ));
    let tunnel_task = tokio::spawn(serve_listener(
        tunnel,
        runtime.clone(),
        acceptor,
        shutdown_rx,
        ListenerAdmission {
            preauth_limit: tunnel_preauth,
            authenticated_limit: authenticated_tunnels,
            source_counts,
            tunnel: true,
        },
    ));
    tokio::signal::ctrl_c()
        .await
        .map_err(|error| worker_agent_error(format!("shutdown signal failed: {error}")))?;
    runtime.closing.store(true, Ordering::Release);
    let _ = shutdown_tx.send(true);
    control_task
        .await
        .map_err(|error| worker_agent_error(format!("control listener task failed: {error}")))??;
    tunnel_task
        .await
        .map_err(|error| worker_agent_error(format!("tunnel listener task failed: {error}")))??;
    stop_rpc(&runtime)?;
    append_audit(&runtime, "agent", "stopped", "secure Worker Agent stopped")?;
    Ok(())
}

async fn send_control(
    connection: &WorkerAgentConnection,
    action: ControlAction,
    limit: Option<usize>,
) -> Result<ControlResponse, String> {
    send_control_with_audit_cursor(connection, action, limit, None, None).await
}

async fn send_control_with_audit_cursor(
    connection: &WorkerAgentConnection,
    action: ControlAction,
    limit: Option<usize>,
    from_sequence: Option<u64>,
    checkpoint_hash: Option<String>,
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
        from_sequence,
        checkpoint_hash,
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
    const MAX_REMOTE_AUDIT_RECORDS: usize = 200_000;
    const MAX_REMOTE_AUDIT_BYTES: usize = 128 * 1024 * 1024;
    let mut sequence = connection.audit_sequence;
    let mut checkpoint_hash = connection.audit_hash.clone();
    let mut entries = Vec::new();
    let mut retained_bytes = 0_usize;
    loop {
        let response = send_control_with_audit_cursor(
            connection,
            ControlAction::Audit,
            Some(limit.clamp(1, MAX_AUDIT_RESULTS)),
            Some(sequence),
            Some(checkpoint_hash.clone()),
        )
        .await?;
        if response.audit.is_empty() && response.has_more {
            return Err(worker_agent_error(
                "Agent returned an empty non-terminal audit page",
            ));
        }
        for entry in response.audit {
            retained_bytes = retained_bytes
                .checked_add(
                    serde_json::to_vec(&entry)
                        .map_err(|error| worker_agent_error(error.to_string()))?
                        .len(),
                )
                .ok_or_else(|| worker_agent_error("remote audit retained-size overflow"))?;
            if entries.len() >= MAX_REMOTE_AUDIT_RECORDS || retained_bytes > MAX_REMOTE_AUDIT_BYTES
            {
                return Err(worker_agent_error(
                    "remote audit exceeds the manager work budget",
                ));
            }
            sequence = entry.sequence;
            checkpoint_hash = entry.hash.clone();
            entries.push(entry);
        }
        if !response.has_more {
            break;
        }
    }
    Ok(entries)
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
    let result = tokio::time::timeout(
        MAX_TUNNEL_LIFETIME,
        tokio::io::copy_bidirectional(&mut local, &mut tls),
    )
    .await
    .map_err(|_| worker_agent_error("local tunnel bridge exceeded its maximum lifetime"))?;
    let _ = local.shutdown().await;
    let _ = tls.shutdown().await;
    result
        .map(|_| ())
        .map_err(|error| worker_agent_error(format!("local tunnel bridge failed: {error}")))
}

#[cfg(windows)]
fn windows_process_sid(process_id: Option<u32>) -> Result<String, String> {
    crate::persistence::windows_process_sid(process_id).map_err(worker_agent_error)
}

fn tracked_rpc_child_pid(runtime: &AgentRuntime) -> Result<u32, String> {
    let mut child = runtime
        .rpc_child
        .lock()
        .map_err(|_| worker_agent_error("rpc process lock is poisoned"))?;
    let child = child
        .as_mut()
        .ok_or_else(|| worker_agent_error("rpc-server is not tracked by the Agent"))?;
    if child
        .try_wait()
        .map_err(|error| worker_agent_error(format!("failed to inspect rpc-server: {error}")))?
        .is_some()
    {
        return Err(worker_agent_error("tracked rpc-server has exited"));
    }
    Ok(child.id())
}

#[cfg(windows)]
fn verify_rpc_socket_owner(runtime: &AgentRuntime, stream: &TcpStream) -> Result<(), String> {
    #[cfg(test)]
    if runtime.rpc_test_override.load(Ordering::Acquire) {
        return Ok(());
    }
    let expected = tracked_rpc_child_pid(runtime)?;
    let actual = windows_tcp_peer_pid(stream)?;
    if actual != expected {
        return Err(worker_agent_error(
            "rpc loopback socket is not owned by the tracked rpc-server child",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn verify_rpc_socket_owner(runtime: &AgentRuntime, stream: &TcpStream) -> Result<(), String> {
    #[cfg(test)]
    if runtime.rpc_test_override.load(Ordering::Acquire) {
        return Ok(());
    }
    let expected = tracked_rpc_child_pid(runtime)?;
    let actual = linux_tcp_peer_pid(stream)?;
    if actual != expected {
        return Err(worker_agent_error(
            "rpc loopback socket is not owned by the tracked rpc-server child",
        ));
    }
    Ok(())
}

#[cfg(all(not(windows), not(target_os = "linux")))]
fn verify_rpc_socket_owner(_runtime: &AgentRuntime, _stream: &TcpStream) -> Result<(), String> {
    Err(worker_agent_error(
        "verified rpc-server socket ownership is unavailable on this platform",
    ))
}

#[cfg(windows)]
fn windows_tcp_peer_pid(stream: &TcpStream) -> Result<u32, String> {
    use std::ptr;
    use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetTcpTable2, MIB_TCPTABLE2, MIB_TCP_STATE_ESTAB,
    };

    let peer = stream
        .peer_addr()
        .map_err(|error| worker_agent_error(error.to_string()))?;
    let local = stream
        .local_addr()
        .map_err(|error| worker_agent_error(error.to_string()))?;
    let (SocketAddr::V4(peer), SocketAddr::V4(local)) = (peer, local) else {
        return Err(worker_agent_error(
            "local bridge owner lookup requires an IPv4 loopback socket",
        ));
    };
    let mut length = 0_u32;
    let initial = unsafe { GetTcpTable2(ptr::null_mut(), &mut length, 0) };
    if initial != ERROR_INSUFFICIENT_BUFFER || length == 0 {
        return Err(worker_agent_error("failed to size the TCP owner table"));
    }
    let mut table = vec![0_u8; length as usize];
    let status =
        unsafe { GetTcpTable2(table.as_mut_ptr().cast::<MIB_TCPTABLE2>(), &mut length, 0) };
    if status != NO_ERROR {
        return Err(worker_agent_error("failed to inspect the TCP owner table"));
    }
    let table = table.as_ptr().cast::<MIB_TCPTABLE2>();
    let rows = unsafe {
        std::slice::from_raw_parts((*table).table.as_ptr(), (*table).dwNumEntries as usize)
    };
    let mut matched_pid = None;
    for row in rows {
        let row_local_port = u16::from_be((row.dwLocalPort & 0xffff) as u16);
        let row_remote_port = u16::from_be((row.dwRemotePort & 0xffff) as u16);
        let row_local_addr = std::net::Ipv4Addr::from(u32::from_be(row.dwLocalAddr));
        let row_remote_addr = std::net::Ipv4Addr::from(u32::from_be(row.dwRemoteAddr));
        if row.dwState == MIB_TCP_STATE_ESTAB as u32
            && row_local_addr == *peer.ip()
            && row_remote_addr == *local.ip()
            && row_local_port == peer.port()
            && row_remote_port == local.port()
            && matched_pid.replace(row.dwOwningPid).is_some()
        {
            return Err(worker_agent_error(
                "local bridge peer owner lookup is ambiguous",
            ));
        }
    }
    matched_pid.ok_or_else(|| worker_agent_error("local bridge peer PID was not found"))
}

#[cfg(windows)]
async fn authorize_local_bridge_peer(stream: &TcpStream) -> Result<(), String> {
    let mut last_error = None;
    for _ in 0..5 {
        match windows_tcp_peer_pid(stream) {
            Ok(pid) => {
                if windows_process_sid(Some(pid))? != windows_process_sid(None)? {
                    return Err(worker_agent_error(
                        "local bridge peer belongs to another OS user",
                    ));
                }
                if crate::deployment_identity::is_authorized_launch_process(pid) {
                    return Ok(());
                }
                return Err(worker_agent_error(
                    "local bridge peer is not a manager-authorized server process",
                ));
            }
            Err(error) => last_error = Some(error),
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Err(last_error.unwrap_or_else(|| worker_agent_error("local bridge peer is unauthorized")))
}

#[cfg(all(unix, target_os = "linux"))]
fn linux_proc_ipv4_endpoint(value: &str) -> Option<std::net::SocketAddrV4> {
    let (address, port) = value.split_once(':')?;
    if address.len() != 8 {
        return None;
    }
    let encoded = u32::from_str_radix(address, 16).ok()?;
    let address = std::net::Ipv4Addr::from(encoded.to_le_bytes());
    let port = u16::from_str_radix(port, 16).ok()?;
    Some(std::net::SocketAddrV4::new(address, port))
}

#[cfg(all(unix, target_os = "linux"))]
fn linux_tcp_peer_pid(stream: &TcpStream) -> Result<u32, String> {
    let peer = stream
        .peer_addr()
        .map_err(|error| worker_agent_error(error.to_string()))?;
    let local = stream
        .local_addr()
        .map_err(|error| worker_agent_error(error.to_string()))?;
    let (SocketAddr::V4(peer), SocketAddr::V4(local)) = (peer, local) else {
        return Err(worker_agent_error(
            "local bridge owner lookup requires an IPv4 loopback socket",
        ));
    };
    let table = std::fs::read_to_string("/proc/net/tcp").map_err(|error| {
        worker_agent_error(format!("failed to inspect local TCP owners: {error}"))
    })?;
    let mut socket_match = None;
    for line in table.lines().skip(1) {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 10 || fields[3] != "01" {
            continue;
        }
        let row_local = linux_proc_ipv4_endpoint(fields[1]);
        let row_remote = linux_proc_ipv4_endpoint(fields[2]);
        if row_local.as_ref() == Some(peer) && row_remote.as_ref() == Some(local) {
            let candidate = (fields[7].parse::<u32>().ok(), fields[9].parse::<u64>().ok());
            if socket_match.replace(candidate).is_some() {
                return Err(worker_agent_error(
                    "local bridge peer owner lookup is ambiguous",
                ));
            }
        }
    }
    let (socket_uid, socket_inode) =
        socket_match.ok_or_else(|| worker_agent_error("local bridge peer socket was not found"))?;
    let uid =
        socket_uid.ok_or_else(|| worker_agent_error("local bridge peer UID was not found"))?;
    if uid != unsafe { libc::geteuid() } {
        return Err(worker_agent_error(
            "local bridge peer belongs to another OS user",
        ));
    }
    let inode =
        socket_inode.ok_or_else(|| worker_agent_error("local bridge peer socket was not found"))?;
    let expected = format!("socket:[{inode}]");
    let processes = std::fs::read_dir("/proc")
        .map_err(|error| worker_agent_error(format!("failed to inspect /proc: {error}")))?;
    let mut inspected = 0_usize;
    for process in processes.flatten() {
        let Some(pid) = process
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        inspected += 1;
        if inspected > 16_384 {
            return Err(worker_agent_error(
                "local bridge process lookup exceeded its work budget",
            ));
        }
        let Ok(descriptors) = std::fs::read_dir(process.path().join("fd")) else {
            continue;
        };
        for descriptor in descriptors.flatten().take(4_096) {
            if std::fs::read_link(descriptor.path())
                .ok()
                .is_some_and(|target| target == std::path::Path::new(&expected))
            {
                return Ok(pid);
            }
        }
    }
    Err(worker_agent_error("local bridge peer PID was not found"))
}

#[cfg(all(unix, target_os = "linux"))]
async fn authorize_local_bridge_peer(stream: &TcpStream) -> Result<(), String> {
    let pid = linux_tcp_peer_pid(stream)?;
    if crate::deployment_identity::is_authorized_launch_process(pid) {
        return Ok(());
    }
    Err(worker_agent_error(
        "local bridge peer is not a manager-authorized server process",
    ))
}

#[cfg(all(unix, not(target_os = "linux")))]
async fn authorize_local_bridge_peer(_stream: &TcpStream) -> Result<(), String> {
    Err(worker_agent_error(
        "owner-verified local bridges are unavailable on this platform",
    ))
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
        let connection_limit = Arc::new(Semaphore::new(MAX_LOCAL_BRIDGE_CONNECTIONS));
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
                            let Ok(permit) = connection_limit.clone().try_acquire_owned() else {
                                drop(stream);
                                continue;
                            };
                            let connection = match connection.lock() {
                                Ok(connection) => connection.clone(),
                                Err(_) => {
                                    eprintln!("{}", worker_agent_error("bridge connection lock is poisoned"));
                                    continue;
                                }
                            };
                            connections.spawn(async move {
                                let _permit = permit;
                                if authorize_local_bridge_peer(&stream).await.is_err() {
                                    return;
                                }
                                // Re-resolve the exact four-tuple immediately
                                // before credentialed forwarding to close the
                                // owner-lookup race window.
                                if authorize_local_bridge_peer(&stream).await.is_err() {
                                    return;
                                }
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
    std::fs::create_dir_all(parent).map_err(|error| {
        worker_agent_error(format!("failed to create config directory: {error}"))
    })?;
    crate::persistence::enforce_private_directory(parent).map_err(worker_agent_error)?;
    let canonical_parent = std::fs::canonicalize(parent)
        .map_err(|error| worker_agent_error(format!("config directory is unavailable: {error}")))?;
    for (path, label) in [
        (&token_path, "token file"),
        (&audit_path, "audit log"),
        (&rpc_log_path, "RPC log"),
    ] {
        let candidate_parent = path
            .parent()
            .ok_or_else(|| worker_agent_error(format!("{label} has no parent directory")))?;
        let candidate_parent = std::fs::canonicalize(candidate_parent).map_err(|error| {
            worker_agent_error(format!("{label} directory is unavailable: {error}"))
        })?;
        if !crate::path_utils::paths_equal(&candidate_parent, &canonical_parent) {
            return Err(worker_agent_error(format!(
                "{label} must reside directly in the private Agent config directory"
            )));
        }
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
    let rpc_binary_path = validate_rpc_binary(&rpc_binary_path)?;
    let rpc_executable =
        crate::deployment_identity::ArtifactLease::open_owner_protected_executable(
            &rpc_binary_path,
        )
        .map_err(worker_agent_error)?;
    let rpc_artifact_identity = rpc_executable.identity().clone();
    let rpc_binary_path = rpc_executable.canonical_path().to_path_buf();
    drop(rpc_executable);
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
        rpc_artifact_identity,
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
    // Running Agent processes revalidate tunnel generations every 100 ms.
    // Wait for that revocation window before reporting rotation success.
    std::thread::sleep(Duration::from_millis(500));
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
                rpc_artifact_identity: crate::deployment_identity::ArtifactIdentity::default(),
                rpc_port: 50052,
                audit_path: directory.join("audit.jsonl"),
                rpc_log_path: directory.join("rpc.log"),
                devices: Vec::new(),
            },
            certificate_sha256: "a".repeat(64),
            rpc_child: Mutex::new(None),
            lifecycle_lock: Mutex::new(()),
            closing: AtomicBool::new(false),
            audit_lock: Mutex::new(()),
            audit_reserved_bytes: Mutex::new(0),
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
                from_sequence: None,
                checkpoint_hash: None,
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

    #[tokio::test]
    async fn rpc_start_fails_closed_without_a_private_child_transport() {
        let directory = test_directory("rpc-isolation");
        std::fs::create_dir_all(&directory).unwrap();
        let runtime = test_runtime(&directory);
        let response = dispatch_control(
            runtime.clone(),
            ControlRequest {
                protocol_version: AGENT_PROTOCOL_VERSION,
                token: "a".repeat(64),
                expected_agent_id: runtime.config.agent_id.clone(),
                action: ControlAction::RpcStart,
                limit: None,
                from_sequence: None,
                checkpoint_hash: None,
            },
        )
        .await;
        assert!(!response.ok);
        assert_eq!(
            response.error.as_ref().map(|error| error.code.as_str()),
            Some("ACTION_FAILED")
        );
        assert!(response.error.as_ref().is_some_and(|error| error
            .message
            .contains("unauthenticated loopback TCP endpoint")));
        assert!(runtime.rpc_child.lock().unwrap().is_none());
        let audit = read_verified_audit(&runtime.config.audit_path).unwrap();
        assert_eq!(audit.len(), 2);
        assert_eq!(audit[1].outcome, "failed");
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
        assert!(entries.is_empty());
        let denial_path = runtime.config.audit_path.with_extension("auth-denials.log");
        let denials = std::fs::read_to_string(denial_path).unwrap();
        assert_eq!(denials.lines().count(), 1);
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
            audit_sequence: 0,
            audit_hash: String::new(),
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

    #[test]
    fn manager_audit_checkpoint_rejects_rollbacks_and_forks() {
        let agent_id = uuid::Uuid::new_v4().to_string();
        let make_entry = |sequence: u64, previous_hash: String| {
            let mut entry = WorkerAgentAuditEntry {
                sequence,
                timestamp: chrono::Utc::now().to_rfc3339(),
                agent_id: agent_id.clone(),
                event: "status".into(),
                outcome: "allowed".into(),
                detail: "request completed".into(),
                previous_hash,
                hash: String::new(),
            };
            entry.hash = audit_hash(&entry).unwrap();
            entry
        };
        let first = make_entry(1, String::new());
        let second = make_entry(2, first.hash.clone());
        let checkpoint =
            verify_remote_audit_extension(&agent_id, 0, "", &[first.clone(), second.clone()])
                .unwrap();
        assert_eq!(checkpoint, (2, second.hash.clone()));
        assert!(verify_remote_audit_extension(
            &agent_id,
            checkpoint.0,
            &checkpoint.1,
            std::slice::from_ref(&first),
        )
        .is_err());

        let fork = make_entry(3, "forged-previous-hash".into());
        assert!(verify_remote_audit_extension(
            &agent_id,
            checkpoint.0,
            &checkpoint.1,
            &[second.clone(), fork],
        )
        .is_err());
        let extension = make_entry(3, second.hash.clone());
        assert_eq!(
            verify_remote_audit_extension(
                &agent_id,
                checkpoint.0,
                &checkpoint.1,
                &[second, extension.clone()],
            )
            .unwrap(),
            (3, extension.hash)
        );
    }

    #[test]
    fn audit_pages_advance_only_from_an_exact_hash_checkpoint() {
        let directory = test_directory("audit-pages");
        std::fs::create_dir_all(&directory).unwrap();
        let runtime = test_runtime(&directory);
        for detail in ["first", "second", "third"] {
            append_audit(&runtime, "status", "allowed", detail).unwrap();
        }

        let first_page = audit_page(
            &runtime,
            &ControlRequest {
                protocol_version: AGENT_PROTOCOL_VERSION,
                token: String::new(),
                expected_agent_id: runtime.config.agent_id.clone(),
                action: ControlAction::Audit,
                limit: Some(2),
                from_sequence: Some(0),
                checkpoint_hash: None,
            },
        )
        .unwrap();
        assert_eq!(
            first_page
                .audit
                .iter()
                .map(|entry| entry.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(first_page.has_more);
        assert_eq!(first_page.next_sequence, Some(2));

        let checkpoint_hash = first_page.audit.last().unwrap().hash.clone();
        let second_page = audit_page(
            &runtime,
            &ControlRequest {
                protocol_version: AGENT_PROTOCOL_VERSION,
                token: String::new(),
                expected_agent_id: runtime.config.agent_id.clone(),
                action: ControlAction::Audit,
                limit: Some(2),
                from_sequence: Some(2),
                checkpoint_hash: Some(checkpoint_hash),
            },
        )
        .unwrap();
        assert_eq!(second_page.audit.len(), 1);
        assert_eq!(second_page.audit[0].sequence, 3);
        assert!(!second_page.has_more);
        assert!(audit_page(
            &runtime,
            &ControlRequest {
                protocol_version: AGENT_PROTOCOL_VERSION,
                token: String::new(),
                expected_agent_id: runtime.config.agent_id.clone(),
                action: ControlAction::Audit,
                limit: Some(2),
                from_sequence: Some(2),
                checkpoint_hash: Some("0".repeat(64)),
            },
        )
        .is_err());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn audit_rotation_preserves_one_verified_logical_chain() {
        let directory = test_directory("audit-rotation");
        std::fs::create_dir_all(&directory).unwrap();
        let runtime = test_runtime(&directory);
        append_audit(
            &runtime,
            "status",
            "allowed",
            "x".repeat(AUDIT_ROTATE_AT_BYTES as usize),
        )
        .unwrap();
        append_audit(&runtime, "status", "allowed", "after rotation").unwrap();

        let segments = audit_segment_paths(&runtime.config.audit_path).unwrap();
        assert_eq!(segments.len(), 1);
        let entries = read_verified_audit(&runtime.config.audit_path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].sequence, 2);
        assert_eq!(entries[1].previous_hash, entries[0].hash);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn audit_rotation_refuses_to_overwrite_a_full_retention_set() {
        let directory = test_directory("audit-retention");
        std::fs::create_dir_all(&directory).unwrap();
        let runtime = test_runtime(&directory);
        append_audit(
            &runtime,
            "status",
            "allowed",
            "x".repeat(AUDIT_ROTATE_AT_BYTES as usize),
        )
        .unwrap();
        for index in 0..16 {
            let segment = runtime
                .config
                .audit_path
                .with_file_name(format!("audit.jsonl.segment-{index:020}-{index:020}"));
            crate::persistence::atomic_write(&segment, b"", None).unwrap();
        }

        let error = append_audit(&runtime, "status", "allowed", "must fail").unwrap_err();
        assert!(error.contains("retention is full"));
        assert_eq!(
            read_verified_audit(&runtime.config.audit_path)
                .unwrap()
                .len(),
            1
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(any(windows, target_os = "linux"))]
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
            rpc_artifact_identity: crate::deployment_identity::ArtifactIdentity::default(),
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
            lifecycle_lock: Mutex::new(()),
            closing: AtomicBool::new(false),
            audit_lock: Mutex::new(()),
            audit_reserved_bytes: Mutex::new(0),
            auth_failure_audit: Mutex::new(AuthFailureAuditState::default()),
            rpc_test_override: AtomicBool::new(true),
        });
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let source_counts = Arc::new(Mutex::new(HashMap::new()));
        let control_task = tokio::spawn(serve_listener(
            control_listener,
            runtime.clone(),
            acceptor.clone(),
            shutdown_rx.clone(),
            ListenerAdmission {
                preauth_limit: Arc::new(Semaphore::new(MAX_PREAUTH_CONTROL_CONNECTIONS)),
                authenticated_limit: Arc::new(Semaphore::new(
                    MAX_AUTHENTICATED_CONTROL_CONNECTIONS,
                )),
                source_counts: source_counts.clone(),
                tunnel: false,
            },
        ));
        let tunnel_task = tokio::spawn(serve_listener(
            tunnel_listener,
            runtime,
            acceptor,
            shutdown_rx,
            ListenerAdmission {
                preauth_limit: Arc::new(Semaphore::new(MAX_PREAUTH_TUNNEL_CONNECTIONS)),
                authenticated_limit: Arc::new(Semaphore::new(MAX_AUTHENTICATED_TUNNELS)),
                source_counts,
                tunnel: true,
            },
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
            audit_sequence: 0,
            audit_hash: String::new(),
        };
        let remote = get_remote_status(&connection).await.unwrap();
        assert_eq!(remote.agent_id, config.agent_id);
        assert!(remote.rpc_running);

        let local_port = ensure_manager_bridge("e2e-worker", connection.clone(), 0)
            .await
            .unwrap();
        let (test_start_time, test_executable) =
            crate::commands::server::read_process_identity(std::process::id()).unwrap();
        let _test_process_authorization =
            crate::deployment_identity::register_authorized_launch_process(
                "worker-agent-e2e-client",
                std::process::id(),
                test_start_time,
                &test_executable,
            );
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
        crate::persistence::enforce_private_directory(&directory).unwrap();
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_path = directory.join("agent.crt");
        let key_path = directory.join("agent.key");
        let config_path = directory.join("agent.json");
        let rpc_path = directory.join(expected_rpc_binary_name());
        std::fs::write(&cert_path, certified.cert.pem()).unwrap();
        std::fs::write(&key_path, certified.key_pair.serialize_pem()).unwrap();
        std::fs::write(&rpc_path, b"test fixture").unwrap();
        crate::persistence::enforce_private_file(&rpc_path).unwrap();
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
