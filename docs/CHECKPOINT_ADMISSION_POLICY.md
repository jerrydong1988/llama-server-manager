# Checkpoint admission and local qualification

Checkpoint admission and saved-state compatibility are separate decisions. The
embedded reviewed profiles remain available at their recorded scope. Additional
compatible Windows/ROCm configurations can earn a local, experimental
qualification without adding their model or runtime hashes to source code.

`kv_checkpoint.allow_local_validation` defaults to `false`. Enabling it alone
does not qualify a configuration, start an instance, or change its engine.

| Eligibility | Behavior |
| --- | --- |
| `reviewed` | The existing standard support contract applies. |
| `needs_validation` | A compatible candidate; persistence stays disabled until its isolated test passes and local validation is enabled. |
| `local_validated` | This exact configuration has a current local receipt. It remains experimental. |
| `unsupported` | A structural requirement is missing; the current local validator cannot admit it. |

The UI exposes **Run isolated validation** and cancellation. The owner CLI/MCP
operation is `checkpoints.validate` with an instance `id`; it returns a durable
task. A running validation can request cancellation through `tasks.cancel`.
Cancellation waits for the current bounded native request to finish or time out
before reclaiming owned processes. It never cancels a production instance.

## Candidate scope

The initial local validator supports Windows/ROCm, managed loopback text
inference, full GPU offload, FA enabled, fitting disabled, unified KV, and:

- 1, 2, or 4 slots;
- a configured shared-pool budget of 1K–32K tokens multiplied by the slot count;
- 1–32 context checkpoints and 1–8 draft tokens;
- uniform f16 or q8_0 target/draft K/V settings;
- ordinary draft, MTP, DFlash, ngram-mod, or a supported ngram-mod/draft chain.

These are **candidate bounds**, not a declaration that every combination works.
Each additional tuple must pass its own test. Architecture, model-artifact,
multimodal, LoRA, custom-argument, external lookup-cache, RPC, and retention
requirements still apply. Unknown runtime bundles must expose the required API
capabilities and have the complete supported ROCm external-asset layout.

With `--kv-unified`, `ctx_size` is the shared pool capacity. Dividing it by the
slot count describes the test's per-slot budget, not an independent hard context
limit. The validator checks the actual slot count and reported shared capacity.

## What the validator does

The selected configuration is copied in memory. Only isolation controls are
replaced: a private directory, an available loopback port, a temporary API key,
and the slot-save destination. The original instance configuration is not
written, and existing engines are not stopped or switched.

1. Run actual GPU inference. Require evidence of ROCm/HIP full layer offload.
   Each slot receives its own code and secret so a shared output cannot conceal
   swapped slot state.
2. Save each slot and inspect bounded GGSQ v4, SCKP v1 and SDFT v1 appendices.
   Windowed/recurrent models must produce checkpoints. Draft modes must include
   a nonempty live draft-context payload. Record speculative-blob lengths,
   including legitimate empty blobs.
3. Fully exit the saving process. Start a fresh process, restore **all** slots
   before testing any of them, and require complete byte/token consumption.
4. Require identical generated text/tokens. Record both draft counters and
   acceptance rates; allow at most one configured draft batch of counter drift
   and at most five percentage points of acceptance-rate loss. This accommodates
   proposal-boundary variation without accepting a material regression. The
   request must retrieve its slot's secret and reuse the saved prefix, allowing
   one token of boundary reevaluation. Startup and HTTP 200 do not count as this
   evidence.
5. Rewrite the long prefix and retrieve its final record using a three-digit
   grammar that allows all 1,000 answers. Require the correct answer in the cold
   reference, cache-enabled rewrite and verified full recomputation; this avoids treating
   arbitrary long-continuation differences as restoration failures. Mixed
   strategies also exercise a deliberately rejected draft
   continuation and compare its output and subsequent unconstrained generation
   with a separate no-draft process.
6. Restore a deliberately truncated private copy. Apply the manager's full-read
   and token-count guards, discard the entire affected process, and prove that
   unchanged good files still restore and infer in another fresh process.
7. Recheck runtime/model fingerprints and the Windows machine/GPU-driver and
   relevant environment identity before atomically publishing the receipt.

The Windows TCP listener owner is checked before each HTTP request; requests
are rejected when the listener belongs to another local process. Cancellation
is scoped to its validation ID, including stale requests from a previous attempt.
All test processes are owned by the validator. Windows jobs reclaim them when
the owning process exits. Native requests and the campaign have time limits;
one data directory cannot run two concurrent qualifications.

Successful test payloads are removed after the child processes exit; logs,
format records and compact inference evidence remain available. Failed attempts
retain their evidence and cannot create a new qualification. Revalidation blocks
the prior local receipt until success; cancellation preserves a previously
completed, unblocked receipt and cannot revive one blocked by an earlier failure. Neither action modifies existing committed generations.

## Identity and storage

- Cache identity continues to bind target/draft content hashes, runtime identity,
  state-affecting parameters and topology. The local-validation switch itself
  does not change this identity.
- Explicit emission/inheritance of state-affecting arguments is included.
  Equivalent field aliases normalize together. Within the same emission policy,
  making logging, monitoring or network controls explicit does not invalidate
  cached model state. Switching between legacy and selective emission still does.
- Native `LLAMA_ARG_*` environment defaults are bound by a digest, without
  persisting raw values, but admission rejects these unmanaged overrides. A
  hashed environment path does not hash the model/adapter file it could load.
  Configure those parameters in the instance instead. Windows keys are compared
  without case sensitivity. Legacy configurations without these defaults or
  explicit parameter intent retain their existing fingerprint representation.
- Newly admitted ROCm bundles include the executable/libraries plus `rocblas`,
  `hipblaslt` and `.kpack` assets. Existing reviewed legacy identities remain
  unchanged.
- Receipts live under `kv-checkpoints/.qualifications`; temporary runs live under
  `kv-checkpoints/.qualification-runs`. Dotted names cannot be instance IDs.
  These directories use the existing private-directory and reparse protections.
- Both direct launch and the background runtime recheck admission. The runtime
  advertises `kv_checkpoint_v4_local_qualification`; older runtime owners cannot
  silently consume the new eligibility contract.

Checksums, immutable generations, atomic latest-pointer replacement, complete
restore-response validation, routing exclusion during recovery and whole-process
replacement after partial failure remain mandatory.

## Evidence limits

A local pass certifies the recorded experiment on that machine, not an entire
architecture family or a mature engine release. It does not establish bitwise
logit equivalence, all workload correctness, allocator upper bounds, or safety
under arbitrary concurrent memory pressure. An engine may serialize an empty
opaque speculative blob; the report must not turn that into proof of a nonempty
serializer or of historical ngram-cache persistence. Dynamic ngram exit-save
qualification remains a separate contract.

Use `scripts/test-checkpoint-admission-real.cjs` for new isolated data roots and
`scripts/test-manager-multislot-real.cjs --local-validation true` for the manager
lifecycle campaign. The latter retains automatic save, failed-save protection,
corrupted-last-slot process replacement, controller shutdown, sparse slots and
longer-prefix checks. It accepts `--context-budget` and `--kv`; the context
budget is multiplied by `--parallel` to obtain the shared pool size.

Published local acceptance results and remaining limits are recorded separately;
the historical [b12000 acceptance](B12000_CHECKPOINT_ACCEPTANCE.md) remains
evidence for its original combinations.
