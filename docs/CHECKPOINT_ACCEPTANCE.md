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

## Rollback

Revert the relevant manager PR through another PR. Do not change production
engine selection or saved instance settings for this acceptance. A registry
removal makes that bundle unqualified on the next probe/launch; it must not
reinterpret old cache bytes. Keep immutable generations for the old build until
the user explicitly clears them. Old manager builds ignore added result fields.
Stop only the isolated campaign's identity-checked PIDs, preserve its report and
logs, and remove large disposable payloads only inside its owned root.
