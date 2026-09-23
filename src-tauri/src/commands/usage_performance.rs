use super::proxy_usage::UsageRecord;
use super::usage_histogram::{Histogram, Percentiles};
use super::usage_store::{UsageQuery, DAY_MS};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const HOUR_MS: i64 = 3_600_000;
pub(super) const HOURLY_DAYS: i64 = 30;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Aggregate {
    requests: u64,
    success: u64,
    first_record_at: i64,
    last_record_at: i64,
    duration: Histogram,
    queue: Histogram,
    first_output: Histogram,
}

impl Aggregate {
    fn record(r: &UsageRecord) -> Self {
        let mut value = Self {
            requests: 1,
            success: u64::from(r.forwarded && r.outcome == "success"),
            first_record_at: r.completed_at,
            last_record_at: r.completed_at,
            ..Self::default()
        };
        if r.queue_entered {
            value.queue.observe(r.queue_ms);
        }
        if value.success > 0 {
            value.duration.observe(r.duration_ms);
            if let Some(ms) = r.first_output_ms {
                value.first_output.observe(ms);
            }
        }
        value
    }

    fn merge(&mut self, other: &Self) {
        self.requests = self.requests.saturating_add(other.requests);
        self.success = self.success.saturating_add(other.success);
        if self.first_record_at == 0
            || (other.first_record_at > 0 && other.first_record_at < self.first_record_at)
        {
            self.first_record_at = other.first_record_at;
        }
        self.last_record_at = self.last_record_at.max(other.last_record_at);
        self.duration.merge(&other.duration);
        self.queue.merge(&other.queue);
        self.first_output.merge(&other.first_output);
    }

    fn view(&self) -> PerformanceSummary {
        PerformanceSummary {
            requests: self.requests,
            success: self.success,
            first_record_at: (self.requests > 0).then_some(self.first_record_at),
            duration: self.duration.view(),
            queue: self.queue.view(),
            first_output: self.first_output.view(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerformanceSummary {
    requests: u64,
    success: u64,
    first_record_at: Option<i64>,
    duration: Percentiles,
    queue: Percentiles,
    first_output: Percentiles,
}

#[derive(Debug, Serialize)]
pub(crate) struct PerformanceGroup {
    id: String,
    name: String,
    summary: PerformanceSummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerformanceReport {
    summary: PerformanceSummary,
    keys: Vec<PerformanceGroup>,
    models: Vec<PerformanceGroup>,
    instances: Vec<PerformanceGroup>,
    periods: Vec<PerformanceGroup>,
    period_ms: i64,
    baseline: PerformanceSummary,
    baseline_from: i64,
    baseline_to: i64,
    elevated: Vec<String>,
    updated_at: i64,
}

pub(super) fn schema(conn: &Connection) -> Result<(), String> {
    for table in ["usage_performance_daily", "usage_performance_hourly"] {
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {table} (
            id TEXT PRIMARY KEY, bucket INTEGER NOT NULL, key_id TEXT NOT NULL, key_name TEXT NOT NULL,
            model TEXT NOT NULL, instance_id TEXT NOT NULL, endpoint TEXT NOT NULL, kind TEXT NOT NULL, summary TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS {table}_bucket ON {table}(bucket);
            CREATE INDEX IF NOT EXISTS {table}_key_bucket ON {table}(key_id,bucket);"
        )).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Called only after the event ID was inserted, inside the accounting transaction.
pub(super) fn record(conn: &Connection, r: &UsageRecord) -> Result<(), String> {
    if r.kind == "count" {
        return Ok(());
    }
    let update = Aggregate::record(r);
    for (table, width) in [
        ("usage_performance_daily", DAY_MS),
        ("usage_performance_hourly", HOUR_MS),
    ] {
        let bucket = r.completed_at.div_euclid(width) * width;
        let id = serde_json::to_string(&(
            bucket,
            &r.key_id,
            &r.model,
            &r.instance_id,
            &r.endpoint,
            &r.kind,
        ))
        .map_err(|e| e.to_string())?;
        let previous: Option<String> = conn
            .query_row(
                &format!("SELECT summary FROM {table} WHERE id=?1"),
                [&id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let mut aggregate: Aggregate = previous
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let latest_name = r.completed_at >= aggregate.last_record_at;
        aggregate.merge(&update);
        conn.execute(
            &format!(
                "INSERT INTO {table} VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
            ON CONFLICT(id) DO UPDATE SET summary=excluded.summary,
            key_name=CASE WHEN ?10 THEN excluded.key_name ELSE key_name END"
            ),
            params![
                id,
                bucket,
                r.key_id,
                r.key_name,
                r.model,
                r.instance_id,
                r.endpoint,
                r.kind,
                serde_json::to_string(&aggregate).map_err(|e| e.to_string())?,
                latest_name
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

type StoredRow = (i64, String, String, String, String, Aggregate);

fn rows(conn: &Connection, table: &str, q: &UsageQuery) -> Result<Vec<StoredRow>, String> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT bucket,key_id,key_name,model,instance_id,summary FROM {table}
        WHERE bucket>=?1 AND bucket<?2 AND (?3 IS NULL OR key_id=?3) AND (?4 IS NULL OR model=?4)
        AND (?5 IS NULL OR instance_id=?5) AND (?6 IS NULL OR endpoint=?6)
        AND ((?7 IS NULL AND kind<>'count') OR kind=?7) LIMIT 10001"
        ))
        .map_err(|e| e.to_string())?;
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
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    rows.enumerate()
        .map(|(index, r)| {
            if index >= 10_000 {
                return Err("性能查询数据过多，请缩小时间范围或增加筛选条件".into());
            }
            let (bucket, key, name, model, instance, json) = r.map_err(|e| e.to_string())?;
            Ok((
                bucket,
                key,
                name,
                model,
                instance,
                serde_json::from_str(&json).map_err(|e| e.to_string())?,
            ))
        })
        .collect()
}

fn query(conn: &Connection, q: &UsageQuery, now: i64) -> Result<PerformanceReport, String> {
    let mut total = Aggregate::default();
    let mut groups: [BTreeMap<String, (String, Aggregate)>; 3] = Default::default();
    let mut periods: BTreeMap<i64, Aggregate> = BTreeMap::new();
    let days = rows(conn, "usage_performance_daily", q)?;
    let hourly = q.to - q.from <= 14 * DAY_MS
        && q.from >= now.div_euclid(DAY_MS) * DAY_MS - HOURLY_DAYS * DAY_MS;
    let period_ms = if !hourly {
        DAY_MS
    } else if q.to - q.from <= 2 * DAY_MS {
        HOUR_MS
    } else {
        6 * HOUR_MS
    };
    for (bucket, key, name, model, instance, summary) in &days {
        total.merge(summary);
        for (i, (id, name)) in [(key, name), (model, model), (instance, instance)]
            .into_iter()
            .enumerate()
        {
            let group = groups[i]
                .entry(id.clone())
                .or_insert_with(|| (name.clone(), Aggregate::default()));
            if summary.last_record_at >= group.1.last_record_at {
                group.0 = name.clone();
            }
            group.1.merge(summary);
        }
        if !hourly {
            periods.entry(*bucket).or_default().merge(summary);
        }
    }
    if hourly {
        for (bucket, _, _, _, _, summary) in rows(conn, "usage_performance_hourly", q)? {
            periods
                .entry(bucket.div_euclid(period_ms) * period_ms)
                .or_default()
                .merge(&summary);
        }
    }
    let baseline_from = q.from.saturating_sub(7 * DAY_MS).max(0);
    let baseline_query = UsageQuery {
        from: baseline_from,
        to: q.from,
        ..q.clone()
    };
    let mut baseline = Aggregate::default();
    for (_, _, _, _, _, summary) in rows(conn, "usage_performance_daily", &baseline_query)? {
        baseline.merge(&summary);
    }
    let current = total.view();
    let reference = baseline.view();
    let elevated = [
        ("duration", &current.duration, &reference.duration),
        ("queue", &current.queue, &reference.queue),
        (
            "firstOutput",
            &current.first_output,
            &reference.first_output,
        ),
    ]
    .into_iter()
    .filter(|(_, a, b)| {
        a.samples >= 20
            && b.samples >= 20
            && a.p95
                .zip(b.p95)
                .is_some_and(|(now, before)| before > 0 && now > before.saturating_mul(2))
    })
    .map(|(name, _, _)| name.to_string())
    .collect();
    let [keys, models, instances] = groups.map(|group| {
        group
            .into_iter()
            .map(|(id, (name, aggregate))| PerformanceGroup {
                id,
                name,
                summary: aggregate.view(),
            })
            .collect()
    });
    Ok(PerformanceReport {
        summary: current,
        keys,
        models,
        instances,
        periods: periods
            .into_iter()
            .map(|(bucket, aggregate)| PerformanceGroup {
                id: bucket.to_string(),
                name: bucket.to_string(),
                summary: aggregate.view(),
            })
            .collect(),
        period_ms,
        baseline: reference,
        baseline_from,
        baseline_to: q.from,
        elevated,
        updated_at: now,
    })
}

#[tauri::command]
pub async fn get_router_performance(
    query: UsageQuery,
) -> crate::error::AppResult<PerformanceReport> {
    query
        .validate_range(true)
        .map_err(crate::error::AppError::from)?;
    tokio::task::spawn_blocking(move || {
        let conn = super::usage_store::open()?;
        conn.execute_batch("BEGIN DEFERRED")
            .map_err(|e| e.to_string())?;
        let result = self::query(&conn, &query, super::telemetry::current_time_ms());
        conn.execute_batch("ROLLBACK").map_err(|e| e.to_string())?;
        result
    })
    .await
    .map_err(|e| crate::error::AppError::from(e.to_string()))?
    .map_err(Into::into)
}

pub(super) fn clear(conn: &Connection, from: i64, to: i64) -> Result<(), String> {
    for table in ["usage_performance_daily", "usage_performance_hourly"] {
        conn.execute(
            &format!("DELETE FROM {table} WHERE bucket>=?1 AND bucket<?2"),
            params![from, to],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) fn prune(conn: &Connection, day: i64) -> Result<(), String> {
    for (table, days) in [
        ("usage_performance_daily", 365),
        ("usage_performance_hourly", HOURLY_DAYS),
    ] {
        conn.execute(
            &format!("DELETE FROM {table} WHERE bucket<?1"),
            [day - days * DAY_MS],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(at: i64, duration: u64, first: Option<u64>) -> UsageRecord {
        serde_json::from_value(json!({"requestId":at.to_string(),"keyId":"k","keyName":"Client","model":"m","instanceId":"i",
            "endpoint":"/v1/chat/completions","kind":"generation","startedAt":at-100,"completedAt":at,
            "httpStatus":200,"forwarded":true,"outcome":"success","quality":"complete","source":"openai","tokens":{},
            "durationMs":duration,"queueMs":0,"queueEntered":true,"firstOutputMs":first,"finishReason":null,"items":null})).unwrap()
    }

    fn filters(from: i64, to: i64) -> UsageQuery {
        serde_json::from_value(json!({"from":from,"to":to})).unwrap()
    }

    #[test]
    fn distributions_merge_samples_not_percentiles_and_keep_missing_separate_from_zero() {
        let conn = Connection::open_in_memory().unwrap();
        schema(&conn).unwrap();
        for i in 0..100 {
            record(&conn, &request(DAY_MS + HOUR_MS + i, 10, Some(0))).unwrap();
        }
        record(&conn, &request(DAY_MS + 2 * HOUR_MS, 1000, None)).unwrap();
        let mut rejected = request(DAY_MS + 3 * HOUR_MS, 9999, None);
        rejected.forwarded = false;
        rejected.outcome = "rejected".into();
        rejected.queue_ms = 50;
        record(&conn, &rejected).unwrap();
        let mut count = request(DAY_MS + 4 * HOUR_MS, 9999, Some(9999));
        count.kind = "count".into();
        record(&conn, &count).unwrap();
        let report = query(&conn, &filters(DAY_MS, 2 * DAY_MS), 2 * DAY_MS).unwrap();
        assert_eq!(report.summary.requests, 102);
        assert_eq!(report.summary.duration.samples, 101);
        assert_eq!(report.summary.duration.p95, Some(10));
        assert_eq!(report.summary.first_output.samples, 100);
        assert_eq!(report.summary.first_output.p50, Some(0));
        assert_eq!(report.summary.queue.samples, 102);
        assert_eq!(report.period_ms, HOUR_MS);
        assert_eq!(report.periods.len(), 3);
        assert_eq!(report.keys[0].summary.duration.p95, Some(10));
        assert_eq!(
            query(
                &conn,
                &UsageQuery {
                    model: Some("missing".into()),
                    ..filters(DAY_MS, 2 * DAY_MS)
                },
                2 * DAY_MS
            )
            .unwrap()
            .summary
            .duration
            .p95,
            None
        );
    }

    #[test]
    fn baseline_requires_samples_hourly_retention_and_clear_are_consistent() {
        let conn = Connection::open_in_memory().unwrap();
        schema(&conn).unwrap();
        for i in 0..20 {
            record(&conn, &request(10 * DAY_MS + i, 10, None)).unwrap();
            record(&conn, &request(17 * DAY_MS + i, 100, None)).unwrap();
        }
        let report = query(&conn, &filters(17 * DAY_MS, 18 * DAY_MS), 18 * DAY_MS).unwrap();
        assert_eq!(report.elevated, vec!["duration"]);
        assert_eq!(report.baseline_from, 10 * DAY_MS);
        assert!(
            query(&conn, &filters(10 * DAY_MS, 11 * DAY_MS), 18 * DAY_MS)
                .unwrap()
                .elevated
                .is_empty()
        );
        prune(&conn, 48 * DAY_MS).unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM usage_performance_hourly", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        let historical = query(&conn, &filters(17 * DAY_MS, 18 * DAY_MS), 48 * DAY_MS).unwrap();
        assert_eq!(historical.period_ms, DAY_MS);
        assert_eq!(historical.summary.duration.samples, 20);
        assert_eq!(historical.periods.len(), 1);
        clear(&conn, 17 * DAY_MS, 18 * DAY_MS).unwrap();
        assert_eq!(
            query(&conn, &filters(17 * DAY_MS, 18 * DAY_MS), 48 * DAY_MS)
                .unwrap()
                .summary
                .requests,
            0
        );
        prune(&conn, 376 * DAY_MS).unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM usage_performance_daily", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
