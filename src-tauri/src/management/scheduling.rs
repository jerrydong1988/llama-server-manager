use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use serde_json::Value;

#[derive(Default)]
pub(super) struct ResourceLocks(Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>);

impl ResourceLocks {
    pub async fn acquire(&self, operation: &str, args: &Value) -> tokio::sync::OwnedMutexGuard<()> {
        let key = resource_key(operation, args);
        let lock = {
            let mut locks = self.0.lock().unwrap();
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key, Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }
}

fn resource_key(operation: &str, args: &Value) -> String {
    let domain = operation.split('.').next().unwrap_or(operation);
    match domain {
        "instances" | "presets" | "checkpoints" => {
            format!("instance:{}", args["id"].as_str().unwrap_or("create"))
        }
        // These commands operate on shared domain state. Their network-only
        // preparation runs before this lock; commit functions also enforce CAS.
        _ => domain.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn slow_domains_do_not_block_stop_and_same_instance_stays_serial() {
        let locks = ResourceLocks::default();
        let _download = locks.acquire("downloads.add", &json!({})).await;
        let _engine = locks.acquire("engines.probe_all", &json!({})).await;
        let instance = locks.acquire("instances.start", &json!({"id":"one"})).await;
        let other = tokio::time::timeout(std::time::Duration::from_millis(100), locks.acquire("instances.stop", &json!({"id":"two"}))).await;
        assert!(other.is_ok());
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), locks.acquire("instances.stop", &json!({"id":"one"}))).await.is_err());
        drop(instance);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(100), locks.acquire("checkpoints.clear", &json!({"id":"one"}))).await.is_ok());
    }
}
