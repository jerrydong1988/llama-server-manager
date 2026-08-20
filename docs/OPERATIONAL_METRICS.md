# Operational Metrics and Alerts

Llama Server Manager uses one privacy-safe operational contract across the routing page, background runtime status, Prometheus `/metrics`, SQLite request telemetry, and canary observations. The contract covers router-visible TTFT, queue wait and depth, errors, concurrency saturation, and explicitly reported prompt-cache reuse.

这是 Phase 2 的运营观测契约：界面、后台运行时、Prometheus、SQLite 历史和金丝雀证据使用相同定义。未观测到的 TTFT 或缓存字段保持未知，不会写成 0；告警只提供处置建议，不会自动调整并发、提升候选版本或回滚部署。

This is the fourth workstream of [Phase 2 — Managed Deployment](PRODUCT_ROADMAP.md#phase-2--managed-deployment). It adds no hosted collector, external monitoring dependency, automatic rollout decision, or Phase 3 worker aggregation.

## Stable signal definitions

| Signal | Definition | Availability |
|---|---|---|
| TTFT | Milliseconds from successful proxy authentication/admission entry until the first non-empty downstream response-body chunk is ready. It includes router queue wait, routing, preflight, upstream prompt processing, and response rewriting. | Unknown for requests that fail before a body chunk or protocols that return an empty body. |
| Queue wait | Milliseconds spent waiting for the proxy's global concurrency permit. An immediately admitted request records zero. | Available for accepted inference requests; queue timeouts are counted separately. |
| Queue depth | Requests currently waiting for the global concurrency permit. | Live runtime gauge only. |
| Error rate | Failed completed outcomes and proxy rejections divided by all bounded-window outcomes. A completed response fails when transport/stream handling fails or its status is outside `200..399`. | Five-minute runtime window. Alerts require at least five outcomes, except queue-timeout and live-saturation alerts. |
| Saturation | Current global in-flight inference requests divided by the configured global concurrency limit. | Live runtime gauge. It does not claim that the model, GPU, RAM, VRAM, or target slots can safely accept a higher limit. |
| Observed cache reuse | Cached prompt tokens explicitly reported by upstream response usage/timing metadata divided by explicitly reported total prompt/input tokens. | Unknown when the upstream response omits either usable total-token or cache-token evidence. A low value is not inherently an error. |
| Canary operational evidence | Target-scoped success/failure counts, TTFT P95, queue-wait P95, and observed cache-reuse basis points since rollout creation. | Captured only by the operator's **Observe** action and sealed into the existing canary audit chain. |

The proxy accepts OpenAI `prompt_tokens_details.cached_tokens` and `input_tokens_details.cached_tokens`, Anthropic-style cache-read fields when present, and llama.cpp timing metadata. It records only numeric counts and timings. Prompts, generated content, response chunks, API keys, model paths, and request bodies are not retained by this feature.

## Five-minute operational window

The runtime retains at most 4,096 content-free outcome records and removes entries older than five minutes. Restarting the routing runtime intentionally resets this live window and all process-lifetime Prometheus counters. SQLite request rows remain subject to the existing telemetry retention and pruning controls.

The routing page shows:

- TTFT P95 and its sample count;
- current queue depth and queue-wait P95;
- observed cached and total prompt tokens plus their reuse percentage;
- bounded-window error rate;
- current concurrency saturation; and
- bounded-window outcomes plus process-lifetime queue timeouts.

`—` means unknown or unavailable. It never means zero.

## Deterministic alert rules

Rules are intentionally fixed for this workstream so the same snapshot has the same meaning in the UI and runtime protocol.

| Alert | Warning | Critical | First operator action |
|---|---:|---:|---|
| Error rate | at least 5 outcomes and rate at least 10% | at least 5 outcomes and rate at least 25% | Inspect target health, upstream status, and recent errors before changing rollout traffic. |
| TTFT P95 | at least 5 TTFT samples and P95 at least 3,000 ms | at least 5 TTFT samples and P95 at least 10,000 ms | Compare queue P95 first; if queueing is normal, inspect context size, GPU pressure, and cache evidence. |
| Queue-wait P95 | at least 5 queue samples and P95 at least 250 ms | at least 5 queue samples and P95 at least 1,000 ms | Inspect global/per-target limits and slot/resource headroom before raising concurrency. |
| Queue timeouts | at least 1 in the live window | at least 3 in the live window | Reduce load or add verified capacity; confirm the configured queue timeout and target availability. |
| Saturation | at least 85% | at least 100% with a non-empty queue | Use the Resource Planner and live slot/resource signals before changing concurrency. |

There is deliberately no low-cache-reuse alert. Cache value depends on prompt similarity, slot policy, request shape, and engine support; a compulsory minimum would turn a workload characteristic into a false incident.

Alerts are advisory. They do not mutate proxy configuration, stop instances, change canary traffic, promote a candidate, or roll back a Deployment Revision.

## Prometheus endpoint

`GET /metrics` includes the existing request, duration, target-health, and in-flight metrics plus:

- `lsm_router_queue_depth`
- `lsm_router_queued_requests_total`
- `lsm_router_queue_timeouts_total`
- `lsm_router_queue_wait_milliseconds`
- `lsm_router_ttft_milliseconds`
- `lsm_router_prompt_tokens_observed_total`
- `lsm_router_prompt_tokens_cached_total`
- `lsm_router_saturation_ratio`
- `lsm_router_operational_alert{alert="...",severity="..."}`

Histograms use explicit millisecond buckets ending in `+Inf`. The operational-alert gauge exports active alerts only. Scrapers should use counter deltas and their own durable retention; the manager does not claim to be a metrics database.

## Investigation order

1. Confirm `/live`, `/ready`, healthy/unhealthy route counts, and current canary drift.
2. Separate queue pressure from model latency using queue P95 and TTFT P95.
3. Check active requests, target slot capacity, CPU/GPU/RAM/VRAM signals, and the latest Resource Plan.
4. Inspect recent HTTP status and transport errors without exposing request content.
5. Treat cache reuse as supporting evidence only. Verify that the target and protocol actually report cache usage before drawing a conclusion.
6. Make traffic, concurrency, abort, promotion, or rollback changes explicitly and re-observe the window.

## Compatibility and persistence

Telemetry schema version 9 adds nullable `queue_time_ms`, `ttft_ms`, and `cached_prompt_tokens` columns to existing request rows. Old rows remain readable with unknown values. Runtime-service protocol payloads default the new operational snapshot when communicating with an older persisted status shape.

Canary evidence fields are optional and omitted when unavailable, preserving the integrity material of audit events created before this workstream. New observations add operational values without changing the manual promotion, abort, rollback, drift, or revision-binding rules documented in [Model and Engine Canary Rollouts](CANARY_ROLLOUTS.md).
