# Fixed b11769 cache lifecycle

This integration is opt-in and does not change an existing engine selection or
instance configuration. It does not modify llama.cpp. A release/version number,
`--cache-disk`, or a successful HTTP response is not sufficient evidence of full
draft restoration.

## Capability evidence and limits

The Windows profile `windows-rocm-b11769-sckp1-sdft1` identifies a tested local
ROCm build from llama.cpp commit `4e201388c319fc9f747fbe6a666793e2dc2a6d8e`:

- launcher SHA-256: `5c081b5db3fc7d62ba11f7b79253df7edf39975835fa8e9da5932301723410dd`
- launcher and 28 adjacent runtime libraries, using the existing checkpoint
  artifact-set hashing algorithm:
  `51721bd434546a540548be496a58c9878774d41f7bc594b245031f38a1e0dfeb`

Its help text predates its actual SCKP/SDFT implementation. The profile corrects
that false negative without granting support to other builds named b11769.
Changing a runtime library invalidates the evidence. Old capability records and
persisted eligibility decisions must be refreshed; an old daemon cannot serve
the new cache lifecycle protocol.
The test keeps the original release-root `.kpack`, `hipblaslt`, and `rocblas`
payloads alongside `bin`. The executable/DLL identity is not a replacement for
those GPU runtime assets or for real inference on the target machine.

SCKP support and qualified draft modes are separate. The profile permits MTP,
DFlash, and same-model simple draft within the tested configuration envelope:
explicit context at most 4096, positive context-checkpoint count, full target/draft GPU offload, FA on, fit off,
f16 target/draft caches, and the existing single-slot/local/managed eligibility
checks. Quantized draft cache, EAGLE3, other simple draft pairs and larger/auto
contexts remain ineligible. Unknown future engines with a SCKP help marker do
not automatically qualify all advertised draft modes.

Hardware/model evidence is Qwen3.8-27B (qwen35 hybrid/recurrent), its MTP and
DFlash2 sidecars, on Radeon 8060S with the fixed HIP build. This is not a claim
that every model, AMD GPU, MoE partial-offload layout, or nonempty speculative
payload has been validated. The exercised SDFT speculative payload is empty;
EAGLE3 is excluded. Manager retains restore/resave byte verification and fresh
process recovery on failed restore. It never equates HTTP 200 with complete
state restoration.

## Windows shutdown

Both direct management and runtime service use the same native process adapter:

1. Create a private console hidden from its first frame, with only the intended
   stdin/stdout/stderr handles inherited. Do not inherit the background
   launcher's ignore-Ctrl+C flag.
   Short verbatim drive paths are converted to ordinary paths only after checking
   that they canonicalize to the same file. This avoids the fixed ROCm package's
   invalid mixed-separator Tensile library path on DFlash inference. Long/device
   paths remain unchanged and are not covered by that workaround.
2. Quiesce manager routing and drain the instance. Existing KV checkpoints save
   through unique scratch filenames before shutdown.
3. A short-lived helper checks PID, creation time, executable and console
   membership, attaches only to that console and sends Ctrl+C. Concurrent stop
   paths serialize the signal, avoiding llama.cpp's second-interrupt abort.
4. Wait up to 30 seconds for native shutdown. Only the child monitor's actual
   exit code may authorize candidate inspection. Failure/timeout falls back to
   terminating the recorded process tree and retaining the previous cache.

Processes started by older manager versions lack private-console evidence and
keep the fallback behavior. External dynamic cache arguments, including manual
and custom arguments, never gain implicit exit writes. GUI background detach
continues to keep workloads running; it is not an exit/save event. Forced
termination and power loss do not become graceful saves.

## Managed dynamic n-gram cache

`lookup_cache_managed` defaults to false. With ngram-cache selected, enabling it
uses the configured dynamic path only as a read-only seed (which may be absent).
It requires a structured managed launch without custom overrides, so the model
identity cannot diverge from the effective command. The setting is dormant when
ngram-cache is not selected.
It does not rewrite that instance field or the external file. A legacy seed has
no model identity; users must select a seed from the same tokenizer. Structural
validation does not establish its provenance.

Manager-owned generations live under
`<data-dir>/lookup-cache/<instance>/<model-and-engine-and-seed-identity>/`.
The model digest includes tokenizer data. Each namespace has an exclusive
cross-process lock, a unique writable working copy, immutable generation files,
and `latest.json` / `previous.json`. Publication validates bounded legacy records,
size and SHA-256, syncs a new generation, preserves the old pointer, then atomically
replaces the latest pointer. A corrupt newest generation can fall back to the
previous valid one. Empty, malformed, oversized, forced-exit and failed-publication
candidates do not replace the old generation. The validator caps payloads at
256 MiB and streams records instead of allocating from untrusted counts.

Normal exit plus a structurally valid changed candidate is reported as
`candidate_committed_flush_unverified`. An unchanged file is
`unchanged_flush_unverified`; neither is a verified complete flush. b11769 can
silently ignore write/close errors, and truncation at a complete-record boundary
is indistinguishable from a smaller valid legacy cache. `full_flush_verified`
therefore remains false. The engine must gain an explicit, checked/atomic writer
before stronger guarantees are justified. Native writes outside manager-owned
copies remain outside these protections. n-gram data is not target/draft KV,
recurrent or speculative checkpoint state.

`instances.stop` returns the recorded lookup-cache outcome. Namespace
`last-result.json` and runtime diagnostics retain it even after service shutdown.
Obsolete model/engine namespaces are retained for explicit rollback; the per-file
bound is not a global disk quota or a native RAM/VRAM allocation bound.

## Validation and reproduction

Unit/regression coverage includes capability evidence and invalidation, draft
scope rejection, routing drain with KV disabled, exclusive writer ownership,
truncated/huge-count/empty records, failed publication preserving the old pointer
and bytes, previous-generation recovery, and identity-checked private-console
signalling that leaves another process alive.

The opt-in script `scripts/test-manager-cache-lifecycle-real.cjs` requires a NEW
absolute `--root`, `--profile windows-rocm-b11769-sckp1-sdft1` and fixed `--engine`, plus `--bin`, `--model`, `--mtp`, and
optional `--dflash`. It creates its own management data, loopback instance,
processes and caches. It never targets the production data directory.
An optional `--fingerprints` imports only a prior artifact fingerprint cache;
the store still checks the current artifact metadata. It does not import engine
state, instance configuration, or KV/n-gram payloads. The real
matrix covers normal stop/restart, denied native writes, forced exit, service
shutdown, and fresh-PID restore with real generation for MTP/DFlash/same-model
simple draft. With context checkpoints disabled, it verifies that the existing
automatic-policy gate remains closed, then tests MTP/DFlash through explicit
native slot save/restore across manager-launched processes. Those two boundary
cases are engine evidence, not automatic-checkpoint support. `--boundary-only true`
runs just that boundary and the configuration-rejection checks. The suite compares
tokens, output, accepted/generated draft counts and cache reuse, and rejects
unqualified draft configurations. Reports and owned logs are retained separately
from large disposable cache payloads.

The private-console test is opt-in (`cargo test ...
private_console_signal_checks_identity_and_preserves_other_process -- --ignored`)
and requires `LSM_TEST_HELPER_EXE` pointing to the built manager and
`LSM_TEST_PYTHON` pointing to Python. It launches only disposable signal fixtures.

Observed on 2026-10-21 with the artifact identities above:

| Scenario | Observed result |
| --- | --- |
| Managed n-gram normal stop and restart | Clean native exit; private candidate published; external seed path untouched |
| Native write to read-only working copy | Previous generation pointer unchanged; reported unverified flush |
| Forced engine termination | Previous generation retained; abnormal exit recorded |
| Background management service shutdown | Clean exit published a private candidate with flush unverified; owned engine and controllers exited without respawning |
| Manager automatic restore, MTP / DFlash2 / same-model simple draft, two context checkpoints | Three fresh-process restores followed by real HIP inference; tokens and text matched live continuation; cache reuse 65 tokens and draft acceptance 35/35 in each case |
| Zero context checkpoints, MTP / DFlash2 | Automatic policy correctly rejected both; two explicit native API fresh-process restores matched live continuation with cache reuse 65 and draft acceptance 35/35 |
| Unqualified q8 draft cache / 8192 context / EAGLE3 | Automatic eligibility rejected |

The primary campaign completed the four shutdown/cache cases and three automatic
restore cases, then stopped on a harness assertion that incorrectly expected
automatic zero-checkpoint eligibility. After correcting that expectation, the
boundary campaign passed both native API cases and the rejection checks. The
original failed report is retained; it is not presented as one passing full run.
Both boundary files contain SDFT v1 with nonempty draft bytes and zero speculative
bytes. The inference workload is a deterministic short continuation, not a broad
quality or acceptance-rate benchmark. Post-inference RAM/VRAM snapshots were
collected; they are not peak measurements or allocation upper bounds.

Local checks passed: release checks (including 184 browser tests), production
frontend build, 594 Rust tests (three explicitly ignored fixtures), clippy with
warnings denied for project code, the opt-in private-console isolation fixture,
runtime-service recovery regression, and 265 CLI/IPC/MCP integration checks.
The synthetic service fixtures are separate from the real HIP evidence above.

Native disk-full/delayed-close failure, every ACL/concurrent-writer combination,
forced power loss, actual legacy GUI exit, nonempty speculative payload recovery,
and exhaustive GPU allocation upper bounds remain separate validation
requirements. Building or health alone is not GPU recovery evidence.

## Rollback

1. Stop only the trial instance through its matching manager; wait for its PID
   and task completion. Record any forced-exit or unverified-flush result.
2. Preserve both generation pointers/payloads and the trial report. Never copy a
   candidate over the original seed or over an unrelated instance's cache.
3. Disable managed dynamic caching / automatic checkpoints in the trial config,
   or restore its prior config snapshot. A cold start is the default rollback;
   do not feed new state formats to an older engine.
4. Revert the manager change through a scoped PR if necessary. Keep the old
   engine and its instance configuration unchanged. The protocol check requires
   stopping/updating the matching daemon before a version rollback; do not force
   a live daemon replacement.
5. Clean only the sentinel-marked test root after its owned processes have exited.
   Production paths, model files, external seeds and existing engines are not
   rollback cleanup targets.
