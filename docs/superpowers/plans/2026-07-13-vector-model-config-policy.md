# Vector Model Configuration Policy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Guarantee that every vector-model instance is created, persisted, previewed, and launched with a clean embedding/reranking configuration and no inference-only parameters.

**Architecture:** Add a pure TypeScript workload classifier and normalizer as the frontend policy authority, plus a matching Rust normalizer at the IPC/launch boundary. All UI and store entry points consume the policy; explicit field coverage tests and command-level Rust tests prevent future bypasses.

**Tech Stack:** React 18, TypeScript 5, Zustand 5, Tauri 2, Rust, serde, Node.js regression scripts, esbuild.

## Global Constraints

- New vector instances are clean by construction and do not show a cleanup notification.
- Existing instances switched to vector models are normalized atomically and show a grouped cleanup summary.
- `custom_args` is unavailable and empty in vector mode.
- Generic embedding models use model-default pooling unless the user selected a valid pooling mode; rerankers force `pooling=rank`.
- `batch_size` must be less than or equal to `ubatch_size` in vector mode.
- Preview and launch must use the same normalized configuration.
- Existing inference and MTP behavior must remain unchanged.
- Do not add runtime dependencies.

---

### Task 1: Pure Frontend Vector Policy

**Files:**
- Create: `src/modelPolicy.ts`
- Create: `scripts/test-vector-model-policy.cjs`
- Modify: `src/store/types.ts`
- Modify: `package.json`

**Interfaces:**
- Consumes: `InstanceConfig`, `ModelInfo`, and `defaultInstanceConfig()`.
- Produces: `detectModelWorkload(model, modelPath)`, `normalizeInstanceConfig(config, model)`, `VECTOR_ALLOWED_FIELDS`, `VectorCleanupChange`, and `ModelWorkload`.

- [ ] **Step 1: Add a failing bundled TypeScript policy test**

Create a Node script that uses the existing `esbuild` dependency to bundle an in-memory test entry. The entry imports `src/modelPolicy.ts`, uses `node:assert`, and verifies capability/filename classification, reranker pooling, polluted MTP/custom-argument cleanup, batch normalization, and complete key classification.

```js
const entry = `
  import assert from 'node:assert/strict'
  import { defaultInstanceConfig } from './src/store/defaults'
  import { detectModelWorkload, normalizeInstanceConfig, VECTOR_CLASSIFIED_FIELDS } from './src/modelPolicy'
  const polluted = { ...defaultInstanceConfig(), embedding: true, spec_type: 'draft-mtp', custom_args: ['--spec-type draft-mtp'], batch_size: 2048, ubatch_size: 512 }
  const result = normalizeInstanceConfig(polluted, null)
  assert.equal(result.config.spec_type, '')
  assert.deepEqual(result.config.custom_args, [])
  assert.equal(result.config.batch_size, 512)
  assert.deepEqual(new Set(Object.keys(defaultInstanceConfig())), VECTOR_CLASSIFIED_FIELDS)
`
```

- [ ] **Step 2: Run the policy test and verify it fails**

Run: `node scripts/test-vector-model-policy.cjs`

Expected: FAIL because `src/modelPolicy.ts` does not exist.

- [ ] **Step 3: Implement the pure policy and capability types**

Add optional `is_embedding_model` and `is_reranker_model` fields to `ModelCapabilities`. Implement conservative metadata-first classification and an explicit allowed-field set. Derive incompatible keys from `defaultInstanceConfig()` and reset them to defaults.

```ts
export type ModelWorkload = 'inference' | 'embedding' | 'reranker'

export interface VectorCleanupChange {
  key: keyof InstanceConfig
  group: 'speculative' | 'generation' | 'chat' | 'multimodal' | 'custom' | 'runtime'
  before: unknown
  after: unknown
}

export function normalizeInstanceConfig(config: InstanceConfig, model?: ModelInfo | null) {
  const workload = detectModelWorkload(model, config.model_path, config)
  if (workload === 'inference') return { config: { ...config }, workload, vectorMode: false, changes: [] }
  const defaults = defaultInstanceConfig()
  const next = { ...config, embedding: true }
  for (const key of VECTOR_INCOMPATIBLE_FIELDS) next[key] = defaults[key] as never
  if (workload === 'reranker') { next.reranking = true; next.pooling = 'rank' }
  if (next.batch_size > next.ubatch_size) next.batch_size = next.ubatch_size
  return { config: next, workload, vectorMode: true, changes: diffVectorCleanup(config, next) }
}
```

- [ ] **Step 4: Add the regression script to the package test chain**

Add `node scripts/test-vector-model-policy.cjs` to `test:regressions` before cross-platform tests.

- [ ] **Step 5: Run policy tests and TypeScript checks**

Run: `node scripts/test-vector-model-policy.cjs && npx tsc --noEmit`

Expected: policy assertions pass and TypeScript exits with code 0.

- [ ] **Step 6: Commit the frontend policy**

```bash
git add src/modelPolicy.ts src/store/types.ts scripts/test-vector-model-policy.cjs package.json
git commit -m "feat: add vector configuration policy"
```

### Task 2: Rust Model Classification and Launch Normalization

**Files:**
- Create: `src-tauri/src/vector_policy.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/models.rs`
- Modify: `src-tauri/src/utils.rs`
- Modify: `src-tauri/src/commands/server.rs`
- Modify: `src-tauri/src/commands/config.rs`

**Interfaces:**
- Consumes: `InstanceConfig`, GGUF architecture metadata, and model path.
- Produces: `classify_model_workload(architecture, path)`, `normalize_for_vector(config)`, and `normalize_for_launch(config)`.

- [ ] **Step 1: Add failing Rust policy and command tests**

Add tests for embedding/reranker classification and a deliberately polluted embedding config containing `draft-mtp`, draft KV cache, chat, sampling, media, agent/tool, slot prompt, and custom arguments.

```rust
#[test]
fn embedding_command_rejects_all_inference_only_flags() {
    let mut config = cfg();
    config.embedding = true;
    config.spec_type = "draft-mtp".into();
    config.custom_args = vec!["--spec-type draft-mtp".into(), "--temp 1.5".into()];
    let cmd = generate_command(&config, "");
    for forbidden in ["--spec-type", "--temp", "--draft-model", "-ctkd"] {
        assert!(!cmd.iter().any(|arg| arg == forbidden), "leaked {forbidden}");
    }
}
```

- [ ] **Step 2: Run the focused Rust tests and verify at least one fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml embedding_command_rejects_all_inference_only_flags -- --exact`

Expected: FAIL because custom arguments leak `--spec-type` and `--temp`.

- [ ] **Step 3: Implement Rust workload classification and config normalization**

Create `vector_policy.rs`, add the module in `main.rs`, and update scanner capabilities. The normalizer must set embedding/reranking invariants, reset every inference-only field, clear custom arguments, and reduce batch to ubatch.

```rust
pub fn normalize_for_launch(mut config: InstanceConfig) -> VectorNormalization {
    let detected = classify_path(&config.model_path);
    let vector_mode = config.embedding || config.reranking || detected.is_vector();
    if !vector_mode { return VectorNormalization::unchanged(config); }
    let defaults = InstanceConfig::default();
    config.embedding = true;
    config.spec_type = defaults.spec_type;
    config.draft_model_path = defaults.draft_model_path;
    config.custom_args.clear();
    if config.batch_size > config.ubatch_size { config.batch_size = config.ubatch_size; }
    VectorNormalization::from_changes(config)
}
```

- [ ] **Step 4: Route preview, start, and save through the normalizer**

Normalize before both `generate_server_command` and `start_server` call `generate_command`. Normalize the instance map in `save_config` before persistence and state replacement. Ensure `generate_command` itself also has vector gates so direct unit calls cannot leak flags.

- [ ] **Step 5: Run Rust policy tests, all tests, and Clippy**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked -- -D warnings
```

Expected: all tests pass and Clippy emits no warnings.

- [ ] **Step 6: Commit the backend defense**

```bash
git add src-tauri/src/vector_policy.rs src-tauri/src/main.rs src-tauri/src/models.rs src-tauri/src/utils.rs src-tauri/src/commands/server.rs src-tauri/src/commands/config.rs
git commit -m "fix: enforce vector launch configuration"
```

### Task 3: Store Reconciliation, Save, Preview, and Start

**Files:**
- Modify: `src/store/instanceSlice.ts`
- Modify: `src/store/bootstrap.ts`
- Modify: `src/store/coreSlice.ts`
- Modify: `scripts/test-vector-model-policy.cjs`

**Interfaces:**
- Consumes: `normalizeInstanceConfig(config, model)`.
- Produces: normalized instance creation/save/preview/start inputs and one-time bootstrap migration persistence.

- [ ] **Step 1: Extend the regression script with store-boundary assertions**

Assert that `instanceSlice.ts` normalizes before `generate_server_command`, `start_server`, and `save_config`, and that bootstrap/core model loads reconcile instances after model inventory becomes available.

- [ ] **Step 2: Run the regression script and verify it fails**

Run: `node scripts/test-vector-model-policy.cjs`

Expected: FAIL on missing store integration markers.

- [ ] **Step 3: Add store normalization helpers**

Implement a helper that matches models by normalized path, normalizes each instance, updates Zustand only when values changed, and returns whether persistence is required.

```ts
function normalizeStoredConfig(config: InstanceConfig, models: ModelInfo[]) {
  const model = models.find(item => normalizePath(item.path) === normalizePath(config.model_path))
  return normalizeInstanceConfig(config, model)
}
```

- [ ] **Step 4: Normalize all persistence and execution paths**

Use normalized configuration for command preview, manual/dashboard/direct start, auto-start, and save serialization. After model scanning/bootstrap, reconcile legacy instances and enqueue one save only when at least one config changed.

- [ ] **Step 5: Run regression and TypeScript checks**

Run: `npm run test:regressions && npx tsc --noEmit`

Expected: all scripts pass and TypeScript exits with code 0.

- [ ] **Step 6: Commit store integration**

```bash
git add src/store/instanceSlice.ts src/store/bootstrap.ts src/store/coreSlice.ts scripts/test-vector-model-policy.cjs
git commit -m "fix: normalize vector instances across store paths"
```

### Task 4: New Instance and Existing Model-Switch Flows

**Files:**
- Modify: `src/components/InstanceManager.tsx`
- Modify: `src/components/ConfigPage.tsx`
- Modify: `src/i18n/zh-CN.ts`
- Modify: `src/i18n/en-US.ts`
- Modify: `scripts/test-vector-model-policy.cjs`

**Interfaces:**
- Consumes: workload detection and normalization result changes.
- Produces: clean-by-construction new instances and grouped existing-instance cleanup summaries.

- [ ] **Step 1: Add integration assertions for both user flows**

Check that `handleCreate` normalizes after applying identity fields and that primary model selection in `ConfigPage` normalizes a single candidate object and stores cleanup changes.

- [ ] **Step 2: Run the regression script and verify it fails**

Run: `node scripts/test-vector-model-policy.cjs`

Expected: FAIL because creation and model switching do not yet invoke the policy.

- [ ] **Step 3: Normalize new instance creation without notification**

Build the default config, apply model/engine/port values, call `normalizeInstanceConfig(config, model)`, and pass `result.config` to `addInstance`.

- [ ] **Step 4: Normalize existing model switches atomically**

Replace sequential `set('model_path')` calls with one candidate object containing model and projector updates. Normalize it and set both local config and cleanup summary in one state update.

- [ ] **Step 5: Render a grouped cleanup summary**

Add bilingual text for removed speculative, generation, chat, multimodal, custom, and runtime adjustments. Show the summary only for an existing-instance switch, clear it after save or another non-vector switch, and never expose custom-argument values.

- [ ] **Step 6: Run regression, encoding, and build checks**

Run: `node scripts/test-vector-model-policy.cjs && npm run check:encoding && npm run build`

Expected: all checks pass.

- [ ] **Step 7: Commit both user flows**

```bash
git add src/components/InstanceManager.tsx src/components/ConfigPage.tsx src/i18n/zh-CN.ts src/i18n/en-US.ts scripts/test-vector-model-policy.cjs
git commit -m "fix: clean vector configs during create and model switch"
```

### Task 5: Vector-Only Configuration UI

**Files:**
- Modify: `src/components/ConfigPage.tsx`
- Modify: `src/components/ConfigPage/sections.tsx`
- Modify: `src/components/ConfigPage/activeParams.ts`
- Modify: `scripts/test-vector-model-policy.cjs`

**Interfaces:**
- Consumes: `vectorMode`, allowed-field policy, and normalized local config.
- Produces: a UI that renders only vector-specific and supported shared groups in vector mode.

- [ ] **Step 1: Add UI integration assertions**

Assert that inference-only sections and presets are filtered using policy data, not independent local heuristics, and that the custom argument editor is absent in vector mode.

- [ ] **Step 2: Run the regression script and verify it fails**

Run: `node scripts/test-vector-model-policy.cjs`

Expected: FAIL because current controls only use scattered `disabled={isEmbedding}` checks.

- [ ] **Step 3: Replace local embedding detection with the central workload result**

Remove `EMBED_ARCHS` and the auto-enable effect from `ConfigPage`. Use `detectModelWorkload` and normalized local state as the sole UI source.

- [ ] **Step 4: Filter sections, presets, active parameters, and search**

Do not render reasoning, sampling, speculative, model adaptation, multimodal, router, agent/tool, slot prompt, or custom-argument controls in vector mode. Keep identity, vector, hardware, execution, network, cluster/RPC, and observability controls. Ensure quick presets contain no inference-only changes.

- [ ] **Step 5: Run regression and frontend build checks**

Run: `npm run test:regressions && npm run build`

Expected: all tests and Vite production build pass.

- [ ] **Step 6: Commit UI enforcement**

```bash
git add src/components/ConfigPage.tsx src/components/ConfigPage/sections.tsx src/components/ConfigPage/activeParams.ts scripts/test-vector-model-policy.cjs
git commit -m "fix: restrict vector configuration interface"
```

### Task 6: Complete Verification and Audit

**Files:**
- Modify only if verification exposes a defect.

**Interfaces:**
- Consumes: all previous tasks.
- Produces: release-ready evidence and a clean working tree.

- [ ] **Step 1: Run the full frontend release suite**

Run: `npm run check:release && npm run build`

Expected: all release scripts, regression tests, TypeScript checks, and production build pass.

- [ ] **Step 2: Run the complete Rust suite**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked -- -D warnings
```

Expected: all tests pass and no warnings are emitted.

- [ ] **Step 3: Inspect generated command invariants**

Confirm tests cover generic embedding, reranking, polluted MTP, custom arguments, legacy `embedding=false` detection, batch mismatch, and unaffected inference MTP output.

- [ ] **Step 4: Run repository hygiene checks**

Run: `git diff --check && git status --short`

Expected: no whitespace errors and only intended implementation files are modified.

- [ ] **Step 5: Perform a final focused code review**

Review the diff for bypasses in create, switch, save, preview, dashboard start, direct start, and auto-start. Fix any finding and rerun the affected checks.
