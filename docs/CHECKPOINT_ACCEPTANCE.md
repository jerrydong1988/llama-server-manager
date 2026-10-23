# Checkpoint reliability contract and acceptance

## Contract

`src-tauri/src/commands/persistence_profiles.json` is the reviewed build registry.
It is compiled into the manager; it is not an instance setting or a user-editable
capability override. Qualification matches the launcher and adjacent runtime DLL
content digest, not a directory name, version string, help text or release label.
The probe cache revision changes when this contract changes. A cached profile ID
and its draft modes must match the same registry entry at launch.

| Provider | Evidence and boundary |
| --- | --- |
| Slot KV / prefill checkpoint | Slot API save, verified size/hash, manifest-last generation, fresh-PID restore and round-trip validation. Reuse and output correctness require a real request. |
| Dynamic n-gram | Only the private working copy is passed to the engine. Clean exit permits bounded structural validation and candidate publication. Native write/close failures are not reliably acknowledged; `full_flush_verified` remains false. |

`latest_generation_valid` and `previous_generation_valid` report validated local
recovery files after dynamic-cache finalization. They do not certify a complete
native flush. A successful exit may still leave an incomplete but structurally
valid candidate; the previous generation is retained. No manager change can
turn the native format into a transactional writer or resume a mid-generation
random-number stream that the engine does not serialize.

Qualified draft configuration remains a single slot, full GPU offload, FA on,
fit off, f16 KV, explicit context up to 4096, no RPC/device overrides or MoE CPU
placement, and positive context checkpoints where required. Simple draft is
limited to the same target/draft model. Qwen35 hybrid evidence does not qualify
other hybrid architectures or GPUs. Registry entries establish the tested
configuration envelope; they do not certify every model within that envelope.

## Reproducible isolated acceptance

Run `scripts/test-manager-cache-lifecycle-real.cjs` with absolute `--bin`,
`--engine`, `--model`, `--mtp`, optional `--dflash`, a NEW `--root`, and
`--profile windows-rocm-b11838-sckp1-sdft1`. Use `--port` for a free dedicated
loopback port. The script checks the launcher hash and the manager's full bundle
qualification, inventories kernel/runtime asset hashes, creates separate manager
data and instance configuration, and records owned PIDs and their exit before
restart. It never imports production settings. `--fingerprints` may seed only
the metadata-validated hash cache from an earlier isolated run.

| Matrix | Required assertion |
| --- | --- |
| Graceful Windows stop / restart | Private console, real HIP inference, normal exit, candidate status, new PID |
| Read-only native working file | Old committed pointer unchanged, no verified-flush claim |
| Forced process termination | Old committed generation unchanged |
| Background service shutdown | Engine and service actually exit; no respawn |
| MTP / DFlash2 / same-model simple | Save, fully exit, fresh process, restore, real generation, tokens/text and draft counts compared with live baseline, nonzero cache reuse |
| No context checkpoints | Automatic qualification denied; separate explicit native API experiment, never counted as automatic support |
| Quantized draft KV / oversized context / EAGLE3 | Unqualified configuration denied |
| Corrupt latest / valid previous | Fallback remains valid even after a new publication |

Real GPU reports are separate from Rust fault-injection and browser/CLI CI.
Disk-full and close-error injection does not mean the real disk was filled.
Memory snapshots are not peak RAM/VRAM bounds. A zero-length serialized
speculative payload cannot prove restoration of a nonempty speculative state.
GUI detach is not shutdown. The direct GUI path and daemon path share private
console handling but require separate lifecycle evidence.

## Idle saving and long-prefix retention

`kv_checkpoint.idle_save_seconds` defaults to 0 (off); enabled values normalize
to 5–86,400 seconds. It requires checkpoint eligibility and `auto_save`. The
direct GUI owner and daemon share the same implementation. Slot activity and
completed inference logs mark the state dirty, then a quiet period triggers a
save. Unchanged idle state is not repeatedly written. Attempts, including
failures, are limited to one per minute per process. This bounds write frequency,
not payload size or RAM/VRAM allocation inside the native engine. Missing both
slot and log evidence can still miss a request; stop-time saving remains available.

Saving closes proxy admission, rechecks active requests and slot state, commits
through the existing verified store, then restores Ready or ReadyCold without
inventing a restore event. It shares a PID lock with stop/termination. A stop
waits for an in-progress save; the native slot operation retains its bounded
30-minute timeout. There is no native cancellation guarantee. Failed saves retain
old committed data and resume routing. Clients bypassing the manager proxy are
outside admission guarantees.

`kv_checkpoint.preserve_longer` defaults to false. When enabled, a shorter slot
save does not replace the latest valid compatible checkpoint. The candidate is
discarded and status reports `longer_checkpoint_retained`. The threshold is based
on actual saved tokens, not telemetry counters that may reset at idle. This
protects a saved main prefix from a later short title; it neither pins RAM entries
nor selects a matching prefix among arbitrary historical conversations. Disable
the policy to deliberately replace it, or clear checkpoints while stopped.

Use `--idle-only true` with the real acceptance script to exercise both MTP and
target-only inference: long-prefix inference, idle save without restarting, live continuation, short title,
controlled stop retaining the longer generation, fresh-PID restore and matching
inference with more than 1000 reused tokens, and recovery of that generation after
a forced process exit. This is separate from the full lifecycle/draft matrix.

## Recorded b11838 acceptance (2026-10-24, local time)

Fixed engine: build 11838, commit `4231b23dc`, reviewed launcher/DLL digests in
the registry. Hardware telemetry identified AMD Radeon 8060S. Models were
Qwen3.8-27B UD-Q4_K_M, its Q4_0 MTP head, and Qwen3.8-27B DFlash2 Q4_K_M;
simple draft used the target model itself. All runs used separate data roots,
loopback ports and identity-checked processes. Production settings were not used.

| Campaign | Observed result |
| --- | --- |
| `b11838-contract-01` | Full lifecycle matrix passed. Automatic fresh-process MTP, DFlash2 and simple draft each reused 65 tokens, evaluated 1 prompt token, matched live output/token IDs and accepted 35/35 draft tokens for this deterministic fixture. Both zero-checkpoint cases remained blocked by automatic policy. |
| `b11838-idle-01` | MTP and target-only idle saves each committed 1,703 tokens without restarting. A later short title did not change the committed pointer/generation. Fresh-process restore and a further forced-exit recovery each reused 1,703 tokens, evaluated 1 prompt token and matched the live token sequence. MTP accepted 35/35 draft tokens. |

The MTP idle generation was 589,355,560 bytes and its measured manager save
duration was 6,764 ms. These are observations, not latency or allocation bounds.
The arithmetic fixture's 100% draft acceptance is not a general quality/performance
benchmark. Both reports ended with `passed: true` and successful owned-service
cleanup; an independent audit found neither test port listening nor owned processes
remaining. Raw reports/logs are retained locally under these campaign IDs.
Current GUI instance commands delegate to the runtime supervisor. The legacy
direct-owner integration shares the save helper but was not separately driven
through native GUI controls in this campaign.

Local automated validation: 595 manager Rust tests plus 4 CLI tests passed
(3 opt-in tests ignored in the ordinary suite), strict clippy passed, 185 browser
tests and the release checks passed, and the production frontend build passed.
The separate opt-in Windows private-console signal identity/isolation test passed.

## Rollback procedure

Revert the relevant manager PR through another PR. Do not change production
engine selection or saved instance settings for this acceptance. A registry
removal makes that bundle unqualified on the next probe/launch; it must not
reinterpret old cache bytes. Keep immutable generations for the old build until
the user explicitly clears them. Old manager builds ignore added result fields.
Stop only the isolated campaign's identity-checked PIDs, preserve its report and
logs, and remove large disposable payloads only inside its owned root.
