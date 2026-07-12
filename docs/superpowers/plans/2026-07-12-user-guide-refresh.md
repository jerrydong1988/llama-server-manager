# 使用说明完整对齐实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 `v2.9.22` 的 README、完整双语手册、应用内说明、交互式引导和随包截图统一为一套可离线验证的发布级使用说明。

**Architecture:** `GUIDE.md` 是唯一完整内容源，README 只保留快速开始和精选截图。图片放在 Vite `public/docs/guide/` 中，应用内渲染前将仓库路径转换为运行时路径；交互引导数据独立到 `guideTour.ts`，由静态脚本校验导航、选择器、章节、锚点和图片资产的一致性。

**Tech Stack:** React 18、TypeScript、Vite、Tauri 2、marked、driver.js、Node.js CommonJS、GitHub Actions、PNG。

## Global Constraints

- 发布基线版本固定为 `v2.9.22`，应用内版本仍从 `package.json` 动态替换。
- 不引入外部文档生成器、托管文档服务或新测试框架。
- 所有说明保持中英双语，中文在前、英文紧随。
- 截图必须来自当前构建，不伪造运行指标，不暴露用户名、私有路径、API Key、IP 或 SSH 凭据。
- 所有图片必须同时能被 GitHub Markdown 和离线 Tauri 应用加载。
- Windows 文件修改只使用 `apply_patch`；禁止使用 `Set-Content` 或 `Out-File` 写源码和 Markdown。
- 任务完成时清理临时捕获文件、标注工作页和未交付的生成物。

---

### Task 1: 建立使用说明一致性检查

**Files:**
- Create: `scripts/check-guide.cjs`
- Modify: `package.json`
- Test: `scripts/check-guide.cjs` 自执行断言

**Interfaces:**
- Consumes: `package.json.version`、`GUIDE.md`、`README.md`、`src/App.tsx`、`src/components/guide/guideTour.ts` 和 `public/docs/guide/*.png`。
- Produces: `npm run check:guide`，成功时输出 `Guide check passed`，失败时逐行输出可定位的问题并以状态码 1 退出。

- [ ] **Step 1: 写入首先会失败的检查脚本**

脚本必须使用 `fs.readFileSync(..., 'utf8')`，并实现以下常量和检查：

```js
const REQUIRED_SECTIONS = [
  '快速开始 / Quick Start',
  '系统总览 / Dashboard',
  '模型仓库 / Model Repository',
  '下载管理 / Download Manager',
  '引擎管理 / Engine Management',
  '实例管理 / Instance Management',
  '参数配置 / Parameter Configuration',
  '集群管理 / Cluster Management',
  '实例路由 / Instance Routing',
  '性能监控 / Performance Monitoring',
  '监控大屏 / Monitoring Wall',
  '服务器日志 / Server Logs',
  '常见问题 / FAQ',
]

const REQUIRED_IMAGES = [
  '01-dashboard.png', '02-model-repository.png', '03-download-manager.png',
  '04-engine-manager.png', '05-instance-manager.png', '06-configuration.png',
  '07-cluster-manager.png', '08-instance-routing.png', '09-performance.png',
  '10-monitoring-wall.png', '11-server-logs.png', '12-in-app-guide.png',
  'flow-01-first-run.png', 'flow-02-start-and-diagnose.png', 'flow-03-route-requests.png',
]

const REQUIRED_TOUR_TABS = [
  'dashboard', 'model-repo', 'downloads', 'engine', 'instances', 'config',
  'cluster', 'proxy', 'perf', 'bigscreen', 'logs',
]
```

检查必须覆盖版本、必需章节、Markdown 图片路径、图片文件存在性、目录锚点、导航 tab、tour tab 和 `data-guide` 选择器。错误聚合后统一退出，避免一次只暴露一个问题。

- [ ] **Step 2: 运行脚本证明当前说明不满足要求**

Run: `node scripts/check-guide.cjs`

Expected: FAIL，至少报告 `GUIDE.md` 版本仍为 `v2.9.15`、缺少实例路由与监控大屏章节、缺少 `public/docs/guide/` 图片。

- [ ] **Step 3: 暴露 npm 命令**

在 `package.json` 的 `scripts` 中加入：

```json
"check:guide": "node scripts/check-guide.cjs"
```

- [ ] **Step 4: 验证失败来自内容缺口而非脚本异常**

Run: `npm run check:guide`

Expected: 命令正常执行检查逻辑并以状态码 1 结束；输出只包含明确的使用说明缺口，不包含 JavaScript 堆栈。

- [ ] **Step 5: 提交检查基线**

```powershell
git add -- scripts/check-guide.cjs package.json
git commit -m "test: add user guide consistency checks"
```

---

### Task 2: 完善应用内说明渲染和 11 步交互引导

**Files:**
- Create: `src/components/guide/guideTour.ts`
- Modify: `src/components/GuidePage.tsx`
- Modify: `src/components/ProxyPage.tsx`
- Modify: `src/components/BigScreenPage.tsx`
- Modify: `src/components/PerformancePage/PerformancePage.tsx`
- Existing target: `src/components/ConfigPage.tsx`
- Test: `npm run check:guide`、TypeScript compiler

**Interfaces:**
- Consumes: `lang: 'zh-CN' | 'en-US'`、`setActiveTab(tab: string)`、`GUIDE.md?raw`。
- Produces: `GuideTourStep`、`GUIDE_TOUR_STEPS`、`getGuideTourSteps(lang)`；应用内图片路径 `/docs/guide/*.png`；安全的内部锚点和外部 HTTPS 链接处理。

- [ ] **Step 1: 创建强类型 tour 数据**

```ts
export type GuideTourStep = {
  id: string
  tab: string
  selector: string
  zh: { title: string; description: string }
  en: { title: string; description: string }
}

export const GUIDE_TOUR_STEPS: GuideTourStep[] = [
  { id: 'dashboard', tab: 'dashboard', selector: '[data-guide="dashboard"]', zh: { title: '系统总览', description: '查看系统资源、实例状态与整体运行健康度。' }, en: { title: 'Dashboard', description: 'Review system resources, instance state, and overall health.' } },
  { id: 'models', tab: 'model-repo', selector: '[data-guide="model-search"]', zh: { title: '模型仓库', description: '扫描和管理本地 GGUF 模型与投影器。' }, en: { title: 'Model Repository', description: 'Scan and manage local GGUF models and projectors.' } },
  { id: 'downloads', tab: 'downloads', selector: '[data-guide="download-source"]', zh: { title: '下载管理', description: '浏览远程仓库并管理下载队列与恢复策略。' }, en: { title: 'Downloads', description: 'Browse repositories and manage queues and resume policy.' } },
  { id: 'engines', tab: 'engine', selector: '[data-guide="engine-scan"]', zh: { title: '引擎管理', description: '扫描 llama-server 并选择默认运行引擎。' }, en: { title: 'Engines', description: 'Scan llama-server binaries and choose the default runtime.' } },
  { id: 'instances', tab: 'instances', selector: '[data-guide="instance-create"]', zh: { title: '实例管理', description: '创建、启动和维护独立服务实例。' }, en: { title: 'Instances', description: 'Create, start, and maintain server instances.' } },
  { id: 'config', tab: 'config', selector: '[data-guide="config-save"]', zh: { title: '参数配置', description: '按实例调整参数并查看分级校验提示。' }, en: { title: 'Configuration', description: 'Tune instance parameters and review validation findings.' } },
  { id: 'cluster', tab: 'cluster', selector: '[data-guide="cluster-scan"]', zh: { title: '集群管理', description: '发现或启动本地与远程 RPC Worker。' }, en: { title: 'Cluster', description: 'Discover or launch local and remote RPC workers.' } },
  { id: 'proxy', tab: 'proxy', selector: '[data-guide="proxy-overview"]', zh: { title: '实例路由', description: '把多个运行实例统一到 OpenAI 兼容入口。' }, en: { title: 'Instance Routing', description: 'Expose running instances through one OpenAI-compatible endpoint.' } },
  { id: 'performance', tab: 'perf', selector: '[data-guide="perf-select"]', zh: { title: '性能监控', description: '查看资源信号、吞吐、槽位与请求分析。' }, en: { title: 'Performance', description: 'Inspect resources, throughput, slots, and request analysis.' } },
  { id: 'bigscreen', tab: 'bigscreen', selector: '[data-guide="monitoring-wall"]', zh: { title: '监控大屏', description: '集中观察服务健康、吞吐、压力和告警。' }, en: { title: 'Monitoring Wall', description: 'Watch service health, throughput, pressure, and alerts.' } },
  { id: 'logs', tab: 'logs', selector: '[data-guide="logs-clear"]', zh: { title: '服务器日志', description: '筛选实时日志并定位启动与健康检查问题。' }, en: { title: 'Logs', description: 'Filter live logs and diagnose startup or health failures.' } },
]

export function getGuideTourSteps(lang: string) {
  return GUIDE_TOUR_STEPS.map(step => {
    const copy = lang === 'zh-CN' ? step.zh : step.en
    return {
      id: step.id,
      tab: step.tab,
      selector: step.selector,
      title: copy.title,
      description: copy.description,
    }
  })
}
```

- [ ] **Step 2: 给缺失页面添加稳定目标**

在实例路由首个主 `Surface` 添加 `data-guide="proxy-overview"`；在监控大屏根容器添加 `data-guide="monitoring-wall"`；在性能页运行实例选择器容器添加或确认唯一的 `data-guide="perf-select"`。不得改变样式或事件行为。

- [ ] **Step 3: 改造 GuidePage 渲染器**

加入路径转换并保留清理流程：

```ts
function normalizeGuideAssetPaths(markdown: string): string {
  return markdown.replaceAll('(public/docs/guide/', '(/docs/guide/')
}

function renderMD(markdown: string): string {
  const processed = normalizeGuideAssetPaths(markdown).replace(/^## (.+)$/gm, (_, title: string) => {
    const id = title.toLowerCase().replace(/[^a-z0-9\u4e00-\u9fa5]+/g, '-')
    return `<h2 id="${id}">${title}</h2>`
  })
  return sanitize(marked.parse(processed, { async: false }) as string)
}
```

扩展允许属性为 `loading`、`width`、`height`，仍拦截 `javascript:`、`vbscript:` 和事件属性。内容区点击 `#anchor` 时在本页滚动；点击 `https://` 链接时调用项目已有的 `@tauri-apps/plugin-shell`：

```ts
import { open as openExternal } from '@tauri-apps/plugin-shell'

await openExternal(href)
```

外部链接不得替换应用 WebView。

- [ ] **Step 4: 使用独立 tour 数据并保证返回说明页**

`startTour` 使用 `getGuideTourSteps(lang)`，配置页在没有实例时跳过。每一步目标超时只记录 `console.warn` 并继续；循环放在 `try/finally` 中，最终调用 `setActiveTab('guide')`。

- [ ] **Step 5: 运行静态检查和 TypeScript 编译**

Run: `npm run check:guide`

Expected: 仍因文档和图片缺失而 FAIL，但不再报告缺少 tour tab 或 `data-guide` 目标。

Run: `node .\node_modules\typescript\bin\tsc --noEmit`

Expected: PASS，0 个 TypeScript 错误。

- [ ] **Step 6: 提交应用内说明与引导**

```powershell
git add -- src/components/guide/guideTour.ts src/components/GuidePage.tsx src/components/ProxyPage.tsx src/components/BigScreenPage.tsx src/components/PerformancePage/PerformancePage.tsx
git commit -m "feat: align in-app guide tour"
```

---

### Task 3: 重写完整双语手册和 README

**Files:**
- Modify: `GUIDE.md`
- Modify: `README.md`
- Test: `scripts/check-guide.cjs`

**Interfaces:**
- Consumes: `public/docs/guide/*.png` 的固定文件名和当前应用导航名称。
- Produces: `v2.9.22` 完整双语手册、README 五步快速开始、带标题的本地截图入口。

- [ ] **Step 1: 按真实工作流重写 GUIDE.md**

文档必须从以下骨架展开，每节包含用途、前提、编号操作、状态/控制说明、恢复提示和双语图片说明：

```markdown
# Llama Server Manager 使用说明 / User Guide

> v2.9.22 · Windows / macOS / Linux

## 目录 / Table of Contents
## 快速开始 / Quick Start
## 系统总览 / Dashboard
## 模型仓库 / Model Repository
## 下载管理 / Download Manager
## 引擎管理 / Engine Management
## 实例管理 / Instance Management
## 参数配置 / Parameter Configuration
## 集群管理 / Cluster Management
## 实例路由 / Instance Routing
## 性能监控 / Performance Monitoring
## 监控大屏 / Monitoring Wall
## 服务器日志 / Server Logs
## 应用设置与数据安全 / Application Settings and Data Safety
## 常见问题 / FAQ
## 发版前自检 / Release Validation
```

实例路由必须说明统一端点、模型别名、鉴权、后台保活和退出确认；下载管理必须说明并发、限速、手动/启动恢复；数据安全必须说明 `instances.json.bak` 自动回退、API Key 文件和日志持久化。

- [ ] **Step 2: 为所有图像写入固定引用**

每个页面截图使用：

```markdown
![系统总览展示资源、实例和运行状态 / Dashboard with resources, instances, and service health](public/docs/guide/01-dashboard.png)
```

三个流程图分别放在快速开始、启动诊断、实例路由章节。禁止使用无意义的 `alt="image"`。

- [ ] **Step 3: 重写 README 的说明入口**

README 保留产品定位、下载入口、五步快速开始、六张精选截图、当前功能地图、系统要求、源码构建和完整手册链接。删除现有 11 张无标题远程 attachment 图片，精选图直接复用 `public/docs/guide/`。

- [ ] **Step 4: 检查 Markdown 内容层**

Run: `npm run check:guide`

Expected: 章节、版本、锚点和引用路径检查通过；如果图片尚未采集，只报告 15 个具体图片文件缺失。

Run: `node scripts\check-encoding.mjs`

Expected: `UTF-8 check passed`。

- [ ] **Step 5: 提交文案更新**

```powershell
git add -- GUIDE.md README.md
git commit -m "docs: rewrite bilingual user guide"
```

---

### Task 4: 采集实机截图并制作流程标注图

**Files:**
- Create: `public/docs/guide/01-dashboard.png`
- Create: `public/docs/guide/02-model-repository.png`
- Create: `public/docs/guide/03-download-manager.png`
- Create: `public/docs/guide/04-engine-manager.png`
- Create: `public/docs/guide/05-instance-manager.png`
- Create: `public/docs/guide/06-configuration.png`
- Create: `public/docs/guide/07-cluster-manager.png`
- Create: `public/docs/guide/08-instance-routing.png`
- Create: `public/docs/guide/09-performance.png`
- Create: `public/docs/guide/10-monitoring-wall.png`
- Create: `public/docs/guide/11-server-logs.png`
- Create: `public/docs/guide/12-in-app-guide.png`
- Create: `public/docs/guide/flow-01-first-run.png`
- Create: `public/docs/guide/flow-02-start-and-diagnose.png`
- Create: `public/docs/guide/flow-03-route-requests.png`
- Test: visual inspection and `npm run check:guide`

**Interfaces:**
- Consumes: current Windows Tauri development build and final page labels.
- Produces: 15 privacy-reviewed PNG assets, 1440 by 900 base capture where practical, readable in GitHub and Tauri WebView.

- [ ] **Step 1: 启动当前桌面构建并固定捕获条件**

Run: `npm run tauri dev`

Expected: `LlamaServerManager` 窗口打开，版本为 `v2.9.22`，窗口内容区按 1440 by 900 目标尺寸显示，深色主题启用。

- [ ] **Step 2: 逐页采集 12 张真实截图**

按导航顺序打开页面并保存为规定文件名。截图前检查可见区域；涉及真实路径、地址或密钥时先在截图构图中避开，无法避开时使用不改变 UI 结构的实体色遮盖。缺少硬件或运行实例的页面保留真实空状态。

- [ ] **Step 3: 制作三张确定性标注图**

使用实际截图作为底图，通过本地 HTML/CSS 标注板叠加数字圆点、箭头、边框和中英双语短标签，再由浏览器截取为 PNG。标注板只作为临时生成工具，不提交；不得使用生成式方法重绘界面。

- [ ] **Step 4: 逐图隐私与真实性复核**

检查所有图片：无 `C:\Users\<name>`、API Key、Token、私有 IP、SSH 用户或无关桌面区域；按钮位置、页面布局、状态颜色和指标值未被改造；最长英文标签没有被截断。

- [ ] **Step 5: 验证图片尺寸、路径和构建复制**

Run: `npm run check:guide`

Expected: `Guide check passed`。

Run: `npm run build`

Expected: PASS，且 `dist/docs/guide/` 含 15 张 PNG。

- [ ] **Step 6: 提交图片资产**

```powershell
git add -- public/docs/guide
git commit -m "docs: add offline guide screenshots"
```

---

### Task 5: 接入 CI 并完成桌面离线验收

**Files:**
- Modify: `.github/workflows/build.yml`
- Test: full frontend, Rust, documentation, desktop visual, and Git checks

**Interfaces:**
- Consumes: `npm run check:guide`。
- Produces: 四平台打包前统一执行的说明一致性门禁和完整发布前验证证据。

- [ ] **Step 1: 在每个平台构建前加入说明检查**

在每个 job 的 `npm ci` 后、`npm run tauri build` 前加入：

```yaml
      - name: Validate user guide
        run: npm run check:guide
```

- [ ] **Step 2: 运行全部静态和单元验证**

```powershell
npm run check:guide
node scripts\check-encoding.mjs
node scripts\check-monitoring-theme.mjs
node scripts\test-bootstrap-health.cjs
node scripts\test-download-restore-merge.cjs
node .\node_modules\typescript\bin\tsc --noEmit
npm run build
cargo test --manifest-path src-tauri\Cargo.toml
git diff --check
```

Expected: 所有命令状态码为 0；Rust 测试 0 failures；前端生产构建成功；补丁无空白错误。

- [ ] **Step 3: 验证离线应用内说明**

断开浏览器网络请求或阻止外部访问后启动桌面应用，打开使用说明，确认 15 张本地图片加载、目录锚点滚动正确、外部链接不会替换应用 WebView。

- [ ] **Step 4: 运行完整交互式引导**

从使用说明启动引导，确认 11 个目标依次出现；配置页无实例时可跳过；关闭或完成后回到使用说明；全过程不创建、修改、启动或停止资源。

- [ ] **Step 5: 检查桌面和紧凑窗口布局**

至少检查 1440 by 900 与 1024 by 768：侧边目录可滚动、正文图片不横向溢出、caption 和表格可读、driver.js 浮层不遮挡当前目标。

- [ ] **Step 6: 清理并检查工作树**

删除临时标注 HTML、原始未裁剪截图和捕获缓存。运行：

```powershell
git status --short
git ls-files public/docs/guide
```

Expected: 只有明确交付文件或已提交内容；图片清单恰好包含 15 张 PNG。

- [ ] **Step 7: 提交 CI 和最终修正**

```powershell
git add -- .github/workflows/build.yml GUIDE.md README.md package.json scripts/check-guide.cjs src/components public/docs/guide
git diff --cached --check
git commit -m "docs: complete release guide alignment"
```
