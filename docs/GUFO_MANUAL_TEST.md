# Gufo 接入人工测试包

这是独立的 Windows x64 便携测试包，面向当前本机的两套 GGUF 模型。
不安装、不覆盖日常 Manager 或 Gufo，不注册开机启动，也不自动加载大模型。
包含 Gufo 0.28.1（408a04357827）及其运行库副本，GGUF 权重仍从原位置只读使用。

## 启动与停止

1. 将压缩包完整解压到一个固定目录，双击 **Start.cmd**。不要直接运行 bin 中的 exe。
2. 首次启动会验证程序校验和，创建两套独立实例，并打开 Manager 界面。
3. 在实例管理中启动一套模型。显示“模型加载中”时等待，Flash-Next 首次可能需要数分钟。
4. 切换模型时，先停止当前实例，再启动另一套。不要同时加载这两套大模型。
5. 测试结束双击 **Stop.cmd**，关闭该测试包的管理服务、路由和模型。**Status.cmd** 可查看状态。

独立数据目录：`%LOCALAPPDATA%\LlamaServerManager-Gufo-Test\<包目录名>`。
每个实例的缓存分别在其 `gufo\<实例ID>\snapshots` 内。日常 Gufo 缓存不会被使用。
BUILD.json 中的包标识决定数据目录身份。首次初始化后请保持解压路径不变。

## 接口

| 用途 | 地址 / 模型名 |
| --- | --- |
| 统一 API | `http://127.0.0.1:18580/v1` |
| 27B + DFlash2 | `qwen3.8-27b`，实例端口 18581 |
| Flash-Next + MTP | `qwen3.8-flash-next`，实例端口 18582 |

默认只监听本机，无 API Key；客户端必须填 Key 时可填任意非空值。
也可在统一 API 页面添加实际的 Key、并发限制和硬 Token 配额。
Chat Completions、Responses、Anthropic Messages 均使用引擎原生协议。
如端口已被占用，在实例配置和统一 API 页面选择其他空闲端口；程序不会终止端口占用者。

## 建议人工检查

- 启停两套模型，检查加载状态、日志、模型名和端口。
- 在参数页面查看主模型、投影文件、MTP / DFlash2、思考模式、262144 上下文和独立缓存配置。
- 参数入口位于实例管理页：选中实例，在下方点击“配置参数”。
- 用兼容客户端测试文字对话、流式输出、工具调用、图片、取消生成和历史消息。
- 性能监控中核对 Prefill / Decode、活动请求及系统资源；缺失指标显示不可用。
- 隐藏再打开界面，确认后台实例仍能管理；停止再启动模型，确认磁盘缓存可以复用。
- 在统一 API 页面测试 Key 和 Token 硬配额；超出限额应拒绝请求，不能静默绕过。

Gufo 的上下文占用比例只描述正在处理的请求，不代表 RAM、显存或保留缓存大小。
llama.cpp 的槽位检查点操作不适用于 Gufo；Gufo 自行管理其磁盘缓存。
262K 是配置容量，并不表示已经验证完整 262K 输入；本包不承诺两套大模型同时驻留。
Responses 使用 Gufo 的原生能力边界，不提供云端会话存储或后台任务模拟。

## 本轮 API 修复与保留行为

本包修复本地拒绝计数、路由错误分类、CORS 响应头和 404/405 错误信封。
详细决定见包内 `AUDIT-DECISIONS.md`。

- 单槽位忙碌时继续立即返回 **503 / route_capacity**，带 `Retry-After: 1`；
  客户端采用有次数/总时限上限的退避重试。该数值不是槽位一秒内释放的保证。
- 停机/加载中返回 **503 / route_unavailable**，未知模型返回 **404 / model_not_found**。
  没有默认目标且缺失 model 返回 **400 / model_required**。
- `queueTimeoutMs` 只控制全局/Key 准入等待，不增加目标队列或引擎会话。
- 查 Manager 请求账本用 `x-lsm-request-id`；上游 `x-request-id` 保留用于引擎日志。

Gufo 的小输出预算可能全部用于思考。检查 `finish_reason=length`、
Responses 的 `status=incomplete`、Messages 的 `stop_reason=max_tokens`，
不能仅根据 HTTP 200 判断是否完成。短答案可显式使用：

| 协议 | 关闭思考的请求字段 |
| --- | --- |
| Chat Completions | `"reasoning_effort":"none"` |
| Responses | `"reasoning":{"effort":"none"}` |
| Anthropic Messages | `"thinking":{"type":"disabled"}` |

这三种写法不能跨协议照搬。Responses 应传 `reasoning.effort`（例如上表的
嵌套对象），不是 Chat 的顶层 `reasoning_effort`；当前 Gufo 会以 400 拒绝
后者。Manager 保留此校验，不做隐式参数转换。

Chat 还支持 `chat_template_kwargs.enable_thinking=false` 和
`thinking.type=disabled`；顶层 `think`、`enable_thinking`、
`reasoning.enabled` 不是该 Gufo Chat 接口的有效替代写法。
Chat 的显式思考预算 `reasoning_budget_tokens` 应放在顶层；它不保证任意任务都能
在极短总预算内输出正文。计数与生成必须带相同的思考/工具/图片设置。

`stop=["\n\n"]` 会作用于 Gufo 的原始输出，可能命中思考到正文之间的空行，
产生 `finish_reason=stop` 的空正文。避免把空行设为通用 stop；对纯短答显式关闭思考。
Manager 不删除 stop，也不替用户续写或提高预算。
Chat 流式需要 usage 时传 `stream_options.include_usage=true`。
Messages 首帧 usage 暂为零，使用末帧累计值并加上缓存输入；
本地 thinking signature 不具备 Claude 云端签名验证能力。

## 工具调用历史与截断

统一 API 的 `POST /v1/chat/completions` 会检查现代工具历史：每个 assistant
`tool_calls[].id` 必须由且仅由一个 `role:tool` 消息的 `tool_call_id` 应答，
并在后续非 tool 消息或新一轮生成之前完成。缺失、错配、重复或孤立结果
返回 `400`、`error.type=invalid_request_error`、`error.code=invalid_tool_history`；
`error.param` 指向消息/字段位置，例如 `messages[2].tool_call_id`。
错误请求不会进入模型生成、Token 预检或配额预留，本地拒绝计数加一次。

并行结果允许乱序；失败或取消也可以作为正常工具结果回传。历史工具不必仍在
当前 tools 列表中。该校验不解释工具参数、不修改历史，不作用于 Token 计数、
Responses、Messages 或旧式 function_call/function 消息。直连 Gufo 端口
绕过此 Manager 校验，仍由原生引擎决定是否接受历史。

`tool_choice=required` 或指定某个函数仍受总输出预算约束，不保证调用一定完整。
128/256 Token 都不是通用安全阈值；关闭思考也不保证较长参数能容纳在小预算内。
Agent 客户端应把 `finish_reason=length`（以及 Responses incomplete、
Messages max_tokens）视为未完成，不能把空工具调用列表直接当作任务结束。
不要执行尚未拼接完整的 SSE 工具参数。需要重试时保留已有执行记录，
对有副作用的工具做幂等/去重，避免整轮重放导致重复操作；失败/取消结果须用
原调用 ID 回传。Manager 不自动提高预算、关闭思考或重试工具。

`BUILD.json` 和 `SHA256SUMS.txt` 记录候选程序及引擎身份。测试包不是正式安装版或自动更新渠道。
首次初始化后，界面内修改会持久保存；再次运行 Start.cmd 不会重置配置。
如权重被移动，优先在参数页面更新路径；首次初始化前也可编辑 profiles.json。
测试包停止后可删除解压目录；如需同时移除测试数据，只删除上面明确列出的独立数据目录。
