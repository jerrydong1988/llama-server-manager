use super::wire::Failure;
use serde_json::{json, Value};

#[derive(Clone)]
pub struct Operation {
    pub name: &'static str,
    pub description: &'static str,
    pub permission: &'static str,
    pub fields: &'static str,
}

impl Operation {
    pub fn schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for field in self.fields.split_whitespace() {
            let (name, kind) = field.split_once(':').expect("catalog field");
            let optional = name.ends_with('?');
            let name = name.trim_end_matches('?');
            let schema = if self.name == "tasks.clear" && name == "statuses" {
                json!({"type":"array","items":{"type":"string","enum":["succeeded","failed","cancelled","interrupted"]},"minItems":1,"maxItems":4,"uniqueItems":true})
            } else if name == "query" && self.name.starts_with("usage.") {
                usage_query_schema(self.name == "usage.requests")
            } else {
                match kind {
                    "strings" => json!({"type":"array","items":{"type":"string"},"maxItems":1000}),
                    "integer" => json!({"type":"integer","minimum":0}),
                    "string" => json!({"type":"string","minLength":1,"maxLength":32768}),
                    _ => json!({"type":kind}),
                }
            };
            properties.insert(name.into(), schema);
            if !optional {
                required.push(name);
            }
        }
        if self.permission != "read" && !matches!(self.name, "tasks.delete" | "tasks.clear") {
            properties.insert("idempotency_key".into(), json!({"type":"string","minLength":1,"maxLength":128,"description":"Stable key for retrying this exact operation without executing it twice"}));
        }
        json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
    }

    pub fn validate(&self, arguments: &Value) -> Result<(), Failure> {
        let object = arguments
            .as_object()
            .ok_or_else(|| Failure::new("VALIDATION", "arguments must be an object"))?;
        let schema = self.schema();
        let properties = schema["properties"].as_object().expect("properties");
        for name in schema["required"].as_array().expect("required") {
            if !object.contains_key(name.as_str().expect("name")) {
                return Err(Failure::new(
                    "VALIDATION",
                    format!("missing argument {name}"),
                ));
            }
        }
        for (key, value) in object {
            let field = properties
                .get(key)
                .ok_or_else(|| Failure::new("VALIDATION", format!("unknown argument {key}")))?;
            let valid = match field["type"].as_str().unwrap_or("") {
                "string" => value
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && s.len() <= 32768),
                "integer" => value.as_u64().is_some(),
                "boolean" => value.is_boolean(),
                "object" => value.is_object(),
                "array" => value
                    .as_array()
                    .is_some_and(|items| items.len() <= 1000 && items.iter().all(Value::is_string)),
                _ => false,
            };
            if !valid {
                return Err(Failure::new(
                    "VALIDATION",
                    format!("invalid type or range for {key}"),
                ));
            }
        }
        Ok(())
    }

    pub fn tool(&self) -> Value {
        json!({"name":self.name.replace('.', "_"),"description":self.description,"inputSchema":self.schema(),
            "annotations":{"readOnlyHint":self.permission == "read","destructiveHint":self.permission == "delete","openWorldHint":true},
            "outputSchema":{"type":"object","properties":{"version":{"type":"integer"},"id":{"type":"string"},"ok":{"type":"boolean"},"data":{},"error":{"type":"object"}},"required":["version","id","ok"]}})
    }
}

fn usage_query_schema(requests: bool) -> Value {
    let mut properties = serde_json::Map::new();
    for name in ["from", "to"] {
        properties.insert(name.into(), json!({"type":"integer","minimum":0,"maximum":i64::MAX,"description":"Unix time in milliseconds; summary/performance/export require UTC midnight boundaries"}));
    }
    for name in ["keyId", "model", "instanceId", "endpoint", "kind"] {
        properties.insert(name.into(), json!({"type":"string","maxLength":1024}));
    }
    if requests {
        for name in ["outcome", "failureCode", "requestId"] {
            properties.insert(name.into(), json!({"type":"string","maxLength":128}));
        }
        for name in ["minDurationMs", "minQueueMs", "minFirstOutputMs"] {
            properties.insert(
                name.into(),
                json!({"type":"integer","minimum":0,"maximum":u32::MAX}),
            );
        }
        properties.insert("cursor".into(),json!({"type":"object","properties":{"completedAt":{"type":"integer","minimum":0},"requestId":{"type":"string","maxLength":128},"snapshotRowId":{"type":"integer","minimum":0}},"required":["completedAt","requestId","snapshotRowId"],"additionalProperties":false}));
    }
    json!({"type":"object","properties":properties,"required":["from","to"],"additionalProperties":false})
}

pub fn operations() -> Vec<Operation> {
    let entries = [
        ("service.status","Management host and inference runtime status","read",""),
        ("service.capabilities","Discover supported management operations and schemas","read",""),
        ("service.show","Show the desktop window","write",""),
        ("service.hide","Hide the desktop window without stopping workloads","write",""),
        ("runtime.status","Read independent runtime status without enabling background mode","read",""),
        ("runtime.clear_error","Acknowledge the last runtime error","write",""),
        ("runtime.detach","Enable background/login recovery, acknowledge handoff, then exit the management host","write","enable:boolean"),
        ("runtime.stop","Disable background mode and stop the independent runtime and workloads","write",""),
        ("service.stop","Stop the management host and its managed workloads","admin",""),
        ("tasks.list","List recent management tasks","read",""),
        ("tasks.get","Get completion, result or failure of a management task","read","id:string"),
        ("tasks.cancel","Cancel a queued task, including network preparation; running operations use resource stop/pause","write","id:string"),
        ("tasks.delete","Delete one finished task record and its idempotency key; queued/running tasks are protected. Returns deletion counts immediately, without creating a task.","delete","id:string"),
        ("tasks.clear","Clear finished task history and its idempotency keys, optionally filtered by terminal statuses. Active tasks are retained. Returns deletion counts immediately, without creating a task.","delete","statuses?:strings"),
        ("paths.list","List explicitly authorized directories","read",""),
        ("paths.authorize","Authorize an existing directory; local owner CLI only","admin","purpose:string path:string"),
        ("paths.revoke","Revoke an explicitly authorized directory; local owner CLI only","admin","purpose:string path:string"),
        ("models.list","List discovered local models","read",""),
        ("models.refresh","Refresh configured model directories","write",""),
        ("models.metadata","Read GGUF model metadata","read","path:string"),
        ("models.open_folder","Open an authorized model folder","write","path:string"),
        ("models.delete_preview","Inspect model files that would be deleted","read","path:string"),
        ("models.delete","Delete a model using managed file ownership checks","delete","path:string"),
        ("engines.list","List engines, default engine and configured directories","read",""),
        ("engines.refresh","Refresh configured engine directories","write",""),
        ("engines.probe","Probe engine identity and supported llama.cpp options","write","id:string"),
        ("engines.probe_all","Probe all discovered engines and return per-engine outcomes","write",""),
        ("engines.open_folder","Open an authorized engine directory in the desktop file manager","write","id:string"),
        ("engines.rename","Rename an engine","write","id:string name:string"),
        ("engines.default","Select the default engine, or clear the explicit default with clear=true","write","id?:string clear?:boolean revision:string"),
        ("engines.references","Inspect instance and runtime references to an engine","read","id:string"),
        ("engines.delete_preview","Inspect removal blockers; engine files are preserved","read","id:string"),
        ("engines.delete","Remove an unreferenced engine from inventory, preserving files","delete","id:string"),
        ("directories.add","Add an already authorized model or engine directory","write","purpose:string path:string revision:string"),
        ("directories.remove","Remove a scan directory without deleting its files","write","purpose:string path:string revision:string"),
        ("downloads.browse","List downloadable files in a Hugging Face or ModelScope repository","read","source:string repo:string"),
        ("downloads.list","Read durable download queue and scheduling limits","read",""),
        ("downloads.local_files","Inspect sizes of files in authorized model/download directories","read","paths:strings"),
        ("downloads.delete_file","Delete a downloaded file by its durable task identity, respecting references","delete","id:string"),
        ("downloads.add","Queue selected repository files for download","write","source:string repo:string files:strings directory:string"),
        ("downloads.pause","Pause one file in a download task","write","id:string file:string"),
        ("downloads.resume","Resume a durable download task","write","id:string"),
        ("downloads.cancel","Cancel a file and clean only its owned partial output","delete","id:string file:string"),
        ("downloads.retry","Reset a failed download task for redownload","write","id:string"),
        ("downloads.pause_all","Pause all active downloads","write",""),
        ("downloads.resume_all","Resume paused downloads","write",""),
        ("downloads.cancel_all","Cancel all download tasks","delete",""),
        ("downloads.clear","Remove completed, error or cancelled task records; preserves downloaded files","delete","statuses:strings"),
        ("downloads.settings","Set download scheduling and recovery policy","write","concurrency?:integer bytes_per_second?:integer low_priority?:boolean resume_policy?:string"),
        ("instances.list","List configured instances and current runtime state","read",""),
        ("instances.get","Read one instance configuration and revision (secrets redacted)","read","id:string"),
        ("instances.defaults","Read default instance configuration and allowed fields","read",""),
        ("instances.create","Create an instance from validated configuration fields","write","id?:string config:object revision:string"),
        ("instances.clone","Clone an instance; caller must choose a distinct name/port","write","id:string new_id?:string patch:object revision:string"),
        ("instances.update","Patch one instance without replacing unrelated configuration","write","id:string patch:object revision:string"),
        ("instances.delete","Delete a stopped instance configuration, preserving model files","delete","id:string revision:string"),
        ("instances.validate","Validate instance settings and produce its launch command","read","id?:string config?:object"),
        ("instances.start","Start a configured instance; inspect health separately","write","id:string"),
        ("instances.stop","Stop an instance","write","id:string"),
        ("instances.health","Read current inference health and phase metrics","read","id:string"),
        ("instances.logs","Read a bounded tail of an instance log","read","id:string lines?:integer"),
        ("instances.clear_log","Delete the persisted log of a stopped instance","delete","id:string"),
        ("instances.slots","Read actual engine slot state","read","id:string"),
        ("instances.metrics","Read actual engine metrics; null means unavailable","read","id:string"),
        ("instances.test_connection","Test the running endpoint using its actual host, port, prefix and credentials","read","id:string"),
        ("instances.resources","Read process and hardware resource measurements for a running instance","read","id:string"),
        ("instances.open_browser","Open the running instance web interface","write","id:string"),
        ("checkpoints.list","List checkpoint states; absence does not mean a saved checkpoint exists","read",""),
        ("checkpoints.status","Read one instance checkpoint state","read","id:string"),
        ("checkpoints.eligibility","Assess checkpoint support for the configured instance and engine","read","id:string"),
        ("checkpoints.validate","Run isolated local checkpoint inference qualification without changing an instance","write","id:string"),
        ("checkpoints.clear","Clear a stopped instance checkpoint through the owning runtime","delete","id:string"),
        ("presets.list","List built-in configuration templates shared with the GUI","read",""),
        ("presets.preview","Preview template differences and validated launch command without saving","read","id:string preset:string"),
        ("presets.apply","Apply a built-in template to saved configuration; does not restart the instance","write","id:string preset:string revision:string"),
        ("cluster.list","List configured RPC workers","read",""),
        ("cluster.scan","Discover RPC workers on local networks","cluster",""),
        ("cluster.test","Test a worker address","read","host:string port:integer"),
        ("cluster.add","Register an RPC worker; trust=true explicitly trusts this unauthenticated endpoint","cluster","host:string port:integer name?:string trust:boolean"),
        ("cluster.approve","Approve a discovered RPC worker with explicit trust=true","cluster","id:string trust:boolean"),
        ("cluster.remove","Remove a worker registration","cluster","id:string"),
        ("cluster.start_local","Start and register a local RPC worker from an authorized engine","cluster","port:integer engine_id?:string"),
        ("cluster.mdns_status","Read continuous mDNS discovery state","read",""),
        ("cluster.mdns_start","Start continuous mDNS discovery; discovered nodes still require approval","cluster",""),
        ("cluster.mdns_stop","Stop continuous mDNS discovery","cluster",""),
        ("cluster.stop","Stop a managed worker","cluster","id:string"),
        ("cluster.ssh_start","Launch RPC through an SSH key and managed tunnel","cluster","host:string user:string key_path:string port:integer ssh_port?:integer binary?:string os?:string"),
        ("cluster.ssh_stop","Stop a managed SSH tunnel","cluster","port:integer"),
        ("cluster.metrics","Read cluster metrics","read",""),
        ("routes.get","Read router configuration (secrets redacted) and revision","read",""),
        ("routes.update","Patch router configuration while preserving unspecified secrets","write","patch:object revision:string"),
        ("routes.status","Read proxy listener status","read",""),
        ("routes.targets","List eligible route targets","read",""),
        ("routes.test","Preview route selection for a model","read","model:string"),
        ("routes.start","Start the API router","write",""),
        ("routes.stop","Stop the API router","write",""),
        ("routes.restart","Restart the API router","write",""),
        ("usage.summary","Read router usage for UTC-aligned day boundaries in milliseconds","read","query:object"),
        ("usage.requests","Read a cursor-paginated request ledger with millisecond time bounds","read","query:object"),
        ("usage.performance","Read route latency and throughput aggregates for UTC days","read","query:object"),
        ("usage.budgets","Read key budgets, enforced quotas and accounting health","read",""),
        ("usage.export","Return CSV summary content; does not write arbitrary local files","read","query:object group?:string"),
        ("usage.clear","Clear usage history in UTC day range; does not reset enforced quotas","delete","from:integer to:integer"),
        ("monitoring.series","Read recent monitoring frames including distinct Prefill and Decode phases","read","id?:string range_ms?:integer"),
        ("telemetry.overview","Read persistent telemetry storage and session overview","read",""),
        ("telemetry.sessions","List recent persisted inference sessions","read","limit?:integer"),
        ("telemetry.samples","Read bounded metric samples for a session","read","id:string limit?:integer"),
        ("telemetry.detail","Read session samples, requests, analysis and diagnostics","read","id:string sample_limit?:integer request_limit?:integer"),
        ("telemetry.analysis","Read session performance analysis","read","id:string"),
        ("telemetry.diagnostics","Read session diagnostic findings","read","id:string"),
        ("telemetry.requests","Read bounded inference request history","read","id:string limit?:integer"),
        ("telemetry.prune","Prune telemetry older than retention_days (1..365)","delete","retention_days:integer"),
        ("telemetry.optimize","Optimize telemetry storage while all instances are stopped","delete",""),
        ("storage.inventory","Inspect owned storage groups, cleanup blockers and external references","read",""),
        ("storage.external","Inspect external artifacts referenced by instance configuration","read",""),
        ("storage.cleanup","Clean one known disposable storage group; protected data stays protected","delete","group:string"),
        ("storage.webview_cleanup","Schedule or cancel WebView cache cleanup on next startup","delete","enabled:boolean"),
        ("system.port","Check whether a host/port is available for binding","read","port:integer host?:string"),
        ("system.autostart","Read desktop application login startup status","read",""),
        ("system.set_autostart","Set desktop login startup; local owner CLI only","admin","enabled:boolean"),
        ("system.health","Read local CPU, GPU and memory health","read",""),
    ];
    entries
        .into_iter()
        .map(|(name, description, permission, fields)| Operation {
            name,
            description,
            permission,
            fields,
        })
        .collect()
}

pub fn find(name: &str) -> Option<Operation> {
    operations().into_iter().find(|op| op.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_has_unique_names_and_rejects_unknown_fields() {
        let mut names = std::collections::HashSet::new();
        for operation in operations() {
            assert!(names.insert(operation.name));
            assert!(operation.validate(&json!({"unknown":true})).is_err());
            assert_eq!(operation.schema()["additionalProperties"], false);
        }
        let op = find("engines.rename").unwrap();
        assert!(op.validate(&json!({"id":"a","name":"New"})).is_ok());
        assert!(op.validate(&json!({"id":"a"})).is_err());
    }
}
