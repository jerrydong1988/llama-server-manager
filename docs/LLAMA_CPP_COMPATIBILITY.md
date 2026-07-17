# llama.cpp 参数兼容机制 / Compatibility Policy

程序通过三层互补机制保持参数同步，避免依赖维护者记忆或临近发版时人工排查。

The application uses three complementary controls so parameter support does not depend on memory or last-minute release review.

## 支持范围 / Support Window

- 最新官方稳定版是静态参数基线。
- 前两个稳定版通过运行时能力协商继续兼容。
- 仅出现在 `master` 的参数视为已复核的前瞻能力，不冒充稳定版支持。
- 第三方分支和本地构建版本，以自身 `llama-server --help` 实际公开的参数为准。

- The latest official stable release is authoritative.
- The previous two stable releases remain usable through runtime negotiation.
- `master`-only flags are reviewed preview capabilities.
- Forks are supported according to their own `llama-server --help` output.

## 运行时协商 / Runtime Negotiation

引擎扫描只读取文件系统，不执行全部被发现的二进制。程序仅在实例明确选择了某个引擎，或用户在“引擎管理”中主动操作时进行探测。

探测不经过 Shell，只直接执行 `--version` 和 `--help`；同时限制执行时间和输出容量，在 Windows 隐藏控制台，并持续排空输出管道。结果绑定二进制指纹，文件变化后自动失效。

版本识别与参数能力相互独立。程序只接受 `version:`、`llama-server version` 或 `llama.cpp version` 等明确版本行；初始化日志不会被当作版本。未能识别标准版本号时会显示提醒，但只要 `--help` 能力完整，参数仍按实际能力严格校验。

参数能力分为三档：

- `detected`：生成完整命令，并在保存配置和启动进程前拦截不受支持的活动参数。
- `partial`：完整配置继续保留，命令仅传递已识别参数以及模型、地址端口、工作模式和认证等必要参数。
- `unprobed`、`timeout` 或 `failed`：完整配置继续保留，启动时使用最小必要命令；批量参数预设暂不启用。

保守模式不会静默删除配置。更换回能力完整的引擎后，原有参数可以重新参与校验和命令生成。Embedding 与 Reranker 的工作模式参数以及认证/TLS 等安全参数不会因保守模式被静默移除。

Scanning never executes every discovered binary. Probes run only for explicitly selected engines, without a shell, with time and output limits. Version recognition is independent from parameter capability detection. Complete results enforce compatibility at save and launch; partial results retain recognized and essential flags; unknown results use a minimal command without deleting saved configuration.

## 上游监控 / Upstream Watcher

`scripts/check-llama-upstream.cjs` 读取官方最新稳定版和 `master` 的 `tools/server/README.md` 参数表，并在 `scripts/llama-parameter-baseline.json` 中保存参数名、别名、语法摘要及说明/默认值摘要。

定时工作流 `llama.cpp Upstream Watch` 会分类报告参数新增、删除、别名变化、语法变化和说明/默认值变化。稳定版漂移会使兼容性任务失败；仅主分支发生变化时作为前瞻提醒。两者共用一个去重后的 GitHub 跟踪 Issue，基线始终需要人工复核，不会自动合并修改。

The scheduled watcher classifies additions, removals, alias, syntax, and description/default changes. Stable drift fails the compatibility job; master-only drift remains a canary warning. Baseline updates always require review.

```text
npm run check:llama-upstream
node scripts/check-llama-upstream.cjs --write
```

常规发版检查保持离线，只使用已提交的稳定版注册表和已复核的前瞻参数注册表，验证 Rust 实际生成的全部参数。
