use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use tauri::State;

use crate::models::{AppState, WorkerInfo, WorkerOrigin, WorkerStatus};
use crate::utils;

static WORKERS_WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn workers_path() -> PathBuf {
    let config_dir = utils::get_data_dir().join("configs");
    // Legacy records are deliberately not imported: their endpoint, token,
    // and certificate fields predate the private enrollment boundary. The
    // operator must re-enroll those workers.
    config_dir.join("workers.json")
}

/// Only cryptographically enrolled Secure Worker Agents are durable workers.
/// Legacy address-only RPC workers are intentionally dropped during migration.
fn is_persistable_worker(worker: &WorkerInfo) -> bool {
    !worker.auto_discovered && worker.origin == WorkerOrigin::Agent && worker.agent.is_some()
}

pub fn load_workers() -> Vec<WorkerInfo> {
    load_workers_from(&workers_path())
}

fn load_workers_from(path: &std::path::Path) -> Vec<WorkerInfo> {
    crate::persistence::read_private_file_bounded(path, 4 * 1024 * 1024)
        .ok()
        .flatten()
        .and_then(|contents| serde_json::from_slice::<Vec<WorkerInfo>>(&contents).ok())
        .map(|workers| {
            workers
                .into_iter()
                .filter(is_persistable_worker)
                .map(|mut worker| {
                    worker.status = WorkerStatus::Unknown;
                    worker
                })
                .collect()
        })
        .unwrap_or_default()
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
        workers.retain(is_persistable_worker);
        let previous = workers.clone();
        let result = update(&mut workers);
        workers.retain(is_persistable_worker);
        (result, previous, workers.clone())
    };
    if serde_json::to_vec(&previous).ok() == serde_json::to_vec(&snapshot).ok() {
        return Ok(result);
    }
    if let Err(error) = save_workers(&snapshot) {
        if let Ok(mut workers) = state.workers.lock() {
            *workers = previous;
        }
        return Err(error);
    }
    Ok(result)
}

pub async fn remove_worker(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let worker = state
        .workers
        .lock()
        .map_err(|_| "worker state lock is poisoned".to_string())?
        .iter()
        .find(|worker| worker.id == id && is_persistable_worker(worker))
        .cloned()
        .ok_or_else(|| "Secure Worker Agent not found".to_string())?;
    if let Some(connection) = &worker.agent {
        if let Err(error) = crate::worker_agent::stop_remote_rpc(connection).await {
            eprintln!(
                "Secure Worker Agent cleanup failed for {}: {error}",
                worker.id
            );
        }
    }
    let _ = crate::worker_agent::stop_manager_bridge(&worker.id).await;
    update_workers(&state, |workers| {
        workers.retain(|candidate| candidate.id != id)
    })
}

pub async fn get_workers(state: State<'_, AppState>) -> Result<Vec<WorkerInfo>, String> {
    state
        .workers
        .lock()
        .map(|workers| {
            workers
                .iter()
                .filter(|worker| is_persistable_worker(worker))
                .cloned()
                .collect()
        })
        .map_err(|_| "worker state lock is poisoned".to_string())
}

pub async fn get_cluster_metrics(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let workers = get_workers(state).await?;
    let worker_metrics = workers
        .iter()
        .map(|worker| {
            serde_json::json!({
                "host": worker.host,
                "port": worker.port,
                "name": worker.name,
                "online": worker.status == WorkerStatus::Online,
                "devices": worker.devices.iter().map(|device| serde_json::json!({
                    "type": device.device_type,
                    "name": device.name,
                    "vram_mb": device.vram_mb,
                    "free_mb": device.free_mb,
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let online_workers = workers
        .iter()
        .filter(|worker| worker.status == WorkerStatus::Online)
        .count();
    Ok(serde_json::json!({
        "total_workers": workers.len(),
        "online_workers": online_workers,
        "worker_metrics": worker_metrics,
    }))
}

#[allow(dead_code, unused_imports, unused_mut)]
pub mod ipc {
    use super::*;

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
    ) -> crate::error::AppResult<Vec<crate::models::FrontendWorkerInfo>> {
        super::get_workers(state)
            .await
            .map(|workers| {
                workers
                    .iter()
                    .map(crate::models::FrontendWorkerInfo::from)
                    .collect()
            })
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_workers_are_not_persistable() {
        let worker = WorkerInfo {
            id: "legacy".into(),
            host: "127.0.0.1".into(),
            port: 50052,
            name: "legacy".into(),
            origin: WorkerOrigin::Manual,
            devices: Vec::new(),
            status: WorkerStatus::Unknown,
            last_seen: None,
            auto_discovered: false,
            agent: None,
        };
        assert!(!is_persistable_worker(&worker));
    }
}
