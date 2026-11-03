# b12061 checkpoint evaluation on Windows/ROCm

This evaluation does **not** add a reviewed persistence profile for b12061.
The environment-fingerprint fix is independent of qualifying that engine.
Existing engine selection and production instance configuration were preserved.

## Identity and upstream gate

The official [b12061 tag](https://github.com/ggml-org/llama.cpp/releases/tag/b12061)
resolves to d02b4c0d66796e0a44380611d038123fefb9fe83. The official comparison
from #29510's merge (1b6935b227b9f9aa89deea8016c0e208f547446f) is ahead by two
commits and behind by zero. Inclusion was checked by commit ancestry.

The actual inference target was the existing local
build-hip-rocm103a20261102-vs2026-b12061 bundle: ROCm 10.3.0a20261102,
Windows, Radeon 8060S/gfx1151, driver 32.0.32015.7029, NO_VMM=1.
Only isolated test processes used GGML_CUDA_NO_PINNED=1.

| Local identity | SHA-256 |
| --- | --- |
| Launcher | 9997dd4d5ef4ba8ecfd6bd7279239c51fca42d25480764e241ab0b9766c9d574 |
| Launcher and 28 adjacent DLLs | 21f34ddb2c0d693940c7ff3c56ae0232a34546fab9fb452fd30e12a73e5ffe83 |
| Complete runtime, including 272 auxiliary files | e522e2e4465903f1e3cfe3a719dfa586849c61a39310f887ac35bb8cfc2ba87b |
| Test manager | 1df437c3471c4ca6c1792027d5c91b6246c784fdf921d607d7dfe9e9cbdbad8f |

Loaded HIP, ggml, rocBLAS, hipBLAS, COMGR and kpack modules came from this
bundle. The full digest uses the manager's runtime-set-v2 canonicalization,
including ../.kpack labels.

The separate official ROCm 10.0 archive was downloaded and checked: 267,328,670
bytes, SHA-256 22bf8ccfffbb18af375780a1e4b3c3bf497680c14dbb495ec710f210eca617f9,
56 entries, all ZIP CRCs and required runtime-file checks passed. It was not
executed; local ROCm 10.3 inference does not qualify that package.

The b12061 [release build](https://github.com/ggml-org/llama.cpp/actions/runs/41493290475)
and Windows ROCm packaging passed. The earlier exact #29510
[HIP quality check](https://github.com/ggml-org/llama.cpp/actions/runs/41479370838)
still failed, and [#31550](https://github.com/ggml-org/llama.cpp/pull/31550) was
unmerged at the evaluation cutoff. Package builds do not clear that gate or
prove recovery. The b12061 self-hosted ROCm inference job was still queued.

## Environment identity change

LLAMA_KV_ROWS_DISABLE now participates in local qualification's
hardware/environment digest. Absent, empty, 0, and 1 are distinct.
Upstream checks presence, so 0 is not an enabled-mode control.
Windows key case and enumeration order do not change identity; unrelated
environment variables remain excluded. Receipt schema 3 rejects older experimental
receipts even if the new environment digest matches: the old receipt cannot prove
whether the formerly ignored variable was present. Reviewed profiles and saved
cache formats remain unchanged.

Rust tests cover receipt rejection, presence/value differences, key case,
ordering, legacy receipt rejection, retained existing prefixes and hardware changes. No saved-state
format or supported model/build changes.

The opt-in admission harness can restart isolated controllers with 0 and 1
and check receipt invalidation. That **real-receipt branch was not reached for
b12061** because of the capability gate below. Unit coverage is not reported
as that integration result.

## Native recovery evidence

Target: Qwen3.8-27B UD-Q4_K_M, SHA-256
322e194ff79741c7baa497c240f677f54b201b0efab44ca8e50f122b39123482.
MTP: Q4_0, SHA-256
50d9ce5a6da381bbcfb31061cf73df94a90e6faf8efeddee379a9cb8f1501c6e.

The four-slot MTP + ngram-mod campaign used a 16K shared context, f16 KV, full
GPU offload, FA on, fit off and two context checkpoints. Distinct per-slot
answers prevent a swapped slot from passing solely because both outputs match.

native-mixed-four-01 passed. Saving PID 13664 fully exited before PID 2372
restored **all four slots before any inference**. Output text and token IDs,
draft counters and cache reuse matched the live controls. Reuse was
2,347 / 2,487 / 2,487 / 2,487 tokens; each slot accepted 38/38 draft tokens.
The mixed rollback fixture accepted 61/64 and recorded speculative checkpoint
restoration; its result and subsequent unconstrained continuation matched a
separate no-draft process.

Main state was about 311–320 MB per slot; each file had two SCKP v1 entries and
a live draft payload of about 9.7–10.2 MB. GGSQ was v4 and SDFT was v1.
MTP's opaque speculative payload was empty. This does not prove nonempty
MTP internal-state persistence, arbitrary mid-step resumption or the unmerged
[#30141](https://github.com/ggml-org/llama.cpp/pull/30141) proposal.

HIP logs reported that Flash Attention over K/V rows was unsupported and disabled
for both contexts. This is actual fallback evidence, not a claim that NVIDIA's
optimized rows kernel ran or improved ROCm speed.

Malformed main state and invalid draft versions were detected. Subsequent
same-process inference checks are only stability observations. A later fully
fresh process restored the unchanged original files and inferred successfully.
RAM prompt-cache rejection was exercised; it is not a disk-full test.

native-ordinary-two-01 also passed: ordinary draft used the same target GGUF
in two independent model contexts, two slots and an 8K shared context. PID 13688
exited before PID 41496 restored both slots. Output text/tokens matched, reuse
was 2,347 / 2,487 tokens and accepted/proposed counts were 29/29 in both slots.
Live target and draft state were each about 311–320 MB, in addition to the
checkpoint payloads. The opaque speculative payload was empty. This tests
independent KV/recurrent restoration with this same-model draft, not arbitrary
different draft models or a nonempty MTP internal-state payload. The complete
malformed-restore, RAM-rejection and final fresh-process recovery campaign passed.

Observed host-wide sampled maxima were 21,289 MiB RAM / 22,027 MiB VRAM for
the mixed run and 20,364 MiB / 33,263 MiB for ordinary draft. Do not sum UMA
RAM/VRAM or treat these samples as per-process allocation bounds.

native-disabled-zero-four-01 repeated the complete four-slot mixed campaign
with LLAMA_KV_ROWS_DISABLE=0 and passed. Native logs explicitly recorded the
variable disabling KV rows. Saving PID 40636 exited before PID 48332 restored
all slots; reuse and 38/38 draft counts matched the unset run's controls.
Overwriting the test-owned, read-only good cache returned HTTP 500. Its SHA-256
and all other original cache hashes remained unchanged, and subsequent fresh
processes restored and inferred from those files. This proves the tested
read-only rejection path only: disk-full during writing, close errors,
concurrent writers and interruption of an overwrite remain unvalidated.

All owned test processes exited. The two pre-existing installed manager service
PIDs remained in place and all eight production configuration hashes matched
the before-test snapshot. No installation, engine switch or release occurred.

## Manager gate and remaining acceptance

Three initial manager campaigns were rejected before GPU inference:
contextCheckpointPersistence=false, with unsupported multi-slot, draft and
hybrid reasons. b12061's help says only "path to save slot kv cache"; it lacks
the explicit checkpoint marker used for unknown builds. The exact local bundle
also has no embedded reviewed profile.

Native API results do not override that gate. This run has not accepted b12061
manager automatic save/restore, whole-process disposal after a failed restore,
controller-exit saving or real-receipt environment invalidation.
Earlier b12000 evidence is not transferred to b12061.

After the relevant upstream gate is resolved, the remaining work is an exact
artifact capability proposal and full isolated manager lifecycle campaign, then
all six required PR checks and post-merge verification. Do not weaken the global
help detector or infer support from a version number.

## Evidence and validation

Evidence is under C:/Users/Jerry/Documents/Codex/2026-11-03/b12061-checkpoints.
It includes official API responses, archive verification, the local bundle
manifest, loaded modules, test-manager hashes, native logs and reports.
Unsuccessful admission attempts remain recorded.

The native campaigns used the listed test-manager binary before the final
receipt-schema migration. They did not enter automatic qualification. The schema
migration is covered by the subsequent Rust receipt tests, not presented as an
additional GPU acceptance run.

Local checks passed: production build, release checks including 192 browser
tests, 662 Rust tests with four ignored, Clippy with warnings denied, seven
checkpoint-evidence tests, and the official stable compatibility gate.
Master-only compatibility drift is a canary, not stable support evidence.

RAM/VRAM values are host-wide sampled maxima, not allocation bounds; UMA
accounting can overlap. No other GPU, toolchain, model, context length,
quantization, nonempty MTP state or universal crash-safe cache guarantee follows
from these results.

