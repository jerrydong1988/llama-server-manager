use std::process::Command;
use std::net::TcpStream;

fn detect_remote_os(host: &str, ssh_user: &str, ssh_key_path: Option<&str>, ssh_port: u16) -> Option<String> {
    let mut c = Command::new("ssh");
    c.arg("-o").arg("StrictHostKeyChecking=no")
     .arg("-o").arg("ConnectTimeout=5")
     .arg("-o").arg("BatchMode=yes")
     .arg("-p").arg(ssh_port.to_string());

    if let Some(k) = ssh_key_path { c.arg("-i").arg(k); }

    c.arg(format!("{}@{}", ssh_user, host))
     .arg("uname -s 2>/dev/null || ver 2>NUL");

    c.output().ok().and_then(|o| {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if s.contains("Linux") { Some("linux".into()) }
        else if s.contains("Darwin") { Some("macos".into()) }
        else if s.contains("Windows") || s.contains("Microsoft") { Some("windows".into()) }
        else { None }
    })
}

fn build_remote_cmd(binary: &str, port: u16, remote_os: &str) -> String {
    match remote_os {
        "linux" | "macos" => format!(
            "nohup '{}' --host 0.0.0.0 --port {} > /tmp/rpc-server-{}.log 2>&1 & sleep 2; cat /tmp/rpc-server-{}.log 2>/dev/null; echo '---PORT-CHECK---'; ss -tlnp 2>/dev/null | grep :{} || netstat -tlnp 2>/dev/null | grep :{} || echo 'PORT-NOT-FOUND'",
            binary, port, port, port, port, port
        ),
        "windows" => format!(
            "powershell -Command \"[Console]::OutputEncoding=[Text.Encoding]::UTF8; $tn='rpc-'+[System.Guid]::NewGuid().ToString('N').Substring(0,8); $a=New-ScheduledTaskAction -Execute '{}' -Argument '--host 0.0.0.0 --port {}'; $t=New-ScheduledTaskTrigger -Once -At (Get-Date).AddSeconds(2); Register-ScheduledTask -TaskName $tn -Action $a -Trigger $t -Force|Out-Null; Start-ScheduledTask -TaskName $tn|Out-Null; Start-Sleep 5; $f=netstat -an 2>$null|Select-String ':{} '; if($f){{Write-Output 'PORT-OK'}}else{{Write-Output 'PORT-NOT-FOUND'}}; Unregister-ScheduledTask -TaskName $tn -Confirm:$false -ErrorAction SilentlyContinue\"",
            binary.replace('\'', "''"), port, port
        ),
        _ => format!(
            "nohup '{}' --host 0.0.0.0 --port {} > /tmp/rpc-server-{}.log 2>&1 & sleep 2; cat /tmp/rpc-server-{}.log 2>/dev/null; echo '---PORT-CHECK---'; ss -tlnp 2>/dev/null | grep :{} || netstat -tlnp 2>/dev/null | grep :{} || echo 'PORT-NOT-FOUND'",
            binary, port, port, port, port, port
        ),
    }
}

#[tauri::command]
pub async fn ssh_launch_rpc(
    host: String,
    ssh_user: String,
    ssh_key_path: Option<String>,
    ssh_password: Option<String>,
    rpc_port: u16,
    remote_rpc_path: Option<String>,
    ssh_port: Option<u16>,
    remote_os: Option<String>,
) -> Result<serde_json::Value, String> {
    let ssh_port = ssh_port.unwrap_or(22);
    let rpc_binary = remote_rpc_path
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "rpc-server".to_string());

    // 自动检测远程 OS，或使用用户指定的
    let os = remote_os
        .filter(|o| !o.is_empty() && o != "auto")
        .or_else(|| detect_remote_os(&host, &ssh_user, ssh_key_path.as_deref(), ssh_port))
        .unwrap_or_else(|| "linux".to_string());

    let cmd = build_remote_cmd(&rpc_binary, rpc_port, &os);

    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let mut ssh_cmd = Command::new("ssh");

        ssh_cmd
            .arg("-o").arg("StrictHostKeyChecking=no")
            .arg("-o").arg("ConnectTimeout=5")
            .arg("-p").arg(ssh_port.to_string());

        if let Some(ref key_path) = ssh_key_path {
            ssh_cmd.arg("-i").arg(key_path);
            ssh_cmd.arg("-o").arg("BatchMode=yes");
        } else if let Some(ref _pwd) = ssh_password {
            return Err("GUI SSH 暂不支持密码认证，请使用密钥文件".into());
        } else {
            return Err("未提供 SSH 密钥或密码".into());
        }

        ssh_cmd
            .arg(format!("{}@{}", ssh_user, host))
            .arg(&cmd);

        let output = ssh_cmd.output()
            .map_err(|e| format!("无法启动 SSH: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !output.status.success() {
            let msg = if !stderr.trim().is_empty() { stderr.to_string() } else { stdout.to_string() };
            return Err(format!("SSH 执行失败: {}", msg.trim()));
        }

        // 检查远程端口是否在监听
        let combined = format!("{}{}", stdout, stderr);
        if combined.contains("PORT-NOT-FOUND") {
            // 提取启动日志（PORT-CHECK 之前的部分）
            let log_part = stdout.split("---PORT-CHECK---").next().unwrap_or("").trim();
            let log_msg = if log_part.is_empty() { "无输出".to_string() } else { log_part.to_string() };
            return Err(format!("rpc-server 启动失败\n日志: {}", log_msg));
        }

        // 远程验证通过 — 本地再做一次确认
        match TcpStream::connect_timeout(
            &format!("{}:{}", host, rpc_port).parse().unwrap(),
            std::time::Duration::from_secs(5),
        ) {
            Ok(_) => Ok(serde_json::json!({ "ok": true, "host": host, "port": rpc_port, "remote_os": os })),
            Err(e) => Err(format!("rpc-server 启动后无法连接 ({}): {}", os, e)),
        }
    }).await
    .map_err(|e| format!("SSH 任务失败: {}", e))?;

    result
}
