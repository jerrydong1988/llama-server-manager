use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{any, get},
    Json, Router,
};
use futures_util::StreamExt;
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Manager;

use crate::commands::config::update_and_persist;
use crate::commands::server::effective_api_key;
use crate::commands::telemetry::{record_proxy_request, ProxyRequestRecord};
use crate::models::{AppState, InstanceConfig, ProxyConfig, ProxyStatus, ProxyTarget};

static PROXY_TASK_COUNTER: AtomicU32 = AtomicU32::new(0);
const PROXY_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const PROXY_ABORT_TIMEOUT: Duration = Duration::from_secs(1);

struct ResolvedProxyTarget {
    public: ProxyTarget,
    api_key: String,
    api_prefix: String,
    telemetry_session_id: Option<String>,
}

struct ProxyTelemetryGuard {
    session_id: Option<String>,
    task_id: u32,
    model: Option<String>,
    target_instance_id: String,
    http_status: u16,
    started_at: std::time::Instant,
    recorded: bool,
}

impl ProxyTelemetryGuard {
    fn record_once(&mut self, error_text: Option<String>) {
        if self.recorded {
            return;
        }
        self.recorded = true;
        let _ = record_proxy_request(
            self.session_id.as_deref(),
            &ProxyRequestRecord {
                task_id: self.task_id,
                model: self.model.clone(),
                target_instance_id: self.target_instance_id.clone(),
                http_status: Some(self.http_status),
                duration_ms: self.started_at.elapsed().as_secs_f64() * 1000.0,
                error_text,
            },
        );
    }
}

impl Drop for ProxyTelemetryGuard {
    fn drop(&mut self) {
        self.record_once(Some(
            "client disconnected before upstream stream completed".to_string(),
        ));
    }
}

fn proxy_bound_addr(config: &ProxyConfig) -> String {
    format!("{}:{}", config.host, config.port)
}

fn proxy_bind_error_message(bind_addr: &str, err: &std::io::Error) -> String {
    if err.kind() == std::io::ErrorKind::AddrInUse {
        format!(
            "failed to bind proxy {}: address is already in use. If background keep-alive was enabled, another manager process may still be serving this route from the tray. Exit the old tray process or choose another port.",
            bind_addr
        )
    } else {
        format!("failed to bind proxy {}: {}", bind_addr, err)
    }
}

async fn await_proxy_task_shutdown(
    shutdown_sender: Option<tokio::sync::oneshot::Sender<()>>,
    server_task: Option<tokio::task::JoinHandle<()>>,
) -> Result<(), String> {
    if let Some(sender) = shutdown_sender {
        let _ = sender.send(());
    }

    if let Some(mut task) = server_task {
        tokio::select! {
            result = &mut task => {
                result.map_err(|err| format!("proxy server task failed during shutdown: {}", err))?;
            }
            _ = tokio::time::sleep(PROXY_SHUTDOWN_TIMEOUT) => {
                task.abort();
                let abort_result = tokio::time::timeout(PROXY_ABORT_TIMEOUT, task)
                    .await
                    .map_err(|_| "proxy server did not stop after abort".to_string())?;
                if let Err(err) = abort_result {
                    if !err.is_cancelled() {
                        return Err(format!("proxy server task failed during abort: {}", err));
                    }
                }
                return Err("proxy server did not stop within 3 seconds; forced shutdown was requested".to_string());
            }
        }
    }

    Ok(())
}

async fn discard_finished_proxy_task(state: &AppState) {
    let task = {
        let mut guard = state.proxy_task.lock().unwrap();
        if guard
            .as_ref()
            .map(|task| task.is_finished())
            .unwrap_or(false)
        {
            guard.take()
        } else {
            None
        }
    };

    if let Some(task) = task {
        let _ = task.await;
    }
}

async fn shutdown_proxy_runtime(state: &AppState) -> Result<(), String> {
    let sender = state.proxy_shutdown.lock().unwrap().take();
    let task = state.proxy_task.lock().unwrap().take();
    let result = await_proxy_task_shutdown(sender, task).await;

    *state.proxy_bound_addr.lock().unwrap() = None;
    if let Err(err) = &result {
        *state.proxy_last_error.lock().unwrap() = Some(err.clone());
    }

    result
}

fn next_proxy_task_id() -> u32 {
    let existing = PROXY_TASK_COUNTER.load(Ordering::Relaxed);
    if existing == 0 {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u32)
            .unwrap_or(1)
            | 0x8000_0000;
        let seed = seed.max(1);
        let _ = PROXY_TASK_COUNTER.compare_exchange(0, seed, Ordering::Relaxed, Ordering::Relaxed);
    }
    PROXY_TASK_COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn is_local_bind_host(host: &str) -> bool {
    matches!(
        host.trim(),
        "" | "localhost" | "127.0.0.1" | "::1" | "[::1]"
    )
}

fn proxy_status_from_state(state: &AppState) -> ProxyStatus {
    let config = state.proxy_config.lock().unwrap().clone();
    let running = state.proxy_shutdown.lock().unwrap().is_some();
    let active_routes = config.routes.iter().filter(|route| route.enabled).count();
    let last_error = state.proxy_last_error.lock().unwrap().clone();
    let actual_bound_addr = state.proxy_bound_addr.lock().unwrap().clone();
    ProxyStatus {
        running,
        bound_addr: actual_bound_addr.unwrap_or_else(|| proxy_bound_addr(&config)),
        active_routes,
        last_error,
    }
}

fn normalize_host(host: &str) -> String {
    if host == "0.0.0.0" {
        "127.0.0.1".into()
    } else {
        host.to_string()
    }
}

fn proxy_target_from_instance(id: &str, config: &InstanceConfig, running: bool) -> ProxyTarget {
    ProxyTarget {
        instance_id: id.to_string(),
        name: config.name.clone(),
        alias: config.alias.clone(),
        host: normalize_host(&config.host),
        port: config.port,
        running,
    }
}

fn list_proxy_targets_inner(state: &AppState) -> Vec<ProxyTarget> {
    let instances = state.instances.lock().unwrap().clone();
    let running = state.running.lock().unwrap().clone();
    instances
        .iter()
        .map(|(id, config)| proxy_target_from_instance(id, config, running.contains_key(id)))
        .collect()
}

fn resolve_proxy_target(
    state: &AppState,
    requested_model: Option<&str>,
) -> Option<ResolvedProxyTarget> {
    let proxy_config = state.proxy_config.lock().unwrap().clone();
    let instances = state.instances.lock().unwrap().clone();
    let running = state.running.lock().unwrap().clone();

    let mut candidate_ids: Vec<String> = Vec::new();
    if let Some(model) = requested_model {
        let mut routes = proxy_config.routes.clone();
        routes.sort_by_key(|route| route.priority);
        for route in routes {
            if route.enabled
                && route.model_alias == model
                && !route.target_instance_id.is_empty()
                && running.contains_key(&route.target_instance_id)
            {
                candidate_ids.push(route.target_instance_id);
            }
        }

        for (id, config) in &instances {
            if running.contains_key(id)
                && (config.alias == model || config.name == model || id == model)
            {
                candidate_ids.push(id.clone());
            }
        }
    }

    if !proxy_config.default_instance_id.is_empty()
        && running.contains_key(&proxy_config.default_instance_id)
    {
        candidate_ids.push(proxy_config.default_instance_id.clone());
    }

    if proxy_config.routing_strategy == "firstHealthy" || candidate_ids.is_empty() {
        for id in running.keys() {
            candidate_ids.push(id.clone());
        }
    }

    for id in candidate_ids {
        if let (Some(config), Some(running_info)) = (instances.get(&id), running.get(&id)) {
            let public = ProxyTarget {
                instance_id: id.clone(),
                name: config.name.clone(),
                alias: config.alias.clone(),
                host: normalize_host(&running_info.host),
                port: running_info.port,
                running: true,
            };
            return Some(ResolvedProxyTarget {
                public,
                api_key: effective_api_key(config),
                api_prefix: config.api_prefix.clone(),
                telemetry_session_id: running_info.telemetry_session_id.clone(),
            });
        }
    }

    None
}

fn requested_model_from_body(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("model")
                .and_then(|model| model.as_str())
                .map(|s| s.to_string())
        })
}

fn is_proxy_authorized(config: &ProxyConfig, headers: &HeaderMap) -> bool {
    if config.public_api_key.trim().is_empty() {
        return true;
    }
    let expected = config.public_api_key.trim();
    let bearer = format!("Bearer {}", expected);
    let auth_ok = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(|value| value == bearer || value == expected)
        .unwrap_or(false);
    let api_key_ok = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(|value| value == expected)
        .unwrap_or(false);
    auth_ok || api_key_ok
}

fn target_url(target: &ResolvedProxyTarget, uri: &Uri) -> String {
    let original_path = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(uri.path());
    let prefix = target.api_prefix.trim_matches('/');
    if prefix.is_empty() {
        format!(
            "http://{}:{}{}",
            target.public.host, target.public.port, original_path
        )
    } else if original_path == format!("/{}", prefix)
        || original_path.starts_with(&format!("/{}/", prefix))
        || original_path.starts_with(&format!("/{}?", prefix))
    {
        format!(
            "http://{}:{}{}",
            target.public.host, target.public.port, original_path
        )
    } else {
        format!(
            "http://{}:{}/{}{}",
            target.public.host, target.public.port, prefix, original_path
        )
    }
}

fn plain_response(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

async fn proxy_health(State(app): State<tauri::AppHandle>) -> Json<ProxyStatus> {
    let state = app.state::<AppState>();
    Json(proxy_status_from_state(&state))
}

async fn proxy_index(State(app): State<tauri::AppHandle>) -> Json<serde_json::Value> {
    let state = app.state::<AppState>();
    let status = proxy_status_from_state(&state);
    Json(json!({
        "service": "llama-server-manager routing proxy",
        "status": if status.running { "running" } else { "stopped" },
        "bound_addr": status.bound_addr,
        "active_routes": status.active_routes,
        "endpoints": {
            "health": "/health",
            "models": "/v1/models",
            "chat_completions": "/v1/chat/completions",
            "completions": "/v1/completions",
            "embeddings": "/v1/embeddings"
        },
        "message": "Use OpenAI-compatible clients against the /v1 endpoints."
    }))
}

async fn proxy_models(State(app): State<tauri::AppHandle>) -> Json<serde_json::Value> {
    let state = app.state::<AppState>();
    let config = state.proxy_config.lock().unwrap().clone();
    let targets = list_proxy_targets_inner(&state);
    let mut ids: Vec<String> = config
        .routes
        .iter()
        .filter(|route| route.enabled && !route.model_alias.is_empty())
        .map(|route| route.model_alias.clone())
        .collect();

    for target in targets.iter().filter(|target| target.running) {
        if !target.alias.is_empty() {
            ids.push(target.alias.clone());
        }
        ids.push(target.instance_id.clone());
        ids.push(target.name.clone());
    }
    ids.sort();
    ids.dedup();

    Json(json!({
        "object": "list",
        "data": ids.into_iter().map(|id| json!({
            "id": id,
            "object": "model",
            "owned_by": "llama-server-manager"
        })).collect::<Vec<_>>()
    }))
}

async fn proxy_openai(
    State(app): State<tauri::AppHandle>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let state = app.state::<AppState>();
    let proxy_config = state.proxy_config.lock().unwrap().clone();
    if !is_proxy_authorized(&proxy_config, &headers) {
        return plain_response(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let requested_model = requested_model_from_body(&body);
    let target = match resolve_proxy_target(&state, requested_model.as_deref()) {
        Some(target) => target,
        None => {
            return plain_response(
                StatusCode::BAD_GATEWAY,
                "no running instance matches the requested model",
            )
        }
    };
    let started_at = std::time::Instant::now();
    let proxy_task_id = next_proxy_task_id();

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(
            proxy_config.timeout_ms.max(1_000),
        ))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            return plain_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("proxy client error: {}", err),
            )
        }
    };

    let reqwest_method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
        Ok(method) => method,
        Err(err) => {
            return plain_response(StatusCode::BAD_REQUEST, &format!("invalid method: {}", err))
        }
    };

    let mut request = client.request(reqwest_method, target_url(&target, &uri));
    for (name, value) in headers.iter() {
        let lower = name.as_str().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "host" | "content-length" | "connection" | "authorization"
        ) {
            continue;
        }
        request = request.header(name.as_str(), value.as_bytes());
    }
    if !target.api_key.trim().is_empty() {
        request = request.bearer_auth(target.api_key.trim());
    }

    let response = match request.body(body.to_vec()).send().await {
        Ok(response) => response,
        Err(err) => {
            let _ = record_proxy_request(
                target.telemetry_session_id.as_deref(),
                &ProxyRequestRecord {
                    task_id: proxy_task_id,
                    model: requested_model.clone(),
                    target_instance_id: target.public.instance_id.clone(),
                    http_status: None,
                    duration_ms: started_at.elapsed().as_secs_f64() * 1000.0,
                    error_text: Some(err.to_string()),
                },
            );
            return plain_response(
                StatusCode::BAD_GATEWAY,
                &format!("upstream request failed: {}", err),
            );
        }
    };

    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for (name, value) in response.headers().iter() {
        let lower = name.as_str().to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "connection" | "content-length" | "transfer-encoding"
        ) {
            continue;
        }
        if let (Ok(header_name), Ok(header_value)) = (
            HeaderName::from_bytes(name.as_str().as_bytes()),
            HeaderValue::from_bytes(value.as_bytes()),
        ) {
            builder = builder.header(header_name, header_value);
        }
    }

    let http_status = status.as_u16();
    let status_success = status.is_success();
    let telemetry_guard = ProxyTelemetryGuard {
        session_id: target.telemetry_session_id.clone(),
        task_id: proxy_task_id,
        model: requested_model.clone(),
        target_instance_id: target.public.instance_id.clone(),
        http_status,
        started_at,
        recorded: false,
    };
    let upstream_stream = Box::pin(response.bytes_stream());
    let stream = futures_util::stream::unfold(
        (upstream_stream, false, telemetry_guard),
        move |(mut upstream_stream, finalized, mut telemetry_guard)| {
            async move {
                if finalized {
                    return None;
                }
                match upstream_stream.as_mut().next().await {
                    Some(Ok(bytes)) => Some((Ok(bytes), (upstream_stream, false, telemetry_guard))),
                    Some(Err(err)) => {
                        let error_text = err.to_string();
                        telemetry_guard.record_once(Some(error_text.clone()));
                        Some((
                            Err(std::io::Error::new(std::io::ErrorKind::Other, error_text)),
                            (upstream_stream, true, telemetry_guard),
                        ))
                    }
                    None => {
                        telemetry_guard.record_once(if status_success {
                            None
                        } else {
                            Some(format!("upstream returned {}", http_status))
                        });
                        None
                    }
                }
            }
        },
    );
    builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|err| {
            plain_response(
                StatusCode::BAD_GATEWAY,
                &format!("proxy response error: {}", err),
            )
        })
}

fn proxy_router(app: tauri::AppHandle) -> Router {
    Router::new()
        .route("/", get(proxy_index))
        .route("/health", get(proxy_health))
        .route("/v1/models", get(proxy_models))
        .route("/v1/chat/completions", any(proxy_openai))
        .route("/v1/completions", any(proxy_openai))
        .route("/v1/embeddings", any(proxy_openai))
        .with_state(app)
}

#[tauri::command]
pub async fn get_proxy_config(state: tauri::State<'_, AppState>) -> Result<ProxyConfig, String> {
    Ok(state.proxy_config.lock().unwrap().clone())
}

#[tauri::command]
pub async fn save_proxy_config(
    config: ProxyConfig,
    state: tauri::State<'_, AppState>,
) -> Result<ProxyConfig, String> {
    *state.proxy_config.lock().unwrap() = config.clone();
    update_and_persist(&state, |global| {
        global.proxy_config = config.clone();
    })?;
    Ok(config)
}

#[tauri::command]
pub async fn get_proxy_status(state: tauri::State<'_, AppState>) -> Result<ProxyStatus, String> {
    Ok(proxy_status_from_state(&state))
}

#[tauri::command]
pub async fn list_proxy_targets(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ProxyTarget>, String> {
    Ok(list_proxy_targets_inner(&state))
}

#[tauri::command]
pub async fn test_proxy_route(
    model: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<ProxyTarget, String> {
    resolve_proxy_target(&state, model.as_deref())
        .map(|target| target.public)
        .ok_or_else(|| "no running instance matches the requested model".to_string())
}

pub async fn start_proxy_for_app(app: tauri::AppHandle) -> Result<ProxyStatus, String> {
    let state = app.state::<AppState>();
    discard_finished_proxy_task(state.inner()).await;
    if state.proxy_shutdown.lock().unwrap().is_some() {
        return Ok(proxy_status_from_state(&state));
    }

    let mut config = state.proxy_config.lock().unwrap().clone();
    config.enabled = true;
    if !is_local_bind_host(&config.host) && config.public_api_key.trim().is_empty() {
        let msg = "代理监听非本机地址时必须设置公开 API Key".to_string();
        *state.proxy_last_error.lock().unwrap() = Some(msg.clone());
        return Err(msg);
    }
    let bind_addr = proxy_bound_addr(&config);
    let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
        Ok(listener) => listener,
        Err(err) => {
            let msg = proxy_bind_error_message(&bind_addr, &err);
            *state.proxy_last_error.lock().unwrap() = Some(msg.clone());
            return Err(msg);
        }
    };

    if let Err(err) = update_and_persist(&state, |global| {
        global.proxy_config = config.clone();
    }) {
        *state.proxy_last_error.lock().unwrap() = Some(err.clone());
        return Err(err);
    }

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    *state.proxy_shutdown.lock().unwrap() = Some(shutdown_tx);
    *state.proxy_bound_addr.lock().unwrap() = Some(bind_addr.clone());
    *state.proxy_last_error.lock().unwrap() = None;
    *state.proxy_config.lock().unwrap() = config.clone();
    let app_for_server = app.clone();
    let server_task = tokio::spawn(async move {
        let result = axum::serve(listener, proxy_router(app_for_server.clone()))
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
        if let Some(state) = app_for_server.try_state::<AppState>() {
            if let Err(err) = result {
                *state.proxy_last_error.lock().unwrap() =
                    Some(format!("proxy server error: {}", err));
            }
            *state.proxy_shutdown.lock().unwrap() = None;
            *state.proxy_bound_addr.lock().unwrap() = None;
            let _ = state.proxy_task.lock().unwrap().take();
        }
    });
    *state.proxy_task.lock().unwrap() = Some(server_task);

    Ok(proxy_status_from_state(&state))
}

#[tauri::command]
pub async fn start_proxy(app: tauri::AppHandle) -> Result<ProxyStatus, String> {
    start_proxy_for_app(app).await
}

#[tauri::command]
pub async fn stop_proxy(state: tauri::State<'_, AppState>) -> Result<ProxyStatus, String> {
    shutdown_proxy_runtime(state.inner()).await?;
    {
        let mut config = state.proxy_config.lock().unwrap();
        config.enabled = false;
    }
    let config = state.proxy_config.lock().unwrap().clone();
    update_and_persist(&state, |global| {
        global.proxy_config = config.clone();
    })?;
    Ok(proxy_status_from_state(&state))
}

#[tauri::command]
pub async fn restart_proxy(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<ProxyStatus, String> {
    shutdown_proxy_runtime(state.inner()).await?;
    start_proxy_for_app(app).await
}

pub async fn shutdown_proxy_for_app(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(state) = app.try_state::<AppState>() {
        shutdown_proxy_runtime(state.inner()).await
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bind_error_mentions_background_keepalive_when_address_is_in_use() {
        let err = std::io::Error::new(std::io::ErrorKind::AddrInUse, "address already in use");
        let message = super::proxy_bind_error_message("127.0.0.1:11435", &err);

        assert!(message.contains("127.0.0.1:11435"));
        assert!(message.contains("already in use"));
        assert!(message.contains("background keep-alive"));
    }

    #[tokio::test]
    async fn proxy_shutdown_sends_signal_and_waits_for_server_task() {
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _ = shutdown_rx.await;
        });

        let result = super::await_proxy_task_shutdown(Some(shutdown_tx), Some(task)).await;

        assert!(result.is_ok());
    }
}
