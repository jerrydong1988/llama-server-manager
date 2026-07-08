use crate::commands::cluster::{load_workers, save_workers};
use crate::models::{AppState, WorkerInfo, WorkerStatus};
use tauri::State;
use tokio::sync::Mutex;

static DISCOVERY_ACTIVE: Mutex<bool> = Mutex::const_new(false);

#[tauri::command]
pub async fn start_mdns_discovery(_state: State<'_, AppState>) -> Result<String, String> {
    let mut active = DISCOVERY_ACTIVE.lock().await;
    if *active {
        return Ok("already running".into());
    }
    *active = true;
    drop(active);

    // spawn background task — operates on workers.json directly, no state needed
    tokio::spawn(async move {
        if let Err(e) = run_mdns_discovery().await {
            eprintln!("mDNS discovery error: {}", e);
        }
    });

    Ok("started".into())
}

#[tauri::command]
pub async fn stop_mdns_discovery() -> Result<String, String> {
    let mut active = DISCOVERY_ACTIVE.lock().await;
    *active = false;
    Ok("stopped".into())
}

async fn run_mdns_discovery() -> Result<(), String> {
    let mdns = mdns_sd::ServiceDaemon::new().map_err(|e| format!("mdns init: {}", e))?;
    let receiver = mdns
        .browse("_rpc._tcp.local.")
        .map_err(|e| format!("mdns browse: {}", e))?;

    while *DISCOVERY_ACTIVE.lock().await {
        match tokio::time::timeout(std::time::Duration::from_secs(3), receiver.recv_async()).await {
            Ok(Ok(event)) => {
                if let mdns_sd::ServiceEvent::ServiceResolved(info) = event {
                    let host = info
                        .get_addresses()
                        .iter()
                        .find(|a| a.is_ipv4())
                        .map(|a| a.to_string())
                        .unwrap_or_else(|| info.get_hostname().to_string());
                    let port = info.get_port();

                    let mut workers = load_workers();
                    if !workers.iter().any(|w| w.host == host && w.port == port) {
                        workers.push(WorkerInfo {
                            id: format!("mdns-{}", host.replace('.', "-")),
                            host,
                            port,
                            name: info
                                .get_fullname()
                                .trim_end_matches("._rpc._tcp.local.")
                                .to_string(),
                            devices: Vec::new(),
                            status: WorkerStatus::Online,
                            last_seen: Some(chrono::Utc::now().to_rfc3339()),
                            auto_discovered: true,
                        });
                        save_workers(&workers);
                    }
                }
            }
            Ok(Err(_)) | Err(_) => {}
        }
    }

    let _ = mdns.shutdown();
    Ok(())
}
