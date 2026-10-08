//! Counter operations compatible with the declared Rust 1.80 minimum.
//! `fetch_update` is deprecated in 1.99; its replacement needs Rust 1.95.
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub(super) fn reserve(counter: &AtomicUsize, amount: usize, limit: usize) -> bool {
    let mut current = counter.load(Ordering::Acquire);
    loop {
        let Some(next) = current.checked_add(amount).filter(|next| *next <= limit) else {
            return false;
        };
        match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(actual) => current = actual,
        }
    }
}

pub(super) fn saturating_sub(counter: &AtomicU64, amount: u64) {
    let mut current = counter.load(Ordering::Relaxed);
    while let Err(actual) = counter.compare_exchange_weak(
        current,
        current.saturating_sub(amount),
        Ordering::Relaxed,
        Ordering::Relaxed,
    ) {
        current = actual;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    #[test]
    fn reservation_rejects_overflow_and_preserves_capacity_on_failure() {
        let counter = AtomicUsize::new(1);
        assert!(!reserve(&counter, usize::MAX, 100));
        assert_eq!(counter.load(Ordering::Acquire), 1);
        assert!(reserve(&counter, 99, 100));
        assert!(!reserve(&counter, 1, 100));
        assert_eq!(counter.load(Ordering::Acquire), 100);
    }

    #[test]
    fn concurrent_reservations_never_exceed_the_limit() {
        let counter = AtomicUsize::new(0);
        let start = Barrier::new(8);
        let granted = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        (0..1000).filter(|_| reserve(&counter, 1, 4000)).count()
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .sum::<usize>()
        });
        assert_eq!(granted, 4000);
        assert_eq!(counter.load(Ordering::Acquire), 4000);
    }

    #[test]
    fn decrements_saturate_without_losing_concurrent_increments() {
        let counter = AtomicU64::new(3);
        saturating_sub(&counter, 5);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        saturating_sub(&counter, u64::MAX);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        let start = Barrier::new(8);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    start.wait();
                    for _ in 0..1000 {
                        counter.fetch_add(2, Ordering::Relaxed);
                        saturating_sub(&counter, 1);
                    }
                });
            }
        });
        assert_eq!(counter.load(Ordering::Relaxed), 8000);
    }
}
