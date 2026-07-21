use axum::{
    body::{Body, Bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get},
    Json, Router,
};
use futures_util::{StreamExt, TryStreamExt};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Manager;
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_util::io::StreamReader;

use crate::commands::config::update_and_persist;
use crate::commands::server::{effective_api_key, effective_server_scheme};
use crate::commands::telemetry::{
    current_time_ms, record_proxy_request, record_vector_activity, ProxyRequestRecord,
    VectorActivityRecord,
};
use crate::commands::vector_metrics::VectorEventSource;
use crate::models::{
    public_model_id, AppState, InstanceConfig, ProxyConfig, ProxyRoute, ProxyStatus, ProxyTarget,
};
use crate::vector_policy::ModelWorkload;

static PROXY_TASK_COUNTER: AtomicU32 = AtomicU32::new(0);
static PROXY_HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(30))
        .build()
        .expect("proxy HTTP client configuration must be valid")
});
const PROXY_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const PROXY_ABORT_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
struct VectorRequestMetadata {
    workload: ModelWorkload,
    endpoint: String,
    item_count: u64,
}

fn classify_vector_endpoint(path: &str) -> Option<ModelWorkload> {
    match path {
        "/embedding" | "/embeddings" | "/v1/embeddings" => Some(ModelWorkload::Embedding),
        "/rerank" | "/reranking" | "/v1/rerank" | "/v1/reranking" => Some(ModelWorkload::Reranker),
        _ => None,
    }
}

fn embedding_item_count(value: &serde_json::Value) -> u64 {
    match value {
        serde_json::Value::String(_) => 1,
        serde_json::Value::Array(items) if items.is_empty() => 0,
        serde_json::Value::Array(items) if items.iter().all(serde_json::Value::is_number) => 1,
        serde_json::Value::Array(items) => items.len() as u64,
        _ => 0,
    }
}

fn vector_request_metadata(path: &str, body: &[u8]) -> Option<VectorRequestMetadata> {
    let workload = classify_vector_endpoint(path)?;
    let parsed = serde_json::from_slice::<serde_json::Value>(body).ok();
    let item_count = match workload {
        ModelWorkload::Embedding => parsed
            .as_ref()
            .and_then(|value| value.get("input").or_else(|| value.get("content")))
            .map(embedding_item_count)
            .unwrap_or(0),
        ModelWorkload::Reranker => parsed
            .as_ref()
            .and_then(|value| value.get("documents"))
            .and_then(serde_json::Value::as_array)
            .map(|documents| documents.len() as u64)
            .unwrap_or(0),
        ModelWorkload::Inference => 0,
    };
    Some(VectorRequestMetadata {
        workload,
        endpoint: path.to_string(),
        item_count,
    })
}

fn vector_endpoint_matches_target(
    endpoint_workload: Option<ModelWorkload>,
    target_workload: ModelWorkload,
) -> bool {
    match endpoint_workload {
        Some(workload) => workload == target_workload,
        None => true,
    }
}

fn instance_workload(config: &InstanceConfig) -> ModelWorkload {
    if config.reranking {
        ModelWorkload::Reranker
    } else if config.embedding {
        ModelWorkload::Embedding
    } else {
        ModelWorkload::Inference
    }
}

fn stored_instance_workload(config: &InstanceConfig, stored_workload: &str) -> ModelWorkload {
    if stored_workload.trim().is_empty() {
        instance_workload(config)
    } else {
        ModelWorkload::from_storage(stored_workload)
    }
}

fn stored_target_matches_endpoint(
    config: &InstanceConfig,
    stored_workload: &str,
    endpoint_workload: Option<ModelWorkload>,
) -> bool {
    vector_endpoint_matches_target(
        endpoint_workload,
        stored_instance_workload(config, stored_workload),
    )
}

struct ResolvedProxyTarget {
    public: ProxyTarget,
    upstream_model_id: String,
    api_key: String,
    api_prefix: String,
    scheme: &'static str,
    telemetry_session_id: Option<String>,
    workload: ModelWorkload,
}

#[derive(Clone)]
pub(crate) struct ProxyRuntimeSnapshot {
    pub config: ProxyConfig,
    pub instances: HashMap<String, InstanceConfig>,
    pub running: HashMap<String, crate::models::RunningInstance>,
    pub bound_addr: String,
    pub last_error: Option<String>,
}

pub(crate) trait ProxyDataSource: Send + Sync {
    fn proxy_snapshot(&self) -> ProxyRuntimeSnapshot;
}

#[derive(Clone)]
struct ProxyRouterState {
    source: Arc<dyn ProxyDataSource>,
}

#[derive(Clone)]
struct TauriProxyDataSource {
    app: tauri::AppHandle,
}

impl ProxyDataSource for TauriProxyDataSource {
    fn proxy_snapshot(&self) -> ProxyRuntimeSnapshot {
        let state = self.app.state::<AppState>();
        let config = state.proxy_config.lock().unwrap().clone();
        let bound_addr = state
            .proxy_bound_addr
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| proxy_bound_addr(&config));
        let last_error = state.proxy_last_error.lock().unwrap().clone();
        let instances = state.instances.lock().unwrap().clone();
        let running = state.running.lock().unwrap().clone();
        ProxyRuntimeSnapshot {
            bound_addr,
            last_error,
            instances,
            running,
            config,
        }
    }
}

struct ProxyTelemetryGuard {
    session_id: Option<String>,
    task_id: u32,
    model: Option<String>,
    target_instance_id: String,
    http_status: u16,
    started_at: std::time::Instant,
    started_at_ms: i64,
    vector_metadata: Option<VectorRequestMetadata>,
    recorded: bool,
}

struct ProxyTelemetryRecord {
    task_id: u32,
    model: Option<String>,
    target_instance_id: String,
    http_status: Option<u16>,
    started_at_ms: i64,
    duration_ms: f64,
    error_text: Option<String>,
}

impl ProxyTelemetryGuard {
    fn record_once(&mut self, error_text: Option<String>) {
        if self.recorded {
            return;
        }
        self.recorded = true;
        let _ = record_proxy_telemetry(
            self.session_id.as_deref(),
            &ProxyTelemetryRecord {
                task_id: self.task_id,
                model: self.model.clone(),
                target_instance_id: self.target_instance_id.clone(),
                http_status: Some(self.http_status),
                started_at_ms: self.started_at_ms,
                duration_ms: self.started_at.elapsed().as_secs_f64() * 1000.0,
                error_text,
            },
            self.vector_metadata.as_ref(),
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

fn record_proxy_telemetry(
    session_id: Option<&str>,
    record: &ProxyTelemetryRecord,
    vector_metadata: Option<&VectorRequestMetadata>,
) -> Result<(), String> {
    if let Some(metadata) = vector_metadata {
        let completed_at = current_time_ms().max(record.started_at_ms);
        crate::commands::monitoring::record_vector_activity(
            &record.target_instance_id,
            session_id,
            metadata.workload,
            crate::commands::monitoring::VectorMetricSource::Proxy,
            completed_at,
            metadata.item_count,
            None,
            record.duration_ms,
            record
                .http_status
                .is_some_and(|status| (200..300).contains(&status))
                && record.error_text.is_none(),
        );
        return record_vector_activity(
            session_id,
            &VectorActivityRecord {
                source: VectorEventSource::Proxy,
                source_event_id: i64::from(record.task_id),
                workload: metadata.workload,
                endpoint: Some(metadata.endpoint.clone()),
                started_at: record.started_at_ms,
                completed_at,
                duration_ms: record.duration_ms,
                item_count: metadata.item_count,
                input_tokens: None,
                http_status: record.http_status,
                error_text: record.error_text.clone(),
            },
        )
        .map(|_| ());
    }
    record_proxy_request(
        session_id,
        &ProxyRequestRecord {
            task_id: record.task_id,
            model: record.model.clone(),
            target_instance_id: record.target_instance_id.clone(),
            http_status: record.http_status,
            duration_ms: record.duration_ms,
            error_text: record.error_text.clone(),
        },
    )
}

fn proxy_bound_addr(config: &ProxyConfig) -> String {
    crate::utils::format_host_port(config.host.trim(), config.port)
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

fn proxy_status_from_snapshot(snapshot: &ProxyRuntimeSnapshot) -> ProxyStatus {
    ProxyStatus {
        running: true,
        bound_addr: snapshot.bound_addr.clone(),
        active_routes: snapshot
            .config
            .routes
            .iter()
            .filter(|route| route.enabled)
            .count(),
        last_error: snapshot.last_error.clone(),
    }
}

fn route_is_configured(route: &ProxyRoute) -> bool {
    route.enabled
        && !route.model_alias.trim().is_empty()
        && !route.target_instance_id.trim().is_empty()
}

fn preferred_public_route<'a>(
    config: &'a ProxyConfig,
    target_instance_id: &str,
) -> Option<&'a ProxyRoute> {
    let target_instance_id = target_instance_id.trim();
    config
        .routes
        .iter()
        .filter(|route| {
            route_is_configured(route) && route.target_instance_id.trim() == target_instance_id
        })
        .min_by_key(|route| route.priority)
}

pub(crate) fn normalize_and_validate_proxy_config(
    mut config: ProxyConfig,
    instances: &HashMap<String, InstanceConfig>,
) -> Result<ProxyConfig, String> {
    config.default_instance_id = config.default_instance_id.trim().to_string();
    let mut route_ids = HashSet::new();
    for route in &mut config.routes {
        route.id = route.id.trim().to_string();
        route.model_alias = route.model_alias.trim().to_string();
        route.target_instance_id = route.target_instance_id.trim().to_string();
        if route.id.is_empty() || !route_ids.insert(route.id.clone()) {
            loop {
                let replacement = uuid::Uuid::new_v4().to_string();
                if route_ids.insert(replacement.clone()) {
                    route.id = replacement;
                    break;
                }
            }
        }
    }

    for (index, route) in config.routes.iter().enumerate() {
        if !route.enabled {
            continue;
        }
        if route.model_alias.trim().is_empty() {
            return Err(format!("第 {} 条已启用路由缺少对外模型名", index + 1));
        }
        if route.target_instance_id.trim().is_empty() {
            return Err(format!("第 {} 条已启用路由缺少目标实例", index + 1));
        }
        if !instances.contains_key(route.target_instance_id.trim()) {
            return Err(format!("第 {} 条已启用路由的目标实例不存在", index + 1));
        }
    }
    Ok(config)
}

#[cfg(test)]
fn validate_proxy_routes(
    config: &ProxyConfig,
    instances: &HashMap<String, InstanceConfig>,
) -> Result<(), String> {
    normalize_and_validate_proxy_config(config.clone(), instances).map(|_| ())
}

fn normalize_proxy_config_for_state(
    state: &AppState,
    config: ProxyConfig,
) -> Result<ProxyConfig, String> {
    let instances = state.instances.lock().unwrap();
    normalize_and_validate_proxy_config(config, &instances)
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
        alias: public_model_id(config),
        host: normalize_host(&config.host),
        port: config.port,
        running,
    }
}

fn list_proxy_targets_inner(state: &AppState) -> Vec<ProxyTarget> {
    let instances = state.instances.lock().unwrap().clone();
    let running = state.running.lock().unwrap().clone();
    list_proxy_targets_from(&instances, &running)
}

fn list_proxy_targets_from(
    instances: &HashMap<String, InstanceConfig>,
    running: &HashMap<String, crate::models::RunningInstance>,
) -> Vec<ProxyTarget> {
    instances
        .iter()
        .map(|(id, stored_config)| {
            let running_info = running.get(id);
            let config = running_info
                .and_then(|info| info.launch_config.as_ref())
                .unwrap_or(stored_config);
            proxy_target_from_instance(id, config, running_info.is_some())
        })
        .collect()
}

fn resolve_proxy_target(
    state: &AppState,
    requested_model: Option<&str>,
    endpoint_workload: Option<ModelWorkload>,
) -> Option<ResolvedProxyTarget> {
    let proxy_config = state.proxy_config.lock().unwrap().clone();
    let running = state.running.lock().unwrap().clone();
    let instances = state.instances.lock().unwrap();
    resolve_proxy_target_from(
        &proxy_config,
        &instances,
        &running,
        requested_model,
        endpoint_workload,
    )
}

fn resolve_proxy_target_from(
    proxy_config: &ProxyConfig,
    instances: &HashMap<String, InstanceConfig>,
    running: &HashMap<String, crate::models::RunningInstance>,
    requested_model: Option<&str>,
    endpoint_workload: Option<ModelWorkload>,
) -> Option<ResolvedProxyTarget> {
    let mut had_candidate = false;
    let routed_target_ids = proxy_config
        .routes
        .iter()
        .filter(|route| route_is_configured(route))
        .map(|route| route.target_instance_id.trim())
        .collect::<HashSet<_>>();
    let requested_model = requested_model
        .map(str::trim)
        .filter(|model| !model.is_empty());
    let resolve_id = |id: &str| {
        let stored_config = instances.get(id)?;
        let running_info = running.get(id)?;
        let config = running_info.launch_config.as_ref().unwrap_or(stored_config);
        if !stored_target_matches_endpoint(config, &running_info.workload, endpoint_workload) {
            return None;
        }
        let workload = stored_instance_workload(config, &running_info.workload);
        Some(ResolvedProxyTarget {
            public: ProxyTarget {
                instance_id: id.to_string(),
                name: config.name.clone(),
                alias: public_model_id(config),
                host: normalize_host(&running_info.host),
                port: running_info.port,
                running: true,
            },
            upstream_model_id: if config.alias.trim().is_empty() {
                config.model_path.trim().to_string()
            } else {
                config.alias.trim().to_string()
            },
            api_key: effective_api_key(config),
            api_prefix: config.api_prefix.clone(),
            scheme: effective_server_scheme(config),
            telemetry_session_id: running_info.telemetry_session_id.clone(),
            workload,
        })
    };

    if let Some(model) = requested_model {
        let mut routes = proxy_config.routes.iter().collect::<Vec<_>>();
        routes.sort_by_key(|route| route.priority);
        for route in routes {
            if route.enabled && route.model_alias.trim() == model {
                had_candidate = true;
                let target_instance_id = route.target_instance_id.trim();
                if !target_instance_id.is_empty() && running.contains_key(target_instance_id) {
                    if let Some(target) = resolve_id(target_instance_id) {
                        return Some(target);
                    }
                }
            }
        }

        // A matching public route is authoritative. If all of its configured
        // targets are unavailable, do not silently send the request to an
        // unrelated model through the generic first-healthy fallback.
        if had_candidate {
            return None;
        }

        for (id, stored_config) in instances.iter() {
            let config = running
                .get(id)
                .and_then(|running_info| running_info.launch_config.as_ref())
                .unwrap_or(stored_config);
            if public_model_id(config) == model || config.name.trim() == model || id == model {
                had_candidate = true;
                if routed_target_ids.contains(id.as_str()) {
                    continue;
                }
                if let Some(target) = resolve_id(id) {
                    return Some(target);
                }
            }
        }

        if had_candidate {
            return None;
        }
    }

    let default_instance_id = proxy_config.default_instance_id.trim();
    if !default_instance_id.is_empty() && running.contains_key(default_instance_id) {
        had_candidate = true;
        if let Some(target) = resolve_id(default_instance_id) {
            return Some(target);
        }
    }

    if proxy_config.routing_strategy == "firstHealthy" || !had_candidate {
        for id in running.keys() {
            if requested_model.is_some() && routed_target_ids.contains(id.as_str()) {
                continue;
            }
            if let Some(target) = resolve_id(id) {
                return Some(target);
            }
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

fn rewrite_request_model(body: &Bytes, upstream_model_id: &str) -> Bytes {
    if upstream_model_id.trim().is_empty() {
        return body.clone();
    }
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return body.clone();
    };
    let Some(object) = value.as_object_mut() else {
        return body.clone();
    };
    object.insert(
        "model".to_string(),
        serde_json::Value::String(upstream_model_id.trim().to_string()),
    );
    serde_json::to_vec(&value)
        .map(Bytes::from)
        .unwrap_or_else(|_| body.clone())
}

fn public_response_model(
    proxy_config: &ProxyConfig,
    target: &ProxyTarget,
    requested_model: Option<&str>,
) -> String {
    let requested = requested_model
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(requested) = requested {
        let route_is_public = proxy_config.routes.iter().any(|route| {
            route.enabled
                && route.target_instance_id.trim() == target.instance_id.trim()
                && route.model_alias.trim() == requested
        });
        if route_is_public {
            return requested.to_string();
        }
        if let Some(route) = preferred_public_route(proxy_config, &target.instance_id) {
            return route.model_alias.trim().to_string();
        }
        if requested == target.alias.trim() {
            return requested.to_string();
        }
        if requested == target.name.trim() && !requested.contains('/') && !requested.contains('\\')
        {
            return requested.to_string();
        }
    }
    if let Some(route) = preferred_public_route(proxy_config, &target.instance_id) {
        return route.model_alias.trim().to_string();
    }
    let alias = target.alias.trim();
    if alias.is_empty() {
        "model".to_string()
    } else {
        alias.to_string()
    }
}

fn rewrite_json_response_model(body: Bytes, public_model_id: &str) -> Bytes {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return body;
    };
    let Some(object) = value.as_object_mut() else {
        return body;
    };
    if !object.contains_key("model") {
        return body;
    }
    object.insert(
        "model".to_string(),
        serde_json::Value::String(public_model_id.to_string()),
    );
    serde_json::to_vec(&value).map(Bytes::from).unwrap_or(body)
}

fn rewrite_sse_response_line(line: &str, public_model_id: &str) -> String {
    let Some(payload) = line.strip_prefix("data:") else {
        return line.to_string();
    };
    let payload = payload.trim_start();
    if payload.is_empty() || payload == "[DONE]" {
        return line.to_string();
    }
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(payload.as_bytes()) else {
        return line.to_string();
    };
    let Some(object) = value.as_object_mut() else {
        return line.to_string();
    };
    if !object.contains_key("model") {
        return line.to_string();
    }
    object.insert(
        "model".to_string(),
        serde_json::Value::String(public_model_id.to_string()),
    );
    let Ok(rewritten) = serde_json::to_string(&value) else {
        return line.to_string();
    };
    format!("data: {rewritten}")
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
        .map(|value| {
            constant_time_eq(value.as_bytes(), bearer.as_bytes())
                || constant_time_eq(value.as_bytes(), expected.as_bytes())
        })
        .unwrap_or(false);
    let api_key_ok = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(|value| constant_time_eq(value.as_bytes(), expected.as_bytes()))
        .unwrap_or(false);
    auth_ok || api_key_ok
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}

fn proxy_request_is_authorized(config: &ProxyConfig, _path: &str, headers: &HeaderMap) -> bool {
    is_proxy_authorized(config, headers)
}

fn authorize_and_strip_proxy_credentials(config: &ProxyConfig, headers: &mut HeaderMap) -> bool {
    if !proxy_request_is_authorized(config, "", headers) {
        return false;
    }
    headers.remove("authorization");
    headers.remove("x-api-key");
    true
}

async fn proxy_auth_middleware(
    State(router_state): State<ProxyRouterState>,
    mut request: Request,
    next: Next,
) -> Response {
    let snapshot = router_state.source.proxy_snapshot();
    if !authorize_and_strip_proxy_credentials(&snapshot.config, request.headers_mut()) {
        return plain_response(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    next.run(request).await
}

fn target_url(target: &ResolvedProxyTarget, uri: &Uri) -> String {
    let original_path = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(uri.path());
    let prefix = target.api_prefix.trim_matches('/');
    let upstream_path = if prefix.is_empty()
        || original_path == format!("/{}", prefix)
        || original_path.starts_with(&format!("/{}/", prefix))
        || original_path.starts_with(&format!("/{}?", prefix))
    {
        original_path.to_string()
    } else {
        format!("/{prefix}{original_path}")
    };
    crate::utils::service_url(
        target.scheme,
        &target.public.host,
        target.public.port,
        "",
        &upstream_path,
    )
}

fn validate_proxy_config_update(
    current: &ProxyConfig,
    next: &ProxyConfig,
    running: bool,
    actual_bound_addr: Option<&str>,
) -> Result<(), String> {
    if !running {
        return Ok(());
    }
    let current_bound_addr = proxy_bound_addr(current);
    let bound_addr = actual_bound_addr.unwrap_or(&current_bound_addr);
    if proxy_bound_addr(next) != bound_addr {
        return Err("代理运行期间不能修改监听地址或端口；请先停止代理再保存".to_string());
    }
    let bound_host = bound_addr
        .strip_prefix('[')
        .and_then(|value| value.split_once(']').map(|(host, _)| host))
        .or_else(|| bound_addr.rsplit_once(':').map(|(host, _)| host))
        .unwrap_or(bound_addr);
    if !is_local_bind_host(bound_host) && next.public_api_key.trim().is_empty() {
        return Err("代理正在监听非本机地址，公开 API Key 不能为空".to_string());
    }
    Ok(())
}

fn connection_header_tokens(headers: &HeaderMap) -> HashSet<String> {
    headers
        .get_all("connection")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}

fn is_hop_by_hop_header(name: &str, connection_tokens: &HashSet<String>) -> bool {
    connection_tokens.contains(name)
        || matches!(
            name,
            "connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
        )
}

fn should_forward_request_header(name: &str, connection_tokens: &HashSet<String>) -> bool {
    !matches!(
        name,
        "host" | "content-length" | "accept-encoding" | "authorization" | "x-api-key"
    ) && !is_hop_by_hop_header(name, connection_tokens)
}

fn plain_response(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

async fn proxy_health(State(router_state): State<ProxyRouterState>) -> Json<ProxyStatus> {
    Json(proxy_status_from_snapshot(
        &router_state.source.proxy_snapshot(),
    ))
}

async fn proxy_index(State(router_state): State<ProxyRouterState>) -> Json<serde_json::Value> {
    let status = proxy_status_from_snapshot(&router_state.source.proxy_snapshot());
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

async fn proxy_models(State(router_state): State<ProxyRouterState>) -> Json<serde_json::Value> {
    let snapshot = router_state.source.proxy_snapshot();
    let config = snapshot.config;
    let targets = list_proxy_targets_from(&snapshot.instances, &snapshot.running);
    let ids = listed_proxy_model_ids(&config, &targets);

    Json(json!({
        "object": "list",
        "data": ids.into_iter().map(|id| json!({
            "id": id,
            "object": "model",
            "owned_by": "llama-server-manager"
        })).collect::<Vec<_>>()
    }))
}

fn listed_proxy_model_ids(config: &ProxyConfig, targets: &[ProxyTarget]) -> Vec<String> {
    let running_ids = targets
        .iter()
        .filter(|target| target.running)
        .map(|target| target.instance_id.as_str())
        .collect::<HashSet<_>>();
    let routed_target_ids = config
        .routes
        .iter()
        .filter(|route| route_is_configured(route))
        .map(|route| route.target_instance_id.trim())
        .collect::<HashSet<_>>();
    let mut ids = config
        .routes
        .iter()
        .filter(|route| {
            route.enabled
                && !route.model_alias.trim().is_empty()
                && running_ids.contains(route.target_instance_id.trim())
        })
        .map(|route| route.model_alias.trim().to_string())
        .collect::<Vec<_>>();

    ids.extend(
        targets
            .iter()
            .filter(|target| {
                target.running
                    && !target.alias.trim().is_empty()
                    && !routed_target_ids.contains(target.instance_id.as_str())
            })
            .map(|target| target.alias.trim().to_string()),
    );
    ids.sort();
    ids.dedup();
    ids
}

async fn proxy_openai(
    State(router_state): State<ProxyRouterState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let snapshot = router_state.source.proxy_snapshot();
    let proxy_config = snapshot.config.clone();
    let requested_model = requested_model_from_body(&body);
    let vector_metadata = vector_request_metadata(uri.path(), &body);
    let target = match resolve_proxy_target_from(
        &snapshot.config,
        &snapshot.instances,
        &snapshot.running,
        requested_model.as_deref(),
        vector_metadata.as_ref().map(|metadata| metadata.workload),
    ) {
        Some(target) => target,
        None => {
            return plain_response(
                StatusCode::BAD_GATEWAY,
                "no running instance matches the requested model",
            )
        }
    };
    if !vector_endpoint_matches_target(
        vector_metadata.as_ref().map(|metadata| metadata.workload),
        target.workload,
    ) {
        return plain_response(
            StatusCode::BAD_REQUEST,
            "selected target does not support the requested vector endpoint",
        );
    }
    let response_model =
        public_response_model(&snapshot.config, &target.public, requested_model.as_deref());
    let upstream_body = rewrite_request_model(&body, &target.upstream_model_id);
    let started_at = std::time::Instant::now();
    let started_at_ms = current_time_ms();
    let proxy_task_id = next_proxy_task_id();

    let reqwest_method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
        Ok(method) => method,
        Err(err) => {
            return plain_response(StatusCode::BAD_REQUEST, &format!("invalid method: {}", err))
        }
    };

    let mut request = PROXY_HTTP_CLIENT
        .request(reqwest_method, target_url(&target, &uri))
        .timeout(Duration::from_millis(proxy_config.timeout_ms.max(1_000)))
        .header("accept-encoding", "identity");
    let connection_tokens = connection_header_tokens(&headers);
    for (name, value) in headers.iter() {
        let lower = name.as_str().to_ascii_lowercase();
        if !should_forward_request_header(&lower, &connection_tokens) {
            continue;
        }
        request = request.header(name.as_str(), value.as_bytes());
    }
    if !target.api_key.trim().is_empty() {
        request = request.bearer_auth(target.api_key.trim());
    }

    let response = match request.body(upstream_body).send().await {
        Ok(response) => response,
        Err(err) => {
            let _ = record_proxy_telemetry(
                target.telemetry_session_id.as_deref(),
                &ProxyTelemetryRecord {
                    task_id: proxy_task_id,
                    model: requested_model.clone(),
                    target_instance_id: target.public.instance_id.clone(),
                    http_status: None,
                    started_at_ms,
                    duration_ms: started_at.elapsed().as_secs_f64() * 1000.0,
                    error_text: Some(err.to_string()),
                },
                vector_metadata.as_ref(),
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
    let response_content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let response_is_sse = response_content_type.contains("text/event-stream");
    let response_is_json = response_content_type.contains("json");
    let response_connection_tokens = connection_header_tokens(response.headers());
    for (name, value) in response.headers().iter() {
        let lower = name.as_str().to_ascii_lowercase();
        if lower == "content-length" || is_hop_by_hop_header(&lower, &response_connection_tokens) {
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
    let mut telemetry_guard = ProxyTelemetryGuard {
        session_id: target.telemetry_session_id.clone(),
        task_id: proxy_task_id,
        model: requested_model.clone(),
        target_instance_id: target.public.instance_id.clone(),
        http_status,
        started_at,
        started_at_ms,
        vector_metadata,
        recorded: false,
    };
    if response_is_json && !response_is_sse {
        let response_body = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(err) => {
                let error_text = err.to_string();
                telemetry_guard.record_once(Some(error_text.clone()));
                return plain_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("proxy response error: {error_text}"),
                );
            }
        };
        telemetry_guard.record_once(if status_success {
            None
        } else {
            Some(format!("upstream returned {}", http_status))
        });
        let response_body = rewrite_json_response_model(response_body, &response_model);
        return builder
            .body(Body::from(response_body))
            .unwrap_or_else(|err| {
                plain_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("proxy response error: {}", err),
                )
            });
    }

    if response_is_sse {
        let upstream_stream = response
            .bytes_stream()
            .map_err(|err| std::io::Error::other(err.to_string()));
        let line_stream = Box::pin(FramedRead::new(
            StreamReader::new(upstream_stream),
            LinesCodec::new_with_max_length(16 * 1024 * 1024),
        ));
        let stream = futures_util::stream::unfold(
            (line_stream, false, telemetry_guard, response_model),
            move |(mut line_stream, finalized, mut telemetry_guard, response_model)| async move {
                if finalized {
                    return None;
                }
                match line_stream.as_mut().next().await {
                    Some(Ok(line)) => {
                        let line = rewrite_sse_response_line(&line, &response_model);
                        Some((
                            Ok::<_, std::io::Error>(Bytes::from(format!("{line}\n"))),
                            (line_stream, false, telemetry_guard, response_model),
                        ))
                    }
                    Some(Err(err)) => {
                        let error_text = err.to_string();
                        telemetry_guard.record_once(Some(error_text.clone()));
                        Some((
                            Err(std::io::Error::other(error_text)),
                            (line_stream, true, telemetry_guard, response_model),
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
            },
        );
        return builder
            .body(Body::from_stream(stream))
            .unwrap_or_else(|err| {
                plain_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("proxy response error: {}", err),
                )
            });
    }

    let upstream_stream = Box::pin(response.bytes_stream());
    let stream = futures_util::stream::unfold(
        (upstream_stream, false, telemetry_guard),
        move |(mut upstream_stream, finalized, mut telemetry_guard)| async move {
            if finalized {
                return None;
            }
            match upstream_stream.as_mut().next().await {
                Some(Ok(bytes)) => Some((Ok(bytes), (upstream_stream, false, telemetry_guard))),
                Some(Err(err)) => {
                    let error_text = err.to_string();
                    telemetry_guard.record_once(Some(error_text.clone()));
                    Some((
                        Err(std::io::Error::other(error_text)),
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

pub(crate) fn proxy_router_from_source(source: Arc<dyn ProxyDataSource>) -> Router {
    let router_state = ProxyRouterState { source };
    let auth_layer = middleware::from_fn_with_state(router_state.clone(), proxy_auth_middleware);
    Router::new()
        .route("/", get(proxy_index))
        .route("/health", get(proxy_health))
        .route("/v1/models", get(proxy_models))
        .route("/v1/chat/completions", any(proxy_openai))
        .route("/v1/completions", any(proxy_openai))
        .route("/embedding", any(proxy_openai))
        .route("/embeddings", any(proxy_openai))
        .route("/v1/embeddings", any(proxy_openai))
        .route("/rerank", any(proxy_openai))
        .route("/reranking", any(proxy_openai))
        .route("/v1/rerank", any(proxy_openai))
        .route("/v1/reranking", any(proxy_openai))
        .route_layer(auth_layer)
        .with_state(router_state)
}

fn proxy_router(app: tauri::AppHandle) -> Router {
    proxy_router_from_source(Arc::new(TauriProxyDataSource { app }))
}

pub async fn get_proxy_config(state: tauri::State<'_, AppState>) -> Result<ProxyConfig, String> {
    Ok(state.proxy_config.lock().unwrap().clone())
}

pub async fn save_proxy_config(
    config: ProxyConfig,
    state: tauri::State<'_, AppState>,
) -> Result<ProxyConfig, String> {
    let config = {
        let instances = state.instances.lock().unwrap();
        normalize_and_validate_proxy_config(config, &instances)?
    };
    let _transition = state.proxy_lifecycle_lock.lock().await;
    let current = state.proxy_config.lock().unwrap().clone();
    let runtime_status = if crate::runtime_service::manages_instances() {
        Some(crate::runtime_service::ensure_runtime_service().await?)
    } else {
        None
    };
    let running = runtime_status
        .as_ref()
        .map(|status| status.proxy.running)
        .unwrap_or_else(|| state.proxy_shutdown.lock().unwrap().is_some());
    let bound_addr = runtime_status
        .as_ref()
        .map(|status| status.proxy.bound_addr.clone())
        .or_else(|| state.proxy_bound_addr.lock().unwrap().clone());
    validate_proxy_config_update(&current, &config, running, bound_addr.as_deref())?;
    let runtime_mode_changed = config.runtime_service_enabled != current.runtime_service_enabled;
    if runtime_mode_changed && config.runtime_service_enabled {
        let local_running = state.running.lock().unwrap().clone();
        let managed_running = runtime_status
            .as_ref()
            .map(|status| &status.running)
            .ok_or_else(|| "后台运行时状态不可用".to_string())?;
        let unmanaged = local_running
            .keys()
            .filter(|instance_id| !managed_running.contains_key(*instance_id))
            .cloned()
            .collect::<Vec<_>>();
        if !unmanaged.is_empty() {
            return Err(format!(
                "启用独立后台运行时前，请先停止并重新启动这些旧进程实例：{}",
                unmanaged.join(", ")
            ));
        }
    }

    let sync_generation = crate::runtime_service::mark_config_sync_pending();
    let apply_result = async {
        update_and_persist(&state, |global| {
            global.proxy_config = config.clone();
        })?;
        *state.proxy_config.lock().unwrap() = config.clone();
        if crate::runtime_service::manages_instances() {
            crate::runtime_service::sync_app_config(&state).await?;
        }
        if runtime_mode_changed {
            if config.runtime_service_enabled {
                crate::runtime_service::autostart::enable_runtime_autostart()?;
                crate::runtime_service::set_background_enabled(true).await?;
            } else {
                crate::runtime_service::set_background_enabled(false).await?;
                crate::runtime_service::autostart::disable_runtime_autostart()?;
                // Refresh the status after registration removal. The command is
                // idempotent and keeps the runtime's cached registration view honest.
                crate::runtime_service::set_background_enabled(false).await?;
            }
        }
        Ok::<(), String>(())
    }
    .await;

    if let Err(error) = apply_result {
        *state.proxy_config.lock().unwrap() = current.clone();
        let rollback_generation = crate::runtime_service::mark_config_sync_pending();
        let mut rollback_errors = Vec::new();
        if let Err(rollback_error) = update_and_persist(&state, |global| {
            global.proxy_config = current.clone();
        }) {
            rollback_errors.push(rollback_error);
        }
        if crate::runtime_service::manages_instances() {
            if let Err(rollback_error) = crate::runtime_service::sync_app_config(&state).await {
                rollback_errors.push(rollback_error);
            }
        }
        if runtime_mode_changed {
            let autostart_rollback = if current.runtime_service_enabled {
                crate::runtime_service::autostart::enable_runtime_autostart()
            } else {
                crate::runtime_service::autostart::disable_runtime_autostart()
            };
            if let Err(rollback_error) = autostart_rollback {
                rollback_errors.push(rollback_error);
            }
        }
        if crate::runtime_service::manages_instances() {
            if let Err(rollback_error) =
                crate::runtime_service::set_background_enabled(current.runtime_service_enabled)
                    .await
            {
                rollback_errors.push(rollback_error);
            }
        }
        if rollback_errors.is_empty() {
            crate::runtime_service::mark_config_sync_complete(rollback_generation);
        }
        return if rollback_errors.is_empty() {
            Err(error)
        } else {
            Err(format!(
                "{error}; 回滚后台运行时设置时又发生错误：{}",
                rollback_errors.join("; ")
            ))
        };
    }
    crate::runtime_service::mark_config_sync_complete(sync_generation);
    Ok(config)
}

pub async fn get_proxy_status(state: tauri::State<'_, AppState>) -> Result<ProxyStatus, String> {
    if crate::runtime_service::manages_instances() {
        return crate::runtime_service::ensure_runtime_service()
            .await
            .map(|status| status.proxy);
    }
    Ok(proxy_status_from_state(&state))
}

pub async fn list_proxy_targets(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ProxyTarget>, String> {
    if crate::runtime_service::manages_instances() {
        let status = crate::runtime_service::ensure_runtime_service().await?;
        let instances = state.instances.lock().unwrap().clone();
        return Ok(list_proxy_targets_from(&instances, &status.running));
    }
    Ok(list_proxy_targets_inner(&state))
}

pub async fn test_proxy_route(
    model: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<ProxyTarget, String> {
    if crate::runtime_service::manages_instances() {
        let status = crate::runtime_service::ensure_runtime_service().await?;
        let proxy_config = state.proxy_config.lock().unwrap().clone();
        let instances = state.instances.lock().unwrap().clone();
        return resolve_proxy_target_from(
            &proxy_config,
            &instances,
            &status.running,
            model.as_deref(),
            None,
        )
        .map(|target| target.public)
        .ok_or_else(|| "no running instance matches the requested model".to_string());
    }
    resolve_proxy_target(&state, model.as_deref(), None)
        .map(|target| target.public)
        .ok_or_else(|| "no running instance matches the requested model".to_string())
}

async fn start_proxy_locked(app: tauri::AppHandle) -> Result<ProxyStatus, String> {
    let state = app.state::<AppState>();
    discard_finished_proxy_task(state.inner()).await;
    if state.proxy_shutdown.lock().unwrap().is_some() {
        return Ok(proxy_status_from_state(&state));
    }

    let mut config = match normalize_proxy_config_for_state(
        state.inner(),
        state.proxy_config.lock().unwrap().clone(),
    ) {
        Ok(config) => config,
        Err(error) => {
            *state.proxy_last_error.lock().unwrap() = Some(error.clone());
            return Err(error);
        }
    };
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

pub async fn start_proxy_for_app(app: tauri::AppHandle) -> Result<ProxyStatus, String> {
    let state = app.state::<AppState>();
    let _transition = state.proxy_lifecycle_lock.lock().await;
    if crate::runtime_service::manages_instances() {
        let previous = state.proxy_config.lock().unwrap().clone();
        let mut config = match normalize_proxy_config_for_state(state.inner(), previous.clone()) {
            Ok(config) => config,
            Err(error) => {
                *state.proxy_last_error.lock().unwrap() = Some(error.clone());
                return Err(error);
            }
        };
        let was_running = crate::runtime_service::ensure_runtime_service()
            .await?
            .proxy
            .running;
        config.enabled = true;
        update_and_persist(&state, |global| global.proxy_config = config.clone())?;
        *state.proxy_config.lock().unwrap() = config.clone();
        let sync_generation = crate::runtime_service::mark_config_sync_pending();
        let start_result = async {
            crate::runtime_service::sync_app_config(&state).await?;
            crate::runtime_service::start_proxy().await
        }
        .await;
        match start_result {
            Ok(status) => {
                crate::runtime_service::mark_config_sync_complete(sync_generation);
                return Ok(status);
            }
            Err(error) => {
                *state.proxy_config.lock().unwrap() = previous.clone();
                let rollback_generation = crate::runtime_service::mark_config_sync_pending();
                let mut rollback_errors = Vec::new();
                if let Err(rollback_error) = update_and_persist(&state, |global| {
                    global.proxy_config = previous.clone();
                }) {
                    rollback_errors.push(rollback_error);
                }
                if let Err(rollback_error) = crate::runtime_service::sync_app_config(&state).await {
                    rollback_errors.push(rollback_error);
                } else {
                    let lifecycle_result = if was_running {
                        crate::runtime_service::start_proxy().await.map(|_| ())
                    } else {
                        crate::runtime_service::stop_proxy().await.map(|_| ())
                    };
                    if let Err(rollback_error) = lifecycle_result {
                        rollback_errors.push(rollback_error);
                    }
                }
                if rollback_errors.is_empty() {
                    crate::runtime_service::mark_config_sync_complete(rollback_generation);
                    return Err(error);
                }
                return Err(format!(
                    "{error}; 回滚路由启动状态时又发生错误：{}",
                    rollback_errors.join("; ")
                ));
            }
        }
    }
    let app_for_start = app.clone();
    start_proxy_locked(app_for_start).await
}

pub async fn start_proxy(app: tauri::AppHandle) -> Result<ProxyStatus, String> {
    start_proxy_for_app(app).await
}

pub async fn stop_proxy(state: tauri::State<'_, AppState>) -> Result<ProxyStatus, String> {
    let _transition = state.proxy_lifecycle_lock.lock().await;
    if crate::runtime_service::manages_instances() {
        let previous = state.proxy_config.lock().unwrap().clone();
        let was_running = crate::runtime_service::ensure_runtime_service()
            .await?
            .proxy
            .running;
        let mut config = previous.clone();
        config.enabled = false;
        update_and_persist(&state, |global| global.proxy_config = config.clone())?;
        *state.proxy_config.lock().unwrap() = config.clone();
        let sync_generation = crate::runtime_service::mark_config_sync_pending();
        let stop_result = async {
            crate::runtime_service::sync_app_config(&state).await?;
            crate::runtime_service::stop_proxy().await
        }
        .await;
        match stop_result {
            Ok(status) => {
                crate::runtime_service::mark_config_sync_complete(sync_generation);
                return Ok(status);
            }
            Err(error) => {
                *state.proxy_config.lock().unwrap() = previous.clone();
                let rollback_generation = crate::runtime_service::mark_config_sync_pending();
                let mut rollback_errors = Vec::new();
                if let Err(rollback_error) = update_and_persist(&state, |global| {
                    global.proxy_config = previous.clone();
                }) {
                    rollback_errors.push(rollback_error);
                }
                if let Err(rollback_error) = crate::runtime_service::sync_app_config(&state).await {
                    rollback_errors.push(rollback_error);
                } else {
                    let lifecycle_result = if was_running {
                        crate::runtime_service::start_proxy().await.map(|_| ())
                    } else {
                        crate::runtime_service::stop_proxy().await.map(|_| ())
                    };
                    if let Err(rollback_error) = lifecycle_result {
                        rollback_errors.push(rollback_error);
                    }
                }
                if rollback_errors.is_empty() {
                    crate::runtime_service::mark_config_sync_complete(rollback_generation);
                    return Err(error);
                }
                return Err(format!(
                    "{error}; 回滚路由停止状态时又发生错误：{}",
                    rollback_errors.join("; ")
                ));
            }
        }
    }
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

pub async fn restart_proxy(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<ProxyStatus, String> {
    if crate::runtime_service::manages_instances() {
        let _transition = state.proxy_lifecycle_lock.lock().await;
        let current = state.proxy_config.lock().unwrap().clone();
        if !current.enabled {
            return Err("路由服务尚未启用".into());
        }
        let config = match normalize_proxy_config_for_state(state.inner(), current) {
            Ok(config) => config,
            Err(error) => {
                *state.proxy_last_error.lock().unwrap() = Some(error.clone());
                return Err(error);
            }
        };
        update_and_persist(&state, |global| global.proxy_config = config.clone())?;
        *state.proxy_config.lock().unwrap() = config;
        crate::runtime_service::stop_proxy().await?;
        let sync_generation = crate::runtime_service::mark_config_sync_pending();
        crate::runtime_service::sync_app_config(&state).await?;
        let status = crate::runtime_service::start_proxy().await?;
        crate::runtime_service::mark_config_sync_complete(sync_generation);
        return Ok(status);
    }
    let _transition = state.proxy_lifecycle_lock.lock().await;
    let config = match normalize_proxy_config_for_state(
        state.inner(),
        state.proxy_config.lock().unwrap().clone(),
    ) {
        Ok(config) => config,
        Err(error) => {
            *state.proxy_last_error.lock().unwrap() = Some(error.clone());
            return Err(error);
        }
    };
    update_and_persist(&state, |global| global.proxy_config = config.clone())?;
    *state.proxy_config.lock().unwrap() = config;
    shutdown_proxy_runtime(state.inner()).await?;
    start_proxy_locked(app).await
}

pub async fn shutdown_proxy_for_app(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(state) = app.try_state::<AppState>() {
        let _transition = state.proxy_lifecycle_lock.lock().await;
        if crate::runtime_service::manages_instances() {
            let mut config = state.proxy_config.lock().unwrap().clone();
            config.enabled = false;
            update_and_persist(&state, |global| global.proxy_config = config.clone())?;
            *state.proxy_config.lock().unwrap() = config;
            let sync_generation = crate::runtime_service::mark_config_sync_pending();
            crate::runtime_service::sync_app_config(&state).await?;
            crate::runtime_service::stop_proxy().await?;
            crate::runtime_service::mark_config_sync_complete(sync_generation);
            return Ok(());
        }
        shutdown_proxy_runtime(state.inner()).await
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::models::{InstanceConfig, ProxyConfig, ProxyRoute, ProxyTarget, RunningInstance};
    use crate::vector_policy::ModelWorkload;
    use axum::body::Body;
    use axum::extract::State;
    use axum::http::{HeaderMap, Uri};
    use axum::response::{IntoResponse, Response};
    use axum::routing::post;
    use axum::{Json, Router};
    use bytes::Bytes;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct TestProxySource {
        snapshot: super::ProxyRuntimeSnapshot,
    }

    impl super::ProxyDataSource for TestProxySource {
        fn proxy_snapshot(&self) -> super::ProxyRuntimeSnapshot {
            self.snapshot.clone()
        }
    }

    async fn spawn_test_router(
        router: Router,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (address, task)
    }

    async fn mock_private_model_upstream(
        State(received_models): State<Arc<Mutex<Vec<String>>>>,
        Json(body): Json<serde_json::Value>,
    ) -> Response {
        received_models.lock().unwrap().push(
            body.get("model")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        );
        if body.get("stream").and_then(serde_json::Value::as_bool) == Some(true) {
            return Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from(concat!(
                    "data: {\"id\":\"chatcmpl-test\",\"model\":\"C:\\\\private\\\\model.gguf\",\"choices\":[]}\n\n",
                    "data: [DONE]\n\n"
                )))
                .unwrap();
        }
        Json(json!({
            "id": "chatcmpl-test",
            "model": r"C:\private\model.gguf",
            "choices": []
        }))
        .into_response()
    }

    #[test]
    fn target_url_brackets_ipv6_and_preserves_prefix_and_query() {
        let target = super::ResolvedProxyTarget {
            public: ProxyTarget {
                instance_id: "ipv6".into(),
                name: "IPv6".into(),
                alias: "".into(),
                host: "::1".into(),
                port: 8080,
                running: true,
            },
            upstream_model_id: "model".into(),
            api_key: String::new(),
            api_prefix: "v1".into(),
            scheme: "https",
            telemetry_session_id: None,
            workload: ModelWorkload::Inference,
        };
        let uri: Uri = "/models?limit=1".parse().unwrap();

        assert_eq!(
            super::target_url(&target, &uri),
            "https://[::1]:8080/v1/models?limit=1"
        );
    }

    #[test]
    fn model_discovery_exposes_only_public_ids_for_running_targets() {
        let config = ProxyConfig {
            routes: vec![
                ProxyRoute {
                    model_alias: "route-model".into(),
                    target_instance_id: "internal-running-uuid".into(),
                    ..ProxyRoute::default()
                },
                ProxyRoute {
                    model_alias: "stopped-route".into(),
                    target_instance_id: "internal-stopped-uuid".into(),
                    ..ProxyRoute::default()
                },
            ],
            ..ProxyConfig::default()
        };
        let targets = vec![
            ProxyTarget {
                instance_id: "internal-running-uuid".into(),
                name: "Friendly name".into(),
                alias: "public-model".into(),
                host: "127.0.0.1".into(),
                port: 8080,
                running: true,
            },
            ProxyTarget {
                instance_id: "internal-stopped-uuid".into(),
                name: "Stopped name".into(),
                alias: "stopped-model".into(),
                host: "127.0.0.1".into(),
                port: 8081,
                running: false,
            },
            ProxyTarget {
                instance_id: "unrouted-running-uuid".into(),
                name: "Unrouted name".into(),
                alias: "unrouted-model".into(),
                host: "127.0.0.1".into(),
                port: 8082,
                running: true,
            },
        ];

        assert_eq!(
            super::listed_proxy_model_ids(&config, &targets),
            vec!["route-model".to_string(), "unrouted-model".to_string()]
        );
    }

    #[test]
    fn proxy_target_derives_a_safe_alias_when_configuration_is_empty() {
        let config = InstanceConfig {
            name: String::new(),
            model_path: r"C:\private\models\Safe-Model-Q8_0.gguf".into(),
            alias: String::new(),
            ..InstanceConfig::default()
        };

        let target = super::proxy_target_from_instance("internal-uuid", &config, true);

        assert_eq!(target.alias, "Safe-Model-Q8_0");
        assert_ne!(target.alias, target.instance_id);
        assert!(!target.alias.contains("private"));
    }

    #[test]
    fn proxy_translates_model_ids_in_requests_json_and_sse_responses() {
        let request = Bytes::from_static(br#"{"model":"route-model","messages":[]}"#);
        let rewritten = super::rewrite_request_model(&request, "backend-model");
        let rewritten_json: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();
        assert_eq!(rewritten_json["model"], "backend-model");

        let response = Bytes::from_static(
            br#"{"id":"chatcmpl-test","model":"C:\\private\\model.gguf","choices":[]}"#,
        );
        let rewritten = super::rewrite_json_response_model(response, "route-model");
        let rewritten_json: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();
        assert_eq!(rewritten_json["model"], "route-model");
        assert!(!String::from_utf8_lossy(&rewritten).contains("private"));

        let sse = super::rewrite_sse_response_line(
            r#"data: {"id":"chatcmpl-test","model":"C:\\private\\model.gguf","choices":[]}"#,
            "route-model",
        );
        assert!(sse.contains(r#""model":"route-model""#));
        assert!(!sse.contains("private"));
        assert_eq!(
            super::rewrite_sse_response_line("data: [DONE]", "route-model"),
            "data: [DONE]"
        );
    }

    #[test]
    fn response_model_uses_route_alias_and_hides_internal_selectors() {
        let target = ProxyTarget {
            instance_id: "internal-uuid".into(),
            name: "Friendly name".into(),
            alias: "public-model".into(),
            host: "127.0.0.1".into(),
            port: 8080,
            running: true,
        };
        let config = ProxyConfig {
            routes: vec![ProxyRoute {
                model_alias: "route-model".into(),
                target_instance_id: target.instance_id.clone(),
                ..ProxyRoute::default()
            }],
            ..ProxyConfig::default()
        };

        assert_eq!(
            super::public_response_model(&config, &target, Some("route-model")),
            "route-model"
        );
        assert_eq!(
            super::public_response_model(&config, &target, Some("internal-uuid")),
            "route-model"
        );
        assert_eq!(
            super::public_response_model(&config, &target, Some(r"C:\private\model.gguf")),
            "route-model"
        );
    }

    #[test]
    fn explicit_route_hides_instance_alias_from_direct_resolution() {
        let instance_id = "routed-instance".to_string();
        let instance = InstanceConfig {
            id: instance_id.clone(),
            alias: "internal-upstream-alias".into(),
            port: 18080,
            ..InstanceConfig::default()
        };
        let instances = HashMap::from([(instance_id.clone(), instance.clone())]);
        let running = HashMap::from([(
            instance_id.clone(),
            RunningInstance {
                instance_id: instance_id.clone(),
                pid: 1,
                port: 18080,
                host: "127.0.0.1".into(),
                start_time: 0,
                executable_path: String::new(),
                telemetry_session_id: None,
                workload: "inference".into(),
                launch_config: Some(instance),
            },
        )]);
        let config = ProxyConfig {
            default_instance_id: instance_id.clone(),
            routes: vec![ProxyRoute {
                model_alias: "public-route-name".into(),
                target_instance_id: instance_id.clone(),
                ..ProxyRoute::default()
            }],
            ..ProxyConfig::default()
        };

        assert!(super::resolve_proxy_target_from(
            &config,
            &instances,
            &running,
            Some("internal-upstream-alias"),
            None,
        )
        .is_none());
        let routed = super::resolve_proxy_target_from(
            &config,
            &instances,
            &running,
            Some("public-route-name"),
            None,
        )
        .expect("the explicit public name must resolve");
        assert_eq!(routed.public.instance_id, instance_id);
        assert_eq!(routed.upstream_model_id, "internal-upstream-alias");
    }

    #[test]
    fn explicit_routes_fail_over_by_priority_without_using_unrelated_models() {
        let primary_id = "primary-instance".to_string();
        let backup_id = "backup-instance".to_string();
        let primary = InstanceConfig {
            id: primary_id.clone(),
            alias: "primary-upstream".into(),
            port: 18080,
            ..InstanceConfig::default()
        };
        let backup = InstanceConfig {
            id: backup_id.clone(),
            alias: "backup-upstream".into(),
            port: 18081,
            ..InstanceConfig::default()
        };
        let instances = HashMap::from([
            (primary_id.clone(), primary.clone()),
            (backup_id.clone(), backup.clone()),
        ]);
        let mut running = HashMap::from([
            (
                primary_id.clone(),
                RunningInstance {
                    instance_id: primary_id.clone(),
                    pid: 1,
                    port: 18080,
                    host: "127.0.0.1".into(),
                    start_time: 0,
                    executable_path: String::new(),
                    telemetry_session_id: None,
                    workload: "inference".into(),
                    launch_config: Some(primary),
                },
            ),
            (
                backup_id.clone(),
                RunningInstance {
                    instance_id: backup_id.clone(),
                    pid: 2,
                    port: 18081,
                    host: "127.0.0.1".into(),
                    start_time: 0,
                    executable_path: String::new(),
                    telemetry_session_id: None,
                    workload: "inference".into(),
                    launch_config: Some(backup),
                },
            ),
        ]);
        let config = ProxyConfig {
            routes: vec![
                ProxyRoute {
                    model_alias: "public-model".into(),
                    target_instance_id: backup_id.clone(),
                    priority: 20,
                    ..ProxyRoute::default()
                },
                ProxyRoute {
                    model_alias: "public-model".into(),
                    target_instance_id: primary_id.clone(),
                    priority: 10,
                    ..ProxyRoute::default()
                },
            ],
            ..ProxyConfig::default()
        };

        let selected = super::resolve_proxy_target_from(
            &config,
            &instances,
            &running,
            Some("public-model"),
            None,
        )
        .unwrap();
        assert_eq!(selected.public.instance_id, primary_id);

        running.remove(&primary_id);
        let selected = super::resolve_proxy_target_from(
            &config,
            &instances,
            &running,
            Some("public-model"),
            None,
        )
        .unwrap();
        assert_eq!(selected.public.instance_id, backup_id);
        assert!(super::resolve_proxy_target_from(
            &config,
            &instances,
            &running,
            Some("unknown-model"),
            None,
        )
        .is_none());
    }

    #[test]
    fn enabled_routes_require_a_public_name_and_target() {
        let instances = HashMap::from([("target".into(), InstanceConfig::default())]);
        let missing_name = ProxyConfig {
            routes: vec![ProxyRoute {
                target_instance_id: "target".into(),
                ..ProxyRoute::default()
            }],
            ..ProxyConfig::default()
        };
        assert!(super::validate_proxy_routes(&missing_name, &instances)
            .unwrap_err()
            .contains("对外模型名"));

        let missing_target = ProxyConfig {
            routes: vec![ProxyRoute {
                model_alias: "public-model".into(),
                ..ProxyRoute::default()
            }],
            ..ProxyConfig::default()
        };
        assert!(super::validate_proxy_routes(&missing_target, &instances)
            .unwrap_err()
            .contains("目标实例"));

        let unknown_target = ProxyConfig {
            routes: vec![ProxyRoute {
                model_alias: "public-model".into(),
                target_instance_id: "missing-target".into(),
                ..ProxyRoute::default()
            }],
            ..ProxyConfig::default()
        };
        assert!(super::validate_proxy_routes(&unknown_target, &instances)
            .unwrap_err()
            .contains("目标实例不存在"));

        let disabled_draft = ProxyConfig {
            routes: vec![ProxyRoute {
                enabled: false,
                ..ProxyRoute::default()
            }],
            ..ProxyConfig::default()
        };
        assert!(super::validate_proxy_routes(&disabled_draft, &instances).is_ok());
    }

    #[test]
    fn route_normalization_trims_fields_and_repairs_empty_or_duplicate_ids() {
        let instances = HashMap::from([("target".into(), InstanceConfig::default())]);
        let config = ProxyConfig {
            default_instance_id: " target ".into(),
            routes: vec![
                ProxyRoute {
                    id: " duplicate ".into(),
                    model_alias: " public-model ".into(),
                    target_instance_id: " target ".into(),
                    ..ProxyRoute::default()
                },
                ProxyRoute {
                    id: "duplicate".into(),
                    model_alias: " backup-model ".into(),
                    target_instance_id: " target ".into(),
                    ..ProxyRoute::default()
                },
                ProxyRoute {
                    id: "   ".into(),
                    model_alias: " third-model ".into(),
                    target_instance_id: " target ".into(),
                    ..ProxyRoute::default()
                },
            ],
            ..ProxyConfig::default()
        };

        let normalized = super::normalize_and_validate_proxy_config(config, &instances).unwrap();
        assert_eq!(normalized.default_instance_id, "target");
        assert_eq!(normalized.routes[0].id, "duplicate");
        assert_eq!(normalized.routes[0].model_alias, "public-model");
        assert_eq!(normalized.routes[0].target_instance_id, "target");
        assert!(normalized.routes.iter().all(|route| !route.id.is_empty()));
        assert_eq!(
            normalized
                .routes
                .iter()
                .map(|route| route.id.as_str())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            normalized.routes.len()
        );
    }

    #[test]
    fn route_resolution_defensively_accepts_legacy_whitespace_target_ids() {
        let instance_id = "target".to_string();
        let instance = InstanceConfig {
            id: instance_id.clone(),
            alias: "upstream-model".into(),
            port: 18080,
            ..InstanceConfig::default()
        };
        let instances = HashMap::from([(instance_id.clone(), instance.clone())]);
        let running = HashMap::from([(
            instance_id.clone(),
            RunningInstance {
                instance_id: instance_id.clone(),
                pid: 1,
                port: 18080,
                host: "127.0.0.1".into(),
                start_time: 0,
                executable_path: String::new(),
                telemetry_session_id: None,
                workload: "inference".into(),
                launch_config: Some(instance),
            },
        )]);
        let config = ProxyConfig {
            routes: vec![ProxyRoute {
                model_alias: " public-model ".into(),
                target_instance_id: " target ".into(),
                ..ProxyRoute::default()
            }],
            ..ProxyConfig::default()
        };

        let resolved = super::resolve_proxy_target_from(
            &config,
            &instances,
            &running,
            Some("public-model"),
            None,
        )
        .expect("legacy whitespace must not make a saved route unreachable");
        assert_eq!(resolved.public.instance_id, instance_id);
        assert_eq!(
            super::listed_proxy_model_ids(&config, &[resolved.public]),
            vec!["public-model".to_string()]
        );
    }

    #[test]
    fn advertised_launch_alias_resolves_to_the_same_running_instance() {
        let launched_id = "launched-instance".to_string();
        let fallback_id = "fallback-instance".to_string();
        let stored_launched = InstanceConfig {
            id: launched_id.clone(),
            alias: "edited-after-start".into(),
            port: 18080,
            ..InstanceConfig::default()
        };
        let launched = InstanceConfig {
            alias: "launch-public".into(),
            ..stored_launched.clone()
        };
        let fallback = InstanceConfig {
            id: fallback_id.clone(),
            alias: "fallback-public".into(),
            port: 18081,
            ..InstanceConfig::default()
        };
        let instances = HashMap::from([
            (launched_id.clone(), stored_launched),
            (fallback_id.clone(), fallback.clone()),
        ]);
        let running = HashMap::from([
            (
                launched_id.clone(),
                RunningInstance {
                    instance_id: launched_id.clone(),
                    pid: 1,
                    port: 18080,
                    host: "127.0.0.1".into(),
                    start_time: 0,
                    executable_path: String::new(),
                    telemetry_session_id: None,
                    workload: "inference".into(),
                    launch_config: Some(launched),
                },
            ),
            (
                fallback_id.clone(),
                RunningInstance {
                    instance_id: fallback_id.clone(),
                    pid: 2,
                    port: 18081,
                    host: "127.0.0.1".into(),
                    start_time: 0,
                    executable_path: String::new(),
                    telemetry_session_id: None,
                    workload: "inference".into(),
                    launch_config: Some(fallback),
                },
            ),
        ]);
        let config = ProxyConfig {
            default_instance_id: fallback_id,
            ..ProxyConfig::default()
        };

        let resolved = super::resolve_proxy_target_from(
            &config,
            &instances,
            &running,
            Some("launch-public"),
            None,
        )
        .expect("advertised launch alias must resolve");

        assert_eq!(resolved.public.instance_id, launched_id);
        assert_eq!(resolved.public.alias, "launch-public");
    }

    #[test]
    fn stopped_unrouted_alias_does_not_fall_through_to_another_instance() {
        let stopped_id = "stopped-instance".to_string();
        let fallback_id = "fallback-instance".to_string();
        let stopped = InstanceConfig {
            id: stopped_id.clone(),
            alias: "stopped-model".into(),
            port: 18080,
            ..InstanceConfig::default()
        };
        let fallback = InstanceConfig {
            id: fallback_id.clone(),
            alias: "fallback-model".into(),
            port: 18081,
            ..InstanceConfig::default()
        };
        let instances = HashMap::from([
            (stopped_id, stopped),
            (fallback_id.clone(), fallback.clone()),
        ]);
        let running = HashMap::from([(
            fallback_id.clone(),
            RunningInstance {
                instance_id: fallback_id.clone(),
                pid: 2,
                port: 18081,
                host: "127.0.0.1".into(),
                start_time: 0,
                executable_path: String::new(),
                telemetry_session_id: None,
                workload: "inference".into(),
                launch_config: Some(fallback),
            },
        )]);
        let config = ProxyConfig {
            default_instance_id: fallback_id.clone(),
            ..ProxyConfig::default()
        };

        assert!(super::resolve_proxy_target_from(
            &config,
            &instances,
            &running,
            Some("stopped-model"),
            None,
        )
        .is_none());

        let fallback = super::resolve_proxy_target_from(
            &config,
            &instances,
            &running,
            Some("unknown-model"),
            None,
        )
        .expect("an unknown model may still use the configured default instance");
        assert_eq!(fallback.public.instance_id, fallback_id);
    }

    #[tokio::test]
    async fn proxy_boundary_hides_private_models_for_json_and_sse() {
        let received_models = Arc::new(Mutex::new(Vec::new()));
        let upstream_router = Router::new()
            .route("/v1/chat/completions", post(mock_private_model_upstream))
            .with_state(received_models.clone());
        let (upstream_address, upstream_task) = spawn_test_router(upstream_router).await;

        let instance_id = "internal-instance-uuid".to_string();
        let instance = InstanceConfig {
            id: instance_id.clone(),
            name: "Public fallback".into(),
            model_path: r"C:\private\model.gguf".into(),
            alias: String::new(),
            host: upstream_address.ip().to_string(),
            port: upstream_address.port(),
            ..InstanceConfig::default()
        };
        let proxy_config = ProxyConfig {
            enabled: true,
            default_instance_id: instance_id.clone(),
            routes: vec![ProxyRoute {
                model_alias: "route-model".into(),
                target_instance_id: instance_id.clone(),
                ..ProxyRoute::default()
            }],
            ..ProxyConfig::default()
        };
        let snapshot = super::ProxyRuntimeSnapshot {
            config: proxy_config,
            instances: HashMap::from([(instance_id.clone(), instance.clone())]),
            running: HashMap::from([(
                instance_id.clone(),
                RunningInstance {
                    instance_id: instance_id.clone(),
                    pid: std::process::id(),
                    port: upstream_address.port(),
                    host: upstream_address.ip().to_string(),
                    start_time: 0,
                    executable_path: String::new(),
                    telemetry_session_id: None,
                    workload: "inference".into(),
                    launch_config: Some(instance),
                },
            )]),
            bound_addr: String::new(),
            last_error: None,
        };
        let proxy_router = super::proxy_router_from_source(Arc::new(TestProxySource { snapshot }));
        let (proxy_address, proxy_task) = spawn_test_router(proxy_router).await;
        let client = reqwest::Client::new();

        let models: serde_json::Value = client
            .get(format!("http://{proxy_address}/v1/models"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let model_ids = models["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|model| model["id"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(model_ids, vec!["route-model"]);
        assert!(!models.to_string().contains("Public fallback"));
        assert!(!models.to_string().contains(&instance_id));
        assert!(!models.to_string().contains("private"));

        let json_response: serde_json::Value = client
            .post(format!("http://{proxy_address}/v1/chat/completions"))
            .json(&json!({ "model": "route-model", "messages": [], "stream": false }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(json_response["model"], "route-model");
        assert!(!json_response.to_string().contains("private"));

        let sse_response = client
            .post(format!("http://{proxy_address}/v1/chat/completions"))
            .json(&json!({ "model": "route-model", "messages": [], "stream": true }))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(sse_response.contains(r#""model":"route-model""#));
        assert!(!sse_response.contains("private"));

        assert_eq!(
            *received_models.lock().unwrap(),
            vec![
                r"C:\private\model.gguf".to_string(),
                r"C:\private\model.gguf".to_string()
            ]
        );
        proxy_task.abort();
        upstream_task.abort();
    }

    #[test]
    fn public_credentials_and_connection_headers_are_never_forwarded() {
        let mut headers = HeaderMap::new();
        headers.insert("connection", "keep-alive, x-private-hop".parse().unwrap());
        let tokens = super::connection_header_tokens(&headers);

        assert!(!super::should_forward_request_header(
            "authorization",
            &tokens
        ));
        assert!(!super::should_forward_request_header("x-api-key", &tokens));
        assert!(!super::should_forward_request_header(
            "accept-encoding",
            &tokens
        ));
        assert!(!super::should_forward_request_header("keep-alive", &tokens));
        assert!(!super::should_forward_request_header(
            "x-private-hop",
            &tokens
        ));
        assert!(super::should_forward_request_header(
            "content-type",
            &tokens
        ));
    }

    #[test]
    fn successful_proxy_authentication_consumes_public_credentials_once() {
        let config = ProxyConfig {
            public_api_key: "secret".into(),
            ..ProxyConfig::default()
        };
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer secret".parse().unwrap());
        headers.insert("x-api-key", "secret".parse().unwrap());
        headers.insert("content-type", "application/json".parse().unwrap());

        assert!(super::authorize_and_strip_proxy_credentials(
            &config,
            &mut headers
        ));
        assert!(!headers.contains_key("authorization"));
        assert!(!headers.contains_key("x-api-key"));
        assert!(headers.contains_key("content-type"));
    }

    #[test]
    fn proxy_auth_rejects_near_matches_and_accepts_both_supported_headers() {
        let config = ProxyConfig {
            public_api_key: "secret".into(),
            ..ProxyConfig::default()
        };
        for value in ["Bearer secre", "Bearer secret!", "secret!", ""] {
            let mut headers = HeaderMap::new();
            headers.insert("authorization", value.parse().unwrap());
            assert!(!super::is_proxy_authorized(&config, &headers));
        }
        let mut bearer = HeaderMap::new();
        bearer.insert("authorization", "Bearer secret".parse().unwrap());
        assert!(super::is_proxy_authorized(&config, &bearer));
        let mut api_key = HeaderMap::new();
        api_key.insert("x-api-key", "secret".parse().unwrap());
        assert!(super::is_proxy_authorized(&config, &api_key));
    }

    #[test]
    fn running_public_proxy_cannot_drop_auth_or_rebind_silently() {
        let current = ProxyConfig {
            host: "0.0.0.0".into(),
            port: 11435,
            public_api_key: "secret".into(),
            ..ProxyConfig::default()
        };
        let mut no_key = current.clone();
        no_key.public_api_key.clear();
        assert!(super::validate_proxy_config_update(
            &current,
            &no_key,
            true,
            Some("0.0.0.0:11435"),
        )
        .is_err());

        let mut rebound = current.clone();
        rebound.port += 1;
        assert!(super::validate_proxy_config_update(
            &current,
            &rebound,
            true,
            Some("0.0.0.0:11435"),
        )
        .is_err());
        assert!(super::validate_proxy_config_update(&current, &no_key, false, None).is_ok());

        let local = ProxyConfig {
            host: "127.0.0.1".into(),
            public_api_key: String::new(),
            ..ProxyConfig::default()
        };
        assert!(
            super::validate_proxy_config_update(&local, &local, true, Some("127.0.0.1:11435"),)
                .is_ok()
        );

        let stale_display = ProxyConfig {
            host: "127.0.0.1".into(),
            port: 11435,
            public_api_key: String::new(),
            ..ProxyConfig::default()
        };
        assert!(super::validate_proxy_config_update(
            &stale_display,
            &stale_display,
            true,
            Some("0.0.0.0:11435"),
        )
        .is_err());
    }

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

    #[test]
    fn proxy_auth_policy_applies_to_discovery_endpoints() {
        let config = ProxyConfig {
            public_api_key: "secret".into(),
            ..ProxyConfig::default()
        };
        let headers = HeaderMap::new();

        for path in ["/", "/health", "/v1/models", "/v1/chat/completions"] {
            assert!(!super::proxy_request_is_authorized(&config, path, &headers));
        }
    }

    #[test]
    fn vector_endpoint_classification_covers_supported_aliases() {
        for path in ["/embedding", "/embeddings", "/v1/embeddings"] {
            assert_eq!(
                super::classify_vector_endpoint(path),
                Some(ModelWorkload::Embedding)
            );
        }
        for path in ["/rerank", "/reranking", "/v1/rerank", "/v1/reranking"] {
            assert_eq!(
                super::classify_vector_endpoint(path),
                Some(ModelWorkload::Reranker)
            );
        }
        assert_eq!(
            super::classify_vector_endpoint("/v1/chat/completions"),
            None
        );
    }

    #[test]
    fn vector_request_metadata_counts_items_without_retaining_content() {
        let cases = [
            (
                "/v1/embeddings",
                br#"{"input":"private text"}"#.as_slice(),
                1,
            ),
            (
                "/v1/embeddings",
                br#"{"input":["private one","private two"]}"#.as_slice(),
                2,
            ),
            ("/embedding", br#"{"content":[12,13,14]}"#.as_slice(), 1),
            (
                "/embeddings",
                br#"{"content":[[1,2],[3,4],[5,6]]}"#.as_slice(),
                3,
            ),
            (
                "/v1/rerank",
                br#"{"query":"private query","documents":["private a","private b","private c"]}"#
                    .as_slice(),
                3,
            ),
        ];

        for (path, body, expected) in cases {
            let metadata = super::vector_request_metadata(path, body).unwrap();
            assert_eq!(metadata.item_count, expected);
            let debug = format!("{metadata:?}");
            assert!(!debug.contains("private"));
        }
        assert_eq!(
            super::vector_request_metadata("/v1/embeddings", b"not-json")
                .unwrap()
                .item_count,
            0
        );
    }

    #[test]
    fn vector_endpoint_requires_matching_target_workload() {
        assert!(super::vector_endpoint_matches_target(
            Some(ModelWorkload::Embedding),
            ModelWorkload::Embedding
        ));
        assert!(!super::vector_endpoint_matches_target(
            Some(ModelWorkload::Reranker),
            ModelWorkload::Embedding
        ));
        assert!(super::vector_endpoint_matches_target(
            None,
            ModelWorkload::Inference
        ));
    }

    #[test]
    fn vector_target_filter_uses_instance_workload() {
        let embedding = InstanceConfig {
            embedding: true,
            ..InstanceConfig::default()
        };
        let reranker = InstanceConfig {
            reranking: true,
            ..InstanceConfig::default()
        };
        let inference = InstanceConfig::default();

        assert!(super::stored_target_matches_endpoint(
            &embedding,
            "",
            Some(ModelWorkload::Embedding)
        ));
        assert!(!super::stored_target_matches_endpoint(
            &inference,
            "",
            Some(ModelWorkload::Embedding)
        ));
        assert!(super::stored_target_matches_endpoint(
            &reranker,
            "",
            Some(ModelWorkload::Reranker)
        ));
        assert!(super::stored_target_matches_endpoint(
            &inference,
            "embedding",
            Some(ModelWorkload::Embedding)
        ));
        assert!(!super::stored_target_matches_endpoint(
            &reranker,
            "embedding",
            Some(ModelWorkload::Reranker)
        ));
    }
}

// IPC compatibility boundary: legacy command internals keep their existing error flow,
// while every registered command serializes a stable AppError object.
#[allow(dead_code, unused_imports, unused_mut)] // Tauri references adapters through generated macros.
pub mod ipc {
    use super::*;

    #[tauri::command]
    pub async fn get_proxy_config(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<ProxyConfig> {
        super::get_proxy_config(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn save_proxy_config(
        config: ProxyConfig,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<ProxyConfig> {
        super::save_proxy_config(config, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn get_proxy_status(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<ProxyStatus> {
        super::get_proxy_status(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn list_proxy_targets(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<Vec<ProxyTarget>> {
        super::list_proxy_targets(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn test_proxy_route(
        model: Option<String>,
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<ProxyTarget> {
        super::test_proxy_route(model, state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn start_proxy(app: tauri::AppHandle) -> crate::error::AppResult<ProxyStatus> {
        super::start_proxy(app)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn stop_proxy(
        state: tauri::State<'_, AppState>,
    ) -> crate::error::AppResult<ProxyStatus> {
        super::stop_proxy(state)
            .await
            .map_err(crate::error::AppError::from)
    }

    #[tauri::command]
    pub async fn restart_proxy(
        state: tauri::State<'_, AppState>,
        app: tauri::AppHandle,
    ) -> crate::error::AppResult<ProxyStatus> {
        super::restart_proxy(state, app)
            .await
            .map_err(crate::error::AppError::from)
    }
}
