//! Pre-execution journal changes must commit before publishing memory or
//! allowing side effects. Terminal outcomes are handled separately by Host.
use super::{persist_tasks, wire::Failure, Task};
use std::path::Path;

pub(super) fn enqueue(directory: &Path, tasks: &mut Vec<Task>, task: Task) -> Result<(), Failure> {
    let mut next = tasks.clone();
    while next.len() >= 200 {
        if let Some(index) = next.iter().position(|task| task.status != "queued" && task.status != "running") {
            next.remove(index);
        } else {
            break;
        }
    }
    next.push(task);
    persist_tasks(directory, &next)?;
    *tasks = next;
    Ok(())
}

pub(super) fn begin(directory: &Path, tasks: &mut [Task], id: &str) -> Result<bool, Failure> {
    let index = tasks.iter().position(|task| task.id == id)
        .ok_or_else(|| Failure::new("NOT_FOUND", "task not found"))?;
    if tasks[index].status != "queued" { return Ok(false); }
    let previous = tasks[index].clone();
    tasks[index].status = "running".into();
    tasks[index].updated_at = chrono::Utc::now().timestamp_millis();
    if let Err(error) = persist_tasks(directory, tasks) {
        tasks[index] = previous;
        return Err(error);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("lsm-journal-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    fn task(id: usize, status: &str) -> Task {
        Task { id: id.to_string(), operation: "models.refresh".into(),
            request_digest: format!("digest-{id}"), idempotency_key: Some(format!("key-{id}")),
            status: status.into(), created_at: 1, updated_at: 2,
            result: Some(json!({"retained":id})), error: None }
    }

    #[test]
    fn rejected_enqueue_keeps_history_idempotency_and_active_records() {
        let directory = Directory::new();
        let mut tasks: Vec<_> = (0..200).map(|id| task(id, if id == 0 { "queued" } else { "succeeded" })).collect();
        let before = serde_json::to_value(&tasks).unwrap();
        std::fs::create_dir(directory.0.join("tasks.json")).unwrap();
        assert!(enqueue(&directory.0, &mut tasks, task(200, "queued")).is_err());
        assert_eq!(serde_json::to_value(&tasks).unwrap(), before);
        std::fs::remove_dir(directory.0.join("tasks.json")).unwrap();
        enqueue(&directory.0, &mut tasks, task(200, "queued")).unwrap();
        assert_eq!(tasks.len(), 200);
        assert_eq!(tasks[0].id, "0");
        assert_eq!(tasks[1].id, "2");
        let disk: serde_json::Value = serde_json::from_slice(&std::fs::read(directory.0.join("tasks.json")).unwrap()).unwrap();
        assert_eq!(disk, serde_json::to_value(&tasks).unwrap());
    }

    #[test]
    fn failed_begin_restores_queued_record_and_never_restarts_terminal_tasks() {
        let directory = Directory::new();
        let mut tasks = vec![task(0, "queued")];
        let before = serde_json::to_value(&tasks).unwrap();
        std::fs::create_dir(directory.0.join("tasks.json")).unwrap();
        assert!(begin(&directory.0, &mut tasks, "0").is_err());
        assert_eq!(serde_json::to_value(&tasks).unwrap(), before);
        std::fs::remove_dir(directory.0.join("tasks.json")).unwrap();
        assert!(begin(&directory.0, &mut tasks, "0").unwrap());
        for status in ["running", "cancelled", "failed", "succeeded", "interrupted"] {
            tasks[0].status = status.into();
            assert!(!begin(&directory.0, &mut tasks, "0").unwrap());
            assert_eq!(tasks[0].status, status);
        }
    }
}
