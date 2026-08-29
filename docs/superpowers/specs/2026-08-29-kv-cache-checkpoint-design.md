# llama-server KV / Prefill Cache Checkpoint 设计

日期：2026-08-29

状态：已实现；默认关闭并标记实验性

## 1. 背景

`llama-server` 的 KV cache 位于推理进程内存中。只要本机 `llama-server` 进程退出或重启，现有 slot 和已经完成的 prefill 都会消失。DeepSeek Harness 在新会话中重新注入相同的系统提示、仓库说明和工具上下文时，模型必须再次处理完整前缀，因此首 token 延迟可能远大于后续生成延迟。

本项目已经能够为实例生成 `--cache-prompt`、`--slots` 和 `--slot-save-path` 参数，并采集 `/slots` 状态，但还没有在实例生命周期中调用 slot save/restore API，也没有持久化格式、兼容性验证、完整性校验或恢复期间的路由门。

llama.cpp 当前公开以下能力：

- 通过 `--slot-save-path` 指定 slot 文件目录。
- `POST /slots/{id}?action=save` 保存 slot。
- `POST /slots/{id}?action=restore` 恢复 slot。
- `POST /slots/{id}?action=erase` 清除 slot。

这些 API 是必要的底层机制，但不是一个完整的服务管理方案。管理器仍需决定何时停止接收请求、保存哪些 slot、如何证明文件属于同一模型和运行配置、何时对 Harness 宣告实例可用，以及任何失败发生后如何安全回到冷启动。

## 2. 最终目标

在用户显式启用后，llama-server-manager 应做到：

1. 在受控停止本机受管 `llama-server` 之前保存有价值的 prefill/KV 状态。
2. 在相同模型、相同引擎和相同状态相关配置再次启动时，恢复最新完整检查点。
3. 恢复必须发生在管理器代理把实例标记为可路由之前。
4. 不兼容、损坏、缺失、超时、不支持或无收益的检查点必须安全退化为冷启动。
5. 检查点失败不得阻止实例启动或导致未经验证的 KV 状态参与推理。
6. 用户能够看到最近保存/恢复结果、占用空间和失败原因，并能够禁用或清除检查点。

最终用户路径为：DeepSeek Harness 使用 llama-server-manager 的稳定代理地址；Harness 等待代理 readiness 后发起第一个请求。直连 `llama-server` 端口无法由管理器完全阻止，因此不在“恢复一定先于首请求”的承诺范围内。

## 3. 验收定义

### 3.1 功能验收

- 使用受支持的文本 GGUF、单 slot 和固定配置完成一次长前缀请求。
- 通过管理器执行受控停止，产生一份完整、可校验的 checkpoint generation。
- 使用相同配置重启实例，代理在 restore 完成前保持 not ready。
- 代理 ready 后发送同一长前缀，llama.cpp 返回的缓存命中 token 数证明大部分前缀被复用；不得只以 `restore` HTTP 200 作为成功证据。
- 修改任一强兼容字段后重启，旧 checkpoint 不得进入 restore，实例应冷启动并显示 fingerprint mismatch。
- 篡改或截断 slot 文件后重启，校验必须在调用 restore 前失败；实例仍应变为可用冷启动状态。
- restore 请求超时、返回非法 token 数或 restore 后 slot 状态不一致时，应主动 erase 目标 slot，再进入冷启动可用状态。

### 3.2 性能验收

- 真实目标模型上分别记录 cold prefill、checkpoint verify、restore 和 warm prefill 时间。
- warm 请求必须观测到有效 cache hit；相同输入长度下的首 token 延迟应低于 cold prefill。
- 性能数值作为本机验收证据记录，不写成跨硬件的固定承诺，也不作为易抖动的 CI 阈值。

### 3.3 竞态验收

- 人为延迟 restore，并在此期间轮询代理 readiness；代理必须持续返回 not ready。
- restore 期间直接向代理发送推理请求时，不得将请求转发到该实例；返回可重试的 503，或选择另一个已经 ready 的候选实例。
- stop 进入 draining 后不得再向该实例分配新请求；已有请求必须在有界时间内完成，超时后放弃本次保存并继续停止。

## 4. 非目标

第一版不实现：

1. 在进程崩溃、断电或操作系统强制终止时创建新检查点。最近一次已提交 generation 可以保留并在以后恢复。
2. 由 OpenAI/Anthropic API 客户端选择、上传、下载或命名 checkpoint。
3. 任意会话的多 checkpoint 快速切换、跨实例迁移或跨机器分发。
4. llama.cpp router mode、远程实例、手工命令模式和非 llama.cpp 引擎。
5. 多 slot 调度、并发会话映射和部分 slot 恢复。第一版只支持有效 `parallel = 1` 的 slot 0。
6. Embedding、Reranker、推测解码、LoRA、multimodal/mmproj、混合或 recurrent 状态。
7. 跨 llama.cpp 二进制版本复用。引擎内容变化即视为不兼容。
8. 修改或分发定制 llama.cpp 二进制。

这些限制优先保证错误检查点不会进入推理。后续扩大范围必须先增加对应真实引擎回归用例和 fingerprint 字段，再单独设计。

## 5. 开源项目调研结论

调研样本按 2026-08-29 GitHub 公开数据选取。star 数只用于说明社区采用面，不作为代码质量或 checkpoint 成熟度证明：

| 项目 | Stars（快照） | 与磁盘 checkpoint 的关系 |
| --- | ---: | --- |
| [Ollama](https://github.com/ollama/ollama) | 179,673 | 稳定版以 runner 驻留为主；有未合并的 scheduler-owned 持久化 PR |
| [llama.cpp](https://github.com/ggml-org/llama.cpp) | 126,125 | 提供 slot save/restore 官方底层 API |
| [LocalAI](https://github.com/mudler/LocalAI) | 48,727 | 主要使用进程内 cache/worker 生命周期 |
| [KoboldCpp](https://github.com/LostRuins/koboldcpp) | 11,558 | RAM smart context 成熟，磁盘状态仍是需求方向 |
| [GPUStack](https://github.com/gpustack/gpustack) | 5,571 | 服务编排可下传底层能力，未见完整 slot checkpoint 生命周期 |
| [llama-swap](https://github.com/mostlygeek/llama-swap) | 5,502 | 社区明确提出 restore/readiness 竞态和 hook/shim 思路 |
| [Warpdrv](https://github.com/mikjee/warpdrv) | 107 | 小众但直接实现了自动 save/load、metadata、容量和 UI 概念 |

### 5.1 llama.cpp

[官方 server 文档](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md#post-slotsid_slotactionsave-save-the-prompt-cache-of-the-specified-slot-to-a-file) 给出了 slot save/restore/erase API。这是本项目唯一依赖的底层接口。

上游测试已经覆盖普通 slot round trip 和部分跨重启场景，但当前仍有需要管理层防御的问题：

- [hybrid/recurrent restore 可能报告成功却没有实际复用](https://github.com/ggml-org/llama.cpp/issues/25913)。
- [部分跨重启场景会出现 n_restored 成功但后续 cache_n 为 0](https://github.com/ggml-org/llama.cpp/issues/26676)。
- [`--slot-save-path` 内符号链接逃逸问题](https://github.com/ggml-org/llama.cpp/issues/26315) 的[修复 PR](https://github.com/ggml-org/llama.cpp/pull/26316) 在本设计日期仍未合并。

因此实现必须自行验证文件、slot 状态和实际复用，不能把 HTTP 200 当作充分证明，也不能把用户可写的任意目录和文件名直接交给 endpoint。

### 5.2 Ollama

Ollama 稳定版本主要通过 runner 驻留保持内存 cache。其尚未合并的[持久化 prefill cache PR #17953](https://github.com/ollama/ollama/pull/17953) 提供了值得借鉴的管理层设计：scheduler 拥有生命周期、模型/运行配置 SHA-256 身份、校验和、manifest-last 提交、容量 LRU、fault injection 和 fail-open 冷启动。

本项目借鉴这些设计原则，不依赖该 PR，不复制其源码，也不把仍处于实验状态的性能数据当作本项目保证。

### 5.3 llama-swap

[llama-swap discussion #615](https://github.com/mostlygeek/llama-swap/discussions/615) 明确描述了关键竞态：llama.cpp 已经 health-ready 时，外层交换器可能在 restore 前转发首请求。讨论中的代理 shim 思路是先启动内部随机端口、等待引擎健康、restore，最后才开始代理。

本项目已有稳定路由代理，因此不额外引入 wrapper 进程，而是在现有目标选择和 readiness 中加入 checkpoint gate。

### 5.4 Warpdrv、KoboldCpp、LocalAI 和 GPUStack

- [Warpdrv](https://github.com/mikjee/warpdrv) 已实现 slot 文件、sidecar metadata、自动保存/加载、容量限制和 UI 概念。其许可证为 AGPL-3.0，llama-server-manager 为 MIT；本项目只借鉴产品和状态机思路，基于官方 API 独立实现，不复制源码。其仅以模型文件名和大小建立兼容性的做法不足以保证安全恢复。
- [KoboldCpp](https://github.com/LostRuins/koboldcpp) 的 smart context 主要是进程内 RAM snapshot；磁盘状态仍是长期需求，说明“内存复用”和“跨进程持久化”必须分开描述。
- [LocalAI](https://github.com/mudler/LocalAI) 和 [GPUStack](https://github.com/gpustack/gpustack) 更偏向保持 worker/runner 或暴露底层参数，没有提供可直接复用的完整 llama.cpp slot 生命周期契约。

结论是：主流项目的稳定路径仍以进程驻留为主；真正的磁盘 checkpoint 必须由实例 scheduler/lifecycle 层控制，而不是增加一个客户端 API 开关。

## 6. 支持矩阵

| 条件 | 第一版行为 |
| --- | --- |
| 本机、受管、结构化 launch mode | 支持候选 |
| 标准文本生成 GGUF | 支持候选，仍需真实 cache hit 验证 |
| `parallel = 1`、slot 0 | 支持 |
| `cache_prompt = true`、`slots_enabled = true` | 必需；配置页提示并提供显式修复动作，不静默改写 |
| `cache_idle_slots = true`、`cache_ram > 0` 或 `-1` | Harness 会先发标题/元数据请求，必须用二级 prompt cache 保留已恢复长前缀 |
| 已知 sliding-window attention 模型 | 只有启用 `swa_full` 且引擎支持该参数时才支持；否则 restore 200 仍可能没有真实 cache hit |
| loopback HTTP upstream | 支持 |
| 手工命令、自定义未知参数 | 不支持，冷启动并说明原因 |
| router / models preset / remote target | 不支持 |
| Embedding / Reranker | 不支持 |
| draft/speculative decoding | 不支持 |
| LoRA 或 request-time LoRA | 不支持 |
| mmproj / multimodal | 不支持 |
| 已知 hybrid / recurrent 架构 | 不支持 |
| TLS upstream 或非 loopback bind | 第一版不支持；不降低 TLS 校验来换取 restore |
| 引擎缺少 `--slots`、`--slot-save-path`、`--cache-ram` 或 `--cache-idle-slots` | 不支持 |
| fingerprint 无法计算 | 不 restore、不保存，实例冷启动 |

“不支持”只关闭本次运行的 checkpoint，不阻止实例正常启动。

## 7. 架构决策

### 7.1 所有权

新增 Rust `CheckpointCoordinator`，由实例生命周期持有并调用。GUI 直管路径和独立 runtime service 必须共享同一套核心实现和 manifest 格式，不能各自实现一份行为不同的 save/restore。

CheckpointCoordinator 负责：

- eligibility 判断与明确原因。
- 运行目录准备和有效启动参数注入。
- fingerprint 计算与缓存。
- manifest 与 generation 存储。
- slot HTTP client。
- save、verify、restore、erase、容量清理。
- 每实例阶段、最近结果和错误状态。

server/runtime supervisor 负责：

- 持有已有的 per-instance lifecycle lock。
- 在启动、健康探测、停止和异常退出的正确位置调用 coordinator。
- 把 checkpoint readiness 提供给路由代理和 UI。
- 在 checkpoint 失败后继续原有启动或停止流程。

### 7.2 状态机

```text
STOPPED
   |
   v
STARTING -> ENGINE_HEALTHY -> RESTORING -> READY
                |                |
                | no checkpoint  | verify/restore failed
                +--------------> READY_COLD

READY / READY_COLD -> DRAINING -> SAVING -> STOPPING -> STOPPED
                           |          |
                           | timeout  | save failed
                           +----------+----------> STOPPING
```

规则：

- `ENGINE_HEALTHY` 仅表示 llama-server 的 `/health` 可访问，不表示代理可路由。
- 只有 `READY` 和 `READY_COLD` 可进入代理候选集合。
- `READY_COLD` 是可用状态，同时保留本次未恢复原因。
- 未启用 checkpoint 的实例沿用现有 health 行为，不增加额外启动等待。
- 所有阶段转换带 `instance_id + expected_pid`，旧进程的晚到事件不得覆盖新进程状态。

### 7.3 启动流程

1. 在 spawn 前检查配置和引擎能力。
2. 为 eligible 实例创建管理器私有的确定性 scratch 目录，并清理其中不属于活进程的旧暂存文件。
3. 由管理器写入有效 `--slot-save-path <scratch>`，同时确保 slots API 和 prompt cache 可用。启用受管 checkpoint 时不接受用户自定义 slot 保存目录。
4. 登记 `STARTING` 和 `routable = false`，再启动进程。
5. 现有 health monitor 首次确认引擎健康后进入 `ENGINE_HEALTHY`。
6. 如果 auto-restore 关闭、没有 generation、fingerprint 不匹配或检查点不合格，进入 `READY_COLD`。
7. 如果存在兼容 generation：
   - 读取并严格解析 manifest。
   - 验证路径、普通文件类型、文件大小和 SHA-256。
   - 将 slot payload 复制到本次 scratch 的管理器生成文件名。
   - 调用 slot 0 restore，验证 `id_slot`、`n_restored` 和 `n_read`。
   - 再读取 `/slots`，确认 slot 0 的 prompt token 状态与 manifest 一致。
8. 成功进入 `READY`；任何一步失败时调用 erase，记录可见原因并进入 `READY_COLD`。
9. 代理在下一次 snapshot 中才会看见该目标。

### 7.4 停止流程

1. 在已有 lifecycle lock 内把实例置为 `DRAINING` 并立即从代理候选中移除。
2. 等待代理中该实例的 in-flight 请求和 `/slots` 的 processing 状态归零，使用有界 drain timeout。
3. 超时表示状态仍可能变化：跳过本次保存，记录原因，继续停止。
4. eligible、auto-save 且 slot 0 token 数达到最低阈值时进入 `SAVING`。
5. 调用 save 写入 scratch 中唯一、不可由客户端指定的 basename。
6. 校验响应、普通文件、字节数，计算 SHA-256，并同步文件。
7. 在新的 pending generation 中放置 payload；manifest 最后写入并同步。
8. 原子重命名 pending generation，随后原子更新 `latest.json`。
9. 新 generation 提交成功前不得覆盖或删除上一个可用 generation。
10. 运行容量清理，再调用现有进程终止逻辑。
11. save 的任何错误都不得阻止 `STOPPING`。

应用 UI 退出但实例由 runtime service 继续运行时不保存；runtime service 受控升级、手工停止和正常关闭实例时可以保存。异常进程退出只记录“未保存”，不尝试从已退出进程创建 checkpoint。

## 8. 存储布局与提交协议

所有路径位于应用数据目录，不使用模型目录、仓库目录或用户自定义 slot path：

```text
<app-data>/kv-checkpoints/
  fingerprints-v1.json
  <instance-id>/
    scratch/
      <manager-generated>.bin
    <fingerprint>/
      latest.json
      generations/
        <generation-id>/
          manifest.json
          slot-0.bin
        .pending-<generation-id>/
```

安全和原子性要求：

- Windows 目录 ACL 限制到当前用户；Unix 目录/文件分别使用 0700/0600。
- instance id、fingerprint、generation id 和 slot basename 都由管理器验证或生成，不接受分隔符、`..`、绝对路径或客户端输入。
- save/restore 前后都检查 scratch payload 是普通文件且不为 symlink/reparse point。
- scratch 只供当前受管实例与 coordinator 使用；持久化 generation 不直接暴露给 llama-server。
- pending 目录在 manifest 完成前不算有效 generation；启动时可清理遗留 pending。
- `latest.json` 损坏时允许扫描已经完整提交的 generation，不能把 pending 当作 fallback。
- 每实例默认容量上限为 8 GiB，UI 可调。单个新 generation 超过上限时放弃新 generation 并保留旧 generation。
- 提交后按 generation 最后使用时间清理，正在 restore、最新提交和本次 pending 均不得被清理线程删除。

checkpoint 包含由用户提示派生的模型状态，应按敏感本地数据处理。日志、事件和 UI 不显示 payload 内容、token id 或哈希前的路径。

## 9. Manifest v1

```json
{
  "schemaVersion": 1,
  "stateFormat": "llama.cpp-slot-state",
  "generationId": "uuid",
  "instanceId": "instance-id",
  "fingerprint": {
    "algorithm": "sha256",
    "digest": "hex",
    "modelSha256": "hex",
    "engineSha256": "hex",
    "engineVersion": "bNNNNN",
    "backend": "vulkan"
  },
  "createdAt": "RFC3339",
  "slots": [
    {
      "id": 0,
      "filename": "slot-0.bin",
      "promptTokens": 1745,
      "bytes": 14309796,
      "sha256": "hex"
    }
  ]
}
```

解析规则：

- 未知的未来 `schemaVersion` 必须拒绝，不能按 v1 猜测。
- v1 必须恰好包含 slot 0；重复 slot、负值、零字节、非法摘要和额外路径成分均拒绝。
- manifest 自身的稳定 JSON 表示参与 `latest.json` 指针校验。
- restore 前以磁盘实际字节重新计算 payload SHA-256。

## 10. 兼容性 fingerprint

fingerprint 使用稳定序列化后的 canonical object 计算 SHA-256。v1 采用严格命中，宁可 cold miss，不允许可疑 hit。

必须包含：

- fingerprint schema 和 checkpoint manifest schema。
- 模型完整内容 SHA-256；分片模型第一版不支持。
- llama-server 可执行文件完整内容 SHA-256、已探测版本和 backend。
- `ctx_size`、`parallel`、continuous batching、KV unified、KV K/V type、KV offload。
- flash attention、SWA full、context shift。
- RoPE scaling/base/scale/frequency 和全部 YaRN 参数。
- batch/ubatch、device、GPU layers、split mode、tensor split 和 main GPU。
- Jinja、chat template、template file 内容摘要、reasoning 格式相关启动设置。
- cache prompt、cache RAM、idle-slot cache 和影响 slot/prefix 行为的 server 参数。

第一版通过 eligibility 排除 LoRA、mmproj、draft model、lookup cache、router、manual command 和 custom args，避免无法完整规范化的状态。

明确排除 fingerprint：

- host、port、API key、CORS、TLS 文件路径、日志和 metrics 开关。
- UI 名称、alias、tags、自动启动和代理配置。
- 仅影响采样输出而不改变 KV 表示的请求级 sampling 默认值。

模型完整哈希按 canonical path、size、mtime 纳入本地缓存。首次启用时可在实例运行期间后台计算；哈希尚未完成只会使该次 restore/save 跳过，不得退回到“文件名 + 大小”身份。

## 11. 配置与 UI

在 `InstanceConfig` 增加向后兼容的嵌套配置，旧配置默认关闭：

```rust
pub struct KvCheckpointConfig {
    pub enabled: bool,
    pub auto_save: bool,
    pub auto_restore: bool,
    pub storage_limit_gib: u32,
    pub minimum_prompt_tokens: u32,
}
```

默认值：`enabled=false`、`auto_save=true`、`auto_restore=true`、`storage_limit_gib=8`、`minimum_prompt_tokens=256`。只有 `enabled` 打开时其余字段生效。

配置页显示：

- 实验性和支持范围说明。
- 开关、自动保存、自动恢复、容量和最低 token 阈值。
- 当前配置 eligibility；不支持时列出具体原因。
- 对 `parallel`、prompt cache、idle-slot cache、Cache RAM、slots 和按模型需要的 SWA full 等字段提供需要用户确认的修复动作；确认文案必须说明额外内存影响，保存过程本身不静默改写其他推理参数。
- 提醒 Harness 必须使用管理器代理才能获得 restore-before-first-request 保证。

实例页显示：

- 当前阶段、是否 routable。
- 最近保存/恢复时间、token 数、文件大小和耗时。
- cold fallback 或 skipped 的可读原因。
- “清除检查点”操作。第一版不开放任意文件选择和手工 restore。

清除操作只删除选定实例的已验证 checkpoint 根目录；正在运行、saving 或 restoring 时拒绝。删除前由后端重新校验目标在应用 checkpoint 根目录之内。

## 12. 可观测性与错误语义

新增稳定状态数据：

```text
phase: disabled | ineligible | starting | engine_healthy | restoring |
       ready | ready_cold | draining | saving | stopping | stopped
routable: boolean
lastOperation: none | save | restore | clear
lastOutcome: success | skipped | failed
reasonCode: stable machine-readable code
message: localized user-facing text
generationId, promptTokens, bytes, durationMs, updatedAt
```

reason code 至少覆盖：unsupported configuration、engine capability missing、fingerprint unavailable/mismatch、no checkpoint、below token threshold、busy timeout、checksum mismatch、manifest invalid、restore response invalid、slot state mismatch、storage limit、I/O error 和 HTTP timeout。

错误信息不得包含 API key、checkpoint 内容或未经脱敏的完整命令。checkpoint 错误不复用通用“server failed”语义；实例已经 cold-ready 时 UI 不能显示为启动失败。

## 13. 代理 readiness 契约

现有代理会独立探测所有 running targets。只修改 health monitor 不足以消除竞态，因此 `ProxyRuntimeSnapshot` 和 request resolution 都必须使用 checkpoint-filtered running map：

- checkpoint gate absent：保持现有行为。
- gate present 且 `routable=false`：不进入探测目标和请求候选。
- gate 变为 `routable=true`：下一 snapshot 开始探测和路由。
- draining 时先设置 false，再等待活动请求。

`/live` 只表示代理进程存在；`/ready` 只有至少一个 routable 且健康的 route 时成功。没有候选但存在 restoring 实例时，503 响应带可重试语义和 checkpoint phase，不泄露内部路径。

## 14. 测试策略

### 14.1 Rust 单元测试

- 旧配置反序列化默认关闭，新配置稳定 round trip。
- eligibility 支持矩阵逐项拒绝并返回稳定 reason code。
- fingerprint 对所有强兼容字段敏感，对 host/port/API key 等字段稳定。
- 模型/引擎内容改变使 fingerprint 改变；哈希缓存失效规则正确。
- manifest 严格解析、future schema、路径穿越、symlink/reparse、重复 slot、截断和摘要错误。
- pending/manifest-last 原子提交；每个故障注入点均保留上一个 generation。
- 容量清理、单 generation 超限、正在使用 generation 保护。
- slot client 的成功、错误 JSON、超时、token/byte 不一致和 erase fallback。
- 状态机只允许合法转换，旧 PID 事件不能覆盖新实例。

### 14.2 无模型集成测试

使用本地 Axum fake llama-server 和临时目录证明：

- health 成功后 restore 延迟期间代理仍 not ready。
- restore 完成后目标才进入路由。
- restore 失败先 erase，再 cold-ready。
- stop 先 gate/drain、后 save、manifest commit、最后 terminate。
- busy timeout 跳过 save 但仍停止。
- runtime service 与 GUI 直管路径产生相同 manifest 和状态。
- runtime state/schema 从旧版本恢复时新增字段默认安全。
- 异常退出不产生新 generation。

### 14.3 前端测试

- 关闭、eligible、ineligible、restoring、ready、ready-cold、saving 和失败状态。
- eligibility 警告不被保存配置流程静默清除。
- 数值边界、单位和旧配置默认值。
- 清除确认和运行中拒绝。
- 中文/英文文案、窄屏和明暗主题。

### 14.4 真实 llama-server 验收

- 使用项目支持窗口内的真实 llama-server 和标准文本 GGUF。
- 用 OpenAI-compatible 请求创建足够长的稳定前缀，记录 `prompt_n/cache_n` 或等价 prompt progress。
- 受控停止并确认 generation 文件、manifest 和 SHA-256。
- 重启后通过管理器代理等待 readiness，再发送同一前缀。
- 证明 restore 后的实际 cache hit 和 TTFT 改善。
- 分别执行 config mismatch、payload corruption、restore timeout 和 direct-port caveat 用例。
- 最后用实际 DeepSeek Harness 路由重复同一上下文，确认 Harness 首请求发生在代理 ready 之后并命中缓存。

真实性能验收是合并前人工/本机证据；CI 使用确定性 fake server 验证生命周期和故障安全。

### 14.5 实施期真实验收结论

2026-08-29 使用真实 HIP `llama-server`、标准单文件 Gemma 4 GGUF 和 DeepSeek Harness 完成跨进程验证：

- 相同长前缀冷启动的 prompt evaluation 约为 5.63 秒；同进程复用约为 34 毫秒。
- 默认 sliding-window cache 虽然 slot save/restore 返回成功且 payload round trip 一致，但重启后 `cache_n = 0`，证明 HTTP 200 不能作为真实复用证据。
- 启用 `swa_full` 后，重启恢复的相同 OpenAI 请求得到 `cache_n = 7164`、prompt evaluation 约 34 毫秒。
- DeepSeek Harness 新会话先发标题请求，再发主上下文。启用 idle-slot cache 和 Cache RAM 后，第二个新会话的主请求得到 `n_past = 7418`，prompt evaluation 从约 5.66 秒降至约 32 毫秒。
- 恢复期间代理返回可重试 503；fingerprint mismatch 和损坏副本均在 restore 前退化为可路由冷启动。确定性故障注入另外覆盖精确 checksum mismatch、restore timeout、partial erase 和 manifest-last 原子性。

因此实施将 `swa_full`（仅 SWA 模型）、idle-slot cache 和非零 Cache RAM 从调优建议提升为资格条件，并把相关状态纳入严格 fingerprint。完整引擎/模型摘要、generation 校验和 Harness 会话证据保留在实现 PR 的验收记录中，不把用户本机路径或 prompt-derived payload 提交到仓库。

## 15. 发布与回滚

- 功能默认关闭并标记实验性，不改变现有实例命令和代理行为。
- 新配置字段全部有 serde/TypeScript 默认值；旧 runtime state 可读取。
- 禁用功能后不自动删除已有 checkpoint，用户可显式清除。
- 代码回滚后 checkpoint 目录只是未使用的本地数据，不影响旧版本启动。
- 实现 PR 不修改 llama.cpp 参数兼容基线，除非真实上游支持窗口验证要求同步；该变化需单独说明。

## 16. 后续扩展门槛

多 slot、multimodal、hybrid/recurrent、跨引擎版本或 session-pinned checkpoint 只有在满足以下条件后才能进入后续设计：

1. 官方 API 对目标状态提供稳定语义。
2. fingerprint 能表达新增状态。
3. 有跨重启真实 cache-hit 测试，而不只是 restore 返回值。
4. 有并发 slot 到 Harness 会话的确定映射。
5. 当前 v1 检查点仍可安全忽略或迁移，不允许静默解释为新格式。
