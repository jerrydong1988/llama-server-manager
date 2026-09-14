//! A held OS lock distinguishes a live writer from a crashed process, including
//! when a GUI and an independent routing service share the database.
use fs2::FileExt;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

pub(super) fn schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage_sessions (
        id TEXT PRIMARY KEY, updated INTEGER NOT NULL, state TEXT NOT NULL,
        pending INTEGER NOT NULL, last_commit INTEGER, write_delay INTEGER);",
    )
    .map_err(|e| e.to_string())
}

pub(super) struct WriterSession {
    pub id: String,
    file: File,
    path: PathBuf,
}

impl WriterSession {
    pub fn start(database: &Path) -> Result<Self, String> {
        let directory = database.with_extension("writers");
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let id = uuid::Uuid::new_v4().to_string();
        let path = directory.join(format!("{id}.lock"));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        FileExt::try_lock_exclusive(&file).map_err(|e| e.to_string())?;
        Ok(Self { id, file, path })
    }

    pub fn update(
        &self,
        conn: &Connection,
        pending: u64,
        commit: Option<i64>,
        delay: Option<i64>,
        closed: bool,
    ) -> Result<(), String> {
        conn.execute("INSERT INTO usage_sessions VALUES (?1,?2,?3,?4,?5,?6)
            ON CONFLICT(id) DO UPDATE SET updated=excluded.updated,state=excluded.state,pending=excluded.pending,
            last_commit=COALESCE(excluded.last_commit,last_commit),write_delay=COALESCE(excluded.write_delay,write_delay)",
            params![self.id, super::telemetry::current_time_ms(), if closed { "closed" } else { "active" }, pending.min(i64::MAX as u64) as i64, commit, delay]).map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl Drop for WriterSession {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageHealth {
    pub pending_records: u64,
    pub last_commit_at: Option<i64>,
    pub write_delay_ms: Option<i64>,
    pub interrupted_sessions: u64,
    pub last_interruption_at: Option<i64>,
}

pub(super) fn inspect(conn: &Connection, database: &Path) -> Result<UsageHealth, String> {
    let mut result = UsageHealth::default();
    let mut stmt = conn
        .prepare("SELECT id,updated,state,pending,last_commit,write_delay FROM usage_sessions")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let sessions = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    for (id, updated, state, pending, commit, delay) in sessions {
        if commit > result.last_commit_at {
            result.last_commit_at = commit;
            result.write_delay_ms = delay;
        }
        if state == "closed" {
            continue;
        }
        // Database text never becomes an unchecked filesystem path.
        let live = if uuid::Uuid::parse_str(&id).is_ok() {
            let path = database
                .with_extension("writers")
                .join(format!("{id}.lock"));
            match OpenOptions::new().read(true).write(true).open(path) {
                Ok(file) => match FileExt::try_lock_exclusive(&file) {
                    Ok(()) => {
                        let _ = FileExt::unlock(&file);
                        false
                    }
                    Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                        true
                    }
                    Err(e) => return Err(format!("Cannot inspect usage writer: {e}")),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(e) => return Err(format!("Cannot inspect usage writer: {e}")),
            }
        } else {
            false
        };
        if live {
            result.pending_records = result.pending_records.saturating_add(pending.max(0) as u64);
        } else {
            // The writer may have completed a clean shutdown after the first
            // read. Recheck outside that snapshot after observing lock release.
            let state: String = conn
                .query_row("SELECT state FROM usage_sessions WHERE id=?1", [&id], |r| {
                    r.get(0)
                })
                .map_err(|e| e.to_string())?;
            if state == "closed" {
                continue;
            }
            result.interrupted_sessions += 1;
            result.last_interruption_at =
                Some(result.last_interruption_at.unwrap_or(0).max(updated));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_writers_are_not_crashes_and_closed_sessions_are_not_gaps() {
        let path = std::env::temp_dir().join(format!("lsm-health-{}.db", uuid::Uuid::new_v4()));
        let conn = Connection::open_in_memory().unwrap();
        schema(&conn).unwrap();
        let session = WriterSession::start(&path).unwrap();
        session
            .update(&conn, 3, Some(100), Some(25), false)
            .unwrap();
        let health = inspect(&conn, &path).unwrap();
        assert_eq!(health.interrupted_sessions, 0);
        assert_eq!(health.pending_records, 3);
        assert_eq!(health.write_delay_ms, Some(25));
        // Dropping without a close marker models OS lock release after a crash.
        drop(session);
        assert_eq!(inspect(&conn, &path).unwrap().interrupted_sessions, 1);
        let session = WriterSession::start(&path).unwrap();
        session.update(&conn, 0, Some(200), Some(1), true).unwrap();
        drop(session);
        assert_eq!(inspect(&conn, &path).unwrap().interrupted_sessions, 1);
        std::fs::remove_dir(path.with_extension("writers")).unwrap();
    }
}
