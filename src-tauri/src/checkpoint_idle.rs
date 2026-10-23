//! Shared direct/daemon idle-save path. It never starts, stops or restores an engine.
use crate::checkpoint::{CheckpointCoordinator, LlamaSlotClient};
use crate::commands::telemetry::SlotSnapshotRecord;
use crate::models::RunningInstance;
use std::time::Duration;

pub(crate) fn poll(
    coordinator: &CheckpointCoordinator,
    running: &RunningInstance,
    slots: &[SlotSnapshotRecord],
    active_requests: impl Fn() -> usize,
) {
    let Some(config) = running.launch_config.as_ref() else {
        return;
    };
    if !config.kv_checkpoint.enabled || config.kv_checkpoint.idle_save_seconds == 0 {
        return;
    }
    let lock = crate::engine_process::termination_lock(running.pid);
    let Ok(_operation) = lock.try_lock() else {
        return;
    };
    if !crate::commands::server::running_instance_matches_live_process(running)
        || !coordinator.idle_save_due(&running.instance_id, running.pid, slots, active_requests())
    {
        return;
    }
    let id = &running.instance_id;
    let Some(before) = coordinator.status(id) else {
        return;
    };
    let Ok(Some((fingerprint, policy))) = coordinator.registered_checkpoint(id, running.pid) else {
        return;
    };
    if coordinator.begin_draining(id, running.pid).is_err() {
        return;
    }
    // Recheck proxy admission after closing the gate. Direct-port traffic is
    // outside that guarantee; the save path also checks current slot state.
    if active_requests() > 0 {
        let _ = coordinator.skip_save_busy(id, running.pid, 0);
    } else {
        match LlamaSlotClient::new(config, Duration::from_secs(30 * 60)) {
            Ok(client) => {
                let _ =
                    coordinator.save_before_stop(id, running.pid, &fingerprint, &policy, &client);
            }
            Err(error) => {
                let _ = coordinator.fail_save_setup(id, running.pid, error, 0);
            }
        }
    }
    // Saving is not evidence of a disk restore. Retain Ready vs ReadyCold.
    let _ = coordinator.resume_after_stop_failure(id, running.pid, before.phase);
}
