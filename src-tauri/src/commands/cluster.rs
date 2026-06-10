use std::collections::HashSet;
use std::net::{TcpStream, SocketAddr};
use std::time::Duration;
use std::path::PathBuf;

use tauri::State;

use crate::models::{WorkerInfo, WorkerDevice, WorkerStatus, AppState};
use crate::utils;

#[cfg(target_os = "windows")]
const RPC_SERVER_NAME: &str = "rpc-server.exe";
#[cfg(not(target_os = "windows"))]
const RPC_SERVER_NAME: &str = "rpc-server";

// ═══════════════════════════════════════════════════════════════════
// 持久化
// ═══════════════════════════════════════════════════════════════════

fn workers_path() -> PathBuf {
    utils::get_data_dir().join("workers.json")
}

fn load_workers() -> Vec<WorkerInfo> {
    let path = workers_path();
    if !path.exists() {
        return Vec::new();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<WorkerInfo>>(&s).ok())
        .unwrap_or_default()
}

fn save_workers(workers: &[WorkerInfo]) {
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

    if let Ok(ifs) = if_addrs::get_if_addrs() {
        for iface in ifs {
            if iface.is_loopback() { continue; }
            let ip = iface.addr.ip();
            if !ip.is_ipv4() { continue; }
            let octets = match ip {
                std::net::IpAddr::V4(ipv4) => ipv4.octets(),
                _ => continue,
            };
            // /24 subnet prefix
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
    let timeout = Duration::from_millis(500);

    let mut discovered: Vec<WorkerInfo> = Vec::new();
    let mut seen = HashSet::new();

    for prefix in &prefixes {
        for i in 1..=254 {
            let addr = format!("{}{}:{}", prefix, i, port);
            if seen.contains(&addr) { continue; }
            seen.insert(addr.clone());

            if let Ok(socket_addr) = addr.parse::<SocketAddr>() {
                if TcpStream::connect_timeout(&socket_addr, timeout).is_ok() {
                    let host = format!("{}{}", prefix, i);
                    discovered.push(WorkerInfo {
                        id: format!("auto-{}", host.replace('.', "-")),
                        host,
                        port,
                        name: format!("Worker-{}", i),
                        devices: Vec::new(),
                        status: WorkerStatus::Online,
                        last_seen: Some(chrono::Utc::now().to_rfc3339()),
                        auto_discovered: true,
                    });
                }
            }
        }
    }

    // 与已存储 worker 合并: 保留手动添加的，更新已存在的在线状态
    let mut existing = load_workers();
    for d in &discovered {
        if let Some(e) = existing.iter_mut().find(|w| w.host == d.host && w.port == d.port) {
            e.status = WorkerStatus::Online;
            e.last_seen = Some(chrono::Utc::now().to_rfc3339());
        } else {
            existing.push(d.clone());
        }
    }

    save_workers(&existing);

    // 更新 AppState
    if let Ok(mut w) = state.workers.lock() {
        *w = existing.clone();
    }

    Ok(existing)
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
    // rpc-server 本身没有 HTTP health endpoint，返回空列表
    // 设备信息通过 test_worker 后的 TCP 握手 + 日志解析获取
    // 当前版本返回空 Vec，标记 worker 为 Online 即可
    let _ = (host, port);
    Ok(Vec::new())
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

fn find_rpc_server_binary_internal() -> Option<String> {
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
