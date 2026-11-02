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

| 情况 | HTTP / 错误标识（默认指 error.code） | 处理 |
| --- | --- | --- |
| 健康匹配目标全部满载 | 503 / `route_capacity` | `Retry-After: 1`，提示客户端有界重试 |
| 已配置公开路由的目标停机、加载中或不健康 | 503 / `route_unavailable` | `Retry-After: 1`；不承诺一秒内就绪 |
| 未知公开模型 | 404 / `model_not_found` | 检查模型名，不静默回退 |
| model 缺失/null，且无可解析的默认目标 | 400 / `model_required` | 修改请求或配置默认实例 |
| model 是错误类型或空字符串 | 400 / **error.type**=`invalid_request_error` | OpenAI 的 code 为 null；Anthropic 此类通用错误不带 code/param。不再当作省略 model。 |
| 运行模型与向量端点任务类型不匹配 | 400 / `model_endpoint_mismatch` | 选用匹配的 embedding/rerank 模型 |
| 全局/Key 准入超时或限流 | 429，保留现有行为 | 与目标忙碌分开 |
| 路径不存在 / 方法不支持 | 404 / `endpoint_not_found`；405 / `method_not_allowed` | 返回相应协议 JSON 信封；Messages 的版本门可能先返回 400，见下文。 |

上述路由错误码在 OpenAI 和 Anthropic 信封中一致，并与请求账本中的
诊断代码一致。保留已配置默认实例和既有单个隐式目标回退规则。
实机回归另外修复了客户端混合请求头的协议选择：已知推理端点以 URL
为准，Chat/Responses 即使携带 anthropic-version 也保持 OpenAI 错误与
x-request-id；共享发现端点按该请求头选择请求 ID 头及错误信封，成功 JSON
使用同时包含 OpenAI/Anthropic 字段的统一模型描述，不按头重写数据结构。
严格路由仍只接受公开别名；不通过错误分类暴露内部实例 ID、私有名称或路径。
模型列表继续只展示运行中的模型，管理界面负责展示停机配置。

在 CORS、鉴权和权限检查通过后，Messages 路径先校验 anthropic-version，
再进入方法处理。因此 GET /v1/messages 缺版本时返回 400，带有效版本时
返回 405；OPTIONS 预检单独处理。保留此顺序，不把同时存在多个问题的
请求强制改为优先报告方法错误。

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

## 第二轮剩余 INFO/LOW 的复核

依据 `guf-api-audit-r2-20261102/reports/REPORT-R2.md` §5，核对源码、
原始指标文本、SDK 类型及官方协议说明；本轮仅整理文档，不改程序或部署。

| 项目 | 决定 |
| --- | --- |
| L-01 版本门先于方法检查 | 行为合理；补明校验顺序，不调整 400/405 优先级。 |
| L-02 type 被列成 code | 确认文档错误，已修正上表，并区分 null 与字段缺失。 |
| L-03 7 组指标缺 HELP | 事实成立，但当前 Prometheus 0.0.4 文本格式允许省略 HELP，采集不受影响；在路由说明中补指标释义，不为元数据注释单独修改二进制或重打包。 |
| L-04 模型发现成功体不按版本头切换 | 保留统一描述，补文档。data 并非 OpenAI 独有；当前还包含 first_id/last_id/has_more 及 type/display_name/created_at 等 Anthropic 字段，不能定性成“仅 OpenAI 格式”。不据此承诺完整覆盖所有云端模型元数据。 |
| L-05 停机目标无 target_* 样本 | 两类接口覆盖范围不同，补明配置路由与运行目标缓存的口径；缺失不等于健康，不新增混淆两种口径的样本。 |
| L-06 Responses 不接受顶层 reasoning_effort | 是请求字段用错协议；Responses 使用 reasoning.effort，Chat 使用 reasoning_effort。修正测试/调用示例，不在网关做隐式参数映射，也不归类为 Gufo 缺少标准 Responses 推理控制。 |

协议依据：[Prometheus 文本格式的可选 HELP](https://prometheus.io/docs/instrumenting/exposition_formats/)、
[Anthropic 模型列表](https://platform.claude.com/docs/en/api/typescript/models/list)、
[OpenAI 推理字段说明](https://developers.openai.com/api/docs/guides/reasoning)。

## 27B 工具调用复核与处理 — 2026-11-03

依据 `guf-api-audit-27b-20261102/reports/REPORT-27B.md`、原始请求与
Manager 侧六次直连/代理对照复测，用户授权后实现以下范围：

- **A-02：修复 Manager 的 Chat 生成入口。** 新增工具调用与结果的结构配对
  校验，缺失/重复/错配/孤立结果返回 `400 invalid_tool_history` 和消息位置。
  在目标许可、预检与配额预留之前拒绝；保持统一计数和账本诊断。
  同一 assistant 批次 ID 唯一，每个调用恰好一个结果；并行结果允许乱序，
  已完成的不同轮次不做全局 ID 去重。失败/取消是合法工具结果。
  不把历史函数名强制绑定到当前 tools，不校验参数含义，也不修改工具载荷。
  范围限于 `/v1/chat/completions`，不把 Chat 配对规则套到计数接口、
  Responses 的状态化流程、Messages 或旧式 function_call/function 历史。
  Gufo 二进制基线保留，直连原生端口不受此网关校验保护。
- **A-01：保留预算语义，完善工具场景说明。** length/incomplete/max_tokens
  是截断信号；required/named 不绕过预算。不得把 128 当通用安全阈值，
  报告自己已有 named + 128 截断反例。关闭思考也不能保证任意长参数完整。
  客户端应检查完成状态、完整拼接参数，并在重试前处理工具执行去重。
- **A-03/TP-03：性能观察。** 优先按需挂载工具、缩小候选集合与上下文；
  不根据不同模型/负载的接受率直接判断 DFlash2 净收益，不调整当前部署参数。
- **TP-01/TP-02：保留 strict 约束。** Gufo 当前对单个工具参数 schema 编译
  累计 enum/const 值，超过 1000 拒绝；OpenAI 也有总 enum 值 1000 的限制。
  additionalProperties=false、全部属性 required 是 strict 规则，不作为故障修复。
  错误缺少工具/字段路径可由 Gufo 校验器后续完善，本轮不在 Manager 复制编译器。
- **TP-04/05/06/07/08：保留行为。** 未知工具名已有明确拒绝，能力 null
  表示未知，计数须同参，draft 指标按协议读取。tool_choice=none 禁止本轮用工具，
  模型说明没有实时工具不能单独定为故障。报告预算脚本将 named 标为 required，
  覆盖范围需按实际请求重新标注。

回归包括非法历史的同步/流式拒绝、合法并行结果乱序、错误结果、历史工具移除、
旧式函数/assistant 文本预填充、计数与其他协议不被拦截，以及 Gufo 两模型和
llama.cpp 的真实工具往返、拒绝增量与账本 forwarded/outcome/failure 核对。
