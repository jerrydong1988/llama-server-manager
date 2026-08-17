use crate::models::{InstanceConfig, ProxyConfig, ProxyStatus, RunningInstance};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const RUNTIME_PROTOCOL_VERSION: u32 = 1;
pub const RUNTIME_STATE_SCHEMA_VERSION: u32 = 2;
pub const MAX_RUNTIME_FRAME_BYTES: usize = 8 * 1024 * 1024;
pub const BACKGROUND_DETACH_CAPABILITY: &str = "background_detach_v1";
pub const RUNTIME_ERROR_ACK_CAPABILITY: &str = "runtime_error_ack_v1";
pub const CONFIG_SYNC_ACK_CAPABILITY: &str = "config_sync_ack_v1";
pub const INSTANCE_RECOVERY_CAPABILITY: &str = "instance_recovery_v1";
pub const INSTANCE_RECOVERY_MAX_ATTEMPTS: u32 = 3;
pub const INSTANCE_RECOVERY_BACKOFF_SECS: [u64; 3] = [2, 10, 30];
pub const INSTANCE_RECOVERY_STABLE_SECS: u64 = 5 * 60;

fn default_manual_recovery() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeLaunchSpec {
    pub instance_id: String,
    pub config: InstanceConfig,
    /// The persisted command is an exact launch snapshot. A later configuration
    /// edit must not make failure recovery start that stale command again.
    #[serde(default)]
    pub launch_config_stale: bool,
    pub engine_backend: String,
    pub command: Vec<String>,
    pub command_display: String,
    pub workload: String,
    #[serde(default)]
    pub working_directory: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeFailureKind {
    StartupFailure,
    UnexpectedExit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceRecoveryPhase {
    Failed,
    Waiting,
    Monitoring,
    Restoring,
    CrashLoop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFailure {
    pub kind: RuntimeFailureKind,
    pub message: String,
    pub exit_code: Option<i32>,
    pub occurred_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceRecoveryStatus {
    pub phase: InstanceRecoveryPhase,
    /// Number of automatic restart attempts already started for this incident.
    pub restart_attempts: u32,
    pub max_restart_attempts: u32,
    pub next_retry_at: Option<u64>,
    /// The first failure in the active incident is immutable across retries.
    pub origin_failure: RuntimeFailure,
    /// The most recent failure is kept separately for current diagnostics.
    pub last_failure: RuntimeFailure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeServiceStatus {
    pub protocol_version: u32,
    pub service_version: String,
    pub service_pid: u32,
    /// Feature markers let a freshly updated GUI detect an older daemon from
    /// the same package version without breaking the v1 wire protocol.
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub config_revision: u64,
    pub background_enabled: bool,
    pub registered_for_login: bool,
    #[serde(default)]
    pub last_error: Option<String>,
    pub proxy: ProxyStatus,
    pub running: HashMap<String, RunningInstance>,
    #[serde(default)]
    pub health: HashMap<String, String>,
    #[serde(default)]
    pub monitoring: HashMap<String, crate::commands::monitoring::MonitoringFrame>,
    #[serde(default)]
    pub performance: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub recovery: HashMap<String, InstanceRecoveryStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", content = "payload", rename_all = "snake_case")]
pub enum RuntimeCommand {
    Ping,
    Heartbeat {
        gui_pid: u32,
    },
    GetStatus,
    SyncConfig {
        revision: u64,
        proxy_config: ProxyConfig,
        instances: HashMap<String, InstanceConfig>,
    },
    /// Atomically synchronizes the GUI snapshot, verifies that every managed
    /// workload is owned by this daemon, and only then enables detached mode.
    PrepareBackgroundDetach {
        revision: u64,
        proxy_config: ProxyConfig,
        instances: HashMap<String, InstanceConfig>,
        expected_running: HashMap<String, RunningInstance>,
    },
    StartInstance {
        spec: Box<RuntimeLaunchSpec>,
        /// Older GUI clients only issued operator-triggered starts, so a
        /// missing field must retain that behavior across an in-place upgrade.
        #[serde(default = "default_manual_recovery")]
        manual_recovery: bool,
    },
    StopInstance {
        instance_id: String,
    },
    ClearLastError,
    StartProxy,
    StopProxy,
    SetBackgroundEnabled {
        enabled: bool,
    },
    Shutdown {
        stop_instances: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeRequest {
    pub protocol_version: u32,
    pub request_id: String,
    pub token: String,
    pub command: RuntimeCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", content = "payload", rename_all = "snake_case")]
pub enum RuntimeReply {
    Pong,
    Ack,
    Status(Box<RuntimeServiceStatus>),
    Instance(Box<RunningInstance>),
    ProxyStatus(ProxyStatus),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeResponse {
    pub protocol_version: u32,
    pub request_id: String,
    pub reply: Option<RuntimeReply>,
    pub error: Option<String>,
}

impl RuntimeResponse {
    pub fn success(request_id: String, reply: RuntimeReply) -> Self {
        Self {
            protocol_version: RUNTIME_PROTOCOL_VERSION,
            request_id,
            reply: Some(reply),
            error: None,
        }
    }

    pub fn failure(request_id: String, error: impl Into<String>) -> Self {
        Self {
            protocol_version: RUNTIME_PROTOCOL_VERSION,
            request_id,
            reply: None,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PersistedRuntimeState {
    pub schema_version: u32,
    pub config_revision: u64,
    pub background_enabled: bool,
    pub proxy_config: ProxyConfig,
    pub instances: HashMap<String, InstanceConfig>,
    pub desired_instances: HashMap<String, RuntimeLaunchSpec>,
    pub running: HashMap<String, RunningInstance>,
    pub recovery: HashMap<String, InstanceRecoveryStatus>,
}

impl Default for PersistedRuntimeState {
    fn default() -> Self {
        Self {
            schema_version: RUNTIME_STATE_SCHEMA_VERSION,
            config_revision: 0,
            background_enabled: false,
            proxy_config: ProxyConfig::default(),
            instances: HashMap::new(),
            desired_instances: HashMap::new(),
            running: HashMap::new(),
            recovery: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_round_trip_preserves_tagged_command() {
        let request = RuntimeRequest {
            protocol_version: RUNTIME_PROTOCOL_VERSION,
            request_id: "request-1".into(),
            token: "secret".into(),
            command: RuntimeCommand::SetBackgroundEnabled { enabled: true },
        };
        let json = serde_json::to_string(&request).unwrap();
        let decoded: RuntimeRequest = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            decoded.command,
            RuntimeCommand::SetBackgroundEnabled { enabled: true }
        ));
    }

    #[test]
    fn error_acknowledgement_command_uses_a_payload_free_wire_shape() {
        let request = RuntimeRequest {
            protocol_version: RUNTIME_PROTOCOL_VERSION,
            request_id: "request-2".into(),
            token: "secret".into(),
            command: RuntimeCommand::ClearLastError,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["command"]["command"], "clear_last_error");
        assert!(json["command"].get("payload").is_none());

        let decoded: RuntimeRequest = serde_json::from_value(json).unwrap();
        assert!(matches!(decoded.command, RuntimeCommand::ClearLastError));
    }

    #[test]
    fn legacy_start_command_defaults_to_operator_recovery() {
        let spec = RuntimeLaunchSpec {
            instance_id: "instance-1".into(),
            config: InstanceConfig::default(),
            launch_config_stale: false,
            engine_backend: "test".into(),
            command: vec!["llama-server".into()],
            command_display: "llama-server".into(),
            workload: "inference".into(),
            working_directory: None,
        };
        let request = serde_json::json!({
            "protocol_version": RUNTIME_PROTOCOL_VERSION,
            "request_id": "legacy-start",
            "token": "secret",
            "command": {
                "command": "start_instance",
                "payload": { "spec": spec }
            }
        });
        let decoded: RuntimeRequest = serde_json::from_value(request).unwrap();
        assert!(matches!(
            decoded.command,
            RuntimeCommand::StartInstance {
                manual_recovery: true,
                ..
            }
        ));
    }

    #[test]
    fn persisted_state_accepts_missing_future_fields() {
        let state: PersistedRuntimeState = serde_json::from_str("{}").unwrap();
        assert_eq!(state.schema_version, RUNTIME_STATE_SCHEMA_VERSION);
        assert!(!state.background_enabled);
        assert!(state.desired_instances.is_empty());
        assert!(state.recovery.is_empty());
    }

    #[test]
    fn status_accepts_payloads_from_the_first_runtime_schema() {
        let value = serde_json::json!({
            "protocol_version": RUNTIME_PROTOCOL_VERSION,
            "service_version": "2.9.30",
            "service_pid": 42,
            "background_enabled": false,
            "registered_for_login": false,
            "proxy": ProxyStatus {
                running: false,
                bound_addr: "127.0.0.1:11435".into(),
                active_routes: 0,
                healthy_routes: 0,
                unhealthy_routes: 0,
                in_flight_requests: 0,
                total_requests: 0,
                last_error: None,
            },
            "running": {},
        });
        let status: RuntimeServiceStatus = serde_json::from_value(value).unwrap();
        assert_eq!(status.config_revision, 0);
        assert!(status.capabilities.is_empty());
        assert!(status.last_error.is_none());
        assert!(status.health.is_empty());
        assert!(status.monitoring.is_empty());
        assert!(status.performance.is_empty());
        assert!(status.recovery.is_empty());
    }

    #[test]
    fn background_detach_command_round_trip_preserves_expected_workloads() {
        let expected = RunningInstance {
            instance_id: "instance-1".into(),
            pid: 42,
            port: 8080,
            host: "127.0.0.1".into(),
            start_time: 100,
            executable_path: "/tmp/llama-server".into(),
            telemetry_session_id: None,
            workload: "inference".into(),
            launch_config: None,
        };
        let request = RuntimeRequest {
            protocol_version: RUNTIME_PROTOCOL_VERSION,
            request_id: "detach-1".into(),
            token: "secret".into(),
            command: RuntimeCommand::PrepareBackgroundDetach {
                revision: 7,
                proxy_config: ProxyConfig::default(),
                instances: HashMap::new(),
                expected_running: [("instance-1".to_string(), expected)].into_iter().collect(),
            },
        };
        let decoded: RuntimeRequest =
            serde_json::from_str(&serde_json::to_string(&request).unwrap()).unwrap();
        match decoded.command {
            RuntimeCommand::PrepareBackgroundDetach {
                revision,
                expected_running,
                ..
            } => {
                assert_eq!(revision, 7);
                assert_eq!(expected_running["instance-1"].pid, 42);
            }
            _ => panic!("unexpected command variant"),
        }
    }
}
