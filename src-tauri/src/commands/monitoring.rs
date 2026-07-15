use crate::commands::telemetry::LlamaMetricSample;
use crate::models::{AppState, SystemMetrics};
use crate::vector_policy::ModelWorkload;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager};

const FRAME_INTERVAL_MS: i64 = 1_000;
const FRAME_RETENTION: usize = 3_600;
const MAX_VECTOR_EVENTS: usize = 10_000;
const TASK_FRESHNESS_MS: i64 = 2_500;
const METRIC_FRESHNESS_MS: i64 = 8_000;
const SYSTEM_FRESHNESS_MS: i64 = 15_000;
const VECTOR_WINDOW_MS: i64 = 3_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitoringFrame {
    pub instance_id: String,
    pub session_id: Option<String>,
    pub session_started_at: i64,
    pub ts: i64,
    pub workload: String,
    pub state: String,
    pub throughput: Option<f64>,
    pub throughput_unit: String,
    pub output_tokens_per_second: Option<f64>,
    pub input_tokens_per_second: Option<f64>,
    pub items_per_second: Option<f64>,
    pub active_requests: u64,
    pub queued_requests: u64,
    pub slot_capacity: Option<u64>,
    pub busy_slots: Option<u64>,
    pub average_latency_ms: Option<f64>,
    pub success_rate: Option<f64>,
    pub source: String,
    pub data_age_ms: u64,
    pub system: Option<SystemMetrics>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorMetricSource {
    Log,
    Proxy,
}

#[derive(Debug, Clone)]
struct Timed<T> {
    value: T,
    ts: i64,
}

#[derive(Debug, Clone, Copy)]
struct TaskSnapshot {
    active: u64,
    throughput: f64,
}

#[derive(Debug, Clone, Copy)]
struct SlotSnapshot {
    capacity: u64,
    busy: u64,
}

#[derive(Debug, Clone)]
struct VectorLiveEvent {
    source: VectorMetricSource,
    completed_at: i64,
    item_count: u64,
    input_tokens: Option<u64>,
    duration_ms: f64,
    succeeded: bool,
}

fn aggregate_completed_input_tps(events: &[&VectorLiveEvent]) -> Option<f64> {
    let mut input_tokens = 0_u64;
    let mut earliest_start = i64::MAX;
    let mut latest_end = i64::MIN;
    for event in events {
        let Some(tokens) = event.input_tokens.filter(|tokens| *tokens > 0) else {
            continue;
        };
        if !event.duration_ms.is_finite() || event.duration_ms <= 0.0 {
            continue;
        }
        let duration_ms = event.duration_ms.ceil().min(i64::MAX as f64) as i64;
        let started_at = event.completed_at.saturating_sub(duration_ms);
        input_tokens = input_tokens.saturating_add(tokens);
        earliest_start = earliest_start.min(started_at);
        latest_end = latest_end.max(event.completed_at);
    }
    let observed_ms = latest_end.saturating_sub(earliest_start);
    (input_tokens > 0 && observed_ms > 0)
        .then_some(input_tokens as f64 / (observed_ms as f64 / 1_000.0))
}

struct InstanceMonitoringState {
    instance_id: String,
    session_id: Option<String>,
    workload: ModelWorkload,
    registered_at: i64,
    frame_loop_pid: Option<u32>,
    system: Option<Timed<SystemMetrics>>,
    llama: Option<Timed<LlamaMetricSample>>,
    tasks: Option<Timed<TaskSnapshot>>,
    slots: Option<Timed<SlotSnapshot>>,
    vector_events: VecDeque<VectorLiveEvent>,
    timeline: VecDeque<MonitoringFrame>,
}

impl InstanceMonitoringState {
    fn new(instance_id: &str, session_id: Option<&str>, workload: ModelWorkload, now: i64) -> Self {
        Self {
            instance_id: instance_id.to_string(),
            session_id: session_id.map(ToString::to_string),
            workload,
            registered_at: now,
            frame_loop_pid: None,
            system: None,
            llama: None,
            tasks: None,
            slots: None,
            vector_events: VecDeque::new(),
            timeline: VecDeque::with_capacity(FRAME_RETENTION),
        }
    }

    fn reset_session(
        &mut self,
        session_id: Option<&str>,
        workload: ModelWorkload,
        now: i64,
        force: bool,
    ) {
        let next_session = session_id.map(ToString::to_string);
        if !force && self.session_id == next_session && self.workload == workload {
            return;
        }
        self.session_id = next_session;
        self.workload = workload;
        self.registered_at = now.max(self.registered_at.saturating_add(1));
        self.system = None;
        self.llama = None;
        self.tasks = None;
        self.slots = None;
        self.vector_events.clear();
        self.timeline.clear();
    }

    fn push_frame(&mut self, frame: MonitoringFrame) {
        if self.timeline.back().map(|item| item.ts) == Some(frame.ts) {
            self.timeline.pop_back();
        }
        self.timeline.push_back(frame);
        while self.timeline.len() > FRAME_RETENTION {
            self.timeline.pop_front();
        }
    }

    fn trim_vector_events(&mut self, now: i64) {
        let oldest = now.saturating_sub(60_000);
        while self
            .vector_events
            .front()
            .is_some_and(|event| event.completed_at < oldest)
        {
            self.vector_events.pop_front();
        }
    }

    fn build_frame(&mut self, now: i64) -> MonitoringFrame {
        self.trim_vector_events(now);
        let frame_ts = now - now.rem_euclid(FRAME_INTERVAL_MS);
        let task_fresh = self
            .tasks
            .as_ref()
            .filter(|item| is_fresh(item.ts, now, TASK_FRESHNESS_MS));
        let llama_fresh = self
            .llama
            .as_ref()
            .filter(|item| is_fresh(item.ts, now, METRIC_FRESHNESS_MS));
        let slots_fresh = self
            .slots
            .as_ref()
            .filter(|item| is_fresh(item.ts, now, METRIC_FRESHNESS_MS));
        let system_fresh = self
            .system
            .as_ref()
            .filter(|item| is_fresh(item.ts, now, SYSTEM_FRESHNESS_MS));

        let active_requests = task_fresh
            .map(|item| item.value.active)
            .or_else(|| llama_fresh.map(|item| item.value.requests_processing))
            .unwrap_or(0);
        let queued_requests = llama_fresh
            .map(|item| item.value.requests_deferred)
            .unwrap_or(0);
        let slot_capacity = slots_fresh.map(|item| item.value.capacity);
        let busy_slots = slots_fresh.map(|item| item.value.busy);

        let recent_events = self
            .vector_events
            .iter()
            .filter(|event| event.completed_at >= now.saturating_sub(VECTOR_WINDOW_MS))
            .collect::<Vec<_>>();
        let has_proxy_events = recent_events
            .iter()
            .any(|event| event.source == VectorMetricSource::Proxy);
        let rate_events = recent_events
            .iter()
            .copied()
            .filter(|event| !has_proxy_events || event.source == VectorMetricSource::Proxy)
            .collect::<Vec<_>>();
        let log_events = recent_events
            .iter()
            .copied()
            .filter(|event| event.source == VectorMetricSource::Log)
            .collect::<Vec<_>>();

        let items_per_second = self.workload.is_vector().then(|| {
            rate_events
                .iter()
                .map(|event| event.item_count as f64)
                .sum::<f64>()
                / (VECTOR_WINDOW_MS as f64 / 1_000.0)
        });
        let average_latency_ms = average_value(
            rate_events
                .iter()
                .map(|event| event.duration_ms)
                .filter(|value| value.is_finite() && *value >= 0.0),
        );
        let success_rate = (!rate_events.is_empty()).then(|| {
            rate_events.iter().filter(|event| event.succeeded).count() as f64
                / rate_events.len() as f64
                * 100.0
        });
        let completed_input_tps = aggregate_completed_input_tps(&log_events);

        let mut source = "unavailable";
        let output_tokens_per_second = if self.workload == ModelWorkload::Inference {
            if active_requests == 0 {
                source = "idle";
                Some(0.0)
            } else if let Some(tasks) = task_fresh.filter(|item| item.value.throughput > 0.0) {
                source = "task";
                Some(tasks.value.throughput)
            } else if let Some(llama) = llama_fresh {
                source = "llama";
                Some(llama.value.tokens_per_sec.max(0.0))
            } else {
                None
            }
        } else {
            None
        };
        let input_tokens_per_second = if self.workload.is_vector() {
            if active_requests > 0 {
                if let Some(llama) = llama_fresh {
                    source = "llama";
                    Some(llama.value.prompt_tokens_per_sec.max(0.0))
                } else if let Some(tasks) = task_fresh.filter(|item| item.value.throughput > 0.0) {
                    source = "task";
                    Some(tasks.value.throughput)
                } else {
                    None
                }
            } else if let Some(value) = completed_input_tps {
                source = "vector-log";
                Some(value)
            } else {
                source = "idle";
                Some(0.0)
            }
        } else {
            llama_fresh.map(|item| item.value.prompt_tokens_per_sec.max(0.0))
        };

        let throughput = if self.workload == ModelWorkload::Inference {
            output_tokens_per_second
        } else {
            input_tokens_per_second
        };
        let vector_activity = self.workload.is_vector()
            && (items_per_second.unwrap_or(0.0) > 0.0
                || input_tokens_per_second.unwrap_or(0.0) > 0.0);
        let has_fresh_data = task_fresh.is_some()
            || llama_fresh.is_some()
            || slots_fresh.is_some()
            || system_fresh.is_some();
        let state = if active_requests > 0 || vector_activity {
            "active"
        } else if has_fresh_data {
            "idle"
        } else if now.saturating_sub(self.registered_at) < SYSTEM_FRESHNESS_MS {
            "warming"
        } else {
            "unavailable"
        };
        let newest_ts = [
            self.system.as_ref().map(|item| item.ts),
            self.llama.as_ref().map(|item| item.ts),
            self.tasks.as_ref().map(|item| item.ts),
            self.slots.as_ref().map(|item| item.ts),
        ]
        .into_iter()
        .flatten()
        .max();

        MonitoringFrame {
            instance_id: self.instance_id.clone(),
            session_id: self.session_id.clone(),
            session_started_at: self.registered_at,
            ts: frame_ts,
            workload: self.workload.as_str().to_string(),
            state: state.to_string(),
            throughput,
            throughput_unit: if self.workload == ModelWorkload::Inference {
                "tok/s"
            } else {
                "input tok/s"
            }
            .to_string(),
            output_tokens_per_second,
            input_tokens_per_second,
            items_per_second,
            active_requests,
            queued_requests,
            slot_capacity,
            busy_slots,
            average_latency_ms,
            success_rate,
            source: source.to_string(),
            data_age_ms: newest_ts
                .map(|timestamp| now.saturating_sub(timestamp) as u64)
                .unwrap_or_else(|| now.saturating_sub(self.registered_at) as u64),
            system: system_fresh.map(|item| item.value.clone()),
        }
    }
}

static MONITORING: LazyLock<Mutex<HashMap<String, InstanceMonitoringState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn is_fresh(timestamp: i64, now: i64, maximum_age: i64) -> bool {
    timestamp <= now && now.saturating_sub(timestamp) <= maximum_age
}

fn average_value(values: impl Iterator<Item = f64>) -> Option<f64> {
    let (total, count) = values.fold((0.0, 0_u64), |(total, count), value| {
        (total + value, count + 1)
    });
    (count > 0).then_some(total / count as f64)
}

fn current_time_ms() -> i64 {
    crate::commands::telemetry::current_time_ms()
}

fn with_state(
    instance_id: &str,
    session_id: Option<&str>,
    workload: ModelWorkload,
    update: impl FnOnce(&mut InstanceMonitoringState),
) {
    let now = current_time_ms();
    let mut registry = MONITORING.lock().unwrap();
    let state = registry
        .entry(instance_id.to_string())
        .or_insert_with(|| InstanceMonitoringState::new(instance_id, session_id, workload, now));
    if session_id.is_some() || state.session_id.is_none() {
        state.reset_session(session_id, workload, now, false);
    } else if state.workload != workload {
        let existing_session = state.session_id.clone();
        state.reset_session(existing_session.as_deref(), workload, now, false);
    }
    update(state);
}

pub fn update_metrics(
    instance_id: &str,
    session_id: Option<&str>,
    workload: ModelWorkload,
    system: SystemMetrics,
    llama: Option<LlamaMetricSample>,
) {
    let now = current_time_ms();
    with_state(instance_id, session_id, workload, |state| {
        state.system = Some(Timed {
            value: system,
            ts: now,
        });
        state.llama = llama.map(|value| Timed { value, ts: now });
    });
}

pub fn update_tasks(
    instance_id: &str,
    session_id: Option<&str>,
    workload: ModelWorkload,
    active: u64,
    throughput: f64,
) {
    let now = current_time_ms();
    with_state(instance_id, session_id, workload, |state| {
        state.tasks = Some(Timed {
            value: TaskSnapshot {
                active,
                throughput: throughput.max(0.0),
            },
            ts: now,
        });
    });
}

pub fn update_slots(
    instance_id: &str,
    session_id: Option<&str>,
    workload: ModelWorkload,
    capacity: u64,
    busy: u64,
) {
    let now = current_time_ms();
    with_state(instance_id, session_id, workload, |state| {
        state.slots = Some(Timed {
            value: SlotSnapshot { capacity, busy },
            ts: now,
        });
    });
}

#[allow(clippy::too_many_arguments)]
pub fn record_vector_activity(
    instance_id: &str,
    session_id: Option<&str>,
    workload: ModelWorkload,
    source: VectorMetricSource,
    completed_at: i64,
    item_count: u64,
    input_tokens: Option<u64>,
    duration_ms: f64,
    succeeded: bool,
) {
    if !workload.is_vector() {
        return;
    }
    with_state(instance_id, session_id, workload, |state| {
        state.vector_events.push_back(VectorLiveEvent {
            source,
            completed_at,
            item_count,
            input_tokens,
            duration_ms,
            succeeded,
        });
        while state.vector_events.len() > MAX_VECTOR_EVENTS {
            state.vector_events.pop_front();
        }
        state.trim_vector_events(current_time_ms());
    });
}

fn sample_frame(instance_id: &str, now: i64) -> Option<MonitoringFrame> {
    let mut registry = MONITORING.lock().unwrap();
    let state = registry.get_mut(instance_id)?;
    let frame = state.build_frame(now);
    state.push_frame(frame.clone());
    Some(frame)
}

pub fn start_frame_loop(
    instance_id: String,
    expected_pid: u32,
    session_id: Option<String>,
    workload: ModelWorkload,
    app: tauri::AppHandle,
) {
    let should_start = {
        let now = current_time_ms();
        let mut registry = MONITORING.lock().unwrap();
        let state = registry.entry(instance_id.clone()).or_insert_with(|| {
            InstanceMonitoringState::new(&instance_id, session_id.as_deref(), workload, now)
        });
        let same_process = state.frame_loop_pid == Some(expected_pid);
        state.reset_session(session_id.as_deref(), workload, now, !same_process);
        if same_process {
            false
        } else {
            state.frame_loop_pid = Some(expected_pid);
            true
        }
    };
    if !should_start {
        return;
    }

    std::thread::spawn(move || loop {
        let iteration_started = std::time::Instant::now();
        let is_current_process = {
            let state = app.state::<AppState>();
            let running = state.running.lock().unwrap();
            running
                .get(&instance_id)
                .is_some_and(|instance| instance.pid == expected_pid)
        };
        if !is_current_process {
            let mut registry = MONITORING.lock().unwrap();
            let owns_state = registry
                .get(&instance_id)
                .is_some_and(|state| state.frame_loop_pid == Some(expected_pid));
            if owns_state {
                registry.remove(&instance_id);
            }
            break;
        }

        if let Some(frame) = sample_frame(&instance_id, current_time_ms()) {
            let _ = app.emit("monitoring-frame", frame);
        }
        std::thread::sleep(
            Duration::from_millis(FRAME_INTERVAL_MS as u64)
                .saturating_sub(iteration_started.elapsed()),
        );
    });
}

#[tauri::command]
pub fn get_monitoring_series(
    instance_id: Option<String>,
    range_ms: Option<u64>,
) -> Vec<MonitoringFrame> {
    let now = current_time_ms();
    let range = range_ms.unwrap_or(3_600_000).clamp(1_000, 3_600_000) as i64;
    let start = now.saturating_sub(range);
    let registry = MONITORING.lock().unwrap();
    let mut frames = registry
        .values()
        .filter(|state| {
            instance_id
                .as_deref()
                .map_or(true, |expected| state.instance_id == expected)
        })
        .flat_map(|state| state.timeline.iter())
        .filter(|frame| frame.ts >= start)
        .cloned()
        .collect::<Vec<_>>();
    frames.sort_by(|left, right| {
        left.ts
            .cmp(&right.ts)
            .then_with(|| left.instance_id.cmp(&right.instance_id))
    });
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    fn llama(processing: u64, output_tps: f64, input_tps: f64) -> LlamaMetricSample {
        LlamaMetricSample {
            tokens_per_sec: output_tps,
            prompt_tokens: 0,
            gen_tokens: 0,
            decode_calls_total: 0,
            max_tokens_observed: 0,
            prompt_tokens_per_sec: input_tps,
            requests_processing: processing,
            requests_deferred: 0,
            busy_slots_per_decode: 0.0,
        }
    }

    #[test]
    fn inference_prefers_fresh_task_throughput() {
        let now = 20_000;
        let mut state = InstanceMonitoringState::new(
            "instance",
            Some("session"),
            ModelWorkload::Inference,
            now,
        );
        state.llama = Some(Timed {
            value: llama(1, 30.0, 0.0),
            ts: now,
        });
        state.tasks = Some(Timed {
            value: TaskSnapshot {
                active: 1,
                throughput: 60.0,
            },
            ts: now,
        });

        let frame = state.build_frame(now);

        assert_eq!(frame.throughput, Some(60.0));
        assert_eq!(frame.source, "task");
        assert_eq!(frame.throughput_unit, "tok/s");
    }

    #[test]
    fn idle_inference_is_zero_instead_of_reusing_last_rate() {
        let now = 20_000;
        let mut state = InstanceMonitoringState::new(
            "instance",
            Some("session"),
            ModelWorkload::Inference,
            now,
        );
        state.llama = Some(Timed {
            value: llama(0, 72.0, 0.0),
            ts: now,
        });

        let frame = state.build_frame(now);

        assert_eq!(frame.throughput, Some(0.0));
        assert_eq!(frame.state, "idle");
        assert_eq!(frame.source, "idle");
    }

    #[test]
    fn fresh_task_completion_overrides_slower_metrics_polling() {
        let now = 20_000;
        let mut state = InstanceMonitoringState::new(
            "instance",
            Some("session"),
            ModelWorkload::Inference,
            now,
        );
        state.llama = Some(Timed {
            value: llama(1, 72.0, 0.0),
            ts: now.saturating_sub(1_000),
        });
        state.tasks = Some(Timed {
            value: TaskSnapshot {
                active: 0,
                throughput: 0.0,
            },
            ts: now,
        });

        let frame = state.build_frame(now);

        assert_eq!(frame.active_requests, 0);
        assert_eq!(frame.throughput, Some(0.0));
        assert_eq!(frame.state, "idle");
    }

    #[test]
    fn vector_frame_keeps_input_and_item_rates_separate() {
        let now = 20_000;
        let mut state = InstanceMonitoringState::new(
            "instance",
            Some("session"),
            ModelWorkload::Embedding,
            now,
        );
        state.llama = Some(Timed {
            value: llama(1, 99.0, 240.0),
            ts: now,
        });
        state.vector_events.push_back(VectorLiveEvent {
            source: VectorMetricSource::Proxy,
            completed_at: now,
            item_count: 6,
            input_tokens: None,
            duration_ms: 120.0,
            succeeded: true,
        });

        let frame = state.build_frame(now);

        assert_eq!(frame.throughput, Some(240.0));
        assert_eq!(frame.input_tokens_per_second, Some(240.0));
        assert_eq!(frame.items_per_second, Some(2.0));
        assert_eq!(frame.output_tokens_per_second, None);
        assert_eq!(frame.success_rate, Some(100.0));
        assert_eq!(frame.throughput_unit, "input tok/s");
    }

    #[test]
    fn concurrent_vector_completions_use_wall_clock_throughput() {
        let now = 20_000;
        let events = [
            VectorLiveEvent {
                source: VectorMetricSource::Log,
                completed_at: now,
                item_count: 1,
                input_tokens: Some(100),
                duration_ms: 1_000.0,
                succeeded: true,
            },
            VectorLiveEvent {
                source: VectorMetricSource::Log,
                completed_at: now,
                item_count: 1,
                input_tokens: Some(100),
                duration_ms: 1_000.0,
                succeeded: true,
            },
        ];
        let refs = events.iter().collect::<Vec<_>>();

        assert_eq!(aggregate_completed_input_tps(&refs), Some(200.0));
    }

    #[test]
    fn sequential_vector_completions_include_the_observed_work_span() {
        let now = 20_000;
        let events = [
            VectorLiveEvent {
                source: VectorMetricSource::Log,
                completed_at: now - 1_000,
                item_count: 1,
                input_tokens: Some(100),
                duration_ms: 1_000.0,
                succeeded: true,
            },
            VectorLiveEvent {
                source: VectorMetricSource::Log,
                completed_at: now,
                item_count: 1,
                input_tokens: Some(100),
                duration_ms: 1_000.0,
                succeeded: true,
            },
        ];
        let refs = events.iter().collect::<Vec<_>>();

        assert_eq!(aggregate_completed_input_tps(&refs), Some(100.0));
    }
}
