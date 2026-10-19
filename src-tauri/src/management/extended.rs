//! Non-cluster adapters. Domain validation and persistence remain shared with Tauri commands.
use super::{
    operations::{self, engine, instance, text, value},
    wire::Failure,
};
use crate::{
    commands::{
        checkpoint, download, monitoring, scanner, server, telemetry, usage_budgets,
        usage_performance, usage_requests, usage_store,
    },
    models::AppState,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::path::Path;
use tauri::Manager;

fn bounded(args: &Value, key: &str, max: u32) -> Result<Option<u32>, Failure> {
    args.get(key)
        .map(|v| {
            v.as_u64()
                .filter(|n| *n > 0 && *n <= u64::from(max))
                .map(|n| n as u32)
                .ok_or_else(|| Failure::new("VALIDATION", format!("{key} must be 1..{max}")))
        })
        .transpose()
}

fn timestamp(args: &Value, key: &str) -> Result<i64, Failure> {
    args[key].as_i64().filter(|n| *n >= 0).ok_or_else(|| {
        Failure::new(
            "VALIDATION",
            format!("{key} must be a nonnegative i64 timestamp"),
        )
    })
}

pub(super) fn usage_query<T: DeserializeOwned>(
    input: &Value,
    requests: bool,
) -> Result<T, Failure> {
    let object = input
        .as_object()
        .ok_or_else(|| Failure::new("VALIDATION", "query must be an object"))?;
    for key in object.keys() {
        let allowed = [
            "from",
            "to",
            "keyId",
            "model",
            "instanceId",
            "endpoint",
            "kind",
        ]
        .contains(&key.as_str())
            || requests
                && [
                    "outcome",
                    "failureCode",
                    "requestId",
                    "minDurationMs",
                    "minQueueMs",
                    "minFirstOutputMs",
                    "cursor",
                ]
                .contains(&key.as_str());
        if !allowed {
            return Err(Failure::new(
                "VALIDATION",
                format!("unknown query field: {key}"),
            ));
        }
    }
    if let Some(cursor) = input.get("cursor").filter(|v| !v.is_null()) {
        let cursor = cursor
            .as_object()
            .ok_or_else(|| Failure::new("VALIDATION", "cursor must be an object"))?;
        if cursor
            .keys()
            .any(|k| !["completedAt", "requestId", "snapshotRowId"].contains(&k.as_str()))
        {
            return Err(Failure::new("VALIDATION", "unknown cursor field"));
        }
    }
    serde_json::from_value(input.clone()).map_err(|e| Failure::new("VALIDATION", e.to_string()))
}

fn presets() -> Value {
    serde_json::from_str(include_str!("../../../src/configPresets.json")).expect("built-in presets")
}

fn csv_cell(value: &Value) -> String {
    let mut text = match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if text.starts_with(['=', '+', '-', '@', '\t', '\r', '\n']) {
        text.insert(0, '\'');
    }
    format!("\"{}\"", text.replace('"', "\"\""))
}

fn usage_csv(report: &Value, group: &str) -> Result<String, Failure> {
    if !["keys", "models", "instances", "days", "endpoints"].contains(&group) {
        return Err(Failure::new(
            "VALIDATION",
            "group must be keys, models, instances, days or endpoints",
        ));
    }
    let fields = [
        "requests",
        "forwarded",
        "success",
        "failed",
        "rejected",
        "cancelled",
        "incomplete",
        "input",
        "output",
        "cached",
        "inputKnown",
        "outputKnown",
        "complete",
        "partial",
        "unknown",
        "lastUsed",
    ];
    let mut rows = vec![format!("group,id,name,{}", fields.join(","))];
    for row in report[group].as_array().into_iter().flatten() {
        let mut cells = vec![
            csv_cell(&json!(group)),
            csv_cell(&row["id"]),
            csv_cell(&row["name"]),
        ];
        for field in fields {
            let known = match field {
                "input" => "inputKnown",
                "output" => "outputKnown",
                "cached" => "cacheKnown",
                _ => "",
            };
            cells.push(
                if !known.is_empty() && row["summary"][known].as_u64().unwrap_or(0) == 0 {
                    "\"\"".into()
                } else {
                    csv_cell(&row["summary"][field])
                },
            );
        }
        rows.push(cells.join(","));
    }
    Ok(format!("\u{feff}{}\r\n", rows.join("\r\n")))
}

async fn preset(app: &tauri::AppHandle, name: &str, args: &Value) -> Result<Value, Failure> {
    if name == "presets.list" {
        return Ok(
            json!({"presets":presets(),"kind":"built_in","application":"saved configuration only; no automatic restart"}),
        );
    }
    let state = app.state::<AppState>();
    let id = text(args, "id");
    let current = instance(&state, &id)?;
    let templates = presets();
    let changes = templates
        .get(text(args, "preset"))
        .ok_or_else(|| Failure::new("NOT_FOUND", "unknown built-in preset"))?;
    operations::ensure_inventory(app).await?;
    let selected = engine(&state, &current.engine_id)?;
    if current.launch_mode == "manual" || selected.capabilities.status != "detected" {
        return Err(Failure::new("VALIDATION", "presets require managed mode and fully detected engine capabilities; probe the engine first"));
    }
    let before = value(&current)?;
    let differences: Vec<Value> = changes
        .as_object()
        .expect("preset object")
        .iter()
        .filter(|(key, next)| before[*key] != **next)
        .map(|(key, next)| json!({"field":key,"before":before[key],"after":next}))
        .collect();
    if name == "presets.apply" {
        return Box::pin(operations::instances(
            app,
            "instances.update",
            &json!({"id":id,"revision":args["revision"],"patch":changes}),
        ))
        .await;
    }
    let mut next = before;
    operations::patch_instance(&mut next, changes)?;
    let config =
        serde_json::from_value(next).map_err(|e| Failure::new("VALIDATION", e.to_string()))?;
    let validation = server::generate_server_command(config, selected.exe, state).await?;
    Ok(
        json!({"preset":args["preset"],"differences":differences,"patch":changes,"validation":validation,"saved":false}),
    )
}

pub(super) async fn dispatch(
    app: &tauri::AppHandle,
    name: &str,
    args: &Value,
) -> Result<Value, Failure> {
    let state = app.state::<AppState>();
    let id = text(args, "id");
    match name {
        name if name.starts_with("presets.") => preset(app, name, args).await,
        "service.hide" => {
            crate::hide_window(app.clone());
            Ok(json!({"hidden":true}))
        }
        "runtime.status" => value(crate::runtime_service::runtime_status().await?),
        "runtime.clear_error" => {
            crate::runtime_service::clear_last_error().await?;
            Ok(json!({"cleared":true}))
        }
        "runtime.detach" => {
            let handoff = crate::prepare_background_handoff(app, args["enable"] == true).await?;
            let status = value(crate::runtime_service::runtime_status().await?)?;
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _handoff = handoff;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                crate::finalize_app_exit(&app, true);
            });
            Ok(json!({"shutdown_scheduled":true,"stop_workloads":false,"runtime":status}))
        }
        "runtime.stop" => {
            crate::stop_background_runtime(app.clone()).await?;
            Ok(json!({"stopped":true,"background_enabled":false}))
        }
        "models.open_folder" => {
            scanner::open_model_folder(text(args, "path")).await?;
            Ok(json!({"opened":true}))
        }
        "downloads.local_files" => {
            let paths: Vec<String> = serde_json::from_value(args["paths"].clone())
                .map_err(|e| Failure::new("VALIDATION", e.to_string()))?;
            for path in &paths {
                let parent = Path::new(path).parent().ok_or_else(|| {
                    Failure::new("VALIDATION", "path must have a parent directory")
                })?;
                crate::security::require_authorized_model_root(parent).or_else(|_| {
                    crate::security::resolve_authorized_download_root(
                        &crate::utils::get_data_dir(),
                        &parent.to_string_lossy(),
                    )
                })?;
                if Path::new(path).exists() {
                    crate::security::require_path_within_root(Path::new(path), parent)?;
                }
            }
            value(download::check_local_files(paths).await?)
        }
        "downloads.delete_file" => {
            download::delete_managed_local_file(id, state).await?;
            Ok(json!({"deleted":true}))
        }
        "instances.clear_log" => {
            instance(&state, &id)?;
            value(server::delete_instance_log(id, state).await?)
        }
        "instances.slots" => {
            instance(&state, &id)?;
            value(server::get_slots(id, state).await?)
        }
        "instances.metrics" => {
            instance(&state, &id)?;
            value(server::get_metrics(id, state).await?)
        }
        "instances.test_connection" => {
            instance(&state, &id)?;
            value(server::test_connection(id, state).await?)
        }
        "instances.resources" => {
            instance(&state, &id)?;
            value(server::get_system_metrics(id, state).await?)
        }
        "instances.open_browser" => {
            let config = instance(&state, &id)?;
            if !state.running.lock().unwrap().contains_key(&id) {
                return Err(Failure::new("CONFLICT", "instance is not running"));
            }
            server::open_browser(
                config.host,
                config.port,
                None,
                Some(config.api_prefix),
                Some(id),
                state,
            )
            .await?;
            Ok(json!({"opened":true}))
        }
        "checkpoints.list" => value(checkpoint::list_checkpoint_statuses(state).await?),
        "checkpoints.status" => {
            instance(&state, &id)?;
            value(checkpoint::get_checkpoint_status(id, state).await?)
        }
        "checkpoints.clear" => {
            instance(&state, &id)?;
            value(checkpoint::clear_checkpoint(id, state, app.clone()).await?)
        }
        "checkpoints.eligibility" => {
            operations::ensure_inventory(app).await?;
            let config = instance(&state, &id)?;
            let selected = engine(&state, &config.engine_id)?;
            value(server::get_checkpoint_eligibility(config, selected.exe, state).await?)
        }
        "usage.summary" => {
            value(usage_store::get_router_usage(usage_query(&args["query"], false)?).await?)
        }
        "usage.requests" => value(
            usage_requests::get_router_usage_requests(usage_query(&args["query"], true)?).await?,
        ),
        "usage.performance" => value(
            usage_performance::get_router_performance(usage_query(&args["query"], false)?).await?,
        ),
        "usage.budgets" => value(usage_budgets::get_router_budgets(state).await?),
        "usage.export" => {
            let group = args["group"].as_str().unwrap_or("keys");
            let report =
                value(usage_store::get_router_usage(usage_query(&args["query"], false)?).await?)?;
            Ok(
                json!({"content_type":"text/csv;charset=utf-8","content":usage_csv(&report,group)?,"group":group,"query":args["query"]}),
            )
        }
        "usage.clear" => {
            usage_store::clear_router_usage(timestamp(args, "from")?, timestamp(args, "to")?)
                .await?;
            Ok(json!({"cleared":true,"quotas_reset":false}))
        }
        "monitoring.series" => {
            if !id.is_empty() {
                instance(&state, &id)?;
            }
            value(monitoring::get_monitoring_series(
                (!id.is_empty()).then_some(id),
                args["range_ms"].as_u64(),
            ))
        }
        "telemetry.overview" => value(telemetry::get_telemetry_overview().await?),
        "telemetry.sessions" => {
            value(telemetry::list_telemetry_sessions(bounded(args, "limit", 200)?).await?)
        }
        "telemetry.samples" => value(
            telemetry::get_telemetry_session_samples(id, bounded(args, "limit", 1000)?).await?,
        ),
        "telemetry.detail" => value(
            telemetry::get_telemetry_session_detail(
                id,
                bounded(args, "sample_limit", 1000)?,
                bounded(args, "request_limit", 200)?,
            )
            .await?,
        ),
        "telemetry.analysis" => value(telemetry::get_telemetry_session_analysis(id).await?),
        "telemetry.diagnostics" => value(telemetry::get_telemetry_session_diagnostics(id).await?),
        "telemetry.requests" => {
            value(telemetry::list_inference_requests(id, bounded(args, "limit", 200)?).await?)
        }
        "telemetry.prune" => {
            value(telemetry::prune_telemetry(bounded(args, "retention_days", 365)?).await?)
        }
        "telemetry.optimize" => value(telemetry::optimize_telemetry_storage(state).await?),
        "storage.inventory" => {
            value(crate::storage_maintenance::get_storage_maintenance_inventory(state).await?)
        }
        "storage.external" => {
            value(crate::external_artifacts::get_external_artifact_inventory(state).await?)
        }
        "storage.cleanup" => value(
            crate::storage_maintenance::cleanup_storage_group(text(args, "group"), state).await?,
        ),
        "storage.webview_cleanup" => value(
            crate::storage_maintenance::schedule_webview_cache_cleanup(args["enabled"] == true)
                .await?,
        ),
        "system.port" => {
            let port = args["port"]
                .as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .filter(|p| *p > 0)
                .ok_or_else(|| Failure::new("VALIDATION", "port must be 1..65535"))?;
            value(server::check_port(port, args["host"].as_str().map(str::to_string)).await?)
        }
        "system.autostart" => value(crate::commands::autostart::is_autostart_enabled()?),
        "system.set_autostart" => {
            let enabled = args["enabled"] == true;
            if enabled != crate::commands::autostart::is_autostart_enabled()? {
                if enabled {
                    crate::commands::autostart::enable_autostart()?;
                } else {
                    crate::commands::autostart::disable_autostart()?;
                }
            }
            value(crate::commands::autostart::is_autostart_enabled()?)
        }
        _ => Err(Failure::new("NOT_FOUND", "operation has no implementation")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queries_reject_typos_and_overflow_instead_of_ignoring_filters() {
        assert!(usage_query::<usage_store::UsageQuery>(
            &json!({"from":0,"to":86400000,"keyid":"x"}),
            false
        )
        .is_err());
        assert!(usage_query::<usage_requests::RequestQuery>(&json!({"from":0,"to":86400000,"cursor":{"completedAt":1,"requestId":"x","snapshotRowId":2,"typo":1}}),true).is_err());
        assert!(bounded(&json!({"limit":4294967297u64}), "limit", 200).is_err());
        assert!(timestamp(&json!({"from":u64::MAX}), "from").is_err());
    }
    #[test]
    fn export_preserves_unavailable_counters_and_quotes_untrusted_labels() {
        let result = usage_csv(&json!({"keys":[{"id":"k","name":"=1+1,\"label\"","summary":{"input":0,"output":5,"inputKnown":0,"outputKnown":1}}]}),"keys").unwrap();
        assert!(result.contains("\"'=1+1,\"\"label\"\"\""));
        assert!(result.contains("\"\",\"5\",\"\""));
        assert!(usage_csv(&json!({}), "invalid").is_err());
    }
    #[test]
    fn builtin_presets_only_change_known_parameter_fields() {
        let defaults = value(crate::models::InstanceConfig::default()).unwrap();
        for changes in presets().as_object().unwrap().values() {
            let mut target = defaults.clone();
            operations::patch_instance(&mut target, changes).unwrap();
            let parsed: crate::models::InstanceConfig = serde_json::from_value(target).unwrap();
            assert_eq!(parsed.model_path, "");
            assert_eq!(parsed.api_key, "");
            for key in changes.as_object().unwrap().keys() {
                assert!(parsed.explicit_overrides.as_ref().unwrap().contains(key));
            }
        }
    }
}
