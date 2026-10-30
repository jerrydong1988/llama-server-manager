# Local b12000 Windows/ROCm checkpoint acceptance

## Build identity and contract

This campaign qualifies the local
`build-hip-rocm103a20261029-vs2026-b12000/bin` bundle on Windows with Radeon
8060S, driver `32.0.32015.7029`, with `ROCm : NO_VMM = 1`. The embedded build and official
[b12000 tag](https://github.com/ggml-org/llama.cpp/releases/tag/b12000) resolve to
`68d20b884c3878217de79eade0a249efde96ab38`. An ancestor check confirms
[#31350](https://github.com/ggml-org/llama.cpp/pull/31350); the later batched-copy
change #30881 is not in this build.

| Identity | SHA-256 |
| --- | --- |
| Launcher | `71188bbb8f78e0a53c78c642b0295abdf80e9e6c7cfdaae1ed372c05b26cdd0a` |
| Launcher and 28 adjacent DLLs | `2ff974ba6d955ea6f3453d85e09c020e143fea2583102ae053b10f4c8ea9dd34` |
| Complete runtime, including 272 auxiliary files | `0b6d1705b4b0daf2a5f768d1e7e5bb450c0bcd65a01709502fe7553ba9c09e56` |

The auxiliary digest covers `rocblas`, `hipblaslt`, and sibling `.kpack` using
runtime-set-v2. Loaded-module inspection verified the running HIP, rocBLAS,
hipBLAS, COMGR and kpack DLLs came from this bundle. This evidence does not
qualify arbitrary b12000 builds or the official ROCm 10.0 package.

Persistence revision 7 requires a fresh capability probe. Model scopes now
describe allowed speculative combinations; individual algorithm support cannot
qualify a mixed pipeline. Existing scopes default to pure `draft-mtp`. Mixed
draft configurations without a matching reviewed model scope fail closed.

The new four-slot scope is restricted to total context 16384, f16 target/draft
KV, unified KV, two checkpoints, full GPU offload, FA on, fit off,
`cache_idle_slots=false`, and three MTP draft tokens. It uses these exact files:

- Qwen3.8-27B UD-Q4_K_M:
  `322e194ff79741c7baa497c240f677f54b201b0efab44ca8e50f122b39123482`.
- Q4_0 MTP:
  `50d9ce5a6da381bbcfb31061cf73df94a90e6faf8efeddee379a9cb8f1501c6e`.

The strategies are pure `draft-mtp` and `ngram-mod,draft-mtp`, with default ngram
parameters. Unreviewed combinations, custom ngram overrides, two-slot topology,
Gemma, quantized KV, larger contexts and multimodal persistence are not added.
Single-slot pure draft retains the existing 4096-context/f16/full-offload
restrictions; ordinary draft must use the same GGUF as the target. Single-slot
mixed draft is not qualified by the four-slot scope.

## Native recovery and rollback

`native-qwen-4slot-01` passed in 439.7 seconds. PID 78468 exited before PID 57256
restored all four slots and completed real GPU inference. Output text, token
IDs, draft counts and prompt reuse matched the live references. Prefix edits,
truncated target state, invalid SDFT version, RAM-cache rejection and recovery
from the unchanged good files all passed. All four engine PIDs exited.

`native-mixed-4slot-03` passed in 314.9 seconds. After the no-draft reference,
mixed PID 42064 exited before PID 59128 restored the four slots. The same strict
fresh-process comparisons passed, followed by long-prefix rollback, full
recomputation, malformed-state inference, RAM-cache rejection and a final new
process recovering the original good files.

The targeted mixed fixture observed **61 accepted out of a 64-token draft**:
three rejected tokens with `n_rs_seq=3`. Native trace explicitly records
speculative checkpoint restoration. Its result matches a separate no-draft
process. A subsequent **unconstrained** continuation reuses the corrected cache
and matches the no-draft reference in text and token IDs. The grammar used to
place the rejection is a controlled rollback fixture, not general model-quality
evidence. One- and two-token rejection cases were not separately measured.

The pinned HIP dispatch requires RDNA3.5 and at least 2048 tokens to select its
MMA slice path. From that dispatch and the logged 65-token verification batch,
the exercised path is inferred to be HIP autoregressive. This is not a GPU
profiler trace and does not validate CUDA's 64-token MMA path or all HIP kernel
shapes. This boundary remains explicit despite successful checkpoint restoration.

The two earlier mixed attempts remain in the evidence directory. The first
stopped at an overstrict same-process health comparison: ngram learning changed
draft counts from 119/119 to 125/125 while output and acceptance rate stayed
identical. Only that health comparison now allows equal acceptance rates;
fresh-process restore still requires exact counts. The second completed the
recovery matrix but could not assert accepted/total rollback counts because
that native trace line requires `LLAMA_TRACE=1`. The final run enables trace
only for its isolated processes and requires the actual short-rejection event.

## Manager automatic persistence

`manager-mixed-4slot-01` passed in 1036.7 seconds. PID 61144 exited after
automatic stop-save; PID 41552 restored all four slots and matched reference
output, token IDs, draft counts and prompt reuse. The campaign also passed:

- Denied final-pointer replacement retained the old manifest and payload hashes.
- Invalid draft version and truncated target state in the last slot isolated
  the entire partially restored process. Rejected PIDs 31276 and 35200 were
  never routable; replacement processes performed cold inference on all slots,
  and the previous good generation restored successfully in further new PIDs.
- A busy nonzero slot blocked idle saving. Shorter per-slot requests retained
  the committed longer prefix. Idle generations recovered after forced exit.
- Controller shutdown/restart restored all four slots. Sparse generations
  restored populated slots while untouched slots remained usable.
- Nine unreviewed shape, model, algorithm-combination and override cases were
  rejected. All 14 distinct engine PIDs exited; cleanup found no owned process.

`manager-qwen-4slot-01` independently passed the same manager matrix for pure
MTP in 1080.4 seconds, with an empty owned-process list after cleanup.

`lifecycle-qwen-01` passed in 482.6 seconds. Single-slot MTP, DFlash2 and
same-GGUF ordinary draft each exited the old process, restored in a new PID and
completed matching inference with two configured checkpoints. MTP and DFlash2
also passed the zero-checkpoint native-API boundary cases; manager automatic
persistence remained blocked for that configuration. Unqualified draft launch
settings stayed blocked. The DFlash2 Q4_K_M model SHA-256 was
`1a25c56858e1ebe93f2718ac1d49d1151f9323325c1bbfd6209370f4db131ebd`.

Both that campaign and the independent `lookup-payload-01` directory (128.2
seconds) exercised instance stop and background-service shutdown. Each produced
a nonempty 3120-byte dynamic ngram payload, then verified its exact old hash
after denied native writes, forced termination and clean shutdown. A final
cross-campaign check confirmed both old payloads still matched
`ad2f256e07fa9dca5c60e46b291ce2352a6d5f5be2e8f5ce56f6b881c87f5e8a`.
Publishing a private candidate still reports `candidate_committed_flush_unverified`;
these results do not assert native destructor execution on force-kill or
complete filesystem durability.

Final closeout found no owned engine/controller/CLI process or test listener on
ports 19000-19013. All six production configuration files retained their
baseline hashes, and installed manager PIDs 22132/63760 remained unchanged.
Rehashing the complete b12000 runtime and all three model files matched the
tested identities. No engine selection, production instance configuration,
version tag or release was changed.

## Local validation and retained evidence

`npm run build`, `npm run check:release` (including 187 browser tests), the full
locked Rust test suite and all-target/all-feature Clippy with warnings denied
passed. The final focused capability suite passed 22 tests; checkpoint evidence
passed seven tests, including strict fresh-process counters and bounded format
parsing. The tested manager/CLI binaries had these SHA-256 identities:

- Manager: `43ae8ead7814abc1b79bc2691df83d4b334f13dffc615dc93b22edd829b501dc`.
- CLI: `4ff3630738ff85b2ef31ef8771a48d6b91e7ced639796df0a9c59c2045a034fd`.

Each entry below identifies the retained `report.json`; `acceptance-summary.json`
also records final model/runtime/configuration hashes and process cleanup.

| Campaign | Report SHA-256 |
| --- | --- |
| native-qwen-4slot-01 | `9957821c74c1f8c0b780873cd46660a0fe996e82fdcbd5a7ed028014613b0e91` |
| native-mixed-4slot-03 | `c500ab19fa73c39551e771be6f532d5b18d9ca2c6a59aa6205c50faf1ab18381` |
| manager-mixed-4slot-01 | `351ee65f068786011328461d403c6aaba11e642bd89405b44cd62d4f81d110d9` |
| manager-qwen-4slot-01 | `7af25502df8e012538a023157251a046747b5c0d0ccaab76d73042ed598af19b` |
| lifecycle-qwen-01 | `d7052e37658eaa26adac308abbd00b17eeac182c1de7ab46581e561d600b1d58` |
| lookup-payload-01 | `a06406c363ba714458ff17a84153e0ab82f3d26d2b3592568d12500356cfe864` |

## Evidence boundaries

The files use GGSQ v4, SCKP v1 and SDFT v1, with nonempty target and draft
KV/recurrent payloads. The speculative blob is empty for this MTP implementation.
This is not a qualification of arbitrary nonempty speculative-state serializers.
The process-wide ngram table is rebuilt from prompt data; unrelated learned
ngrams and all historical per-mode counters are not serialized by SDFT.

Native partial draft restoration can still return HTTP 200 while consuming
fewer bytes. Manager must reject it, quarantine the generation and replace the
uncertain process before routing. Dynamic ngram files are separate from these
KV/recurrent payloads. Forced termination cannot promise to save in-memory
changes, and `full_flush_verified` remains false.

| Campaign | Host RAM / VRAM maximum (MiB) | Samples / failed telemetry calls |
| --- | --- | --- |
| Native pure MTP | 21845.80 / 23970 | 117 / 0 |
| Native mixed | 29703.84 / 24294 | 122 / 0 |
| Manager mixed | 28723.01 / 25853 | 260 / 2 |
| Manager pure MTP | 28974.83 / 25499 | 290 / 4 |
| Windows lifecycle | 26399.76 / 35220 | 164 / 2 |
| Independent ngram directory | 24897.11 / 19877 | 26 / 2 |

These are host-wide samples at a nominal two-second interval, including the
listed telemetry gaps. The native mixed run overlapped local compilation.
They are not per-engine allocation bounds or comparable performance benchmarks.
Total configured context 16384 is split across four slots; it is not a
single-slot 16384-token stress result. Matching generated tokens does not imply
bitwise logits equality. Disk-full, close/power-loss and concurrent-writer
durability are not established by read-only-file faults.

Raw reports, logs, source excerpts and identities are retained under the
isolated `2026-10-30/b12000-checkpoint-validation` campaign. Reproduce native
mixed acceptance with `scripts/test-checkpoint-native-real.cjs --mode mtp-ngram
--parallel 4 --predict 128`; supply absolute engine/model/draft/manager paths,
a new `--root`, and a free loopback port. `GGML_CUDA_NO_PINNED=1` is confined to
the test process tree. The campaign never uses the default manager data root.
