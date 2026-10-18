//! Inference observations are separate from engine/request averages. Counters are
//! scoped to a process session, slot and task; replayed logs never enter this path.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

const WINDOW_MS: i64 = 3_000;
const FRESH_MS: i64 = 5_000;
const MAX_TASKS: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateReading {
    pub value: Option<f64>,
    pub state: String,
    pub source: String,
    pub sampled_at: Option<i64>,
    pub window_ms: Option<u64>,
    pub covered: usize,
    pub expected: usize,
}

impl RateReading {
    fn empty(state: &str) -> Self {
        Self {
            value: None,
            state: state.into(),
            source: "none".into(),
            sampled_at: None,
            window_ms: None,
            covered: 0,
            expected: 1,
        }
    }
    fn idle() -> Self {
        Self {
            value: Some(0.0),
            state: "idle".into(),
            expected: 0,
            ..Self::empty("idle")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceTask {
    pub slot_id: u32,
    pub task_id: u64,
    pub phase: String,
    pub started_at: i64,
    pub processed_tokens: Option<u64>,
    pub generated_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub progress: Option<f64>,
    pub prefill_average: Option<f64>,
    pub decode_average: Option<f64>,
    pub prefill: RateReading,
    pub decode: RateReading,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceThroughput {
    pub prefill: RateReading,
    pub decode: RateReading,
    pub tasks: Vec<InferenceTask>,
}

#[derive(Debug, Clone)]
pub struct SlotObservation {
    pub slot: u32,
    pub task: Option<u64>,
    pub processing: bool,
    pub processed: Option<u64>,
    pub generated: Option<u64>,
    pub cached: Option<u64>,
}

pub enum LogObservation {
    Start,
    Prefill {
        tokens: u64,
        elapsed_ms: f64,
        average: f64,
        progress: Option<f64>,
    },
    Decode {
        tokens: u64,
        average: f64,
        recent: Option<f64>,
    },
    Finish,
}

#[derive(Default)]
struct CounterWindow(VecDeque<(f64, u64)>);

impl CounterWindow {
    fn push(&mut self, time: f64, count: u64) -> bool {
        if !time.is_finite() || time < 0.0 {
            return false;
        }
        if let Some(&(previous_time, previous_count)) = self.0.back() {
            if time == previous_time && count == previous_count {
                return false;
            }
            if time <= previous_time
                || count < previous_count
                || time - previous_time > FRESH_MS as f64
            {
                self.0.clear();
            }
        }
        self.0.push_back((time, count));
        while self.0.len() > 2 && self.0[1].0 < time - WINDOW_MS as f64 {
            self.0.pop_front();
        }
        while self.0.len() > 128 {
            self.0.pop_front();
        }
        true
    }
    fn rate(&self) -> Option<(f64, u64)> {
        let (first_time, first_count) = *self.0.front()?;
        let (last_time, last_count) = *self.0.back()?;
        let ms = last_time - first_time;
        // Long gaps are not recent samples, and rounded log clocks can repeat.
        (ms >= 100.0 && ms <= FRESH_MS as f64).then(|| {
            (
                (last_count - first_count) as f64 * 1_000.0 / ms,
                ms.round() as u64,
            )
        })
    }
}

#[derive(Default)]
struct Signal {
    samples: CounterWindow,
    updated: Option<i64>,
    direct: Option<(f64, u64)>,
}

impl Signal {
    fn reading(&self, now: i64, source: &str) -> RateReading {
        let Some(ts) = self.updated else {
            return RateReading::empty("sampling");
        };
        if now < ts || now - ts > FRESH_MS {
            return RateReading::empty("stale");
        }
        let value = self.direct.or_else(|| self.samples.rate());
        let Some((rate, window)) = value else {
            return RateReading::empty("sampling");
        };
        RateReading {
            value: Some(rate),
            state: "live".into(),
            source: source.into(),
            sampled_at: Some(ts),
            window_ms: Some(window),
            covered: 1,
            expected: 1,
        }
    }
    fn push(&mut self, now: i64, clock: f64, count: u64) {
        if self.samples.push(clock, count) {
            self.updated = Some(now);
        }
    }
}

struct Task {
    slot: u32,
    id: u64,
    started: i64,
    observed: i64,
    phase: &'static str,
    processed: Option<u64>,
    generated: Option<u64>,
    cached: Option<u64>,
    progress: Option<f64>,
    prefill_average: Option<f64>,
    decode_average: Option<f64>,
    decode_log_tokens: Option<u64>,
    slots_available: Option<bool>,
    prefill_log: Signal,
    decode_log: Signal,
    prefill_slots: Signal,
    decode_slots: Signal,
}

impl Task {
    fn new(slot: u32, id: u64, now: i64) -> Self {
        Self {
            slot,
            id,
            started: now,
            observed: now,
            phase: "unknown",
            processed: None,
            generated: None,
            cached: None,
            progress: None,
            prefill_average: None,
            decode_average: None,
            decode_log_tokens: None,
            slots_available: None,
            prefill_log: Signal::default(),
            decode_log: Signal::default(),
            prefill_slots: Signal::default(),
            decode_slots: Signal::default(),
        }
    }
    fn snapshot(&self, now: i64) -> InferenceTask {
        let fresh = now >= self.observed && now - self.observed <= FRESH_MS;
        let phase = if fresh { self.phase } else { "unknown" };
        let reading = |wanted: &str, log: &Signal, slots: &Signal| {
            if !fresh {
                return RateReading::empty("stale");
            }
            if phase == "unknown" {
                return RateReading::empty(if self.slots_available == Some(false) {
                    "unavailable"
                } else {
                    "sampling"
                });
            }
            if phase != wanted {
                return RateReading::idle();
            }
            let from_log = log.reading(now, "log");
            let from_slots = slots.reading(now, "slots");
            if from_log.value.is_some() {
                from_log
            } else if from_slots.value.is_some() {
                from_slots
            } else if from_log.state == "stale"
                && (from_slots.state != "sampling" || slots.updated.is_none())
            {
                from_log
            } else {
                from_slots
            }
        };
        InferenceTask {
            slot_id: self.slot,
            task_id: self.id,
            phase: phase.into(),
            started_at: self.started,
            processed_tokens: self.processed,
            generated_tokens: self.generated,
            cached_tokens: self.cached,
            progress: self.progress,
            prefill_average: self.prefill_average,
            decode_average: self.decode_average,
            prefill: reading("prefill", &self.prefill_log, &self.prefill_slots),
            decode: reading("decode", &self.decode_log, &self.decode_slots),
        }
    }
}

#[derive(Default)]
pub struct InferenceTracker {
    tasks: HashMap<(u32, u64), Task>,
    finished: HashMap<(u32, u64), i64>,
    slots_at: Option<i64>,
    unidentified_slots: HashSet<u32>,
    last_log: Option<i64>,
}

impl InferenceTracker {
    fn task(&mut self, slot: u32, id: u64, now: i64) -> &mut Task {
        self.tasks.retain(|(other_slot, other_id), task| {
            (*other_slot != slot || *other_id == id) && now - task.observed <= 60_000
        });
        if self.tasks.len() >= MAX_TASKS && !self.tasks.contains_key(&(slot, id)) {
            if let Some(key) = self
                .tasks
                .iter()
                .min_by_key(|(_, task)| task.observed)
                .map(|(key, _)| *key)
            {
                self.tasks.remove(&key);
            }
        }
        self.tasks
            .entry((slot, id))
            .or_insert_with(|| Task::new(slot, id, now))
    }

    pub fn log(&mut self, slot: u32, id: u64, observation: LogObservation, now: i64) {
        self.last_log = Some(now);
        self.finished.retain(|_, ts| now - *ts <= 30_000);
        if matches!(observation, LogObservation::Finish) {
            // An unidentified busy slot may already belong to the next task.
            // Only a fresh slot observation or identified task can resolve it.
            self.tasks.remove(&(slot, id));
            self.finished.insert((slot, id), now);
            return;
        }
        // A late log cannot resurrect a released task.
        if self.finished.contains_key(&(slot, id)) {
            return;
        }
        self.unidentified_slots.remove(&slot);
        let task = self.task(slot, id, now);
        task.observed = now;
        match observation {
            LogObservation::Start => {}
            LogObservation::Prefill {
                tokens,
                elapsed_ms,
                average,
                progress,
            } => {
                if task.phase == "decode" {
                    return;
                }
                task.phase = "prefill";
                task.processed = Some(tokens);
                task.progress =
                    progress.filter(|value| value.is_finite() && (0.0..=1.0).contains(value));
                task.prefill_average = valid_rate(average);
                task.prefill_log.push(now, elapsed_ms, tokens);
            }
            LogObservation::Decode {
                tokens,
                average,
                recent,
            } => {
                task.phase = "decode";
                task.generated = Some(tokens);
                task.decode_average = valid_rate(average);
                let previous = task.decode_log_tokens.replace(tokens).unwrap_or(0);
                if tokens != previous {
                    // Upstream's first tg_3s line waits for 100 tokens, so its
                    // actual interval can greatly exceed three seconds. Infer
                    // the interval from that line's token delta and rate. A
                    // restored log reader waits for a second line if needed.
                    task.decode_log.direct = recent.and_then(valid_rate).and_then(|rate| {
                        let delta = tokens.checked_sub(previous)?;
                        let ms = delta as f64 * 1000.0 / rate;
                        (rate > 0.0 && ms >= 100.0 && ms <= FRESH_MS as f64)
                            .then_some((rate, ms.round() as u64))
                    });
                    task.decode_log.updated = Some(now);
                }
            }
            LogObservation::Finish => unreachable!(),
        }
    }

    pub fn slots(&mut self, slots: &[SlotObservation], sampled_at: i64) {
        if self.slots_at.is_some_and(|ts| sampled_at <= ts) {
            return;
        }
        self.slots_at = Some(sampled_at);
        self.unidentified_slots = slots
            .iter()
            .filter(|slot| slot.processing && slot.task.is_none())
            .map(|slot| slot.slot)
            .collect();
        self.tasks.retain(|(slot, id), task| {
            task.observed > sampled_at
                || slots
                    .iter()
                    .any(|value| value.slot == *slot && value.processing && value.task == Some(*id))
        });
        for slot in slots.iter().filter(|slot| slot.processing) {
            let Some(id) = slot.task else {
                continue;
            };
            if self
                .tasks
                .values()
                .any(|task| task.slot == slot.slot && task.observed > sampled_at)
            {
                continue;
            }
            if self
                .finished
                .get(&(slot.slot, id))
                .is_some_and(|ts| *ts >= sampled_at)
            {
                continue;
            }
            let task = self.task(slot.slot, id, sampled_at);
            if sampled_at < task.observed {
                continue;
            }
            task.observed = sampled_at;
            task.cached = slot.cached;
            task.slots_available = Some(
                slot.generated.is_some()
                    && (slot.generated.is_some_and(|count| count > 0) || slot.processed.is_some()),
            );
            if task.slots_available == Some(false)
                && ![task.prefill_log.updated, task.decode_log.updated]
                    .iter()
                    .flatten()
                    .any(|ts| sampled_at - *ts <= FRESH_MS)
            {
                task.phase = "unknown";
            }
            if let Some(count) = slot.generated {
                task.generated = Some(count);
                if count > 0 {
                    task.phase = "decode";
                    // Do not dilute Decode with the preceding Prefill interval.
                    task.decode_slots.push(sampled_at, sampled_at as f64, count);
                }
            }
            if let Some(count) = slot.processed {
                task.processed = Some(count);
                task.prefill_slots
                    .push(sampled_at, sampled_at as f64, count);
                if task.phase != "decode" && count > 0 && slot.generated == Some(0) {
                    task.phase = "prefill";
                }
            }
        }
    }

    pub fn snapshot(&self, now: i64, active_hint: Option<u64>) -> InferenceThroughput {
        let fresh_slots = self
            .slots_at
            .is_some_and(|ts| now >= ts && now - ts <= FRESH_MS);
        let log_after_slots = self
            .last_log
            .is_some_and(|ts| Some(ts) > self.slots_at && now - ts <= FRESH_MS);
        let expected = if fresh_slots {
            Some(
                self.tasks.len()
                    + self
                        .unidentified_slots
                        .iter()
                        .filter(|slot| !self.tasks.values().any(|task| task.slot == **slot))
                        .count(),
            )
        } else if log_after_slots {
            Some(self.tasks.len())
        } else {
            active_hint.map(|n| n as usize)
        };
        let mut tasks: Vec<_> = self.tasks.values().map(|task| task.snapshot(now)).collect();
        tasks.sort_by_key(|task| (task.slot_id, task.task_id));
        if expected == Some(0) {
            tasks.clear();
        }
        let aggregate = |prefill: bool| {
            if expected == Some(0) {
                return RateReading::idle();
            }
            let expected = expected.unwrap_or(tasks.len()).max(tasks.len());
            if expected == 0 {
                return RateReading::empty("unavailable");
            }
            let readings: Vec<_> = tasks
                .iter()
                .map(|task| if prefill { &task.prefill } else { &task.decode })
                .collect();
            let covered = readings.iter().filter(|r| r.value.is_some()).count();
            let mut result = RateReading::empty("sampling");
            result.expected = expected;
            result.covered = covered;
            if covered > 0 {
                result.value = Some(readings.iter().filter_map(|r| r.value).sum());
                result.state = if covered < expected {
                    "partial"
                } else if readings.iter().all(|r| r.state == "idle") {
                    "idle"
                } else {
                    "live"
                }
                .into();
                let sources: Vec<_> = readings
                    .iter()
                    .filter(|r| r.state == "live")
                    .map(|r| r.source.as_str())
                    .collect();
                result.source = if sources.iter().all(|s| *s == "log") {
                    "log"
                } else if sources.iter().all(|s| *s == "slots") {
                    "slots"
                } else {
                    "mixed"
                }
                .into();
                result.sampled_at = readings.iter().filter_map(|r| r.sampled_at).min();
                result.window_ms = readings.iter().filter_map(|r| r.window_ms).max();
            } else if readings.iter().any(|r| r.state == "stale") {
                result.state = "stale".into();
            } else if readings.is_empty() || readings.iter().any(|r| r.state == "unavailable") {
                result.state = "unavailable".into();
            }
            result
        };
        InferenceThroughput {
            prefill: aggregate(true),
            decode: aggregate(false),
            tasks,
        }
    }
}

fn valid_rate(value: f64) -> Option<f64> {
    (value.is_finite() && value >= 0.0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(id: u64, processed: u64, generated: u64) -> SlotObservation {
        SlotObservation {
            slot: id as u32,
            task: Some(id),
            processing: true,
            processed: Some(processed),
            generated: Some(generated),
            cached: Some(100_000),
        }
    }
    fn prompt(tokens: u64, ms: f64) -> LogObservation {
        LogObservation::Prefill {
            tokens,
            elapsed_ms: ms,
            average: tokens as f64 * 1000.0 / ms,
            progress: Some(0.5),
        }
    }

    #[test]
    fn prefill_deltas_are_distinct_from_phase_average_and_arrival_time() {
        let mut tracker = InferenceTracker::default();
        tracker.log(0, 1, prompt(3_000, 3_000.0), 10_000);
        assert_eq!(tracker.snapshot(10_000, None).prefill.state, "sampling");
        // Buffered lines arrive together, but the engine elapsed clock is authoritative.
        tracker.log(0, 1, prompt(9_000, 6_000.0), 10_001);
        let snapshot = tracker.snapshot(10_001, None);
        assert_eq!(snapshot.prefill.value, Some(2_000.0));
        assert_eq!(snapshot.tasks[0].prefill_average, Some(1_500.0));
        assert_eq!(snapshot.decode.value, Some(0.0));
        assert_eq!(snapshot.prefill.window_ms, Some(3_000));
    }

    #[test]
    fn slots_restore_requires_new_samples_and_excludes_cached_tokens() {
        let mut tracker = InferenceTracker::default();
        tracker.slots(&[slot(1, 4_000, 0)], 1000);
        assert_eq!(tracker.snapshot(1000, None).prefill.value, None);
        tracker.slots(&[slot(1, 5_000, 0)], 2000);
        assert_eq!(tracker.snapshot(2000, None).prefill.value, Some(1000.0));
        assert_eq!(
            tracker.snapshot(2000, None).tasks[0].cached_tokens,
            Some(100_000)
        );
        tracker.slots(&[slot(1, 5_100, 4)], 3000);
        let snapshot = tracker.snapshot(3000, None);
        assert_eq!(snapshot.prefill.state, "idle");
        assert_eq!(snapshot.tasks[0].phase, "decode");
        assert_eq!(snapshot.decode.value, None);
        tracker.slots(&[slot(1, 5_100, 14)], 4000);
        assert_eq!(tracker.snapshot(4000, None).decode.value, Some(10.0));
        // A second manager session cannot reuse the previous session's counter baseline.
        let mut restored = InferenceTracker::default();
        restored.slots(&[slot(1, 5_100, 400)], 10_000);
        assert_eq!(restored.snapshot(10_000, None).decode.value, None);
        restored.slots(&[slot(1, 5_100, 410)], 11_000);
        assert_eq!(restored.snapshot(11_000, None).decode.value, Some(10.0));
    }

    #[test]
    fn stale_unknown_and_idle_are_different() {
        let mut tracker = InferenceTracker::default();
        assert_eq!(tracker.snapshot(0, None).prefill.state, "unavailable");
        tracker.slots(&[slot(1, 100, 0)], 1000);
        tracker.slots(&[slot(1, 200, 0)], 2000);
        let stale = tracker.snapshot(8000, Some(1));
        assert_eq!(stale.prefill.state, "stale");
        assert_eq!(stale.prefill.value, None);
        assert_eq!(stale.decode.value, None);
        tracker.slots(&[], 9000);
        assert_eq!(tracker.snapshot(9000, None).prefill.value, Some(0.0));
        assert!(tracker.snapshot(9000, None).tasks.is_empty());
    }

    #[test]
    fn missing_slot_phase_counters_do_not_invent_zero_decode() {
        let mut tracker = InferenceTracker::default();
        assert_eq!(tracker.snapshot(1000, Some(1)).prefill.state, "unavailable");
        let missing = SlotObservation {
            generated: None,
            ..slot(1, 1000, 0)
        };
        tracker.slots(std::slice::from_ref(&missing), 1000);
        tracker.slots(&[missing], 2000);
        let snapshot = tracker.snapshot(2000, None);
        assert_eq!(snapshot.prefill.state, "unavailable");
        assert_eq!(snapshot.decode.value, None);
    }

    #[test]
    fn concurrent_phases_and_missing_tasks_keep_coverage_visible() {
        let mut tracker = InferenceTracker::default();
        tracker.slots(&[slot(1, 1000, 0), slot(2, 5000, 100)], 1000);
        tracker.slots(&[slot(1, 3000, 0), slot(2, 5000, 140)], 2000);
        let complete = tracker.snapshot(2000, None);
        assert_eq!(complete.prefill.value, Some(2000.0));
        assert_eq!(complete.decode.value, Some(40.0));
        let unknown = SlotObservation {
            task: None,
            ..slot(3, 0, 0)
        };
        tracker.slots(&[slot(1, 5000, 0), slot(2, 5000, 180), unknown], 3000);
        let partial = tracker.snapshot(3000, None);
        assert_eq!(partial.prefill.state, "partial");
        assert_eq!(partial.decode.covered, 2);
        assert_eq!(partial.decode.expected, 3);
        tracker.log(1, 1, prompt(6000, 3000.0), 3500);
        assert_eq!(
            tracker.snapshot(3500, None).prefill.expected,
            3,
            "a log update must not hide an unidentified busy slot"
        );
    }

    #[test]
    fn completion_and_replacement_reject_inflight_old_slot_samples() {
        let mut tracker = InferenceTracker::default();
        tracker.log(1, 1, LogObservation::Start, 1000);
        tracker.log(1, 1, LogObservation::Finish, 3000);
        tracker.slots(&[slot(1, 1000, 10)], 2000);
        assert!(tracker.snapshot(3000, None).tasks.is_empty());
        tracker.log(1, 2, LogObservation::Start, 4000);
        tracker.slots(&[slot(1, 1000, 10)], 3500);
        let snapshot = tracker.snapshot(4000, None);
        assert_eq!(snapshot.tasks.len(), 1);
        assert_eq!(snapshot.tasks[0].task_id, 2);
        assert_eq!(snapshot.prefill.value, None);
    }

    #[test]
    fn old_task_release_cannot_clear_an_unidentified_replacement() {
        let mut tracker = InferenceTracker::default();
        tracker.log(1, 1, LogObservation::Start, 1000);
        tracker.slots(
            &[SlotObservation {
                task: None,
                ..slot(1, 0, 0)
            }],
            2000,
        );
        tracker.log(1, 1, LogObservation::Finish, 2500);
        let snapshot = tracker.snapshot(2500, None);
        assert_eq!(snapshot.prefill.value, None);
        assert_eq!(snapshot.decode.expected, 1);
        tracker.slots(&[], 3000);
        assert_eq!(tracker.snapshot(3000, None).decode.value, Some(0.0));
    }

    #[test]
    fn counter_reset_and_long_gaps_never_create_spikes() {
        let mut signal = Signal::default();
        signal.push(1000, 1000.0, 10000);
        signal.push(2000, 2000.0, 10);
        assert_eq!(signal.reading(2000, "slots").value, None);
        signal.push(3000, 3000.0, 20);
        assert_eq!(signal.reading(3000, "slots").value, Some(10.0));
        signal.push(20_000, 20_000.0, 2000);
        assert_eq!(signal.reading(20_000, "slots").value, None);
        signal.push(21_000, 21_000.0, 2010);
        assert_eq!(signal.reading(21_000, "slots").value, Some(10.0));
        signal.push(25_000, 21_000.0, 2010);
        assert_eq!(
            signal.reading(27_000, "log").state,
            "stale",
            "duplicate log samples must not extend freshness"
        );
    }

    #[test]
    fn average_decode_without_recent_timing_is_not_live() {
        let mut tracker = InferenceTracker::default();
        tracker.log(
            1,
            2,
            LogObservation::Decode {
                tokens: 200,
                average: 42.0,
                recent: None,
            },
            1000,
        );
        let snapshot = tracker.snapshot(1000, None);
        assert_eq!(snapshot.decode.value, None);
        assert_eq!(snapshot.tasks[0].decode_average, Some(42.0));
        tracker.log(
            1,
            2,
            LogObservation::Decode {
                tokens: 300,
                average: 40.0,
                recent: Some(35.0),
            },
            4000,
        );
        assert_eq!(tracker.snapshot(4000, None).decode.value, Some(35.0));
    }

    #[test]
    fn first_decode_log_may_cover_more_than_three_seconds() {
        let mut tracker = InferenceTracker::default();
        tracker.log(
            0,
            1,
            LogObservation::Decode {
                tokens: 100,
                average: 10.0,
                recent: Some(10.0),
            },
            10_000,
        );
        assert_eq!(
            tracker.snapshot(10_000, None).decode.value,
            None,
            "a ten-second first generation interval is not a recent three-second rate"
        );
        tracker.log(
            0,
            1,
            LogObservation::Decode {
                tokens: 130,
                average: 10.0,
                recent: Some(10.0),
            },
            13_000,
        );
        assert_eq!(tracker.snapshot(13_000, None).decode.window_ms, Some(3000));
        tracker.log(
            0,
            1,
            LogObservation::Decode {
                tokens: 130,
                average: 10.0,
                recent: Some(10.0),
            },
            18_000,
        );
        assert_eq!(
            tracker.snapshot(19_000, None).decode.value,
            None,
            "duplicate generation logs must not keep a rate fresh"
        );
    }
}
