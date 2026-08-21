use crate::models::{ProxyOperationalAlert, ProxyOperationalSnapshot};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;

const LATENCY_BUCKETS_MS: [u64; 11] = [
    10,
    25,
    50,
    100,
    250,
    500,
    1_000,
    3_000,
    5_000,
    10_000,
    u64::MAX,
];
const MAX_SCHEDULING_COUNTERS: usize = 4_096;
const OPERATIONAL_WINDOW: Duration = Duration::from_secs(300);
const MAX_OPERATIONAL_EVENTS: usize = 4_096;
const MIN_ALERT_SAMPLES: usize = 5;

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
    queued_requests_total: AtomicU64,
    queue_timeouts_total: AtomicU64,
    queue_wait_sum_ms: AtomicU64,
    queue_wait_buckets: Mutex<[u64; LATENCY_BUCKETS_MS.len()]>,
    ttft_sum_ms: AtomicU64,
    ttft_samples: AtomicU64,
    ttft_buckets: Mutex<[u64; LATENCY_BUCKETS_MS.len()]>,
    prompt_tokens_observed: AtomicU64,
    cached_prompt_tokens: AtomicU64,
    recent: Mutex<VecDeque<OperationalEvent>>,
}

#[derive(Debug, Clone)]
struct OperationalEvent {
    completed_at: Instant,
    queue_wait_ms: Option<u64>,
    ttft_ms: Option<u64>,
    prompt_tokens: Option<u64>,
    cached_prompt_tokens: Option<u64>,
    failed: bool,
    queue_timeout: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RequestObservation {
    pub duration_ms: u64,
    pub queue_wait_ms: u64,
    pub ttft_ms: Option<u64>,
    pub prompt_tokens: Option<u64>,
    pub cached_prompt_tokens: Option<u64>,
    pub failed: bool,
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

pub struct RouterRuntime {
    started_at: Instant,
    draining_targets: Mutex<HashSet<String>>,
    targets: Mutex<HashMap<String, TargetRuntime>>,
    scheduling_counters: Mutex<HashMap<String, u64>>,
    limiter: DynamicConcurrencyLimiter,
    queue_depth: AtomicUsize,
    in_flight_body_bytes: AtomicUsize,
    rate_buckets: Mutex<HashMap<String, RateBucket>>,
    metrics: RouterMetrics,
}

impl Default for RouterRuntime {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            draining_targets: Mutex::new(HashSet::new()),
            targets: Mutex::new(HashMap::new()),
            scheduling_counters: Mutex::new(HashMap::new()),
            limiter: DynamicConcurrencyLimiter::default(),
            queue_depth: AtomicUsize::new(0),
            in_flight_body_bytes: AtomicUsize::new(0),
            rate_buckets: Mutex::new(HashMap::new()),
            metrics: RouterMetrics::default(),
        }
    }
}

pub(crate) struct GlobalRequestPermit {
    runtime: Arc<RouterRuntime>,
    queue_wait_ms: u64,
}

struct QueueDepthGuard {
    runtime: Arc<RouterRuntime>,
}

impl Drop for QueueDepthGuard {
    fn drop(&mut self) {
        self.runtime.queue_depth.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct InFlightBodyPermit {
    runtime: Arc<RouterRuntime>,
    bytes: usize,
}

impl Drop for InFlightBodyPermit {
    fn drop(&mut self) {
        self.runtime
            .in_flight_body_bytes
            .fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

impl Drop for GlobalRequestPermit {
    fn drop(&mut self) {
        self.runtime.limiter.active.fetch_sub(1, Ordering::AcqRel);
        self.runtime.limiter.notify.notify_one();
    }
}

impl GlobalRequestPermit {
    pub(crate) fn queue_wait_ms(&self) -> u64 {
        self.queue_wait_ms
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
    fn try_acquire_global_slot(&self, limit: usize) -> bool {
        loop {
            let active = self.limiter.active.load(Ordering::Acquire);
            if active >= limit {
                return false;
            }
            if self
                .limiter
                .active
                .compare_exchange_weak(active, active + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return true;
            }
        }
    }

    fn record_histogram(buckets: &Mutex<[u64; LATENCY_BUCKETS_MS.len()]>, value_ms: u64) {
        if let Some(index) = LATENCY_BUCKETS_MS
            .iter()
            .position(|upper| value_ms <= *upper)
        {
            let mut buckets = buckets.lock().unwrap();
            buckets[index] = buckets[index].saturating_add(1);
        }
    }

    fn push_operational_event(&self, event: OperationalEvent) {
        let cutoff = Instant::now()
            .checked_sub(OPERATIONAL_WINDOW)
            .unwrap_or_else(Instant::now);
        let mut recent = self.metrics.recent.lock().unwrap();
        while recent
            .front()
            .is_some_and(|existing| existing.completed_at < cutoff)
        {
            recent.pop_front();
        }
        recent.push_back(event);
        while recent.len() > MAX_OPERATIONAL_EVENTS {
            recent.pop_front();
        }
    }

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
        if self.try_acquire_global_slot(limit) {
            self.metrics.total_requests.fetch_add(1, Ordering::Relaxed);
            Self::record_histogram(&self.metrics.queue_wait_buckets, 0);
            return Some(GlobalRequestPermit {
                runtime: self.clone(),
                queue_wait_ms: 0,
            });
        }
        self.metrics
            .queued_requests_total
            .fetch_add(1, Ordering::Relaxed);
        self.queue_depth.fetch_add(1, Ordering::AcqRel);
        let queue_guard = QueueDepthGuard {
            runtime: self.clone(),
        };
        let queued_at = Instant::now();
        let acquire = async {
            loop {
                if self.try_acquire_global_slot(limit) {
                    self.metrics.total_requests.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                self.limiter.notify.notified().await;
            }
        };
        let result = match tokio::time::timeout(queue_timeout, acquire).await {
            Ok(()) => {
                let queue_wait_ms = queued_at.elapsed().as_millis().min(u64::MAX as u128) as u64;
                self.metrics
                    .queue_wait_sum_ms
                    .fetch_add(queue_wait_ms, Ordering::Relaxed);
                Self::record_histogram(&self.metrics.queue_wait_buckets, queue_wait_ms);
                Some(GlobalRequestPermit {
                    runtime: self.clone(),
                    queue_wait_ms,
                })
            }
            Err(_) => {
                let queue_wait_ms = queued_at.elapsed().as_millis().min(u64::MAX as u128) as u64;
                self.metrics
                    .rejected_requests
                    .fetch_add(1, Ordering::Relaxed);
                self.metrics
                    .queue_timeouts_total
                    .fetch_add(1, Ordering::Relaxed);
                self.push_operational_event(OperationalEvent {
                    completed_at: Instant::now(),
                    queue_wait_ms: Some(queue_wait_ms),
                    ttft_ms: None,
                    prompt_tokens: None,
                    cached_prompt_tokens: None,
                    failed: true,
                    queue_timeout: true,
                });
                None
            }
        };
        drop(queue_guard);
        result
    }

    pub(crate) fn queue_depth(&self) -> usize {
        self.queue_depth.load(Ordering::Acquire)
    }

    pub(crate) fn in_flight_requests(&self) -> usize {
        self.limiter.active.load(Ordering::Acquire)
    }

    pub(crate) fn try_acquire_body_bytes(
        self: &Arc<Self>,
        bytes: usize,
        limit: usize,
    ) -> Option<InFlightBodyPermit> {
        loop {
            let active = self.in_flight_body_bytes.load(Ordering::Acquire);
            let next = active.checked_add(bytes)?;
            if next > limit {
                return None;
            }
            if self
                .in_flight_body_bytes
                .compare_exchange_weak(active, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(InFlightBodyPermit {
                    runtime: self.clone(),
                    bytes,
                });
            }
        }
    }

    pub(crate) fn in_flight_body_bytes(&self) -> usize {
        self.in_flight_body_bytes.load(Ordering::Acquire)
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
        self.record_rejected();
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
        let draining_targets = self.draining_targets.lock().unwrap();
        let targets = self.targets.lock().unwrap();
        let mut eligible = candidates
            .iter()
            .filter(|candidate| {
                if draining_targets.contains(&candidate.instance_id) {
                    return false;
                }
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
        let draining_targets = self.draining_targets.lock().unwrap();
        if draining_targets.contains(&candidate.instance_id) {
            return None;
        }
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

    pub(crate) fn set_target_draining(&self, instance_id: &str, draining: bool) {
        let mut targets = self.draining_targets.lock().unwrap();
        if draining {
            targets.insert(instance_id.to_string());
        } else {
            targets.remove(instance_id);
        }
    }

    pub(crate) fn target_active_requests(&self, instance_id: &str) -> usize {
        self.targets
            .lock()
            .unwrap()
            .get(instance_id)
            .map(|target| target.active_requests)
            .unwrap_or(0)
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

    pub(crate) fn record_completed(&self, observation: RequestObservation) {
        self.metrics
            .completed_requests
            .fetch_add(1, Ordering::Relaxed);
        self.metrics
            .duration_sum_ms
            .fetch_add(observation.duration_ms, Ordering::Relaxed);
        Self::record_histogram(&self.metrics.latency_buckets, observation.duration_ms);
        if let Some(ttft_ms) = observation.ttft_ms {
            self.metrics.ttft_samples.fetch_add(1, Ordering::Relaxed);
            self.metrics
                .ttft_sum_ms
                .fetch_add(ttft_ms, Ordering::Relaxed);
            Self::record_histogram(&self.metrics.ttft_buckets, ttft_ms);
        }
        if let Some(prompt_tokens) = observation.prompt_tokens {
            self.metrics
                .prompt_tokens_observed
                .fetch_add(prompt_tokens, Ordering::Relaxed);
        }
        if let Some(cached_prompt_tokens) = observation.cached_prompt_tokens {
            self.metrics
                .cached_prompt_tokens
                .fetch_add(cached_prompt_tokens, Ordering::Relaxed);
        }
        self.push_operational_event(OperationalEvent {
            completed_at: Instant::now(),
            queue_wait_ms: Some(observation.queue_wait_ms),
            ttft_ms: observation.ttft_ms,
            prompt_tokens: observation.prompt_tokens,
            cached_prompt_tokens: observation.cached_prompt_tokens,
            failed: observation.failed,
            queue_timeout: false,
        });
    }

    pub(crate) fn record_rejected(&self) {
        self.metrics
            .rejected_requests
            .fetch_add(1, Ordering::Relaxed);
        self.push_operational_event(OperationalEvent {
            completed_at: Instant::now(),
            queue_wait_ms: None,
            ttft_ms: None,
            prompt_tokens: None,
            cached_prompt_tokens: None,
            failed: true,
            queue_timeout: false,
        });
    }

    fn percentile(values: &mut [u64], percentile: f64) -> Option<u64> {
        if values.is_empty() {
            return None;
        }
        values.sort_unstable();
        let index = ((values.len() - 1) as f64 * percentile).ceil() as usize;
        values.get(index).copied()
    }

    pub(crate) fn operational_snapshot(
        &self,
        max_concurrent_requests: u32,
    ) -> ProxyOperationalSnapshot {
        let cutoff = Instant::now()
            .checked_sub(OPERATIONAL_WINDOW)
            .unwrap_or_else(Instant::now);
        let mut recent = self.metrics.recent.lock().unwrap();
        while recent
            .front()
            .is_some_and(|event| event.completed_at < cutoff)
        {
            recent.pop_front();
        }
        let events = recent.iter().cloned().collect::<Vec<_>>();
        drop(recent);

        let request_count = events.len() as u64;
        let failed_request_count = events.iter().filter(|event| event.failed).count() as u64;
        let error_rate_percent = (request_count > 0)
            .then_some(failed_request_count as f64 / request_count as f64 * 100.0);
        let mut queue_waits = events
            .iter()
            .filter_map(|event| event.queue_wait_ms)
            .collect::<Vec<_>>();
        let queue_wait_sample_count = queue_waits.len();
        let mut ttft = events
            .iter()
            .filter_map(|event| event.ttft_ms)
            .collect::<Vec<_>>();
        let ttft_sample_count = ttft.len() as u64;
        let queue_wait_p95_ms = Self::percentile(&mut queue_waits, 0.95);
        let ttft_p50_ms = Self::percentile(&mut ttft.clone(), 0.50);
        let ttft_p95_ms = Self::percentile(&mut ttft, 0.95);
        let prompt_tokens_observed = events
            .iter()
            .filter_map(|event| event.prompt_tokens)
            .fold(0_u64, u64::saturating_add);
        let cached_prompt_tokens = events
            .iter()
            .filter_map(|event| event.cached_prompt_tokens)
            .fold(0_u64, u64::saturating_add);
        let cache_reuse_percent = (prompt_tokens_observed > 0).then_some(
            cached_prompt_tokens.min(prompt_tokens_observed) as f64 / prompt_tokens_observed as f64
                * 100.0,
        );
        let queue_timeouts = events.iter().filter(|event| event.queue_timeout).count();
        let in_flight_requests = self.in_flight_requests();
        let queue_depth = self.queue_depth();
        let effective_limit = max_concurrent_requests.max(1);
        let saturation_percent = in_flight_requests as f64 / effective_limit as f64 * 100.0;
        let mut alerts = Vec::new();
        let mut add_alert = |id: &str, severity: &str, observed: f64, threshold: f64| {
            alerts.push(ProxyOperationalAlert {
                id: id.to_string(),
                severity: severity.to_string(),
                observed,
                threshold,
            });
        };
        if events.len() >= MIN_ALERT_SAMPLES {
            if let Some(error_rate) = error_rate_percent {
                if error_rate >= 25.0 {
                    add_alert("error_rate", "critical", error_rate, 25.0);
                } else if error_rate >= 10.0 {
                    add_alert("error_rate", "warning", error_rate, 10.0);
                }
            }
        }
        if ttft_sample_count as usize >= MIN_ALERT_SAMPLES {
            if let Some(value) = ttft_p95_ms {
                if value >= 10_000 {
                    add_alert("ttft_p95", "critical", value as f64, 10_000.0);
                } else if value >= 3_000 {
                    add_alert("ttft_p95", "warning", value as f64, 3_000.0);
                }
            }
        }
        if queue_wait_sample_count >= MIN_ALERT_SAMPLES {
            if let Some(value) = queue_wait_p95_ms {
                if value >= 1_000 {
                    add_alert("queue_wait_p95", "critical", value as f64, 1_000.0);
                } else if value >= 250 {
                    add_alert("queue_wait_p95", "warning", value as f64, 250.0);
                }
            }
        }
        if queue_timeouts >= 3 {
            add_alert("queue_timeouts", "critical", queue_timeouts as f64, 3.0);
        } else if queue_timeouts > 0 {
            add_alert("queue_timeouts", "warning", queue_timeouts as f64, 1.0);
        }
        if saturation_percent >= 100.0 && queue_depth > 0 {
            add_alert("saturation", "critical", saturation_percent, 100.0);
        } else if saturation_percent >= 85.0 {
            add_alert("saturation", "warning", saturation_percent, 85.0);
        }

        ProxyOperationalSnapshot {
            window_seconds: OPERATIONAL_WINDOW.as_secs(),
            request_count,
            failed_request_count,
            error_rate_percent,
            queue_depth,
            queued_requests_total: self.metrics.queued_requests_total.load(Ordering::Relaxed),
            queue_timeouts_total: self.metrics.queue_timeouts_total.load(Ordering::Relaxed),
            queue_wait_p95_ms,
            ttft_sample_count,
            ttft_p50_ms,
            ttft_p95_ms,
            prompt_tokens_observed,
            cached_prompt_tokens,
            cache_reuse_percent,
            in_flight_requests,
            max_concurrent_requests: effective_limit,
            saturation_percent,
            alerts,
        }
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
        let draining = self.draining_targets.lock().unwrap().contains(instance_id);
        let targets = self.targets.lock().unwrap();
        let target = targets.get(instance_id).cloned().unwrap_or_default();
        TargetHealthSnapshot {
            instance_id: instance_id.to_string(),
            status: if draining {
                "draining".to_string()
            } else if target.ready {
                "ready".to_string()
            } else if target.circuit_open_until_ms > now_ms() {
                "circuit_open".to_string()
            } else {
                "unavailable".to_string()
            },
            ready: !draining && target.ready && target.circuit_open_until_ms <= now_ms(),
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
        let draining_targets = self.draining_targets.lock().unwrap();
        let targets = self.targets.lock().unwrap();
        let now = now_ms();
        route_instance_ids
            .into_iter()
            .fold((0, 0), |(healthy, unhealthy), id| {
                let ready = !draining_targets.contains(&id)
                    && targets
                        .get(&id)
                        .is_some_and(|target| target.ready && target.circuit_open_until_ms <= now);
                if ready {
                    (healthy + 1, unhealthy)
                } else {
                    (healthy, unhealthy + 1)
                }
            })
    }

    pub(crate) fn prometheus_metrics(&self, max_concurrent_requests: u32) -> String {
        let total = self.metrics.total_requests.load(Ordering::Relaxed);
        let completed = self.metrics.completed_requests.load(Ordering::Relaxed);
        let rejected = self.metrics.rejected_requests.load(Ordering::Relaxed);
        let upstream_errors = self.metrics.upstream_errors.load(Ordering::Relaxed);
        let duration_sum = self.metrics.duration_sum_ms.load(Ordering::Relaxed);
        let buckets = *self.metrics.latency_buckets.lock().unwrap();
        let queue_wait_sum = self.metrics.queue_wait_sum_ms.load(Ordering::Relaxed);
        let queue_wait_buckets = *self.metrics.queue_wait_buckets.lock().unwrap();
        let ttft_sum = self.metrics.ttft_sum_ms.load(Ordering::Relaxed);
        let ttft_samples = self.metrics.ttft_samples.load(Ordering::Relaxed);
        let ttft_buckets = *self.metrics.ttft_buckets.lock().unwrap();
        let operational = self.operational_snapshot(max_concurrent_requests);
        let draining_targets = self.draining_targets.lock().unwrap();
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
        output.push_str("# HELP lsm_router_queue_depth Requests currently waiting for the global concurrency limiter.\n");
        output.push_str("# TYPE lsm_router_queue_depth gauge\n");
        output.push_str(&format!("lsm_router_queue_depth {}\n", self.queue_depth()));
        output.push_str("# TYPE lsm_router_queued_requests_total counter\n");
        output.push_str(&format!(
            "lsm_router_queued_requests_total {}\n",
            self.metrics.queued_requests_total.load(Ordering::Relaxed)
        ));
        output.push_str("# TYPE lsm_router_queue_timeouts_total counter\n");
        output.push_str(&format!(
            "lsm_router_queue_timeouts_total {}\n",
            self.metrics.queue_timeouts_total.load(Ordering::Relaxed)
        ));
        output.push_str("# TYPE lsm_router_in_flight_request_body_bytes gauge\n");
        output.push_str(&format!(
            "lsm_router_in_flight_request_body_bytes {}\n",
            self.in_flight_body_bytes()
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
        output.push_str("# HELP lsm_router_queue_wait_milliseconds Time spent waiting for global router admission.\n");
        output.push_str("# TYPE lsm_router_queue_wait_milliseconds histogram\n");
        let mut cumulative = 0u64;
        for (index, upper) in LATENCY_BUCKETS_MS.iter().enumerate() {
            cumulative = cumulative.saturating_add(queue_wait_buckets[index]);
            let label = if *upper == u64::MAX {
                "+Inf".to_string()
            } else {
                upper.to_string()
            };
            output.push_str(&format!(
                "lsm_router_queue_wait_milliseconds_bucket{{le=\"{label}\"}} {cumulative}\n"
            ));
        }
        output.push_str(&format!(
            "lsm_router_queue_wait_milliseconds_sum {queue_wait_sum}\nlsm_router_queue_wait_milliseconds_count {total}\n"
        ));
        output.push_str("# HELP lsm_router_ttft_milliseconds Time from authenticated request admission to the first downstream response body chunk.\n");
        output.push_str("# TYPE lsm_router_ttft_milliseconds histogram\n");
        let mut cumulative = 0u64;
        for (index, upper) in LATENCY_BUCKETS_MS.iter().enumerate() {
            cumulative = cumulative.saturating_add(ttft_buckets[index]);
            let label = if *upper == u64::MAX {
                "+Inf".to_string()
            } else {
                upper.to_string()
            };
            output.push_str(&format!(
                "lsm_router_ttft_milliseconds_bucket{{le=\"{label}\"}} {cumulative}\n"
            ));
        }
        output.push_str(&format!(
            "lsm_router_ttft_milliseconds_sum {ttft_sum}\nlsm_router_ttft_milliseconds_count {ttft_samples}\n"
        ));
        output.push_str("# HELP lsm_router_prompt_tokens_observed_total Prompt tokens explicitly reported by upstream responses.\n");
        output.push_str("# TYPE lsm_router_prompt_tokens_observed_total counter\n");
        output.push_str(&format!(
            "lsm_router_prompt_tokens_observed_total {}\n",
            self.metrics.prompt_tokens_observed.load(Ordering::Relaxed)
        ));
        output.push_str("# HELP lsm_router_prompt_tokens_cached_total Prompt tokens explicitly reported as cache-reused by upstream responses.\n");
        output.push_str("# TYPE lsm_router_prompt_tokens_cached_total counter\n");
        output.push_str(&format!(
            "lsm_router_prompt_tokens_cached_total {}\n",
            self.metrics.cached_prompt_tokens.load(Ordering::Relaxed)
        ));
        output.push_str("# HELP lsm_router_saturation_ratio Current global in-flight requests divided by the configured limit.\n");
        output.push_str("# TYPE lsm_router_saturation_ratio gauge\n");
        output.push_str(&format!(
            "lsm_router_saturation_ratio {:.6}\n",
            operational.saturation_percent / 100.0
        ));
        output.push_str("# HELP lsm_router_operational_alert Active deterministic operational alerts by identifier and severity.\n");
        output.push_str("# TYPE lsm_router_operational_alert gauge\n");
        for alert in &operational.alerts {
            output.push_str(&format!(
                "lsm_router_operational_alert{{alert=\"{}\",severity=\"{}\"}} 1\n",
                alert.id, alert.severity
            ));
        }
        output.push_str("# TYPE lsm_router_target_ready gauge\n");
        output.push_str("# TYPE lsm_router_target_active_requests gauge\n");
        for (instance_id, target) in targets.iter() {
            let label = instance_id.replace('\\', "\\\\").replace('"', "\\\"");
            let ready = u8::from(
                !draining_targets.contains(instance_id)
                    && target.ready
                    && target.circuit_open_until_ms <= now_ms(),
            );
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
    fn draining_rejects_new_admissions_without_losing_existing_request_counts() {
        let runtime = Arc::new(RouterRuntime::default());
        let primary = candidate("primary", 0, 1);
        let standby = candidate("standby", 10, 1);
        let existing = runtime.acquire_target(&primary).unwrap();
        assert_eq!(runtime.target_active_requests("primary"), 1);

        runtime.set_target_draining("primary", true);
        assert_eq!(
            runtime
                .select_target(&[primary.clone(), standby], "priorityFailover", "model-a")
                .unwrap()
                .instance_id,
            "standby"
        );
        assert!(runtime.acquire_target(&primary).is_none());
        assert_eq!(runtime.target_snapshot("primary").status, "draining");
        assert_eq!(runtime.target_active_requests("primary"), 1);

        drop(existing);
        assert_eq!(runtime.target_active_requests("primary"), 0);
        runtime.set_target_draining("primary", false);
        assert!(runtime.acquire_target(&primary).is_some());
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

    #[test]
    fn in_flight_body_budget_is_aggregate_and_releases_cleanly() {
        let runtime = Arc::new(RouterRuntime::default());
        let first = runtime.try_acquire_body_bytes(60, 100).unwrap();
        assert_eq!(runtime.in_flight_body_bytes(), 60);
        assert!(runtime.try_acquire_body_bytes(41, 100).is_none());
        let second = runtime.try_acquire_body_bytes(40, 100).unwrap();
        assert_eq!(runtime.in_flight_body_bytes(), 100);
        drop(first);
        assert_eq!(runtime.in_flight_body_bytes(), 40);
        drop(second);
        assert_eq!(runtime.in_flight_body_bytes(), 0);
    }

    #[test]
    fn operational_snapshot_uses_bounded_signals_and_deterministic_alerts() {
        let runtime = RouterRuntime::default();
        for _ in 0..5 {
            runtime.record_completed(RequestObservation {
                duration_ms: 15_000,
                queue_wait_ms: 1_500,
                ttft_ms: Some(12_000),
                prompt_tokens: Some(100),
                cached_prompt_tokens: Some(40),
                failed: true,
            });
        }
        let snapshot = runtime.operational_snapshot(8);
        assert_eq!(snapshot.window_seconds, 300);
        assert_eq!(snapshot.request_count, 5);
        assert_eq!(snapshot.failed_request_count, 5);
        assert_eq!(snapshot.ttft_p95_ms, Some(12_000));
        assert_eq!(snapshot.queue_wait_p95_ms, Some(1_500));
        assert_eq!(snapshot.cache_reuse_percent, Some(40.0));
        assert!(snapshot
            .alerts
            .iter()
            .any(|alert| alert.id == "error_rate" && alert.severity == "critical"));
        assert!(snapshot
            .alerts
            .iter()
            .any(|alert| alert.id == "ttft_p95" && alert.severity == "critical"));
        assert!(snapshot
            .alerts
            .iter()
            .any(|alert| alert.id == "queue_wait_p95" && alert.severity == "critical"));

        let sparse_metrics = RouterRuntime::default();
        sparse_metrics.record_completed(RequestObservation {
            duration_ms: 15_000,
            queue_wait_ms: 1_500,
            ttft_ms: Some(12_000),
            prompt_tokens: None,
            cached_prompt_tokens: None,
            failed: true,
        });
        for _ in 0..4 {
            sparse_metrics.record_rejected();
        }
        let sparse_snapshot = sparse_metrics.operational_snapshot(8);
        assert!(sparse_snapshot
            .alerts
            .iter()
            .all(|alert| alert.id != "ttft_p95" && alert.id != "queue_wait_p95"));

        let metrics = runtime.prometheus_metrics(8);
        assert!(metrics.contains("lsm_router_ttft_milliseconds_bucket"));
        assert!(metrics.contains("lsm_router_queue_wait_milliseconds_bucket"));
        assert!(metrics.contains("lsm_router_prompt_tokens_cached_total 200"));
        assert!(metrics.contains("alert=\"ttft_p95\",severity=\"critical\""));
    }

    #[tokio::test]
    async fn queue_timeouts_release_depth_and_surface_an_alert() {
        let runtime = Arc::new(RouterRuntime::default());
        let first = runtime
            .acquire_global(1, Duration::from_millis(20))
            .await
            .unwrap();
        assert!(runtime
            .acquire_global(1, Duration::from_millis(1))
            .await
            .is_none());
        assert_eq!(runtime.queue_depth(), 0);
        let snapshot = runtime.operational_snapshot(1);
        assert_eq!(snapshot.queue_timeouts_total, 1);
        assert!(snapshot
            .alerts
            .iter()
            .any(|alert| alert.id == "queue_timeouts" && alert.severity == "warning"));
        drop(first);
    }
}
