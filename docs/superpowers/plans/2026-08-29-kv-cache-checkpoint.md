# KV / Prefill Cache Checkpoint Implementation Plan

**Goal:** 在 llama-server-manager 中实现默认关闭、管理器拥有生命周期的 llama.cpp slot checkpoint，使兼容恢复在代理可路由之前完成，所有失败安全退化为冷启动。

**Design:** `docs/superpowers/specs/2026-08-29-kv-cache-checkpoint-design.md`

**Tech Stack:** Tauri 2、Rust 1.80、reqwest blocking、serde/serde_json、sha2、uuid、Tokio、Axum、React 18、TypeScript 5、Zustand 5、Node.js regression scripts。

**Implementation record (2026-08-29):** Tasks 1-9 completed on the implementation branch, including real llama-server and DeepSeek Harness acceptance. Task 10 records the remaining release-gate and pull-request delivery work; detailed measured results are captured in the design implementation record and implementation PR evidence. The unchecked boxes below preserve the original execution specification rather than serving as a live tracker.

## Global Constraints

- 只实现设计文档的第一版支持矩阵；不得为了提高命中率放宽 fingerprint 或偷偷支持未验收模型类型。
- 功能默认关闭。关闭时生成命令、健康探测、代理和停止行为必须保持不变。
- 所有 checkpoint failure 都是 fail-open cold start；corrupt/partial restore 必须 erase。
- 只有管理器生成 slot filename；持久化数据只位于应用数据目录。
- save 必须发生在进程终止之前；restore 必须发生在代理 routable 之前。
- 直管和 runtime service 必须调用同一核心实现。
- 不复制 Warpdrv、Ollama 未合并 PR 或其他项目源码；只使用官方 llama.cpp HTTP API 独立实现。
- 所有 Windows 手工文件编辑使用 `apply_patch`；每个 Rust/TypeScript 任务完成后立即运行对应编译和 focused tests。
- 每个任务提交范围明确的小 commit；不得把临时模型、checkpoint、日志、报告或测试截图提交到仓库。

---

### Task 1: Configuration, Status Contracts, and Eligibility

**Files:**

- Modify: `src-tauri/src/models.rs`
- Modify: `src/store/types.ts`
- Modify: `src/store/defaults.ts` or the current instance default source
- Create: `src-tauri/src/checkpoint.rs`
- Modify: `src-tauri/src/main.rs`
- Create: `scripts/test-kv-checkpoint.cjs`
- Modify: `package.json`

**Deliverable:** 向后兼容的 `KvCheckpointConfig`、`CheckpointStatus`、稳定 reason codes 和保守 eligibility evaluator。

- [ ] Add failing Rust serde/default/eligibility tests for old configs and every supported/unsupported matrix row.
- [ ] Add a failing bundled TypeScript contract/default test.
- [ ] Implement `KvCheckpointConfig` with disabled legacy default and bounded normalization.
- [ ] Implement serializable status/phase/outcome contracts shared by direct and runtime paths.
- [ ] Implement eligibility using managed launch, inference workload, single slot, loopback HTTP, engine capability and excluded-state checks.
- [ ] Register the Node regression script in `test:regressions`.
- [ ] Run focused Rust tests, the Node script, TypeScript no-emit and encoding check.
- [ ] Commit as `feat: define kv checkpoint policy contracts`.

### Task 2: Fingerprint and Private Checkpoint Store

**Files:**

- Modify: `src-tauri/src/checkpoint.rs`
- Modify: `src-tauri/src/commands/engine_capabilities.rs` only if a reusable full binary hash helper belongs there

**Deliverable:** strict v1 fingerprint、manifest parser、private scratch、atomic generations、hash cache 和 per-instance LRU。

- [ ] Add failing tests for canonical serialization and every included/excluded config field.
- [ ] Add failing tests for model/engine content mutation and fingerprint cache invalidation.
- [ ] Add manifest parser tests for future schema, duplicate slot, path traversal, malformed digest, zero/truncated payload and mismatched identity.
- [ ] Add Windows/Unix private-path and symlink/reparse rejection tests where the platform supports them.
- [ ] Add fault-injection tests for payload copy, file sync, manifest write, generation rename and latest pointer update; previous generation must survive each failure.
- [ ] Implement streaming SHA-256 and atomic `fingerprints-v1.json` cache keyed by canonical path/size/mtime.
- [ ] Implement deterministic application-data root and scratch path validation.
- [ ] Implement pending generation, manifest-last commit, latest fallback scan and startup cleanup.
- [ ] Implement bounded per-instance LRU and oversize-generation refusal.
- [ ] Run checkpoint module tests, Rust fmt check and Cargo check.
- [ ] Commit as `feat: add private kv checkpoint store`.

### Task 3: llama.cpp Slot Client and Coordinator State Machine

**Files:**

- Modify: `src-tauri/src/checkpoint.rs`

**Deliverable:** validated `/health`、`/slots`、save/restore/erase client and fail-open coordinator operations.

- [ ] Define an internal slot backend trait so storage/lifecycle tests do not need a model.
- [ ] Add fake backend tests for legal/illegal state transitions and stale PID events.
- [ ] Add response tests for id mismatch, token/byte mismatch, invalid JSON, HTTP failure and timeout.
- [ ] Implement manager-only generated basename and authorization headers using effective instance API key.
- [ ] Implement restore: verify first, copy to scratch, restore slot 0, query slot state, erase on every partial failure.
- [ ] Implement save: require idle/useful slot, save to scratch, verify bytes, hash and commit generation.
- [ ] Ensure status messages contain stable reason codes and no secrets/paths.
- [ ] Run all checkpoint tests and Cargo check.
- [ ] Commit as `feat: orchestrate llama slot checkpoints`.

### Task 4: Effective Launch Arguments and Direct Lifecycle

**Files:**

- Modify: `src-tauri/src/commands/server.rs`
- Modify: `src-tauri/src/models.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `scripts/test-instance-lifecycle-coordinator.cjs`
- Modify: `scripts/test-kv-checkpoint.cjs`

**Deliverable:** GUI/direct path prepares private slot directory, gates readiness, restores on first engine health and saves before termination.

- [ ] Add failing command-generation tests proving disabled configs are byte-for-byte unchanged and enabled configs receive one managed `--slot-save-path`.
- [ ] Reject/mark ineligible conflicting user slot path without mutating the stored value silently.
- [ ] Add checkpoint coordinator state to `AppState` and initialize it once.
- [ ] Mark eligible instances non-routable before spawn with expected PID binding after spawn.
- [ ] Hook first successful direct monitor health transition to one-shot restore/cold-ready.
- [ ] Hook stop before `terminate_running_instance`: gate, drain bounded slot activity, save or skip, then always continue termination.
- [ ] Ensure startup rollback and unexpected exit clear gates/scratch without committing a generation.
- [ ] Emit additive checkpoint status events without changing existing `server-started` semantics.
- [ ] Run direct lifecycle regressions, checkpoint tests and Cargo check.
- [ ] Commit as `feat: checkpoint direct instance lifecycle`.

### Task 5: Runtime Service Lifecycle and Wire Compatibility

**Files:**

- Modify: `src-tauri/src/runtime_service/protocol.rs`
- Modify: `src-tauri/src/runtime_service/supervisor.rs`
- Modify: `src-tauri/src/runtime_service/mod.rs`
- Modify: `scripts/test-runtime-service.cjs`
- Modify: `scripts/test-kv-checkpoint.cjs`

**Deliverable:** independent runtime uses identical coordinator semantics and exposes additive checkpoint status with backward-compatible defaults.

- [ ] Add failing protocol tests reading the previous runtime schema without checkpoint fields.
- [ ] Add fake-process/fake-server tests for restore after health and save before force termination.
- [ ] Add coordinator and status registry to `RuntimeSupervisor`.
- [ ] Inject the same effective managed slot path before runtime spawn.
- [ ] Keep runtime health `pending` through restore; publish routable only on ready/ready-cold.
- [ ] Gate and save in manual stop, runtime upgrade and stop-all paths; do not save on detached GUI quit or unexpected process exit.
- [ ] Persist only necessary additive state; recover scratch/fingerprint context deterministically after supervisor restart.
- [ ] Surface runtime checkpoint status to GUI reconciliation and events.
- [ ] Run runtime service suite, checkpoint suite and Cargo check.
- [ ] Commit as `feat: checkpoint background runtime instances`.

### Task 6: Proxy Readiness and Drain Gate

**Files:**

- Modify: `src-tauri/src/commands/proxy.rs`
- Modify: `src-tauri/src/commands/proxy_runtime.rs`
- Modify: `src-tauri/src/runtime_service/supervisor.rs`
- Modify: `scripts/test-kv-checkpoint.cjs`

**Deliverable:** restoring/draining targets are absent from both active probing and request resolution; `/live` and `/ready` keep distinct semantics.

- [ ] Add a delayed-restore integration test where engine health is 200 but proxy readiness remains 503.
- [ ] Add a request-race test proving no inference request reaches the fake upstream before restore completion.
- [ ] Add multi-target test proving another ready instance remains routable while one restores.
- [ ] Extend `ProxyRuntimeSnapshot` with or derive a filtered routable running map for both Tauri and runtime sources.
- [ ] Apply the same gate inside custom `resolve_proxy_request`; snapshot filtering alone is insufficient.
- [ ] On draining, remove target before waiting for in-flight/slot idle state.
- [ ] Return bounded, retryable 503 metadata without filesystem details when all matching targets are gated.
- [ ] Run proxy Rust tests, router regressions and checkpoint integration tests.
- [ ] Commit as `feat: gate routing on checkpoint readiness`.

### Task 7: Backend Commands and Frontend Controls

**Files:**

- Modify: `src-tauri/src/checkpoint.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src/store/types.ts`
- Modify: `src/store/instanceSlice.ts`
- Modify: relevant `src/components/ConfigPage/*`
- Modify: `src/components/InstanceManager.tsx`
- Modify: `src/i18n.ts`
- Modify: `scripts/test-kv-checkpoint.cjs`
- Modify: relevant component/i18n/theme checks

**Deliverable:** opt-in config, eligibility explanation, live status and safe per-instance clear operation.

- [ ] Add backend tests for get-status/list and exact-instance clear; reject clear while running/saving/restoring.
- [ ] Register additive Tauri commands and runtime-aware status retrieval.
- [ ] Add failing frontend tests for old defaults, field bounds and each user-visible phase.
- [ ] Add the experimental config section and proxy requirement explanation.
- [ ] Show latest save/restore tokens, bytes, duration and cold fallback reason in instance management.
- [ ] Add clear confirmation; never expose arbitrary restore/file picker.
- [ ] Add Chinese and English strings and extend component budget/theme/i18n guards.
- [ ] Run frontend regression, TypeScript, encoding, i18n, component and theme checks.
- [ ] Commit as `feat: expose kv checkpoint controls`.

### Task 8: Deterministic Integration and Fault Injection

**Files:**

- Create or modify Rust tests colocated with `src-tauri/src/checkpoint.rs` and proxy/runtime modules
- Modify: `scripts/test-kv-checkpoint.cjs`
- Modify implementation only when tests expose defects

**Deliverable:** no-model end-to-end proof of ordering, atomicity and fail-open behavior across both managers.

- [ ] Build an Axum fake llama-server with controllable health, slot state, save/restore delay and malformed responses.
- [ ] Prove `STARTING -> ENGINE_HEALTHY -> RESTORING -> READY` order and proxy exclusion.
- [ ] Prove mismatch/no-checkpoint/disabled paths become ready-cold or preserve legacy behavior as designed.
- [ ] Inject corruption, truncation, unreadable files, unwritable store, timeout and process exit at every operation boundary.
- [ ] Prove partial restore always attempts erase and no failed save replaces the previous generation.
- [ ] Prove stop ordering is gate -> drain -> save/skip -> terminate for direct and runtime paths.
- [ ] Prove no checkpoint content, API key or raw prompt is written to logs/status.
- [ ] Run full Rust tests and all Node regressions.
- [ ] Commit as `test: verify kv checkpoint failure safety`.

### Task 9: Real llama-server and DeepSeek Harness Acceptance

**Files:**

- Create test helpers only if they are deterministic and appropriate to keep; otherwise retain commands/results in PR evidence, not the repository
- Modify user documentation after behavior is verified

**Deliverable:** measured cross-restart cache hit on a real supported model and final Harness routing proof.

- [ ] Locate a user-approved/local test GGUF and supported llama-server; do not alter active user sessions.
- [ ] Record exact engine hash/version/backend, model hash, config and baseline cold request metrics.
- [ ] Populate a long prefix through the manager proxy and perform controlled stop.
- [ ] Verify committed manifest and payload checksum without exposing prompt-derived bytes.
- [ ] Restart unchanged, poll proxy readiness, repeat the prefix and capture actual `cache_n`/prompt progress and TTFT.
- [ ] Change a fingerprinted config field and prove cold fallback without restore.
- [ ] Corrupt a disposable copied generation and prove pre-restore rejection plus cold availability.
- [ ] Point DeepSeek Harness at the manager proxy and repeat the same-context new-session path; prove the request occurs after ready and benefits from cache reuse.
- [ ] Restore/leave user runtime configuration in its original state and remove disposable checkpoint/test data.

### Task 10: Documentation, Full Validation, PR, and Fresh-Master Acceptance

**Files:**

- Modify: `README.md`
- Modify: `GUIDE.md`
- Modify: `docs/LLAMA_CPP_COMPATIBILITY.md`
- Modify: guide/check scripts if required
- Modify implementation only for defects found during audit

**Deliverable:** release-ready implementation merged through the repository rules with a clean checkout.

- [ ] Document supported matrix, proxy requirement, sensitive local storage, failure behavior, capacity and clear operation in Chinese/English surfaces.
- [ ] Run `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check`.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml --locked`.
- [ ] Run `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked -- -D warnings` with only the repository-required temporary ignored resources, then remove them.
- [ ] Run `npm run check:release` and `npm run build`.
- [ ] Run `git diff --check`, encoding checks and a focused security/privacy audit.
- [ ] Apply the post-task cleanup checklist: remove temp checkpoints, fake engines, models, logs, reports, screenshots and helper artifacts; prove only intended changes remain.
- [ ] Push the implementation branch and open a PR to `master`.
- [ ] Resolve all review conversations and wait for `quality`, `build-windows`, `build-macos`, `build-linux` and `build-linux-arm64` on the current head.
- [ ] Update from `master` if required and rerun affected validation.
- [ ] Merge through GitHub; never push directly to `master`.
- [ ] Pull fresh `master`, verify the merge commit, required post-merge CI, clean worktree and one final real checkpoint smoke test.
