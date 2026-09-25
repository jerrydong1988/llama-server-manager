//! Authenticate the server before sending the runtime token or launch configuration.
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use tokio::net::windows::named_pipe::NamedPipeClient;
use windows_sys::Win32::{
    Security::{EqualSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER},
    System::{
        Pipes::GetNamedPipeServerProcessId,
        Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
};

fn token_user(process: windows_sys::Win32::Foundation::HANDLE) -> Result<Vec<usize>, String> {
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            return Err("cannot query runtime peer token".into());
        }
        let token = OwnedHandle::from_raw_handle(token);
        let mut size = 0;
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            std::ptr::null_mut(),
            0,
            &mut size,
        );
        if size == 0 || size > 65536 {
            return Err("invalid runtime peer identity size".into());
        }
        // TOKEN_USER contains a pointer and must be naturally aligned.
        let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        if GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        ) == 0
        {
            return Err("cannot read runtime peer identity".into());
        }
        Ok(buffer)
    }
}

pub(super) fn validate(client: &NamedPipeClient) -> Result<(), String> {
    unsafe {
        let mut pid = 0;
        if GetNamedPipeServerProcessId(client.as_raw_handle(), &mut pid) == 0 || pid == 0 {
            return Err("cannot identify runtime pipe server".into());
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err("cannot inspect runtime pipe server".into());
        }
        let process = OwnedHandle::from_raw_handle(process);
        let peer = token_user(process.as_raw_handle())?;
        let current = token_user(GetCurrentProcess())?;
        let peer = &*peer.as_ptr().cast::<TOKEN_USER>();
        let current = &*current.as_ptr().cast::<TOKEN_USER>();
        if EqualSid(peer.User.Sid, current.User.Sid) == 0 {
            return Err("runtime pipe server belongs to another user".into());
        }
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        if QueryFullProcessImageNameW(process.as_raw_handle(), 0, path.as_mut_ptr(), &mut length)
            == 0
        {
            return Err("cannot read runtime pipe server executable".into());
        }
        let peer_exe = std::path::PathBuf::from(String::from_utf16_lossy(&path[..length as usize]));
        let expected = std::env::current_exe().map_err(|error| error.to_string())?;
        if crate::path_utils::path_identity_key(&peer_exe)
            != crate::path_utils::path_identity_key(&expected)
        {
            return Err("runtime pipe server is not this application".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::{Child, Command};
    use std::time::Duration;
    use tokio::io::AsyncReadExt;
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

    const TEST_NAME: &str = "runtime_service::transport::pipe_peer::tests::pipe_peer_is_verified_before_any_application_data_is_sent";
    const PIPE_ENV: &str = "LSM_PIPE_PEER_TEST_PIPE";
    const READY_ENV: &str = "LSM_PIPE_PEER_TEST_READY";

    struct PeerFixture {
        directory: PathBuf,
        process: Option<Child>,
    }

    impl PeerFixture {
        fn start(pipe: &str) -> Self {
            let directory = std::env::temp_dir().join(format!("lsm-peer-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&directory).unwrap();
            let mut fixture = Self {
                directory,
                process: None,
            };
            // A native child at another executable path exercises the real
            // identity check without depending on PowerShell cold startup.
            let executable = fixture.directory.join("peer.exe");
            std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
            let mut command = Command::new(executable);
            use std::os::windows::process::CommandExt;
            command
                .args(["--exact", TEST_NAME, "--nocapture"])
                .env(PIPE_ENV, pipe)
                .env(READY_ENV, fixture.directory.join("ready"))
                .creation_flags(0x08000000);
            fixture.process = Some(command.spawn().unwrap());
            fixture
        }

        async fn ready(&mut self) {
            tokio::time::timeout(Duration::from_secs(30), async {
                while !self.directory.join("ready").exists() {
                    if let Some(status) = self.process.as_mut().unwrap().try_wait().unwrap() {
                        panic!("native pipe fixture exited before readiness: {status}");
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("native pipe fixture did not become ready");
        }

        async fn assert_success(&mut self) {
            let status = tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    if let Some(status) = self.process.as_mut().unwrap().try_wait().unwrap() {
                        break status;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("native pipe fixture did not exit after disconnect");
            assert!(status.success(), "native pipe fixture failed: {status}");
        }
    }

    impl Drop for PeerFixture {
        fn drop(&mut self) {
            if let Some(process) = self.process.as_mut() {
                let _ = process.kill();
                let _ = process.wait();
            }
            let _ = std::fs::remove_file(self.directory.join("peer.exe"));
            let _ = std::fs::remove_file(self.directory.join("ready"));
            let _ = std::fs::remove_dir(&self.directory);
        }
    }

    #[tokio::test]
    async fn pipe_peer_is_verified_before_any_application_data_is_sent() {
        if let Some(pipe) = std::env::var_os(PIPE_ENV) {
            let mut server = ServerOptions::new()
                .first_pipe_instance(true)
                .create(pipe)
                .unwrap();
            std::fs::write(std::env::var_os(READY_ENV).unwrap(), b"ready").unwrap();
            tokio::time::timeout(Duration::from_secs(30), async {
                server.connect().await.unwrap();
                let mut byte = [0];
                match server.read(&mut byte).await {
                    Ok(0) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
                    result => panic!("untrusted peer received application data: {result:?}"),
                }
            })
            .await
            .expect("native pipe fixture did not receive a disconnect");
            return;
        }
        let name = format!(r"\\.\pipe\lsm-peer-test-{}", uuid::Uuid::new_v4());
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .unwrap();
        let client = ClientOptions::new().open(&name).unwrap();
        validate(&client).unwrap();
        drop(client);
        drop(server);
        let name = format!(r"\\.\pipe\lsm-peer-test-{}", uuid::Uuid::new_v4());
        let mut fixture = PeerFixture::start(&name);
        fixture.ready().await;
        let client = ClientOptions::new().open(&name).unwrap();
        assert!(validate(&client)
            .unwrap_err()
            .contains("not this application"));
        drop(client);
        fixture.assert_success().await;
    }
}
