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
- 推测解码关闭，或只使用当前引擎 `--help` 明确报告的类型。`ngram-*` 可从恢复后的 target prompt 重建；`draft-*` 还要求引擎在 `--slot-save-path` 帮助中明确声明支持 `slot KV cache and context checkpoints`，证明 slot 文件会同时携带 target/draft 上下文。
- 配置外部草稿模型时必须显式选择至少一个受支持的 `draft-*` 类型，且草稿 GGUF（包括完整分片集）必须可读取；`spec-default`、未知类型和外部 lookup cache 仍不支持检查点。
- 模型架构必须可读，且不属于已有反例证明的 hybrid/recurrent 架构；不使用多模型 preset、router、Embedding、Reranker、LoRA 或 mmproj/multimodal。自定义参数只允许检查点安全分类器明确认可的纯加载 I/O 参数；当前包括取值为 `auto`、`on` 或 `off` 的 `--lazy-mode`、`-lzm` 与旧 `--tensor-read-lazy`。未知、缺值、非法、互相冲突或会改变推理状态的自定义参数仍会阻断，并在配置页显示具体标志。

The experimental v3 scope supports one manager-owned local text-generation slot. Every selected speculative type must be reported by the current engine. Rebuildable `ngram-*` types remain eligible on the original slot format; `draft-*` types additionally require the explicit context-checkpoint help marker introduced by a compatible llama.cpp build. Custom arguments remain fail-closed except for explicitly classified load-I/O-only forms; the current safe set is `--lazy-mode`, `-lzm`, and legacy `--tensor-read-lazy` with `auto`, `on`, or `off`. External lookup state, automatic speculation, multimodal state, and known hybrid or recurrent architectures remain excluded. Sliding-window models additionally require full SWA cache.

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

以下情况都会安全退化为可路由的冷启动，而不会阻止实例启动：

- 没有检查点、自动恢复关闭或提示 token 低于保存阈值。
- 任一主模型或草稿模型分片、引擎启动器或相邻动态运行库、引擎版本/backend、规范化 spec-type 或其他强兼容配置改变。
- manifest、大小、摘要、slot API 响应或恢复后状态不一致。
- 保存/恢复超时、I/O 错误或容量限制。

Any missing, incompatible, corrupt, timed-out, or otherwise unverifiable checkpoint fails open to a routable cold start. A partially restored slot is erased before cold routing.

## 隐私、容量与清除 / Privacy, Capacity, and Clear

Checkpoint 文件包含由系统提示、仓库说明、工具定义和用户上下文派生的模型状态，应按敏感数据处理。管理器把文件放在当前用户的私有应用数据目录中，不在日志或状态消息中暴露 payload、提示词、API Key 或路径。它们不是可跨模型、跨引擎版本或跨机器移植的会话备份。

Checkpoint files contain prompt-derived model state and must be treated as sensitive local data. They are stored under the current user's private application data, while logs and status messages omit payloads, prompts, API keys, and private paths. A checkpoint is not a portable conversation backup.

每个实例使用独立容量上限和 generation LRU。恢复时只保留一个 scratch payload：restore 返回后先删除输入 scratch，再创建验证副本；创建每个已知大小的 scratch 前都会检查 payload 大小外加 64 MiB 余量。因此瞬时空间约为已保留 generations 加一个 payload，而不是同一 payload 的三份副本。slot save/restore 的总请求上限为 30 分钟；erase 清理保留 30 秒上限，健康与 slots 探测仍使用最多 2 秒的短超时。只有实例完全停止且没有保存/恢复操作时，才能在实例页点击“清除检查点”。清除只删除该实例经过边界校验的 checkpoint 根目录，不删除模型或实例配置，且不可恢复。

Each instance has its own capacity limit and generation LRU. Restore keeps only one scratch payload at a time and checks for the payload size plus 64 MiB of free-space headroom before each known-size staging operation. Slot save/restore requests may run for up to 30 minutes; erase retains a 30-second ceiling, while health and slot probes retain a short two-second ceiling. Clear is allowed only while the instance is fully stopped and no save or restore is active. It removes only that instance's validated checkpoint root, not its model or configuration, and cannot be undone.

## 验证与排障 / Verification and Troubleshooting

- `Ready (restored)` 表示检查点通过完整验证并完成 restore；`Ready (cold)` 表示实例可用但本次没有使用检查点。
- 重启恢复期间，代理对该实例返回可重试的 `503` 和 `Retry-After`，不会把请求提前送入引擎。
- 真正的收益应由 llama.cpp 日志中的 `cache_n`、`n_past` 或 prompt evaluation 时间证明；slot restore 返回 HTTP 200 本身不代表前缀已被实际复用。
- `Fingerprint mismatch` 通常表示模型、引擎或状态相关配置改变；这是安全 miss，不应通过手工复制或修改 manifest 绕过。
- 滑动窗口模型若显示需要 SWA Full Cache，请先评估额外 KV 内存后再应用该设置。

`Ready (restored)` means a verified restore completed. `Ready (cold)` means the instance is usable without a checkpoint. Confirm real reuse with llama.cpp `cache_n`, `n_past`, or prompt-evaluation metrics; an HTTP 200 from the restore endpoint alone is not sufficient evidence.
