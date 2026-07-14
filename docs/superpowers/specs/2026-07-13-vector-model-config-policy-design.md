# Vector Model Configuration Policy Design

## Context

The application detects some vector models in `ConfigPage` and enables `embedding`, but the current behavior is only a partial UI guard. A vector instance can still retain or acquire generation-only configuration through instance creation, presets, custom arguments, persisted legacy configuration, command preview, or auto-start. The Rust command builder filters some sampling flags but does not enforce a complete vector-mode contract.

This design establishes one policy for recognizing vector workloads and one normalization boundary that every creation, edit, save, preview, and start path must use.

## Goals

- New instances created from vector models start from a clean vector configuration.
- Existing inference instances switched to vector models have incompatible values removed immediately and visibly.
- Persisted vector configurations contain no hidden generation, chat, speculative decoding, MTP, multimodal, or custom-argument state.
- Command preview and actual launch produce the same normalized command.
- Legacy and auto-started instances are protected even when the configuration page is never opened.
- Embedding and reranking models retain the shared runtime controls they need.
- Automated tests fail when a newly added configuration field is not classified by the vector policy.

## Non-Goals

- Detecting every future model family from its filename with perfect certainty.
- Automatically restoring generation values after a vector instance is switched back to an inference model.
- Supporting arbitrary custom command-line arguments in vector mode.
- Changing the behavior of inference-model instances that do not enter vector mode.

## Model Workload Classification

The scanner will expose two model capabilities:

- `is_embedding_model`: the model is intended for embedding/vector output.
- `is_reranker_model`: the model is intended for ranking and therefore also requires embedding mode.

Classification uses GGUF metadata first and normalized filename hints second. Existing architecture and filename heuristics will move into a single model-policy module instead of remaining local to `ConfigPage`. Reranker-specific names such as `rerank` and `reranker` take precedence over generic embedding hints.

The frontend uses the indexed `ModelInfo.capabilities` result. A compatibility fallback applies the same conservative architecture and filename hints when an older cached index does not contain the new fields.

The Rust start boundary rechecks the selected GGUF path when `embedding` was not persisted. This protects legacy configuration and auto-start without requiring the user to visit the configuration page first. Explicit `embedding=true` remains sufficient for manually configured vector workloads whose model cannot be classified.

## Canonical Vector Policy

Vector mode is active when either the selected model is classified as embedding/reranking or `config.embedding` is already enabled.

Every `InstanceConfig` field must belong to exactly one of these sets:

1. Identity and selection: instance id/name, engine, model path, alias, and auto-start.
2. Shared runtime: context size, GPU offload, CPU threads, batch/ubatch, parallelism, continuous batching, memory mapping/loading, device selection, network, TLS, API authentication, RPC workers, logging, metrics, and other transport/observability settings that remain valid for vector serving.
3. Vector-specific: `embedding`, `pooling`, `embd_normalize`, and `reranking`.
4. Inference-only: LoRA/chat/reasoning/grammar, sampling, generation, speculative decoding/MTP, draft-model KV cache, multimodal/projector/media, assistant/agent/tools, model router, slot-prompt generation behavior, and custom arguments.

The TypeScript policy exports an explicit allowed-field set and derives the incompatible set from the complete default configuration. A coverage assertion verifies that every key in `defaultInstanceConfig()` is classified. New fields therefore cannot silently bypass vector normalization.

The allowed set is intentionally explicit:

- Identity: `id`, `name`, `engine_id`, `model_path`, `alias`, `auto_start`.
- Context and execution: `ctx_size`, `ctx_size_auto`, `gpu_layers_auto`, `gpu_layers`, `threads`, `threads_batch`, `threads_http`, `batch_size`, `ubatch_size`, `parallel`, `cont_batching`, `warmup`.
- Position and model loading: all RoPE/YaRN fields, `flash_attn`, `moe_cpu_layers`, `cpu_moe`, `mlock`, `no_mmap`, `no_repack`, `direct_io`, `numa`, `perf`, `check_tensors`, `fit`, `fit_target`, `fit_ctx`, `cache_type_k`, `cache_type_v`, `kv_unified`, `cache_idle_slots`, `no_kv_offload`, `device`, `split_mode`, `tensor_split`, `main_gpu`, `override_kv`.
- Network and operation: `host`, `port`, API key fields, TLS fields, `path_prefix`, `api_prefix`, `no_ui`, `offline`, `metrics`, `props`, `slots_enabled`, `timeout`, `sleep_idle`, `verbose`, `rpc_servers`, `sse_ping_interval`, `reuse_port`.
- Vector workload: `embedding`, `pooling`, `embd_normalize`, `reranking`.

Everything not listed above is inference-only in vector mode. This includes prompt-cache tuning (`cache_prompt`, `keep`, `cache_reuse`, `cache_ram`, context checkpoint fields), context shifting, draft cache, speculative decoding, generation, chat/reasoning, multimodal/media, assistant tooling, router/multi-model fields, slot prompt persistence/similarity, and custom arguments.

Inference-only fields are reset to their values from `defaultInstanceConfig()`. They are not merely hidden. `custom_args` is always cleared in vector mode because unknown flags cannot be proven safe.

## Vector Defaults and Invariants

Normalization applies the following vector invariants:

- `embedding` is always `true`.
- Generic embedding models leave `pooling` empty so llama-server uses the model's declared/default pooling mode.
- Reranker models set `reranking=true` and `pooling=rank`.
- Non-reranker embedding models set `reranking=false`; an explicitly selected valid pooling mode is retained.
- `batch_size` must not exceed `ubatch_size`. When it does, `batch_size` is reduced to `ubatch_size`, matching llama-server's own safety adjustment and avoiding a startup warning.
- Draft KV cache, `spec_type`, draft model values, MTP values, and all generation values are reset even if the command builder would otherwise ignore them.
- `custom_args` is empty.

## User Flows

### New Instance

When a model is selected in the creation dialog, its workload is classified immediately. On creation, defaults are built first, identity/shared values are assigned, then vector normalization runs before the instance is added or saved.

The new vector instance therefore has no previous state to clean and no cleanup notification is shown. Its first command preview and first launch already include the correct embedding/reranking flags.

### Existing Instance Switched to a Vector Model

Selecting a vector model is one atomic draft update:

1. Replace the model path and associated projector path.
2. Normalize the full draft with vector mode enabled.
3. Compute the list of values changed by normalization.
4. Display one cleanup summary grouped by speculative decoding, generation/sampling, chat/reasoning, multimodal, and custom arguments.

The removed values are not retained in hidden state and are not restored when switching back to an inference model. Switching back starts from the current normalized configuration and the normal inference defaults.

### Existing Vector and Legacy Instances

Loading configuration reconciles instances after model inventory is available. Vector models with stale generation state are normalized in memory and persisted once. The UI may show a non-blocking migration summary for the actively edited instance, while background and auto-start paths remain non-interactive.

## Configuration UI

When vector mode is active:

- Show a clear vector/reranker mode banner.
- Show basic vector controls, shared performance/hardware controls, network/authentication, cluster/RPC, and observability controls.
- Do not render inference-only groups in parameter navigation, search results, active-parameter counts, diffs, or reset actions.
- Do not offer generation or MTP presets. A vector-safe preset may be added using only allowed fields.
- Do not render the custom argument editor; the vector-mode banner explains that structured vector/runtime fields must be used.
- Filter cleanup-only changes from the ordinary unsaved-diff presentation and display them in the dedicated cleanup summary.

UI visibility is a usability feature, not the security boundary. All setters, preset application, save, preview, and start paths still normalize independently.

## Save, Preview, and Start Boundaries

The frontend provides a pure `normalizeInstanceConfig(config, model)` function returning:

- the normalized configuration;
- whether vector mode is active;
- the detected workload kind;
- a typed list of changed fields and cleanup groups.

It is called by instance creation, model selection, preset application, save serialization, command preview, manual start, and auto-start reconciliation.

The Rust backend provides a matching defensive normalization step before `generate_command`. It derives vector mode from the incoming configuration and, when necessary, GGUF model classification. The command builder consumes only the normalized value. The backend never appends custom arguments for vector mode and never emits inference-only command flags.

Both `generate_server_command` and `start_server` call the same backend normalization function, guaranteeing preview/start parity.

## Error and Notification Behavior

- Automatic cleanup is not a blocking error.
- Existing-instance model switches show a concise grouped summary with the number of removed/adjusted values.
- Save and launch continue with the normalized configuration.
- Backend-only cleanup is logged with field names but does not expose secrets or full custom-argument values.
- Failure to read model metadata does not block a configuration already marked `embedding=true`.
- A model that cannot be classified and is not explicitly in embedding mode continues as an inference model.

## Migration

The new capability fields use serde/default-compatible optional values, so old model-index rows and configuration files remain readable. Rescanning refreshes exact capabilities. Frontend fallback detection handles cached rows until then.

No configuration schema version bump is required because the existing `embedding`, `pooling`, and `reranking` fields remain canonical. Legacy vector configurations are normalized on load and persisted through the existing save queue.

## Test Strategy

### TypeScript Policy Tests

- Detect embedding and reranker models from capabilities, architecture, and filename fallback.
- Do not misclassify representative inference and vision models.
- New vector configuration contains no inference-only non-default values.
- Existing polluted configuration returns a complete cleanup list and clean result.
- Switching back to inference does not restore removed values.
- Reranker normalization forces `pooling=rank`.
- Generic embedding normalization preserves a valid explicit pooling choice and otherwise uses model default.
- Batch is reduced when `batch_size > ubatch_size`.
- Every `InstanceConfig` key is classified.

### Rust Tests

- Embedding commands omit all sampling, reasoning, chat, grammar, multimodal, speculative/MTP, draft-cache, agent/tool, slot-prompt, and custom flags.
- Reranking commands contain the correct vector flags.
- Polluted legacy input and custom arguments cannot leak into generated commands.
- `generate_server_command` and `start_server` use the same normalization path.
- Metadata/filename fallback identifies representative vector and reranker models.
- Batch/ubatch normalization matches the frontend policy.

### Integration and Regression Tests

- Create a vector instance and inspect the persisted configuration and command preview.
- Switch a polluted inference instance to a vector model and verify the cleanup summary and persisted state.
- Load a legacy polluted vector instance without opening ConfigPage, then preview/start it.
- Exercise dashboard start, instance-manager start, direct start from command preview, and auto-start.
- Verify inference MTP configuration remains unchanged and still generates `--spec-type draft-mtp`.
- Run release checks, TypeScript build, Rust tests, and Clippy with warnings denied.

## Acceptance Criteria

- No supported application path can save or launch a vector instance with inference-only parameter state.
- MTP and custom-argument bypasses are blocked in both frontend and backend.
- New and migrated vector instances launch with `--embedding` or `--reranking` and without incompatible flags.
- Existing inference/MTP behavior is unchanged outside vector mode.
- Parameter policy coverage tests prevent future unclassified fields.
