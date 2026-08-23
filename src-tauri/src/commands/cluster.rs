use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use sysinfo::{Pid, ProcessesToUpdate, System};
use tauri::State;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tokio::time::timeout as tokio_timeout;

use crate::models::{AppState, WorkerDevice, WorkerInfo, WorkerOrigin, WorkerStatus};
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

static LOCAL_RPC_WORKERS: LazyLock<Mutex<HashMap<u16, Child>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static WORKERS_WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
pub(crate) const MAX_AUTO_DISCOVERED_WORKERS: usize = 128;

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
        .map(|workers| workers.into_iter().filter(is_persistable_worker).collect())
        .unwrap_or_default()
}

fn is_persistable_worker(worker: &WorkerInfo) -> bool {
    !worker.auto_discovered && worker.origin != WorkerOrigin::Ssh
}

pub(crate) fn save_workers(workers: &[WorkerInfo]) -> Result<(), String> {
    save_workers_to(&workers_path(), workers)
}

fn save_workers_to(path: &std::path::Path, workers: &[WorkerInfo]) -> Result<(), String> {
    let workers = workers
        .iter()
        .filter(|worker| is_persistable_worker(worker))
        .collect::<Vec<_>>();
    let json = serde_json::to_vec_pretty(&workers)
        .map_err(|error| format!("failed to serialize workers: {error}"))?;
    crate::persistence::atomic_write(path, &json, None)
}

pub(crate) fn update_workers<R, F>(state: &AppState, update: F) -> Result<R, String>
where
    F: FnOnce(&mut Vec<WorkerInfo>) -> R,
{
    let _write_guard = WORKERS_WRITE_LOCK
        .lock()
        .map_err(|_| "worker persistence lock is poisoned".to_string())?;
    let (result, previous, snapshot) = {
        let mut workers = state
            .workers
            .lock()
            .map_err(|_| "worker state lock is poisoned".to_string())?;
        let previous = workers.clone();
        let result = update(&mut workers);
        (result, previous, workers.clone())
    };
    let trusted_snapshot = snapshot
        .iter()
        .filter(|worker| is_persistable_worker(worker))
        .cloned()
        .collect::<Vec<_>>();
    let trusted_previous = previous
        .iter()
        .filter(|worker| is_persistable_worker(worker))
        .cloned()
        .collect::<Vec<_>>();
    if serde_json::to_vec(&trusted_previous).ok() == serde_json::to_vec(&trusted_snapshot).ok() {
        return Ok(result);
    }
    if let Err(error) = save_workers(&trusted_snapshot) {
        if let Ok(mut workers) = state.workers.lock() {
            *workers = previous;
        }
        return Err(error);
    }
    Ok(result)
}

pub(crate) fn stable_discovered_worker_id(host: &str, port: u16, service: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in format!("{host}|{port}|{service}").bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("auto-{hash:016x}")
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
                            if list.len() < MAX_AUTO_DISCOVERED_WORKERS {
                                list.push(WorkerInfo {
                                    id: stable_discovered_worker_id(&host, port, &worker_name),
                                    host,
                                    port,
                                    name: worker_name,
                                    origin: WorkerOrigin::Manual,
                                    devices: Vec::new(),
                                    status: WorkerStatus::Unknown,
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
        for h in handles {
            let _ = h.await;
        }
        let discovered = discovered.lock().await;
        discovered.clone()
    };

    match tokio_timeout(overall_timeout, scan_future).await {
        Ok(results) => {
            let merged = update_workers(&state, |existing| {
                for discovered in &results {
                    if let Some(worker) = existing.iter_mut().find(|worker| {
                        worker.host == discovered.host && worker.port == discovered.port
                    }) {
                        if worker.auto_discovered {
                            worker.status = WorkerStatus::Unknown;
                        }
                        worker.last_seen = Some(chrono::Utc::now().to_rfc3339());
                    } else if existing
                        .iter()
                        .filter(|worker| worker.auto_discovered)
                        .count()
                        < MAX_AUTO_DISCOVERED_WORKERS
                    {
                        existing.push(discovered.clone());
                    }
                }
                existing.clone()
            })?;
            Ok(merged)
        }
        Err(_) => {
            let partial = discovered.lock().await.clone();
            update_workers(&state, |existing| {
                for discovered in &partial {
                    if let Some(worker) = existing.iter_mut().find(|worker| {
                        worker.host == discovered.host && worker.port == discovered.port
                    }) {
                        if worker.auto_discovered {
                            worker.status = WorkerStatus::Unknown;
                        }
                        worker.last_seen = Some(chrono::Utc::now().to_rfc3339());
                    } else if existing
                        .iter()
                        .filter(|worker| worker.auto_discovered)
                        .count()
                        < MAX_AUTO_DISCOVERED_WORKERS
                    {
                        existing.push(discovered.clone());
                    }
                }
                existing.clone()
            })
        }
    }
}

pub async fn test_worker(
    host: String,
    port: u16,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let timeout = Duration::from_secs(3);

    match utils::connect_tcp(&host, port, timeout) {
        Ok(_) => {
            update_workers(&state, |workers| {
                if let Some(worker) = workers
                    .iter_mut()
                    .find(|worker| worker.host == host && worker.port == port)
                {
                    worker.status = if worker.auto_discovered {
                        WorkerStatus::Unknown
                    } else {
                        WorkerStatus::Online
                    };
                    worker.last_seen = Some(chrono::Utc::now().to_rfc3339());
                }
            })?;

            Ok(serde_json::json!({ "ok": true }))
        }
        Err(e) => {
            update_workers(&state, |workers| {
                if let Some(worker) = workers
                    .iter_mut()
                    .find(|worker| worker.host == host && worker.port == port)
                {
                    worker.status = WorkerStatus::Offline;
                }
            })?;

            Ok(serde_json::json!({ "ok": false, "error": e }))
        }
    }
}

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

pub async fn is_local_host(host: String) -> Result<bool, String> {
    Ok(host == "127.0.0.1" || host == "localhost" || host == "::1" || is_local_ip(&host))
}

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

pub async fn add_worker(
    host: String,
    port: u16,
    name: String,
    origin: Option<WorkerOrigin>,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<WorkerInfo, String> {
    confirm_worker_enrollment(&app, &host, port).await?;
    let origin = origin.unwrap_or_default();
    update_workers(&state, |workers| {
        if let Some(existing) = workers
            .iter_mut()
            .find(|worker| worker.host == host && worker.port == port)
        {
            if !name.is_empty() {
                existing.name = name.clone();
            }
            existing.origin = origin;
            existing.auto_discovered = false;
            existing.status = WorkerStatus::Unknown;
            return existing.clone();
        }

        let worker = WorkerInfo {
            id: uuid::Uuid::new_v4().to_string(),
            host: host.clone(),
            port,
            name: if name.is_empty() { host.clone() } else { name },
            origin,
            devices: Vec::new(),
            status: WorkerStatus::Unknown,
            last_seen: None,
            auto_discovered: false,
        };
        workers.push(worker.clone());
        worker
    })
}

async fn confirm_worker_enrollment(
    app: &tauri::AppHandle,
    host: &str,
    port: u16,
) -> Result<(), String> {
    let host = host.trim();
    if host.is_empty()
        || host.len() > 255
        || port == 0
        || host.chars().any(|character| character.is_control())
    {
        return Err("Worker endpoint is invalid".to_string());
    }
    let message = format!(
        "是否信任 RPC 节点 {host}:{port}？\n\nRPC 节点没有可验证的身份，可能影响推理结果的完整性与可用性。请只批准你控制的节点。"
    );
    let app = app.clone();
    let approved = tokio::task::spawn_blocking(move || {
        app.dialog()
            .message(message)
            .title("确认信任 RPC 节点")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "信任".to_string(),
                "取消".to_string(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|error| format!("Worker approval dialog failed: {error}"))?;
    if approved {
        Ok(())
    } else {
        Err("Worker enrollment was not approved".to_string())
    }
}

pub async fn approve_worker(
    id: String,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<WorkerInfo, String> {
    let candidate = state
        .workers
        .lock()
        .map_err(|_| "worker state lock is poisoned".to_string())?
        .iter()
        .find(|worker| worker.id == id && worker.auto_discovered)
        .cloned()
        .ok_or_else(|| "Discovered worker not found".to_string())?;
    confirm_worker_enrollment(&app, &candidate.host, candidate.port).await?;
    update_workers(&state, |workers| {
        let worker = workers
            .iter_mut()
            .find(|worker| worker.id == id && worker.auto_discovered)
            .ok_or_else(|| "Discovered worker not found".to_string())?;
        worker.auto_discovered = false;
        worker.status = WorkerStatus::Unknown;
        Ok(worker.clone())
    })?
}

pub async fn remove_worker(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let worker = state
        .workers
        .lock()
        .map_err(|_| "worker state lock is poisoned".to_string())?
        .iter()
        .find(|worker| worker.id == id)
        .cloned();
    if let Some(worker) = worker {
        match worker.origin {
            WorkerOrigin::Local => {
                let _ = stop_local_worker(worker.port).await?;
            }
            WorkerOrigin::Ssh => {
                let _ = crate::commands::cluster_ssh::stop_ssh_tunnel(worker.port).await?;
            }
            WorkerOrigin::Manual => {}
        }
    }
    update_workers(&state, |workers| workers.retain(|worker| worker.id != id))
}

pub async fn get_workers(state: State<'_, AppState>) -> Result<Vec<WorkerInfo>, String> {
    state
        .workers
        .lock()
        .map(|workers| workers.clone())
        .map_err(|_| "worker state lock is poisoned".to_string())
}

pub async fn find_rpc_server_binary(_state: State<'_, AppState>) -> Result<Option<String>, String> {
    // #8: Delegate to the internal function to remove duplication.
    Ok(find_rpc_server_binary_internal())
}

pub async fn generate_rpc_launch_cmd(port: u16) -> Result<String, String> {
    let binary = find_rpc_server_binary_internal().unwrap_or_else(|| RPC_SERVER_NAME.to_string());
    Ok(format!(
        "{} --host 127.0.0.1 --port {}",
        quote_launch_binary(&binary),
        port
    ))
}

fn quote_launch_binary(binary: &str) -> String {
    if !binary.chars().any(|character| {
        character.is_whitespace() || matches!(character, '"' | '\'' | '&' | '(' | ')')
    }) {
        return binary.to_string();
    }
    #[cfg(target_os = "windows")]
    {
        format!("\"{}\"", binary.replace('"', "\\\""))
    }
    #[cfg(not(target_os = "windows"))]
    {
        format!("'{}'", binary.replace('\'', "'\"'\"'"))
    }
}

pub async fn stop_local_worker(port: u16) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || {
        if let Some(mut child) = LOCAL_RPC_WORKERS.lock().unwrap().remove(&port) {
            terminate_rpc_child(&mut child)?;
            return Ok(true);
        }
        stop_verified_rpc_server(port)
    })
    .await
    .map_err(|e| format!("停止 Worker 失败: {}", e))?
}

pub async fn stop_worker(id: String, state: State<'_, AppState>) -> Result<bool, String> {
    let worker = state
        .workers
        .lock()
        .map_err(|_| "worker state lock is poisoned".to_string())?
        .iter()
        .find(|worker| worker.id == id)
        .cloned()
        .ok_or_else(|| "Worker not found".to_string())?;
    let stopped = match worker.origin {
        WorkerOrigin::Local => stop_local_worker(worker.port).await?,
        WorkerOrigin::Ssh => crate::commands::cluster_ssh::stop_ssh_tunnel(worker.port).await?,
        WorkerOrigin::Manual => {
            return Err("Only manager-owned local or SSH workers can be stopped".to_string())
        }
    };
    update_workers(&state, |workers| {
        if let Some(worker) = workers.iter_mut().find(|candidate| candidate.id == id) {
            worker.status = WorkerStatus::Offline;
        }
    })?;
    Ok(stopped)
}

pub fn stop_all_local_rpc_workers(managed_ports: &[u16]) {
    let workers = {
        let mut workers = LOCAL_RPC_WORKERS.lock().unwrap();
        std::mem::take(&mut *workers)
    };
    let tracked_ports = workers.keys().copied().collect::<HashSet<_>>();
    for (port, mut child) in workers {
        if let Err(error) = terminate_rpc_child(&mut child) {
            eprintln!("Failed to terminate local Worker on port {port}: {error}");
        }
    }
    for port in managed_ports {
        if !tracked_ports.contains(port) {
            if let Err(error) = stop_verified_rpc_server(*port) {
                eprintln!("Failed to terminate restored local Worker on port {port}: {error}");
            }
        }
    }
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
    None
}

pub async fn get_cluster_metrics(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let workers = state
        .workers
        .lock()
        .map_err(|_| "worker state lock is poisoned".to_string())?
        .clone();
    let online: Vec<WorkerInfo> = workers
        .iter()
        .filter(|w| w.status == WorkerStatus::Online)
        .cloned()
        .collect();
    let probes = online.into_iter().map(|worker| async move {
        let reachable = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            tokio::net::TcpStream::connect((worker.host.as_str(), worker.port)),
        )
        .await
        .is_ok_and(|result| result.is_ok());
        let metrics = serde_json::json!({
            "host": worker.host,
            "port": worker.port,
            "name": worker.name,
            "online": reachable,
            "devices": worker.devices.iter().map(|d| serde_json::json!({
                "type": d.device_type,
                "name": d.name,
                "vram_mb": d.vram_mb,
                "free_mb": d.free_mb,
            })).collect::<Vec<_>>(),
        });
        (reachable, metrics)
    });
    let probe_results = futures_util::future::join_all(probes).await;
    let reachable_count = probe_results
        .iter()
        .filter(|(reachable, _)| *reachable)
        .count() as u32;
    let worker_metrics = probe_results
        .into_iter()
        .map(|(_, metrics)| metrics)
        .collect::<Vec<_>>();

    Ok(serde_json::json!({
        "total_workers": workers.len(),
        "online_workers": reachable_count,
        "worker_metrics": worker_metrics,
    }))
}

pub async fn start_local_rpc(
    engine_dir: Option<String>,
    port: u16,
) -> Result<serde_json::Value, String> {
    if port == 0 {
        return Err("RPC port must be between 1 and 65535".to_string());
    }
    if LOCAL_RPC_WORKERS.lock().unwrap().contains_key(&port) {
        return Err(format!("本地 Worker 端口 {port} 已由管理器占用"));
    }
    let engine_dir = engine_dir
        .map(|dir| {
            crate::security::require_authorized_engine_root(std::path::Path::new(&dir))
                .map(|path| path.to_string_lossy().to_string())
        })
        .transpose()?;
    tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let availability_probe = std::net::TcpListener::bind(("127.0.0.1", port))
            .map_err(|error| format!("本地 Worker 端口 {port} 不可用: {error}"))?;
        drop(availability_probe);
        let binary = if let Some(ref dir) = engine_dir {
            let path = std::path::Path::new(dir);
            #[cfg(target_os = "windows")]
            let exe = path.join("rpc-server.exe");
            #[cfg(not(target_os = "windows"))]
            let exe = path.join("rpc-server");
            if exe.exists() {
                crate::security::require_path_within_root(&exe, path)?
                    .to_string_lossy()
                    .to_string()
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
            Ok(_) => {
                if let Some(status) = child
                    .try_wait()
                    .map_err(|error| format!("无法检查 rpc-server 状态: {error}"))?
                {
                    return Err(format!("rpc-server 提前退出: {status}"));
                }
                let pid = child.id();
                let mut workers = LOCAL_RPC_WORKERS.lock().unwrap();
                if workers.contains_key(&port) {
                    drop(workers);
                    terminate_rpc_child(&mut child)?;
                    return Err(format!("本地 Worker 端口 {port} 已由管理器占用"));
                }
                workers.insert(port, child);
                Ok(serde_json::json!({ "ok": true, "port": port, "pid": pid }))
            }
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

        let discovered = WorkerInfo {
            id: "auto-1".into(),
            host: "192.168.1.11".into(),
            port: 50052,
            name: "Discovered Worker".into(),
            origin: WorkerOrigin::Manual,
            devices: Vec::new(),
            status: WorkerStatus::Unknown,
            last_seen: Some("2024-01-01T00:00:00Z".into()),
            auto_discovered: true,
        };
        let test_workers = vec![
            WorkerInfo {
                id: "test-1".into(),
                host: "192.168.1.10".into(),
                port: 50052,
                name: "Test Worker".into(),
                origin: WorkerOrigin::Manual,
                devices: Vec::new(),
                status: WorkerStatus::Online,
                last_seen: Some("2024-01-01T00:00:00Z".into()),
                auto_discovered: false,
            },
            WorkerInfo {
                id: "local-1".into(),
                host: "127.0.0.1".into(),
                port: 50053,
                name: "Local Worker".into(),
                origin: WorkerOrigin::Local,
                devices: Vec::new(),
                status: WorkerStatus::Online,
                last_seen: None,
                auto_discovered: false,
            },
            WorkerInfo {
                id: "ssh-1".into(),
                host: "127.0.0.1".into(),
                port: 50054,
                name: "SSH Worker".into(),
                origin: WorkerOrigin::Ssh,
                devices: Vec::new(),
                status: WorkerStatus::Online,
                last_seen: None,
                auto_discovered: false,
            },
            discovered.clone(),
        ];

        save_workers_to(&test_path, &test_workers).unwrap();
        let loaded = load_workers_from(&test_path);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].host, "192.168.1.10");
        assert_eq!(loaded[1].origin, WorkerOrigin::Local);
        assert!(loaded
            .iter()
            .all(|worker| worker.origin != WorkerOrigin::Ssh));
        assert_ne!(loaded[0].id, discovered.id);

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
    fn discovered_worker_identity_includes_host_port_and_service() {
        let first = stable_discovered_worker_id("worker.local", 50052, "rpc-a");
        let same = stable_discovered_worker_id("worker.local", 50052, "rpc-a");
        let other_port = stable_discovered_worker_id("worker.local", 50053, "rpc-a");
        let other_service = stable_discovered_worker_id("worker.local", 50052, "rpc-b");

        assert_eq!(first, same);
        assert_ne!(first, other_port);
        assert_ne!(first, other_service);
    }

    #[test]
    fn launch_binary_with_spaces_is_shell_quoted() {
        let quoted = quote_launch_binary("C:/Program Files/Llama/rpc-server.exe");

        #[cfg(target_os = "windows")]
        assert_eq!(quoted, "\"C:/Program Files/Llama/rpc-server.exe\"");
        #[cfg(not(target_os = "windows"))]
        assert_eq!(quoted, "'C:/Program Files/Llama/rpc-server.exe'");
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

// IPC compatibility boundary: legacy command internals keep their existing error flow,
// while every registered command serializes a stable AppError object.
#[allow(dead_code, unused_imports, unused_mut)] // Tauri references adapters through generated macros.
pub mod ipc {
    use super::*;

    #[tauri::command]
    pub async fn scan_workers_tcp(
        state: State<'_, AppState>,
    ) -> crate::error::AppResult<Vec<WorkerInfo>> {
        super::scan_workers_tcp(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn test_worker(
        host: String,
        port: u16,
        state: State<'_, AppState>,
    ) -> crate::error::AppResult<serde_json::Value> {
        super::test_worker(host, port, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_worker_info(
        host: String,
        _port: u16,
    ) -> crate::error::AppResult<Vec<WorkerDevice>> {
        super::get_worker_info(host, _port)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn is_local_host(host: String) -> crate::error::AppResult<bool> {
        super::is_local_host(host)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_local_host() -> crate::error::AppResult<String> {
        super::get_local_host()
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn add_worker(
        host: String,
        port: u16,
        name: String,
        origin: Option<WorkerOrigin>,
        state: State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<WorkerInfo> {
        super::add_worker(host, port, name, origin, state, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn approve_worker(
        id: String,
        state: State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<WorkerInfo> {
        super::approve_worker(id, state, app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn remove_worker(
        id: String,
        state: State<'_, AppState>,
    ) -> crate::error::AppResult<()> {
        super::remove_worker(id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_workers(
        state: State<'_, AppState>,
    ) -> crate::error::AppResult<Vec<WorkerInfo>> {
        super::get_workers(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn find_rpc_server_binary(
        _state: State<'_, AppState>,
    ) -> crate::error::AppResult<Option<String>> {
        super::find_rpc_server_binary(_state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn generate_rpc_launch_cmd(port: u16) -> crate::error::AppResult<String> {
        super::generate_rpc_launch_cmd(port)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn stop_local_worker(port: u16) -> crate::error::AppResult<bool> {
        super::stop_local_worker(port)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn stop_worker(
        id: String,
        state: State<'_, AppState>,
    ) -> crate::error::AppResult<bool> {
        super::stop_worker(id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_cluster_metrics(
        state: State<'_, AppState>,
    ) -> crate::error::AppResult<serde_json::Value> {
        super::get_cluster_metrics(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn start_local_rpc(
        engine_dir: Option<String>,
        port: u16,
    ) -> crate::error::AppResult<serde_json::Value> {
        super::start_local_rpc(engine_dir, port)
            .await
            .map_err(crate::error::AppError::from)
    }
}
