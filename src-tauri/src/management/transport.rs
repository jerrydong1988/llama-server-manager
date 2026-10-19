use super::wire::{self, Failure, Request, Response};
use std::{
    future::Future,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub fn ensure_private_dir(data: &Path) -> Result<PathBuf, String> {
    let dir = data.join("management");
    if let Ok(metadata) = std::fs::symlink_metadata(&dir) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("management directory must be a regular directory".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("management directory must not be a reparse point".into());
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != unsafe { libc::geteuid() } {
                return Err("management directory belongs to another user".into());
            }
        }
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    protect_windows_directory(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

pub fn read_token(data: &Path) -> Result<String, String> {
    let path = wire::token_path(data);
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|_| "management service is not initialized; run lsm service start".to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != 64 {
        return Err("invalid management credential file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err("management credentials must be private to this user".into());
        }
    }
    let token = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if token.len() != 64 || !token.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid management credentials".into());
    }
    Ok(token)
}

pub fn initialize_token(data: &Path) -> Result<String, String> {
    ensure_private_dir(data)?;
    if wire::token_path(data).exists() {
        return read_token(data);
    }
    use std::io::Write;
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(wire::token_path(data))
        .map_err(|e| e.to_string())?;
    file.write_all(token.as_bytes())
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(token)
}

pub fn endpoint(data: &Path, token: &str) -> PathBuf {
    let suffix = wire::digest(format!("{}:{token}", wire::data_identity(data)).as_bytes());
    #[cfg(windows)]
    {
        PathBuf::from(format!(
            r"\\.\pipe\llama-server-manager-management-{}",
            &suffix[..32]
        ))
    }
    #[cfg(unix)]
    {
        PathBuf::from("/tmp")
            .join(format!("lsm-mgmt-{}", &suffix[..32]))
            .join("control.sock")
    }
}

async fn read_frame<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Vec<u8>, String> {
    let length = stream.read_u32_le().await.map_err(|e| e.to_string())? as usize;
    if length == 0 || length > wire::MAX_FRAME {
        return Err("invalid management frame length".into());
    }
    let mut bytes = vec![0; length];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

async fn write_frame<S: AsyncWrite + Unpin>(stream: &mut S, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > wire::MAX_FRAME {
        return Err("management response exceeds frame limit".into());
    }
    stream
        .write_u32_le(bytes.len() as u32)
        .await
        .map_err(|e| e.to_string())?;
    stream.write_all(bytes).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())
}

async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    request: &Request,
) -> Result<Response, String> {
    write_frame(
        &mut stream,
        &serde_json::to_vec(request).map_err(|e| e.to_string())?,
    )
    .await?;
    let response: Response =
        serde_json::from_slice(&read_frame(&mut stream).await?).map_err(|e| e.to_string())?;
    if response.version != wire::VERSION || response.id != request.id {
        return Err("management response identity/version mismatch".into());
    }
    Ok(response)
}

pub async fn send(data: &Path, app_exe: &Path, request: &Request) -> Result<Response, String> {
    let address = endpoint(data, &request.token);
    #[cfg(windows)]
    let stream = {
        use tokio::net::windows::named_pipe::ClientOptions;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let client = loop {
            match ClientOptions::new().open(&address) {
                Ok(client) => break client,
                Err(_) if std::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await
                }
                Err(e) => return Err(format!("management host unavailable: {e}")),
            }
        };
        validate_peer(&client, app_exe)?;
        client
    };
    #[cfg(unix)]
    let stream = {
        let _ = app_exe;
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let metadata = std::fs::symlink_metadata(&address).map_err(|e| e.to_string())?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err("untrusted management endpoint".into());
        }
        tokio::net::UnixStream::connect(address)
            .await
            .map_err(|e| e.to_string())?
    };
    tokio::time::timeout(Duration::from_secs(60), exchange(stream, request))
        .await
        .map_err(|_| {
            "management request timed out; inspect task status before retrying".to_string()
        })?
}

async fn handle<S, H, F>(mut stream: S, handler: H)
where
    S: AsyncRead + AsyncWrite + Unpin,
    H: Fn(Request) -> F,
    F: Future<Output = Response>,
{
    let response = match tokio::time::timeout(Duration::from_secs(5), read_frame(&mut stream)).await
    {
        Ok(Ok(bytes)) => match serde_json::from_slice::<Request>(&bytes) {
            Ok(request) => handler(request).await,
            Err(_) => Response::result(
                String::new(),
                Err(Failure::new("INVALID_REQUEST", "invalid request envelope")),
            ),
        },
        _ => Response::result(
            String::new(),
            Err(Failure::new(
                "INVALID_FRAME",
                "invalid or incomplete management frame",
            )),
        ),
    };
    if let Ok(mut bytes) = serde_json::to_vec(&response) {
        if bytes.len() > wire::MAX_FRAME {
            bytes = serde_json::to_vec(&Response::result(
                response.id,
                Err(Failure::new(
                    "OUTPUT_TOO_LARGE",
                    "result exceeds 8 MiB; narrow the query",
                )),
            ))
            .unwrap_or_default();
        }
        let _ =
            tokio::time::timeout(Duration::from_secs(5), write_frame(&mut stream, &bytes)).await;
    }
}

pub async fn serve<H, F>(data: PathBuf, token: String, handler: H) -> Result<(), String>
where
    H: Fn(Request) -> F + Clone + Send + Sync + 'static,
    F: Future<Output = Response> + Send + 'static,
{
    let address = endpoint(&data, &token);
    let connections = std::sync::Arc::new(tokio::sync::Semaphore::new(32));
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ServerOptions;
        let mut first = true;
        loop {
            let permit = connections
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| e.to_string())?;
            let server = ServerOptions::new()
                .reject_remote_clients(true)
                .first_pipe_instance(first)
                .create(&address)
                .map_err(|e| e.to_string())?;
            first = false;
            server.connect().await.map_err(|e| e.to_string())?;
            if validate_client(&server).is_err() {
                continue;
            }
            let handler = handler.clone();
            tokio::spawn(async move {
                let _permit = permit;
                handle(server, handler).await
            });
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
        let parent = address.parent().ok_or("missing socket parent")?;
        if let Ok(metadata) = std::fs::symlink_metadata(parent) {
            if metadata.file_type().is_symlink() || metadata.uid() != unsafe { libc::geteuid() } {
                return Err("untrusted management socket directory".into());
            }
        }
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        if let Ok(metadata) = std::fs::symlink_metadata(&address) {
            if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::geteuid() } {
                return Err("untrusted existing management socket".into());
            }
            std::fs::remove_file(&address).map_err(|e| e.to_string())?;
        }
        let listener = tokio::net::UnixListener::bind(&address).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&address, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        loop {
            let permit = connections
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| e.to_string())?;
            let (stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
            if stream.peer_cred().map_err(|e| e.to_string())?.uid() != unsafe { libc::geteuid() } {
                continue;
            }
            let handler = handler.clone();
            tokio::spawn(async move {
                let _permit = permit;
                handle(stream, handler).await
            });
        }
    }
}

#[cfg(windows)]
fn protect_windows_directory(path: &Path) -> Result<(), String> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    let system = PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot is unavailable")?)
        .join("System32");
    let identity = Command::new(system.join("whoami.exe"))
        .creation_flags(0x08000000)
        .args(["/user", "/fo", "csv", "/nh"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !identity.status.success() {
        return Err("cannot determine management directory owner".into());
    }
    let text = String::from_utf8_lossy(&identity.stdout);
    let sid = text
        .trim()
        .rsplit(',')
        .next()
        .map(|value| value.trim().trim_matches('"'))
        .filter(|value| {
            value.starts_with("S-1-")
                && value
                    .bytes()
                    .all(|c| c.is_ascii_digit() || c == b'S' || c == b'-')
        })
        .ok_or("invalid management owner SID")?;
    let status = Command::new(system.join("icacls.exe"))
        .creation_flags(0x08000000)
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("*{sid}:(OI)(CI)F"))
        .arg("/Q")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("cannot protect management directory permissions".into());
    }
    Ok(())
}

#[cfg(windows)]
fn validate_client(
    server: &tokio::net::windows::named_pipe::NamedPipeServer,
) -> Result<(), String> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::{
        Pipes::GetNamedPipeClientProcessId,
        Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    unsafe {
        let mut pid = 0;
        if GetNamedPipeClientProcessId(server.as_raw_handle(), &mut pid) == 0 {
            return Err("cannot identify management client".into());
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err("cannot inspect management client account".into());
        }
        let process = OwnedHandle::from_raw_handle(process);
        validate_same_user(process.as_raw_handle())
    }
}

#[cfg(windows)]
fn validate_peer(
    client: &tokio::net::windows::named_pipe::NamedPipeClient,
    expected: &Path,
) -> Result<(), String> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::{
        Pipes::GetNamedPipeServerProcessId,
        Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    unsafe {
        let mut pid = 0;
        if GetNamedPipeServerProcessId(client.as_raw_handle(), &mut pid) == 0 {
            return Err("cannot identify management host".into());
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err("cannot inspect management host".into());
        }
        let process = OwnedHandle::from_raw_handle(process);
        validate_same_user(process.as_raw_handle())?;
        let mut name = vec![0u16; 32768];
        let mut length = name.len() as u32;
        if QueryFullProcessImageNameW(process.as_raw_handle(), 0, name.as_mut_ptr(), &mut length)
            == 0
        {
            return Err("cannot identify management executable".into());
        }
        let actual = PathBuf::from(String::from_utf16_lossy(&name[..length as usize]));
        let normalize = |p: &Path| {
            std::fs::canonicalize(p)
                .map(|v| {
                    v.to_string_lossy()
                        .trim_start_matches(r"\\?\")
                        .to_lowercase()
                })
                .map_err(|e| e.to_string())
        };
        if normalize(&actual)? != normalize(expected)? {
            return Err(
                "management host executable differs from --app; refusing to send credentials"
                    .into(),
            );
        }
    }
    Ok(())
}

#[cfg(windows)]
fn validate_same_user(peer: windows_sys::Win32::Foundation::HANDLE) -> Result<(), String> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Security::{EqualSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    fn user(process: windows_sys::Win32::Foundation::HANDLE) -> Result<Vec<usize>, String> {
        unsafe {
            let mut token = std::ptr::null_mut();
            if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
                return Err("cannot inspect management peer account".into());
            }
            let token = OwnedHandle::from_raw_handle(token);
            let mut length = 0;
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                std::ptr::null_mut(),
                0,
                &mut length,
            );
            if length == 0 || length > 65536 {
                return Err("invalid management peer account".into());
            }
            let mut bytes = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
            if GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                bytes.as_mut_ptr().cast(),
                length,
                &mut length,
            ) == 0
            {
                return Err("cannot read management peer account".into());
            }
            Ok(bytes)
        }
    }
    unsafe {
        let peer = user(peer)?;
        let current = user(GetCurrentProcess())?;
        let peer = &*peer.as_ptr().cast::<TOKEN_USER>();
        let current = &*current.as_ptr().cast::<TOKEN_USER>();
        if EqualSid(peer.User.Sid, current.User.Sid) == 0 {
            return Err("management host belongs to another account".into());
        }
    }
    Ok(())
}
