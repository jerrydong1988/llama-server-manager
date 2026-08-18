# Configuration Revisions and Rollback

Llama Server Manager keeps a bounded, immutable configuration history for each instance. The history is intended for operator diagnosis and manual recovery; it does not add automatic rollout or rollback policy.

## What creates a revision

A revision is created only when the normalized deployment configuration changes. This includes engine and model selection, launch parameters, network and authentication settings, **Auto Start**, and **Failure Recovery** policy. The instance ID and display name are presentation identity and do not create revisions by themselves.

Existing instances receive one migration baseline on first load. Creating an instance, saving a deployment-affecting change, or completing a rollback creates a new immutable event with:

- a unique revision ID;
- a SHA-256 configuration fingerprint;
- a parent revision link and timestamp;
- a creation reason; and
- the selected target for rollback-created revisions.

The active configuration and its revision are committed together through the existing atomic `instances.json` write. A failed write does not update the in-memory configuration shown by the application.

## Inspect history and diffs

Open **Configuration**, select an instance, and use **Configuration Revisions** in the right-hand panel. Expand a revision to inspect its field-level change summary.

Historical snapshots never cross the frontend boundary. API keys, credential-file paths, TLS material, manual commands, custom arguments, MCP server configuration, and embedded UI configuration are represented only as set/empty state or item counts. Long non-secret values are bounded. The unsaved-change review uses the same redaction policy.

The local `instances.json` and `instances.json.bak` files still contain the private configuration needed for an exact rollback. Protect both files as sensitive application data and do not attach them to public bug reports.

## Mark a known-good revision

Expand a valid revision and choose **Mark known good**. Exactly one revision per instance can hold this operator-controlled pointer. Moving the pointer is recorded in a bounded audit trail.

Known-good is a protected reference, not an automatic promotion or rollback policy. It does not start, stop, or change an instance. The retention policy keeps at most 50 revisions per instance while preserving the current and explicitly known-good revision.

## Roll back safely

Before rollback:

1. Stop the instance.
2. Cancel any active Failure Recovery incident.
3. Wait for any start, stop, save, or rollback action to finish.
4. Refresh revision history if another window or operation changed the configuration.

Expand the target revision, choose **Rollback**, review the confirmation, and confirm. A successful rollback:

- restores the target deployment configuration;
- preserves the current instance ID and display name;
- creates a new immutable revision instead of rewriting history;
- persists before the frontend is updated; and
- queues the background runtime configuration sync.

Rollback does not silently restart the instance. Review the restored configuration, then start it when ready.

The backend rejects unknown or pruned revision IDs, revisions belonging to another instance, stale fingerprints, corrupted snapshots, no-op targets, and lifecycle races. If a stale error appears, refresh history and reassess the target. If integrity validation fails, the revision remains visible for diagnosis but cannot be marked known-good or restored.

## Recovery and troubleshooting

Configuration history uses the same primary/backup recovery path as the active configuration. If `instances.json` is unreadable, the manager attempts `instances.json.bak` and preserves its revision history and known-good pointer. If both files are damaged, back up the configuration directory before attempting manual repair.

Do not edit revision IDs, fingerprints, parent links, or snapshots by hand. A mismatch is treated as corruption and rollback is blocked. Do not delete the entire configuration directory to clear one bad revision; retain the files for diagnosis and restore from a trusted backup instead.

Common operator responses:

- **Instance is running or recovering:** Stop it and cancel recovery, then retry.
- **Configuration changed / stale fingerprint:** Refresh the panel and choose a target again.
- **Revision missing:** It may have been pruned by the 50-revision retention limit.
- **Integrity check failed:** Do not force the rollback; restore a trusted backup or recreate the intended configuration through the editor.
- **Persistence failed:** Check directory permissions and available disk space. The previously durable configuration remains authoritative.
