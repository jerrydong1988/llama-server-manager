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

## 有状态检查点采用更严格策略 / Stricter Stateful Checkpoint Policy

引擎公开某个参数只代表命令语法可用，不代表跨进程 slot 状态一定能被真实复用。实验性 KV / Prefill Cache Checkpoint 因此使用独立的证据门：引擎必须明确公开 slots、slot-save-path、Cache RAM 和 idle-slot cache 能力；模型、引擎二进制、版本/backend 和全部状态相关配置必须命中完整 SHA-256 fingerprint。完整分片 GGUF 会按索引对每个分片做内容摘要后再聚合；任一分片变化都产生安全 miss。滑动窗口模型还必须启用且支持 SWA 完整缓存。

Advertising a flag proves only command-line availability, not reusable cross-process state. The experimental checkpoint feature therefore requires the complete slot and prompt-cache capability set plus an exact model-artifact set, engine, version/backend, and state-bearing configuration fingerprint. Complete GGUF shard sets are hashed per shard and then aggregated, so any changed shard causes a safe miss. Sliding-window models additionally require supported full SWA cache.

推测解码能力探测还会读取引擎实际报告的 `--spec-type` 候选。普通命令可以包含多个逗号分隔类型，并按 llama.cpp 固定优先级规范化；checkpoint 只允许当前引擎明确报告的可重建 `ngram-*` 集合。任何 draft/MTP、未知类型或外部 lookup 状态都退回冷启动。`qwen4exp` 已由 B10679 跨进程实测及上游实现确认使用 hybrid recurrent memory：普通 prompt cache 和 `ngram-mod` 可用，但 slot restore 后没有 target cache hit，因此列入 checkpoint 反例，而不是根据模型名称乐观放行。

Runtime probing also records the engine-reported `--spec-type` choices. Ordinary commands may combine comma-separated types in normalized llama.cpp priority order; checkpointing allows only reported, rebuildable `ngram-*` sets. Draft/MTP, unknown types, and external lookup state fall back cold. B10679 cross-process testing and the corresponding llama.cpp implementation confirm that `qwen4exp` uses hybrid recurrent memory: ordinary prompt caching and `ngram-mod` work, but restored slot state does not yield a target cache hit, so the architecture is an explicit checkpoint counterexample.

资格不满足、fingerprint miss、损坏或 restore 验证失败都会安全回到冷启动；不会因为引擎处于通用参数支持窗口就放宽有状态兼容条件。完整范围和操作说明见 [KV / Prefill Cache Checkpoint](KV_CACHE_CHECKPOINT.md)。

Ineligibility, fingerprint misses, corruption, and restore-validation failures always fall back to a cold start. The general parameter support window never relaxes state compatibility. See [KV / Prefill Cache Checkpoint](KV_CACHE_CHECKPOINT.md) for the full matrix and operating guide.

## 上游监控 / Upstream Watcher

`scripts/check-llama-upstream.cjs` 读取官方最新稳定版和 `master` 的 `tools/server/README.md` 参数表，并在 `scripts/llama-parameter-baseline.json` 中保存参数名、别名、语法摘要及说明/默认值摘要。

定时工作流 `llama.cpp Upstream Watch` 会分类报告参数新增、删除、别名变化、语法变化和说明/默认值变化。稳定版漂移会使兼容性任务失败；仅主分支发生变化时作为前瞻提醒。两者共用一个去重后的 GitHub 跟踪 Issue，基线始终需要人工复核，不会自动合并修改。

The scheduled watcher classifies additions, removals, alias, syntax, and description/default changes. Stable drift fails the compatibility job; master-only drift remains a canary warning. Baseline updates always require review.

```text
npm run check:llama-upstream
node scripts/check-llama-upstream.cjs --write
```

常规发版检查保持离线，只使用已提交的稳定版注册表和已复核的前瞻参数注册表，验证 Rust 实际生成的全部参数。
