use crate::commands::cluster::{stable_discovered_worker_id, update_workers};
use crate::models::{AppState, WorkerInfo, WorkerStatus};
use tauri::Manager;
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;

struct DiscoveryTask {
    cancel: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

static DISCOVERY_TASK: Mutex<Option<DiscoveryTask>> = Mutex::const_new(None);

pub async fn start_mdns_discovery(app: tauri::AppHandle) -> Result<String, String> {
    let mut task = DISCOVERY_TASK.lock().await;
    if task
        .as_ref()
        .is_some_and(|discovery| !discovery.handle.is_finished())
    {
        return Ok("already running".into());
    }
    if let Some(previous) = task.take() {
        let _ = previous.handle.await;
    }

    let (cancel, cancel_receiver) = oneshot::channel();
    let handle = tokio::spawn(async move {
        if let Err(e) = run_mdns_discovery(app, cancel_receiver).await {
            eprintln!("mDNS discovery error: {}", e);
        }
    });
    *task = Some(DiscoveryTask { cancel, handle });

    Ok("started".into())
}

pub async fn stop_mdns_discovery() -> Result<String, String> {
    let Some(mut task) = DISCOVERY_TASK.lock().await.take() else {
        return Ok("stopped".into());
    };

    let _ = task.cancel.send(());
    if tokio::time::timeout(std::time::Duration::from_secs(5), &mut task.handle)
        .await
        .is_err()
    {
        task.handle.abort();
        let _ = task.handle.await;
    }
    Ok("stopped".into())
}

async fn run_mdns_discovery(
    app: tauri::AppHandle,
    mut cancel: oneshot::Receiver<()>,
) -> Result<(), String> {
    let mdns = mdns_sd::ServiceDaemon::new().map_err(|e| format!("mdns init: {}", e))?;
    let receiver = mdns
        .browse("_rpc._tcp.local.")
        .map_err(|e| format!("mdns browse: {}", e))?;

    loop {
        tokio::select! {
            _ = &mut cancel => break,
            event = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                receiver.recv_async(),
            ) => {
                if let Ok(Ok(mdns_sd::ServiceEvent::ServiceResolved(info))) = event {
                    let host = info
                        .get_addresses()
                        .iter()
                        .find(|a| a.is_ipv4())
                        .map(|a| a.to_string())
                        .unwrap_or_else(|| info.get_hostname().to_string());
                    let port = info.get_port();
                    let service_name = info
                        .get_fullname()
                        .trim_end_matches("._rpc._tcp.local.")
                        .to_string();
                    let state = app.state::<AppState>();
                    update_workers(&state, |workers| {
                        if let Some(worker) = workers
                            .iter_mut()
                            .find(|worker| worker.host == host && worker.port == port)
                        {
                            worker.status = WorkerStatus::Online;
                            worker.last_seen = Some(chrono::Utc::now().to_rfc3339());
                            return;
                        }
                        workers.push(WorkerInfo {
                            id: stable_discovered_worker_id(&host, port, &service_name),
                            host,
                            port,
                            name: service_name,
                            devices: Vec::new(),
                            status: WorkerStatus::Online,
                            last_seen: Some(chrono::Utc::now().to_rfc3339()),
                            auto_discovered: true,
                        });
                    })?;
                }
            }
        }
    }

    let _ = mdns.shutdown();
    Ok(())
}

// IPC compatibility boundary: legacy command internals keep their existing error flow,
// while every registered command serializes a stable AppError object.
#[allow(dead_code, unused_imports, unused_mut)] // Tauri references adapters through generated macros.
pub mod ipc {
    use super::*;

    #[tauri::command]
    pub async fn start_mdns_discovery(app: tauri::AppHandle) -> crate::error::AppResult<String> {
        super::start_mdns_discovery(app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn stop_mdns_discovery() -> crate::error::AppResult<String> {
        super::stop_mdns_discovery()
            .await
            .map_err(crate::error::AppError::from)
    }
}
