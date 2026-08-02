# 生产级模型路由器：API、调度与运维说明

实例路由默认监听 `127.0.0.1:11435`，在同一入口提供 OpenAI 与 Anthropic 原生协议端点。路由器不会把两种请求体互相翻译：OpenAI 客户端使用 OpenAI 端点，Anthropic 客户端使用 Messages 端点；两者共享严格模型路由、目标健康状态、调度、鉴权、流量治理和遥测。

面向普通用户的最新说明同时发布在[在线路由文档](https://docs.cnzone.net/docs/routing)。Anthropic 与 Claude Code 的专门配置见 [ANTHROPIC_API_COMPATIBILITY.md](ANTHROPIC_API_COMPATIBILITY.md)。

## API 兼容范围

### OpenAI 格式

| 能力 | 端点 |
| --- | --- |
| 模型列表与单模型信息 | `GET /v1/models`、`GET /v1/models/:model_id` |
| Chat Completions，同步与 SSE | `POST /v1/chat/completions` |
| Chat 输入 Token 计数 | `POST /v1/chat/completions/input_tokens` |
| Legacy Completions | `POST /v1/completions` |
| Responses，同步与 SSE | `POST /v1/responses` |
| Responses 输入 Token 计数 | `POST /v1/responses/input_tokens` |
| Embeddings | `POST /v1/embeddings`，并兼容 `/embedding`、`/embeddings` |
| Rerank | `POST /v1/rerank`，并兼容 `/rerank`、`/reranking`、`/v1/reranking` |

Chat Completions 与 Responses 会保持工具调用、结构化输出、usage 和 SSE 事件，仅把协议规定的响应模型标识改回公开模型名。OpenAI 错误统一为 `error.message/type/param/code`，响应包含 `x-request-id`。路由器使用官方 `openai` JavaScript SDK 对模型发现、Chat、Responses、工具、流式和 Token Count 做契约测试。

### Anthropic 格式

| 能力 | 端点 |
| --- | --- |
| Messages，同步与 SSE | `POST /v1/messages` |
| 输入 Token 计数 | `POST /v1/messages/count_tokens` |
| 模型发现 | `GET /v1/models`、`GET /v1/models/:model_id` |

Messages 的文本、图片、thinking、`tool_use`、`tool_result`、usage 和流式事件由目标 llama-server 的原生协议处理。错误采用 Anthropic `type: error` 信封并包含 `request-id`；速率限制使用 Anthropic 请求速率响应头。工具调用要求目标实例启用 `--jinja`。官方 `@anthropic-ai/sdk` 契约测试覆盖同步、流式、图片、工具、Token Count 和模型发现。

### 运维与能力发现

| 能力 | 端点与行为 |
| --- | --- |
| 路由器入口 | `GET /`，列出当前受支持端点 |
| 兼容健康状态 | `GET /health` |
| 存活探针 | `GET /live`：只证明路由进程可响应 |
| 就绪探针 | `GET /ready`：立即探测后端；无健康路由时返回 `503` |
| Prometheus | `GET /metrics`：请求、拒绝、上游错误、耗时、并发、目标就绪和运行时间 |
| 安全模型能力 | `GET /props?model=<公开模型名>` |
| 安全槽位状态 | `GET /slots?model=<公开模型名>`；可附加 `fail_on_no_slot=1` |

`/props` 只公开 `n_ctx`、槽位数量、模板能力、多模态能力、睡眠状态和路由健康摘要；不会暴露模型路径或聊天模板正文。`/slots` 只公开每个槽位的 `id`、`n_ctx`、`speculative`、`is_processing` 和 `n_past`，不会暴露提示词或 Token。比如后端 `http://127.0.0.1:8080/slots` 报告 `n_ctx: 131072` 时，客户端可通过 `GET /slots?model=my-model` 从统一路由取得同一个 128K 上下文信息。

`/v1/models` 与 `/v1/models/:model_id` 会把同一个上下文窗口同时写入 `context_length`、`context_window` 和 vLLM 兼容的 `max_model_len`。运行时探测到的 `n_ctx` 优先；首次探测完成前，仅将管理器明确配置且未启用自动上下文的 `ctx_size` 作为兜底。模型发现本身不会发起探测或改变目标健康状态。一个公开模型映射到多个故障转移目标时，路由器只声明所有候选目标都能安全支持的最小上下文；任一候选既没有有效运行时值也没有明确配置值时，三个字段都返回 `null`，不会猜测或夸大能力。工具和多模态能力同样采用保守聚合。

## 上下文超限保护

当选中目标已有可信 `n_ctx` 时，路由器会在生成请求进入推理队列前执行 Token 预检：

- Chat Completions 使用目标的 `/v1/chat/completions/input_tokens`。
- Responses 使用目标的 `/v1/responses/input_tokens`。
- Anthropic Messages 使用目标的 `/v1/messages/count_tokens`。
- Legacy Completions 的单条或小批量 prompt 使用目标的 `/tokenize`，并与实际请求相同地加入特殊 Token。

预检比较“模板化后的输入 Token + 请求的最大输出 Token”与目标实际上下文窗口。超过限制时不发送生成请求，OpenAI 返回 `400 invalid_request_error` 和 `code: context_length_exceeded`，Anthropic 返回 `400 invalid_request_error` 信封；两者都附带输入、输出、上下文的机器可读详情和 `x-llama-server-manager-*` 响应头。`n_ctx`/`max_model_len` 表示输入与输出合计窗口，不是单独的最大输出长度。

Token 计数端点由目标 llama-server 提供。如果目标版本不支持、请求超时或计数响应无效，路由器会保持兼容并继续转发原请求，让目标执行最终校验；不会用字符数或经验比例制造可能误判中文、代码、工具定义和多模态请求的伪精确结果。客户端仍应根据 `/v1/models` 的上下文元数据提前压缩会话。

## 严格模型边界

建议始终启用 `strictModelRouting`：

- 存在显式路由表时，只有精确匹配的公开模型名可以调用；内部实例 ID、实例名称、模型文件路径和上游 `--alias` 都不是公共选择器。
- 请求显式指定未知模型时返回 `404`，不会静默投递到默认实例。
- 默认实例只处理没有 `model` 字段的请求。
- 没有显式路由表时，只接受运行实例的公开别名精确匹配；别名为空时使用从模型文件名派生的安全公开名。
- `/v1/models` 和响应中的 `model` 只公开上述公共标识。

关闭严格模式只用于迁移旧配置；它会恢复未知选择器向默认实例回退的旧行为，不建议用于生产。

## 调度、健康和熔断

优先级数值越小，层级越高。只有最高且存在健康目标的优先级层参与当前调度；该层全部不可用后才进入下一层。

| 策略 | 配置值 | 行为 |
| --- | --- | --- |
| 优先级故障切换 | `priorityFailover` | 按稳定配置顺序选择最高优先级健康目标 |
| 轮询 | `roundRobin` | 在同优先级健康目标间均匀轮询 |
| 最空闲 | `leastBusy` | 综合路由器活动请求数和 `/slots` 压力选择目标 |
| 加权 | `weighted` | 按每条路由的 `weight` 在同优先级目标间分配 |

每条路由的 `maxConcurrentRequests` 可限制单个目标；设为 `0` 时继承全局容量。路由器定时请求 `/health`，并缓存 `/props` 与 `/slots` 能力。连续失败达到 `unhealthyThreshold` 后打开熔断器，`recoveryCooldownMs` 冷却期间排除目标；后续健康探测成功才恢复。网络错误、上游 `5xx` 和 `429` 都会计入目标故障状态，当前已经收到的 HTTP 错误会按相应协议返回，后续请求自动避开已熔断目标。

## 超时、并发和限流

- `connectTimeoutMs`：建立上游连接的上限。
- `timeoutMs`：非流式请求总时限。
- `streamingIdleTimeoutMs`：流式响应连续无数据的上限，不限制正常长会话总时长。
- `healthCheckIntervalMs` / `healthCheckTimeoutMs`：主动探测周期与单次探测上限。
- `maxConcurrentRequests`：整个路由器同时处理的推理请求上限。
- `queueTimeoutMs`：全局并发满后允许排队的时间；超时返回 `429` 与 `Retry-After`。
- `requestsPerMinute`：全局 Token Bucket 请求速率；`0` 表示不限。单 Key 的值为 `0` 时继承全局设置。

请求体默认上限为 64 MiB；Anthropic Messages 上限为 32 MiB。缓冲 JSON 响应限制为 16 MiB，SSE 逐行处理并采用独立空闲超时。

## 鉴权、权限和 CORS

路由器支持旧版单一 API Key，以及多个具名 API Key。新 Key 自动生成高熵值，保存后只持久化 SHA-256 摘要，原始 Bearer 凭据无法从配置恢复；请在首次保存前复制并交付给客户端。公开凭据在路由边界被移除，目标只会收到该实例自己的 API Key。

客户端可使用：

```text
Authorization: Bearer <router-api-key>
```

或：

```text
x-api-key: <router-api-key>
```

多 Key 权限范围：

- `inference`：所有推理、向量和重排 POST 端点。
- `discovery`：根目录、模型发现、健康/就绪、指标、`/props` 和 `/slots`。
- 空权限列表为兼容用途，等同同时授予两项；生产配置建议显式选择。

CORS 只接受逗号分隔的精确 HTTP(S) Origin，例如 `https://app.example.com`。不支持 `*`、路径、查询参数或 `file:`；列表为空时拒绝所有带 `Origin` 的跨域请求。合法预检返回允许方法、请求头、公开响应头和 10 分钟缓存。

内置路由器只允许回环地址。跨机器访问应在本机前置提供 TLS 和访问控制的反向代理、Cloudflare Tunnel、VPN 或 SSH 隧道，不应把明文 HTTP 监听直接暴露到局域网或公网。

## 兼容边界

“OpenAI / Anthropic 兼容”指本地 llama-server 能提供的推理协议，不表示模拟云厂商完整产品面。路由器不实现 OpenAI Files、Batches、Assistants、Audio、Images、Fine-tuning，也不模拟 Anthropic Message Batches、Files、服务端 Web Search、云端 prompt caching、计费或组织管理。未知云端端点不会被转发到任意后端。

目标模型、聊天模板和 llama-server 版本仍决定具体的视觉、工具、thinking、结构化输出等能力。客户端应先读取 `/v1/models`、`/props` 与 `/slots`，并将协议错误、`404`、`429`、`502/503` 和流式断开纳入正常重试/降级策略。

## Production router summary

The router exposes native OpenAI Chat Completions, Responses, Completions, embeddings and rerank endpoints together with Anthropic Messages and token counting. Model discovery publishes `context_length`, `context_window`, and vLLM-compatible `max_model_len` using the safe minimum across failover targets. Exact token-count preflight rejects oversized generation requests in OpenAI or Anthropic error format when the target supports counting, while unsupported counters fail open to the target's own validation. Exact public model IDs form a strict security boundary. Priority tiers, round-robin/least-busy/weighted scheduling, active probes, circuit breaking, independent streaming timeouts, concurrency queues, scoped hashed API keys, exact-origin CORS, safe `/props` and `/slots` discovery, request IDs, and Prometheus metrics are built in. The listener remains loopback-only; terminate TLS at a trusted local reverse proxy or tunnel for remote access.
