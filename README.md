# Llama Server Manager / Llama 服务器管理器

> 现代化 Llama.cpp 服务器图形化管理器 | Modern GUI for managing llama.cpp servers

一个功能完整的桌面应用程序，用于管理 `llama-server` 的全生命周期：**下载模型 → 选择引擎 → 配置参数 → 启动监控**。告别复杂的命令行参数。

A fully-featured desktop application for managing the `llama-server` lifecycle: **download models → select engines → configure parameters → start & monitor**. Say goodbye to complex CLI arguments.

[📥 **下载最新版 / Download Latest**](https://github.com/jerrydong1988/llama-server-manager/releases/latest) | [📖 **使用教程 / User Guide**](GUIDE.md)

---

## 界面预览 / Screenshots
<img width="1280" height="830" alt="image" src="https://github.com/user-attachments/assets/5fd90304-fc39-4f33-8ceb-681890584297" />
<img width="1280" height="800" alt="image" src="https://github.com/user-attachments/assets/9cd8588a-c17d-45a8-a619-bcfb6d3f55d5" />
<img width="1280" height="830" alt="image" src="https://github.com/user-attachments/assets/7edba269-1147-408b-b61c-38c5825450ac" />
<img width="1280" height="830" alt="image" src="https://github.com/user-attachments/assets/29d10bff-bad8-4f8b-a385-2d925bc35c6a" />
<img width="1280" height="1390" alt="image" src="https://github.com/user-attachments/assets/a34b0460-4e19-43fb-b576-63263d0ff5f5" />
<img width="1280" height="830" alt="image" src="https://github.com/user-attachments/assets/974a8169-9d48-4f8b-a3dd-452818f041e0" />


---

## 功能特性 / Features

### 模型仓库 / Model Repository
- 多目录递归扫描，支持 LM Studio / NovaMax 等任意目录结构
- GGUF 元信息自动解析（架构 / 上下文长度 / 量化类型）
- 自适应递归树结构，按实际文件系统层级展示
- 从 ModelScope（魔搭社区）直接下载模型，支持断点续传、独立文件下载/暂停/继续/取消
- 一键在资源管理器中打开、从磁盘删除
- Multi-directory recursive scanning, supports LM Studio/NovaMax
- Automatic GGUF metadata parsing (architecture / context length / quantization)
- Adaptive recursive tree matching actual filesystem hierarchy
- Direct model download from ModelScope with resume, per-file download/pause/resume/cancel
- Open in Explorer, delete from disk

### 引擎管理 / Engine Management
- 多引擎递归扫描，自动识别子目录中的 `llama-server.exe`
- 后端类型自动检测（CUDA / ROCm / Vulkan / CPU）
- 多引擎根目录管理，设为默认引擎
- Recursive engine scanning, auto-discovers llama-server.exe in subdirectories
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
- 全部 llama-server 参数：模型/生成/性能/高级/网络 & API
- 高级采样总开关，一键启用/禁用 Mirostat/XTC/DRY 等
- 向量模型自动检测，锁定不适用参数
- 模型路径仓库树选择器，自动关联 mmproj
- 所有参数带悬停提示说明
- Unified config page associated per instance
- Full llama-server parameters: model/generation/performance/advanced/network & API
- Advanced sampling master toggle for Mirostat/XTC/DRY
- Embedding model auto-detection with irrelevant parameter locking
- Model path tree picker with automatic mmproj association
- Tooltip hints on all parameters

### 其他 / Other
- 服务器日志实时捕获，关键词高亮
- 系统托盘支持，关闭窗口隐藏到托盘
- 明暗主题切换（持久化）+ 窗口尺寸/位置记忆
- 完整中英双语界面
- 配置持久化（JSON）+ 自动更新检查
- 端口冲突检测
- Real-time server log capture with keyword highlighting
- System tray support, close to tray
- Light/dark theme toggle (persistent) + window size/position memory
- Full i18n support (Chinese / English)
- Config persistence (JSON) + auto-update check
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

---

## 系统要求 / Requirements

- Windows 10/11（主要支持 / Primary）
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

编译后的可执行文件位于 `src-tauri/target/release/` 目录。

The compiled executable is located in `src-tauri/target/release/`.

---

## 项目结构 / Project Structure

```
llama-server-manager/
├── src/                    # React 前端源码 / Frontend source
│   ├── components/         # 组件 / Components
│   ├── i18n/               # 国际化 / Internationalization
│   ├── store.ts            # 全局状态 / Global state
│   ├── App.tsx             # 主应用 / Main app
│   └── main.tsx            # 入口 / Entry point
├── src-tauri/              # Rust 后端 / Rust backend
│   ├── src/
│   │   └── main.rs         # 核心逻辑 / Core logic
│   ├── Cargo.toml          # Rust 依赖 / Rust deps
│   └── tauri.conf.json     # Tauri 配置 / Tauri config
├── package.json            # Node 依赖 / Node deps
└── vite.config.ts          # Vite 配置 / Vite config
```

---

## 使用教程 / User Guide

详细操作指南请参阅 [GUIDE.md](GUIDE.md)

Please refer to [GUIDE.md](GUIDE.md) for a detailed user guide.

---

## 许可证 / License

MIT
