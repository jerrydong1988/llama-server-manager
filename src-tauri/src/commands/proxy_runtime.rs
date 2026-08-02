use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;

const LATENCY_BUCKETS_MS: [u64; 9] = [10, 25, 50, 100, 250, 500, 1_000, 5_000, u64::MAX];
const MAX_SCHEDULING_COUNTERS: usize = 4_096;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct TargetCapabilities {
    pub context_length: Option<u64>,
    pub total_slots: Option<u64>,
    pub busy_slots: Option<u64>,
    pub modalities: Value,
    pub chat_template_caps: Value,
    pub is_sleeping: Option<bool>,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TargetHealthSnapshot {
    pub instance_id: String,
    pub status: String,
    pub ready: bool,
    pub consecutive_failures: u32,
    pub circuit_open_until_ms: u64,
    pub last_checked_at_ms: u64,
    pub last_success_at_ms: u64,
    pub latency_ms: Option<f64>,
    pub active_requests: usize,
    pub last_error: Option<String>,
    pub capabilities: TargetCapabilities,
}

#[derive(Debug, Clone)]
struct TargetRuntime {
    ready: bool,
    consecutive_failures: u32,
    circuit_open_until_ms: u64,
    last_checked_at_ms: u64,
    last_success_at_ms: u64,
    latency_ms: Option<f64>,
    active_requests: usize,
    last_error: Option<String>,
    capabilities: TargetCapabilities,
}

impl Default for TargetRuntime {
    fn default() -> Self {
        Self {
            // A running process is provisionally eligible until the first active probe.
            ready: true,
            consecutive_failures: 0,
            circuit_open_until_ms: 0,
            last_checked_at_ms: 0,
            last_success_at_ms: 0,
            latency_ms: None,
            active_requests: 0,
            last_error: None,
            capabilities: TargetCapabilities::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RoutingCandidate {
    pub instance_id: String,
    pub priority: i32,
    pub weight: u32,
    pub max_concurrent_requests: u32,
}

#[derive(Debug)]
struct RateBucket {
    tokens: f64,
    capacity: u32,
    last_refill: Instant,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RateLimitDecision {
    pub allowed: bool,
    pub limit: u32,
    pub remaining: u32,
    pub retry_after_secs: u64,
}

#[derive(Default)]
struct RouterMetrics {
    total_requests: AtomicU64,
    rejected_requests: AtomicU64,
    upstream_errors: AtomicU64,
    completed_requests: AtomicU64,
    duration_sum_ms: AtomicU64,
    latency_buckets: Mutex<[u64; LATENCY_BUCKETS_MS.len()]>,
}

struct DynamicConcurrencyLimiter {
    active: AtomicUsize,
    notify: Notify,
}

impl Default for DynamicConcurrencyLimiter {
    fn default() -> Self {
        Self {
            active: AtomicUsize::new(0),
            notify: Notify::new(),
        }
    }
}

pub(crate) struct RouterRuntime {
    started_at: Instant,
    targets: Mutex<HashMap<String, TargetRuntime>>,
    scheduling_counters: Mutex<HashMap<String, u64>>,
    limiter: DynamicConcurrencyLimiter,
    rate_buckets: Mutex<HashMap<String, RateBucket>>,
    metrics: RouterMetrics,
}

impl Default for RouterRuntime {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            targets: Mutex::new(HashMap::new()),
            scheduling_counters: Mutex::new(HashMap::new()),
            limiter: DynamicConcurrencyLimiter::default(),
            rate_buckets: Mutex::new(HashMap::new()),
            metrics: RouterMetrics::default(),
        }
    }
}

pub(crate) struct GlobalRequestPermit {
    runtime: Arc<RouterRuntime>,
}

impl Drop for GlobalRequestPermit {
    fn drop(&mut self) {
        self.runtime.limiter.active.fetch_sub(1, Ordering::AcqRel);
        self.runtime.limiter.notify.notify_one();
    }
}

pub(crate) struct TargetRequestPermit {
    runtime: Arc<RouterRuntime>,
    instance_id: String,
}

impl Drop for TargetRequestPermit {
    fn drop(&mut self) {
        if let Ok(mut targets) = self.runtime.targets.lock() {
            if let Some(target) = targets.get_mut(&self.instance_id) {
                target.active_requests = target.active_requests.saturating_sub(1);
            }
        }
    }
}

impl RouterRuntime {
    fn next_scheduling_ticket(&self, strategy: &str, routing_key: &str) -> u64 {
        let key = format!("{strategy}\u{1f}{routing_key}");
        let mut counters = self.scheduling_counters.lock().unwrap();
        if !counters.contains_key(&key) && counters.len() >= MAX_SCHEDULING_COUNTERS {
            counters.clear();
        }
        let counter = counters.entry(key).or_default();
        let ticket = *counter;
        *counter = counter.wrapping_add(1);
        ticket
    }

    pub(crate) async fn acquire_global(
        self: &Arc<Self>,
        max_concurrent_requests: u32,
        queue_timeout: Duration,
    ) -> Option<GlobalRequestPermit> {
        let limit = max_concurrent_requests.max(1) as usize;
        let acquire = async {
            loop {
                let active = self.limiter.active.load(Ordering::Acquire);
                if active < limit
                    && self
                        .limiter
                        .active
                        .compare_exchange(active, active + 1, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                {
                    self.metrics.total_requests.fetch_add(1, Ordering::Relaxed);
                    return GlobalRequestPermit {
                        runtime: self.clone(),
                    };
                }
                self.limiter.notify.notified().await;
            }
        };
        match tokio::time::timeout(queue_timeout, acquire).await {
            Ok(permit) => Some(permit),
            Err(_) => {
                self.metrics
                    .rejected_requests
                    .fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    pub(crate) fn in_flight_requests(&self) -> usize {
        self.limiter.active.load(Ordering::Acquire)
    }

    pub(crate) fn total_requests(&self) -> u64 {
        self.metrics.total_requests.load(Ordering::Relaxed)
    }

    pub(crate) fn check_rate_limit(&self, client_id: &str, limit: u32) -> RateLimitDecision {
        if limit == 0 {
            return RateLimitDecision {
                allowed: true,
                limit: 0,
                remaining: u32::MAX,
                retry_after_secs: 0,
            };
        }
        let mut buckets = self.rate_buckets.lock().unwrap();
        let bucket = buckets
            .entry(client_id.to_string())
            .or_insert_with(|| RateBucket {
                tokens: limit as f64,
                capacity: limit,
                last_refill: Instant::now(),
            });
        if bucket.capacity != limit {
            bucket.capacity = limit;
            bucket.tokens = bucket.tokens.min(limit as f64);
        }
        let elapsed = bucket.last_refill.elapsed().as_secs_f64();
        let refill_per_second = limit as f64 / 60.0;
        bucket.tokens = (bucket.tokens + elapsed * refill_per_second).min(limit as f64);
        bucket.last_refill = Instant::now();
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            return RateLimitDecision {
                allowed: true,
                limit,
                remaining: bucket.tokens.floor() as u32,
                retry_after_secs: 0,
            };
        }
        self.metrics
            .rejected_requests
            .fetch_add(1, Ordering::Relaxed);
        RateLimitDecision {
            allowed: false,
            limit,
            remaining: 0,
            retry_after_secs: ((1.0 - bucket.tokens) / refill_per_second).ceil().max(1.0) as u64,
        }
    }

    pub(crate) fn select_target(
        &self,
        candidates: &[RoutingCandidate],
        strategy: &str,
        routing_key: &str,
    ) -> Option<RoutingCandidate> {
        let now = now_ms();
        let targets = self.targets.lock().unwrap();
        let mut eligible = candidates
            .iter()
            .filter(|candidate| {
                let state = targets.get(&candidate.instance_id);
                let circuit_open =
                    state.is_some_and(|state| state.circuit_open_until_ms > now || !state.ready);
                let active = state.map(|state| state.active_requests).unwrap_or(0);
                let at_capacity = candidate.max_concurrent_requests > 0
                    && active >= candidate.max_concurrent_requests as usize;
                !circuit_open && !at_capacity
            })
            .cloned()
            .collect::<Vec<_>>();
        if eligible.is_empty() {
            return None;
        }
        let best_priority = eligible.iter().map(|candidate| candidate.priority).min()?;
        eligible.retain(|candidate| candidate.priority == best_priority);
        match strategy {
            "roundRobin" => {
                let index =
                    self.next_scheduling_ticket(strategy, routing_key) as usize % eligible.len();
                eligible.get(index).cloned()
            }
            "leastBusy" => eligible.into_iter().min_by(|left, right| {
                let score = |candidate: &RoutingCandidate| {
                    let state = targets.get(&candidate.instance_id);
                    let active = state.map(|state| state.active_requests).unwrap_or(0) as f64;
                    let slot_pressure = state
                        .and_then(|state| {
                            Some((
                                state.capabilities.busy_slots? as f64,
                                state.capabilities.total_slots? as f64,
                            ))
                        })
                        .map(|(busy, total)| if total > 0.0 { busy / total } else { 0.0 })
                        .unwrap_or(0.0);
                    active + slot_pressure
                };
                score(left)
                    .partial_cmp(&score(right))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            "weighted" => {
                let total_weight = eligible
                    .iter()
                    .map(|candidate| candidate.weight.max(1) as u64)
                    .sum::<u64>();
                let mut ticket = self.next_scheduling_ticket(strategy, routing_key) % total_weight;
                for candidate in eligible {
                    let weight = candidate.weight.max(1) as u64;
                    if ticket < weight {
                        return Some(candidate);
                    }
                    ticket -= weight;
                }
                None
            }
            _ => eligible.into_iter().next(),
        }
    }

    pub(crate) fn acquire_target(
        self: &Arc<Self>,
        candidate: &RoutingCandidate,
    ) -> Option<TargetRequestPermit> {
        let mut targets = self.targets.lock().unwrap();
        let target = targets.entry(candidate.instance_id.clone()).or_default();
        if candidate.max_concurrent_requests > 0
            && target.active_requests >= candidate.max_concurrent_requests as usize
        {
            return None;
        }
        target.active_requests += 1;
        Some(TargetRequestPermit {
            runtime: self.clone(),
            instance_id: candidate.instance_id.clone(),
        })
    }

    pub(crate) fn mark_probe_success(
        &self,
        instance_id: &str,
        latency_ms: f64,
        capabilities: Option<TargetCapabilities>,
    ) {
        let timestamp = now_ms();
        let mut targets = self.targets.lock().unwrap();
        let target = targets.entry(instance_id.to_string()).or_default();
        target.ready = true;
        target.consecutive_failures = 0;
        target.circuit_open_until_ms = 0;
        target.last_checked_at_ms = timestamp;
        target.last_success_at_ms = timestamp;
        target.latency_ms = Some(match target.latency_ms {
            Some(previous) => previous * 0.8 + latency_ms * 0.2,
            None => latency_ms,
        });
        target.last_error = None;
        if let Some(capabilities) = capabilities {
            target.capabilities = capabilities;
        }
    }

    pub(crate) fn mark_probe_failure(
        &self,
        instance_id: &str,
        error: String,
        unhealthy_threshold: u32,
        recovery_cooldown: Duration,
    ) {
        let timestamp = now_ms();
        let mut targets = self.targets.lock().unwrap();
        let target = targets.entry(instance_id.to_string()).or_default();
        target.last_checked_at_ms = timestamp;
        target.consecutive_failures = target.consecutive_failures.saturating_add(1);
        target.last_error = Some(error);
        if target.consecutive_failures >= unhealthy_threshold.max(1) {
            target.ready = false;
            target.circuit_open_until_ms = timestamp
                .saturating_add(recovery_cooldown.as_millis().min(u64::MAX as u128) as u64);
        }
    }

    pub(crate) fn mark_request_failure(
        &self,
        instance_id: &str,
        error: String,
        unhealthy_threshold: u32,
        recovery_cooldown: Duration,
    ) {
        self.metrics.upstream_errors.fetch_add(1, Ordering::Relaxed);
        self.mark_probe_failure(instance_id, error, unhealthy_threshold, recovery_cooldown);
    }

    pub(crate) fn record_completed(&self, duration_ms: u64) {
        self.metrics
            .completed_requests
            .fetch_add(1, Ordering::Relaxed);
        self.metrics
            .duration_sum_ms
            .fetch_add(duration_ms, Ordering::Relaxed);
        let mut buckets = self.metrics.latency_buckets.lock().unwrap();
        if let Some(index) = LATENCY_BUCKETS_MS
            .iter()
            .position(|upper| duration_ms <= *upper)
        {
            buckets[index] = buckets[index].saturating_add(1);
        }
    }

    pub(crate) fn record_rejected(&self) {
        self.metrics
            .rejected_requests
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn capabilities_stale(&self, instance_id: &str, max_age: Duration) -> bool {
        let targets = self.targets.lock().unwrap();
        let updated_at = targets
            .get(instance_id)
            .map(|target| target.capabilities.updated_at_ms)
            .unwrap_or(0);
        updated_at == 0 || now_ms().saturating_sub(updated_at) > max_age.as_millis() as u64
    }

    pub(crate) fn target_snapshot(&self, instance_id: &str) -> TargetHealthSnapshot {
        let targets = self.targets.lock().unwrap();
        let target = targets.get(instance_id).cloned().unwrap_or_default();
        TargetHealthSnapshot {
            instance_id: instance_id.to_string(),
            status: if target.ready {
                "ready".to_string()
            } else if target.circuit_open_until_ms > now_ms() {
                "circuit_open".to_string()
            } else {
                "unavailable".to_string()
            },
            ready: target.ready && target.circuit_open_until_ms <= now_ms(),
            consecutive_failures: target.consecutive_failures,
            circuit_open_until_ms: target.circuit_open_until_ms,
            last_checked_at_ms: target.last_checked_at_ms,
            last_success_at_ms: target.last_success_at_ms,
            latency_ms: target.latency_ms,
            active_requests: target.active_requests,
            last_error: target.last_error,
            capabilities: target.capabilities,
        }
    }

    pub(crate) fn snapshots(
        &self,
        instance_ids: impl IntoIterator<Item = String>,
    ) -> Vec<TargetHealthSnapshot> {
        instance_ids
            .into_iter()
            .map(|instance_id| self.target_snapshot(&instance_id))
            .collect()
    }

    pub(crate) fn retain_targets(&self, instance_ids: &HashSet<String>) {
        self.targets
            .lock()
            .unwrap()
            .retain(|instance_id, _| instance_ids.contains(instance_id));
    }

    pub(crate) fn route_health_counts(
        &self,
        route_instance_ids: impl IntoIterator<Item = String>,
    ) -> (usize, usize) {
        let targets = self.targets.lock().unwrap();
        let now = now_ms();
        route_instance_ids
            .into_iter()
            .fold((0, 0), |(healthy, unhealthy), id| {
                let ready = targets
                    .get(&id)
                    .is_some_and(|target| target.ready && target.circuit_open_until_ms <= now);
                if ready {
                    (healthy + 1, unhealthy)
                } else {
                    (healthy, unhealthy + 1)
                }
            })
    }

    pub(crate) fn prometheus_metrics(&self) -> String {
        let total = self.metrics.total_requests.load(Ordering::Relaxed);
        let completed = self.metrics.completed_requests.load(Ordering::Relaxed);
        let rejected = self.metrics.rejected_requests.load(Ordering::Relaxed);
        let upstream_errors = self.metrics.upstream_errors.load(Ordering::Relaxed);
        let duration_sum = self.metrics.duration_sum_ms.load(Ordering::Relaxed);
        let buckets = *self.metrics.latency_buckets.lock().unwrap();
        let targets = self.targets.lock().unwrap();
        let mut output = String::new();
        output.push_str("# HELP lsm_router_requests_total Total accepted router requests.\n");
        output.push_str("# TYPE lsm_router_requests_total counter\n");
        output.push_str(&format!("lsm_router_requests_total {total}\n"));
        output.push_str("# TYPE lsm_router_requests_completed_total counter\n");
        output.push_str(&format!(
            "lsm_router_requests_completed_total {completed}\n"
        ));
        output.push_str("# TYPE lsm_router_requests_rejected_total counter\n");
        output.push_str(&format!("lsm_router_requests_rejected_total {rejected}\n"));
        output.push_str("# TYPE lsm_router_upstream_errors_total counter\n");
        output.push_str(&format!(
            "lsm_router_upstream_errors_total {upstream_errors}\n"
        ));
        output.push_str("# TYPE lsm_router_in_flight_requests gauge\n");
        output.push_str(&format!(
            "lsm_router_in_flight_requests {}\n",
            self.in_flight_requests()
        ));
        output.push_str("# TYPE lsm_router_request_duration_milliseconds histogram\n");
        let mut cumulative = 0u64;
        for (index, upper) in LATENCY_BUCKETS_MS.iter().enumerate() {
            cumulative = cumulative.saturating_add(buckets[index]);
            let label = if *upper == u64::MAX {
                "+Inf".to_string()
            } else {
                upper.to_string()
            };
            output.push_str(&format!(
                "lsm_router_request_duration_milliseconds_bucket{{le=\"{label}\"}} {cumulative}\n"
            ));
        }
        output.push_str(&format!(
            "lsm_router_request_duration_milliseconds_sum {duration_sum}\nlsm_router_request_duration_milliseconds_count {completed}\n"
        ));
        output.push_str("# TYPE lsm_router_target_ready gauge\n");
        output.push_str("# TYPE lsm_router_target_active_requests gauge\n");
        for (instance_id, target) in targets.iter() {
            let label = instance_id.replace('\\', "\\\\").replace('"', "\\\"");
            let ready = u8::from(target.ready && target.circuit_open_until_ms <= now_ms());
            output.push_str(&format!(
                "lsm_router_target_ready{{instance_id=\"{label}\"}} {ready}\n"
            ));
            output.push_str(&format!(
                "lsm_router_target_active_requests{{instance_id=\"{label}\"}} {}\n",
                target.active_requests
            ));
        }
        output.push_str("# TYPE lsm_router_uptime_seconds gauge\n");
        output.push_str(&format!(
            "lsm_router_uptime_seconds {}\n",
            self.started_at.elapsed().as_secs()
        ));
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, priority: i32, weight: u32) -> RoutingCandidate {
        RoutingCandidate {
            instance_id: id.into(),
            priority,
            weight,
            max_concurrent_requests: 0,
        }
    }

    #[test]
    fn priority_tiers_are_authoritative_for_every_strategy() {
        let runtime = RouterRuntime::default();
        let candidates = vec![candidate("primary", 0, 1), candidate("standby", 10, 100)];
        for strategy in ["priorityFailover", "roundRobin", "leastBusy", "weighted"] {
            assert_eq!(
                runtime
                    .select_target(&candidates, strategy, "model-a")
                    .unwrap()
                    .instance_id,
                "primary"
            );
        }
    }

    #[test]
    fn open_circuit_excludes_target_until_a_probe_recovers_it() {
        let runtime = RouterRuntime::default();
        runtime.mark_probe_failure("primary", "offline".into(), 1, Duration::from_secs(60));
        let candidates = vec![candidate("primary", 0, 1), candidate("standby", 10, 1)];
        assert_eq!(
            runtime
                .select_target(&candidates, "priorityFailover", "model-a")
                .unwrap()
                .instance_id,
            "standby"
        );
        runtime.mark_probe_success("primary", 1.0, None);
        assert_eq!(
            runtime
                .select_target(&candidates, "priorityFailover", "model-a")
                .unwrap()
                .instance_id,
            "primary"
        );
    }

    #[test]
    fn token_bucket_returns_retry_metadata() {
        let runtime = RouterRuntime::default();
        assert!(runtime.check_rate_limit("client", 1).allowed);
        let rejected = runtime.check_rate_limit("client", 1);
        assert!(!rejected.allowed);
        assert_eq!(rejected.remaining, 0);
        assert!(rejected.retry_after_secs >= 1);
    }

    #[test]
    fn weighted_round_robin_and_least_busy_scheduling_are_deterministic() {
        let weighted = RouterRuntime::default();
        let candidates = vec![candidate("one", 0, 1), candidate("three", 0, 3)];
        let mut counts = HashMap::new();
        for _ in 0..40 {
            let selected = weighted
                .select_target(&candidates, "weighted", "model-a")
                .unwrap();
            *counts.entry(selected.instance_id).or_insert(0usize) += 1;
        }
        assert_eq!(counts.get("one"), Some(&10));
        assert_eq!(counts.get("three"), Some(&30));

        let runtime = Arc::new(RouterRuntime::default());
        let first = runtime.acquire_target(&candidates[0]).unwrap();
        assert_eq!(
            runtime
                .select_target(&candidates, "leastBusy", "model-a")
                .unwrap()
                .instance_id,
            "three"
        );
        drop(first);
        let sequence = (0..4)
            .map(|_| {
                runtime
                    .select_target(&candidates, "roundRobin", "model-a")
                    .unwrap()
                    .instance_id
            })
            .collect::<Vec<_>>();
        assert_eq!(sequence, vec!["one", "three", "one", "three"]);
    }

    #[test]
    fn round_robin_state_is_isolated_between_routing_groups() {
        let runtime = RouterRuntime::default();
        let candidates = vec![candidate("one", 0, 1), candidate("two", 0, 1)];
        assert_eq!(
            runtime
                .select_target(&candidates, "roundRobin", "model-a")
                .unwrap()
                .instance_id,
            "one"
        );
        assert_eq!(
            runtime
                .select_target(&candidates, "roundRobin", "model-b")
                .unwrap()
                .instance_id,
            "one"
        );
        assert_eq!(
            runtime
                .select_target(&candidates, "roundRobin", "model-a")
                .unwrap()
                .instance_id,
            "two"
        );
    }

    #[tokio::test]
    async fn global_and_target_concurrency_limits_release_cleanly() {
        let runtime = Arc::new(RouterRuntime::default());
        let first = runtime
            .acquire_global(1, Duration::from_millis(20))
            .await
            .unwrap();
        assert_eq!(runtime.in_flight_requests(), 1);
        assert!(runtime
            .acquire_global(1, Duration::from_millis(10))
            .await
            .is_none());
        drop(first);
        let second = runtime
            .acquire_global(1, Duration::from_millis(20))
            .await
            .unwrap();
        assert_eq!(runtime.in_flight_requests(), 1);
        drop(second);
        assert_eq!(runtime.in_flight_requests(), 0);

        let limited = RoutingCandidate {
            max_concurrent_requests: 1,
            ..candidate("limited", 0, 1)
        };
        let target = runtime.acquire_target(&limited).unwrap();
        assert!(runtime.acquire_target(&limited).is_none());
        drop(target);
        assert!(runtime.acquire_target(&limited).is_some());
    }
}
