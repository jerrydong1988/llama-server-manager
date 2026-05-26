# Llama Server Manager 使用教程 / User Guide

> v2.0.6

---

## 目录 / Table of Contents

1. [快速开始 / Quick Start](#快速开始--quick-start)
2. [模型仓库 / Model Repository](#模型仓库--model-repository)
3. [引擎管理 / Engine Management](#引擎管理--engine-management)
4. [实例管理 / Instance Management](#实例管理--instance-management)
5. [参数配置 / Parameter Configuration](#参数配置--parameter-configuration)
6. [服务器日志 / Server Logs](#服务器日志--server-logs)
7. [高级功能 / Advanced Features](#高级功能--advanced-features)
8. [常见问题 / FAQ](#常见问题--faq)

---

## 快速开始 / Quick Start

### 下载与安装 / Download & Install

1. 从 [GitHub Releases](https://github.com/jerrydong1988/llama-server-manager/releases/latest) 下载最新的 `llama-server-manager.exe`
2. 将它放到一个独立文件夹中（如 `D:\LlamaManager\`）
3. 双击运行即可，无需安装

> 确保你的系统已安装 [llama.cpp](https://github.com/ggerganov/llama.cpp) 的 `llama-server.exe`。如果没有，程序内可以通过「引擎管理」添加。

1. Download the latest `llama-server-manager.exe` from [GitHub Releases](https://github.com/jerrydong1988/llama-server-manager/releases/latest)
2. Place it in a dedicated folder (e.g. `D:\LlamaManager\`)
3. Double-click to run — no installation required

> Make sure you have `llama-server.exe` from [llama.cpp](https://github.com/ggerganov/llama.cpp) installed. If not, you can add it via "Engine Management".

### 界面布局 / Interface Layout

程序有 5 个标签页，左侧导航栏按操作流程排列：

```
模型仓库 → 引擎管理 → 实例管理 → 参数配置 → 服务器日志
```

**建议操作顺序：**

1. ① **模型仓库**：添加你存放 GGUF 模型的本地目录
2. ② **引擎管理**：添加 llama-server.exe 所在的目录
3. ③ **实例管理**：创建一个实例（选择模型 + 引擎 + 端口）
4. ④ **参数配置**：根据需要调整启动参数
5. ⑤ **启动实例**：回到实例管理，点击启动

The app has 5 tabs in the sidebar, ordered by workflow:

```
Model Repo → Engines → Instances → Config → Server Logs
```

**Recommended workflow:**

1. ① **Model Repo**: Add your local GGUF model directories
2. ② **Engines**: Add your llama-server.exe directory
3. ③ **Instances**: Create an instance (select model + engine + port)
4. ④ **Config**: Adjust startup parameters as needed
5. ⑤ **Start**: Go back to Instances and click Start

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
  📁 Qwen\
    📁 Qwen3-Embedding-4B-GGUF\
      📄 Qwen3-Embedding-4B-Q8_0.gguf     模型    Q8_0    4.2 GB
```

- 📂 黄色文件夹 = 根目录 / Root directory
- 📁 = 子目录 / Subdirectory（可按需折叠/展开 / collapsible）
- 📄 蓝色 = 模型文件 / Model file
- 📷 紫色 = 多模态投影器 / Multimodal projector (mmproj)

### GGUF 元信息 / GGUF Metadata

程序自动从 GGUF 文件头读取以下信息：

- **架构** / Architecture（如 llama、qwen、mistral）
- **上下文长度** / Context Length（如 32768）
- **量化类型** / Quantization Type（如 Q4_K、F16、BF16、MXFP4）

The app automatically reads the following from GGUF headers:

- **Architecture** (e.g. llama, qwen, mistral)
- **Context Length** (e.g. 32768)
- **Quantization Type** (e.g. Q4_K, F16, BF16, MXFP4)

### ModelScope 模型下载 / ModelScope Download

1. 点击「从 ModelScope 下载」按钮
2. 输入仓库 ID（如 `unsloth/Qwen3.6-35B-A3B-GGUF`）
3. 点击「浏览文件」
4. 每个文件旁边有独立的 `⬇ 下载` 按钮
5. 下载中可 `⏸ 暂停`（保留进度，下次继续）或 `✕ 取消`（清除文件）
6. 下载完成后文件自动出现在模型仓库中

1. Click "Download from ModelScope"
2. Enter repo ID (e.g. `unsloth/Qwen3.6-35B-A3B-GGUF`)
3. Click "Browse Files"
4. Each file has an independent `⬇ Download` button
5. During download: `⏸ Pause` (keeps progress for resume) or `✕ Cancel` (clears file)
6. Downloaded files automatically appear in Model Repository

### 管理操作 / File Operations

- 📂 按钮 = 在资源管理器中打开 / Open in Explorer
- 🗑 按钮 = 确认后从磁盘永久删除 / Delete permanently from disk (with confirmation)

---

## 引擎管理 / Engine Management

### 添加引擎目录 / Adding Engine Directories

点击「添加引擎根目录」，选择包含 `llama-server.exe` 的**父级目录**。程序会递归扫描子目录（1-2 层），自动发现所有版本。

Click "Add Engine Directory" and select the **parent folder** containing `llama-server.exe`. The app scans subdirectories (1-2 levels) to auto-discover all versions.

### 引擎类型识别 / Backend Detection

程序自动检测引擎的后端类型：

| 标记 | 说明 |
|------|------|
| CUDA | 检测到 CUDA/cuBLAS 相关 DLL |
| ROCm | 检测到 ROCm/HIP 相关文件 |
| Vulkan | 检测到 Vulkan 相关文件 |
| CPU | 未检测到任何 GPU 后端 |

Each engine card shows its backend type automatically.

### 设为默认引擎 / Set as Default

点击引擎卡片下的「设为默认」按钮。默认引擎会被新创建的实例自动选用（可在实例卡片上单独覆盖）。

Click "Set as Default" on an engine card. New instances will use this engine unless overridden on the instance card.

---

## 实例管理 / Instance Management

### 创建实例 / Creating an Instance

1. 点击「创建实例」
2. 填写实例名称（如 `Qwen-35B-API`）
3. 点击 📂 按钮从模型仓库树中选择一个主模型
4. 选择引擎（默认会选中全局默认引擎）
5. 设置端口（注意端口冲突提示）
6. 点击「创建」

1. Click "Create Instance"
2. Enter a name (e.g. `Qwen-35B-API`)
3. Click 📂 to select a model from the repository tree
4. Select an engine (default engine is pre-selected)
5. Set port (watch for conflict warnings)
6. Click "Create"

### 启动/停止实例 / Start / Stop

- 绿色 `▶ 启动` 按钮：启动实例。启动后按钮变为红色 `■ 停止`
- 实例运行中右上角显示状态图标：
  - ◯ 灰色空心圆 = 已停止 / Stopped
  - ◌ 蓝色旋转环 = 启动中 / Starting
  - ✓ 绿色对勾 = 运行正常 / Running healthy
  - ✕ 红色叉 = 错误 / Error

- Green `▶ Start` button: starts the instance
- After starting, the button changes to red `■ Stop`
- Status icons in the top-right of each card:
  - ◯ Gray circle = Stopped
  - ◌ Blue spinner = Starting
  - ✓ Green check = Running healthy
  - ✕ Red X = Error

### 实例卡片功能 / Instance Card Functions

| 按钮 | 功能 |
|------|------|
| ▶/■ | 启动/停止 / Start/Stop |
| 🌐 | 在浏览器中打开 API 页面 / Open API in browser |
| 📶 | 测试连接（ping /health 端点）/ Test connection |
| ⌨ | 查看生成的命令行 / View generated command |
| ⚙ | 跳转到参数配置页 / Jump to config page |
| ↑↓ | 排序（调整实例卡片顺序）/ Reorder |
| ✏️ | 编辑实例名称 / Edit instance name |
| 🗑 | 删除实例 / Delete instance |

### 每实例独立选择引擎 / Per-instance Engine Selection

点击实例卡片上的引擎名称 → 弹出引擎选择弹窗 → 选择该实例专属引擎。此设置会覆盖全局默认引擎。

Click the engine name on an instance card → a picker dialog opens → select an engine for this specific instance. This overrides the global default engine for this instance only.

### 快捷键 / Keyboard Shortcuts

| 快捷键 | 功能 |
|--------|------|
| `Ctrl + Enter` | 启动第一个已停止的实例 / Start first stopped instance |
| `Ctrl + S` | 保存全部配置 / Save all configs |

---

## 参数配置 / Parameter Configuration

### 打开配置页 / Opening Config Page

在实例管理页点击实例卡片的 ⚙ 按钮，自动跳转到参数配置页。

Click the ⚙ button on an instance card to jump to the config page.

配置页包含 5 个折叠组：

The config page has 5 collapsible sections:

| 组 | 内容 |
|------|------|
| **基本参数** Basic | 模型路径、别名、LoRA、投影器、聊天模板、推理参数 |
| **生成参数** Generation | 温度、Top-K/P、重复惩罚、种子、Min-P 等 |
| **高级采样** Advanced Sampling | Mirostat、XTC、动态温度、DRY（有总开关） |
| **性能配置** Performance | 上下文大小、GPU 层数、线程、批处理 |
| **网络 & API** Network & API | 主机、端口、API 密钥、Embedding 模式、SSL |

### 高级采样开关 / Advanced Sampling Toggle

高级采样默认关闭。每个参数展开后会有一个「已关闭/已开启」切换按钮。关闭时所有高级采样参数不参与启动命令。

Advanced sampling is off by default. Each section has an on/off toggle. When off, none of the advanced sampling parameters are included in the startup command.

### 向量模型检测 / Embedding Model Detection

当选择向量模型（如 BGE、GTE、Qwen3-Embedding）时，程序自动：

1. 启用 Embedding 模式
2. 所有文本生成相关参数被锁定（显示 🛑 标记）
3. 池化策略自动设为 `mean`

When an embedding model is selected (e.g. BGE, GTE, Qwen3-Embedding), the app automatically:

1. Enables Embedding mode
2. Locks all text generation params (marked with 🛑)
3. Sets pooling strategy to `mean`

### 参数悬停提示 / Tooltips

所有参数名称和输入框都有鼠标悬停提示，说明该参数的作用和推荐值。

All parameter labels and inputs have hover tooltips explaining their purpose and recommended values.

### 模型路径树选择器 / Model Path Picker

模型路径旁的 📂 按钮可以打开模型仓库树，一键选择模型文件，同时自动检测同目录下的 mmproj 文件并填入投影器路径。

The 📂 button next to Model Path opens the repository tree for one-click model selection, also auto-detecting mmproj files in the same directory.

---

## 服务器日志 / Server Logs

- 实时显示实例的 `stdout`/`stderr` 输出
- 关键词自动高亮：红色=错误、黄色=警告、绿色=就绪、青色=性能
- 可按实例筛选查看
- 点击「清空日志」清除当前显示

- Real-time display of instance `stdout`/`stderr` output
- Keyword auto-highlighting: red=error, yellow=warning, green=ready, cyan=performance
- Filter by instance
- "Clear Logs" to reset

---

## 高级功能 / Advanced Features

### 中英双语 / i18n

侧边栏底部的 `EN`/`中` 按钮可切换界面语言。所有界面文字完整支持中英双语。

The `EN`/`中` button at the bottom of the sidebar switches the entire UI between Chinese and English.

### 主题切换 / Theme Toggle

点击 ☀/🌙 按钮切换深色/浅色主题。主题偏好会在下次启动时自动恢复。

Click ☀/🌙 to toggle dark/light theme. Theme preference persists across restarts.

### 系统托盘 / System Tray

关闭窗口（右上角 ✕）时程序不会退出，而是最小化到系统托盘。右键托盘图标可「显示窗口」或「退出」。

- 左键点击托盘图标 = 显示窗口
- 右键点击托盘图标 = 菜单（显示/退出）

When you close the window (✕), the app minimizes to the system tray instead of exiting.

- Left click tray icon = Show window
- Right click tray icon = Menu (Show / Quit)

### 配置文件 / Config Persistence

所有配置（模型目录、引擎目录、实例、引擎设置、主题偏好、窗口位置）自动保存在 `configs/` 目录下的 JSON 文件中。关闭程序后重新打开，一切配置自动恢复。

All configs (model dirs, engine dirs, instances, engine settings, theme, window position) are auto-saved as JSON files in the `configs/` directory. Everything is restored when you reopen the app.

**注意：** 如果程序更新后配置文件格式不兼容，删除 `configs/instances.json` 重新配置即可。

**Note:** If the config format becomes incompatible after an update, simply delete `configs/instances.json` and reconfigure.

### 自动更新检查 / Auto-update

程序启动时自动检测 GitHub Release 是否有新版本。如有更新，侧边栏底部显示绿色提示横幅，点击跳转到下载页。

The app automatically checks GitHub Releases for updates on startup. If a newer version is available, a green banner appears at the bottom of the sidebar — click to open the download page.

---

## 常见问题 / FAQ

**Q: 为什么启动后提示"未检测到引擎"？**
A: 请先在「引擎管理」中添加 llma-server.exe 所在的目录。程序需要知道 llama-server 的位置才能启动实例。

**Q: Why does it say "No engines detected"?**
A: Go to "Engine Management" and add the directory containing llama-server.exe first.

---

**Q: 如何添加多个版本的引擎？**
A: 在「引擎管理」中，选择包含多个引擎子目录的**父级目录**（如 `C:\llama.cpp\`），程序会自动发现所有子目录中的 llama-server.exe。

**Q: How to add multiple engine versions?**
A: In "Engine Management", select the **parent directory** (e.g. `C:\llama.cpp\`) containing multiple engine subdirectories. The app auto-discovers all llama-server.exe files.

---

**Q: 为什么模型下拉选不了引擎？**
A: 点击实例卡片上的引擎名称 → 弹出全局弹窗 → 点击选择即可。这不是下拉框，是弹窗选择器。

**Q: Why can't I select an engine from the dropdown?**
A: Click the engine name on the instance card → a dialog pops up → click to select. It's a dialog picker, not a dropdown.

---

**Q: 配置文件在哪里？**
A: 在 `llama-server-manager.exe` 同目录下的 `configs/` 文件夹中。`instances.json` 是主配置文件。

**Q: Where are the config files?**
A: In the `configs/` folder next to `llama-server-manager.exe`. `instances.json` is the main config file.

---

**Q: 如何同时运行多个模型？**
A: 创建多个实例，设置不同的端口即可。每个实例独立运行。

**Q: How to run multiple models simultaneously?**
A: Create multiple instances with different ports. Each instance runs independently.
