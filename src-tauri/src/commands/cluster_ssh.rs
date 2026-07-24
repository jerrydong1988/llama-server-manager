use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

static SSH_TUNNELS: LazyLock<Mutex<HashMap<u16, Child>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn append_ssh_destination(command: &mut Command, ssh_user: &str, host: &str) {
    // OpenSSH keeps parsing option-looking arguments until `--`, even when each
    // argument is passed without a shell. Keep the user-controlled destination
    // behind the option terminator so it can never become an SSH option.
    command.arg("--").arg(format!("{}@{}", ssh_user, host));
}

fn detect_remote_os(
    host: &str,
    ssh_user: &str,
    ssh_key_path: Option<&str>,
    ssh_port: u16,
) -> Option<String> {
    let mut c = Command::new("ssh");
    // #1: Use accept-new instead of no so the first connection is trusted and later ones are verified.
    c.arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("ConnectTimeout=5")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-p")
        .arg(ssh_port.to_string());

    if let Some(k) = ssh_key_path {
        c.arg("-i").arg(k);
    }

    append_ssh_destination(&mut c, ssh_user, host);
    c.arg("uname -s 2>/dev/null || ver 2>NUL");

    c.output().ok().and_then(|o| {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if s.contains("Linux") {
            Some("linux".into())
        } else if s.contains("Darwin") {
            Some("macos".into())
        } else if s.contains("Windows") || s.contains("Microsoft") {
            Some("windows".into())
        } else {
            None
        }
    })
}

fn escape_shell_single_quote(s: &str) -> String {
    s.replace('\'', "'\\''")
}

fn build_unix_remote_cmd(binary: &str, port: u16, port_check: &str) -> String {
    let safe_binary = escape_shell_single_quote(binary);
    format!(
        "nohup '{}' --host 127.0.0.1 --port {} >/dev/null 2>&1 & rpc_pid=$!; attempt=0; while [ $attempt -lt 25 ]; do if {}; then echo 'PORT-OK'; exit 0; fi; if ! kill -0 \"$rpc_pid\" 2>/dev/null; then break; fi; attempt=$((attempt + 1)); sleep 0.2; done; echo 'PORT-NOT-FOUND'",
        safe_binary, port, port_check
    )
}

fn build_windows_remote_script(binary: &str, port: u16) -> String {
    let binary = binary.replace('\'', "''");
    format!(
        "[Console]::OutputEncoding=[Text.Encoding]::UTF8; \
         $tn='rpc-'+[System.Guid]::NewGuid().ToString('N').Substring(0,8); \
         $a=New-ScheduledTaskAction -Execute '{binary}' -Argument '--host 127.0.0.1 --port {port}'; \
         $t=New-ScheduledTaskTrigger -Once -At (Get-Date).AddSeconds(2); \
         Register-ScheduledTask -TaskName $tn -Action $a -Trigger $t -Force|Out-Null; \
         Start-ScheduledTask -TaskName $tn|Out-Null; \
         $ok=$false; \
         for($i=0;$i -lt 25 -and -not $ok;$i++){{\
           $ok=[bool](Get-NetTCPConnection -State Listen -LocalAddress 127.0.0.1 -LocalPort {port} -ErrorAction SilentlyContinue); \
           if(-not $ok){{Start-Sleep -Milliseconds 200}}\
         }}; \
         if($ok){{Write-Output 'PORT-OK'}}else{{Write-Output 'PORT-NOT-FOUND'}}; \
         Unregister-ScheduledTask -TaskName $tn -Confirm:$false -ErrorAction SilentlyContinue"
    )
}

fn encode_powershell_command(script: &str) -> String {
    let mut utf16le = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        utf16le.extend_from_slice(&unit.to_le_bytes());
    }
    BASE64_STANDARD.encode(utf16le)
}

fn build_remote_cmd(binary: &str, port: u16, remote_os: &str) -> String {
    match remote_os {
        "linux" => build_unix_remote_cmd(
            binary,
            port,
            &format!(
                "ss -tlnH 2>/dev/null | awk -v port='{}' '$4 ~ (\":\" port \"$\") {{ found=1 }} END {{ exit !found }}' || netstat -tln 2>/dev/null | awk -v port='{}' '$4 ~ (\":\" port \"$\") {{ found=1 }} END {{ exit !found }}'",
                port, port
            ),
        ),
        "macos" => build_unix_remote_cmd(
            binary,
            port,
            &format!(
                "lsof -nP -iTCP:{} -sTCP:LISTEN >/dev/null 2>&1",
                port
            ),
        ),
        "windows" => format!(
            "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {}",
            encode_powershell_command(&build_windows_remote_script(binary, port))
        ),
        _ => build_unix_remote_cmd(
            binary,
            port,
            &format!(
                "ss -tlnH 2>/dev/null | awk -v port='{}' '$4 ~ (\":\" port \"$\") {{ found=1 }} END {{ exit !found }}' || netstat -tln 2>/dev/null | awk -v port='{}' '$4 ~ (\":\" port \"$\") {{ found=1 }} END {{ exit !found }}'",
                port, port
            ),
        ),
    }
}

fn validate_ssh_identity(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty()
        || value.len() > 255
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        || value.contains('@')
    {
        return Err(format!("Invalid SSH {field}"));
    }
    Ok(())
}

fn available_loopback_port() -> Result<u16, String> {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|error| format!("无法分配 SSH 本地转发端口: {error}"))
}

fn wait_for_tunnel(child: &mut Child, port: u16, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    while Instant::now() < deadline {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("无法检查 SSH 隧道状态: {error}"))?
        {
            return Err(format!("SSH 隧道提前退出: {status}"));
        }
        if std::net::TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("SSH 本地转发未在超时前就绪".to_string())
}

fn stop_tunnel_child(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub fn stop_all_ssh_tunnels() {
    let tunnels = {
        let mut tunnels = SSH_TUNNELS.lock().unwrap();
        std::mem::take(&mut *tunnels)
    };
    for (_, child) in tunnels {
        stop_tunnel_child(child);
    }
}

pub async fn stop_ssh_tunnel(port: u16) -> Result<bool, String> {
    let child = SSH_TUNNELS.lock().unwrap().remove(&port);
    if let Some(child) = child {
        stop_tunnel_child(child);
        Ok(true)
    } else {
        Ok(false)
    }
}

pub async fn ssh_launch_rpc(
    host: String,
    ssh_user: String,
    ssh_key_path: Option<String>,
    rpc_port: u16,
    remote_rpc_path: Option<String>,
    ssh_port: Option<u16>,
    remote_os: Option<String>,
) -> Result<serde_json::Value, String> {
    validate_ssh_identity(&host, "host")?;
    validate_ssh_identity(&ssh_user, "user")?;
    let ssh_port = ssh_port.unwrap_or(22);
    let rpc_binary = remote_rpc_path
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "rpc-server".to_string());

    // Auto-detect the remote OS, or use the user-specified value.
    let os = remote_os
        .filter(|o| !o.is_empty() && o != "auto")
        .or_else(|| detect_remote_os(&host, &ssh_user, ssh_key_path.as_deref(), ssh_port))
        .unwrap_or_else(|| "linux".to_string());

    let cmd = build_remote_cmd(&rpc_binary, rpc_port, &os);

    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let mut ssh_cmd = Command::new("ssh");

        ssh_cmd
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new")
            .arg("-o")
            .arg("ConnectTimeout=5")
            .arg("-p")
            .arg(ssh_port.to_string());

        let key_path = ssh_key_path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .ok_or_else(|| "SSH key file is required.\n必须提供 SSH 密钥文件。".to_string())?;
        ssh_cmd.arg("-i").arg(key_path);
        ssh_cmd.arg("-o").arg("BatchMode=yes");

        append_ssh_destination(&mut ssh_cmd, &ssh_user, &host);
        ssh_cmd.arg(&cmd);

        let output = ssh_cmd
            .output()
            .map_err(|e| format!("无法启动 SSH: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !output.status.success() {
            let msg = if !stderr.trim().is_empty() {
                stderr.to_string()
            } else {
                stdout.to_string()
            };
            return Err(format!("SSH 执行失败: {}", msg.trim()));
        }

        // Check whether the remote loopback port is listening.
        let combined = format!("{}{}", stdout, stderr);
        if !combined.contains("PORT-OK") || combined.contains("PORT-NOT-FOUND") {
            return Err(
                "rpc-server 未在 5 秒内就绪，请检查远程程序路径、执行权限和运行依赖".to_string(),
            );
        }

        let local_port = available_loopback_port()?;
        let mut tunnel = Command::new("ssh");
        tunnel
            .arg("-N")
            .arg("-T")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new")
            .arg("-o")
            .arg("ConnectTimeout=5")
            .arg("-o")
            .arg("ExitOnForwardFailure=yes")
            .arg("-o")
            .arg("ServerAliveInterval=15")
            .arg("-o")
            .arg("ServerAliveCountMax=3")
            .arg("-p")
            .arg(ssh_port.to_string())
            .arg("-i")
            .arg(key_path)
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-L")
            .arg(format!("127.0.0.1:{local_port}:127.0.0.1:{rpc_port}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        append_ssh_destination(&mut tunnel, &ssh_user, &host);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            tunnel.creation_flags(0x08000000);
        }
        let mut tunnel = tunnel
            .spawn()
            .map_err(|error| format!("无法启动 SSH 本地转发: {error}"))?;
        if let Err(error) = wait_for_tunnel(&mut tunnel, local_port, Duration::from_secs(5)) {
            stop_tunnel_child(tunnel);
            return Err(error);
        }
        SSH_TUNNELS.lock().unwrap().insert(local_port, tunnel);
        Ok(serde_json::json!({
            "ok": true,
            "host": "127.0.0.1",
            "port": local_port,
            "remote_port": rpc_port,
            "remote_os": os
        }))
    })
    .await
    .map_err(|e| format!("SSH 任务失败: {}", e))?;

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_args_for_destination(ssh_user: &str, host: &str) -> Vec<String> {
        let mut command = Command::new("ssh");
        command.arg("-p").arg("22");
        append_ssh_destination(&mut command, ssh_user, host);
        command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn ssh_destination_is_separated_from_options() {
        assert_eq!(
            ssh_args_for_destination("alice", "example.com"),
            ["-p", "22", "--", "alice@example.com"]
        );
    }

    #[test]
    fn option_looking_ssh_user_stays_in_the_destination_operand() {
        assert_eq!(
            ssh_args_for_destination("-oProxyCommand=echo-option-smuggled", "target"),
            [
                "-p",
                "22",
                "--",
                "-oProxyCommand=echo-option-smuggled@target"
            ]
        );
    }

    #[test]
    fn linux_remote_command_uses_linux_socket_tools() {
        let command = build_remote_cmd("/opt/llama/rpc-server", 50052, "linux");
        assert!(command.contains("ss -tlnH"));
        assert!(command.contains("port \"$\""));
        assert!(!command.contains("grep -q ':50052'"));
        assert!(command.contains("PORT-OK"));
        assert!(command.contains("--host 127.0.0.1"));
        assert!(!command.contains("--host 0.0.0.0"));
        assert!(!command.contains("/tmp/rpc-server"));
    }

    #[test]
    fn macos_remote_command_uses_lsof_instead_of_linux_netstat_flags() {
        let command = build_remote_cmd("/Applications/llama rpc/rpc-server", 50052, "macos");
        assert!(command.contains("lsof -nP -iTCP:50052 -sTCP:LISTEN"));
        assert!(!command.contains("ss -tlnH"));
        assert!(!command.contains("netstat -tlnp"));
        assert!(command.contains("PORT-OK"));
        assert!(command.contains("--host 127.0.0.1"));
        assert!(!command.contains("/tmp/rpc-server"));
    }

    #[test]
    fn windows_remote_command_uses_encoded_data_only() {
        let command = build_remote_cmd(
            r"C:\rpc'; Start-Process calc; #\rpc-server.exe",
            50052,
            "windows",
        );
        let encoded = command
            .strip_prefix("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ")
            .unwrap();
        assert!(encoded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')));
        assert!(!command.contains("Start-Process"));
        assert!(!command.contains("-Command \""));
    }
}

// IPC compatibility boundary: legacy command internals keep their existing error flow,
// while every registered command serializes a stable AppError object.
#[allow(dead_code, unused_imports, unused_mut)] // Tauri references adapters through generated macros.
pub mod ipc {
    use super::*;

    #[tauri::command]
    pub async fn ssh_launch_rpc(
        host: String,
        ssh_user: String,
        ssh_key_path: Option<String>,
        rpc_port: u16,
        remote_rpc_path: Option<String>,
        ssh_port: Option<u16>,
        remote_os: Option<String>,
    ) -> crate::error::AppResult<serde_json::Value> {
        super::ssh_launch_rpc(
            host,
            ssh_user,
            ssh_key_path,
            rpc_port,
            remote_rpc_path,
            ssh_port,
            remote_os,
        )
        .await
        .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn stop_ssh_tunnel(port: u16) -> crate::error::AppResult<bool> {
        super::stop_ssh_tunnel(port)
            .await
            .map_err(crate::error::AppError::from)
    }
}
