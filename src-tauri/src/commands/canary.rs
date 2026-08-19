use crate::canary::{CanaryRolloutInspection, CanaryTargetHealth};
use crate::commands::config::{
    load_global_config_for_update_unlocked, persist_global_config_unlocked, CONFIG_WRITE_LOCK,
};
use crate::models::{AppState, GlobalConfig, RunningInstance};
use std::collections::HashMap;

struct RuntimeContext {
    running: HashMap<String, RunningInstance>,
    proxy_running: bool,
    health: HashMap<String, CanaryTargetHealth>,
}

fn health_ready(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "ok" | "ready" | "healthy" | "running"
    )
}

async fn runtime_context(state: &AppState) -> Result<RuntimeContext, String> {
    if crate::runtime_service::manages_instances() {
        let status = crate::runtime_service::ensure_runtime_service().await?;
        let health = status
            .running
            .keys()
            .map(|instance_id| {
                let value = status
                    .health
                    .get(instance_id)
                    .cloned()
                    .unwrap_or_else(|| "pending".into());
                (
                    instance_id.clone(),
                    CanaryTargetHealth {
                        instance_id: instance_id.clone(),
                        ready: health_ready(&value),
                        status: value,
                    },
                )
            })
            .collect();
        return Ok(RuntimeContext {
            running: status.running,
            proxy_running: status.proxy.running,
            health,
        });
    }

    let running = state.running.lock().unwrap().clone();
    let proxy_running = state.proxy_shutdown.lock().unwrap().is_some();
    let router_runtime = state.proxy_router_runtime.lock().unwrap().clone();
    let health = running
        .keys()
        .map(|instance_id| {
            let (status, ready) = router_runtime
                .as_ref()
                .map(|runtime| runtime.target_snapshot(instance_id))
                .map(|snapshot| {
                    (
                        snapshot.status,
                        snapshot.ready && snapshot.last_checked_at_ms > 0,
                    )
                })
                .unwrap_or_else(|| ("pending".into(), false));
            (
                instance_id.clone(),
                CanaryTargetHealth {
                    instance_id: instance_id.clone(),
                    status,
                    ready,
                },
            )
        })
        .collect();
    Ok(RuntimeContext {
        running,
        proxy_running,
        health,
    })
}

fn load_catalog(state: &AppState) -> Result<GlobalConfig, String> {
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap();
    let config_dir = state.config_dir.lock().unwrap().clone();
    let mut global = load_global_config_for_update_unlocked(&config_dir)?;
    let revision_changed = crate::config_revision::ensure_current_config_revisions(&mut global)?;
    let deployment_changed = crate::deployment::ensure_deployments(&mut global)?;
    let canary_changed = crate::canary::ensure_canary_catalog(&mut global)?;
    if revision_changed || deployment_changed || canary_changed {
        persist_global_config_unlocked(&config_dir, &global)?;
    }
    Ok(global)
}

fn inspect_id(
    global: &GlobalConfig,
    context: &RuntimeContext,
    rollout_id: &str,
) -> Result<CanaryRolloutInspection, String> {
    let record = global
        .canary_rollouts
        .iter()
        .find(|record| record.id == rollout_id)
        .ok_or_else(|| format!("canary rollout {rollout_id} does not exist"))?;
    Ok(crate::canary::inspect_rollout(
        record,
        global,
        &context.running,
        context.proxy_running,
        &context.health,
    ))
}

async fn inspect_after(
    state: &AppState,
    rollout_id: &str,
) -> Result<CanaryRolloutInspection, String> {
    let context = runtime_context(state).await?;
    let global = load_catalog(state)?;
    inspect_id(&global, &context, rollout_id)
}

fn restore_persisted_catalog(
    state: &AppState,
    previous_proxy: crate::models::ProxyConfig,
    previous_schema: u32,
    previous_rollouts: Vec<crate::canary::CanaryRolloutRecord>,
) -> Result<(), String> {
    let _guard = CONFIG_WRITE_LOCK.lock().unwrap();
    let config_dir = state.config_dir.lock().unwrap().clone();
    let mut global = load_global_config_for_update_unlocked(&config_dir)?;
    global.proxy_config = previous_proxy;
    global.canary_schema_version = previous_schema;
    global.canary_rollouts = previous_rollouts;
    persist_global_config_unlocked(&config_dir, &global).map(|_| ())
}

async fn mutate_rollout<F>(state: &AppState, mutation: F) -> Result<String, String>
where
    F: FnOnce(&mut GlobalConfig, &RuntimeContext, i64) -> Result<String, String>,
{
    let _transition = state.proxy_lifecycle_lock.lock().await;
    let context = runtime_context(state).await?;
    let now_ms = crate::commands::telemetry::current_time_ms();
    let config_dir = state.config_dir.lock().unwrap().clone();
    let (rollout_id, previous_proxy, previous_schema, previous_rollouts, next_proxy) = {
        let _guard = CONFIG_WRITE_LOCK.lock().unwrap();
        let mut global = load_global_config_for_update_unlocked(&config_dir)?;
        crate::config_revision::ensure_current_config_revisions(&mut global)?;
        crate::deployment::ensure_deployments(&mut global)?;
        crate::canary::ensure_canary_catalog(&mut global)?;

        let normalized = crate::commands::proxy::normalize_and_validate_proxy_config(
            global.proxy_config.clone(),
            &global.instances,
        )?;
        if normalized != global.proxy_config {
            return Err(
                "save and validate the current proxy configuration before starting a canary rollout"
                    .into(),
            );
        }

        let previous_proxy = global.proxy_config.clone();
        let previous_schema = global.canary_schema_version;
        let previous_rollouts = global.canary_rollouts.clone();
        let rollout_id = mutation(&mut global, &context, now_ms)?;
        let active_instances = crate::canary::active_instance_ids(&global);
        if let Some(instance_id) = state
            .starting
            .lock()
            .unwrap()
            .iter()
            .find(|instance_id| active_instances.contains(*instance_id))
            .cloned()
        {
            return Err(format!(
                "instance {instance_id} is starting; wait for it to settle before changing canary routing"
            ));
        }
        let validated = crate::commands::proxy::normalize_and_validate_proxy_config(
            global.proxy_config.clone(),
            &global.instances,
        )?;
        if validated.canary_routes != global.proxy_config.canary_routes {
            return Err("canary routing overlay failed validation".into());
        }
        global.proxy_config = validated;
        persist_global_config_unlocked(&config_dir, &global)?;
        (
            rollout_id,
            previous_proxy,
            previous_schema,
            previous_rollouts,
            global.proxy_config.clone(),
        )
    };

    *state.proxy_config.lock().unwrap() = next_proxy;
    if !crate::runtime_service::manages_instances() {
        return Ok(rollout_id);
    }

    let sync_generation = crate::runtime_service::mark_config_sync_pending();
    if let Err(error) = crate::runtime_service::sync_app_config(state).await {
        *state.proxy_config.lock().unwrap() = previous_proxy.clone();
        let rollback_generation = crate::runtime_service::mark_config_sync_pending();
        let mut rollback_errors = Vec::new();
        if let Err(rollback_error) =
            restore_persisted_catalog(state, previous_proxy, previous_schema, previous_rollouts)
        {
            rollback_errors.push(rollback_error);
        }
        if let Err(rollback_error) = crate::runtime_service::sync_app_config(state).await {
            rollback_errors.push(rollback_error);
        }
        if rollback_errors.is_empty() {
            crate::runtime_service::mark_config_sync_complete(rollback_generation);
        }
        return if rollback_errors.is_empty() {
            Err(error)
        } else {
            Err(format!(
                "{error}; restoring canary state also failed: {}",
                rollback_errors.join("; ")
            ))
        };
    }
    crate::runtime_service::mark_config_sync_complete(sync_generation);
    Ok(rollout_id)
}

pub async fn list_canary_rollouts(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<CanaryRolloutInspection>, String> {
    let context = runtime_context(&state).await?;
    let global = load_catalog(&state)?;
    Ok(crate::canary::inspections(
        &global,
        &context.running,
        context.proxy_running,
        &context.health,
    ))
}

pub async fn create_canary_rollout(
    stable_instance_id: String,
    candidate_instance_id: String,
    model_alias: String,
    candidate_weight: u32,
    state: tauri::State<'_, AppState>,
) -> Result<CanaryRolloutInspection, String> {
    let rollout_id = mutate_rollout(&state, move |global, context, now_ms| {
        crate::canary::create_rollout(
            global,
            &context.running,
            context.proxy_running,
            crate::canary::CanaryRolloutCreate {
                stable_instance_id: &stable_instance_id,
                candidate_instance_id: &candidate_instance_id,
                model_alias: &model_alias,
                candidate_weight,
            },
            now_ms,
        )
    })
    .await?;
    inspect_after(&state, &rollout_id).await
}

pub async fn set_canary_weight(
    rollout_id: String,
    candidate_weight: u32,
    state: tauri::State<'_, AppState>,
) -> Result<CanaryRolloutInspection, String> {
    let id = rollout_id.clone();
    mutate_rollout(&state, move |global, context, now_ms| {
        crate::canary::set_weight(
            global,
            &context.running,
            context.proxy_running,
            &rollout_id,
            candidate_weight,
            now_ms,
        )?;
        Ok(rollout_id)
    })
    .await?;
    inspect_after(&state, &id).await
}

pub async fn promote_canary_rollout(
    rollout_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<CanaryRolloutInspection, String> {
    let id = rollout_id.clone();
    mutate_rollout(&state, move |global, context, now_ms| {
        let candidate_id = global
            .canary_rollouts
            .iter()
            .find(|record| record.id == rollout_id)
            .map(|record| record.candidate_instance_id.clone())
            .ok_or_else(|| format!("canary rollout {rollout_id} does not exist"))?;
        let candidate_ready = context
            .health
            .get(&candidate_id)
            .is_some_and(|health| health.ready);
        crate::canary::promote(
            global,
            &context.running,
            context.proxy_running,
            candidate_ready,
            &rollout_id,
            now_ms,
        )?;
        Ok(rollout_id)
    })
    .await?;
    inspect_after(&state, &id).await
}

pub async fn abort_canary_rollout(
    rollout_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<CanaryRolloutInspection, String> {
    let id = rollout_id.clone();
    mutate_rollout(&state, move |global, _, now_ms| {
        crate::canary::abort(global, &rollout_id, now_ms)?;
        Ok(rollout_id)
    })
    .await?;
    inspect_after(&state, &id).await
}

pub async fn rollback_canary_rollout(
    rollout_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<CanaryRolloutInspection, String> {
    let id = rollout_id.clone();
    mutate_rollout(&state, move |global, _, now_ms| {
        crate::canary::rollback(global, &rollout_id, now_ms)?;
        Ok(rollout_id)
    })
    .await?;
    inspect_after(&state, &id).await
}

pub async fn observe_canary_rollout(
    rollout_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<CanaryRolloutInspection, String> {
    let _transition = state.proxy_lifecycle_lock.lock().await;
    let context = runtime_context(&state).await?;
    let snapshot = load_catalog(&state)?;
    let record = snapshot
        .canary_rollouts
        .iter()
        .find(|record| record.id == rollout_id)
        .cloned()
        .ok_or_else(|| format!("canary rollout {rollout_id} does not exist"))?;
    let (stable_evidence, candidate_evidence) =
        crate::commands::telemetry::canary_request_evidence(
            record.stable_instance_id,
            record.candidate_instance_id,
            record.created_at,
        )
        .await?;

    let global = {
        let _guard = CONFIG_WRITE_LOCK.lock().unwrap();
        let config_dir = state.config_dir.lock().unwrap().clone();
        let mut global = load_global_config_for_update_unlocked(&config_dir)?;
        crate::config_revision::ensure_current_config_revisions(&mut global)?;
        crate::deployment::ensure_deployments(&mut global)?;
        crate::canary::record_observation(
            &mut global,
            &context.running,
            context.proxy_running,
            &rollout_id,
            stable_evidence,
            candidate_evidence,
            crate::commands::telemetry::current_time_ms(),
        )?;
        persist_global_config_unlocked(&config_dir, &global)?;
        global
    };
    inspect_id(&global, &context, &rollout_id)
}

#[cfg(test)]
mod tests {
    use super::health_ready;

    #[test]
    fn promotion_health_is_fail_closed() {
        for status in ["ok", "ready", "healthy", "running"] {
            assert!(health_ready(status));
        }
        for status in ["pending", "fail", "unknown", ""] {
            assert!(!health_ready(status));
        }
    }
}

// IPC compatibility boundary: command internals keep String errors for reuse in
// tests while registered commands return the application's stable error shape.
#[allow(dead_code, unused_imports)]
pub mod ipc {
    use super::*;

    #[tauri::command]
    pub async fn list_canary_rollouts(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<Vec<CanaryRolloutInspection>> {
        super::list_canary_rollouts(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn create_canary_rollout(
        stable_instance_id: String,
        candidate_instance_id: String,
        model_alias: String,
        candidate_weight: u32,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<CanaryRolloutInspection> {
        super::create_canary_rollout(
            stable_instance_id,
            candidate_instance_id,
            model_alias,
            candidate_weight,
            state,
        )
        .await
        .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn observe_canary_rollout(
        rollout_id: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<CanaryRolloutInspection> {
        super::observe_canary_rollout(rollout_id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn set_canary_weight(
        rollout_id: String,
        candidate_weight: u32,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<CanaryRolloutInspection> {
        super::set_canary_weight(rollout_id, candidate_weight, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn promote_canary_rollout(
        rollout_id: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<CanaryRolloutInspection> {
        super::promote_canary_rollout(rollout_id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn abort_canary_rollout(
        rollout_id: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<CanaryRolloutInspection> {
        super::abort_canary_rollout(rollout_id, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn rollback_canary_rollout(
        rollout_id: String,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<CanaryRolloutInspection> {
        super::rollback_canary_rollout(rollout_id, state)
            .await
            .map_err(crate::error::AppError::from)
    }
}
