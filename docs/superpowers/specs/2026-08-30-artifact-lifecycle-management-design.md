# Companion artifact lifecycle management

## Goal

Prevent long-running installations from accumulating files that no active instance, download, or application component can still use, without deleting operator-owned paths or weakening the existing path-containment checks.

## Ownership classes

| Class | Examples | Default policy |
| --- | --- | --- |
| Manager-owned | KV checkpoints, instance logs, download ledgers, atomic-write scratch files | Automatically reconcile and garbage-collect inside the private application data root |
| Manager-selected engine output | Managed slot paths and managed prompt or lookup-cache locations | Track size and lifecycle; delete only when the manager created and registered the path |
| Operator-owned | Custom arguments, arbitrary lookup caches, custom log files, model roots | Inventory and warn only; never delete automatically |
| Platform-owned | WebView2 caches, updater staging, Windows crash dumps | Report separately; clean only a narrowly identified cache or after explicit confirmation |

## Safety invariants

1. Every destructive path is derived from a durable ownership record or a fixed application-private root.
2. Canonical containment, symlink and Windows reparse-point checks run immediately before deletion.
3. Active writers hold a lease and are excluded from maintenance.
4. A failed cleanup preserves the ownership record so the operation can be retried.
5. User-selected external paths are never inferred from filename patterns alone.
6. Maintenance supports an inventory-only preview before user-confirmed platform cleanup.

## Lifecycle triggers

- Application/runtime-service startup: reconcile interrupted manager writes and stale metadata.
- Every 24 hours while the runtime service is alive: enforce age and size policies.
- Instance deletion: stop, persist or drain as configured, delete owned checkpoint/log artifacts, then remove configuration ownership.
- Download completion: move ownership from queue/inflight state into the durable completed-artifact ledger.
- Download cancellation or failed-task clearing: stop the writer, delete registered partial files and metadata, then remove task ownership.
- Successful update/relaunch: remove inactive updater staging for installed or older versions.
- Manual storage maintenance: present manager, engine, operator, and platform categories separately.

## Retention policy

- Checkpoints: the configured per-instance limit is a hard aggregate limit across fingerprints. The active generation is protected; historical fingerprints are ordinary LRU candidates. Corrupt final generations and interrupted pending generations are not restorable and are reclaimable.
- Fingerprint hashes: 256 entries, 90-day inactivity TTL.
- Completed downloads: retained in a separate ownership ledger until the final artifact is removed or reconciliation proves it no longer exists.
- Failed downloads: clearing the task also removes its registered `.part` and `.part.json`; retry keeps them.
- Instance logs: each active writer retains at most 32 MiB, compacting to an 8 MiB tail; inactive configured-instance logs expire after 30 days and orphan logs after 7 days. Instance deletion removes the exact private log before configuration ownership is released.
- Runtime-service diagnostics: compact above 4 MiB to a 1 MiB recent tail, including while the background-only runtime remains alive.
- Telemetry: 14-day row retention runs at startup and every 24 hours in the runtime service. Startup reconciles open sessions against live persisted process identities, and every prune attempts a truncating WAL checkpoint plus query-plan optimization.
- Quarantine and atomic scratch: age-limited and count-limited inside fixed application roots.

## Delivery batches

1. Checkpoint hard limits, deletion cascade, completed-download ownership, and partial-download cleanup.
2. Instance/runtime logs and background telemetry maintenance.
3. Sharded model artifact-set deletion, authorization revocation, and external-output inventory.
4. Updater, WebView2, crash-dump, quarantine, atomic scratch, and developer-temp maintenance surfaces.

Each batch must retain cold-start or no-op fallback behavior on maintenance failure, pass Rust and frontend regression suites, and use the protected pull-request workflow.
