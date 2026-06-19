# Llama Server Manager / Llama 服务器管理器
## Windows · macOS · Linux 三平台均已实测验证通过
> 现代化 Llama.cpp 服务器图形化管理器 | Modern GUI for managing llama.cpp servers

一个功能完整的桌面应用程序，用于管理 `llama-server` 的全生命周期：**下载模型 → 选择引擎 → 配置参数 → 启动监控 → 性能分析**。告别复杂的命令行参数。

A fully-featured desktop application for managing the `llama-server` lifecycle: **download models → select engines → configure parameters → start & monitor → performance analytics**. Say goodbye to complex CLI arguments.

[📥 **下载最新版 / Download Latest**](https://github.com/jerrydong1988/llama-server-manager/releases/latest) | [📖 **使用教程 / User Guide**](GUIDE.md)

---

## 界面预览 / Screenshots
<img width="1724" height="1390" alt="image" src="https://github.com/user-attachments/assets/23ab9965-8939-44de-a0d5-c32043c31886" />
<img width="1724" height="1390" alt="image" src="https://github.com/user-attachments/assets/ce91cf93-d65e-4ca9-be42-f5e243090dab" />
<img width="1724" height="1390" alt="image" src="https://github.com/user-attachments/assets/d9fefe94-5221-4cf4-aad5-85b09bcaa16d" />
<img width="1724" height="1390" alt="image" src="https://github.com/user-attachments/assets/97700da9-3910-41e1-87a2-fc64060085f6" />
<img width="1724" height="1390" alt="image" src="https://github.com/user-attachments/assets/33e1d815-fbce-47bd-947e-8b0cb13ee0cc" />
<img width="1724" height="1390" alt="image" src="https://github.com/user-attachments/assets/488b1263-18e5-42bf-8e4c-02e021b233e1" />
<img width="1724" height="1390" alt="image" src="https://github.com/user-attachments/assets/c9692084-c6f4-4ccd-bc37-2b6f7656ec2b" />
<img width="1724" height="1390" alt="image" src="https://github.com/user-attachments/assets/06d17944-06ec-4139-bc81-e2f89bc59d3b" />
<img width="1724" height="1390" alt="image" src="https://github.com/user-attachments/assets/9618ded1-d360-4c94-8b81-b62a447d15e0" />


- 明暗主题切换（持久化）+ 窗口尺寸/位置记忆
<img width="1724" height="1390" alt="image" src="https://github.com/user-attachments/assets/321176a3-9bd5-4b37-bcb7-c465910f7f6b" />
- 完整中英双语界面
<img width="1724" height="1390" alt="image" src="https://github.com/user-attachments/assets/025c8056-22ca-4778-a24a-b74b3f010303" />







---

## 功能特性 / Features

### 性能监控 / Performance Monitoring
- **系统健康仪表**：总览页启动即显示 CPU/内存/GPU/显存实时数据（不依赖运行实例）
- 实时 CPU/GPU/显存指标（AMD GPU 通过 ADLX，NVIDIA GPU 通过 NVML）
- 系统级 CPU/RAM 上下文（进程占用 / 系统总量对比）
- 推理吞吐（tokens/s / 提示速度 / 排队深度 / 活跃槽位）
- 累计 token 统计（提示+生成+总计）
- **性能分析面板**：日志驱动的每请求剖面（提示/生成阶段吞吐、推测解码接受率、速度曲线）
- 自适应降级：ADLX → NVML → sysinfo → 静默回退
- **System health dashboard**: CPU/Memory/GPU/VRAM visible immediately on launch (no instance required)
- Real-time CPU/GPU/VRAM metrics (AMD via ADLX, NVIDIA via NVML)
- System-level CPU/RAM context (process vs total)
- Inference throughput (tokens/s, prompt speed, queue depth, busy slots)
- Cumulative token stats (prompt + generation + total)
- **Performance Analysis panel**: per-request profiling from logs (prompt/gen throughput, spec decode acceptance rate, speed curve)
- Graceful degradation: ADLX → NVML → sysinfo → silent fallback
  
### 模型仓库 / Model Repository
- 多目录递归扫描，支持 LM Studio / NovaMax 等任意目录结构
- GGUF 元信息自动解析（架构 / 上下文长度 / 量化类型）
- 自适应递归树结构，按实际文件系统层级展示
- **分片模型智能检测**：自动识别 -NNNNN-of-NNNNN 分片文件，Dashboard 排除重复统计
- 一键在资源管理器中打开、从磁盘删除
- Multi-directory recursive scanning, supports LM Studio/NovaMax
- Automatic GGUF metadata parsing (architecture / context length / quantization)
- Adaptive recursive tree matching actual filesystem hierarchy
- **Sharded model detection**: auto-identifies shard files, excludes from model count
- Open in Explorer, delete from disk

### 下载管理 / Download Manager
- 独立的下载管理页面，统一管理所有模型下载任务
- 支持 ModelScope（魔搭社区）和 HuggingFace 双源浏览和下载
- 下载队列管理，最多 3 并发，支持暂停/继续/取消
- 断点续传，500ms 限流进度推送，速度 + ETA 显示
- 自动检测本地已下载文件，避免重复下载
- Standalone download manager for all model download tasks
- ModelScope + HuggingFace dual-source browsing and downloading
- Download queue with 3 concurrent limit, pause/resume/cancel
- Resume support with 500ms throttled progress, speed + ETA display
- Auto-detect locally downloaded files to prevent re-download

### 引擎管理 / Engine Management
- 多引擎递归扫描，自动识别子目录中的 `llama-server`
- 后端类型自动检测（CUDA / ROCm / Vulkan / CPU）
- 多引擎根目录管理，设为默认引擎
- Recursive engine scanning, auto-discovers llama-server in subdirectories
- Automatic backend detection (CUDA / ROCm / Vulkan / CPU)
- Multi-root directory management, set default engine

### 实例管理 / Instance Management
- 多实例并行运行，每个实例独立配置
- 每个实例可独立选择引擎，覆盖全局默认
- 一键启停，进程实时监控（1 秒快速故障检测），测试连接
- 健康检查，自动识别启动失败
- 生成命令行预览，一键复制/直接启动
- 一键在浏览器打开 API 页面
- 实例排序（↑↓），名称可编辑（✏️）
- 键盘快捷键：`Ctrl+Enter` 启动/停止，`Ctrl+S` 保存配置
- Multiple parallel instances with independent configs
- Per-instance engine selection, overriding global default
- One-click start/stop with real-time process monitoring (1s fault detection), test connection
- Health check with automatic startup failure detection
- Command-line preview with copy & direct launch
- Open API page in browser
- Instance reordering (↑↓), name editing (✏️)
- Keyboard shortcuts: `Ctrl+Enter` start/stop, `Ctrl+S` save configs

### 参数配置 / Configuration
- 统一参数配置页面，按实例关联
- **159 个参数**完整覆盖 llama.cpp 最新主分支（71% 覆盖率，覆盖全部生成/采样/缓存/推测解码/服务核心场景），含 RoPE/YaRN 上下文缩放、推测解码、GPU 设备、多模型路由等全部参数
- **智能参数校验**：保存时自动检测 26 条规则，分 high/medium/low 三级彩色横幅警告
- **参数搜索**：输入参数名或 CLI 标志（如 -cms），自动展开分组、amber 高亮匹配字段、脉冲闪现 + 自动滚屏定位
- 高级采样总开关 + 推测解码/专家设置子开关，一键启用/禁用相关参数
- 向量模型自动检测，锁定不适用参数
- 模型路径仓库树选择器，自动关联 mmproj
- 所有参数带悬停提示说明（中英双语）
- Unified config page associated per instance
- **159 parameters** covering llama.cpp latest master, including RoPE/YaRN scaling, speculative decoding, GPU devices, multi-model routing
- **Smart validation**: 26 rules auto-checked on save, high/medium/low severity color-coded banners
- **Parameter search**: type name or CLI flag, auto-expand sections, amber highlight + flash animation + auto-scroll
- Advanced sampling master toggle + speculative decoding/expert settings sub-toggles
- Embedding model auto-detection with irrelevant parameter locking
- Model path tree picker with automatic mmproj association
- Bilingual tooltip hints on all parameters

### 其他 / Other
- 服务器日志实时捕获，关键词彩色高亮，**自动滚动 + 暂停/恢复**
- 启动命令自动记录到日志（含完整命令行和 PID）
- 系统托盘支持，关闭窗口隐藏到托盘
- **单实例检测**：重复打开自动聚焦已有窗口
- **Tauri 原生确认对话框**：删除/移除操作均有原生警告弹窗
- 明暗主题切换（持久化）+ 窗口尺寸/位置记忆
- 完整中英双语界面
- 配置持久化（JSON）+ **自动备份恢复** + 自动更新检查
- 模型并发下载限制（避免资源耗尽）
- 端口冲突检测
- Real-time server log capture with keyword color highlighting, **auto-scroll + pause/resume**
- Startup command auto-logged to console (full CLI + PID)
- System tray support, close to tray
- **Single instance detection**: auto-focus existing window on re-launch
- **Tauri native confirm dialogs**: native warning popups for all delete/remove operations
- Light/dark theme toggle (persistent) + window size/position memory
- Full i18n support (Chinese / English)
- Config persistence (JSON) + **auto backup/restore** + auto-update check
- Concurrent download limiting (prevents resource exhaustion)
- Port conflict detection

---

## 技术栈 / Tech Stack

| 层 / Layer | 技术 / Technology |
|-------------|-------------------|
| 后端 / Backend | Rust + Tauri 2.x |
| 前端 / Frontend | React 18 + TypeScript + Vite |
| UI 样式 / Styling | Tailwind CSS |
| 状态管理 / State | Zustand |
| 图标 / Icons | Lucide React |
| 系统监控 / System Metrics | sysinfo (CPU/RAM) + ADLX (AMD GPU) + NVML (NVIDIA GPU) |

---

## 系统要求 / Requirements

- Windows 10/11
- macOS 13+（Apple Silicon）
- Linux (Ubuntu 22.04+)
- Rust 1.75+
- Node.js 18+

---

## 从源码构建 / Build from Source

```bash
# 克隆仓库 / Clone repo
git clone https://github.com/jerrydong1988/llama-server-manager.git
cd llama-server-manager

# 安装依赖 / Install dependencies
npm install

# 开发模式 / Dev mode
npm run tauri dev

# 生产构建 / Production build
npm run tauri build
```

Linux 构建需先安装系统依赖 / Linux requires system dependencies:
```bash
sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf
```

macOS 编译后运行需解除 Gatekeeper / macOS requires removing quarantine:
```bash
xattr -cr /Applications/LlamaServerManager.app
```

---

## 项目结构 / Project Structure

```
llama-server-manager/
├── src/                    # React 前端 / Frontend
│   ├── components/
│   │   ├── ConfigPage/     # 参数配置子组件
│   │   ├── PerformancePage/ # 性能监控 + 性能分析面板
│   │   ├── ClusterPage/    # 集群管理
│   │   ├── DownloadManager.tsx  # 下载管理
│   │   ├── InstanceManager.tsx
│   │   ├── ModelRepo.tsx
│   │   ├── EngineManager.tsx
│   │   └── LogsViewer.tsx
│   ├── store/              # 状态管理 / State (Zustand)
│   │   ├── types.ts        # 类型定义
│   │   └── defaults.ts     # 默认配置
│   ├── i18n/               # 国际化 (中/英)
│   │   └── utils/           # 跨平台路径工具
│   ├── validators.ts       # 参数校验引擎 (26 规则)
│   ├── App.tsx
│   └── main.tsx
├── src-tauri/              # Rust 后端 / Backend
│   ├── src/
│   │   ├── main.rs         # 入口 + 窗口/托盘
│   │   ├── models.rs       # 数据结构定义
│   │   ├── utils.rs        # GGUF 解析 / 后端检测 / 路径
│   │   ├── build.rs        # 构建脚本
│   │   └── commands/       # 命令模块 (11 文件)
│   │       ├── config.rs   # 配置持久化 + 备份恢复
│   │       ├── server.rs   # 命令生成 + 启停 + 健康检查 + 指标采集 + 日志性能解析
│   │       ├── scanner.rs  # 模型/引擎扫描
│   │       ├── download.rs # ModelScope/HuggingFace 下载
│   │       ├── adlx.rs     # ADLX GPU 指标采集 (AMD)
│   │       ├── nvml.rs     # NVML GPU 指标采集 (NVIDIA)
│   │       ├── cluster.rs  # 集群管理
│   │       ├── cluster_network.rs
│   │       ├── cluster_mdns.rs
│   │       └── cluster_ssh.rs
│   └── Cargo.toml
├── .github/workflows/      # CI/CD 三平台自动构建
├── package.json
└── vite.config.ts
```

---

## 使用教程 / User Guide

详细操作指南请参阅 [GUIDE.md](GUIDE.md)

Please refer to [GUIDE.md](GUIDE.md) for a detailed user guide.

---

## 许可证 / License

MIT
