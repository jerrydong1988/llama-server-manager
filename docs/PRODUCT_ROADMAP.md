# Product Roadmap

This document is the repository source of truth for the long-term product direction of Llama Server Manager (LSM). GitHub milestones and tracking issues mirror this document; they do not replace it.

## Product Direction

LSM is a lightweight, cross-platform control plane for operating local and private `llama.cpp` inference. Its core job is to turn an engine, model, configuration, and routing policy into a deployment that is recoverable, observable, versioned, and safe to change.

The product should remain useful on a single workstation while building a deliberate path toward managed multi-worker operation.

## Current State

| Phase | Status | Tracking issue | Milestone |
| --- | --- | --- | --- |
| Phase 1 — Reliable Runtime | Complete | [#53](https://github.com/jerrydong1988/llama-server-manager/issues/53) | [Milestone 1](https://github.com/jerrydong1988/llama-server-manager/milestone/1) |
| Phase 2 — Managed Deployment | Complete | [#54](https://github.com/jerrydong1988/llama-server-manager/issues/54) | [Milestone 2](https://github.com/jerrydong1988/llama-server-manager/milestone/2) |
| Phase 3 — Distributed Control Plane | Complete | [#55](https://github.com/jerrydong1988/llama-server-manager/issues/55) | [Milestone 3](https://github.com/jerrydong1988/llama-server-manager/milestone/3) |

- **Current phase:** None — Phase 1 through Phase 3 are complete.
- **Last roadmap review:** 2026-08-21
- **Program status:** The three-phase roadmap is complete. Any successor phase requires a dedicated roadmap change.
- **Phase order:** Phase 1 → Phase 2 → Phase 3
- **Completion basis:** Phase 3 tracker [#55](https://github.com/jerrydong1988/llama-server-manager/issues/55) records accepted exit evidence for every Phase 3 workstream and has no unresolved exit blocker.

Only a dedicated roadmap pull request may change the current-phase marker, phase order, product direction, or exit gates.

## Product Boundaries

### In scope

- `llama.cpp` engine discovery, qualification, compatibility, and lifecycle control.
- Model, configuration, deployment, and routing management.
- Recovery, rollback, observability, resource planning, and controlled rollout.
- Secure multi-worker coordination that preserves a first-class single-node mode.

### Not product pillars

- Chat, RAG, agent, MCP, voice, or general AI application authoring.
- A broad multi-provider gateway unrelated to `llama.cpp` operations.
- A full Kubernetes replacement or general-purpose cluster scheduler.
- Arbitrary remote shell execution or unrestricted lifecycle hooks.
- Engine build recipes as a product center before qualification and release flows are mature.

Capabilities such as KV-cache checkpointing may be evaluated as supporting mechanisms for versioned deployment, recovery, or residency. They do not become independent product pillars without evidence and an explicit roadmap change.

## Cross-Phase Spine: Versioned Deployments

A deployment revision progressively binds:

- an engine artifact and verified fingerprint;
- a model artifact and identity;
- an immutable configuration revision;
- runtime and recovery policy;
- rollout and routing policy; and
- qualification and operational evidence.

Phase 1 establishes identity, recovery, and rollback. Phase 2 makes revisions deployable and observable. Phase 3 places and routes them across secure workers.

## Phase 1 — Reliable Runtime

**Status:** Complete. Exit accepted on 2026-08-19 in [tracker #53](https://github.com/jerrydong1988/llama-server-manager/issues/53).

**Outcome:** Make a local `llama.cpp` deployment recoverable, traceable, and safe to change.

**Entry condition:** Existing single-node lifecycle, compatibility probing, persistence, and cross-platform release gates remain the baseline.

### Workstreams

1. **Instance self-healing and Crash Loop protection**
   - Classify expected stops, startup failures, and unexpected exits.
   - Apply bounded restart policy with backoff and an explicit Crash Loop state.
   - Preserve the original diagnostic evidence and always provide manual recovery.
2. **Configuration revisions, diff, and rollback**
   - Record immutable revisions for deployment-affecting configuration.
   - Show meaningful differences without exposing secrets.
   - Restore a known-good revision through a tested, auditable path.
3. **Engine qualification MVP**
   - Produce a qualification report for version, capabilities, startup, health, and representative inference.
   - Bind results to the engine fingerprint and invalidate them when the artifact changes.
   - Fail safely when compatibility is unknown or qualification is incomplete.
4. **Versioned-deployment identity foundation**
   - Define stable identities for engine, model, configuration, and qualification evidence.
   - Avoid introducing Phase 2 rollout policy before these identities are reliable.

### Exit gate

- Recovery behavior is bounded, diagnosable, and covered on supported platforms.
- Crash Loop protection cannot restart indefinitely or erase the originating failure.
- Configuration revision, diff, and rollback paths have regression and migration coverage.
- Engine qualification is reproducible, fingerprint-bound, and safely blocks incompatible deployment.
- Operator documentation, relevant local checks, and required cross-platform CI pass.
- [Phase 1 tracker #53](https://github.com/jerrydong1988/llama-server-manager/issues/53) links the implementation evidence and records an explicit exit review.

## Phase 2 — Managed Deployment

**Status:** Complete. Exit accepted on 2026-08-20 in [tracker #54](https://github.com/jerrydong1988/llama-server-manager/issues/54).

**Outcome:** Turn reliable instances into observable, versioned, policy-driven deployments.

**Entry condition:** Phase 1 is marked complete through the transition protocol and its tracker contains accepted exit evidence.

### Workstreams

1. **Deployment abstraction**
   - Bind artifacts, configuration revision, qualification, runtime policy, and routing state.
2. **Resource planner**
   - Estimate memory and runtime feasibility before mutating active deployments.
3. **Model and engine canary rollout**
   - Support operator-controlled promotion, observation, abort, and rollback.
4. **Operational metrics and alerts**
   - Expose TTFT, queue pressure, cache behavior, errors, saturation, and rollout health.

### Exit gate

- A deployment revision is reproducible and has an explicit rollback target.
- Resource planning reports feasibility and uncertainty before changes are applied.
- Canary promotion and rollback are operator-controlled, observable, and auditable.
- TTFT, queue, cache, error, and saturation signals have stable definitions and actionable alerts.
- End-to-end lifecycle tests, operator documentation, relevant local checks, and required cross-platform CI pass.
- [Phase 2 tracker #54](https://github.com/jerrydong1988/llama-server-manager/issues/54) links the implementation evidence and records an explicit exit review.

## Phase 3 — Distributed Control Plane

**Status:** Complete. Exit accepted on 2026-08-21 in [tracker #55](https://github.com/jerrydong1988/llama-server-manager/issues/55).

**Outcome:** Coordinate secure workers and resource-aware routing while preserving first-class single-node operation.

**Entry condition:** Phase 2 is marked complete through the transition protocol and deployment revisions are stable enough to place across workers.

### Workstreams

1. **Automatic model-residency scheduling**
   - Place, warm, drain, and evict revisions within declared resource budgets.
2. **Headless CLI**
   - Provide stable lifecycle commands, structured output, exit codes, and automation-safe authentication.
3. **Secure Worker Agent**
   - Use authenticated, encrypted, least-privileged coordination without arbitrary remote shell access.
4. **Session- and cache-aware routing**
   - Prefer useful locality while preserving health, capacity, and failover correctness.

### Exit gate

- Placement decisions are deterministic enough to explain and respect declared resource budgets.
- Draining, failure, and failover cannot silently lose deployment or routing state.
- CLI contracts are documented and covered by automation tests.
- Worker communication is authenticated, encrypted, least-privileged, and auditable.
- Cache-aware routing has end-to-end correctness tests and observable fallback behavior.
- Single-node operation remains supported without requiring a Worker Agent.
- [Phase 3 tracker #55](https://github.com/jerrydong1988/llama-server-manager/issues/55) links the implementation evidence and records an explicit exit review.

## Execution and Anti-Drift Rules

Every product feature issue, Codex Goal, and pull request must identify:

1. the roadmap phase and tracking issue;
2. the workstream or exit criterion it advances;
3. its explicit non-goals;
4. the commands or artifacts that prove completion; and
5. the condition at which work must stop.

Work may enter the current phase when it is:

- a listed current-phase workstream;
- enabling work required by a current-phase exit criterion;
- a defect, security fix, compatibility update, release task, or maintenance requirement; or
- authorized by an explicit roadmap-change pull request.

Ideas for later phases must be captured in their tracking issue or a linked candidate issue. They must not be implemented opportunistically during current-phase work.

## Phase Transition Protocol

1. Complete the current phase tracker and link objective evidence for every exit criterion.
2. Run the relevant local validation and all required cross-platform CI.
3. Perform a dedicated phase-exit review; unresolved blockers keep the phase current.
4. Merge a dedicated roadmap pull request that marks the phase complete and the next phase current. When the final planned phase exits, mark the roadmap complete without inventing a successor phase.
5. Update the affected milestone descriptions and phase tracking issues.
6. Only then start a Codex Goal or implementation issue for the next phase. If no successor phase is planned, new product work requires a dedicated roadmap change first.

Closing a milestone or shipping a release does not automatically activate the next phase.

## Codex Goal Contract

Use a Codex Goal for one bounded work package, not for the complete multi-year roadmap. A goal should read this document and the active tracking issue first, name its stopping condition, validate progress at checkpoints, and explicitly avoid starting the next work package.

Template:

```text
Complete <tracking issue or bounded work package> for <roadmap phase>.
Read docs/PRODUCT_ROADMAP.md and <tracking issue> before changing code.
Do not implement later-phase work.
Validate with <commands and evidence>.
Stop when <verifiable end state> is reached and update the tracker.
```

## Review Cadence

- Review roadmap alignment when scoping each product feature.
- Update the active tracker after each merged work package.
- Audit roadmap, milestone, and tracker consistency before every formal release.
- Re-evaluate product direction and phase boundaries only through an explicit roadmap pull request.
