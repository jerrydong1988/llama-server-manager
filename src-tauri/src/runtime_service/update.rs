//! Quiesce GUI requests before an installer replaces the runtime executable.
use super::{protocol::RuntimeCommand, protocol::RuntimeReply, transport};
use std::fs::File;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::{RwLock, RwLockReadGuard};

// Keeping the OS lock until this GUI exits prevents its bridge, or a second
// process, from starting a daemon between shutdown and installer launch.
static UPDATE_LEASE: RwLock<Option<File>> = RwLock::const_new(None);
const UPDATE_PENDING: &str = "runtime is paused for application update";

pub(super) async fn request_permit() -> Result<RwLockReadGuard<'static, Option<File>>, String> {
    let permit = UPDATE_LEASE.read().await;
    if permit.is_some() {
        return Err(UPDATE_PENDING.into());
    }
    Ok(permit)
}

pub(super) fn is_update_pause(error: &str) -> bool {
    error == UPDATE_PENDING
}

#[tauri::command]
pub async fn prepare_app_update() -> Result<(), String> {
    // Drain in-flight requests, including recovery/startup, before shutdown.
    let mut lease = tokio::time::timeout(Duration::from_secs(30), UPDATE_LEASE.write())
        .await
        .map_err(|_| "timed out waiting for runtime operations before update".to_string())?;
    if lease.is_some() {
        return Ok(());
    }
    let lock = match transport::acquire_runtime_lock()? {
        Some(lock) => lock,
        None => {
            let token = super::load_control_token()?;
            // Preserve desired workloads and routing settings for the new
            // daemon, or for recovery if the installer cannot be launched.
            let reply = tokio::time::timeout(
                Duration::from_secs(60),
                super::call_with_token(
                    token,
                    RuntimeCommand::Shutdown {
                        stop_instances: false,
                    },
                ),
            )
            .await
            .map_err(|_| "timed out stopping runtime before update".to_string())??;
            if !matches!(reply, RuntimeReply::Ack) {
                return Err("runtime returned an unexpected update shutdown response".into());
            }
            transport::wait_for_runtime_lock(Duration::from_secs(10)).await?
        }
    };
    super::RUNTIME_READY.store(false, Ordering::Release);
    *lease = Some(lock);
    Ok(())
}

#[tauri::command]
pub async fn resume_app_after_update() -> Result<(), String> {
    UPDATE_LEASE.write().await.take();
    // Wake the bridge to restore the preserved workloads and routing snapshot.
    super::mark_config_sync_pending();
    Ok(())
}

#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;
