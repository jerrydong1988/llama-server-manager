//! Router accounting has its own retention and no foreign key to an instance run.
use super::proxy_usage::UsageRecord;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, LazyLock, Mutex,
};
use std::time::Duration;

pub(crate) const DAY_MS: i64 = 86_400_000;
const DETAIL_DAYS: i64 = 90;
const SUMMARY_DAYS: i64 = 365;
const QUEUE_CAPACITY: usize = 8192;
static WRITER: Mutex<Option<mpsc::SyncSender<Write>>> = Mutex::new(None);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static WRITE_ERRORS: AtomicU64 = AtomicU64::new(0);
static WRITER_ID: LazyLock<String> = LazyLock::new(|| uuid::Uuid::new_v4().to_string());

enum Write {
    Record(Box<UsageRecord>),
    Flush(mpsc::Sender<Result<(), String>>),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct UsageSummary {
    pub requests: u64,
    pub forwarded: u64,
    pub success: u64,
    pub failed: u64,
    pub rejected: u64,
    pub cancelled: u64,
    pub incomplete: u64,
    pub complete: u64,
    pub partial: u64,
    pub unknown: u64,
    pub not_applicable: u64,
    pub input: u64,
    pub output: u64,
    pub cached: u64,
    pub cache_input: u64,
    pub input_known: u64,
    pub output_known: u64,
    pub cache_known: u64,
    pub items: u64,
    pub duration_ms: u64,
    pub queue_ms: u64,
    pub first_output_ms: u64,
    pub first_output_count: u64,
    pub last_used: i64,
}

impl UsageSummary {
    pub fn add(&mut self, other: &Self) {
        macro_rules! sum { ($($field:ident),*) => { $(self.$field = self.$field.saturating_add(other.$field);)* }; }
        sum!(
            requests,
            forwarded,
            success,
            failed,
            rejected,
            cancelled,
            incomplete,
            complete,
            partial,
            unknown,
            not_applicable,
            input,
            output,
            cached,
            cache_input,
            input_known,
            output_known,
            cache_known,
            items,
            duration_ms,
            queue_ms,
            first_output_ms,
            first_output_count
        );
        self.last_used = self.last_used.max(other.last_used);
    }
    fn record(r: &UsageRecord) -> Self {
        let paired_cache = r.tokens.cached.zip(r.tokens.input);
        Self {
            requests: 1,
            forwarded: u64::from(r.forwarded),
            success: u64::from(r.outcome == "success"),
            failed: u64::from(r.outcome == "failed"),
            rejected: u64::from(r.outcome == "rejected"),
            cancelled: u64::from(r.outcome == "cancelled"),
            incomplete: u64::from(r.outcome == "incomplete"),
            complete: u64::from(r.quality == "complete"),
            partial: u64::from(r.quality == "partial"),
            unknown: u64::from(r.quality == "unknown"),
            not_applicable: u64::from(r.quality == "not_applicable"),
            input: r.tokens.input.unwrap_or(0),
            output: r.tokens.output.unwrap_or(0),
            cached: paired_cache.map_or(0, |(c, _)| c),
            cache_input: paired_cache.map_or(0, |(_, i)| i),
            input_known: u64::from(r.tokens.input.is_some()),
            output_known: u64::from(r.tokens.output.is_some()),
            cache_known: u64::from(paired_cache.is_some()),
            items: r.items.unwrap_or(0),
            duration_ms: r.duration_ms,
            queue_ms: r.queue_ms,
            first_output_ms: r.first_output_ms.unwrap_or(0),
            first_output_count: u64::from(r.first_output_ms.is_some()),
            last_used: r.completed_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageGroup {
    pub id: String,
    pub name: String,
    pub summary: UsageSummary,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageQuery {
    pub from: i64,
    pub to: i64,
    pub key_id: Option<String>,
    pub model: Option<String>,
    pub instance_id: Option<String>,
    pub endpoint: Option<String>,
    pub kind: Option<String>,
}

impl UsageQuery {
    fn validate(&self) -> Result<(), String> {
        if self.from < 0
            || self.to <= self.from
            || self.to - self.from > SUMMARY_DAYS * DAY_MS
            || self.from % DAY_MS != 0
            || self.to % DAY_MS != 0
        {
            return Err("Select a UTC day range of 1–365 days".into());
        }
        for s in [
            &self.key_id,
            &self.model,
            &self.instance_id,
            &self.endpoint,
            &self.kind,
        ]
        .into_iter()
        .flatten()
        {
            if s.len() > 1024 {
                return Err("Usage filter is too long".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageReport {
    pub summary: UsageSummary,
    pub keys: Vec<UsageGroup>,
    pub models: Vec<UsageGroup>,
    pub instances: Vec<UsageGroup>,
    pub days: Vec<UsageGroup>,
    pub endpoints: Vec<UsageGroup>,
    pub recent: Vec<UsageRecord>,
    pub recent_truncated: bool,
    pub dropped_records: u64,
    pub write_errors: u64,
    pub last_write_error: Option<String>,
    pub updated_at: i64,
    pub detail_days: i64,
    pub summary_days: i64,
}

fn schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage_events (
        id TEXT PRIMARY KEY, day INTEGER NOT NULL, completed INTEGER NOT NULL,
        key_id TEXT NOT NULL, model TEXT NOT NULL, instance_id TEXT NOT NULL,
        endpoint TEXT NOT NULL, kind TEXT NOT NULL, record TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS usage_events_completed ON usage_events(completed);
        CREATE TABLE IF NOT EXISTS usage_daily (
        id TEXT PRIMARY KEY, day INTEGER NOT NULL, key_id TEXT NOT NULL, key_name TEXT NOT NULL,
        model TEXT NOT NULL, instance_id TEXT NOT NULL, endpoint TEXT NOT NULL, kind TEXT NOT NULL,
        summary TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS usage_daily_day ON usage_daily(day);
        CREATE INDEX IF NOT EXISTS usage_daily_key_day ON usage_daily(key_id, day);
        CREATE TABLE IF NOT EXISTS usage_writers (
        id TEXT PRIMARY KEY, updated INTEGER NOT NULL, dropped INTEGER NOT NULL,
        errors INTEGER NOT NULL, last_error TEXT);
        CREATE TABLE IF NOT EXISTS usage_deletions (from_day INTEGER NOT NULL, to_day INTEGER NOT NULL, cleared_at INTEGER NOT NULL);
        PRAGMA user_version=1;",
    )
    .map_err(|e| e.to_string())
}

fn open() -> Result<Connection, String> {
    let dir = crate::utils::get_data_dir();
    open_path(&dir.join("router-usage.db"))
}

fn open_path(path: &std::path::Path) -> Result<Connection, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_secs(3))
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| e.to_string())?;
    schema(&conn)?;
    Ok(conn)
}

fn insert(conn: &Connection, r: &UsageRecord) -> Result<(), String> {
    let day = r.completed_at.div_euclid(DAY_MS) * DAY_MS;
    let deleted: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM usage_deletions WHERE ?1>=from_day AND ?1<to_day AND ?2<=cleared_at)", params![day,r.completed_at], |row| row.get(0)).map_err(|e| e.to_string())?;
    if deleted {
        return Ok(());
    }
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO usage_events VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                r.request_id,
                day,
                r.completed_at,
                r.key_id,
                r.model,
                r.instance_id,
                r.endpoint,
                r.kind,
                serde_json::to_string(r).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
    if inserted == 0 {
        return Ok(());
    }
    let id = serde_json::to_string(&(
        day,
        &r.key_id,
        &r.model,
        &r.instance_id,
        &r.endpoint,
        &r.kind,
    ))
    .map_err(|e| e.to_string())?;
    let previous: Option<String> = conn
        .query_row(
            "SELECT summary FROM usage_daily WHERE id=?1",
            [&id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let mut summary: UsageSummary = previous
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let latest_name = r.completed_at >= summary.last_used;
    summary.add(&UsageSummary::record(r));
    conn.execute(
        "INSERT INTO usage_daily VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
        ON CONFLICT(id) DO UPDATE SET summary=excluded.summary,
        key_name=CASE WHEN ?10 THEN excluded.key_name ELSE key_name END",
        params![
            id,
            day,
            r.key_id,
            r.key_name,
            r.model,
            r.instance_id,
            r.endpoint,
            r.kind,
            serde_json::to_string(&summary).map_err(|e| e.to_string())?,
            latest_name
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn write_batch(conn: &mut Connection, records: &[Box<UsageRecord>]) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for record in records {
        insert(&tx, record)?;
    }
    tx.commit().map_err(|e| e.to_string())
}

fn health(conn: &Connection, error: Option<&str>) -> Result<(), String> {
    conn.execute("INSERT INTO usage_writers VALUES (?1,?2,?3,?4,?5)
        ON CONFLICT(id) DO UPDATE SET updated=excluded.updated,dropped=excluded.dropped,errors=excluded.errors,last_error=excluded.last_error",
        params![&*WRITER_ID, super::telemetry::current_time_ms(), DROPPED.load(Ordering::Relaxed).min(i64::MAX as u64) as i64, WRITE_ERRORS.load(Ordering::Relaxed).min(i64::MAX as u64) as i64, error]).map_err(|e| e.to_string())?;
    Ok(())
}

fn writer_loop(receiver: mpsc::Receiver<Write>, path: std::path::PathBuf) {
    let mut conn = None;
    let mut last_error: Option<String> = None;
    let mut last_prune = 0;
    loop {
        let first = receiver.recv_timeout(Duration::from_secs(2));
        if matches!(first, Err(mpsc::RecvTimeoutError::Disconnected)) {
            break;
        }
        let mut records = Vec::new();
        let mut flushes = Vec::new();
        if let Ok(first) = first {
            let mut pending = Some(first);
            while let Some(write) = pending {
                match write {
                    Write::Record(r) => records.push(r),
                    Write::Flush(f) => {
                        flushes.push(f);
                        break;
                    }
                }
                if records.len() >= 128 {
                    break;
                }
                pending = receiver.try_recv().ok();
            }
        }
        if conn.is_none() {
            match open_path(&path) {
                Ok(c) => conn = Some(c),
                Err(e) => last_error = Some(e),
            }
        }
        let result = if let Some(c) = conn.as_mut() {
            let mut result = write_batch(c, &records);
            // Idempotent event IDs make a retry safe even after an uncertain commit.
            if result.is_err() {
                result = write_batch(c, &records);
            }
            result
        } else {
            Err(last_error
                .clone()
                .unwrap_or_else(|| "Usage storage unavailable".into()))
        };
        if let Err(e) = &result {
            DROPPED.fetch_add(records.len() as u64, Ordering::Relaxed);
            WRITE_ERRORS.fetch_add(1, Ordering::Relaxed);
            last_error = Some(e.clone());
        }
        if let Some(c) = conn.as_mut() {
            let now = super::telemetry::current_time_ms();
            if now - last_prune > 900_000 {
                if let Err(e) = prune(c, now) {
                    last_error = Some(e);
                }
                last_prune = now;
            }
            if let Err(e) = health(c, last_error.as_deref()) {
                last_error = Some(e);
            }
        }
        for flush in flushes {
            let _ = flush.send(result.clone());
        }
    }
}

fn writer() -> Result<mpsc::SyncSender<Write>, String> {
    let mut guard = WRITER.lock().map_err(|e| e.to_string())?;
    if let Some(w) = guard.as_ref() {
        return Ok(w.clone());
    }
    let (tx, rx) = mpsc::sync_channel(QUEUE_CAPACITY);
    let path = crate::utils::get_data_dir().join("router-usage.db");
    std::thread::Builder::new()
        .name("router-usage-writer".into())
        .spawn(move || writer_loop(rx, path))
        .map_err(|e| e.to_string())?;
    *guard = Some(tx.clone());
    Ok(tx)
}

pub(crate) fn record(record: UsageRecord) {
    if cfg!(test) {
        capture_test_record(&record);
        return;
    }
    if writer()
        .and_then(|w| {
            w.try_send(Write::Record(Box::new(record)))
                .map_err(|e| e.to_string())
        })
        .is_err()
    {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

fn capture_test_record(record: &UsageRecord) {
    #[cfg(test)]
    TEST_RECORDS.lock().unwrap().push(record.clone());
    #[cfg(not(test))]
    let _ = record;
}

pub(crate) fn flush() -> Result<(), String> {
    let writer = WRITER.lock().map_err(|e| e.to_string())?.clone();
    let Some(writer) = writer else {
        return Ok(());
    };
    let (tx, rx) = mpsc::channel();
    writer
        .try_send(Write::Flush(tx))
        .map_err(|e| e.to_string())?;
    rx.recv_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?
}

fn prune(conn: &mut Connection, now: i64) -> Result<(), String> {
    let day = now.div_euclid(DAY_MS) * DAY_MS;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM usage_events WHERE day<?1",
        [day - DETAIL_DAYS * DAY_MS],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM usage_daily WHERE day<?1",
        [day - SUMMARY_DAYS * DAY_MS],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

const FILTER: &str = "day>=?1 AND day<?2 AND (?3 IS NULL OR key_id=?3) AND (?4 IS NULL OR model=?4)
    AND (?5 IS NULL OR instance_id=?5) AND (?6 IS NULL OR endpoint=?6)
    AND ((?7 IS NULL AND kind<>'count') OR kind=?7)";

fn query(conn: &Connection, q: &UsageQuery) -> Result<UsageReport, String> {
    use std::collections::BTreeMap;
    let mut total = UsageSummary::default();
    let mut groups: [BTreeMap<String, UsageGroup>; 5] = Default::default();
    let mut statement = conn.prepare(&format!("SELECT day,key_id,key_name,model,instance_id,endpoint,summary FROM usage_daily WHERE {FILTER}")).map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(
            params![
                q.from,
                q.to,
                q.key_id,
                q.model,
                q.instance_id,
                q.endpoint,
                q.kind
            ],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (day, key, name, model, instance, endpoint, json) = row.map_err(|e| e.to_string())?;
        let summary: UsageSummary = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        total.add(&summary);
        for (i, (id, name)) in [
            (key, name),
            (model.clone(), model),
            (instance.clone(), instance),
            (day.to_string(), day.to_string()),
            (endpoint.clone(), endpoint),
        ]
        .into_iter()
        .enumerate()
        {
            let group = groups[i].entry(id.clone()).or_insert_with(|| UsageGroup {
                id,
                name: name.clone(),
                summary: Default::default(),
            });
            if summary.last_used >= group.summary.last_used {
                group.name = name;
            }
            group.summary.add(&summary);
        }
    }
    let mut statement = conn.prepare(&format!("SELECT record FROM usage_events WHERE {FILTER} ORDER BY completed DESC,id DESC LIMIT 101")).map_err(|e| e.to_string())?;
    let records = statement
        .query_map(
            params![
                q.from,
                q.to,
                q.key_id,
                q.model,
                q.instance_id,
                q.endpoint,
                q.kind
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?;
    let mut recent = Vec::new();
    for r in records {
        recent
            .push(serde_json::from_str(&r.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?);
    }
    let recent_truncated = recent.len() > 100;
    recent.truncate(100);
    let (dropped, errors): (i64, i64) = conn
        .query_row(
            "SELECT COALESCE(SUM(dropped),0),COALESCE(SUM(errors),0) FROM usage_writers",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let error: Option<String> = conn.query_row("SELECT last_error FROM usage_writers WHERE last_error IS NOT NULL ORDER BY updated DESC LIMIT 1", [], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
    let [keys, models, instances, days, endpoints] = groups.map(|m| m.into_values().collect());
    Ok(UsageReport {
        summary: total,
        keys,
        models,
        instances,
        days,
        endpoints,
        recent,
        recent_truncated,
        dropped_records: dropped.max(0) as u64,
        write_errors: errors.max(0) as u64,
        last_write_error: error,
        updated_at: super::telemetry::current_time_ms(),
        detail_days: DETAIL_DAYS,
        summary_days: SUMMARY_DAYS,
    })
}

#[tauri::command]
pub async fn get_router_usage(query: UsageQuery) -> crate::error::AppResult<UsageReport> {
    query.validate().map_err(crate::error::AppError::from)?;
    tokio::task::spawn_blocking(move || {
        let conn = open()?;
        // A single read transaction keeps totals, groups and details on the same snapshot.
        conn.execute_batch("BEGIN DEFERRED")
            .map_err(|e| e.to_string())?;
        let report = self::query(&conn, &query);
        conn.execute_batch("ROLLBACK").map_err(|e| e.to_string())?;
        report
    })
    .await
    .map_err(|e| crate::error::AppError::from(e.to_string()))?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn clear_router_usage(from: i64, to: i64) -> crate::error::AppResult<()> {
    UsageQuery {
        from,
        to,
        key_id: None,
        model: None,
        instance_id: None,
        endpoint: None,
        kind: None,
    }
    .validate()
    .map_err(crate::error::AppError::from)?;
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut conn = open()?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO usage_deletions VALUES (?1,?2,?3)",
            params![from, to, super::telemetry::current_time_ms()],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM usage_events WHERE day>=?1 AND day<?2",
            params![from, to],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM usage_daily WHERE day>=?1 AND day<?2",
            params![from, to],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| crate::error::AppError::from(e.to_string()))?
    .map_err(Into::into)
}

#[cfg(test)]
pub(crate) static TEST_RECORDS: Mutex<Vec<UsageRecord>> = Mutex::new(Vec::new());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::usage_protocol::TokenUsage;
    fn request() -> UsageRecord {
        UsageRecord {
            request_id: "once".into(),
            key_id: "key-one".into(),
            key_name: "Client".into(),
            model: "model".into(),
            instance_id: "instance".into(),
            endpoint: "/v1/chat/completions".into(),
            kind: "generation".into(),
            started_at: DAY_MS,
            completed_at: DAY_MS + 10,
            http_status: 200,
            forwarded: true,
            outcome: "success".into(),
            quality: "complete".into(),
            source: "upstream_usage".into(),
            tokens: TokenUsage {
                input: Some(10),
                output: Some(2),
                cached: Some(8),
                ..Default::default()
            },
            duration_ms: 10,
            queue_ms: 0,
            first_output_ms: None,
            finish_reason: None,
            items: None,
        }
    }
    #[test]
    fn accounting_is_idempotent_independent_and_retains_daily_history() {
        let mut conn = Connection::open_in_memory().unwrap();
        schema(&conn).unwrap();
        let r = request();
        write_batch(&mut conn, &[Box::new(r.clone()), Box::new(r)]).unwrap();
        let q = UsageQuery {
            from: DAY_MS,
            to: 2 * DAY_MS,
            key_id: None,
            model: None,
            instance_id: None,
            endpoint: None,
            kind: None,
        };
        let report = query(&conn, &q).unwrap();
        assert_eq!(report.summary.requests, 1);
        assert_eq!(report.summary.input, 10);
        assert_eq!(report.summary.cached, 8);
        assert_eq!(report.keys[0].id, "key-one");
        prune(&mut conn, 100 * DAY_MS).unwrap();
        let report = query(&conn, &q).unwrap();
        assert!(report.recent.is_empty());
        assert_eq!(report.summary.requests, 1);
        prune(&mut conn, 400 * DAY_MS).unwrap();
        assert_eq!(query(&conn, &q).unwrap().summary.requests, 0);
    }
    #[test]
    fn filters_preserve_identity_and_do_not_mix_counting_with_inference() {
        let mut conn = Connection::open_in_memory().unwrap();
        schema(&conn).unwrap();
        let mut a = request();
        let mut b = request();
        b.request_id = "two".into();
        b.key_id = "different".into();
        let mut c = request();
        c.request_id = "count".into();
        c.kind = "count".into();
        c.quality = "not_applicable".into();
        a.key_name = "Same name".into();
        b.key_name = a.key_name.clone();
        write_batch(&mut conn, &[Box::new(a), Box::new(b), Box::new(c)]).unwrap();
        let mut q = UsageQuery {
            from: DAY_MS,
            to: 2 * DAY_MS,
            key_id: None,
            model: None,
            instance_id: None,
            endpoint: None,
            kind: None,
        };
        let report = query(&conn, &q).unwrap();
        assert_eq!(report.keys.len(), 2);
        assert_eq!(report.summary.requests, 2);
        q.key_id = Some("different".into());
        assert_eq!(query(&conn, &q).unwrap().summary.requests, 1);
        q.key_id = Some("' OR 1=1 --".into());
        assert_eq!(query(&conn, &q).unwrap().summary.requests, 0);
    }

    #[test]
    fn deleted_ranges_do_not_reappear_when_queued_records_arrive() {
        let conn = Connection::open_in_memory().unwrap();
        schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO usage_deletions VALUES (?1,?2,?3)",
            params![DAY_MS, 2 * DAY_MS, DAY_MS + 50],
        )
        .unwrap();
        let mut r = request();
        insert(&conn, &r).unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM usage_events", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        r.completed_at = DAY_MS + 100;
        insert(&conn, &r).unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM usage_events", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn background_writer_flushes_and_a_new_connection_reads_persisted_history() {
        let path = std::env::temp_dir().join(format!("lsm-usage-test-{}.db", uuid::Uuid::new_v4()));
        let writer_path = path.clone();
        let (tx, rx) = mpsc::sync_channel(32);
        let worker = std::thread::spawn(move || writer_loop(rx, writer_path));
        for i in 0..20 {
            let mut r = request();
            r.request_id = format!("persist-{i}");
            r.completed_at = super::super::telemetry::current_time_ms();
            tx.send(Write::Record(Box::new(r))).unwrap();
        }
        let (waiter, wait) = mpsc::channel();
        tx.send(Write::Flush(waiter)).unwrap();
        wait.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
        drop(tx);
        worker.join().unwrap();
        {
            let conn = open_path(&path).unwrap();
            assert_eq!(
                conn.query_row("SELECT COUNT(*) FROM usage_events", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                20
            );
            let json: String = conn
                .query_row("SELECT summary FROM usage_daily", [], |r| r.get(0))
                .unwrap();
            let summary: UsageSummary = serde_json::from_str(&json).unwrap();
            assert_eq!(summary.requests, 20);
            assert_eq!(summary.input, 200);
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn failed_transaction_does_not_leave_partial_totals_and_retry_is_safe() {
        let mut conn = Connection::open_in_memory().unwrap();
        schema(&conn).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_test BEFORE INSERT ON usage_events WHEN NEW.id='bad' BEGIN SELECT RAISE(ABORT,'test write failure'); END;").unwrap();
        let a = request();
        let mut b = request();
        b.request_id = "bad".into();
        let batch = [Box::new(a), Box::new(b)];
        assert!(write_batch(&mut conn, &batch).is_err());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM usage_daily", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        conn.execute_batch("DROP TRIGGER reject_test").unwrap();
        write_batch(&mut conn, &batch).unwrap();
        write_batch(&mut conn, &batch).unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM usage_events", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
}
