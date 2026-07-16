use std::process::Command;

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

    c.arg(format!("{}@{}", ssh_user, host))
        .arg("uname -s 2>/dev/null || ver 2>NUL");

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
        "nohup '{}' --host 0.0.0.0 --port {} >/dev/null 2>&1 & rpc_pid=$!; attempt=0; while [ $attempt -lt 25 ]; do if {}; then echo 'PORT-OK'; exit 0; fi; if ! kill -0 \"$rpc_pid\" 2>/dev/null; then break; fi; attempt=$((attempt + 1)); sleep 0.2; done; echo 'PORT-NOT-FOUND'",
        safe_binary, port, port_check
    )
}

fn build_remote_cmd(binary: &str, port: u16, remote_os: &str) -> String {
    match remote_os {
        "linux" => build_unix_remote_cmd(
            binary,
            port,
            &format!(
                "ss -tlnp 2>/dev/null | grep -q ':{}' || netstat -tlnp 2>/dev/null | grep -q ':{}'",
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
            "powershell -Command \"[Console]::OutputEncoding=[Text.Encoding]::UTF8; $tn='rpc-'+[System.Guid]::NewGuid().ToString('N').Substring(0,8); $a=New-ScheduledTaskAction -Execute 'powershell.exe' -Argument '-WindowStyle Hidden -NoLogo -NonInteractive -Command \"\"& ''{0}'' --host 0.0.0.0 --port {1}\"\"'; $t=New-ScheduledTaskTrigger -Once -At (Get-Date).AddSeconds(2); Register-ScheduledTask -TaskName $tn -Action $a -Trigger $t -Force|Out-Null; Start-ScheduledTask -TaskName $tn|Out-Null; $ok=$false; for($i=0;$i -lt 25 -and -not $ok;$i++){{$ok=[bool](netstat -an 2>$null|Select-String ':{1} '); if(-not $ok){{Start-Sleep -Milliseconds 200}}}}; if($ok){{Write-Output 'PORT-OK'}}else{{Write-Output 'PORT-NOT-FOUND'}}; Unregister-ScheduledTask -TaskName $tn -Confirm:$false -ErrorAction SilentlyContinue\"",
            binary.replace('\'', "''"), port
        ),
        _ => build_unix_remote_cmd(
            binary,
            port,
            &format!(
                "ss -tlnp 2>/dev/null | grep -q ':{}' || netstat -tlnp 2>/dev/null | grep -q ':{}'",
                port, port
            ),
        ),
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

        ssh_cmd.arg(format!("{}@{}", ssh_user, host)).arg(&cmd);

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

        // Check whether the remote port is listening.
        let combined = format!("{}{}", stdout, stderr);
        if combined.contains("PORT-NOT-FOUND") {
            return Err(
                "rpc-server 未在 5 秒内就绪，请检查远程程序路径、执行权限和运行依赖".to_string(),
            );
        }

        // Remote validation passed; verify locally once more.
        match crate::utils::connect_tcp(&host, rpc_port, std::time::Duration::from_secs(5)) {
            Ok(_) => Ok(
                serde_json::json!({ "ok": true, "host": host, "port": rpc_port, "remote_os": os }),
            ),
            Err(e) => Err(format!("rpc-server 启动后无法连接 ({}): {}", os, e)),
        }
    })
    .await
    .map_err(|e| format!("SSH 任务失败: {}", e))?;

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_remote_command_uses_linux_socket_tools() {
        let command = build_remote_cmd("/opt/llama/rpc-server", 50052, "linux");
        assert!(command.contains("ss -tlnp"));
        assert!(command.contains("PORT-OK"));
        assert!(!command.contains("/tmp/rpc-server"));
    }

    #[test]
    fn macos_remote_command_uses_lsof_instead_of_linux_netstat_flags() {
        let command = build_remote_cmd("/Applications/llama rpc/rpc-server", 50052, "macos");
        assert!(command.contains("lsof -nP -iTCP:50052 -sTCP:LISTEN"));
        assert!(!command.contains("ss -tlnp"));
        assert!(!command.contains("netstat -tlnp"));
        assert!(command.contains("PORT-OK"));
        assert!(!command.contains("/tmp/rpc-server"));
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
}
