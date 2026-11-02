# Gufo integration pre-release audit — 2026-11-03

Scope: changes since v2.9.54, including Gufo engine discovery/configuration,
process and cache isolation, readiness, routing admission, protocol errors,
tool-history validation, telemetry and release packaging. This supplements the
manual API findings in `GUFO_AUDIT_DECISIONS.md`.

## Findings and fixes

| Finding | Trigger and impact | Resolution and regression |
| --- | --- | --- |
| Shared instance authentication lost | Switching between llama.cpp and Gufo reset an existing inline API key. | Preserve `api_key` with the common model/port identity; test both switch directions. Engine-specific settings still reset. |
| Stale target permit released a new target's slot | A stopped target was evicted, recreated with the same ID, and admitted a new request before an old response body dropped. The old permit decremented the new counter. | Bind permits to the target runtime identity. Reproduction asserts the new request stays counted and a competing request receives capacity rejection. |
| Duplicate engine inventory | One overlapping search root reused its cache while another scanned the same executables. | Deduplicate cached and scanned engines using the same inventory identity. The regression covers co-located llama.cpp and Gufo executables. |
| Failed save overwrote a newer update | A Gufo save failed after another operation updated the same instance. Unconditional rollback restored stale configuration. | Roll back only if the current configuration is still the submitted snapshot. A browser test injects an update while persistence is pending. |
| Unix cache writer lock outlived its owner | Linux CI failed an immediate restart with “dynamic cache is owned by another running instance”. A fork-inherited descriptor can retain `flock` after the parent's descriptor closes. | A writer-lock guard explicitly unlocks on success, failure and drop. The Unix regression keeps a duplicated descriptor alive, then checks immediate reacquisition and exclusive ownership of the next writer. The descriptor behavior was also reproduced in an isolated Linux container. |

The first four findings have failing-before regression evidence. The Unix issue
has the original failing Linux CI job and a deterministic descriptor-level
reproduction; its Rust regression is exercised by Unix CI.

## Preserved behavior

- Full target slots immediately return `503 route_capacity`; no target queue,
  automatic retry or engine-side request rewriting is added.
- Native reasoning/output budgets and stop sequences remain engine behavior.
  Clients must inspect completion status; a fixed token minimum cannot guarantee
  a completed tool call.
- Chat tool-history validation checks pairing, not tool payload meaning. Other
  native protocols and token-count endpoints retain their contracts.
- Existing llama.cpp instances, checkpoints and the connect-only MCP lifecycle
  remain supported. Gufo qualification is for Windows and the two tested local
  model profiles, not a claim of universal engine/model compatibility.

## Verification and release gates

Required evidence: `npm run check:release`, production frontend build, locked
Rust tests, warning-denying Clippy, current upstream compatibility check, and all
six protected-branch CI checks. A release tag must identify the merged version
commit. Updater signatures, public download hashes and website synchronization
are checked by the release workflows.

Earlier real-engine qualification covered both Gufo profiles at a configured
262K context and a real llama.cpp control, including tool rounds, malformed
history rejection, streaming, cancellation and preserved production-file hashes.
This is not a claim that full 262K input, GPU reset recovery or indefinite
unattended operation was tested. Final execution results are recorded with the
release validation below.
