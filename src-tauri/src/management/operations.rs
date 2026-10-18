use super::wire::Failure;
use crate::{
    commands::{
        cluster, cluster_ssh, config, download, engine_capabilities, proxy, scanner, server,
    },
    models::{AppState, EngineInfo, GlobalConfig, InstanceConfig},
};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::Path;
use tauri::Manager;

pub fn value<T: Serialize>(value: T) -> Result<Value, Failure> {
    serde_json::to_value(value).map_err(|e| Failure::new("SERIALIZATION", e.to_string()))
}
fn text(args: &Value, key: &str) -> String {
    args[key].as_str().unwrap_or("").to_string()
}
fn optional(args: &Value, key: &str) -> Option<String> {
    args[key].as_str().map(str::to_string)
}
fn port(args: &Value, key: &str) -> Result<u16, Failure> {
    args[key]
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| Failure::new("VALIDATION", format!("{key} must be 1..65535")))
}
fn global(state: &AppState) -> GlobalConfig {
    let directory = state.config_dir.lock().unwrap().clone();
    config::read_config_from_disk(&directory)
}
fn revision(state: &AppState) -> String {
    config::management_config_revision(&global(state))
}
fn identity(id: &str) -> Result<(), Failure> {
    if id.is_empty()
        || id.len() > 100
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(Failure::new(
            "VALIDATION",
            "id must contain 1..100 ASCII letters, numbers, hyphens or underscores",
        ));
    }
    Ok(())
}

fn patch(target: &mut Value, update: &Value) -> Result<(), Failure> {
    let updates = update
        .as_object()
        .ok_or_else(|| Failure::new("VALIDATION", "patch must be an object"))?;
    let object = target
        .as_object_mut()
        .ok_or_else(|| Failure::new("VALIDATION", "target must be an object"))?;
    for (key, next) in updates {
        if !object.contains_key(key) {
            return Err(Failure::new(
                "VALIDATION",
                format!("unknown configuration field: {key}"),
            ));
        }
        if next == "[REDACTED]" {
            return Err(Failure::new(
                "VALIDATION",
                "omit redacted fields from configuration patches",
            ));
        }
        let current = object.get_mut(key).expect("checked field");
        if current.is_object() && next.is_object() {
            patch(current, next)?;
        } else {
            *current = next.clone();
        }
    }
    Ok(())
}

fn reject_dropped_fields(input: &Value, output: &Value) -> Result<(), Failure> {
    match (input, output) {
        (Value::Object(input), Value::Object(output)) => {
            for (key, value) in input {
                let next = output.get(key).ok_or_else(|| {
                    Failure::new(
                        "VALIDATION",
                        format!("unknown nested configuration field: {key}"),
                    )
                })?;
                reject_dropped_fields(value, next)?;
            }
        }
        (Value::Array(input), Value::Array(output)) => {
            for (value, next) in input.iter().zip(output) {
                reject_dropped_fields(value, next)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn patch_instance(target: &mut Value, changes: &Value) -> Result<(), Failure> {
    const METADATA: &[&str] = &[
        "launch_mode",
        "manual_command",
        "explicit_overrides",
        "id",
        "name",
        "engine_id",
        "model_path",
        "host",
        "port",
        "auto_start",
        "kv_checkpoint",
    ];
    let defaults = value(InstanceConfig::default())?;
    let mut overrides: Vec<String> = match target["explicit_overrides"].as_array() {
        Some(fields) => fields
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        None => target
            .as_object()
            .ok_or_else(|| Failure::new("VALIDATION", "configuration must be an object"))?
            .iter()
            .filter(|(key, current)| {
                !METADATA.contains(&key.as_str()) && **current != defaults[*key]
            })
            .map(|(key, _)| key.clone())
            .collect(),
    };
    patch(target, changes)?;
    if let Some(explicit) = changes.get("explicit_overrides") {
        let fields = explicit.as_array().ok_or_else(|| {
            Failure::new(
                "VALIDATION",
                "explicit_overrides must be an array; use [] to inherit all engine parameters",
            )
        })?;
        overrides = fields
            .iter()
            .map(|field| {
                field
                    .as_str()
                    .filter(|name| defaults.get(*name).is_some() && !METADATA.contains(name))
                    .map(str::to_string)
                    .ok_or_else(|| {
                        Failure::new(
                            "VALIDATION",
                            "explicit_overrides contains an unknown or non-parameter field",
                        )
                    })
            })
            .collect::<Result<_, _>>()?;
    } else {
        overrides.extend(
            changes
                .as_object()
                .expect("validated patch")
                .keys()
                .filter(|key| !METADATA.contains(&key.as_str()))
                .cloned(),
        );
    }
    overrides.sort();
    overrides.dedup();
    target["explicit_overrides"] = json!(overrides);
    Ok(())
}

fn preserve_router_keys(current: &Value, patch: &mut Value) -> Result<(), Failure> {
    if let Some(keys) = patch.get_mut("api_keys").and_then(Value::as_array_mut) {
        for next in keys {
            if !next.is_object() || next["id"].as_str().map_or(true, str::is_empty) {
                return Err(Failure::new(
                    "VALIDATION",
                    "API key entries require an object with a nonempty id",
                ));
            }
            let old = current["api_keys"]
                .as_array()
                .and_then(|keys| keys.iter().find(|key| key["id"] == next["id"]));
            if next.get("key").is_none() || next["key"] == "[REDACTED]" {
                if let Some(old) = old {
                    next["key"] = old["key"].clone();
                } else {
                    return Err(Failure::new(
                        "VALIDATION",
                        "a new API key entry must provide its key",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn instance(state: &AppState, id: &str) -> Result<InstanceConfig, Failure> {
    state
        .instances
        .lock()
        .unwrap()
        .get(id)
        .cloned()
        .ok_or_else(|| Failure::new("NOT_FOUND", "instance not found"))
}

fn engine(state: &AppState, configured: &str) -> Result<EngineInfo, Failure> {
    let configured = if configured.is_empty() {
        global(state).default_engine_id
    } else {
        configured.into()
    };
    let engines = state.engines.lock().unwrap();
    engines
        .iter()
        .find(|engine| {
            crate::path_utils::paths_equal(Path::new(&engine.id), Path::new(&configured))
        })
        .or_else(|| {
            if configured.is_empty() {
                engines.first()
            } else {
                None
            }
        })
        .cloned()
        .ok_or_else(|| {
            Failure::new(
                "NOT_FOUND",
                "configured engine is unavailable; refresh engines or select a valid engine",
            )
        })
}

fn references(state: &AppState, id: &str) -> Value {
    let global = global(state);
    let fallback = if global.default_engine_id.is_empty() {
        state
            .engines
            .lock()
            .unwrap()
            .first()
            .map(|engine| engine.id.clone())
            .unwrap_or_default()
    } else {
        global.default_engine_id.clone()
    };
    let instances = global
        .instances
        .iter()
        .filter(|(_, instance)| {
            let effective = if instance.engine_id.is_empty() {
                &fallback
            } else {
                &instance.engine_id
            };
            crate::path_utils::paths_equal(Path::new(effective), Path::new(id))
        })
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    let running = state
        .running
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, running)| {
            running.launch_config.as_ref().is_some_and(|config| {
                let selected = if config.engine_id.is_empty() {
                    &fallback
                } else {
                    &config.engine_id
                };
                crate::path_utils::paths_equal(Path::new(selected), Path::new(id))
            })
        })
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    json!({"instances":instances,"running":running,"default":crate::path_utils::paths_equal(Path::new(&global.default_engine_id),Path::new(id))})
}

async fn ensure_inventory(app: &tauri::AppHandle) -> Result<(), Failure> {
    let state = app.state::<AppState>();
    if state.engines.lock().unwrap().is_empty() {
        let paths = global(&state).engine_dirs;
        scanner::scan_engines(paths, state).await?;
    }
    Ok(())
}

pub async fn dispatch(app: &tauri::AppHandle, name: &str, args: &Value) -> Result<Value, Failure> {
    let state = app.state::<AppState>();
    let id = text(args, "id");
    match name {
        "service.status" => Ok(
            json!({"version":env!("CARGO_PKG_VERSION"),"protocol":super::wire::VERSION,"pid":std::process::id(),"data_directory":crate::utils::get_data_dir(),"revision":revision(&state),"runtime":crate::runtime_service::runtime_status().await.ok(),"desktop_session_required":cfg!(target_os="linux")}),
        ),
        "service.show" => {
            crate::ensure_main_window(app).map_err(|e| Failure::new("UI", e.to_string()))?;
            if let Some(window) = app.get_webview_window("main") {
                window
                    .show()
                    .map_err(|e| Failure::new("UI", e.to_string()))?;
                let _ = window.set_focus();
            }
            Ok(json!({"shown":true}))
        }
        "service.stop" => {
            proxy::shutdown_proxy_for_app(app).await?;
            crate::runtime_service::shutdown(true).await?;
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                crate::finalize_app_exit(&app, false);
            });
            Ok(json!({"shutdown_scheduled":true,"stop_workloads":true}))
        }
        "paths.list" => value(crate::security::list_authorized_directories().await),
        "paths.authorize" => {
            let purpose = text(args, "purpose");
            if !["engine", "model", "download"].contains(&purpose.as_str()) {
                return Err(Failure::new(
                    "VALIDATION",
                    "purpose must be engine, model or download",
                ));
            }
            crate::security::record_authorized_root(&purpose, Path::new(&text(args, "path")))?;
            value(crate::security::list_authorized_directories().await)
        }
        "paths.revoke" => value(
            crate::security::revoke_authorized_directory(text(args, "purpose"), text(args, "path"))
                .await?,
        ),
        "models.list" => value(scanner::get_models(state).await?),
        "models.refresh" => {
            value(scanner::scan_models(global(&state).model_dirs, state, app.clone()).await?)
        }
        "models.metadata" => value(scanner::read_gguf_metadata(text(args, "path")).await?),
        "models.delete_preview" => {
            value(scanner::preview_model_deletion(text(args, "path"), state).await?)
        }
        "models.delete" => value(scanner::delete_model_file(text(args, "path"), state).await?),
        "engines.list" => {
            let global = global(&state);
            Ok(
                json!({"engines":scanner::get_engines(state).await?,"default_engine_id":global.default_engine_id,"directories":global.engine_dirs,"revision":config::management_config_revision(&global)}),
            )
        }
        "engines.refresh" => value(scanner::scan_engines(global(&state).engine_dirs, state).await?),
        "engines.probe" => value(engine_capabilities::probe_engine_capabilities(id, state).await?),
        "engines.probe_all" => {
            let ids = state
                .engines
                .lock()
                .unwrap()
                .iter()
                .map(|engine| engine.id.clone())
                .collect::<Vec<_>>();
            let mut results = Vec::new();
            for id in ids {
                match engine_capabilities::probe_engine_capabilities(id.clone(), state.clone())
                    .await
                {
                    Ok(engine) => results.push(json!({"id":id,"ok":true,"engine":engine})),
                    Err(error) => results.push(json!({"id":id,"ok":false,"error":error})),
                }
            }
            Ok(json!({"engines":results}))
        }
        "engines.open_folder" => {
            let selected = engine(&state, &id)?;
            scanner::open_engine_folder(selected.dir).await?;
            Ok(json!({"opened":true}))
        }
        "engines.rename" => {
            scanner::rename_engine(id, text(args, "name"), state).await?;
            Ok(json!({"renamed":true}))
        }
        "engines.references" | "engines.delete_preview" => {
            let selected = engine(&state, &id)?;
            Ok(
                json!({"engine":selected,"references":references(&state,&id),"effect":"remove_inventory_entry","files_deleted":false,"note":"A later scan can rediscover this engine. Remove its scan directory to stop discovering it."}),
            )
        }
        "engines.delete" => {
            let refs = references(&state, &id);
            if refs["default"] == true
                || refs["instances"].as_array().is_some_and(|v| !v.is_empty())
                || refs["running"].as_array().is_some_and(|v| !v.is_empty())
            {
                return Err(Failure::new(
                    "CONFLICT",
                    "engine is default, referenced or running",
                ));
            }
            scanner::delete_engine(id, state).await?;
            Ok(json!({"removed":true,"files_deleted":false}))
        }
        "engines.default" => {
            let clear = args["clear"] == true;
            if clear != id.is_empty() {
                return Err(Failure::new(
                    "VALIDATION",
                    "provide id or clear=true, exclusively",
                ));
            }
            let selected = if clear {
                String::new()
            } else {
                engine(&state, &id)?.id
            };
            let global = config::management_update(&state, &text(args, "revision"), |global| {
                global.default_engine_id = selected;
                Ok(())
            })?;
            Ok(json!({"revision":config::management_config_revision(&global)}))
        }
        "directories.add" | "directories.remove" => {
            let purpose = text(args, "purpose");
            let path = text(args, "path");
            if !["engine", "model"].contains(&purpose.as_str()) {
                return Err(Failure::new(
                    "VALIDATION",
                    "purpose must be model or engine",
                ));
            }
            let path = if name.ends_with("add") {
                if purpose == "engine" {
                    crate::security::require_authorized_engine_root(Path::new(&path))?
                } else {
                    crate::security::require_authorized_model_root(Path::new(&path))?
                }
            } else {
                Path::new(&path).to_path_buf()
            };
            let updated = config::management_update(&state, &text(args, "revision"), |global| {
                let directories = if purpose == "engine" {
                    &mut global.engine_dirs
                } else {
                    &mut global.model_dirs
                };
                directories.retain(|old| !crate::path_utils::paths_equal(Path::new(old), &path));
                if name.ends_with("add") {
                    directories.push(path.to_string_lossy().into());
                }
                Ok(())
            })?;
            Ok(
                json!({"revision":config::management_config_revision(&updated),"files_deleted":false}),
            )
        }
        name if name.starts_with("downloads.") => downloads(app, name, args).await,
        name if name.starts_with("instances.") => instances(app, name, args).await,
        "cluster.list" => value(cluster::get_workers(state).await?),
        "cluster.scan" => value(cluster::scan_workers_tcp(state).await?),
        "cluster.test" => {
            value(cluster::test_worker(text(args, "host"), port(args, "port")?, state).await?)
        }
        "cluster.add" => {
            require_worker_trust(args)?;
            value(cluster::enroll_authorized_worker(
                text(args, "host"),
                port(args, "port")?,
                optional(args, "name").unwrap_or_else(|| text(args, "host")),
                None,
                &state,
            )?)
        }
        "cluster.approve" => {
            require_worker_trust(args)?;
            value(cluster::approve_authorized_worker(&id, &state)?)
        }
        "cluster.remove" => value(cluster::remove_worker(id, state).await?),
        "cluster.start_local" => {
            ensure_inventory(app).await?;
            let selected = engine(&state, &text(args, "engine_id"))?;
            let port = port(args, "port")?;
            let process = cluster::start_local_rpc(Some(selected.dir), port).await?;
            match cluster::enroll_authorized_worker(
                "127.0.0.1".into(),
                port,
                format!("Local-{port}"),
                Some(crate::models::WorkerOrigin::Local),
                &state,
            ) {
                Ok(worker) => Ok(json!({"process":process,"worker":worker})),
                Err(error) => {
                    cluster::stop_local_worker(port).await?;
                    Err(error.into())
                }
            }
        }
        "cluster.mdns_status" => {
            Ok(json!({"active":crate::commands::cluster_mdns::is_mdns_discovery_active().await}))
        }
        "cluster.mdns_start" => {
            value(crate::commands::cluster_mdns::start_mdns_discovery(app.clone()).await?)
        }
        "cluster.mdns_stop" => value(crate::commands::cluster_mdns::stop_mdns_discovery().await?),
        "cluster.stop" => value(cluster::stop_worker(id, state).await?),
        "cluster.ssh_start" => {
            let tunnel = cluster_ssh::ssh_launch_rpc(
                text(args, "host"),
                text(args, "user"),
                Some(text(args, "key_path")),
                port(args, "port")?,
                optional(args, "binary"),
                if args.get("ssh_port").is_some() {
                    Some(port(args, "ssh_port")?)
                } else {
                    None
                },
                optional(args, "os"),
            )
            .await?;
            let port = tunnel["port"]
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())
                .ok_or_else(|| Failure::new("PROTOCOL", "SSH tunnel did not return a port"))?;
            match cluster::enroll_authorized_worker(
                "127.0.0.1".into(),
                port,
                text(args, "host"),
                Some(crate::models::WorkerOrigin::Ssh),
                &state,
            ) {
                Ok(worker) => Ok(json!({"tunnel":tunnel,"worker":worker})),
                Err(error) => {
                    cluster_ssh::stop_ssh_tunnel(port).await?;
                    Err(error.into())
                }
            }
        }
        "cluster.ssh_stop" => value(cluster_ssh::stop_ssh_tunnel(port(args, "port")?).await?),
        "cluster.metrics" => value(cluster::get_cluster_metrics(state).await?),
        "routes.get" => {
            let config = proxy::get_proxy_config(state).await?;
            Ok(json!({"revision":proxy::config_revision(&config),"config":config}))
        }
        "routes.update" => {
            let mut current = value(proxy::get_proxy_config(state.clone()).await?)?;
            let mut changes = args["patch"].clone();
            preserve_router_keys(&current, &mut changes)?;
            patch(&mut current, &changes)?;
            let updated: crate::models::ProxyConfig = serde_json::from_value(current.clone())
                .map_err(|e| Failure::new("VALIDATION", e.to_string()))?;
            reject_dropped_fields(&changes, &value(&updated)?)?;
            let config =
                proxy::save_proxy_config(updated, state, Some(text(args, "revision"))).await?;
            Ok(json!({"revision":proxy::config_revision(&config),"config":config}))
        }
        "routes.status" => value(proxy::get_proxy_status(state).await?),
        "routes.targets" => value(proxy::list_proxy_targets(state).await?),
        "routes.test" => value(proxy::test_proxy_route(Some(text(args, "model")), state).await?),
        "routes.start" => value(proxy::start_proxy(app.clone()).await?),
        "routes.stop" => value(proxy::stop_proxy(state).await?),
        "routes.restart" => value(proxy::restart_proxy(state, app.clone()).await?),
        "system.health" => value(server::get_system_health().await?),
        _ => Err(Failure::new("NOT_FOUND", "operation has no implementation")),
    }
}

fn require_worker_trust(args: &Value) -> Result<(), Failure> {
    if args["trust"] != true {
        return Err(Failure::new("TRUST_REQUIRED", "set trust=true only for an RPC endpoint you control; RPC has no authenticated identity"));
    }
    Ok(())
}

async fn instances(app: &tauri::AppHandle, name: &str, args: &Value) -> Result<Value, Failure> {
    let state = app.state::<AppState>();
    let id = text(args, "id");
    match name {
        "instances.list" => {
            let instances = state.instances.lock().unwrap().clone();
            let runtime = crate::runtime_service::runtime_status().await.ok();
            Ok(json!({"instances":instances,"runtime":runtime,"revision":revision(&state)}))
        }
        "instances.get" => {
            Ok(json!({"id":id,"config":instance(&state,&id)?,"revision":revision(&state)}))
        }
        "instances.defaults" => {
            let defaults = value(InstanceConfig::default())?;
            Ok(
                json!({"fields":defaults.as_object().unwrap().keys().collect::<Vec<_>>(),"config":defaults,"revision":revision(&state)}),
            )
        }
        "instances.health" => {
            instance(&state, &id)?;
            let status = crate::runtime_service::runtime_status().await?;
            Ok(
                json!({"running":status.running.get(&id),"health":status.health.get(&id),"monitoring":status.monitoring.get(&id),"performance":status.performance.get(&id)}),
            )
        }
        "instances.logs" => {
            identity(&id)?;
            instance(&state, &id)?;
            let path = state
                .config_dir
                .lock()
                .unwrap()
                .join("logs")
                .join(format!("{id}.log"));
            let count = args["lines"].as_u64().unwrap_or(100).min(1000) as usize;
            use std::io::{Read, Seek, SeekFrom};
            let mut file =
                std::fs::File::open(path).map_err(|e| Failure::new("IO", e.to_string()))?;
            let size = file
                .metadata()
                .map_err(|e| Failure::new("IO", e.to_string()))?
                .len();
            file.seek(SeekFrom::Start(size.saturating_sub(256 * 1024)))
                .map_err(|e| Failure::new("IO", e.to_string()))?;
            let mut bytes = Vec::new();
            file.take(256 * 1024)
                .read_to_end(&mut bytes)
                .map_err(|e| Failure::new("IO", e.to_string()))?;
            let text = String::from_utf8_lossy(&bytes);
            let lines = text
                .lines()
                .rev()
                .take(count)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(server::redact_command_secrets)
                .collect::<Vec<_>>();
            Ok(json!({"lines":lines,"truncated":size>256*1024}))
        }
        "instances.stop" => {
            instance(&state, &id)?;
            server::stop_server(id, state, app.clone()).await?;
            Ok(json!({"stopped":true}))
        }
        "instances.delete" => {
            identity(&id)?;
            if state.running.lock().unwrap().contains_key(&id)
                || state.starting.lock().unwrap().contains(&id)
            {
                return Err(Failure::new(
                    "CONFLICT",
                    "stop the instance before deleting its configuration",
                ));
            }
            let updated = config::management_update(&state, &text(args, "revision"), |global| {
                if global.proxy_config.default_instance_id == id
                    || global
                        .proxy_config
                        .routes
                        .iter()
                        .any(|route| route.target_instance_id == id)
                {
                    return Err("instance is referenced by router configuration; remove its routes/default target first".into());
                }
                if global.instances.remove(&id).is_none() {
                    return Err("instance not found".into());
                }
                global.instance_order.retain(|other| other != &id);
                Ok(())
            })?;
            Ok(
                json!({"deleted":id,"revision":config::management_config_revision(&updated),"model_files_deleted":false}),
            )
        }
        "instances.create" | "instances.clone" | "instances.update" => {
            ensure_inventory(app).await?;
            let new_id = match name {
                "instances.create" => {
                    optional(args, "id").unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
                }
                "instances.clone" => {
                    optional(args, "new_id").unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
                }
                _ => id.clone(),
            };
            identity(&new_id)?;
            let mut edited = if name == "instances.create" {
                value(InstanceConfig::default())?
            } else {
                value(instance(&state, &id)?)?
            };
            patch_instance(
                &mut edited,
                if name == "instances.create" {
                    &args["config"]
                } else {
                    &args["patch"]
                },
            )?;
            let mut configured: InstanceConfig = serde_json::from_value(edited.clone())
                .map_err(|e| Failure::new("VALIDATION", e.to_string()))?;
            reject_dropped_fields(
                if name == "instances.create" {
                    &args["config"]
                } else {
                    &args["patch"]
                },
                &value(&configured)?,
            )?;
            configured.id = new_id.clone();
            let selected = engine(&state, &configured.engine_id)?;
            configured.engine_id = selected.id.clone();
            let preview =
                server::generate_server_command(configured.clone(), selected.exe, state.clone())
                    .await?;
            let updated = config::management_update(&state, &text(args, "revision"), |global| {
                if name != "instances.update" && global.instances.contains_key(&new_id) {
                    return Err("instance id already exists".into());
                }
                if name == "instances.update" && !global.instances.contains_key(&new_id) {
                    return Err("instance not found".into());
                }
                if global.instances.iter().any(|(other, config)| {
                    other != &new_id
                        && config.port == configured.port
                        && config.host == configured.host
                }) {
                    return Err("instance host/port conflicts with another configuration".into());
                }
                global.instances.insert(new_id.clone(), configured);
                if !global.instance_order.contains(&new_id) {
                    global.instance_order.push(new_id.clone());
                }
                Ok(())
            })?;
            Ok(
                json!({"id":new_id,"revision":config::management_config_revision(&updated),"validation":value(preview)?}),
            )
        }
        "instances.validate" | "instances.start" => {
            ensure_inventory(app).await?;
            let configured = if let Some(config) = args.get("config") {
                let mut defaults = value(InstanceConfig::default())?;
                patch_instance(&mut defaults, config)?;
                let configured: InstanceConfig = serde_json::from_value(defaults)
                    .map_err(|e| Failure::new("VALIDATION", e.to_string()))?;
                reject_dropped_fields(config, &value(&configured)?)?;
                configured
            } else {
                instance(&state, &id)?
            };
            let selected = engine(&state, &configured.engine_id)?;
            if name == "instances.validate" {
                value(server::generate_server_command(configured, selected.exe, state).await?)
            } else {
                server::start_server(
                    id.clone(),
                    configured,
                    selected.exe,
                    selected.backend,
                    state,
                    app.clone(),
                )
                .await?;
                Ok(json!({"id":id,"started":true,"health":"pending"}))
            }
        }
        _ => Err(Failure::new("NOT_FOUND", "unknown instance operation")),
    }
}

async fn browse(source: &str, repo: String) -> Result<Vec<crate::models::MsFileEntry>, Failure> {
    match source {
        "huggingface" => Ok(download::browse_huggingface(repo).await?),
        "modelscope" => Ok(download::browse_modelscope(repo).await?),
        _ => Err(Failure::new(
            "VALIDATION",
            "source must be huggingface or modelscope",
        )),
    }
}

async fn downloads(app: &tauri::AppHandle, name: &str, args: &Value) -> Result<Value, Failure> {
    let state = app.state::<AppState>();
    let id = text(args, "id");
    match name {
        "downloads.list" => value(download::get_download_manager_snapshot(state).await?),
        "downloads.browse" => value(browse(&text(args, "source"), text(args, "repo")).await?),
        "downloads.add" => {
            let source = text(args, "source");
            let repo = text(args, "repo");
            let available = browse(&source, repo.clone()).await?;
            let requested = args["files"].as_array().expect("validated");
            if requested.is_empty() {
                return Err(Failure::new("VALIDATION", "select at least one file"));
            }
            let mut files = Vec::new();
            for path in requested {
                let path = path.as_str().expect("validated");
                let mut file = available
                    .iter()
                    .find(|file| file.path == path || file.name == path)
                    .cloned()
                    .ok_or_else(|| {
                        Failure::new("NOT_FOUND", format!("repository file not found: {path}"))
                    })?;
                file.task_id = Some(uuid::Uuid::new_v4().to_string());
                // Control requests can arrive before the first progress event.
                file.run_id = Some(uuid::Uuid::new_v4().to_string());
                file.version = Some(0);
                files.push(file);
            }
            let id = uuid::Uuid::new_v4().to_string();
            let file_tasks = files
                .iter()
                .map(|file| json!({"file":file.path,"task_id":file.task_id}))
                .collect::<Vec<_>>();
            let entry = crate::models::PersistedQueueEntry {
                id: id.clone(),
                repo_id: repo,
                source,
                files,
                save_dir: text(args, "directory"),
                added_at: chrono::Utc::now().timestamp_millis() as u64,
                status: "queued".into(),
                retries: 0,
                max_retries: 3,
                last_error: None,
            };
            download::enqueue_download_queue(entry, state, app.clone()).await?;
            Ok(json!({"download_id":id,"files":file_tasks,"status":"queued"}))
        }
        "downloads.pause" | "downloads.cancel" => {
            let snapshot = download::get_download_manager_snapshot(state.clone()).await?;
            let entry = snapshot
                .queue
                .iter()
                .find(|entry| {
                    entry.id == id
                        || entry
                            .files
                            .iter()
                            .any(|file| file.task_id.as_ref() == Some(&id))
                })
                .ok_or_else(|| Failure::new("NOT_FOUND", "download task not found"))?;
            let file = entry
                .files
                .iter()
                .find(|file| file.name == text(args, "file") || file.path == text(args, "file"))
                .ok_or_else(|| Failure::new("NOT_FOUND", "download file not found"))?;
            let task_id = file.task_id.clone().unwrap_or(id);
            if name == "downloads.pause" {
                download::pause_file_download(task_id, file.run_id.clone(), state).await?;
            } else {
                download::cancel_and_cleanup_download(
                    task_id,
                    file.name.clone(),
                    file.run_id.clone(),
                    file.version,
                    state,
                    app.clone(),
                )
                .await?;
            }
            Ok(json!({"requested":true}))
        }
        "downloads.resume" => value(download::resume_download_task(id, app.clone()).await?),
        "downloads.retry" => {
            download::reset_download_for_redownload(id.clone(), state).await?;
            value(download::resume_download_task(id, app.clone()).await?)
        }
        "downloads.pause_all" => value(download::pause_all_downloads(state).await?),
        "downloads.resume_all" => value(download::resume_all_downloads(app.clone()).await?),
        "downloads.cancel_all" => value(download::cancel_all_downloads(state, app.clone()).await?),
        "downloads.clear" => {
            let selected = args["statuses"].as_array().expect("validated");
            if selected.is_empty() || selected.iter().any(|status| {
                !matches!(status.as_str(), Some("completed" | "error" | "cancelled"))
            }) {
                return Err(Failure::new("VALIDATION", "clear supports completed, error and cancelled records only; cancel active or paused tasks explicitly"));
            }
            let statuses = args["statuses"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_string())
                .collect();
            value(download::clear_download_tasks_by_status(statuses, Some(false), state).await?)
        }
        "downloads.settings" => {
            if optional(args, "resume_policy")
                .is_some_and(|policy| policy != "manual" && policy != "auto_on_launch")
            {
                return Err(Failure::new(
                    "VALIDATION",
                    "resume_policy must be manual or auto_on_launch",
                ));
            }
            if let Some(limit) = args["concurrency"].as_u64() {
                if !(1..=16).contains(&limit) {
                    return Err(Failure::new("VALIDATION", "concurrency must be 1..16"));
                }
            }
            if let Some(limit) = args["concurrency"].as_u64() {
                download::set_download_concurrency(limit as usize, state.clone(), app.clone())
                    .await?;
            }
            if let Some(limit) = args["bytes_per_second"].as_u64() {
                download::set_download_bandwidth_limit(limit, state.clone()).await?;
            }
            if let Some(enabled) = args["low_priority"].as_bool() {
                download::set_download_low_priority_throttle(enabled, app.clone()).await?;
            }
            if let Some(policy) = optional(args, "resume_policy") {
                download::set_download_resume_policy(policy, state.clone()).await?;
            }
            value(download::get_download_manager_snapshot(state).await?)
        }
        _ => Err(Failure::new("NOT_FOUND", "unknown download operation")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patches_do_not_silently_ignore_unknown_fields_or_redacted_secrets() {
        let mut target = json!({"name":"a","nested":{"port":8080},"api_key":"private"});
        assert!(patch(&mut target, &json!({"typo":true})).is_err());
        assert!(patch(&mut target, &json!({"api_key":"[REDACTED]"})).is_err());
        patch(&mut target, &json!({"nested":{"port":8081}})).unwrap();
        assert_eq!(target["nested"]["port"], 8081);
        assert_eq!(target["api_key"], "private");
        assert!(identity("../escape").is_err());
        assert!(identity("instance-123").is_ok());
    }

    #[test]
    fn router_key_roundtrip_preserves_secrets_and_rejects_malformed_entries() {
        let current = json!({"api_keys":[{"id":"one","key":"private","name":"old"}]});
        let mut changes = json!({"api_keys":[{"id":"one","key":"[REDACTED]","name":"new"}]});
        preserve_router_keys(&current, &mut changes).unwrap();
        assert_eq!(changes["api_keys"][0]["key"], "private");
        assert!(preserve_router_keys(&current, &mut json!({"api_keys":[null]})).is_err());
        assert!(preserve_router_keys(&current, &mut json!({"api_keys":[{"id":"new"}]})).is_err());
        assert!(reject_dropped_fields(
            &json!({"routes":[{"typo":true}]}),
            &json!({"routes":[{"id":"x"}]})
        )
        .is_err());
    }

    #[test]
    fn instance_patch_pins_even_default_values_and_respects_explicit_inheritance() {
        let mut configured = value(InstanceConfig::default()).unwrap();
        let temp = configured["temp"].clone();
        configured["explicit_overrides"] = json!([]);
        patch_instance(&mut configured, &json!({"temp":temp,"name":"Changed"})).unwrap();
        assert_eq!(configured["explicit_overrides"], json!(["temp"]));
        let parsed: InstanceConfig = serde_json::from_value(configured.clone()).unwrap();
        assert!(server::generate_command(&parsed, "llama-server")
            .iter()
            .any(|arg| arg == "--temp"));
        patch_instance(&mut configured, &json!({"explicit_overrides":[]})).unwrap();
        let parsed: InstanceConfig = serde_json::from_value(configured.clone()).unwrap();
        assert!(!server::generate_command(&parsed, "llama-server")
            .iter()
            .any(|arg| arg == "--temp"));
        assert!(patch_instance(&mut configured, &json!({"explicit_overrides":["typo"]})).is_err());
    }

    #[test]
    fn decimal_parameter_patches_match_defaults_and_launch_arguments() {
        let mut configured = value(InstanceConfig::default()).unwrap();
        assert_eq!(configured["temp"], 0.8);
        assert_eq!(configured["top_p"], 0.95);
        patch_instance(&mut configured, &json!({"name":"Legacy defaults"})).unwrap();
        assert_eq!(configured["explicit_overrides"], json!([]));
        patch_instance(&mut configured, &json!({"temp":0.12345679,"top_p":0.95})).unwrap();
        let parsed: InstanceConfig = serde_json::from_value(configured).unwrap();
        let command = server::generate_command(&parsed, "llama-server");
        for (flag, value) in [("--temp", "0.12345679"), ("--top-p", "0.95")] {
            assert!(command.windows(2).any(|args| args == [flag, value]));
        }
        let mut reloaded = value(parsed).unwrap();
        patch_instance(&mut reloaded, &json!({"explicit_overrides":[]})).unwrap();
        let inherited: InstanceConfig = serde_json::from_value(reloaded).unwrap();
        assert!(!server::generate_command(&inherited, "llama-server").iter().any(|arg| arg == "--temp"));
    }
}
