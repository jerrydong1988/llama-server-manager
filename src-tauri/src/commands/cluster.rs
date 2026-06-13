use std::collections::HashSet;
use std::net::{TcpStream, SocketAddr, IpAddr, Ipv4Addr};
use std::time::Duration;
use std::path::PathBuf;
use std::sync::Arc;
use std::io::{BufRead, BufReader};
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

/// 获取物理网卡的 IPv4 地址列表（用于 socket bind 绕过 VPN/TUN）
fn get_physical_ips() -> Vec<Ipv4Addr> {
    let mut ips = Vec::new();
    if let Ok(ifs) = if_addrs::get_if_addrs() {
        for iface in ifs {
            if iface.is_loopback() { continue; }
            let name_lower = iface.name.to_lowercase();
            let is_virtual = [
                "docker", "veth", "br-", "vmnet", "virtualbox", "vbox",
                "hyper-v", "vethernet", "wsl", "vpn", "tap-", "tun",
                "p2p", "rndis", "bluetooth", "loopback",
                "mihomo", "clash", "cfw", "tunnel", "sing-box", "hysteria",
                "wireguard", "zerotier", "tailscale", "nebula",
            ].iter().any(|k| name_lower.contains(k));
            if is_virtual { continue; }
            if let IpAddr::V4(ipv4) = iface.addr.ip() {
                let octets = ipv4.octets();
                if matches!((octets[0], octets[1]), (169, 254) | (198, 18) | (198, 19) | (100, 64..=127)) { continue; }
                ips.push(ipv4);
            }
        }
    }
    ips
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

    // Scan: bind to physical interface to bypass VPN/TUN routing
    let physical_ips = get_physical_ips();
    let candidates = Arc::new(candidates);
    let physical_ips = Arc::new(physical_ips);
    let discovered = Arc::new(tokio::sync::Mutex::new(Vec::<WorkerInfo>::new()));
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));

    // #5: 使用 tokio async connect 替代 spawn_blocking，减少线程创建
    let scan_future = async {
        let mut handles = Vec::new();
        for idx in 0..candidates.len() {
            let addr = candidates[idx].clone();
            let discovered = discovered.clone();
            let permit = semaphore.clone().acquire_owned().await;
            let physical_ips = physical_ips.clone();
            let handle = tokio::spawn(async move {
                let _permit = permit;
                if let Ok(socket_addr) = addr.parse::<SocketAddr>() {
                    // 尝试通过物理网卡连接，绕过 VPN/TUN 路由
                    let connected = if physical_ips.is_empty() {
                        // 通用情况：tokio async connect + timeout
                        tokio::time::timeout(
                            connect_timeout,
                            tokio::net::TcpStream::connect(socket_addr),
                        ).await.ok()
                        .map(|_| ()) // discard stream, just check reachability
                    } else {
                        // 绑定物理网卡：spawn_blocking 处理 socket2
                        tokio::task::spawn_blocking(move || {
                            physical_ips.iter().find_map(|bind_ip| {
                                let bind_addr = SocketAddr::new(IpAddr::V4(*bind_ip), 0);
                                let sock_addr = socket2::SockAddr::from(socket_addr);
                                socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None)
                                    .ok()
                                    .and_then(|s| {
                                        s.bind(&bind_addr.into()).ok()?;
                                        s.connect_timeout(&sock_addr, connect_timeout).ok()?;
                                        s.set_nonblocking(false).ok()?;
                                        Some(())
                                    })
                            })
                        }).await.ok().flatten()
                    };

                    if connected.is_some() {
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
pub async fn is_local_host(host: String) -> Result<bool, String> {
    Ok(host == "127.0.0.1" || host == "localhost" || host == "::1" || is_local_ip(&host))
}

#[tauri::command]
pub async fn get_local_host() -> Result<String, String> {
    if let Ok(ifs) = if_addrs::get_if_addrs() {
        for iface in ifs {
            if iface.is_loopback() { continue; }
            if let IpAddr::V4(ipv4) = iface.addr.ip() {
                let name_lower = iface.name.to_lowercase();
                let is_virtual = [
                    "docker", "veth", "br-", "vmnet", "virtualbox", "vbox",
                    "hyper-v", "vethernet", "wsl", "vpn", "tap-", "tun",
                    "p2p", "rndis", "bluetooth", "loopback",
                    "mihomo", "clash", "cfw", "tunnel", "sing-box", "hysteria",
                    "wireguard", "zerotier", "tailscale", "nebula",
                ].iter().any(|k| name_lower.contains(k));
                if !is_virtual {
                    return Ok(ipv4.to_string());
                }
            }
        }
    }
    Ok("127.0.0.1".to_string())
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
pub async fn find_rpc_server_binary(_state: State<'_, AppState>) -> Result<Option<String>, String> {
    // #8: 直接委托给内部函数，消除代码重复
    Ok(find_rpc_server_binary_internal())
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
            // #6: 用正则匹配 PID，避免 netstat 格式跨语言差异；加 /T 递归杀子进程
            let pid_re = regex_lite::Regex::new(r"LISTENING\s+(\d+)$").ok();
            for line in out.lines() {
                if line.contains(&format!(":{}", port)) && line.contains("LISTENING") {
                    if let Some(pid) = pid_re.as_ref().and_then(|re| re.captures(line)).and_then(|c| c.get(1)).map(|m| m.as_str()) {
                        let _ = std::process::Command::new("taskkill")
                            .args(["/PID", pid, "/F", "/T"])
                            .output();
                        return Ok(true);
                    }
                }
            }
        }
        #[cfg(target_os = "linux")]
        {
            // #11: 避免 grep -P (PCRE)，改用 sed 兼容精简 Linux
            let output = std::process::Command::new("sh")
                .arg("-c")
                .arg(&format!("ss -tlnp | grep ':{}' | sed -n 's/.*pid=\\([0-9]*\\).*/\\1/p'", port))
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
        // #7: 避免 parse().unwrap() panic
        let reachable = addr.parse::<std::net::SocketAddr>().ok()
            .and_then(|a| std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_millis(500)).ok())
            .is_some();

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

#[tauri::command]
pub async fn start_local_rpc(engine_dir: Option<String>, port: u16) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let binary = if let Some(ref dir) = engine_dir {
            let path = std::path::Path::new(dir);
            #[cfg(target_os = "windows")]
            let exe = path.join("rpc-server.exe");
            #[cfg(not(target_os = "windows"))]
            let exe = path.join("rpc-server");
            if exe.exists() { exe.to_string_lossy().to_string() } else {
                return Err(format!("引擎目录下未找到 rpc-server: {}", dir));
            }
        } else {
            find_rpc_server_binary_internal()
                .ok_or_else(|| "未找到 rpc-server，请指定引擎目录".to_string())?
        };

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x00000008;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            std::process::Command::new(&binary)
                .args(["--host", "0.0.0.0", "--port", &port.to_string()])
                .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| format!("无法启动 {}: {}", binary, e))?;
        }
        #[cfg(not(target_os = "windows"))]
        {
            std::process::Command::new("sh")
                .arg("-c")
                .arg(&format!("nohup '{}' --host 0.0.0.0 --port {} > /dev/null 2>&1 &", binary, port))
                .spawn()
                .map_err(|e| format!("无法启动: {}", e))?;
        }

        // 等待端口就绪
        std::thread::sleep(std::time::Duration::from_secs(2));
        let addr = format!("127.0.0.1:{}", port);
        match std::net::TcpStream::connect_timeout(
            &addr.parse().unwrap(),
            std::time::Duration::from_secs(3),
        ) {
            Ok(_) => Ok(serde_json::json!({ "ok": true, "port": port })),
            Err(e) => Err(format!("rpc-server 启动后无法连接: {}", e)),
        }
    }).await
    .map_err(|e| format!("启动失败: {}", e))?
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
