//! Journal maintenance shares the task transition lock. Commit to disk before
//! replacing memory so a failed cleanup never hides still-persisted records.
use super::{persist_tasks, wire::Failure, Task};
use serde_json::{json, Value};
use std::path::Path;

pub(super) const FINISHED: [&str; 4] = ["succeeded", "failed", "cancelled", "interrupted"];

pub(super) enum Selection<'a> {
    One(&'a str),
    Finished(Option<&'a [String]>),
}

pub(super) fn remove(
    directory: &Path,
    tasks: &mut Vec<Task>,
    selection: Selection<'_>,
) -> Result<Value, Failure> {
    match &selection {
        Selection::One(id) => {
            let task = tasks.iter().find(|task| task.id == *id)
                .ok_or_else(|| Failure::new("NOT_FOUND", "task not found"))?;
            if !FINISHED.contains(&task.status.as_str()) {
                return Err(Failure::new("TASK_ACTIVE", "only finished task records can be deleted; queued and running tasks are protected"));
            }
        }
        Selection::Finished(Some(statuses)) => {
            if statuses.is_empty() || statuses.len() > FINISHED.len()
                || statuses.iter().collect::<std::collections::HashSet<_>>().len() != statuses.len()
                || statuses.iter().any(|status| !FINISHED.contains(&status.as_str())) {
                return Err(Failure::new("VALIDATION", "statuses must be a non-empty subset of succeeded, failed, cancelled, interrupted"));
            }
        }
        Selection::Finished(None) => {}
    }
    let selected = |task: &Task| match &selection {
        Selection::One(id) => task.id == *id,
        Selection::Finished(statuses) => FINISHED.contains(&task.status.as_str())
            && statuses.map_or(true, |statuses| statuses.contains(&task.status)),
    };
    let deleted_ids: Vec<_> = tasks.iter().filter(|task| selected(task)).map(|task| task.id.clone()).collect();
    if !deleted_ids.is_empty() {
        let retained: Vec<_> = tasks.iter().filter(|task| !selected(task)).cloned().collect();
        persist_tasks(directory, &retained)?;
        *tasks = retained;
    }
    Ok(json!({"deleted_ids":deleted_ids,"deleted_count":deleted_ids.len(),"retained_count":tasks.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("lsm-task-history-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path { &self.0 }
    }
    impl Drop for Directory {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    fn fixture() -> Vec<Task> {
        ["queued", "running", "succeeded", "failed", "cancelled", "interrupted", "future-state"]
            .into_iter().map(|status| Task {
                id: status.into(), operation: "models.refresh".into(),
                request_digest: "digest".into(), idempotency_key: Some(format!("key-{status}")),
                status: status.into(), created_at: 1, updated_at: 2,
                result: Some(json!({"preserved":true})), error: None,
            }).collect()
    }

    #[test]
    fn single_delete_protects_active_and_unknown_states_and_preserves_other_records() {
        let directory = Directory::new();
        let mut tasks = fixture();
        persist_tasks(directory.path(), &tasks).unwrap();
        for id in ["queued", "running", "future-state"] {
            assert_eq!(remove(directory.path(), &mut tasks, Selection::One(id)).unwrap_err().code, "TASK_ACTIVE");
        }
        assert_eq!(remove(directory.path(), &mut tasks, Selection::One("missing")).unwrap_err().code, "NOT_FOUND");
        let before = serde_json::to_value(&tasks).unwrap();
        for id in FINISHED {
            let result = remove(directory.path(), &mut tasks, Selection::One(id)).unwrap();
            assert_eq!(result["deleted_ids"], json!([id]));
            assert_eq!(result["deleted_count"], 1);
        }
        let disk: Value = serde_json::from_slice(&std::fs::read(directory.path().join("tasks.json")).unwrap()).unwrap();
        assert_eq!(disk, serde_json::to_value(&tasks).unwrap());
        assert_eq!(disk, json!([before[0], before[1], before[6]]));
    }

    #[test]
    fn bulk_cleanup_validates_filters_and_is_a_noop_when_empty() {
        let directory = Directory::new();
        let mut tasks = fixture();
        for statuses in [vec![], vec!["running".into()], vec!["failed".into(), "queued".into()], vec!["unknown".into()], vec!["failed".into(), "failed".into()]] {
            assert_eq!(remove(directory.path(), &mut tasks, Selection::Finished(Some(&statuses))).unwrap_err().code, "VALIDATION");
            assert_eq!(tasks.len(), 7);
        }
        let result = remove(directory.path(), &mut tasks, Selection::Finished(Some(&["failed".into(), "interrupted".into()]))).unwrap();
        assert_eq!(result["deleted_ids"], json!(["failed", "interrupted"]));
        assert_eq!(remove(directory.path(), &mut tasks, Selection::Finished(None)).unwrap()["deleted_count"], 2);
        assert_eq!(remove(directory.path(), &mut tasks, Selection::Finished(None)).unwrap()["deleted_count"], 0);
        assert_eq!(tasks.iter().map(|task| task.id.as_str()).collect::<Vec<_>>(), ["queued", "running", "future-state"]);
    }

    #[test]
    fn failed_persistence_keeps_the_complete_journal_in_memory() {
        let directory = Directory::new();
        // A directory in place of the journal makes the atomic file commit fail.
        std::fs::create_dir(directory.path().join("tasks.json")).unwrap();
        let mut tasks = fixture();
        let before = serde_json::to_value(&tasks).unwrap();
        assert!(remove(directory.path(), &mut tasks, Selection::Finished(None)).is_err());
        assert_eq!(serde_json::to_value(tasks).unwrap(), before);
    }
}
