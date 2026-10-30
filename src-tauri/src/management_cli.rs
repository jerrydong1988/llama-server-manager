//! Console companion. No window is created by this binary; MCP stdout is protocol only.
#[path = "management/catalog.rs"]
mod catalog;
#[allow(dead_code)]
#[path = "management/transport.rs"]
mod transport;
#[allow(dead_code)] // Shared contract is compiled independently into host and client.
#[path = "management/wire.rs"]
mod wire;

use serde_json::{json, Value};
use std::{
    io::{BufRead, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

struct Options {
    data: PathBuf,
    app: PathBuf,
    words: Vec<String>,
    arguments: Value,
    wait: bool,
    timeout: u64,
    key: Option<String>,
    permissions: Vec<String>,
}

fn parse() -> Result<Options, String> {
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let app = current
        .parent()
        .ok_or("missing executable directory")?
        .join(if cfg!(windows) {
            "llama-server-manager.exe"
        } else {
            "llama-server-manager"
        });
    let mut result = Options {
        data: wire::default_data_dir(),
        app,
        words: Vec::new(),
        arguments: json!({}),
        wait: false,
        timeout: 300,
        key: None,
        permissions: vec!["read".into()],
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => {
                result.data = PathBuf::from(args.next().ok_or("--data-dir needs a path")?)
            }
            "--app" => {
                result.app = PathBuf::from(args.next().ok_or("--app needs an executable path")?)
            }
            "--args" => {
                let data = args.next().ok_or("--args needs JSON or @file.json")?;
                let data = if let Some(path) = data.strip_prefix('@') {
                    std::fs::read_to_string(path).map_err(|e| e.to_string())?
                } else {
                    data
                };
                result.arguments = serde_json::from_str(&data)
                    .map_err(|e| format!("invalid arguments JSON: {e}"))?;
            }
            "--idempotency-key" => {
                result.key = Some(args.next().ok_or("--idempotency-key needs a value")?)
            }
            "--wait" => result.wait = true,
            "--timeout" => {
                result.timeout = args
                    .next()
                    .ok_or("--timeout needs seconds")?
                    .parse()
                    .map_err(|_| "invalid timeout")?
            }
            "--json" => {}
            "--allow-write" => result.permissions.push("write".into()),
            "--allow-delete" => result.permissions.push("delete".into()),
            "--allow-cluster" => result.permissions.push("cluster".into()),
            "--help" | "-h" => result.words = vec!["help".into()],
            value if value.starts_with('-') => return Err(format!("unknown option: {value}")),
            _ => result.words.push(arg),
        }
    }
    if !result.data.is_absolute() {
        result.data = std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(result.data);
    }
    if !result.app.is_absolute() {
        result.app = std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(result.app);
    }
    if result.timeout == 0 || result.timeout > 86400 {
        return Err("timeout must be 1..86400 seconds".into());
    }
    Ok(result)
}

async fn request(
    options: &Options,
    operation: &str,
    arguments: Value,
    key: Option<String>,
) -> Result<wire::Response, String> {
    let token = transport::read_token(&options.data)?;
    let request = wire::Request {
        version: wire::VERSION,
        id: uuid::Uuid::new_v4().to_string(),
        token,
        operation: operation.into(),
        arguments,
        idempotency_key: key,
    };
    transport::send(&options.data, &options.app, &request).await
}

async fn start(options: &Options) -> Result<wire::Response, String> {
    if let Ok(response) = request(options, "service.status", json!({}), None).await {
        if response.ok {
            return Ok(response);
        }
    }
    if !options.app.is_file() {
        return Err(format!(
            "application executable not found: {}; use --app",
            options.app.display()
        ));
    }
    transport::ensure_private_dir(&options.data)?;
    let output = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(options.data.join("management/host.log"))
        .map_err(|e| e.to_string())?;
    let mut command = std::process::Command::new(&options.app);
    command
        .arg("--management-service")
        .arg("--management-data-dir")
        .arg(&options.data)
        .stdin(std::process::Stdio::null())
        .stdout(output.try_clone().map_err(|e| e.to_string())?)
        .stderr(output);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // The hidden GUI host outlives this CLI. Never let it inherit the CLI/MCP
        // pipe handles: Node/Python harnesses otherwise wait for stdout EOF until
        // the GUI exits, even though the console process already returned 0.
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{
            GetHandleInformation, SetHandleInformation, HANDLE_FLAG_INHERIT,
        };
        for handle in [
            std::io::stdin().as_raw_handle(),
            std::io::stdout().as_raw_handle(),
            std::io::stderr().as_raw_handle(),
        ] {
            let mut flags = 0;
            unsafe {
                if GetHandleInformation(handle, &mut flags) != 0
                    && flags & HANDLE_FLAG_INHERIT != 0
                    && SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) == 0
                {
                    return Err("cannot isolate management host standard handles".into());
                }
            }
        }
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Ok(response) = request(options, "service.status", json!({}), None).await {
            if response.ok {
                return Ok(response);
            }
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!("management host exited ({status}); inspect management/host.log; close another application using a different data directory"));
        }
        if Instant::now() >= deadline {
            return Err("management host did not become ready within 45 seconds; inspect management/host.log".into());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn wait_task(
    options: &Options,
    mut response: wire::Response,
) -> Result<wire::Response, String> {
    let Some(id) = response
        .data
        .as_ref()
        .and_then(|v| v["task_id"].as_str())
        .map(str::to_string)
    else {
        return Ok(response);
    };
    let deadline = Instant::now() + Duration::from_secs(options.timeout);
    loop {
        response = request(options, "tasks.get", json!({"id":id}), None).await?;
        let status = response
            .data
            .as_ref()
            .and_then(|v| v["status"].as_str())
            .unwrap_or("");
        match status {
            "succeeded" => return Ok(response),
            "failed" | "interrupted" | "cancelled" => {
                response.ok = false;
                response.error = response
                    .data
                    .as_ref()
                    .and_then(|v| serde_json::from_value(v["error"].clone()).ok());
                return Ok(response);
            }
            _ if !response.ok => return Ok(response),
            _ => {}
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "task {id} is still pending; query tasks.get before retrying"
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn mcp_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

async fn mcp_call(options: &Options, message: Value) -> Option<Value> {
    let id = message.get("id").cloned()?;
    let method = message["method"].as_str().unwrap_or("");
    if message["jsonrpc"] != "2.0" {
        return Some(mcp_error(id, -32600, "invalid JSON-RPC envelope"));
    }
    let result = match method {
        "initialize" => {
            let requested = message["params"]["protocolVersion"].as_str().unwrap_or("");
            let version =
                if ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"].contains(&requested) {
                    requested
                } else {
                    "2025-11-25"
                };
            json!({"protocolVersion":version,"capabilities":{"tools":{}},"serverInfo":{"name":"llama-server-manager","version":env!("CARGO_PKG_VERSION")},"instructions":"Connect-only: this MCP process never starts or restarts the management host. If unavailable, ask the user to open the desktop app or explicitly run lsm service start with the same --app and --data-dir. Most mutations return durable task_id values. Query tasks_get until completion. tasks_cancel, tasks_delete and tasks_clear return results immediately. Deleting task history also removes its idempotency keys; retrying the original operation can execute it again. Refresh revision before configuration changes. Readiness is distinct from process startup. Do not retry an interrupted task without inspecting resource state."})
        }
        "ping" => json!({}),
        "tools/list" => {
            json!({"tools":catalog::operations().iter().filter(|op|options.permissions.iter().any(|p|p==op.permission)).map(catalog::Operation::tool).collect::<Vec<_>>()})
        }
        "tools/call" => {
            let name = message["params"]["name"].as_str().unwrap_or("");
            let Some(operation) = catalog::operations()
                .into_iter()
                .find(|op| op.name.replace('.', "_") == name)
            else {
                return Some(mcp_error(id, -32602, "unknown tool"));
            };
            if !options
                .permissions
                .iter()
                .any(|permission| permission == operation.permission)
            {
                return Some(mcp_error(
                    id,
                    -32602,
                    "tool is not enabled by this MCP server's permissions",
                ));
            }
            let arguments = message["params"]
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if let Err(error) = operation.validate(&arguments) {
                return Some(mcp_error(id, -32602, &error.message));
            }
            match request(options, operation.name, arguments, None).await {
                Ok(response) => {
                    json!({"content":[{"type":"text","text":serde_json::to_string(&response).unwrap()}],"structuredContent":response,"isError":!response.ok})
                }
                Err(error) => {
                    let failure = json!({"ok":false,"error":{
                        "code":"MANAGEMENT_CONNECTION_FAILED",
                        "message":format!("Management request failed; no host was started or restarted. Open the desktop app or explicitly run lsm service start with the same --app and --data-dir. Details: {error}"),
                        "retryable":false
                    }});
                    json!({"content":[{"type":"text","text":failure.to_string()}],"structuredContent":failure,"isError":true})
                }
            }
        }
        _ => return Some(mcp_error(id, -32601, "method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

async fn mcp(options: &Options) -> Result<(), String> {
    // Connecting a harness must not undo an explicit desktop/service exit.
    // Discovery is local; tools attach to a host started by the user and never spawn one.
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    loop {
        use std::io::Read;
        let mut bytes = Vec::new();
        let count = (&mut input)
            .take(wire::MAX_FRAME as u64 + 1)
            .read_until(b'\n', &mut bytes)
            .map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        if bytes.len() > wire::MAX_FRAME {
            return Err("MCP message exceeds maximum frame size".into());
        }
        let response = match serde_json::from_slice(&bytes) {
            Ok(message) => mcp_call(options, message).await,
            Err(_) => Some(mcp_error(Value::Null, -32700, "invalid JSON")),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut output, &response).map_err(|e| e.to_string())?;
            output.write_all(b"\n").map_err(|e| e.to_string())?;
            output.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

async fn run(options: Options) -> Result<bool, String> {
    if options.words.is_empty() || options.words[0] == "help" {
        println!("lsm <group> <operation> [--args JSON|@file] [--wait] [--json]\nlsm service start [--app PATH] [--data-dir PATH]\nlsm schema [group.operation]\nlsm mcp [--allow-write] [--allow-delete] [--allow-cluster]\nGlobal options: --app PATH --data-dir PATH --timeout SECONDS --idempotency-key KEY\nAll operation output is JSON. Schema lists exact fields. Most mutations return task_id; tasks cancel/delete/clear return immediate results.\nMCP defaults to read-only and only connects to an already running host. Open the desktop app or explicitly run lsm service start first; MCP never starts or restarts it. Directory authorization is local CLI only.");
        return Ok(true);
    }
    if options.words[0] == "schema" {
        let operations=catalog::operations().into_iter().filter(|op|options.words.get(1).map_or(true, |name|name==op.name)).map(|op|json!({"operation":op.name,"description":op.description,"permission":op.permission,"schema":op.schema()})).collect::<Vec<_>>();
        if operations.is_empty() {
            return Err("unknown operation".into());
        }
        println!("{}", serde_json::to_string_pretty(&operations).unwrap());
        return Ok(true);
    }
    if options.words[0] == "mcp" {
        mcp(&options).await?;
        return Ok(true);
    }
    let operation = options.words.join(".");
    let mut response = if operation == "service.start" {
        start(&options).await?
    } else {
        let op = catalog::find(&operation)
            .ok_or_else(|| format!("unknown operation {operation}; run lsm schema"))?;
        op.validate(&options.arguments).map_err(|e| e.message)?;
        request(
            &options,
            &operation,
            options.arguments.clone(),
            options.key.clone(),
        )
        .await?
    };
    if options.wait && response.ok {
        response = wait_task(&options, response).await?;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&response).map_err(|e| e.to_string())?
    );
    Ok(response.ok)
}

fn main() {
    let result = parse().and_then(|options| {
        tokio::runtime::Runtime::new()
            .map_err(|e| e.to_string())
            .and_then(|runtime| runtime.block_on(run(options)))
    });
    match result {
        Ok(true) => {}
        Ok(false) => std::process::exit(2),
        Err(error) => {
            eprintln!(
                "{}",
                json!({"ok":false,"error":{"code":"CLIENT_ERROR","message":error}})
            );
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> Options {
        Options {
            data: PathBuf::new(),
            app: PathBuf::new(),
            words: vec![],
            arguments: json!({}),
            wait: false,
            timeout: 1,
            key: None,
            permissions: vec!["read".into()],
        }
    }
    #[tokio::test]
    async fn mcp_denies_mutations_without_contacting_host() {
        let response=mcp_call(&options(),json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"engines_delete","arguments":{"id":"engine"}}})).await.unwrap();
        assert_eq!(response["error"]["code"], -32602);
        let listed = mcp_call(
            &options(),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        )
        .await
        .unwrap();
        assert!(listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["annotations"]["readOnlyHint"] == true));
        assert!(mcp_call(
            &options(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .await
        .is_none());
    }
}
