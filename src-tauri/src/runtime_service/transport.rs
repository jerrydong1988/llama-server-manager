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
        .join(format!("llama-server-manager-{data_suffix:016x}"))
        .join(format!("control-{suffix}.sock"));
    if socket_path_fits(&fallback) {
        return fallback;
    }
    // macOS per-user temp directories can be too long for sockaddr_un. The
    // child directory below is still ownership-checked and restricted to 0700.
    std::path::Path::new(SHORT_RUNTIME_SOCKET_ROOT)
        .join(format!("llama-server-manager-{data_suffix:016x}"))
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

async fn handle_connection<S, H, F>(mut stream: S, handler: H)
where
    S: AsyncRead + AsyncWrite + Unpin,
    H: Fn(RuntimeRequest) -> F,
    F: Future<Output = RuntimeResponse>,
{
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

    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|error| format!("runtime control socket failed: {error}"))?;
        let handler = handler.clone();
        tokio::spawn(async move { handle_connection(stream, handler).await });
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

    let pipe_name = control_pipe_name(&control_token);
    let mut first_instance = true;
    let mut initializer = Some(initializer);
    loop {
        let mut options = ServerOptions::new();
        options.reject_remote_clients(true);
        if first_instance {
            options.first_pipe_instance(true);
        }
        let server = options
            .create(&pipe_name)
            .map_err(|error| format!("failed to create runtime control pipe: {error}"))?;
        first_instance = false;
        if let Some(initializer) = initializer.take() {
            initializer().await?;
        }
        server
            .connect()
            .await
            .map_err(|error| format!("runtime control pipe failed: {error}"))?;
        let handler = handler.clone();
        tokio::spawn(async move { handle_connection(server, handler).await });
    }
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
            Ok(client) => return Ok(client),
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

pub async fn send_request(request: &RuntimeRequest) -> Result<RuntimeResponse, String> {
    if request.protocol_version != RUNTIME_PROTOCOL_VERSION {
        return Err("runtime request protocol version is invalid".into());
    }
    let payload = serde_json::to_vec(request)
        .map_err(|error| format!("failed to serialize runtime request: {error}"))?;
    let mut stream = tokio::time::timeout(RUNTIME_CONNECT_TIMEOUT, connect(&request.token))
        .await
        .map_err(|_| "timed out connecting to runtime service".to_string())??;
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
        if connect(control_token).await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

pub async fn wait_until_stopped(control_token: &str, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if connect(control_token).await.is_err() {
            return true;
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
