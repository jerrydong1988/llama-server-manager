# Local checkpoint admission acceptance

This records the Windows/ROCm experiment for the admission policy, rather than
adding a new universal engine or model qualification. See
[admission policy](CHECKPOINT_ADMISSION_POLICY.md) for the candidate bounds and
[b12000 acceptance](B12000_CHECKPOINT_ACCEPTANCE.md) for the earlier reviewed
scope. The embedded reviewed profiles are unchanged.

## Runtime and isolation

- Windows, Radeon 8060S, driver `32.0.32015.7029`, local ROCm 10.3 build of b12000,
  base commit `68d20b884c3878217de79eade0a249efde96ab38`.
- Launcher SHA-256:
  `71188bbb8f78e0a53c78c642b0295abdf80e9e6c7cfdaae1ed372c05b26cdd0a`.
- Full runtime, including auxiliary kernel/data files:
  `0b6d1705b4b0daf2a5f768d1e7e5bb450c0bcd65a01709502fe7553ba9c09e56`.
- Separate manager data roots, loopback ports, model-state files and owned
  processes. Process-local `GGML_CUDA_NO_PINNED=1`; no existing engine selection
  or instance configuration was changed.
- The saving process exits completely before restoration. All slots are restored
  before inference on any slot. Per-slot secrets and output tokens distinguish
  real state reuse from health checks, HTTP success or same-process restore.

Qwen target: `Qwen3.8-27B-UD-Q4_K_M.gguf`, SHA-256
`322e194ff79741c7baa497c240f677f54b201b0efab44ca8e50f122b39123482`.
MTP: `mtp-Qwen3.8-27B-Q4_0.gguf`, SHA-256
`50d9ce5a6da381bbcfb31061cf73df94a90e6faf8efeddee379a9cb8f1501c6e`.

Gemma target: `gemma-4-31B-it-Q4_K_M.gguf`, SHA-256
`38bd64c852c4b460434cc7162fa9bdcf242faf86502581a754cb72956bb17f84`.
MTP: `mtp-gemma-4-31B-it-F16.gguf`, SHA-256
`c19d08440ed71bc0fb57b8d09f5eef925c13ad19b2bdbb6e7784e75740e1b3ad`.

## Recorded inference

The context column is a **shared KV-pool capacity**. Its per-slot budget is not
an independent hard slot limit. Counts below are accepted/proposed draft tokens.

| Model/mode | Slots / shared context | KV | Saved / reused tokens by slot | Draft before → after | Evidence |
| --- | --- | --- | --- | --- | --- |
| Qwen, ngram-mod + MTP | 1 / 4K | f16 | 2627 / 2627 | 52/57 → 52/57 | `matrix-07`, individual case passed |
| Qwen, ngram-mod + MTP | 2 / 16K | f16 | 5315 / 5315; 5059 / 5059 | 52/57; 51/62, unchanged | `matrix-07`, individual case passed |
| Qwen, ngram-mod + MTP | 1 / 8K | q8_0 | 5315 / 5315 | 52/57 → 52/57 | `matrix-09`, individual case passed |
| Qwen, MTP, 32 checkpoint capacity | 1 / 8K | q8_0 | 5315 / 5315 | 46/51 → 46/51 | `matrix-09`, individual case passed |
| Qwen, ngram-mod + MTP | 4 / 64K | f16 | 10691 / 10691; 10179 / 10179 on each other slot | 52/57; 51/62; 52/58 → 52/57; 52/58 | `matrix-09`, complete campaign passed |
| Gemma 31B, MTP, 4 checkpoint capacity | 1 / 8K | f16 | 5060 / 5060 | 45/54 → 45/54 | `gemma-31b-02`, complete campaign passed |

In the q8_0 MTP case with 32 checkpoint capacity, the rewritten request reused
4,776 tokens. The mixed-mode rewrite cases fell back to zero cached tokens. All
cold controls reported zero reuse; a cache-enabled request alone is not claimed
as checkpoint-reuse evidence.
Gemma's rewritten request reused 4,520 tokens and its cold control reused zero.

Each successful local qualification also checks prefix rewriting, a cold
recomputation control, truncated-state rejection, whole-process disposal and
subsequent inference from the unchanged good files. Mixed strategies additionally
compare forced rejection/rollback and the following unconstrained generation
against a separate no-draft process.

Cancellation was exercised with a real validation child: the task reached
`cancelled` only after cleanup, the prior unblocked receipt remained byte-identical
and no owned native engine remained. Unit coverage separately prevents a later
cancellation from reviving a receipt blocked by an earlier failed test.

## Manager lifecycle

`manager-local-2slot-02` passed the full manager campaign with Qwen MTP +
ngram-mod, two slots, a 16K shared context and f16 KV. It acquired an authentic
local qualification before testing:

- Automatic all-slot save on stop, then real inference in a fresh process.
- Denied latest-pointer replacement preserving the previous valid generation.
- Invalid last-slot draft version and truncated main state causing whole-process
  isolation; unchanged good generations subsequently restored and inferred.
- A busy nonzero slot blocking idle save, and independent longer-prefix retention.
- Restore/inference after force-exit and after normal controller shutdown.
- A sparse generation with only the nonzero slot populated.
- Rejection of nine changed or unsupported configurations.

The controller-shutdown restore reused 2,601 tokens in each slot, with 46/46
accepted/proposed draft tokens. All owned processes exited; no forced cleanup
or shutdown error was recorded.

## Controls and retained negative evidence

- An initial long-prefix control forced 64 tokens past a natural numeric answer;
  its cold recomputation diverged into unrelated prose although the examined
  restoration outputs matched. The final control asks for the last record's
  three-digit code, allowing all 1,000 answers and requiring the correct value.
  Fresh-process secret generation still requires identical text and token IDs.
- In `matrix-07`, a 4-slot/64K attempt preserved the slot-2 output, reused 10,179
  tokens and accepted 52 draft tokens; proposal counts were 58 versus 57. The
  original exact-counter requirement rejected it. The final policy allows at
  most one configured draft batch of count drift and no more than five percentage
  points of acceptance-rate loss. It records both counts/rates and rejects missing,
  inconsistent or materially degraded counters.
- `matrix-08` passed its first restored-inference case, then hit a JavaScript
  harness hashing error before cancellation. The harness was corrected and the
  case repeated; that incomplete campaign is not counted as a complete pass.
- Native truncated restore can return HTTP 200. Admission still rejects an
  incomplete byte/token restore and discards that process. HTTP success is never
  used as the correctness assertion.
- `gemma-31b-01` exposed a validator mistake: it compared the response against the
  truncated file's size instead of the trusted original saved size. Production
  restore already used manifest metadata. The validator and production path now
  share that count comparison. The successful repeat recorded 2,931,219,512 bytes
  reported for a truncated file versus 2,931,219,544 trusted saved bytes, plus a
  changed hash; the affected native process was discarded. The raw native API
  still returned HTTP 200, so its malformed-file robustness is not certified.
- The first manager lifecycle campaign (`manager-local-2slot-01`) reached the
  post-force-exit inference check, then a CLI `instances.health` call produced no
  structured response. Independent queries still observed a healthy engine and
  a verified restore. This campaign remains failed; the exact cause was not
  established. The harness now records exit/signal/timeout details and bounds
  ordinary read calls independently of long validation tasks. It does not retry
  mutations or promote the failed campaign. The complete fresh-root repeat
  `manager-local-2slot-02` passed without reproducing that CLI failure.

Earlier attempt logs remain in their separate evidence directories. They were
not overwritten or converted into successful receipts.

## Scope and evidence storage

Raw local reports, native logs, format inspections and compact before/after
inference records are under
`C:\Users\Jerry\Documents\Codex\2026-10-30\checkpoint-admission-validation`.
Each `report.json` records its tested manager executable hash. Initial f16 cases
used `b65dd3bc8aaab91684a1d42a9c830f7d4546da5340e5d3c7d5beb60aa40c7812`;
`matrix-09` used
`05195a1ed311dd764657b5b772a6da8993f705b40a407fa5d0250cbdf8ded133`.

The subsequent manager build is
`cc655dfd3d265050b9eaa5ad7dc865dddbad27ac172f5143969ecbe7c35e5697`.
It uses development-package optimization for the manager (`opt-level=2`) and
SHA-256 support crates (`opt-level=3`) to avoid debug-only scanning/hashing cost
on gigabyte states. The llama.cpp engine and inference settings are unchanged;
these runs are correctness evidence, not inference-speed comparisons.
The final shared-count-guard build used for `gemma-31b-02` is
`ee6b0ba92533bc378cac3ad9fcb355a128feb9e1850fc0245fcd83a5eb65ce9b`.

GGSQ v4 / SCKP v1 / SDFT v1 inspections record main, draft and speculative-blob
lengths. An empty opaque speculative blob is reported as empty; these experiments
do not prove persistence of a historical dynamic ngram dictionary. Windows
destructor-based ngram exit-save and arbitrary abnormal shutdown remain separate
contracts.
Gemma's assistant draft record was 8 bytes, with an empty opaque speculative
blob. This is evidence for the tested Gemma assistant mode, not a claim of an
independent ordinary-draft KV payload: the pinned implementation attaches the
assistant to `ctx_other` and shares the target model's KV layers
([context](https://github.com/ggml-org/llama.cpp/blob/68d20b884c3878217de79eade0a249efde96ab38/src/llama-context.cpp#L239),
[memory setup](https://github.com/ggml-org/llama.cpp/blob/68d20b884c3878217de79eade0a249efde96ab38/src/llama-model.cpp#L3311)).

Memory measurements are host-wide samples reported by `system.health`, including
other activity; UMA RAM/VRAM accounting can overlap and must not be summed.
They are not per-process GPU peaks or allocator upper bounds.
No timing, bitwise-logit equivalence, other GPU architecture, other engine build,
unbounded context or universal model-family claim follows from these passes.
For scale, the 4-slot Qwen campaign sampled maxima of 30,268 MiB host RAM and
27,770 MiB host-reported VRAM; Gemma sampled 28,582 MiB and 25,133 MiB. These
samples are not resource-allocation guarantees.

Source changes keep immutable generations, checksum/identity matching,
latest-pointer atomic replacement and whole-process failed-restore isolation.
New combinations remain experimental and require their own local test. UI,
CLI/MCP and runtime owners enforce the same receipt decision.
