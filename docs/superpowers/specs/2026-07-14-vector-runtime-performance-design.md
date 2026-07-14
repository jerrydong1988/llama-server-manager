# 向量模型运行性能监控设计

日期：2026-07-14

状态：第一阶段已完成并通过验证

## 1. 背景

当前性能监控以文本生成工作负载为中心。实时主指标、趋势、历史摘要和诊断主要使用生成 tokens/s、生成 token 数、生成阶段耗时、KV/slot 和推测解码数据。

Embedding 与 Reranker 不进入文本生成阶段。当前 `llama-server` 的 Prometheus `/metrics` 端点虽然提供 prompt、generation、队列和 decode 指标，但没有向量项、重排文档、向量请求延迟等专用指标；同时，当前上游向量任务完成路径不会按生成请求的方式提交 prompt/generation 性能汇总。因此，运行中的向量实例只能稳定显示 CPU、内存、GPU、显存等资源指标，核心业务性能通常表现为 0 或无数据。

本设计在不修改用户所选 `llama-server` 二进制、不强制用户通过代理调用实例的前提下，为 Embedding 和 Reranker 增加符合其工作负载语义的运行性能分析。

## 2. 目标

1. 自动区分 `inference`、`embedding` 和 `reranker` 遥测会话。
2. 对直连实例端口的向量流量采集输入 token、处理项、任务耗时和活动趋势。
3. 对经过内置实例路由代理的向量流量补充 HTTP 请求数量、请求耗时、状态码和失败率。
4. 保留 CPU、内存、GPU、显存、队列与并发等现有通用指标。
5. 性能页按工作负载切换指标、趋势、历史摘要、基线和诊断，不再对向量模型展示误导性的生成指标。
6. 保持现有生成模型监控行为和历史数据兼容。
7. 不持久化输入文本、查询、文档内容或输出向量。

## 3. 非目标

1. 本阶段不评估检索或排序质量，不实现 Recall@K、MRR、NDCG 等指标。
2. 本阶段不内置固定测试集或主动压测工具。
3. 不修改、替换或分发定制版 `llama-server`。
4. 不承诺从上游日志重建原始 HTTP 批次边界。直连流量使用任务项口径，代理流量使用 HTTP 请求口径。
5. 不将向量模型的处理项速度与文本生成 tokens/s 放在同一历史基线中比较。

## 4. 方案比较与决策

### 4.1 只使用 `/metrics`

优点是改动小、格式稳定。缺点是当前上游没有向量项、文档项和请求延迟指标，短请求也可能在轮询间隔内完成。该方案无法满足核心目标。

### 4.2 只监控内置代理

优点是 HTTP 边界、状态码和耗时准确。缺点是用户可以直接调用实例端口，Reranker 代理别名目前也不完整。该方案会漏掉常见直连流量。

### 4.3 混合采集

采用以下三层数据源：

- `/metrics` 和进程采样：资源、排队、slot 与 decode 活动。
- `llama-server` 日志：覆盖直连与代理流量的向量任务项、输入 token 和任务耗时。
- 内置代理：补充经过代理的 HTTP 请求数量、项目数量、状态码、请求耗时和失败率。

本设计选择混合采集。不同来源承担不同指标口径，不对同一数值相加，避免重复统计。

## 5. 指标口径

### 5.1 Embedding

- 输入吞吐：最近 60 秒工作窗口内完成的输入 token 数 / 窗口秒数。
- 向量项吞吐：最近 60 秒工作窗口内完成的向量任务项数 / 窗口秒数。
- 任务耗时 P50/P95：日志中单个向量任务项从启动到释放的耗时分位数。
- HTTP 请求耗时 P50/P95：仅使用代理请求事件计算。
- 处理总量：已完成向量任务项数量。
- HTTP 请求数、成功率和失败率：仅在存在代理事件时显示。

### 5.2 Reranker

- 输入吞吐：最近 60 秒工作窗口内完成的 query-document 输入 token 数 / 窗口秒数。
- 文档项吞吐：最近 60 秒工作窗口内完成的重排文档任务项数 / 窗口秒数。
- 任务耗时 P50/P95：单个 query-document 任务项耗时分位数。
- HTTP 请求耗时 P50/P95：仅使用代理请求事件计算。
- 处理总量：已完成的重排文档任务项数量。
- HTTP 请求数、成功率和失败率：仅在存在代理事件时显示。

### 5.3 通用指标

- 实例 CPU、内存、GPU、显存和运行时间。
- 系统 CPU 与系统内存。
- 处理中请求、延迟队列和 busy slot。
- `llama_decode()` 累计调用数和最大观测 token 数。

`llamacpp:n_decode_total` 不再命名为“请求数”。它是 decode 调用计数，不能等同于 HTTP 请求数量。

## 6. 数据模型

### 6.1 运行会话

在 `run_sessions` 增加：

```sql
workload TEXT NOT NULL DEFAULT 'inference'
```

新会话从已经规范化的实例配置确定工作负载：

- `reranking = true` 为 `reranker`。
- 否则 `embedding = true` 为 `embedding`。
- 其余为 `inference`。

历史会话按 `command_line` 回填：优先匹配 `--reranking`，其次匹配 `--embedding`，其余保持 `inference`。会话工作负载一旦创建不随实例后续配置修改。

### 6.2 向量活动事件

新增 `vector_activity_events`：

```sql
CREATE TABLE vector_activity_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id      TEXT NOT NULL,
    source          TEXT NOT NULL,
    source_event_id INTEGER NOT NULL,
    workload        TEXT NOT NULL,
    endpoint        TEXT,
    started_at      INTEGER NOT NULL,
    completed_at    INTEGER NOT NULL,
    duration_ms     REAL NOT NULL,
    item_count      INTEGER NOT NULL DEFAULT 1,
    input_tokens    INTEGER,
    http_status     INTEGER,
    error_text      TEXT,
    UNIQUE(session_id, source, source_event_id),
    FOREIGN KEY(session_id) REFERENCES run_sessions(id) ON DELETE CASCADE
);
```

`source` 取值为 `log` 或 `proxy`：

- `log` 事件用于任务项吞吐、输入 token、任务耗时和处理总量。
- `proxy` 事件用于 HTTP 请求数、项目数量、请求耗时、状态码和失败率。

代理流量同时产生两类事件时，查询层按指标选择来源，不合并为同一计数。

### 6.3 通用采样

在通用指标采样中增加明确命名的 `decode_calls_total` 与 `max_tokens_observed`。旧 `requests_total` 字段继续保留用于数据库兼容，但不再作为请求数展示或参与新分析。

## 7. 采集流程

### 7.1 会话启动

实例启动使用规范化后的配置创建遥测会话，并将工作负载传给日志采集线程。应用重启后恢复运行实例时，从持久化实例配置或会话记录恢复同一工作负载口径。

### 7.2 日志事件

扩展现有 `PerfParser`：

1. 识别任务启动并保存 task id、slot id 和本地开始时间。
2. 识别 `stop processing: n_tokens = N`，保存输入 token 数和完成时间。
3. 对向量会话写入一条 `source=log` 事件，`item_count=1`。
4. 对文本生成会话继续执行现有 prompt/generation 解析，不改变已有记录。
5. 重放历史日志时沿用 `(session_id, source, source_event_id)` 唯一约束，避免重复写入。

批量 Embedding 的每个子任务代表一个向量项；Reranker 的每个子任务代表一个 query-document 文档项。日志事件不冒充 HTTP 请求数量。

### 7.3 代理事件

为实例路由代理补齐以下向量端点：

- `/embedding`
- `/embeddings`
- `/v1/embeddings`
- `/rerank`
- `/reranking`
- `/v1/rerank`
- `/v1/reranking`

请求体在转发前仅用于计算 `item_count`：Embedding 统计 input 项，Reranker 统计 documents 项。原始文本不写入日志或数据库。

代理响应流结束或失败时写入 `source=proxy` 事件。请求耗时以完整响应消费结束为完成点，状态码和错误信息沿用现有遥测脱敏边界。

### 7.4 `/metrics` 与资源采样

保留现有 5 秒资源采样。新增解析：

- `llamacpp:n_decode_total`
- `llamacpp:n_tokens_max`

向量业务吞吐不依赖 `prompt_tokens_seconds` 或 `predicted_tokens_seconds`，避免上游向量路径不更新这些值时显示错误结果。

## 8. 聚合与分析

### 8.1 趋势

向量趋势使用固定时间桶：

- `input_tokens / bucket_seconds`
- `completed_items / bucket_seconds`

空闲时间桶显示 0，非空时间桶反映实际完成吞吐。并发任务在同一时间桶内自然累加，因此不会按单任务耗时错误地低估服务总吞吐。

### 8.2 分位数

Rust 查询层按完成时间读取会话事件，对有效、有限且非负的耗时排序后计算 P50/P95。日志任务耗时与代理 HTTP 耗时分别计算和标注。

### 8.3 历史基线

历史基线只比较：

- 相同模型路径或模型名。
- 相同工作负载。
- 相同后端类型。

Embedding 比较输入 tokens/s、向量项/s 和任务 P95；Reranker 比较输入 tokens/s、文档项/s 和任务 P95。生成模型继续使用原有生成吞吐基线。

## 9. 界面设计

性能页继续使用现有三栏结构和通用资源卡，不新增独立页面。

### 9.1 工作负载标识

实例标题与历史会话增加 `生成`、`Embedding` 或 `Reranker` 标识。历史会话使用会话自身的工作负载，不读取实例当前配置。

### 9.2 主指标

- 生成模型：保持当前生成 tokens/s 与队列压力。
- Embedding：当前输入 tokens/s、向量项/s、任务 P95 与队列压力。
- Reranker：当前输入 tokens/s、文档项/s、任务 P95 与队列压力。

### 9.3 趋势与摘要

向量会话提供输入吞吐和处理项吞吐两个趋势模式。会话摘要显示处理总量、平均输入速度、任务 P50/P95；有代理样本时增加 HTTP 请求数、HTTP P50/P95 和失败率。

### 9.4 活动任务

向量活动任务显示 slot、运行耗时和工作负载类型，不显示已生成 token、生成速度或推测解码接受率。

### 9.5 诊断

向量会话保留以下诊断：

- 资源或显存压力。
- 队列积压。
- GPU 利用率偏低且 CPU 压力高。
- 输入吞吐低于同模型历史基线。
- P95 任务耗时明显回退。
- 指标来源不完整。

向量会话不运行生成阶段、长对话上下文、KV 缓存和推测解码诊断。

## 10. 不完整数据与错误处理

1. `/metrics` 不可用时继续保存进程和系统资源指标。
2. 日志格式不兼容时不显示伪造的 0 吞吐；界面显示“向量活动指标不可用”，资源和队列指标继续工作。
3. 代理事件存在但日志事件缺失时，显示 HTTP 请求指标，并将输入 token/任务项吞吐标记为不可用。
4. 日志事件存在但用户直连实例时，显示任务项指标，不显示 HTTP 失败率或伪造 HTTP 请求数。
5. 数据库单次事件写入失败不终止实例，不影响后续事件采集；错误写入服务器日志并保持采集线程存活。
6. 非法耗时、负值和非有限浮点数不进入分位数或吞吐计算。

## 11. 隐私与存储

- 不保存请求正文、查询、文档、token 序列或输出向量。
- 仅保存类型、计数、时间、状态码和脱敏错误信息。
- `vector_activity_events` 与会话使用外键级联删除，沿用现有遥测保留期清理。
- 为 `(session_id, completed_at)` 和 `(session_id, source, completed_at)` 建立索引，避免历史趋势查询退化。

## 12. 兼容性

- SQLite 迁移必须幂等，旧数据库可直接升级，重复启动不会重复迁移或回填。
- 生成会话默认值为 `inference`，现有 TypeScript/Rust 序列化字段保持向后兼容。
- 日志解析同时覆盖项目现有测试格式和当前上游 `llama-server` 格式。
- 不要求特定 GPU 后端，CPU、CUDA、ROCm、Vulkan 和 Metal 共享相同业务事件口径。
- 对不支持新增代理别名的旧客户端没有行为变化；现有代理端点继续工作。

## 13. 测试策略

### 13.1 Rust 单元测试

- Embedding 单项和批量子任务日志。
- Reranker 多文档子任务日志。
- 文本生成日志回归。
- 日志重放幂等与 task id 去重。
- 数据库新建、旧库迁移、重复迁移和级联清理。
- 固定时间桶、并发完成吞吐、P50/P95 和非法样本过滤。
- 代理端点识别、请求 item_count、成功、失败和响应中断。
- 日志与代理双来源不重复计数。

### 13.2 前端测试

- 三种工作负载的指标选择和标签。
- 向量会话不显示生成与推测解码指标。
- 仅日志、仅代理、完整数据和无业务数据四种状态。
- 会话切换、实例更换模型后历史工作负载不漂移。
- 相同模型和工作负载的历史基线过滤。

### 13.3 集成与发布检查

- 使用模拟 `/metrics`、代表性日志和代理请求完成无模型集成测试。
- 使用真实 Embedding 与 Reranker 实例分别验证直连和代理调用。
- 验证停止、重启、应用重启恢复和遥测清理。
- 运行 TypeScript 检查、前端构建、Rust 测试、格式检查、Clippy 严格警告检查和发布检查。

## 14. 实施边界

第一阶段完成标准：

1. Embedding 和 Reranker 在真实请求后能显示非生成语义的吞吐、耗时和处理总量。
2. 直连流量至少具备任务项和输入 token 指标。
3. 代理流量具备精确 HTTP 请求耗时、数量和失败率。
4. 生成模型性能页无功能回退。
5. 旧遥测数据库可无损升级。
6. 缺失数据时界面明确表达来源和可用性，不以 0 冒充测量结果。

向量质量评测方向暂缓，不属于本次提交范围；本阶段不创建相关设计或实现。
