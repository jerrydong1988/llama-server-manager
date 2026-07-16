use crate::commands::vector_metrics::{VectorEventSource, VectorTrendBucket};
use crate::models::{InstanceConfig, SystemMetrics};
use crate::vector_policy::ModelWorkload;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: i64 = 7;
const VECTOR_RATE_WINDOW_MS: i64 = 60_000;
const TELEMETRY_WRITE_QUEUE_CAPACITY: usize = 4_096;
const TELEMETRY_WRITE_BATCH_SIZE: usize = 128;

static TELEMETRY_SCHEMA_READY: AtomicBool = AtomicBool::new(false);
static TELEMETRY_SCHEMA_LOCK: Mutex<()> = Mutex::new(());
static TELEMETRY_WRITER: Mutex<Option<mpsc::SyncSender<TelemetryWrite>>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct TelemetryOverview {
    pub active_sessions: u32,
    pub sessions_24h: u32,
    pub avg_tokens_per_sec_24h: f64,
    pub peak_vram_mb_24h: f64,
    pub latest_samples: Vec<TelemetrySampleSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TelemetrySessionSummary {
    pub id: String,
    pub instance_id: String,
    pub instance_name: String,
    pub model_name: String,
    pub model_path: String,
    pub engine_id: String,
    pub backend: String,
    pub workload: String,
    pub started_at: i64,
    pub stopped_at: Option<i64>,
    pub duration_secs: Option<i64>,
    pub avg_tokens_per_sec: f64,
    pub peak_vram_mb: f64,
    pub sample_count: u32,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TelemetrySampleSummary {
    pub session_id: String,
    pub instance_id: String,
    pub ts: i64,
    pub cpu_percent: Option<f64>,
    pub memory_mb: Option<f64>,
    pub gpu_percent: Option<f64>,
    pub vram_used_mb: Option<f64>,
    pub vram_total_mb: Option<f64>,
    pub system_cpu_percent: Option<f64>,
    pub system_memory_used_mb: Option<f64>,
    pub system_memory_total_mb: Option<f64>,
    pub gpu_vendor: Option<String>,
    pub gpu_name: Option<String>,
    pub tokens_per_sec: Option<f64>,
    pub prompt_tokens_per_sec: Option<f64>,
    pub prompt_tokens_total: Option<i64>,
    pub generated_tokens_total: Option<i64>,
    pub requests_total: Option<i64>,
    pub decode_calls_total: Option<i64>,
    pub max_tokens_observed: Option<i64>,
    pub requests_processing: Option<i64>,
    pub requests_deferred: Option<i64>,
    pub busy_slots_per_decode: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InferenceRequestSummary {
    pub session_id: String,
    pub task_id: u32,
    pub slot_id: u32,
    pub completed_at: i64,
    pub source: String,
    pub model: Option<String>,
    pub target_instance_id: Option<String>,
    pub http_status: Option<u16>,
    pub error_text: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub prompt_time_ms: Option<f64>,
    pub prompt_tps: Option<f64>,
    pub generated_tokens: Option<u64>,
    pub generation_time_ms: Option<f64>,
    pub generation_tps: Option<f64>,
    pub total_tokens: Option<u64>,
    pub total_time_ms: Option<f64>,
    pub spec_accept_rate: Option<f64>,
    pub spec_accepted: Option<u64>,
    pub spec_generated: Option<u64>,
    pub spec_gen_time_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TelemetrySessionAnalysis {
    pub request_count: u32,
    pub avg_prompt_tokens: f64,
    pub avg_generated_tokens: f64,
    pub avg_total_tokens: f64,
    pub avg_prompt_tps: f64,
    pub avg_generation_tps: f64,
    pub avg_total_time_ms: f64,
    pub max_total_tokens: u64,
    pub avg_busy_slots: f64,
    pub max_busy_slots: u32,
    pub avg_cached_slots: f64,
    pub max_context_tokens: u32,
    pub slot_sample_count: u32,
    pub speculative_analysis: Option<SpeculativeTelemetryAnalysis>,
    pub vector_analysis: Option<VectorTelemetryAnalysis>,
    pub vector_baseline: Option<VectorTelemetryBaseline>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeculativeTelemetryAnalysis {
    pub request_count: u32,
    pub acceptance_rate: Option<f64>,
    pub accepted_tokens: u64,
    pub generated_tokens: u64,
    pub avg_generation_time_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorTelemetryAnalysis {
    pub workload: String,
    pub log_available: bool,
    pub proxy_available: bool,
    pub completed_items: Option<u64>,
    pub input_tokens: Option<u64>,
    pub average_input_tokens_per_second: Option<f64>,
    pub average_items_per_second: Option<f64>,
    pub task_duration_p50_ms: Option<f64>,
    pub task_duration_p95_ms: Option<f64>,
    pub proxy_request_count: Option<u64>,
    pub proxy_item_count: Option<u64>,
    pub proxy_duration_p50_ms: Option<f64>,
    pub proxy_duration_p95_ms: Option<f64>,
    pub proxy_success_rate: Option<f64>,
    pub proxy_failure_rate: Option<f64>,
    pub trend: Vec<VectorTrendBucket>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorTelemetryBaseline {
    pub session_count: u32,
    pub average_input_tokens_per_second: Option<f64>,
    pub average_items_per_second: Option<f64>,
    pub task_duration_p95_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticFinding {
    pub id: String,
    pub severity: String,
    pub confidence: f64,
    pub title: String,
    pub summary: String,
    pub evidence: Vec<String>,
    pub recommendation: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TelemetrySessionDetail {
    pub samples: Vec<TelemetrySampleSummary>,
    pub requests: Vec<InferenceRequestSummary>,
    pub analysis: TelemetrySessionAnalysis,
    pub diagnostics: Vec<DiagnosticFinding>,
}

#[derive(Debug, Clone)]
pub struct SlotSnapshotRecord {
    pub slot_id: u32,
    pub is_processing: bool,
    pub n_ctx: u32,
    pub n_past: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct InferenceRequestRecord {
    pub task_id: u32,
    pub slot_id: u32,
    pub prompt_tokens: Option<u64>,
    pub prompt_time_ms: Option<f64>,
    pub prompt_tps: Option<f64>,
    pub generated_tokens: Option<u64>,
    pub generation_time_ms: Option<f64>,
    pub generation_tps: Option<f64>,
    pub total_tokens: Option<u64>,
    pub total_time_ms: Option<f64>,
    pub spec_accept_rate: Option<f64>,
    pub spec_accepted: Option<u64>,
    pub spec_generated: Option<u64>,
    pub spec_gen_time_ms: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ProxyRequestRecord {
    pub task_id: u32,
    pub model: Option<String>,
    pub target_instance_id: String,
    pub http_status: Option<u16>,
    pub duration_ms: f64,
    pub error_text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VectorActivityRecord {
    pub source: VectorEventSource,
    pub source_event_id: i64,
    pub workload: ModelWorkload,
    pub endpoint: Option<String>,
    pub started_at: i64,
    pub completed_at: i64,
    pub duration_ms: f64,
    pub item_count: u64,
    pub input_tokens: Option<u64>,
    pub http_status: Option<u16>,
    pub error_text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LlamaMetricSample {
    pub tokens_per_sec: f64,
    pub prompt_tokens: u64,
    pub gen_tokens: u64,
    pub decode_calls_total: u64,
    pub max_tokens_observed: u64,
    pub prompt_tokens_per_sec: f64,
    pub requests_processing: u64,
    pub requests_deferred: u64,
    pub busy_slots_per_decode: f64,
}

enum TelemetryWrite {
    Metric {
        session_id: String,
        instance_id: String,
        timestamp: i64,
        system: SystemMetrics,
        llama: Option<LlamaMetricSample>,
    },
    Inference {
        session_id: String,
        completed_at: i64,
        record: InferenceRequestRecord,
    },
    Proxy {
        session_id: String,
        completed_at: i64,
        record: ProxyRequestRecord,
    },
    Vector {
        session_id: String,
        record: VectorActivityRecord,
    },
    Slots {
        session_id: String,
        instance_id: String,
        timestamp: i64,
        slots: Vec<SlotSnapshotRecord>,
    },
    Flush(mpsc::Sender<Result<(), String>>),
    Prune {
        before: i64,
        waiter: mpsc::Sender<Result<u32, String>>,
    },
}

fn telemetry_db_path() -> PathBuf {
    crate::utils::get_data_dir().join("telemetry.db")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn current_time_ms() -> i64 {
    now_ms()
}

fn open_raw_connection() -> Result<Connection, String> {
    let path = telemetry_db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("无法创建遥测数据目录: {}", e))?;
    }
    let conn = Connection::open(path).map_err(|e| format!("无法打开遥测数据库: {}", e))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("failed to configure telemetry busy timeout: {e}"))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| format!("无法启用遥测数据库外键: {}", e))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| format!("failed to configure telemetry synchronous mode: {e}"))?;
    Ok(conn)
}

pub(crate) fn initialize_telemetry_storage() -> Result<(), String> {
    if TELEMETRY_SCHEMA_READY.load(Ordering::Acquire) {
        return Ok(());
    }
    let _guard = TELEMETRY_SCHEMA_LOCK.lock().unwrap();
    if TELEMETRY_SCHEMA_READY.load(Ordering::Acquire) {
        return Ok(());
    }
    let conn = open_raw_connection()?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("无法启用遥测数据库 WAL: {}", e))?;
    init_schema(&conn)?;
    TELEMETRY_SCHEMA_READY.store(true, Ordering::Release);
    Ok(())
}

fn open_connection() -> Result<Connection, String> {
    initialize_telemetry_storage()?;
    open_raw_connection()
}

fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS run_sessions (
            id TEXT PRIMARY KEY,
            instance_id TEXT NOT NULL,
            instance_name TEXT NOT NULL,
            model_name TEXT NOT NULL,
            model_path TEXT NOT NULL,
            engine_id TEXT NOT NULL,
            backend TEXT NOT NULL,
            config_hash TEXT NOT NULL,
            command_line TEXT NOT NULL,
            workload TEXT NOT NULL DEFAULT 'inference',
            started_at INTEGER NOT NULL,
            stopped_at INTEGER,
            exit_code INTEGER,
            stop_reason TEXT
        );

        CREATE TABLE IF NOT EXISTS metric_samples (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            instance_id TEXT NOT NULL,
            ts INTEGER NOT NULL,
            cpu_percent REAL,
            memory_mb REAL,
            gpu_percent REAL,
            vram_used_mb REAL,
            vram_total_mb REAL,
            system_cpu_percent REAL,
            system_memory_used_mb REAL,
            system_memory_total_mb REAL,
            gpu_vendor TEXT,
            gpu_name TEXT,
            tokens_per_sec REAL,
            prompt_tokens_total INTEGER,
            generated_tokens_total INTEGER,
            requests_total INTEGER,
            decode_calls_total INTEGER,
            max_tokens_observed INTEGER,
            prompt_tokens_per_sec REAL,
            requests_processing INTEGER,
            requests_deferred INTEGER,
            busy_slots_per_decode REAL,
            FOREIGN KEY(session_id) REFERENCES run_sessions(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_metric_samples_session_ts ON metric_samples(session_id, ts);
        CREATE INDEX IF NOT EXISTS idx_metric_samples_instance_ts ON metric_samples(instance_id, ts);
        CREATE INDEX IF NOT EXISTS idx_run_sessions_instance_started ON run_sessions(instance_id, started_at DESC);
        CREATE INDEX IF NOT EXISTS idx_metric_samples_ts ON metric_samples(ts DESC);
        CREATE INDEX IF NOT EXISTS idx_run_sessions_started ON run_sessions(started_at DESC);
        CREATE INDEX IF NOT EXISTS idx_run_sessions_stopped ON run_sessions(stopped_at);
        CREATE INDEX IF NOT EXISTS idx_run_sessions_active_started
            ON run_sessions(started_at DESC) WHERE stopped_at IS NULL;

        CREATE TABLE IF NOT EXISTS inference_requests (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            task_id INTEGER NOT NULL,
            slot_id INTEGER NOT NULL,
            completed_at INTEGER NOT NULL,
            source TEXT NOT NULL DEFAULT 'log',
            model TEXT,
            target_instance_id TEXT,
            http_status INTEGER,
            error_text TEXT,
            prompt_tokens INTEGER,
            prompt_time_ms REAL,
            prompt_tps REAL,
            generated_tokens INTEGER,
            generation_time_ms REAL,
            generation_tps REAL,
            total_tokens INTEGER,
            total_time_ms REAL,
            spec_accept_rate REAL,
            spec_accepted INTEGER,
            spec_generated INTEGER,
            spec_gen_time_ms REAL,
            UNIQUE(session_id, task_id),
            FOREIGN KEY(session_id) REFERENCES run_sessions(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_inference_requests_session_completed
            ON inference_requests(session_id, completed_at DESC);
        CREATE INDEX IF NOT EXISTS idx_inference_requests_completed
            ON inference_requests(completed_at);
        CREATE TABLE IF NOT EXISTS slot_snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            instance_id TEXT NOT NULL,
            ts INTEGER NOT NULL,
            slot_id INTEGER NOT NULL,
            is_processing INTEGER NOT NULL,
            n_ctx INTEGER NOT NULL,
            n_past INTEGER,
            FOREIGN KEY(session_id) REFERENCES run_sessions(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_slot_snapshots_session_ts
            ON slot_snapshots(session_id, ts);
        CREATE INDEX IF NOT EXISTS idx_slot_snapshots_ts ON slot_snapshots(ts);

        CREATE TABLE IF NOT EXISTS vector_activity_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            source TEXT NOT NULL CHECK(source IN ('log', 'proxy')),
            source_event_id INTEGER NOT NULL,
            workload TEXT NOT NULL CHECK(workload IN ('embedding', 'reranker')),
            endpoint TEXT,
            started_at INTEGER NOT NULL,
            completed_at INTEGER NOT NULL,
            duration_ms REAL NOT NULL,
            item_count INTEGER NOT NULL DEFAULT 1 CHECK(item_count >= 0),
            input_tokens INTEGER CHECK(input_tokens IS NULL OR input_tokens >= 0),
            http_status INTEGER,
            error_text TEXT,
            UNIQUE(session_id, source, source_event_id),
            FOREIGN KEY(session_id) REFERENCES run_sessions(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_vector_activity_session_completed
            ON vector_activity_events(session_id, completed_at);
        CREATE INDEX IF NOT EXISTS idx_vector_activity_session_source_completed
            ON vector_activity_events(session_id, source, completed_at);
        CREATE INDEX IF NOT EXISTS idx_vector_activity_session_source_duration
            ON vector_activity_events(session_id, source, duration_ms);
        CREATE INDEX IF NOT EXISTS idx_vector_activity_completed
            ON vector_activity_events(completed_at);
        "#,
    )
    .map_err(|e| format!("无法初始化遥测数据库: {}", e))?;
    let stored_version = conn
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|e| format!("无法读取遥测 schema 版本: {e}"))?;
    if stored_version < SCHEMA_VERSION {
        migrate_inference_request_columns(conn)?;
        migrate_vector_schema(conn)?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_inference_requests_source_completed
            ON inference_requests(source, completed_at DESC)",
        [],
    )
    .map_err(|e| format!("无法创建推理请求来源索引: {}", e))?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|e| format!("无法写入遥测 schema 版本: {}", e))?;
    Ok(())
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| format!("failed to read {table} schema: {e}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("failed to query {table} schema: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to parse {table} schema: {e}"))
}

fn migrate_inference_request_columns(conn: &Connection) -> Result<(), String> {
    let columns = table_columns(conn, "inference_requests")?;

    let additions = [
        ("source", "TEXT NOT NULL DEFAULT 'log'"),
        ("model", "TEXT"),
        ("target_instance_id", "TEXT"),
        ("http_status", "INTEGER"),
        ("error_text", "TEXT"),
    ];
    for (name, definition) in additions {
        if !columns.iter().any(|column| column == name) {
            conn.execute(
                &format!(
                    "ALTER TABLE inference_requests ADD COLUMN {} {}",
                    name, definition
                ),
                [],
            )
            .map_err(|e| format!("failed to migrate inference request schema: {}", e))?;
        }
    }
    Ok(())
}

fn migrate_vector_schema(conn: &Connection) -> Result<(), String> {
    let session_columns = table_columns(conn, "run_sessions")?;
    if !session_columns.iter().any(|column| column == "workload") {
        conn.execute(
            "ALTER TABLE run_sessions ADD COLUMN workload TEXT NOT NULL DEFAULT 'inference'",
            [],
        )
        .map_err(|e| format!("failed to add telemetry workload column: {e}"))?;
    }
    let sessions = {
        let mut statement = conn
            .prepare(
                "SELECT id, command_line FROM run_sessions
                 WHERE workload IS NULL OR workload = '' OR workload = 'inference'",
            )
            .map_err(|e| format!("failed to prepare telemetry workload backfill: {e}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("failed to query telemetry workload backfill: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("failed to read telemetry workload backfill: {e}"))?
    };
    for (session_id, command_line) in sessions {
        let workload = ModelWorkload::from_command_line(&command_line);
        conn.execute(
            "UPDATE run_sessions SET workload = ?2 WHERE id = ?1",
            params![session_id, workload.as_str()],
        )
        .map_err(|e| format!("failed to backfill telemetry workload: {e}"))?;
    }
    let stored_workloads = {
        let mut statement = conn
            .prepare("SELECT id, workload FROM run_sessions")
            .map_err(|e| format!("failed to prepare telemetry workload validation: {e}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("failed to query telemetry workload validation: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("failed to read telemetry workload validation: {e}"))?
    };
    for (session_id, stored) in stored_workloads {
        let canonical = ModelWorkload::from_storage(&stored).as_str();
        if canonical != stored {
            conn.execute(
                "UPDATE run_sessions SET workload = ?2 WHERE id = ?1",
                params![session_id, canonical],
            )
            .map_err(|e| format!("failed to normalize telemetry workload: {e}"))?;
        }
    }

    let sample_columns = table_columns(conn, "metric_samples")?;
    for (name, definition) in [
        ("decode_calls_total", "INTEGER"),
        ("max_tokens_observed", "INTEGER"),
        ("gpu_name", "TEXT"),
    ] {
        if !sample_columns.iter().any(|column| column == name) {
            conn.execute(
                &format!("ALTER TABLE metric_samples ADD COLUMN {name} {definition}"),
                [],
            )
            .map_err(|e| format!("failed to migrate metric sample schema: {e}"))?;
        }
    }
    Ok(())
}

struct RunSessionStart<'a> {
    id: &'a str,
    instance_id: &'a str,
    instance_name: &'a str,
    model_path: &'a str,
    engine_id: &'a str,
    backend: &'a str,
    config_hash: &'a str,
    command_line: &'a str,
    workload: ModelWorkload,
    started_at: i64,
}

fn insert_run_session(conn: &Connection, session: &RunSessionStart<'_>) -> Result<(), String> {
    let model_name = std::path::Path::new(session.model_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(session.model_path);
    conn.execute(
        "INSERT INTO run_sessions
            (id, instance_id, instance_name, model_name, model_path, engine_id, backend,
             config_hash, command_line, workload, started_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            session.id,
            session.instance_id,
            session.instance_name,
            model_name,
            session.model_path,
            session.engine_id,
            session.backend,
            session.config_hash,
            session.command_line,
            session.workload.as_str(),
            session.started_at,
        ],
    )
    .map_err(|e| format!("无法创建运行遥测会话: {e}"))?;
    Ok(())
}

fn session_workload_from_connection(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<ModelWorkload>, String> {
    let stored = conn
        .query_row(
            "SELECT workload FROM run_sessions WHERE id = ?1",
            params![session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("无法读取运行遥测工作负载: {e}"))?;
    Ok(stored.map(|value| ModelWorkload::from_storage(&value)))
}

pub fn session_workload(session_id: Option<&str>) -> Result<ModelWorkload, String> {
    let Some(session_id) = session_id else {
        return Ok(ModelWorkload::Inference);
    };
    let conn = open_connection()?;
    Ok(session_workload_from_connection(&conn, session_id)?.unwrap_or(ModelWorkload::Inference))
}

pub fn begin_run_session(
    instance_id: &str,
    config: &InstanceConfig,
    backend: &str,
    config_hash: &str,
    command_line: &str,
    workload: ModelWorkload,
) -> Result<String, String> {
    let conn = open_connection()?;
    let id = uuid::Uuid::new_v4().to_string();
    insert_run_session(
        &conn,
        &RunSessionStart {
            id: &id,
            instance_id,
            instance_name: &config.name,
            model_path: &config.model_path,
            engine_id: &config.engine_id,
            backend,
            config_hash,
            command_line,
            workload,
            started_at: now_ms(),
        },
    )?;
    Ok(id)
}

pub fn finish_run_session(
    session_id: Option<&str>,
    exit_code: Option<i32>,
    stop_reason: &str,
) -> Result<(), String> {
    let Some(session_id) = session_id else {
        return Ok(());
    };
    let conn = open_connection()?;
    conn.execute(
        "UPDATE run_sessions
         SET stopped_at = COALESCE(stopped_at, ?2), exit_code = COALESCE(exit_code, ?3), stop_reason = COALESCE(stop_reason, ?4)
         WHERE id = ?1",
        params![session_id, now_ms(), exit_code, stop_reason],
    )
    .map_err(|e| format!("无法结束运行遥测会话: {}", e))?;
    Ok(())
}

fn telemetry_writer() -> Result<mpsc::SyncSender<TelemetryWrite>, String> {
    let mut writer = TELEMETRY_WRITER.lock().unwrap();
    if let Some(sender) = writer.as_ref() {
        return Ok(sender.clone());
    }
    initialize_telemetry_storage()?;
    let (sender, receiver) = mpsc::sync_channel(TELEMETRY_WRITE_QUEUE_CAPACITY);
    std::thread::Builder::new()
        .name("telemetry-writer".to_string())
        .spawn(move || telemetry_writer_loop(receiver))
        .map_err(|e| format!("failed to start telemetry writer: {e}"))?;
    *writer = Some(sender.clone());
    Ok(sender)
}

fn reset_telemetry_writer() {
    *TELEMETRY_WRITER.lock().unwrap() = None;
}

fn enqueue_telemetry(write: TelemetryWrite) -> Result<(), String> {
    let writer = telemetry_writer()?;
    match writer.try_send(write) {
        Ok(()) => Ok(()),
        Err(mpsc::TrySendError::Disconnected(pending)) => {
            reset_telemetry_writer();
            match telemetry_writer()?.try_send(pending) {
                Ok(()) => Ok(()),
                Err(mpsc::TrySendError::Full(_)) => Err(
                    "telemetry write queue is full; sample was dropped to protect runtime latency"
                        .to_string(),
                ),
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    Err("telemetry writer is unavailable after restart".to_string())
                }
            }
        }
        Err(error) => Err(match error {
            mpsc::TrySendError::Full(_) => {
                "telemetry write queue is full; sample was dropped to protect runtime latency"
                    .to_string()
            }
            mpsc::TrySendError::Disconnected(_) => "telemetry writer is unavailable".to_string(),
        }),
    }
}

pub(crate) fn flush_telemetry_writer() -> Result<(), String> {
    let (sender, receiver) = mpsc::channel();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut flush = TelemetryWrite::Flush(sender);
    loop {
        let writer = telemetry_writer()?;
        match writer.try_send(flush) {
            Ok(()) => break,
            Err(mpsc::TrySendError::Full(pending)) if std::time::Instant::now() < deadline => {
                flush = pending;
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(mpsc::TrySendError::Full(_)) => {
                return Err("timed out while queueing telemetry flush".to_string());
            }
            Err(mpsc::TrySendError::Disconnected(pending)) => {
                reset_telemetry_writer();
                flush = pending;
                if std::time::Instant::now() >= deadline {
                    return Err("telemetry writer is unavailable".to_string());
                }
            }
        }
    }
    receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "timed out while flushing telemetry writes".to_string())?
}

fn telemetry_writer_loop(receiver: mpsc::Receiver<TelemetryWrite>) {
    let mut conn = match open_connection() {
        Ok(conn) => conn,
        Err(error) => {
            eprintln!("Telemetry writer failed to open database: {error}");
            return;
        }
    };
    let mut pending_control = None;
    let mut last_write_error: Option<String> = None;
    loop {
        let first = match pending_control.take() {
            Some(write) => write,
            None => match receiver.recv() {
                Ok(write) => write,
                Err(_) => break,
            },
        };
        match first {
            TelemetryWrite::Flush(waiter) => {
                let result = last_write_error.take().map_or(Ok(()), Err);
                let _ = waiter.send(result);
                continue;
            }
            TelemetryWrite::Prune { before, waiter } => {
                let mut result = if let Some(error) = last_write_error.take() {
                    Err(error)
                } else {
                    prune_connection(&mut conn, before)
                };
                if matches!(result, Ok(affected) if affected > 0) {
                    if let Err(error) =
                        conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE); PRAGMA optimize;")
                    {
                        result = Err(format!("Telemetry cleanup maintenance failed: {error}"));
                    }
                }
                let _ = waiter.send(result);
                continue;
            }
            write => {
                let mut batch = Vec::with_capacity(TELEMETRY_WRITE_BATCH_SIZE);
                batch.push(write);
                while batch.len() < TELEMETRY_WRITE_BATCH_SIZE {
                    match receiver.try_recv() {
                        Ok(write @ (TelemetryWrite::Flush(_) | TelemetryWrite::Prune { .. })) => {
                            pending_control = Some(write);
                            break;
                        }
                        Ok(write) => batch.push(write),
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => break,
                    }
                }
                let transaction = match conn.transaction() {
                    Ok(transaction) => transaction,
                    Err(error) => {
                        let message =
                            format!("Telemetry writer failed to begin transaction: {error}");
                        eprintln!("{message}");
                        last_write_error = Some(message);
                        continue;
                    }
                };
                let mut apply_error = None;
                for write in batch {
                    if let Err(error) = apply_telemetry_write(&transaction, write) {
                        apply_error = Some(error);
                        break;
                    }
                }
                let result = if let Some(error) = apply_error {
                    transaction
                        .rollback()
                        .map_err(|rollback| {
                            format!("Telemetry write failed: {error}; rollback failed: {rollback}")
                        })
                        .and(Err(format!(
                            "Telemetry write batch was rolled back: {error}"
                        )))
                } else {
                    transaction.commit().map_err(|error| {
                        format!("Telemetry writer failed to commit transaction: {error}")
                    })
                };
                if let Err(error) = result {
                    eprintln!("{error}");
                    last_write_error = Some(error);
                }
            }
        }
    }
}

fn apply_telemetry_write(conn: &Connection, write: TelemetryWrite) -> Result<(), String> {
    match write {
        TelemetryWrite::Metric {
            session_id,
            instance_id,
            timestamp,
            system,
            llama,
        } => insert_metric_sample(
            conn,
            &session_id,
            &instance_id,
            timestamp,
            &system,
            llama.as_ref(),
        ),
        TelemetryWrite::Inference {
            session_id,
            completed_at,
            record,
        } => insert_inference_request(conn, &session_id, completed_at, &record),
        TelemetryWrite::Proxy {
            session_id,
            completed_at,
            record,
        } => insert_proxy_request(conn, &session_id, completed_at, &record),
        TelemetryWrite::Vector { session_id, record } => {
            record_vector_activity_in_connection(conn, &session_id, &record).map(|_| ())
        }
        TelemetryWrite::Slots {
            session_id,
            instance_id,
            timestamp,
            slots,
        } => insert_slot_snapshots(conn, &session_id, &instance_id, timestamp, &slots),
        TelemetryWrite::Flush(_) | TelemetryWrite::Prune { .. } => {
            Err("telemetry control message reached the data writer".into())
        }
    }
}

pub fn record_metric_sample(
    session_id: Option<&str>,
    instance_id: &str,
    system: &SystemMetrics,
    llama: Option<&LlamaMetricSample>,
) -> Result<(), String> {
    let Some(session_id) = session_id else {
        return Ok(());
    };
    enqueue_telemetry(TelemetryWrite::Metric {
        session_id: session_id.to_string(),
        instance_id: instance_id.to_string(),
        timestamp: now_ms(),
        system: system.clone(),
        llama: llama.cloned(),
    })
}

fn insert_metric_sample(
    conn: &Connection,
    session_id: &str,
    instance_id: &str,
    timestamp: i64,
    system: &SystemMetrics,
    llama: Option<&LlamaMetricSample>,
) -> Result<(), String> {
    let cpu = Some(system.cpu_percent as f64);
    let memory = Some(system.memory_mb);
    let gpu = system.gpu_percent.map(|v| v as f64);
    let sys_cpu = system.system_cpu_percent.map(|v| v as f64);
    let tokens_per_sec = llama.map(|metric| {
        if metric.requests_processing > 0 {
            metric.tokens_per_sec
        } else {
            0.0
        }
    });
    let prompt_tokens_per_sec = llama.map(|metric| {
        if metric.requests_processing > 0 {
            metric.prompt_tokens_per_sec
        } else {
            0.0
        }
    });
    conn.execute(
        "INSERT INTO metric_samples
            (session_id, instance_id, ts, cpu_percent, memory_mb, gpu_percent, vram_used_mb, vram_total_mb,
             system_cpu_percent, system_memory_used_mb, system_memory_total_mb, gpu_vendor, gpu_name, tokens_per_sec,
             prompt_tokens_total, generated_tokens_total, requests_total, decode_calls_total,
             max_tokens_observed, prompt_tokens_per_sec, requests_processing, requests_deferred,
             busy_slots_per_decode)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                 ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
        params![
            session_id,
            instance_id,
            timestamp,
            cpu,
            memory,
            gpu,
            system.vram_used_mb,
            system.vram_total_mb,
            sys_cpu,
            system.system_memory_used_mb,
            system.system_memory_total_mb,
            system.gpu_vendor.as_deref(),
            system.gpu_name.as_deref(),
            tokens_per_sec,
            llama.map(|m| m.prompt_tokens as i64),
            llama.map(|m| m.gen_tokens as i64),
            llama.map(|m| m.decode_calls_total as i64),
            llama.map(|m| m.decode_calls_total as i64),
            llama.map(|m| m.max_tokens_observed as i64),
            prompt_tokens_per_sec,
            llama.map(|m| m.requests_processing as i64),
            llama.map(|m| m.requests_deferred as i64),
            llama.map(|m| m.busy_slots_per_decode),
        ],
    )
    .map_err(|e| format!("无法写入遥测采样: {}", e))?;
    Ok(())
}

pub fn record_inference_request(
    session_id: Option<&str>,
    record: &InferenceRequestRecord,
) -> Result<(), String> {
    let Some(session_id) = session_id else {
        return Ok(());
    };
    enqueue_telemetry(TelemetryWrite::Inference {
        session_id: session_id.to_string(),
        completed_at: now_ms(),
        record: record.clone(),
    })
}

fn insert_inference_request(
    conn: &Connection,
    session_id: &str,
    completed_at: i64,
    record: &InferenceRequestRecord,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO inference_requests
            (session_id, task_id, slot_id, completed_at, source, prompt_tokens, prompt_time_ms, prompt_tps,
             generated_tokens, generation_time_ms, generation_tps, total_tokens, total_time_ms,
             spec_accept_rate, spec_accepted, spec_generated, spec_gen_time_ms)
         VALUES (?1, ?2, ?3, ?4, 'log', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
         ON CONFLICT(session_id, task_id) DO UPDATE SET
             slot_id = excluded.slot_id,
             completed_at = inference_requests.completed_at,
             source = excluded.source,
             prompt_tokens = excluded.prompt_tokens,
             prompt_time_ms = excluded.prompt_time_ms,
             prompt_tps = excluded.prompt_tps,
             generated_tokens = excluded.generated_tokens,
             generation_time_ms = excluded.generation_time_ms,
             generation_tps = excluded.generation_tps,
             total_tokens = excluded.total_tokens,
             total_time_ms = excluded.total_time_ms,
             spec_accept_rate = excluded.spec_accept_rate,
             spec_accepted = excluded.spec_accepted,
             spec_generated = excluded.spec_generated,
             spec_gen_time_ms = excluded.spec_gen_time_ms",
        params![
            session_id,
            record.task_id,
            record.slot_id,
            completed_at,
            record.prompt_tokens.map(|v| v as i64),
            record.prompt_time_ms,
            record.prompt_tps,
            record.generated_tokens.map(|v| v as i64),
            record.generation_time_ms,
            record.generation_tps,
            record.total_tokens.map(|v| v as i64),
            record.total_time_ms,
            record.spec_accept_rate,
            record.spec_accepted.map(|v| v as i64),
            record.spec_generated.map(|v| v as i64),
            record.spec_gen_time_ms,
        ],
    )
    .map_err(|e| format!("无法写入推理请求遥测: {}", e))?;
    Ok(())
}

pub fn record_proxy_request(
    session_id: Option<&str>,
    record: &ProxyRequestRecord,
) -> Result<(), String> {
    let Some(session_id) = session_id else {
        return Ok(());
    };
    enqueue_telemetry(TelemetryWrite::Proxy {
        session_id: session_id.to_string(),
        completed_at: now_ms(),
        record: record.clone(),
    })
}

fn insert_proxy_request(
    conn: &Connection,
    session_id: &str,
    completed_at: i64,
    record: &ProxyRequestRecord,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO inference_requests
            (session_id, task_id, slot_id, completed_at, source, model, target_instance_id,
             http_status, error_text, total_time_ms)
         VALUES (?1, ?2, 0, ?3, 'proxy', ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(session_id, task_id) DO UPDATE SET
             completed_at = excluded.completed_at,
             source = excluded.source,
             model = excluded.model,
             target_instance_id = excluded.target_instance_id,
             http_status = excluded.http_status,
             error_text = excluded.error_text,
             total_time_ms = excluded.total_time_ms",
        params![
            session_id,
            record.task_id,
            completed_at,
            record.model.as_deref(),
            record.target_instance_id,
            record.http_status.map(|v| v as i64),
            record.error_text.as_deref(),
            record.duration_ms,
        ],
    )
    .map_err(|e| format!("failed to write proxy request telemetry: {}", e))?;
    Ok(())
}

pub fn record_vector_activity(
    session_id: Option<&str>,
    record: &VectorActivityRecord,
) -> Result<bool, String> {
    let Some(session_id) = session_id else {
        return Ok(false);
    };
    if !record.workload.is_vector() {
        return Err("vector activity requires a vector workload".to_string());
    }
    enqueue_telemetry(TelemetryWrite::Vector {
        session_id: session_id.to_string(),
        record: record.clone(),
    })?;
    Ok(true)
}

fn record_vector_activity_in_connection(
    conn: &Connection,
    session_id: &str,
    record: &VectorActivityRecord,
) -> Result<bool, String> {
    if !record.workload.is_vector() {
        return Err("vector activity requires a vector workload".to_string());
    }
    let session_workload = session_workload_from_connection(conn, session_id)?
        .ok_or_else(|| "vector activity session does not exist".to_string())?;
    if session_workload != record.workload {
        return Err(format!(
            "vector activity workload {} does not match session workload {}",
            record.workload.as_str(),
            session_workload.as_str()
        ));
    }
    if record.source_event_id < 0 {
        return Err("vector activity source event id must be non-negative".to_string());
    }
    if record.started_at < 0 || record.completed_at < record.started_at {
        return Err("vector activity timestamps are invalid".to_string());
    }
    if !record.duration_ms.is_finite() || record.duration_ms < 0.0 {
        return Err("vector activity duration is invalid".to_string());
    }
    let item_count = i64::try_from(record.item_count)
        .map_err(|_| "vector activity item count is too large".to_string())?;
    let input_tokens = record
        .input_tokens
        .map(i64::try_from)
        .transpose()
        .map_err(|_| "vector activity input token count is too large".to_string())?;
    let error_text = record.error_text.as_deref().map(sanitize_telemetry_error);
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO vector_activity_events
                (session_id, source, source_event_id, workload, endpoint, started_at,
                 completed_at, duration_ms, item_count, input_tokens, http_status, error_text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                session_id,
                record.source.as_str(),
                record.source_event_id,
                record.workload.as_str(),
                record.endpoint.as_deref(),
                record.started_at,
                record.completed_at,
                record.duration_ms,
                item_count,
                input_tokens,
                record.http_status,
                error_text,
            ],
        )
        .map_err(|e| format!("无法写入向量活动遥测: {e}"))?;
    Ok(inserted == 1)
}

fn sanitize_telemetry_error(value: &str) -> String {
    value
        .chars()
        .take(512)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

pub fn record_slot_snapshots(
    session_id: Option<&str>,
    instance_id: &str,
    slots: &[SlotSnapshotRecord],
) -> Result<(), String> {
    let Some(session_id) = session_id else {
        return Ok(());
    };
    if slots.is_empty() {
        return Ok(());
    }
    enqueue_telemetry(TelemetryWrite::Slots {
        session_id: session_id.to_string(),
        instance_id: instance_id.to_string(),
        timestamp: now_ms(),
        slots: slots.to_vec(),
    })
}

fn insert_slot_snapshots(
    conn: &Connection,
    session_id: &str,
    instance_id: &str,
    timestamp: i64,
    slots: &[SlotSnapshotRecord],
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "INSERT INTO slot_snapshots
                (session_id, instance_id, ts, slot_id, is_processing, n_ctx, n_past)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(|e| format!("无法准备 slot 遥测写入: {}", e))?;
    for slot in slots {
        stmt.execute(params![
            session_id,
            instance_id,
            timestamp,
            slot.slot_id,
            if slot.is_processing { 1 } else { 0 },
            slot.n_ctx,
            slot.n_past.map(|v| v as i64),
        ])
        .map_err(|e| format!("无法写入 slot 遥测: {}", e))?;
    }
    Ok(())
}

pub async fn get_telemetry_overview() -> Result<TelemetryOverview, String> {
    tokio::task::spawn_blocking(|| {
        let conn = open_connection()?;
        let since = now_ms() - 24 * 60 * 60 * 1000;
        let active_sessions: u32 = conn
            .query_row("SELECT COUNT(*) FROM run_sessions WHERE stopped_at IS NULL", [], |row| row.get::<_, i64>(0))
            .map(|v| v as u32)
            .unwrap_or(0);
        let sessions_24h: u32 = conn
            .query_row("SELECT COUNT(*) FROM run_sessions WHERE started_at >= ?1", params![since], |row| row.get::<_, i64>(0))
            .map(|v| v as u32)
            .unwrap_or(0);
        let avg_tokens_per_sec_24h = conn
            .query_row(
                "SELECT AVG(tokens_per_sec) FROM metric_samples WHERE ts >= ?1 AND tokens_per_sec > 0",
                params![since],
                |row| row.get::<_, Option<f64>>(0),
            )
            .unwrap_or(None)
            .unwrap_or(0.0);
        let peak_vram_mb_24h = conn
            .query_row(
                "SELECT MAX(vram_used_mb) FROM metric_samples WHERE ts >= ?1",
                params![since],
                |row| row.get::<_, Option<f64>>(0),
            )
            .unwrap_or(None)
            .unwrap_or(0.0);
        let latest_samples = latest_samples(&conn, 8)?;
        Ok(TelemetryOverview {
            active_sessions,
            sessions_24h,
            avg_tokens_per_sec_24h,
            peak_vram_mb_24h,
            latest_samples,
        })
    })
    .await
    .map_err(|e| format!("遥测概览查询失败: {}", e))?
}

pub async fn list_telemetry_sessions(
    limit: Option<u32>,
) -> Result<Vec<TelemetrySessionSummary>, String> {
    let limit = limit.unwrap_or(20).clamp(1, 200);
    tokio::task::spawn_blocking(move || {
        let conn = open_connection()?;
        query_telemetry_sessions(&conn, now_ms(), limit)
    })
    .await
    .map_err(|e| format!("遥测会话查询失败: {}", e))?
}

fn query_telemetry_sessions(
    conn: &Connection,
    current_time: i64,
    limit: u32,
) -> Result<Vec<TelemetrySessionSummary>, String> {
    let mut stmt = conn
        .prepare(
            "WITH recent_sessions AS MATERIALIZED (
                SELECT * FROM run_sessions ORDER BY started_at DESC LIMIT ?2
             ), metric_stats AS (
                SELECT session_id,
                    AVG(CASE WHEN tokens_per_sec > 0 THEN tokens_per_sec END) AS avg_tps,
                    MAX(vram_used_mb) AS peak_vram,
                    COUNT(*) AS sample_count
                FROM metric_samples
                WHERE session_id IN (SELECT id FROM recent_sessions)
                GROUP BY session_id
             )
             SELECT
                s.id, s.instance_id, s.instance_name, s.model_name, s.model_path, s.engine_id,
                s.backend, s.workload, s.started_at, s.stopped_at,
                CASE
                    WHEN COALESCE(s.stopped_at, ?1) <= s.started_at THEN 0
                    ELSE CAST((COALESCE(s.stopped_at, ?1) - s.started_at) / 1000 AS INTEGER)
                END,
                COALESCE(m.avg_tps, 0),
                COALESCE(m.peak_vram, 0),
                COALESCE(m.sample_count, 0),
                s.stop_reason
             FROM recent_sessions s
             LEFT JOIN metric_stats m ON m.session_id = s.id
             ORDER BY s.started_at DESC
             ",
        )
        .map_err(|e| format!("无法准备遥测会话查询: {e}"))?;
    let rows = stmt
        .query_map(params![current_time, limit], |row| {
            Ok(TelemetrySessionSummary {
                id: row.get(0)?,
                instance_id: row.get(1)?,
                instance_name: row.get(2)?,
                model_name: row.get(3)?,
                model_path: row.get(4)?,
                engine_id: row.get(5)?,
                backend: row.get(6)?,
                workload: row.get(7)?,
                started_at: row.get(8)?,
                stopped_at: row.get(9)?,
                duration_secs: row.get(10)?,
                avg_tokens_per_sec: row.get(11)?,
                peak_vram_mb: row.get(12)?,
                sample_count: row.get::<_, i64>(13)?.max(0) as u32,
                stop_reason: row.get(14)?,
            })
        })
        .map_err(|e| format!("无法查询遥测会话: {e}"))?;
    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row.map_err(|e| format!("无法读取遥测会话: {e}"))?);
    }
    Ok(sessions)
}

pub async fn get_telemetry_session_samples(
    session_id: String,
    limit: Option<u32>,
) -> Result<Vec<TelemetrySampleSummary>, String> {
    let limit = limit.unwrap_or(120).clamp(1, 1000);
    tokio::task::spawn_blocking(move || {
        let conn = open_connection()?;
        samples_for_session(&conn, &session_id, limit)
    })
    .await
    .map_err(|e| format!("遥测采样查询失败: {}", e))?
}

pub async fn get_telemetry_session_detail(
    session_id: String,
    sample_limit: Option<u32>,
    request_limit: Option<u32>,
) -> Result<TelemetrySessionDetail, String> {
    let sample_limit = sample_limit.unwrap_or(220).clamp(1, 1_000);
    let request_limit = request_limit.unwrap_or(18).clamp(1, 200);
    tokio::task::spawn_blocking(move || {
        flush_telemetry_writer()?;
        let conn = open_connection()?;
        let samples = samples_for_session(&conn, &session_id, sample_limit)?;
        let requests = inference_requests_for_session(&conn, &session_id, request_limit)?;
        let analysis = query_session_analysis(&conn, &session_id)?;
        let diagnostics = query_session_diagnostics_with_analysis(&conn, &session_id, &analysis)?;
        Ok(TelemetrySessionDetail {
            samples,
            requests,
            analysis,
            diagnostics,
        })
    })
    .await
    .map_err(|e| format!("遥测会话详情查询失败: {e}"))?
}

pub async fn prune_telemetry(retention_days: Option<u32>) -> Result<u32, String> {
    let days = retention_days.unwrap_or(14).clamp(1, 365);
    tokio::task::spawn_blocking(move || prune_telemetry_storage(days))
        .await
        .map_err(|e| format!("遥测清理失败: {}", e))?
}

fn prune_connection(conn: &mut Connection, before: i64) -> Result<u32, String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("Unable to begin telemetry cleanup transaction: {}", e))?;
    let mut affected = 0_u32;
    for (table, column) in [
        ("metric_samples", "ts"),
        ("slot_snapshots", "ts"),
        ("inference_requests", "completed_at"),
        ("vector_activity_events", "completed_at"),
    ] {
        affected += tx
            .execute(
                &format!("DELETE FROM {} WHERE {} < ?1", table, column),
                params![before],
            )
            .map_err(|e| format!("Unable to prune {}: {}", table, e))? as u32;
    }
    affected += tx
        .execute(
            "DELETE FROM run_sessions WHERE stopped_at IS NOT NULL AND stopped_at < ?1",
            params![before],
        )
        .map_err(|e| format!("Unable to prune telemetry sessions: {}", e))? as u32;
    tx.commit()
        .map_err(|e| format!("Unable to commit telemetry cleanup: {}", e))?;
    Ok(affected)
}

pub(crate) fn prune_telemetry_storage(retention_days: u32) -> Result<u32, String> {
    let days = retention_days.clamp(1, 365);
    let before = now_ms() - days as i64 * 24 * 60 * 60 * 1000;
    let (sender, receiver) = mpsc::channel();
    telemetry_writer()?
        .send(TelemetryWrite::Prune {
            before,
            waiter: sender,
        })
        .map_err(|_| "telemetry writer is unavailable during cleanup".to_string())?;
    receiver
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "timed out while pruning telemetry storage".to_string())?
}

pub async fn get_telemetry_session_analysis(
    session_id: String,
) -> Result<TelemetrySessionAnalysis, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_connection()?;
        query_session_analysis(&conn, &session_id)
    })
    .await
    .map_err(|e| format!("会话分析查询失败: {}", e))?
}

#[derive(Debug)]
struct SessionMetricStats {
    sample_count: u32,
    avg_tps: f64,
    max_tps: f64,
    avg_gpu: f64,
    avg_instance_cpu: f64,
    avg_system_cpu: f64,
    max_vram_ratio: f64,
    avg_processing: f64,
    max_processing: u32,
    avg_deferred: f64,
    max_deferred: u32,
    avg_busy_slots_metric: f64,
    max_busy_slots_metric: f64,
}

fn query_session_analysis(
    conn: &Connection,
    session_id: &str,
) -> Result<TelemetrySessionAnalysis, String> {
    let request_stats = conn
        .query_row(
            "SELECT
                COUNT(*),
                COALESCE(AVG(prompt_tokens), 0),
                COALESCE(AVG(generated_tokens), 0),
                COALESCE(AVG(total_tokens), 0),
                COALESCE(AVG(prompt_tps), 0),
                COALESCE(AVG(generation_tps), 0),
                COALESCE(AVG(total_time_ms), 0),
                COALESCE(MAX(total_tokens), 0)
             FROM inference_requests
             WHERE session_id = ?1 AND source = 'log'",
            params![session_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, f64>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        )
        .map_err(|e| format!("无法查询请求分析摘要: {}", e))?;

    let slot_stats = conn
        .query_row(
            "WITH per_ts AS (
                SELECT
                    ts,
                    SUM(CASE WHEN is_processing != 0 THEN 1 ELSE 0 END) AS busy_slots,
                    SUM(CASE WHEN is_processing = 0 AND n_ctx > 0 THEN 1 ELSE 0 END) AS cached_slots,
                    MAX(COALESCE(n_past, n_ctx, 0)) AS context_tokens
                FROM slot_snapshots
                WHERE session_id = ?1
                GROUP BY ts
             )
             SELECT
                COALESCE(AVG(busy_slots), 0),
                COALESCE(MAX(busy_slots), 0),
                COALESCE(AVG(cached_slots), 0),
                COALESCE(MAX(context_tokens), 0),
                COUNT(*)
             FROM per_ts",
            params![session_id],
            |row| {
                Ok((
                    row.get::<_, f64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .map_err(|e| format!("无法查询 slot 分析摘要: {}", e))?;

    let speculative_analysis = query_speculative_analysis(conn, session_id)?;
    let vector_analysis = query_vector_analysis(conn, session_id)?;
    let vector_baseline = if vector_analysis.is_some() {
        query_vector_baseline_for_session(conn, session_id)?
    } else {
        None
    };
    Ok(TelemetrySessionAnalysis {
        request_count: request_stats.0.max(0) as u32,
        avg_prompt_tokens: request_stats.1,
        avg_generated_tokens: request_stats.2,
        avg_total_tokens: request_stats.3,
        avg_prompt_tps: request_stats.4,
        avg_generation_tps: request_stats.5,
        avg_total_time_ms: request_stats.6,
        max_total_tokens: request_stats.7.max(0) as u64,
        avg_busy_slots: slot_stats.0,
        max_busy_slots: slot_stats.1.max(0) as u32,
        avg_cached_slots: slot_stats.2,
        max_context_tokens: slot_stats.3.max(0) as u32,
        slot_sample_count: slot_stats.4.max(0) as u32,
        speculative_analysis,
        vector_analysis,
        vector_baseline,
    })
}

fn query_speculative_analysis(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<SpeculativeTelemetryAnalysis>, String> {
    let stats = conn
        .query_row(
            "SELECT
                COUNT(*),
                COALESCE(SUM(spec_accepted), 0),
                COALESCE(SUM(spec_generated), 0),
                CASE
                    WHEN COALESCE(SUM(spec_generated), 0) > 0
                    THEN 1.0 * COALESCE(SUM(spec_accepted), 0) / SUM(spec_generated)
                    ELSE AVG(spec_accept_rate)
                END,
                AVG(spec_gen_time_ms)
             FROM inference_requests
             WHERE session_id = ?1
               AND source = 'log'
               AND (
                    spec_accept_rate IS NOT NULL
                    OR spec_accepted IS NOT NULL
                    OR spec_generated IS NOT NULL
                    OR spec_gen_time_ms IS NOT NULL
               )",
            params![session_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<f64>>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                ))
            },
        )
        .map_err(|e| format!("无法查询推测解码分析摘要: {}", e))?;

    if stats.0 <= 0 {
        return Ok(None);
    }

    Ok(Some(SpeculativeTelemetryAnalysis {
        request_count: stats.0 as u32,
        acceptance_rate: stats.3.map(|value| value.clamp(0.0, 1.0)),
        accepted_tokens: stats.1.max(0) as u64,
        generated_tokens: stats.2.max(0) as u64,
        avg_generation_time_ms: stats.4.filter(|value| value.is_finite() && *value >= 0.0),
    }))
}

fn query_vector_analysis(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<VectorTelemetryAnalysis>, String> {
    query_vector_analysis_inner(conn, session_id, true)
}

#[derive(Debug, Default)]
struct VectorSourceTotals {
    event_count: u64,
    item_count: u64,
    input_tokens: Option<u64>,
    success_count: u64,
    first_started_at: Option<i64>,
    last_completed_at: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
struct VectorQueryScope<'a> {
    session_id: &'a str,
    source: VectorEventSource,
    workload: ModelWorkload,
    range_start: i64,
    range_end: i64,
}

fn query_vector_source_totals(
    conn: &Connection,
    scope: VectorQueryScope<'_>,
) -> Result<VectorSourceTotals, String> {
    let values = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(item_count), 0), SUM(input_tokens),
                    COALESCE(SUM(CASE
                        WHEN error_text IS NULL AND http_status >= 200 AND http_status < 300
                        THEN 1 ELSE 0 END), 0),
                    MIN(started_at), MAX(completed_at)
             FROM vector_activity_events
             WHERE session_id = ?1 AND source = ?2 AND workload = ?3
               AND completed_at >= ?4 AND completed_at <= ?5
               AND duration_ms >= 0",
            params![
                scope.session_id,
                scope.source.as_str(),
                scope.workload.as_str(),
                scope.range_start,
                scope.range_end
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            },
        )
        .map_err(|e| format!("无法汇总向量活动: {e}"))?;
    Ok(VectorSourceTotals {
        event_count: u64::try_from(values.0).map_err(|_| "向量活动数量无效".to_string())?,
        item_count: u64::try_from(values.1).map_err(|_| "向量活动项目总数无效".to_string())?,
        input_tokens: values
            .2
            .map(u64::try_from)
            .transpose()
            .map_err(|_| "向量活动输入 token 总数无效".to_string())?,
        success_count: u64::try_from(values.3).map_err(|_| "向量代理成功数无效".to_string())?,
        first_started_at: values.4,
        last_completed_at: values.5,
    })
}

fn query_vector_duration_percentile(
    conn: &Connection,
    scope: VectorQueryScope<'_>,
    event_count: u64,
    percentile: u64,
) -> Result<Option<f64>, String> {
    if event_count == 0 || percentile == 0 || percentile > 100 {
        return Ok(None);
    }
    let rank = ((u128::from(event_count) * u128::from(percentile)).saturating_add(99) / 100)
        .max(1)
        .saturating_sub(1);
    let offset = i64::try_from(rank).map_err(|_| "向量分位数样本过多".to_string())?;
    let value = conn
        .query_row(
            "SELECT duration_ms
             FROM vector_activity_events
             WHERE session_id = ?1 AND source = ?2 AND workload = ?3
               AND completed_at >= ?4 AND completed_at <= ?5
               AND duration_ms >= 0
             ORDER BY duration_ms
             LIMIT 1 OFFSET ?6",
            params![
                scope.session_id,
                scope.source.as_str(),
                scope.workload.as_str(),
                scope.range_start,
                scope.range_end,
                offset
            ],
            |row| row.get::<_, f64>(0),
        )
        .optional()
        .map_err(|e| format!("无法读取向量活动分位数: {e}"))?;
    Ok(value.filter(|duration| duration.is_finite() && *duration >= 0.0))
}

fn query_vector_trend(
    conn: &Connection,
    session_id: &str,
    workload: ModelWorkload,
    range_start: i64,
    range_end: i64,
    bucket_ms: i64,
    input_tokens_available: bool,
) -> Result<Vec<VectorTrendBucket>, String> {
    let range_ms = range_end.saturating_sub(range_start);
    let bucket_count = range_ms.saturating_add(bucket_ms - 1) / bucket_ms;
    let bucket_count =
        usize::try_from(bucket_count).map_err(|_| "向量趋势时间范围过大".to_string())?;
    if bucket_count == 0 || bucket_count > 120 {
        return Ok(Vec::new());
    }
    let last_bucket = i64::try_from(bucket_count.saturating_sub(1))
        .map_err(|_| "向量趋势桶数量无效".to_string())?;
    let mut buckets = (0..bucket_count)
        .map(|index| {
            let timestamp = range_start.saturating_add((index as i64).saturating_mul(bucket_ms));
            VectorTrendBucket {
                timestamp,
                input_tokens_per_second: input_tokens_available.then_some(0.0),
                items_per_second: 0.0,
            }
        })
        .collect::<Vec<_>>();
    let mut statement = conn
        .prepare(
            "SELECT CASE WHEN completed_at >= ?5 THEN ?7
                         ELSE (completed_at - ?4) / ?6 END AS bucket_index,
                    COALESCE(SUM(item_count), 0), SUM(input_tokens)
             FROM vector_activity_events
             WHERE session_id = ?1 AND source = ?3 AND workload = ?2
               AND completed_at >= ?4 AND completed_at <= ?5
               AND duration_ms >= 0
             GROUP BY bucket_index
             ORDER BY bucket_index",
        )
        .map_err(|e| format!("无法准备向量趋势查询: {e}"))?;
    let rows = statement
        .query_map(
            params![
                session_id,
                workload.as_str(),
                VectorEventSource::Log.as_str(),
                range_start,
                range_end,
                bucket_ms,
                last_bucket
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .map_err(|e| format!("无法查询向量趋势: {e}"))?;
    for row in rows {
        let (index, item_count, input_tokens) =
            row.map_err(|e| format!("无法读取向量趋势: {e}"))?;
        let Ok(index) = usize::try_from(index) else {
            continue;
        };
        let Some(bucket) = buckets.get_mut(index) else {
            continue;
        };
        let bucket_end = bucket.timestamp.saturating_add(bucket_ms).min(range_end);
        let seconds = (bucket_end.saturating_sub(bucket.timestamp)) as f64 / 1_000.0;
        if seconds <= 0.0 {
            continue;
        }
        let item_count =
            u64::try_from(item_count).map_err(|_| "向量趋势项目总数无效".to_string())?;
        bucket.items_per_second = item_count as f64 / seconds;
        if input_tokens_available {
            let input_tokens = input_tokens
                .map(u64::try_from)
                .transpose()
                .map_err(|_| "向量趋势输入 token 总数无效".to_string())?
                .unwrap_or(0);
            bucket.input_tokens_per_second = Some(input_tokens as f64 / seconds);
        }
    }
    Ok(buckets)
}

fn query_vector_analysis_inner(
    conn: &Connection,
    session_id: &str,
    include_trend: bool,
) -> Result<Option<VectorTelemetryAnalysis>, String> {
    let session = conn
        .query_row(
            "SELECT workload, started_at, stopped_at FROM run_sessions WHERE id = ?1",
            params![session_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|e| format!("无法读取向量遥测会话: {e}"))?;
    let Some((stored_workload, started_at, stopped_at)) = session else {
        return Ok(None);
    };
    let workload = ModelWorkload::from_storage(&stored_workload);
    if !workload.is_vector() {
        return Ok(None);
    }

    let range_start = started_at.max(0);
    let range_end = stopped_at
        .unwrap_or_else(now_ms)
        .max(range_start.saturating_add(1_000));
    let range_ms = range_end.saturating_sub(range_start).max(1_000);
    let bucket_ms = range_ms.saturating_add(119) / 120;
    let bucket_ms = bucket_ms.max(1_000);
    let log_scope = VectorQueryScope {
        session_id,
        source: VectorEventSource::Log,
        workload,
        range_start,
        range_end,
    };
    let proxy_scope = VectorQueryScope {
        session_id,
        source: VectorEventSource::Proxy,
        workload,
        range_start,
        range_end,
    };
    let log = query_vector_source_totals(conn, log_scope)?;
    let proxy = query_vector_source_totals(conn, proxy_scope)?;
    let log_available = log.event_count > 0;
    let proxy_available = proxy.event_count > 0;
    let rate_end = if stopped_at.is_some() {
        log.last_completed_at.unwrap_or(range_end)
    } else {
        range_end
    };
    let rate_start = log
        .first_started_at
        .unwrap_or(rate_end)
        .max(rate_end.saturating_sub(VECTOR_RATE_WINDOW_MS));
    let rate_window_ms = rate_end.saturating_sub(rate_start).max(1_000);
    let rate_window_seconds = rate_window_ms as f64 / 1_000.0;
    let rate_log = if log_available {
        query_vector_source_totals(
            conn,
            VectorQueryScope {
                session_id,
                source: VectorEventSource::Log,
                workload,
                range_start: rate_start,
                range_end: rate_end,
            },
        )?
    } else {
        VectorSourceTotals::default()
    };
    let task_duration_p50_ms =
        query_vector_duration_percentile(conn, log_scope, log.event_count, 50)?;
    let task_duration_p95_ms =
        query_vector_duration_percentile(conn, log_scope, log.event_count, 95)?;
    let proxy_duration_p50_ms =
        query_vector_duration_percentile(conn, proxy_scope, proxy.event_count, 50)?;
    let proxy_duration_p95_ms =
        query_vector_duration_percentile(conn, proxy_scope, proxy.event_count, 95)?;
    let trend = if include_trend && log_available {
        query_vector_trend(
            conn,
            session_id,
            workload,
            range_start,
            range_end,
            bucket_ms,
            log.input_tokens.is_some(),
        )?
    } else {
        Vec::new()
    };
    let proxy_failure_count = proxy.event_count.saturating_sub(proxy.success_count);

    Ok(Some(VectorTelemetryAnalysis {
        workload: workload.as_str().to_string(),
        log_available,
        proxy_available,
        completed_items: log_available.then_some(log.item_count),
        input_tokens: log.input_tokens,
        average_input_tokens_per_second: if rate_log.event_count == 0 && log.input_tokens.is_some()
        {
            Some(0.0)
        } else {
            rate_log
                .input_tokens
                .map(|tokens| tokens as f64 / rate_window_seconds)
        },
        average_items_per_second: log_available
            .then_some(rate_log.item_count as f64 / rate_window_seconds),
        task_duration_p50_ms,
        task_duration_p95_ms,
        proxy_request_count: proxy_available.then_some(proxy.event_count),
        proxy_item_count: proxy_available.then_some(proxy.item_count),
        proxy_duration_p50_ms,
        proxy_duration_p95_ms,
        proxy_success_rate: proxy_available
            .then_some(proxy.success_count as f64 / proxy.event_count as f64),
        proxy_failure_rate: proxy_available
            .then_some(proxy_failure_count as f64 / proxy.event_count as f64),
        trend,
    }))
}

fn query_vector_baseline(
    conn: &Connection,
    session_id: &str,
    model_name: &str,
    model_path: &str,
    workload: ModelWorkload,
    backend: &str,
) -> Result<VectorTelemetryBaseline, String> {
    conn.query_row(
        "WITH eligible AS (
             SELECT id, workload, started_at, stopped_at
             FROM run_sessions
             WHERE id != ?1
               AND workload = ?2
               AND backend = ?3
               AND stopped_at IS NOT NULL
               AND (model_path = ?4 OR model_name = ?5)
             ORDER BY started_at DESC
             LIMIT 50
         ), events AS (
             SELECT e.session_id, e.started_at, e.completed_at, e.duration_ms,
                    e.item_count, e.input_tokens
             FROM vector_activity_events e
             JOIN eligible s ON s.id = e.session_id
             WHERE e.source = 'log'
               AND e.workload = s.workload
               AND e.completed_at >= s.started_at
               AND e.completed_at <= s.stopped_at
               AND e.duration_ms >= 0
         ), bounds AS (
             SELECT session_id, MIN(started_at) AS first_started_at,
                    MAX(completed_at) AS last_completed_at
             FROM events
             GROUP BY session_id
         ), rate_sums AS (
             SELECT e.session_id, SUM(e.item_count) AS item_count,
                    SUM(e.input_tokens) AS input_tokens
             FROM events e
             JOIN bounds b ON b.session_id = e.session_id
             WHERE e.completed_at >= MAX(b.first_started_at, b.last_completed_at - ?6)
               AND e.completed_at <= b.last_completed_at
             GROUP BY e.session_id
         ), ranked AS (
             SELECT session_id, duration_ms,
                    ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY duration_ms) AS rank,
                    COUNT(*) OVER (PARTITION BY session_id) AS sample_count
             FROM events
         ), percentiles AS (
             SELECT session_id,
                    MAX(CASE WHEN rank = ((sample_count * 95 + 99) / 100)
                             THEN duration_ms END) AS p95_ms
             FROM ranked
             GROUP BY session_id
         ), per_session AS (
             SELECT b.session_id,
                    r.input_tokens / MAX((b.last_completed_at - MAX(b.first_started_at, b.last_completed_at - ?6)) / 1000.0, 1.0)
                        AS input_rate,
                    r.item_count / MAX((b.last_completed_at - MAX(b.first_started_at, b.last_completed_at - ?6)) / 1000.0, 1.0)
                        AS item_rate,
                    p.p95_ms
             FROM bounds b
             JOIN rate_sums r ON r.session_id = b.session_id
             JOIN percentiles p ON p.session_id = b.session_id
         )
         SELECT COUNT(*), AVG(input_rate), AVG(item_rate), AVG(p95_ms)
         FROM per_session",
        params![
            session_id,
            workload.as_str(),
            backend,
            model_path,
            model_name,
            VECTOR_RATE_WINDOW_MS,
        ],
        |row| {
            Ok(VectorTelemetryBaseline {
                session_count: row.get::<_, i64>(0)?.max(0) as u32,
                average_input_tokens_per_second: row.get(1)?,
                average_items_per_second: row.get(2)?,
                task_duration_p95_ms: row.get(3)?,
            })
        },
    )
    .map_err(|e| format!("无法查询向量历史基线: {e}"))
}

fn query_vector_baseline_for_session(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<VectorTelemetryBaseline>, String> {
    let session = conn
        .query_row(
            "SELECT model_name, model_path, workload, backend
             FROM run_sessions WHERE id = ?1",
            params![session_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| format!("无法读取向量基线会话: {e}"))?;
    let Some((model_name, model_path, stored_workload, backend)) = session else {
        return Ok(None);
    };
    let workload = ModelWorkload::from_storage(&stored_workload);
    if !workload.is_vector() {
        return Ok(None);
    }
    query_vector_baseline(
        conn,
        session_id,
        &model_name,
        &model_path,
        workload,
        &backend,
    )
    .map(Some)
}

fn query_session_metric_stats(
    conn: &Connection,
    session_id: &str,
) -> Result<SessionMetricStats, String> {
    conn.query_row(
        "SELECT
            COUNT(*),
            COALESCE(AVG(CASE WHEN tokens_per_sec > 0 THEN tokens_per_sec END), 0),
            COALESCE(MAX(tokens_per_sec), 0),
            COALESCE(AVG(gpu_percent), 0),
            COALESCE(AVG(cpu_percent), 0),
            COALESCE(AVG(system_cpu_percent), 0),
            COALESCE(MAX(CASE WHEN vram_total_mb > 0 THEN vram_used_mb / vram_total_mb ELSE 0 END), 0),
            COALESCE(AVG(requests_processing), 0),
            COALESCE(MAX(requests_processing), 0),
            COALESCE(AVG(requests_deferred), 0),
            COALESCE(MAX(requests_deferred), 0),
            COALESCE(AVG(busy_slots_per_decode), 0),
            COALESCE(MAX(busy_slots_per_decode), 0)
         FROM metric_samples
         WHERE session_id = ?1",
        params![session_id],
        |row| {
            Ok(SessionMetricStats {
                sample_count: row.get::<_, i64>(0)?.max(0) as u32,
                avg_tps: row.get(1)?,
                max_tps: row.get(2)?,
                avg_gpu: row.get(3)?,
                avg_instance_cpu: row.get(4)?,
                avg_system_cpu: row.get(5)?,
                max_vram_ratio: row.get(6)?,
                avg_processing: row.get(7)?,
                max_processing: row.get::<_, i64>(8)?.max(0) as u32,
                avg_deferred: row.get(9)?,
                max_deferred: row.get::<_, i64>(10)?.max(0) as u32,
                avg_busy_slots_metric: row.get(11)?,
                max_busy_slots_metric: row.get(12)?,
            })
        },
    )
    .map_err(|e| format!("无法查询会话指标摘要: {}", e))
}

fn diagnostic_finding(
    id: &str,
    severity: &str,
    confidence: f64,
    title: &str,
    summary: &str,
    evidence: Vec<String>,
    recommendation: Vec<String>,
) -> DiagnosticFinding {
    DiagnosticFinding {
        id: id.to_string(),
        severity: severity.to_string(),
        confidence,
        title: title.to_string(),
        summary: summary.to_string(),
        evidence,
        recommendation,
    }
}

fn fmt_diag_rate(value: f64) -> String {
    if !value.is_finite() || value <= 0.0 {
        "--".to_string()
    } else {
        format!("{:.1} tok/s", value)
    }
}

fn fmt_diag_percent(value: f64) -> String {
    if !value.is_finite() {
        "--".to_string()
    } else {
        format!("{:.0}%", value)
    }
}

#[derive(Debug)]
struct SessionDiagnosticContext {
    model_name: String,
    engine_id: String,
    backend: String,
    workload: ModelWorkload,
}

pub async fn get_telemetry_session_diagnostics(
    session_id: String,
) -> Result<Vec<DiagnosticFinding>, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_connection()?;
        query_session_diagnostics(&conn, &session_id)
    })
    .await
    .map_err(|e| format!("会话诊断查询失败: {}", e))?
}

fn query_session_diagnostics(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<DiagnosticFinding>, String> {
    let analysis = query_session_analysis(conn, session_id)?;
    query_session_diagnostics_with_analysis(conn, session_id, &analysis)
}

fn query_session_diagnostics_with_analysis(
    conn: &Connection,
    session_id: &str,
    analysis: &TelemetrySessionAnalysis,
) -> Result<Vec<DiagnosticFinding>, String> {
    let session = conn
        .query_row(
            "SELECT model_name, engine_id, backend, workload
                 FROM run_sessions WHERE id = ?1",
            params![session_id],
            |row| {
                Ok(SessionDiagnosticContext {
                    model_name: row.get(0)?,
                    engine_id: row.get(1)?,
                    backend: row.get(2)?,
                    workload: ModelWorkload::from_storage(&row.get::<_, String>(3)?),
                })
            },
        )
        .optional()
        .map_err(|e| format!("无法读取诊断会话: {}", e))?
        .ok_or_else(|| "找不到对应的遥测会话".to_string())?;
    let metrics = query_session_metric_stats(conn, session_id)?;
    if session.workload.is_vector() {
        return build_vector_diagnostics(&session, analysis, &metrics);
    }
    let baseline = conn
        .query_row(
            "WITH per_session AS (
                    SELECT s.id, AVG(m.tokens_per_sec) AS avg_tps, COUNT(*) AS samples
                    FROM run_sessions s
                    JOIN metric_samples m ON m.session_id = s.id
                    WHERE s.model_name = ?1 AND s.id != ?2 AND m.tokens_per_sec > 0
                    GROUP BY s.id
                    HAVING samples >= 3
                 )
                 SELECT COALESCE(AVG(avg_tps), 0), COUNT(*) FROM per_session",
            params![session.model_name.as_str(), session_id],
            |row| Ok((row.get::<_, f64>(0)?, row.get::<_, i64>(1)?.max(0) as u32)),
        )
        .map_err(|e| format!("无法查询历史基线: {}", e))?;

    let mut findings = Vec::new();

    if metrics.sample_count == 0 {
        findings.push(diagnostic_finding(
            "no_metric_samples",
            "info",
            0.98,
            "遥测样本不足",
            "当前会话还没有可用于分析的资源或吞吐采样。",
            vec!["采样数 0".to_string()],
            vec!["保持实例运行一段时间，或发起一次推理请求后再查看诊断。".to_string()],
        ));
        return Ok(findings);
    }

    if baseline.1 >= 2
        && baseline.0 > 0.0
        && metrics.avg_tps > 0.0
        && metrics.avg_tps < baseline.0 * 0.75
    {
        findings.push(diagnostic_finding(
            "throughput_regression",
            "warning",
            0.82,
            "吞吐低于历史基线",
            "同一模型的本次平均吞吐明显低于历史会话，可能存在参数、后端或系统负载差异。",
            vec![
                format!("本次平均 {}", fmt_diag_rate(metrics.avg_tps)),
                format!("历史基线 {}", fmt_diag_rate(baseline.0)),
                format!("历史样本 {} 个会话", baseline.1),
            ],
            vec![
                "对比本次与历史会话的引擎、后端、上下文长度、批处理参数和 GPU 层数。".to_string(),
                "检查是否有其他进程占用 CPU/GPU 或显存。".to_string(),
            ],
        ));
    }

    if metrics.max_vram_ratio >= 0.92 {
        findings.push(diagnostic_finding(
            "vram_pressure",
            if metrics.max_vram_ratio >= 0.98 {
                "critical"
            } else {
                "warning"
            },
            0.9,
            "显存压力偏高",
            "会话期间显存占用接近上限，可能导致后续请求失败、回退到 CPU 或响应抖动。",
            vec![format!(
                "显存峰值占比 {}",
                fmt_diag_percent(metrics.max_vram_ratio * 100.0)
            )],
            vec![
                "降低上下文长度、并发槽位或 GPU offload 层数。".to_string(),
                "避免同一 GPU 上同时运行多个大模型实例。".to_string(),
            ],
        ));
    }

    if metrics.max_deferred > 0 || metrics.avg_deferred >= 0.5 {
        findings.push(diagnostic_finding(
            "queue_pressure",
            "warning",
            0.86,
            "请求排队压力",
            "监控到延迟队列，说明当前实例吞吐或 slot 配置可能无法覆盖请求峰值。",
            vec![
                format!("最大延迟请求 {}", metrics.max_deferred),
                format!("平均处理中 {:.1}", metrics.avg_processing),
                format!("最大处理中 {}", metrics.max_processing),
            ],
            vec![
                "根据业务目标增加并发 slot，或拆分为多个实例分担请求。".to_string(),
                "如果延迟集中出现在长上下文请求，优先限制单次请求上下文规模。".to_string(),
            ],
        ));
    }

    if metrics.avg_tps > 0.0
        && metrics.avg_gpu < 35.0
        && (metrics.avg_system_cpu > 55.0 || metrics.avg_instance_cpu > 55.0)
    {
        findings.push(diagnostic_finding(
                "gpu_underutilized",
                "warning",
                0.74,
                "GPU 利用率偏低",
                "吞吐产生时 GPU 平均利用率较低，同时 CPU 负载较高，可能存在 CPU 侧瓶颈或 GPU offload 不充分。",
                vec![
                    format!("GPU 平均 {}", fmt_diag_percent(metrics.avg_gpu)),
                    format!("系统 CPU 平均 {}", fmt_diag_percent(metrics.avg_system_cpu)),
                    format!("实例 CPU 平均 {}", fmt_diag_percent(metrics.avg_instance_cpu)),
                ],
                vec![
                    "检查引擎是否使用了预期的 CUDA/ROCm/Vulkan 后端。".to_string(),
                    "尝试增加 GPU offload 层数，或调整 batch/ubatch 配置。".to_string(),
                ],
            ));
    }

    if analysis.request_count == 0 {
        findings.push(diagnostic_finding(
                "no_request_records",
                "info",
                0.78,
                "暂无请求级样本",
                "资源采样已经写入，但还没有解析到已完成请求，因此暂时无法判断 prompt 和 generation 阶段瓶颈。",
                vec![format!("资源采样 {} 条", metrics.sample_count)],
                vec!["完成至少一次推理请求后，诊断会补充 token、耗时和吞吐拆解。".to_string()],
            ));
    } else {
        if analysis.avg_prompt_tps > 0.0
            && analysis.avg_generation_tps > 0.0
            && analysis.avg_prompt_tps < analysis.avg_generation_tps * 0.55
        {
            findings.push(diagnostic_finding(
                    "prompt_eval_bottleneck",
                    "info",
                    0.72,
                    "提示词处理偏慢",
                    "prompt 阶段吞吐显著低于 generation 阶段，长提示词或 KV cache 命中率可能影响首 token 延迟。",
                    vec![
                        format!("Prompt {}", fmt_diag_rate(analysis.avg_prompt_tps)),
                        format!("Generation {}", fmt_diag_rate(analysis.avg_generation_tps)),
                    ],
                    vec![
                        "关注长提示词请求的首 token 延迟，必要时优化 prompt 模板或启用可复用上下文。".to_string(),
                        "结合 slot 缓存数据判断是否需要调整 cache/keep 参数。".to_string(),
                    ],
                ));
        }

        if analysis.avg_total_time_ms > 60_000.0 {
            findings.push(diagnostic_finding(
                "long_request_latency",
                "warning",
                0.8,
                "请求耗时偏长",
                "平均请求总耗时超过 60 秒，交互式使用可能感到明显等待。",
                vec![
                    format!("平均耗时 {:.1} 秒", analysis.avg_total_time_ms / 1000.0),
                    format!("平均生成 token {:.0}", analysis.avg_generated_tokens),
                ],
                vec![
                    "检查 max_tokens、上下文长度和并发设置是否符合目标场景。".to_string(),
                    "若是批处理场景，可以将该会话单独标记为长任务基线。".to_string(),
                ],
            ));
        }
    }

    if analysis.slot_sample_count > 0 && analysis.avg_cached_slots >= 1.0 {
        findings.push(diagnostic_finding(
            "slot_cache_observed",
            "info",
            0.68,
            "检测到 slot 缓存",
            "会话中存在空闲但带上下文的 slot，说明实例可能正在保留 KV cache。",
            vec![
                format!("平均缓存 slot {:.1}", analysis.avg_cached_slots),
                format!("slot 采样 {} 条", analysis.slot_sample_count),
            ],
            vec![
                "如果重复对话较多，这是有益信号；如果显存紧张，可降低保留上下文或并发槽位。"
                    .to_string(),
            ],
        ));
    }

    if analysis.max_context_tokens >= 32_768 {
        findings.push(diagnostic_finding(
            "large_context_window",
            "info",
            0.66,
            "上下文窗口较大",
            "该会话观测到较大的上下文窗口，吞吐和显存占用可能受上下文规模影响。",
            vec![format!("最大上下文 {} tokens", analysis.max_context_tokens)],
            vec!["如不需要长上下文，降低 ctx-size 通常可以改善显存压力和延迟。".to_string()],
        ));
    }

    if findings.iter().all(|item| item.severity == "info") && analysis.request_count > 0 {
        findings.insert(
            0,
            diagnostic_finding(
                "session_healthy",
                "success",
                0.76,
                "未发现明显瓶颈",
                "基于当前采样、请求和历史基线，暂未发现需要立即处理的性能异常。",
                vec![
                    format!("请求 {} 次", analysis.request_count),
                    format!("平均生成 {}", fmt_diag_rate(analysis.avg_generation_tps)),
                    format!("峰值吞吐 {}", fmt_diag_rate(metrics.max_tps)),
                ],
                vec!["可以继续积累更多会话样本，后续基线判断会更稳定。".to_string()],
            ),
        );
    }

    if findings.is_empty() {
        findings.push(diagnostic_finding(
            "baseline_collecting",
            "info",
            0.7,
            "正在建立分析基线",
            "当前数据可用于展示趋势，但历史基线或请求级样本仍偏少。",
            vec![
                format!("引擎 {} / {}", session.engine_id, session.backend),
                format!("资源采样 {} 条", metrics.sample_count),
                format!("请求 {} 次", analysis.request_count),
                format!(
                    "指标 busy slot 平均 {:.1} / 峰值 {:.1}",
                    metrics.avg_busy_slots_metric, metrics.max_busy_slots_metric
                ),
            ],
            vec![
                "多运行几组相同模型和相同参数的会话后，诊断会自动给出更明确的对比结论。"
                    .to_string(),
            ],
        ));
    }

    Ok(findings)
}

fn build_vector_diagnostics(
    session: &SessionDiagnosticContext,
    analysis: &TelemetrySessionAnalysis,
    metrics: &SessionMetricStats,
) -> Result<Vec<DiagnosticFinding>, String> {
    let vector = analysis
        .vector_analysis
        .as_ref()
        .ok_or_else(|| "向量会话缺少工作负载分析结果".to_string())?;
    let baseline = analysis
        .vector_baseline
        .as_ref()
        .ok_or_else(|| "向量会话缺少历史基线分析结果".to_string())?;
    let mut findings = Vec::new();

    if metrics.sample_count == 0 {
        findings.push(diagnostic_finding(
            "no_metric_samples",
            "info",
            0.98,
            "资源遥测样本不足",
            "当前向量会话还没有可用于分析的 CPU、GPU、显存或队列采样。",
            vec!["资源采样数 0".to_string()],
            vec!["保持实例运行一段时间并发起向量请求，资源诊断会随采样自动补充。".to_string()],
        ));
    }

    if metrics.max_vram_ratio >= 0.92 {
        findings.push(diagnostic_finding(
            "vram_pressure",
            if metrics.max_vram_ratio >= 0.98 {
                "critical"
            } else {
                "warning"
            },
            0.9,
            "显存压力偏高",
            "向量会话期间显存占用接近上限，可能导致请求失败、回退到 CPU 或延迟抖动。",
            vec![format!(
                "显存峰值占比 {}",
                fmt_diag_percent(metrics.max_vram_ratio * 100.0)
            )],
            vec![
                "降低批处理规模、并发槽位或 GPU offload 层数。".to_string(),
                "避免同一 GPU 上同时运行多个大模型实例。".to_string(),
            ],
        ));
    }

    if metrics.max_deferred > 0 || metrics.avg_deferred >= 0.5 {
        findings.push(diagnostic_finding(
            "queue_pressure",
            "warning",
            0.86,
            "向量任务排队压力",
            "监控到延迟队列，说明当前向量吞吐或 slot 配置可能无法覆盖请求峰值。",
            vec![
                format!("最大延迟请求 {}", metrics.max_deferred),
                format!("平均处理中 {:.1}", metrics.avg_processing),
                format!("最大处理中 {}", metrics.max_processing),
            ],
            vec![
                "根据批处理和延迟目标调整并发 slot，或拆分为多个实例分担请求。".to_string(),
                "检查调用方是否短时间提交了超出服务处理能力的大批量任务。".to_string(),
            ],
        ));
    }

    let has_vector_throughput = vector
        .average_input_tokens_per_second
        .is_some_and(|value| value > 0.0)
        || vector
            .average_items_per_second
            .is_some_and(|value| value > 0.0);
    if has_vector_throughput
        && metrics.avg_gpu < 35.0
        && (metrics.avg_system_cpu > 55.0 || metrics.avg_instance_cpu > 55.0)
    {
        findings.push(diagnostic_finding(
            "gpu_underutilized",
            "warning",
            0.74,
            "GPU 利用率偏低",
            "向量任务产生吞吐时 GPU 平均利用率较低，同时 CPU 负载较高，可能存在 CPU 侧瓶颈或 GPU offload 不充分。",
            vec![
                format!("GPU 平均 {}", fmt_diag_percent(metrics.avg_gpu)),
                format!(
                    "系统 CPU 平均 {}",
                    fmt_diag_percent(metrics.avg_system_cpu)
                ),
                format!(
                    "实例 CPU 平均 {}",
                    fmt_diag_percent(metrics.avg_instance_cpu)
                ),
            ],
            vec![
                "检查引擎是否使用了预期的 CUDA/ROCm/Vulkan 后端。".to_string(),
                "尝试增加 GPU offload 层数，或调整 batch/ubatch 配置。".to_string(),
            ],
        ));
    }

    if !vector.log_available || !vector.proxy_available {
        let log_state = if vector.log_available {
            "日志任务事件可用"
        } else {
            "日志任务事件不可用"
        };
        let proxy_state = if vector.proxy_available {
            "代理 HTTP 事件可用"
        } else {
            "代理 HTTP 事件不可用"
        };
        let recommendation = if !vector.log_available {
            "确认当前 llama-server 日志格式可被识别；缺少日志事件时无法统计输入吞吐和任务耗时。"
        } else {
            "直连服务时代理指标不可用属于正常情况；需要 HTTP 请求耗时和失败率时请通过实例路由访问。"
        };
        findings.push(diagnostic_finding(
            "vector_source_incomplete",
            "info",
            0.96,
            "向量指标来源不完整",
            "日志任务与代理 HTTP 使用独立统计口径，当前只能展示已采集来源对应的指标。",
            vec![log_state.to_string(), proxy_state.to_string()],
            vec![recommendation.to_string()],
        ));
    }

    let input_regressed = match (
        vector.average_input_tokens_per_second,
        baseline.average_input_tokens_per_second,
    ) {
        (Some(current), Some(historical)) if historical > 0.0 => current < historical * 0.75,
        _ => false,
    };
    let items_regressed = match (
        vector.average_items_per_second,
        baseline.average_items_per_second,
    ) {
        (Some(current), Some(historical)) if historical > 0.0 => current < historical * 0.75,
        _ => false,
    };
    if baseline.session_count >= 2 && (input_regressed || items_regressed) {
        let mut evidence = vec![format!("同类历史会话 {} 个", baseline.session_count)];
        if let (Some(current), Some(historical)) = (
            vector.average_input_tokens_per_second,
            baseline.average_input_tokens_per_second,
        ) {
            evidence.push(format!(
                "输入吞吐：本次 {:.1} / 历史 {:.1} tokens/s",
                current, historical
            ));
        }
        if let (Some(current), Some(historical)) = (
            vector.average_items_per_second,
            baseline.average_items_per_second,
        ) {
            evidence.push(format!(
                "处理项吞吐：本次 {:.1} / 历史 {:.1} 项/s",
                current, historical
            ));
        }
        findings.push(diagnostic_finding(
            "vector_throughput_regression",
            "warning",
            0.84,
            "向量吞吐低于历史基线",
            "相同模型、工作负载和后端下，本次向量吞吐明显低于历史会话。",
            evidence,
            vec![
                "对比批处理大小、并发槽位、GPU offload 和调用方请求节奏。".to_string(),
                "检查本次会话是否与其他 CPU/GPU 密集任务同时运行。".to_string(),
            ],
        ));
    }

    let latency_regressed = match (vector.task_duration_p95_ms, baseline.task_duration_p95_ms) {
        (Some(current), Some(historical)) if historical > 0.0 => current > historical * 1.35,
        _ => false,
    };
    if baseline.session_count >= 2 && latency_regressed {
        findings.push(diagnostic_finding(
            "vector_task_latency_regression",
            "warning",
            0.82,
            "向量任务 P95 耗时回退",
            "相同模型、工作负载和后端下，本次慢任务耗时明显高于历史会话。",
            vec![
                format!(
                    "本次 P95 {:.1} ms",
                    vector.task_duration_p95_ms.unwrap_or_default()
                ),
                format!(
                    "历史 P95 {:.1} ms",
                    baseline.task_duration_p95_ms.unwrap_or_default()
                ),
                format!("同类历史会话 {} 个", baseline.session_count),
            ],
            vec![
                "检查批量输入是否变大，以及是否出现资源争用或请求排队。".to_string(),
                "使用相同输入规模重复测试，以区分负载差异和运行时回退。".to_string(),
            ],
        ));
    }

    if baseline.session_count < 2 && vector.log_available {
        findings.push(diagnostic_finding(
            "vector_baseline_collecting",
            "info",
            0.72,
            "正在建立向量性能基线",
            "同模型、同工作负载和同后端的历史会话还不足，暂不判断吞吐或 P95 回退。",
            vec![format!("可用历史会话 {} 个", baseline.session_count)],
            vec!["再完成至少两次相同环境下的向量任务，会话诊断会自动启用历史比较。".to_string()],
        ));
    }

    let has_problem = findings
        .iter()
        .any(|finding| matches!(finding.severity.as_str(), "warning" | "critical"));
    if !has_problem && metrics.sample_count > 0 && vector.log_available {
        let item_label = if session.workload == ModelWorkload::Reranker {
            "文档项"
        } else {
            "向量项"
        };
        findings.insert(
            0,
            diagnostic_finding(
                "vector_session_healthy",
                "success",
                0.78,
                "未发现明显向量性能瓶颈",
                "基于当前资源、任务事件和同类历史会话，暂未发现需要立即处理的性能异常。",
                vec![
                    format!(
                        "已完成 {} {}",
                        vector.completed_items.unwrap_or_default(),
                        item_label
                    ),
                    format!(
                        "平均处理速度 {:.1} 项/s",
                        vector.average_items_per_second.unwrap_or_default()
                    ),
                    format!("资源采样 {} 条", metrics.sample_count),
                ],
                vec!["继续积累相同环境下的会话样本，可提高历史基线判断稳定性。".to_string()],
            ),
        );
    }

    Ok(findings)
}

pub async fn list_inference_requests(
    session_id: String,
    limit: Option<u32>,
) -> Result<Vec<InferenceRequestSummary>, String> {
    let limit = limit.unwrap_or(20).clamp(1, 200);
    tokio::task::spawn_blocking(move || {
        let conn = open_connection()?;
        inference_requests_for_session(&conn, &session_id, limit)
    })
    .await
    .map_err(|e| format!("推理请求查询失败: {}", e))?
}

fn inference_requests_for_session(
    conn: &Connection,
    session_id: &str,
    limit: u32,
) -> Result<Vec<InferenceRequestSummary>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT session_id, task_id, slot_id, completed_at, source, model, target_instance_id, http_status, error_text,
                    prompt_tokens, prompt_time_ms, prompt_tps,
                    generated_tokens, generation_time_ms, generation_tps, total_tokens, total_time_ms,
                    spec_accept_rate, spec_accepted, spec_generated, spec_gen_time_ms
             FROM inference_requests
             WHERE session_id = ?1
             ORDER BY completed_at DESC
             LIMIT ?2",
        )
        .map_err(|e| format!("无法准备推理请求查询: {}", e))?;
    let rows = stmt
        .query_map(params![session_id, limit], |row| {
            Ok(InferenceRequestSummary {
                session_id: row.get(0)?,
                task_id: row.get(1)?,
                slot_id: row.get(2)?,
                completed_at: row.get(3)?,
                source: row
                    .get::<_, Option<String>>(4)?
                    .unwrap_or_else(|| "log".into()),
                model: row.get(5)?,
                target_instance_id: row.get(6)?,
                http_status: row.get::<_, Option<i64>>(7)?.map(|v| v as u16),
                error_text: row.get(8)?,
                prompt_tokens: row.get::<_, Option<i64>>(9)?.map(|v| v as u64),
                prompt_time_ms: row.get(10)?,
                prompt_tps: row.get(11)?,
                generated_tokens: row.get::<_, Option<i64>>(12)?.map(|v| v as u64),
                generation_time_ms: row.get(13)?,
                generation_tps: row.get(14)?,
                total_tokens: row.get::<_, Option<i64>>(15)?.map(|v| v as u64),
                total_time_ms: row.get(16)?,
                spec_accept_rate: row.get(17)?,
                spec_accepted: row.get::<_, Option<i64>>(18)?.map(|v| v as u64),
                spec_generated: row.get::<_, Option<i64>>(19)?.map(|v| v as u64),
                spec_gen_time_ms: row.get(20)?,
            })
        })
        .map_err(|e| format!("无法查询推理请求: {}", e))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("无法读取推理请求: {}", e))
}

pub fn latest_open_session_id(instance_id: &str) -> Result<Option<String>, String> {
    let conn = open_connection()?;
    conn.query_row(
        "SELECT id FROM run_sessions WHERE instance_id = ?1 AND stopped_at IS NULL ORDER BY started_at DESC LIMIT 1",
        params![instance_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| format!("无法读取活动遥测会话: {}", e))
}

fn latest_samples(conn: &Connection, limit: u32) -> Result<Vec<TelemetrySampleSummary>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT session_id, instance_id, ts, cpu_percent, memory_mb, gpu_percent, vram_used_mb, vram_total_mb,
                    system_cpu_percent, system_memory_used_mb, system_memory_total_mb, gpu_vendor, gpu_name, tokens_per_sec,
                    prompt_tokens_per_sec, prompt_tokens_total, generated_tokens_total, requests_total,
                    decode_calls_total, max_tokens_observed, requests_processing, requests_deferred,
                    busy_slots_per_decode
             FROM metric_samples
             ORDER BY ts DESC
             LIMIT ?1",
        )
        .map_err(|e| format!("无法准备最新采样查询: {}", e))?;
    let rows = stmt
        .query_map(params![limit], sample_from_row)
        .map_err(|e| format!("无法查询最新采样: {}", e))?;
    let mut samples = Vec::new();
    for row in rows {
        samples.push(row.map_err(|e| format!("无法读取遥测采样: {}", e))?);
    }
    Ok(samples)
}

fn samples_for_session(
    conn: &Connection,
    session_id: &str,
    limit: u32,
) -> Result<Vec<TelemetrySampleSummary>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT session_id, instance_id, ts, cpu_percent, memory_mb, gpu_percent, vram_used_mb, vram_total_mb,
                    system_cpu_percent, system_memory_used_mb, system_memory_total_mb, gpu_vendor, gpu_name, tokens_per_sec,
                    prompt_tokens_per_sec, prompt_tokens_total, generated_tokens_total, requests_total,
                    decode_calls_total, max_tokens_observed, requests_processing, requests_deferred,
                    busy_slots_per_decode
             FROM metric_samples
             WHERE session_id = ?1
             ORDER BY ts DESC
             LIMIT ?2",
        )
        .map_err(|e| format!("无法准备会话采样查询: {}", e))?;
    let rows = stmt
        .query_map(params![session_id, limit], sample_from_row)
        .map_err(|e| format!("无法查询会话采样: {}", e))?;
    let mut samples = Vec::new();
    for row in rows {
        samples.push(row.map_err(|e| format!("无法读取遥测采样: {}", e))?);
    }
    samples.reverse();
    Ok(samples)
}

fn sample_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TelemetrySampleSummary> {
    Ok(TelemetrySampleSummary {
        session_id: row.get(0)?,
        instance_id: row.get(1)?,
        ts: row.get(2)?,
        cpu_percent: row.get(3)?,
        memory_mb: row.get(4)?,
        gpu_percent: row.get(5)?,
        vram_used_mb: row.get(6)?,
        vram_total_mb: row.get(7)?,
        system_cpu_percent: row.get(8)?,
        system_memory_used_mb: row.get(9)?,
        system_memory_total_mb: row.get(10)?,
        gpu_vendor: row.get(11)?,
        gpu_name: row.get(12)?,
        tokens_per_sec: row.get(13)?,
        prompt_tokens_per_sec: row.get(14)?,
        prompt_tokens_total: row.get(15)?,
        generated_tokens_total: row.get(16)?,
        requests_total: row.get(17)?,
        decode_calls_total: row.get(18)?,
        max_tokens_observed: row.get(19)?,
        requests_processing: row.get(20)?,
        requests_deferred: row.get(21)?,
        busy_slots_per_decode: row.get(22)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version_four_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;
            PRAGMA user_version = 4;
            CREATE TABLE run_sessions (
                id TEXT PRIMARY KEY,
                instance_id TEXT NOT NULL,
                instance_name TEXT NOT NULL,
                model_name TEXT NOT NULL,
                model_path TEXT NOT NULL,
                engine_id TEXT NOT NULL,
                backend TEXT NOT NULL,
                config_hash TEXT NOT NULL,
                command_line TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                stopped_at INTEGER,
                exit_code INTEGER,
                stop_reason TEXT
            );
            CREATE TABLE metric_samples (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                instance_id TEXT NOT NULL,
                ts INTEGER NOT NULL,
                requests_total INTEGER,
                FOREIGN KEY(session_id) REFERENCES run_sessions(id) ON DELETE CASCADE
            );
            INSERT INTO run_sessions
                (id, instance_id, instance_name, model_name, model_path, engine_id, backend,
                 config_hash, command_line, started_at)
            VALUES
                ('inference', 'i1', 'Inference', 'llama.gguf', 'llama.gguf', 'e1', 'cpu',
                 'h1', 'llama-server --model llama.gguf', 1),
                ('embedding', 'i2', 'Embedding', 'embed.gguf', 'embed.gguf', 'e1', 'cuda',
                 'h2', 'llama-server --embedding --model embed.gguf', 2),
                ('reranker', 'i3', 'Reranker', 'rerank.gguf', 'rerank.gguf', 'e1', 'vulkan',
                 'h3', 'llama-server --embedding --reranking --model rerank.gguf', 3);
            "#,
        )
        .unwrap();
        conn
    }

    fn column_names(conn: &Connection, table: &str) -> Vec<String> {
        let mut statement = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn seed_vector_session(
        conn: &Connection,
        id: &'static str,
        model_path: &'static str,
        backend: &'static str,
        workload: ModelWorkload,
    ) {
        insert_run_session(
            conn,
            &RunSessionStart {
                id,
                instance_id: id,
                instance_name: id,
                model_path,
                engine_id: "engine",
                backend,
                config_hash: "hash",
                command_line: match workload {
                    ModelWorkload::Inference => "llama-server",
                    ModelWorkload::Embedding => "llama-server --embedding",
                    ModelWorkload::Reranker => "llama-server --embedding --reranking",
                },
                workload,
                started_at: 0,
            },
        )
        .unwrap();
        conn.execute(
            "UPDATE run_sessions SET stopped_at = 10000 WHERE id = ?1",
            params![id],
        )
        .unwrap();
    }

    fn seed_vector_log_event(
        conn: &Connection,
        session_id: &str,
        workload: ModelWorkload,
        source_event_id: i64,
        input_tokens: u64,
        item_count: u64,
        duration_ms: f64,
    ) {
        record_vector_activity_in_connection(
            conn,
            session_id,
            &VectorActivityRecord {
                source: VectorEventSource::Log,
                source_event_id,
                workload,
                endpoint: None,
                started_at: 1_000,
                completed_at: 5_000,
                duration_ms,
                item_count,
                input_tokens: Some(input_tokens),
                http_status: None,
                error_text: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn run_session_insert_persists_workload() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        insert_run_session(
            &conn,
            &RunSessionStart {
                id: "session-reranker",
                instance_id: "instance-reranker",
                instance_name: "Reranker",
                model_path: "C:/models/reranker.gguf",
                engine_id: "engine",
                backend: "vulkan",
                config_hash: "hash",
                command_line: "llama-server --embedding --reranking",
                workload: ModelWorkload::Reranker,
                started_at: 42,
            },
        )
        .unwrap();

        let workload: String = conn
            .query_row(
                "SELECT workload FROM run_sessions WHERE id = 'session-reranker'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(workload, "reranker");
        assert_eq!(
            session_workload_from_connection(&conn, "session-reranker").unwrap(),
            Some(ModelWorkload::Reranker)
        );
        assert_eq!(
            session_workload_from_connection(&conn, "missing").unwrap(),
            None
        );
    }

    #[test]
    fn metric_sample_persists_decode_calls_and_max_tokens() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        insert_run_session(
            &conn,
            &RunSessionStart {
                id: "session-metrics",
                instance_id: "instance-metrics",
                instance_name: "Metrics",
                model_path: "model.gguf",
                engine_id: "engine",
                backend: "cpu",
                config_hash: "hash",
                command_line: "llama-server --model model.gguf",
                workload: ModelWorkload::Inference,
                started_at: 1,
            },
        )
        .unwrap();
        let system = SystemMetrics {
            cpu_percent: 1.0,
            memory_mb: 2.0,
            uptime_secs: 3,
            gpu_percent: None,
            vram_used_mb: None,
            vram_total_mb: None,
            system_cpu_percent: None,
            system_memory_total_mb: None,
            system_memory_used_mb: None,
            gpu_vendor: Some("AMD".into()),
            gpu_name: Some("AMD Radeon(TM) 8060S Graphics".into()),
        };
        let llama = LlamaMetricSample {
            tokens_per_sec: 4.0,
            prompt_tokens: 5,
            gen_tokens: 6,
            decode_calls_total: 17,
            max_tokens_observed: 4096,
            prompt_tokens_per_sec: 7.0,
            requests_processing: 1,
            requests_deferred: 0,
            busy_slots_per_decode: 1.0,
        };

        insert_metric_sample(
            &conn,
            "session-metrics",
            "instance-metrics",
            100,
            &system,
            Some(&llama),
        )
        .unwrap();

        let values: (i64, i64, i64) = conn
            .query_row(
                "SELECT requests_total, decode_calls_total, max_tokens_observed
                 FROM metric_samples WHERE session_id = 'session-metrics'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(values, (17, 17, 4096));
        let sample = samples_for_session(&conn, "session-metrics", 1).unwrap();
        assert_eq!(sample[0].gpu_vendor.as_deref(), Some("AMD"));
        assert_eq!(
            sample[0].gpu_name.as_deref(),
            Some("AMD Radeon(TM) 8060S Graphics")
        );
    }

    #[test]
    fn idle_metric_sample_zeroes_stale_throughput() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        insert_run_session(
            &conn,
            &RunSessionStart {
                id: "session-idle-metrics",
                instance_id: "instance-idle-metrics",
                instance_name: "Idle metrics",
                model_path: "model.gguf",
                engine_id: "engine",
                backend: "cpu",
                config_hash: "hash",
                command_line: "llama-server --model model.gguf",
                workload: ModelWorkload::Inference,
                started_at: 1,
            },
        )
        .unwrap();
        let system = SystemMetrics {
            cpu_percent: 1.0,
            memory_mb: 2.0,
            uptime_secs: 3,
            gpu_percent: None,
            vram_used_mb: None,
            vram_total_mb: None,
            system_cpu_percent: None,
            system_memory_total_mb: None,
            system_memory_used_mb: None,
            gpu_vendor: None,
            gpu_name: None,
        };
        let llama = LlamaMetricSample {
            tokens_per_sec: 72.6,
            prompt_tokens: 5,
            gen_tokens: 6,
            decode_calls_total: 17,
            max_tokens_observed: 4096,
            prompt_tokens_per_sec: 140.0,
            requests_processing: 0,
            requests_deferred: 0,
            busy_slots_per_decode: 0.0,
        };

        insert_metric_sample(
            &conn,
            "session-idle-metrics",
            "instance-idle-metrics",
            100,
            &system,
            Some(&llama),
        )
        .unwrap();

        let values: (f64, f64, i64) = conn
            .query_row(
                "SELECT tokens_per_sec, prompt_tokens_per_sec, requests_processing
                 FROM metric_samples WHERE session_id = 'session-idle-metrics' AND ts = 100",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(values, (0.0, 0.0, 0));

        let active_llama = LlamaMetricSample {
            tokens_per_sec: 45.0,
            prompt_tokens_per_sec: 90.0,
            requests_processing: 1,
            busy_slots_per_decode: 1.0,
            ..llama
        };
        insert_metric_sample(
            &conn,
            "session-idle-metrics",
            "instance-idle-metrics",
            200,
            &system,
            Some(&active_llama),
        )
        .unwrap();

        let stats = query_session_metric_stats(&conn, "session-idle-metrics").unwrap();
        assert_eq!(stats.sample_count, 2);
        assert_eq!(stats.avg_tps, 45.0);
        assert_eq!(stats.max_tps, 45.0);
    }

    #[test]
    fn inference_log_replay_preserves_original_completion_time() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        insert_run_session(
            &conn,
            &RunSessionStart {
                id: "session-replay",
                instance_id: "instance-replay",
                instance_name: "Replay",
                model_path: "model.gguf",
                engine_id: "engine",
                backend: "cpu",
                config_hash: "hash",
                command_line: "llama-server --model model.gguf",
                workload: ModelWorkload::Inference,
                started_at: 1,
            },
        )
        .unwrap();
        let mut record = InferenceRequestRecord {
            task_id: 7,
            slot_id: 0,
            prompt_tokens: Some(10),
            prompt_time_ms: None,
            prompt_tps: None,
            generated_tokens: Some(20),
            generation_time_ms: None,
            generation_tps: Some(30.0),
            total_tokens: Some(30),
            total_time_ms: None,
            spec_accept_rate: None,
            spec_accepted: None,
            spec_generated: None,
            spec_gen_time_ms: None,
        };

        insert_inference_request(&conn, "session-replay", 1_000, &record).unwrap();
        record.generation_tps = Some(42.0);
        insert_inference_request(&conn, "session-replay", 9_000, &record).unwrap();

        let values: (i64, f64) = conn
            .query_row(
                "SELECT completed_at, generation_tps FROM inference_requests
                 WHERE session_id = 'session-replay' AND task_id = 7",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(values, (1_000, 42.0));
    }

    #[test]
    fn vector_activity_insert_is_idempotent_and_validated() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        insert_run_session(
            &conn,
            &RunSessionStart {
                id: "session-vector-event",
                instance_id: "instance-vector-event",
                instance_name: "Vector",
                model_path: "embedding.gguf",
                engine_id: "engine",
                backend: "cpu",
                config_hash: "hash",
                command_line: "llama-server --embedding",
                workload: ModelWorkload::Embedding,
                started_at: 1,
            },
        )
        .unwrap();
        let record = VectorActivityRecord {
            source: VectorEventSource::Log,
            source_event_id: 9,
            workload: ModelWorkload::Embedding,
            endpoint: None,
            started_at: 10,
            completed_at: 20,
            duration_ms: 10.0,
            item_count: 1,
            input_tokens: Some(32),
            http_status: None,
            error_text: None,
        };

        assert!(
            record_vector_activity_in_connection(&conn, "session-vector-event", &record).unwrap()
        );
        assert!(
            !record_vector_activity_in_connection(&conn, "session-vector-event", &record).unwrap()
        );
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM vector_activity_events", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);

        let mut invalid = record.clone();
        invalid.workload = ModelWorkload::Inference;
        assert!(
            record_vector_activity_in_connection(&conn, "session-vector-event", &invalid).is_err()
        );
        invalid.workload = ModelWorkload::Reranker;
        let mismatch =
            record_vector_activity_in_connection(&conn, "session-vector-event", &invalid)
                .unwrap_err();
        assert!(mismatch.contains("session workload"));
        invalid.workload = ModelWorkload::Embedding;
        invalid.duration_ms = f64::NAN;
        assert!(
            record_vector_activity_in_connection(&conn, "session-vector-event", &invalid).is_err()
        );
    }

    #[test]
    fn vector_analysis_keeps_sources_and_availability_separate() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        for (id, workload, command_line) in [
            (
                "analysis-vector",
                ModelWorkload::Embedding,
                "llama-server --embedding",
            ),
            (
                "analysis-inference",
                ModelWorkload::Inference,
                "llama-server",
            ),
        ] {
            insert_run_session(
                &conn,
                &RunSessionStart {
                    id,
                    instance_id: id,
                    instance_name: id,
                    model_path: "model.gguf",
                    engine_id: "engine",
                    backend: "cpu",
                    config_hash: "hash",
                    command_line,
                    workload,
                    started_at: 0,
                },
            )
            .unwrap();
            conn.execute(
                "UPDATE run_sessions SET stopped_at = 10000 WHERE id = ?1",
                params![id],
            )
            .unwrap();
        }
        let log = VectorActivityRecord {
            source: VectorEventSource::Log,
            source_event_id: 1,
            workload: ModelWorkload::Embedding,
            endpoint: None,
            started_at: 1_000,
            completed_at: 5_000,
            duration_ms: 4_000.0,
            item_count: 4,
            input_tokens: Some(40),
            http_status: None,
            error_text: None,
        };
        let proxy = VectorActivityRecord {
            source: VectorEventSource::Proxy,
            source_event_id: 2,
            workload: ModelWorkload::Embedding,
            endpoint: Some("/v1/embeddings".to_string()),
            started_at: 900,
            completed_at: 5_100,
            duration_ms: 4_200.0,
            item_count: 4,
            input_tokens: None,
            http_status: Some(200),
            error_text: None,
        };
        record_vector_activity_in_connection(&conn, "analysis-vector", &log).unwrap();
        record_vector_activity_in_connection(&conn, "analysis-vector", &proxy).unwrap();

        let analysis = query_session_analysis(&conn, "analysis-vector").unwrap();
        let vector = analysis
            .vector_analysis
            .expect("vector session should expose vector analysis");
        assert_eq!(vector.workload, "embedding");
        assert!(vector.log_available);
        assert_eq!(vector.completed_items, Some(4));
        assert_eq!(vector.input_tokens, Some(40));
        assert_eq!(vector.average_input_tokens_per_second, Some(10.0));
        assert_eq!(vector.average_items_per_second, Some(1.0));
        assert!(vector.proxy_available);
        assert_eq!(vector.proxy_request_count, Some(1));
        assert_eq!(vector.proxy_item_count, Some(4));
        assert_eq!(vector.proxy_success_rate, Some(1.0));
        assert!(!vector.trend.is_empty());

        let inference = query_session_analysis(&conn, "analysis-inference").unwrap();
        assert!(inference.vector_analysis.is_none());
    }

    #[test]
    fn speculative_analysis_uses_weighted_token_acceptance() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        insert_run_session(
            &conn,
            &RunSessionStart {
                id: "analysis-speculative",
                instance_id: "spec-instance",
                instance_name: "spec-instance",
                model_path: "model.gguf",
                engine_id: "engine",
                backend: "cpu",
                config_hash: "hash",
                command_line: "llama-server --spec-type draft-mtp",
                workload: ModelWorkload::Inference,
                started_at: 0,
            },
        )
        .unwrap();

        for (task_id, accepted, generated, rate, generation_time_ms) in [
            (1, 8, 10, Some(0.8), Some(12.0)),
            (2, 2, 30, Some(0.2), Some(28.0)),
        ] {
            insert_inference_request(
                &conn,
                "analysis-speculative",
                task_id as i64 * 1_000,
                &InferenceRequestRecord {
                    task_id,
                    slot_id: 0,
                    prompt_tokens: None,
                    prompt_time_ms: None,
                    prompt_tps: None,
                    generated_tokens: None,
                    generation_time_ms: None,
                    generation_tps: None,
                    total_tokens: None,
                    total_time_ms: None,
                    spec_accept_rate: rate,
                    spec_accepted: Some(accepted),
                    spec_generated: Some(generated),
                    spec_gen_time_ms: generation_time_ms,
                },
            )
            .unwrap();
        }

        let analysis = query_session_analysis(&conn, "analysis-speculative").unwrap();
        let speculative = analysis
            .speculative_analysis
            .expect("speculative requests should expose an aggregate");
        assert_eq!(speculative.request_count, 2);
        assert_eq!(speculative.accepted_tokens, 10);
        assert_eq!(speculative.generated_tokens, 40);
        assert_eq!(speculative.acceptance_rate, Some(0.25));
        assert_eq!(speculative.avg_generation_time_ms, Some(20.0));
    }

    #[test]
    fn vector_analysis_aggregates_concurrent_buckets_and_exact_percentiles() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        seed_vector_session(
            &conn,
            "aggregate-vector",
            "C:/models/embed.gguf",
            "cpu",
            ModelWorkload::Embedding,
        );
        for (source_event_id, started_at, completed_at, duration_ms, item_count, input_tokens) in [
            (1, 1_000, 1_100, 10.0, 2, 20),
            (2, 1_000, 1_900, 20.0, 3, 30),
            (3, 2_000, 2_100, 30.0, 1, 5),
            (4, 2_000, 2_200, 40.0, 1, 5),
        ] {
            record_vector_activity_in_connection(
                &conn,
                "aggregate-vector",
                &VectorActivityRecord {
                    source: VectorEventSource::Log,
                    source_event_id,
                    workload: ModelWorkload::Embedding,
                    endpoint: None,
                    started_at,
                    completed_at,
                    duration_ms,
                    item_count,
                    input_tokens: Some(input_tokens),
                    http_status: None,
                    error_text: None,
                },
            )
            .unwrap();
        }

        let analysis = query_vector_analysis(&conn, "aggregate-vector")
            .unwrap()
            .unwrap();
        assert_eq!(analysis.completed_items, Some(7));
        assert_eq!(analysis.input_tokens, Some(60));
        assert_eq!(analysis.task_duration_p50_ms, Some(20.0));
        assert_eq!(analysis.task_duration_p95_ms, Some(40.0));
        assert_eq!(analysis.trend[1].items_per_second, 5.0);
        assert_eq!(analysis.trend[1].input_tokens_per_second, Some(50.0));
        assert_eq!(analysis.trend[2].items_per_second, 2.0);
        assert_eq!(analysis.trend[2].input_tokens_per_second, Some(10.0));
    }

    #[test]
    fn vector_analysis_distinguishes_proxy_only_and_no_business_data() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        seed_vector_session(
            &conn,
            "proxy-only",
            "C:/models/embed.gguf",
            "cuda",
            ModelWorkload::Embedding,
        );
        seed_vector_session(
            &conn,
            "no-events",
            "C:/models/embed.gguf",
            "cuda",
            ModelWorkload::Embedding,
        );
        let proxy = VectorActivityRecord {
            source: VectorEventSource::Proxy,
            source_event_id: 1,
            workload: ModelWorkload::Embedding,
            endpoint: Some("/v1/embeddings".to_string()),
            started_at: 1_000,
            completed_at: 2_000,
            duration_ms: 1_000.0,
            item_count: 3,
            input_tokens: None,
            http_status: Some(200),
            error_text: None,
        };
        record_vector_activity_in_connection(&conn, "proxy-only", &proxy).unwrap();

        let proxy_only = query_vector_analysis(&conn, "proxy-only").unwrap().unwrap();
        assert!(!proxy_only.log_available);
        assert!(proxy_only.proxy_available);
        assert_eq!(proxy_only.completed_items, None);
        assert_eq!(proxy_only.average_items_per_second, None);
        assert_eq!(proxy_only.proxy_request_count, Some(1));
        assert!(proxy_only.trend.is_empty());

        let no_events = query_vector_analysis(&conn, "no-events").unwrap().unwrap();
        assert!(!no_events.log_available);
        assert!(!no_events.proxy_available);
        assert_eq!(no_events.completed_items, None);
        assert_eq!(no_events.proxy_request_count, None);
        assert!(no_events.trend.is_empty());
    }

    #[test]
    fn vector_baseline_requires_matching_model_workload_and_backend() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        for id in ["current", "matching-a", "matching-b", "active-match"] {
            seed_vector_session(
                &conn,
                id,
                "C:/models/embed.gguf",
                "cuda",
                ModelWorkload::Embedding,
            );
        }
        seed_vector_session(
            &conn,
            "wrong-workload",
            "C:/models/embed.gguf",
            "cuda",
            ModelWorkload::Reranker,
        );
        seed_vector_session(
            &conn,
            "wrong-backend",
            "C:/models/embed.gguf",
            "vulkan",
            ModelWorkload::Embedding,
        );
        for (id, workload) in [
            ("matching-a", ModelWorkload::Embedding),
            ("matching-b", ModelWorkload::Embedding),
            ("active-match", ModelWorkload::Embedding),
            ("wrong-workload", ModelWorkload::Reranker),
            ("wrong-backend", ModelWorkload::Embedding),
        ] {
            seed_vector_log_event(&conn, id, workload, 1, 1_000, 100, 100.0);
        }
        conn.execute(
            "UPDATE run_sessions SET stopped_at = NULL WHERE id = 'active-match'",
            [],
        )
        .unwrap();

        let baseline = query_vector_baseline(
            &conn,
            "current",
            "embed.gguf",
            "C:/models/embed.gguf",
            ModelWorkload::Embedding,
            "cuda",
        )
        .unwrap();

        assert_eq!(baseline.session_count, 2);
        assert_eq!(baseline.average_input_tokens_per_second, Some(250.0));
        assert_eq!(baseline.average_items_per_second, Some(25.0));
        assert_eq!(baseline.task_duration_p95_ms, Some(100.0));

        let session_analysis = query_session_analysis(&conn, "current").unwrap();
        let exposed = session_analysis
            .vector_baseline
            .expect("vector session should expose its filtered baseline");
        assert_eq!(exposed.session_count, 2);
        assert_eq!(exposed.average_items_per_second, Some(25.0));
    }

    #[test]
    fn vector_rates_use_the_recent_observed_work_window() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        seed_vector_session(
            &conn,
            "work-window",
            "C:/models/embed.gguf",
            "cpu",
            ModelWorkload::Embedding,
        );
        conn.execute(
            "UPDATE run_sessions SET stopped_at = 100000 WHERE id = 'work-window'",
            [],
        )
        .unwrap();
        record_vector_activity_in_connection(
            &conn,
            "work-window",
            &VectorActivityRecord {
                source: VectorEventSource::Log,
                source_event_id: 1,
                workload: ModelWorkload::Embedding,
                endpoint: None,
                started_at: 50_000,
                completed_at: 51_000,
                duration_ms: 1_000.0,
                item_count: 10,
                input_tokens: Some(100),
                http_status: None,
                error_text: None,
            },
        )
        .unwrap();

        let analysis = query_vector_analysis(&conn, "work-window")
            .unwrap()
            .unwrap();
        assert_eq!(analysis.completed_items, Some(10));
        assert_eq!(analysis.average_input_tokens_per_second, Some(100.0));
        assert_eq!(analysis.average_items_per_second, Some(10.0));
    }

    #[test]
    fn vector_analysis_includes_an_event_completed_at_session_stop() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        seed_vector_session(
            &conn,
            "boundary",
            "C:/models/embed.gguf",
            "cpu",
            ModelWorkload::Embedding,
        );
        record_vector_activity_in_connection(
            &conn,
            "boundary",
            &VectorActivityRecord {
                source: VectorEventSource::Log,
                source_event_id: 1,
                workload: ModelWorkload::Embedding,
                endpoint: None,
                started_at: 9_000,
                completed_at: 10_000,
                duration_ms: 1_000.0,
                item_count: 2,
                input_tokens: Some(20),
                http_status: None,
                error_text: None,
            },
        )
        .unwrap();

        let analysis = query_vector_analysis(&conn, "boundary").unwrap().unwrap();
        assert_eq!(analysis.completed_items, Some(2));
        assert_eq!(analysis.input_tokens, Some(20));
        assert_eq!(analysis.task_duration_p95_ms, Some(1_000.0));
    }

    #[test]
    fn vector_diagnostics_exclude_generation_findings_and_use_vector_baseline() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        for id in ["current", "baseline-a", "baseline-b"] {
            seed_vector_session(
                &conn,
                id,
                "C:/models/embed.gguf",
                "cuda",
                ModelWorkload::Embedding,
            );
        }
        seed_vector_log_event(
            &conn,
            "current",
            ModelWorkload::Embedding,
            1,
            100,
            10,
            500.0,
        );
        for id in ["baseline-a", "baseline-b"] {
            seed_vector_log_event(&conn, id, ModelWorkload::Embedding, 1, 1_000, 100, 100.0);
        }

        let findings = query_session_diagnostics(&conn, "current").unwrap();
        let ids = findings
            .iter()
            .map(|finding| finding.id.as_str())
            .collect::<Vec<_>>();

        assert!(ids.contains(&"vector_throughput_regression"));
        assert!(ids.contains(&"vector_task_latency_regression"));
        assert!(ids.contains(&"vector_source_incomplete"));
        for generation_only in [
            "throughput_regression",
            "no_request_records",
            "prompt_eval_bottleneck",
            "long_request_latency",
            "slot_cache_observed",
            "large_context_window",
        ] {
            assert!(!ids.contains(&generation_only));
        }
    }

    #[test]
    fn session_summaries_keep_their_persisted_workload() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        insert_run_session(
            &conn,
            &RunSessionStart {
                id: "historical-reranker",
                instance_id: "mutable-instance",
                instance_name: "Historical",
                model_path: "old-reranker.gguf",
                engine_id: "engine",
                backend: "vulkan",
                config_hash: "hash",
                command_line: "llama-server --embedding --reranking",
                workload: ModelWorkload::Reranker,
                started_at: 1_000,
            },
        )
        .unwrap();

        let sessions = query_telemetry_sessions(&conn, 2_000, 20).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].instance_id, "mutable-instance");
        assert_eq!(sessions[0].workload, "reranker");
    }

    #[test]
    fn global_time_queries_use_dedicated_indexes() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        for (sql, expected_index) in [
            (
                "EXPLAIN QUERY PLAN SELECT AVG(tokens_per_sec) FROM metric_samples WHERE ts >= 1",
                "idx_metric_samples_ts",
            ),
            (
                "EXPLAIN QUERY PLAN SELECT * FROM metric_samples ORDER BY ts DESC LIMIT 10",
                "idx_metric_samples_ts",
            ),
            (
                "EXPLAIN QUERY PLAN DELETE FROM vector_activity_events WHERE completed_at < 1",
                "idx_vector_activity_completed",
            ),
        ] {
            let mut statement = conn.prepare(sql).unwrap();
            let details = statement
                .query_map([], |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .join(" ");
            assert!(
                details.contains(expected_index),
                "query plan did not use {expected_index}: {details}"
            );
        }
    }

    #[test]
    fn version_four_database_migrates_vector_schema_once() {
        let conn = version_four_connection();

        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap();

        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(
            column_names(&conn, "run_sessions")
                .iter()
                .filter(|column| column.as_str() == "workload")
                .count(),
            1
        );
        for (session_id, expected) in [
            ("inference", "inference"),
            ("embedding", "embedding"),
            ("reranker", "reranker"),
        ] {
            let workload: String = conn
                .query_row(
                    "SELECT workload FROM run_sessions WHERE id = ?1",
                    params![session_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(workload, expected);
        }
        let sample_columns = column_names(&conn, "metric_samples");
        assert!(sample_columns
            .iter()
            .any(|column| column == "decode_calls_total"));
        assert!(sample_columns
            .iter()
            .any(|column| column == "max_tokens_observed"));
        assert!(sample_columns.iter().any(|column| column == "gpu_name"));
        for index_name in [
            "idx_vector_activity_session_completed",
            "idx_vector_activity_session_source_completed",
            "idx_vector_activity_session_source_duration",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                    params![index_name],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
        }

        conn.execute(
            "INSERT INTO vector_activity_events
                (session_id, source, source_event_id, workload, started_at, completed_at,
                 duration_ms, item_count, input_tokens)
             VALUES ('embedding', 'log', 7, 'embedding', 10, 20, 10.0, 1, 8)",
            [],
        )
        .unwrap();
        let duplicate = conn.execute(
            "INSERT INTO vector_activity_events
                (session_id, source, source_event_id, workload, started_at, completed_at,
                 duration_ms, item_count, input_tokens)
             VALUES ('embedding', 'log', 7, 'embedding', 10, 20, 10.0, 1, 8)",
            [],
        );
        assert!(duplicate.is_err());
        conn.execute("DELETE FROM run_sessions WHERE id = 'embedding'", [])
            .unwrap();
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM vector_activity_events", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn current_schema_initialization_does_not_rescan_session_workloads() {
        let conn = version_four_connection();
        init_schema(&conn).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject_workload_rescan
             BEFORE UPDATE OF workload ON run_sessions
             BEGIN
               SELECT RAISE(ABORT, 'workload migration reran');
             END;",
        )
        .unwrap();

        init_schema(&conn).unwrap();

        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn pruning_removes_old_samples_from_active_sessions() {
        let mut conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO run_sessions
                (id, instance_id, instance_name, model_name, model_path, engine_id, backend,
                 config_hash, command_line, started_at)
             VALUES ('active', 'instance', 'Instance', 'Model', 'model.gguf', 'engine', 'cpu',
                     'hash', 'cmd', 1)",
            [],
        )
        .unwrap();
        for ts in [100_i64, 1_000_i64] {
            conn.execute(
                "INSERT INTO metric_samples (session_id, instance_id, ts)
                 VALUES ('active', 'instance', ?1)",
                params![ts],
            )
            .unwrap();
        }

        let removed = prune_connection(&mut conn, 500).unwrap();
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM metric_samples", [], |row| row.get(0))
            .unwrap();

        assert_eq!(removed, 1);
        assert_eq!(remaining, 1);
    }

    #[test]
    fn pruning_removes_old_vector_events_from_active_sessions() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        init_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO run_sessions
                (id, instance_id, instance_name, model_name, model_path, engine_id, backend,
                 config_hash, command_line, workload, started_at)
             VALUES ('active-vector', 'instance', 'Instance', 'Model', 'model.gguf', 'engine',
                     'cpu', 'hash', 'cmd --embedding', 'embedding', 1)",
            [],
        )
        .unwrap();
        for (source_event_id, completed_at) in [(1_i64, 100_i64), (2_i64, 1_000_i64)] {
            conn.execute(
                "INSERT INTO vector_activity_events
                    (session_id, source, source_event_id, workload, started_at, completed_at,
                     duration_ms, item_count)
                 VALUES ('active-vector', 'log', ?1, 'embedding', ?2 - 10, ?2, 10.0, 1)",
                params![source_event_id, completed_at],
            )
            .unwrap();
        }

        let removed = prune_connection(&mut conn, 500).unwrap();
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM vector_activity_events", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert_eq!(removed, 1);
        assert_eq!(remaining, 1);
    }
}

// IPC compatibility boundary: legacy command internals keep their existing error flow,
// while every registered command serializes a stable AppError object.
#[allow(dead_code, unused_imports, unused_mut)] // Tauri references adapters through generated macros.
pub mod ipc {
    use super::*;

    #[tauri::command]
    pub async fn get_telemetry_overview() -> crate::error::AppResult<TelemetryOverview> {
        super::get_telemetry_overview()
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn list_telemetry_sessions(
        limit: Option<u32>,
    ) -> crate::error::AppResult<Vec<TelemetrySessionSummary>> {
        super::list_telemetry_sessions(limit)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_telemetry_session_samples(
        session_id: String,
        limit: Option<u32>,
    ) -> crate::error::AppResult<Vec<TelemetrySampleSummary>> {
        super::get_telemetry_session_samples(session_id, limit)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_telemetry_session_detail(
        session_id: String,
        sample_limit: Option<u32>,
        request_limit: Option<u32>,
    ) -> crate::error::AppResult<TelemetrySessionDetail> {
        super::get_telemetry_session_detail(session_id, sample_limit, request_limit)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn prune_telemetry(retention_days: Option<u32>) -> crate::error::AppResult<u32> {
        super::prune_telemetry(retention_days)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_telemetry_session_analysis(
        session_id: String,
    ) -> crate::error::AppResult<TelemetrySessionAnalysis> {
        super::get_telemetry_session_analysis(session_id)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_telemetry_session_diagnostics(
        session_id: String,
    ) -> crate::error::AppResult<Vec<DiagnosticFinding>> {
        super::get_telemetry_session_diagnostics(session_id)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn list_inference_requests(
        session_id: String,
        limit: Option<u32>,
    ) -> crate::error::AppResult<Vec<InferenceRequestSummary>> {
        super::list_inference_requests(session_id, limit)
            .await
            .map_err(crate::error::AppError::from)
    }
}
