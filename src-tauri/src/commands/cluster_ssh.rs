use std::io::Read;
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
    // 远程机器上使用 rpc-server（假设在 PATH），或者用户指定的路径
    let rpc_binary = remote_rpc_path
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "rpc-server".to_string());

    let cmd = format!("nohup {} --host 0.0.0.0 --port {} > /dev/null 2>&1 &", rpc_binary, rpc_port);

    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let tcp = TcpStream::connect(format!("{}:{}", host, ssh_port))
            .map_err(|e| format!("SSH 连接失败 ({}:{}): {}", host, ssh_port, e))?;

        let mut sess = ssh2::Session::new()
            .map_err(|e| format!("SSH session: {}", e))?;
        sess.set_tcp_stream(tcp);
        sess.handshake()
            .map_err(|e| format!("SSH 握手失败: {}", e))?;

        // 认证: 密钥优先，密码降级
        if let Some(ref key_path) = ssh_key_path {
            sess.userauth_pubkey_file(&ssh_user, None, std::path::Path::new(key_path), None)
                .map_err(|e| format!("SSH 密钥认证失败: {}", e))?;
        } else if let Some(ref pwd) = ssh_password {
            sess.userauth_password(&ssh_user, pwd)
                .map_err(|e| format!("SSH 密码认证失败: {}", e))?;
        } else {
            return Err("未提供 SSH 密钥或密码".into());
        }

        // 执行启动命令（后台运行，不等待输出）
        let mut channel = sess.channel_session()
            .map_err(|e| format!("SSH channel: {}", e))?;
        channel.exec(&cmd)
            .map_err(|e| format!("SSH 执行失败: {}", e))?;

        // 读取 stderr 检查错误
        let mut stderr = String::new();
        channel.stderr().read_to_string(&mut stderr).ok();

        // 等待 channel 关闭
        channel.wait_close().ok();

        let exit_status = channel.exit_status().unwrap_or(-1);

        if exit_status != 0 && !stderr.is_empty() {
            return Err(format!("rpc-server 启动失败 (exit={}): {}", exit_status, stderr));
        }

        // 等待一秒然后测试连接
        std::thread::sleep(std::time::Duration::from_secs(1));

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
