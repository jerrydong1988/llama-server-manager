# b12469 Qwen3.6-35B-A3B embedded MTP checkpoint admission

This registry entry identifies a native persistence **candidate**, not a portable
restore certificate. Automatic checkpoint use still requires an opt-in local
validation receipt for the exact runtime, model, configuration, GPU/driver and
environment. No installed executable or user configuration is changed by this PR.

## Bound identities

- llama.cpp b12469, reported build commit `e0a9118fe9ed9229b715f4bd83fd9767963659c6`.
- Local Windows HIP/ROCm `10.3.0a20261117` bundle; this does not qualify an official
  ROCm 10.0 ZIP or another rebuild with the same version string.
- Launcher SHA-256: `486d6f63dee4e14306cbba5f3096469bfa9faa16fda37e726df0e12ba7d5e0ed`.
- Launcher + 28 adjacent DLLs aggregate:
  `e46ab343dbb9e3ce4f8ace7f3f74fbe1b0e32c0154a8cf2c8bbcb66aa4ed9c4a`.
- Complete runtime aggregate, including 272 ROCm auxiliary files:
  `f19c9d690e892321cd791825a1a8fedaa572c4e6dad9a1a1436436bb26677f19`.
- `Qwen3.6-35B-A3B-UD-Q8_K_XL.gguf`, architecture `qwen35moe`, one embedded nextn
  layer; SHA-256 `6c6b816537abad90b250a0972b345466028d861ddfe316d5f0de31ca6440f781`.
- `draft-mtp` only; no separate draft file, no multimodal projector or LoRA.

The complete bundle digest establishes SCKP/SDFT capability even when `--help`
does not contain the historical context-checkpoint sentence. A version string,
launcher hash, adjacent-DLL-only digest or renamed executable does not suffice.
Managed n-gram persistence is not enabled by this profile.
Native serialization capability is also kept separate from reviewed hybrid
admission: recognizing this bundle cannot admit other Qwen3.5/Qwen3.6 models.

## Candidate matrix

| Slots | Total context | Context budget per slot | Main/draft K and V |
| --- | ---: | ---: | --- |
| 1 | 8192 | 8192 | f16 or q8_0 |
| 2 | 16384 | 8192 | f16 or q8_0 |
| 4 | 32768 | 8192 | f16 only |

Four-slot q8_0 is deliberately excluded from both reviewed admission and local
validation for this bundle. Earlier native testing reproduced a paired concurrent
continuation mismatch in two fresh processes. This change does not diagnose or
fix that native behavior. Two registry scopes prevent the accidental Cartesian
product of `[1,2,4]` slots and `[f16,q8_0]` cache types.

The bounded launch uses full GPU offload, FA on, fit off, unified KV, batch/ubatch
512, draft maximum 3, two context checkpoints and minimum checkpoint step 512.
Quantization must match across main/draft K and V. Changes to these settings,
external draft weights or mixed speculative strategies are not admitted by this
entry. The actual configuration/environment is additionally bound to the receipt;
the local test environment uses `GGML_CUDA_NO_PINNED=1` and `LLAMA_TRACE=1`.

## Validation contract

The validator populates every slot before saving any slot, saves a quiescent
complete generation, and takes matched concurrent live references. It fully exits
the old process, starts a distinct process, restores all slots before inference,
and checks text/token equality, prefix reuse, draft counters and acceptance rate.
The existing tolerance remains at most one draft batch of counter drift and five
percentage points of acceptance loss; output equality is not relaxed.

The numeric rewrite control queries a uniquely labelled `FINAL RECORD`.
The earlier "LAST record" wording returned the dominant old code in the two-slot
cold control before any save/restore; an explicit index helped f16 but still
failed q8_0. Separate full-recompute diagnostics verified the labelled record
with both cache types. The long common prefix and modified tail remain, and the
code is input record data, not an answer prefix or a forced grammar literal.
The grammar still permits every three-digit answer.

The unconstrained 64-token concurrent replay remains the equality oracle. A
separate short retrieval query must return the correct slot secret, with a
grammar permitting **all four** slot secrets. This avoids mistaking a reasoning
response truncated before its answer for a restore failure, while strengthening
the old substring check into an exact answer check. It does not grammar-constrain
the replay used to compare tokens or draft acceptance.

The malformed-file check discards the failed process and repeats restoration and
inference in another clean process. Hybrid validation requires persisted context
checkpoints as well as the live main/draft state. Grammar-controlled queries allow
EOS. The validation launcher shares the identity-preserving Windows path handling
used by normal engine launch.

Receipt schema 4 requires this stronger sequence. Previous local receipts must be
revalidated; existing payloads are not rewritten or deleted. Capability and
persisted admission revisions also advance, preventing stale approvals from being
reused after the policy change.

## Reproduction and evidence

`scripts/test-b12469-checkpoint-real.cjs` takes explicit `--bin`, `--root`,
`--engine` and `--model` paths. The root must not exist. It exercises the actual
manager capability probe and local validator before automatic lifecycle tests.
No production data root or system login registration is used.

The foreground-controlled path in current Manager delegates normal engine
ownership to the runtime service. The separate background campaign runs that
runtime without a management host, using the existing debug-only isolated login
registration fixture. This is not a claim of testing Windows login itself or a
separate legacy GUI-owned engine path.

## Completed lifecycle evidence

On Windows, Radeon 8060S (`gfx1151`), driver `32.0.32015.7029`, each of the five
combinations completed all 13 lifecycle stages and seven negative admission
checks. These are real HIP inference results, separate from cross-platform CI.

| Slots / KV | Lifecycle report | Sampled host RAM MB | Sampled host GPU MB | Samples / read gaps |
| --- | --- | ---: | ---: | ---: |
| 1 / f16 | `campaign-v3` | 18668.8 | 39550 | 120 / 24 |
| 1 / q8_0 | `campaign-resume` | 17688.0 | 39429 | 118 / 22 |
| 2 / f16 | `campaign-final` | 18022.2 | 39909 | 128 / 25 |
| 2 / q8_0 | `campaign-reviewed` | 18114.4 | 40023 | 118 / 24 |
| 4 / f16 | `campaign-reviewed` | 18149.2 | 41051 | 148 / 31 |

Each completed case covers:

- Manager qualification after the old process exits, then automatic stop/save
  and restart/restore with concurrent text/token equality and active draft reuse.
- Denied latest-pointer replacement: the failed save leaves the previous pointer,
  manifest and slot hashes intact. This is a specific Windows read-only-file
  fault, not proof for every disk or filesystem failure.
- A malformed final SDFT appendix with valid outer hashes: the partially restored
  process is never routable, is discarded, and a cold replacement performs real
  inference. The original good generation then restores in a fresh process.
- Forced termination of an owned engine preserves its **already published**
  generation, which is restored in a fresh process. This does not claim that
  forced termination saves any new state or executes destructors.
- Management-host shutdown saves, standalone background runtime restores and
  infers, background shutdown saves again, and a new runtime restores and infers.

Inspected files contain GGSQ v4, SCKP v1 and SDFT v1, nonempty main and draft live
state, an 8192-byte live speculative blob, and two checkpoints with nonempty
target, draft and speculative data. This establishes the recorded embedded-MTP
case; opaque contents are not generalized to other speculative implementations.

The memory figures are host-wide sampled observations with two-second sampling.
Read gaps include periods when the management host was intentionally stopped.
They are neither complete process peaks nor allocator upper bounds, and the RAM
and GPU counters must not be added on this shared-memory GPU.

### Provenance and retained failures

The original private reports, validator logs, receipt/format records, frozen
harnesses and `acceptance-results.json` remain in the task evidence directory
`b12469-manager-admission`. Completed cases in a campaign remain separately
identifiable even when a later case failed. No failed attempt was relabelled as
a pass.

| Lifecycle report | Report SHA-256 |
| --- | --- |
| `campaign-v3` | `6ca57dfe5aa77ac5a8bad5478e63544b62cfe487a8b2a09ac9bc76ee7076d89e` |
| `campaign-resume` | `401ebf3622070c0f537dfc91c95c0d9700320a33ecc057719747e5a01eac2526` |
| `campaign-final` | `e75fa3af65372885b03da2b81021defa561e78668b1ff0ee2f994011bf4d2bda` |
| `campaign-reviewed` | `3f08c0a90f701e98f8e1caa2989e1d5c1140b6fe941afca72376e64666eadc78` |

The first two reports use Manager SHA-256
`5a27c2b58fd94774128c97d842da77ce5382e79bfe3d77bd86847426984cb51b`;
`campaign-final` uses
`2ad94466d8f02f6d10930b032f15d22e795a98fe71e7f6e37ea1304b8156f493`;
`campaign-reviewed` uses
`b0dd463ce93f64b3b5c8de5d1c936194325be6c0f6f2cf811050cad24d39cfed`.
These are isolated debug builds. SHA dependency optimization was supplied only
as build-command configuration; Cargo profiles in the repository were unchanged.

An earlier single-slot q8_0 attempt timed out during model loading before KV
allocation/inference. A final-candidate two-slot q8_0 run matched its normal
cross-process outputs but timed out loading the third process, before the
malformed-state restore check. That run generated no qualification receipt.
The cause remains unknown; later fresh-process passes do not diagnose or fix
this intermittent loading failure. Cold retrieval-control failures and truncated
reasoning are retained with the fixture diagnostics described above. An earlier
failure-injection harness changed auto-save after launch and received `skipped`;
the completed campaigns enable it before launch and require an actual failure.

The final source candidate, Manager SHA-256
`0796e37ce9a97c6e77bf72c57519254f2eadb108682de76f28dcceddd9adb7b9`,
requalified all five pairs. The first three completed in `campaign-pr-validation`
(report SHA-256 `b5fbcb1d96f23106636ffd5cc410e2010752b305ea700340169acc1f0ba7a74e`);
four-slot f16 and the independent two-slot q8_0 rerun completed in `campaign-pr-tail`
(`33b618331bac16ddf7458910df8888d96ccc125c433205d35ae646d072ecb37c`).
The first report retains its later loading failure; it is not an overall pass.
The final receipts verify strict concurrent replay, exact slot secrets and final
record answers, cache reuse, positive draft acceptance, exited/distinct processes
and 35 negative admission checks. Lifecycle code and the admitted local launch
path remain unchanged by the final restriction on unrelated hybrid models.

The private source manifest SHA-256 is
`28747efcc06270a96fec07251ad6c52fa67436e8ad2c02c6dc6bf39a74b738b6`;
the combined acceptance summary SHA-256 is
`3410b4f031628a44d4175799ddbb46df600f8a2c98bfacc4e4f1ee2578239f6d`.
All owned test processes exited. Post-run hashes of 379 protected deployment,
runtime and configuration files matched the resumed baseline, and the original
desktop/runtime PIDs remained unchanged.

Build success, health, HTTP 200 and same-process restore are not acceptance
evidence. This campaign does not qualify four-slot q8_0, a different model/build,
larger contexts, external drafts, short prefixes without a checkpoint, bitwise
logits, every multi-slot scheduling pattern, memory-pressure upper bounds,
disk-full/close/ACL failures, or native standalone overwrite safety. Those remain
outside this bounded admission evidence. Existing immutable generations,
fingerprints, local revalidation and failed-restore process replacement remain
required. No production binary, configuration, running instance, release or tag
is changed.
