use std::process::Command;
use std::net::TcpStream;

#[tauri::command]
pub async fn ssh_launch_rpc(
    host: String,
    ssh_user: String,
    ssh_key_path: Option<String>,
    ssh_password: Option<String>,
    rpc_port: u16,
    remote_rpc_path: Option<String>,
    ssh_port: Option<u16>,
) -> Result<serde_json::Value, String> {
    let ssh_port = ssh_port.unwrap_or(22);
    let rpc_binary = remote_rpc_path
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "rpc-server".to_string());

    let cmd = format!("nohup {} --host 0.0.0.0 --port {} > /dev/null 2>&1 &", rpc_binary, rpc_port);

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

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let msg = if !stderr.trim().is_empty() { stderr.to_string() } else { stdout.to_string() };
            return Err(format!("SSH 执行失败: {}", msg.trim()));
        }

        std::thread::sleep(std::time::Duration::from_secs(2));

        match TcpStream::connect_timeout(
            &format!("{}:{}", host, rpc_port).parse().unwrap(),
            std::time::Duration::from_secs(3),
        ) {
            Ok(_) => Ok(serde_json::json!({ "ok": true, "host": host, "port": rpc_port })),
            Err(e) => Err(format!("rpc-server 启动后无法连接: {}", e)),
        }
    }).await
    .map_err(|e| format!("SSH 任务失败: {}", e))?;

    result
}
