use crate::models::SystemMetrics;
use crate::vector_policy::ModelWorkload;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: i64 = 5;

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
    pub tokens_per_sec: Option<f64>,
    pub prompt_tokens_per_sec: Option<f64>,
    pub prompt_tokens_total: Option<i64>,
    pub generated_tokens_total: Option<i64>,
    pub requests_total: Option<i64>,
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
pub struct LlamaMetricSample {
    pub tokens_per_sec: f64,
    pub prompt_tokens: u64,
    pub gen_tokens: u64,
    pub requests: u64,
    pub prompt_tokens_per_sec: f64,
    pub requests_processing: u64,
    pub requests_deferred: u64,
    pub busy_slots_per_decode: f64,
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

fn open_connection() -> Result<Connection, String> {
    let path = telemetry_db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("无法创建遥测数据目录: {}", e))?;
    }
    let conn = Connection::open(path).map_err(|e| format!("无法打开遥测数据库: {}", e))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("无法启用遥测数据库 WAL: {}", e))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| format!("无法启用遥测数据库外键: {}", e))?;
    init_schema(&conn)?;
    Ok(conn)
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
        "#,
    )
    .map_err(|e| format!("无法初始化遥测数据库: {}", e))?;
    migrate_inference_request_columns(conn)?;
    migrate_vector_schema(conn)?;
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

pub fn begin_run_session(
    instance_id: &str,
    instance_name: &str,
    model_path: &str,
    engine_id: &str,
    backend: &str,
    config_hash: &str,
    command_line: &str,
) -> Result<String, String> {
    let conn = open_connection()?;
    let id = uuid::Uuid::new_v4().to_string();
    let model_name = std::path::Path::new(model_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(model_path)
        .to_string();
    conn.execute(
        "INSERT INTO run_sessions
            (id, instance_id, instance_name, model_name, model_path, engine_id, backend, config_hash, command_line, started_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            instance_id,
            instance_name,
            model_name,
            model_path,
            engine_id,
            backend,
            config_hash,
            command_line,
            now_ms(),
        ],
    )
    .map_err(|e| format!("无法创建运行遥测会话: {}", e))?;
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

pub fn record_metric_sample(
    session_id: Option<&str>,
    instance_id: &str,
    system: &SystemMetrics,
    llama: Option<&LlamaMetricSample>,
) -> Result<(), String> {
    let Some(session_id) = session_id else {
        return Ok(());
    };
    let conn = open_connection()?;
    let cpu = Some(system.cpu_percent as f64);
    let memory = Some(system.memory_mb);
    let gpu = system.gpu_percent.map(|v| v as f64);
    let sys_cpu = system.system_cpu_percent.map(|v| v as f64);
    conn.execute(
        "INSERT INTO metric_samples
            (session_id, instance_id, ts, cpu_percent, memory_mb, gpu_percent, vram_used_mb, vram_total_mb,
             system_cpu_percent, system_memory_used_mb, system_memory_total_mb, gpu_vendor, tokens_per_sec,
             prompt_tokens_total, generated_tokens_total, requests_total, prompt_tokens_per_sec,
             requests_processing, requests_deferred, busy_slots_per_decode)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        params![
            session_id,
            instance_id,
            now_ms(),
            cpu,
            memory,
            gpu,
            system.vram_used_mb,
            system.vram_total_mb,
            sys_cpu,
            system.system_memory_used_mb,
            system.system_memory_total_mb,
            system.gpu_vendor.as_deref(),
            llama.map(|m| m.tokens_per_sec),
            llama.map(|m| m.prompt_tokens as i64),
            llama.map(|m| m.gen_tokens as i64),
            llama.map(|m| m.requests as i64),
            llama.map(|m| m.prompt_tokens_per_sec),
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
    let conn = open_connection()?;
    conn.execute(
        "INSERT INTO inference_requests
            (session_id, task_id, slot_id, completed_at, source, prompt_tokens, prompt_time_ms, prompt_tps,
             generated_tokens, generation_time_ms, generation_tps, total_tokens, total_time_ms,
             spec_accept_rate, spec_accepted, spec_generated, spec_gen_time_ms)
         VALUES (?1, ?2, ?3, ?4, 'log', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
         ON CONFLICT(session_id, task_id) DO UPDATE SET
             slot_id = excluded.slot_id,
             completed_at = excluded.completed_at,
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
            now_ms(),
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
    let conn = open_connection()?;
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
            now_ms(),
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
    let mut conn = open_connection()?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("无法开始 slot 遥测事务: {}", e))?;
    let ts = now_ms();
    {
        let mut stmt = tx
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
                ts,
                slot.slot_id,
                if slot.is_processing { 1 } else { 0 },
                slot.n_ctx,
                slot.n_past.map(|v| v as i64),
            ])
            .map_err(|e| format!("无法写入 slot 遥测: {}", e))?;
        }
    }
    tx.commit()
        .map_err(|e| format!("无法提交 slot 遥测事务: {}", e))?;
    Ok(())
}

#[tauri::command]
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

#[tauri::command]
pub async fn list_telemetry_sessions(
    limit: Option<u32>,
) -> Result<Vec<TelemetrySessionSummary>, String> {
    let limit = limit.unwrap_or(20).clamp(1, 200);
    tokio::task::spawn_blocking(move || {
        let conn = open_connection()?;
        let current_time = now_ms();
        let mut stmt = conn
            .prepare(
                "SELECT
                    s.id, s.instance_id, s.instance_name, s.model_name, s.model_path, s.engine_id, s.backend,
                    s.started_at, s.stopped_at,
                    CASE
                        WHEN COALESCE(s.stopped_at, ?1) <= s.started_at THEN 0
                        ELSE CAST((COALESCE(s.stopped_at, ?1) - s.started_at) / 1000 AS INTEGER)
                    END,
                    COALESCE((SELECT AVG(m.tokens_per_sec) FROM metric_samples m WHERE m.session_id = s.id AND m.tokens_per_sec > 0), 0),
                    COALESCE((SELECT MAX(m.vram_used_mb) FROM metric_samples m WHERE m.session_id = s.id), 0),
                    COALESCE((SELECT COUNT(*) FROM metric_samples m WHERE m.session_id = s.id), 0),
                    s.stop_reason
                 FROM run_sessions s
                 ORDER BY s.started_at DESC
                 LIMIT ?2",
            )
            .map_err(|e| format!("无法准备遥测会话查询: {}", e))?;
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
                    started_at: row.get(7)?,
                    stopped_at: row.get(8)?,
                    duration_secs: row.get(9)?,
                    avg_tokens_per_sec: row.get(10)?,
                    peak_vram_mb: row.get(11)?,
                    sample_count: row.get::<_, i64>(12)? as u32,
                    stop_reason: row.get(13)?,
                })
            })
            .map_err(|e| format!("无法查询遥测会话: {}", e))?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row.map_err(|e| format!("无法读取遥测会话: {}", e))?);
        }
        Ok(sessions)
    })
    .await
    .map_err(|e| format!("遥测会话查询失败: {}", e))?
}

#[tauri::command]
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

#[tauri::command]
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
    let mut conn = open_connection()?;
    let affected = prune_connection(&mut conn, before)?;
    if affected > 0 {
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;");
    }
    Ok(affected)
}

#[tauri::command]
pub async fn get_telemetry_session_analysis(
    session_id: String,
) -> Result<TelemetrySessionAnalysis, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_connection()?;
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
        })
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
    })
}

fn query_session_metric_stats(
    conn: &Connection,
    session_id: &str,
) -> Result<SessionMetricStats, String> {
    conn.query_row(
        "SELECT
            COUNT(*),
            COALESCE(AVG(tokens_per_sec), 0),
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

#[tauri::command]
pub async fn get_telemetry_session_diagnostics(
    session_id: String,
) -> Result<Vec<DiagnosticFinding>, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_connection()?;
        let session = conn
            .query_row(
                "SELECT model_name, engine_id, backend FROM run_sessions WHERE id = ?1",
                params![session_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
            )
            .optional()
            .map_err(|e| format!("无法读取诊断会话: {}", e))?
            .ok_or_else(|| "找不到对应的遥测会话".to_string())?;
        let analysis = query_session_analysis(&conn, &session_id)?;
        let metrics = query_session_metric_stats(&conn, &session_id)?;
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
                params![session.0.as_str(), session_id.as_str()],
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

        if baseline.1 >= 2 && baseline.0 > 0.0 && metrics.avg_tps > 0.0 && metrics.avg_tps < baseline.0 * 0.75 {
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
                if metrics.max_vram_ratio >= 0.98 { "critical" } else { "warning" },
                0.9,
                "显存压力偏高",
                "会话期间显存占用接近上限，可能导致后续请求失败、回退到 CPU 或响应抖动。",
                vec![format!("显存峰值占比 {}", fmt_diag_percent(metrics.max_vram_ratio * 100.0))],
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

        if metrics.avg_tps > 0.0 && metrics.avg_gpu < 35.0 && (metrics.avg_system_cpu > 55.0 || metrics.avg_instance_cpu > 55.0) {
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
                    "如果重复对话较多，这是有益信号；如果显存紧张，可降低保留上下文或并发槽位。".to_string(),
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
                vec![
                    "如不需要长上下文，降低 ctx-size 通常可以改善显存压力和延迟。".to_string(),
                ],
            ));
        }

        if findings.iter().all(|item| item.severity == "info") && analysis.request_count > 0 {
            findings.insert(0, diagnostic_finding(
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
            ));
        }

        if findings.is_empty() {
            findings.push(diagnostic_finding(
                "baseline_collecting",
                "info",
                0.7,
                "正在建立分析基线",
                "当前数据可用于展示趋势，但历史基线或请求级样本仍偏少。",
                vec![
                    format!("引擎 {} / {}", session.1, session.2),
                    format!("资源采样 {} 条", metrics.sample_count),
                    format!("请求 {} 次", analysis.request_count),
                    format!("指标 busy slot 平均 {:.1} / 峰值 {:.1}", metrics.avg_busy_slots_metric, metrics.max_busy_slots_metric),
                ],
                vec!["多运行几组相同模型和相同参数的会话后，诊断会自动给出更明确的对比结论。".to_string()],
            ));
        }

        Ok(findings)
    })
    .await
    .map_err(|e| format!("会话诊断查询失败: {}", e))?
}

#[tauri::command]
pub async fn list_inference_requests(
    session_id: String,
    limit: Option<u32>,
) -> Result<Vec<InferenceRequestSummary>, String> {
    let limit = limit.unwrap_or(20).clamp(1, 200);
    tokio::task::spawn_blocking(move || {
        let conn = open_connection()?;
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
                    source: row.get::<_, Option<String>>(4)?.unwrap_or_else(|| "log".into()),
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
        let mut requests = Vec::new();
        for row in rows {
            requests.push(row.map_err(|e| format!("无法读取推理请求: {}", e))?);
        }
        Ok(requests)
    })
    .await
    .map_err(|e| format!("推理请求查询失败: {}", e))?
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
                    system_cpu_percent, system_memory_used_mb, system_memory_total_mb, tokens_per_sec,
                    prompt_tokens_per_sec, prompt_tokens_total, generated_tokens_total, requests_total,
                    requests_processing, requests_deferred, busy_slots_per_decode
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
                    system_cpu_percent, system_memory_used_mb, system_memory_total_mb, tokens_per_sec,
                    prompt_tokens_per_sec, prompt_tokens_total, generated_tokens_total, requests_total,
                    requests_processing, requests_deferred, busy_slots_per_decode
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
        tokens_per_sec: row.get(11)?,
        prompt_tokens_per_sec: row.get(12)?,
        prompt_tokens_total: row.get(13)?,
        generated_tokens_total: row.get(14)?,
        requests_total: row.get(15)?,
        requests_processing: row.get(16)?,
        requests_deferred: row.get(17)?,
        busy_slots_per_decode: row.get(18)?,
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

    #[test]
    fn version_four_database_migrates_vector_schema_once() {
        let conn = version_four_connection();

        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap();

        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 5);
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
        for index_name in [
            "idx_vector_activity_session_completed",
            "idx_vector_activity_session_source_completed",
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
