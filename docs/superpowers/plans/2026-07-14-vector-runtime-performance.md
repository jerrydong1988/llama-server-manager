# Vector Runtime Performance Monitoring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 Embedding 与 Reranker 实例提供准确、来源明确且不泄露请求内容的运行性能监控，同时保持文本生成监控与旧遥测数据库兼容。

**Architecture:** 在既有进程和 `/metrics` 采样之外，增加工作负载感知的遥测会话、日志任务事件和代理 HTTP 事件。Rust 查询层按 `log` 与 `proxy` 来源分别聚合业务指标，前端根据会话工作负载选择指标、趋势、诊断和历史基线，绝不把 decode 调用数冒充 HTTP 请求数。

**Tech Stack:** Tauri 2、Rust、rusqlite、Axum 0.7、reqwest、regex-lite、React 18、TypeScript 5、Zustand 5、Node.js 回归脚本、esbuild、SQLite。

## Global Constraints

- 第一阶段只实现向量服务运行性能；不实现 Recall@K、MRR、NDCG、测试集管理或主动压测。
- 不修改或替换用户选择的 `llama-server`，直连流量必须可通过现有日志采集得到任务项指标。
- `log` 与 `proxy` 是不同统计口径；代理请求产生两类事件时，不得相加为同一个请求或项目总数。
- 不保存请求正文、查询、文档、token 序列、模型输出或向量；仅保存类型、计数、时间、状态和脱敏错误。
- 新会话工作负载来自规范化后的启动配置；历史会话按自身记录展示，不随实例后续换模漂移。
- 旧数据库迁移必须幂等；旧 TypeScript/Rust 字段保持可反序列化；旧 `requests_total` 仅兼容保留，不再显示为请求数。
- Embedding、Reranker 和 inference 的历史基线不得交叉比较。
- 缺少日志事件或代理事件时必须显示“不可用”或来源说明，不得用 `0` 冒充已测量结果。
- 生成模型现有采集、活动请求、趋势、诊断和大屏行为不得回退。
- 不增加新的运行时依赖；数据库事件写入失败不能终止实例或代理服务。
- 所有手工文件编辑使用 `apply_patch`，每个任务结束后运行对应测试并提交小范围 commit。

---

### Task 1: Workload Serialization and Telemetry Schema Migration

**Files:**
- Modify: `src-tauri/src/vector_policy.rs`
- Modify: `src-tauri/src/commands/telemetry.rs`

**Interfaces:**
- Consumes: `ModelWorkload`, normalized launch configuration, existing telemetry database versions 1-4.
- Produces: stable storage values `inference | embedding | reranker`, schema version 5, workload-aware sessions, vector event storage, explicit decode sample fields.

- [ ] **Step 1: Add failing workload and migration tests**

Add tests for stable workload strings, historical command-line inference, fresh schema creation, version-4 migration, repeated migration, workload backfill, indexes, uniqueness, and session cascade deletion.

```rust
#[test]
fn workload_storage_round_trip_is_stable() {
    assert_eq!(ModelWorkload::Embedding.as_str(), "embedding");
    assert_eq!(ModelWorkload::from_storage("reranker"), ModelWorkload::Reranker);
    assert_eq!(ModelWorkload::from_command_line("llama-server --embedding"), ModelWorkload::Embedding);
    assert_eq!(ModelWorkload::from_command_line("llama-server --embedding --reranking"), ModelWorkload::Reranker);
}

#[test]
fn version_four_database_migrates_vector_schema_once() {
    let conn = version_four_connection();
    init_schema(&conn).unwrap();
    init_schema(&conn).unwrap();
    assert_eq!(schema_version(&conn), 5);
    assert_eq!(session_workload(&conn, "rerank-session"), "reranker");
    assert!(has_index(&conn, "idx_vector_activity_session_completed"));
}
```

- [ ] **Step 2: Run focused Rust tests and verify failure**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked vector_policy::tests::workload_storage_round_trip_is_stable
cargo test --manifest-path src-tauri/Cargo.toml --locked commands::telemetry::tests::version_four_database_migrates_vector_schema_once
```

Expected: FAIL because workload serialization and schema version 5 do not exist.

- [ ] **Step 3: Add stable workload conversion helpers**

Extend `ModelWorkload` without changing current classification precedence.

```rust
impl ModelWorkload {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inference => "inference",
            Self::Embedding => "embedding",
            Self::Reranker => "reranker",
        }
    }

    pub fn from_storage(value: &str) -> Self { /* unknown -> inference */ }
    pub fn from_command_line(command_line: &str) -> Self { /* reranking before embedding */ }
}
```

- [ ] **Step 4: Implement idempotent schema version 5**

Increase `SCHEMA_VERSION` to 5. Add `run_sessions.workload`, `metric_samples.decode_calls_total`, `metric_samples.max_tokens_observed`, and `vector_activity_events`. Use column-existence checks before `ALTER TABLE`, backfill session workload from `command_line`, create both event indexes with `IF NOT EXISTS`, and retain `requests_total` unchanged for compatibility.

```sql
CREATE TABLE IF NOT EXISTS vector_activity_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    source TEXT NOT NULL CHECK(source IN ('log', 'proxy')),
    source_event_id INTEGER NOT NULL,
    workload TEXT NOT NULL CHECK(workload IN ('embedding', 'reranker')),
    endpoint TEXT,
    started_at INTEGER NOT NULL,
    completed_at INTEGER NOT NULL,
    duration_ms REAL NOT NULL,
    item_count INTEGER NOT NULL DEFAULT 1 CHECK(item_count >= 0),
    input_tokens INTEGER CHECK(input_tokens IS NULL OR input_tokens >= 0),
    http_status INTEGER,
    error_text TEXT,
    UNIQUE(session_id, source, source_event_id),
    FOREIGN KEY(session_id) REFERENCES run_sessions(id) ON DELETE CASCADE
);
```

- [ ] **Step 5: Keep pruning and deletion lifecycle complete**

Verify old-session deletion cascades to vector events. Extend active-session retention pruning to delete vector events older than the cutoff, matching the current behavior for metric samples and request rows.

- [ ] **Step 6: Run migration, retention, and complete Rust tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked commands::telemetry
cargo test --manifest-path src-tauri/Cargo.toml --locked vector_policy
```

Expected: all focused tests pass; repeated migration leaves one workload column and one copy of each index.

- [ ] **Step 7: Commit schema and workload storage**

```bash
git add src-tauri/src/vector_policy.rs src-tauri/src/commands/telemetry.rs
git commit -m "feat: add workload-aware telemetry schema"
```

### Task 2: Pure Vector Event Aggregation

**Files:**
- Create: `src-tauri/src/commands/vector_metrics.rs`
- Modify: `src-tauri/src/commands/mod.rs`

**Interfaces:**
- Consumes: completed vector events, time range, bucket width, source selector.
- Produces: sanitized percentiles, fixed time buckets, log-task summary, proxy-HTTP summary, explicit source availability.

- [ ] **Step 1: Add failing aggregation tests**

Cover empty input, boundary timestamps, concurrent completions, empty buckets, P50/P95, even sample counts, invalid durations, source separation, and input token absence.

```rust
#[test]
fn buckets_sum_concurrent_completions_and_fill_idle_intervals() {
    let events = vec![
        event("log", 10_100, 2, Some(20)),
        event("log", 10_900, 3, Some(30)),
        event("log", 20_100, 1, Some(5)),
    ];
    let buckets = aggregate_buckets(&events, 10_000, 40_000, 10_000);
    assert_eq!(buckets.iter().map(|b| b.items_per_second).collect::<Vec<_>>(), vec![0.5, 0.1, 0.0]);
    assert_eq!(buckets[0].input_tokens_per_second, Some(5.0));
}

#[test]
fn log_and_proxy_summaries_do_not_double_count() {
    let summary = summarize(&[log_event(4), proxy_event(4, 200)], 10_000);
    assert_eq!(summary.log.completed_items, 4);
    assert_eq!(summary.proxy.request_count, 1);
}
```

- [ ] **Step 2: Run focused test and verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked commands::vector_metrics`

Expected: FAIL because the module does not exist.

- [ ] **Step 3: Implement small pure data types**

```rust
pub(crate) enum VectorEventSource { Log, Proxy }

pub(crate) struct VectorEventPoint {
    pub source: VectorEventSource,
    pub completed_at: i64,
    pub duration_ms: f64,
    pub item_count: u64,
    pub input_tokens: Option<u64>,
    pub http_status: Option<u16>,
    pub has_error: bool,
}

pub(crate) struct VectorTrendBucket {
    pub timestamp: i64,
    pub input_tokens_per_second: Option<f64>,
    pub items_per_second: f64,
}
```

Provide a nearest-rank percentile helper with an explicit documented rule. Reject negative/non-finite durations and invalid ranges before aggregation. Preserve `None` for input throughput when no token samples exist.

- [ ] **Step 4: Implement source-specific summaries**

Log summary owns completed items, input tokens, item throughput and task P50/P95. Proxy summary owns HTTP request count, proxy item count, success/failure counts, success/failure rates and HTTP P50/P95. Do not expose a combined total.

- [ ] **Step 5: Run focused tests and Clippy for the module**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked commands::vector_metrics
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked -- -D warnings
```

Expected: tests pass and no dead-code or float warnings remain.

- [ ] **Step 6: Commit aggregation primitives**

```bash
git add src-tauri/src/commands/vector_metrics.rs src-tauri/src/commands/mod.rs
git commit -m "feat: aggregate vector runtime metrics"
```

### Task 3: Workload-Aware Session Start and Common Samples

**Files:**
- Modify: `src-tauri/src/commands/server.rs`
- Modify: `src-tauri/src/commands/telemetry.rs`
- Modify: `src-tauri/src/models.rs`

**Interfaces:**
- Consumes: `normalize_for_launch(config).workload`, `llamacpp:n_decode_total`, `llamacpp:n_tokens_max`.
- Produces: workload-pinned run sessions, restored workload context, unambiguous common sample fields.

- [ ] **Step 1: Add failing session and metrics parser tests**

Assert that a reranker session persists `reranker`, an embedding restart keeps `embedding`, and Prometheus parsing maps `n_decode_total` to `decode_calls_total` and `n_tokens_max` to `max_tokens_observed` without calling either an HTTP request count.

- [ ] **Step 2: Run focused tests and verify failure**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked commands::server::tests::vector_workload_is_pinned_to_run_session
cargo test --manifest-path src-tauri/Cargo.toml --locked commands::server::tests::llama_metrics_use_unambiguous_decode_names
```

Expected: FAIL on missing workload argument and sample fields.

- [ ] **Step 3: Thread workload through all session creation paths**

Retain the `VectorNormalization` result in `start_server` instead of immediately calling `.into_config()`. Change `begin_run_session` to accept `ModelWorkload`. Pass the same value into log collection and store it on `RunningInstance` only if restoration needs it; prefer the persisted session value when reconnecting an existing telemetry session.

```rust
let normalized = normalize_for_launch(config);
let workload = normalized.workload;
let config = normalized.into_config();
let session_id = telemetry::begin_run_session(&app, &config, workload, &command_line)?;
```

Audit manual start, auto-start, restored-running-instance and log-tail attachment paths so no call site silently defaults a vector launch to inference.

- [ ] **Step 4: Rename new common sample semantics**

Extend `LlamaMetricSample`, insert/query structs and serialized summaries with `decode_calls_total` and `max_tokens_observed`. Continue populating legacy `requests_total` only as required for old rows, but stop using its label or value in new frontend analysis.

- [ ] **Step 5: Run server and telemetry tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked commands::server
cargo test --manifest-path src-tauri/Cargo.toml --locked commands::telemetry
```

Expected: all tests pass and inference session creation remains unchanged.

- [ ] **Step 6: Commit workload propagation**

```bash
git add src-tauri/src/commands/server.rs src-tauri/src/commands/telemetry.rs src-tauri/src/models.rs
git commit -m "feat: propagate workload into telemetry sessions"
```

### Task 4: Vector Log Task Capture and Idempotent Persistence

**Files:**
- Modify: `src-tauri/src/commands/server.rs`
- Modify: `src-tauri/src/commands/telemetry.rs`

**Interfaces:**
- Consumes: representative current `llama-server` task launch, `stop processing: n_tokens = N`, release and cancellation logs.
- Produces: one `source=log` vector event per completed task item, local task duration, input token count, replay-safe source event IDs.

- [ ] **Step 1: Add representative failing parser tests**

Use complete log fixtures for a batch Embedding request, a multi-document Reranker request, a cancelled task, a replayed line, and the existing inference log fixture.

```rust
#[test]
fn embedding_child_tasks_emit_one_log_event_each() {
    let mut parser = PerfParser::new(ModelWorkload::Embedding);
    let events = feed(&mut parser, EMBEDDING_BATCH_LOG);
    assert_eq!(events.len(), 3);
    assert_eq!(events.iter().map(|e| e.item_count).sum::<u64>(), 3);
    assert_eq!(events.iter().map(|e| e.input_tokens.unwrap()).sum::<u64>(), 96);
}

#[test]
fn inference_fixture_does_not_emit_vector_events() {
    let mut parser = PerfParser::new(ModelWorkload::Inference);
    assert!(feed(&mut parser, EXISTING_GENERATION_LOG).vector_events().is_empty());
}
```

- [ ] **Step 2: Run parser tests and verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked commands::server::tests::embedding_child_tasks_emit_one_log_event_each`

Expected: FAIL because `PerfParser` has no workload-aware vector event.

- [ ] **Step 3: Extend parser state conservatively**

Track task ID, slot ID, local `Instant`/epoch start, optional input token count and terminal state. The parser must emit only after a valid vector task completion. Do not infer HTTP batch boundaries from child tasks. Bound retained pending tasks and clear terminal/cancelled entries to avoid unbounded memory on malformed logs.

- [ ] **Step 4: Add idempotent event recording**

Introduce `VectorActivityRecord` and `record_vector_activity`. Use `(session_id, source, source_event_id)` uniqueness with `INSERT OR IGNORE`; return whether a row was inserted so tests can prove replay safety. Sanitize error text through the existing telemetry error boundary.

```rust
pub(crate) struct VectorActivityRecord<'a> {
    pub session_id: &'a str,
    pub source: VectorEventSource,
    pub source_event_id: i64,
    pub workload: ModelWorkload,
    pub endpoint: Option<&'a str>,
    pub started_at: i64,
    pub completed_at: i64,
    pub duration_ms: f64,
    pub item_count: u64,
    pub input_tokens: Option<u64>,
    pub http_status: Option<u16>,
    pub error_text: Option<&'a str>,
}
```

- [ ] **Step 5: Keep collection resilient**

On one database write failure, write a concise server log message and continue parsing subsequent lines. Ensure stopped instances flush or discard unfinished parser state without synthetic completed events.

- [ ] **Step 6: Run parser, persistence, and generation regression tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked commands::server
cargo test --manifest-path src-tauri/Cargo.toml --locked commands::telemetry
```

Expected: vector fixtures pass, replay inserts once, and existing inference request timing tests remain green.

- [ ] **Step 7: Commit log collection**

```bash
git add src-tauri/src/commands/server.rs src-tauri/src/commands/telemetry.rs
git commit -m "feat: collect vector task activity from logs"
```

### Task 5: Vector Proxy Endpoints, Item Counting, and HTTP Events

**Files:**
- Modify: `src-tauri/src/commands/proxy.rs`
- Modify: `src-tauri/src/commands/telemetry.rs`

**Interfaces:**
- Consumes: vector proxy path, JSON request body, resolved target workload, full upstream response/failure.
- Produces: forwarded Embedding/Reranker aliases and one source-specific proxy event containing only counts, timing, status and sanitized error.

- [ ] **Step 1: Add failing endpoint and item-count tests**

Cover all seven aliases, unsupported paths, Embedding string/array/token-array inputs, Reranker `documents`, malformed/non-JSON bodies, status failures, body-stream failures and privacy assertions.

```rust
#[test]
fn vector_endpoint_classification_covers_supported_aliases() {
    for path in ["/embedding", "/embeddings", "/v1/embeddings"] {
        assert_eq!(classify_vector_endpoint(path), Some(ModelWorkload::Embedding));
    }
    for path in ["/rerank", "/reranking", "/v1/rerank", "/v1/reranking"] {
        assert_eq!(classify_vector_endpoint(path), Some(ModelWorkload::Reranker));
    }
}

#[test]
fn proxy_event_never_contains_request_content() {
    let event = vector_proxy_event(RERANK_BODY_WITH_PRIVATE_TEXT);
    assert_eq!(event.item_count, 3);
    assert!(!format!("{event:?}").contains("private document"));
}
```

- [ ] **Step 2: Run focused proxy tests and verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked commands::proxy::tests::vector_endpoint_classification_covers_supported_aliases`

Expected: FAIL because only the existing OpenAI route set is registered.

- [ ] **Step 3: Add route classification and body count helpers**

Parse JSON into `serde_json::Value` only long enough to count top-level input/documents items. Treat a single string or one token array as one Embedding item; treat an array of strings/token arrays as its length; treat Reranker documents array length as item count. Return `None` when the count is unknowable rather than inventing zero. Drop parsed values immediately and never add request bodies to logs or errors.

- [ ] **Step 4: Register aliases through the existing proxy handler**

Route all aliases through the same authorization, target selection, timeout, headers and forwarding behavior as current API endpoints. Preserve the original path sent upstream. Reject a workload/path mismatch explicitly instead of forwarding a rerank request to an Embedding-only instance.

- [ ] **Step 5: Record complete proxy lifecycle**

Allocate proxy source IDs independently from log task IDs. Record start before forwarding and complete only after the upstream response body is fully consumed/forwarded or an error terminates it. Save status and sanitized error. Proxy event write failure must not replace the upstream response.

- [ ] **Step 6: Prove source isolation**

Add an integration-style database test where one proxied vector call also has four log child events. Assert the log summary reports four completed items and the proxy summary reports one HTTP request/four request items, without a combined value of eight.

- [ ] **Step 7: Run proxy and telemetry tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked commands::proxy
cargo test --manifest-path src-tauri/Cargo.toml --locked commands::telemetry
```

Expected: aliases, privacy, failures and source-isolation tests all pass.

- [ ] **Step 8: Commit proxy support**

```bash
git add src-tauri/src/commands/proxy.rs src-tauri/src/commands/telemetry.rs
git commit -m "feat: monitor vector requests through instance proxy"
```

### Task 6: Workload-Aware Telemetry Queries and Diagnostics

**Files:**
- Modify: `src-tauri/src/commands/telemetry.rs`
- Modify: `src-tauri/src/main.rs`

**Interfaces:**
- Consumes: workload-pinned session rows, vector events, metric samples, model/backend identity.
- Produces: serializable vector analysis/trend/source availability, workload-filtered baselines and vector-safe diagnostics.

- [ ] **Step 1: Add failing query contract tests**

Seed in-memory inference, Embedding and Reranker sessions. Cover only-log, only-proxy, full-data, no-business-data, invalid event, same-model/different-workload, same-workload/different-backend and instance-after-model-switch cases.

- [ ] **Step 2: Run focused telemetry query tests and verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked commands::telemetry::tests::vector_analysis_keeps_sources_and_availability_separate`

Expected: FAIL because current analysis is generation-only.

- [ ] **Step 3: Extend response contracts additively**

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorTelemetryAnalysis {
    pub workload: String,
    pub log_available: bool,
    pub proxy_available: bool,
    pub completed_items: Option<u64>,
    pub input_tokens: Option<u64>,
    pub average_input_tokens_per_second: Option<f64>,
    pub average_items_per_second: Option<f64>,
    pub task_duration_p50_ms: Option<f64>,
    pub task_duration_p95_ms: Option<f64>,
    pub proxy_request_count: Option<u64>,
    pub proxy_item_count: Option<u64>,
    pub proxy_duration_p50_ms: Option<f64>,
    pub proxy_duration_p95_ms: Option<f64>,
    pub proxy_success_rate: Option<f64>,
    pub proxy_failure_rate: Option<f64>,
    pub trend: Vec<VectorTrendBucket>,
}
```

Add `workload` to session summaries. Add `vector_analysis: Option<VectorTelemetryAnalysis>` to session analysis rather than removing inference fields, preserving old frontend compatibility during rollout.

- [ ] **Step 4: Query vector events with bounded ranges**

Use the `(session_id, source, completed_at)` index. Clamp bucket/range arguments to existing telemetry limits. Keep `None` for unavailable measurements; a measured idle bucket may contain numeric zero only after at least one source event establishes source availability.

- [ ] **Step 5: Filter historical baselines correctly**

Compare only matching model identity, `workload` and backend. Embedding baseline fields are input tokens/s, vector items/s and task P95; Reranker uses input tokens/s, document items/s and task P95. Existing inference baseline remains unchanged.

- [ ] **Step 6: Gate diagnostics by workload**

Keep resource pressure, queue, CPU/GPU balance, vector throughput regression, task P95 regression and source-incomplete diagnostics. Do not execute generation phase, context/KV cache or speculative decoding findings for vector sessions.

- [ ] **Step 7: Register any new Tauri query command**

Prefer extending `get_telemetry_session_analysis` to avoid an extra round trip. If a dedicated trend command is required for range changes, register it in `main.rs` and document its exact frontend invoke name in the TypeScript task.

- [ ] **Step 8: Run query and full backend tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked commands::telemetry
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

Expected: all vector query states pass and existing inference serialization tests remain compatible.

- [ ] **Step 9: Commit query layer**

```bash
git add src-tauri/src/commands/telemetry.rs src-tauri/src/main.rs
git commit -m "feat: expose vector telemetry analysis"
```

### Task 7: Frontend Workload Presentation Model

**Files:**
- Create: `src/components/PerformancePage/vectorPerformance.ts`
- Create: `scripts/test-vector-performance.cjs`
- Modify: `src/store/types.ts`
- Modify: `package.json`

**Interfaces:**
- Consumes: additive Rust telemetry contracts and current language.
- Produces: workload labels, source availability state, vector KPI rows, trend series, session comparison rows and diagnostic gating.

- [ ] **Step 1: Add a failing bundled TypeScript test**

Follow `scripts/test-vector-model-policy.cjs`: bundle a TypeScript entry with esbuild and assert all three workloads plus only-log, only-proxy, full-data and no-data states.

```ts
assert.deepEqual(
  buildVectorKpis(embeddingAnalysis, 'zh'),
  [
    { key: 'input', value: '42.0 tok/s', available: true },
    { key: 'items', value: '8.0 项/s', available: true },
    { key: 'p95', value: '18 ms', available: true },
  ],
)
assert.equal(buildPerformanceMode(inferenceSession).kind, 'inference')
assert.equal(buildPerformanceMode(noLogVectorSession).inputThroughput, null)
```

- [ ] **Step 2: Run script and verify failure**

Run: `node scripts/test-vector-performance.cjs`

Expected: FAIL because `vectorPerformance.ts` and vector telemetry types do not exist.

- [ ] **Step 3: Add TypeScript contracts matching Rust exactly**

Add `ModelWorkload = 'inference' | 'embedding' | 'reranker'` to shared store types if the existing frontend policy type cannot be imported without a cycle. Extend `TelemetrySessionSummary`, `TelemetrySampleSummary` and `TelemetrySessionAnalysis` additively. Use `number | null` for unavailable numeric values; do not coerce with `|| 0`.

- [ ] **Step 4: Implement pure view-model helpers**

Build workload-specific labels (`生成`, `Embedding`, `Reranker`), KPI selection, trend selection, source-state text, historical comparison rows and active-request column selection. Keep all branching outside the JSX where practical.

- [ ] **Step 5: Add source-level safeguards to regression script**

Assert that vector helpers never read `tokens_per_sec`, speculative acceptance or KV/context fields; assert that request count comes only from `proxy_request_count`; assert no helper labels `requests_total` as a request count.

- [ ] **Step 6: Add test to release chain**

Append `node scripts/test-vector-performance.cjs` to `test:regressions` near the existing vector policy test.

- [ ] **Step 7: Run test, TypeScript and encoding checks**

Run:

```bash
node scripts/test-vector-performance.cjs
npx tsc --noEmit
npm run check:encoding
```

Expected: all checks pass with Chinese source intact.

- [ ] **Step 8: Commit frontend model**

```bash
git add src/components/PerformancePage/vectorPerformance.ts scripts/test-vector-performance.cjs src/store/types.ts package.json
git commit -m "feat: add vector performance view model"
```

### Task 8: Performance Page Vector Experience

**Files:**
- Modify: `src/components/PerformancePage/PerformancePage.tsx`
- Modify: `src/components/PerformancePage/PerfAnalysis.tsx`
- Modify: `src/components/monitoring/MonitoringPrimitives.tsx`
- Modify: `src/components/monitoring/monitoringViewModel.ts`
- Modify: `scripts/test-vector-performance.cjs`
- Modify: `scripts/check-monitoring-theme.mjs`

**Interfaces:**
- Consumes: frontend performance view model and existing three-column page layout.
- Produces: workload badge, vector KPIs, two vector trends, source-aware summaries, vector-safe activities and diagnostics.

- [ ] **Step 1: Add failing component integration assertions**

Assert the page uses the pure mode helper, renders a workload badge, does not use generation TPS for vector current throughput, displays source-unavailable text, and switches active task fields by workload.

- [ ] **Step 2: Run frontend regression and verify failure**

Run: `node scripts/test-vector-performance.cjs`

Expected: FAIL on missing PerformancePage integration markers.

- [ ] **Step 3: Render workload-aware primary metrics**

Keep current inference cards unchanged. For Embedding show input tokens/s, vector items/s, task P95 and queue pressure. For Reranker show input tokens/s, document items/s, task P95 and queue pressure. Add a compact badge beside instance/session identity; use existing badge primitives and theme tokens.

- [ ] **Step 4: Render trends and session summaries**

Provide input throughput and item throughput trend modes for vector sessions. Show completed items, average input speed and task P50/P95. Add proxy HTTP count/P50/P95/failure rate only when `proxyAvailable` is true. Preserve numeric zero inside measured idle buckets but show an explicit unavailable state when no log source exists.

- [ ] **Step 5: Adapt active tasks and diagnostics**

Vector active rows show slot, elapsed time and workload. Hide generated token count, generation TPS and speculative acceptance. Filter inference-only diagnostics and ensure “指标来源不完整” is visible without blocking resource cards.

- [ ] **Step 6: Keep historical sessions stable across model changes**

All historical cards and comparisons must use `session.workload`; do not classify from the selected instance’s current model. Verify an inference session remains labeled inference after the instance is changed to Reranker.

- [ ] **Step 7: Check responsive/theme behavior**

Extend monitoring theme checks to cover new badges, unavailable states and selected trend controls. Ensure no fixed-width text overlap at desktop and narrow widths and no light-theme low-contrast text.

- [ ] **Step 8: Run regression, theme and production build checks**

Run:

```bash
npm run test:regressions
npm run check:monitoring-theme
npm run build
```

Expected: all tests pass and Vite production build completes without warnings.

- [ ] **Step 9: Commit the vector performance interface**

```bash
git add src/components/PerformancePage/PerformancePage.tsx src/components/PerformancePage/PerfAnalysis.tsx src/components/monitoring/MonitoringPrimitives.tsx src/components/monitoring/monitoringViewModel.ts scripts/test-vector-performance.cjs scripts/check-monitoring-theme.mjs
git commit -m "feat: display vector runtime performance"
```

### Task 9: User Documentation Alignment

**Files:**
- Modify: `GUIDE.md`
- Modify: `README.md`
- Modify: `src/components/guide/guideTour.ts`
- Modify: `scripts/check-guide.cjs`

**Interfaces:**
- Consumes: final verified vector performance behavior.
- Produces: aligned repository and offline in-app documentation without release-validation content.

- [ ] **Step 1: Add failing guide checks**

Require the performance sections to mention Embedding/Reranker workload metrics, log-versus-proxy source meaning, direct-call coverage and unavailable-state behavior in both Chinese and English where the document is bilingual.

- [ ] **Step 2: Run guide check and verify failure**

Run: `node scripts/check-guide.cjs`

Expected: FAIL because current guide describes only tokens/s and generation request analysis.

- [ ] **Step 3: Update GUIDE, README and in-app guide**

Explain that generation uses tokens/s, Embedding uses input tokens/s and vector items/s, Reranker uses input tokens/s and document items/s. Clarify that direct calls provide task metrics from logs while proxy calls add HTTP count/latency/failure rate. Update the performance walkthrough description. `GuidePage.tsx` already imports `GUIDE.md?raw`, so do not create a second in-app content source. Keep the existing performance screenshot when its generation-model state remains accurate; do not fabricate a vector screenshot without a verified live instance.

- [ ] **Step 4: Run guide, encoding and release checks**

Run:

```bash
node scripts/check-guide.cjs
npm run check:encoding
npm run check:release
```

Expected: documentation checks and all release regressions pass.

- [ ] **Step 5: Commit documentation**

```bash
git add GUIDE.md README.md src/components/guide/guideTour.ts scripts/check-guide.cjs
git commit -m "docs: explain vector performance monitoring"
```

### Task 10: Integration Verification, Audit, and Stage-Two Design

**Files:**
- Create after stage-one verification: `docs/superpowers/specs/2026-07-14-vector-quality-evaluation-design.md`
- Modify implementation files only if verification exposes a defect.

**Interfaces:**
- Consumes: all stage-one implementation tasks.
- Produces: release-ready evidence for runtime monitoring and a discussion-only quality-evaluation design.

- [ ] **Step 1: Run formatter checks**

Run:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
```

The repository does not install Prettier or define a frontend formatter command. Do not add a formatter dependency merely for this task; rely on TypeScript, production build, source checks and existing release scripts.

- [ ] **Step 2: Run the full frontend release suite**

Run:

```bash
npm run check:release
npm run build
```

Expected: all regression, guide, encoding, cross-platform and TypeScript checks pass; production build succeeds.

- [ ] **Step 3: Run the complete Rust suite**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked -- -D warnings
```

Expected: every test passes and Clippy reports no warnings.

- [ ] **Step 4: Verify migration and source semantics with a no-model integration fixture**

Use an in-memory/temp SQLite database, representative log lines and an Axum test upstream. Prove: old DB upgrades; direct vector logs create nonzero task metrics; proxy request adds one HTTP event; log/proxy do not double count; failure produces proxy failure rate; no private request content exists in persisted rows.

- [ ] **Step 5: Perform visual verification**

Start the app/dev server as supported by the repository. Use browser automation or the desktop app to inspect dark/light themes and desktop/narrow widths. Check inference, Embedding, Reranker, only-log, only-proxy and unavailable states. Capture screenshots only as test evidence unless documentation assets are intentionally updated.

- [ ] **Step 6: Perform a focused final code audit**

Review the complete diff for schema migration safety, task parser bounds, proxy body privacy, request-body size handling, source double counting, workload drift, unavailable-vs-zero semantics, generation regressions, cross-platform paths and warning-free code. Fix every release-impacting issue and rerun affected suites.

- [ ] **Step 7: Check repository hygiene and commit final fixes**

Run:

```bash
git diff --check
git status --short
git log --oneline --decorate -12
```

Remove temporary databases, logs, screenshots, generated reports and helper files that are not deliverables. Commit only intentional final fixes.

- [ ] **Step 8: Write the stage-two quality evaluation design only**

After stage one passes, write a separate design covering: Embedding retrieval datasets and corpus/query relevance structure; Reranker candidate-set inputs; Recall@K, Precision@K, MRR, MAP and NDCG definitions; dataset import and privacy; deterministic run manifests; warm-up/repetition; hardware/model/backend identity; baseline comparison; result storage; cancellation; offline operation; UI workflow; export; testing and phased implementation options.

Do not implement quality evaluation code, download a benchmark dataset or alter runtime metrics based on this design until the user approves it.

- [ ] **Step 9: Self-review stage-two proposal for decision readiness**

The proposal must compare at least three implementation scopes (minimal built-in evaluator, extensible local benchmark workspace, external-tool integration), state time/storage/privacy tradeoffs, and recommend one option without treating paid services or cloud access as required.
