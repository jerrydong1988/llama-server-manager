# Gufo managed engines — integration and validation

Manager can discover and launch a local Windows `gufo.exe` alongside llama.cpp.
The initial qualified profiles are Qwen3.8-Flash-Next with MTP and Qwen3.8-27B
with DFlash2, each using one execution session. This does not qualify other
Gufo modalities, model families, platforms, or concurrent residency.

## Configuration and isolation

Add the Gufo **runtime directory**, including its adjacent DLLs, in Engine
Management and probe it. Create an instance with that engine, then configure
the model's first GGUF shard, matching projector and speculative sidecar in
the Gufo configuration page. A model-family choice is a preflight constraint;
the engine still validates actual tensor/artifact compatibility while loading.

`engine_kind` is `llama_cpp` or `gufo`, independently of the GPU `backend`.
Missing `engine_kind` retains llama.cpp behavior. Selecting a different engine
type resets tuning while preserving the instance identity, model paths, alias
and port. Automatic engine fallback stays within the instance's engine type.
Existing llama.cpp engine IDs remain directory based; Gufo IDs identify its
executable so the two binaries can coexist in one directory.

The shared Rust launch validator is used by GUI previews, actual launches,
CLI and MCP. Gufo has a nested `gufo` configuration. It generates
`gufo serve ... llm ...` arguments directly; llama.cpp flag projection and
checkpoint qualification are not used. Unknown Gufo fields and unsupported
parameter combinations are rejected. Managed Gufo currently binds only to
`127.0.0.1`; custom/manual commands are not supported for managed Gufo instances.

Every Gufo instance owns `<Manager data directory>/gufo/<instance id>/`:

- `snapshots/`: optional disk cache, with its own capacity limit.
- `local/`: per-child `LOCALAPPDATA` and `XDG_CACHE_HOME`, isolating artifact caches.
- `temp/`: per-child `TEMP` and `TMP`.

The process working directory is the same private instance directory. Parent
environment variables and the Gufo-GUI launcher configuration are not changed.
RAM caching distinguishes automatic, off and a fixed GiB limit. `auto` emits
`--cache-ram-bytes 0`; `off` emits `--cache-ram-bytes off`.

For an isolated trial, launch the candidate Manager with
`--management-data-dir <absolute-new-directory>`, or use `lsm --app <candidate>
--data-dir <absolute-new-directory> service start`. The MCP client remains
connect-only. Use separate test ports and stop one large model before starting
the other.

## Lifecycle and capability boundaries

The existing runtime owns the process, bounded logs and stop operation. Gufo
readiness requires `/ready` to report both `status=ready` and the expected model
alias. A live process remains loading until that probe succeeds or the configured
startup grace expires (default 600 seconds; accepted range 60–3600). llama.cpp
retains its existing probe and 120-second grace. A timeout reports failure;
it does not repeatedly restart the model.

The runtime advertises `gufo_engine_v2`; a client requiring this capability
cannot treat a phase-one daemon as API-capable, even with an identical app version.
Persisted Gufo launch arguments
are reconstructed and checked against their configuration before replay.

On Windows, engines use a private hidden console. Stop sends Ctrl+C, waits up
to the existing 30-second grace, and falls back to terminating the owned process
tree. Gufo can drain accepted disk writes during graceful shutdown. Forced
termination cannot promise recovery of pending writes. Starting reconciles the
authoritative daemon state first, allowing immediate recovery after an unexpected
exit without waiting for the GUI heartbeat. A pre-spawn port check rejects an
occupied Gufo endpoint without stopping the existing owner.

Gufo instances participate in the unified API router. Readiness uses `/ready`
with the expected model alias, including before the first routing probe succeeds.
Chat Completions, Responses and Anthropic Messages use native engine endpoints;
model aliases, streaming events, tools, images, errors, cancellation and usage
pass through the existing router contracts. Input-token count endpoints are
queried on the selected executable; 404/501 remains unavailable. Hard quotas
reject requests when exact metering is unavailable, while ordinary forwarding
retains the engine's own limits. Legacy Completions/tokenization and cloud-only
features are not emulated.

Gufo omits llama.cpp's decode-call count and average busy-slots-per-decode.
Those fields are optional (null), not zero, so available throughput and request
metrics still reach CLI/MCP, the live monitor and telemetry. Gufo's context
occupancy ratio is not physical memory or retained-cache usage. Process RAM,
device memory and slot observations remain separate. llama.cpp checkpoint actions
are ineligible for Gufo; Gufo owns its isolated snapshot cache.

## Reproducible validation

Run the ordinary repository gates, including `npm run check:release`,
`npm run build`, Cargo tests and Clippy, and the runtime/management suites.
`scripts/test-gufo-engine.cjs` covers legacy/default engine selection and
cross-engine configuration isolation; the Gufo Rust tests exercise invalid
shards/sidecars, command construction and bounded startup policy.

The real-engine runner is opt-in and requires a **new**, absolute output root:

```powershell
node scripts/test-gufo-real.cjs --root C:/Temp/lsm-gufo-32k --launcher C:/path/to/Gufo/launcher.json --context 32768
node scripts/test-gufo-real.cjs --root C:/Temp/lsm-gufo-262k --launcher C:/path/to/Gufo/launcher.json --context 262144
node scripts/test-gufo-real.cjs --root C:/Temp/lsm-gufo-full --launcher C:/path/to/Gufo/launcher.json --context 262144 --extended true --lifecycle true
node scripts/test-gufo-real.cjs --root C:/Temp/lsm-llama-regression --launcher C:/path/to/Gufo/launcher.json --only-llama true --llama-engine C:/path/to/llama-server.exe
```

It reads the existing launcher without importing or changing it, constructs new
Manager configurations, launches via CLI, observes health/logs and stops via MCP.
It records raw responses, effective `/props`, cache isolation, protected-file
hashes, original cache metadata and remaining owned PIDs. The llama.cpp control
deliberately omits `engine_kind` to exercise legacy defaults. Use short roots
outside watched source trees if Windows rejects atomic JSON replacement; retain
any failed attempt's report instead of counting it as a pass.

Extended mode checks authenticated direct/proxy text, SSE, Responses, Messages,
exact counts (including tools/images), multimodal inference, cancellation, errors,
context preflight, monitoring and hard quotas. Lifecycle mode checks disk restore,
window hide/show, abnormal exit and explicit recovery, occupied ports and stopping
during load. `scripts/package-gufo-trial.cjs` creates the isolated portable Windows
candidate described in [the manual test guide](GUFO_MANUAL_TEST.md).

The November 2, 2026 local acceptance used Gufo 0.28.1 revision `408a04357827`
(the deployed Windows compiler-flag fix), both existing model/projector profiles,
32K and 262K configured contexts. All four short inference runs returned the
expected answer with actual speculative draft proposals. One Flash startup and
request cycle took about 263 seconds and remained loading past 120 seconds.
Both models stopped and released their ports; accepted cache writes were
published during graceful exit. A llama.cpp b12032/Qwen3.8-27B control also passed.

The subsequent 262K-context acceptance exercised both profiles through authenticated
native and unified APIs: counts matched across paths, including tools and images;
Chat, Responses and Messages streaming, real tool calls, vision, cancellation,
context rejection, usage and hard quotas passed. Both profiles restored disk
snapshots across process restarts (2,275-token repeated prompt), preserved the
inference process when hiding/showing the window, rejected occupied test ports,
and stopped during loading. An intentionally terminated 27B process was detected
and immediately restarted successfully. These tests exposed and fixed stale GUI
runtime state and Windows Gufo port reuse. The original programs, launcher/Manager
configurations and Gufo cache inventory remained unchanged; no owned PIDs remained.

A 262K context allocation is **not** a complete 262K input test. Full-capacity
prompts, extended unattended runs, GPU-reset recovery and simultaneous residency
remain outside this qualification. Pending writes cannot be guaranteed after a
forced kill; controlled shutdown and reuse of completed snapshots were verified.

## Manual testing follow-up — 2026-11-02

Status at initial recording: **recorded, not fixed**. The user subsequently
authorized autonomous triage and fixes. Final dispositions superseding this
pending list are in [GUFO_AUDIT_DECISIONS.md](GUFO_AUDIT_DECISIONS.md).
The original running deployment remains unchanged.
The observations below concern Manager candidate `779fc0c`, Gufo 0.28.1
`408a04357827`, and the running Flash-Next/MTP profile (262144 context, one session).
The earlier acceptance above does not establish target-level queuing or complete
rejection-metric coverage.

### Pending work

| ID | Classification | Finding and intended scope |
| --- | --- | --- |
| GUFO-01 | Confirmed Manager defect | Capacity rejection does not increment `lsm_router_requests_rejected_total`. Audit target-selection and permit-acquisition rejection paths and count each rejected request exactly once. Preserve the documented meaning of `requests_total`: admission to the router, not successful inference. |
| GUFO-02 | Router design gap | Global/Key admission can queue, but target capacity exhaustion immediately rejects requests. Design bounded waiting for a compatible model/target without increasing engine execution sessions. Include queue limits, deadlines, cancellation, capacity-release wakeups, failover, shutdown and isolation between unrelated models/Keys. Enabling the existing fair queue alone does not add target-level waiting. |
| GUFO-03 | Error/observability improvement | The target-selection response combines unavailable targets and saturated targets into the same 503 message with `error.code=null`; a permit-acquisition race can instead return 429. Define distinguishable, consistent capacity/unavailability errors, retry guidance and queue/rejection measurements. Review existing client compatibility before changing HTTP status behavior. |
| GUFO-04 | Documentation and contract coverage | Document valid reasoning controls separately for Chat, Responses and Messages. Cover tiny output budgets, explicit reasoning budgets, unsupported aliases and direct/proxy parity. Decide whether to warn about or reject specific misleading aliases; do not indiscriminately reject all unknown extension fields. |

### Reproduction and acceptance evidence

- **GUFO-01/02:** with a single active streamed request, a second routed request
  returned 503, `all matching routes are unavailable or at capacity`.
  `requests_total` changed from 143 to 145 while `requests_rejected_total` stayed
  at 1. Cancelling the first request released the permit; in-flight returned to 0.
  The current entry limit is 64, each test route is limited to 1, and entry queue
  timeout is 1000 ms. The second request passes entry admission before failing
  target selection. Source: `commands/proxy.rs::proxy_upstream` and
  `commands/proxy_runtime.rs::{select_target,acquire_target,acquire_request}`.
- **GUFO-01 acceptance:** a capacity rejection increments the rejected counter
  exactly once, including the acquisition race; allowed requests do not. Verify
  HTTP outcomes, Prometheus counters and request-ledger classification together.
- **GUFO-02 acceptance:** two requests to a one-session target execute serially
  when capacity becomes available within the configured wait; timeout and queue
  overflow return explicit errors. Queued cancellation must not reach the engine
  or leak permits. Exercise streaming, multiple aliases, distinct models/Keys,
  stopping targets, and legacy llama.cpp behavior. Observed 0.22-second release
  in Hermes testing is not a guaranteed upper bound or a fixed retry policy.
- **GUFO-04 evidence:** with the same short Chat prompt and `max_tokens=32`,
  default thinking returned empty content and `finish_reason=length`; disabling
  thinking returned `首都` with one completion token. `reasoning_budget_tokens=8`
  returned the answer with 11 total completion tokens, both direct and proxied.
  There is no universal rule that 64 output tokens guarantees visible content.

Local, untracked raw evidence is retained in
`.temp/hermes-api-review/{capacity-metrics,contracts,native-and-budget}.json`.
Hermes source tests and its writing sweep are under
`%LOCALAPPDATA%/hermes/cache/scratch/svc_test/`. The compact observations above
remain in this document even if those local scratch files are later removed.

### Excluded findings and observations to retain

- **Responses `parallel_tool_calls=false`: not a confirmed defect.** Hermes's
  Responses test supplied no tools. The field describes that request, not the
  server's global capabilities. Controlled Responses tests returned three
  function calls with `parallel_tool_calls=true`, and one with `false`.
- **Reasoning aliases:** `reasoning_effort:none`,
  `chat_template_kwargs.enable_thinking:false`, and
  `thinking:{type:"disabled"}` worked in Chat. Top-level `think:false`,
  `enable_thinking:false`, and `reasoning:{enabled:false}` were ignored by Chat;
  the behavior also occurred directly against Gufo. Responses accepts
  `reasoning:{effort:"none"}` and rejects `reasoning:{enabled:false}` with 400.
  Manager was not stripping the supported controls.
- **Cache pressure:** logs contain `capture_refused reason=byte_capacity`.
  This concerns snapshot capacity/reclaimability and does not alone establish
  process OOM or an inference failure. The running profile uses
  `cache_ram_mode=auto`; changing only `cache_ram_gib` has no effect. Any fixed
  limit change needs memory-headroom and cache-reuse validation, not an assumed
  benefit from a larger value. No cache setting was changed during this review.
- **Quality evidence:** Hermes's writing sweep counts success as normal finish
  plus more than ten visible characters; it does not validate requested length
  or factual quality. The coding 5/5 is one generated program passing five test
  cases. Low effort's improvement is useful small-sample evidence, not a general
  guarantee of unchanged reasoning quality.
- **Concurrency terminology:** engine sessions, router admission and queued
  requests are distinct. `total_slots` is observed engine metadata, not a knob.
  A single execution session can still serve queued requests sequentially.

No source code, configuration or deployment was changed in the initial recording.
The service was left running and ready with no active inference after the probes.
