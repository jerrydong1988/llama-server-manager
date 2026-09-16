use super::usage_store::{UsageSummary, DAY_MS};
use crate::models::{AppState, ProxyApiKey};
use chrono::{Datelike, TimeZone, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PeriodUsage {
    used: u64,
    partial: u64,
    unknown: u64,
}

impl PeriodUsage {
    fn add(&mut self, summary: &UsageSummary) {
        self.used = self
            .used
            .saturating_add(summary.input)
            .saturating_add(summary.output);
        self.partial = self.partial.saturating_add(summary.partial);
        self.unknown = self.unknown.saturating_add(summary.unknown);
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BudgetKey {
    id: String,
    name: String,
    enabled: bool,
    daily_budget: u64,
    monthly_budget: u64,
    day: PeriodUsage,
    month: PeriodUsage,
    quota: super::usage_quota::KeyQuota,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BudgetReport {
    keys: Vec<BudgetKey>,
    day_from: i64,
    month_from: i64,
    updated_at: i64,
    dropped_records: u64,
    write_errors: u64,
    health: super::usage_health::UsageHealth,
}

pub(super) fn periods(now: i64) -> Result<(i64, i64), String> {
    let date = Utc
        .timestamp_millis_opt(now)
        .single()
        .ok_or("Invalid budget timestamp")?;
    let month = Utc
        .with_ymd_and_hms(date.year(), date.month(), 1, 0, 0, 0)
        .single()
        .ok_or("Invalid budget month")?;
    Ok((now.div_euclid(DAY_MS) * DAY_MS, month.timestamp_millis()))
}

fn query(
    conn: &Connection,
    keys: &[ProxyApiKey],
    day: i64,
    month: i64,
) -> Result<Vec<BudgetKey>, String> {
    let mut totals: HashMap<String, (PeriodUsage, PeriodUsage)> = HashMap::new();
    let mut statement = conn
        .prepare(
            "SELECT day,key_id,summary FROM usage_daily WHERE day>=?1 AND day<?2 AND kind<>'count'",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![month, day + DAY_MS], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (bucket, key, json) = row.map_err(|e| e.to_string())?;
        let summary: UsageSummary = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        let total = totals.entry(key).or_default();
        if bucket == day {
            total.0.add(&summary);
        }
        total.1.add(&summary);
    }
    Ok(keys
        .iter()
        .map(|key| {
            let (day, month) = totals.remove(&key.id).unwrap_or_default();
            BudgetKey {
                id: key.id.clone(),
                name: key.name.clone(),
                enabled: key.enabled,
                daily_budget: key.daily_token_budget,
                monthly_budget: key.monthly_token_budget,
                day,
                month,
                quota: Default::default(),
            }
        })
        .collect())
}

#[tauri::command]
pub async fn get_router_budgets(
    state: tauri::State<'_, AppState>,
) -> crate::error::AppResult<BudgetReport> {
    let keys = state.proxy_config.lock().unwrap().api_keys.clone();
    tokio::task::spawn_blocking(move || {
        let now = super::telemetry::current_time_ms();
        let (day, month) = periods(now)?;
        let conn = super::usage_store::open()?;
        conn.execute_batch("BEGIN DEFERRED")
            .map_err(|e| e.to_string())?;
        let result = query(&conn, &keys, day, month);
        let (dropped, errors): (i64, i64) = conn
            .query_row(
                "SELECT COALESCE(SUM(dropped),0),COALESCE(SUM(errors),0) FROM usage_writers",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        conn.execute_batch("ROLLBACK").map_err(|e| e.to_string())?;
        let health = super::usage_health::inspect(
            &conn,
            &crate::utils::get_data_dir().join("router-usage.db"),
        );
        let mut rows = result?;
        for (row, quota) in rows.iter_mut().zip(super::usage_quota::report(&keys, now)?) {
            row.quota = quota;
        }
        Ok::<_, String>(BudgetReport {
            keys: rows,
            day_from: day,
            month_from: month,
            updated_at: now,
            dropped_records: dropped.max(0) as u64,
            write_errors: errors.max(0) as u64,
            health: health?,
        })
    })
    .await
    .map_err(|e| crate::error::AppError::from(e.to_string()))?
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_month_boundaries_and_partial_usage_do_not_double_count_cache() {
        let now = Utc
            .with_ymd_and_hms(2024, 3, 1, 0, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(periods(now).unwrap(), (now, now));
        let (day, month) = periods(now - 1).unwrap();
        assert_eq!(day, now - DAY_MS);
        assert_eq!(
            month,
            Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0)
                .unwrap()
                .timestamp_millis()
        );
        let conn = Connection::open_in_memory().unwrap();
        super::super::usage_store::schema(&conn).unwrap();
        let summary = UsageSummary {
            input: 100,
            output: 20,
            cached: 90,
            partial: 1,
            unknown: 2,
            ..Default::default()
        };
        for (id, bucket, kind) in [
            ("today", day, "generation"),
            ("month", month, "embedding"),
            ("old", month - DAY_MS, "generation"),
            ("count", day, "count"),
        ] {
            conn.execute("INSERT INTO usage_daily VALUES (?1,?2,'key','Old name','model','instance','/v1/test',?3,?4)",params![id,bucket,kind,serde_json::to_string(&summary).unwrap()]).unwrap();
        }
        let keys = vec![ProxyApiKey {
            id: "key".into(),
            name: "Renamed".into(),
            daily_token_budget: 10,
            monthly_token_budget: 20,
            ..Default::default()
        }];
        let result = query(&conn, &keys, day, month).unwrap();
        assert_eq!(result[0].name, "Renamed");
        assert_eq!(result[0].day.used, 120);
        assert_eq!(result[0].month.used, 240);
        assert_eq!(result[0].month.partial, 2);
        assert_eq!(result[0].month.unknown, 4);
        assert_eq!(result[0].daily_budget, 10);
        assert_eq!(result[0].monthly_budget, 20);
    }
}
