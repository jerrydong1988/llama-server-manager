# Automatic Model Residency

Automatic model residency is the first implementation workstream in [Phase 3 — Distributed Control Plane](PRODUCT_ROADMAP.md#phase-3--distributed-control-plane), tracked by [#76](https://github.com/jerrydong1988/llama-server-manager/issues/76). It reconciles exact current [Deployment Revisions](DEPLOYMENTS.md) on manager-controlled local instances while preserving a complete zero-Worker single-node mode.

## Safety boundary

The current Cluster-page Worker is a trusted llama.cpp `rpc-server` compute endpoint. It does not provide an authenticated lifecycle protocol for launching a remote `llama-server`. Residency therefore never runs remote shell commands and never treats an RPC Worker as a remote lifecycle target. That boundary changes only in the separate Secure Worker Agent workstream.

Manual instance start and stop remain available. The scheduler manages only instances explicitly present in its policy, and it evicts only placements it previously adopted or warmed. Disabling the scheduler pauses reconciliation; it does not stop workloads. To remove a scheduler-owned placement, keep the scheduler enabled, disable that instance's intent, preview the plan, and apply it.

## Policy and deterministic placement

The policy declares:

- a worst-case host RAM budget;
- a worst-case accelerator VRAM budget, which may be zero for CPU-only plans;
- a drain timeout from 5 to 3,600 seconds;
- an enabled flag and numeric priority for each managed instance.

Lower numeric priorities are considered first. Ties use the stable instance ID. Each candidate must have a valid current Deployment Revision, a known engine, no unresolved canary rollout, and a `feasible` resource estimate. Unknown, constrained, or infeasible estimates are not warmed automatically.

RAM and VRAM accounting uses each resource plan's maximum estimate rather than its expected estimate. A candidate is selected only if adding both maxima remains within both declared budgets. Identical policy, revision, engine, model, resource estimate, and running-revision inputs produce the same SHA-256 plan ID and operation order. Wall-clock generation time and audit history are deliberately excluded from the identity.

When an already managed placement becomes unselected because its intent is disabled or a declared budget is exhausted, the plan schedules eviction. Missing artifacts, missing engines, unknown estimates, and active canary rollouts hold the current placement instead of turning control-plane uncertainty into a destructive action.

## Reconciliation order

Applying a plan executes a stable three-stage sequence:

1. **Drain** — persist the draining phase, reject new router admissions for the target, and continue counting requests that already hold a target permit.
2. **Evict** — wait until the target's in-flight count reaches zero, stop the local instance, and persist the evicted phase. A timeout or stop failure leaves the routing drain in place and records the failure.
3. **Warm** — start or adopt the exact current Deployment Revision, verify the running deployment and revision IDs, persist the resident phase, and clear any previous drain.

All drain starts and warm or eviction outcomes are appended to a bounded audit trail. Placement state, plan linkage, failure text, and the routing-drained flag live in the atomic `instances.json` configuration and its existing backup. Historical residency state is not included in the frontend startup injection; it is available only through the dedicated inspection command.

## Restart and recovery

On startup, persisted `routingDrained` placements are restored into application state before the proxy starts. A newly created router runtime receives the drain set before it accepts traffic. Opening the Cluster page re-inspects the durable catalog and reapplies the same set, so an unfinished or failed eviction cannot silently resume admissions.

An evicted placement can be warmed by applying a later selected plan. A failed eviction remains drained until a successful eviction or warm completion clears it. Removing an instance prunes its stale policy intent but retains placement and audit evidence with an explicit migration event.

## Operator workflow

1. Materialize qualified Deployment Revisions by starting eligible instances at least once.
2. Open **Cluster → Automatic model residency**.
3. Declare RAM, VRAM, and drain timeout budgets; enable the desired instances and assign priorities.
4. Select **Save and preview**. Review selected decisions, reason codes, worst-case usage, plan ID, and ordered operations.
5. Select **Apply plan**. Keep the application running until the operation list is reconciled, or return later to inspect persistent failure evidence.

The control surface reports registered RPC Workers for context and explicitly marks that the Worker Agent is unavailable. No Worker is required for planning or execution.

## Validation

The implementation is covered by Rust tests for deterministic identities, budget enforcement, fail-closed estimates, schema migration, persistent drains, and router admission control; frontend regression checks for serialization and operation ordering; and a browser test for saving and applying a single-node plan.
