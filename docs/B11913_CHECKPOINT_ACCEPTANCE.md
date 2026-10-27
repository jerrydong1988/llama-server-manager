# Local b11913 Windows/ROCm checkpoint acceptance

## Immutable build and scope

The qualified build is the local `build-hip-rocm103a20261027-vs2026-b11913/bin`
bundle, compiled with Clang 24 for Windows AMD64. Its embedded build commit and
the official [b11913 tag](https://github.com/ggml-org/llama.cpp/releases/tag/b11913)
both resolve to `7aca0d2ea8d92b56911233b822db01f9fbb1ae50`.
This is a qualification of these local bytes, not of every b11913 build or the
official Windows/ROCm download.

| Identity | SHA-256 |
| --- | --- |
| Launcher | `b48a870997027d567ae29e586db27ae8eb44e2a34a9b2f0a10f6a5470f184b5a` |
| Launcher and 28 adjacent DLLs | `e0fbc00b164e6553815ffd8a4a0fc0d0c9503279c7b990e7449f43c119f05dcd` |
| Complete runtime, including 272 auxiliary files | `1b51535a99ab6e567c91cf129a4c621e6048be8cec68bf39e774b793525e1dff` |

Auxiliary identity covers `bin/rocblas`, `bin/hipblaslt` and sibling `.kpack`,
using the existing fail-closed runtime-set-v2 scheme. The base DLL digest alone
cannot qualify. Loaded-module inspection confirmed that the live engine used
the HIP/ROCm DLLs from this bundle. Existing b11849 identities remain separate.
Persistence revision 6 invalidates old cached probe receipts, including an
earlier unqualified probe of an otherwise unchanged b11913 executable.

The host is Windows with Radeon 8060S, driver `32.0.32015.7029`, VMM disabled.
`GGML_CUDA_NO_PINNED=1` applies only to test processes. Launches use full GPU
offload, FA on, fit off and f16 target/draft KV. Four-slot qualification is
restricted to total context 16384, unified KV, two context checkpoints,
`cache_idle_slots=false`, MTP with three draft tokens and this exact pair:

- Qwen3.8-27B UD-Q4_K_M:
  `322e194ff79741c7baa497c240f677f54b201b0efab44ca8e50f122b39123482`.
- Q4_0 MTP:
  `50d9ce5a6da381bbcfb31061cf73df94a90e6faf8efeddee379a9cb8f1501c6e`.

Single-slot ordinary draft is limited to the same target/draft GGUF. The
single-slot MTP/DFlash contract retains the existing 4096-context, f16 and
full-offload restrictions. There is no b11913 Gemma, quantized KV, two-slot,
larger-context, nonempty speculative-state or EAGLE3 qualification in this change.

## Native evidence

`native-qwen-4slot-01` passed in 223.6 seconds on 2026-10-27. Old process 40248
exited before process 47176 restored all four slots and performed real GPU
generation. Text, token IDs, draft counters and cache reuse matched the live
references. Each original slot saved 2347 logical tokens in 634232056 bytes.
The files contain GGSQ v4, SCKP v1 with two checkpoints, and SDFT v1; the live
draft payload is 9669696 bytes. Qwen's target state includes its recurrent state.
The speculative blob is empty for this implementation and does not establish
support for nonempty speculative-state implementations.

The same run passed an answer-changing late prefix rewrite with partial reuse,
an earlier rewrite requiring full recomputation, malformed main/draft files,
inference on every slot after those faults, and actual RAM-cache rejection.
A final new process restored the unchanged good files and inferred successfully.
The malformed SDFT case still returned native HTTP 200 while consuming only
624562336 of 634232056 bytes. This upstream partial-read behavior is not fixed
by qualification: manager must reject the incomplete restore and replace the
process. The truncated-main case returned HTTP 400.
All four owned engine PIDs exited normally; no owned processes remained.
Host-wide sampled RAM/VRAM maxima were 20866.88/23298 MiB with 89 samples and no
sampling errors. These are sampled observations, not allocation upper bounds.

## Manager campaign

`manager-qwen-4slot-01` passed in 990.9 seconds, with no owned processes left
after cleanup. Process 45696 automatically saved all four slots and exited;
process 31280 restored them before routing and completed real inference.
Every slot retained 2601 logical tokens in 651931792 bytes.

| Case | Observed result |
| --- | --- |
| Normal stop and fresh process | All four states restored; text/token IDs, draft counters and reuse matched live references |
| Latest-pointer write denied | Save failed; old pointer and all payload hashes stayed unchanged |
| Last-slot SDFT version invalid | PID 45352 was never observed routable and exited; cold replacement 67056 inferred on every slot |
| Last-slot main state truncated | PID 77732 was never observed routable and exited; cold replacement 1892 inferred on every slot |
| Recovery after each fault | A further new process restored the untouched good generation and inferred on all slots |
| Idle and forced exit | A busy nonzero slot blocked saving; slot 0 retained 2601 tokens while slot 3 advanced to 3171; both restored and inferred after forced exit |
| Controller exit and restart | Old controller and engine exited; new controller restored all slots and completed inference |
| Sparse generation | Only populated slot 3 was saved, restored and used for inference |
| Negative contract | Wrong idle-cache setting, slot count, context, draft model, draft KV or checkpoint count was rejected |

The measured idle save took 55264 ms and committed 2647447048 bytes. Routing
is held during this operation; this remains a substantial pause, not a
low-latency save promise. No deployed instance's save interval was changed.
The largest observed engine peak working set was 3368972288 bytes. Host-wide
sampled RAM/VRAM maxima were 23141.99/23780 MiB, with 284 successful samples
and two sampling errors; the gaps rule out a continuous peak claim.

The completed multi-slot report SHA-256 is
`ae6ad63a7cdb873a1274462c27bb57e04852c6d2d8dc1b89299528a717682c73`.
Raw reports and logs are retained in the isolated
`2026-10-27/b11913-checkpoint-validation` campaign. The candidate manager SHA-256
is `da681c37b6208382ca153b3d778b9f8050f6d7768d390ce2e16b8f9e9593c05b`;
the CLI SHA-256 is
`e1102f6d0e92feb07006a3ac834fd1f015e958aadab80d452cbbb25d7a217693`.

`lifecycle-qwen-01` passed in 442.6 seconds. Normal Windows stop published a
private n-gram candidate, denied writes and forced exit retained the old pointer,
and background-service shutdown preserved a valid fallback. All outcomes kept
`full_flush_verified=false`.

The single-slot MTP (21528 -> 58256), DFlash (75476 -> 2284), and same-model
ordinary-draft (17604 -> 70300) cases each confirmed old-process exit before
automatic fresh-process restore. Every restored continuation matched live text,
token IDs and draft counts, reused 65 tokens and accepted 35/35 drafted tokens.
These are short-prefix fixtures; the four-slot campaign above supplies the
actual retained-context-checkpoint coverage. MTP and DFlash with checkpoint
count zero also passed native API fresh-process inference, while manager
automatic persistence correctly remained blocked.

The lifecycle report SHA-256 is
`16b42056ab16d3e1879bb8a2c59615d7b57d82d328e3453d53a92f9c8ee0fe32`.
Sampled RAM/VRAM maxima were 22189.67/34776 MiB, with 156 successful samples
and two sampling errors.

`lookup-payload-01` passed the strengthened n-gram-only campaign in 132.7
seconds. The committed 3120-byte payload retained SHA-256
`ad2f256e07fa9dca5c60e46b291ce2352a6d5f5be2e8f5ce56f6b881c87f5e8a`
after native write denial, forced exit and a subsequent clean controller exit.
Both the old pointer and the data bytes are now checked, rather than inferring
payload preservation from an unchanged pointer. The report SHA-256 is
`9edc1e7ebeb500ebc1f609ce07d86b5c0eb2f4de7d97ad295b24403735a5c273`.
`--lookup-only true` reproduces this targeted check without rerunning the draft
matrix; it rejects combination with other campaign selectors.

All owned processes and loopback listeners were absent at closeout. Production
instance and path-authorization files were byte-for-byte unchanged. Rehashing
the launcher, DLLs and auxiliary assets reproduced the original runtime identity.
Local checks passed: production frontend build, release checks (186 browser
tests plus regression/type/release gates), 613 Rust manager tests (four ignored),
four CLI tests and all-target/all-feature Clippy with application warnings denied.

## Validation boundaries

Dynamic n-gram data is separate from target/draft KV and recurrent state. A
clean Windows exit may publish a structurally valid private n-gram candidate,
but upstream does not provide a verified write/close receipt:
`full_flush_verified` must remain false. Forced termination retains an already
committed generation; it cannot promise to save changes still in memory.

Read-only-file and pointer-replacement faults exercise real Windows write
failures, not physical disk-full, close-error or power-loss durability. Concurrent
writers and the GUI direct-owner application-exit path are not separately
qualified by this background-service campaign. Routing protection applies to
manager-admitted requests; direct native API traffic can bypass admission.

The deterministic repetition fixtures check restoration correctness and draft
continuity. Their acceptance ratios are not conversation-quality or performance
benchmarks. Neither sampled host RAM/VRAM nor process peak working sets prove an
allocation upper bound. No Linux/CUDA/CPU result is substituted for this GPU run,
and its result is not extended to wave64, VMM or partial MoE offload.

## Reproduction and rollback

Use the existing `scripts/test-checkpoint-native-real.cjs`,
`scripts/test-manager-multislot-real.cjs` and
`scripts/test-manager-cache-lifecycle-real.cjs` with absolute paths, a new private
`--root`, a free loopback `--port` and the fixed local `--engine`. Manager runs
use `--profile windows-rocm-b11913-rocm103-sckp1-sdft1`. The native run has
`--mode mtp --parallel 4`; the multi-slot manager run has `--parallel 4`.
The lifecycle run supplies the MTP above and Qwen3.8-27B-DFlash2-Q4_K_M.
Keep GPU campaigns sequential and retain their reports, logs and owned-PID checks.

Revert this profile through a PR if qualification must be withdrawn. Keep the
existing engine/configuration and old checkpoint generations intact. A different
runtime identity starts in its own namespace; do not relabel old cache manifests.
Stop only campaign-owned services and engines through their controlled paths.
Before opening schema-2 test data in an older manager, retain an untouched backup
and use a compatible schema-1 backup or start cold.

No installed engine, production instance configuration, llama.cpp source,
release tag or published application is changed by this qualification.
