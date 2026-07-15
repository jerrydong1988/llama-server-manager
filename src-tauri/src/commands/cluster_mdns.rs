use crate::commands::cluster::{load_workers, save_workers};
use crate::models::{WorkerInfo, WorkerStatus};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;

struct DiscoveryTask {
    cancel: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

static DISCOVERY_TASK: Mutex<Option<DiscoveryTask>> = Mutex::const_new(None);

#[tauri::command]
pub async fn start_mdns_discovery() -> Result<String, String> {
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
        if let Err(e) = run_mdns_discovery(cancel_receiver).await {
            eprintln!("mDNS discovery error: {}", e);
        }
    });
    *task = Some(DiscoveryTask { cancel, handle });

    Ok("started".into())
}

#[tauri::command]
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

async fn run_mdns_discovery(mut cancel: oneshot::Receiver<()>) -> Result<(), String> {
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
        }
    }

    let _ = mdns.shutdown();
    Ok(())
}
