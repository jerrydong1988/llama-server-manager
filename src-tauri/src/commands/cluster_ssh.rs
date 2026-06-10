use std::io::Read;
use std::net::TcpStream;

use crate::commands::cluster::find_rpc_server_binary_internal;

#[tauri::command]
pub async fn ssh_launch_rpc(
    host: String,
    ssh_user: String,
    ssh_key_path: Option<String>,
    ssh_password: Option<String>,
    rpc_port: u16,
) -> Result<serde_json::Value, String> {
    let rpc_binary = find_rpc_server_binary_internal()
        .unwrap_or_else(|| {
            #[cfg(target_os = "windows")]
            { "rpc-server.exe".to_string() }
            #[cfg(not(target_os = "windows"))]
            { "rpc-server".to_string() }
        });

    let cmd = format!("{} --host 0.0.0.0 --port {} &", rpc_binary, rpc_port);

    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let tcp = TcpStream::connect(format!("{}:22", host))
            .map_err(|e| format!("SSH 连接失败 ({}:22): {}", host, e))?;

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
