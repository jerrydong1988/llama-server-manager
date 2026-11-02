# 两轮人工 API 测试的处理决定

适用范围：Gufo 接入分支，基线 Manager `779fc0c`、Gufo 0.28.1
`408a04357827`。依据 2026-11-02 Hermes 测试、DeepSeek Harness 的
`guf-api-audit-20261102/reports/REPORT.md`、原始响应、当前源码及对照复测。
外部报告保留原样；本文件记录项目采纳的结论。Gufo 程序和原部署不变。

## 路由处理决定

**目标满载保留快速拒绝，不新增目标级队列。** 引擎执行槽位、全局/Key
准入和目标容量是不同层次。当前 `queue_timeout_ms` 只控制前者中的准入等待。
在读取完整请求、取得全局许可后再排目标队列，会占用全局额度和请求体内存，
放大单个热门模型对其他模型的影响；引擎还可能存在自己的队列。
本轮保持现有可预测的容量边界，调用方使用有上限、有抖动的退避重试，
不要无限重试，也不要仅为容纳等待者增加 Gufo sessions。

修复后目标选择与槽位预留在同一锁内完成，消除两步竞争导致的偶发 429。
忙碌目标仍可以切换到其他符合条件的备用目标。

| 情况 | HTTP / error.code | 处理 |
| --- | --- | --- |
| 健康匹配目标全部满载 | 503 / `route_capacity` | `Retry-After: 1`，提示客户端有界重试 |
| 已配置公开路由的目标停机、加载中或不健康 | 503 / `route_unavailable` | `Retry-After: 1`；不承诺一秒内就绪 |
| 未知公开模型 | 404 / `model_not_found` | 检查模型名，不静默回退 |
| model 缺失/null，且无可解析的默认目标 | 400 / `model_required` | 修改请求或配置默认实例 |
| model 是错误类型或空字符串 | 400 / `invalid_request_error` | 不再当作省略 model |
| 运行模型与向量端点任务类型不匹配 | 400 / `model_endpoint_mismatch` | 选用匹配的 embedding/rerank 模型 |
| 全局/Key 准入超时或限流 | 429，保留现有行为 | 与目标忙碌分开 |
| 路径不存在 / 方法不支持 | 404 / `endpoint_not_found`；405 / `method_not_allowed` | 返回相应协议 JSON 信封 |

上述路由错误码在 OpenAI 和 Anthropic 信封中一致，并与请求账本中的
诊断代码一致。保留已配置默认实例和既有单个隐式目标回退规则。
严格路由仍只接受公开别名；不通过错误分类暴露内部实例 ID、私有名称或路径。
模型列表继续只展示运行中的模型，管理界面负责展示停机配置。

拒绝计数统一在 HTTP 响应出口统计：已知推理/计数端点的 POST 请求，
在尚未转发生成/计数请求时返回的本地 4xx/5xx，每次只计一次。
包括认证、权限、限流、准入、请求体、模型路由、上下文预检和配额拒绝。
上游错误、成功请求、客户端取消不算本地拒绝。
`requests_total` 仍表示通过全局准入的请求，不是所有到达请求；
因此不能假定它恒等于 completed + rejected。
`completed_total` 和耗时直方图保留生命周期结束的原义，不改名为成功率；
需要成功率或取消率时使用账本 `forwarded/outcome`，业务性能视图已过滤成功请求。

## 每项发现的最终归类

| 来源 | 决定 | 说明 |
| --- | --- | --- |
| Hermes 1/2；D-01；GUFO-04 | 保留协议行为，补说明和对照测试 | 思考会消耗输出预算；空正文且 length/incomplete/max_tokens 是截断，不是服务死循环。不自动关思考，不把 200 截断改成 5xx。 |
| Hermes 3 | 保留缓存配置 | `capture_refused byte_capacity` 表示快照容量压力，不等于 OOM。auto 模式下单改 cache_ram_gib 无效；不盲目扩容。 |
| Hermes 4 | 保留单执行会话 | sessions、路由准入和排队不是同一个参数；total_slots 是观测值。 |
| Hermes 5；D-03；GUFO-02 | 保留目标快速拒绝 | 采用上面的显式路由策略，关闭本轮的“必须加目标队列”待办。 |
| Hermes 7；D-05；GUFO-01 | 修复 | 所有本地推理拒绝统一计数，避免漏记/重复。 |
| D-04；GUFO-03 | 修复 | 原子选择/占用，明确容量与不可用代码、状态及重试提示。 |
| D-06 | 修复 | 缺失模型不再误报 503；保留有效默认目标。SDK 默认有界重试，不是报告所说的无限重试。 |
| D-09 | 修复错误说明 | 无匹配任务目标时尚未转发，不能说引擎 501 被丢弃；改为明确的模型/端点不匹配。 |
| D-10 | 保留发现范围，完善推理错误 | 不把停机模型混入可调用模型列表；公开已配置离线路由的推理请求返回 route_unavailable。 |
| D-16 | 修复 | 移除上游 CORS 头，再按 Manager 白名单生成；原有 Origin 拒绝仍有效，并非已证实的绕过。 |
| D-17 | 修复 | 未知路径与不支持的方法返回 JSON 错误信封。 |
| D-02 | 保留 Gufo 基线语义，明确兼容边界 | stop 应用于原始生成文本，会命中 `</think>\n\n`。不在网关删除 stop、剥离思考或续生成；需要短文本时显式关闭思考，避免把空行作为通用停止词。 |
| Hermes 6 | 误判，不改代码 | 无工具的 Responses 请求返回 parallel_tool_calls=false 不代表全局不支持；带工具的受控测试 true 可多调用，false 单调用。 |
| Hermes 8；GUFO-04 | 文档/契约测试 | 支持的控制可透传；非支持别名由原生引擎处理，不新增全局未知字段拒绝，避免破坏 llama.cpp 与后续引擎扩展。 |
| D-07 | 保留 ID 分工，补文档 | 保留上游 x-request-id/request-id；x-lsm-request-id 关联 Manager 账本，不能丢失引擎日志关联。 |
| D-08 | 保留未知能力值 | null 不是 false，不按模型名字猜 tools=true；未证实特定客户端自动禁用工具。 |
| D-11 | 保留能力边界 | Responses 无云端 store/previous_response_id；不模拟持久会话。 |
| D-12 | 保留容量声明及性能边界 | 100K 首字时间是观测，完整 262K 时间只是外推，不能称实测。 |
| D-13/14 | 保留原生 Messages 行为并说明 | 最终 usage 正确，首帧是暂定值；空 thinking signature 不提供 Claude 签名验证能力，不伪造签名。 |
| D-15 | 保留标准及扩展字段 | Chat 流式 usage 为 opt-in；首 SSE 元数据事件不是首答案 Token，性能扩展字段按协议读取。 |
| D-18 | 保留 trim 归一化 | 归一化后精确匹配公开别名，仍无内部选择器回退；GET 发现路径保持原样精确匹配。 |
| D-19 | 保留生命周期统计，澄清口径 | 取消不是成功；短样本/取消时的瞬时 TPS 不能当持续吞吐。 |
| D-20 | 保留当前本机配置 | 无 Key 仅是回环测试配置；0/null 可能表示未知或采集缺失，不代表实际内存为零。 |

## 纠正测试方法

- “605 Token 超过 400–600 上限”的请求实际是 `max_tokens=900`；没有违反 API 上限。
- Messages 计数比较必须使用相同思考设置，生成输入合计应含
  `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`。
  受控复测：开启思考 57=57；关闭思考 17=7+10，未见并发污染。
- 性能观测不能直接归因为 MTP 收益，须同任务 MTP 开/关对照。
  401 条登记包含 INFO、对照与重试，不等于 401 个独立质量问题。
- 原始 stop 复测证明停止发生于模板分隔符，不能写成 Manager 删除已生成答案。

## 验证入口

- Rust HTTP/并发回归：`proxy_manual_audit_tests.rs` 与 `proxy_runtime.rs`。
- 实机复测：`test-gufo-real.cjs --extended true` 使用独立目录/端口/缓存，
  顺序加载两套 Gufo 模型；`--llama-engine` 同时验证 llama.cpp。
- `router-audit-acceptance.cjs` 对实际服务验证分类、计数、CORS、取消后恢复，
  Gufo 额外比较相同思考设置的计数，以及预算/stop 的直连与代理结果。
- 必须完成 `npm run check:release`、生产构建、Rust 测试及实际新包验证。
  最终运行结果记录在本任务的交付说明中，未执行的测试不计通过。
