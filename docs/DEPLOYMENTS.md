# Managed Deployments and Revisions

Llama Server Manager treats every configured instance as one stable **Deployment**. A qualified launch request materializes an immutable **Deployment Revision** that records exactly what the manager accepted for that launch.

This is the first workstream of [Phase 2 — Managed Deployment](PRODUCT_ROADMAP.md#phase-2--managed-deployment). It remains a single-machine foundation: it does not schedule workers, estimate resources, or roll back automatically. The later [model and engine canary workflow](CANARY_ROLLOUTS.md) uses its immutable revision bindings for explicit promotion and rollback decisions.

## What a revision binds

Each revision contains identifiers and small policy snapshots, not a second copy of the full configuration:

- the verified engine artifact identity;
- the verified model artifact identity;
- the immutable configuration revision and configuration identity;
- the accepted engine-qualification evidence;
- manager-owned `auto_start` and failure-recovery policy; and
- the routing state relevant to this instance: proxy enablement, default-target status, routing strategy, and explicit routes targeting the instance.

Routes that target other instances are excluded. Historical deployment IPC does not expose model paths, engine paths, API keys, certificates, command lines, or full configuration snapshots.

## Identity and integrity

The stable Deployment ID is derived from the instance ID. A Deployment Revision ID is content-addressed from the stable Deployment, Phase 1 composite identity, runtime policy, and routing snapshot. Repeating the same qualified launch reuses the same revision.

Every stored revision also has an integrity seal that covers its content-addressed ID and creation time. The manager rejects duplicate IDs, broken current or rollback pointers, altered revision content, unsupported future schemas, and revision bindings that do not belong to the instance.

The catalog retains at most 32 revisions per Deployment while preserving the current and explicit rollback-target revisions.

## Lifecycle

1. The instance configuration must first be saved as an immutable configuration revision.
2. Launch preflight revalidates the configured engine, qualification evidence, model inventory identity, configuration identity, command capabilities, authentication, TLS, and effective arguments.
3. Under the serialized configuration-write boundary, the manager creates or reuses a Deployment Revision and persists it before spawning a process.
4. Foreground and independent-runtime launches record the stable Deployment ID and exact Revision ID on the running instance.
5. The independent runtime persists the complete, secret-free revision binding with its desired launch snapshot. Recovery revalidates engine, model, configuration, runtime policy, routing, revision identity, and integrity before starting anything.

If the process cannot be started after a revision is materialized, the revision remains as the validated desired state. It does not claim that the process became healthy.

## Operator states

The Instance Manager shows one of four states:

| State | Meaning | Operator action |
| --- | --- | --- |
| Ready | The current artifacts, configuration, policy, and routing reproduce the current revision. | No action is required. |
| Not materialized | A migrated or newly created instance has no qualified launch revision yet. | Complete qualification and start the instance. |
| Needs new revision | One or more bound inputs changed, or the running process is on another revision. | Review the change and start the instance to materialize it. |
| Invalid | Schema, identity, integrity, or a revision pointer failed validation. | Do not rely on recovery; repair or restore the configuration catalog first. |

The panel shows the current revision, running revision, explicit rollback target, runtime and routing summaries, and bounded revision history. It never starts, promotes, or rolls back a process by itself.

## Migration and compatibility

Existing `instances.json` files migrate by adding an empty stable Deployment record for every existing instance. No process is started and no historical artifact claim is invented. The next qualified start materializes the first revision. Deleting an instance removes its Deployment record through the normal configuration-save path.

Runtime-state schemas 1–3 migrate to schema 4 for read compatibility. Their launch specifications do not contain a valid Deployment Revision, so automatic recovery fails closed until the operator starts the instance through the current application and creates a schema-4 binding.

Future configuration or runtime schemas are rejected instead of being silently downgraded.

## Concurrency and invalidation

Configuration saves and Deployment catalog writes share one serialized, atomic persistence boundary. An instance start reservation blocks deployment-affecting configuration edits while preflight and materialization are in progress. Proxy saves also reject routing changes that affect an instance currently starting.

Display-name changes do not invalidate a revision. Changes to artifacts, deployment-affecting configuration, recovery policy, relevant routing, or the current/running revision relationship do invalidate it. Unrelated routes do not.

Persistence failure rolls back a spawned process through the existing lifecycle safeguards. Qualification, identity, integrity, schema, and recovery validation failures are fail-closed.

## Rollback semantics

When a new revision becomes current, the previous current revision becomes the explicit rollback target. This pointer is traceability evidence; Phase 2's Deployment abstraction does not automatically apply it.

An operator can explicitly restore an earlier configuration through **Configuration Revisions**, verify qualification and routing, and start the instance. That qualified start will reuse an identical historical Deployment Revision when every bound input matches, or create a new one when any input differs. The [canary rollout](CANARY_ROLLOUTS.md) provides operator-controlled promotion and restoration of base routing; automatic rollback remains outside this workstream.

## Validation

The implementation is covered by Rust tests for deterministic identity, migration, integrity and pointer rejection, no-op reuse, transition and retention behavior, route isolation, staleness, and runtime schema/recovery validation. Browser tests cover English and Chinese operator states. `scripts/test-runtime-service.cjs` exercises the complete revision binding across runtime start, persistence, detach, restart, and recovery.
