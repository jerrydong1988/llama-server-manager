# KV / Prefill Cache Checkpoint v2 实施与验收计划

**设计：** `docs/superpowers/specs/2026-08-29-kv-cache-checkpoint-v2-design.md`

**实施记录（2026-08-29）：** 核心实现与目标模型技术矩阵已完成。Qwen3.8-Flash-Next 的普通 prompt cache 和 `ngram-mod` 通过，但 `qwen4exp` 跨进程 restore 后两组均为 `cache_n=0`；因此交付行为是架构识别后提前回退冷启动，而不是对该模型启用持久检查点。最终 DeepSeek Harness 日常应用确认仍由用户在合并版本上执行。

## Task 1：推测解码组合契约

- 新增共享的 spec token 解析、规范化、集合判断与固定运行优先级。
- 引擎 `--help` 探测并持久化 `speculativeTypes`，旧 inventory/runtime JSON 缺字段时默认空列表。
- 配置页把单选改成多选，保留空值、`none` 和未知旧值的可逆语义。
- 修正 validator、parameter catalog、active params、监控和命令生成测试。

## Task 2：逻辑分片模型与全内容指纹

- 按父目录、BASE、index、total 分组；第一分片作为入口，后续分片作为 continuation。
- 补齐组内元数据传播和不完整/冲突集合测试。
- 新增有界 artifact resolver；完整内容哈希在 blocking worker 中执行。
- 让 checkpoint `modelSha256` 表示完整模型 artifact set，升级 fingerprint schema 并验证旧 generation 安全 miss。

## Task 3：v2 资格矩阵

- 允许完整分片文本模型。
- 允许仅由 `ngram-*` 组成的 spec 集合。
- 拒绝任何 `draft-*`、`spec-default`、外部 lookup cache、multimodal 和不完整模型集合，并保留冷启动。
- 将规范化 spec 集纳入 fingerprint；补齐 UI 中英文资格说明。

## Task 4：自动化验证

- Rust：eligibility、artifact resolver、aggregate digest、hash cache、fingerprint、serde、命令组合。
- TypeScript：spec parser、dependency、validator、monitoring、UI multi-select。
- Browser：选择两个 spec 类型、保存/重载、命令预览、`none` 互斥、引擎动态候选。
- 运行聚焦测试后执行完整 Rust、npm release gate 和 production build。

## Task 5：Qwen3.8-Flash-Next 技术验收

- 先确认没有需要保留的活动模型会话；不强制终止用户工作负载。
- 对目标三分片模型运行 A/B/C/D 矩阵并证明跨 PID 的真实 cache hit。
- 如果真实 cache hit 不成立，以测量结果收紧架构支持矩阵，并验证产品在哈希/restore 前 fail-open cold；不得为了满足预期而放宽。
- 验证缺分片、draft 组合和 fingerprint mismatch 均 fail-open cold。
- 恢复用户原配置，清除只属于本次测试的 checkpoint、日志和临时配置；不删除模型、引擎或用户数据。

## Task 6：文档与交付

- 更新公开支持矩阵、Qwen 限制、组合语义和排障说明。
- 做编码、隐私、路径边界、dirty-worktree 和生成物审计。
- 逻辑提交到 `codex/kv-checkpoint-v2`，push 并创建 targeting `master` 的 PR。
- 等待并复核全部必需 CI，解决 review conversation，通过 GitHub 合并。
- 同步 fresh `master`，核对合并提交、CI 和干净工作树，再交给用户做 DeepSeek Harness 实际应用测试。
