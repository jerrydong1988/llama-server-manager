# Llama Server Manager 使用说明 / User Guide

> v2.9.45 · Windows / macOS / Linux

本说明按实际操作顺序介绍模型、引擎、实例、路由和监控功能。应用内“使用说明”页面会随安装包离线提供同一份内容和图片。

This guide follows the real workflow from models and engines to instances, routing, and monitoring. The in-app Guide ships the same content and images for offline use.

在线最新版本请访问[文档主站](https://docs.cnzone.net/docs)，正式安装包请从[下载中心](https://download.cnzone.net/)获取。

Visit the [documentation site](https://docs.cnzone.net/docs) for the latest online version and the [Download Center](https://download.cnzone.net/) for official installers.

---

## 目录 / Table of Contents

1. [快速开始 / Quick Start](#快速开始-quick-start)
2. [系统总览 / Dashboard](#系统总览-dashboard)
3. [模型仓库 / Model Repository](#模型仓库-model-repository)
4. [下载管理 / Download Manager](#下载管理-download-manager)
5. [引擎管理 / Engine Management](#引擎管理-engine-management)
6. [实例管理 / Instance Management](#实例管理-instance-management)
7. [参数配置 / Parameter Configuration](#参数配置-parameter-configuration)
8. [集群管理 / Cluster Management](#集群管理-cluster-management)
9. [实例路由 / Instance Routing](#实例路由-instance-routing)
10. [性能监控 / Performance Monitoring](#性能监控-performance-monitoring)
11. [监控大屏 / Monitoring Wall](#监控大屏-monitoring-wall)
12. [服务器日志 / Server Logs](#服务器日志-server-logs)
13. [常见问题 / FAQ](#常见问题-faq)

---

## 快速开始 / Quick Start

### 安装 / Install

1. 从[下载中心](https://download.cnzone.net/)下载对应平台安装包；无法访问时可使用 [GitHub Releases](https://github.com/jerrydong1988/llama-server-manager/releases/latest) 备用源。
2. Windows 使用 MSI 或 NSIS 安装包；macOS 使用 DMG；Linux 使用 DEB。自 `v2.9.43` 起暂停发布 AppImage，以避免旧版内置 GLib/Wayland 与新系统图形栈混用造成空白窗口。
3. 准备与本机后端匹配的 `llama-server`，例如 CUDA、ROCm、Vulkan 或 CPU 构建。
4. 准备本地 GGUF 模型，或稍后从 ModelScope / HuggingFace 下载。

1. Download the package for your platform from the [Download Center](https://download.cnzone.net/), with [GitHub Releases](https://github.com/jerrydong1988/llama-server-manager/releases/latest) available as an alternative source.
2. Use MSI or NSIS on Windows, DMG on macOS, and DEB on Linux. AppImage distribution is suspended starting with `v2.9.43` because mixing older bundled GLib/Wayland libraries with newer system graphics stacks can produce a blank window.
3. Prepare a `llama-server` build for your backend, such as CUDA, ROCm, Vulkan, or CPU.
4. Prepare a local GGUF model, or download one later from ModelScope or HuggingFace.

Linux DEB 安装后可正常配置“开机自启动”，版本更新需要从下载中心或 GitHub Releases 手动安装。正式标签构建在配置证书时执行 Windows 签名与 macOS 签名和公证；未配置时仍会发布文件名带 `-unsigned` 的 Windows 安装包和带 `-adhoc` 的 macOS DMG。普通 CI 产物仅用于测试。

Linux DEB installations can use autostart normally, while version updates must be installed manually from the Download Center or GitHub Releases. Tagged builds use formal Windows signing and macOS signing or notarization only when credentials are configured; otherwise the release publishes clearly labeled `-unsigned` Windows installers and `-adhoc` macOS DMGs. Regular CI artifacts are for testing only.

### 应用更新 / Application Updates

应用启动后会通过项目的 Cloudflare R2 更新服务检查新版本。发现更新时，顶部会显示版本按钮；点击后需要再次确认，应用才会下载经过 Tauri 签名校验的更新包、安装并重启。若实例或统一路由仍在运行，确认框会明确提示可能中断当前任务。

The app checks the project's Cloudflare R2 update service at startup. When an update is available, a version button appears in the header. The signed package is downloaded, installed, and followed by a restart only after confirmation. If instances or routing are active, the confirmation warns that current work may be interrupted.

`v2.9.35` 及更早版本没有内置 Tauri Updater，因此必须先手动安装 `v2.9.36` 或更新版本；之后 Windows 与 macOS 才能使用应用内更新。Linux 当前仅发布 DEB，暂不提供应用内更新，请从下载中心或 GitHub Releases 手动安装。

`v2.9.35` and earlier do not contain Tauri Updater, so `v2.9.36` or later must first be installed manually before Windows and macOS can use in-app updates. Linux currently ships only DEB packages and does not provide in-app updates; install updates manually from the Download Center or GitHub Releases.

### 首次运行的五个步骤 / Five First-Run Steps

1. 在“模型仓库”添加 GGUF 模型目录并完成扫描。
2. 在“引擎管理”添加包含 `llama-server` 的目录，并设置默认引擎。
3. 在“实例管理”创建实例，选择模型、引擎和可用端口。
4. 在“参数配置”检查模型路径、上下文、GPU 层数和服务参数，然后保存。
5. 回到“实例管理”启动实例，通过“性能监控”和“服务器日志”确认运行状态。

1. Add and scan a GGUF directory in Model Repository.
2. Add a directory containing `llama-server` in Engine Management and choose the default engine.
3. Create an instance with a model, engine, and free port.
4. Review the model path, context, GPU layers, and service options in Configuration, then save.
5. Start the instance and verify it in Performance Monitoring and Server Logs.

![首次运行：模型、引擎和实例的设置顺序 / First run: model, engine, and instance setup](public/docs/guide/flow-01-first-run.png)

### 界面导航 / Navigation

左侧导航按“运行概况、资源准备、服务配置、分布式与路由、诊断、帮助”的顺序排列，共 12 个入口：系统总览、模型仓库、下载管理、引擎管理、实例管理、参数配置、集群管理、实例路由、性能监控、监控大屏、服务器日志和使用说明。

The sidebar contains 12 entries ordered around status, resources, service configuration, distributed routing, diagnostics, and help: Dashboard, Model Repository, Downloads, Engines, Instances, Configuration, Cluster, Instance Routing, Performance, Monitoring Wall, Logs, and Guide.

按 `Ctrl+K` 打开任务中心，可快速跳转页面、启动或停止实例、处理下载和查看诊断。`Ctrl+Enter` 启动或停止一个实例，`Ctrl+S` 保存配置。

Press `Ctrl+K` for the command center. `Ctrl+Enter` starts or stops an instance, and `Ctrl+S` saves configuration.

---

## 系统总览 / Dashboard

系统总览是启动后的运行控制台。即使没有实例运行，也会显示系统 CPU、内存以及可用的 GPU / 显存信息；同时汇总实例、模型、引擎、下载和需要处理的问题。

Dashboard is the launch-time control surface. It shows system CPU, memory, and available GPU or VRAM signals even before an instance is running, together with instance, model, engine, download, and attention summaries.

![系统总览展示资源、实例和运行状态 / Dashboard with resources, instances, and service health](public/docs/guide/01-dashboard.png)

### 主要区域 / Main Areas

- 顶部指标：运行实例、已登记模型与引擎、活动下载。
- 系统健康：CPU、内存、GPU 和显存的实时压力。
- 实例控制：直接查看状态并启动或停止实例。
- 关注中心：引擎缺失、模型为空、实例异常或下载失败等可操作提示。
- 最近活动：请求、下载和日志摘要。

- Top metrics for running instances, registered models and engines, and active downloads.
- System health for CPU, memory, GPU, and VRAM pressure.
- Instance controls for status and start or stop actions.
- Actionable attention items for missing resources, unhealthy instances, or failed downloads.
- Recent request, download, and log activity.

如果总览提示“未登记运行引擎”或“模型仓库为空”，先按提示按钮跳转并完成资源扫描，不要直接在实例页反复启动。

When Dashboard reports a missing engine or empty model inventory, follow the action to register the resource before retrying an instance start.

---

## 模型仓库 / Model Repository

模型仓库递归扫描本地目录，识别 GGUF 模型、分片、`mmproj` 投影器和 imatrix 文件，并从 GGUF 头读取架构、上下文长度、量化类型和能力摘要。

Model Repository recursively scans local folders for GGUF models, shards, `mmproj` projectors, and imatrix files, and reads architecture, context length, quantization, and capability metadata from GGUF headers.

![模型仓库的目录树、搜索和元信息 / Model repository tree, search, and metadata](public/docs/guide/02-model-repository.png)

### 添加和扫描目录 / Add and Scan Directories

1. 点击“添加模型目录”。
2. 选择包含一个或多个模型子目录的根目录。
3. 等待递归扫描完成；后续重新扫描会复用未变化目录的缓存。
4. 使用搜索框按文件名、架构或量化类型过滤。
5. 展开目录树查看模型、投影器和分片关系。

1. Select Add Model Directory.
2. Choose a root containing one or more model subfolders.
3. Wait for recursive scanning; later scans reuse unchanged directory results.
4. Filter by file name, architecture, or quantization.
5. Expand the tree to inspect models, projectors, and shards.

### 管理操作 / Management

- “在资源管理器中打开”定位文件。
- 删除操作会显示原生确认框；确认后从磁盘永久删除，请先确认没有实例正在使用该文件。
- 分片模型按一组展示，统计时不会把每个分片重复算作独立模型。
- 视觉模型通常需要匹配的 `mmproj`；在实例或参数页选择主模型时可自动关联同目录投影器。

- Open in Explorer or Finder locates the file.
- Delete uses a native confirmation and permanently removes the file; make sure no instance is using it.
- Sharded models are grouped and not double-counted as separate models.
- Vision models commonly need a matching `mmproj`, which can be associated from the same directory.

---

## 下载管理 / Download Manager

下载管理支持 ModelScope 和 HuggingFace，可浏览仓库文件、选择单文件或批量下载，并在应用重启后恢复队列状态。

Downloads supports ModelScope and HuggingFace repository browsing, single or batch downloads, and persistent queue restoration after an app restart.

![下载队列、传输策略和仓库浏览 / Download queue, transfer policy, and repository browser](public/docs/guide/03-download-manager.png)

### 浏览与下载 / Browse and Download

1. 选择 ModelScope 或 HuggingFace。
2. 输入仓库 ID，例如 `Qwen/Qwen3-8B-GGUF`。
3. 选择保存目录并点击“浏览”。
4. 在文件列表中选择单文件下载，或批量加入队列。
5. 使用任务卡片暂停、继续、重试或取消。

1. Select ModelScope or HuggingFace.
2. Enter a repository ID such as `Qwen/Qwen3-8B-GGUF`.
3. Choose a save directory and browse the repository.
4. Download one file or enqueue a batch.
5. Pause, resume, retry, or cancel from task cards.

### 传输策略 / Transfer Policy

- 默认恢复策略为“手动”：应用重启后保留队列，由用户决定何时恢复。
- “启动时自动恢复”会在应用启动后恢复可继续的任务。
- 默认并发数为 1，可在策略面板调整；增加并发会提高带宽和磁盘压力。
- 带宽限制为 0 时不限速，可按所选单位设置全局上限。
- 低优先级节流适合边下载边推理，代价是下载时间增加。
- 服务端支持 Range 时会使用断点续传；已存在且大小匹配的文件会标记完成，避免重复下载。

- Manual resume is the default: queues persist across restarts and resume only when requested.
- Auto on launch resumes eligible tasks after startup.
- Concurrency defaults to 1 and can be raised at the cost of bandwidth and disk pressure.
- A bandwidth limit of 0 means unlimited.
- Low-priority throttling reduces interference with inference but extends download time.
- Range-capable servers support resume, and matching local files are detected to avoid duplicate downloads.

下载失败时先展开错误信息；鉴权失败检查仓库权限，空间不足清理目标磁盘，网络中断则保留任务后重试。取消任务会停止传输；删除最终文件前会确认目标路径属于该下载任务。

For failed downloads, inspect the error details. Check repository access for authorization failures, free disk space for write failures, and retry retained tasks after a network interruption.

---

## 引擎管理 / Engine Management

引擎管理扫描 `llama-server` 可执行文件，自动识别 CUDA、ROCm、Vulkan 或 CPU 后端，并允许多个版本并存。

Engine Management scans `llama-server` executables, detects CUDA, ROCm, Vulkan, or CPU backends, and supports multiple installed versions.

![引擎扫描、后端识别和默认引擎 / Engine scanning, backend detection, and default selection](public/docs/guide/04-engine-manager.png)

### 登记引擎 / Register Engines

1. 点击“添加引擎根目录”。
2. 选择包含一个或多个 `llama-server` 构建目录的父目录。
3. 扫描后检查可执行路径和后端标签。
4. 为常用版本设置易识别名称，并设为默认引擎。

1. Select Add Engine Root.
2. Choose a parent folder containing one or more `llama-server` builds.
3. Verify executable paths and backend labels after scanning.
4. Name the common version and set it as default.

新实例优先使用默认引擎；每个实例仍可覆盖为不同版本。升级 llama.cpp 后重新扫描即可保留多个版本并逐实例切换。

New instances prefer the default engine, while each instance can override it. Rescan after a llama.cpp upgrade to keep versions side by side.

---

## 实例管理 / Instance Management

实例把模型、引擎、端口和独立参数组合成一个可启动服务。多个实例可以同时运行，但必须使用不同端口，并考虑显存和内存总量。

An instance combines a model, engine, port, and independent configuration into a runnable service. Multiple instances may run together with unique ports and sufficient memory.

![实例列表、创建入口和运行控制 / Instance list, creation, and runtime controls](public/docs/guide/05-instance-manager.png)

### 创建实例 / Create an Instance

1. 点击“创建实例”。
2. 输入实例名称。
3. 从模型树选择主模型，并确认需要的 `mmproj`。
4. 选择引擎；留空时使用默认引擎。
5. 输入端口并等待端口可用性检查。
6. 创建后进入参数配置检查详细参数。

1. Select Create Instance.
2. Enter an instance name.
3. Choose the main model and any required `mmproj`.
4. Select an engine or use the default.
5. Enter a port and wait for availability validation.
6. Open Configuration to review detailed parameters.

### 运行控制 / Runtime Controls

- 启动前会生成命令、检查端口和必要路径。
- 状态依次可能为已停止、启动中、运行中或错误。
- “测试连接”使用实例鉴权设置检查健康或模型接口。
- “打开 API 页面”会把通配监听地址转换为本机可访问地址。
- 命令预览可复制完整启动参数，便于复现问题。
- 可重命名、排序或删除实例；删除前会原生确认。

- Startup generates the command and validates ports and required paths.
- Status may be stopped, starting, running, or error.
- Test Connection checks health or model endpoints using the instance authentication settings.
- Open API maps wildcard bind hosts to a local browser address.
- Command Preview copies the complete launch command for diagnosis.
- Instances can be renamed, reordered, or deleted with confirmation.

![启动、健康检查、性能和日志诊断流程 / Start, health, performance, and log diagnosis](public/docs/guide/flow-02-start-and-diagnose.png)

启动失败时不要只重复点击启动。先看实例错误状态，再打开服务器日志检查完整命令和 stderr；常见原因是端口占用、路径不存在、后端与硬件不匹配或显存不足。

When startup fails, inspect the instance state and server logs instead of repeatedly retrying. Common causes are port conflicts, missing paths, backend mismatch, or insufficient VRAM.

---

## 参数配置 / Parameter Configuration

参数配置按当前实例保存，覆盖模型、生成、采样、性能、上下文、网络、鉴权、缓存、推测解码和多模型路由等结构化选项。程序会读取所选 `llama-server --help` 协商实际能力；当前上游稳定版基线跟踪 248 个参数条目。

Configuration is stored per instance and covers structured options for models, generation, sampling, performance, context, networking, authentication, cache, speculative decoding, and routing. The app negotiates actual capabilities from the selected `llama-server --help`; the current stable upstream baseline tracks 248 parameter entries.

![参数搜索、预设、分组和校验提示 / Configuration search, presets, groups, and validation](public/docs/guide/06-configuration.png)

### 推荐操作 / Recommended Workflow

1. 在页面顶部确认当前实例。
2. 先使用场景预设作为起点，再按硬件调整。
3. 使用搜索框输入参数名或 CLI 标志，例如 `ctx`、`gpu-layers` 或 `api-key-file`。
4. 查看参数悬停提示和右侧活动参数摘要。
5. 点击保存并处理红、黄、蓝三级校验提示。

1. Confirm the selected instance.
2. Start from a scenario preset, then tune for the hardware.
3. Search by option name or CLI flag such as `ctx`, `gpu-layers`, or `api-key-file`.
4. Review tooltips and the active-parameter summary.
5. Save and address red, amber, or blue validation findings.

### 关键配置 / Important Settings

- 上下文越大，KV 缓存占用越高；显存紧张时优先降低上下文、批大小或 GPU 层数。
- API Key 可以直接填写，也可以通过 API Key 文件提供；健康检查、测试连接、指标读取和实例路由会使用有效密钥。
- 非本机监听会扩大访问范围，应配置鉴权并检查防火墙。
- 向量模型会锁定不适用的生成参数。
- `--spec-type` 支持从当前引擎能力生成逗号分隔的多选组合；llama.cpp 使用固定运行优先级，勾选顺序不代表执行顺序。含 `draft-*` 的组合仍需匹配的内置或外部草稿能力。
- 自定义参数会原样追加，使用前核对当前 `llama-server --help`。

- Larger context increases KV cache usage; reduce context, batch size, or GPU layers when memory is tight.
- API keys may be inline or file-based; health, connection tests, metrics, and routing use the effective key.
- Non-local binding increases exposure and should use authentication and firewall controls.
- Embedding models lock irrelevant generation options.
- `--spec-type` supports comma-separated multi-selection from the current engine's reported capabilities. llama.cpp uses a fixed runtime priority; selection order is not execution order. Any `draft-*` choice still requires compatible built-in or external draft support.
- Custom arguments are appended as entered and should be checked against the current `llama-server --help`.

### KV / Prefill 缓存检查点 / Cache Checkpoint

实验性 KV / Prefill Cache Checkpoint 默认关闭。它在受控停止本机受管实例时保存单个文本生成 slot，并在相同模型、引擎和强兼容配置重启后、管理器代理重新开放路由前完成验证与恢复。检查点失败只会进入 `Ready (cold)`，不会阻止实例启动。

The experimental KV / Prefill Cache Checkpoint is off by default. It saves one managed local text-generation slot on a controlled stop and verifies restore before the manager proxy reopens routing. Any failure falls back to `Ready (cold)` without blocking startup.

使用要求：

- `parallel = 1`，启用 prompt cache、slots、idle-slot cache，并让 Cache RAM 为正数或 `-1`。
- 滑动窗口模型启用 SWA 完整缓存；请先评估额外 KV 内存。
- 使用单文件或同目录完整分片集的文本 GGUF、本机受管 loopback HTTP 引擎；不要使用 router、多模型、Embedding、Reranker、LoRA 或 mmproj。自定义参数默认阻断；只有安全分类器明确认可的加载 I/O 参数可放行，当前为 `--lazy-mode` / `-lzm` / 旧 `--tensor-read-lazy` 的 `auto`、`on`、`off`。界面会列出其他具体阻断标志。
- 推测解码可以关闭，也可以选择当前引擎明确报告的组合。`ngram-*` 可从 target prompt 重建；`draft-*` 还要求引擎的 `--slot-save-path` 帮助明确包含 `slot KV cache and context checkpoints`，证明 target/draft context 会共同序列化。未知类型、`spec-default` 或外部 lookup cache 仍会安全回退冷启动。
- 已知 hybrid/recurrent 架构仍不支持。Qwen3.8-Flash-Next 的 `qwen4exp` 可正常使用同进程 prompt cache 和 `ngram-mod`，但 B10679 跨进程实测 restore 后 `cache_n = 0`，因此不能启用持久检查点。
- DeepSeek Harness 必须连接管理器代理而不是实例直连端口。Harness 的标题请求可能先占用 slot，idle-slot cache 和足够的 Cache RAM 用于保留刚恢复的长前缀。
- 从实例页查看 `Ready (restored)` / `Ready (cold)`、token、文件大小和原因；只有实例完全停止时才能清除数据。

Complete shard sets are fingerprinted as one logical model, including every shard. Rebuildable `ngram-*` types use the original slot format; `draft-*` additionally requires an engine that explicitly advertises target/draft context checkpoints. Custom arguments fail closed except for validated lazy-loading aliases and values. Hybrid/recurrent state remains unsupported. Checkpoint files contain sensitive prompt-derived state and are not portable session backups. Confirm actual benefit with llama.cpp `cache_n`, `n_past`, or prompt-evaluation metrics, not only a successful restore response. See [KV / Prefill Cache Checkpoint](docs/KV_CACHE_CHECKPOINT.md) for the compatibility matrix, lifecycle, privacy model, and troubleshooting steps.

---

## 集群管理 / Cluster Management

集群管理用于发现和维护 llama.cpp RPC Worker，并把 Worker 地址写入实例的 RPC 配置。支持局域网发现、TCP 扫描、本机启动和 SSH 远程启动。

Cluster Management discovers and maintains llama.cpp RPC workers and feeds worker addresses into instance RPC configuration. It supports LAN discovery, TCP scanning, local launch, and SSH launch.

![Worker 发现、网络信息和启动方式 / Worker discovery, network details, and launch methods](public/docs/guide/07-cluster-manager.png)

### 使用步骤 / Workflow

1. 扫描局域网 Worker，或手动添加主机和端口。
2. 测试连接并检查设备、内存和在线状态。
3. 本机 Worker 可选择引擎后启动；远程 Worker 需填写 SSH 连接与远端可执行路径。
4. 在实例参数配置中选择 Worker，生成 `rpc_servers`。
5. 启动实例后从日志确认 RPC 设备已连接。

1. Scan the LAN or manually add a worker host and port.
2. Test connectivity and review device, memory, and online state.
3. Launch a local worker from an engine, or provide SSH and remote executable details.
4. Select workers in instance configuration to generate `rpc_servers`.
5. Confirm RPC devices in logs after starting the instance.

USB4 适配器信息用于识别高速直连网络，但不会替代操作系统网络配置。扫描不到 Worker 时检查同网段、防火墙、RPC 端口和远端进程。

USB4 adapter details help identify high-speed direct links but do not replace OS network configuration. Check subnet, firewall, RPC port, and remote process when discovery fails.

Worker 地址支持 IPv4、主机名和 IPv6。手动填写带端口的 IPv6 地址时使用 `[::1]:50052` 形式，避免与 IPv6 地址自身的冒号混淆。

Worker addresses support IPv4, hostnames, and IPv6. When entering an IPv6 address with a port manually, use `[::1]:50052` so the port is unambiguous.

---

## 实例路由 / Instance Routing

实例路由在同一监听地址提供 OpenAI 与 Anthropic 原生 API 格式，根据精确公开模型名把流量转发到 `llama-server`。默认监听 `127.0.0.1:11435`，并集成严格模型边界、主动健康探测、熔断、调度、并发/限流和 Prometheus 指标。

Instance Routing exposes native OpenAI and Anthropic formats on one listener, routing exact public model IDs to `llama-server` with active probes, circuit breaking, scheduling, concurrency/rate controls, and Prometheus metrics. The default listener is `127.0.0.1:11435`.

![统一端点、路由规则和后端目标 / Unified endpoint, route rules, and backend targets](public/docs/guide/08-instance-routing.png)

### 配置和启动 / Configure and Start

1. 先启动至少一个后端实例。
2. 设置监听主机和端口。
3. 保持“严格模型路由”启用，为规则填写客户端使用的公开模型名并选择目标；相同模型名可配置多个优先级层。
4. 选择优先级故障切换、轮询、最空闲或加权调度；按工作负载设置全局与单目标并发、排队、健康探测和流式空闲超时。
5. 需要时添加具名 API Key，在首次保存前复制原文；小眼睛只会将尚未保存的明文显示 10 秒，保存后只保留不可逆摘要。
6. 保存并启动路由，然后用 `/ready`、`/v1/models`、`/slots?model=<公开模型名>`、OpenAI 或 Anthropic 客户端验证。

1. Start at least one backend instance.
2. Set the listen host and port.
3. Keep strict routing enabled, define exact public model IDs, and add priority tiers where failover is needed.
4. Choose priority failover, round-robin, least-busy, or weighted scheduling and set global/per-target traffic limits.
5. Add scoped API keys when needed and copy each plaintext secret before the first save; only a non-recoverable digest is persisted.
6. Start routing and validate `/ready`, `/v1/models`, `/slots?model=<public-model>`, and the native OpenAI or Anthropic client path.

![实例、别名和统一 API 的请求路径 / Request path from instances and aliases to the unified API](public/docs/guide/flow-03-route-requests.png)

### API 格式与 Claude Code / API Formats and Claude Code

| 客户端能力 / Client capability | 端点 / Endpoint |
|---|---|
| OpenAI 模型发现 / Model discovery | `GET /v1/models`、`GET /v1/models/:model_id` |
| OpenAI Chat Completions | `POST /v1/chat/completions` |
| OpenAI Responses | `POST /v1/responses` |
| OpenAI 输入 Token 计数 / Input token counting | `POST /v1/chat/completions/input_tokens`、`POST /v1/responses/input_tokens` |
| OpenAI Embeddings / Rerank | `POST /v1/embeddings`、`POST /v1/rerank` |
| Anthropic Messages，同步与 SSE 流式 / Messages, sync and SSE | `POST /v1/messages` |
| Anthropic 输入 Token 计数 / Input token counting | `POST /v1/messages/count_tokens` |
| 上下文与槽位发现 / Context and slots | `GET /props?model=...`、`GET /slots?model=...` |
| 存活、就绪与指标 / Liveness, readiness, metrics | `GET /live`、`GET /ready`、`GET /metrics` |

Anthropic 路径支持文本、图片、thinking、工具调用、工具结果和流式事件。工具调用要求目标实例启用 `--jinja`。Claude Code 应把 `ANTHROPIC_BASE_URL` 指向统一路由根地址（不要附加 `/v1`），并将 `ANTHROPIC_MODEL` 设为已配置的公开模型名；配置代理密钥时同时设置 `ANTHROPIC_AUTH_TOKEN`。

从 llama.cpp `b10354` 起，可在实例参数的服务扩展区域配置 `--tools-runtime`，把内置工具放入 Docker、Podman、已有容器或 SSH 目标中执行；留空时工具直接使用主机环境。该隔离能力仍为实验性功能，配置远程或容器运行时时仍需限制监听地址、CORS Origin 和 API Key。

`/v1/messages` 与 `/v1/messages/count_tokens` 必须携带 `anthropic-version: 2023-06-01`。官方 Anthropic SDK 会自动发送；自定义 HTTP 客户端缺失或发送其他版本时，路由器返回 `400 invalid_request_error`。

Anthropic routes support text, images, thinking, tool calls, tool results, and streaming events. Tool use requires `--jinja` on the target instance. Point Claude Code's `ANTHROPIC_BASE_URL` at the routing root without `/v1`, set `ANTHROPIC_MODEL` to a configured public model name, and set `ANTHROPIC_AUTH_TOKEN` when the proxy uses authentication.

Starting with llama.cpp `b10354`, configure `--tools-runtime` in the instance server-extension parameters to run built-in tools in Docker, Podman, an existing container, or an SSH target. An empty value uses the host environment. This isolation remains experimental, and remote/container runtimes still require restricted bind addresses, CORS origins, and API keys.

`/v1/messages` and `/v1/messages/count_tokens` require `anthropic-version: 2023-06-01`. Official Anthropic SDKs send it automatically; custom HTTP clients receive `400 invalid_request_error` when the header is missing or unsupported.

代理不会在 OpenAI 与 Anthropic 请求体之间互相转换；两种客户端调用各自的协议端点，但共享同一套路由规则、公开模型名和鉴权配置。`/props` 与 `/slots` 会透传必要的上下文和负载信息，同时移除模型路径、模板正文、提示词和 Token。prompt caching、服务端工具等云端专属能力不会由管理器模拟，最终能力取决于目标 `llama-server` 与模型。

`/v1/models` 会把后端运行时 `n_ctx` 同时公开为 `context_length`、`context_window` 和 vLLM 兼容的 `max_model_len`；首次探测前可使用明确的非自动 `ctx_size` 配置，故障转移目标不一致时采用安全最小值，任一目标未知时不猜测。对于 Chat Completions、Responses、Legacy Completions 和 Anthropic Messages，路由会优先调用目标原生 Token 计数接口，比较输入与最大输出之和；超出上下文时在进入推理前返回 `400`，OpenAI 使用 `context_length_exceeded`，Anthropic 使用 `invalid_request_error`。旧版目标缺少计数接口时继续转发，由目标执行最终校验，不使用字符数估算。

The proxy does not translate request bodies between OpenAI and Anthropic formats. Each client uses native endpoints while sharing exact public model IDs and authentication. `/props` and `/slots` preserve context/capacity discovery while removing paths, templates, prompts, and tokens. Cloud-only capabilities are not emulated by the manager.

`/v1/models` publishes the runtime `n_ctx` as `context_length`, `context_window`, and vLLM-compatible `max_model_len`, with an explicit non-auto `ctx_size` fallback before the first probe, the safe minimum across failover targets, and no guessed value when any target is unknown. Chat Completions, Responses, small Legacy Completions batches, and Anthropic Messages use the target's native token counter before inference; oversized requests receive an OpenAI `400 context_length_exceeded` or Anthropic `400 invalid_request_error`. Older targets without a compatible counter fail open to their own final validation rather than using an inaccurate character estimate.

完整端点、调度、错误格式、超时、权限与部署边界见[生产级模型路由器说明](docs/ROUTER_API_COMPATIBILITY.md)。

### 安全与后台保活 / Security and Background Keep-Alive

- 内置路由只允许监听回环地址；远程访问必须使用提供 TLS 的可信反向代理、Tunnel、VPN 或 SSH 隧道。
- CORS Origin 只决定“哪个网页可跨域调用”，API Key 决定“谁在调用”，路由决定“公开模型名去哪个实例”；三者互相独立，不需要绑定。
- Key 不绑定某个 Origin 或某条路由；拥有“推理”权限的 Key 可调用所有已发布路由。允许的 Origin 仍须携带有效 Key，桌面与 CLI 客户端则不受 CORS 限制。
- Origin 填浏览器地址栏中的“协议 + 主机 + 端口”，不含路径，例如 `http://localhost:3000`；多个值用逗号分隔并精确匹配。
- 路由依据实际 `/health` 探测和熔断状态选择目标，网络错误、`5xx` 或 `429` 会推动后续请求切换到健康层级。
- 未开启独立后台运行时，实例与路由仍由隔离的运行时进程托管，但主程序退出后会自动停止；仅关闭窗口则仍可继续在托盘运行。
- 开启“独立后台运行时”后，退出管理界面不会中断已托管实例或统一端点，并会注册当前用户登录后的自动恢复。Windows 使用当前用户启动项，macOS 使用用户 LaunchAgent，Linux 优先使用 systemd 用户服务并在不可用时回退到 XDG Autostart；第一阶段不安装需要管理员权限的系统级服务。
- 退出应用时会明确提供“退出界面并保持后台运行”与“停止实例、路由并退出”两种选择；运行时升级会保留运行意图，受控重启实例后恢复路由、日志和监控监督链。
- 登录恢复使用当前图形用户会话的标准环境，不会保存临时 Shell 环境变量；依赖自定义环境变量的引擎应改用稳定的系统/用户环境设置。
- 移动便携版程序或卸载前，请先关闭“独立后台运行时”，让程序清理当前用户的登录启动项。
- 第一阶段保证正常退出管理界面后持续运行，并在当前用户再次登录时恢复；它不是机器级高可用服务。管理界面打开时会自动拉起异常退出的运行时；界面已退出后若本次运行时自身崩溃，需要重新打开管理器或重新登录。macOS/Linux 在登录启动器接管后可继续使用其失败重启策略，Windows 登录项只负责登录恢复。
- 修改未保存的路由草稿后直接启动时，应用会先保存有效配置，避免界面与后台状态不一致。

- The built-in listener is loopback-only; use a trusted TLS reverse proxy, tunnel, VPN, or SSH tunnel for remote access.
- CORS Origin answers which web page may call across origins, an API key identifies the caller, and a route maps a public model to an instance. These layers are independent and need no binding.
- A key is not bound to an origin or one route. Inference-scoped keys may call every published route; allowed browser origins still need a valid key, while desktop and CLI clients that omit `Origin` are unaffected by CORS.
- Enter only the browser address bar's scheme, host, and port, such as `http://localhost:3000`. Separate multiple exact origins with commas and omit paths.
- Active `/health` probes and circuit state drive selection; network errors, `5xx`, and `429` move subsequent traffic to a healthy tier.
- Without the independent runtime option, an isolated runtime still supervises instances and routing while the app is open, but stops them after the main process exits; closing only the window keeps the tray session alive.
- Enabling the independent background runtime preserves managed instances and the unified endpoint after the management UI exits and registers per-user login recovery. Windows uses the current-user startup entry, macOS a user LaunchAgent, and Linux a systemd user service with XDG Autostart fallback. Phase one does not install an administrator-managed system service.
- Exit confirmation distinguishes keeping the runtime from stopping instances and routing. Runtime upgrades retain desired state and restore each instance under a fresh logging and monitoring supervisor.
- Login recovery uses the standard graphical user-session environment and does not capture temporary shell variables. Engines that depend on custom variables should use stable system or user environment settings.
- Before moving a portable installation or uninstalling, disable the independent runtime so the app can remove its per-user login entry.
- Phase one guarantees continuity after a normal UI exit and recovery at the next user login; it is not a machine-level high-availability service. The open UI relaunches a failed runtime. If a directly spawned runtime itself crashes after the UI has exited, reopen the manager or sign in again. A login-started macOS/Linux runtime can then use its launcher restart policy; the Windows login entry provides login recovery only.
- Starting with a valid unsaved draft persists it before launch.

---

## 性能监控 / Performance Monitoring

性能监控结合系统指标、实例指标、slots、日志时序和 SQLite 遥测，并按会话锁定生成、Embedding 或 Reranker 工作负载，展示与当前服务类型匹配的运行吞吐、历史基线和诊断建议。

Performance Monitoring combines system signals, instance metrics, slots, log timing, and SQLite telemetry. Each session is pinned to its generation, Embedding, or Reranker workload so throughput, history, and diagnostics keep the correct meaning after configuration changes.

![实例选择、资源指标、吞吐和诊断 / Instance selection, resource metrics, throughput, and diagnostics](public/docs/guide/09-performance.png)

### 查看指标 / Read the Metrics

1. 从左侧选择正在运行的实例，工作负载徽标会显示本次会话实际记录的服务类型。
2. CPU、内存、GPU 和显存是三类工作负载共用的资源信号。
3. 生成模型查看输出 tokens/s、提示处理速度、排队深度和忙碌槽位。
4. Embedding：输入 tokens/s、向量项/s 使用最近 60 秒工作窗口，并显示任务耗时 P50 / P95 和整段会话已完成向量项数。
5. Reranker：输入 tokens/s、文档项/s 使用最近 60 秒工作窗口，并显示任务耗时 P50 / P95 和整段会话已完成文档项数。
6. 任务日志覆盖应用内代理请求和绕过代理的直连请求，用于统计任务吞吐与耗时；代理请求统计仅覆盖经过实例路由的 HTTP 请求，并补充请求数、失败率及请求耗时。
7. 页面会分别标明日志来源和代理来源。某个来源没有可确认的数据时显示“不可用”或 `--`，不会用 `0` 冒充已测量结果；已测量时间桶内确实空闲时才显示零值。
8. 历史基线只比较相同模型、工作负载和后端的会话。向量服务的诊断聚焦资源压力、吞吐和 P95 变化，不显示生成模型专用的 KV 缓存、上下文和解码建议。

1. Select a running instance; the workload badge shows the service type recorded for that session.
2. CPU, memory, GPU, and VRAM are common resource signals across all workloads.
3. Generation workloads show output tokens/s, prompt processing speed, queue depth, and busy slots.
4. Embedding: input tokens/s and vector items/s over the latest 60-second work window, plus task P50/P95 latency and session-wide completed vector items.
5. Reranker: input tokens/s and document items/s over the latest 60-second work window, plus task P50/P95 latency and session-wide completed document items.
6. Task logs cover both proxied requests and direct requests that bypass the app proxy, providing task throughput and latency. Proxied request telemetry covers only HTTP traffic routed by the app and adds request count, failure rate, and request latency.
7. Log and proxy source availability are shown separately. Missing evidence is labeled unavailable or `--`, never presented as a measured zero; zero is used only for an observed idle bucket.
8. Historical baselines compare sessions with the same model, workload, and backend. Vector diagnostics focus on resource pressure, throughput, and P95 changes without generation-only KV-cache, context, or decoding advice.

AMD 指标优先使用 ADLX，NVIDIA 使用 NVML，无法取得 GPU 指标时会回退到系统指标。页面没有实例时不会保留上一个实例的过期实时数据。

AMD signals prefer ADLX and NVIDIA uses NVML; the app falls back to system metrics when GPU telemetry is unavailable. Stale live instance metrics are cleared when no instance is selected.

---

## 监控大屏 / Monitoring Wall

监控大屏把实例、吞吐、请求压力、下载、日志和告警压缩到一页，适合持续观察，不用于修改配置。

Monitoring Wall condenses instances, throughput, request pressure, downloads, logs, and alerts into one read-only operational view.

![监控大屏的服务健康、吞吐、压力和活动 / Monitoring wall with health, throughput, pressure, and activity](public/docs/guide/10-monitoring-wall.png)

- 顶部显示更新时间和整体服务状态。
- KPI 展示运行实例、当前与峰值吞吐、请求压力和告警数。
- 实例吞吐按实例聚合，避免把同一请求或排队数重复计算。
- 下载和日志区域用于发现近期失败，不替代下载页或日志页的详细操作。
- 数据不可用时显示真实空状态或降级信息，不填充模拟指标。

- The header shows update time and overall service status.
- KPIs cover running instances, current and peak throughput, request pressure, and alerts.
- Throughput is aggregated per instance without double-counting request or queue data.
- Download and log summaries surface recent failures but do not replace detailed pages.
- Unavailable data remains an honest empty or degraded state.

---

## 服务器日志 / Server Logs

服务器日志集中显示实例 stdout / stderr、启动命令、PID、健康检查和性能时序。日志按实例写入文件，应用重启后可恢复查看。

Server Logs collects instance stdout and stderr, startup commands, PIDs, health checks, and timing output. Per-instance logs persist and can be restored after restart.

![实时日志、实例筛选和自动跟随 / Live logs, instance filtering, and tail follow](public/docs/guide/11-server-logs.png)

### 使用方法 / Usage

- 按实例筛选，或查看全部实例。
- 自动跟随开启时保持在最新日志；向上滚动会暂停跟随，返回底部后可恢复。
- 错误、警告、就绪和性能关键词使用不同颜色。
- 清空日志只清理当前视图对应内容，操作前确认筛选范围。
- 启动失败先查完整命令，再查紧随其后的 stderr。

- Filter by one instance or view all.
- Tail follow stays on the newest line; scrolling up pauses it until returning to the bottom.
- Errors, warnings, readiness, and performance terms use distinct colors.
- Clear applies to the selected log scope.
- For startup failures, inspect the full command and the following stderr lines.

常见信号：`address already in use` 表示端口冲突；模型文件打开失败表示路径或权限问题；GPU 分配失败通常需要降低 GPU 层数、上下文或批大小；健康接口暂时返回错误但模型接口可用时，连接测试会使用兼容回退判断。

Common signals include port conflicts, model path or permission errors, GPU allocation failures, and transient health endpoint errors. Connection tests can use compatible model endpoint fallback when appropriate.

---

## 常见问题 / FAQ

### 为什么没有检测到引擎？ / Why is no engine detected?

进入“引擎管理”，添加包含实际 `llama-server` 可执行文件的根目录并重新扫描。若只选择了源码目录而没有构建产物，不会识别为引擎。

Add and rescan a root containing the actual `llama-server` executable. A source-only directory is not an engine.

### 为什么实例端口不可用？ / Why is the instance port unavailable?

该端口正在被其他实例或进程监听。选择新端口，或停止占用端口的进程后等待检查刷新。实例路由端口也不能与后端实例端口重复。

Another process is listening on the port. Choose another port or stop the owner. The routing listener also needs a unique port.

### 为什么实例立即进入错误状态？ / Why does an instance fail immediately?

打开服务器日志，检查启动命令后的第一条错误。依次核对模型路径、引擎路径、后端类型、端口、GPU 层数、上下文和可用内存。

Open Server Logs and inspect the first error after the startup command. Check model and engine paths, backend, port, GPU layers, context, and memory.

### API 返回未授权怎么办？ / What if the API returns unauthorized?

确认客户端使用的是实例 API Key 或实例路由的代理 API Key。若实例通过 `api_key_file` 提供密钥，检查文件存在、可读，并确认第一条非空内容正确。

Use the instance key or the routing proxy key as appropriate. For `api_key_file`, verify the file and its first non-empty line.

### 为什么非本机路由无法启动？ / Why can routing not bind publicly?

内置实例路由有意只接受 `127.0.0.1`、`localhost` 或 `::1`。请恢复回环监听，并通过本机 TLS 反向代理、Cloudflare Tunnel、VPN 或 SSH 隧道向其他设备提供服务。

The built-in router intentionally accepts loopback hosts only. Restore a loopback listener and expose it through a local TLS reverse proxy, Cloudflare Tunnel, VPN, or SSH tunnel.

### 应用重启后下载怎么办？ / What happens to downloads after restart?

队列和断点状态会保存。默认“手动”策略等待你点击恢复；选择“启动时自动恢复”后，符合条件的任务会自动继续。

Queue and partial state persist. Manual policy waits for user action; Auto on Launch resumes eligible tasks.

### 性能页为什么没有 GPU 数据？ / Why is GPU telemetry missing?

确认驱动正常并且当前平台可使用 ADLX 或 NVML。采集失败时应用会回退系统指标；这不一定表示实例未使用 GPU，应结合服务器日志确认后端加载。

Verify the driver and ADLX or NVML availability. System fallback does not by itself mean the server is not using a GPU; confirm backend loading in logs.

macOS 当前没有 ADLX 或 NVML 数据源，因此 Apple GPU/统一内存不会显示为独立显存指标；CPU、系统内存、吞吐、slots 和日志遥测仍可使用。

macOS currently has no ADLX or NVML source, so Apple GPU and unified memory are not reported as separate VRAM metrics. CPU, system memory, throughput, slots, and log telemetry remain available.

### 主配置损坏怎么办？ / What if the main configuration is corrupt?

应用会自动尝试 `instances.json.bak`。仍无法启动时，先备份整个配置目录，再检查两个 JSON 文件；不要直接删除日志、下载状态或遥测数据库来猜测修复。

The app automatically tries `instances.json.bak`. If recovery still fails, back up the full configuration directory before inspecting both JSON files.

### 应用内图片是否需要联网？ / Do in-app guide images require a network connection?

不需要。说明截图位于安装包的 `docs/guide` 资源目录中；GitHub README 和手册也引用仓库内同一批文件。

No. Guide images ship under `docs/guide` in the frontend bundle and are shared with repository documentation.
