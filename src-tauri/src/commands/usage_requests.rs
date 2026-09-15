use super::{proxy_usage::UsageRecord, usage_store::UsageQuery};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestCursor {
    pub completed_at: i64,
    pub request_id: String,
    pub snapshot_row_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestQuery {
    #[serde(flatten)]
    pub filters: UsageQuery,
    pub outcome: Option<String>,
    pub failure_code: Option<String>,
    pub request_id: Option<String>,
    pub min_duration_ms: Option<u32>,
    pub min_queue_ms: Option<u32>,
    pub min_first_output_ms: Option<u32>,
    pub cursor: Option<RequestCursor>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestPage {
    pub records: Vec<UsageRecord>,
    pub next_cursor: Option<RequestCursor>,
    pub first_cursor: RequestCursor,
}

impl RequestQuery {
    fn validate(&self) -> Result<(), String> {
        self.filters.validate_range(false)?;
        if [&self.outcome, &self.failure_code, &self.request_id]
            .into_iter()
            .flatten()
            .any(|s| s.len() > 128)
        {
            return Err("Request filter is too long".into());
        }
        if self.cursor.as_ref().is_some_and(|c| {
            c.snapshot_row_id < 0
                || c.request_id.len() > 128
                || c.completed_at < self.filters.from
                || c.completed_at > self.filters.to
        }) {
            return Err("Invalid request page cursor".into());
        }
        Ok(())
    }
}

fn query(conn: &Connection, q: &RequestQuery) -> Result<RequestPage, String> {
    let snapshot = match &q.cursor {
        Some(c) => c.snapshot_row_id,
        None => conn
            .query_row("SELECT COALESCE(MAX(rowid),0) FROM usage_events", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?,
    };
    // The high-water mark excludes late commits as well as new requests. The
    // composite key avoids duplicates/skips when completion timestamps coincide.
    let mut stmt = conn.prepare("SELECT record FROM usage_events WHERE
        completed>=?1 AND completed<?2 AND (?3 IS NULL OR key_id=?3)
        AND (?4 IS NULL OR model=?4) AND (?5 IS NULL OR instance_id=?5)
        AND (?6 IS NULL OR endpoint=?6) AND ((?7 IS NULL AND kind<>'count') OR kind=?7)
        AND (?8 IS NULL OR json_extract(record,'$.outcome')=?8)
        AND (?9 IS NULL OR json_extract(record,'$.failure.code')=?9)
        AND (?10 IS NULL OR id=?10 OR json_extract(record,'$.responseRequestId')=?10 OR json_extract(record,'$.responseXRequestId')=?10 OR json_extract(record,'$.upstreamRequestId')=?10)
        AND rowid<=?11 AND (?12 IS NULL OR (completed,id)<(?12,?13))
        AND (?14 IS NULL OR json_extract(record,'$.durationMs')>=?14)
        AND (?15 IS NULL OR (json_extract(record,'$.queueEntered')=1 AND json_extract(record,'$.queueMs')>=?15))
        AND (?16 IS NULL OR json_extract(record,'$.firstOutputMs')>=?16)
        ORDER BY completed DESC,id DESC LIMIT 51").map_err(|e| e.to_string())?;
    let f = &q.filters;
    let rows = stmt
        .query_map(
            params![
                f.from,
                f.to,
                f.key_id,
                f.model,
                f.instance_id,
                f.endpoint,
                f.kind,
                q.outcome,
                q.failure_code,
                q.request_id,
                snapshot,
                q.cursor.as_ref().map(|c| c.completed_at),
                q.cursor.as_ref().map(|c| &c.request_id),
                q.min_duration_ms,
                q.min_queue_ms,
                q.min_first_output_ms
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?;
    let mut records: Vec<UsageRecord> = rows
        .map(|r| serde_json::from_str(&r.map_err(|e| e.to_string())?).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    let more = records.len() > 50;
    records.truncate(50);
    let next_cursor = records.last().filter(|_| more).map(|r| RequestCursor {
        completed_at: r.completed_at,
        request_id: r.request_id.clone(),
        snapshot_row_id: snapshot,
    });
    Ok(RequestPage {
        records,
        next_cursor,
        first_cursor: RequestCursor {
            completed_at: q.filters.to,
            request_id: String::new(),
            snapshot_row_id: snapshot,
        },
    })
}

#[tauri::command]
pub async fn get_router_usage_requests(
    query: RequestQuery,
) -> crate::error::AppResult<RequestPage> {
    query.validate().map_err(crate::error::AppError::from)?;
    tokio::task::spawn_blocking(move || {
        let conn = super::usage_store::open()?;
        conn.execute_batch("BEGIN DEFERRED")
            .map_err(|e| e.to_string())?;
        let page = self::query(&conn, &query);
        conn.execute_batch("ROLLBACK").map_err(|e| e.to_string())?;
        page
    })
    .await
    .map_err(|e| crate::error::AppError::from(e.to_string()))?
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::super::usage_store::DAY_MS;
    use super::*;
    use serde_json::json;

    #[test]
    fn pages_preserve_ties_exclude_late_commits_and_search_correlated_ids() {
        let conn = Connection::open_in_memory().unwrap();
        super::super::usage_store::schema(&conn).unwrap();
        let record = |id: &str| {
            json!({"requestId":id,"keyId":"key","keyName":"Client","model":"model","instanceId":"instance",
            "endpoint":"/v1/messages","kind":"generation","startedAt":DAY_MS,"completedAt":DAY_MS+10,
            "httpStatus":400,"forwarded":false,"outcome":"rejected","quality":"not_applicable","source":"none","tokens":{},
            "durationMs":10,"queueMs":0,"firstOutputMs":null,"finishReason":null,"items":null,
            "failure":{"stage":"preflight","code":"context_length_exceeded","reason":"Safe reason"},"responseRequestId":format!("req-{id}")})
        };
        let insert = |id: &str| {
            conn.execute("INSERT INTO usage_events VALUES (?1,?2,?3,'key','model','instance','/v1/messages','generation',?4)", params![id,DAY_MS,DAY_MS+10,record(id).to_string()]).unwrap();
        };
        for i in 0..125 {
            insert(&format!("id-{i:03}"));
        }
        let slow: RequestQuery =
            serde_json::from_value(json!({"from":DAY_MS,"to":2*DAY_MS,"minDurationMs":11}))
                .unwrap();
        assert!(query(&conn, &slow).unwrap().records.is_empty());
        let slow = RequestQuery {
            min_duration_ms: Some(10),
            ..slow
        };
        assert_eq!(query(&conn, &slow).unwrap().records.len(), 50);
        // Legacy records have no observed queue marker; absent first output is not zero.
        assert!(query(
            &conn,
            &RequestQuery {
                min_queue_ms: Some(0),
                ..slow.clone()
            }
        )
        .unwrap()
        .records
        .is_empty());
        assert!(query(
            &conn,
            &RequestQuery {
                min_first_output_ms: Some(0),
                ..slow
            }
        )
        .unwrap()
        .records
        .is_empty());
        let mut q: RequestQuery =
            serde_json::from_value(json!({"from":DAY_MS+1,"to":DAY_MS+20})).unwrap();
        q.validate().unwrap();
        let first = query(&conn, &q).unwrap();
        assert_eq!(first.records.len(), 50);
        let first_cursor = first.first_cursor.clone();
        insert("id-001-late");
        let mut ids: Vec<_> = first.records.into_iter().map(|r| r.request_id).collect();
        q.cursor = first.next_cursor;
        while q.cursor.is_some() {
            let page = query(&conn, &q).unwrap();
            ids.extend(page.records.into_iter().map(|r| r.request_id));
            q.cursor = page.next_cursor;
        }
        assert_eq!(ids.len(), 125);
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 125);
        q.cursor = Some(first_cursor);
        q.validate().unwrap();
        assert_eq!(query(&conn, &q).unwrap().records[0].request_id, "id-124");
        q.cursor = None;
        q.request_id = Some("req-id-001".into());
        q.failure_code = Some("context_length_exceeded".into());
        assert_eq!(query(&conn, &q).unwrap().records.len(), 1);
        q.outcome = Some("success".into());
        assert!(query(&conn, &q).unwrap().records.is_empty());
        q.outcome = None;
        q.request_id = Some("' OR 1=1 --".into());
        assert!(query(&conn, &q).unwrap().records.is_empty());
    }
}
