# b11849 Windows / ROCm checkpoint acceptance

The later [Gemma 4 MTP acceptance](GEMMA4_CHECKPOINT_ACCEPTANCE.md) adds an exact
model-pair scope for f16/q8_0 at 4096/16384 context. The Qwen evidence and generic
build restrictions below remain distinct from that follow-up.

The subsequent [manager multi-slot acceptance](MULTI_SLOT_CHECKPOINT_ACCEPTANCE.md)
adds narrowly qualified four-slot Qwen 27B and Gemma 31B MTP scopes. Native results
in this historical report alone do not grant manager multi-slot eligibility.

## Artifact and evidence scope

The candidate uses the [official b11849 Windows ROCm archive](https://github.com/ggml-org/llama.cpp/releases/tag/b11849),
commit `0ae0e1976670fac25f4e0ed864df30021accc764` (the actual tag target),
with an isolated copy of the complete local ROCm
`10.3.0a20261023` runtime previously used by b11838. It is **not** qualification
of the unchanged official ZIP, every build named b11849, or every AMD GPU.

On this host the original archive did not enumerate a GPU: its `ggml-hip.dll`
imports `hipblas.dll`, which was absent, and Windows LoadLibrary returned 126.
The original ZIP and extracted directory were preserved. The derived directory
contains the fixed runtime DLLs, rocBLAS/hipBLASLt data and sibling `.kpack` tree.
Only this complete content identity is eligible for the new profile.

| Identity | SHA-256 |
| --- | --- |
| Original official ZIP | `5120074589ccb380eccc8170315ee78dac7eb7c6aec8d826aa33aef5d8e7abcf` |
| b11849 launcher | `c0f82373ff2fbf69f764a37460d28870018ce5d3808495e02aa3db064aaf68a4` |
| Launcher + 42 adjacent DLLs | `0f0a3a4da5876f98c8e8c84280c86dd47edd9a46472cfb6d7df782aaa6efa022` |
| Above + 272 auxiliary files | `354bb24c430c58dc0c67155e0ada89a4eaf0882e7672f193b17441d057423c55` |

The new registry entry requires `rocblas/`, `hipblaslt/` beside the executable
and `../.kpack/`. Their relative names and full content hashes participate in
the v2 identity and hence checkpoint namespaces and managed lookup namespaces.
Missing, changed or additional assets cannot inherit the qualification. Empty,
unreadable and linked trees fail closed. Existing reviewed b11769/b11838 v1
identities remain unchanged. Probe revision 3 invalidates stale capability caches.

The cache now retains at most 4096 file identities (previously 256) and writes
artifact-set hash-cache updates in batches. The original limit caused this
315-component runtime to evict entries during its own probe, repeatedly hashing
unchanged files. The bounded batch implementation retains metadata/file-identity
validation and full hashes on changes. A 320-file regression checks warm-cache
retention, changed-file invalidation and missing-file rejection. The first manager
failure campaign was stopped before inference when this performance regression
was discovered; it is not counted as successful acceptance.

Host: AMD Radeon 8060S, `gfx1151`, physical wave32, Windows, VMM unavailable.
Every test process received `GGML_CUDA_NO_PINNED=1`; no global environment or
saved production settings were changed. Models were Qwen3.8-27B UD-Q4_K_M,
its Q4_0 MTP head and Qwen3.8-27B DFlash2 Q4_K_M. Ordinary draft used the same
GGUF as the target. Full target/draft GPU offload, FA on, fit off, f16 KV and
deterministic sampling were used. These results do not establish wave64/MI100,
VMM, partial MoE offload, quantized KV, different-model simple draft or EAGLE3.

## Reproducible acceptance

`scripts/test-checkpoint-native-real.cjs` takes absolute `--bin`, `--root` (new),
`--engine`, `--model`, `--draft`, plus `--mode mtp|dflash|simple`,
`--parallel 1|2|4` and a free loopback `--port`. It never enables automatic
manager persistence to qualify an unreviewed candidate. Four slots use a shared
16384-token pool, not an assertion of a 4096-token context allocation per slot.

The bounded evidence reader reports GGSQ v4, SCKP v1 checkpoint payloads,
SDFT v1 full draft state and speculative blob lengths separately. It does not
deserialize tensors or prove semantic model compatibility. Assertions require
complete byte/token consumption, old PID exit, a fresh PID, real generation,
matching deterministic token IDs/text, matching draft counts and nonzero reuse.

| Case | Required evidence |
| --- | --- |
| Four live slot states | Separate saved files, restore all into a new process, infer each slot |
| Long-prefix rewrite | Change the answer from 391 to 437; a late rewrite must reuse an earlier checkpoint, while a rewrite before all retained checkpoints must re-prefill. Both match cold references, then every slot infers correctly. |
| Truncated main state / invalid SDFT version | Native failure or incomplete byte consumption, subsequent slot inference, old files unchanged |
| RAM prompt-cache rejection | `--cache-ram 1` actually logs rejected over-limit candidates; all slots still infer |
| Recovery after faults | Another new process restores the original immutable files and infers correctly |
| Manager failure isolation | `--failure-only true` on the lifecycle script injects disposable manifest-valid/native-invalid generations; observes failed PID unroutable and gone, replacement cold PID, quarantine marker, real inference, unchanged auto-restore policy, then good-generation automatic restore |

The lifecycle script retains its full Windows stop/write-denial/forced-exit/
daemon-shutdown matrix and `--idle-only true` long-prefix retention campaign.
It requires the exact `--profile windows-rocm-b11849-rocm103-sckp1-sdft1`.
No test imports a real instance configuration. The new evidence-reader tests run
inside `test:regressions` and the normal release/CI checks; GPU campaigns remain
explicit opt-in local acceptance, not claims about CPU CI.

## Native observations (2026-10-24)

| Run | Observed result |
| --- | --- |
| `native-mtp-4slot-03` | Four slots passed all native cases with the optimized manager. Each saved 2347 tokens in 634232056 bytes, including two SCKP records and 9669696 bytes of full draft state. Fresh-process inference reused 2347 tokens, evaluated 1 prompt token and accepted 29/29 proposed draft tokens. |
| `native-dflash-1slot-01` | All native cases passed. 745674768-byte state, two SCKP records and 42048696 bytes of full draft state. The same 2347-token reuse and 29/29 acceptance for this fixture. |
| `native-simple-1slot-02` | All native cases and normal owned-process cleanup passed. 1249115164-byte state, two SCKP records and 310764092 bytes of full draft state; 2347 reused tokens and 29/29 acceptance. |
| `manager-failure-02` | Both manifest-valid/native-invalid generations were quarantined. Failed PIDs 39000/32872 were never observed routable and exited; replacement PIDs 3216/46040 performed correct cold inference. Each subsequent fresh-process automatic restore of the original generation reused 1655 tokens and matched live tokens/text/draft counters. Auto-restore stayed enabled and original files retained their hashes. Normal cleanup passed. |
| `manager-full-01` | Windows graceful stop, write denial, forced termination and service shutdown passed the managed lookup-cache assertions. MTP, DFlash and same-model simple draft automatically restored in fresh PIDs, reused 65 tokens, evaluated 1 prompt token and accepted 35/35 draft tokens with matching output. MTP/DFlash short-prefix native restores with zero checkpoints also inferred correctly, while manager automatic eligibility remained blocked. Quantized draft KV, context 8192, EAGLE3 and parallel=2 were rejected. Normal cleanup passed. |
| `manager-idle-01` | Optimized manager build: MTP and no-draft idle saves retained 1703-token generations after short-title requests and forced exits. Fresh PIDs restored and reused all 1703 tokens, evaluated 1 prompt token and matched live token IDs/text. MTP accepted the same 35/35 draft tokens. Original latest-pointer hashes stayed unchanged; normal cleanup passed. |

In the final four-slot MTP run, the answer-changing late rewrite reused 1808
tokens and evaluated 504; the earlier rewrite reused zero and evaluated 2312.
Both generated the new answer `437`, matched cold-reference tokens/text and
accepted 29/29 draft tokens. All four slots inferred after each fault, and another
fresh process recovered the original immutable files after RAM-cache rejection.
The earlier `native-mtp-4slot-02` run had only tested a rewrite that fell back to
full prefill; it is not the evidence for the new partial-reuse assertion.

The first simple-draft run passed inference assertions but needed forced cleanup
of one remaining management host after its 45-second deadline. It is retained
as a failed campaign, not silently counted as a pass. The complete second run
passed normal cleanup; the first host-exit delay was not reproduced or explained.
The second run's process peak working set was 3212972032 bytes. Its sampled
host-wide RAM/VRAM maxima were 26006.7/32714 MiB, including concurrent development
work, and therefore do not establish isolated per-process GPU allocation bounds.
The fixed manager's warm capability checks measured 254–549 ms, compared with
30319 ms during the earlier cache-thrashing campaign. These debug-build samples
are not production latency guarantees. Model hashes seeded from earlier isolated
acceptance remained subject to the normal file-identity/size/timestamp validation;
the native campaigns also independently hashed the selected model files.

An invalid SDFT version returned HTTP 200 but stopped before the draft appendix
(`624562336` of `634232056` bytes for MTP). This is **incomplete restoration**.
The manager's existing byte-count and round-trip checks must reject it and
replace the process before routing. Native same-process recovery after a failed
request is recorded only as that narrower observation.

The tested modes serialize zero extra speculative-blob bytes. In the pinned
[speculative implementation](https://github.com/ggml-org/llama.cpp/blob/0ae0e1976670fac25f4e0ed864df30021accc764/common/speculative.cpp),
the nonempty get_state/set_state override belongs to EAGLE3; simple, MTP and
DFlash use the default empty blob. Therefore these tests cover all state exposed
by the tested modes, including main/draft KV and hybrid recurrent state, but
do not certify a nonempty EAGLE3 speculative-state restore. EAGLE3 remains blocked.

Four-slot native results in this report alone did not enable manager multi-slot
persistence. Later exact exceptions are documented in the linked multi-slot
acceptance; generic single-slot and draft restrictions remain. No SWA bitwise-logits claim
is made. The repeated arithmetic fixture is a correctness/recovery sentinel,
not a representative performance or draft-acceptance benchmark.

Reports retain process-lifetime peak working sets when sampled before stop and
2-second host-wide RAM/VRAM telemetry maxima. Host GPU telemetry includes other
processes; it is not a per-process peak, and neither measurement is an allocation
upper bound. Disk-full/close errors, concurrent disk writers, power-loss durability,
nonempty EAGLE3 state and GUI-driven legacy direct-owner shutdown remain separate
validation gaps. Native write completion is not certified by a clean exit alone.
The lookup-cache tests continue to require `full_flush_verified=false`; publishing
a structurally valid private candidate does not prove native n-gram write/close
completion. No test equates this lookup data with full model/draft state.

Local validation also passed 599 manager Rust tests and four CLI tests, strict
Clippy, the production frontend build, and `check:release` including 185 browser
tests. Three existing opt-in Rust tests remain ignored by the normal suite.
The five bounded evidence tests run in normal CI; the real GPU runs remain
separate opt-in evidence. The manager full matrix took 14.7 minutes, failure
isolation 5.8 minutes and optimized idle/retention acceptance 4.3 minutes on this
host; the final four-slot MTP campaign took 3.9 minutes. These exclude compilation
and initial fingerprinting. These are observed
validation costs, not performance promises.

## Rollback

Remove the b11849 profile through a normal revert PR, preserving historical
cache files. The next probe/launch refuses automatic draft persistence for the
unqualified bundle. The older reviewed profiles and namespaces are unchanged.
Do not switch engines or edit saved instance settings to roll back this change.
Stop only campaign-owned PIDs, verify their paths/command lines and loopback
ports, and retain the report, raw logs and runtime provenance. Reverting the
acceptance scripts has no effect on running engines.
