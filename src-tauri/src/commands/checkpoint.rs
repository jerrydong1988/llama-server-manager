use crate::checkpoint::{CheckpointReasonCode, CheckpointStatus, CheckpointStoreError};
use crate::error::{AppError, AppResult};
use crate::models::AppState;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::Emitter;

struct ClearReservation<'a> {
    instance_id: String,
    starting: &'a Mutex<std::collections::HashSet<String>>,
}

impl Drop for ClearReservation<'_> {
    fn drop(&mut self) {
        self.starting.lock().unwrap().remove(&self.instance_id);
    }
}

fn reserve_checkpoint_clear<'a>(
    state: &'a AppState,
    instance_id: &str,
) -> AppResult<ClearReservation<'a>> {
    crate::commands::server::validate_instance_id(instance_id)?;
    let running = state.running.lock().unwrap();
    let mut starting = state.starting.lock().unwrap();
    if running.contains_key(instance_id) || !starting.insert(instance_id.to_string()) {
        return Err(AppError::new(
            "CHECKPOINT_CLEAR_CONFLICT",
            "checkpoint cannot be cleared while the instance is running or starting",
            false,
        ));
    }
    Ok(ClearReservation {
        instance_id: instance_id.to_string(),
        starting: &state.starting,
    })
}

fn checkpoint_store_error(error: CheckpointStoreError, instance_id: &str) -> AppError {
    let (code, retryable) = match error.reason_code {
        CheckpointReasonCode::ClearWhileRunning => ("CHECKPOINT_CLEAR_CONFLICT", false),
        CheckpointReasonCode::IoError => ("CHECKPOINT_CLEAR_IO", true),
        CheckpointReasonCode::ManifestInvalid => ("CHECKPOINT_CLEAR_REJECTED", false),
        _ => ("CHECKPOINT_CLEAR_FAILED", false),
    };
    AppError::new(code, error.to_string(), retryable).with_context("instanceId", instance_id)
}

fn runtime_checkpoint_error(code: &'static str, message: String, instance_id: &str) -> AppError {
    let conflict = message
        .to_ascii_lowercase()
        .contains("cannot be cleared while");
    AppError::new(
        if conflict {
            "CHECKPOINT_CLEAR_CONFLICT"
        } else {
            code
        },
        message,
        !conflict,
    )
    .with_context("instanceId", instance_id)
}

#[tauri::command]
pub async fn list_checkpoint_statuses(
    state: tauri::State<'_, AppState>,
) -> AppResult<HashMap<String, CheckpointStatus>> {
    let mut statuses = state.checkpoint_coordinator.statuses();
    if crate::runtime_service::manages_instances() {
        statuses.extend(
            crate::runtime_service::runtime_status()
                .await
                .map_err(|message| AppError::new("CHECKPOINT_STATUS_UNAVAILABLE", message, true))?
                .checkpoints,
        );
    }
    Ok(statuses)
}

#[tauri::command]
pub async fn get_checkpoint_status(
    instance_id: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<Option<CheckpointStatus>> {
    crate::commands::server::validate_instance_id(&instance_id)?;
    Ok(list_checkpoint_statuses(state).await?.remove(&instance_id))
}

#[tauri::command]
pub async fn clear_checkpoint(
    instance_id: String,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> AppResult<CheckpointStatus> {
    let status = if crate::runtime_service::manages_instances() {
        crate::runtime_service::clear_checkpoint(instance_id.clone())
            .await
            .map_err(|message| {
                runtime_checkpoint_error("CHECKPOINT_RUNTIME_UNAVAILABLE", message, &instance_id)
            })?
    } else {
        let _reservation = reserve_checkpoint_clear(state.inner(), &instance_id)?;
        state
            .checkpoint_coordinator
            .clear_instance(&instance_id)
            .map_err(|error| checkpoint_store_error(error, &instance_id))?
    };
    let _ = app.emit("checkpoint-status", &status);
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_errors_are_structured_and_do_not_expose_private_state() {
        let conflict = checkpoint_store_error(
            CheckpointStoreError::new(
                CheckpointReasonCode::ClearWhileRunning,
                "checkpoint cannot be cleared while the instance is running",
            ),
            "instance-1",
        );
        assert_eq!(conflict.code, "CHECKPOINT_CLEAR_CONFLICT");
        assert!(!conflict.retryable);
        assert_eq!(conflict.context["instanceId"], "instance-1");
        assert!(!conflict.message.contains(['/', '\\']));
        assert!(!conflict.message.contains("prompt"));

        let unavailable = runtime_checkpoint_error(
            "CHECKPOINT_RUNTIME_UNAVAILABLE",
            "runtime endpoint unavailable".into(),
            "instance-1",
        );
        assert_eq!(unavailable.code, "CHECKPOINT_RUNTIME_UNAVAILABLE");
        assert!(unavailable.retryable);
    }
}
