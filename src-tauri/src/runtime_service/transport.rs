use super::protocol::{
    RuntimeRequest, RuntimeResponse, MAX_RUNTIME_FRAME_BYTES, RUNTIME_PROTOCOL_VERSION,
};
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const RUNTIME_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const RUNTIME_IO_TIMEOUT: Duration = Duration::from_secs(10);
const RUNTIME_RESPONSE_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_RUNTIME_PREAUTH_CONNECTIONS: usize = 16;
const RUNTIME_HANDSHAKE_DOMAIN: &[u8] = b"llama-server-manager:runtime-handshake:v1\0";
#[cfg(any(unix, test))]
const MAX_RUNTIME_SOCKET_PATH_BYTES: usize = 90;
#[cfg(any(unix, test))]
const SHORT_RUNTIME_SOCKET_ROOT: &str = "/tmp";

#[cfg(any(unix, test))]
fn stable_path_hash(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn runtime_dir() -> PathBuf {
    crate::utils::get_data_dir().join("runtime")
}

pub fn runtime_state_path() -> PathBuf {
    runtime_dir().join("runtime-state.json")
}

pub fn control_token_path() -> PathBuf {
    runtime_dir().join("control-token")
}

pub fn service_log_path() -> PathBuf {
    runtime_dir().join("runtime-service.log")
}

pub fn runtime_lock_path() -> PathBuf {
    runtime_dir().join("runtime-service.lock")
}

pub fn service_pid_path() -> PathBuf {
    runtime_dir().join("runtime-service.pid")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RuntimeHandshakeRequest {
    protocol_version: u32,
    nonce: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RuntimeHandshakeResponse {
    protocol_version: u32,
    nonce: String,
    service_pid: u32,
    proof: String,
}

fn handshake_proof(control_token: &str, nonce: &str, service_pid: u32) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(RUNTIME_HANDSHAKE_DOMAIN);
    digest.update(control_token.as_bytes());
    digest.update([0]);
    digest.update(nonce.as_bytes());
    digest.update([0]);
    digest.update(service_pid.to_le_bytes());
    format!("{:x}", digest.finalize())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn persist_service_pid() -> Result<(), String> {
    crate::persistence::atomic_write(
        &service_pid_path(),
        std::process::id().to_string().as_bytes(),
        None,
    )
}

fn expected_service_pid() -> Result<u32, String> {
    crate::persistence::enforce_private_file(&service_pid_path())?;
    std::fs::read_to_string(service_pid_path())
        .map_err(|error| format!("failed to read runtime service identity: {error}"))?
        .trim()
        .parse::<u32>()
        .map_err(|_| "runtime service identity is invalid".to_string())
}

async fn authenticate_runtime_server<S>(stream: &mut S, control_token: &str) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let nonce = uuid::Uuid::new_v4().to_string();
    let handshake = RuntimeHandshakeRequest {
        protocol_version: RUNTIME_PROTOCOL_VERSION,
        nonce: nonce.clone(),
    };
    let handshake = serde_json::to_vec(&handshake)
        .map_err(|error| format!("failed to encode runtime handshake: {error}"))?;
    tokio::time::timeout(RUNTIME_IO_TIMEOUT, write_frame(stream, &handshake))
        .await
        .map_err(|_| "timed out writing runtime handshake".to_string())??;
    let handshake = tokio::time::timeout(RUNTIME_IO_TIMEOUT, read_frame(stream))
        .await
        .map_err(|_| "timed out reading runtime handshake".to_string())??;
    let handshake: RuntimeHandshakeResponse = serde_json::from_slice(&handshake)
        .map_err(|error| format!("failed to decode runtime handshake: {error}"))?;
    let expected_pid = expected_service_pid()?;
    let expected_proof = handshake_proof(control_token, &nonce, expected_pid);
    if handshake.protocol_version != RUNTIME_PROTOCOL_VERSION
        || handshake.nonce != nonce
        || handshake.service_pid != expected_pid
        || !constant_time_eq(handshake.proof.as_bytes(), expected_proof.as_bytes())
    {
        return Err("runtime server authentication failed before sending credentials".into());
    }
    Ok(())
}

pub(super) fn acquire_runtime_lock() -> Result<Option<File>, String> {
    let runtime_dir = runtime_dir();
    std::fs::create_dir_all(&runtime_dir)
        .map_err(|error| format!("failed to create runtime directory: {error}"))?;
    let path = runtime_lock_path();
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| format!("failed to open runtime lock {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("failed to protect runtime lock: {error}"))?;
    }
    match FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(file)),
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            Ok(None)
        }
        Err(error) => Err(format!("failed to acquire runtime lock: {error}")),
    }
}

fn endpoint_suffix(control_token: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;

    let digest = Sha256::digest(control_token.as_bytes());
    let mut suffix = String::with_capacity(32);
    for byte in &digest[..16] {
        write!(&mut suffix, "{byte:02x}").expect("writing to a String cannot fail");
    }
    suffix
}

#[cfg(any(unix, test))]
fn socket_path_fits(path: &std::path::Path) -> bool {
    path.to_string_lossy().len() <= MAX_RUNTIME_SOCKET_PATH_BYTES
}

#[cfg(any(unix, test))]
fn control_socket_path_for(
    data_dir: &std::path::Path,
    temp_dir: &std::path::Path,
    control_token: &str,
) -> PathBuf {
    let suffix = endpoint_suffix(control_token);
    let preferred = data_dir
        .join("runtime")
        .join(format!("control-{suffix}.sock"));
    if socket_path_fits(&preferred) {
        return preferred;
    }
    let data_suffix = stable_path_hash(&data_dir.to_string_lossy());
    let fallback = temp_dir
        .join(format!("lsm-{data_suffix:016x}-{suffix}"))
        .join(format!("control-{suffix}.sock"));
    if socket_path_fits(&fallback) {
        return fallback;
    }
    // macOS per-user temp directories can be too long for sockaddr_un. The
    // child directory below is still ownership-checked and restricted to 0700.
    std::path::Path::new(SHORT_RUNTIME_SOCKET_ROOT)
        .join(format!("lsm-{suffix}"))
        .join(format!("control-{suffix}.sock"))
}

#[cfg(unix)]
pub fn control_socket_path(control_token: &str) -> PathBuf {
    control_socket_path_for(
        &crate::utils::get_data_dir(),
        &std::env::temp_dir(),
        control_token,
    )
}

#[cfg(windows)]
pub fn control_pipe_name(control_token: &str) -> String {
    let suffix = endpoint_suffix(control_token);
    format!(r"\\.\pipe\llama-server-manager-runtime-{suffix}")
}

#[cfg(unix)]
fn protect_and_validate_socket_parent(parent: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    if let Ok(metadata) = std::fs::symlink_metadata(parent) {
        if metadata.file_type().is_symlink() {
            return Err("runtime socket directory cannot be a symlink".to_string());
        }
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err("runtime socket directory is owned by another user".to_string());
        }
    } else {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create runtime socket directory: {error}"))?;
    }
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("failed to protect runtime socket directory: {error}"))?;
    let metadata = std::fs::symlink_metadata(parent)
        .map_err(|error| format!("failed to inspect runtime socket directory: {error}"))?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
        return Err("runtime socket directory is not private to the current user".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn validate_control_socket(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    let parent = path
        .parent()
        .ok_or_else(|| "runtime control socket has no parent directory".to_string())?;
    protect_and_validate_socket_parent(parent)?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect runtime control socket: {error}"))?;
    if !metadata.file_type().is_socket() {
        return Err("runtime control endpoint is not a Unix socket".to_string());
    }
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
        return Err("runtime control socket is not private to the current user".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn cleanup_stale_control_sockets(parent: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    for entry in std::fs::read_dir(parent)
        .map_err(|error| format!("failed to inspect runtime socket directory: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("failed to inspect runtime socket entry: {error}"))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != "control.sock" && !(name.starts_with("control-") && name.ends_with(".sock")) {
            continue;
        }
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect stale runtime endpoint: {error}"))?;
        if metadata.file_type().is_symlink()
            || (metadata.file_type().is_socket() && metadata.uid() == unsafe { libc::geteuid() })
        {
            std::fs::remove_file(&path)
                .map_err(|error| format!("failed to remove stale runtime endpoint: {error}"))?;
        } else {
            return Err(format!(
                "refusing to replace unexpected runtime endpoint {}",
                path.display()
            ));
        }
    }
    Ok(())
}

async fn read_frame<S>(stream: &mut S) -> Result<Vec<u8>, String>
where
    S: AsyncRead + Unpin,
{
    let mut length = [0_u8; 4];
    stream
        .read_exact(&mut length)
        .await
        .map_err(|error| format!("failed to read runtime frame length: {error}"))?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > MAX_RUNTIME_FRAME_BYTES {
        return Err(format!("invalid runtime frame length: {length}"));
    }
    let mut payload = vec![0_u8; length];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(|error| format!("failed to read runtime frame: {error}"))?;
    Ok(payload)
}

async fn write_frame<S>(stream: &mut S, payload: &[u8]) -> Result<(), String>
where
    S: AsyncWrite + Unpin,
{
    if payload.is_empty() || payload.len() > MAX_RUNTIME_FRAME_BYTES {
        return Err(format!(
            "invalid runtime response length: {}",
            payload.len()
        ));
    }
    stream
        .write_all(&(payload.len() as u32).to_le_bytes())
        .await
        .map_err(|error| format!("failed to write runtime frame length: {error}"))?;
    stream
        .write_all(payload)
        .await
        .map_err(|error| format!("failed to write runtime frame: {error}"))?;
    stream
        .flush()
        .await
        .map_err(|error| format!("failed to flush runtime frame: {error}"))
}

async fn handle_connection<S, H, F>(mut stream: S, control_token: String, handler: H)
where
    S: AsyncRead + AsyncWrite + Unpin,
    H: Fn(RuntimeRequest) -> F,
    F: Future<Output = RuntimeResponse>,
{
    let handshake = match tokio::time::timeout(RUNTIME_IO_TIMEOUT, read_frame(&mut stream)).await {
        Ok(Ok(payload)) => serde_json::from_slice::<RuntimeHandshakeRequest>(&payload).ok(),
        _ => None,
    };
    let Some(handshake) = handshake.filter(|handshake| {
        handshake.protocol_version == RUNTIME_PROTOCOL_VERSION
            && !handshake.nonce.is_empty()
            && handshake.nonce.len() <= 128
    }) else {
        return;
    };
    let handshake_response = RuntimeHandshakeResponse {
        protocol_version: RUNTIME_PROTOCOL_VERSION,
        nonce: handshake.nonce.clone(),
        service_pid: std::process::id(),
        proof: handshake_proof(&control_token, &handshake.nonce, std::process::id()),
    };
    let Ok(payload) = serde_json::to_vec(&handshake_response) else {
        return;
    };
    match tokio::time::timeout(RUNTIME_IO_TIMEOUT, write_frame(&mut stream, &payload)).await {
        Ok(Ok(())) => {}
        _ => return,
    }
    let response = match tokio::time::timeout(RUNTIME_IO_TIMEOUT, read_frame(&mut stream)).await {
        Ok(Ok(payload)) => match serde_json::from_slice::<RuntimeRequest>(&payload) {
            Ok(request) => handler(request).await,
            Err(error) => RuntimeResponse::failure(
                "invalid-request".into(),
                format!("invalid runtime request: {error}"),
            ),
        },
        Ok(Err(error)) => RuntimeResponse::failure("invalid-frame".into(), error),
        Err(_) => RuntimeResponse::failure(
            "request-timeout".into(),
            "runtime request timed out before a complete frame was received",
        ),
    };
    if let Ok(payload) = serde_json::to_vec(&response) {
        let _ = tokio::time::timeout(RUNTIME_IO_TIMEOUT, write_frame(&mut stream, &payload)).await;
    }
}

#[cfg(unix)]
pub async fn run_server<I, IF, H, F>(
    _runtime_lock: File,
    control_token: String,
    initializer: I,
    handler: H,
) -> Result<(), String>
where
    I: FnOnce() -> IF,
    IF: Future<Output = Result<(), String>>,
    H: Fn(RuntimeRequest) -> F + Clone + Send + Sync + 'static,
    F: Future<Output = RuntimeResponse> + Send + 'static,
{
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixListener;

    let socket_path = control_socket_path(&control_token);
    let parent = socket_path
        .parent()
        .ok_or_else(|| "runtime control socket has no parent directory".to_string())?;
    protect_and_validate_socket_parent(parent)?;
    cleanup_stale_control_sockets(parent)?;

    let listener = UnixListener::bind(&socket_path)
        .map_err(|error| format!("failed to bind runtime control socket: {error}"))?;
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("failed to protect runtime control socket: {error}"))?;
    initializer().await?;
    persist_service_pid()?;
    let preauth_limit =
        std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_RUNTIME_PREAUTH_CONNECTIONS));

    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|error| format!("runtime control socket failed: {error}"))?;
        let handler = handler.clone();
        let control_token = control_token.clone();
        let Ok(permit) = preauth_limit.clone().try_acquire_owned() else {
            drop(stream);
            continue;
        };
        tokio::spawn(async move {
            let _permit = permit;
            handle_connection(stream, control_token, handler).await
        });
    }
}

#[cfg(windows)]
pub async fn run_server<I, IF, H, F>(
    _runtime_lock: File,
    control_token: String,
    initializer: I,
    handler: H,
) -> Result<(), String>
where
    I: FnOnce() -> IF,
    IF: Future<Output = Result<(), String>>,
    H: Fn(RuntimeRequest) -> F + Clone + Send + Sync + 'static,
    F: Future<Output = RuntimeResponse> + Send + 'static,
{
    use tokio::net::windows::named_pipe::ServerOptions;
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

    let pipe_name = control_pipe_name(&control_token);
    let mut first_instance = true;
    let mut initializer = Some(initializer);
    let preauth_limit =
        std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_RUNTIME_PREAUTH_CONNECTIONS));
    loop {
        let mut options = ServerOptions::new();
        options.reject_remote_clients(true);
        if first_instance {
            options.first_pipe_instance(true);
        }
        let server = crate::persistence::with_private_security_descriptor(|descriptor| {
            let mut attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            unsafe {
                options.create_with_security_attributes_raw(
                    &pipe_name,
                    (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
                )
            }
            .map_err(|error| format!("failed to create private runtime control pipe: {error}"))
        })?;
        first_instance = false;
        if let Some(initializer) = initializer.take() {
            initializer().await?;
            persist_service_pid()?;
        }
        server
            .connect()
            .await
            .map_err(|error| format!("runtime control pipe failed: {error}"))?;
        if validate_named_pipe_client(&server).is_err() {
            drop(server);
            continue;
        }
        let Ok(permit) = preauth_limit.clone().try_acquire_owned() else {
            drop(server);
            continue;
        };
        let handler = handler.clone();
        let control_token = control_token.clone();
        tokio::spawn(async move {
            let _permit = permit;
            handle_connection(server, control_token, handler).await
        });
    }
}

#[cfg(windows)]
fn validate_named_pipe_client(
    server: &tokio::net::windows::named_pipe::NamedPipeServer,
) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;
    let mut pid = 0_u32;
    let ok = unsafe { GetNamedPipeClientProcessId(server.as_raw_handle() as _, &mut pid) };
    if ok == 0 || pid == 0 {
        return Err("failed to identify runtime pipe client".into());
    }
    if crate::persistence::windows_process_sid(Some(pid))?
        != crate::persistence::windows_process_sid(None)?
    {
        return Err("runtime pipe client belongs to another OS user".into());
    }
    Ok(())
}

#[cfg(unix)]
async fn connect(control_token: &str) -> Result<tokio::net::UnixStream, String> {
    let socket_path = control_socket_path(control_token);
    validate_control_socket(&socket_path)?;
    tokio::net::UnixStream::connect(socket_path)
        .await
        .map_err(|error| format!("failed to connect to runtime service: {error}"))
}

#[cfg(windows)]
async fn connect(
    control_token: &str,
) -> Result<tokio::net::windows::named_pipe::NamedPipeClient, String> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let pipe_name = control_pipe_name(control_token);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        match ClientOptions::new().open(&pipe_name) {
            Ok(client) => {
                validate_named_pipe_server(&client)?;
                return Ok(client);
            }
            Err(error) if std::time::Instant::now() < deadline => {
                let _ = error;
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            Err(error) => {
                return Err(format!("failed to connect to runtime service: {error}"));
            }
        }
    }
}

#[cfg(windows)]
fn validate_named_pipe_server(
    client: &tokio::net::windows::named_pipe::NamedPipeClient,
) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
    let mut pid = 0_u32;
    let ok = unsafe { GetNamedPipeServerProcessId(client.as_raw_handle() as _, &mut pid) };
    if ok == 0 || pid == 0 {
        return Err("failed to identify runtime pipe server".into());
    }
    if pid != expected_service_pid()? {
        return Err("runtime pipe server PID does not match the private service identity".into());
    }
    if crate::persistence::windows_process_sid(Some(pid))?
        != crate::persistence::windows_process_sid(None)?
    {
        return Err("runtime pipe server belongs to another OS user".into());
    }
    Ok(())
}

pub async fn send_request(request: &RuntimeRequest) -> Result<RuntimeResponse, String> {
    if request.protocol_version != RUNTIME_PROTOCOL_VERSION {
        return Err("runtime request protocol version is invalid".into());
    }
    let payload = serde_json::to_vec(request)
        .map_err(|error| format!("failed to serialize runtime request: {error}"))?;
    let mut stream = tokio::time::timeout(RUNTIME_CONNECT_TIMEOUT, connect(&request.token))
        .await
        .map_err(|_| "timed out connecting to runtime service".to_string())??;
    authenticate_runtime_server(&mut stream, &request.token).await?;
    tokio::time::timeout(RUNTIME_IO_TIMEOUT, write_frame(&mut stream, &payload))
        .await
        .map_err(|_| "timed out writing to runtime service".to_string())??;
    let response = tokio::time::timeout(RUNTIME_RESPONSE_TIMEOUT, read_frame(&mut stream))
        .await
        .map_err(|_| "timed out reading from runtime service".to_string())??;
    let response: RuntimeResponse = serde_json::from_slice(&response)
        .map_err(|error| format!("failed to decode runtime response: {error}"))?;
    if response.protocol_version != RUNTIME_PROTOCOL_VERSION {
        return Err(format!(
            "runtime protocol mismatch: expected {}, received {}",
            RUNTIME_PROTOCOL_VERSION, response.protocol_version
        ));
    }
    if response.request_id != request.request_id {
        return Err("runtime response request id mismatch".into());
    }
    Ok(response)
}

pub async fn wait_until_ready(control_token: &str, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Ok(mut stream) = connect(control_token).await {
            if authenticate_runtime_server(&mut stream, control_token)
                .await
                .is_ok()
            {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

pub async fn wait_until_stopped(control_token: &str, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        match connect(control_token).await {
            Err(_) => return true,
            Ok(mut stream) => {
                if authenticate_runtime_server(&mut stream, control_token)
                    .await
                    .is_err()
                {
                    return true;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_hash_is_repeatable_and_path_specific() {
        assert_eq!(stable_path_hash("alpha"), stable_path_hash("alpha"));
        assert_ne!(stable_path_hash("alpha"), stable_path_hash("beta"));
    }

    #[test]
    fn long_system_temp_path_uses_bounded_short_socket_fallback() {
        let system_temp = std::path::Path::new("/var/folders/pd/2_nlvl1s4k121pdk4d5_2c8m0000gn/T");
        let data_dir = system_temp.join("lsm-runtime-smoke-123456");
        let endpoint = control_socket_path_for(&data_dir, system_temp, "test-control-token");

        assert!(endpoint.starts_with(SHORT_RUNTIME_SOCKET_ROOT));
        assert!(socket_path_fits(&endpoint));
        assert!(endpoint
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("control-")));
    }

    #[test]
    fn runtime_paths_are_scoped_below_the_application_data_directory() {
        assert!(runtime_state_path().ends_with("runtime-state.json"));
        assert!(control_token_path().ends_with("control-token"));
        assert!(service_log_path().ends_with("runtime-service.log"));
        assert!(runtime_lock_path().ends_with("runtime-service.lock"));
    }

    #[test]
    fn control_endpoint_is_derived_from_the_secret_token() {
        assert_ne!(endpoint_suffix("token-a"), endpoint_suffix("token-b"));
        assert_eq!(endpoint_suffix("token-a").len(), 32);
        #[cfg(windows)]
        assert_ne!(control_pipe_name("token-a"), control_pipe_name("token-b"));
        #[cfg(unix)]
        assert_ne!(
            control_socket_path("token-a"),
            control_socket_path("token-b")
        );
    }

    #[test]
    fn runtime_handshake_proof_binds_token_nonce_and_service_pid() {
        let baseline = handshake_proof("token-a", "nonce-a", 100);
        assert_eq!(baseline, handshake_proof("token-a", "nonce-a", 100));
        assert_ne!(baseline, handshake_proof("token-b", "nonce-a", 100));
        assert_ne!(baseline, handshake_proof("token-a", "nonce-b", 100));
        assert_ne!(baseline, handshake_proof("token-a", "nonce-a", 101));
        assert!(constant_time_eq(baseline.as_bytes(), baseline.as_bytes()));
        assert!(!constant_time_eq(
            baseline.as_bytes(),
            handshake_proof("token-b", "nonce-a", 100).as_bytes()
        ));
    }

    #[tokio::test]
    async fn framing_round_trip_preserves_payload() {
        let (mut writer, mut reader) = tokio::io::duplex(128);
        let payload = br#"{"command":"ping"}"#.to_vec();
        let expected = payload.clone();
        let write = tokio::spawn(async move { write_frame(&mut writer, &payload).await });
        assert_eq!(read_frame(&mut reader).await.unwrap(), expected);
        write.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn framing_rejects_oversized_requests_before_allocation() {
        let (mut writer, mut reader) = tokio::io::duplex(16);
        writer
            .write_all(&((MAX_RUNTIME_FRAME_BYTES as u32) + 1).to_le_bytes())
            .await
            .unwrap();
        let error = read_frame(&mut reader).await.unwrap_err();
        assert!(error.contains("invalid runtime frame length"));
    }
}
