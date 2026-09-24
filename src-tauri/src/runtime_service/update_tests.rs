use super::*;
use crate::runtime_service as runtime;
use std::path::Path;
use std::process::{Child, Command};

const TEST_NAME: &str = "runtime_service::update::tests::update_handoff_with_real_runtime_and_lock";
const ROLE: &str = "LSM_UPDATE_TEST_ROLE";
const DATA: &str = "LSM_UPDATE_TEST_DATA";

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn child(role: &str, data: &Path) -> ChildGuard {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", TEST_NAME, "--nocapture"])
        .env(ROLE, role)
        .env(DATA, data);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    ChildGuard(command.spawn().unwrap())
}

async fn service_ready() {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if runtime::runtime_status().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    })
    .await
    .expect("isolated runtime did not become ready");
}

async fn coordinate(data: &Path) {
    let original_token = runtime::load_or_create_control_token().unwrap();
    let unavailable = transport::acquire_runtime_lock().unwrap().unwrap();
    assert!(prepare_app_update().await.is_err());
    assert!(
        request_permit().await.is_ok(),
        "failed preparation must not pause requests"
    );
    assert_eq!(runtime::load_control_token().unwrap(), original_token);
    assert!(transport::wait_for_runtime_lock(Duration::from_millis(50))
        .await
        .is_err());
    drop(unavailable);

    // Checkpoint hydration during a GUI startup must join the bridge's startup
    // transition instead of immediately querying an unavailable/retiring peer.
    let transition = runtime::RUNTIME_START_LOCK.lock().await;
    let hydration = tokio::spawn(runtime::checkpoint_statuses());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !hydration.is_finished(),
        "hydration bypassed runtime startup"
    );
    let mut service = child("runtime", data);
    service_ready().await;
    drop(transition);
    assert!(hydration.await.unwrap().unwrap().is_empty());
    runtime::heartbeat().await.unwrap();
    let original_pid = runtime::ensure_runtime_service().await.unwrap().service_pid;
    assert_eq!(original_pid, service.0.id());

    // Preparation must drain a request before shutting down the actual peer.
    let permit = request_permit().await.unwrap();
    let preparing = tokio::spawn(prepare_app_update());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!preparing.is_finished());
    drop(permit);
    preparing.await.unwrap().unwrap();
    service.0.wait().unwrap();
    assert!(transport::acquire_runtime_lock().unwrap().is_none());
    assert!(is_update_pause(&runtime::heartbeat().await.unwrap_err()));
    assert!(is_update_pause(
        &runtime::checkpoint_statuses().await.unwrap_err()
    ));
    assert!(is_update_pause(
        &runtime::ensure_runtime_service().await.unwrap_err()
    ));
    prepare_app_update().await.unwrap(); // idempotent, preserves the lease

    // Failed/cancelled installation releases the lease and permits the bridge
    // to reconnect. Only this fixture's private runtime is ever started.
    resume_app_after_update().await.unwrap();
    assert!(transport::acquire_runtime_lock().unwrap().is_some());
    let mut restored = child("runtime", data);
    service_ready().await;
    assert_eq!(
        runtime::ensure_runtime_service().await.unwrap().service_pid,
        restored.0.id()
    );
    prepare_app_update().await.unwrap();
    restored.0.wait().unwrap();
    resume_app_after_update().await.unwrap();

    // An exiting legacy process may outlive the old eight-second retry budget.
    let mut holder = child("lock", data);
    let marker = data.join("lock-ready");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let mut token = runtime::load_control_token().unwrap();
    let error = runtime::wait_for_existing_runtime(&mut token, Duration::from_millis(100))
        .await
        .unwrap_err();
    assert!(error.contains("authenticated service is unavailable"));
    assert!(transport::acquire_runtime_lock().unwrap().is_none());
    let lock = runtime::wait_for_existing_runtime(&mut token, Duration::from_secs(30))
        .await
        .unwrap()
        .expect("lock release must be detected even without a reachable endpoint");
    holder.0.wait().unwrap();
    assert!(transport::acquire_runtime_lock().unwrap().is_none());
    drop(lock);
}

#[test]
fn update_handoff_with_real_runtime_and_lock() {
    if let Ok(role) = std::env::var(ROLE) {
        let data = std::path::PathBuf::from(std::env::var_os(DATA).unwrap());
        crate::utils::set_data_dir_override(data.clone()).unwrap();
        match role.as_str() {
            "runtime" => runtime::run_runtime_service().unwrap(),
            "lock" => {
                let _lock = transport::acquire_runtime_lock().unwrap().unwrap();
                std::fs::write(data.join("lock-ready"), b"ready").unwrap();
                std::thread::sleep(Duration::from_secs(9));
            }
            "coordinator" => tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(coordinate(&data)),
            _ => panic!("unexpected test role"),
        }
        return;
    }
    // Separate process avoids changing the app data override or global gate
    // used by other parallel tests. Never uses the installed app's data.
    let data = std::env::temp_dir().join(format!("lsm-update-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&data).unwrap();
    let mut coordinator = child("coordinator", &data);
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = coordinator.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "update regression timed out"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    drop(coordinator);
    let _ = std::fs::remove_dir_all(&data);
    assert!(
        status.success(),
        "isolated update regression failed: {status}"
    );
}
