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
    #[tokio::test]
    async fn pipe_peer_is_verified_before_any_application_data_is_sent() {
        use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
        let name = format!(r"\\.\pipe\lsm-peer-test-{}", uuid::Uuid::new_v4());
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .unwrap();
        let client = ClientOptions::new().open(&name).unwrap();
        validate(&client).unwrap();
        drop(client);
        drop(server);
        let bare = format!("lsm-peer-test-{}", uuid::Uuid::new_v4());
        let script = format!("$p = [IO.Pipes.NamedPipeServerStream]::new('{bare}'); $p.WaitForConnection(); Start-Sleep -Seconds 15");
        let mut command = std::process::Command::new("powershell.exe");
        use std::os::windows::process::CommandExt;
        command
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(0x08000000);
        let mut process = command.spawn().unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Ok(client) = ClientOptions::new().open(format!(r"\\.\pipe\{bare}")) {
                    break validate(&client);
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        let _ = process.kill();
        let _ = process.wait();
        assert!(result
            .unwrap()
            .unwrap_err()
            .contains("not this application"));
    }
}
