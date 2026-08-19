use axum::{
    body::{Body, Bytes},
    extract::{rejection::BytesRejection, DefaultBodyLimit, Extension, Path, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use futures_util::{StreamExt, TryStreamExt};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Manager;
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_util::io::StreamReader;

use crate::commands::config::update_and_persist;
use crate::commands::proxy_protocol::{
    add_format_header, context_limit_error_response, ensure_request_id_header, error_response,
    request_format, response_request_id, rewrite_json_response, rewrite_sse_line, ProxyApiFormat,
};
use crate::commands::proxy_runtime::{
    GlobalRequestPermit, InFlightBodyPermit, RouterRuntime, RoutingCandidate, TargetCapabilities,
    TargetHealthSnapshot, TargetRequestPermit,
};
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
static PROXY_HTTP_CLIENTS: LazyLock<Mutex<HashMap<u64, reqwest::Client>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
const PROXY_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const PROXY_ABORT_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_PROXY_REQUEST_BODY_BYTES: usize = 64 * 1024 * 1024;
const MAX_PROXY_JSON_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOKEN_COUNT_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_COMPLETION_PREFLIGHT_PROMPTS: usize = 16;
const MAX_PROXY_MODEL_SELECTOR_BYTES: usize = 512;
const MAX_ANTHROPIC_REQUEST_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_PROXY_IN_FLIGHT_BODY_BYTES: usize = 256 * 1024 * 1024;
const TARGET_CAPABILITY_MAX_AGE: Duration = Duration::from_secs(60);
const PROXY_API_KEY_HASH_PREFIX: &str = "sha256:";
const SUPPORTED_ANTHROPIC_VERSION: &str = "2023-06-01";

fn proxy_http_client(connect_timeout_ms: u64) -> reqwest::Client {
    let connect_timeout_ms = connect_timeout_ms.clamp(100, 60_000);
    let mut clients = PROXY_HTTP_CLIENTS.lock().unwrap();
    clients
        .entry(connect_timeout_ms)
        .or_insert_with(|| {
            reqwest::Client::builder()
                .connect_timeout(Duration::from_millis(connect_timeout_ms))
                .pool_idle_timeout(Duration::from_secs(90))
                .tcp_keepalive(Duration::from_secs(30))
                .build()
                .expect("proxy HTTP client configuration must be valid")
        })
        .clone()
}

fn hash_proxy_api_key(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    format!(
        "{PROXY_API_KEY_HASH_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
    )
}

fn is_hashed_proxy_api_key(value: &str) -> bool {
    value
        .strip_prefix(PROXY_API_KEY_HASH_PREFIX)
        .is_some_and(|digest| {
            digest.len() == 43
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
}

fn proxy_api_key_matches(stored: &str, presented: &str) -> bool {
    let candidate = if is_hashed_proxy_api_key(stored) {
        hash_proxy_api_key(presented)
    } else {
        presented.to_string()
    };
    constant_time_eq(stored.as_bytes(), candidate.as_bytes())
}

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

#[derive(Clone)]
struct ResolvedProxyTarget {
    public: ProxyTarget,
    upstream_model_id: String,
    api_key: String,
    api_prefix: String,
    scheme: &'static str,
    configured_context_length: Option<u64>,
    telemetry_session_id: Option<String>,
    workload: ModelWorkload,
    route_priority: i32,
    route_weight: u32,
    route_max_concurrent_requests: u32,
}

pub(crate) struct ProxyRequestResolution {
    config: ProxyConfig,
    candidates: Vec<ResolvedProxyTarget>,
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

    fn proxy_config(&self) -> ProxyConfig {
        self.proxy_snapshot().config
    }

    fn resolve_proxy_request(
        &self,
        requested_model: Option<&str>,
        endpoint_workload: Option<ModelWorkload>,
    ) -> ProxyRequestResolution {
        let snapshot = self.proxy_snapshot();
        proxy_request_resolution_from(
            snapshot.config,
            &snapshot.instances,
            &snapshot.running,
            requested_model,
            endpoint_workload,
        )
    }
}

#[derive(Clone)]
struct ProxyRouterState {
    source: Arc<dyn ProxyDataSource>,
    runtime: Arc<RouterRuntime>,
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

    fn proxy_config(&self) -> ProxyConfig {
        self.app
            .state::<AppState>()
            .proxy_config
            .lock()
            .unwrap()
            .clone()
    }

    fn resolve_proxy_request(
        &self,
        requested_model: Option<&str>,
        endpoint_workload: Option<ModelWorkload>,
    ) -> ProxyRequestResolution {
        let state = self.app.state::<AppState>();
        let config = state.proxy_config.lock().unwrap().clone();
        // Keep a single, documented lock order for the two runtime maps. The
        // request path only clones the selected target instead of both maps.
        let instances = state.instances.lock().unwrap();
        let running = state.running.lock().unwrap();
        proxy_request_resolution_from(
            config,
            &instances,
            &running,
            requested_model,
            endpoint_workload,
        )
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
    api_format: ProxyApiFormat,
    recorded: bool,
    runtime: Arc<RouterRuntime>,
    _global_permit: Option<GlobalRequestPermit>,
    _body_permit: Option<InFlightBodyPermit>,
    _target_permit: Option<TargetRequestPermit>,
}

struct ProxyAdmissionGuards {
    global: GlobalRequestPermit,
    body: InFlightBodyPermit,
}

#[derive(Clone)]
struct ProxyAdmissionPermit(Arc<Mutex<Option<ProxyAdmissionGuards>>>);

impl ProxyAdmissionPermit {
    fn new(global: GlobalRequestPermit, body: InFlightBodyPermit) -> Self {
        Self(Arc::new(Mutex::new(Some(ProxyAdmissionGuards {
            global,
            body,
        }))))
    }

    fn take(&self) -> Option<ProxyAdmissionGuards> {
        self.0.lock().ok()?.take()
    }
}

struct ProxyTelemetryRecord {
    task_id: u32,
    model: Option<String>,
    target_instance_id: String,
    http_status: Option<u16>,
    started_at_ms: i64,
    duration_ms: f64,
    error_text: Option<String>,
    api_format: ProxyApiFormat,
}

impl ProxyTelemetryGuard {
    fn record_once(&mut self, error_text: Option<String>) {
        if self.recorded {
            return;
        }
        self.recorded = true;
        self.runtime
            .record_completed(self.started_at.elapsed().as_millis().min(u64::MAX as u128) as u64);
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
                api_format: self.api_format,
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
            api_format: record.api_format.as_str().to_string(),
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
    *state.proxy_router_runtime.lock().unwrap() = None;
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
    let trimmed = host.trim();
    let host = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(trimmed);
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn proxy_status_from_state(state: &AppState) -> ProxyStatus {
    let config = state.proxy_config.lock().unwrap().clone();
    let running = state.proxy_shutdown.lock().unwrap().is_some();
    let active_routes = config.routes.iter().filter(|route| route.enabled).count();
    let last_error = state.proxy_last_error.lock().unwrap().clone();
    let actual_bound_addr = state.proxy_bound_addr.lock().unwrap().clone();
    let bound_addr = actual_bound_addr.unwrap_or_else(|| proxy_bound_addr(&config));
    if running {
        if let Some(runtime) = state.proxy_router_runtime.lock().unwrap().clone() {
            let snapshot = ProxyRuntimeSnapshot {
                config,
                instances: state.instances.lock().unwrap().clone(),
                running: state.running.lock().unwrap().clone(),
                bound_addr,
                last_error,
            };
            return status_with_runtime(&snapshot, &runtime);
        }
    }
    ProxyStatus {
        running,
        bound_addr,
        active_routes,
        healthy_routes: 0,
        unhealthy_routes: active_routes,
        in_flight_requests: 0,
        total_requests: 0,
        last_error,
    }
}

fn proxy_status_from_snapshot(snapshot: &ProxyRuntimeSnapshot) -> ProxyStatus {
    let active_routes = snapshot
        .config
        .routes
        .iter()
        .filter(|route| route.enabled)
        .count();
    ProxyStatus {
        running: true,
        bound_addr: snapshot.bound_addr.clone(),
        active_routes,
        healthy_routes: 0,
        unhealthy_routes: active_routes,
        in_flight_requests: 0,
        total_requests: 0,
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
    config.host = config.host.trim().to_string();
    if config.host.is_empty() {
        config.host = "127.0.0.1".to_string();
    }
    if !is_local_bind_host(&config.host) {
        return Err(
            "内置代理仅允许监听本机回环地址；远程访问请使用提供 TLS 的反向代理或 SSH 隧道。"
                .to_string(),
        );
    }
    config.default_instance_id = config.default_instance_id.trim().to_string();
    config.routing_strategy = match config.routing_strategy.trim() {
        "" | "firstHealthy" | "priorityFailover" => "priorityFailover".to_string(),
        "roundRobin" => "roundRobin".to_string(),
        "leastBusy" => "leastBusy".to_string(),
        "weighted" => "weighted".to_string(),
        unsupported => return Err(format!("不支持的路由策略：{unsupported}")),
    };
    config.timeout_ms = config.timeout_ms.clamp(1_000, 24 * 60 * 60 * 1_000);
    config.connect_timeout_ms = config.connect_timeout_ms.clamp(100, 60_000);
    config.streaming_idle_timeout_ms = config
        .streaming_idle_timeout_ms
        .clamp(1_000, 24 * 60 * 60 * 1_000);
    config.health_check_interval_ms = config.health_check_interval_ms.clamp(1_000, 300_000);
    config.health_check_timeout_ms = config
        .health_check_timeout_ms
        .clamp(250, config.health_check_interval_ms.max(250));
    config.unhealthy_threshold = config.unhealthy_threshold.clamp(1, 100);
    config.recovery_cooldown_ms = config.recovery_cooldown_ms.clamp(1_000, 3_600_000);
    config.max_concurrent_requests = config.max_concurrent_requests.clamp(1, 100_000);
    config.queue_timeout_ms = config.queue_timeout_ms.clamp(10, 300_000);
    config.requests_per_minute = config.requests_per_minute.min(10_000_000);

    let legacy_key = config.public_api_key.trim();
    if !legacy_key.is_empty() {
        let legacy_hash = if is_hashed_proxy_api_key(legacy_key) {
            legacy_key.to_string()
        } else {
            hash_proxy_api_key(legacy_key)
        };
        if let Some(existing) = config.api_keys.iter_mut().find(|api_key| {
            let candidate = api_key.key.trim();
            let candidate_hash = if is_hashed_proxy_api_key(candidate) {
                candidate.to_string()
            } else {
                hash_proxy_api_key(candidate)
            };
            candidate_hash == legacy_hash
        }) {
            existing.enabled = true;
            existing.scopes = vec!["inference".into(), "discovery".into()];
        } else {
            config.api_keys.push(crate::models::ProxyApiKey {
                id: "migrated-legacy-key".into(),
                name: "Migrated legacy key".into(),
                key: legacy_hash,
                enabled: true,
                scopes: vec!["inference".into(), "discovery".into()],
                requests_per_minute: 0,
            });
        }
    }
    config.public_api_key.clear();

    let mut origins = HashSet::new();
    config.cors_allowed_origins = config
        .cors_allowed_origins
        .into_iter()
        .map(|origin| origin.trim().trim_end_matches('/').to_string())
        .filter(|origin| !origin.is_empty())
        .map(|origin| {
            let parsed = reqwest::Url::parse(&origin)
                .map_err(|_| format!("无效的 CORS Origin：{origin}"))?;
            if !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.path() != "/"
                || parsed.query().is_some()
                || parsed.fragment().is_some()
            {
                return Err(format!("CORS Origin 必须是纯 HTTP(S) 源地址：{origin}"));
            }
            Ok(origin)
        })
        .collect::<Result<Vec<_>, String>>()?
        .into_iter()
        .filter(|origin| origins.insert(origin.to_ascii_lowercase()))
        .collect();

    let valid_scopes = HashSet::from(["inference", "discovery"]);
    let mut key_ids = HashSet::new();
    let mut key_values = HashSet::new();
    for api_key in &mut config.api_keys {
        api_key.id = api_key.id.trim().to_string();
        api_key.name = api_key.name.trim().to_string();
        api_key.key = api_key.key.trim().to_string();
        if api_key.id.is_empty() || !key_ids.insert(api_key.id.clone()) {
            loop {
                let replacement = uuid::Uuid::new_v4().to_string();
                if key_ids.insert(replacement.clone()) {
                    api_key.id = replacement;
                    break;
                }
            }
        }
        api_key.scopes = api_key
            .scopes
            .iter()
            .map(|scope| scope.trim().to_ascii_lowercase())
            .filter(|scope| !scope.is_empty())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        api_key.scopes.sort();
        if api_key
            .scopes
            .iter()
            .any(|scope| !valid_scopes.contains(scope.as_str()))
        {
            return Err(format!("API Key {} 包含不支持的权限范围", api_key.name));
        }
        api_key.requests_per_minute = api_key.requests_per_minute.min(10_000_000);
        if api_key.enabled && !is_hashed_proxy_api_key(&api_key.key) && api_key.key.len() < 16 {
            return Err(format!("API Key {} 至少需要 16 个字符", api_key.name));
        }
        if api_key.key.len() >= 16 && !is_hashed_proxy_api_key(&api_key.key) {
            api_key.key = hash_proxy_api_key(&api_key.key);
        }
        if api_key.enabled && !key_values.insert(api_key.key.clone()) {
            return Err("启用的代理 API Key 不能重复".to_string());
        }
    }

    let mut route_ids = HashSet::new();
    for route in &mut config.routes {
        route.id = route.id.trim().to_string();
        route.model_alias = route.model_alias.trim().to_string();
        route.target_instance_id = route.target_instance_id.trim().to_string();
        route.weight = route.weight.clamp(1, 1_000);
        route.max_concurrent_requests = route.max_concurrent_requests.min(100_000);
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

fn resolved_target_for_id(
    instances: &HashMap<String, InstanceConfig>,
    running: &HashMap<String, crate::models::RunningInstance>,
    id: &str,
    endpoint_workload: Option<ModelWorkload>,
    route_priority: i32,
    route_weight: u32,
    route_max_concurrent_requests: u32,
) -> Option<ResolvedProxyTarget> {
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
        configured_context_length: (!config.ctx_size_auto && config.ctx_size > 0)
            .then_some(config.ctx_size as u64),
        telemetry_session_id: running_info.telemetry_session_id.clone(),
        workload,
        route_priority,
        route_weight: route_weight.max(1),
        route_max_concurrent_requests,
    })
}

fn resolve_proxy_candidates_from(
    proxy_config: &ProxyConfig,
    instances: &HashMap<String, InstanceConfig>,
    running: &HashMap<String, crate::models::RunningInstance>,
    requested_model: Option<&str>,
    endpoint_workload: Option<ModelWorkload>,
) -> Vec<ResolvedProxyTarget> {
    let requested_model = requested_model
        .map(str::trim)
        .filter(|model| !model.is_empty());

    if let Some(model) = requested_model {
        let matching_routes = proxy_config
            .routes
            .iter()
            .enumerate()
            .filter(|(_, route)| route_is_configured(route) && route.model_alias.trim() == model)
            .collect::<Vec<_>>();
        if !matching_routes.is_empty() {
            let mut candidates = matching_routes
                .into_iter()
                .filter_map(|(index, route)| {
                    resolved_target_for_id(
                        instances,
                        running,
                        route.target_instance_id.trim(),
                        endpoint_workload,
                        route.priority,
                        route.weight,
                        route.max_concurrent_requests,
                    )
                    .map(|target| (route.priority, index, target))
                })
                .collect::<Vec<_>>();
            candidates.sort_by_key(|(priority, index, _)| (*priority, *index));
            return candidates
                .into_iter()
                .map(|(_, _, target)| target)
                .collect();
        }

        let has_explicit_routes = proxy_config.routes.iter().any(route_is_configured);
        if proxy_config.strict_model_routing && has_explicit_routes {
            return Vec::new();
        }
        let routed_ids = proxy_config
            .routes
            .iter()
            .filter(|route| route_is_configured(route))
            .map(|route| route.target_instance_id.trim())
            .collect::<HashSet<_>>();
        let mut candidates = Vec::new();
        let mut ids = instances.keys().collect::<Vec<_>>();
        ids.sort();
        for id in ids {
            let stored_config = &instances[id];
            let config = running
                .get(id)
                .and_then(|running_info| running_info.launch_config.as_ref())
                .unwrap_or(stored_config);
            let public_match = public_model_id(config) == model;
            let legacy_match = !proxy_config.strict_model_routing
                && (config.name.trim() == model || id.as_str() == model);
            if (public_match || legacy_match) && !routed_ids.contains(id.as_str()) {
                if let Some(target) =
                    resolved_target_for_id(instances, running, id, endpoint_workload, 0, 1, 0)
                {
                    candidates.push(target);
                }
            }
        }
        if candidates.is_empty() && !proxy_config.strict_model_routing {
            let default_instance_id = proxy_config.default_instance_id.trim();
            if !default_instance_id.is_empty() {
                if let Some(target) = resolved_target_for_id(
                    instances,
                    running,
                    default_instance_id,
                    endpoint_workload,
                    0,
                    1,
                    0,
                ) {
                    candidates.push(target);
                }
            }
        }
        return candidates;
    }

    let default_instance_id = proxy_config.default_instance_id.trim();
    if !default_instance_id.is_empty() {
        if let Some(target) = resolved_target_for_id(
            instances,
            running,
            default_instance_id,
            endpoint_workload,
            0,
            1,
            0,
        ) {
            return vec![target];
        }
    }

    let routed_ids = proxy_config
        .routes
        .iter()
        .filter(|route| route_is_configured(route))
        .map(|route| route.target_instance_id.trim())
        .collect::<HashSet<_>>();
    let mut ids = running.keys().collect::<Vec<_>>();
    ids.sort();
    let mut candidates = ids
        .into_iter()
        .filter(|id| !routed_ids.contains(id.as_str()))
        .filter_map(|id| resolved_target_for_id(instances, running, id, endpoint_workload, 0, 1, 0))
        .collect::<Vec<_>>();
    if proxy_config.strict_model_routing && candidates.len() != 1 {
        candidates.clear();
    }
    candidates
}

fn resolve_proxy_target_from(
    proxy_config: &ProxyConfig,
    instances: &HashMap<String, InstanceConfig>,
    running: &HashMap<String, crate::models::RunningInstance>,
    requested_model: Option<&str>,
    endpoint_workload: Option<ModelWorkload>,
) -> Option<ResolvedProxyTarget> {
    resolve_proxy_candidates_from(
        proxy_config,
        instances,
        running,
        requested_model,
        endpoint_workload,
    )
    .into_iter()
    .next()
}

fn all_resolved_targets(snapshot: &ProxyRuntimeSnapshot) -> Vec<ResolvedProxyTarget> {
    let mut ids = snapshot.running.keys().collect::<Vec<_>>();
    ids.sort();
    ids.into_iter()
        .filter_map(|id| {
            resolved_target_for_id(&snapshot.instances, &snapshot.running, id, None, 0, 1, 0)
        })
        .collect()
}

pub(crate) fn proxy_request_resolution_from(
    config: ProxyConfig,
    instances: &HashMap<String, InstanceConfig>,
    running: &HashMap<String, crate::models::RunningInstance>,
    requested_model: Option<&str>,
    endpoint_workload: Option<ModelWorkload>,
) -> ProxyRequestResolution {
    let candidates = resolve_proxy_candidates_from(
        &config,
        instances,
        running,
        requested_model,
        endpoint_workload,
    );
    ProxyRequestResolution { config, candidates }
}

fn requested_model_from_body(body: &[u8]) -> Result<Option<String>, String> {
    let value = serde_json::from_slice::<serde_json::Value>(body)
        .map_err(|_| "request body must be a valid JSON object".to_string())?;
    if !value.is_object() {
        return Err("request body must be a JSON object".to_string());
    }
    let Some(model) = value.get("model").and_then(|model| model.as_str()) else {
        return Ok(None);
    };
    if model.len() > MAX_PROXY_MODEL_SELECTOR_BYTES {
        return Err(format!(
            "model selector exceeds {} bytes",
            MAX_PROXY_MODEL_SELECTOR_BYTES
        ));
    }
    Ok(Some(model.to_string()))
}

fn request_uses_streaming(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("stream").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy)]
enum InputTokenCounter {
    Native(&'static str),
    Completion,
}

#[derive(Debug, Clone, Copy)]
struct ContextPreflightSpec {
    counter: InputTokenCounter,
    error_param: &'static str,
    requested_output_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContextLimitViolation {
    error_param: &'static str,
    input_tokens: Option<u64>,
    requested_output_tokens: u64,
    context_window: u64,
}

fn context_preflight_spec(path: &str, body: &[u8]) -> Option<ContextPreflightSpec> {
    let value = serde_json::from_slice::<serde_json::Value>(body).ok()?;
    let output_tokens = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| value.get(*name).and_then(serde_json::Value::as_u64))
            .unwrap_or(0)
            .max(1)
    };
    match path {
        "/v1/chat/completions" => Some(ContextPreflightSpec {
            counter: InputTokenCounter::Native("/v1/chat/completions/input_tokens"),
            error_param: "messages",
            requested_output_tokens: output_tokens(&["max_completion_tokens", "max_tokens"]),
        }),
        "/v1/responses" => Some(ContextPreflightSpec {
            counter: InputTokenCounter::Native("/v1/responses/input_tokens"),
            error_param: "input",
            requested_output_tokens: output_tokens(&["max_output_tokens"]),
        }),
        "/v1/messages" => Some(ContextPreflightSpec {
            counter: InputTokenCounter::Native("/v1/messages/count_tokens"),
            error_param: "messages",
            requested_output_tokens: output_tokens(&["max_tokens"]),
        }),
        "/v1/completions" => Some(ContextPreflightSpec {
            counter: InputTokenCounter::Completion,
            error_param: "prompt",
            requested_output_tokens: output_tokens(&["max_tokens"]),
        }),
        _ => None,
    }
}

fn completion_tokenize_bodies(body: &[u8]) -> Option<Vec<Bytes>> {
    let value = serde_json::from_slice::<serde_json::Value>(body).ok()?;
    let prompt = value.get("prompt")?;
    let prompts = match prompt {
        serde_json::Value::Array(items)
            if items.iter().all(|item| item.is_string() || item.is_array()) =>
        {
            if items.is_empty() || items.len() > MAX_COMPLETION_PREFLIGHT_PROMPTS {
                return None;
            }
            items.iter().collect::<Vec<_>>()
        }
        _ => vec![prompt],
    };
    prompts
        .into_iter()
        .map(|content| {
            serde_json::to_vec(&json!({
                "content": content,
                "add_special": true,
                "parse_special": true,
            }))
            .ok()
            .map(Bytes::from)
        })
        .collect()
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

fn is_proxy_authorized(api_key: &str, headers: &HeaderMap) -> bool {
    if api_key.trim().is_empty() {
        return true;
    }
    let expected = api_key.trim();
    let auth_ok = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .map(|value| {
            value
                .find(char::is_whitespace)
                .and_then(|separator| {
                    value[..separator]
                        .eq_ignore_ascii_case("bearer")
                        .then(|| value[separator..].trim_start())
                })
                .unwrap_or(value)
        })
        .map(|value| proxy_api_key_matches(expected, value))
        .unwrap_or(false);
    let api_key_ok = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(|value| proxy_api_key_matches(expected, value))
        .unwrap_or(false);
    auth_ok || api_key_ok
}

#[derive(Debug, Clone)]
struct ProxyAuthContext {
    client_id: String,
    requests_per_minute: u32,
    scopes: Vec<String>,
}

fn request_scope(path: &str) -> &'static str {
    if path == "/"
        || matches!(
            path,
            "/health" | "/live" | "/ready" | "/metrics" | "/props" | "/slots"
        )
        || path == "/v1/models"
        || path.starts_with("/v1/models/")
    {
        "discovery"
    } else {
        "inference"
    }
}

fn authenticate_proxy_request(
    config: &ProxyConfig,
    _path: &str,
    headers: &HeaderMap,
) -> Option<ProxyAuthContext> {
    let enabled_keys = config
        .api_keys
        .iter()
        .filter(|api_key| api_key.enabled && !api_key.key.trim().is_empty())
        .collect::<Vec<_>>();
    if enabled_keys.is_empty() {
        return Some(ProxyAuthContext {
            client_id: "anonymous".into(),
            requests_per_minute: config.requests_per_minute,
            scopes: vec!["inference".into(), "discovery".into()],
        });
    }
    for api_key in enabled_keys {
        if is_proxy_authorized(&api_key.key, headers) {
            return Some(ProxyAuthContext {
                client_id: api_key.id.clone(),
                requests_per_minute: if api_key.requests_per_minute == 0 {
                    config.requests_per_minute
                } else {
                    api_key.requests_per_minute
                },
                scopes: api_key.scopes.clone(),
            });
        }
    }
    None
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

#[cfg(test)]
fn proxy_request_is_authorized(config: &ProxyConfig, path: &str, headers: &HeaderMap) -> bool {
    authenticate_proxy_request(config, path, headers).is_some()
}

#[cfg(test)]
fn authorize_and_strip_proxy_credentials(
    config: &ProxyConfig,
    path: &str,
    headers: &mut HeaderMap,
) -> bool {
    if authenticate_proxy_request(config, path, headers).is_none() {
        return false;
    }
    headers.remove("authorization");
    headers.remove("x-api-key");
    true
}

fn cors_origin(config: &ProxyConfig, headers: &HeaderMap) -> Result<Option<String>, String> {
    let Some(origin) = headers.get("origin").and_then(|value| value.to_str().ok()) else {
        return Ok(None);
    };
    let normalized = origin.trim().trim_end_matches('/');
    if config
        .cors_allowed_origins
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(normalized))
    {
        Ok(Some(normalized.to_string()))
    } else {
        Err("origin is not allowed by the router CORS policy".into())
    }
}

fn apply_cors_headers(response: &mut Response, origin: Option<&str>) {
    let Some(origin) = origin else {
        return;
    };
    if let Ok(value) = HeaderValue::from_str(origin) {
        response
            .headers_mut()
            .insert("access-control-allow-origin", value);
    }
    response
        .headers_mut()
        .append("vary", HeaderValue::from_static("Origin"));
    response.headers_mut().insert(
        "access-control-allow-methods",
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    response.headers_mut().insert(
        "access-control-allow-headers",
        HeaderValue::from_static(
            "authorization, content-type, x-api-key, anthropic-version, anthropic-beta, x-request-id, openai-organization, openai-project",
        ),
    );
    response.headers_mut().insert(
        "access-control-expose-headers",
        HeaderValue::from_static(
            "request-id, x-request-id, retry-after, x-ratelimit-limit-requests, x-ratelimit-remaining-requests, anthropic-ratelimit-requests-limit, anthropic-ratelimit-requests-remaining",
        ),
    );
    response
        .headers_mut()
        .insert("access-control-max-age", HeaderValue::from_static("600"));
}

fn apply_rate_headers(response: &mut Response, format: ProxyApiFormat, limit: u32, remaining: u32) {
    if limit == 0 {
        return;
    }
    let (limit_name, remaining_name) = if format.is_anthropic() {
        (
            "anthropic-ratelimit-requests-limit",
            "anthropic-ratelimit-requests-remaining",
        )
    } else {
        (
            "x-ratelimit-limit-requests",
            "x-ratelimit-remaining-requests",
        )
    };
    if let Ok(value) = HeaderValue::from_str(&limit.to_string()) {
        response.headers_mut().insert(limit_name, value);
    }
    if let Ok(value) = HeaderValue::from_str(&remaining.to_string()) {
        response.headers_mut().insert(remaining_name, value);
    }
}

fn validate_anthropic_version(path: &str, headers: &HeaderMap) -> Result<(), String> {
    if !matches!(path, "/v1/messages" | "/v1/messages/count_tokens") {
        return Ok(());
    }
    let Some(version) = headers.get("anthropic-version") else {
        return Err(format!(
            "anthropic-version header is required; supported version is {SUPPORTED_ANTHROPIC_VERSION}"
        ));
    };
    let version = version
        .to_str()
        .map(str::trim)
        .map_err(|_| "anthropic-version header must be valid ASCII".to_string())?;
    if version != SUPPORTED_ANTHROPIC_VERSION {
        return Err(format!(
            "unsupported anthropic-version {version:?}; supported version is {SUPPORTED_ANTHROPIC_VERSION}"
        ));
    }
    Ok(())
}

fn request_body_limit(path: &str) -> usize {
    if matches!(path, "/v1/messages" | "/v1/messages/count_tokens") {
        MAX_ANTHROPIC_REQUEST_BODY_BYTES
    } else {
        MAX_PROXY_REQUEST_BODY_BYTES
    }
}

fn request_body_reservation(path: &str, headers: &HeaderMap) -> Result<usize, String> {
    let limit = request_body_limit(path);
    let Some(value) = headers.get("content-length") else {
        return Ok(limit);
    };
    let declared = value
        .to_str()
        .map_err(|_| "content-length header must be valid ASCII".to_string())?
        .trim()
        .parse::<usize>()
        .map_err(|_| "content-length header must be a non-negative integer".to_string())?;
    if declared > limit {
        return Err(format!("request body exceeds the {limit} byte limit"));
    }
    Ok(declared)
}

async fn proxy_security_middleware(
    State(router_state): State<ProxyRouterState>,
    mut request: Request,
    next: Next,
) -> Response {
    let config = router_state.source.proxy_config();
    let format = request_format(
        request.uri().path(),
        request.headers().contains_key("anthropic-version"),
    );
    let origin = match cors_origin(&config, request.headers()) {
        Ok(origin) => origin,
        Err(error) => return error_response(format, StatusCode::FORBIDDEN, &error),
    };
    if request.method() == Method::OPTIONS {
        let mut response = StatusCode::NO_CONTENT.into_response();
        apply_cors_headers(&mut response, origin.as_deref());
        ensure_request_id_header(&mut response, format);
        return response;
    }
    let Some(auth) = authenticate_proxy_request(&config, request.uri().path(), request.headers())
    else {
        let mut response = error_response(format, StatusCode::UNAUTHORIZED, "unauthorized");
        apply_cors_headers(&mut response, origin.as_deref());
        return response;
    };
    if !auth.scopes.is_empty()
        && !auth
            .scopes
            .iter()
            .any(|scope| scope == request_scope(request.uri().path()))
    {
        let mut response = error_response(format, StatusCode::FORBIDDEN, "API key scope denied");
        apply_cors_headers(&mut response, origin.as_deref());
        return response;
    }
    if let Err(error) = validate_anthropic_version(request.uri().path(), request.headers()) {
        let mut response = error_response(format, StatusCode::BAD_REQUEST, &error);
        apply_cors_headers(&mut response, origin.as_deref());
        return response;
    }
    let rate = router_state
        .runtime
        .check_rate_limit(&auth.client_id, auth.requests_per_minute);
    if !rate.allowed {
        let mut response =
            error_response(format, StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded");
        if let Ok(value) = HeaderValue::from_str(&rate.retry_after_secs.to_string()) {
            response.headers_mut().insert("retry-after", value);
        }
        apply_rate_headers(&mut response, format, rate.limit, rate.remaining);
        apply_cors_headers(&mut response, origin.as_deref());
        return response;
    }
    if request.method() == Method::POST && request_scope(request.uri().path()) == "inference" {
        let Some(permit) = router_state
            .runtime
            .acquire_global(
                config.max_concurrent_requests,
                Duration::from_millis(config.queue_timeout_ms),
            )
            .await
        else {
            let mut response = error_response(
                format,
                StatusCode::TOO_MANY_REQUESTS,
                "router concurrency limit exceeded",
            );
            if let Ok(value) = HeaderValue::from_str(
                &config
                    .queue_timeout_ms
                    .saturating_add(999)
                    .div_ceil(1_000)
                    .to_string(),
            ) {
                response.headers_mut().insert("retry-after", value);
            }
            ensure_request_id_header(&mut response, format);
            apply_rate_headers(&mut response, format, rate.limit, rate.remaining);
            apply_cors_headers(&mut response, origin.as_deref());
            return response;
        };
        let reservation = match request_body_reservation(request.uri().path(), request.headers()) {
            Ok(reservation) => reservation,
            Err(error) => {
                let mut response = error_response(format, StatusCode::PAYLOAD_TOO_LARGE, &error);
                ensure_request_id_header(&mut response, format);
                apply_rate_headers(&mut response, format, rate.limit, rate.remaining);
                apply_cors_headers(&mut response, origin.as_deref());
                return response;
            }
        };
        let Some(body_permit) = router_state
            .runtime
            .try_acquire_body_bytes(reservation, MAX_PROXY_IN_FLIGHT_BODY_BYTES)
        else {
            router_state.runtime.record_rejected();
            let mut response = error_response(
                format,
                StatusCode::TOO_MANY_REQUESTS,
                "router in-flight request body budget exceeded",
            );
            response
                .headers_mut()
                .insert("retry-after", HeaderValue::from_static("1"));
            ensure_request_id_header(&mut response, format);
            apply_rate_headers(&mut response, format, rate.limit, rate.remaining);
            apply_cors_headers(&mut response, origin.as_deref());
            return response;
        };
        request
            .extensions_mut()
            .insert(ProxyAdmissionPermit::new(permit, body_permit));
    }
    request.headers_mut().remove("authorization");
    request.headers_mut().remove("x-api-key");
    let mut response = next.run(request).await;
    ensure_request_id_header(&mut response, format);
    apply_rate_headers(&mut response, format, rate.limit, rate.remaining);
    apply_cors_headers(&mut response, origin.as_deref());
    response
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
    if !is_local_bind_host(bound_host) {
        return Err("代理运行期间检测到非本机监听地址；请停止代理并改为回环地址".to_string());
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

fn apply_target_request_headers(
    mut request: reqwest::RequestBuilder,
    headers: &HeaderMap,
    target: &ResolvedProxyTarget,
) -> reqwest::RequestBuilder {
    let connection_tokens = connection_header_tokens(headers);
    for (name, value) in headers.iter() {
        let lower = name.as_str().to_ascii_lowercase();
        if should_forward_request_header(&lower, &connection_tokens) {
            request = request.header(name.as_str(), value.as_bytes());
        }
    }
    if !target.api_key.trim().is_empty() {
        request = request.bearer_auth(target.api_key.trim());
    }
    request
}

fn append_bounded_response_chunk(
    body: &mut Vec<u8>,
    chunk: &[u8],
    limit: usize,
) -> Result<(), String> {
    if body.len().saturating_add(chunk.len()) > limit {
        return Err(format!("upstream JSON response exceeds {limit} bytes"));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

async fn collect_bounded_response_body(
    response: reqwest::Response,
    limit: usize,
) -> Result<Bytes, String> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(format!("upstream JSON response exceeds {limit} bytes"));
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| error.to_string())?;
        append_bounded_response_chunk(&mut body, &chunk, limit)?;
    }
    Ok(Bytes::from(body))
}

async fn fetch_token_count_value(
    client: &reqwest::Client,
    target: &ResolvedProxyTarget,
    headers: &HeaderMap,
    path: &str,
    body: Bytes,
    proxy_config: &ProxyConfig,
) -> Option<serde_json::Value> {
    let timeout_ms = proxy_config
        .health_check_timeout_ms
        .max(1_000)
        .min(proxy_config.timeout_ms.max(1_000));
    let request = client
        .post(target_url_for_path(target, path))
        .timeout(Duration::from_millis(timeout_ms))
        .header("accept", "application/json")
        .header("accept-encoding", "identity")
        .header("content-type", "application/json");
    let response = apply_target_request_headers(request, headers, target)
        .body(body)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = collect_bounded_response_body(response, MAX_TOKEN_COUNT_RESPONSE_BYTES)
        .await
        .ok()?;
    serde_json::from_slice(&body).ok()
}

async fn fetch_input_token_count(
    client: &reqwest::Client,
    target: &ResolvedProxyTarget,
    headers: &HeaderMap,
    upstream_body: &Bytes,
    counter: InputTokenCounter,
    proxy_config: &ProxyConfig,
) -> Option<u64> {
    match counter {
        InputTokenCounter::Native(path) => fetch_token_count_value(
            client,
            target,
            headers,
            path,
            upstream_body.clone(),
            proxy_config,
        )
        .await?
        .get("input_tokens")
        .and_then(serde_json::Value::as_u64),
        InputTokenCounter::Completion => {
            let bodies = completion_tokenize_bodies(upstream_body)?;
            let counts = futures_util::future::join_all(bodies.into_iter().map(|body| {
                fetch_token_count_value(client, target, headers, "/tokenize", body, proxy_config)
            }))
            .await;
            let mut maximum = None;
            for value in counts {
                let count = value?
                    .get("tokens")
                    .and_then(serde_json::Value::as_array)?
                    .len() as u64;
                maximum = Some(maximum.map_or(count, |current: u64| current.max(count)));
            }
            maximum
        }
    }
}

async fn context_limit_violation(
    client: &reqwest::Client,
    target: &ResolvedProxyTarget,
    headers: &HeaderMap,
    path: &str,
    upstream_body: &Bytes,
    proxy_config: &ProxyConfig,
    context_window: u64,
) -> Option<ContextLimitViolation> {
    if context_window == 0 {
        return None;
    }
    let spec = context_preflight_spec(path, upstream_body)?;
    if spec.requested_output_tokens > context_window {
        return Some(ContextLimitViolation {
            error_param: spec.error_param,
            input_tokens: None,
            requested_output_tokens: spec.requested_output_tokens,
            context_window,
        });
    }
    let input_tokens = fetch_input_token_count(
        client,
        target,
        headers,
        upstream_body,
        spec.counter,
        proxy_config,
    )
    .await?;
    (input_tokens.saturating_add(spec.requested_output_tokens) > context_window).then_some(
        ContextLimitViolation {
            error_param: spec.error_param,
            input_tokens: Some(input_tokens),
            requested_output_tokens: spec.requested_output_tokens,
            context_window,
        },
    )
}

fn target_url_for_path(target: &ResolvedProxyTarget, path: &str) -> String {
    let uri = path
        .parse::<Uri>()
        .unwrap_or_else(|_| Uri::from_static("/health"));
    target_url(target, &uri)
}

fn query_parameter(uri: &Uri, name: &str) -> Option<String> {
    let url = reqwest::Url::parse(&format!("http://router.local{}", uri)).ok()?;
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

fn routing_candidate(target: &ResolvedProxyTarget) -> RoutingCandidate {
    RoutingCandidate {
        instance_id: target.public.instance_id.clone(),
        priority: target.route_priority,
        weight: target.route_weight,
        max_concurrent_requests: target.route_max_concurrent_requests,
    }
}

fn routing_group_key(
    requested_model: Option<&str>,
    endpoint_workload: Option<ModelWorkload>,
) -> String {
    format!(
        "{}:{}",
        endpoint_workload
            .unwrap_or(ModelWorkload::Inference)
            .as_str(),
        requested_model.unwrap_or("__default__")
    )
}

fn select_resolved_target(
    router_state: &ProxyRouterState,
    requested_model: Option<&str>,
    endpoint_workload: Option<ModelWorkload>,
) -> Option<(ProxyConfig, ResolvedProxyTarget)> {
    let snapshot = router_state.source.proxy_snapshot();
    let candidates = resolve_proxy_candidates_from(
        &snapshot.config,
        &snapshot.instances,
        &snapshot.running,
        requested_model,
        endpoint_workload,
    );
    let scheduling = candidates.iter().map(routing_candidate).collect::<Vec<_>>();
    let selected = router_state.runtime.select_target(
        &scheduling,
        &snapshot.config.routing_strategy,
        &routing_group_key(requested_model, endpoint_workload),
    )?;
    let target = candidates
        .into_iter()
        .find(|target| target.public.instance_id == selected.instance_id)?;
    Some((snapshot.config, target))
}

async fn fetch_target_json(
    target: &ResolvedProxyTarget,
    path: &str,
    config: &ProxyConfig,
) -> Result<serde_json::Value, String> {
    let client = proxy_http_client(config.connect_timeout_ms);
    let mut request = client
        .get(target_url_for_path(target, path))
        .timeout(Duration::from_millis(
            config.health_check_timeout_ms.max(250),
        ))
        .header("accept-encoding", "identity");
    if !target.api_key.trim().is_empty() {
        request = request.bearer_auth(target.api_key.trim());
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("upstream returned {}", response.status().as_u16()));
    }
    response.json().await.map_err(|error| error.to_string())
}

fn capabilities_from_values(
    props: Option<&serde_json::Value>,
    slots: Option<&serde_json::Value>,
    previous: &TargetCapabilities,
) -> TargetCapabilities {
    let mut capabilities = previous.clone();
    if let Some(props) = props {
        capabilities.context_length = props
            .pointer("/default_generation_settings/n_ctx")
            .and_then(serde_json::Value::as_u64)
            .or(capabilities.context_length);
        capabilities.total_slots = props
            .get("total_slots")
            .and_then(serde_json::Value::as_u64)
            .or(capabilities.total_slots);
        capabilities.modalities = props
            .get("modalities")
            .cloned()
            .unwrap_or_else(|| capabilities.modalities.clone());
        capabilities.chat_template_caps = props
            .get("chat_template_caps")
            .cloned()
            .unwrap_or_else(|| capabilities.chat_template_caps.clone());
        capabilities.is_sleeping = props
            .get("is_sleeping")
            .and_then(serde_json::Value::as_bool)
            .or(capabilities.is_sleeping);
    }
    if let Some(items) = slots.and_then(serde_json::Value::as_array) {
        capabilities.total_slots = Some(items.len() as u64);
        capabilities.busy_slots = Some(
            items
                .iter()
                .filter(|slot| {
                    slot.get("is_processing")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                })
                .count() as u64,
        );
        capabilities.context_length = items
            .iter()
            .filter_map(|slot| slot.get("n_ctx").and_then(serde_json::Value::as_u64))
            .min()
            .or(capabilities.context_length);
    }
    capabilities.updated_at_ms = current_time_ms().max(0) as u64;
    capabilities
}

async fn probe_target(
    runtime: Arc<RouterRuntime>,
    target: ResolvedProxyTarget,
    config: ProxyConfig,
) {
    let started = std::time::Instant::now();
    let health = fetch_target_json(&target, "/health", &config).await;
    if let Err(error) = health {
        runtime.mark_probe_failure(
            &target.public.instance_id,
            error,
            config.unhealthy_threshold,
            Duration::from_millis(config.recovery_cooldown_ms),
        );
        return;
    }
    let previous = runtime
        .target_snapshot(&target.public.instance_id)
        .capabilities;
    let props = if runtime.capabilities_stale(&target.public.instance_id, TARGET_CAPABILITY_MAX_AGE)
    {
        fetch_target_json(&target, "/props", &config).await.ok()
    } else {
        None
    };
    let slots = fetch_target_json(&target, "/slots", &config).await.ok();
    let capabilities = capabilities_from_values(props.as_ref(), slots.as_ref(), &previous);
    runtime.mark_probe_success(
        &target.public.instance_id,
        started.elapsed().as_secs_f64() * 1_000.0,
        Some(capabilities),
    );
}

async fn probe_snapshot_targets(source: Arc<dyn ProxyDataSource>, runtime: Arc<RouterRuntime>) {
    let snapshot = source.proxy_snapshot();
    let targets = all_resolved_targets(&snapshot);
    let target_ids = targets
        .iter()
        .map(|target| target.public.instance_id.clone())
        .collect::<HashSet<_>>();
    runtime.retain_targets(&target_ids);
    futures_util::future::join_all(
        targets
            .into_iter()
            .map(|target| probe_target(runtime.clone(), target, snapshot.config.clone())),
    )
    .await;
}

fn spawn_health_probe_loop(source: Weak<dyn ProxyDataSource>, runtime: Weak<RouterRuntime>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        loop {
            let (Some(source), Some(runtime)) = (source.upgrade(), runtime.upgrade()) else {
                break;
            };
            let interval_ms = source.proxy_config().health_check_interval_ms.max(1_000);
            probe_snapshot_targets(source, runtime).await;
            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        }
    });
}

pub(crate) fn status_with_runtime(
    snapshot: &ProxyRuntimeSnapshot,
    runtime: &RouterRuntime,
) -> ProxyStatus {
    let mut status = proxy_status_from_snapshot(snapshot);
    let mut route_ids = snapshot
        .config
        .routes
        .iter()
        .filter(|route| route_is_configured(route))
        .map(|route| route.target_instance_id.clone())
        .collect::<Vec<_>>();
    let explicitly_routed = route_ids.iter().cloned().collect::<HashSet<_>>();
    route_ids.extend(
        snapshot
            .running
            .keys()
            .filter(|id| !explicitly_routed.contains(id.as_str()))
            .cloned(),
    );
    status.active_routes = route_ids.len();
    let (healthy, unhealthy) = runtime.route_health_counts(route_ids);
    status.healthy_routes = healthy;
    status.unhealthy_routes = unhealthy;
    status.in_flight_requests = runtime.in_flight_requests();
    status.total_requests = runtime.total_requests();
    status
}

async fn proxy_health(State(router_state): State<ProxyRouterState>) -> Json<ProxyStatus> {
    let snapshot = router_state.source.proxy_snapshot();
    Json(status_with_runtime(&snapshot, &router_state.runtime))
}

async fn proxy_live(State(router_state): State<ProxyRouterState>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "live",
        "service": "llama-server-manager routing proxy",
        "in_flight_requests": router_state.runtime.in_flight_requests(),
        "total_requests": router_state.runtime.total_requests(),
    }))
}

async fn proxy_ready(State(router_state): State<ProxyRouterState>) -> Response {
    probe_snapshot_targets(router_state.source.clone(), router_state.runtime.clone()).await;
    let snapshot = router_state.source.proxy_snapshot();
    let status = status_with_runtime(&snapshot, &router_state.runtime);
    let target_ids = snapshot.running.keys().cloned().collect::<Vec<_>>();
    let targets = router_state.runtime.snapshots(target_ids);
    let ready = status.healthy_routes > 0;
    let code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(json!({
            "status": if ready { "ready" } else { "unavailable" },
            "healthy_routes": status.healthy_routes,
            "unhealthy_routes": status.unhealthy_routes,
            "targets": targets,
        })),
    )
        .into_response()
}

async fn proxy_metrics(State(router_state): State<ProxyRouterState>) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/plain; version=0.0.4; charset=utf-8")
        .body(Body::from(router_state.runtime.prometheus_metrics()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn sanitized_props(
    model: &str,
    value: &serde_json::Value,
    health: &TargetHealthSnapshot,
) -> serde_json::Value {
    json!({
        "model": model,
        "default_generation_settings": {
            "n_ctx": value.pointer("/default_generation_settings/n_ctx").and_then(serde_json::Value::as_u64)
        },
        "total_slots": value.get("total_slots").and_then(serde_json::Value::as_u64),
        "chat_template_caps": value.get("chat_template_caps").cloned().unwrap_or(serde_json::Value::Null),
        "modalities": value.get("modalities").cloned().unwrap_or(serde_json::Value::Null),
        "is_sleeping": value.get("is_sleeping").and_then(serde_json::Value::as_bool),
        "router": {
            "status": health.status,
            "latency_ms": health.latency_ms,
            "active_requests": health.active_requests,
        }
    })
}

fn sanitized_slots(value: &serde_json::Value) -> serde_json::Value {
    serde_json::Value::Array(
        value
            .as_array()
            .into_iter()
            .flatten()
            .map(|slot| {
                let mut sanitized = serde_json::Map::new();
                for key in ["id", "n_ctx", "speculative", "is_processing", "n_past"] {
                    if let Some(value) = slot.get(key) {
                        sanitized.insert(key.to_string(), value.clone());
                    }
                }
                serde_json::Value::Object(sanitized)
            })
            .collect(),
    )
}

async fn proxy_props(State(router_state): State<ProxyRouterState>, uri: Uri) -> Response {
    let requested_model = query_parameter(&uri, "model");
    let Some((config, target)) =
        select_resolved_target(&router_state, requested_model.as_deref(), None)
    else {
        return error_response(
            ProxyApiFormat::OpenAi,
            StatusCode::NOT_FOUND,
            "no ready model matches the selector",
        );
    };
    match fetch_target_json(&target, "/props", &config).await {
        Ok(value) => {
            let public_model =
                public_response_model(&config, &target.public, requested_model.as_deref());
            Json(sanitized_props(
                &public_model,
                &value,
                &router_state
                    .runtime
                    .target_snapshot(&target.public.instance_id),
            ))
            .into_response()
        }
        Err(error) => error_response(
            ProxyApiFormat::OpenAi,
            StatusCode::BAD_GATEWAY,
            &format!("upstream props request failed: {error}"),
        ),
    }
}

async fn proxy_slots(State(router_state): State<ProxyRouterState>, uri: Uri) -> Response {
    let requested_model = query_parameter(&uri, "model");
    let Some((config, target)) =
        select_resolved_target(&router_state, requested_model.as_deref(), None)
    else {
        return error_response(
            ProxyApiFormat::OpenAi,
            StatusCode::NOT_FOUND,
            "no ready model matches the selector",
        );
    };
    let upstream_path = if query_parameter(&uri, "fail_on_no_slot").as_deref() == Some("1") {
        "/slots?fail_on_no_slot=1"
    } else {
        "/slots"
    };
    match fetch_target_json(&target, upstream_path, &config).await {
        Ok(value) => Json(sanitized_slots(&value)).into_response(),
        Err(error) => error_response(
            ProxyApiFormat::OpenAi,
            StatusCode::BAD_GATEWAY,
            &format!("upstream slots request failed: {error}"),
        ),
    }
}

async fn proxy_index(State(router_state): State<ProxyRouterState>) -> Json<serde_json::Value> {
    let snapshot = router_state.source.proxy_snapshot();
    let status = status_with_runtime(&snapshot, &router_state.runtime);
    Json(json!({
        "service": "llama-server-manager routing proxy",
        "status": if status.running { "running" } else { "stopped" },
        "bound_addr": status.bound_addr,
        "active_routes": status.active_routes,
        "endpoints": {
            "health": "/health",
            "liveness": "/live",
            "readiness": "/ready",
            "metrics": "/metrics",
            "props": "/props?model={public_model}",
            "slots": "/slots?model={public_model}",
            "models": "/v1/models",
            "chat_completions": "/v1/chat/completions",
            "chat_completions_input_tokens": "/v1/chat/completions/input_tokens",
            "completions": "/v1/completions",
            "responses": "/v1/responses",
            "responses_input_tokens": "/v1/responses/input_tokens",
            "embeddings": "/v1/embeddings",
            "anthropic_messages": "/v1/messages",
            "anthropic_count_tokens": "/v1/messages/count_tokens"
        },
        "api_formats": ["openai", "anthropic"],
        "context_safety": {
            "model_metadata_fields": ["context_length", "context_window", "max_model_len"],
            "failover_advertises_minimum": true,
            "generation_preflight": ["chat_completions", "completions", "responses", "anthropic_messages"]
        },
        "anthropic_compatibility": {
            "transport": "llama.cpp native Messages API",
            "minimum_supported_baseline": "b10199",
            "request_limit_bytes": MAX_ANTHROPIC_REQUEST_BODY_BYTES,
            "tools_require": "--jinja",
            "supported": ["messages", "streaming", "usage", "tool_use", "tool_result", "image_blocks", "count_tokens", "model_discovery"],
            "cloud_only_features": "passed through without manager-side emulation"
        },
        "message": "Use OpenAI or Anthropic-compatible clients against the /v1 endpoints."
    }))
}

fn model_health_snapshots(
    runtime: &RouterRuntime,
    candidates: &[ResolvedProxyTarget],
) -> Vec<TargetHealthSnapshot> {
    let mut seen = HashSet::new();
    candidates
        .iter()
        .filter(|target| seen.insert(target.public.instance_id.clone()))
        .map(|target| {
            let mut snapshot = runtime.target_snapshot(&target.public.instance_id);
            if snapshot.capabilities.context_length.is_none() {
                snapshot.capabilities.context_length = target.configured_context_length;
            }
            snapshot
        })
        .collect()
}

fn safe_context_window(health: &[TargetHealthSnapshot]) -> Option<u64> {
    if health.is_empty()
        || health.iter().any(|target| {
            target
                .capabilities
                .context_length
                .map_or(true, |context| context == 0)
        })
    {
        return None;
    }
    health
        .iter()
        .filter_map(|target| target.capabilities.context_length)
        .min()
}

fn aggregate_target_capability(
    health: &[TargetHealthSnapshot],
    value: impl Fn(&TargetCapabilities) -> Option<bool>,
) -> Option<bool> {
    if health.is_empty() {
        return None;
    }
    let mut unknown = false;
    for target in health {
        match value(&target.capabilities) {
            Some(false) => return Some(false),
            Some(true) => {}
            None => unknown = true,
        }
    }
    (!unknown).then_some(true)
}

fn aggregate_model_status(health: &[TargetHealthSnapshot]) -> &'static str {
    if health.is_empty() {
        return "unknown";
    }
    let ready = health.iter().filter(|target| target.ready).count();
    if ready == health.len() {
        "ready"
    } else if ready > 0 {
        "degraded"
    } else if health.iter().any(|target| target.status == "circuit_open") {
        "circuit_open"
    } else {
        "unavailable"
    }
}

fn proxy_model_descriptor(id: String, health: &[TargetHealthSnapshot]) -> serde_json::Value {
    let context_window = safe_context_window(health);
    let status = aggregate_model_status(health);
    let tools = aggregate_target_capability(health, |caps| {
        caps.chat_template_caps
            .get("supports_tools")
            .and_then(serde_json::Value::as_bool)
    });
    let vision = aggregate_target_capability(health, |caps| {
        caps.modalities
            .get("vision")
            .and_then(serde_json::Value::as_bool)
    });
    let video = aggregate_target_capability(health, |caps| {
        caps.modalities
            .get("video")
            .and_then(serde_json::Value::as_bool)
    });
    json!({
        "id": id,
        "object": "model",
        "owned_by": "llama-server-manager",
        "created": 0,
        "type": "model",
        "display_name": id,
        "created_at": "1970-01-01T00:00:00Z",
        "context_length": context_window,
        "context_window": context_window,
        "max_model_len": context_window,
        "status": status,
        "capabilities": {
            "chat_completions": true,
            "responses": true,
            "anthropic_messages": true,
            "tools": tools,
            "vision": vision,
            "video": video,
        }
    })
}

async fn proxy_models(State(router_state): State<ProxyRouterState>) -> Json<serde_json::Value> {
    let snapshot = router_state.source.proxy_snapshot();
    let config = snapshot.config;
    let targets = list_proxy_targets_from(&snapshot.instances, &snapshot.running);
    let ids = listed_proxy_model_ids(&config, &targets);
    let model_candidates = ids
        .iter()
        .map(|id| {
            (
                id.clone(),
                resolve_proxy_candidates_from(
                    &config,
                    &snapshot.instances,
                    &snapshot.running,
                    Some(id),
                    None,
                ),
            )
        })
        .collect::<Vec<_>>();
    let first_id = ids.first().cloned();
    let last_id = ids.last().cloned();
    Json(json!({
        "object": "list",
        "data": model_candidates.into_iter().map(|(id, candidates)| {
            let health = model_health_snapshots(&router_state.runtime, &candidates);
            proxy_model_descriptor(id, &health)
        }).collect::<Vec<_>>(),
        "first_id": first_id,
        "last_id": last_id,
        "has_more": false
    }))
}

async fn proxy_model(
    State(router_state): State<ProxyRouterState>,
    Path(model_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let snapshot = router_state.source.proxy_snapshot();
    let targets = list_proxy_targets_from(&snapshot.instances, &snapshot.running);
    let format = request_format("/v1/models", headers.contains_key("anthropic-version"));
    if listed_proxy_model_ids(&snapshot.config, &targets)
        .into_iter()
        .any(|id| id == model_id)
    {
        let candidates = resolve_proxy_candidates_from(
            &snapshot.config,
            &snapshot.instances,
            &snapshot.running,
            Some(&model_id),
            None,
        );
        let health = model_health_snapshots(&router_state.runtime, &candidates);
        let mut response = Json(proxy_model_descriptor(model_id, &health)).into_response();
        add_format_header(&mut response, format);
        response
    } else {
        error_response(format, StatusCode::NOT_FOUND, "model not found")
    }
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

async fn proxy_upstream(
    State(router_state): State<ProxyRouterState>,
    Extension(admission): Extension<ProxyAdmissionPermit>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let api_format = ProxyApiFormat::from_path(uri.path());
    let Some(admission_guards) = admission.take() else {
        return error_response(
            api_format,
            StatusCode::INTERNAL_SERVER_ERROR,
            "router admission permit is unavailable",
        );
    };
    let ProxyAdmissionGuards {
        global: global_permit,
        body: body_permit,
    } = admission_guards;
    if api_format.is_anthropic() && method != Method::POST {
        return error_response(
            api_format,
            StatusCode::METHOD_NOT_ALLOWED,
            "Anthropic Messages endpoints require POST",
        );
    }
    let body = match body {
        Ok(body) => body,
        Err(rejection) => {
            let status = rejection.into_response().status();
            let message = if status == StatusCode::PAYLOAD_TOO_LARGE {
                format!(
                    "request body exceeds the {} byte limit",
                    MAX_ANTHROPIC_REQUEST_BODY_BYTES
                )
            } else {
                "failed to read request body".to_string()
            };
            return error_response(api_format, status, &message);
        }
    };
    let requested_model = match requested_model_from_body(&body) {
        Ok(model) => model,
        Err(error) => return error_response(api_format, StatusCode::BAD_REQUEST, &error),
    };
    let request_streaming = request_uses_streaming(&body);
    let vector_metadata = vector_request_metadata(uri.path(), &body);
    let resolution = router_state.source.resolve_proxy_request(
        requested_model.as_deref(),
        vector_metadata.as_ref().map(|metadata| metadata.workload),
    );
    let proxy_config = resolution.config;
    let mut candidates = resolution.candidates;
    let routing_key = routing_group_key(
        requested_model.as_deref(),
        vector_metadata.as_ref().map(|metadata| metadata.workload),
    );
    if candidates.is_empty() {
        return error_response(
            api_format,
            if requested_model.is_some() {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            },
            "no public route matches the requested model",
        );
    }
    let (target, target_permit) = loop {
        let scheduling = candidates.iter().map(routing_candidate).collect::<Vec<_>>();
        let Some(selected) = router_state.runtime.select_target(
            &scheduling,
            &proxy_config.routing_strategy,
            &routing_key,
        ) else {
            return error_response(
                api_format,
                StatusCode::SERVICE_UNAVAILABLE,
                "all matching routes are unavailable or at capacity",
            );
        };
        let Some(index) = candidates
            .iter()
            .position(|target| target.public.instance_id == selected.instance_id)
        else {
            return error_response(
                api_format,
                StatusCode::BAD_GATEWAY,
                "router selected an unknown target",
            );
        };
        let target = candidates.remove(index);
        if let Some(permit) = router_state.runtime.acquire_target(&selected) {
            break (target, permit);
        }
        if candidates.is_empty() {
            return error_response(
                api_format,
                StatusCode::TOO_MANY_REQUESTS,
                "all matching targets are at capacity",
            );
        }
    };
    if !vector_endpoint_matches_target(
        vector_metadata.as_ref().map(|metadata| metadata.workload),
        target.workload,
    ) {
        return error_response(
            api_format,
            StatusCode::BAD_REQUEST,
            "selected target does not support the requested vector endpoint",
        );
    }
    let response_model =
        public_response_model(&proxy_config, &target.public, requested_model.as_deref());
    let upstream_body = rewrite_request_model(&body, &target.upstream_model_id);
    let started_at = std::time::Instant::now();
    let started_at_ms = current_time_ms();
    let proxy_task_id = next_proxy_task_id();
    let client = proxy_http_client(proxy_config.connect_timeout_ms);

    let context_window = router_state
        .runtime
        .target_snapshot(&target.public.instance_id)
        .capabilities
        .context_length
        .or(target.configured_context_length)
        .filter(|value| *value > 0);
    if let Some(context_window) = context_window {
        if let Some(violation) = context_limit_violation(
            &client,
            &target,
            &headers,
            uri.path(),
            &upstream_body,
            &proxy_config,
            context_window,
        )
        .await
        {
            router_state.runtime.record_rejected();
            let error_text = format!(
                "context window exceeded: input_tokens={:?}, requested_output_tokens={}, context_window={}",
                violation.input_tokens,
                violation.requested_output_tokens,
                violation.context_window
            );
            let _ = record_proxy_telemetry(
                target.telemetry_session_id.as_deref(),
                &ProxyTelemetryRecord {
                    task_id: proxy_task_id,
                    model: requested_model.clone(),
                    target_instance_id: target.public.instance_id.clone(),
                    http_status: Some(StatusCode::BAD_REQUEST.as_u16()),
                    started_at_ms,
                    duration_ms: started_at.elapsed().as_secs_f64() * 1000.0,
                    error_text: Some(error_text),
                    api_format,
                },
                vector_metadata.as_ref(),
            );
            return context_limit_error_response(
                api_format,
                violation.error_param,
                violation.input_tokens,
                violation.requested_output_tokens,
                violation.context_window,
            );
        }
    }

    let reqwest_method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
        Ok(method) => method,
        Err(err) => {
            return error_response(
                api_format,
                StatusCode::BAD_REQUEST,
                &format!("invalid method: {}", err),
            )
        }
    };

    let mut request = client
        .request(reqwest_method, target_url(&target, &uri))
        .header("accept-encoding", "identity");
    if !request_streaming {
        request = request.timeout(Duration::from_millis(proxy_config.timeout_ms.max(1_000)));
    }
    request = apply_target_request_headers(request, &headers, &target);

    let response = match request.body(upstream_body).send().await {
        Ok(response) => response,
        Err(err) => {
            router_state.runtime.mark_request_failure(
                &target.public.instance_id,
                err.to_string(),
                proxy_config.unhealthy_threshold,
                Duration::from_millis(proxy_config.recovery_cooldown_ms),
            );
            router_state
                .runtime
                .record_completed(started_at.elapsed().as_millis().min(u64::MAX as u128) as u64);
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
                    api_format,
                },
                vector_metadata.as_ref(),
            );
            return error_response(
                api_format,
                StatusCode::BAD_GATEWAY,
                &format!("upstream request failed: {}", err),
            );
        }
    };
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        router_state.runtime.mark_request_failure(
            &target.public.instance_id,
            format!("upstream returned {}", status.as_u16()),
            proxy_config.unhealthy_threshold,
            Duration::from_millis(proxy_config.recovery_cooldown_ms),
        );
    } else {
        router_state.runtime.mark_probe_success(
            &target.public.instance_id,
            started_at.elapsed().as_secs_f64() * 1_000.0,
            None,
        );
    }
    let mut builder = Response::builder().status(status);
    let response_content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let response_is_sse = response_content_type.contains("text/event-stream");
    let response_is_json = response_content_type.contains("json");
    let status_success = status.is_success();
    let response_connection_tokens = connection_header_tokens(response.headers());
    for (name, value) in response.headers().iter() {
        let lower = name.as_str().to_ascii_lowercase();
        if lower == "content-length"
            || (!status_success && lower == "content-type")
            || (api_format.is_anthropic() && !status_success && lower == "request-id")
            || is_hop_by_hop_header(&lower, &response_connection_tokens)
        {
            continue;
        }
        if let (Ok(header_name), Ok(header_value)) = (
            HeaderName::from_bytes(name.as_str().as_bytes()),
            HeaderValue::from_bytes(value.as_bytes()),
        ) {
            builder = builder.header(header_name, header_value);
        }
    }
    if !status_success {
        builder = builder.header("content-type", "application/json");
    }

    let http_status = status.as_u16();
    let mut telemetry_guard = ProxyTelemetryGuard {
        session_id: target.telemetry_session_id.clone(),
        task_id: proxy_task_id,
        model: requested_model.clone(),
        target_instance_id: target.public.instance_id.clone(),
        http_status,
        started_at,
        started_at_ms,
        vector_metadata,
        api_format,
        recorded: false,
        runtime: router_state.runtime.clone(),
        _global_permit: Some(global_permit),
        _body_permit: Some(body_permit),
        _target_permit: Some(target_permit),
    };
    if (response_is_json && !response_is_sse) || (api_format.is_anthropic() && !status_success) {
        let response_body =
            match collect_bounded_response_body(response, MAX_PROXY_JSON_RESPONSE_BYTES).await {
                Ok(bytes) => bytes,
                Err(error_text) => {
                    telemetry_guard.record_once(Some(error_text.clone()));
                    return error_response(
                        api_format,
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
        let response_body =
            rewrite_json_response(response_body, &response_model, api_format, status);
        if api_format.is_anthropic() && !status_success {
            if let Some(request_id) = response_request_id(&response_body) {
                builder = builder.header("request-id", request_id);
            }
        }
        return match builder.body(Body::from(response_body)) {
            Ok(mut response) => {
                add_format_header(&mut response, api_format);
                response
            }
            Err(err) => error_response(
                api_format,
                StatusCode::BAD_GATEWAY,
                &format!("proxy response error: {}", err),
            ),
        };
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
            (
                line_stream,
                false,
                telemetry_guard,
                response_model,
                api_format,
                Duration::from_millis(proxy_config.streaming_idle_timeout_ms),
            ),
            move |(
                mut line_stream,
                finalized,
                mut telemetry_guard,
                response_model,
                api_format,
                idle_timeout,
            )| async move {
                if finalized {
                    return None;
                }
                match tokio::time::timeout(idle_timeout, line_stream.as_mut().next()).await {
                    Ok(Some(Ok(line))) => {
                        let line = rewrite_sse_line(&line, &response_model, api_format);
                        Some((
                            Ok::<_, std::io::Error>(Bytes::from(format!("{line}\n"))),
                            (
                                line_stream,
                                false,
                                telemetry_guard,
                                response_model,
                                api_format,
                                idle_timeout,
                            ),
                        ))
                    }
                    Ok(Some(Err(err))) => {
                        let error_text = err.to_string();
                        telemetry_guard.record_once(Some(error_text.clone()));
                        Some((
                            Err(std::io::Error::other(error_text)),
                            (
                                line_stream,
                                true,
                                telemetry_guard,
                                response_model,
                                api_format,
                                idle_timeout,
                            ),
                        ))
                    }
                    Ok(None) => {
                        telemetry_guard.record_once(if status_success {
                            None
                        } else {
                            Some(format!("upstream returned {}", http_status))
                        });
                        None
                    }
                    Err(_) => {
                        let error_text = format!(
                            "upstream stream was idle for {} milliseconds",
                            idle_timeout.as_millis()
                        );
                        telemetry_guard.record_once(Some(error_text.clone()));
                        Some((
                            Err(std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                error_text,
                            )),
                            (
                                line_stream,
                                true,
                                telemetry_guard,
                                response_model,
                                api_format,
                                idle_timeout,
                            ),
                        ))
                    }
                }
            },
        );
        return match builder.body(Body::from_stream(stream)) {
            Ok(mut response) => {
                add_format_header(&mut response, api_format);
                response
            }
            Err(err) => error_response(
                api_format,
                StatusCode::BAD_GATEWAY,
                &format!("proxy response error: {}", err),
            ),
        };
    }

    let upstream_stream = Box::pin(response.bytes_stream());
    let idle_timeout = Duration::from_millis(proxy_config.streaming_idle_timeout_ms);
    let stream = futures_util::stream::unfold(
        (upstream_stream, false, telemetry_guard, idle_timeout),
        move |(mut upstream_stream, finalized, mut telemetry_guard, idle_timeout)| async move {
            if finalized {
                return None;
            }
            match tokio::time::timeout(idle_timeout, upstream_stream.as_mut().next()).await {
                Ok(Some(Ok(bytes))) => Some((
                    Ok(bytes),
                    (upstream_stream, false, telemetry_guard, idle_timeout),
                )),
                Ok(Some(Err(err))) => {
                    let error_text = err.to_string();
                    telemetry_guard.record_once(Some(error_text.clone()));
                    Some((
                        Err(std::io::Error::other(error_text)),
                        (upstream_stream, true, telemetry_guard, idle_timeout),
                    ))
                }
                Ok(None) => {
                    telemetry_guard.record_once(if status_success {
                        None
                    } else {
                        Some(format!("upstream returned {}", http_status))
                    });
                    None
                }
                Err(_) => {
                    let error_text = format!(
                        "upstream stream was idle for {} milliseconds",
                        idle_timeout.as_millis()
                    );
                    telemetry_guard.record_once(Some(error_text.clone()));
                    Some((
                        Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            error_text,
                        )),
                        (upstream_stream, true, telemetry_guard, idle_timeout),
                    ))
                }
            }
        },
    );
    match builder.body(Body::from_stream(stream)) {
        Ok(mut response) => {
            add_format_header(&mut response, api_format);
            response
        }
        Err(err) => error_response(
            api_format,
            StatusCode::BAD_GATEWAY,
            &format!("proxy response error: {}", err),
        ),
    }
}

fn proxy_router_from_source_with_runtime_and_limits(
    source: Arc<dyn ProxyDataSource>,
    runtime: Arc<RouterRuntime>,
    request_body_limit: usize,
    anthropic_request_body_limit: usize,
) -> Router {
    let weak_source: Weak<dyn ProxyDataSource> = Arc::downgrade(&source);
    spawn_health_probe_loop(weak_source, Arc::downgrade(&runtime));
    let router_state = ProxyRouterState { source, runtime };
    let security_layer =
        middleware::from_fn_with_state(router_state.clone(), proxy_security_middleware);
    Router::new()
        .route("/", get(proxy_index))
        .route("/health", get(proxy_health))
        .route("/live", get(proxy_live))
        .route("/ready", get(proxy_ready))
        .route("/metrics", get(proxy_metrics))
        .route("/props", get(proxy_props))
        .route("/slots", get(proxy_slots))
        .route("/v1/models", get(proxy_models))
        .route("/v1/models/:model_id", get(proxy_model))
        .route("/v1/chat/completions", post(proxy_upstream))
        .route("/v1/chat/completions/input_tokens", post(proxy_upstream))
        .route("/v1/completions", post(proxy_upstream))
        .route("/v1/responses", post(proxy_upstream))
        .route("/v1/responses/input_tokens", post(proxy_upstream))
        .route(
            "/v1/messages",
            post(proxy_upstream).layer(DefaultBodyLimit::max(anthropic_request_body_limit)),
        )
        .route(
            "/v1/messages/count_tokens",
            post(proxy_upstream).layer(DefaultBodyLimit::max(anthropic_request_body_limit)),
        )
        .route("/embedding", post(proxy_upstream))
        .route("/embeddings", post(proxy_upstream))
        .route("/v1/embeddings", post(proxy_upstream))
        .route("/rerank", post(proxy_upstream))
        .route("/reranking", post(proxy_upstream))
        .route("/v1/rerank", post(proxy_upstream))
        .route("/v1/reranking", post(proxy_upstream))
        .route_layer(security_layer)
        .layer(DefaultBodyLimit::max(request_body_limit))
        .with_state(router_state)
}

#[cfg(test)]
fn proxy_router_from_source_with_limits(
    source: Arc<dyn ProxyDataSource>,
    request_body_limit: usize,
    anthropic_request_body_limit: usize,
) -> Router {
    proxy_router_from_source_with_runtime_and_limits(
        source,
        Arc::new(RouterRuntime::default()),
        request_body_limit,
        anthropic_request_body_limit,
    )
}

pub(crate) fn proxy_router_from_source_with_runtime(
    source: Arc<dyn ProxyDataSource>,
) -> (Router, Arc<RouterRuntime>) {
    let runtime = Arc::new(RouterRuntime::default());
    let router = proxy_router_from_source_with_runtime_and_limits(
        source,
        runtime.clone(),
        MAX_PROXY_REQUEST_BODY_BYTES,
        MAX_ANTHROPIC_REQUEST_BODY_BYTES,
    );
    (router, runtime)
}

#[cfg(test)]
pub(crate) fn proxy_router_from_source(source: Arc<dyn ProxyDataSource>) -> Router {
    proxy_router_from_source_with_limits(
        source,
        MAX_PROXY_REQUEST_BODY_BYTES,
        MAX_ANTHROPIC_REQUEST_BODY_BYTES,
    )
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
    let deployment_routing_changes = {
        let instances = state.instances.lock().unwrap();
        crate::deployment::routing_changed_instance_ids(
            &current,
            &config,
            instances.keys().cloned(),
        )
    };
    let lifecycle_conflict = {
        let starting = state.starting.lock().unwrap();
        deployment_routing_changes
            .iter()
            .find(|instance_id| starting.contains(instance_id.as_str()))
            .cloned()
    };
    if let Some(instance_id) = lifecycle_conflict {
        return Err(format!(
            "实例 {instance_id} 正在启动，部署路由状态暂时不能修改；请等待启动完成后重试"
        ));
    }
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
        crate::commands::config::update_proxy_config_and_persist(&state, &config)?;
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
    if !is_local_bind_host(&config.host) {
        let msg = "内置代理仅允许监听本机回环地址".to_string();
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
    let source: Arc<dyn ProxyDataSource> = Arc::new(TauriProxyDataSource { app: app.clone() });
    let (router, router_runtime) = proxy_router_from_source_with_runtime(source);
    *state.proxy_router_runtime.lock().unwrap() = Some(router_runtime);
    let server_task = tokio::spawn(async move {
        let result = axum::serve(listener, router)
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
            *state.proxy_router_runtime.lock().unwrap() = None;
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
    use crate::models::{
        InstanceConfig, ProxyApiKey, ProxyConfig, ProxyRoute, ProxyTarget, RunningInstance,
    };
    use crate::vector_policy::ModelWorkload;
    use axum::body::Body;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode, Uri};
    use axum::response::{IntoResponse, Response};
    use axum::routing::post;
    use axum::{Json, Router};
    use bytes::Bytes;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
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

    async fn mock_openai_upstream(uri: Uri, body: Bytes) -> Response {
        if uri.path() == "/health" {
            return Json(json!({ "status": "ok" })).into_response();
        }
        if uri.path() == "/props" {
            return Json(json!({
                "default_generation_settings": { "n_ctx": 131072 },
                "total_slots": 4,
                "chat_template_caps": { "supports_tools": true },
                "modalities": { "vision": true },
                "model_path": "C:\\private\\openai.gguf",
                "chat_template": "private template"
            }))
            .into_response();
        }
        if uri.path() == "/slots" {
            return Json(json!([
                { "id": 0, "n_ctx": 131072, "is_processing": false, "prompt": "private prompt" },
                { "id": 1, "n_ctx": 131072, "is_processing": false, "tokens": [1, 2, 3] }
            ]))
            .into_response();
        }
        let request: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        if uri.path() == "/v1/responses/input_tokens" {
            return Json(json!({
                "object": "response.input_tokens",
                "input_tokens": 5
            }))
            .into_response();
        }
        if uri.path() == "/v1/responses" {
            if request.get("stream").and_then(serde_json::Value::as_bool) == Some(true) {
                return Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from(concat!(
                        "event: response.created\n",
                        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_local\",\"object\":\"response\",\"created_at\":0,\"status\":\"in_progress\",\"model\":\"upstream-private\",\"output\":[]}}\n\n",
                        "event: response.output_text.delta\n",
                        "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_local\",\"output_index\":0,\"content_index\":0,\"delta\":\"Hello from Responses\"}\n\n",
                        "event: response.completed\n",
                        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_local\",\"object\":\"response\",\"created_at\":0,\"status\":\"completed\",\"model\":\"upstream-private\",\"output\":[]}}\n\n"
                    )))
                    .unwrap();
            }
            return Json(json!({
                "id": "resp_local",
                "object": "response",
                "created_at": 0,
                "status": "completed",
                "model": "upstream-private",
                "output_text": "Hello from Responses",
                "output": [{
                    "id": "msg_local",
                    "type": "message",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{
                        "type": "output_text",
                        "text": "Hello from Responses",
                        "annotations": []
                    }]
                }],
                "usage": { "input_tokens": 5, "output_tokens": 4, "total_tokens": 9 }
            }))
            .into_response();
        }
        if uri.path() == "/v1/chat/completions" {
            if request.get("stream").and_then(serde_json::Value::as_bool) == Some(true) {
                return Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from(concat!(
                        "data: {\"id\":\"chatcmpl_local\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"upstream-private\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":\"stop\"}]}\n\n",
                        "data: [DONE]\n\n"
                    )))
                    .unwrap();
            }
            return Json(json!({
                "id": "chatcmpl_local",
                "object": "chat.completion",
                "created": 0,
                "model": "upstream-private",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_local",
                            "type": "function",
                            "function": { "name": "get_weather", "arguments": "{\"city\":\"Shanghai\"}" }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": { "prompt_tokens": 5, "completion_tokens": 4, "total_tokens": 9 }
            }))
            .into_response();
        }
        super::error_response(
            super::ProxyApiFormat::OpenAi,
            StatusCode::NOT_FOUND,
            "unsupported mock endpoint",
        )
    }

    #[derive(Default)]
    struct ContextGuardUpstreamState {
        count_requests: AtomicUsize,
        generation_requests: AtomicUsize,
    }

    async fn mock_context_guard_upstream(
        State(state): State<Arc<ContextGuardUpstreamState>>,
        uri: Uri,
        body: Bytes,
    ) -> Response {
        match uri.path() {
            "/health" => return Json(json!({ "status": "ok" })).into_response(),
            "/props" => {
                return Json(json!({
                    "default_generation_settings": { "n_ctx": 10 },
                    "total_slots": 1,
                    "chat_template_caps": { "supports_tools": true },
                    "modalities": { "vision": false, "video": false }
                }))
                .into_response()
            }
            "/slots" => {
                return Json(json!([
                    { "id": 0, "n_ctx": 10, "is_processing": false }
                ]))
                .into_response()
            }
            "/v1/chat/completions/input_tokens"
            | "/v1/responses/input_tokens"
            | "/v1/messages/count_tokens" => {
                state.count_requests.fetch_add(1, Ordering::Relaxed);
                return Json(json!({
                    "object": "response.input_tokens",
                    "input_tokens": 8
                }))
                .into_response();
            }
            "/tokenize" => {
                state.count_requests.fetch_add(1, Ordering::Relaxed);
                return Json(json!({ "tokens": [1, 2, 3, 4, 5, 6, 7, 8] })).into_response();
            }
            _ => {}
        }
        state.generation_requests.fetch_add(1, Ordering::Relaxed);
        let request: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        if uri.path() == "/v1/messages" {
            return Json(json!({
                "id": "msg_allowed",
                "type": "message",
                "role": "assistant",
                "model": "upstream-private",
                "content": [],
                "stop_reason": "end_turn",
                "usage": { "input_tokens": 8, "output_tokens": 1 }
            }))
            .into_response();
        }
        Json(json!({
            "id": "chatcmpl_allowed",
            "object": "chat.completion",
            "model": request.get("model").cloned().unwrap_or_default(),
            "choices": []
        }))
        .into_response()
    }

    #[derive(Debug, Clone)]
    struct CapturedAnthropicRequest {
        path: String,
        body: serde_json::Value,
        anthropic_version: Option<String>,
        anthropic_beta: Option<String>,
        authorization: Option<String>,
        x_api_key: Option<String>,
    }

    async fn mock_anthropic_upstream(
        State(received): State<Arc<Mutex<Vec<CapturedAnthropicRequest>>>>,
        uri: Uri,
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> Response {
        let header = |name: &str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        };
        received.lock().unwrap().push(CapturedAnthropicRequest {
            path: uri.path().to_string(),
            body: body.clone(),
            anthropic_version: header("anthropic-version"),
            anthropic_beta: header("anthropic-beta"),
            authorization: header("authorization"),
            x_api_key: header("x-api-key"),
        });

        if body
            .get("metadata")
            .and_then(|metadata| metadata.get("user_id"))
            .and_then(serde_json::Value::as_str)
            == Some("upstream-error")
        {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": { "message": "Messages API unavailable" } })),
            )
                .into_response();
        }
        if uri.path() == "/v1/messages/count_tokens" {
            return Json(json!({ "input_tokens": 23 })).into_response();
        }
        if body.get("stream").and_then(serde_json::Value::as_bool) == Some(true) {
            return Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from(concat!(
                    "event: message_start\n",
                    "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_local\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"upstream-private\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":23,\"output_tokens\":0}}}\n\n",
                    "event: content_block_start\n",
                    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_local\",\"name\":\"get_weather\",\"input\":{}}}\n\n",
                    "event: content_block_delta\n",
                    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\\\"Shanghai\\\"}\"}}\n\n",
                    "event: content_block_stop\n",
                    "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
                    "event: message_delta\n",
                    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":7}}\n\n",
                    "event: message_stop\n",
                    "data: {\"type\":\"message_stop\"}\n\n"
                )))
                .unwrap();
        }
        Json(json!({
            "id": "msg_local",
            "type": "message",
            "role": "assistant",
            "model": "upstream-private",
            "content": [{
                "type": "tool_use",
                "id": "toolu_local",
                "name": "get_weather",
                "input": { "city": "Shanghai" }
            }],
            "stop_reason": "tool_use",
            "stop_sequence": null,
            "usage": { "input_tokens": 23, "output_tokens": 7 }
        }))
        .into_response()
    }

    fn anthropic_proxy_snapshot(
        upstream_address: std::net::SocketAddr,
    ) -> super::ProxyRuntimeSnapshot {
        let instance_id = "anthropic-instance".to_string();
        let instance = InstanceConfig {
            id: instance_id.clone(),
            name: "Private Anthropic target".into(),
            model_path: r"C:\private\anthropic.gguf".into(),
            alias: "upstream-private".into(),
            api_key: "upstream-secret".into(),
            host: upstream_address.ip().to_string(),
            port: upstream_address.port(),
            ..InstanceConfig::default()
        };
        super::ProxyRuntimeSnapshot {
            config: ProxyConfig {
                enabled: true,
                api_keys: vec![ProxyApiKey {
                    id: "anthropic-sdk-client".into(),
                    name: "Anthropic SDK client".into(),
                    key: "public-sdk-key".into(),
                    ..ProxyApiKey::default()
                }],
                default_instance_id: instance_id.clone(),
                routes: vec![ProxyRoute {
                    model_alias: "local-claude".into(),
                    target_instance_id: instance_id.clone(),
                    ..ProxyRoute::default()
                }],
                ..ProxyConfig::default()
            },
            instances: HashMap::from([(instance_id.clone(), instance.clone())]),
            running: HashMap::from([(
                instance_id.clone(),
                RunningInstance {
                    instance_id,
                    pid: std::process::id(),
                    port: upstream_address.port(),
                    host: upstream_address.ip().to_string(),
                    start_time: 0,
                    executable_path: String::new(),
                    telemetry_session_id: None,
                    workload: "inference".into(),
                    launch_config: Some(instance),
                    deployment_identity: Default::default(),
                    deployment_id: String::new(),
                    deployment_revision_id: String::new(),
                },
            )]),
            bound_addr: String::new(),
            last_error: None,
        }
    }

    fn openai_proxy_snapshot(
        upstream_address: std::net::SocketAddr,
        api_key: &str,
    ) -> super::ProxyRuntimeSnapshot {
        let instance_id = "openai-upstream".to_string();
        let instance = InstanceConfig {
            id: instance_id.clone(),
            name: "Private OpenAI backend".into(),
            model_path: r"C:\private\openai.gguf".into(),
            alias: "upstream-private".into(),
            host: upstream_address.ip().to_string(),
            port: upstream_address.port(),
            ..InstanceConfig::default()
        };
        let api_keys = if api_key.is_empty() {
            Vec::new()
        } else {
            vec![ProxyApiKey {
                id: "openai-sdk-client".into(),
                name: "OpenAI SDK client".into(),
                key: api_key.into(),
                ..ProxyApiKey::default()
            }]
        };
        super::ProxyRuntimeSnapshot {
            config: ProxyConfig {
                enabled: true,
                api_keys,
                routes: vec![ProxyRoute {
                    model_alias: "local-openai".into(),
                    target_instance_id: instance_id.clone(),
                    ..ProxyRoute::default()
                }],
                ..ProxyConfig::default()
            },
            instances: HashMap::from([(instance_id.clone(), instance.clone())]),
            running: HashMap::from([(
                instance_id.clone(),
                RunningInstance {
                    instance_id,
                    pid: std::process::id(),
                    port: upstream_address.port(),
                    host: upstream_address.ip().to_string(),
                    start_time: 0,
                    executable_path: String::new(),
                    telemetry_session_id: None,
                    workload: "inference".into(),
                    launch_config: Some(instance),
                    deployment_identity: Default::default(),
                    deployment_id: String::new(),
                    deployment_revision_id: String::new(),
                },
            )]),
            bound_addr: String::new(),
            last_error: None,
        }
    }

    fn target_health(
        instance_id: &str,
        context_length: Option<u64>,
        ready: bool,
        supports_tools: Option<bool>,
    ) -> super::TargetHealthSnapshot {
        super::TargetHealthSnapshot {
            instance_id: instance_id.into(),
            status: if ready { "ready" } else { "unavailable" }.into(),
            ready,
            consecutive_failures: 0,
            circuit_open_until_ms: 0,
            last_checked_at_ms: 0,
            last_success_at_ms: 0,
            latency_ms: None,
            active_requests: 0,
            last_error: None,
            capabilities: super::TargetCapabilities {
                context_length,
                chat_template_caps: supports_tools
                    .map(|value| json!({ "supports_tools": value }))
                    .unwrap_or_default(),
                modalities: json!({ "vision": false, "video": false }),
                ..super::TargetCapabilities::default()
            },
        }
    }

    #[test]
    fn model_metadata_uses_compatibility_aliases_and_safe_failover_capabilities() {
        let descriptor = super::proxy_model_descriptor(
            "public-model".into(),
            &[
                target_health("primary", Some(131_072), true, Some(true)),
                target_health("fallback", Some(65_536), false, Some(false)),
            ],
        );
        assert_eq!(descriptor["context_length"], 65_536);
        assert_eq!(descriptor["context_window"], 65_536);
        assert_eq!(descriptor["max_model_len"], 65_536);
        assert_eq!(descriptor["status"], "degraded");
        assert_eq!(descriptor["capabilities"]["tools"], false);

        let unknown = super::proxy_model_descriptor(
            "public-model".into(),
            &[
                target_health("primary", Some(131_072), true, Some(true)),
                target_health("fallback", None, true, Some(true)),
            ],
        );
        assert!(unknown["context_length"].is_null());
        assert!(unknown["context_window"].is_null());
        assert!(unknown["max_model_len"].is_null());
    }

    #[test]
    fn model_metadata_uses_only_explicit_context_before_runtime_probe() {
        let mut snapshot = openai_proxy_snapshot("127.0.0.1:18080".parse().unwrap(), "");
        for instance in snapshot.instances.values_mut() {
            instance.ctx_size = 131_072;
            instance.ctx_size_auto = false;
        }
        for running in snapshot.running.values_mut() {
            let config = running.launch_config.as_mut().unwrap();
            config.ctx_size = 131_072;
            config.ctx_size_auto = false;
        }
        let candidates = super::resolve_proxy_candidates_from(
            &snapshot.config,
            &snapshot.instances,
            &snapshot.running,
            Some("local-openai"),
            None,
        );
        let runtime = super::RouterRuntime::default();
        let configured = super::model_health_snapshots(&runtime, &candidates);
        assert_eq!(super::safe_context_window(&configured), Some(131_072));

        runtime.mark_probe_success(
            "openai-upstream",
            1.0,
            Some(super::TargetCapabilities {
                context_length: Some(65_536),
                ..super::TargetCapabilities::default()
            }),
        );
        let probed = super::model_health_snapshots(&runtime, &candidates);
        assert_eq!(super::safe_context_window(&probed), Some(65_536));

        for running in snapshot.running.values_mut() {
            running.launch_config.as_mut().unwrap().ctx_size_auto = true;
        }
        let automatic = super::resolve_proxy_candidates_from(
            &snapshot.config,
            &snapshot.instances,
            &snapshot.running,
            Some("local-openai"),
            None,
        );
        let unknown_runtime = super::RouterRuntime::default();
        let unknown = super::model_health_snapshots(&unknown_runtime, &automatic);
        assert_eq!(super::safe_context_window(&unknown), None);
    }

    #[test]
    fn capability_discovery_uses_the_smallest_slot_context() {
        let capabilities = super::capabilities_from_values(
            None,
            Some(&json!([
                { "id": 0, "n_ctx": 131_072, "is_processing": false },
                { "id": 1, "n_ctx": 65_536, "is_processing": false }
            ])),
            &super::TargetCapabilities::default(),
        );
        assert_eq!(capabilities.context_length, Some(65_536));
    }

    #[test]
    fn proxy_listener_defaults_to_loopback_and_rejects_cleartext_public_bindings() {
        let instances = HashMap::new();
        let local = ProxyConfig {
            host: "   ".into(),
            ..ProxyConfig::default()
        };
        let normalized = super::normalize_and_validate_proxy_config(local, &instances).unwrap();
        assert_eq!(normalized.host, "127.0.0.1");

        let public = ProxyConfig {
            host: "0.0.0.0".into(),
            ..ProxyConfig::default()
        };
        assert!(super::normalize_and_validate_proxy_config(public, &instances).is_err());
    }

    #[test]
    fn production_router_settings_are_normalized_and_security_boundaries_rejected() {
        let instances = HashMap::new();
        let normalized = super::normalize_and_validate_proxy_config(
            ProxyConfig {
                routing_strategy: "firstHealthy".into(),
                connect_timeout_ms: 1,
                max_concurrent_requests: 0,
                cors_allowed_origins: vec![
                    " https://app.example.com/ ".into(),
                    "https://APP.example.com".into(),
                ],
                api_keys: vec![ProxyApiKey {
                    id: " key-id ".into(),
                    name: " Browser client ".into(),
                    key: " 0123456789abcdef ".into(),
                    enabled: true,
                    scopes: vec!["Discovery".into(), "discovery".into()],
                    requests_per_minute: 50,
                }],
                ..ProxyConfig::default()
            },
            &instances,
        )
        .unwrap();
        assert_eq!(normalized.routing_strategy, "priorityFailover");
        assert_eq!(normalized.connect_timeout_ms, 100);
        assert_eq!(normalized.max_concurrent_requests, 1);
        assert_eq!(
            normalized.cors_allowed_origins,
            vec!["https://app.example.com"]
        );
        assert_eq!(normalized.api_keys[0].id, "key-id");
        assert_eq!(normalized.api_keys[0].name, "Browser client");
        assert_eq!(normalized.api_keys[0].scopes, vec!["discovery"]);
        assert!(normalized.api_keys[0]
            .key
            .starts_with(super::PROXY_API_KEY_HASH_PREFIX));
        assert!(!normalized.api_keys[0].key.contains("0123456789abcdef"));
        let mut hashed_headers = HeaderMap::new();
        hashed_headers.insert("authorization", "Bearer 0123456789abcdef".parse().unwrap());
        assert!(super::is_proxy_authorized(
            &normalized.api_keys[0].key,
            &hashed_headers
        ));
        hashed_headers.insert(
            "authorization",
            format!("Bearer {}", normalized.api_keys[0].key)
                .parse()
                .unwrap(),
        );
        assert!(!super::is_proxy_authorized(
            &normalized.api_keys[0].key,
            &hashed_headers
        ));

        for origin in [
            "*",
            "file:///tmp/client",
            "https://app.example.com/path",
            "https://user:password@app.example.com",
        ] {
            let rejected = ProxyConfig {
                cors_allowed_origins: vec![origin.into()],
                ..ProxyConfig::default()
            };
            assert!(super::normalize_and_validate_proxy_config(rejected, &instances).is_err());
        }
        let short_key = ProxyConfig {
            api_keys: vec![ProxyApiKey {
                key: "too-short".into(),
                ..ProxyApiKey::default()
            }],
            ..ProxyConfig::default()
        };
        assert!(super::normalize_and_validate_proxy_config(short_key, &instances).is_err());
        let unsupported_scope = ProxyConfig {
            api_keys: vec![ProxyApiKey {
                key: "0123456789abcdef".into(),
                scopes: vec!["admin".into()],
                ..ProxyApiKey::default()
            }],
            ..ProxyConfig::default()
        };
        assert!(super::normalize_and_validate_proxy_config(unsupported_scope, &instances).is_err());
    }

    #[test]
    fn legacy_single_key_is_migrated_once_into_a_scoped_hashed_key() {
        let normalized = super::normalize_and_validate_proxy_config(
            ProxyConfig {
                public_api_key: "legacy-secret".into(),
                ..ProxyConfig::default()
            },
            &HashMap::new(),
        )
        .unwrap();

        assert!(normalized.public_api_key.is_empty());
        assert_eq!(normalized.api_keys.len(), 1);
        assert_eq!(normalized.api_keys[0].id, "migrated-legacy-key");
        assert_eq!(
            normalized.api_keys[0].scopes,
            vec!["discovery", "inference"]
        );
        assert!(normalized.api_keys[0]
            .key
            .starts_with(super::PROXY_API_KEY_HASH_PREFIX));
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer legacy-secret".parse().unwrap());
        assert!(
            super::authenticate_proxy_request(&normalized, "/v1/chat/completions", &headers)
                .is_some()
        );
    }

    #[tokio::test]
    async fn runtime_status_reports_real_health_traffic_and_implicit_alias_routes() {
        let mut snapshot = openai_proxy_snapshot("127.0.0.1:18080".parse().unwrap(), "");
        let runtime = Arc::new(super::RouterRuntime::default());
        runtime.mark_probe_success("openai-upstream", 2.5, None);
        let permit = runtime
            .acquire_global(4, std::time::Duration::from_millis(10))
            .await
            .unwrap();

        let status = super::status_with_runtime(&snapshot, &runtime);
        assert_eq!(status.active_routes, 1);
        assert_eq!(status.healthy_routes, 1);
        assert_eq!(status.unhealthy_routes, 0);
        assert_eq!(status.in_flight_requests, 1);
        assert_eq!(status.total_requests, 1);

        let mut implicit_instance = snapshot.instances["openai-upstream"].clone();
        implicit_instance.id = "implicit-upstream".into();
        implicit_instance.alias = "implicit-public-model".into();
        let mut implicit_running = snapshot.running["openai-upstream"].clone();
        implicit_running.instance_id = "implicit-upstream".into();
        implicit_running.launch_config = Some(implicit_instance.clone());
        snapshot
            .instances
            .insert("implicit-upstream".into(), implicit_instance);
        snapshot
            .running
            .insert("implicit-upstream".into(), implicit_running);
        runtime.mark_probe_success("implicit-upstream", 1.5, None);
        let mixed_status = super::status_with_runtime(&snapshot, &runtime);
        assert_eq!(mixed_status.active_routes, 2);
        assert_eq!(mixed_status.healthy_routes, 2);
        assert_eq!(mixed_status.unhealthy_routes, 0);

        snapshot.config.routes.clear();
        let fallback_status = super::status_with_runtime(&snapshot, &runtime);
        assert_eq!(fallback_status.active_routes, 2);
        assert_eq!(fallback_status.healthy_routes, 2);
        drop(permit);
    }

    #[test]
    fn model_selector_and_buffered_json_response_are_bounded() {
        let accepted = serde_json::to_vec(&json!({ "model": "public-model" })).unwrap();
        assert_eq!(
            super::requested_model_from_body(&accepted)
                .unwrap()
                .as_deref(),
            Some("public-model")
        );
        let oversized = serde_json::to_vec(&json!({
            "model": "x".repeat(super::MAX_PROXY_MODEL_SELECTOR_BYTES + 1)
        }))
        .unwrap();
        assert!(super::requested_model_from_body(&oversized).is_err());

        let mut body = Vec::new();
        super::append_bounded_response_chunk(&mut body, b"1234", 5).unwrap();
        assert!(super::append_bounded_response_chunk(&mut body, b"67", 5).is_err());
        assert_eq!(body, b"1234");
    }

    #[tokio::test]
    async fn proxy_router_applies_an_explicit_request_body_limit() {
        let configured_limit = std::hint::black_box(super::MAX_PROXY_REQUEST_BODY_BYTES);
        assert!(configured_limit > 2 * 1024 * 1024);
        let snapshot = super::ProxyRuntimeSnapshot {
            config: ProxyConfig::default(),
            instances: HashMap::new(),
            running: HashMap::new(),
            bound_addr: String::new(),
            last_error: None,
        };
        let router = super::proxy_router_from_source_with_limits(
            Arc::new(TestProxySource { snapshot }),
            1_024,
            super::MAX_ANTHROPIC_REQUEST_BODY_BYTES,
        );
        let (address, task) = spawn_test_router(router).await;
        let client = reqwest::Client::new();

        let accepted = serde_json::to_vec(&json!({
            "model": "public-model",
            "input": "x".repeat(800),
        }))
        .unwrap();
        assert!(accepted.len() < 1_024);
        let accepted_status = client
            .post(format!("http://{address}/v1/embeddings"))
            .header("content-type", "application/json")
            .body(accepted)
            .send()
            .await
            .unwrap()
            .status();
        assert_ne!(accepted_status, StatusCode::PAYLOAD_TOO_LARGE);

        let rejected = serde_json::to_vec(&json!({
            "model": "public-model",
            "input": "x".repeat(1_024),
        }))
        .unwrap();
        assert!(rejected.len() > 1_024);
        let rejected_status = client
            .post(format!("http://{address}/v1/embeddings"))
            .header("content-type", "application/json")
            .body(rejected)
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(rejected_status, StatusCode::PAYLOAD_TOO_LARGE);

        task.abort();
    }

    #[tokio::test]
    async fn security_regression_global_permit_is_acquired_before_request_body_buffering() {
        use futures_util::StreamExt;

        let snapshot = super::ProxyRuntimeSnapshot {
            config: ProxyConfig {
                max_concurrent_requests: 1,
                queue_timeout_ms: 50,
                ..ProxyConfig::default()
            },
            instances: HashMap::new(),
            running: HashMap::new(),
            bound_addr: String::new(),
            last_error: None,
        };
        let runtime = Arc::new(super::RouterRuntime::default());
        let router = super::proxy_router_from_source_with_runtime_and_limits(
            Arc::new(TestProxySource { snapshot }),
            runtime.clone(),
            super::MAX_PROXY_REQUEST_BODY_BYTES,
            super::MAX_ANTHROPIC_REQUEST_BODY_BYTES,
        );
        let (address, server_task) = spawn_test_router(router).await;
        let body_stream = futures_util::stream::once(async {
            Ok::<Bytes, std::io::Error>(Bytes::from_static(b"{"))
        })
        .chain(futures_util::stream::pending::<Result<Bytes, std::io::Error>>());
        let request_task = tokio::spawn(async move {
            reqwest::Client::new()
                .post(format!("http://{address}/v1/embeddings"))
                .header("content-type", "application/json")
                .body(reqwest::Body::wrap_stream(body_stream))
                .send()
                .await
        });

        let mut observed_admission = false;
        for _ in 0..50 {
            if runtime.in_flight_requests() == 1 {
                observed_admission = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        request_task.abort();
        server_task.abort();
        for _ in 0..50 {
            if runtime.in_flight_requests() == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        assert!(
            observed_admission,
            "the concurrency budget must cover clients while their request bodies are still arriving"
        );
        assert_eq!(
            runtime.in_flight_requests(),
            0,
            "cancelling a partially uploaded request must release its permit"
        );
    }

    #[tokio::test]
    async fn anthropic_request_limit_returns_the_anthropic_413_shape() {
        let snapshot = super::ProxyRuntimeSnapshot {
            config: ProxyConfig::default(),
            instances: HashMap::new(),
            running: HashMap::new(),
            bound_addr: String::new(),
            last_error: None,
        };
        let router = super::proxy_router_from_source_with_limits(
            Arc::new(TestProxySource { snapshot }),
            super::MAX_PROXY_REQUEST_BODY_BYTES,
            64,
        );
        let (address, task) = spawn_test_router(router).await;

        let response = reqwest::Client::new()
            .post(format!("http://{address}/v1/messages"))
            .header("content-type", "application/json")
            .header("anthropic-version", super::SUPPORTED_ANTHROPIC_VERSION)
            .body("x".repeat(65))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(response.headers().contains_key("request-id"));
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["type"], "error");
        assert_eq!(body["error"]["type"], "request_too_large");

        task.abort();
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
            configured_context_length: None,
            telemetry_session_id: None,
            workload: ModelWorkload::Inference,
            route_priority: 0,
            route_weight: 1,
            route_max_concurrent_requests: 0,
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
        let rewritten = super::rewrite_json_response(
            response,
            "route-model",
            super::ProxyApiFormat::OpenAi,
            StatusCode::OK,
        );
        let rewritten_json: serde_json::Value = serde_json::from_slice(&rewritten).unwrap();
        assert_eq!(rewritten_json["model"], "route-model");
        assert!(!String::from_utf8_lossy(&rewritten).contains("private"));

        let sse = super::rewrite_sse_line(
            r#"data: {"id":"chatcmpl-test","model":"C:\\private\\model.gguf","choices":[]}"#,
            "route-model",
            super::ProxyApiFormat::OpenAi,
        );
        assert!(sse.contains(r#""model":"route-model""#));
        assert!(!sse.contains("private"));
        assert_eq!(
            super::rewrite_sse_line("data: [DONE]", "route-model", super::ProxyApiFormat::OpenAi,),
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
                deployment_identity: Default::default(),
                deployment_id: String::new(),
                deployment_revision_id: String::new(),
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
                    deployment_identity: Default::default(),
                    deployment_id: String::new(),
                    deployment_revision_id: String::new(),
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
                    deployment_identity: Default::default(),
                    deployment_id: String::new(),
                    deployment_revision_id: String::new(),
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
                deployment_identity: Default::default(),
                deployment_id: String::new(),
                deployment_revision_id: String::new(),
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
                    deployment_identity: Default::default(),
                    deployment_id: String::new(),
                    deployment_revision_id: String::new(),
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
                    deployment_identity: Default::default(),
                    deployment_id: String::new(),
                    deployment_revision_id: String::new(),
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
                deployment_identity: Default::default(),
                deployment_id: String::new(),
                deployment_revision_id: String::new(),
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

        assert!(super::resolve_proxy_target_from(
            &config,
            &instances,
            &running,
            Some("unknown-model"),
            None,
        )
        .is_none());

        let legacy_config = ProxyConfig {
            strict_model_routing: false,
            ..config
        };
        let fallback = super::resolve_proxy_target_from(
            &legacy_config,
            &instances,
            &running,
            Some("unknown-model"),
            None,
        )
        .expect("legacy permissive routing may still use the configured default instance");
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
                    deployment_identity: Default::default(),
                    deployment_id: String::new(),
                    deployment_revision_id: String::new(),
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

    #[tokio::test]
    async fn discovery_endpoints_expose_capabilities_without_private_backend_state() {
        let upstream_router = Router::new()
            .route("/health", axum::routing::any(mock_openai_upstream))
            .route("/props", axum::routing::any(mock_openai_upstream))
            .route("/slots", axum::routing::any(mock_openai_upstream));
        let (upstream_address, upstream_task) = spawn_test_router(upstream_router).await;
        let proxy_router = super::proxy_router_from_source(Arc::new(TestProxySource {
            snapshot: openai_proxy_snapshot(upstream_address, ""),
        }));
        let (proxy_address, proxy_task) = spawn_test_router(proxy_router).await;
        let client = reqwest::Client::new();

        let props: serde_json::Value = client
            .get(format!("http://{proxy_address}/props?model=local-openai"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(props["model"], "local-openai");
        assert_eq!(props["default_generation_settings"]["n_ctx"], 131072);
        assert_eq!(props["total_slots"], 4);
        assert!(props.get("model_path").is_none());
        assert!(props.get("chat_template").is_none());
        assert!(!props.to_string().contains("private template"));

        let slots: serde_json::Value = client
            .get(format!("http://{proxy_address}/slots?model=local-openai"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let first = slots.as_array().unwrap().first().unwrap();
        assert_eq!(first["n_ctx"], 131072);
        assert!(first.get("prompt").is_none());
        assert!(slots.as_array().unwrap()[1].get("tokens").is_none());

        proxy_task.abort();
        upstream_task.abort();
    }

    #[tokio::test]
    async fn context_preflight_rejects_openai_and_anthropic_overflow_before_generation() {
        let upstream_state = Arc::new(ContextGuardUpstreamState::default());
        let upstream_router = Router::new()
            .fallback(axum::routing::any(mock_context_guard_upstream))
            .with_state(upstream_state.clone());
        let (upstream_address, upstream_task) = spawn_test_router(upstream_router).await;
        let snapshot = openai_proxy_snapshot(upstream_address, "");
        let (proxy_router, runtime) =
            super::proxy_router_from_source_with_runtime(Arc::new(TestProxySource { snapshot }));
        runtime.mark_probe_success(
            "openai-upstream",
            1.0,
            Some(super::TargetCapabilities {
                context_length: Some(10),
                updated_at_ms: u64::MAX,
                ..super::TargetCapabilities::default()
            }),
        );
        let (proxy_address, proxy_task) = spawn_test_router(proxy_router).await;
        let client = reqwest::Client::new();

        for (path, request) in [
            (
                "/v1/chat/completions",
                json!({
                    "model": "local-openai",
                    "messages": [{ "role": "user", "content": "hello" }],
                    "max_tokens": 3
                }),
            ),
            (
                "/v1/completions",
                json!({
                    "model": "local-openai",
                    "prompt": "hello",
                    "max_tokens": 3
                }),
            ),
            (
                "/v1/responses",
                json!({
                    "model": "local-openai",
                    "input": "hello",
                    "max_output_tokens": 3
                }),
            ),
        ] {
            let response = client
                .post(format!("http://{proxy_address}{path}"))
                .json(&request)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(
                response
                    .headers()
                    .get("x-llama-server-manager-context-window")
                    .and_then(|value| value.to_str().ok()),
                Some("10")
            );
            let body: serde_json::Value = response.json().await.unwrap();
            assert_eq!(body["error"]["code"], "context_length_exceeded");
            assert_eq!(body["error"]["details"]["input_tokens"], 8);
        }

        let anthropic = client
            .post(format!("http://{proxy_address}/v1/messages"))
            .header("anthropic-version", "2023-06-01")
            .json(&json!({
                "model": "local-openai",
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 3
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(anthropic.status(), StatusCode::BAD_REQUEST);
        assert!(anthropic.headers().contains_key("request-id"));
        assert_eq!(
            anthropic
                .headers()
                .get("x-llama-server-manager-api-format")
                .and_then(|value| value.to_str().ok()),
            Some("anthropic")
        );
        let anthropic_body: serde_json::Value = anthropic.json().await.unwrap();
        assert_eq!(anthropic_body["type"], "error");
        assert_eq!(anthropic_body["error"]["type"], "invalid_request_error");
        assert_eq!(
            upstream_state.generation_requests.load(Ordering::Relaxed),
            0
        );
        assert_eq!(upstream_state.count_requests.load(Ordering::Relaxed), 4);

        let allowed = client
            .post(format!("http://{proxy_address}/v1/chat/completions"))
            .json(&json!({
                "model": "local-openai",
                "messages": [{ "role": "user", "content": "hello" }],
                "max_tokens": 2
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(allowed.status(), StatusCode::OK);
        assert_eq!(
            upstream_state.generation_requests.load(Ordering::Relaxed),
            1
        );

        proxy_task.abort();
        upstream_task.abort();
    }

    #[tokio::test]
    async fn scoped_keys_rate_limits_and_exact_cors_are_enforced() {
        let snapshot = super::ProxyRuntimeSnapshot {
            config: ProxyConfig {
                enabled: true,
                cors_allowed_origins: vec!["https://app.example.com".into()],
                api_keys: vec![
                    ProxyApiKey {
                        id: "discovery-client".into(),
                        name: "Discovery".into(),
                        key: "discovery-key-123456".into(),
                        enabled: true,
                        scopes: vec!["discovery".into()],
                        requests_per_minute: 1,
                    },
                    ProxyApiKey {
                        id: "inference-client".into(),
                        name: "Inference".into(),
                        key: "inference-key-123456".into(),
                        enabled: true,
                        scopes: vec!["inference".into()],
                        requests_per_minute: 0,
                    },
                ],
                ..ProxyConfig::default()
            },
            instances: HashMap::new(),
            running: HashMap::new(),
            bound_addr: String::new(),
            last_error: None,
        };
        let router = super::proxy_router_from_source(Arc::new(TestProxySource { snapshot }));
        let (address, task) = spawn_test_router(router).await;
        let client = reqwest::Client::new();

        let unauthorized = client
            .get(format!("http://{address}/live"))
            .send()
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert!(unauthorized.headers().contains_key("x-request-id"));
        let unauthorized_body: serde_json::Value = unauthorized.json().await.unwrap();
        assert_eq!(unauthorized_body["error"]["type"], "authentication_error");

        let preflight = client
            .request(
                reqwest::Method::OPTIONS,
                format!("http://{address}/v1/responses"),
            )
            .header("origin", "https://app.example.com")
            .send()
            .await
            .unwrap();
        assert_eq!(preflight.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            preflight
                .headers()
                .get("access-control-allow-origin")
                .and_then(|value| value.to_str().ok()),
            Some("https://app.example.com")
        );

        let denied_origin = client
            .get(format!("http://{address}/live"))
            .header("origin", "https://evil.example.com")
            .header("authorization", "Bearer discovery-key-123456")
            .send()
            .await
            .unwrap();
        assert_eq!(denied_origin.status(), StatusCode::FORBIDDEN);

        let scope_denied = client
            .get(format!("http://{address}/v1/models"))
            .header("authorization", "Bearer inference-key-123456")
            .send()
            .await
            .unwrap();
        assert_eq!(scope_denied.status(), StatusCode::FORBIDDEN);

        let accepted = client
            .get(format!("http://{address}/live"))
            .header("origin", "https://app.example.com")
            .header("x-api-key", "discovery-key-123456")
            .send()
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);
        assert_eq!(
            accepted
                .headers()
                .get("x-ratelimit-limit-requests")
                .and_then(|value| value.to_str().ok()),
            Some("1")
        );

        let rate_limited = client
            .get(format!("http://{address}/live"))
            .header("x-api-key", "discovery-key-123456")
            .send()
            .await
            .unwrap();
        assert_eq!(rate_limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(rate_limited.headers().contains_key("retry-after"));
        assert_eq!(
            rate_limited
                .headers()
                .get("x-ratelimit-remaining-requests")
                .and_then(|value| value.to_str().ok()),
            Some("0")
        );

        task.abort();
    }

    #[tokio::test]
    async fn official_openai_sdk_exercises_chat_responses_streaming_tokens_and_models() {
        let upstream_router = Router::new()
            .route("/health", axum::routing::any(mock_openai_upstream))
            .route("/props", axum::routing::any(mock_openai_upstream))
            .route("/slots", axum::routing::any(mock_openai_upstream))
            .route(
                "/v1/chat/completions",
                axum::routing::any(mock_openai_upstream),
            )
            .route("/v1/responses", axum::routing::any(mock_openai_upstream))
            .route(
                "/v1/responses/input_tokens",
                axum::routing::any(mock_openai_upstream),
            );
        let (upstream_address, upstream_task) = spawn_test_router(upstream_router).await;
        let snapshot = openai_proxy_snapshot(upstream_address, "public-sdk-key");
        let proxy_router = super::proxy_router_from_source(Arc::new(TestProxySource { snapshot }));
        let (proxy_address, proxy_task) = spawn_test_router(proxy_router).await;
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("test-openai-sdk-client.mjs");
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            tokio::process::Command::new("node")
                .arg(script)
                .arg(format!("http://{proxy_address}"))
                .arg("local-openai")
                .output(),
        )
        .await
        .expect("OpenAI SDK smoke test timed out")
        .expect("Node.js must be available for the official OpenAI SDK smoke test");
        assert!(
            output.status.success(),
            "OpenAI SDK smoke failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let sdk_result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(sdk_result["model"], "local-openai");
        assert_eq!(sdk_result["inputTokens"], 5);
        assert!(sdk_result["chatChunks"].as_u64().unwrap_or(0) >= 1);
        assert!(sdk_result["responseEvents"].as_u64().unwrap_or(0) >= 2);

        proxy_task.abort();
        upstream_task.abort();
    }

    #[tokio::test]
    async fn official_anthropic_sdk_exercises_messages_tools_images_streaming_and_models() {
        let captured = Arc::new(Mutex::new(Vec::<CapturedAnthropicRequest>::new()));
        let upstream_router = Router::new()
            .route("/v1/messages", post(mock_anthropic_upstream))
            .route("/v1/messages/count_tokens", post(mock_anthropic_upstream))
            .with_state(captured.clone());
        let (upstream_address, upstream_task) = spawn_test_router(upstream_router).await;
        let proxy_router = super::proxy_router_from_source(Arc::new(TestProxySource {
            snapshot: anthropic_proxy_snapshot(upstream_address),
        }));
        let (proxy_address, proxy_task) = spawn_test_router(proxy_router).await;

        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("test-anthropic-sdk-client.mjs");
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            tokio::process::Command::new("node")
                .arg(script)
                .arg(format!("http://{proxy_address}"))
                .arg("local-claude")
                .output(),
        )
        .await
        .expect("Anthropic SDK smoke test timed out")
        .expect("Node.js must be available for the official Anthropic SDK smoke test");
        assert!(
            output.status.success(),
            "Anthropic SDK smoke failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let sdk_result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(sdk_result["model"], "local-claude");
        assert_eq!(sdk_result["inputTokens"], 23);
        assert_eq!(sdk_result["streamEvents"], 6);

        let stream_response = reqwest::Client::new()
            .post(format!("http://{proxy_address}/v1/messages"))
            .header("x-api-key", "public-sdk-key")
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "prompt-caching-2024-07-31")
            .json(&json!({
                "model": "local-claude",
                "max_tokens": 8,
                "stream": true,
                "messages": [{ "role": "user", "content": "hello" }]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(stream_response.status(), StatusCode::OK);
        assert_eq!(
            stream_response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );
        let stream_body = stream_response.text().await.unwrap();
        assert!(stream_body.contains("event: message_start"));

        let captured = captured.lock().unwrap().clone();
        assert_eq!(captured.len(), 5);
        assert!(captured
            .iter()
            .all(|request| request.body["model"] == "upstream-private"));
        assert!(captured
            .iter()
            .all(|request| request.anthropic_version.as_deref() == Some("2023-06-01")));
        assert!(captured.iter().all(|request| {
            request.anthropic_beta.as_deref() == Some("prompt-caching-2024-07-31")
        }));
        assert!(captured
            .iter()
            .all(|request| request.authorization.as_deref() == Some("Bearer upstream-secret")));
        assert!(captured.iter().all(|request| request.x_api_key.is_none()));
        assert!(captured
            .iter()
            .any(|request| request.path == "/v1/messages/count_tokens"));
        assert!(captured.iter().any(|request| {
            request.body["messages"].as_array().is_some_and(|messages| {
                messages.iter().any(|message| {
                    message["content"].as_array().is_some_and(|content| {
                        content.iter().any(|block| block["type"] == "tool_result")
                    })
                })
            })
        }));
        assert!(captured.iter().any(|request| {
            request.body["messages"][0]["content"]
                .as_array()
                .is_some_and(|content| content.iter().any(|block| block["type"] == "image"))
        }));
        assert!(captured
            .iter()
            .any(|request| { request.body["system"][0]["cache_control"]["type"] == "ephemeral" }));
        assert!(captured
            .iter()
            .any(|request| request.body["thinking"]["budget_tokens"] == 1024));

        proxy_task.abort();
        upstream_task.abort();
    }

    #[tokio::test]
    async fn anthropic_auth_and_upstream_failures_use_anthropic_errors() {
        let captured = Arc::new(Mutex::new(Vec::<CapturedAnthropicRequest>::new()));
        let upstream_router = Router::new()
            .route("/v1/messages", post(mock_anthropic_upstream))
            .with_state(captured.clone());
        let (upstream_address, upstream_task) = spawn_test_router(upstream_router).await;
        let proxy_router = super::proxy_router_from_source(Arc::new(TestProxySource {
            snapshot: anthropic_proxy_snapshot(upstream_address),
        }));
        let (proxy_address, proxy_task) = spawn_test_router(proxy_router).await;
        let client = reqwest::Client::new();

        let unauthorized = client
            .post(format!("http://{proxy_address}/v1/messages"))
            .json(&json!({
                "model": "local-claude",
                "max_tokens": 8,
                "messages": [{ "role": "user", "content": "hello" }]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            unauthorized
                .headers()
                .get("x-llama-server-manager-api-format")
                .and_then(|value| value.to_str().ok()),
            Some("anthropic")
        );
        assert!(unauthorized.headers().contains_key("request-id"));
        let unauthorized_body: serde_json::Value = unauthorized.json().await.unwrap();
        assert_eq!(unauthorized_body["type"], "error");
        assert_eq!(unauthorized_body["error"]["type"], "authentication_error");

        for (path, version, expected_message) in [
            ("/v1/messages", None, "anthropic-version header is required"),
            (
                "/v1/messages/count_tokens",
                Some("2099-01-01"),
                "unsupported anthropic-version",
            ),
        ] {
            let mut request = client
                .post(format!("http://{proxy_address}{path}"))
                .header("x-api-key", "public-sdk-key");
            if let Some(version) = version {
                request = request.header("anthropic-version", version);
            }
            let invalid_version = request
                .json(&json!({
                    "model": "local-claude",
                    "max_tokens": 8,
                    "messages": [{ "role": "user", "content": "hello" }]
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(invalid_version.status(), StatusCode::BAD_REQUEST);
            assert!(invalid_version.headers().contains_key("request-id"));
            let invalid_body: serde_json::Value = invalid_version.json().await.unwrap();
            assert_eq!(invalid_body["type"], "error");
            assert_eq!(invalid_body["error"]["type"], "invalid_request_error");
            assert!(invalid_body["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains(expected_message)));
        }
        assert!(captured.lock().unwrap().is_empty());

        let upstream_error = client
            .post(format!("http://{proxy_address}/v1/messages"))
            .header("x-api-key", "public-sdk-key")
            .header("anthropic-version", "2023-06-01")
            .json(&json!({
                "model": "local-claude",
                "max_tokens": 8,
                "metadata": { "user_id": "upstream-error" },
                "messages": [{ "role": "user", "content": "hello" }]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(upstream_error.status(), StatusCode::NOT_FOUND);
        assert!(upstream_error.headers().contains_key("request-id"));
        assert_eq!(
            upstream_error
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
        let upstream_error_body: serde_json::Value = upstream_error.json().await.unwrap();
        assert_eq!(upstream_error_body["type"], "error");
        assert_eq!(upstream_error_body["error"]["type"], "not_found_error");
        assert_eq!(
            upstream_error_body["error"]["message"],
            "Messages API unavailable"
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
            api_keys: vec![ProxyApiKey {
                key: "secret".into(),
                ..ProxyApiKey::default()
            }],
            ..ProxyConfig::default()
        };
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer secret".parse().unwrap());
        headers.insert("x-api-key", "secret".parse().unwrap());
        headers.insert("content-type", "application/json".parse().unwrap());

        assert!(super::authorize_and_strip_proxy_credentials(
            &config,
            "/v1/chat/completions",
            &mut headers
        ));
        assert!(!headers.contains_key("authorization"));
        assert!(!headers.contains_key("x-api-key"));
        assert!(headers.contains_key("content-type"));
    }

    #[test]
    fn proxy_auth_rejects_near_matches_and_accepts_both_supported_headers() {
        for value in ["Bearer secre", "Bearer secret!", "secret!", ""] {
            let mut headers = HeaderMap::new();
            headers.insert("authorization", value.parse().unwrap());
            assert!(!super::is_proxy_authorized("secret", &headers));
        }
        let mut bearer = HeaderMap::new();
        bearer.insert("authorization", "Bearer secret".parse().unwrap());
        assert!(super::is_proxy_authorized("secret", &bearer));
        bearer.insert("authorization", "bearer   secret".parse().unwrap());
        assert!(super::is_proxy_authorized("secret", &bearer));
        let mut api_key = HeaderMap::new();
        api_key.insert("x-api-key", "secret".parse().unwrap());
        assert!(super::is_proxy_authorized("secret", &api_key));
    }

    #[test]
    fn cors_headers_preserve_existing_vary_dimensions() {
        let mut response = Response::builder()
            .header("vary", "Accept")
            .body(Body::empty())
            .unwrap();
        super::apply_cors_headers(&mut response, Some("https://app.example.com"));
        let vary = response
            .headers()
            .get_all("vary")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect::<Vec<_>>();
        assert_eq!(vary, vec!["Accept", "Origin"]);
    }

    #[test]
    fn running_proxy_cannot_rebind_silently() {
        let current = ProxyConfig {
            host: "0.0.0.0".into(),
            port: 11435,
            ..ProxyConfig::default()
        };
        assert!(super::validate_proxy_config_update(
            &current,
            &current,
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
        assert!(super::validate_proxy_config_update(&current, &current, false, None).is_ok());

        let local = ProxyConfig {
            host: "127.0.0.1".into(),
            ..ProxyConfig::default()
        };
        assert!(
            super::validate_proxy_config_update(&local, &local, true, Some("127.0.0.1:11435"),)
                .is_ok()
        );

        let stale_display = ProxyConfig {
            host: "127.0.0.1".into(),
            port: 11435,
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
            api_keys: vec![ProxyApiKey {
                key: "secret".into(),
                ..ProxyApiKey::default()
            }],
            ..ProxyConfig::default()
        };
        let headers = HeaderMap::new();

        for path in [
            "/",
            "/health",
            "/v1/models",
            "/v1/chat/completions",
            "/v1/messages",
            "/v1/messages/count_tokens",
        ] {
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
