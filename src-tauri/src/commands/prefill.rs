use std::collections::VecDeque;
use std::time::Instant;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::gufo::EngineKind;

const FIVE_MINUTES_MS: i64 = 300_000;
const MAX_INDIVIDUAL_POINTS: usize = 300;

// Keep enough adjacent polls to carry short bursts across the detached GUI
// bridge and the slower SQLite snapshot. Consumers deduplicate by sample time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstancePrefill {
    pub ts: i64,
    pub wall_ms: u64,
    pub tokens: Option<u64>,
    pub rate: Option<f64>,
    pub basis: CounterBasis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CounterBasis {
    EngineTime,
    WallTime,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CounterTotals {
    pub prompt: u64,
    pub generated: u64,
    pub seconds: Option<f64>,
}

struct CounterSample {
    kind: EngineKind,
    clock: Instant,
    ts: i64,
    totals: CounterTotals,
}

impl CounterSample {
    fn delta(&self, previous: &Self) -> Option<(u64, u64, Option<f64>)> {
        let wall = self.clock.checked_duration_since(previous.clock)?;
        let wall_ms = u64::try_from(wall.as_millis()).ok()?;
        let max_gap = if self.kind == EngineKind::Gufo {
            8_000
        } else {
            12_000
        };
        if self.kind != previous.kind
            || self.totals.generated < previous.totals.generated
            || !(100..=max_gap).contains(&wall_ms)
            || !(1..=max_gap as i64).contains(&(self.ts - previous.ts))
        {
            return None;
        }
        let tokens = self.totals.prompt.checked_sub(previous.totals.prompt)?;
        let seconds = match self.kind {
            EngineKind::Gufo => wall.as_secs_f64(),
            EngineKind::LlamaCpp => self.totals.seconds? - previous.totals.seconds?,
        };
        if seconds < 0.0 || (tokens > 0 && seconds == 0.0) {
            return None;
        }
        let rate = (tokens > 0)
            .then(|| tokens as f64 / seconds)
            .filter(|n| n.is_finite());
        Some((wall_ms, tokens, rate))
    }
}

#[derive(Default)]
pub struct PrefillCounters {
    previous: Option<CounterSample>,
    intervals: VecDeque<InstancePrefill>,
}

impl PrefillCounters {
    pub fn observe(
        &mut self,
        kind: EngineKind,
        clock: Instant,
        ts: i64,
        counters: Option<CounterTotals>,
    ) {
        let basis = match kind {
            EngineKind::LlamaCpp => CounterBasis::EngineTime,
            EngineKind::Gufo => CounterBasis::WallTime,
        };
        if self
            .intervals
            .back()
            .is_some_and(|old| old.ts >= ts || old.basis != basis)
        {
            self.intervals.clear();
            self.previous = None;
        }
        let current = counters
            .filter(|v| {
                kind == EngineKind::Gufo || v.seconds.is_some_and(|n| n.is_finite() && n >= 0.0)
            })
            .map(|totals| CounterSample {
                kind,
                clock,
                ts,
                totals,
            });
        let delta = current
            .as_ref()
            .zip(self.previous.as_ref())
            .and_then(|(a, b)| a.delta(b));
        let (wall_ms, tokens, rate) =
            delta.map_or((0, None, None), |(ms, n, rate)| (ms, Some(n), rate));
        self.previous = current;
        self.intervals.push_back(InstancePrefill {
            ts,
            basis,
            wall_ms,
            tokens,
            rate,
        });
        while self.intervals.len() > 12 {
            self.intervals.pop_front();
        }
    }

    pub fn recent(&self, now: i64) -> Vec<InstancePrefill> {
        self.intervals
            .iter()
            .filter(|p| p.ts <= now && now - p.ts <= 10_000)
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct GufoCompletion {
    pub id: i64,
    pub tokens: i64,
    pub rate: f64,
    pub measured_ms: Option<f64>,
}

impl GufoCompletion {
    pub fn parse(line: &str) -> Option<Self> {
        // Match the native HTTP completion record, including its logger and
        // structured prefix. Only newly received lines from Gufo reach here.
        let (prefix, fields) = line.split_once("] [http] request=r")?;
        if !prefix.ends_with("[INFO") && !prefix.ends_with("[DEBUG") {
            return None;
        }
        let (id, rest) = fields.split_once(" event=completed method=POST ")?;
        if id.is_empty() || !id.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let (mut status, mut finish, mut tokens, mut rate, mut measured_ms) =
            (None, None, None, None, None);
        for field in rest.split_ascii_whitespace() {
            let Some((key, value)) = field.split_once('=') else {
                continue;
            };
            match key {
                "status" => status = value.parse::<u16>().ok(),
                "finish" => finish = Some(value),
                "outcome" if value != "completed" => return None,
                "prefill_tokens" => tokens = value.parse::<i64>().ok(),
                "prefill_tps" => rate = value.parse::<f64>().ok(),
                "prefill_ms" => measured_ms = value.parse::<f64>().ok(),
                _ => {}
            }
        }
        if !status.is_some_and(|s| (200..300).contains(&s))
            || !matches!(finish, Some("stop" | "length" | "stop_sequence"))
        {
            return None;
        }
        Some(Self {
            id: id.parse().ok()?,
            tokens: tokens.filter(|n| *n > 0)?,
            rate: rate.filter(|v| v.is_finite() && *v > 0.0)?,
            measured_ms: measured_ms.filter(|v: &f64| v.is_finite() && *v > 0.0),
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedPrefill {
    pub ts: i64,
    pub request: Option<String>,
    pub tokens: i64,
    pub elapsed_ms: f64,
    pub count: u32,
    pub estimated: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPrefillWindow {
    pub as_of: i64,
    pub points: Vec<CompletedPrefill>,
    pub latest: Option<CompletedPrefill>,
    pub bucketed: bool,
}

pub fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS gufo_prefill_requests (
        session_id TEXT NOT NULL REFERENCES run_sessions(id) ON DELETE CASCADE,
        request_id INTEGER NOT NULL, completed_at INTEGER NOT NULL, tokens INTEGER NOT NULL,
        rate REAL NOT NULL, measured_ms REAL, PRIMARY KEY(session_id,request_id)
    );
    CREATE INDEX IF NOT EXISTS idx_gufo_prefill_time ON gufo_prefill_requests(session_id,completed_at);")
}

pub fn insert_gufo(
    conn: &Connection,
    session: &str,
    ts: i64,
    record: &GufoCompletion,
) -> rusqlite::Result<()> {
    conn.execute("INSERT INTO gufo_prefill_requests(session_id,request_id,completed_at,tokens,rate,measured_ms)
        VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(session_id,request_id) DO NOTHING",
        params![session,record.id,ts,record.tokens,record.rate,record.measured_ms])?;
    Ok(())
}

// Log rows contain engine work. A routed HTTP observation of the same request
// can have a duration but cannot contribute a second Prefill measurement.
const EVENT_QUERY: &str = "SELECT completed_at AS ts, '#' || task_id AS request, prompt_tokens AS n,
    prompt_time_ms AS ms, prompt_tps AS rate FROM inference_requests
    WHERE session_id=?1 AND completed_at>?2 AND completed_at<=?3 AND source='log' AND prompt_tokens>0
      AND (prompt_time_ms>0 OR prompt_tps>0)
    UNION ALL SELECT completed_at, 'r' || request_id, tokens, measured_ms, rate
    FROM gufo_prefill_requests WHERE session_id=?1 AND completed_at>?2 AND completed_at<=?3";

fn read_point(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<CompletedPrefill>> {
    let tokens: i64 = row.get(2)?;
    let measured = row
        .get::<_, Option<f64>>(3)?
        .filter(|v| v.is_finite() && *v > 0.0);
    let rate = row
        .get::<_, Option<f64>>(4)?
        .filter(|v| v.is_finite() && *v > 0.0);
    let Some(ms) = measured
        .or_else(|| rate.map(|v| tokens as f64 * 1000.0 / v))
        .filter(|v| v.is_finite() && *v > 0.0)
    else {
        return Ok(None);
    };
    Ok(Some(CompletedPrefill {
        ts: row.get(0)?,
        request: Some(row.get(1)?),
        tokens,
        elapsed_ms: ms,
        count: 1,
        estimated: measured.is_none(),
    }))
}

fn add_bucket(buckets: &mut Vec<CompletedPrefill>, mut point: CompletedPrefill) {
    if let Some(last) = buckets
        .last_mut()
        .filter(|b| b.ts / 1000 == point.ts / 1000)
    {
        last.ts = point.ts;
        last.tokens += point.tokens;
        last.elapsed_ms += point.elapsed_ms;
        last.count += point.count;
        last.estimated |= point.estimated;
    } else {
        point.request = None;
        buckets.push(point);
    }
}

pub fn request_window(
    conn: &Connection,
    session: &str,
    end: i64,
) -> rusqlite::Result<RequestPrefillWindow> {
    let mut window = RequestPrefillWindow {
        as_of: end,
        ..Default::default()
    };
    let mut stmt = conn.prepare(&format!("{EVENT_QUERY} ORDER BY ts, request"))?;
    let mut rows = stmt.query(params![session, end - FIVE_MINUTES_MS, end])?;
    while let Some(row) = rows.next()? {
        let Some(point) = read_point(row)? else {
            continue;
        };
        window.latest = Some(point.clone());
        if !window.bucketed && window.points.len() == MAX_INDIVIDUAL_POINTS {
            for old in std::mem::take(&mut window.points) {
                add_bucket(&mut window.points, old);
            }
            window.bucketed = true;
        }
        if window.bucketed {
            add_bucket(&mut window.points, point);
        } else {
            window.points.push(point);
        }
    }
    if window.latest.is_none() {
        let mut last = conn.prepare(&format!(
            "{EVENT_QUERY} ORDER BY ts DESC, request DESC LIMIT 1"
        ))?;
        let mut prior = last.query(params![session, 0, end - FIVE_MINUTES_MS])?;
        if let Some(row) = prior.next()? {
            window.latest = read_point(row)?;
        }
    }
    Ok(window)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn totals(prompt: u64, seconds: Option<f64>) -> Option<CounterTotals> {
        Some(CounterTotals {
            prompt,
            generated: 0,
            seconds,
        })
    }

    #[test]
    fn asynchronous_gufo_tokens_use_poll_time_and_llama_uses_engine_time() {
        let clock = Instant::now();
        let mut tracker = PrefillCounters::default();
        tracker.observe(EngineKind::Gufo, clock, 1000, totals(0, Some(0.0)));
        tracker.observe(
            EngineKind::Gufo,
            clock + Duration::from_millis(1000),
            2000,
            totals(1536, Some(0.0)),
        );
        let first = tracker.recent(2000).pop().unwrap();
        assert_eq!(
            (first.tokens, first.rate, first.basis),
            (Some(1536), Some(1536.0), CounterBasis::WallTime)
        );
        // Request completion advances time after the tokens have already been counted.
        tracker.observe(
            EngineKind::Gufo,
            clock + Duration::from_secs(2),
            3000,
            totals(1536, Some(0.75)),
        );
        assert_eq!(tracker.recent(3000).last().unwrap().rate, None);
        let mut tracker = PrefillCounters::default();
        tracker.observe(
            EngineKind::LlamaCpp,
            clock,
            1000,
            totals(816, Some(5.00973)),
        );
        tracker.observe(
            EngineKind::LlamaCpp,
            clock + Duration::from_secs(5),
            6000,
            totals(975, Some(6.44067)),
        );
        let last = tracker.recent(6000).pop().unwrap();
        assert_eq!(last.basis, CounterBasis::EngineTime);
        assert!((last.rate.unwrap() - 159.0 / 1.43094).abs() < 1.0e-8);
    }

    #[test]
    fn unusable_prefill_counters_do_not_bridge_failures_resets_or_gaps() {
        let start = Instant::now();
        let mut tracker = PrefillCounters::default();
        for (seconds, sample) in [
            (0, totals(900, Some(2.0))),
            (1, None),
            (2, totals(1000, Some(2.1))),
            (3, totals(12, Some(0.1))),
            (4, totals(32, Some(0.1))),
            (5, totals(42, None)),
            (6, totals(63, Some(1.0))),
            (30, totals(100, Some(2.0))),
        ] {
            tracker.observe(
                EngineKind::LlamaCpp,
                start + Duration::from_secs(seconds),
                seconds as i64 * 1000,
                sample,
            );
            assert_eq!(
                tracker.recent(seconds as i64 * 1000).last().unwrap().rate,
                None
            );
        }
        tracker.observe(
            EngineKind::Gufo,
            start + Duration::from_secs(31),
            31_000,
            totals(999, None),
        );
        assert_eq!(tracker.recent(31_000).len(), 1);
        assert!(tracker.recent(42_000).is_empty());
    }

    #[test]
    fn gufo_generation_counter_rollback_discards_the_prefill_baseline() {
        let start = Instant::now();
        let mut tracker = PrefillCounters::default();
        for (ts, prompt, generated) in [(0, 100, 200), (1000, 105, 1), (2000, 135, 10)] {
            tracker.observe(
                EngineKind::Gufo,
                start + Duration::from_millis(ts),
                ts as i64,
                Some(CounterTotals {
                    prompt,
                    generated,
                    seconds: None,
                }),
            );
        }
        let points = tracker.recent(2000);
        assert_eq!(points[1].rate, None);
        assert_eq!(points[2].rate, Some(30.0));
        assert!(GufoCompletion::parse(
            "2026-11-22 00:50:00 [INFO] [cache] event=cost_estimate prefill_ms=47333"
        )
        .is_none());
    }

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE run_sessions(id TEXT PRIMARY KEY);
            INSERT INTO run_sessions VALUES ('s'),('other');
            CREATE TABLE inference_requests(session_id TEXT, task_id INTEGER, completed_at INTEGER, source TEXT,
                prompt_tokens INTEGER,prompt_time_ms REAL,prompt_tps REAL);
            CREATE INDEX idx_request_prefill_test ON inference_requests(session_id,completed_at);").unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn real_gufo_log_shape_counts_only_uncached_work_and_is_idempotent() {
        let line = "2026-11-22 00:50:00 [INFO] [http] request=r1292 event=completed method=POST path=/v1/chat/completions status=200 outcome=completed duration_ms=48273.0 prompt_tokens=77972 prefill_tokens=1320 generated_tokens=1826 finish=stop cached_tokens=76652 ttft_ms=1600 prefill_tps=1069.5 decode_tps=39.3";
        let event = GufoCompletion::parse(line).unwrap();
        assert_eq!(
            (event.id, event.tokens, event.rate, event.measured_ms),
            (1292, 1320, 1069.5, None)
        );
        for invalid in [
            line.replace("event=completed", "event=received"),
            line.replace("status=200", "status=500"),
            line.replace("finish=stop", "finish=cancelled"),
            line.replace("prefill_tokens=1320", "prefill_tokens=0"),
            line.replace("prefill_tps=1069.5", "prefill_tps=NaN"),
        ] {
            assert!(GufoCompletion::parse(&invalid).is_none());
        }
        let conn = db();
        insert_gufo(&conn, "s", 1000, &event).unwrap();
        insert_gufo(&conn, "s", 2000, &event).unwrap();
        let window = request_window(&conn, "s", 3000).unwrap();
        assert_eq!(window.points.len(), 1);
        let point = window.latest.unwrap();
        assert_eq!(
            (point.ts, point.request.as_deref(), point.estimated),
            (1000, Some("r1292"), true)
        );
        let exact = GufoCompletion::parse(&format!("{line} prefill_ms=1234.2")).unwrap();
        assert_eq!(exact.measured_ms, Some(1234.2));
    }

    #[test]
    fn request_prefill_window_replays_real_timings_without_list_limit_or_proxy_duplicates() {
        let conn = db();
        let end = 1_000_000;
        for (id, offset, n, ms) in [
            (570, -120_085, 506, 3337.09),
            (1326, -65_558, 310, 1672.64),
            (1835, -14_327, 159, 1430.94),
        ] {
            conn.execute(
                "INSERT INTO inference_requests VALUES ('s',?1,?2,'log',?3,?4,NULL)",
                params![id, end + offset, n, ms],
            )
            .unwrap();
        }
        conn.execute_batch("INSERT INTO inference_requests VALUES ('s',1,900000,'proxy',999,1,999),
            ('other',2,900000,'log',999,1,999),('s',3,700000,'log',1,1,1000),('s',4,1000001,'log',1,1,1000);").unwrap();
        let window = request_window(&conn, "s", end).unwrap();
        assert_eq!(window.points.len(), 3);
        assert_eq!(window.latest.unwrap().request.as_deref(), Some("#1835"));
        let n: i64 = window.points.iter().map(|p| p.tokens).sum();
        let ms: f64 = window.points.iter().map(|p| p.elapsed_ms).sum();
        assert_eq!(n, 975);
        assert!((n as f64 * 1000.0 / ms - 151.38176618).abs() < 1.0e-6);
        for id in 0..360 {
            conn.execute(
                "INSERT INTO inference_requests VALUES ('s',?1,?2,'log',10,10,NULL)",
                params![10_000 + id, 950_000 + id * 3],
            )
            .unwrap();
        }
        let busy = request_window(&conn, "s", end).unwrap();
        assert!(busy.bucketed && busy.points.len() <= 301);
        assert_eq!(busy.points.iter().map(|p| p.count).sum::<u32>(), 363);
        assert_eq!(busy.points.iter().map(|p| p.tokens).sum::<i64>(), 4575);
        assert!(busy.points.iter().all(|p| p.request.is_none()));
        let old = request_window(&conn, "s", 2_000_000).unwrap();
        assert!(old.points.is_empty());
        assert_eq!(old.latest.unwrap().request.as_deref(), Some("#4"));
    }
}
