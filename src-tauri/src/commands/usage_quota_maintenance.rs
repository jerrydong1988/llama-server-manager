//! Bounded housekeeping. Never expire pending reservations or refund a period.
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde::Serialize;
use std::path::Path;

const RETENTION_DAYS: i64 = 365;
const BATCH: usize = 1000;

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuotaStorage {
    database_bytes: u64,
    wal_bytes: u64,
    reusable_bytes: u64,
    retention_days: i64,
}

pub(crate) fn storage() -> Result<QuotaStorage, String> {
    inspect(&super::path())
}

fn inspect(path: &Path) -> Result<QuotaStorage, String> {
    if !path.exists() {
        return Ok(QuotaStorage {
            retention_days: RETENTION_DAYS,
            ..Default::default()
        });
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    let pages: i64 = conn
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let bytes = |p: &Path| -> Result<u64, String> {
        match std::fs::metadata(p) {
            Ok(value) => Ok(value.len()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(error) => Err(error.to_string()),
        }
    };
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    Ok(QuotaStorage {
        database_bytes: bytes(path)?,
        wal_bytes: bytes(Path::new(&wal))?,
        reusable_bytes: pages.max(0).saturating_mul(page_size.max(0)) as u64,
        retention_days: RETENTION_DAYS,
    })
}

/// Returns true when another bounded pass may be needed. Busy writers take priority.
/// Freed SQLite pages are reused; avoid VACUUM's full database rewrite/admission lock.
pub(crate) fn maintain(path: &Path, now: i64) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    let mut conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::ZERO)
        .map_err(|e| e.to_string())?;
    prune(&mut conn, now, BATCH)
}

fn prune(conn: &mut Connection, now: i64, batch: usize) -> Result<bool, String> {
    let (day, month) = super::super::usage_budgets::periods(now)?;
    let cutoff = day.saturating_sub(RETENTION_DAYS * super::super::usage_store::DAY_MS);
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    // A recently completed old request also receives the full retention window.
    let requests = tx
        .execute(
            "DELETE FROM quota_requests WHERE id IN (
           SELECT id FROM quota_requests WHERE day<?1 AND month<?2 AND updated<?1
           AND state IN ('settled','held') ORDER BY day LIMIT ?3)",
            params![cutoff, month, batch as i64],
        )
        .map_err(|e| e.to_string())?;
    // Keep both original periods for every surviving request, including crash leftovers.
    // Never modify balances; expired periods are removed only after their last request.
    let periods = tx.execute(
        "DELETE FROM quota_periods WHERE rowid IN (
           SELECT p.rowid FROM quota_periods p WHERE p.start<?1 AND p.start<?2 AND p.pending=0
           AND p.kind IN ('day','month')
           AND NOT EXISTS (SELECT 1 FROM quota_requests r WHERE r.key_id=p.key_id AND r.day=p.start AND p.kind='day')
           AND NOT EXISTS (SELECT 1 FROM quota_requests r WHERE r.key_id=p.key_id AND r.month=p.start AND p.kind='month')
           LIMIT ?3)",
        params![cutoff, month, batch as i64],
    ).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(requests == batch || periods == batch)
}

#[cfg(test)]
mod tests {
    use super::super::super::usage_store::DAY_MS;
    use super::super::tests::{db, key, persist};
    use super::super::{open, period, reserve_at, settle};
    use super::*;

    #[test]
    fn expiry_preserves_pending_current_and_recently_settled_old_requests() {
        let (_dir, path) = db();
        let now = 800 * DAY_MS;
        for (id, date, complete) in [
            ("settled", 0, true),
            ("held", DAY_MS, false),
            ("recent", 2 * DAY_MS, true),
        ] {
            persist(reserve_at(&path, &key(), id, 10, date).unwrap());
            settle(&path, id, Some(5), complete).unwrap();
        }
        persist(reserve_at(&path, &key(), "pending", 10, 3 * DAY_MS).unwrap());
        persist(reserve_at(&path, &key(), "current", 90, now).unwrap());
        let mut conn = open(&path).unwrap();
        conn.execute(
            "UPDATE quota_requests SET updated=0 WHERE id IN ('settled','held')",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE quota_requests SET updated=?1 WHERE id='recent'",
            [now],
        )
        .unwrap();
        assert!(!prune(&mut conn, now, BATCH).unwrap());
        let ids: Vec<String> = conn
            .prepare("SELECT id FROM quota_requests ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(ids, ["current", "pending", "recent"]);
        assert_eq!(period(&conn, "key", "day", now).unwrap().held, 90);
        assert_eq!(period(&conn, "key", "month", 0).unwrap().held, 20);
        assert!(matches!(
            reserve_at(&path, &key(), "over", 11, now),
            Err(super::super::QuotaError::Exceeded)
        ));
        // Late settlement still has both original periods; duplicates, even expired ones, do nothing.
        settle(&path, "pending", Some(7), true).unwrap();
        settle(&path, "pending", Some(0), true).unwrap();
        settle(&path, "settled", Some(0), true).unwrap();
        assert_eq!(period(&conn, "key", "month", 0).unwrap().settled, 17);
        assert_eq!(period(&conn, "key", "day", now).unwrap().held, 90);
    }

    #[test]
    fn maintenance_is_bounded_transactional_and_reuses_freed_pages() {
        let (_dir, path) = db();
        let mut conn = open(&path).unwrap();
        let now = 800 * DAY_MS;
        // A populated legacy schema receives the indexes through open(), without resetting balances.
        conn.execute_batch("DROP INDEX quota_request_key_day; DROP INDEX quota_request_key_month;")
            .unwrap();
        drop(conn);
        conn = open(&path).unwrap();
        let tx = conn.transaction().unwrap();
        for n in 0..1100 {
            tx.execute(
                "INSERT INTO quota_requests VALUES (?1,'key',0,0,10,'settled',5,0)",
                [format!("history-{n}")],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        conn.execute_batch("INSERT INTO quota_periods VALUES ('key','day',0,5500,0,0,0);
          CREATE TRIGGER fail_cleanup BEFORE DELETE ON quota_periods BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        // A failure after request deletion rolls the whole pass back.
        assert!(prune(&mut conn, now, 2000).is_err());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM quota_requests", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1100
        );
        conn.execute_batch("DROP TRIGGER fail_cleanup").unwrap();
        assert!(prune(&mut conn, now, BATCH).unwrap());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM quota_requests", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            100
        );
        assert!(!prune(&mut conn, now, BATCH).unwrap());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM quota_periods", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let storage = inspect(&path).unwrap();
        assert_eq!(storage.retention_days, 365);
        assert!(storage.database_bytes + storage.wal_bytes > 0);
        assert!(storage.reusable_bytes > 0);
    }

    #[test]
    fn busy_or_absent_ledger_is_not_changed_by_maintenance() {
        let (_dir, path) = db();
        assert!(!maintain(&path, 800 * DAY_MS).unwrap());
        assert!(!path.exists());
        persist(reserve_at(&path, &key(), "pending", 100, 0).unwrap());
        let mut conn = open(&path).unwrap();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let started = std::time::Instant::now();
        assert!(maintain(&path, 800 * DAY_MS).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        tx.rollback().unwrap();
        assert_eq!(period(&conn, "key", "day", 0).unwrap().held, 100);
        assert!(!maintain(&path, 800 * DAY_MS).unwrap());
        assert_eq!(period(&conn, "key", "day", 0).unwrap().pending, 1);
    }
}
