//! A durable admission ledger, deliberately independent of deletable usage statistics.
use super::proxy_usage::UsageRecord;
use crate::models::ProxyApiKey;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use std::path::{Path, PathBuf};

const MAX_TOKENS: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy)]
pub(crate) enum QuotaError {
    Exceeded,
    Unavailable,
    Unmetered,
    InvalidLimit,
}

impl QuotaError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Exceeded => "token_quota_exceeded",
            Self::Unavailable => "token_quota_unavailable",
            Self::Unmetered => "token_quota_unmetered",
            Self::InvalidLimit => "token_quota_invalid_limit",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Exceeded => "The API key token quota cannot cover this request and existing reservations. Reduce the request or increase the daily/monthly limit.",
            Self::Unavailable => "The token quota ledger is unavailable. The request was not forwarded.",
            Self::Unmetered => "A reliable token reservation is unavailable for this request or target. The request was not forwarded.",
            Self::InvalidLimit => "Hard token quotas require an explicit positive output limit, a single generation, and no output-limit overrides (n_predict, max_new_tokens, best_of).",
        }
    }
}

pub(crate) fn enabled(key: &ProxyApiKey) -> bool {
    key.daily_token_limit > 0 || key.monthly_token_limit > 0
}

fn path() -> PathBuf {
    crate::utils::get_data_dir().join("router-quota.db")
}

fn open(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(3))
        .map_err(|e| e.to_string())?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
         CREATE TABLE IF NOT EXISTS quota_periods (
           key_id TEXT NOT NULL, kind TEXT NOT NULL, start INTEGER NOT NULL,
           settled INTEGER NOT NULL DEFAULT 0, held INTEGER NOT NULL DEFAULT 0,
           pending INTEGER NOT NULL DEFAULT 0, uncertain INTEGER NOT NULL DEFAULT 0,
           PRIMARY KEY(key_id,kind,start));
         CREATE TABLE IF NOT EXISTS quota_requests (
           id TEXT PRIMARY KEY, key_id TEXT NOT NULL, day INTEGER NOT NULL, month INTEGER NOT NULL,
           reserved INTEGER NOT NULL, state TEXT NOT NULL, actual INTEGER, updated INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS quota_request_date ON quota_requests(day);",
    )
    .map_err(|e| e.to_string())?;
    Ok(conn)
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuotaPeriod {
    pub settled: u64,
    pub held: u64,
    pub pending: u64,
    pub uncertain: u64,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KeyQuota {
    pub daily_limit: u64,
    pub monthly_limit: u64,
    pub day: QuotaPeriod,
    pub month: QuotaPeriod,
}

fn period(conn: &Connection, key: &str, kind: &str, start: i64) -> Result<QuotaPeriod, String> {
    conn.query_row(
        "SELECT settled,held,pending,uncertain FROM quota_periods WHERE key_id=?1 AND kind=?2 AND start=?3",
        params![key, kind, start],
        |r| Ok(QuotaPeriod { settled: r.get::<_, i64>(0)? as u64, held: r.get::<_, i64>(1)? as u64, pending: r.get::<_, i64>(2)? as u64, uncertain: r.get::<_, i64>(3)? as u64 }),
    ).optional().map(|v| v.unwrap_or_default()).map_err(|e| e.to_string())
}

pub(crate) fn report(keys: &[ProxyApiKey], now: i64) -> Result<Vec<KeyQuota>, String> {
    let (day, month) = super::usage_budgets::periods(now)?;
    let mut conn = open(&path())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    keys.iter()
        .map(|key| {
            Ok(KeyQuota {
                daily_limit: key.daily_token_limit,
                monthly_limit: key.monthly_token_limit,
                day: period(&tx, &key.id, "day", day)?,
                month: period(&tx, &key.id, "month", month)?,
            })
        })
        .collect()
}

/// The owned guard also refunds a reservation if admission is cancelled before forwarding.
pub(crate) struct QuotaPermit {
    path: PathBuf,
    id: String,
    forwarded: bool,
    finished: bool,
}

fn reserve_at(
    path: &Path,
    key: &ProxyApiKey,
    id: &str,
    amount: u64,
    now: i64,
) -> Result<QuotaPermit, QuotaError> {
    if amount == 0 || amount > MAX_TOKENS {
        return Err(QuotaError::Unmetered);
    }
    let (day, month) = super::usage_budgets::periods(now).map_err(|_| QuotaError::Unavailable)?;
    let mut conn = open(path).map_err(|_| QuotaError::Unavailable)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| QuotaError::Unavailable)?;
    for (kind, start, limit) in [
        ("day", day, key.daily_token_limit),
        ("month", month, key.monthly_token_limit),
    ] {
        let current = period(&tx, &key.id, kind, start).map_err(|_| QuotaError::Unavailable)?;
        let total = current
            .settled
            .saturating_add(current.held)
            .saturating_add(amount);
        if (limit > 0 && total > limit) || total > MAX_TOKENS {
            return Err(QuotaError::Exceeded);
        }
        tx.execute(
            "INSERT INTO quota_periods(key_id,kind,start,held,pending) VALUES (?1,?2,?3,?4,1)
             ON CONFLICT(key_id,kind,start) DO UPDATE SET held=held+excluded.held,pending=pending+1",
            params![key.id, kind, start, amount as i64],
        ).map_err(|_| QuotaError::Unavailable)?;
    }
    tx.execute(
        "INSERT INTO quota_requests VALUES (?1,?2,?3,?4,?5,'pending',NULL,?6)",
        params![id, key.id, day, month, amount as i64, now],
    )
    .map_err(|_| QuotaError::Unavailable)?;
    tx.commit().map_err(|_| QuotaError::Unavailable)?;
    Ok(QuotaPermit {
        path: path.into(),
        id: id.into(),
        forwarded: false,
        finished: false,
    })
}

pub(crate) async fn reserve(
    key: ProxyApiKey,
    id: String,
    amount: u64,
) -> Result<QuotaPermit, QuotaError> {
    tokio::task::spawn_blocking(move || {
        reserve_at(
            &path(),
            &key,
            &id,
            amount,
            super::telemetry::current_time_ms(),
        )
    })
    .await
    .map_err(|_| QuotaError::Unavailable)?
}

fn settle(path: &Path, id: &str, actual: Option<u64>, complete: bool) -> Result<(), String> {
    let mut conn = open(path)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let (key, day, month, reserved, state): (String, i64, i64, u64, String) = tx
        .query_row(
            "SELECT key_id,day,month,reserved,state FROM quota_requests WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get::<_, i64>(3)? as u64,
                    r.get(4)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    if state != "pending" {
        return Ok(());
    }
    let charged = if complete {
        actual.unwrap_or(reserved)
    } else {
        0
    }
    .min(MAX_TOKENS);
    let held = if complete {
        0
    } else {
        actual.unwrap_or(0).max(reserved).min(MAX_TOKENS)
    };
    for (kind, start) in [("day", day), ("month", month)] {
        tx.execute("UPDATE quota_periods SET settled=MIN(?1,settled+?2),held=MIN(?1,held-?3+?4),pending=pending-1,uncertain=uncertain+?5
            WHERE key_id=?6 AND kind=?7 AND start=?8",
            params![MAX_TOKENS as i64, charged as i64, reserved as i64, held as i64, i64::from(!complete), key, kind, start]).map_err(|e| e.to_string())?;
    }
    tx.execute(
        "UPDATE quota_requests SET state=?1,actual=?2,updated=?3 WHERE id=?4",
        params![
            if complete { "settled" } else { "held" },
            actual.map(|v| v.min(MAX_TOKENS) as i64),
            super::telemetry::current_time_ms(),
            id
        ],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

impl QuotaPermit {
    pub fn forwarded(&mut self) {
        self.forwarded = true;
    }

    pub fn finish(mut self, record: &UsageRecord) {
        let actual = (record.tokens.input.is_some() || record.tokens.output.is_some()).then(|| {
            record
                .tokens
                .input
                .unwrap_or(0)
                .saturating_add(record.tokens.output.unwrap_or(0))
        });
        let complete =
            !self.forwarded || (record.outcome == "success" && record.quality == "complete");
        self.dispatch(if self.forwarded { actual } else { Some(0) }, complete);
    }

    fn dispatch(&mut self, actual: Option<u64>, complete: bool) {
        self.finished = true;
        let path = self.path.clone();
        let id = self.id.clone();
        let write = move || {
            if let Err(error) = settle(&path, &id, actual, complete) {
                // A failed settlement leaves the original durable reservation in place.
                eprintln!("quota settlement {id} retained its reservation: {error}");
            }
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(write);
        } else {
            write();
        }
    }
}

impl Drop for QuotaPermit {
    fn drop(&mut self) {
        if !self.finished {
            self.dispatch((!self.forwarded).then_some(0), !self.forwarded);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> ProxyApiKey {
        ProxyApiKey {
            id: "key".into(),
            daily_token_limit: 100,
            monthly_token_limit: 150,
            ..Default::default()
        }
    }
    #[test]
    fn older_configs_keep_hard_limits_disabled() {
        let old: ProxyApiKey = serde_json::from_str(r#"{"id":"old","daily_token_budget":50}"#).unwrap();
        assert!(!enabled(&old));
        assert_eq!((old.daily_token_limit, old.monthly_token_limit), (0, 0));
        assert_eq!(old.daily_token_budget, 50);
    }
    struct TestDir(PathBuf);
    impl Drop for TestDir {
        fn drop(&mut self) {
            assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
            for name in ["quota.db", "quota.db-wal", "quota.db-shm"] {
                let _ = std::fs::remove_file(self.0.join(name));
            }
            let _ = std::fs::remove_dir(&self.0);
        }
    }
    fn db() -> (TestDir, PathBuf) {
        let dir = std::env::temp_dir().join(format!("lsm-quota-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("quota.db");
        (TestDir(dir), path)
    }
    fn persist(mut p: QuotaPermit) {
        p.finished = true;
    }

    #[test]
    fn concurrent_reservations_are_atomic_and_survive_reopen() {
        let (_dir, path) = db();
        open(&path).unwrap();
        let threads: Vec<_> = (0..12)
            .map(|n| {
                let path = path.clone();
                std::thread::spawn(move || {
                    reserve_at(&path, &key(), &n.to_string(), 25, 0).map(persist)
                })
            })
            .collect();
        assert_eq!(
            threads
                .into_iter()
                .filter_map(|t| t.join().unwrap().ok())
                .count(),
            4
        );
        assert_eq!(
            period(&open(&path).unwrap(), "key", "day", 0).unwrap().held,
            100
        );
    }

    #[test]
    fn settlement_is_idempotent_and_unknown_is_not_zero() {
        let (_dir, path) = db();
        persist(reserve_at(&path, &key(), "a", 80, 0).unwrap());
        assert!(matches!(
            reserve_at(&path, &key(), "b", 21, 0),
            Err(QuotaError::Exceeded)
        ));
        settle(&path, "a", Some(20), true).unwrap();
        settle(&path, "a", Some(0), true).unwrap();
        persist(reserve_at(&path, &key(), "b", 80, 0).unwrap());
        settle(&path, "b", None, false).unwrap();
        let p = period(&open(&path).unwrap(), "key", "day", 0).unwrap();
        assert_eq!((p.settled, p.held, p.pending, p.uncertain), (20, 80, 0, 1));
        assert!(matches!(
            reserve_at(&path, &key(), "c", 1, 0),
            Err(QuotaError::Exceeded)
        ));
    }

    #[test]
    fn monthly_limit_and_original_period_apply_after_midnight() {
        let (_dir, path) = db();
        let day = super::super::usage_store::DAY_MS;
        persist(reserve_at(&path, &key(), "a", 90, 0).unwrap());
        settle(&path, "a", Some(90), true).unwrap();
        assert!(matches!(
            reserve_at(&path, &key(), "b", 61, day),
            Err(QuotaError::Exceeded)
        ));
        persist(reserve_at(&path, &key(), "b", 60, day).unwrap());
        settle(&path, "b", Some(40), true).unwrap();
        let conn = open(&path).unwrap();
        assert_eq!(period(&conn, "key", "day", 0).unwrap().settled, 90);
        assert_eq!(period(&conn, "key", "month", 0).unwrap().settled, 130);
        assert_eq!(period(&conn, "key", "day", day).unwrap().settled, 40);
    }

    #[test]
    fn unused_guard_refunds_and_keys_are_isolated() {
        let (_dir, path) = db();
        drop(reserve_at(&path, &key(), "a", 100, 0).unwrap());
        persist(reserve_at(&path, &key(), "b", 100, 0).unwrap());
        let other = ProxyApiKey {
            id: "other".into(),
            ..key()
        };
        persist(reserve_at(&path, &other, "c", 100, 0).unwrap());
        assert_eq!(
            period(&open(&path).unwrap(), "key", "day", 0).unwrap().held,
            100
        );
        assert!(matches!(
            reserve_at(&path.join("bad"), &key(), "d", 1, 0),
            Err(QuotaError::Unavailable)
        ));
    }

    #[test]
    fn duplicate_reservation_and_failed_settlement_roll_back_both_periods() {
        let (_dir, path) = db();
        persist(reserve_at(&path, &key(), "a", 50, 0).unwrap());
        assert!(matches!(
            reserve_at(&path, &key(), "a", 20, 0),
            Err(QuotaError::Unavailable)
        ));
        let conn = open(&path).unwrap();
        conn.execute_batch("CREATE TRIGGER fail_settlement BEFORE UPDATE ON quota_requests BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
        assert!(settle(&path, "a", Some(10), true).is_err());
        for kind in ["day", "month"] {
            let value = period(&conn, "key", kind, 0).unwrap();
            assert_eq!((value.settled, value.held, value.pending), (0, 50, 1));
        }
        conn.execute_batch("DROP TRIGGER fail_settlement").unwrap();
        settle(&path, "a", Some(10), true).unwrap();
        assert_eq!(period(&conn, "key", "day", 0).unwrap().settled, 10);
    }

    #[test]
    fn forwarded_drop_and_overrun_are_conservative_but_new_month_is_independent() {
        let (_dir, path) = db();
        let mut permit = reserve_at(&path, &key(), "a", 50, 0).unwrap();
        permit.forwarded();
        drop(permit);
        assert_eq!(
            period(&open(&path).unwrap(), "key", "day", 0).unwrap().held,
            50
        );
        persist(reserve_at(&path, &key(), "b", 50, 0).unwrap());
        settle(&path, "b", Some(70), true).unwrap();
        assert!(matches!(
            reserve_at(&path, &key(), "c", 1, 0),
            Err(QuotaError::Exceeded)
        ));
        let next_month = 31 * super::super::usage_store::DAY_MS;
        persist(reserve_at(&path, &key(), "c", 100, next_month).unwrap());
        assert_eq!(
            period(&open(&path).unwrap(), "key", "month", next_month)
                .unwrap()
                .held,
            100
        );
    }

    #[test]
    fn busy_ledger_does_not_admit_or_refund() {
        let (_dir, path) = db();
        persist(reserve_at(&path, &key(), "a", 50, 0).unwrap());
        let mut conn = open(&path).unwrap();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(matches!(
            reserve_at(&path, &key(), "b", 1, 0),
            Err(QuotaError::Unavailable)
        ));
        assert!(settle(&path, "a", Some(1), true).is_err());
        tx.rollback().unwrap();
        assert_eq!(period(&conn, "key", "day", 0).unwrap().held, 50);
    }
}
