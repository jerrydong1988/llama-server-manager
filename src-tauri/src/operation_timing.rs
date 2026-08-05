use std::time::Instant;

const SLOW_OPERATION_MS: u128 = 250;

pub struct OperationTiming {
    name: &'static str,
    started_at: Instant,
    previous_at: Instant,
    stages: Vec<(&'static str, u128)>,
    outcome: &'static str,
}

impl OperationTiming {
    pub fn new(name: &'static str) -> Self {
        let now = Instant::now();
        Self {
            name,
            started_at: now,
            previous_at: now,
            stages: Vec::new(),
            outcome: "incomplete",
        }
    }

    pub fn mark(&mut self, stage: &'static str) {
        let now = Instant::now();
        self.stages
            .push((stage, now.duration_since(self.previous_at).as_millis()));
        self.previous_at = now;
    }

    pub fn finish(&mut self, outcome: &'static str) {
        self.outcome = outcome;
    }
}

impl Drop for OperationTiming {
    fn drop(&mut self) {
        let finished_at = Instant::now();
        let total_ms = finished_at.duration_since(self.started_at).as_millis();
        let tracing_enabled = std::env::var("LSM_TRACE_OPERATIONS").is_ok_and(|value| value == "1");
        if total_ms < SLOW_OPERATION_MS && !tracing_enabled {
            return;
        }

        let mut stages = self.stages.clone();
        stages.push((
            "complete",
            finished_at.duration_since(self.previous_at).as_millis(),
        ));
        let breakdown = stages
            .into_iter()
            .map(|(name, elapsed_ms)| format!("{name}={elapsed_ms}ms"))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!(
            "[operation-timing] {} outcome={} total={}ms {}",
            self.name, self.outcome, total_ms, breakdown
        );
    }
}
