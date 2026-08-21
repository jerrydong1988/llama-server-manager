# Headless CLI

`lsm` is the automation-safe command-line companion to Llama Server Manager. It is bundled with the desktop application and operates on the same per-user configuration, runtime service, deployment revisions, engine qualifications, and routing state.

## Installation and discovery

The installer places `lsm` in the application's binary directory:

- Windows: `lsm.exe` is next to the installed application executable. MSI and NSIS do not modify `PATH`; add that directory to your user `PATH` or invoke the absolute path.
- macOS: `/Applications/LlamaServerManager.app/Contents/MacOS/lsm`.
- Linux DEB: `/usr/bin/lsm`.

Source builds create `src-tauri/target/debug/lsm` (or `lsm.exe` on Windows). `npm run tauri build` compiles the desktop application and its native Cargo companion binary together, then includes exactly one `lsm` in the package.

## Commands

```text
lsm [--output text|json] [--data-dir PATH] <command>

status
instance list
instance status <INSTANCE>
instance start <INSTANCE>
instance stop <INSTANCE>
proxy status
proxy start
proxy stop
version
help
```

`--data-dir` is intended for isolated testing or an explicitly separated manager profile and must be an absolute path. Omitting it uses the same platform data directory as the desktop application.

Lifecycle commands use the same persisted configuration and full launch preflight as the GUI. The CLI cannot create an unqualified engine, bypass a stale Deployment Revision, weaken public-bind authentication, or start an infeasible placement. Configure and qualify engines, models, instances, and routes in the GUI before automating their lifecycle.

Starting an instance or proxy acquires runtime residency so the workload survives the CLI process. Stopping the final workload releases CLI-acquired residency unless the existing independent-runtime setting is enabled. Login recovery remains controlled by that setting in the desktop application.

## Structured output contract

`--output json` emits exactly one UTF-8 JSON document to standard output for both successful commands and command errors. Contract version 1 has these envelopes:

```json
{
  "schemaVersion": 1,
  "ok": true,
  "command": "instance.status",
  "data": {}
}
```

```json
{
  "schemaVersion": 1,
  "ok": false,
  "command": "instance.status",
  "error": {
    "code": "INSTANCE_NOT_FOUND",
    "message": "...",
    "retryable": false,
    "context": { "instanceId": "missing" }
  }
}
```

Instance output is deliberately sanitized. It includes identity, lifecycle state, PID, endpoint, health, workload, deployment IDs, and recovery state, but excludes launch arguments, model paths, API keys, credential files, and the runtime control token. IDs are sorted for deterministic list output.

Text mode writes successes to standard output and errors to standard error. A downstream closed pipe is treated as a successful consumer stop.

## Exit codes

| Code | Meaning | Automation action |
|---:|---|---|
| 0 | Success | Continue. |
| 1 | Internal or unclassified failure | Preserve diagnostics and stop. |
| 2 | Invalid command or arguments | Correct the invocation. |
| 3 | Instance or resource not found | Refresh configuration or choose another ID. |
| 4 | Precondition, conflict, stale state, or validation failure | Resolve the reported gate before retrying. |
| 5 | Runtime, network, timeout, or retryable I/O unavailable | Retry with bounded backoff. |
| 6 | Authentication, permission, or security rejection | Correct authority; do not blindly retry. |

The numeric classes are stable within schema version 1. The machine-readable `error.code` is the more specific diagnostic.

## Authentication and concurrency

The CLI authenticates to the local runtime with the private per-user control token already stored in the selected application data directory. There is no token command-line option, and the token is never emitted. Operating-system user permissions remain the trust boundary.

GUI and CLI configuration mutations share a process mutex, a cross-process file lock, atomic primary/backup persistence, and runtime configuration acknowledgements. Concurrent commands serialize at that boundary instead of overwriting a newer `instances.json`. Lifecycle conflicts still fail explicitly; callers should not assume two simultaneous starts of the same instance can both succeed.

## Examples

PowerShell:

```powershell
$status = lsm --output json status | ConvertFrom-Json
if (-not $status.ok) { throw $status.error.message }

lsm --output json instance start my-instance
if ($LASTEXITCODE -ne 0) { throw "instance start failed: $LASTEXITCODE" }
```

POSIX shell:

```bash
lsm --output json instance list | jq -e '.ok and (.data.instances | type == "array")'
lsm --output json proxy start
```

## Validation

`npm run test:headless-cli` uses an isolated data directory and proves the real cross-process instance and proxy lifecycle, JSON envelope, credential redaction, deterministic ordering, error classes, idempotent stop, and authenticated shutdown. Required platform CI also verifies that the CLI is present in Windows, macOS, Linux x86_64, and Linux ARM64 packages.
