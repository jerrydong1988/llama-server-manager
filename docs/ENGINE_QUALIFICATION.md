# Engine Qualification

Engine qualification is the Phase 1 fail-closed gate between discovering a `llama-server` binary and using that binary to start an instance. It records reproducible evidence for one engine artifact and one representative generative GGUF model.

## Operator workflow

1. Add and scan an authorized engine directory and an authorized model directory.
2. Open **Engine Manager**, select the engine, and run the normal capability probe if needed.
3. In **Engine Qualification**, select a primary generative model and choose **Run qualification**.
4. Review all five checks: version, capabilities, startup, health, and inference.
5. Start instances only after the report status is **Passed**.

Old inventory rows remain readable after upgrade, but their qualification status defaults to **Unqualified**. They must be qualified once before the engine can start an instance.

## Qualification profile v1

The application re-probes the explicitly selected engine before launch. Version detection and complete `--help` capability evidence must pass first. The runtime profile then:

- executes only the selected, scanned engine and selected, scanned primary generative model;
- binds to `127.0.0.1` on a temporary free port;
- uses a 512-token context, two CPU threads, and zero GPU layers when those flags are available;
- enables `--no-ui`, `--offline`, and `--log-disable` when the engine reports support;
- waits up to 180 seconds for startup and successful `GET /health` evidence;
- sends the fixed prompt `LSM qualification probe.` to `POST /completion` and requires a successful response with generated output or predicted tokens;
- terminates the complete qualification process tree after pass, failure, timeout, or operator cancellation.

This is a controlled compatibility baseline, not a performance benchmark and not proof that every model or GPU configuration will work.

## Report and invalidation

The persisted report contains schema/profile versions, status, engine fingerprint, detected engine version, capability-help hash, representative model identity/size/modification time, timestamps, per-check duration and details, and a bounded diagnostic. The fixed probe prompt is redacted from diagnostics.

Only a complete, current-profile **Passed** report whose executable fingerprint still matches the selected engine authorizes instance startup. The accepted fingerprint/profile binding is copied into the background runtime launch snapshot, so automatic crash recovery also refuses to execute a replaced or legacy-unbound engine. A rescan or start-time check marks previous evidence **Stale** when the executable, version, or capability evidence changes while preserving the old report for diagnosis. Changing the representative model during the test fails that run.

Statuses:

| Status | Meaning | Startup behavior |
| --- | --- | --- |
| Unqualified | No completed evidence | Blocked |
| Incomplete | Version or capability evidence was insufficient | Blocked |
| Failed | Startup, health, inference, or model-integrity check failed | Blocked |
| Cancelled | Operator cancelled the run | Blocked |
| Stale | The engine evidence changed after a prior run | Blocked |
| Passed | All five checks passed and fingerprint still matches | Allowed |

The backend returns stable gate codes for automation and support: `ENGINE_QUALIFICATION_REQUIRED`, `ENGINE_QUALIFICATION_INCOMPLETE`, `ENGINE_QUALIFICATION_FAILED`, and `ENGINE_QUALIFICATION_STALE`.

## Troubleshooting

- **Incomplete:** run the capability probe and confirm the binary exposes a recognizable version plus `--model`/`-m`, `--host`, and `--port` in `--help`.
- **Startup or health failed:** review the report diagnostic and server logs; confirm the model is readable and local security software did not block the temporary loopback listener.
- **Inference failed:** test the same engine/model combination directly and verify it supports the `/completion` endpoint.
- **Stale:** re-probe and re-run qualification. Stale evidence is intentionally never accepted.
- **Cancellation:** wait for the run to finish cleaning up, then start a new qualification. Only one run per engine is allowed at a time.

No remote listener, arbitrary custom argument, user prompt, API credential, or phase-2 rollout policy is part of profile v1.

## 中文说明

引擎资格认证是 Phase 1 的安全启动门：升级后的旧引擎记录会显示“未认证”，需要在“引擎管理”中选择一个已扫描的主生成模型并完成认证。认证只在 `127.0.0.1` 临时端口上以受控 CPU 基线启动所选引擎，依次验证版本、参数能力、进程启动、健康接口和一次固定提示词的代表性推理；成功、失败、超时或取消后都会终止整个临时进程树。

报告会持久保存并绑定引擎文件指纹。引擎文件、版本或 `--help` 能力证据发生变化时，旧报告会保留但标记为“已失效”，实例启动会被阻止，直到重新认证。该认证证明的是基线兼容性，不代表 GPU 性能，也不保证所有模型和运行参数组合。
