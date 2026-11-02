# Gufo managed instances — phase 1

Manager can discover and launch a local Windows `gufo.exe` alongside llama.cpp.
The initial qualified profiles are Qwen3.8-Flash-Next with MTP and Qwen3.8-27B
with DFlash2, each using one execution session. This does not qualify other
Gufo modalities, model families, platforms, or concurrent residency.

## Configuration and isolation

Add the Gufo **runtime directory**, including its adjacent DLLs, in Engine
Management and probe it. Create an instance with that engine, then configure
the model's first GGUF shard, matching projector and speculative sidecar in
the Gufo configuration page. A model-family choice is a preflight constraint;
the engine still validates actual tensor/artifact compatibility while loading.

`engine_kind` is `llama_cpp` or `gufo`, independently of the GPU `backend`.
Missing `engine_kind` retains llama.cpp behavior. Selecting a different engine
type resets tuning while preserving the instance identity, model paths, alias
and port. Automatic engine fallback stays within the instance's engine type.
Existing llama.cpp engine IDs remain directory based; Gufo IDs identify its
executable so the two binaries can coexist in one directory.

The shared Rust launch validator is used by GUI previews, actual launches,
CLI and MCP. Gufo has a nested `gufo` configuration. It generates
`gufo serve ... llm ...` arguments directly; llama.cpp flag projection and
checkpoint qualification are not used. Unknown Gufo fields and unsupported
parameter combinations are rejected. Managed Gufo currently binds only to
`127.0.0.1`; custom/manual commands are outside this phase.

Every Gufo instance owns `<Manager data directory>/gufo/<instance id>/`:

- `snapshots/`: optional disk cache, with its own capacity limit.
- `local/`: per-child `LOCALAPPDATA` and `XDG_CACHE_HOME`, isolating artifact caches.
- `temp/`: per-child `TEMP` and `TMP`.

The process working directory is the same private instance directory. Parent
environment variables and the Gufo-GUI launcher configuration are not changed.
RAM caching distinguishes automatic, off and a fixed GiB limit. `auto` emits
`--cache-ram-bytes 0`; `off` emits `--cache-ram-bytes off`.

For an isolated trial, launch the candidate Manager with
`--management-data-dir <absolute-new-directory>`, or use `lsm --app <candidate>
--data-dir <absolute-new-directory> service start`. The MCP client remains
connect-only. Use separate test ports and stop one large model before starting
the other.

## Lifecycle and capability boundaries

The existing runtime owns the process, bounded logs and stop operation. Gufo
readiness requires `/ready` to report both `status=ready` and the expected model
alias. A live process remains loading until that probe succeeds or the configured
startup grace expires (default 600 seconds; accepted range 60–3600). llama.cpp
retains its existing probe and 120-second grace. A timeout reports failure;
it does not repeatedly restart the model.

The runtime advertises `gufo_engine_v1`; a client requiring this capability
cannot treat an older daemon as compatible. Persisted Gufo launch arguments
are reconstructed and checked against their configuration before replay.

On Windows, engines use a private hidden console. Stop sends Ctrl+C, waits up
to the existing 30-second grace, and falls back to terminating the owned process
tree. Gufo can drain accepted disk writes during graceful shutdown. This phase
does not promise recovery of pending data after forced termination, nor complete
cache persistence/restart qualification.

Gufo instances are excluded from the unified API router in phase 1. Their local
API can be used directly after readiness. llama.cpp checkpoint actions are
ineligible for Gufo. Exact input-token APIs exist in the qualified Gufo baseline,
but proxy forwarding, hard quotas, full monitoring semantics and extended
lifecycle/cache acceptance remain phase 2/3 work.

## Reproducible validation

Run the ordinary repository gates, including `npm run check:release`,
`npm run build`, Cargo tests and Clippy, and the runtime/management suites.
`scripts/test-gufo-engine.cjs` covers legacy/default engine selection and
cross-engine configuration isolation; the Gufo Rust tests exercise invalid
shards/sidecars, command construction and bounded startup policy.

The real-engine runner is opt-in and requires a **new**, absolute output root:

```powershell
node scripts/test-gufo-real.cjs --root C:/Temp/lsm-gufo-32k --launcher C:/path/to/Gufo/launcher.json --context 32768
node scripts/test-gufo-real.cjs --root C:/Temp/lsm-gufo-262k --launcher C:/path/to/Gufo/launcher.json --context 262144
node scripts/test-gufo-real.cjs --root C:/Temp/lsm-llama-regression --launcher C:/path/to/Gufo/launcher.json --only-llama true --llama-engine C:/path/to/llama-server.exe
```

It reads the existing launcher without importing or changing it, constructs new
Manager configurations, launches via CLI, observes health/logs and stops via MCP.
It records raw responses, effective `/props`, cache isolation, protected-file
hashes, original cache metadata and remaining owned PIDs. The llama.cpp control
deliberately omits `engine_kind` to exercise legacy defaults. Use short roots
outside watched source trees if Windows rejects atomic JSON replacement; retain
any failed attempt's report instead of counting it as a pass.

The November 2, 2026 local acceptance used Gufo 0.28.1 revision `408a04357827`
(the deployed Windows compiler-flag fix), both existing model/projector profiles,
32K and 262K configured contexts. All four short inference runs returned the
expected answer with actual speculative draft proposals. One Flash startup and
request cycle took about 263 seconds and remained loading past 120 seconds.
Both models stopped and released their ports; accepted cache writes were
published during graceful exit. A llama.cpp b12032/Qwen3.8-27B control also passed.

These are short text-inference and management-lifecycle results. A 262K context
allocation is **not** a complete 262K input test. Vision, long conversations,
proxy parity, crash recovery and simultaneous model residency were not qualified
by these runs. The original programs, launcher/Manager configurations and Gufo
cache inventory remained unchanged.
