# Resource Planner

The Resource planner is the Phase 2 pre-mutation safety check for a local managed deployment. It answers two separate questions before configuration persistence and process launch:

1. What range of host RAM and accelerator memory can this configuration require?
2. Does that range fit the capacity available at the time of the check?

It is an estimator, not an automatic tuner or a replacement for llama.cpp `--fit`.

## Operator workflow

The Configuration page recalculates the plan after deployment-affecting edits. A plan becomes stale immediately when the draft changes, and Save remains unavailable until the replacement result is visible. Save then performs a fresh read-only plan before persistence.

Start performs another fresh check before normalized configuration is persisted, a deployment revision is materialized, or a process is spawned. This catches memory consumed or released since the Configuration page was opened.

The four statuses have deliberately different behavior:

| Status | Meaning | Launch behavior |
| --- | --- | --- |
| `feasible` | The full estimate range fits the measured safe headroom. | Continue. |
| `constrained` | The minimum fits, but the upper estimate exceeds safe headroom. | Explain the risk and require operator confirmation. |
| `infeasible` | The minimum estimate exceeds current safe headroom and no unresolved input prevents a reliable decision. | Block before mutation. |
| `unknown` | A material input or capacity boundary cannot be measured safely. | Explain the uncertainty and require operator confirmation. |

Saving an infeasible configuration is allowed. The planner protects deployment mutation, not configuration authoring.

## Inputs and model

The planner uses exact artifact byte sizes and bounded scalar GGUF metadata when they are available. A complete sharded model is summed by shard index; an incomplete set is reported as `unknown` instead of treating one shard as the whole model.

The estimate separates:

- main model, projector, draft model, and adapter weights;
- KV cache based on context, layer count, embedding width, attention heads, sliding-window metadata, and K/V cache types;
- host and accelerator runtime buffers based on batch and micro-batch settings; and
- the demand-driven prompt-cache range up to `--cache-ram`.

GPU residency follows explicit layer counts when the GGUF layer count is known. Automatic GPU layers use current free VRAM and are reported as a range. `--fit` widens the range down to its allowed context minimum when the context was not explicitly pinned. Metal without a separately reported VRAM pool is accounted against unified system memory.

Live capacity comes from the same system sampler used by LSM monitoring. “Available” therefore already reflects other running deployments and unrelated processes. The planner retains a safety reserve of at least 512 MiB or 5% for host RAM and at least 256 MiB or 5% for VRAM. When llama.cpp `--fit` is effective, its target margin (1 GiB by default) is also respected for accelerator memory.

## Confidence and uncertainty

`high`, `medium`, and `low` describe the evidence behind the range; they are not performance scores. Missing GGUF shape metadata widens the KV range. File-backed loading, prompt caching, sliding-window attention, MoE tensor placement, automatic slots, and context checkpoints are reported as explicit assumptions.

The result becomes `unknown` when a safe single-node calculation is impossible, including:

- manual launch commands or custom arguments that may replace resource flags;
- missing artifacts or incomplete shard sets;
- dynamic multi-model residency;
- RPC offload without measured remote capacity;
- row/tensor multi-GPU placement when only aggregate VRAM is available; or
- metadata overrides that change the model shape.

An `unknown` result never becomes an automatic launch block. The operator retains the decision.

## Privacy and side effects

Planning reads file metadata, a bounded portion of GGUF metadata, and the current capacity snapshot. It does not write configuration, load model tensors, start an engine, or modify a deployment.

The IPC report contains numeric ranges, fixed reason codes, and aggregate facts only. It never returns model paths, custom-argument values, API keys, environment values, or other launch secrets.

## Boundaries

The planner does not predict latency or throughput, change context or GPU layers, schedule workers, place or evict models, or implement canary policy. Those responsibilities remain outside this Phase 2 workstream.
