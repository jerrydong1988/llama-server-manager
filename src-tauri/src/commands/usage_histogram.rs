use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Sparse, fixed-boundary buckets: exact below 32 ms, at most 1/16 relative
/// rounding above that. Bucket keys are upper bounds and merge without rebinning.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Histogram {
    buckets: BTreeMap<u64, u64>,
    count: u64,
    max: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Percentiles {
    pub samples: u64,
    pub p50: Option<u64>,
    pub p95: Option<u64>,
    pub max: Option<u64>,
}

impl Histogram {
    pub fn observe(&mut self, value: u64) {
        let exponent = 63_u32.saturating_sub(value.leading_zeros());
        let step = 1_u64 << exponent.saturating_sub(4);
        let upper = value.div_ceil(step).saturating_mul(step);
        let bucket = self.buckets.entry(upper).or_default();
        *bucket = bucket.saturating_add(1);
        self.count = self.count.saturating_add(1);
        self.max = self.max.max(value);
    }

    pub fn merge(&mut self, other: &Self) {
        for (upper, count) in &other.buckets {
            let bucket = self.buckets.entry(*upper).or_default();
            *bucket = bucket.saturating_add(*count);
        }
        self.count = self.count.saturating_add(other.count);
        self.max = self.max.max(other.max);
    }

    fn percentile(&self, percent: u64) -> Option<u64> {
        if self.count == 0 {
            return None;
        }
        let rank = (u128::from(self.count) * u128::from(percent)).div_ceil(100);
        let mut seen = 0_u128;
        for (upper, count) in &self.buckets {
            seen += u128::from(*count);
            if seen >= rank {
                return Some((*upper).min(self.max));
            }
        }
        None
    }

    pub fn view(&self) -> Percentiles {
        Percentiles {
            samples: self.count,
            p50: self.percentile(50),
            p95: self.percentile(95),
            max: (self.count > 0).then_some(self.max),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_zero_outliers_and_merged_population_are_distinct() {
        let mut a = Histogram::default();
        assert_eq!(a.view().p95, None);
        a.observe(0);
        assert_eq!(a.view().p95, Some(0));
        for _ in 0..99 {
            a.observe(10);
        }
        let mut b = Histogram::default();
        b.observe(100_000);
        a.merge(&b);
        assert_eq!(a.view().samples, 101);
        assert_eq!(a.view().p95, Some(10));
        assert_eq!(a.view().max, Some(100_000));
        let decoded: Histogram = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(decoded.view().p95, Some(10));
    }

    #[test]
    fn boundaries_never_understate_rank_and_remain_bounded() {
        let values: Vec<u64> = (0..100_000).chain([u64::MAX - 1, u64::MAX]).collect();
        let mut aggregate = Histogram::default();
        for value in &values {
            aggregate.observe(*value);
        }
        let view = aggregate.view();
        let exact = values[(values.len() * 95).div_ceil(100) - 1];
        let approximate = view.p95.unwrap();
        assert!(approximate >= exact);
        assert!(approximate - exact <= exact / 16 + 1);
        assert!(aggregate.buckets.len() < 1024);
        assert_eq!(view.max, Some(u64::MAX));
    }
}
