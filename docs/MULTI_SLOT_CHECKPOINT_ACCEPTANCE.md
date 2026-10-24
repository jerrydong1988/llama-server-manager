# Manager multi-slot checkpoint acceptance

## Contract and storage changes

Manager checkpoints can contain a bounded set of independent native slots. A
generation is published only after all selected payloads have been written,
synced, hashed and moved into one immutable directory. The latest pointer is
replaced last. Byte limits, reported token counts and pruning use the aggregate
payload size. A shorter slot can retain its previous payload while another slot
advances; an empty slot does not discard useful state in a different slot.

Multi-slot manifests use schema 2 and `slotCount`; single-slot schema 1 remains
readable and writable. IDs and filenames must be unique and within the qualified
topology. Every payload is checked before restore. All selected slots are restored
before any round-trip save/hash validation, so a later restore that damages an
earlier sequence cannot pass verification. Routing remains closed throughout.
A failed restore quarantines the generation and replaces the entire process;
successful earlier slots are not published from the partially restored process.

Idle saving observes every slot and requires a complete idle topology, no admitted
proxy requests and the configured quiet interval. Busy, missing, duplicate and
stale-process observations cannot trigger a save. Status reports saved slot IDs,
coverage and totals; measured reuse is attributed to its actual slot.

## Qualified shape

The runtime is the exact derived b11849/ROCm 10.3 bundle documented in
[B11849_CHECKPOINT_ACCEPTANCE.md](B11849_CHECKPOINT_ACCEPTANCE.md), not every binary
named b11849. Its artifact identity is
`354bb24c430c58dc0c67155e0ada89a4eaf0882e7672f193b17441d057423c55`.
The host is Windows / Radeon 8060S (`gfx1151`, physical wave32), with
`GGML_CUDA_NO_PINNED=1` set only for the isolated test processes.

The qualified registry extension is restricted to four slots, total context 16384,
unified KV, f16 target and draft KV, full GPU offload, FA on, fit off, two context
checkpoints, `draft-mtp` with three draft tokens, and these exact model pairs:

| Model | Target SHA-256 | MTP SHA-256 |
| --- | --- | --- |
| Qwen3.8-27B UD-Q4_K_M / Q4_0 MTP | `322e194ff79741c7baa497c240f677f54b201b0efab44ca8e50f122b39123482` | `50d9ce5a6da381bbcfb31061cf73df94a90e6faf8efeddee379a9cb8f1501c6e` |
| Gemma 4 31B Q4_K_M / Q8_0 MTP | `38bd64c852c4b460434cc7162fa9bdcf242faf86502581a754cb72956bb17f84` | `5ae8b0117bed601e8924c6305bd5b0585de361d51f0e77091bcb4252cf1f27de` |

Multi-slot mode additionally requires explicitly emitted `cache_idle_slots=false`.
The first manager campaign with this option enabled saved only slot 3: b11849's
`[TAG_IDLE_SLOT_CLEAR]` path moves other idle unified-KV slots into its RAM prompt
cache and clears the live slot. `/slots` persistence cannot serialize that RAM
cache as four live slots. This combination is now rejected with
`multi_slot_idle_cache_unsupported`; increasing cache RAM is not a remedy.
Single-slot mode keeps its existing idle-cache retention requirements. The UI
repairs only returned requirements and preserves a qualified four-slot topology.

No two-slot, quantized multi-slot, larger-context, other-model, ordinary-draft,
DFlash, EAGLE3, wave64, VMM or partial-offload qualification is added here. The
older single-slot Gemma 12B scope remains separate. Total context does not promise
four independently fixed allocations in the unified pool.

## Reproducible checks

`scripts/test-manager-multislot-real.cjs` requires Windows and absolute `--bin`,
`--root` (new directory), `--engine`, `--model`, `--mtp`, plus
`--profile windows-rocm-b11849-rocm103-sckp1-sdft1`, `--parallel 4` and a free
loopback `--port`. It creates a private manager data root and instance. It records
hashes, PID lifetimes, native formats, outputs, token IDs, draft counters and
sampled memory. It refuses to terminate a process outside its owned test root.

| Layer | Acceptance |
| --- | --- |
| Store/coordinator tests | Atomic set commit, fault after second move, aggregate quota, old-pointer retention, nonzero-slot activity, sparse generations, duplicate/path/hash rejection, per-slot long-prefix retention, later-slot restore failure and shared-state corruption |
| Browser tests | Repair preserves four slots and disables only incompatible idle caching; slot coverage and measured reuse display the actual slot |
| Manager real process | Populate all four slots; controlled stop auto-saves; old PID exits; fresh PID auto-restores all slots before routing; each performs real GPU generation matching live text/token IDs, draft counters and cache reuse |
| Failure isolation | Rehash a disposable last-slot payload with invalid SDFT version or truncated main state to reach the native reader; failed PID never observed routable and exits, replacement cold process infers on every slot; good generation and desired restore policy remain intact |
| Old-cache protection | Read-only latest pointer makes save fail; previous pointer/payload hashes remain unchanged; forced exit retains the committed generation |
| Idle and sparse saves | Busy slot blocks idle save; short slot 0 retains its longer checkpoint while slot 3 progresses; forced exit and a fresh process recover both; only populated slot 3 is sufficient for a sparse generation |
| Controller exit | Normal isolated daemon shutdown saves all slots; old daemon and engine exit before a new daemon restores and performs inference |
| Negative contract | Wrong parallel count, context, model/draft, draft KV type, checkpoint count or idle-cache setting is rejected |

Native Gemma 31B run `native-gemma31-4slot-02` passed four-slot restore, two
1536-token rollback/recompute cycles, answer-changing early rewrite, malformed
files and RAM-cache rejection. PIDs 51064, 51540, 33516 and 22840 exited. Each
saved file had 2488 tokens and 2720480152 bytes, SCKP v1 with two checkpoints and
SDFT v1. The MTP live draft state is an 8-byte shared-KV marker and speculative
payload is empty: this is not evidence for nonempty speculative/EAGLE3 state.
The earlier native attempt lost an HTTP keepalive connection after synchronous
large-file hashing; the harness now closes each HTTP connection and does not
retry mutations. Failed attempts are retained separately from passing evidence.

Manager Qwen run `manager-qwen-4slot-03` passed on 2026-10-25 (local time),
02:10:06--02:35:10, with normal cleanup and no remaining owned processes.
The initial process 8392 exited before process 26052 restored all four slots.
Each slot retained 2601 tokens in 651931792 bytes. Restored continuations matched
the live text, token IDs and draft counts; each reused 2601 cached tokens and
accepted 35/35 draft tokens. The live SDFT v1 draft payload was 10716176 bytes;
the two SCKP v1 checkpoints and their target/recurrent state were also parsed.
The speculative payload was empty, so this does not qualify nonempty
speculative-state implementations.

Invalid draft-version and truncated-main cases rejected processes 40808 and
29788 respectively. Neither was observed routable; both exited before the cold
replacement inferred successfully on every slot. Fresh processes subsequently
restored the untouched good generation. Idle saving retained slot 0 at 2601
tokens while advancing slot 3 to 3171, then recovered both after forced exit.
Controller shutdown/restart and a sparse generation containing only slot 3 also
passed. All six unqualified launch mutations were rejected.

The candidate manager SHA-256 was
`e74a69e92cca91563c17a534a87a3437a59db2084936d5f5610dd2f332a3c5be`
and management CLI SHA-256 was
`e1102f6d0e92feb07006a3ac834fd1f015e958aadab80d452cbbb25d7a217693`.
Later wording changes do not change the tested backend. Raw reports and logs are
retained under the isolated `2026-10-25/b11849-multislot-validation` campaign.
The Qwen run sampled host RAM at 23232.41 MiB and VRAM at 22313 MiB maximum;
346 samples succeeded and 40 sampling attempts failed. These gaps prevent a
continuous peak claim. The largest observed engine peak working set was
3363803136 bytes, distinct from host RAM and GPU allocation.

The preceding Qwen attempt 02 completed the functional cases but failed when
PowerShell process-memory sampling timed out. It remains a failed report. The
harness now records telemetry errors without interrupting stop/cleanup; it still
fails if PID ownership, exit, restore, inference or payload checks fail. A local
browser run also timed out closing Chromium after its assertions; three focused
repetitions passed. These are not counted as successful full-suite runs.

Manager Gemma run `manager-gemma31-4slot-01` passed on 2026-10-25 (local time),
02:35:10--03:28:47, with normal cleanup and no remaining owned processes.
Process 29136 exited before process 11036 restored all four slots. Each slot
retained 2602 tokens; slots 0--2 were 2729001644 bytes each and slot 3 was
2729820856 bytes. Each restored continuation reused 2602 cached tokens and
accepted 35/35 draft tokens, matching the live text, token IDs and counters.
SCKP v1 contained two checkpoints; SDFT v1 contained the 8-byte shared-KV marker
and an empty speculative payload, consistent with the native campaign above.

Invalid draft-version and truncated-main cases rejected processes 40912 and
21560, followed by cold replacements 18368 and 724. Failed processes were never
observed routable and exited; every replacement slot inferred successfully.
The good generation then restored successfully in new processes after both
faults. Idle saving kept slot 0 at 2602 tokens and advanced slot 3 to 3172;
both survived forced exit and fresh-process inference. Full controller
shutdown/restart, sparse slot-3 save/restore and all six negative contract
mutations passed. The largest observed engine peak working set was 8516448256
bytes. Host RAM/VRAM sampled maxima were 27842.05/25274 MiB, with 396 successful
samples and two failed attempts; these are not continuous peak guarantees.

Completed report SHA-256 values (before any generated payload cleanup):

- Qwen: `1347171c5e0a8e1a4298c9aa14ff57c92d6deb11f7b6fd5c56f2a78a6ff3cc98`
- Gemma 31B: `489460e5aa5e303c55d075843db11bb217c9ce80095f06719e4220195afa6b57`

The measured idle-save operation (including payload verification and quota
maintenance) took 141966 ms for Qwen and 260588 ms for Gemma 31B. The resulting
generations were 2647447048 and 10963529308 bytes respectively. Routing is held
while this work runs. These are substantial pauses, not a low-latency persistence
claim; frequent idle-save intervals can be expensive. This change does not enable
idle saving or change any deployed instance's interval. Staging/previous
generations and test-only reference copies require additional disk space.

Local source checks passed: `npm run check:release` (186 browser tests plus the
release/regression/type checks), `npm run build`, Rust tests (612 manager tests
passed, four ignored, and four CLI tests), and all-target/all-feature Clippy with
warnings denied. `npm run test:runtime-service` passed runtime failure recovery
and all 13 quota client shapes; `npm run test:management` passed 257 CLI/IPC/MCP
checks. Those integration checks use synthetic engines. The runtime fixture now
explicitly declares one slot to match its one-slot eligibility receipt; a stale
receipt with a changed launch topology is rejected in production code and tests.

## Boundaries and rollback

CPU CI verifies manager behavior and fixtures, not Windows/ROCm model inference.
The live comparison uses deterministic, slot-specific sentinel continuations;
its draft acceptance rate is not a general conversation quality or speed claim.
Host-wide sampled RAM/VRAM peaks and process peak working sets are observations,
not allocation upper bounds. Read-only-pointer and injected commit failures do
not establish physical disk-full or power-loss durability. The real controller
campaign exercises the background service path; GUI direct-control exit uses
the shared coordinator but is not an additional real GUI acceptance claim.
Clients must use manager admission/routing for the checkpoint quiescence
guarantee. External traffic sent directly to the native server can bypass it.

Rollback is configuration-preserving: stop the candidate through its controlled
path, archive its private data/checkpoint root, then restore the previous manager
binary. Before handing that root to an older reader, keep an untouched backup:
older managers reject schema 2 and may prune unsupported generations. Use a
compatible schema-1 backup or start cold; do not rewrite a schema-2 manifest as
schema 1. No engine deployment, production instance edit or release is part of
this change.
