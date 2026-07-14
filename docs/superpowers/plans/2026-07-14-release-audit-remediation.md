# v2.9.25 Release Audit Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix every validated release audit issue and publish a verified `v2.9.25` release.

**Architecture:** Keep immediate frontend normalization but make the Rust save result the persisted truth. Add frontend and backend startup single-flight protection, explicit workload fallback semantics, platform-aware path comparison, visible error propagation, and tag-to-version enforcement.

**Tech Stack:** React 18, TypeScript 5, Zustand 5, Tauri 2, Rust 2021, Node regression scripts, GitHub Actions.

## Global Constraints

- Preserve inference and MTP behavior for positively identified inference models.
- Embedding and Reranker launch commands must contain no generation, sampling, speculative, MTP, or custom arguments.
- Do not weaken backend launch-time normalization.
- Windows and macOS certificates remain optional for release.
- Release version is `2.9.25` and tag is `v2.9.25`.
- Every behavior fix follows a failing-test-first cycle.

---

### Task 1: Persisted Configuration Result And Save Recovery

**Files:**
- Modify: `src/store/configSaveCoordinator.ts`
- Modify: `src/store/instanceSlice.ts`
- Modify: `src/components/ConfigPage.tsx`
- Modify: `src-tauri/src/commands/config.rs`
- Test: `scripts/test-config-save-sequencing.cjs`
- Test: `src-tauri/src/commands/config.rs`

**Interfaces:**
- Produces: `LatestSaveCoordinator<T, R>` and `save_config -> HashMap<String, InstanceConfig>`.
- Consumes: existing `synchronizeInstanceSummary` and vector normalization.

- [ ] Add failing tests proving a reported save failure does not poison a later idle check and a backend-normalized result becomes the accepted newest frontend snapshot.
- [ ] Run `node scripts/test-config-save-sequencing.cjs` and the focused Rust config test; confirm the new assertions fail for the expected behavior.
- [ ] Generalize the coordinator result type, clear historical drain errors after settling waiters, return normalized instances from Rust, and apply only the newest save revision in Zustand.
- [ ] Update ConfigPage saved state from the accepted store configuration.
- [ ] Re-run focused tests and commit the passing task.

### Task 2: Workload Detection And Path Equivalence

**Files:**
- Modify: `src/modelPolicy.ts`
- Modify: `src/store/bootstrap.ts`
- Modify: `src/components/ConfigPage.tsx`
- Test: `scripts/test-vector-model-policy.cjs`
- Test: `scripts/test-cross-platform-paths.cjs`

**Interfaces:**
- Produces: explicit workload fallback for unknown architectures and `normalizeModelPath(value, platform)`.
- Consumes: `ModelInfo.capabilities` positive detection and model-selection transitions.

- [ ] Add failing behavior tests for an unknown architecture with explicit `embedding`/`reranking`, switching to identified inference, and mixed Windows UNC separators/casing.
- [ ] Run both scripts and confirm each new assertion fails for the audited reason.
- [ ] Make only positive capability detection authoritative, preserve explicit fallback, and pass platform semantics into model-path matching.
- [ ] Re-run focused tests and commit the passing task.

### Task 3: Startup Single-Flight And Visible Lifecycle Errors

**Files:**
- Modify: `src/store/instanceSlice.ts`
- Modify: `src/store/types.ts`
- Modify: `src/App.tsx`
- Modify: `src-tauri/src/models.rs`
- Modify: `src-tauri/src/commands/server.rs`
- Test: `scripts/test-bootstrap-health.cjs`
- Test: `scripts/test-config-save-sequencing.cjs`
- Test: `src-tauri/src/commands/server.rs`

**Interfaces:**
- Produces: per-instance frontend start guard and backend pre-spawn reservation lifecycle.
- Consumes: runtime warnings, `startInstance`, `stopInstance`, and `AppState.running`.

- [ ] Add failing tests proving duplicate frontend starts share one operation, errors reject and create a runtime warning, and Rust refuses a reserved instance before opening its log.
- [ ] Run focused Node and Rust tests; confirm expected failures.
- [ ] Implement frontend single-flight, propagate lifecycle failures, and reserve/release backend instance IDs around every launch path.
- [ ] Re-run focused tests and commit the passing task.

### Task 4: Dependency And Release Policy

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `.github/workflows/build.yml`
- Modify: `scripts/check-cross-platform-release.cjs`
- Modify: `scripts/check-guide.cjs`
- Modify: `GUIDE.md`
- Modify: `docs/RELEASE_SIGNING.md`

**Interfaces:**
- Produces: tag/version validation and RustSec CI policy.

- [ ] Add failing release-script checks for mismatched `GITHUB_REF_NAME` and the obsolete signing statement.
- [ ] Run the checks with a fake tag and confirm failure is caused by the new requirements.
- [ ] Upgrade compatible direct dependencies, add RustSec audit with documented upstream exceptions, enforce tag equality, and correct signing text.
- [ ] Run release checks, dependency audits, and actionlint; commit the passing task.

### Task 5: Version, Full Verification, And Release

**Files:**
- Modify: `package.json`
- Modify: `package-lock.json`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `GUIDE.md`

**Interfaces:**
- Produces: version `2.9.25` and tag `v2.9.25`.

- [ ] Set all version sources to `2.9.25` and run the fake-tag/version check with `v2.9.25`.
- [ ] Run `npm run check:release`, `npm run build`, Rust format/test/Clippy, npm audit, RustSec audit, actionlint, and `git diff --check`.
- [ ] Perform an independent read-only review of the complete diff and resolve every validated finding.
- [ ] Commit and push the release branch, wait for all five CI jobs, merge to `master`, create and push `v2.9.25`.
- [ ] Monitor the tagged workflow, verify release assets and checksums, and report the published release URL.

