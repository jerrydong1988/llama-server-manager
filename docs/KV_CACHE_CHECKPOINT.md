# KV / Prefill Cache Checkpoint

KV / Prefill Cache Checkpoint 是一个默认关闭的实验性功能。它在受控停止本机 `llama-server` 前保存 slot 0 的提示词/KV 状态，并在相同实例再次启动时、管理器代理开放路由之前恢复该状态。它的目标是减少 DeepSeek Harness 等客户端在新会话中重复注入相同长前缀时的 prefill 等待。

KV / Prefill Cache Checkpoint is an opt-in experimental feature. It saves prompt/KV state from slot 0 before a controlled local `llama-server` stop, then restores it before the manager proxy routes traffic after the next start. Its primary goal is to avoid repeating a long prefill when clients such as DeepSeek Harness inject the same context into a new session.

## 支持范围 / Supported Scope

第一版有意采用严格资格条件。以下条件必须同时成立：

- 实例使用本机、受管、结构化启动模式和单个未分片文本 GGUF。
- `parallel = 1`，启用 prompt cache、slots API 和 idle-slot cache。
- Cache RAM 必须为正数，或设为 `-1` 表示不限制；`0` 会关闭所需的二级 prompt cache。
- 上游是无 TLS、无自定义 path/API prefix 的 loopback HTTP 端点。
- 所选引擎明确公开 `--slots`、`--slot-save-path`、`--cache-ram` 和 `--cache-idle-slots`。
- 滑动窗口注意力模型还必须启用 `--swa-full`，且引擎必须支持该参数。
- 不使用自定义参数、多模型 preset、router、Embedding、Reranker、推测解码、LoRA、mmproj/multimodal 或已知 hybrid/recurrent 架构。

The first version is deliberately conservative. It supports one manager-owned local text-generation slot, a single unsharded GGUF, loopback HTTP, and engines that advertise the required slot and prompt-cache flags. Custom arguments, multi-model modes, vector workloads, speculative decoding, LoRA, multimodal state, and known hybrid or recurrent architectures are excluded. Sliding-window models additionally require full SWA cache.

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

## 生命周期与故障行为 / Lifecycle and Failure Behavior

受控停止时，管理器先从代理移除实例、等待在途请求和 slot 排空，再调用官方 slot save API。payload 会校验大小并计算 SHA-256；新的 generation 采用 manifest-last 提交，只有完整 generation 才能成为最新版本。崩溃、强制退出或排空超时不会产生新的 generation。

启动时，管理器先等待引擎健康，再严格验证模型、引擎和状态相关配置的 fingerprint、manifest、文件类型、大小和 SHA-256。恢复响应和恢复后的 slot 状态还会再次核对。只有完成恢复或明确决定冷启动后，代理才允许该实例接收请求。

On a controlled stop, the manager gates routing, drains requests, saves slot 0, verifies the payload, and commits a manifest-last generation. On startup, it verifies the full compatibility fingerprint and payload before restore, validates the restore round trip, and only then opens the routing gate.

以下情况都会安全退化为可路由的冷启动，而不会阻止实例启动：

- 没有检查点、自动恢复关闭或提示 token 低于保存阈值。
- 模型、引擎二进制、引擎版本/backend 或任一强兼容配置改变。
- manifest、大小、摘要、slot API 响应或恢复后状态不一致。
- 保存/恢复超时、I/O 错误或容量限制。

Any missing, incompatible, corrupt, timed-out, or otherwise unverifiable checkpoint fails open to a routable cold start. A partially restored slot is erased before cold routing.

## 隐私、容量与清除 / Privacy, Capacity, and Clear

Checkpoint 文件包含由系统提示、仓库说明、工具定义和用户上下文派生的模型状态，应按敏感数据处理。管理器把文件放在当前用户的私有应用数据目录中，不在日志或状态消息中暴露 payload、提示词、API Key 或路径。它们不是可跨模型、跨引擎版本或跨机器移植的会话备份。

Checkpoint files contain prompt-derived model state and must be treated as sensitive local data. They are stored under the current user's private application data, while logs and status messages omit payloads, prompts, API keys, and private paths. A checkpoint is not a portable conversation backup.

每个实例使用独立容量上限和 generation LRU。只有实例完全停止且没有保存/恢复操作时，才能在实例页点击“清除检查点”。清除只删除该实例经过边界校验的 checkpoint 根目录，不删除模型或实例配置，且不可恢复。

Each instance has its own capacity limit and generation LRU. Clear is allowed only while the instance is fully stopped and no save or restore is active. It removes only that instance's validated checkpoint root, not its model or configuration, and cannot be undone.

## 验证与排障 / Verification and Troubleshooting

- `Ready (restored)` 表示检查点通过完整验证并完成 restore；`Ready (cold)` 表示实例可用但本次没有使用检查点。
- 重启恢复期间，代理对该实例返回可重试的 `503` 和 `Retry-After`，不会把请求提前送入引擎。
- 真正的收益应由 llama.cpp 日志中的 `cache_n`、`n_past` 或 prompt evaluation 时间证明；slot restore 返回 HTTP 200 本身不代表前缀已被实际复用。
- `Fingerprint mismatch` 通常表示模型、引擎或状态相关配置改变；这是安全 miss，不应通过手工复制或修改 manifest 绕过。
- 滑动窗口模型若显示需要 SWA Full Cache，请先评估额外 KV 内存后再应用该设置。

`Ready (restored)` means a verified restore completed. `Ready (cold)` means the instance is usable without a checkpoint. Confirm real reuse with llama.cpp `cache_n`, `n_past`, or prompt-evaluation metrics; an HTTP 200 from the restore endpoint alone is not sufficient evidence.
