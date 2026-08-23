# Engine Qualification

Engine qualification is a reproducible compatibility smoke test for one scanned `llama-server` artifact and one representative generative GGUF model. It is diagnostic evidence, not a security boundary and not a prerequisite for starting an instance.

Security-sensitive launch checks remain fail-closed independently of qualification: the configured engine must still exist in the scanned inventory, the exact engine and model content identities must bind successfully, the saved configuration and deployment identity must verify, required security flags must be supported, and public listeners must satisfy authentication and TLS policy.

## Operator workflow

1. Add and scan an authorized engine directory and an authorized model directory.
2. Open **Engine Manager**, select the engine, and run the normal capability probe if needed.
3. In **Engine Qualification**, select a representative primary generative model and choose **Run qualification**.
4. Review version, capabilities, startup, health, and inference evidence, including the recorded backend and execution profile.
5. Use a failed, cancelled, stale, or missing report as a warning. The instance may still start, and its actual health state and logs remain authoritative for that concrete configuration.

Old reports remain readable after upgrade. A report from an older qualification profile becomes **Stale** and may be rerun, but it does not block instance startup.

## Qualification profile v3

The application re-probes the explicitly selected engine before launching the temporary server. Version detection and complete `--help` capability evidence must pass before the runtime smoke test. The profile then:

- executes only the selected, scanned engine and selected, scanned primary generative model;
- runs directly from a protected directory or from the verified private Windows engine snapshot when the selected directory is externally writable;
- leaves a large external Windows GGUF in place while binding its complete identity and retaining stable read-only file and ancestor-directory handles;
- binds to `127.0.0.1` on a temporary free port;
- records the detected backend in the report;
- for a GPU backend, requires a supported `--n-gpu-layers`/`-ngl` flag and explicitly requests `999` layers so the test cannot silently use the old CPU-only baseline;
- for a CPU backend, keeps native CPU placement and labels the execution profile accordingly;
- bounds context to 512 tokens, parallelism to one slot, and batch/micro-batch size to 512 when supported;
- disables redundant startup warmup with `--no-warmup` when supported, then still performs a real inference request;
- enables `--no-ui` and `--offline` when supported;
- waits up to 180 seconds for a successful `GET /health`; repeated HTTP 503 responses are reported as a model-loading timeout, not proof of incompatibility;
- sends the fixed prompt `LSM qualification probe.` to `POST /completion` and requires generated output or predicted tokens;
- terminates the complete temporary process tree after pass, failure, timeout, or cancellation.

This is a representative GPU-offload or CPU-backend smoke test. It is not a performance benchmark and cannot prove that every model, context size, batch size, device split, speculative configuration, or manually supplied command will work. Starting an actual instance remains the definitive test of that instance's saved configuration.

## Report, invalidation, and launch policy

The persisted schema-2/profile-3 report contains status, engine fingerprint and complete artifact ID, detected version, capability-help hash, backend and execution profile, representative model identity/size/modification time, timestamps, per-check duration/details, and a bounded diagnostic. A deterministic evidence ID seals a complete passed report. The fixed prompt is redacted from diagnostics.

The deployment identity always binds the exact engine artifact, model artifact, and configuration revision. A current passed report contributes its sealed qualification evidence. When qualification is missing, failed, cancelled, incomplete, or stale, LSM creates deterministic advisory admission evidence from the current engine identity, fingerprint, status, and profile. This preserves reproducible deployment revisions without treating qualification as an authorization decision.

| Status | Meaning | Instance startup |
| --- | --- | --- |
| Unqualified | No completed compatibility evidence | Allowed with warning |
| Incomplete | Version or required capability evidence was insufficient | Allowed with warning |
| Failed | Startup, health, inference, or model-integrity check failed | Allowed with warning |
| Cancelled | Operator cancelled the run | Allowed with warning |
| Stale | Engine or qualification-profile evidence changed | Allowed with warning |
| Passed | All five checks passed for the recorded backend/profile | Allowed |

Qualification status never relaxes the hard artifact, configuration, security, resource, or recovery checks. Replacing an engine or model, changing a bound configuration, tampering with a deployment revision, losing required authentication, or violating the runtime resource plan can still block launch or automatic recovery.

## Troubleshooting

- **Incomplete:** run the capability probe and confirm `--help` exposes `--model`/`-m`, `--host`, `--port`, and for GPU builds `--n-gpu-layers`/`-ngl`.
- **HTTP 503 until timeout:** the temporary server remained in model loading for 180 seconds. Review the diagnostic and try the actual instance configuration; this result alone does not prove incompatibility.
- **Startup or health failed:** review the bounded diagnostic and confirm the model is readable, the selected GPU has sufficient memory, and local security software did not block the loopback listener.
- **Inference failed:** test the same engine/model/backend combination directly and verify the `/completion` endpoint.
- **Stale:** re-probe and rerun qualification when current advisory evidence is useful.
- **Cancellation:** wait for process-tree cleanup, then start another qualification. Only one qualification per engine runs at a time.

No remote listener, arbitrary custom argument, user prompt, API credential, or performance claim is part of profile v3.

## 中文说明

引擎资格认证现在是“兼容性冒烟测试”，不是启动授权门。GPU 引擎会显式请求 `--n-gpu-layers 999`，在限制上下文、并发和批量的临时本机服务中完成模型加载、健康检查与一次真实推理；CPU 引擎则明确记录为 CPU 执行配置。连续 HTTP 503 只表示模型在 180 秒内仍未加载完成，不再被描述成引擎与模型一定不兼容。

未认证、未通过、取消、证据不完整或已失效都会显示告警，但不会单独阻止实例启动。真正继续安全阻断的是引擎或模型制品身份不一致、配置/部署身份失效、安全参数不受支持、公开监听缺少鉴权或 TLS、资源计划不可行等硬性条件。这样既保留了可复现的诊断证据，也让实际实例的真实配置、健康状态和日志成为最终判断依据。
