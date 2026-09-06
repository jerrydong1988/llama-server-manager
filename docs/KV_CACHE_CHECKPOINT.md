# KV / Prefill Cache Checkpoint

KV / Prefill Cache Checkpoint 是一个默认关闭的实验性功能。它在受控停止本机 `llama-server` 前保存 slot 0 的提示词/KV 状态，并在相同实例再次启动时、管理器代理开放路由之前恢复该状态。它的目标是减少 DeepSeek Harness 等客户端在新会话中重复注入相同长前缀时的 prefill 等待。

KV / Prefill Cache Checkpoint is an opt-in experimental feature. It saves prompt/KV state from slot 0 before a controlled local `llama-server` stop, then restores it before the manager proxy routes traffic after the next start. Its primary goal is to avoid repeating a long prefill when clients such as DeepSeek Harness inject the same context into a new session.

## 支持范围 / Supported Scope

当前实验范围采用可验证的资格条件。以下条件必须同时成立：

- 实例使用本机、受管、结构化启动模式和一个文本 GGUF 逻辑模型；单文件和同目录内命名完整、索引连续的分片集均可。
- `parallel = 1`，启用 prompt cache、slots API 和 idle-slot cache。
- Cache RAM 必须为正数，或设为 `-1` 表示不限制；`0` 会关闭所需的二级 prompt cache。
- 上游是无 TLS、无自定义 path/API prefix 的 loopback HTTP 端点。
- 所选引擎明确公开 `--slots`、`--slot-save-path`、`--cache-ram` 和 `--cache-idle-slots`。
- 滑动窗口注意力模型还必须启用 `--swa-full`，且引擎必须支持该参数。
- 推测解码关闭，或只使用当前引擎 `--help` 明确报告的类型。`ngram-*` 可从恢复后的 target prompt 重建；`draft-*` 还要求引擎在 `--slot-save-path` 帮助中明确声明支持 `slot KV cache and context checkpoints`，且 `ctx_checkpoints > 0`。帮助标记是必要的能力声明，实际收益仍须通过跨进程请求验证。
- 配置外部草稿模型时必须显式选择至少一个受支持的 `draft-*` 类型，且草稿 GGUF（包括完整分片集）必须可读取；`spec-default`、未知类型和外部 lookup cache 仍不支持检查点。
- 模型架构必须可读。已知 hybrid/recurrent 架构默认关闭检查点；唯一保留的 hybrid 实验组合是已有跨进程验收记录的 `qwen35`，还须满足上述 context 持久化声明和 `ctx_checkpoints > 0`。`qwen35moe`、`qwen4exp` 等其它混合架构不会因此放行。上游基线同时记录 hybrid/recurrent 分类，新增架构必须重新审阅。
- 不使用多模型 preset、router、Embedding、Reranker、LoRA 或 mmproj/multimodal。自定义参数只允许检查点安全分类器明确认可的纯加载 I/O 参数；当前包括取值为 `auto`、`on` 或 `off` 的 `--lazy-mode`、`-lzm` 与旧 `--tensor-read-lazy`。未知、缺值、非法、互相冲突或会改变推理状态的自定义参数仍会阻断，并在配置页显示具体标志。

The experimental scope supports one manager-owned local text-generation slot. Rebuildable `ngram-*` types use the original slot format; `draft-*` also requires the context-persistence help marker and positive `ctx_checkpoints`. The marker is a capability claim, not proof of reuse. Known hybrid/recurrent architectures are excluded except experimentally validated `qwen35` with both context requirements; this exception does not cover `qwen35moe` or `qwen4exp`. The upstream baseline tracks memory architecture classifications for review. Custom arguments remain fail-closed except validated lazy-loading aliases. External lookup state, automatic speculation, and multimodal state remain excluded. Sliding-window models additionally require full SWA cache.

不符合条件只会关闭本次运行的 checkpoint；实例仍按原来的冷启动流程运行。配置页会显示稳定的资格原因，不会静默猜测兼容性。

An ineligible configuration disables checkpointing only for that run. The instance still cold-starts normally, and the configuration page reports the exact eligibility reason.

## 配置步骤 / Configuration

1. 先在模型仓库和引擎管理中完成模型扫描与引擎能力探测。
2. 打开实例的“参数配置”，找到“KV / Prefill 缓存检查点”。
3. 启用功能，并保持“受控停止时保存”和“路由前恢复”开启。
4. 如果资格检查列出可修复的缓存/slot 条件，审阅内存影响后点击“应用必需设置”。
5. 对滑动窗口模型，确认启用 SWA 完整缓存；它会增加 KV 内存占用。
6. 保存配置，从实例管理页启动实例。
7. 让 DeepSeek Harness 使用“实例路由”页面提供的管理器代理地址，例如 `http://127.0.0.1:<proxy-port>/v1`，不要使用实例的直连端口。

The restore-before-first-request guarantee applies only to traffic through the manager proxy. A client that connects directly to the `llama-server` port can race the restore operation and is outside this guarantee.

## DeepSeek Harness 注意事项 / DeepSeek Harness Notes

DeepSeek Harness 创建新会话时可能先发送一个较短的标题或会话元数据请求，然后再发送完整仓库上下文。该短请求会占用唯一 slot。启用 idle-slot cache 并提供足够的 Cache RAM 后，llama.cpp 可以把刚恢复的长前缀保留在二级 prompt cache 中，供随后主请求复用。因此这两个设置是 Harness 路径的资格条件，而不仅是性能建议。

DeepSeek Harness may send a short title or session-metadata request before its full repository context. That request temporarily occupies the only slot. Idle-slot caching and sufficient Cache RAM allow llama.cpp to retain the restored long prefix in its secondary prompt cache for the subsequent main request. These settings are therefore eligibility requirements for the Harness path, not optional tuning advice.

Cache RAM 是容量上限，不是永久 pin。多个不相关的大前缀仍可能造成淘汰；如实际日志中的 `cache_n` 明显下降，应增加容量、减少同时竞争的前缀，或把标题模型路由到其他实例。

Cache RAM is a capacity limit, not a permanent pin. Competing large prefixes can still evict entries. If `cache_n` drops substantially, increase the budget, reduce competing prefixes, or route title generation to another instance.

受控停止只保存当时 slot 0 的前缀，不会导出 Cache RAM 中其它历史前缀。若停止前最后处理的是短标题请求，保存的可能就是该短前缀。当前 provider 无法提供多前缀磁盘缓存语义。

A controlled stop saves only the current slot 0 prefix, not other historical prefixes in Cache RAM. A short title request immediately before stopping may therefore become the saved prefix. This provider does not offer a persistent multi-prefix cache.

## 推测解码与 Qwen3.8-Flash-Next / Speculation and Qwen3.8-Flash-Next

`--spec-type` 是逗号分隔的候选集合。配置页会在下拉选择器中根据当前引擎探测结果提供多项选择，并按 llama.cpp 的固定运行优先级生成一个规范化参数；用户勾选的先后顺序不改变运行优先级。`ngram-mod,draft-mtp` 只有在引擎明确声明 context-checkpoint 能力时才可使用 checkpoint；旧引擎仍安全回退冷启动。

`--spec-type` is a comma-separated candidate set. The configuration page presents the selected engine's reported choices in a dropdown and emits one normalized value in llama.cpp runtime-priority order. A mixed `ngram-mod,draft-mtp` chain is checkpoint-eligible only when the engine explicitly confirms context-checkpoint persistence; older engines still fall back cold.

本机 B10679 与三分片 Qwen3.8-Flash-Next 验收确认：普通同进程 prompt cache 可把 4805-token prefill 从约 15.11 秒降到约 135 毫秒，`ngram-mod` 也能正常启动和生成；但该 GGUF 的 `qwen4exp` 架构使用 hybrid recurrent memory。跨 PID restore 虽成功读回 4808/4831 token，后续相同前缀仍为 `cache_n = 0`、约 14.54 秒 prefill；引擎同时明确报告 `swa_full` 不适用于该模型。因此它可以使用普通 KV/prompt cache 和 n-gram 推测解码，但当前不能使用持久化 KV checkpoint，管理器会在哈希与 restore 前安全回退冷启动。

Local B10679 testing with the three-shard Qwen3.8-Flash-Next confirmed working in-process prompt reuse and `ngram-mod`, but its `qwen4exp` architecture uses hybrid recurrent memory. Cross-process slot restore read the saved state successfully while the next identical prompt still reported `cache_n = 0`; the engine also disabled unsupported `swa_full`. This model can use ordinary KV/prompt caching and n-gram speculation, but not persistent checkpoint reuse in the current implementation.

本机基于当前 `master` B10688 重放并加固 llama.cpp PR #26004 后，使用 `Qwen3.8-27B-UD-Q8_K_XL.gguf` 与外部 `Qwen3.8-27B-DFlash2-Q4_K_M.gguf` 完成了真实跨进程验收：slot 文件同时恢复 3 个 context checkpoint，冷启动处理 5610 个 prompt token、约 17.81 秒；同进程复用和进程重启恢复后均处理 1028 个，重启后约 4.36 秒。DFlash2 在恢复后实际生成 16 个并接受 12 个 draft token。保存返回值、磁盘文件大小和恢复读取量均为 1,122,352,516 bytes。该模型的 DFlash block size 为 8，因此 `--spec-draft-n-max 15` 会被引擎安全收敛为 7，这与检查点恢复无关。

Local B10688 plus the replayed and hardened llama.cpp PR #26004 passed a real cross-process run with Qwen3.8-27B Q8_K_XL and the external DFlash2 Q4_K_M draft. Three context checkpoints were restored. The cold run processed 5,610 prompt tokens in about 17.81 seconds; both in-process reuse and post-restart restore processed 1,028, with the latter taking about 4.36 seconds. Post-restore speculation generated 16 and accepted 12 draft tokens. The save response, on-disk size, and restore response all reported exactly 1,122,352,516 bytes. This DFlash model has block size 8, so llama.cpp clamps `--spec-draft-n-max 15` to 7 independently of checkpointing.

## 生命周期与故障行为 / Lifecycle and Failure Behavior

受控停止时，管理器先从代理移除实例、等待在途请求和 slot 排空，再调用官方 slot save API。payload 会校验大小并计算 SHA-256；新的 generation 通过同文件系统原子移动进入 manifest-last 提交目录，不再复制一份同等大小的 payload，只有完整 generation 才能成为最新版本。崩溃、强制退出或排空超时不会产生新的 generation。

启动时，管理器先等待引擎健康，再严格验证模型、引擎和状态相关配置的 fingerprint、manifest、文件类型、大小和 SHA-256。恢复响应和恢复后的 slot 状态还会再次核对。只有完成恢复或明确决定冷启动后，代理才允许该实例接收请求。

On a controlled stop, the manager gates routing, drains requests, saves slot 0, verifies the payload, and atomically moves it into a manifest-last generation without duplicating the payload. On startup, it verifies the full compatibility fingerprint and payload before restore, validates the restore round trip, and only then opens the routing gate.

以下情况会回退冷启动；是否需要重建进程取决于 restore 是否已经发送：

- 没有检查点、自动恢复关闭或提示 token 低于保存阈值。
- 任一主模型或草稿模型分片、引擎启动器或相邻动态运行库、引擎版本/backend、规范化 spec-type 或其他强兼容配置改变。
- manifest、大小、摘要、slot API 响应或恢复后状态不一致。
- 保存/恢复超时、I/O 错误或容量限制。

尚未调用 restore 的文件、兼容性或准备阶段失败，可以直接在当前干净进程冷启动。一旦发送 restore，任何错误或后续验证失败都会进入 `Restart required`，保持路由关闭。管理器终止旧 PID，在新进程中跳过本次自动恢复，健康后才进入 `Ready (cold)`；不会保存可能受污染的旧状态，也不会把 `/erase` 成功当作恢复干净的证明。若进程终止或重建失败，实例保持不可路由并报告失败。用户的自动恢复配置保持不变。

失败 generation 会尽力写入拒绝标记，后续启动跳过该文件；正常容量维护会清理不可加载的 generation。即使标记写入失败，本次新进程仍强制跳过恢复。管理器的后台协议包含资格规则版本，旧版本缓存的启动资格不会绕过新规则。

Pre-restore validation failures can cold-start in the untouched process. Once restore has been sent, any failure keeps routing closed until the old PID has been terminated and a fresh process is healthy with restore skipped for that launch. Erase success is not evidence of clean engine state. Failed process replacement remains unavailable. A rejected-generation marker prevents later retries of that file and is subject to normal storage maintenance; the user's auto-restore setting is preserved. Versioned runtime eligibility also rejects stale launch metadata.

## 隐私、容量与清除 / Privacy, Capacity, and Clear

Checkpoint 文件包含由系统提示、仓库说明、工具定义和用户上下文派生的模型状态，应按敏感数据处理。管理器把文件放在当前用户的私有应用数据目录中，不在日志或状态消息中暴露 payload、提示词、API Key 或路径。它们不是可跨模型、跨引擎版本或跨机器移植的会话备份。

Checkpoint files contain prompt-derived model state and must be treated as sensitive local data. They are stored under the current user's private application data, while logs and status messages omit payloads, prompts, API keys, and private paths. A checkpoint is not a portable conversation backup.

每个实例使用独立容量上限和 generation LRU。恢复时只保留一个 scratch payload：restore 返回后先删除输入 scratch，再创建验证副本；创建每个已知大小的 scratch 前都会检查 payload 大小外加 64 MiB 余量。因此瞬时空间约为已保留 generations 加一个 payload，而不是同一 payload 的三份副本。slot save/restore 的总请求上限为 30 分钟，健康与 slots 探测使用最多 2 秒的短超时。只有实例完全停止且没有保存/恢复操作时，才能在实例页点击“清除检查点”。清除只删除该实例经过边界校验的 checkpoint 根目录，不删除模型或实例配置，且不可恢复。

Each instance has its own capacity limit and generation LRU. Restore keeps only one scratch payload at a time and checks for the payload size plus 64 MiB of free-space headroom before each known-size staging operation. Slot save/restore requests may run for up to 30 minutes; health and slot probes retain a two-second ceiling. Clear is allowed only while the instance is fully stopped and no save or restore is active. It removes only that instance's validated checkpoint root, not its model or configuration, and cannot be undone.

## 验证与排障 / Verification and Troubleshooting

- `Ready (file verified)` 表示检查点通过文件及恢复往返验证；`Ready (cold)` 表示实例可用但本次没有使用检查点。
- 重启恢复期间，代理对该实例返回可重试的 `503` 和 `Retry-After`，不会把请求提前送入引擎。
- 真正的收益应由 llama.cpp 日志中的 `cache_n`、`n_past` 或 prompt evaluation 时间证明；slot restore 返回 HTTP 200 本身不代表前缀已被实际复用。
- `Fingerprint mismatch` 通常表示模型、引擎或状态相关配置改变；这是安全 miss，不应通过手工复制或修改 manifest 绕过。
- 滑动窗口模型若显示需要 SWA Full Cache，请先评估额外 KV 内存后再应用该设置。

状态卡会显示恢复后最近一次观察到的已完成请求的 cached/processed token；能关联同一请求日志时还显示 prefill 时间。轮询可能漏掉短请求，这些计数包含引擎普通 prompt cache 的复用，不能全部归因于磁盘检查点。缺失数据保持未知，实际 cached token 为 0 会明确显示为 0。

`Ready (file verified)` means the restore round trip passed. The status card separately reports cached/processed tokens from the latest observed completed request, with prefill time only when matching request logs are available. Polling can miss short requests; counts include ordinary prompt-cache reuse and do not prove checkpoint-specific benefit. Missing metrics stay unknown. Confirm benefit using the same prefix across a real process restart.

## 官方磁盘缓存跟进 / Official Disk Cache Tracking

截至 2026-09-06，[PR #26004](https://github.com/ggml-org/llama.cpp/pull/26004)（slot 文件中的 context checkpoints）和 [PR #28092](https://github.com/ggml-org/llama.cpp/pull/28092)（原生多前缀磁盘缓存）仍未合并，官方稳定版 v0.4.0 尚不包含这两项。当前保留受控 slot provider，不启用未合并的原生磁盘缓存参数。#28092 维护者还计划替换实现，接入时须跟进其最终继任方案。

接入门槛包括：官方合并并进入稳定发布；相关 CI 通过；模型和实例目录隔离；跨 PID 前缀命中及输出正确性；hybrid/draft 状态完整；损坏文件、磁盘满和恢复失败不会污染后续推理。还须核验 [#27530](https://github.com/ggml-org/llama.cpp/pull/27530) / [#27068](https://github.com/ggml-org/llama.cpp/issues/27068) 的失败清理问题及目标平台的 mmap 行为。未来原生 provider 应与受控 slot provider 互斥，避免同时管理同一状态。

As of 2026-09-06, both PRs remain unmerged and absent from stable v0.4.0. Native disk-cache integration is deferred until a maintained implementation ships with validated restart reuse, model isolation, target/draft correctness, failure recovery, and platform support. Any native provider must be mutually exclusive with manager-controlled slot persistence.
