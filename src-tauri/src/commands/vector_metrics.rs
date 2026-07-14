use serde::Serialize;

const MAX_TREND_BUCKETS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorEventSource {
    Log,
    Proxy,
}

impl VectorEventSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Log => "log",
            Self::Proxy => "proxy",
        }
    }

    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "log" => Some(Self::Log),
            "proxy" => Some(Self::Proxy),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VectorEventPoint {
    pub source: VectorEventSource,
    pub completed_at: i64,
    pub duration_ms: f64,
    pub item_count: u64,
    pub input_tokens: Option<u64>,
    pub http_status: Option<u16>,
    pub has_error: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VectorTrendBucket {
    pub timestamp: i64,
    pub input_tokens_per_second: Option<f64>,
    pub items_per_second: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogVectorSummary {
    pub available: bool,
    pub completed_items: u64,
    pub input_tokens: Option<u64>,
    pub average_input_tokens_per_second: Option<f64>,
    pub average_items_per_second: Option<f64>,
    pub task_duration_p50_ms: Option<f64>,
    pub task_duration_p95_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProxyVectorSummary {
    pub available: bool,
    pub request_count: u64,
    pub item_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub success_rate: Option<f64>,
    pub failure_rate: Option<f64>,
    pub http_duration_p50_ms: Option<f64>,
    pub http_duration_p95_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VectorActivitySummary {
    pub log: LogVectorSummary,
    pub proxy: ProxyVectorSummary,
}

pub fn aggregate_log_buckets(
    events: &[VectorEventPoint],
    range_start: i64,
    range_end: i64,
    bucket_ms: i64,
) -> Vec<VectorTrendBucket> {
    if bucket_ms <= 0 || range_end <= range_start {
        return Vec::new();
    }
    let span = range_end.saturating_sub(range_start);
    let bucket_count = span.saturating_add(bucket_ms - 1) / bucket_ms;
    let Ok(bucket_count) = usize::try_from(bucket_count) else {
        return Vec::new();
    };
    if bucket_count == 0 || bucket_count > MAX_TREND_BUCKETS {
        return Vec::new();
    }

    let qualifying = events
        .iter()
        .filter(|event| {
            event.source == VectorEventSource::Log
                && valid_duration(event.duration_ms)
                && event.completed_at >= range_start
                && event.completed_at < range_end
        })
        .collect::<Vec<_>>();
    let tokens_available = qualifying.iter().any(|event| event.input_tokens.is_some());
    let mut item_counts = vec![0_u64; bucket_count];
    let mut token_counts = vec![0_u64; bucket_count];
    for event in qualifying {
        let index = ((event.completed_at - range_start) / bucket_ms) as usize;
        item_counts[index] = item_counts[index].saturating_add(event.item_count);
        if let Some(tokens) = event.input_tokens {
            token_counts[index] = token_counts[index].saturating_add(tokens);
        }
    }

    (0..bucket_count)
        .map(|index| {
            let timestamp = range_start.saturating_add((index as i64).saturating_mul(bucket_ms));
            let bucket_end = timestamp.saturating_add(bucket_ms).min(range_end);
            let seconds = (bucket_end - timestamp) as f64 / 1_000.0;
            VectorTrendBucket {
                timestamp,
                input_tokens_per_second: tokens_available
                    .then(|| token_counts[index] as f64 / seconds),
                items_per_second: item_counts[index] as f64 / seconds,
            }
        })
        .collect()
}

pub fn summarize_vector_events(
    events: &[VectorEventPoint],
    window_seconds: f64,
) -> VectorActivitySummary {
    let valid_window = window_seconds.is_finite() && window_seconds > 0.0;
    let valid_events = events
        .iter()
        .filter(|event| valid_duration(event.duration_ms))
        .collect::<Vec<_>>();
    let log_events = valid_events
        .iter()
        .copied()
        .filter(|event| event.source == VectorEventSource::Log)
        .collect::<Vec<_>>();
    let proxy_events = valid_events
        .iter()
        .copied()
        .filter(|event| event.source == VectorEventSource::Proxy)
        .collect::<Vec<_>>();

    let completed_items = log_events
        .iter()
        .fold(0_u64, |total, event| total.saturating_add(event.item_count));
    let log_input_available = log_events.iter().any(|event| event.input_tokens.is_some());
    let input_tokens = log_input_available.then(|| {
        log_events.iter().fold(0_u64, |total, event| {
            total.saturating_add(event.input_tokens.unwrap_or(0))
        })
    });
    let log_durations = log_events
        .iter()
        .map(|event| event.duration_ms)
        .collect::<Vec<_>>();
    let log_available = !log_events.is_empty();

    let request_count = proxy_events.len() as u64;
    let proxy_item_count = proxy_events
        .iter()
        .fold(0_u64, |total, event| total.saturating_add(event.item_count));
    let success_count = proxy_events
        .iter()
        .filter(|event| {
            !event.has_error
                && event
                    .http_status
                    .is_some_and(|status| (200..300).contains(&status))
        })
        .count() as u64;
    let failure_count = request_count.saturating_sub(success_count);
    let proxy_durations = proxy_events
        .iter()
        .map(|event| event.duration_ms)
        .collect::<Vec<_>>();
    let proxy_available = !proxy_events.is_empty();

    VectorActivitySummary {
        log: LogVectorSummary {
            available: log_available,
            completed_items,
            input_tokens,
            average_input_tokens_per_second: (log_available && log_input_available && valid_window)
                .then(|| input_tokens.unwrap_or(0) as f64 / window_seconds),
            average_items_per_second: (log_available && valid_window)
                .then(|| completed_items as f64 / window_seconds),
            task_duration_p50_ms: percentile(&log_durations, 0.50),
            task_duration_p95_ms: percentile(&log_durations, 0.95),
        },
        proxy: ProxyVectorSummary {
            available: proxy_available,
            request_count,
            item_count: proxy_item_count,
            success_count,
            failure_count,
            success_rate: proxy_available.then(|| success_count as f64 / request_count as f64),
            failure_rate: proxy_available.then(|| failure_count as f64 / request_count as f64),
            http_duration_p50_ms: percentile(&proxy_durations, 0.50),
            http_duration_p95_ms: percentile(&proxy_durations, 0.95),
        },
    }
}

fn valid_duration(duration_ms: f64) -> bool {
    duration_ms.is_finite() && duration_ms >= 0.0
}

fn percentile(values: &[f64], percentile: f64) -> Option<f64> {
    if values.is_empty() || !percentile.is_finite() || !(0.0..=1.0).contains(&percentile) {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (percentile * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted.get(rank.saturating_sub(1)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        source: VectorEventSource,
        completed_at: i64,
        duration_ms: f64,
        item_count: u64,
        input_tokens: Option<u64>,
        http_status: Option<u16>,
        has_error: bool,
    ) -> VectorEventPoint {
        VectorEventPoint {
            source,
            completed_at,
            duration_ms,
            item_count,
            input_tokens,
            http_status,
            has_error,
        }
    }

    #[test]
    fn buckets_sum_concurrent_completions_and_fill_idle_intervals() {
        let events = vec![
            event(
                VectorEventSource::Log,
                10_100,
                20.0,
                2,
                Some(20),
                None,
                false,
            ),
            event(
                VectorEventSource::Log,
                10_900,
                30.0,
                3,
                Some(30),
                None,
                false,
            ),
            event(
                VectorEventSource::Log,
                20_100,
                40.0,
                1,
                Some(5),
                None,
                false,
            ),
        ];

        let buckets = aggregate_log_buckets(&events, 10_000, 40_000, 10_000);

        assert_eq!(buckets.len(), 3);
        assert_eq!(
            buckets
                .iter()
                .map(|bucket| bucket.timestamp)
                .collect::<Vec<_>>(),
            vec![10_000, 20_000, 30_000]
        );
        assert_eq!(
            buckets
                .iter()
                .map(|bucket| bucket.items_per_second)
                .collect::<Vec<_>>(),
            vec![0.5, 0.1, 0.0]
        );
        assert_eq!(buckets[0].input_tokens_per_second, Some(5.0));
        assert_eq!(buckets[1].input_tokens_per_second, Some(0.5));
        assert_eq!(buckets[2].input_tokens_per_second, Some(0.0));
    }

    #[test]
    fn input_throughput_remains_unavailable_without_token_samples() {
        let events = vec![event(
            VectorEventSource::Log,
            10_100,
            10.0,
            1,
            None,
            None,
            false,
        )];

        let buckets = aggregate_log_buckets(&events, 10_000, 30_000, 10_000);

        assert_eq!(buckets.len(), 2);
        assert!(buckets
            .iter()
            .all(|bucket| bucket.input_tokens_per_second.is_none()));
    }

    #[test]
    fn percentiles_use_nearest_rank_and_ignore_invalid_durations() {
        let events = vec![
            event(VectorEventSource::Log, 1, 10.0, 1, Some(1), None, false),
            event(VectorEventSource::Log, 2, 20.0, 1, Some(1), None, false),
            event(VectorEventSource::Log, 3, 30.0, 1, Some(1), None, false),
            event(VectorEventSource::Log, 4, 40.0, 1, Some(1), None, false),
            event(VectorEventSource::Log, 5, -1.0, 100, Some(100), None, false),
            event(
                VectorEventSource::Log,
                6,
                f64::NAN,
                100,
                Some(100),
                None,
                false,
            ),
        ];

        let summary = summarize_vector_events(&events, 10.0);

        assert_eq!(summary.log.completed_items, 4);
        assert_eq!(summary.log.input_tokens, Some(4));
        assert_eq!(summary.log.task_duration_p50_ms, Some(20.0));
        assert_eq!(summary.log.task_duration_p95_ms, Some(40.0));
    }

    #[test]
    fn log_and_proxy_summaries_do_not_double_count() {
        let events = vec![
            event(VectorEventSource::Log, 10, 4.0, 4, Some(40), None, false),
            event(VectorEventSource::Proxy, 11, 8.0, 4, None, Some(200), false),
            event(VectorEventSource::Proxy, 12, 12.0, 2, None, Some(503), true),
        ];

        let summary = summarize_vector_events(&events, 10.0);

        assert!(summary.log.available);
        assert_eq!(summary.log.completed_items, 4);
        assert_eq!(summary.log.input_tokens, Some(40));
        assert_eq!(summary.log.average_items_per_second, Some(0.4));
        assert!(summary.proxy.available);
        assert_eq!(summary.proxy.request_count, 2);
        assert_eq!(summary.proxy.item_count, 6);
        assert_eq!(summary.proxy.success_count, 1);
        assert_eq!(summary.proxy.failure_count, 1);
        assert_eq!(summary.proxy.success_rate, Some(0.5));
        assert_eq!(summary.proxy.failure_rate, Some(0.5));
        assert_eq!(summary.proxy.http_duration_p50_ms, Some(8.0));
        assert_eq!(summary.proxy.http_duration_p95_ms, Some(12.0));
    }

    #[test]
    fn empty_summary_preserves_source_unavailability() {
        let summary = summarize_vector_events(&[], 10.0);

        assert!(!summary.log.available);
        assert_eq!(summary.log.average_items_per_second, None);
        assert_eq!(summary.log.average_input_tokens_per_second, None);
        assert!(!summary.proxy.available);
        assert_eq!(summary.proxy.success_rate, None);
        assert_eq!(summary.proxy.failure_rate, None);
    }
}
