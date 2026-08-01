# Anthropic API 与 Claude Code 兼容说明

实例路由在同一监听地址同时提供 OpenAI 与 Anthropic 两套 API 格式。Anthropic 请求由路由代理完成鉴权、模型选择与公开别名改写，然后无损转发给 llama.cpp 原生 Messages API。

面向普通用户的最新说明同时发布在[在线路由文档](https://docs.cnzone.net/docs/routing)。

## 端点

| 功能 | 端点 |
| --- | --- |
| Messages，同步与 SSE 流式 | `POST /v1/messages` |
| 输入 Token 计数 | `POST /v1/messages/count_tokens` |
| Claude Code / SDK 模型发现 | `GET /v1/models` |
| 单模型信息 | `GET /v1/models/:model_id` |

请求可使用 `x-api-key: <代理 API Key>` 或 `Authorization: Bearer <代理 API Key>`。代理验证公开凭据后会将其移除，仅向目标实例发送该实例自己的 API Key；`anthropic-version`、`anthropic-beta` 与自定义业务请求头会继续转发。

`v2.9.37` 的 llama.cpp 稳定版基线为 `b10215`。该版本原生支持 Messages、SSE、system/messages、采样参数、停止序列、工具选择和 Token Count。后续版本的权威基线以 `scripts/llama-parameter-baseline.json` 为准。工具调用需要在目标实例配置中启用 `--jinja`。请求体上限为 32 MiB，可容纳 Anthropic API 允许的图片等多模态内容块。

## Claude Code（PowerShell）

先在“实例路由”中为本地模型配置一个公开模型名，例如 `local-claude`，启动目标实例与代理，再在启动 Claude Code 的同一 PowerShell 会话中设置：

```powershell
$env:ANTHROPIC_BASE_URL = 'http://127.0.0.1:11435'
$env:ANTHROPIC_AUTH_TOKEN = '<实例路由中配置的代理 API Key>'
$env:ANTHROPIC_MODEL = 'local-claude'
$env:ANTHROPIC_DEFAULT_HAIKU_MODEL = 'local-claude'
$env:ANTHROPIC_DEFAULT_SONNET_MODEL = 'local-claude'
$env:ANTHROPIC_DEFAULT_OPUS_MODEL = 'local-claude'
$env:CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY = '1'
claude --model local-claude
```

如果代理未配置公开 API Key，可省略 `ANTHROPIC_AUTH_TOKEN`。`ANTHROPIC_BASE_URL` 填写代理根地址，不附加 `/v1`；Claude Code 会自行请求 `/v1/messages`。三个 `ANTHROPIC_DEFAULT_*_MODEL` 变量让子代理、快速模型与模型族回退继续使用同一公开路由名；如果你为不同模型族配置了不同本地实例，可以分别填写对应的公开路由名。模型发现需要 Claude Code `v2.1.129` 或更新版本。

## 兼容边界

- 代理不把 OpenAI 请求转换为 Anthropic，也不把 Anthropic 请求转换为 OpenAI；两种客户端各自调用对应路径，共用同一路由表。
- 文本、图片、`tool_use`、`tool_result`、usage 字段和流式事件由 llama.cpp 原生实现处理，代理只改写协议规定的模型标识，不改写工具输入中的同名字段。
- `prompt caching`、服务端 Web Search、Files、Message Batches、引用、云端计费和加密思考等 Anthropic 云服务能力不会由本地代理模拟。相关请求头会透明转发，但最终行为取决于目标 llama-server 与模型。
- llama.cpp 对 Anthropic API 的目标是实用兼容，并不承诺覆盖 Anthropic 云端 API 的每一项扩展。目标实例过旧或不提供 Messages 端点时，代理会返回 Anthropic 格式的明确错误。

## Anthropic API and Claude Code

The routing proxy exposes OpenAI and Anthropic formats on the same listener. Use `POST /v1/messages`, `POST /v1/messages/count_tokens`, and `GET /v1/models`. Point `ANTHROPIC_BASE_URL` at the proxy root, authenticate with either a bearer token or `x-api-key`, and set `ANTHROPIC_MODEL` to a configured public route name. Tool use requires `--jinja` on the target llama-server. Cloud-only Anthropic features are passed through without manager-side emulation and remain dependent on the selected llama-server and model.
