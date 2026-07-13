# 隐私政策 / Privacy Policy

最后更新：2026-07-13

Llama Server Manager 是本地运行的开源桌面应用。项目维护者不经营用户账号、云端遥测平台或广告服务，也不会通过本应用接收用户的配置、日志或性能记录。

## 本地保存的数据

应用会在当前用户的应用数据目录中保存运行所需的数据，包括实例和引擎配置、模型目录、下载队列与历史、服务器日志、集群配置以及性能遥测数据库。这些性能遥测用于应用内监控和诊断，默认不会上传给项目维护者。

配置中可能包含 API Key、SSH 用户名、私有路径或远程服务器地址。用户应保护自己的应用数据目录，并在分享日志或截图前删除敏感信息。

## 网络连接

应用仅为提供明确功能而发起以下连接：

- 启动后访问 GitHub Releases API，检查本项目是否存在新版本。
- 用户浏览或下载模型时访问 ModelScope 或 Hugging Face，并传输所请求的仓库标识、文件路径和正常 HTTP 元数据。
- 用户扫描、测试或使用集群 Worker 时访问局域网设备或用户配置的远程节点。
- 用户启动实例、查看指标或使用统一 API 路由时访问本机或用户配置的 `llama-server` 地址；为鉴权配置的凭据仅发送到对应目标。
- 用户主动打开外部链接时，由系统浏览器访问相应网站。

GitHub、ModelScope、Hugging Face 以及用户配置的远程服务各自适用其隐私政策。应用不控制这些第三方如何处理连接日志和请求元数据。

## 不包含的行为

- 不包含广告或用户行为分析 SDK。
- 不向项目维护者自动发送崩溃报告、性能记录、模型内容或服务器日志。
- 不出售或出租用户数据。

## 用户控制

用户可以删除应用数据目录中的配置、日志、下载记录和本地遥测数据库。卸载应用是否同时删除这些数据取决于操作系统和安装方式。

隐私问题或行为差异可通过项目的 [GitHub Issues](https://github.com/jerrydong1988/llama-server-manager/issues) 报告。公开提交前请先移除 API Key、SSH 凭据、个人路径和其他敏感内容。

---

Llama Server Manager is a locally operated open-source desktop application. The project maintainer does not operate user accounts, hosted telemetry, or advertising services and does not receive application configuration, logs, or local performance records automatically.

The application stores configuration, download state, logs, cluster settings, and a local telemetry database in the current user's application data directory. It connects to the GitHub Releases API for update checks; to ModelScope or Hugging Face when the user browses or downloads models; and to local or user-configured servers and workers for management, health checks, metrics, routing, and SSH/RPC functions. Credentials configured for a server are sent only to the corresponding configured target.

The application contains no advertising or behavior-analytics SDK and does not automatically upload crash reports, performance records, model contents, or server logs to the project maintainer. Third-party services and user-configured remote systems are governed by their own privacy policies.
