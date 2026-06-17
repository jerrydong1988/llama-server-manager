# Llama Server Manager 使用教程 / User Guide

> v2.9.11

---

## 目录 / Table of Contents

1. [快速开始 / Quick Start](#快速开始--quick-start)
2. [模型仓库 / Model Repository](#模型仓库--model-repository)
3. [下载管理 / Download Manager](#下载管理--download-manager)
4. [引擎管理 / Engine Management](#引擎管理--engine-management)
5. [实例管理 / Instance Management](#实例管理--instance-management)
6. [参数配置 / Parameter Configuration](#参数配置--parameter-configuration)
7. [性能监控 / Performance Monitoring](#性能监控--performance-monitoring)
8. [服务器日志 / Server Logs](#服务器日志--server-logs)
9. [集群管理 / Cluster Management](#集群管理--cluster-management)
10. [高级功能 / Advanced Features](#高级功能--advanced-features)
11. [常见问题 / FAQ](#常见问题--faq)

---

## 快速开始 / Quick Start

### 下载与安装 / Download & Install

1. 从 [GitHub Releases](https://github.com/jerrydong1988/llama-server-manager/releases/latest) 下载最新安装包
2. 双击安装或直接运行便携版
3. 确保系统已安装 [llama.cpp](https://github.com/ggerganov/llama.cpp) 的 `llama-server`

1. Download the latest installer from [GitHub Releases](https://github.com/jerrydong1988/llama-server-manager/releases/latest)
2. Double-click to install or run portable version
3. Make sure you have `llama-server` from [llama.cpp](https://github.com/ggerganov/llama.cpp) installed

### 界面布局 / Interface Layout

程序有 8 个标签页，左侧导航栏按操作流程排列：

```
模型仓库 → 下载管理 → 引擎管理 → 实例管理 → 参数配置 → 集群管理 → 性能监控 → 服务器日志
```

**建议操作顺序：**

1. ① **模型仓库**：添加你存放 GGUF 模型的本地目录
2. ② **下载管理**：从 ModelScope/HuggingFace 下载模型（如无可跳过）
3. ③ **引擎管理**：添加 llama-server 所在的目录
4. ④ **实例管理**：创建一个实例（选择模型 + 引擎 + 端口）
5. ⑤ **参数配置**：根据需要调整启动参数
6. ⑥ **启动实例**：回到实例管理，点击启动
7. ⑦ **性能监控**：查看实时指标和性能分析

The app has 8 tabs in the sidebar, ordered by workflow:

```
Model Repo → Downloads → Engines → Instances → Config → Cluster → Performance → Server Logs
```

**Recommended workflow:**

1. ① **Model Repo**: Add your local GGUF model directories
2. ② **Downloads**: Download models from ModelScope/HuggingFace (skip if already have models)
3. ③ **Engines**: Add your llama-server directory
4. ④ **Instances**: Create an instance (select model + engine + port)
5. ⑤ **Config**: Adjust startup parameters as needed
6. ⑥ **Start**: Go back to Instances and click Start
7. ⑦ **Performance**: View real-time metrics and performance analysis

---

## 模型仓库 / Model Repository

### 添加模型目录 / Adding Model Directories

点击「添加模型目录」，选择你存放 `.gguf` 模型文件的文件夹。程序会递归扫描子目录（最多 5 层）。

Click "Add Directory" and select the folder containing your `.gguf` model files. The app will recursively scan subdirectories (up to 5 levels deep).

### 模型树结构 / Model Tree Structure

扫描完成后，模型按实际文件系统层级展示为树状结构：

```
📂 C:\Models\llm\
  📁 unsloth\
    📁 Qwen3.6-35B-A3B-GGUF\
      📄 Qwen3.6-35B-A3B-Q4_K_M.gguf     模型    Q4_K    18.2 GB
      📷 mmproj-BF16.gguf                  投影器   BF16    640 MB
```

- 📂 黄色文件夹 = 根目录 / Root directory
- 📁 = 子目录 / Subdirectory（可按需折叠/展开 / collapsible）
- 📄 蓝色 = 模型文件 / Model file
- 📷 紫色 = 多模态投影器 / Multimodal projector (mmproj)

### GGUF 元信息 / GGUF Metadata

程序自动从 GGUF 文件头读取以下信息：

- **架构** / Architecture（如 llama、qwen、mistral）
- **上下文长度** / Context Length（如 32768）
- **量化类型** / Quantization Type（如 Q4_K、Q4_K_XL、F16、BF16）
- **MTP 推测解码** / MTP speculative decoding support

### 管理操作 / File Operations

- 📂 按钮 = 在资源管理器中打开 / Open in Explorer
- 🗑 按钮 = 确认后从磁盘永久删除 / Delete permanently from disk (with confirmation)

---

## 下载管理 / Download Manager

下载管理是**独立的模型下载中心**，支持 ModelScope（魔搭社区）和 HuggingFace 双源下载。

The Download Manager is a **dedicated download center** supporting both ModelScope and HuggingFace.

### 浏览仓库 / Browse Repo

1. 选择下载源（ModelScope / HuggingFace）
2. 输入仓库 ID（如 `unsloth/Qwen3.6-35B-A3B-GGUF`）
3. 点击「Browse」浏览文件列表
4. 设置保存目录（默认 `models`，支持文件夹选择器）

1. Select source (ModelScope / HuggingFace)
2. Enter repo ID (e.g. `unsloth/Qwen3.6-35B-A3B-GGUF`)
3. Click "Browse" to list files
4. Set save directory (default `models`, folder picker supported)

### 下载操作 / Download Operations

| 操作 | 说明 |
|------|------|
| 单文件下载 | 点击文件旁的 Download 按钮 |
| 全部下载 | 点击 Download All（自动跳过已完成的文件） |
| 暂停 | 下载中点击 Pause，保留已下载进度 |
| 继续 | 点击 Resume，从断点续传 |
| 取消 | 点击 Cancel 删除文件 |

| Operation | Description |
|-----------|-------------|
| Single download | Click Download next to each file |
| Download All | Click Download All (auto-skips completed files) |
| Pause | Click Pause during download, keeps partial progress |
| Resume | Click Resume to continue from breakpoint |
| Cancel | Click Cancel to delete the file |

### 智能特性 / Smart Features

- **并发控制**：最多 3 个文件同时下载，其余排队
- **已下载检测**：浏览时自动识别本地已存在的文件，标记为 ✓ Done，避免重复下载
- **下载路径记忆**：保存目录设置会在下次启动时自动恢复
- **断点续传**：支持 Range 请求，中断后从上次位置继续

- **Concurrency**: Max 3 simultaneous downloads, others queued
- **Downloaded detection**: Auto-detects locally existing files, marks as ✓ Done
- **Path memory**: Save directory setting persists across restarts
- **Resume support**: Range requests for continuing interrupted downloads

---

## 引擎管理 / Engine Management

### 添加引擎目录 / Adding Engine Directories

点击「添加引擎根目录」，选择包含 `llama-server` 的目录。程序会递归扫描子目录，自动发现所有版本。

Click "Add Engine Directory" and select the folder containing `llama-server`. The app scans subdirectories to auto-discover all versions.

### 引擎类型识别 / Backend Detection

| 标记 | 说明 |
|------|------|
| CUDA | NVIDIA GPU 加速 |
| ROCm | AMD GPU 加速 |
| Vulkan | 跨平台 GPU 加速 |
| CPU | 纯 CPU 推理 |

### 设为默认引擎 / Set as Default

点击引擎卡片下的「设为默认」按钮。新创建的实例会自动选用默认引擎。

Click "Set as Default" on an engine card. New instances will use this engine by default.

---

## 实例管理 / Instance Management

### 创建实例 / Creating an Instance

1. 点击「创建实例」
2. 填写实例名称
3. 点击 📂 按钮从模型仓库树中选择模型
4. 选择引擎
5. 设置端口（程序会自动检测端口冲突）
6. 点击「创建」

1. Click "Create Instance"
2. Enter a name
3. Click 📂 to select a model from the repository tree
4. Select an engine
5. Set port (port conflict auto-detected)
6. Click "Create"

### 启动/停止 / Start / Stop

- 绿色 `▶ 启动` → 红色 `■ 停止`
- 状态图标：◯ 已停止 / ◌ 启动中 / ✓ 运行正常 / ✕ 错误

- Green `▶ Start` → Red `■ Stop`
- Status: ◯ Stopped / ◌ Starting / ✓ Healthy / ✕ Error

### 实例卡片功能 / Instance Card Functions

| 按钮 | 功能 |
|------|------|
| ▶/■ | 启动/停止 |
| 🌐 | 在浏览器中打开 API 页面 |
| 📶 | 测试连接 |
| ⌨ | 查看生成的命令行 |
| ⚙ | 跳转到参数配置页 |
| ↑↓ | 排序 |
| ✏️ | 编辑名称 |
| 🗑 | 删除实例 |

### 快捷键 / Keyboard Shortcuts

| 快捷键 | 功能 |
|--------|------|
| `Ctrl + Enter` | 启动/停止实例 |
| `Ctrl + S` | 保存全部配置 |

---

## 参数配置 / Parameter Configuration

### 配置页结构 / Config Page Structure

配置页包含 **6 个折叠组**，覆盖 **159 个参数**：

| 组 | 内容 |
|------|------|
| **基本参数** Basic | 模型路径、LoRA、投影器、聊天模板、推理参数 |
| **生成参数** Generation | 温度、Top-K/P、重复惩罚、种子、Min-P 等 |
| **高级采样** Advanced Sampling | Mirostat、XTC、DRY、动态温度（总开关控制） |
| **性能 & 上下文** Performance | 线程、GPU 层数、批处理、上下文、RoPE/YaRN、Flash Attention |
| **服务 & 网络** Server | 主机、端口、API 密钥、SSL、Embedding |
| **高级参数** Advanced | 内存、KV 缓存、推测解码、GPU 设备、多模型路由 |

### 智能参数校验 / Smart Validation

保存配置时自动检测 **26 条规则**，按严重度分三级：

| 严重度 | 颜色 | 示例 |
|--------|------|------|
| **高** | 红色 | Backend Sampling 在 ROCm GPU 上会产生警告 |
| **中** | 黄色 | 上下文大小超过模型原始上下文 4 倍 |
| **低** | 蓝色 | 同时设置聊天模板和模板文件（冗余） |

### 参数悬停提示 / Tooltips

所有参数名称和输入框都有中英双语悬停提示，说明参数作用和推荐值。

All parameter labels have bilingual hover tooltips explaining purpose and recommended values.

---

## 性能监控 / Performance Monitoring

### 实时指标 / Real-time Metrics

- **CPU / 内存**：进程占用 + 系统总量对比
- **GPU / 显存**：AMD（ADLX）/ NVIDIA（NVML）自动检测
- **推理指标**：tokens/s、提示速度、排队深度、活跃槽位
- **自适应降级**：ADLX → NVML → sysinfo

- **CPU / Memory**: Process usage + system total
- **GPU / VRAM**: AMD (ADLX) / NVIDIA (NVML) auto-detection
- **Inference**: tokens/s, prompt speed, queue depth, busy slots
- **Graceful degradation**: ADLX → NVML → sysinfo

### 性能分析面板 / Performance Analysis

从日志中**实时提取**每请求的性能剖面：

- **生成进度**：当前 token 数 + 预估总量，tg（瞬时生成速度）
- **速度曲线**：n_decoded vs tg 的 SVG 实时折线图
- **推测解码**：接受率 + 接受/生成 token 数
- **完成汇总**：提示阶段（tokens、耗时、t/s）+ 生成阶段 + 总计时

Real-time **per-request profiling** extracted from server logs:

- **Progress**: current tokens + estimated total, tg (instant gen speed)
- **Speed curve**: SVG line chart of n_decoded vs tg
- **Spec decode**: acceptance rate + accepted/generated counts
- **Summary**: prompt phase (tokens, time, t/s) + gen phase + total

---

## 服务器日志 / Server Logs

- 实时显示实例 stdout/stderr 输出
- 关键词自动高亮：红色=错误、黄色=警告、绿色=就绪、青色=性能
- **自动滚动**到最新输出（跟随模式）
- **上滑暂停**：手动上滑查看历史时自动暂停
- **回到底部**：暂停时浮出「⬇ 最新」按钮
- 实例启动时自动打印完整命令行 + PID
- 可按实例筛选 / 清空日志
- **应用重启后日志无缝恢复**

- Real-time stdout/stderr display
- Keyword highlighting: red=error, yellow=warning, green=ready, cyan=performance
- Auto-scroll follow mode + pause on scroll up
- Startup command + PID auto-logged
- Filter by instance / clear logs
- **Logs recover seamlessly after app restart**

---

## 集群管理 / Cluster Management

### 功能概述 / Overview

- **局域网扫描**：自动发现局域网内运行 rpc-server 的 Worker
- **本地启动**：一键启动本机 rpc-server
- **SSH 远程启动**：通过 SSH 在远程机器启动 Worker
- **USB4 网卡检测**：自动检测高速直连网络适配器
- **Worker 管理**：添加/删除/测试连接/查看设备信息

- **LAN scan**: Auto-discover Workers running rpc-server on LAN
- **Local launch**: One-click start rpc-server on this machine
- **SSH remote**: Launch Worker on remote machines via SSH
- **USB4 detection**: Auto-detect high-speed direct network adapters
- **Worker management**: Add/remove/test connection/view device info

---

## 高级功能 / Advanced Features

### 中英双语 / i18n

侧边栏底部的 `EN`/`中` 按钮可切换界面语言。所有界面文字完整支持中英双语，包括集群管理和性能分析面板。

The `EN`/`中` button switches the entire UI between Chinese and English, covering all pages including cluster management and performance analysis.

### 主题切换 / Theme Toggle

点击 ☀/🌙 按钮切换深色/浅色主题。主题偏好持久化。

Click ☀/🌙 to toggle dark/light theme. Preference persists.

### 系统托盘 / System Tray

关闭窗口时程序最小化到系统托盘。左键点击托盘图标显示窗口，右键显示菜单。

Closing the window minimizes to system tray. Left click = show, right click = menu.

### 配置文件 / Config Persistence

所有配置自动保存在 `configs/` 目录的 JSON 文件中。采用**原子写入**（先写临时文件再 rename），崩溃不损坏配置。每次保存自动创建 `.bak` 备份。

All configs auto-saved as JSON in `configs/`. Uses **atomic write** (tmp → rename), crash-safe. Auto `.bak` backup on every save.

### NVIDIA GPU 支持 / NVIDIA GPU Support

程序通过 NVML（NVIDIA Management Library）自动检测 NVIDIA GPU，无需额外配置。GPU 利用率和显存占用实时显示在性能监控页。

The app auto-detects NVIDIA GPUs via NVML. GPU utilization and VRAM usage are displayed in real-time on the Performance page.

### 自动更新检查 / Auto-update

程序启动时自动检测 GitHub Release 是否有新版本。如有更新，侧边栏底部显示绿色提示横幅。

The app auto-checks GitHub Releases on startup. If an update is available, a green banner appears at the bottom of the sidebar.

---

## 常见问题 / FAQ

**Q: 为什么启动后提示"未检测到引擎"？**
A: 请先在「引擎管理」中添加 llama-server 所在的目录。

**Q: Why does it say "No engines detected"?**
A: Go to "Engine Management" and add the directory containing llama-server first.

---

**Q: 如何添加多个版本的引擎？**
A: 选择包含多个引擎子目录的父级目录，程序会自动发现所有版本。

**Q: How to add multiple engine versions?**
A: Select the parent directory containing multiple engine subdirectories - the app auto-discovers all.

---

**Q: 如何从 ModelScope / HuggingFace 下载模型？**
A: 进入「下载管理」页面，选择下载源，输入仓库 ID，点击 Browse 浏览文件后进行下载。

**Q: How to download models from ModelScope / HuggingFace?**
A: Go to "Downloads" page, select source, enter repo ID, click Browse, then download.

---

**Q: 配置文件在哪里？**
A: 在 `configs/` 文件夹中。`instances.json` 是主配置文件。

**Q: Where are the config files?**
A: In the `configs/` folder. `instances.json` is the main config file.

---

**Q: 如何同时运行多个模型？**
A: 创建多个实例，设置不同的端口即可。每个实例独立运行。

**Q: How to run multiple models simultaneously?**
A: Create multiple instances with different ports. Each runs independently.

---

**Q: NVIDIA GPU 需要额外配置吗？**
A: 不需要。程序通过 NVML 自动检测，驱动正常安装即可。

**Q: Does NVIDIA GPU need extra configuration?**
A: No. The app auto-detects via NVML as long as the driver is installed.

---

**Q: 应用重启后日志还在吗？**
A: 在。程序将 llama-server 的输出写入日志文件，重启后自动恢复。

**Q: Are logs preserved after app restart?**
A: Yes. The app writes llama-server output to log files and auto-recovers them on restart.
