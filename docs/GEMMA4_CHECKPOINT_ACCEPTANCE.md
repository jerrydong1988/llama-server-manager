# Gemma 4 MTP checkpoint acceptance on b11849

## Fixed candidate and qualification boundary

This extends the [b11849 acceptance](B11849_CHECKPOINT_ACCEPTANCE.md) for one
specific model pair and the same complete Windows ROCm 10.3 runtime. The original
official archive, missing `hipblas.dll` on this host, remains unqualified unchanged.
The [official tag](https://github.com/ggml-org/llama.cpp/releases/tag/b11849) resolves
to `0ae0e1976670fac25f4e0ed864df30021accc764`; a version string alone is insufficient.

| Artifact | SHA-256 |
| --- | --- |
| Launcher, 42 DLLs and 272 auxiliary assets | `354bb24c430c58dc0c67155e0ada89a4eaf0882e7672f193b17441d057423c55` |
| Gemma 4 12B IT UD-Q8_K_XL target GGUF | `a6ba22cbee602e69cb27c88bc73768db0ee4bda60831898e6db8cb1856e4162a` |
| Gemma 4 12B IT Q8_0 MTP GGUF | `145db9094bc0f85f1701e255a2ed216dcc9800fc8bc8631ad00905b456bd451b` |

Host: Windows, Radeon 8060S / gfx1151 / physical wave32, VMM unavailable.
`GGML_CUDA_NO_PINNED=1` is inherited only by isolated validation processes.
Target and MTP offload all 49/49 and 5/5 layers respectively. FA is on, fit off,
one unified KV slot, two context checkpoints, three maximum draft tokens, and
native windowed SWA (`n_swa=1024`). The candidate matrix is explicit context
4096 or 16384, with all target/draft K/V types consistently f16 or q8_0.
It does not qualify intermediate context sizes, q4 KV, mixed KV types, other
Gemma weights, different draft heads, multiple slots, partial offload or EAGLE3.
The 16K setting was exercised with about 11.5K prompt tokens and up to 1536
generated tokens, not a full-capacity 16K prompt or context-shift soak.

The [open, unconfirmed upstream report #31077](https://github.com/ggml-org/llama.cpp/issues/31077)
concerns Windows/CUDA and a 31B derivative. It motivates rollback and full
recomputation stress; passing the 12B ROCm matrix is not a reproduction or fix
of that report, nor proof for MI100/wave64 or another GPU.

In the pinned [MTP implementation](https://github.com/ggml-org/llama.cpp/blob/0ae0e1976670fac25f4e0ed864df30021accc764/common/speculative.cpp),
Gemma shares the target KV (`is_mem_shared`). The observed SDFT v1 live draft
state is eight bytes and the additional speculative blob is empty. GGSQ v4
contains the shared target KV, and SCKP v1 records the retained checkpoints.
Complete restoration here means all state exposed by this specific shared-KV
mode, not independent draft KV, hybrid recurrent state, or nonempty EAGLE3 state.

## Acceptance procedure

Both harnesses require a new isolated data root and an unused loopback port.
They verify ownership before stopping processes and fail a campaign if normal
cleanup needs force. Real instance settings and the selected production engine
are never imported or changed.

`scripts/test-checkpoint-native-real.cjs` keeps automatic manager persistence
disabled while qualifying the candidate. Supply absolute `--bin`, `--root`,
`--engine`, `--model`, `--draft` plus `--mode mtp --parallel 1 --port 18850`.
Use `--context 16384 --kv q8_0 --records 640 --cycles 2 --stress-predict 1536`
for the long quantized case; substitute f16, or 4096 with 140 records, for the
other cases. `--checkpoints 0` and `--mode none` are explicit controls.

| Check | Required evidence |
| --- | --- |
| Complete cross-process restore | Save full byte/token counts; old PID gone; fresh PID; full native read; actual completion equals live token IDs/text and draft proposal/acceptance counts; reuse at least all saved tokens |
| Changed prefix | Change answer 391 to 437 and compare with a separate short ungrammared cold reference; partial reuse for a late edit, zero reuse before retained checkpoints |
| Actual checkpoint rollback | First generate 1536 tokens to move beyond the SWA window, then edit the suffix; require `restored context checkpoint` in new log bytes, partial reuse and 1536 completed output tokens |
| Full recomputation | `cache_prompt=false`, zero reused tokens, actual long prefill and 1536 output tokens |
| Corrupt main / draft appendix | Disposable copies only; reject failure or incomplete byte count even for HTTP 200; real inference afterward; original hashes unchanged |
| RAM-cache rejection | Log evidence for `exceeds cache size limit`, then inference and another fresh-process restore of the original disk states |
| Manager isolation | Automatic Windows stop/save and new-process restore; inject manifest-valid/native-invalid disposable generations; failed PID never observed routable and gone; replacement cold inference; restore original generation afterward |

Long output stress uses a grammar enforcing repetition of the selected answer;
short correctness and cross-process reference completions are ungrammared.
Grammar stress measures the exercised cache/generation paths, not broad model
quality. Exact token IDs on these fixtures do not establish bitwise logits.

`scripts/test-manager-cache-lifecycle-real.cjs --failure-only true` takes
`--context`, `--kv` and `--records`, plus absolute `--bin`, `--root`, `--engine`,
`--model`, `--mtp`, a free `--port`, and
`--profile windows-rocm-b11849-rocm103-sckp1-sdft1`. Use the candidate manager
built with the exact qualification entry, after native acceptance succeeds.
The inherited broader Windows lookup-cache exit matrix is documented separately;
dynamic n-gram files are not complete model/draft checkpoints.

## Manager contract

`persistence_profiles.json` carries a nested model-pair scope instead of widening
the old build-wide f16/4096 ceiling. `engine_capabilities.rs` matches model
content hashes, the reviewed launch shape and its effective explicit overrides.
Stored values omitted from the command cannot qualify that launch. `server.rs` hashes on a blocking
worker and checks the final launch fingerprint again before preparing scratch
state. A changed runtime or mismatched final model identity cannot retain the
earlier eligibility. Probe revision 4 invalidates stale capability caches.
Checkpoint eligibility revision 4 also prevents a different-policy background
service from trusting the persisted launch decision after upgrade or rollback.
`runtime_service/supervisor.rs` rehashes the complete launch fingerprint on
background recovery, including target, draft, template and state-bearing
configuration, rather than rechecking only engine files. Drift disables that
persisted checkpoint decision before the process can restore or save state.
`checkpoint.rs` accepts windowed SWA only from this reviewed decision, with
context persistence and a nonzero checkpoint count; help flags alone cannot
enable it. Existing manifest hashes, immutable generations, round-trip checks
and failed-restore process replacement remain mandatory.

## Evidence and remaining limits

| Completed native campaign | Observation |
| --- | --- |
| `native-f16-4k-03` | Full campaign passed in 6.95 minutes, including two actual checkpoint rollback/full-recompute cycles with 1536 output tokens each, fresh-process inference, both corrupt-file cases, RAM-cache rejection and normal cleanup. |
| `native-q8-4k-01` | 2488 saved tokens in 556509336 bytes; two SCKP records and eight live-draft bytes. Fresh process reused 2488 tokens, evaluated one, generated 40 with 29/29 draft acceptance. Actual rollback reused 1948 and re-evaluated 505 tokens, then generated 512 with 383/383 accepted; full recomputation, corruption recovery and normal cleanup passed. |
| `native-q8-4k-02` | Final stronger repeat passed in 6.79 minutes. Both observed rollback cycles reused 1948 and re-evaluated 505 tokens; both full recomputations reused zero and evaluated 2453. Each generated 1536 with 1151/1151 draft acceptance. Complete fresh-process restore, both corrupt-file cases, RAM-cache rejection, immutable originals and normal cleanup passed. |
| `native-q8-16k-03` | Full campaign passed in 9.43 minutes. Saved/reused 11488 tokens in 634989336 bytes. Both observed checkpoint rollbacks reused 10948 tokens and re-evaluated 505; each generated 1536 with 1151/1151 draft acceptance. Both full recomputations reused zero and evaluated 11453 tokens. Fresh-process inference matched 40 token IDs/text with 29/29 acceptance. Fault recovery, immutable originals and normal cleanup passed. |
| `native-f16-16k-02` | Same complete matrix passed in 9.24 minutes, including both 1536-token rollback/recompute cycles, 11488-token new-process reuse and 29/29 short-completion draft acceptance. Saved state was 1195076376 bytes; both rollbacks reused 10948 and re-evaluated 505. Fault recovery and normal cleanup passed. |
| `native-q8-16k-cp0-01` | Zero-checkpoint control passed native save/new-process restore, full recomputation and faults. 11488 saved/reused tokens in 278446828 bytes; eight live-draft bytes and no SCKP. Manager automatic eligibility remains blocked for this shape. |
| `native-q8-16k-no-mtp-01` | MTP-off control passed rollback, full recomputation and fresh-process recovery. 11488 saved/reused tokens in 634989288 bytes, two SCKP records and no SDFT. This does not independently qualify no-draft automatic windowed-SWA persistence. |

| Completed manager campaign | Automatic stop/save and fresh-process reuse |
| --- | --- |
| `manager-q8-16k-01` | Passed in 4.90 minutes; 11476 logical tokens, 634884696 bytes; new process reused all 11476, evaluated one, and generated 48 with 35/35 draft acceptance. |
| `manager-f16-16k-01` | Passed in 4.56 minutes; 11476 logical tokens, 1194879576 bytes; the same complete reuse, output equality and 35/35 draft acceptance. |
| `manager-q8-4k-01` | Passed in 3.55 minutes; 2476 logical tokens, 556404696 bytes; the same complete reuse, 48-token output equality and 35/35 draft acceptance. |
| `manager-f16-4k-01` | Passed in 4.15 minutes; 2476 logical tokens, 1047279576 bytes; the same complete reuse, 48-token output equality and 35/35 draft acceptance. |

Each completed manager row also passed manifest-valid/native-invalid SDFT and
truncated-main injection. Both failed PIDs remained unroutable and exited,
replacement processes inferred correctly, and the previous immutable generation
restored successfully afterward. Seven unreviewed configuration mutations were
rejected. Normal service shutdown and engine exit required no forced cleanup.
The GGSQ packed-vector word count includes serialization metadata; the reuse
threshold is the logical `n_saved` count stored in the manifest, not that header
word count.

Across these four manager runs, all eight injected failed-restore PIDs were
observed unroutable before exit, all eight subsequent cold replacement requests
inferred correctly, and all eight later good-generation restores passed. The
28 out-of-scope eligibility checks were all refused. The sampled host RAM maxima
were 21573-24001 MiB; sampled host GPU memory
maxima were 16309-16729 MiB. Native stress had a larger host RAM sample of
26517 MiB and GPU sample of 16753 MiB.

Earlier incomplete campaigns are retained as failures: an initial path spelling
mismatch stopped before launch; an ungrammared 1536-token cold repetition drifted
from the intended answer before any restore; and two early stress fixtures did
not move beyond the SWA window, so their claimed checkpoint path was rejected
by the log assertion. They are not counted as successful full acceptance. The
final stress fixture separates short unconstrained correctness from constrained
long-generation stress and requires observed checkpoint restoration.

Local reports, raw server logs, process IDs and immutable hashes are retained in
the dated `b11849-gemma4-validation` campaign directory outside the repository.
The four final native cases (two cycles each), two native controls, and four
manager cases total approximately 66 minutes, excluding earlier attempts,
compilation and CI. Final audits found no campaign-owned processes or listeners
on ports 18850/18851; the production instance configuration hash was unchanged.
Host RAM/VRAM maxima
are two-second samples including concurrent development work; they are not
per-process GPU peaks or allocation upper bounds. Short prefixes with no SCKP,
disk-full/close-error injection, concurrent writers and power-loss durability
remain separate boundaries. A small deterministic fixture is not a long soak
or proof of general answer quality.

## Rollback

1. Revert only the new model scopes through a normal PR and bump both the probe
   and checkpoint eligibility revisions, leaving older b11769/b11838 and generic
   b11849 contracts unchanged.
2. Re-probe on the next normal manager launch. Unqualified windowed-SWA or
   quantized/long-context draft instances refuse automatic checkpoint recovery
   and use the existing cold-start path. No saved instance edit or engine switch
   is necessary.
3. Retain immutable generations and reports for diagnosis; do not delete or
   rewrite old caches as part of rollback. Stop only campaign-owned processes
   and verify their PIDs and ports are gone.

No release tag, installer deployment or production engine replacement is part
of this change. CI validates manager behavior; local ROCm runs supply GPU evidence.

Local validation also passed `npm run check:release` (185 browser tests plus
regressions and type checks), `npm run build`, 601 manager Rust tests and four
CLI tests, strict all-target/all-feature Clippy, and the release binary build
used by the manager campaigns. `npm run test:runtime-service` also passed on
Windows, including poisoned-process replacement and router quota compatibility.
Its checkpoint fixture obtains the current revision and canonical fingerprint
from a test-only Rust exporter instead of hardcoded fake identities. That
exporter is explicitly invoked by the IPC test; it is one of four tests ignored
by the default Rust test run and is absent from the shipped runtime binary.
