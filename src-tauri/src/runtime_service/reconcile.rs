//! Compare a daemon snapshot with the desktop's projection before publishing
//! any file, monitoring, log-stream or WebView changes.
use crate::models::RunningInstance;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct InstanceChanges<'a> {
    pub stopped: Vec<String>,
    replace: Vec<(&'a String, &'a RunningInstance)>,
    pub reconnect: Vec<(&'a String, &'a RunningInstance)>,
}

impl<'a> InstanceChanges<'a> {
    pub fn between(
        local: &HashMap<String, RunningInstance>,
        known: &HashSet<String>,
        reported: &'a HashMap<String, RunningInstance>,
    ) -> Self {
        let mut changes = Self::default();
        for id in known {
            if !reported.contains_key(id) && local.contains_key(id) {
                changes.stopped.push(id.clone());
            }
        }
        for (id, instance) in reported {
            let changed = local
                .get(id)
                .is_none_or(|current| current.pid != instance.pid);
            if changed {
                changes.replace.push((id, instance));
            }
            if changed || !known.contains(id) {
                changes.reconnect.push((id, instance));
            }
        }
        changes
    }

    pub fn apply(&self, local: &mut HashMap<String, RunningInstance>, known: &mut HashSet<String>) {
        for id in &self.stopped {
            local.remove(id);
        }
        for &(id, record) in &self.replace {
            local.insert(id.clone(), record.clone());
        }
        for &(id, _) in &self.reconnect {
            known.insert(id.clone());
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
    fn only_process_transitions_request_a_write_or_a_log_attachment() {
        let mut known = HashSet::new();
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
        assert_eq!(stopped.stopped, ["worker"]);
        stopped.apply(&mut local, &mut known);
        assert!(local.contains_key("foreground"));
        // A late subscriber must still be able to learn that the worker stopped.
        assert!(known.contains("worker"));
        assert!(!InstanceChanges::between(&local, &known, &reported).running_changed());
    }
}
