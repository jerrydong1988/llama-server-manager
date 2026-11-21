//! Compare a daemon snapshot with the desktop's projection before publishing
//! any file, monitoring, log-stream or WebView changes.
use crate::models::{InstanceProcess, RunningInstance};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

/// Tickets are local to the desktop session. The daemon's configuration revision
/// does not order two status replies about different process lifetimes.
#[derive(Default)]
pub(super) struct SnapshotOrder {
    issued: AtomicU64,
    published: Mutex<u64>,
}

impl SnapshotOrder {
    pub fn begin(&self) -> u64 {
        self.issued.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn accept(&self, ticket: u64) -> Option<MutexGuard<'_, u64>> {
        let mut latest = self.published.lock().unwrap();
        if ticket <= *latest {
            return None;
        }
        *latest = ticket;
        Some(latest)
    }

    pub fn after_mutation(&self) {
        let mut latest = self.published.lock().unwrap();
        *latest = self.begin();
    }
}

#[derive(Default)]
pub(super) struct InstanceChanges<'a> {
    pub stopped: Vec<(String, InstanceProcess)>,
    relinquished: Vec<String>,
    replace: Vec<(&'a String, &'a RunningInstance)>,
    pub reconnect: Vec<(&'a String, &'a RunningInstance)>,
}

impl<'a> InstanceChanges<'a> {
    pub fn between(
        local: &HashMap<String, RunningInstance>,
        known: &HashMap<String, InstanceProcess>,
        reported: &'a HashMap<String, RunningInstance>,
    ) -> Self {
        let mut changes = Self::default();
        for (id, owner) in known {
            if let Some(current) = local.get(id).filter(|_| !reported.contains_key(id)) {
                if owner.matches(current) {
                    changes.stopped.push((id.clone(), owner.clone()));
                } else {
                    changes.relinquished.push(id.clone());
                }
            }
        }
        for (id, instance) in reported {
            let owner = instance.process();
            let changed = local.get(id).is_none_or(|current| !owner.matches(current));
            if changed {
                changes.replace.push((id, instance));
            }
            if changed || known.get(id) != Some(&owner) {
                changes.reconnect.push((id, instance));
            }
        }
        changes
    }

    pub fn apply(
        &self,
        local: &mut HashMap<String, RunningInstance>,
        known: &mut HashMap<String, InstanceProcess>,
    ) {
        for (id, _) in &self.stopped {
            local.remove(id);
        }
        for &(id, record) in &self.replace {
            local.insert(id.clone(), record.clone());
        }
        for id in &self.relinquished {
            known.remove(id);
        }
        for &(id, instance) in &self.reconnect {
            known.insert(id.clone(), instance.process());
        }
    }

    pub fn running_changed(&self) -> bool {
        !self.replace.is_empty() || !self.stopped.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(id: &str, pid: u32) -> RunningInstance {
        serde_json::from_value(serde_json::json!({
            "instance_id":id,"pid":pid,"host":"127.0.0.1","port":8080,
            "start_time":100,"executable_path":"engine/llama-server"
        }))
        .unwrap()
    }

    #[test]
    fn late_replies_cannot_reverse_a_newer_snapshot_or_completed_start_stop() {
        let order = SnapshotOrder::default();
        let delayed = order.begin();
        let newer = order.begin();
        assert!(order.accept(newer).is_some());
        assert!(order.accept(delayed).is_none());
        let in_flight = order.begin();
        order.after_mutation();
        assert!(order.accept(in_flight).is_none());
        assert!(order.accept(order.begin()).is_some());
    }

    #[test]
    fn a_reused_pid_is_a_new_process_and_an_old_daemon_id_cannot_stop_a_gui_process() {
        let mut local = HashMap::new();
        let mut known = Default::default();
        let mut reported = HashMap::from([("worker".into(), process("worker", 42))]);
        InstanceChanges::between(&local, &known, &reported).apply(&mut local, &mut known);
        reported.get_mut("worker").unwrap().start_time += 1;
        let reused = InstanceChanges::between(&local, &known, &reported);
        assert!(
            reused.running_changed(),
            "a recycled PID belongs to a different process lifetime"
        );
        assert_eq!(reused.reconnect.len(), 1);
        reused.apply(&mut local, &mut known);
        reported.clear();
        local.insert("worker".into(), process("worker", 90));
        let gui_owned = InstanceChanges::between(&local, &known, &reported);
        assert!(
            gui_owned.stopped.is_empty(),
            "the old daemon owner cannot retire a GUI replacement"
        );
        gui_owned.apply(&mut local, &mut known);
        assert_eq!(local["worker"].pid, 90);
        assert!(!known.contains_key("worker"));
    }

    #[test]
    fn only_process_transitions_request_a_write_or_a_log_attachment() {
        let mut known = HashMap::new();
        let mut local = HashMap::from([("foreground".into(), process("foreground", 30))]);
        let mut reported = HashMap::from([("worker".into(), process("worker", 42))]);
        let changes = InstanceChanges::between(&local, &known, &reported);
        assert!(changes.running_changed());
        assert_eq!(changes.reconnect.len(), 1);
        changes.apply(&mut local, &mut known);
        let unchanged = InstanceChanges::between(&local, &known, &reported);
        assert!(!unchanged.running_changed());
        assert!(unchanged.reconnect.is_empty());
        reported.get_mut("worker").unwrap().pid = 43;
        let replacement = InstanceChanges::between(&local, &known, &reported);
        assert_eq!(replacement.reconnect.len(), 1);
        replacement.apply(&mut local, &mut known);
        reported.clear();
        let stopped = InstanceChanges::between(&local, &known, &reported);
        assert_eq!(stopped.stopped.len(), 1);
        assert_eq!(stopped.stopped[0].0, "worker");
        stopped.apply(&mut local, &mut known);
        assert!(local.contains_key("foreground"));
        // A late subscriber must still be able to learn that the worker stopped.
        assert!(known.contains_key("worker"));
        assert!(!InstanceChanges::between(&local, &known, &reported).running_changed());
    }
}
