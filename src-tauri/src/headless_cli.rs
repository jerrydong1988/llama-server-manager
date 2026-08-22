use crate::error::{AppError, AppResult};
use crate::models::{AppState, InstanceConfig, RunningInstance};
use crate::runtime_service::protocol::{InstanceRecoveryStatus, RuntimeServiceStatus};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

pub const CLI_SCHEMA_VERSION: u32 = 1;
pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_INTERNAL: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_NOT_FOUND: i32 = 3;
pub const EXIT_PRECONDITION: i32 = 4;
pub const EXIT_UNAVAILABLE: i32 = 5;
pub const EXIT_SECURITY: i32 = 6;

const HELP: &str = "Llama Server Manager headless CLI

Usage:
  lsm [--output text|json] [--data-dir PATH] <command>

Commands:
  status                         Show the authenticated local runtime status
  instance list                 List configured instances in stable ID order
  instance status <INSTANCE>    Show one instance without configuration secrets
  instance start <INSTANCE>     Start through the full deployment preflight
  instance stop <INSTANCE>      Stop an instance (idempotent when configured)
  proxy status                  Show the local routing proxy status
  proxy start                   Persist, synchronize, and start the proxy
  proxy stop                    Persist, synchronize, and stop the proxy
  version                       Show the CLI and contract versions
  help                          Show this help

Global options:
  --output, -o text|json         Output format (default: text)
  --data-dir PATH               Use an isolated absolute application data directory
  --help, -h                    Show help
  --version, -V                 Show version

Authentication:
  The CLI reads the per-user private runtime credential from the selected data
  directory. Tokens are never accepted on the command line or emitted in output.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliCommand {
    Help,
    Version,
    Status,
    InstanceList,
    InstanceStatus(String),
    InstanceStart(String),
    InstanceStop(String),
    ProxyStatus,
    ProxyStart,
    ProxyStop,
    #[cfg(debug_assertions)]
    TestSeedFixture,
}

impl CliCommand {
    fn contract_name(&self) -> &'static str {
        match self {
            Self::Help => "help",
            Self::Version => "version",
            Self::Status => "status",
            Self::InstanceList => "instance.list",
            Self::InstanceStatus(_) => "instance.status",
            Self::InstanceStart(_) => "instance.start",
            Self::InstanceStop(_) => "instance.stop",
            Self::ProxyStatus => "proxy.status",
            Self::ProxyStart => "proxy.start",
            Self::ProxyStop => "proxy.stop",
            #[cfg(debug_assertions)]
            Self::TestSeedFixture => "test.seed-fixture",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Invocation {
    output: OutputFormat,
    data_dir: Option<PathBuf>,
    command: CliCommand,
}

struct CommandResult {
    data: Value,
    text: String,
}

fn usage_error(message: impl Into<String>) -> AppError {
    AppError::new("CLI_USAGE", message, false)
}

fn parse_output(value: &str) -> AppResult<OutputFormat> {
    match value {
        "text" => Ok(OutputFormat::Text),
        "json" => Ok(OutputFormat::Json),
        _ => Err(usage_error("--output must be either text or json")),
    }
}

fn require_instance_id(value: Option<&String>, command: &str) -> AppResult<String> {
    let value = value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| usage_error(format!("{command} requires an instance ID")))?;
    Ok(value.to_string())
}

fn reject_extra(arguments: &[String], expected: usize, command: &str) -> AppResult<()> {
    if arguments.len() == expected {
        Ok(())
    } else {
        Err(usage_error(format!("unexpected argument for {command}")))
    }
}

fn parse_arguments(arguments: Vec<OsString>) -> AppResult<Invocation> {
    let arguments = arguments
        .into_iter()
        .map(|argument| {
            argument
                .into_string()
                .map_err(|_| usage_error("CLI arguments must be valid Unicode"))
        })
        .collect::<AppResult<Vec<_>>>()?;
    let mut output = OutputFormat::Text;
    let mut data_dir = None;
    let mut index = 0;
    while let Some(argument) = arguments.get(index) {
        match argument.as_str() {
            "--output" | "-o" => {
                let value = arguments
                    .get(index + 1)
                    .ok_or_else(|| usage_error("--output requires text or json"))?;
                output = parse_output(value)?;
                index += 2;
            }
            "--data-dir" => {
                let value = arguments
                    .get(index + 1)
                    .ok_or_else(|| usage_error("--data-dir requires an absolute path"))?;
                let path = PathBuf::from(value);
                if !path.is_absolute() {
                    return Err(usage_error("--data-dir must be an absolute path"));
                }
                data_dir = Some(path);
                index += 2;
            }
            value if value.starts_with("--output=") => {
                output = parse_output(&value["--output=".len()..])?;
                index += 1;
            }
            value if value.starts_with("--data-dir=") => {
                let path = PathBuf::from(&value["--data-dir=".len()..]);
                if !path.is_absolute() {
                    return Err(usage_error("--data-dir must be an absolute path"));
                }
                data_dir = Some(path);
                index += 1;
            }
            _ => break,
        }
    }

    let command_arguments = &arguments[index..];
    let command = match command_arguments.first().map(String::as_str) {
        None | Some("help" | "--help" | "-h") => {
            reject_extra(
                command_arguments,
                usize::from(!command_arguments.is_empty()),
                "help",
            )?;
            CliCommand::Help
        }
        Some("version" | "--version" | "-V") => {
            reject_extra(command_arguments, 1, "version")?;
            CliCommand::Version
        }
        Some("status") => {
            reject_extra(command_arguments, 1, "status")?;
            CliCommand::Status
        }
        Some("instance") => match command_arguments.get(1).map(String::as_str) {
            Some("list") => {
                reject_extra(command_arguments, 2, "instance list")?;
                CliCommand::InstanceList
            }
            Some("status") => {
                reject_extra(command_arguments, 3, "instance status")?;
                CliCommand::InstanceStatus(require_instance_id(
                    command_arguments.get(2),
                    "instance status",
                )?)
            }
            Some("start") => {
                reject_extra(command_arguments, 3, "instance start")?;
                CliCommand::InstanceStart(require_instance_id(
                    command_arguments.get(2),
                    "instance start",
                )?)
            }
            Some("stop") => {
                reject_extra(command_arguments, 3, "instance stop")?;
                CliCommand::InstanceStop(require_instance_id(
                    command_arguments.get(2),
                    "instance stop",
                )?)
            }
            _ => return Err(usage_error("expected instance list|status|start|stop")),
        },
        Some("proxy") => match command_arguments.get(1).map(String::as_str) {
            Some("status") => {
                reject_extra(command_arguments, 2, "proxy status")?;
                CliCommand::ProxyStatus
            }
            Some("start") => {
                reject_extra(command_arguments, 2, "proxy start")?;
                CliCommand::ProxyStart
            }
            Some("stop") => {
                reject_extra(command_arguments, 2, "proxy stop")?;
                CliCommand::ProxyStop
            }
            _ => return Err(usage_error("expected proxy status|start|stop")),
        },
        #[cfg(debug_assertions)]
        Some("__test-seed-fixture") => {
            reject_extra(command_arguments, 1, "__test-seed-fixture")?;
            CliCommand::TestSeedFixture
        }
        Some(command) => return Err(usage_error(format!("unknown command: {command}"))),
    };
    Ok(Invocation {
        output,
        data_dir,
        command,
    })
}

fn requested_output(arguments: &[OsString]) -> OutputFormat {
    for (index, argument) in arguments.iter().enumerate() {
        if argument == "--output=json" {
            return OutputFormat::Json;
        }
        if (argument == "--output" || argument == "-o")
            && arguments
                .get(index + 1)
                .is_some_and(|value| value == "json")
        {
            return OutputFormat::Json;
        }
    }
    OutputFormat::Text
}

fn initialize_state(data_dir: Option<PathBuf>) -> AppResult<AppState> {
    if let Some(data_dir) = data_dir {
        crate::utils::set_data_dir_override(data_dir)
            .map_err(|message| AppError::new("CLI_DATA_DIR_INVALID", message, false))?;
    }
    let config_dir = crate::utils::get_data_dir().join("configs");
    let config = crate::commands::config::read_config_from_disk(&config_dir);
    crate::security::initialize_path_authority()
        .map_err(|message| AppError::new("PATH_AUTHORITY_INITIALIZATION_FAILED", message, false))?;
    crate::security::validate_configured_roots(&config.engine_dirs, &config.model_dirs)
        .map_err(|message| AppError::new("PATH_AUTHORITY_REQUIRED", message, false))?;
    crate::commands::model_inventory::initialize_inventory_storage()
        .map_err(|message| AppError::new("INVENTORY_INITIALIZATION_FAILED", message, true))?;
    let models = crate::commands::model_inventory::list_cached_models()
        .map_err(|message| AppError::new("MODEL_INVENTORY_LOAD_FAILED", message, true))?;
    let engines = crate::commands::model_inventory::list_cached_engines()
        .map_err(|message| AppError::new("ENGINE_INVENTORY_LOAD_FAILED", message, true))?;
    Ok(AppState::from_global_config(
        config_dir,
        &config,
        models,
        engines,
        Vec::new(),
    ))
}

#[cfg(debug_assertions)]
fn modified_seconds(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(debug_assertions)]
fn seed_test_fixture(data_dir: Option<PathBuf>) -> AppResult<CommandResult> {
    use crate::commands::model_inventory::{InventoryEngineRecord, InventoryModelRecord};
    use crate::models::{
        EngineCapabilities, EngineInfo, EngineQualificationCheck, EngineQualificationReport,
        ModelCapabilities, ModelInfo, ProxyApiKey,
    };
    use std::collections::HashSet;

    let data_dir = data_dir
        .ok_or_else(|| usage_error("__test-seed-fixture requires an isolated --data-dir"))?;
    crate::utils::set_data_dir_override(data_dir.clone())
        .map_err(|message| AppError::new("CLI_DATA_DIR_INVALID", message, false))?;
    let engine_root = data_dir.join("engines").join("fixture-engine");
    let model_root = data_dir.join("models").join("fixture-model");
    std::fs::create_dir_all(&engine_root)?;
    std::fs::create_dir_all(&model_root)?;
    #[cfg(windows)]
    for managed_root in [data_dir.join("engines"), data_dir.join("models")] {
        let marker = managed_root.join(".fixture-permissions");
        std::fs::write(&marker, b"private fixture root")?;
        crate::persistence::enforce_private_file(&marker).map_err(AppError::from)?;
        std::fs::remove_file(marker)?;
    }

    let source_engine = std::env::current_exe().map_err(|error| {
        AppError::new(
            "CLI_TEST_FIXTURE",
            format!("unable to locate the debug CLI executable: {error}"),
            false,
        )
    })?;
    #[cfg(windows)]
    let engine_path = engine_root.join("llama-server.exe");
    #[cfg(unix)]
    let engine_path = engine_root.join("llama-server");
    std::fs::copy(&source_engine, &engine_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&engine_path)?.permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&engine_path, permissions)?;
    }
    let model_path = model_root.join("fixture.gguf");
    std::fs::write(&model_path, vec![b'f'; 128 * 1024])?;
    #[cfg(windows)]
    {
        crate::persistence::enforce_private_file(&engine_path).map_err(AppError::from)?;
        crate::persistence::enforce_private_file(&model_path).map_err(AppError::from)?;
    }
    // Match the real scanner's stable path identity. CI temporary directories can
    // be aliases (/var -> /private/var on macOS or 8.3 -> long paths on Windows).
    let engine_path = std::fs::canonicalize(&engine_path)?;
    let model_path = std::fs::canonicalize(&model_path)?;

    let engine_identity =
        crate::deployment_identity::artifact_identity_for_path("engine", &engine_path)
            .map_err(AppError::from)?;
    let model_identity =
        crate::deployment_identity::artifact_identity_for_path("model", &model_path)
            .map_err(AppError::from)?;
    let executable = engine_path.to_string_lossy().to_string();
    let mut qualification = EngineQualificationReport {
        status: "passed".into(),
        executable_fingerprint: crate::commands::engine_capabilities::executable_fingerprint(
            &executable,
        ),
        engine_artifact_id: engine_identity.artifact_id.clone(),
        engine_version: "fixture-1".into(),
        help_hash: "fixture-help".into(),
        model_id: "fixture-model".into(),
        model_artifact_id: model_identity.artifact_id.clone(),
        model_name: "fixture.gguf".into(),
        model_size: std::fs::metadata(&model_path)?.len(),
        started_at: Some(1),
        completed_at: Some(2),
        checks: ["version", "capabilities", "startup", "health", "inference"]
            .into_iter()
            .map(|name| EngineQualificationCheck {
                name: name.into(),
                status: "passed".into(),
                duration_ms: 1,
                detail: None,
            })
            .collect(),
        ..EngineQualificationReport::default()
    };
    crate::deployment_identity::seal_qualification_report(&mut qualification)
        .map_err(AppError::from)?;
    let engine = EngineInfo {
        id: executable.clone(),
        name: "CLI fixture engine".into(),
        dir: engine_root.to_string_lossy().to_string(),
        exe: executable.clone(),
        version: "fixture-1".into(),
        backend: "test".into(),
        custom_name: None,
        capabilities: EngineCapabilities {
            qualification,
            ..EngineCapabilities::default()
        },
        artifact_identity: engine_identity,
    };
    let model = ModelInfo {
        id: model_path.to_string_lossy().to_string(),
        name: "CLI fixture model".into(),
        path: model_path.to_string_lossy().to_string(),
        size: std::fs::metadata(&model_path)?.len(),
        architecture: Some("fixture".into()),
        context_length: Some(128),
        quant_type: None,
        has_mtp_head: false,
        capabilities: ModelCapabilities::default(),
        file_type: "GGUF".into(),
        is_shard: false,
        artifact_identity: model_identity,
    };
    crate::commands::model_inventory::initialize_inventory_storage().map_err(AppError::from)?;
    crate::commands::model_inventory::apply_model_scan(
        &[InventoryModelRecord::from_model(
            &model,
            model.path.clone(),
            model_root.to_string_lossy().to_string(),
            modified_seconds(&model_path),
        )],
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .map_err(AppError::from)?;
    crate::commands::model_inventory::apply_engine_scan(
        &[InventoryEngineRecord::from_engine(
            &engine,
            modified_seconds(&engine_path),
            engine_root.to_string_lossy().to_string(),
        )],
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .map_err(AppError::from)?;

    let fixture_listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|error| {
        AppError::new(
            "CLI_TEST_FIXTURE",
            format!("unable to reserve a fixture port: {error}"),
            false,
        )
    })?;
    let fixture_port = fixture_listener
        .local_addr()
        .map_err(|error| {
            AppError::new(
                "CLI_TEST_FIXTURE",
                format!("unable to read the fixture port: {error}"),
                false,
            )
        })?
        .port();
    drop(fixture_listener);
    let manual_command = format!(
        "\"{}\" __test-fixture-server --host 127.0.0.1 --port {fixture_port} --model \"{}\"",
        engine_path.display(),
        model_path.display()
    );
    let instance_id = "cli-fixture";
    let instance = InstanceConfig {
        id: instance_id.into(),
        name: "CLI fixture".into(),
        engine_id: executable,
        model_path: model.path.clone(),
        alias: "cli-fixture-model".into(),
        host: "127.0.0.1".into(),
        port: fixture_port,
        launch_mode: "manual".into(),
        manual_command,
        restart_policy: "never".into(),
        ..InstanceConfig::default()
    };
    let mut secret_fixture = instance.clone();
    secret_fixture.id = "secret-fixture".into();
    secret_fixture.name = "Secret redaction fixture".into();
    secret_fixture.api_key = "fixture-api-secret".into();
    let mut global = crate::commands::config::default_global_config();
    global.engine_dirs = vec![engine_root.to_string_lossy().to_string()];
    global.model_dirs = vec![model_root.to_string_lossy().to_string()];
    global.instances.insert(instance_id.into(), instance);
    global
        .instances
        .insert(secret_fixture.id.clone(), secret_fixture);
    global.instance_order.push(instance_id.into());
    global.instance_order.push("secret-fixture".into());
    let proxy_key = crate::commands::proxy::generate_proxy_api_key();
    global.proxy_config.api_keys = vec![ProxyApiKey {
        id: "cli-fixture-key".into(),
        name: "CLI fixture key".into(),
        key: proxy_key,
        enabled: true,
        scopes: vec!["inference".into(), "discovery".into()],
        ..ProxyApiKey::default()
    }];
    global.proxy_config = crate::commands::proxy::normalize_and_validate_proxy_config(
        global.proxy_config,
        &global.instances,
    )
    .map_err(AppError::from)?;
    crate::config_revision::ensure_current_config_revisions(&mut global).map_err(AppError::from)?;
    crate::commands::config::persist_global_config(&data_dir.join("configs"), &global)
        .map_err(AppError::from)?;

    Ok(CommandResult {
        data: json!({ "instanceId": instance_id }),
        text: instance_id.into(),
    })
}

fn recovery_value(recovery: Option<&InstanceRecoveryStatus>) -> Value {
    recovery.map_or(Value::Null, |recovery| {
        json!({
            "phase": recovery.phase,
            "restartAttempts": recovery.restart_attempts,
            "maxRestartAttempts": recovery.max_restart_attempts,
            "nextRetryAt": recovery.next_retry_at,
            "lastFailure": {
                "kind": recovery.last_failure.kind,
                "exitCode": recovery.last_failure.exit_code,
                "occurredAt": recovery.last_failure.occurred_at,
            }
        })
    })
}

fn instance_value(
    instance_id: &str,
    config: Option<&InstanceConfig>,
    running: Option<&RunningInstance>,
    status: &RuntimeServiceStatus,
) -> Value {
    let recovery = status.recovery.get(instance_id);
    let state = if running.is_some() {
        "running"
    } else if recovery.is_some() {
        "recovering"
    } else {
        "stopped"
    };
    json!({
        "id": instance_id,
        "name": config.map(|config| config.name.as_str()).unwrap_or(""),
        "alias": config.map(|config| config.alias.as_str()).unwrap_or(""),
        "state": state,
        "pid": running.map(|running| running.pid),
        "host": running.map(|running| running.host.as_str()).or_else(|| config.map(|config| config.host.as_str())),
        "port": running.map(|running| running.port).or_else(|| config.map(|config| config.port)),
        "health": status.health.get(instance_id),
        "workload": running.map(|running| running.workload.as_str()),
        "deploymentId": running.map(|running| running.deployment_id.as_str()).filter(|value| !value.is_empty()),
        "deploymentRevisionId": running.map(|running| running.deployment_revision_id.as_str()).filter(|value| !value.is_empty()),
        "recovery": recovery_value(recovery),
    })
}

fn proxy_value(status: &RuntimeServiceStatus) -> Value {
    json!({
        "running": status.proxy.running,
        "boundAddress": status.proxy.bound_addr,
        "activeRoutes": status.proxy.active_routes,
        "healthyRoutes": status.proxy.healthy_routes,
        "unhealthyRoutes": status.proxy.unhealthy_routes,
        "inFlightRequests": status.proxy.in_flight_requests,
        "totalRequests": status.proxy.total_requests,
        "lastError": status.proxy.last_error,
    })
}

fn instance_text(value: &Value) -> String {
    let id = value["id"].as_str().unwrap_or("");
    let name = value["name"].as_str().unwrap_or("");
    let state = value["state"].as_str().unwrap_or("unknown");
    let endpoint = match (value["host"].as_str(), value["port"].as_u64()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        _ => "-".to_string(),
    };
    let pid = value["pid"]
        .as_u64()
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "-".to_string());
    format!("{id}\t{state}\t{pid}\t{endpoint}\t{name}")
}

async fn runtime_status() -> AppResult<RuntimeServiceStatus> {
    crate::runtime_service::ensure_runtime_service()
        .await
        .map_err(|message| AppError::new("RUNTIME_UNAVAILABLE", message, true))
}

async fn acquire_headless_residency() -> AppResult<bool> {
    let status = runtime_status().await?;
    if status.background_enabled {
        return Ok(false);
    }
    crate::runtime_service::set_background_enabled(true)
        .await
        .map_err(|message| AppError::new("RUNTIME_RESIDENCY_FAILED", message, true))?;
    Ok(true)
}

async fn release_headless_residency_on_failure(acquired: bool) {
    if acquired {
        let _ = crate::runtime_service::set_background_enabled(false).await;
    }
}

async fn release_idle_headless_residency(state: &AppState) -> AppResult<()> {
    if state.proxy_config.lock().unwrap().runtime_service_enabled {
        return Ok(());
    }
    let status = crate::runtime_service::runtime_status()
        .await
        .map_err(AppError::from)?;
    if status.running.is_empty() && status.recovery.is_empty() && !status.proxy.running {
        crate::runtime_service::set_background_enabled(false)
            .await
            .map_err(|message| AppError::new("RUNTIME_RESIDENCY_FAILED", message, true))?;
    }
    Ok(())
}

async fn execute(invocation: &Invocation) -> AppResult<CommandResult> {
    match &invocation.command {
        CliCommand::Help => Ok(CommandResult {
            data: json!({ "help": HELP }),
            text: HELP.to_string(),
        }),
        CliCommand::Version => Ok(CommandResult {
            data: json!({
                "version": env!("CARGO_PKG_VERSION"),
                "schemaVersion": CLI_SCHEMA_VERSION,
            }),
            text: format!(
                "lsm {} (contract v{})",
                env!("CARGO_PKG_VERSION"),
                CLI_SCHEMA_VERSION
            ),
        }),
        #[cfg(debug_assertions)]
        CliCommand::TestSeedFixture => seed_test_fixture(invocation.data_dir.clone()),
        _ => {
            let state = initialize_state(invocation.data_dir.clone())?;
            execute_runtime_command(&state, &invocation.command).await
        }
    }
}

async fn execute_runtime_command(
    state: &AppState,
    command: &CliCommand,
) -> AppResult<CommandResult> {
    match command {
        CliCommand::Status => {
            let status = runtime_status().await?;
            let data = json!({
                "service": {
                    "version": status.service_version,
                    "pid": status.service_pid,
                    "protocolVersion": status.protocol_version,
                    "capabilities": status.capabilities,
                    "backgroundEnabled": status.background_enabled,
                    "registeredForLogin": status.registered_for_login,
                    "lastError": status.last_error,
                },
                "instances": {
                    "configured": state.instances.lock().unwrap().len(),
                    "running": status.running.len(),
                    "recovering": status.recovery.len(),
                },
                "proxy": proxy_value(&status),
            });
            let text = format!(
                "Runtime {} (PID {}, protocol {})\nInstances: {} configured, {} running, {} recovering\nProxy: {} ({})",
                data["service"]["version"].as_str().unwrap_or("unknown"),
                data["service"]["pid"].as_u64().unwrap_or(0),
                data["service"]["protocolVersion"].as_u64().unwrap_or(0),
                data["instances"]["configured"].as_u64().unwrap_or(0),
                data["instances"]["running"].as_u64().unwrap_or(0),
                data["instances"]["recovering"].as_u64().unwrap_or(0),
                if data["proxy"]["running"].as_bool().unwrap_or(false) { "running" } else { "stopped" },
                data["proxy"]["boundAddress"].as_str().unwrap_or("-")
            );
            Ok(CommandResult { data, text })
        }
        CliCommand::InstanceList => {
            let status = runtime_status().await?;
            let instances = state.instances.lock().unwrap();
            let mut ids = instances.keys().cloned().collect::<BTreeSet<_>>();
            ids.extend(status.running.keys().cloned());
            ids.extend(status.recovery.keys().cloned());
            let values = ids
                .iter()
                .map(|instance_id| {
                    instance_value(
                        instance_id,
                        instances.get(instance_id),
                        status.running.get(instance_id),
                        &status,
                    )
                })
                .collect::<Vec<_>>();
            let mut lines = vec!["ID\tSTATE\tPID\tENDPOINT\tNAME".to_string()];
            lines.extend(values.iter().map(instance_text));
            Ok(CommandResult {
                data: json!({ "instances": values }),
                text: lines.join("\n"),
            })
        }
        CliCommand::InstanceStatus(instance_id) => {
            let status = runtime_status().await?;
            let instances = state.instances.lock().unwrap();
            let config = instances.get(instance_id);
            let running = status.running.get(instance_id);
            if config.is_none() && running.is_none() && !status.recovery.contains_key(instance_id) {
                return Err(
                    AppError::new("INSTANCE_NOT_FOUND", "找不到指定的实例。", false)
                        .with_context("instanceId", instance_id),
                );
            }
            let data = instance_value(instance_id, config, running, &status);
            Ok(CommandResult {
                text: instance_text(&data),
                data,
            })
        }
        CliCommand::InstanceStart(instance_id) => {
            let acquired_residency = acquire_headless_residency().await?;
            let running = match crate::commands::server::start_configured_runtime_instance(
                state,
                instance_id,
            )
            .await
            {
                Ok(running) => running,
                Err(error) => {
                    release_headless_residency_on_failure(acquired_residency).await;
                    return Err(error);
                }
            };
            let status = crate::runtime_service::runtime_status()
                .await
                .map_err(AppError::from)?;
            let config = state.instances.lock().unwrap();
            let data = instance_value(
                instance_id,
                config.get(instance_id),
                Some(&running),
                &status,
            );
            Ok(CommandResult {
                text: format!("Started {}", instance_text(&data)),
                data,
            })
        }
        CliCommand::InstanceStop(instance_id) => {
            crate::commands::server::stop_configured_runtime_instance(state, instance_id).await?;
            release_idle_headless_residency(state).await?;
            Ok(CommandResult {
                data: json!({ "id": instance_id, "state": "stopped" }),
                text: format!("Stopped {instance_id}"),
            })
        }
        CliCommand::ProxyStatus => {
            let status = runtime_status().await?;
            let data = proxy_value(&status);
            Ok(CommandResult {
                text: format!(
                    "Proxy {} at {} ({} active routes)",
                    if status.proxy.running {
                        "running"
                    } else {
                        "stopped"
                    },
                    status.proxy.bound_addr,
                    status.proxy.active_routes
                ),
                data,
            })
        }
        CliCommand::ProxyStart | CliCommand::ProxyStop => {
            let enabled = matches!(command, CliCommand::ProxyStart);
            let acquired_residency = if enabled {
                acquire_headless_residency().await?
            } else {
                false
            };
            if let Err(error) = crate::commands::proxy::set_runtime_proxy_enabled(state, enabled)
                .await
                .map_err(AppError::from)
            {
                release_headless_residency_on_failure(acquired_residency).await;
                return Err(error);
            }
            if !enabled {
                release_idle_headless_residency(state).await?;
            }
            let status = crate::runtime_service::runtime_status()
                .await
                .map_err(AppError::from)?;
            let data = proxy_value(&status);
            Ok(CommandResult {
                text: format!(
                    "Proxy {} at {}",
                    if status.proxy.running {
                        "running"
                    } else {
                        "stopped"
                    },
                    status.proxy.bound_addr
                ),
                data,
            })
        }
        CliCommand::Help | CliCommand::Version => unreachable!("handled without runtime state"),
        #[cfg(debug_assertions)]
        CliCommand::TestSeedFixture => unreachable!("handled without runtime state"),
    }
}

fn exit_code(error: &AppError) -> i32 {
    let code = error.code.to_ascii_uppercase();
    let message = error.message.to_ascii_lowercase();
    if code == "CLI_USAGE" || code == "CLI_DATA_DIR_INVALID" {
        EXIT_USAGE
    } else if code.contains("NOT_FOUND") || code == "NOT_FOUND" {
        EXIT_NOT_FOUND
    } else if code.contains("UNAUTHORIZED")
        || code.contains("SECURITY")
        || message.contains("unauthorized")
        || message.contains("permission denied")
    {
        EXIT_SECURITY
    } else if code == "RUNTIME_UNAVAILABLE"
        || code == "NETWORK"
        || code == "TIMEOUT"
        || (error.retryable && (code.contains("IO") || code.contains("LOAD")))
    {
        EXIT_UNAVAILABLE
    } else if code.contains("CONFLICT")
        || code.contains("ALREADY")
        || code.contains("REQUIRED")
        || code.contains("MISMATCH")
        || code.contains("STALE")
        || code.contains("UNSAVED")
        || code.contains("INFEASIBLE")
        || code.contains("UNSUPPORTED")
        || code.contains("INVALID")
        || code == "VALIDATION"
    {
        EXIT_PRECONDITION
    } else {
        EXIT_INTERNAL
    }
}

fn write_bytes(stream: &mut dyn Write, value: &str) -> std::io::Result<()> {
    stream.write_all(value.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

fn render_success(invocation: &Invocation, result: CommandResult) -> String {
    match invocation.output {
        OutputFormat::Text => result.text,
        OutputFormat::Json => serde_json::to_string_pretty(&json!({
            "schemaVersion": CLI_SCHEMA_VERSION,
            "ok": true,
            "command": invocation.command.contract_name(),
            "data": result.data,
        }))
        .expect("CLI success envelopes contain serializable values"),
    }
}

fn render_error(command: &str, error: &AppError, output: OutputFormat) -> String {
    match output {
        OutputFormat::Text => format!("{}: {}", error.code, error.message),
        OutputFormat::Json => serde_json::to_string_pretty(&json!({
            "schemaVersion": CLI_SCHEMA_VERSION,
            "ok": false,
            "command": command,
            "error": error,
        }))
        .expect("CLI error envelopes contain serializable values"),
    }
}

pub fn run(arguments: impl IntoIterator<Item = OsString>) -> i32 {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    let requested_output = requested_output(&arguments);
    let invocation = match parse_arguments(arguments) {
        Ok(invocation) => invocation,
        Err(error) => {
            let output = requested_output;
            let rendered = render_error("parse", &error, output);
            let write_result = if output == OutputFormat::Json {
                write_bytes(&mut std::io::stdout().lock(), &rendered)
            } else {
                write_bytes(&mut std::io::stderr().lock(), &rendered)
            };
            if write_result
                .as_ref()
                .is_err_and(|io_error| io_error.kind() != std::io::ErrorKind::BrokenPipe)
            {
                return EXIT_INTERNAL;
            }
            return exit_code(&error);
        }
    };
    let output = invocation.output;
    let command_name = invocation.command.contract_name();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("lsm-cli")
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let app_error =
                AppError::new("CLI_RUNTIME_INITIALIZATION_FAILED", error.to_string(), true);
            let rendered = render_error(command_name, &app_error, output);
            let _ = write_bytes(&mut std::io::stderr().lock(), &rendered);
            return EXIT_INTERNAL;
        }
    };
    match runtime.block_on(execute(&invocation)) {
        Ok(result) => {
            let rendered = render_success(&invocation, result);
            match write_bytes(&mut std::io::stdout().lock(), &rendered) {
                Ok(()) => EXIT_SUCCESS,
                Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => EXIT_SUCCESS,
                Err(_) => EXIT_INTERNAL,
            }
        }
        Err(error) => {
            let rendered = render_error(command_name, &error, output);
            let write_result = if output == OutputFormat::Json {
                write_bytes(&mut std::io::stdout().lock(), &rendered)
            } else {
                write_bytes(&mut std::io::stderr().lock(), &rendered)
            };
            if write_result
                .as_ref()
                .is_err_and(|io_error| io_error.kind() != std::io::ErrorKind::BrokenPipe)
            {
                EXIT_INTERNAL
            } else {
                exit_code(&error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(arguments: &[&str]) -> Vec<OsString> {
        arguments.iter().map(OsString::from).collect()
    }

    #[test]
    fn parser_accepts_stable_command_shapes() {
        let parsed =
            parse_arguments(os(&["--output", "json", "instance", "status", "demo"])).unwrap();
        assert_eq!(parsed.output, OutputFormat::Json);
        assert_eq!(parsed.command, CliCommand::InstanceStatus("demo".into()));
    }

    #[test]
    fn parser_rejects_relative_data_directories_and_extra_arguments() {
        assert_eq!(
            parse_arguments(os(&["--data-dir", "relative", "status"]))
                .unwrap_err()
                .code,
            "CLI_USAGE"
        );
        assert_eq!(
            parse_arguments(os(&["proxy", "status", "extra"]))
                .unwrap_err()
                .code,
            "CLI_USAGE"
        );
    }

    #[test]
    fn exit_codes_are_stable_by_error_class() {
        assert_eq!(exit_code(&usage_error("bad")), EXIT_USAGE);
        assert_eq!(
            exit_code(&AppError::new("INSTANCE_NOT_FOUND", "missing", false)),
            EXIT_NOT_FOUND
        );
        assert_eq!(
            exit_code(&AppError::new(
                "INSTANCE_ALREADY_RUNNING",
                "conflict",
                false
            )),
            EXIT_PRECONDITION
        );
        assert_eq!(
            exit_code(&AppError::new("RUNTIME_UNAVAILABLE", "offline", true)),
            EXIT_UNAVAILABLE
        );
        assert_eq!(
            exit_code(&AppError::new(
                "CONFIGURED_ENGINE_UNAUTHORIZED",
                "denied",
                false
            )),
            EXIT_SECURITY
        );
    }

    #[test]
    fn json_envelopes_are_versioned_and_do_not_echo_secrets() {
        let invocation = Invocation {
            output: OutputFormat::Json,
            data_dir: None,
            command: CliCommand::Status,
        };
        let rendered = render_success(
            &invocation,
            CommandResult {
                data: json!({ "running": true }),
                text: String::new(),
            },
        );
        let value: Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(value["schemaVersion"], CLI_SCHEMA_VERSION);
        assert_eq!(value["command"], "status");
        assert!(value.get("token").is_none());
        assert!(!rendered.contains("apiKey"));
    }
}
