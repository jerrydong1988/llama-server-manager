use crate::error::{AppError, AppResult};
use crate::models::{AppState, GlobalConfig};
use crate::residency::{ResidencyInspection, ResidencyOperationKind, ResidencyPolicy};
use serde::Serialize;
use std::collections::HashSet;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidencyDrainStatus {
    pub instance_id: String,
    pub routing_drained: bool,
    pub active_requests: usize,
}

fn ensure_catalogs(global: &mut GlobalConfig) -> Result<bool, String> {
    let mut changed = crate::config_revision::ensure_current_config_revisions(global)?;
    changed |= crate::deployment::ensure_deployments(global)?;
    changed |= crate::canary::ensure_canary_catalog(global)?;
    changed |= crate::residency::ensure_residency_catalog(global)?;
    Ok(changed)
}

fn sync_drains(state: &AppState, desired: HashSet<String>) {
    let previous = {
        let mut current = state.residency_draining.lock().unwrap();
        let previous = current.clone();
        *current = desired.clone();
        previous
    };
    if let Some(runtime) = state.proxy_router_runtime.lock().unwrap().clone() {
        for instance_id in previous.union(&desired) {
            runtime.set_target_draining(instance_id, desired.contains(instance_id));
        }
    }
}

fn drain_status(state: &AppState, instance_id: &str) -> ResidencyDrainStatus {
    let routing_drained = state
        .residency_draining
        .lock()
        .unwrap()
        .contains(instance_id);
    let active_requests = state
        .proxy_router_runtime
        .lock()
        .unwrap()
        .clone()
        .map(|runtime| runtime.target_active_requests(instance_id))
        .unwrap_or(0);
    ResidencyDrainStatus {
        instance_id: instance_id.to_string(),
        routing_drained,
        active_requests,
    }
}

fn inspect_locked(global: &GlobalConfig, state: &AppState) -> AppResult<ResidencyInspection> {
    let engines = state.engines.lock().unwrap().clone();
    let running = state.running.lock().unwrap().clone();
    let worker_count = state
        .workers
        .lock()
        .unwrap()
        .iter()
        .filter(|worker| !worker.auto_discovered)
        .count();
    Ok(crate::residency::inspection(
        global,
        &engines,
        &running,
        worker_count,
    )?)
}

#[tauri::command]
pub async fn inspect_model_residency(state: State<'_, AppState>) -> AppResult<ResidencyInspection> {
    let (inspection, drains) = {
        let _guard = crate::commands::config::CONFIG_WRITE_LOCK
            .lock()
            .map_err(|_| "config persistence lock is poisoned".to_string())?;
        let config_dir = state.config_dir.lock().unwrap().clone();
        let mut global =
            crate::commands::config::load_global_config_for_update_unlocked(&config_dir)?;
        let changed = ensure_catalogs(&mut global)?;
        if changed {
            crate::commands::config::persist_global_config_unlocked(&config_dir, &global)?;
        }
        let inspection = inspect_locked(&global, state.inner())?;
        let drains = crate::residency::draining_instance_ids(&global);
        (inspection, drains)
    };
    sync_drains(state.inner(), drains);
    Ok(inspection)
}

#[tauri::command]
pub async fn save_model_residency_policy(
    policy: ResidencyPolicy,
    state: State<'_, AppState>,
) -> AppResult<ResidencyInspection> {
    let (inspection, drains) = {
        let _guard = crate::commands::config::CONFIG_WRITE_LOCK
            .lock()
            .map_err(|_| "config persistence lock is poisoned".to_string())?;
        let config_dir = state.config_dir.lock().unwrap().clone();
        let mut global =
            crate::commands::config::load_global_config_for_update_unlocked(&config_dir)?;
        ensure_catalogs(&mut global)?;
        crate::residency::set_policy(&mut global, policy)?;
        crate::commands::config::persist_global_config_unlocked(&config_dir, &global)?;
        let inspection = inspect_locked(&global, state.inner())?;
        let drains = crate::residency::draining_instance_ids(&global);
        (inspection, drains)
    };
    sync_drains(state.inner(), drains);
    Ok(inspection)
}

#[tauri::command]
pub async fn begin_model_residency_drain(
    instance_id: String,
    revision_id: String,
    plan_id: String,
    state: State<'_, AppState>,
) -> AppResult<ResidencyDrainStatus> {
    let drains = {
        let _guard = crate::commands::config::CONFIG_WRITE_LOCK
            .lock()
            .map_err(|_| "config persistence lock is poisoned".to_string())?;
        let config_dir = state.config_dir.lock().unwrap().clone();
        let mut global =
            crate::commands::config::load_global_config_for_update_unlocked(&config_dir)?;
        ensure_catalogs(&mut global)?;
        let inspection = inspect_locked(&global, state.inner())?;
        if inspection.plan.plan_id != plan_id {
            return Err(AppError::new(
                "RESIDENCY_PLAN_CONFLICT",
                "residency plan changed; inspect and confirm the new plan before draining",
                true,
            ));
        }
        let operation = inspection
            .plan
            .operations
            .iter()
            .find(|operation| {
                operation.kind == ResidencyOperationKind::Drain
                    && operation.instance_id == instance_id
                    && operation.revision_id == revision_id
            })
            .cloned()
            .ok_or_else(|| {
                "requested drain is not present in the current residency plan".to_string()
            })?;
        crate::residency::begin_drain(&mut global, &operation, &plan_id)?;
        crate::commands::config::persist_global_config_unlocked(&config_dir, &global)?;
        crate::residency::draining_instance_ids(&global)
    };
    sync_drains(state.inner(), drains);
    Ok(drain_status(state.inner(), &instance_id))
}

#[tauri::command]
pub async fn get_model_residency_drain_status(
    instance_id: String,
    state: State<'_, AppState>,
) -> AppResult<ResidencyDrainStatus> {
    Ok(drain_status(state.inner(), &instance_id))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn complete_model_residency_operation(
    action: String,
    instance_id: String,
    deployment_id: String,
    revision_id: String,
    plan_id: String,
    success: bool,
    error: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<ResidencyInspection> {
    let (inspection, drains) = {
        let _guard = crate::commands::config::CONFIG_WRITE_LOCK
            .lock()
            .map_err(|_| "config persistence lock is poisoned".to_string())?;
        let config_dir = state.config_dir.lock().unwrap().clone();
        let mut global =
            crate::commands::config::load_global_config_for_update_unlocked(&config_dir)?;
        ensure_catalogs(&mut global)?;
        if success {
            let running = state.running.lock().unwrap();
            match action.as_str() {
                "warm"
                    if !running.get(&instance_id).is_some_and(|item| {
                        item.deployment_id == deployment_id
                            && item.deployment_revision_id == revision_id
                    }) =>
                {
                    return Err(AppError::new(
                        "RESIDENCY_WARM_NOT_VERIFIED",
                        "warm completion requires the exact deployment revision to be running",
                        false,
                    ));
                }
                "evict" if running.contains_key(&instance_id) => {
                    return Err(AppError::new(
                        "RESIDENCY_EVICTION_NOT_VERIFIED",
                        "eviction completion requires the instance to be stopped",
                        false,
                    ));
                }
                _ => {}
            }
        }
        crate::residency::finish_operation(
            &mut global,
            &action,
            &instance_id,
            &deployment_id,
            &revision_id,
            &plan_id,
            success,
            error,
        )?;
        crate::commands::config::persist_global_config_unlocked(&config_dir, &global)?;
        let inspection = inspect_locked(&global, state.inner())?;
        let drains = crate::residency::draining_instance_ids(&global);
        (inspection, drains)
    };
    sync_drains(state.inner(), drains);
    Ok(inspection)
}
