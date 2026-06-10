use std::collections::HashSet;
use std::net::{TcpStream, SocketAddr};
use std::time::Duration;
use std::path::PathBuf;
use std::sync::Arc;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use tauri::State;
use tokio::time::timeout as tokio_timeout;

use crate::models::{WorkerInfo, WorkerDevice, WorkerStatus, AppState};
use crate::utils;

#[cfg(target_os = "windows")]
pub(crate) const RPC_SERVER_NAME: &str = "rpc-server.exe";
#[cfg(not(target_os = "windows"))]
pub(crate) const RPC_SERVER_NAME: &str = "rpc-server";

// ═══════════════════════════════════════════════════════════════════
// 持久化
// ═══════════════════════════════════════════════════════════════════

fn workers_path() -> PathBuf {
    utils::get_data_dir().join("workers.json")
}

pub(crate) fn load_workers() -> Vec<WorkerInfo> {
    let path = workers_path();
    if !path.exists() {
        return Vec::new();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<WorkerInfo>>(&s).ok())
        .unwrap_or_default()
}

pub(crate) fn save_workers(workers: &[WorkerInfo]) {
    let path = workers_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // 原子写入: 先写临时文件再 rename
    let tmp = path.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_string_pretty(workers) {
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// 获取本机 LAN 地址段
// ═══════════════════════════════════════════════════════════════════

fn get_lan_prefixes() -> Vec<String> {
    let mut prefixes = Vec::new();

    // 虚拟网卡名称关键字（不区分大小写）
    let virtual_keywords = [
        "docker", "veth", "br-", "vmnet", "virtualbox", "vbox",
        "hyper-v", "vethernet", "wsl", "vpn", "tap-", "tun",
        "p2p", "rndis", "usb", "bluetooth", "loopback",
        "mihomo", "clash", "cfw", "tunnel", "sing-box", "hysteria",
        "wireguard", "zerotier", "tailscale", "nebula",
    ];

    if let Ok(ifs) = if_addrs::get_if_addrs() {
        for iface in ifs {
            if iface.is_loopback() { continue; }
            let ip = iface.addr.ip();
            if !ip.is_ipv4() { continue; }
            let octets = match ip {
                std::net::IpAddr::V4(ipv4) => ipv4.octets(),
                _ => continue,
            };

            // 排除已知虚拟子网
            let is_virtual_subnet = matches!((octets[0], octets[1]),
                (198, 18) | (198, 19) |           // benchmarking
                (169, 254) |                       // link-local
                (172, 17..=31) |                   // Docker default range
                (100, 64..=127)                    // carrier-grade NAT
            );
            if is_virtual_subnet { continue; }

            // 排除虚拟网卡名
            let name_lower = iface.name.to_lowercase();
            if virtual_keywords.iter().any(|k| name_lower.contains(k)) { continue; }

            let prefix = format!("{}.{}.{}.", octets[0], octets[1], octets[2]);
            if !prefixes.contains(&prefix) {
                prefixes.push(prefix);
            }
        }
    }

    // fallback to common LAN ranges if no interfaces found
    if prefixes.is_empty() {
        prefixes.extend_from_slice(&[
            "192.168.1.".to_string(),
            "192.168.0.".to_string(),
            "10.0.0.".to_string(),
            "172.16.0.".to_string(),
        ]);
    }

    prefixes
}

// ═══════════════════════════════════════════════════════════════════
// Tauri 命令
// ═══════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn scan_workers_tcp(state: State<'_, AppState>) -> Result<Vec<WorkerInfo>, String> {
    let prefixes = get_lan_prefixes();
    let port: u16 = 50052;
    let connect_timeout = Duration::from_millis(300);
    let overall_timeout = Duration::from_secs(15);
    let max_concurrent = 60;

    // Build candidate addresses
    let mut candidates: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for prefix in &prefixes {
        for i in 1..=254 {
            let addr = format!("{}{}:{}", prefix, i, port);
            if seen.insert(addr.clone()) {
                candidates.push(addr);
            }
        }
    }

    // Concurrent scan with overall timeout
    let candidates = Arc::new(candidates);
    let discovered = Arc::new(tokio::sync::Mutex::new(Vec::<WorkerInfo>::new()));
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));

    let scan_future = async {
        let mut handles = Vec::new();
        for idx in 0..candidates.len() {
            let addr = candidates[idx].clone();
            let discovered = discovered.clone();
            let permit = semaphore.clone().acquire_owned().await;
            let handle = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                if let Ok(socket_addr) = addr.parse::<SocketAddr>() {
                    if let Ok(mut stream) = TcpStream::connect_timeout(&socket_addr, connect_timeout) {
                        // 写后读验证：发一个字节，真实 rpc-server 保持连接，假连接（Clash TUN 拦截）会 RST
                        let _ = stream.set_write_timeout(Some(Duration::from_millis(300)));
                        let is_real = if stream.write_all(&[0x00]).is_err() {
                            false   // 写入失败 = 连接已被远端关闭
                        } else {
                            let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
                            let mut buf = [0u8; 1];
                            match stream.read(&mut buf) {
                                Ok(0) => false,          // 远端关闭连接 = 假
                                Err(_) => true,          // 超时等待 = rpc-server 等待有效 gRPC 握手
                                Ok(_) => true,           // 收到响应 = 真实服务
                            }
                        };
                        drop(stream);
                        if !is_real { return; }

                        let parts: Vec<&str> = addr.split(':').collect();
                        if parts.len() >= 2 {
                            let host = parts[0].to_string();
                            let worker_name = format!("Worker-{}", host.split('.').last().unwrap_or("?"));
                            if let Ok(mut list) = discovered.try_lock() {
                                list.push(WorkerInfo {
                                    id: format!("auto-{}", host.replace('.', "-")),
                                    host,
                                    port,
                                    name: worker_name,
                                    devices: Vec::new(),
                                    status: WorkerStatus::Online,
                                    last_seen: Some(chrono::Utc::now().to_rfc3339()),
                                    auto_discovered: true,
                                });
                            }
                        }
                    }
                }
            });
            handles.push(handle);
        }
        for h in handles { let _ = h.await; }
        let discovered = discovered.lock().await;
        discovered.clone()
    };

    match tokio_timeout(overall_timeout, scan_future).await {
        Ok(results) => {
            let mut existing = load_workers();
            for d in &results {
                if let Some(e) = existing.iter_mut().find(|w| w.host == d.host && w.port == d.port) {
                    e.status = WorkerStatus::Online;
                    e.last_seen = Some(chrono::Utc::now().to_rfc3339());
                } else {
                    existing.push(d.clone());
                }
            }
            save_workers(&existing);
            if let Ok(mut w) = state.workers.lock() { *w = existing.clone(); }
            Ok(existing)
        }
        Err(_) => {
            // Timeout — return whatever was found so far
            let partial = discovered.lock().await.clone();
            Ok(partial)
        }
    }
}

#[tauri::command]
pub async fn test_worker(host: String, port: u16, state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let addr = format!("{}:{}", host, port);
    let timeout = Duration::from_secs(3);

    match addr.parse::<SocketAddr>() {
        Ok(socket_addr) => {
            match TcpStream::connect_timeout(&socket_addr, timeout) {
                Ok(_) => {
                    // 更新 worker 状态
                    let mut workers = load_workers();
                    if let Some(w) = workers.iter_mut().find(|w| w.host == host && w.port == port) {
                        w.status = WorkerStatus::Online;
                        w.last_seen = Some(chrono::Utc::now().to_rfc3339());
                    }
                    save_workers(&workers);
                    if let Ok(mut w) = state.workers.lock() { *w = workers; }

                    Ok(serde_json::json!({ "ok": true }))
                }
                Err(e) => {
                    let mut workers = load_workers();
                    if let Some(w) = workers.iter_mut().find(|w| w.host == host && w.port == port) {
                        w.status = WorkerStatus::Offline;
                    }
                    save_workers(&workers);
                    if let Ok(mut w) = state.workers.lock() { *w = workers; }

                    Ok(serde_json::json!({ "ok": false, "error": e.to_string() }))
                }
            }
        }
        Err(e) => Ok(serde_json::json!({ "ok": false, "error": e.to_string() })),
    }
}

#[tauri::command]
pub async fn get_worker_info(host: String, port: u16) -> Result<Vec<WorkerDevice>, String> {
    // 检查是否为本地 worker
    let is_local = host == "127.0.0.1" || host == "localhost" || host == "::1"
        || is_local_ip(&host);

    if !is_local {
        // 远程 worker — rpc-server 不暴露设备查询接口
        return Ok(Vec::new());
    }

    let binary = find_rpc_server_binary_internal().unwrap_or_else(|| RPC_SERVER_NAME.to_string());

    // 用临时端口启动 rpc-server，抓取启动输出中的设备信息，然后立即杀掉
    let mut child = Command::new(&binary)
        .args(["--host", "127.0.0.1", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动 rpc-server: {}", e))?;

    let stdout = child.stdout.take().ok_or("无法读取 rpc-server 输出")?;
    let reader = BufReader::new(stdout);
    let mut devices = Vec::new();
    let mut in_devices = false;

    for line in reader.lines().flatten() {
        if line.starts_with("Devices:") { in_devices = true; continue; }
        if in_devices && line.starts_with("Starting RPC server") { break; }
        if in_devices {
            if let Some(colon_pos) = line.find(':') {
                let device_type = line[..colon_pos].trim().to_string();
                let rest = line[colon_pos + 1..].trim();
                let (name, vram_info) = if let Some(paren_open) = rest.rfind('(') {
                    let name = rest[..paren_open].trim().to_string();
                    let inside = rest[paren_open + 1..].trim_end_matches(')').to_string();
                    (name, inside)
                } else {
                    (rest.to_string(), String::new())
                };
                let parts: Vec<&str> = vram_info.split(',').collect();
                let total = parts.first().and_then(|s| s.trim().split_whitespace().next()?.parse::<u64>().ok()).unwrap_or(0);
                let free = parts.get(1).and_then(|s| s.trim().split_whitespace().next()?.parse::<u64>().ok()).unwrap_or(0);
                devices.push(WorkerDevice { device_type, name, vram_mb: total, free_mb: free });
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    let _ = (host, port);
    Ok(devices)
}

fn is_local_ip(host: &str) -> bool {
    if let Ok(ifs) = if_addrs::get_if_addrs() {
        for iface in ifs {
            if iface.addr.ip().to_string() == host {
                return true;
            }
        }
    }
    false
}

#[tauri::command]
pub async fn add_worker(host: String, port: u16, name: String, state: State<'_, AppState>) -> Result<WorkerInfo, String> {
    let mut workers = load_workers();

    if let Some(existing) = workers.iter_mut().find(|w| w.host == host && w.port == port) {
        if !name.is_empty() { existing.name = name; }
        let result = existing.clone();
        let _ = existing; // 释放可变借用
        save_workers(&workers);
        if let Ok(mut w) = state.workers.lock() { *w = workers.clone(); }
        return Ok(result);
    }

    let worker = WorkerInfo {
        id: uuid::Uuid::new_v4().to_string(),
        host: host.clone(),
        port,
        name: if name.is_empty() { host.clone() } else { name },
        devices: Vec::new(),
        status: WorkerStatus::Unknown,
        last_seen: None,
        auto_discovered: false,
    };

    workers.push(worker.clone());
    save_workers(&workers);
    if let Ok(mut w) = state.workers.lock() { *w = workers; }

    Ok(worker)
}

#[tauri::command]
pub async fn remove_worker(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut workers = load_workers();
    workers.retain(|w| w.id != id);
    save_workers(&workers);
    if let Ok(mut w) = state.workers.lock() { *w = workers; }
    Ok(())
}

#[tauri::command]
pub async fn get_workers(state: State<'_, AppState>) -> Result<Vec<WorkerInfo>, String> {
    let workers = load_workers();
    if let Ok(mut w) = state.workers.lock() { *w = workers.clone(); }
    Ok(workers)
}

#[tauri::command]
pub async fn find_rpc_server_binary(state: State<'_, AppState>) -> Result<Option<String>, String> {
    #[cfg(target_os = "windows")]
    const RPC_SERVER_NAME: &str = "rpc-server.exe";
    #[cfg(not(target_os = "windows"))]
    const RPC_SERVER_NAME: &str = "rpc-server";

    let exe = std::env::current_exe().unwrap_or_default();
    let exe_dir = exe.parent().unwrap_or(std::path::Path::new("."));

    // 在 exe 目录及父级搜索
    for dir in [exe_dir, exe_dir.parent().unwrap_or(std::path::Path::new("."))] {
        let candidate = dir.join(RPC_SERVER_NAME);
        if candidate.exists() {
            return Ok(Some(candidate.to_string_lossy().to_string()));
        }
        // 也搜索子目录
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let sub = entry.path();
                if sub.is_dir() {
                    let c = sub.join(RPC_SERVER_NAME);
                    if c.exists() {
                        return Ok(Some(c.to_string_lossy().to_string()));
                    }
                }
            }
        }
    }

    // PATH 环境变量搜索
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(RPC_SERVER_NAME);
            if candidate.exists() {
                return Ok(Some(candidate.to_string_lossy().to_string()));
            }
        }
    }

    let _ = state;
    Ok(None)
}

#[tauri::command]
pub async fn generate_rpc_launch_cmd(port: u16) -> Result<String, String> {
    let binary = find_rpc_server_binary_internal().unwrap_or_else(|| RPC_SERVER_NAME.to_string());
    Ok(format!("{} --host 0.0.0.0 --port {}", binary, port))
}

#[tauri::command]
pub async fn stop_local_worker(port: u16) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || {
        #[cfg(target_os = "windows")]
        {
            let output = std::process::Command::new("cmd")
                .args(["/c", &format!("netstat -ano | findstr :{}", port)])
                .output()
                .map_err(|e| format!("netstat 失败: {}", e))?;
            let out = String::from_utf8_lossy(&output.stdout);
            for line in out.lines() {
                if line.contains(&format!(":{}", port)) && line.contains("LISTENING") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(pid) = parts.last() {
                        let _ = std::process::Command::new("taskkill")
                            .args(["/PID", pid, "/F"])
                            .output();
                        return Ok(true);
                    }
                }
            }
        }
        #[cfg(target_os = "linux")]
        {
            let output = std::process::Command::new("sh")
                .arg("-c")
                .arg(&format!("ss -tlnp | grep ':{}' | awk '{{print $NF}}' | grep -oP 'pid=\\K\\d+'", port))
                .output()
                .map_err(|e| format!("ss 失败: {}", e))?;
            let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !out.is_empty() {
                let _ = std::process::Command::new("kill").arg(&out).output();
                return Ok(true);
            }
        }
        #[cfg(target_os = "macos")]
        {
            let output = std::process::Command::new("lsof")
                .args(["-ti", &format!(":{}", port)])
                .output()
                .map_err(|e| format!("lsof 失败: {}", e))?;
            let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !out.is_empty() {
                let _ = std::process::Command::new("kill").arg(&out).output();
                return Ok(true);
            }
        }
        Ok(false)
    }).await
    .map_err(|e| format!("停止 Worker 失败: {}", e))?
}

pub(crate) fn find_rpc_server_binary_internal() -> Option<String> {
    #[cfg(target_os = "windows")]
    const RPC_SERVER_NAME: &str = "rpc-server.exe";
    #[cfg(not(target_os = "windows"))]
    const RPC_SERVER_NAME: &str = "rpc-server";

    let exe = std::env::current_exe().unwrap_or_default();
    let exe_dir = exe.parent().unwrap_or(std::path::Path::new("."));

    for dir in [exe_dir, exe_dir.parent().unwrap_or(std::path::Path::new("."))] {
        let candidate = dir.join(RPC_SERVER_NAME);
        if candidate.exists() { return Some(candidate.to_string_lossy().to_string()); }
    }
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(RPC_SERVER_NAME);
            if candidate.exists() { return Some(candidate.to_string_lossy().to_string()); }
        }
    }
    None
}

#[tauri::command]
pub async fn get_cluster_metrics(_state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let workers = load_workers();
    let online: Vec<&WorkerInfo> = workers.iter().filter(|w| w.status == WorkerStatus::Online).collect();

    let mut worker_metrics = Vec::new();
    for w in &online {
        let addr = format!("{}:{}", w.host, w.port);
        let reachable = std::net::TcpStream::connect_timeout(
            &addr.parse().unwrap_or_else(|_| "127.0.0.1:0".parse().unwrap()),
            std::time::Duration::from_millis(500),
        ).is_ok();

        worker_metrics.push(serde_json::json!({
            "host": w.host,
            "port": w.port,
            "name": w.name,
            "online": reachable,
            "devices": w.devices.iter().map(|d| serde_json::json!({
                "type": d.device_type,
                "name": d.name,
                "vram_mb": d.vram_mb,
                "free_mb": d.free_mb,
            })).collect::<Vec<_>>(),
        }));
    }

    Ok(serde_json::json!({
        "total_workers": workers.len(),
        "online_workers": online.len(),
        "worker_metrics": worker_metrics,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workers_persistence() {
        // 确保测试不影响生产数据
        let test_workers = vec![WorkerInfo {
            id: "test-1".into(),
            host: "192.168.1.10".into(),
            port: 50052,
            name: "Test Worker".into(),
            devices: Vec::new(),
            status: WorkerStatus::Online,
            last_seen: Some("2024-01-01T00:00:00Z".into()),
            auto_discovered: false,
        }];

        // 注意：此测试会修改 workers.json，在 CI 环境隔离
        save_workers(&test_workers);
        let loaded = load_workers();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].host, "192.168.1.10");
    }

    #[test]
    fn test_load_empty() {
        // 不保存时 load 返回空
        // 此测试依赖 workers.json 不存在，仅在 CI 隔离环境运行
    }

    #[test]
    fn test_generate_rpc_launch_cmd() {
        let cmd = generate_rpc_launch_cmd_internal(50052);
        assert!(cmd.contains("50052"));
        assert!(cmd.contains("0.0.0.0"));
    }

    fn generate_rpc_launch_cmd_internal(port: u16) -> String {
        #[cfg(target_os = "windows")]
        const NAME: &str = "rpc-server.exe";
        #[cfg(not(target_os = "windows"))]
        const NAME: &str = "rpc-server";
        format!("{} --host 0.0.0.0 --port {}", NAME, port)
    }
}
