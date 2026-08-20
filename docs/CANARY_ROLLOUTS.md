# Model and Engine Canary Rollouts

Llama Server Manager can compare two already-running Deployment Revisions behind one public model alias. The operator chooses the stable revision, candidate revision, and initial candidate traffic share, observes current health and request outcomes, and then explicitly promotes, aborts, or rolls back the rollout.

This is the third workstream of [Phase 2 — Managed Deployment](PRODUCT_ROADMAP.md#phase-2--managed-deployment). It is a local, operator-controlled workflow. It does not start or stop instances, relocate workloads, or promote automatically. The shared TTFT, queue, cache, saturation, and alert definitions are documented in [Operational Metrics and Alerts](OPERATIONAL_METRICS.md).

## Prerequisites

Before starting a rollout:

1. Enable and start the production proxy.
2. Start two distinct instances for the same model workload.
3. Make sure both instances are running the current, integrity-valid Deployment Revision shown by the Instance Manager.
4. Open **Proxy Service → Model and engine canary rollout**.

The stable and candidate instances may use different qualified engine artifacts or model artifacts. They must expose the same model workload so one public alias has consistent API semantics.

Only one unresolved rollout can exist at a time. The initial candidate share must be between 1% and 50%. The remaining traffic goes to the stable revision.

## Operator workflow

### 1. Start

Select the stable instance, candidate instance, public model alias, and initial candidate share. Starting the rollout records the exact Deployment and Revision IDs, installs a temporary weighted routing overlay, and appends an integrity-protected audit event.

Base proxy routes and the configured routing strategy are preserved. The overlay applies only to the selected public alias and uses weighted routing for that alias. Other aliases continue to use the existing proxy configuration.

### 2. Observe

Use **Observe now** to record a fresh point-in-time observation. The panel shows:

- rollout state and stable/candidate traffic shares;
- the exact stable and candidate revision IDs;
- current instance health;
- successful and failed proxied request counts, TTFT P95, queue-wait P95, and observed prompt-cache reuse since the rollout began; and
- revision or route drift plus the bounded audit history.

Request evidence is scoped to the two selected targets, begins at the rollout creation time, and counts completed proxy requests. Missing TTFT or cache usage remains unknown. These target values reuse the shared operational definitions, but promotion remains an explicit operator decision and never fires from an alert.

### 3. Adjust traffic

While the rollout is active, the operator can explicitly change the candidate share between 1% and 50%. Each accepted change is persisted and audited. Moving to 100% is intentionally a separate **Promote candidate** action.

### 4. Resolve

- **Promote candidate** requires the candidate to be ready, sends 100% of the alias traffic to it, and retains the stable binding for an explicit rollback.
- **Abort canary** removes the temporary overlay from an active rollout and restores the unchanged base proxy routing.
- **Roll back promotion** removes the temporary overlay after promotion and restores the unchanged base proxy routing.

These actions do not stop either instance and do not rewrite their configuration or Deployment Revisions.

While a rollout is active or promoted, deployment-affecting configuration and base-routing changes for its two bound instances are frozen. Abort or roll back first if either deployment needs a new revision. Unrelated instance and proxy settings remain editable.

## Fail-closed behavior

Every progress action revalidates the proxy, both running instances, both current/running Deployment Revision bindings, rollout integrity, and the exact temporary route overlay. Promotion also revalidates candidate health.

If a bound revision changes, an instance stops, the proxy stops, or the temporary routes no longer match the recorded rollout, observation reports drift and traffic changes or promotion are rejected. Each temporary route also carries its exact required Revision ID: if any enabled route no longer matches its running revision, requests for that canary alias fail closed instead of being silently retargeted to a newer process.

Abort and rollback can still restore base routing after instance revision drift, but they refuse to overwrite an overlay that was changed outside the rollout workflow. Review and repair unexpected proxy configuration changes before proceeding.

## Identity, persistence, and audit

The temporary canary overlay is deliberately separate from the base routes captured by a Deployment Revision. Starting or adjusting a canary therefore does not make the already-running stable and candidate revisions stale. The rollout record independently seals the expected overlay, both deployment bindings, state, traffic share, observations, and audit chain.

The configuration keeps at most 32 rollout records. Each rollout keeps at most 128 audit events and carries a chain anchor across pruning, so retained history can still be verified. Unsupported schemas, modified records, broken event chains, duplicate IDs, invalid state/weight combinations, and mismatched route roles are rejected instead of being repaired silently.

Persistence and live proxy updates use the existing serialized configuration and proxy lifecycle boundaries. If a live update fails, the manager restores the previous persisted rollout and proxy configuration and attempts to restore the previous runtime state.

## Validation coverage

Rust tests cover lifecycle transitions, revision and route drift, candidate health gating, traffic-share boundaries, bounded audit integrity, proxy alias isolation, runtime capability negotiation, and target/time-scoped request evidence. Frontend regression and browser tests cover creation, observation, traffic adjustment, promotion confirmation, and rollback confirmation.
