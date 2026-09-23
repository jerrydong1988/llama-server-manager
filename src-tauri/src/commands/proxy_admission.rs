use crate::models::ProxyConfig;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

const MAX_WAITERS: usize = 4096;
const MAX_KEY_WAITERS: usize = 256;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionKeySnapshot {
    pub id: String,
    pub active: usize,
    pub queued: usize,
    pub limit: usize,
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    async fn saturated_key_cannot_fill_shared_waiters_or_block_another_key() {
        let a = Arc::new(Admission::default());
        a.configure(&ProxyConfig {
            max_concurrent_requests: 2,
            api_keys: vec![ProxyApiKey {
                id: "a".into(),
                max_concurrent_requests: 1,
                ..Default::default()
            }],
            ..Default::default()
        });
        let hold = a.acquire("a", Duration::from_secs(1)).await.unwrap();
        let mut tasks = Vec::new();
        for _ in 0..MAX_KEY_WAITERS {
            let a = a.clone();
            tasks.push(tokio::spawn(async move {
                a.acquire("a", Duration::from_secs(20)).await
            }));
        }
        queued(&a, MAX_KEY_WAITERS).await;
        assert!(a.acquire("a", Duration::from_millis(10)).await.is_none());
        let other = a.acquire("b", Duration::from_millis(100)).await;
        assert!(other.is_some());
        drop(other);
        drop(hold);
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
        assert_eq!(a.snapshot().queued, 0);
    }

    use super::*;
    use crate::models::ProxyApiKey;

    async fn queued(admission: &Admission, count: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while admission.snapshot().queued != count {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn rotates_three_keys_preserving_key_fifo_and_releases_live_dimensions() {
        let a = Arc::new(Admission::default());
        a.configure(&ProxyConfig {
            max_concurrent_requests: 1,
            fair_queue_enabled: true,
            ..Default::default()
        });
        let hold = a.acquire("hold", Duration::from_secs(1)).await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut tasks = Vec::new();
        for (index, key) in ["a", "a", "b", "c", "b", "c"].into_iter().enumerate() {
            let admission = a.clone();
            let tx = tx.clone();
            tasks.push(tokio::spawn(async move {
                let permit = admission
                    .acquire(key, Duration::from_secs(5))
                    .await
                    .unwrap();
                let (release, wait) = tokio::sync::oneshot::channel();
                tx.send((key, index, release)).unwrap();
                let _ = wait.await;
                drop(permit);
            }));
            queued(&a, index + 1).await;
        }
        hold.target("public-model", "instance");
        assert_eq!(a.snapshot().models["public-model"], 1);
        drop(hold);
        for expected in [("a", 0), ("b", 2), ("c", 3), ("a", 1), ("b", 4), ("c", 5)] {
            let (key, index, release) = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!((key, index), expected);
            release.send(()).unwrap();
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(a.active(), 0);
        assert_eq!(a.snapshot().queued, 0);
        assert!(a.snapshot().models.is_empty());
        assert!(a.snapshot().instances.is_empty());
    }

    #[tokio::test]
    async fn key_limits_do_not_block_other_keys_and_hot_updates_wake_waiters() {
        let a = Arc::new(Admission::default());
        let mut config = ProxyConfig {
            max_concurrent_requests: 3,
            api_keys: vec![ProxyApiKey {
                id: "a".into(),
                max_concurrent_requests: 1,
                ..Default::default()
            }],
            ..Default::default()
        };
        a.configure(&config);
        let first = a.acquire("a", Duration::from_secs(1)).await.unwrap();
        let next = {
            let a = a.clone();
            tokio::spawn(async move { a.acquire("a", Duration::from_secs(5)).await.unwrap() })
        };
        queued(&a, 1).await;
        let other = a.acquire("b", Duration::from_millis(100)).await.unwrap();
        assert_eq!(a.active(), 2);
        config.api_keys[0].max_concurrent_requests = 2;
        a.configure(&config);
        let second = tokio::time::timeout(Duration::from_secs(1), next)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(a.active(), 3);
        config.max_concurrent_requests = 1;
        a.configure(&config);
        assert!(a.acquire("c", Duration::from_millis(10)).await.is_none());
        assert_eq!(a.snapshot().queued, 0);
        drop((first, second, other));
        assert_eq!(a.active(), 0);
    }

    #[tokio::test]
    async fn aborted_waiter_and_timeout_leave_no_queue_or_permit() {
        let a = Arc::new(Admission::default());
        a.configure(&ProxyConfig {
            max_concurrent_requests: 1,
            fair_queue_enabled: true,
            ..Default::default()
        });
        let first = a.acquire("a", Duration::from_secs(1)).await.unwrap();
        let next = {
            let a = a.clone();
            tokio::spawn(async move { a.acquire("b", Duration::from_secs(5)).await })
        };
        queued(&a, 1).await;
        next.abort();
        assert!(matches!(next.await, Err(e) if e.is_cancelled()));
        queued(&a, 0).await;
        assert!(a.acquire("c", Duration::from_millis(10)).await.is_none());
        assert_eq!(a.snapshot().queued, 0);
        assert_eq!(a.active(), 1);
        drop(first);
        assert!(a.acquire("d", Duration::from_secs(1)).await.is_some());
        assert_eq!(a.active(), 0);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionSnapshot {
    pub active: usize,
    pub queued: usize,
    pub limit: usize,
    pub queue_limit: usize,
    pub fair: bool,
    pub keys: Vec<AdmissionKeySnapshot>,
    pub models: BTreeMap<String, usize>,
    pub instances: BTreeMap<String, usize>,
}

#[derive(Default)]
struct State {
    limit: usize,
    fair: bool,
    limits: HashMap<String, usize>,
    next: u64,
    waiting: VecDeque<(u64, String)>,
    turns: VecDeque<String>,
    active: HashMap<u64, (String, String, String)>,
    key_active: HashMap<String, usize>,
}

impl State {
    fn key_limit(&self, key: &str) -> usize {
        self.limits
            .get(key)
            .copied()
            .filter(|n| *n > 0)
            .unwrap_or(self.limit)
            .min(self.limit)
    }

    fn available(&self, key: &str) -> bool {
        self.key_active.get(key).copied().unwrap_or(0) < self.key_limit(key)
    }

    fn selected(&self) -> Option<u64> {
        if self.active.len() >= self.limit {
            return None;
        }
        if self.fair {
            let key = self.turns.iter().find(|key| self.available(key))?;
            self.waiting
                .iter()
                .find(|(_, k)| k == key)
                .map(|(id, _)| *id)
        } else {
            self.waiting
                .iter()
                .find(|(_, key)| self.available(key))
                .map(|(id, _)| *id)
        }
    }

    fn remove_waiter(&mut self, id: u64, rotate: bool) -> Option<String> {
        let index = self.waiting.iter().position(|(ticket, _)| *ticket == id)?;
        let (_, key) = self.waiting.remove(index)?;
        let remaining = self.waiting.iter().any(|(_, k)| *k == key);
        if rotate || !remaining {
            self.turns.retain(|k| *k != key);
            if remaining {
                self.turns.push_back(key.clone());
            }
        }
        Some(key)
    }
}

pub(crate) struct Admission {
    state: Mutex<State>,
    notify: Notify,
}

impl Default for Admission {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                limit: 64,
                ..State::default()
            }),
            notify: Notify::new(),
        }
    }
}

pub(crate) struct AdmissionPermit {
    admission: Arc<Admission>,
    id: u64,
}
struct Waiter {
    admission: Arc<Admission>,
    id: u64,
}

impl Drop for Waiter {
    fn drop(&mut self) {
        self.admission
            .state
            .lock()
            .unwrap()
            .remove_waiter(self.id, false);
        self.admission.notify.notify_waiters();
    }
}

impl AdmissionPermit {
    pub(crate) fn target(&self, model: &str, instance: &str) {
        if let Some(value) = self
            .admission
            .state
            .lock()
            .unwrap()
            .active
            .get_mut(&self.id)
        {
            value.1 = model.chars().take(512).collect();
            value.2 = instance.to_string();
        }
    }
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        let mut state = self.admission.state.lock().unwrap();
        if let Some((key, _, _)) = state.active.remove(&self.id) {
            if let Some(count) = state.key_active.get_mut(&key) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    state.key_active.remove(&key);
                }
            }
        }
        drop(state);
        self.admission.notify.notify_waiters();
    }
}

impl Admission {
    pub(crate) fn configure(&self, config: &ProxyConfig) {
        let mut state = self.state.lock().unwrap();
        state.limit = config.max_concurrent_requests.max(1) as usize;
        state.fair = config.fair_queue_enabled;
        state.limits = config
            .api_keys
            .iter()
            .map(|key| (key.id.clone(), key.max_concurrent_requests as usize))
            .collect();
        drop(state);
        self.notify.notify_waiters();
    }

    pub(crate) async fn acquire(
        self: &Arc<Self>,
        key: &str,
        timeout: Duration,
    ) -> Option<AdmissionPermit> {
        let waiter = {
            let mut state = self.state.lock().unwrap();
            if state.active.len() < state.limit
                && state.available(key)
                && state.selected().is_none()
            {
                let id = state.next;
                state.next = state.next.wrapping_add(1);
                *state.key_active.entry(key.to_string()).or_default() += 1;
                state
                    .active
                    .insert(id, (key.to_string(), String::new(), String::new()));
                return Some(AdmissionPermit {
                    admission: self.clone(),
                    id,
                });
            }
            if state.waiting.len() >= MAX_WAITERS
                || state
                    .waiting
                    .iter()
                    .filter(|(_, waiting_key)| waiting_key == key)
                    .count()
                    >= MAX_KEY_WAITERS
            {
                return None;
            }
            let id = state.next;
            state.next = state.next.wrapping_add(1);
            if !state.turns.iter().any(|k| k == key) {
                state.turns.push_back(key.to_string());
            }
            state.waiting.push_back((id, key.to_string()));
            Waiter {
                admission: self.clone(),
                id,
            }
        };
        let acquired = tokio::time::timeout(timeout, async {
            loop {
                // Register before inspecting state; releases and policy updates cannot be lost.
                let notified = self.notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                {
                    let mut state = self.state.lock().unwrap();
                    if state.selected() == Some(waiter.id) {
                        let key = state.remove_waiter(waiter.id, true).unwrap();
                        *state.key_active.entry(key.clone()).or_default() += 1;
                        state
                            .active
                            .insert(waiter.id, (key, String::new(), String::new()));
                        self.notify.notify_waiters();
                        return AdmissionPermit {
                            admission: self.clone(),
                            id: waiter.id,
                        };
                    }
                }
                notified.await;
            }
        })
        .await
        .ok();
        drop(waiter);
        acquired
    }

    pub(crate) fn active(&self) -> usize {
        self.state.lock().unwrap().active.len()
    }

    pub(crate) fn snapshot(&self) -> AdmissionSnapshot {
        let state = self.state.lock().unwrap();
        let mut keys: BTreeMap<String, AdmissionKeySnapshot> = BTreeMap::new();
        for key in state
            .limits
            .keys()
            .chain(state.key_active.keys())
            .chain(state.turns.iter())
        {
            keys.entry(key.clone())
                .or_insert_with(|| AdmissionKeySnapshot {
                    id: key.clone(),
                    active: state.key_active.get(key).copied().unwrap_or(0),
                    queued: 0,
                    limit: state.key_limit(key),
                });
        }
        for (_, key) in &state.waiting {
            keys.get_mut(key).unwrap().queued += 1;
        }
        let mut models = BTreeMap::new();
        let mut instances = BTreeMap::new();
        for (_, model, instance) in state.active.values() {
            if !model.is_empty() {
                *models.entry(model.clone()).or_default() += 1;
            }
            if !instance.is_empty() {
                *instances.entry(instance.clone()).or_default() += 1;
            }
        }
        AdmissionSnapshot {
            active: state.active.len(),
            queued: state.waiting.len(),
            limit: state.limit,
            queue_limit: MAX_WAITERS,
            fair: state.fair,
            keys: keys.into_values().collect(),
            models,
            instances,
        }
    }
}
