# v2.9.25 Release Audit Remediation Design

## Goal

Resolve every validated pre-release audit finding, preserve the vector-model configuration guarantees, and publish `v2.9.25` only after local and cross-platform release checks pass.

## Configuration Authority

The frontend continues to normalize configuration immediately for responsive UI behavior, while Rust remains the final persistence and launch authority. `save_config` returns the normalized instance map that was actually written. The frontend applies that result only for the newest requested save revision, preventing an older save response from overwriting newer in-memory edits. The configuration page derives its saved baseline from the accepted store value.

## Workload Detection

Positive Embedding or Reranker detection remains authoritative and locks the generated workload fields. Negative values produced by filename and architecture heuristics are not treated as proof that a model is inference-only. Unknown architectures may retain an explicit user choice. Selecting a positively identified inference model still clears stale vector flags through the model-selection transition path.

## Save And Lifecycle Errors

The latest-save coordinator reports a failed drain to callers that were waiting for that drain, then clears the historical error. Later idle checks are not poisoned by an already reported failure. Instance start and stop failures create visible localized runtime warnings and reject their promises so auto-start and other callers can distinguish success from failure.

## Single-Flight Startup

The frontend tracks instance IDs with an in-flight start operation. Manual and automatic starts share this guard, and auto-start does not resubmit an instance while normalization updates the store. Rust reserves the instance ID in shared state before opening the log or spawning the process. Every failure path releases the reservation; successful startup replaces it with the real process record.

## Cross-Platform Paths

Model-path comparison receives explicit platform semantics. Windows drive paths, extended paths, backslash UNC paths, and forward-slash UNC paths compare case-insensitively on Windows. POSIX paths retain case-sensitive behavior.

## Dependencies And Release Integrity

Upgrade directly controlled dependencies where the change is compatible, especially `reqwest`. RustSec warnings owned by the Tauri Linux GTK3 stack are documented as upstream exceptions when no compatible project-level upgrade exists. The release quality job runs Rust dependency auditing. Tagged builds must fail unless the tag exactly matches `package.json`, `package-lock.json`, `Cargo.toml`, `Cargo.lock`, `tauri.conf.json`, and `GUIDE.md`.

The guide states that missing Windows or Apple credentials produce clearly labeled unsigned or ad-hoc artifacts. After all checks pass, all project versions become `2.9.25`, the release branch is merged, and tag `v2.9.25` triggers the formal release workflow.

## Test Strategy

- TypeScript behavior tests cover unknown workload overrides, save-result revision handling, cleared historical errors, and Windows UNC equivalence.
- Store/source regression tests cover visible lifecycle failures and auto-start single-flight behavior.
- Rust tests cover normalized save results and pre-spawn instance reservation.
- Release checks cover tag/version mismatch and signing documentation.
- Final validation includes frontend release checks and build, Rust format/test/Clippy, npm and RustSec audits, actionlint, diff checks, and the five-platform GitHub Actions workflow.

