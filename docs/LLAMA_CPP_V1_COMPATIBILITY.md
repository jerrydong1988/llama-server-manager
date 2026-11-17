# llama.cpp v1.0.0 compatibility review

Reviewed on 2026-11-15 (Asia/Shanghai). This is a source and parameter-contract adaptation, not a GPU inference qualification or an installed-engine upgrade.

## Pinned evidence

- Previous stable: [v0.9.0, cd959bae](https://github.com/ggml-org/llama.cpp/tree/cd959bae99ff4bed22bef2604f45241c09bbb8c8).
- Current stable: [v1.0.0, b1b770d](https://github.com/ggml-org/llama.cpp/tree/b1b770d90d94cad9e9ea5b993bdbc383a522fb28).
- [Issue #4](https://github.com/jerrydong1988/llama-server-manager/issues/4) reports 261 → 266 parameter rows: six additions, one removal, seven description/default changes, and one hybrid-classifier addition. Its report was generated on November 14 UTC.
- Parameter definitions: [server README](https://github.com/ggml-org/llama.cpp/blob/b1b770d90d94cad9e9ea5b993bdbc383a522fb28/tools/server/README.md) and [argument parser](https://github.com/ggml-org/llama.cpp/blob/b1b770d90d94cad9e9ea5b993bdbc383a522fb28/common/arg.cpp).

## Impact and adaptation

| Change | Impact | Manager handling |
| --- | --- | --- |
| `--moe-cache-cpu-share` | CPU computes a fraction of full-cache MoE misses; range 0–1, default 0. No general speed improvement is implied. | Reviewed custom argument, checked against the selected engine; no default emitted. |
| `--tts-lang`, `--tts-voice` | Previously TTS-tool-only options now also apply to llama-server. Audio language and voice depend on the speech model. `FNAME` in the language synopsis is not evidence of a filesystem argument; the parser stores the language string. | Reviewed custom arguments. No new audio UI or inference support claim. |
| `--ple`, alias `--shard` | Additional GGUF files can be on other drives and repeated. Existing managed artifact authorization and checkpoint identity do not include these extra files. | Manual commands only, with both aliases blocked in managed custom arguments. Manual launches remain ineligible for managed checkpoint reuse. |
| `--allow-unauthenticated-tools` | Upstream opt-out for tools/MCP/proxy on non-loopback listeners without a key. Default disabled. | Never emitted automatically; manual command only. The manager's existing non-loopback API-key requirement still applies, so this does not bypass manager authentication policy. |
| `--media-allow-private-urls` | Upstream now rejects private HTTP(S) media destinations unless explicitly opted in. LAN image/audio URLs may need a different workflow. | Manual command only; no automatic relaxation. This flag does not lift the Web UI MCP proxy's private-address restriction. |
| Removed `--defrag-thold`, `-dt` | Already deprecated in v0.9.0. Existing custom commands may fail against v1.0.0. | No structured field emitted these flags. Detected v1.0.0 engines reject them; old engines may still accept them. Saved custom commands are not silently rewritten. Manual commands remain the user's responsibility. |
| `--cache-type-k`, `--cache-type-v`, `--spec-draft-type-k`, `--spec-draft-type-v` | Add `f8_e4m3`; default remains `f16`. | Add FP8 to all four selectors. Parse the value list separately for each option and its aliases. FP8 requires a complete probe that explicitly advertises it; old or missing enum metadata cannot authorize it. Legacy types retain compatibility when old help omits enums. Backend validation also covers CLI/MCP requests. |
| `--reasoning-preserve` | The v0.9.0 README said template default, but its argument parser already inserted preserve_reasoning=true. v1.0.0 corrects the README; this is not a new runtime default change between these tags. Template capability is still required. | Preserve the existing inherit/on/off distinction. Inherit emits neither switch; explicit choices stay explicit. Update bilingual guidance. |
| `--cors-origins localhost` | Changes from reflecting any localhost-origin port to same-origin checks. Cross-port browser frontends may be denied. | Explain the change and explicit-origin configuration. No CORS default is forced by this adaptation. |
| `--ui-mcp-proxy` | Only public MCP destinations; local/private addresses are refused. Enabling it now also selects the restricted default CORS origin unless explicitly overridden. | Update guidance. This is llama.cpp's Web UI proxy, separate from this manager's connect-only stdio MCP. |
| Hybrid `kimi-k31` | Added to the upstream hybrid classifier. | Existing normalized `kimik3` coverage already rejects unqualified Kimi K3.1 checkpoint reuse. Add explicit spelling/eligibility regressions. No new persistence admission or eligibility-revision bump is justified. |

The reasoning finding is verified against [v0.9.0 argument parsing](https://github.com/ggml-org/llama.cpp/blob/cd959bae99ff4bed22bef2604f45241c09bbb8c8/common/arg.cpp#L948), which already supplies `preserve_reasoning=true` when omitted. The CORS change is present in [server-http.cpp](https://github.com/ggml-org/llama.cpp/blob/b1b770d90d94cad9e9ea5b993bdbc383a522fb28/tools/server/server-http.cpp), not only in help text. Media URL handling receives the new opt-in in [server-context.cpp](https://github.com/ggml-org/llama.cpp/blob/b1b770d90d94cad9e9ea5b993bdbc383a522fb28/tools/server/server-context.cpp). FP8 is classified as a floating-point type rather than a block-quantized type in [ggml.c](https://github.com/ggml-org/llama.cpp/blob/b1b770d90d94cad9e9ea5b993bdbc383a522fb28/ggml/src/ggml.c), so existing quantized-V warnings are not extended merely from its eight-bit width.

## Validation boundaries

Regression coverage checks wrapped help enums, all ten target/draft KV aliases, old/partial/missing capability evidence, retained legacy values, removed flags, explicit new custom arguments, managed/manual boundaries, inherited reasoning switches, and Kimi K3.1 checkpoint rejection. The compatibility watcher compares immutable stable and master snapshots; master remains a canary and does not authorize features on the selected engine.

Runtime capability metadata revision 3 refreshes older GUI probes to collect KV enums. Existing metadata loads with an empty map; FP8 remains blocked until re-probed. No instance defaults, installed executable, model files, or application data are migrated by this source change.

Before claiming end-to-end support, separately qualify the selected v1.0.0 binary/backend/model for FP8 inference, MoE CPU/GPU sharing, TTS, extra GGUF shards and checkpoint recovery. Updating the parameter baseline does not provide that evidence.
