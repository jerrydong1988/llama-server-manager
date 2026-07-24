use crate::commands::cluster::{
    stable_discovered_worker_id, update_workers, MAX_AUTO_DISCOVERED_WORKERS,
};
use crate::models::{AppState, WorkerInfo, WorkerStatus};
use tauri::Manager;
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;

struct DiscoveryTask {
    cancel: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

static DISCOVERY_TASK: Mutex<Option<DiscoveryTask>> = Mutex::const_new(None);

fn service_name_from_fullname(fullname: &str) -> String {
    fullname.trim_end_matches("._rpc._tcp.local.").to_string()
}

fn remove_discovered_service(workers: &mut Vec<WorkerInfo>, service_name: &str) -> bool {
    let previous = workers.len();
    workers.retain(|worker| !(worker.auto_discovered && worker.name == service_name));
    workers.len() != previous
}

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
                match event {
                    Ok(Ok(mdns_sd::ServiceEvent::ServiceResolved(info))) => {
                        let host = info
                            .get_addresses()
                            .iter()
                            .find(|a| a.is_ipv4())
                            .map(|a| a.to_string())
                            .unwrap_or_else(|| info.get_hostname().to_string());
                        let port = info.get_port();
                        let service_name = service_name_from_fullname(info.get_fullname());
                        if port == 0 || host.len() > 255 || service_name.len() > 255 {
                            continue;
                        }
                        let state = app.state::<AppState>();
                        update_workers(&state, |workers| {
                            if let Some(worker) = workers
                                .iter_mut()
                                .find(|worker| worker.host == host && worker.port == port)
                            {
                                if worker.auto_discovered {
                                    worker.status = WorkerStatus::Unknown;
                                }
                                worker.last_seen = Some(chrono::Utc::now().to_rfc3339());
                                return;
                            }
                            if workers
                                .iter()
                                .filter(|worker| worker.auto_discovered)
                                .count()
                                >= MAX_AUTO_DISCOVERED_WORKERS
                            {
                                return;
                            }
                            workers.push(WorkerInfo {
                                id: stable_discovered_worker_id(&host, port, &service_name),
                                host,
                                port,
                                name: service_name,
                                devices: Vec::new(),
                                status: WorkerStatus::Unknown,
                                last_seen: Some(chrono::Utc::now().to_rfc3339()),
                                auto_discovered: true,
                            });
                        })?;
                    }
                    Ok(Ok(mdns_sd::ServiceEvent::ServiceRemoved(_, fullname))) => {
                        let service_name = service_name_from_fullname(&fullname);
                        let state = app.state::<AppState>();
                        let should_update = state.workers.lock().is_ok_and(|workers| {
                            workers.iter().any(|worker| {
                                worker.auto_discovered && worker.name == service_name
                            })
                        });
                        if should_update {
                            update_workers(&state, |workers| {
                                remove_discovered_service(workers, &service_name);
                            })?;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let _ = mdns.shutdown();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_removal_fullname_maps_to_the_persisted_worker_name() {
        assert_eq!(
            service_name_from_fullname("rpc-worker._rpc._tcp.local."),
            "rpc-worker"
        );

        let discovered = WorkerInfo {
            id: "auto".into(),
            host: "127.0.0.1".into(),
            port: 50052,
            name: "rpc-worker".into(),
            devices: Vec::new(),
            status: WorkerStatus::Online,
            last_seen: None,
            auto_discovered: true,
        };
        let mut manual = discovered.clone();
        manual.id = "manual".into();
        manual.auto_discovered = false;
        let mut workers = vec![discovered, manual];
        assert!(remove_discovered_service(&mut workers, "rpc-worker"));
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0].id, "manual");
    }
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
