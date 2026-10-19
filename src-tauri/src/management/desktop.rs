//! Desktop access to the same journal and cancellation lock used by CLI/MCP.
use super::{safe_error, wire::Failure, Host};
use crate::error::{AppError, AppResult};
use serde_json::Value;
use std::sync::Arc;
use tauri::Manager;

fn host(app: &tauri::AppHandle) -> AppResult<Arc<Host>> {
    app.try_state::<Arc<Host>>()
        .map(|state| state.inner().clone())
        .ok_or_else(|| AppError::new("UNAVAILABLE", "management host is not ready", true))
}

fn desktop_error(error: Failure) -> AppError {
    let error = safe_error(error);
    AppError::new(error.code, error.message, error.retryable)
}

#[tauri::command]
pub async fn list_management_tasks(app: tauri::AppHandle) -> AppResult<Value> {
    Ok(host(&app)?.list_tasks())
}

#[tauri::command]
pub async fn get_management_task(app: tauri::AppHandle, id: String) -> AppResult<Value> {
    host(&app)?.get_task(&id).map_err(desktop_error)
}

#[tauri::command]
pub async fn cancel_management_task(app: tauri::AppHandle, id: String) -> AppResult<Value> {
    host(&app)?.cancel_task(&id).map_err(desktop_error)
}
