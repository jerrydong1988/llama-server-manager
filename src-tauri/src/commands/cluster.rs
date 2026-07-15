use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use sysinfo::{Pid, ProcessesToUpdate, System};
use tauri::State;
use tokio::time::timeout as tokio_timeout;

use crate::models::{AppState, WorkerDevice, WorkerInfo, WorkerStatus};
use crate::utils;

#[cfg(target_os = "windows")]
pub(crate) const RPC_SERVER_NAME: &str = "rpc-server.exe";
#[cfg(not(target_os = "windows"))]
pub(crate) const RPC_SERVER_NAME: &str = "rpc-server";

const VIRTUAL_KEYWORDS: &[&str] = &[
    "docker",
    "veth",
    "br-",
    "vmnet",
    "virtualbox",
    "vbox",
    "hyper-v",
    "vethernet",
    "wsl",
    "vpn",
    "tap-",
    "tun",
    "p2p",
    "rndis",
    "usb",
    "bluetooth",
    "loopback",
    "mihomo",
    "clash",
    "cfw",
    "tunnel",
    "sing-box",
    "hysteria",
    "wireguard",
    "zerotier",
    "tailscale",
    "nebula",
];

// -------------------------------------------------------------------
// Persistence.
// -------------------------------------------------------------------

fn workers_path() -> PathBuf {
    let config_dir = utils::get_data_dir().join("configs");
    // Migrate old workers.json from data_dir root to configs/
    let old_path = utils::get_data_dir().join("workers.json");
    let new_path = config_dir.join("workers.json");
    if old_path.exists() && !new_path.exists() {
        let _ = std::fs::rename(&old_path, &new_path);
    }
    new_path
}

pub(crate) fn load_workers() -> Vec<WorkerInfo> {
    load_workers_from(&workers_path())
}

fn load_workers_from(path: &std::path::Path) -> Vec<WorkerInfo> {
    if !path.exists() {
        return Vec::new();
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<WorkerInfo>>(&s).ok())
        .unwrap_or_default()
}

pub(crate) fn save_workers(workers: &[WorkerInfo]) {
    save_workers_to(&workers_path(), workers)
}

fn save_workers_to(path: &std::path::Path, workers: &[WorkerInfo]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Atomic write: write a temporary file first, then rename it.
    let tmp = path.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_string_pretty(workers) {
        if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

// -------------------------------------------------------------------
// Get local LAN prefixes.
// -------------------------------------------------------------------

fn get_lan_prefixes() -> Vec<String> {
    let mut prefixes = Vec::new();

    // Virtual network adapter keywords, case-insensitive.
    if let Ok(ifs) = if_addrs::get_if_addrs() {
        for iface in ifs {
            if iface.is_loopback() {
                continue;
            }
            let ip = iface.addr.ip();
            if !ip.is_ipv4() {
                continue;
            }
            let octets = match ip {
                std::net::IpAddr::V4(ipv4) => ipv4.octets(),
                _ => continue,
            };

            // Exclude known virtual subnets.
            let is_virtual_subnet = matches!(
                (octets[0], octets[1]),
                (198, 18) | (198, 19) |           // benchmarking
                (169, 254) |                       // link-local
                (172, 17..=31) |                   // Docker default range
                (100, 64..=127) // carrier-grade NAT
            );
            if is_virtual_subnet {
                continue;
            }

            // Exclude virtual network adapters by name.
            let name_lower = iface.name.to_lowercase();
            if VIRTUAL_KEYWORDS.iter().any(|k| name_lower.contains(k)) {
                continue;
            }

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

/// Gets IPv4 addresses from physical adapters for socket binding around VPN/TUN routes.
fn get_physical_ips() -> Vec<Ipv4Addr> {
    let mut ips = Vec::new();
    if let Ok(ifs) = if_addrs::get_if_addrs() {
        for iface in ifs {
            if iface.is_loopback() {
                continue;
            }
            let name_lower = iface.name.to_lowercase();
            let is_virtual = VIRTUAL_KEYWORDS.iter().any(|k| name_lower.contains(k));
            if is_virtual {
                continue;
            }
            if let IpAddr::V4(ipv4) = iface.addr.ip() {
                let octets = ipv4.octets();
                if matches!(
                    (octets[0], octets[1]),
                    (169, 254) | (198, 18) | (198, 19) | (100, 64..=127)
                ) {
                    continue;
                }
                ips.push(ipv4);
            }
        }
    }
    ips
}

// -------------------------------------------------------------------
// Tauri commands.
// -------------------------------------------------------------------

async fn tcp_connect_succeeded(socket_addr: SocketAddr, connect_timeout: Duration) -> bool {
    matches!(
        tokio::time::timeout(connect_timeout, tokio::net::TcpStream::connect(socket_addr)).await,
        Ok(Ok(_))
    )
}

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

    // #5: Use tokio async connect instead of spawn_blocking to reduce thread creation.
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
                    // Try connecting through a physical adapter to bypass VPN/TUN routes.
                    let connected = if physical_ips.is_empty() {
                        // Common case: tokio async connect plus timeout.
                        tcp_connect_succeeded(socket_addr, connect_timeout)
                            .await
                            .then_some(())
                    } else {
                        // Bind physical adapter: use spawn_blocking for socket2.
                        tokio::task::spawn_blocking(move || {
                            physical_ips.iter().find_map(|bind_ip| {
                                let bind_addr = SocketAddr::new(IpAddr::V4(*bind_ip), 0);
                                let sock_addr = socket2::SockAddr::from(socket_addr);
                                socket2::Socket::new(
                                    socket2::Domain::IPV4,
                                    socket2::Type::STREAM,
                                    None,
                                )
                                .ok()
                                .and_then(|s| {
                                    s.bind(&bind_addr.into()).ok()?;
                                    s.connect_timeout(&sock_addr, connect_timeout).ok()?;
                                    s.set_nonblocking(false).ok()?;
                                    Some(())
                                })
                            })
                        })
                        .await
                        .ok()
                        .flatten()
                    };

                    if connected.is_some() {
                        let parts: Vec<&str> = addr.split(':').collect();
                        if parts.len() >= 2 {
                            let host = parts[0].to_string();
                            let worker_name =
                                format!("Worker-{}", host.split('.').next_back().unwrap_or("?"));
                            // Use lock().await instead of try_lock() so high concurrency cannot silently drop discovered workers.
                            let mut list = discovered.lock().await;
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
            });
            handles.push(handle);
        }
        for h in handles {
            let _ = h.await;
        }
        let discovered = discovered.lock().await;
        discovered.clone()
    };

    match tokio_timeout(overall_timeout, scan_future).await {
        Ok(results) => {
            let mut existing = if let Ok(w) = state.workers.lock() {
                if w.is_empty() {
                    load_workers()
                } else {
                    w.clone()
                }
            } else {
                load_workers()
            };
            for d in &results {
                if let Some(e) = existing
                    .iter_mut()
                    .find(|w| w.host == d.host && w.port == d.port)
                {
                    e.status = WorkerStatus::Online;
                    e.last_seen = Some(chrono::Utc::now().to_rfc3339());
                } else {
                    existing.push(d.clone());
                }
            }
            save_workers(&existing);
            if let Ok(mut w) = state.workers.lock() {
                *w = existing.clone();
            }
            Ok(existing)
        }
        Err(_) => {
            // Timeout: return whatever was found so far.
            let partial = discovered.lock().await.clone();
            Ok(partial)
        }
    }
}

#[tauri::command]
pub async fn test_worker(
    host: String,
    port: u16,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let timeout = Duration::from_secs(3);

    match utils::connect_tcp(&host, port, timeout) {
        Ok(_) => {
            let mut workers = load_workers();
            if let Some(w) = workers
                .iter_mut()
                .find(|w| w.host == host && w.port == port)
            {
                w.status = WorkerStatus::Online;
                w.last_seen = Some(chrono::Utc::now().to_rfc3339());
            }
            save_workers(&workers);
            if let Ok(mut w) = state.workers.lock() {
                *w = workers;
            }

            Ok(serde_json::json!({ "ok": true }))
        }
        Err(e) => {
            let mut workers = load_workers();
            if let Some(w) = workers
                .iter_mut()
                .find(|w| w.host == host && w.port == port)
            {
                w.status = WorkerStatus::Offline;
            }
            save_workers(&workers);
            if let Ok(mut w) = state.workers.lock() {
                *w = workers;
            }

            Ok(serde_json::json!({ "ok": false, "error": e }))
        }
    }
}

#[tauri::command]
pub async fn get_worker_info(host: String, _port: u16) -> Result<Vec<WorkerDevice>, String> {
    // Check whether this is a local worker.
    let is_local =
        host == "127.0.0.1" || host == "localhost" || host == "::1" || is_local_ip(&host);

    if !is_local {
        // Remote worker: rpc-server does not expose device query APIs.
        return Ok(Vec::new());
    }

    let binary = find_rpc_server_binary_internal().unwrap_or_else(|| RPC_SERVER_NAME.to_string());

    // Start rpc-server on a temporary port, capture device output, then stop it immediately.
    let devices = tokio::task::spawn_blocking(move || -> Result<Vec<WorkerDevice>, String> {
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

        for line in reader.lines().map_while(Result::ok) {
            if line.starts_with("Devices:") {
                in_devices = true;
                continue;
            }
            if in_devices && line.starts_with("Starting RPC server") {
                break;
            }
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
                    let total = parts
                        .first()
                        .and_then(|s| s.split_whitespace().next()?.parse::<u64>().ok())
                        .unwrap_or(0);
                    let free = parts
                        .get(1)
                        .and_then(|s| s.split_whitespace().next()?.parse::<u64>().ok())
                        .unwrap_or(0);
                    devices.push(WorkerDevice {
                        device_type,
                        name,
                        vram_mb: total,
                        free_mb: free,
                    });
                }
            }
        }

        let _ = child.kill();
        let _ = child.wait();
        Ok(devices)
    })
    .await
    .map_err(|e| format!("任务执行失败: {}", e))??;

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
            if iface.is_loopback() {
                continue;
            }
            if let IpAddr::V4(ipv4) = iface.addr.ip() {
                let name_lower = iface.name.to_lowercase();
                let is_virtual = VIRTUAL_KEYWORDS.iter().any(|k| name_lower.contains(k));
                if !is_virtual {
                    return Ok(ipv4.to_string());
                }
            }
        }
    }
    Ok("127.0.0.1".to_string())
}

#[tauri::command]
pub async fn add_worker(
    host: String,
    port: u16,
    name: String,
    state: State<'_, AppState>,
) -> Result<WorkerInfo, String> {
    let mut workers = load_workers();

    if let Some(existing) = workers
        .iter_mut()
        .find(|w| w.host == host && w.port == port)
    {
        if !name.is_empty() {
            existing.name = name;
        }
        let result = existing.clone();
        let _ = existing; // Release the mutable borrow.
        save_workers(&workers);
        if let Ok(mut w) = state.workers.lock() {
            *w = workers.clone();
        }
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
    if let Ok(mut w) = state.workers.lock() {
        *w = workers;
    }

    Ok(worker)
}

#[tauri::command]
pub async fn remove_worker(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut workers = load_workers();
    workers.retain(|w| w.id != id);
    save_workers(&workers);
    if let Ok(mut w) = state.workers.lock() {
        *w = workers;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_workers(state: State<'_, AppState>) -> Result<Vec<WorkerInfo>, String> {
    let workers = load_workers();
    if let Ok(mut w) = state.workers.lock() {
        *w = workers.clone();
    }
    Ok(workers)
}

#[tauri::command]
pub async fn find_rpc_server_binary(_state: State<'_, AppState>) -> Result<Option<String>, String> {
    // #8: Delegate to the internal function to remove duplication.
    Ok(find_rpc_server_binary_internal())
}

#[tauri::command]
pub async fn generate_rpc_launch_cmd(port: u16) -> Result<String, String> {
    let binary = find_rpc_server_binary_internal().unwrap_or_else(|| RPC_SERVER_NAME.to_string());
    Ok(format!("{} --host 127.0.0.1 --port {}", binary, port))
}

#[tauri::command]
pub async fn stop_local_worker(port: u16) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || stop_verified_rpc_server(port))
        .await
        .map_err(|e| format!("停止 Worker 失败: {}", e))?
}

fn is_rpc_server_executable(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(RPC_SERVER_NAME))
}

fn is_rpc_server_process(pid: u32) -> bool {
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    system
        .process(pid)
        .and_then(|process| process.exe())
        .is_some_and(is_rpc_server_executable)
}

fn wait_for_tcp_ready(addr: SocketAddr, max_wait: Duration) -> Result<(), String> {
    let deadline = std::time::Instant::now() + max_wait;
    let mut last_error = None;

    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            break;
        }
        let remaining = deadline.saturating_duration_since(now);
        let attempt_timeout = remaining.min(Duration::from_millis(250));
        match std::net::TcpStream::connect_timeout(&addr, attempt_timeout) {
            Ok(_) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        std::thread::sleep(
            deadline
                .saturating_duration_since(std::time::Instant::now())
                .min(Duration::from_millis(100)),
        );
    }

    Err(last_error
        .map(|error| error.to_string())
        .unwrap_or_else(|| "readiness check timed out".to_string()))
}

fn terminate_rpc_child(child: &mut std::process::Child) -> Result<(), String> {
    match child.kill() {
        Ok(()) => child
            .wait()
            .map(|_| ())
            .map_err(|error| format!("等待 rpc-server 退出失败: {error}")),
        Err(kill_error) => match child.try_wait() {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(format!("终止 rpc-server 失败: {kill_error}")),
            Err(wait_error) => Err(format!(
                "终止 rpc-server 失败: {kill_error}；查询进程状态失败: {wait_error}"
            )),
        },
    }
}

fn listening_pids(port: u16) -> Result<Vec<u32>, String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("cmd");
        command.creation_flags(0x08000000);
        let output = command
            .args(["/c", &format!("netstat -ano | findstr :{}", port)])
            .output()
            .map_err(|e| format!("netstat 失败: {}", e))?;
        let out = String::from_utf8_lossy(&output.stdout);
        let pid_re = regex_lite::Regex::new(r"LISTENING\s+(\d+)$")
            .map_err(|e| format!("PID 解析器初始化失败: {e}"))?;
        Ok(out
            .lines()
            .filter(|line| line.contains(&format!(":{port}")) && line.contains("LISTENING"))
            .filter_map(|line| pid_re.captures(line))
            .filter_map(|captures| captures.get(1))
            .filter_map(|value| value.as_str().parse::<u32>().ok())
            .collect())
    }
    #[cfg(target_os = "linux")]
    {
        let output = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "ss -tlnp | grep ':{}' | sed -n 's/.*pid=\\([0-9]*\\).*/\\1/p'",
                port
            ))
            .output()
            .map_err(|e| format!("ss 失败: {}", e))?;
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().parse::<u32>().ok())
            .collect())
    }
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("lsof")
            .args(["-ti", &format!(":{}", port)])
            .output()
            .map_err(|e| format!("lsof 失败: {}", e))?;
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().parse::<u32>().ok())
            .collect())
    }
}

fn stop_verified_rpc_server(port: u16) -> Result<bool, String> {
    for pid in listening_pids(port)? {
        if !is_rpc_server_process(pid) {
            continue;
        }
        #[cfg(target_os = "windows")]
        let stopped = {
            use std::os::windows::process::CommandExt;
            let mut command = Command::new("taskkill");
            command.creation_flags(0x08000000);
            command
                .args(["/PID", &pid.to_string(), "/F", "/T"])
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        };
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let stopped = Command::new("kill")
            .arg(pid.to_string())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if stopped {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn find_rpc_server_binary_internal() -> Option<String> {
    #[cfg(target_os = "windows")]
    const RPC_SERVER_NAME: &str = "rpc-server.exe";
    #[cfg(not(target_os = "windows"))]
    const RPC_SERVER_NAME: &str = "rpc-server";

    let exe = std::env::current_exe().unwrap_or_default();
    let exe_dir = exe.parent().unwrap_or(std::path::Path::new("."));

    for dir in [
        exe_dir,
        exe_dir.parent().unwrap_or(std::path::Path::new(".")),
    ] {
        let candidate = dir.join(RPC_SERVER_NAME);
        if candidate.exists() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(RPC_SERVER_NAME);
            if candidate.exists() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }
    None
}

#[tauri::command]
pub async fn get_cluster_metrics(_state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let workers = load_workers();
    let online: Vec<&WorkerInfo> = workers
        .iter()
        .filter(|w| w.status == WorkerStatus::Online)
        .collect();

    let mut worker_metrics = Vec::new();
    let mut reachable_count = 0u32;
    for w in &online {
        let reachable =
            utils::connect_tcp(&w.host, w.port, std::time::Duration::from_millis(500)).is_ok();
        if reachable {
            reachable_count += 1;
        }

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
        "online_workers": reachable_count,
        "worker_metrics": worker_metrics,
    }))
}

#[tauri::command]
pub async fn start_local_rpc(
    engine_dir: Option<String>,
    port: u16,
) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let binary = if let Some(ref dir) = engine_dir {
            let path = std::path::Path::new(dir);
            #[cfg(target_os = "windows")]
            let exe = path.join("rpc-server.exe");
            #[cfg(not(target_os = "windows"))]
            let exe = path.join("rpc-server");
            if exe.exists() {
                exe.to_string_lossy().to_string()
            } else {
                return Err(format!("引擎目录下未找到 rpc-server: {}", dir));
            }
        } else {
            find_rpc_server_binary_internal()
                .ok_or_else(|| "未找到 rpc-server，请指定引擎目录".to_string())?
        };

        #[cfg(target_os = "windows")]
        let mut child = {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x00000008;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            std::process::Command::new(&binary)
                .args(["--host", "127.0.0.1", "--port", &port.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| format!("无法启动 {}: {}", binary, e))?
        };
        #[cfg(not(target_os = "windows"))]
        let mut child = {
            std::process::Command::new("nohup")
                .arg(&binary)
                .args(["--host", "127.0.0.1", "--port", &port.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("无法启动 {}: {}", binary, e))?
        };

        let addr = format!("127.0.0.1:{}", port);
        let sock_addr = addr
            .parse::<std::net::SocketAddr>()
            .map_err(|e| format!("地址解析失败 ({}): {}", addr, e))?;
        match wait_for_tcp_ready(sock_addr, Duration::from_secs(5)) {
            Ok(_) => Ok(serde_json::json!({ "ok": true, "port": port })),
            Err(error) => {
                let message = format!("rpc-server 启动后无法连接: {error}");
                match terminate_rpc_child(&mut child) {
                    Ok(()) => Err(message),
                    Err(cleanup_error) => Err(format!("{message}；{cleanup_error}")),
                }
            }
        }
    })
    .await
    .map_err(|e| format!("启动失败: {}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workers_persistence() {
        let dir = std::env::temp_dir().join("lsm_test_workers");
        let _ = std::fs::create_dir_all(&dir);
        let test_path = dir.join("workers.json");

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

        save_workers_to(&test_path, &test_workers);
        let loaded = load_workers_from(&test_path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].host, "192.168.1.10");

        // Clean up temporary files.
        let _ = std::fs::remove_file(&test_path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_load_empty() {
        let dir = std::env::temp_dir().join("lsm_test_empty");
        let _ = std::fs::create_dir_all(&dir);
        let test_path = dir.join("nonexistent.json");
        let loaded = load_workers_from(&test_path);
        assert!(loaded.is_empty());
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn tcp_readiness_accepts_a_listening_socket() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        assert!(wait_for_tcp_ready(addr, Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn test_generate_rpc_launch_cmd() {
        let cmd = generate_rpc_launch_cmd_internal(50052);
        assert!(cmd.contains("50052"));
        assert!(cmd.contains("127.0.0.1"));
    }

    #[test]
    fn rpc_process_identity_requires_the_expected_executable_name() {
        assert!(is_rpc_server_executable(std::path::Path::new(
            RPC_SERVER_NAME
        )));
        assert!(!is_rpc_server_executable(std::path::Path::new(
            "unrelated-server.exe"
        )));
    }

    #[tokio::test]
    async fn tcp_probe_rejects_a_refused_connection() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);

        assert!(!tcp_connect_succeeded(address, std::time::Duration::from_millis(200)).await);
    }

    fn generate_rpc_launch_cmd_internal(port: u16) -> String {
        #[cfg(target_os = "windows")]
        const NAME: &str = "rpc-server.exe";
        #[cfg(not(target_os = "windows"))]
        const NAME: &str = "rpc-server";
        format!("{} --host 127.0.0.1 --port {}", NAME, port)
    }
}
