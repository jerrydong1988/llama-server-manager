#[allow(dead_code)]
mod catalog;
mod operations;
#[allow(dead_code)]
mod transport;
#[allow(dead_code)] // Also compiled into the console client.
pub mod wire;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{Emitter, Manager};
use wire::{Failure, Request, Response};

impl From<crate::error::AppError> for Failure {
    fn from(error: crate::error::AppError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            retryable: error.retryable,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Task {
    id: String,
    operation: String,
    request_digest: String,
    idempotency_key: Option<String>,
    status: String,
    created_at: i64,
    updated_at: i64,
    result: Option<Value>,
    error: Option<Failure>,
}

struct Host {
    app: tauri::AppHandle,
    token: String,
    directory: PathBuf,
    tasks: Mutex<Vec<Task>>,
    mutation_lock: tokio::sync::Mutex<()>,
    _lock: std::fs::File,
}

fn persist_tasks(directory: &std::path::Path, tasks: &[Task]) -> Result<(), Failure> {
    let value = serde_json::to_vec(tasks).map_err(|e| Failure::new("IO", e.to_string()))?;
    crate::persistence::atomic_write(&directory.join("tasks.json"), &value, None)
        .map_err(Failure::from)
}

impl Host {
    async fn request(self: Arc<Self>, request: Request) -> Response {
        let id = request.id.clone();
        let result = self.execute(request).await.map_err(safe_error);
        Response::result(id, result)
    }

    async fn execute(self: Arc<Self>, mut request: Request) -> Result<Value, Failure> {
        if request.version != wire::VERSION {
            return Err(Failure::new(
                "VERSION_MISMATCH",
                "management protocol version mismatch",
            ));
        }
        if !wire::secret_equal(&request.token, &self.token) {
            return Err(Failure::new(
                "UNAUTHORIZED",
                "management authentication failed",
            ));
        }
        if let Some(key) = request
            .arguments
            .as_object_mut()
            .and_then(|args| args.remove("idempotency_key"))
        {
            let key = key
                .as_str()
                .ok_or_else(|| Failure::new("VALIDATION", "idempotency_key must be a string"))?
                .to_string();
            if request
                .idempotency_key
                .as_ref()
                .is_some_and(|existing| existing != &key)
            {
                return Err(Failure::new("VALIDATION", "conflicting idempotency keys"));
            }
            request.idempotency_key = Some(key);
        }
        if request.id.len() > 128
            || request
                .idempotency_key
                .as_ref()
                .is_some_and(|key| key.is_empty() || key.len() > 128)
        {
            return Err(Failure::new("VALIDATION", "invalid request identity"));
        }
        let operation = catalog::find(&request.operation)
            .ok_or_else(|| Failure::new("NOT_FOUND", "unknown management operation"))?;
        operation.validate(&request.arguments)?;
        match operation.name {
            "service.capabilities" => return Ok(json!({"protocol":wire::VERSION,"version":env!("CARGO_PKG_VERSION"),"operations":catalog::operations().iter().map(|op| json!({"name":op.name,"description":op.description,"permission":op.permission,"schema":op.schema()})).collect::<Vec<_>>() })),
            "tasks.list" => return Ok(json!(self.tasks.lock().unwrap().iter().rev().take(100).map(|task| json!({"id":task.id,"operation":task.operation,"status":task.status,"created_at":task.created_at,"updated_at":task.updated_at})).collect::<Vec<_>>())),
            "tasks.get" => return self.tasks.lock().unwrap().iter().find(|task| task.id == request.arguments["id"].as_str().unwrap_or("")).map(|task| json!(task)).ok_or_else(|| Failure::new("NOT_FOUND", "task not found")),
            _ => {},
        }
        if operation.permission == "read" {
            let mut result =
                operations::dispatch(&self.app, operation.name, &request.arguments).await?;
            wire::redact(&mut result);
            return Ok(result);
        }
        let task_id = {
            let mut tasks = self.tasks.lock().unwrap();
            let digest = wire::digest(
                json!({"operation":request.operation,"arguments":request.arguments})
                    .to_string()
                    .as_bytes(),
            );
            if let Some(key) = &request.idempotency_key {
                if let Some(task) = tasks
                    .iter()
                    .find(|task| task.idempotency_key.as_ref() == Some(key))
                {
                    if task.request_digest != digest {
                        return Err(Failure::new(
                            "IDEMPOTENCY_CONFLICT",
                            "idempotency key was used with different arguments",
                        ));
                    }
                    return Ok(json!({"task_id":task.id,"status":task.status,"replayed":true}));
                }
            }
            if tasks
                .iter()
                .filter(|task| task.status == "queued" || task.status == "running")
                .count()
                >= 32
            {
                return Err(Failure::new("BUSY", "management task queue is full"));
            }
            while tasks.len() >= 200 {
                if let Some(index) = tasks
                    .iter()
                    .position(|task| task.status != "queued" && task.status != "running")
                {
                    tasks.remove(index);
                } else {
                    break;
                }
            }
            let id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().timestamp_millis();
            tasks.push(Task {
                id: id.clone(),
                operation: request.operation.clone(),
                request_digest: digest,
                idempotency_key: request.idempotency_key.clone(),
                status: "queued".into(),
                created_at: now,
                updated_at: now,
                result: None,
                error: None,
            });
            if let Err(error) = persist_tasks(&self.directory, &tasks) {
                tasks.pop();
                return Err(error);
            }
            id
        };
        let host = self.clone();
        let task_copy = task_id.clone();
        tauri::async_runtime::spawn(async move {
            let _guard = host.mutation_lock.lock().await;
            if let Err(error) = host.update_task(&task_copy, "running", None) {
                eprintln!("management task persistence failed: {}", error.message);
                return;
            }
            let result = operations::dispatch(&host.app, &request.operation, &request.arguments)
                .await
                .map(|mut value| {
                    wire::redact(&mut value);
                    value
                })
                .map_err(safe_error);
            let status = if result.is_ok() {
                "succeeded"
            } else {
                "failed"
            };
            if let Err(error) = host.update_task(&task_copy, status, Some(result)) {
                eprintln!("management task persistence failed: {}", error.message);
            }
            let _ = host.app.emit("management-config-changed", json!({"operation":request.operation,"revision":crate::commands::config::management_revision()}));
        });
        Ok(json!({"task_id":task_id,"status":"queued"}))
    }

    fn update_task(
        &self,
        id: &str,
        status: &str,
        result: Option<Result<Value, Failure>>,
    ) -> Result<(), Failure> {
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks
            .iter_mut()
            .find(|task| task.id == id)
            .ok_or_else(|| Failure::new("NOT_FOUND", "task disappeared"))?;
        task.status = status.into();
        task.updated_at = chrono::Utc::now().timestamp_millis();
        match result {
            Some(Ok(value)) => {
                // Completion remains successful even when the full resource snapshot is too large.
                // Keep audit storage bounded; clients can query the resource separately.
                task.result = Some(if value.to_string().len() > 128 * 1024 {
                    json!({"result_omitted":true,"reason":"task snapshot exceeds 128 KiB; query the resource"})
                } else {
                    value
                });
            }
            Some(Err(error)) => task.error = Some(error),
            None => {}
        }
        persist_tasks(&self.directory, &tasks)
    }
}

pub fn start(app: tauri::AppHandle) -> Result<(), String> {
    use fs2::FileExt;
    let data = crate::utils::get_data_dir();
    let directory = transport::ensure_private_dir(&data)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("host.lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock_exclusive()
        .map_err(|_| "another management host owns this data directory".to_string())?;
    let token = transport::initialize_token(&data)?;
    let mut tasks: Vec<Task> = match std::fs::read(directory.join("tasks.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("cannot recover management tasks: {e}"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.to_string()),
    };
    for task in &mut tasks {
        if task.status == "running" || task.status == "queued" {
            task.status = "interrupted".into();
            task.error = Some(Failure::new(
                "INTERRUPTED",
                "host exited before recording completion; inspect resource state before retrying",
            ));
        }
    }
    persist_tasks(&directory, &tasks).map_err(|e| e.message)?;
    let host = Arc::new(Host {
        app: app.clone(),
        token: token.clone(),
        directory,
        tasks: Mutex::new(tasks),
        mutation_lock: tokio::sync::Mutex::new(()),
        _lock: lock,
    });
    tauri::async_runtime::spawn(async move {
        if hidden_start() {
            let state = app.state::<crate::models::AppState>();
            if let Err(error) = crate::commands::scanner::get_cached_scan(state.clone()).await {
                eprintln!("Management inventory restore failed: {error}");
            }
            if let Err(error) =
                crate::commands::download::restore_download_queue(state, app.clone()).await
            {
                eprintln!("Management download restore failed: {error}");
            }
        }
        if let Err(error) =
            transport::serve(data, token, move |request| host.clone().request(request)).await
        {
            eprintln!("Management listener failed: {error}");
        }
    });
    Ok(())
}

pub fn hidden_start() -> bool {
    std::env::args_os().any(|arg| arg == "--management-service")
}

fn safe_error(mut error: Failure) -> Failure {
    error.message = crate::commands::server::redact_command_secrets(&error.message);
    error.message = error.message.chars().take(4096).collect();
    error
}
