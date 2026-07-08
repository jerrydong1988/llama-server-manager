use crate::models::SystemMetrics;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: i64 = 1;

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
            prompt_tokens_per_sec REAL,
            requests_processing INTEGER,
            requests_deferred INTEGER,
            busy_slots_per_decode REAL,
            FOREIGN KEY(session_id) REFERENCES run_sessions(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_metric_samples_session_ts ON metric_samples(session_id, ts);
        CREATE INDEX IF NOT EXISTS idx_metric_samples_instance_ts ON metric_samples(instance_id, ts);
        CREATE INDEX IF NOT EXISTS idx_run_sessions_instance_started ON run_sessions(instance_id, started_at DESC);
        "#,
    )
    .map_err(|e| format!("无法初始化遥测数据库: {}", e))?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|e| format!("无法写入遥测 schema 版本: {}", e))?;
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
pub async fn list_telemetry_sessions(limit: Option<u32>) -> Result<Vec<TelemetrySessionSummary>, String> {
    let limit = limit.unwrap_or(20).clamp(1, 200);
    tokio::task::spawn_blocking(move || {
        let conn = open_connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT
                    s.id, s.instance_id, s.instance_name, s.model_name, s.model_path, s.engine_id, s.backend,
                    s.started_at, s.stopped_at,
                    CASE WHEN s.stopped_at IS NULL THEN NULL ELSE CAST((s.stopped_at - s.started_at) / 1000 AS INTEGER) END,
                    COALESCE((SELECT AVG(m.tokens_per_sec) FROM metric_samples m WHERE m.session_id = s.id AND m.tokens_per_sec > 0), 0),
                    COALESCE((SELECT MAX(m.vram_used_mb) FROM metric_samples m WHERE m.session_id = s.id), 0),
                    COALESCE((SELECT COUNT(*) FROM metric_samples m WHERE m.session_id = s.id), 0),
                    s.stop_reason
                 FROM run_sessions s
                 ORDER BY s.started_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| format!("无法准备遥测会话查询: {}", e))?;
        let rows = stmt
            .query_map(params![limit], |row| {
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
    tokio::task::spawn_blocking(move || {
        let conn = open_connection()?;
        let before = now_ms() - days as i64 * 24 * 60 * 60 * 1000;
        let affected = conn
            .execute(
                "DELETE FROM run_sessions WHERE stopped_at IS NOT NULL AND stopped_at < ?1",
                params![before],
            )
            .map_err(|e| format!("无法清理遥测数据: {}", e))?;
        Ok(affected as u32)
    })
    .await
    .map_err(|e| format!("遥测清理失败: {}", e))?
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
