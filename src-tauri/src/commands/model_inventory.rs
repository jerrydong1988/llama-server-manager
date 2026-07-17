use crate::models::{EngineCapabilities, EngineInfo, ModelCapabilities, ModelInfo};
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MODEL_INVENTORY_SCHEMA_VERSION: i64 = 3;
static INVENTORY_SCHEMA_READY: AtomicBool = AtomicBool::new(false);
static INVENTORY_SCHEMA_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
pub struct InventoryModelRecord {
    pub path: String,
    pub id: String,
    pub display_path: String,
    pub name: String,
    pub scan_root: String,
    pub size: u64,
    pub mtime: u64,
    pub architecture: Option<String>,
    pub context_length: Option<u32>,
    pub quant_type: Option<String>,
    pub has_mtp_head: bool,
    pub capabilities: ModelCapabilities,
    pub file_type: String,
    pub is_shard: bool,
    pub last_seen: i64,
    pub cache_version: i64,
}

#[derive(Debug, Clone)]
pub struct InventoryEngineRecord {
    pub id: String,
    pub name: String,
    pub dir: String,
    pub exe: String,
    pub version: String,
    pub backend: String,
    pub capabilities: EngineCapabilities,
    pub exe_mtime: u64,
    pub scan_root: String,
    pub last_seen: i64,
    pub cache_version: i64,
}

#[derive(Debug, Clone)]
pub struct InventoryDirectoryRecord {
    pub kind: String,
    pub path: String,
    pub scan_root: String,
    pub signature: String,
    pub last_seen: i64,
    pub cache_version: i64,
}

pub type ModelScanIndexes = (
    HashMap<String, InventoryModelRecord>,
    HashMap<String, InventoryDirectoryRecord>,
);
pub type EngineScanIndexes = (
    HashMap<String, InventoryEngineRecord>,
    HashMap<String, InventoryDirectoryRecord>,
);

impl InventoryModelRecord {
    pub fn from_model(
        model: &ModelInfo,
        canonical_path: String,
        scan_root: String,
        mtime: u64,
    ) -> Self {
        Self {
            path: canonical_path,
            id: model.id.clone(),
            display_path: model.path.clone(),
            name: model.name.clone(),
            scan_root,
            size: model.size,
            mtime,
            architecture: model.architecture.clone(),
            context_length: model.context_length,
            quant_type: model.quant_type.clone(),
            has_mtp_head: model.has_mtp_head,
            capabilities: model.capabilities.clone(),
            file_type: model.file_type.clone(),
            is_shard: model.is_shard,
            last_seen: now_secs(),
            cache_version: MODEL_INVENTORY_SCHEMA_VERSION,
        }
    }

    pub fn to_model_info(&self) -> ModelInfo {
        ModelInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            path: self.display_path.clone(),
            size: self.size,
            architecture: self.architecture.clone(),
            context_length: self.context_length,
            quant_type: crate::utils::normalize_quant_type_for_path(
                self.quant_type.clone(),
                Path::new(&self.display_path),
            ),
            has_mtp_head: self.has_mtp_head,
            capabilities: self.capabilities.clone(),
            file_type: self.file_type.clone(),
            is_shard: self.is_shard,
        }
    }
}

impl InventoryEngineRecord {
    pub fn from_engine(engine: &EngineInfo, exe_mtime: u64, scan_root: String) -> Self {
        Self {
            id: engine.id.clone(),
            name: engine.name.clone(),
            dir: engine.dir.clone(),
            exe: engine.exe.clone(),
            version: engine.version.clone(),
            backend: engine.backend.clone(),
            capabilities: engine.capabilities.clone(),
            exe_mtime,
            scan_root,
            last_seen: now_secs(),
            cache_version: MODEL_INVENTORY_SCHEMA_VERSION,
        }
    }

    pub fn to_engine_info(&self) -> EngineInfo {
        EngineInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            dir: self.dir.clone(),
            exe: self.exe.clone(),
            version: self.version.clone(),
            backend: self.backend.clone(),
            custom_name: None,
            capabilities: self.capabilities.clone(),
        }
    }
}

impl InventoryDirectoryRecord {
    pub fn new(kind: &str, path: String, scan_root: String, signature: String) -> Self {
        Self {
            kind: kind.to_string(),
            path,
            scan_root,
            signature,
            last_seen: now_secs(),
            cache_version: MODEL_INVENTORY_SCHEMA_VERSION,
        }
    }
}

fn inventory_db_path() -> PathBuf {
    crate::utils::get_data_dir().join("scan_inventory.db")
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn open_raw_connection() -> Result<Connection, String> {
    let path = inventory_db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create model inventory directory: {}", e))?;
    }
    let conn = Connection::open(path)
        .map_err(|e| format!("failed to open model inventory database: {}", e))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("failed to configure model inventory busy timeout: {e}"))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| format!("failed to configure model inventory synchronous mode: {e}"))?;
    Ok(conn)
}

pub(crate) fn initialize_inventory_storage() -> Result<(), String> {
    if INVENTORY_SCHEMA_READY.load(Ordering::Acquire) {
        return Ok(());
    }
    let _guard = INVENTORY_SCHEMA_LOCK.lock().unwrap();
    if INVENTORY_SCHEMA_READY.load(Ordering::Acquire) {
        return Ok(());
    }
    let conn = open_raw_connection()?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("failed to enable model inventory WAL: {}", e))?;
    init_schema(&conn)?;
    INVENTORY_SCHEMA_READY.store(true, Ordering::Release);
    Ok(())
}

fn open_connection() -> Result<Connection, String> {
    initialize_inventory_storage()?;
    open_raw_connection()
}

fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS model_inventory (
            path TEXT PRIMARY KEY,
            id TEXT NOT NULL,
            display_path TEXT NOT NULL,
            name TEXT NOT NULL,
            scan_root TEXT NOT NULL,
            size INTEGER NOT NULL,
            mtime INTEGER NOT NULL,
            architecture TEXT,
            context_length INTEGER,
            quant_type TEXT,
            has_mtp_head INTEGER NOT NULL,
            capabilities_json TEXT NOT NULL,
            file_type TEXT NOT NULL,
            is_shard INTEGER NOT NULL,
            last_seen INTEGER NOT NULL,
            cache_version INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_model_inventory_scan_root
            ON model_inventory(scan_root);
        CREATE INDEX IF NOT EXISTS idx_model_inventory_type
            ON model_inventory(file_type);
        CREATE INDEX IF NOT EXISTS idx_model_inventory_arch
            ON model_inventory(architecture);

        CREATE TABLE IF NOT EXISTS engine_inventory (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            dir TEXT NOT NULL,
            exe TEXT NOT NULL,
            version TEXT NOT NULL,
            backend TEXT NOT NULL,
            capabilities_json TEXT NOT NULL DEFAULT '{}',
            exe_mtime INTEGER NOT NULL,
            scan_root TEXT NOT NULL,
            last_seen INTEGER NOT NULL,
            cache_version INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_engine_inventory_scan_root
            ON engine_inventory(scan_root);
        CREATE INDEX IF NOT EXISTS idx_engine_inventory_backend
            ON engine_inventory(backend);

        CREATE TABLE IF NOT EXISTS inventory_directories (
            kind TEXT NOT NULL,
            path TEXT NOT NULL,
            scan_root TEXT NOT NULL,
            signature TEXT NOT NULL,
            last_seen INTEGER NOT NULL,
            cache_version INTEGER NOT NULL,
            PRIMARY KEY(kind, path)
        );

        CREATE INDEX IF NOT EXISTS idx_inventory_directories_scan_root
            ON inventory_directories(kind, scan_root);
        "#,
    )
    .map_err(|e| format!("failed to initialize model inventory schema: {}", e))?;
    ensure_column(
        conn,
        "engine_inventory",
        "capabilities_json",
        "TEXT NOT NULL DEFAULT '{}'",
    )?;
    conn.pragma_update(None, "user_version", MODEL_INVENTORY_SCHEMA_VERSION)
        .map_err(|e| format!("failed to write model inventory schema version: {}", e))?;
    Ok(())
}

fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| format!("failed to inspect {table} schema: {e}"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("failed to query {table} schema: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read {table} schema: {e}"))?;
    if columns.iter().any(|existing| existing == column) {
        return Ok(());
    }
    conn.execute_batch(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {definition}"
    ))
    .map_err(|e| format!("failed to add {table}.{column}: {e}"))
}

fn load_directory_index_from_connection(
    conn: &Connection,
    kind: &str,
) -> Result<HashMap<String, InventoryDirectoryRecord>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT kind, path, scan_root, signature, last_seen, cache_version
            FROM inventory_directories
            WHERE kind = ?1
            "#,
        )
        .map_err(|e| format!("failed to prepare directory inventory query: {}", e))?;
    let rows = stmt
        .query_map(params![kind], directory_record_from_row)
        .map_err(|e| format!("failed to query directory inventory: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read directory inventory row: {}", e))?;
    Ok(rows
        .into_iter()
        .filter(|record| record.cache_version == MODEL_INVENTORY_SCHEMA_VERSION)
        .filter(|record| std::path::Path::new(&record.path).is_dir())
        .map(|record| (record.path.clone(), record))
        .collect())
}

fn upsert_directory_records_in_connection(
    conn: &Connection,
    records: &[InventoryDirectoryRecord],
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            r#"
                INSERT INTO inventory_directories (
                    kind, path, scan_root, signature, last_seen, cache_version
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6
                )
                ON CONFLICT(kind, path) DO UPDATE SET
                    scan_root=excluded.scan_root,
                    signature=excluded.signature,
                    last_seen=excluded.last_seen,
                    cache_version=excluded.cache_version
            "#,
        )
        .map_err(|e| format!("failed to prepare directory inventory upsert: {}", e))?;
    for record in records {
        stmt.execute(params![
            record.kind,
            record.path,
            record.scan_root,
            record.signature,
            record.last_seen,
            MODEL_INVENTORY_SCHEMA_VERSION,
        ])
        .map_err(|e| format!("failed to upsert directory inventory row: {}", e))?;
    }
    Ok(())
}

fn prune_absent_directories_in_connection(
    conn: &Connection,
    kind: &str,
    scan_roots: &HashSet<String>,
    seen_dirs: &HashSet<String>,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT path, scan_root FROM inventory_directories WHERE kind = ?1")
        .map_err(|e| format!("failed to prepare directory inventory prune query: {}", e))?;
    let rows = stmt
        .query_map(params![kind], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("failed to query directory inventory for pruning: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read directory inventory prune row: {}", e))?;

    for (path, scan_root) in rows {
        if scan_roots.contains(&scan_root) && !seen_dirs.contains(&path) {
            let _ = conn.execute(
                "DELETE FROM inventory_directories WHERE kind = ?1 AND path = ?2",
                params![kind, path],
            );
        }
    }
    Ok(())
}

pub fn load_model_index() -> Result<HashMap<String, InventoryModelRecord>, String> {
    let conn = open_connection()?;
    load_model_index_from_connection(&conn)
}

fn load_model_index_from_connection(
    conn: &Connection,
) -> Result<HashMap<String, InventoryModelRecord>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT path, id, display_path, name, scan_root, size, mtime,
                   architecture, context_length, quant_type, has_mtp_head,
                   capabilities_json, file_type, is_shard, last_seen, cache_version
            FROM model_inventory
            "#,
        )
        .map_err(|e| format!("failed to prepare model inventory query: {}", e))?;
    let rows = stmt
        .query_map([], record_from_row)
        .map_err(|e| format!("failed to query model inventory: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read model inventory row: {}", e))?;
    Ok(rows
        .into_iter()
        .filter(|record| record.cache_version == MODEL_INVENTORY_SCHEMA_VERSION)
        .filter(|record| std::path::Path::new(&record.path).is_file())
        .map(|record| (record.path.clone(), record))
        .collect())
}

pub fn load_model_scan_indexes() -> Result<ModelScanIndexes, String> {
    let conn = open_connection()?;
    Ok((
        load_model_index_from_connection(&conn)?,
        load_directory_index_from_connection(&conn, "model")?,
    ))
}

pub fn list_cached_models() -> Result<Vec<ModelInfo>, String> {
    let mut records = load_model_index()?.into_values().collect::<Vec<_>>();
    records.sort_by_key(|record| record.name.to_lowercase());
    Ok(records
        .into_iter()
        .map(|record| record.to_model_info())
        .collect())
}

fn upsert_model_records_in_connection(
    conn: &Connection,
    records: &[InventoryModelRecord],
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            r#"
                INSERT INTO model_inventory (
                    path, id, display_path, name, scan_root, size, mtime,
                    architecture, context_length, quant_type, has_mtp_head,
                    capabilities_json, file_type, is_shard, last_seen, cache_version
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
                )
                ON CONFLICT(path) DO UPDATE SET
                    id=excluded.id,
                    display_path=excluded.display_path,
                    name=excluded.name,
                    scan_root=excluded.scan_root,
                    size=excluded.size,
                    mtime=excluded.mtime,
                    architecture=excluded.architecture,
                    context_length=excluded.context_length,
                    quant_type=excluded.quant_type,
                    has_mtp_head=excluded.has_mtp_head,
                    capabilities_json=excluded.capabilities_json,
                    file_type=excluded.file_type,
                    is_shard=excluded.is_shard,
                    last_seen=excluded.last_seen,
                    cache_version=excluded.cache_version
            "#,
        )
        .map_err(|e| format!("failed to prepare model inventory upsert: {}", e))?;
    for record in records {
        let capabilities_json = serde_json::to_string(&record.capabilities)
            .map_err(|e| format!("failed to encode model capabilities: {}", e))?;
        stmt.execute(params![
            record.path,
            record.id,
            record.display_path,
            record.name,
            record.scan_root,
            record.size as i64,
            record.mtime as i64,
            record.architecture,
            record.context_length.map(|value| value as i64),
            record.quant_type,
            if record.has_mtp_head { 1 } else { 0 },
            capabilities_json,
            record.file_type,
            if record.is_shard { 1 } else { 0 },
            record.last_seen,
            MODEL_INVENTORY_SCHEMA_VERSION,
        ])
        .map_err(|e| format!("failed to upsert model inventory row: {}", e))?;
    }
    Ok(())
}

fn prune_absent_models_in_connection(
    conn: &Connection,
    scan_roots: &HashSet<String>,
    seen_paths: &HashSet<String>,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT path, scan_root FROM model_inventory")
        .map_err(|e| format!("failed to prepare model inventory prune query: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("failed to query model inventory for pruning: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read model inventory prune row: {}", e))?;

    for (path, scan_root) in rows {
        if scan_roots.contains(&scan_root) && !seen_paths.contains(&path) {
            let _ = conn.execute("DELETE FROM model_inventory WHERE path = ?1", params![path]);
        }
    }
    Ok(())
}

pub fn apply_model_scan(
    records: &[InventoryModelRecord],
    directory_records: &[InventoryDirectoryRecord],
    scan_roots: &HashSet<String>,
    seen_paths: &HashSet<String>,
    seen_dirs: &HashSet<String>,
) -> Result<(), String> {
    let mut conn = open_connection()?;
    let transaction = conn
        .transaction()
        .map_err(|e| format!("failed to start model scan inventory transaction: {e}"))?;
    if !records.is_empty() {
        upsert_model_records_in_connection(&transaction, records)?;
    }
    if !directory_records.is_empty() {
        upsert_directory_records_in_connection(&transaction, directory_records)?;
    }
    if !scan_roots.is_empty() {
        prune_absent_models_in_connection(&transaction, scan_roots, seen_paths)?;
        prune_absent_directories_in_connection(&transaction, "model", scan_roots, seen_dirs)?;
    }
    transaction
        .commit()
        .map_err(|e| format!("failed to commit model scan inventory transaction: {e}"))
}

pub fn delete_model(path: &str) -> Result<(), String> {
    let conn = open_connection()?;
    conn.execute(
        "DELETE FROM model_inventory WHERE path = ?1 OR display_path = ?1",
        params![path],
    )
    .map_err(|e| format!("failed to delete model inventory row: {}", e))?;
    Ok(())
}

pub fn load_engine_index() -> Result<HashMap<String, InventoryEngineRecord>, String> {
    let conn = open_connection()?;
    load_engine_index_from_connection(&conn)
}

fn load_engine_index_from_connection(
    conn: &Connection,
) -> Result<HashMap<String, InventoryEngineRecord>, String> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, name, dir, exe, version, backend, capabilities_json,
                   exe_mtime, scan_root, last_seen, cache_version
            FROM engine_inventory
            "#,
        )
        .map_err(|e| format!("failed to prepare engine inventory query: {}", e))?;
    let rows = stmt
        .query_map([], engine_record_from_row)
        .map_err(|e| format!("failed to query engine inventory: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read engine inventory row: {}", e))?;
    Ok(rows
        .into_iter()
        .filter(|record| record.cache_version == MODEL_INVENTORY_SCHEMA_VERSION)
        .filter(|record| std::path::Path::new(&record.exe).is_file())
        .map(|record| (record.id.clone(), record))
        .collect())
}

pub fn load_engine_scan_indexes() -> Result<EngineScanIndexes, String> {
    let conn = open_connection()?;
    Ok((
        load_engine_index_from_connection(&conn)?,
        load_directory_index_from_connection(&conn, "engine")?,
    ))
}

pub fn list_cached_engines() -> Result<Vec<EngineInfo>, String> {
    let mut records = load_engine_index()?.into_values().collect::<Vec<_>>();
    records.sort_by_key(|record| record.name.to_lowercase());
    Ok(records
        .into_iter()
        .map(|record| record.to_engine_info())
        .collect())
}

fn upsert_engine_records_in_connection(
    conn: &Connection,
    records: &[InventoryEngineRecord],
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            r#"
                INSERT INTO engine_inventory (
                    id, name, dir, exe, version, backend, capabilities_json,
                    exe_mtime, scan_root, last_seen, cache_version
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11
                )
                ON CONFLICT(id) DO UPDATE SET
                    name=excluded.name,
                    dir=excluded.dir,
                    exe=excluded.exe,
                    version=excluded.version,
                    backend=excluded.backend,
                    capabilities_json=excluded.capabilities_json,
                    exe_mtime=excluded.exe_mtime,
                    scan_root=excluded.scan_root,
                    last_seen=excluded.last_seen,
                    cache_version=excluded.cache_version
            "#,
        )
        .map_err(|e| format!("failed to prepare engine inventory upsert: {}", e))?;
    for record in records {
        stmt.execute(params![
            record.id,
            record.name,
            record.dir,
            record.exe,
            record.version,
            record.backend,
            serde_json::to_string(&record.capabilities)
                .map_err(|e| format!("failed to encode engine capabilities: {e}"))?,
            record.exe_mtime as i64,
            record.scan_root,
            record.last_seen,
            MODEL_INVENTORY_SCHEMA_VERSION,
        ])
        .map_err(|e| format!("failed to upsert engine inventory row: {}", e))?;
    }
    Ok(())
}

pub fn update_engine_probe(engine: &EngineInfo) -> Result<(), String> {
    let conn = open_connection()?;
    let capabilities_json = serde_json::to_string(&engine.capabilities)
        .map_err(|e| format!("failed to encode engine capabilities: {e}"))?;
    let changed = conn
        .execute(
            "UPDATE engine_inventory SET version = ?2, capabilities_json = ?3 WHERE id = ?1",
            params![engine.id, engine.version, capabilities_json],
        )
        .map_err(|e| format!("failed to persist engine capability probe: {e}"))?;
    if changed == 0 {
        return Err("engine is no longer present in the scan inventory".to_string());
    }
    Ok(())
}

fn prune_absent_engines_in_connection(
    conn: &Connection,
    scan_roots: &HashSet<String>,
    seen_ids: &HashSet<String>,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT id, scan_root FROM engine_inventory")
        .map_err(|e| format!("failed to prepare engine inventory prune query: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("failed to query engine inventory for pruning: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read engine inventory prune row: {}", e))?;

    for (id, scan_root) in rows {
        if scan_roots.contains(&scan_root) && !seen_ids.contains(&id) {
            let _ = conn.execute("DELETE FROM engine_inventory WHERE id = ?1", params![id]);
        }
    }
    Ok(())
}

pub fn apply_engine_scan(
    records: &[InventoryEngineRecord],
    directory_records: &[InventoryDirectoryRecord],
    scan_roots: &HashSet<String>,
    seen_ids: &HashSet<String>,
    seen_dirs: &HashSet<String>,
) -> Result<(), String> {
    let mut conn = open_connection()?;
    let transaction = conn
        .transaction()
        .map_err(|e| format!("failed to start engine scan inventory transaction: {e}"))?;
    if !records.is_empty() {
        upsert_engine_records_in_connection(&transaction, records)?;
    }
    if !directory_records.is_empty() {
        upsert_directory_records_in_connection(&transaction, directory_records)?;
    }
    if !scan_roots.is_empty() {
        prune_absent_engines_in_connection(&transaction, scan_roots, seen_ids)?;
        prune_absent_directories_in_connection(&transaction, "engine", scan_roots, seen_dirs)?;
    }
    transaction
        .commit()
        .map_err(|e| format!("failed to commit engine scan inventory transaction: {e}"))
}

pub fn delete_engine(id: &str) -> Result<(), String> {
    let conn = open_connection()?;
    conn.execute("DELETE FROM engine_inventory WHERE id = ?1", params![id])
        .map_err(|e| format!("failed to delete engine inventory row: {}", e))?;
    Ok(())
}

fn record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InventoryModelRecord> {
    let capabilities_json: String = row.get(11)?;
    let capabilities =
        serde_json::from_str::<ModelCapabilities>(&capabilities_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                11,
                rusqlite::types::Type::Text,
                Box::new(err),
            )
        })?;
    let context_length_i64: Option<i64> = row.get(8)?;
    Ok(InventoryModelRecord {
        path: row.get(0)?,
        id: row.get(1)?,
        display_path: row.get(2)?,
        name: row.get(3)?,
        scan_root: row.get(4)?,
        size: row.get::<_, i64>(5)?.max(0) as u64,
        mtime: row.get::<_, i64>(6)?.max(0) as u64,
        architecture: row.get(7)?,
        context_length: context_length_i64.map(|value| value.max(0) as u32),
        quant_type: row.get(9)?,
        has_mtp_head: row.get::<_, i64>(10)? != 0,
        capabilities,
        file_type: row.get(12)?,
        is_shard: row.get::<_, i64>(13)? != 0,
        last_seen: row.get(14)?,
        cache_version: row.get(15)?,
    })
}

fn engine_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InventoryEngineRecord> {
    let capabilities_json: String = row.get(6)?;
    let capabilities =
        serde_json::from_str::<EngineCapabilities>(&capabilities_json).unwrap_or_default();
    Ok(InventoryEngineRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        dir: row.get(2)?,
        exe: row.get(3)?,
        version: row.get(4)?,
        backend: row.get(5)?,
        capabilities,
        exe_mtime: row.get::<_, i64>(7)?.max(0) as u64,
        scan_root: row.get(8)?,
        last_seen: row.get(9)?,
        cache_version: row.get(10)?,
    })
}

fn directory_record_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<InventoryDirectoryRecord> {
    Ok(InventoryDirectoryRecord {
        kind: row.get(0)?,
        path: row.get(1)?,
        scan_root: row.get(2)?,
        signature: row.get(3)?,
        last_seen: row.get(4)?,
        cache_version: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_migration_adds_engine_capabilities_to_existing_database() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE engine_inventory (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                dir TEXT NOT NULL,
                exe TEXT NOT NULL,
                version TEXT NOT NULL,
                backend TEXT NOT NULL,
                exe_mtime INTEGER NOT NULL,
                scan_root TEXT NOT NULL,
                last_seen INTEGER NOT NULL,
                cache_version INTEGER NOT NULL
            );
            INSERT INTO engine_inventory (
                id, name, dir, exe, version, backend, exe_mtime,
                scan_root, last_seen, cache_version
            ) VALUES ('engine', 'engine', '.', 'llama-server', 'old', 'CPU', 0, '.', 0, 2);
            "#,
        )
        .unwrap();

        init_schema(&conn).unwrap();
        let capabilities: String = conn
            .query_row(
                "SELECT capabilities_json FROM engine_inventory WHERE id = 'engine'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(capabilities, "{}");
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            MODEL_INVENTORY_SCHEMA_VERSION
        );
    }

    #[test]
    fn legacy_capability_json_defaults_to_an_unprobed_state() {
        let capabilities = serde_json::from_str::<EngineCapabilities>("{}").unwrap();
        assert_eq!(capabilities.status, "unprobed");
        assert!(capabilities.supported_flags.is_empty());
    }
}
