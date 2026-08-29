# KV / Prefill Cache Checkpoint v2 设计增补

**状态：** 实施基线
**日期：** 2026-08-29
**基于：** `2026-08-29-kv-cache-checkpoint-design.md`

## 1. 目标

v2 在不削弱 v1 fail-open、代理路由门和敏感数据边界的前提下，解决三个已由真实目标模型暴露的问题：

1. `--spec-type` 是逗号分隔的有序候选集合，而当前 UI 只能选一个值。
2. 完整分片 GGUF 被当作若干独立文件，导致第一分片也被标成 shard，并且 checkpoint 只哈希启动路径指向的一个文件。
3. 模型仓库缓存、分片间元数据或现代大词表字符串数组处理不完整时，第一分片已有的 `general.architecture` 仍可能在资格检查中显示未知。

目标验收模型为 Qwen3.8-Flash-Next 的三分片 GGUF，架构元数据为 `qwen4exp`。最终日常 DeepSeek Harness 应用验收由用户完成；本阶段必须先完成可复现的技术验收。

## 2. 非目标与硬边界

- 不把模型卡宣称的原生 N-gram Embedding/PLE 等同于 llama.cpp 的 `ngram-mod`。
- 不根据模型名称推断 MTP。只有 GGUF 元数据明确包含可用 next-token/MTP 层，或配置了外部 draft 模型时，才把 MTP 视为模型能力。
- 不声称 `ngram-mod,draft-mtp` 是并行 ensemble。当前 llama.cpp 按固定优先级逐个尝试实现，先产生草稿的实现结束本轮候选选择。
- 不为 draft/MTP checkpoint 伪造兼容性。当前 slots API 只保存目标 context 的 sequence state；独立 draft context、MTP pending hidden state 和 sampler state不在 payload 中。
- 不在本阶段放宽多 slot、LoRA、multimodal prompt 或自定义启动参数。
- 功能保持默认关闭；任何不确定、缺失或验证失败都只关闭本次 checkpoint，并继续冷启动。

## 3. `--spec-type` 组合语义

### 3.1 能力探测

引擎探测除 `supportedFlags` 外，新增向后兼容的 `speculativeTypes`：从所选可执行文件的 `--help` 中解析 `--spec-type` 声明的逗号分隔候选值。探测不到时 UI 使用与当前受支持 llama.cpp 基线一致的内置候选表，但明确把它视为 fallback，而不是引擎证明。

### 3.2 配置规范化

配置仍持久化为字符串，以兼容旧实例与 runtime 协议。统一解析器执行：

- ASCII 小写、去除首尾空白；
- 逗号切分、去重；
- `none` 只能单独存在；
- 已知类型按 llama.cpp 实际运行优先级规范化；
- 未知但已保存的 token 不静默删除，UI 显示并交由能力校验阻止不受支持的启动。

当前基线运行优先级为：

1. `ngram-simple`
2. `ngram-map-k`
3. `ngram-map-k4v`
4. `ngram-mod`
5. `ngram-cache`
6. `draft-simple`
7. `draft-eagle3`
8. `draft-mtp`
9. `draft-dflash`
10. `draft-dspark`

用户选择顺序不是运行优先级。UI 必须在组合控件旁显示这一点，命令预览只发出一个规范化的 `--spec-type a,b` 参数。

### 3.3 依赖与校验

所有前端依赖、告警、参数激活和监控判断必须使用 token set，而不是字符串相等或 substring：

- 任一 `draft-*` 类型触发 draft 依赖检查；
- `draft-mtp` 仅在 GGUF MTP 元数据或外部 draft 模型存在时通过模型能力检查；
- `ngram-cache` 才激活静态/动态 lookup cache 路径；
- `none` 与其他 token 的组合被规范化为显式关闭；
- 后端继续原样支持逗号组合，并有命令生成回归测试。

## 4. 分片 GGUF 作为一个逻辑模型

### 4.1 分组规则

仅把同一规范化父目录内、文件名匹配 `BASE-00001-of-000NN.gguf`、总数一致且索引完整唯一的文件视为一个分片集。

- 第一分片是逻辑模型入口，`is_shard = false`，可被模型计数和选择器选择。
- 后续分片 `is_shard = true`，仍在模型仓库中可见，但不作为独立模型入口。
- 第一分片缺少的架构、上下文、量化和 capability 元数据，可从同组已成功解析的分片补齐；不会跨目录或跨 BASE 合并。
- tokenizer 等无关大字符串数组采用有项目数上限的流式跳过，不分配数组内容；general.tags 等需要保留的数据继续使用更严格的数量与字节预算。
- 不完整、重复索引或总数冲突的集合不形成逻辑模型，并在 checkpoint 阶段返回精确的 artifact-set 不完整原因。

### 4.2 全分片内容指纹

启动路径仍指向第一分片，checkpoint 模型摘要改为：

- 单文件：保持文件内容 SHA-256；
- 完整分片：按 shard index 排序，对每个文件使用已有 size/mtime 缓存的完整内容 SHA-256，再对 `format version + shard count + index + per-shard digest` 做二次 SHA-256。

路径和盘符不进入摘要，因此内容完全相同的模型可搬迁；任一分片内容变化、缺失、重复或总数变化都会 miss。manifest 字段继续叫 `modelSha256`，但 fingerprint schema 升级，v1 generation 不会被静默解释为 v2。

## 5. Checkpoint 支持矩阵

| 配置 | v2 行为 | 原因 |
| --- | --- | --- |
| 无推测解码、单文件文本模型 | 支持 | v1 已验收 |
| 无推测解码、完整分片文本模型 | 实验支持 | 全分片摘要后状态仍属于同一 target context |
| 仅含 `ngram-*` 的组合 | 实验支持 | 恢复后 llama.cpp 使用 slot prompt token 调用 speculative `begin`，n-gram 状态可重建 |
| 任一 `draft-*`，含 `draft-mtp` | 不支持 checkpoint；冷启动 | slots payload 不包含独立 draft context/MTP 中间状态 |
| `spec-default` | 不支持 checkpoint；冷启动 | 自动选择结果不够明确，fingerprint 无法证明实际实现 |
| 外部 lookup cache 路径 | 本阶段不支持 checkpoint；冷启动 | 外部可变状态尚未纳入内容指纹 |
| mmproj 或 media prompt | 不支持 checkpoint；冷启动 | image embedding/token state 尚无跨重启证明 |
| 不完整分片集 | 不支持 checkpoint；冷启动 | 无法证明模型内容身份 |

已知 hybrid/recurrent 架构仍需现有状态序列化能力和真实 cross-restart cache-hit 证明。`qwen4exp` 不靠名称放行；B10679 源码与实机均证明它使用 hybrid recurrent memory，slot round trip 后相同前缀没有 target cache hit，因此作为明确反例进入 blocklist。

## 6. 指纹与兼容性

fingerprint schema 升级并新增规范化 `specType`。即使 n-gram 状态可由 prompt 重建，配置变化也应产生可解释的安全 miss。现有 engine digest/version/backend、chat template 内容、KV/SWA/RoPE/设备和批处理字段继续参与指纹。

分片集合解析和哈希必须在 blocking worker 中执行；资格预览只做有界、只读的集合完整性检查，不读取全部模型内容。任何哈希期间的 size/mtime 变化都返回 `fingerprint_unavailable`。

## 7. 目标模型技术验收

固定使用同一个 B10679 ROCm 引擎、同一三分片 Qwen3.8-Flash-Next、`parallel=1`、prompt cache、idle-slot cache、非零 Cache RAM、slots API，并记录脱敏的 engine/model aggregate digest。

执行四组矩阵：

| 组 | Checkpoint | `--spec-type` | 必须证明 |
| --- | --- | --- | --- |
| A | 关 | none | 冷启动基线可生成 |
| B | 关 | `ngram-mod` | 引擎接受参数且可生成 |
| C | 开 | none | 受控保存、跨进程恢复、相同长前缀真实 cache hit |
| D | 开 | `ngram-mod` | 保存/恢复成功，恢复后 n-gram 初始化不破坏 target cache hit，输出正常 |

每组记录：启动结果、资格原因、slot save/restore 状态、prompt tokens、`cache_n`/`n_past`、prompt evaluation、首 token 延迟和生成 token/s。C/D 必须用进程 PID 变化证明跨进程；restore HTTP 200 单独不算通过。

另做失败验收：缺一个分片、改一个分片摘要材料、`ngram-mod,draft-mtp`、`spec-default` 和未知 spec token 均不得 restore 错误状态，实例必须冷启动可路由。

### 7.1 2026-08-29 实施验收记录

- 精确 GGUF 元数据：`qwen4exp`、context 262144、IQ4_XS、三分片，无内置 MTP 元数据；原“架构未知”由 tokenizer 大字符串数组超过旧的 skip 项目预算造成，流式跳过修复后可稳定读取。
- 精确引擎：llama.cpp 0.3.0 build 10679；`--help` 报告逗号组合及 `ngram-mod`/`draft-mtp` 等全部候选。
- 全模型聚合摘要：三份逐文件 SHA-256 按 v2 规则聚合为 `3d57027904163eece23f0dd0f3f784eb1a279b50dbb0aee109c2e5aefed572d6`；不记录本机路径。
- A（none，checkpoint 关）：4805-token cold prefill 约 15112 ms，正常生成；同 PID 再请求命中 `cache_n=4801`，prefill 约 135 ms。
- B（ngram-mod，checkpoint 关）：引擎正常启动，`/slots` 报告 `speculative=true` 与 `none,ngram-mod`，完成 24-token 生成。
- C（none，跨 PID）：PID 16776 保存 4808 token/280762008 bytes，PID 18004 restore 成功，但相同前缀 `cache_n=0`、prefill 约 14540 ms，失败。
- D（ngram-mod，跨 PID）：PID 11024 保存 4831 token/281540420 bytes，PID 13412 restore 成功，但相同前缀 `cache_n=0`、prefill 约 14538 ms，失败。
- `--swa-full` 复验：B10679 明确报告该模型不支持 SWA full 并自动关闭。llama.cpp `qwen4exp.cpp` 将大多数层实现为 SSM/recurrent memory，因此 SWA 不能补救。

结论：目标模型支持普通 KV/prompt cache 与 `ngram-mod`，不支持当前 slots API 下的持久 checkpoint 收益。产品验收以提前识别 `qwen4exp`、不执行大模型哈希/restore、保持可路由冷启动为通过；不得把 HTTP restore 成功包装成性能收益。

## 8. 交付门槛

- Rust 单元/集成测试覆盖 shard grouping、完整性、aggregate digest、fingerprint miss、n-gram allow、draft deny 和旧 serde 默认值。
- TypeScript/浏览器测试覆盖多选、规范化、动态候选、未知 token 和实际命令预览。
- `cargo fmt --check`、完整 `cargo test --locked`、clippy、`npm run check:release`、`npm run build`、`git diff --check` 全部通过。
- 通过主题分支和 PR，等待 `quality`、`build-windows`、`build-macos`、`build-linux`、`build-linux-arm64`，合并后 fresh-master 复验。
- 不发布 tag/release；用户完成最终 DeepSeek Harness 日常应用测试后再决定是否扩大支持范围。
