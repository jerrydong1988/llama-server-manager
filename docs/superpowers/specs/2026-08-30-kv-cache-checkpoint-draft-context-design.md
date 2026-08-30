# KV / Prefill Cache Checkpoint：草稿上下文持久化设计增补

日期：2026-08-30
状态：实现并完成本机技术验收，等待用户在 DeepSeek Harness 日常工作流中验收

## 目标

在不降低现有冷启动兜底和兼容性校验的前提下，允许支持完整 slot 上下文序列化的 llama.cpp 引擎，为外部草稿模型和 `draft-*` 推测解码保存、恢复同一份可跨进程使用的状态。

管理器不复制 llama.cpp 的序列化实现，只使用官方 slots API。llama.cpp 负责 slot 文件中的 target/draft context appendix；管理器负责能力协商、生命周期编排、完整性保护、兼容性 fingerprint、路由门和冷启动降级。

## 已验证事实

| 场景 | 引擎 | 结果 |
|---|---|---|
| 未应用 PR #26004，target-only | 当前 master 基线 B10688 | 重启后重复 prompt 仍处理 210 token，而同进程只处理 35 token，红灯 |
| 未应用 PR #26004，外部 draft-simple | 当前 master 基线 B10688 | 同样重启退化为 210 token，红灯 |
| 应用 PR #26004，target-only | 当前 master B10688 | 冷启动 212、同进程 35、重启后 35 token，绿灯 |
| 应用 PR #26004，外部 draft-simple | 当前 master B10688 | 重启后 35 token，且 draft 实际生成与接受均非零，绿灯 |
| Qwen3.8-27B + 外部 DFlash2 | HIP/gfx1151 B10688 + PR #26004 加固版 | 恢复 3 个 context checkpoint；冷启动 5610 token，同进程与重启后均为 1028 token；恢复后 draft 16/accepted 12，绿灯 |

真实模型验收使用：

- target：`Qwen3.8-27B-UD-Q8_K_XL.gguf`
- draft：`Qwen3.8-27B-DFlash2-Q4_K_M.gguf`
- `--spec-type draft-dflash`
- slot 文件：1,122,352,516 bytes，保存返回值、磁盘大小与恢复读取量完全一致
- 冷启动 prompt eval：约 17.81 s；同进程复用：约 3.49 s；重启恢复后：约 4.36 s

这证明收益来自重启后实际减少 prefill，并且 draft 路径在恢复后继续工作；HTTP 200 或“读取了 slot 文件”本身不作为收益证据。

## 能力契约

基础 checkpoint 资格继续要求引擎报告：

- `--slots`
- `--slot-save-path`
- `--cache-ram`
- `--cache-idle-slots`

`ngram-*` 仍可由恢复后的 target prompt 重建，不要求新格式。任何 `draft-*` 还必须满足：

1. 引擎 `--help` 实际报告该具体 `draft-*` 类型；
2. `--slot-save-path` 的帮助包含 `path to save slot KV cache and context checkpoints`；
3. 外部草稿模型存在时，配置显式包含至少一个 `draft-*` 类型；
4. target 与 draft GGUF 文件集合均完整。

帮助文本是当前实验阶段的明确正向能力标记。没有标记不猜测构建号、不扫描私有二进制字符串，也不因 `--slot-save-path` 本身存在就乐观放行。

## Fingerprint v3

恢复兼容性 fingerprint 从 v2 升到 v3，并纳入：

- target GGUF 内容摘要；分片模型为有序聚合摘要；
- draft GGUF 内容摘要；分片模型同样有序聚合；
- llama-server 启动器与同目录全部 `.dll`、`.dylib`、`.so`/版本化 `.so` 的有序聚合摘要；
- 引擎版本、backend、规范化后的 `spec-type`、草稿 KV 类型、draft token/概率/device/backend sampling/线程设置，以及原有全部状态相关参数。

路径本身不参与模型内容身份，因此同内容安全搬迁保持 fingerprint；文件内容、运行库集合或文件名变化会产生安全 miss。旧 v2 generation 不迁移为 v3，而是自然找不到匹配 fingerprint 并冷启动。

引擎能力探测使用快速运行时指纹，同样覆盖启动器和相邻动态库。这样 Windows 动态构建中仅替换 `llama-server-impl.dll` 或 `llama-common.dll` 时，旧的能力探测结果也会立即失效。

## 生命周期与失败语义

生命周期不变：排空代理与 slot 后保存；启动健康后、开放路由前恢复；校验 manifest、大小、SHA-256、restore 响应和恢复后的 slot 状态。任何不合格、缺失、损坏、超时或不匹配都先清除可能的部分状态，再进入可路由冷启动。

以下状态仍明确不支持：

- `--spec-default` 自动推测；
- 外部静态或动态 lookup cache；
- 未知 spec 类型；
- 含外部草稿模型但未显式选择 `draft-*`；
- LoRA、多模态、向量工作负载、多模型路由；
- 已有实测反例的 hybrid/recurrent 架构。

## 安全边界

- Checkpoint 是提示词派生的敏感本地数据，继续使用私有目录、manifest-last 提交、摘要校验和受限路径。
- 管理器不解析 llama.cpp slot payload 或 context appendix，避免复制上游内部格式。
- llama.cpp appendix 读取在分配前同时校验 16 GiB 硬上限与文件实际剩余字节，限制保留的 checkpoint 数量，拒绝无效元数据和尾随数据；append/flush 失败会让 slot save 明确失败。
- 引擎/模型内容变化时不尝试“尽可能恢复”；只允许精确兼容命中。
- 引擎崩溃和强制终止不创建新 generation，保留上一份已提交状态。

## 上游贡献边界

本地 llama.cpp 候选贡献只补充三类内容：

1. 真正跨 `server.stop()` / `server.start()` 的 target-only 与外部 draft 回归测试，证明未打补丁失败、打补丁成功，并检查恢复后 draft 指标非零；
2. 在 `--slot-save-path` 帮助中说明 `KV cache and context checkpoints`，为服务管理器提供正向能力协商契约。
3. 对 context appendix 做剩余长度先验校验、受限保留、无效元数据/尾随数据拒绝和明确写失败传播，并用伪造 16 GiB buffer 长度字段验证服务不会巨额分配或退出。

上游提交、推送和 PR 文字由用户逐行审阅后手工完成；管理器仓库不会复制 PR #26004 的实现源码。

## 验收门

- Rust 全套单元测试通过；
- 前端生产构建、KV checkpoint、引擎能力和推测解码契约测试通过；
- llama.cpp 未打补丁基线的跨进程测试稳定失败；
- 补丁版 CPU 测试、speculative 测试与 HIP 构建通过；
- 真实 Qwen3.8-27B + DFlash2 重启后 prompt 处理量显著低于冷启动，且恢复后 draft 生成/接受均非零；
- 最终由用户在 DeepSeek Harness 新会话工作流确认首响应等待改善、路由顺序和长期稳定性。
