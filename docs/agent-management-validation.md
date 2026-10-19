# CLI / MCP 探索版人工验证

本包来自 `codex/agent-management-exploration`，不是正式版本。确切提交、平台和时间见包内
`BUILD.json`，二进制与文档的 SHA256 见 `SHA256SUMS.txt`；压缩包旁还有独立校验文件。
完整架构、覆盖矩阵和后续边界见 `agent-management-exploration.md`。

## 1. 启动独立测试环境

Windows 解压后应有 `llama-server-manager.exe` 和 `lsm.exe`，放在同一目录。
需要 WebView2；首次运行未签名探索包可能出现系统提示。Linux 需要 GTK/WebKitGTK、桌面会话，
macOS 使用对应架构；本包的 CLI 默认寻找相邻应用二进制，也可用 `--app` 指向已安装应用。
Linux 压缩包不是 AppImage；已有 DEB 构建可用于安装系统依赖。

以下命令在包目录 PowerShell 中运行。将数据目录换成一个新的绝对路径：

```powershell
$AgentData = 'C:\LSM-Agent-Test'
.\lsm.exe --data-dir $AgentData service start
.\lsm.exe --data-dir $AgentData service status
.\lsm.exe --data-dir $AgentData service show --wait
.\lsm.exe schema
.\lsm.exe schema instances.create
```

`service start` 先启动隐藏宿主，`service show` 打开对应的界面。始终传入同一个 `--data-dir`，
这样 CLI 与这个窗口操作同一份配置。省略它会使用应用默认数据目录，不适合第一轮隔离验证。
不要同时让不同版本写同一个目录。关闭窗口只是隐藏；退出 MCP/CLI 不会停止已提交任务。
`service stop --wait` 明确停止宿主及其托管工作负载；不是“仅关闭窗口”。

## 2. PowerShell 参数助手

在 PowerShell 7 中可复制下面的函数。它使用临时 UTF-8 参数文件，避免命令行 JSON 双引号的
转义差异，退出时删除；无需修改全局 PowerShell 配置。响应使用区分大小写的哈希表解析，
以保留引擎参数中的 `-C` / `-c` 等不同键；Windows PowerShell 5.1 请直接读取 CLI 的原始 JSON，
或使用 PowerShell 7 运行此助手。

```powershell
function Invoke-Lsm {
    param([string]$Operation, [hashtable]$InputData = @{}, [switch]$WaitTask)
    $ArgumentFile = [IO.Path]::GetTempFileName()
    try {
        [IO.File]::WriteAllText($ArgumentFile, ($InputData | ConvertTo-Json -Depth 50), [Text.UTF8Encoding]::new($false))
        $CommandArgs = @('--data-dir', $AgentData) + $Operation.Split('.') + @('--args', "@$ArgumentFile")
        if ($WaitTask) { $CommandArgs += @('--wait', '--timeout', '300') }
        $OutputJson = & .\lsm.exe @CommandArgs
        if ($LASTEXITCODE -ne 0) { throw ($OutputJson -join "`n") }
        $OutputJson | ConvertFrom-Json -AsHashtable
    } finally { Remove-Item -LiteralPath $ArgumentFile -Force }
}
```

参数名称、必填项和权限以 `lsm schema <操作名>` 为准。所有业务输出是 JSON；0 表示调用成功，
2 表示服务返回失败，1 表示参数/连接/客户端错误。写操作默认返回 `data.task_id`，加 `--wait`
等到任务结束，结果在 `data.result`，最终状态在 `data.status`。

## 3. 导入引擎与模型，创建实例

```powershell
$EngineDir = 'D:\Engines\llama-bin'
$ModelDir = 'D:\Models'
Invoke-Lsm paths.authorize @{ purpose='engine'; path=$EngineDir } -WaitTask
Invoke-Lsm paths.authorize @{ purpose='model'; path=$ModelDir } -WaitTask
$Revision = (Invoke-Lsm service.status).data.revision
Invoke-Lsm directories.add @{ purpose='engine'; path=$EngineDir; revision=$Revision } -WaitTask
$Revision = (Invoke-Lsm service.status).data.revision
Invoke-Lsm directories.add @{ purpose='model'; path=$ModelDir; revision=$Revision } -WaitTask
Invoke-Lsm engines.refresh -WaitTask
Invoke-Lsm models.refresh -WaitTask
$Inventory = (Invoke-Lsm engines.list).data
$EngineId = $Inventory.engines[0].id
Invoke-Lsm engines.probe @{ id=$EngineId } -WaitTask
Invoke-Lsm engines.default @{ id=$EngineId; revision=$Inventory.revision } -WaitTask

$Revision = (Invoke-Lsm service.status).data.revision
Invoke-Lsm instances.create @{
    id='agent-test'; revision=$Revision
    config=@{ name='Agent 测试'; engine_id=$EngineId; model_path="$ModelDir\your-model.gguf"; host='127.0.0.1'; port=18080 }
} -WaitTask
Invoke-Lsm instances.validate @{ id='agent-test' }
Invoke-Lsm instances.start @{ id='agent-test' } -WaitTask
Invoke-Lsm instances.health @{ id='agent-test' }
Invoke-Lsm instances.logs @{ id='agent-test'; lines=100 }
```

确认引擎列表中选中的是需要测试的引擎，再设置 `$EngineId`。`instances.defaults` 返回全部允许
配置字段与默认值；`engines.probe` 返回所选二进制的实际参数能力。原有 v0.7.0 参数和手动模式
端口规则复用，不另建一套参数转换。未知字段会失败。

例如 `instances.update` 的 `patch:{"temp":0.8}` 会同时把温度标记为显式覆盖，即便它等于当前默认值，
也会进入命令。要恢复引擎继承，从当前 `explicit_overrides` 列表移除该字段，再通过 patch 提交完整列表；
`explicit_overrides:[]` 表示全部参数恢复继承。显式提供此列表时，它具有最终决定权。

创建或更新实例时会先生成和校验启动命令，模型/引擎必须可用。`started:true` 仅表示启动请求完成，
要继续检查健康状态，并发起真实推理请求。更新运行中的实例修改的是下次启动配置；需要显式停启才能生效。

## 4. MCP 接入 Harness

支持 stdio JSON-RPC，协议版本为 `2024-11-05`、`2025-03-26`、`2025-06-18`、`2025-11-25`。
未实现可选 MCP Tasks 扩展，使用普通 `tasks_get` 工具查询异步管理任务。
以下是通用 MCP 配置示例，外层配置位置取决于 Harness：

```json
{
  "mcpServers": {
    "llama-manager": {
      "command": "C:\\LSM-Agent-Package\\lsm.exe",
      "args": ["--data-dir", "C:\\LSM-Agent-Test", "mcp", "--allow-write"]
    }
  }
}
```

- 不加授权参数：只读工具。
- `--allow-write`：刷新、探测、配置、下载入队、实例/路由启停。
- `--allow-delete`：删除模型/实例/引擎库存或下载文件、取消下载、清理下载记录、检查点、用量与可清理存储。
- `--allow-cluster`：节点注册/批准、发现、本地 RPC 和 SSH 管理。
- 每项单独开放。路径授权/撤销、宿主强制退出、登录启动项修改属于本机所有者 CLI，不暴露给 MCP。
  `runtime.detach` 为已授权的后台交接，属于 write；它保留引擎与路由并退出宿主。

工具名使用下划线，例如 `engines_list`、`instances_create`。建议先让 Agent 调用
`service_capabilities`、`instances_defaults`、`engines_list`，再执行具体任务。
目录需要先由 CLI 授权；Agent 不能通过 MCP 自行扩大可访问目录。

示例任务：“刷新引擎和模型，选择指定引擎创建 agent-test 实例，端口 18080；等待启动任务完成，
检查健康状态并报告。”不要把 `task_id` 当作成功：继续查询 `tasks_get`，直到 `succeeded/failed/interrupted/cancelled`。

## 5. 逐项验收与页面位置

| 领域与页面 | 建议验证 | 预期结果 |
| --- | --- | --- |
| 左侧 **引擎管理** | CLI 增加目录、刷新、`engines.rename/probe/probe_all/default` | 界面库存更新；名称与默认选择一致；能力来自实际引擎 |
| 引擎管理 / 实例管理 | `engines.references/delete_preview/delete` | 默认、被配置引用或运行中引擎拒绝移除；允许移除后 `files_deleted:false`，二进制仍在 |
| 引擎管理 | `directories.remove` 后刷新 | 停止扫描该目录，磁盘文件不删除；库存删除后再次扫描原目录会重新发现 |
| 左侧 **模型仓库** | `models.refresh/metadata/delete_preview`；在临时模型上执行删除 | 元信息对应文件；分片删除范围完整；不得删除正在使用的模型 |
| 左侧 **下载管理** | `downloads.browse/add/list`；暂停、恢复、取消、重试 | 文件队列/进度可见；暂停保留可续传内容，取消仅清理受管部分文件 |
| 下载管理 / 下载设置 | `downloads.settings` 设置并发、限速、低优先级、恢复策略 | 宿主重启后配置保留；`manual` 不自动恢复暂停任务，`auto_on_launch` 按原队列规则恢复 |
| 左侧 **实例管理 → 配置参数** | `instances.create/clone/update/validate`，修改温度、上下文等 | 仅指定字段改变；配置搜索可找到对应字段；不支持的参数按原规则报告 |
| 配置参数 → 启动方式/手动命令 | 手动命令省略 `--port`，设置实例端口 | 使用实例端口；命令中的显式端口仍按现有兼容逻辑校验；真实请求可到达 |
| 配置参数 | 未保存温度编辑时由 CLI 更新同一实例 | 草稿保留，旧窗口保存失败；外部结果不被覆盖；备份草稿后重启窗口重新编辑 |
| 实例管理 | `instances.start/health/logs/stop/delete` | 启动/健康分开；日志尾部有界；运行中或仍被路由引用的实例拒绝删除 |
| 左侧 **集群管理** | `cluster.start_local` 指定引擎，随后 list/test/stop/remove | 启动后自动注册本地 Worker；注册失败回收进程；停止后端口释放 |
| 集群管理 | `cluster.add/approve` 使用 `trust:true`；mdns_start/status/stop | 全程无需原生确认弹窗；显式信任才注册，自动发现节点仍需批准 |
| 集群管理 → SSH 启动 | `cluster.ssh_start` 传主机、用户、密钥路径、RPC 端口 | 建立隧道并注册本地转发 Worker；失败可见；ssh_stop/cluster.stop 回收隧道 |
| 左侧 **API 路由** | `routes.get/update/test/targets/start/status/stop/restart` | 共享原有路由校验及启动逻辑；真实转发成功；旧 revision 拒绝 |
| API 路由 → API Key 配置 | 读取后修改 Key 的名称/额度，保留 `key:"[REDACTED]"` 或省略 key | 已有密钥不改变；新增条目必须提供实际 key；输出不泄漏密钥 |
| 左侧 **性能监控 / 大屏** | 运行推理，同时 `instances.health` / `system.health` | 同一运行时数据；Prefill/Decode 实时、阶段平均与不可用状态不混淆 |
| 托盘 / 后台恢复 | 隐藏窗口、退出 Harness，再显示窗口；停止/重启宿主 | 隐藏及客户端 EOF 不停止任务；显式 service.stop 停止；历史任务可查询 |

本机 RPC 仅表示进程与端口可用，分布式推理还需要真实远端 Worker 和实际推理验证。
SSH 停止的端口是返回的本地隧道端口，不是远端 RPC 端口。

## 6. 参数、下载与任务语义

### 参数数值交叉验收

在 **参数配置 → 托管参数模式** 下检查以下位置，也可在参数搜索框输入 flag 直接定位。

| 页面分组 | 参数 | 默认显示 |
| --- | --- | --- |
| 推理与生成 | 温度 `--temp`、Top-P `--top-p` | `0.8`、`0.95` |
| 高级设置 → 高级采样 | Mirostat 学习率 `--mirostat-lr`、XTC 阈值 `--xtc-threshold`、自适应衰减 `--adaptive-decay` | `0.1`、`0.1`、`0.9` |
| 高级设置 → 采样参数扩展 | Min-P `--min-p` | `0.05` |
| 高级设置 → 推测解码 | 草稿分裂概率 `--spec-draft-p-split` | `0.1`；是否可编辑取决于推测模式 |
| 高级设置 → 服务扩展 | slot 提示相似度 `--slot-prompt-similarity` | `0.1` |

1. 切换手动/托管模式，不修改数值：只有模式产生改动，继承参数不自动变成固定参数。
2. 清空温度输入框，逐字输入 `0.83`；对支持负数的自适应目标输入 `-1`。负号、
   小数点和科学计数法的中间输入不应被强制改为 `0`；未完成输入离开焦点时恢复最近有效值。
3. 保存 `temp:0.12345679`、`top_p:0.95`，切换页面及重启宿主后重新查看；CLI
   `instances get`、MCP `instances_get` 和 `data/configs/instances.json` 应一致。
   输入 `0.123456789` 时，保存后应回显实际单精度值 `0.12345679`，没有虚假的未保存改动；
   随后的 CLI/MCP 修改应正常同步到干净的配置页面。
4. 由 CLI/MCP 修改这两个字段，再查看界面；通过 `instances validate` / `instances_validate`
   检查启动命令包含相同数值。`恢复引擎默认` 应取消固定，并从命令中去掉对应参数。
5. 旧文件中的 `0.800000011920929` 等展开值在载入后恢复简洁显示，下次正常保存时写为简洁数字；
   不改变已记录的固定/继承意图，也不修改手动命令文本。
6. 小数参数可输入 `0.51`、`0.123456789` 或 `1e-8`，不应出现步长不匹配提示；
   整数参数可输入非整块大小，例如上下文 `2049`、缓存 `513`。清空整数输入后移开焦点，
   或输入小数后移开焦点，应恢复上一个有效整数，不把空值写成零或把小数截断。
7. CLI/MCP 创建实例或修改配置后，按 `Ctrl+R` 刷新管理窗口，列表、参数与运行状态应继续
   使用宿主的最新配置。关闭窗口、重新显示窗口和重启管理宿主分别验证，不能只测试页面切换。

统一数值序列化覆盖全部 28 个浮点配置字段（采样、RoPE/YaRN、推测解码和 slot 参数）。
这些字段继续使用引擎侧的单精度语义；输出为能回读到同一单精度值的简洁十进制，
不是固定保留两位小数。整数、监控指标及手动命令字符串不经过此处理。

### 下载队列与任务状态

`downloads.browse` 参数：`source` 为 `huggingface` 或 `modelscope`，`repo` 为仓库标识。
`downloads.add` 传 `files` 路径数组及已授权 `directory`；先执行
`paths.authorize {purpose:"download",path:"绝对目录"}`。下载目录仍受原有配置/运行时目录保护约束。

入队返回 `download_id`（队列条目）及 `files[].task_id`（文件任务）。
暂停/取消：`id` 为队列 ID 或文件任务 ID，`file` 为文件名/相对路径；
恢复/重试：`id` 为文件任务 ID。`downloads.clear` 仅清理指定状态记录，不删除已下载模型。
可清理状态为 `completed`、`error`、`cancelled`；运行或暂停中的任务先显式取消。
管理任务完成表示入队或控制操作完成，不代表模型已下载完成；以 `downloads.list` 为准。
`downloads.list` 合并正在运行的进度与持久化历史，暂停/错误记录在重启后仍可查询；
探索版新完成的下载也保留记录，直到显式清理。旧版本已清除的历史不会补造。
单文件暂停也适用于排队文件，其他文件继续排队；恢复时文件任务 ID 保持不变，run ID 和版本更新。

修改请求可传 `idempotency_key` 或 CLI `--idempotency-key`：相同键与参数返回原任务；
相同键不同参数返回 `IDEMPOTENCY_CONFLICT`。只保留最近 200 个管理任务，幂等去重也限定在此窗口内。
`tasks.list` 返回最近 100 个摘要；`tasks.get` 查询具体任务。崩溃时未完成任务变为 `interrupted`，
不会自动重放副作用。调用方必须先核对资源实际状态。

每个连接帧最大 8 MiB；日志最多 1000 行/256 KiB；任务结果超过 128 KiB 时保留“结果省略”标记，
操作完成状态仍保留，应使用对应资源读取接口。最多 32 个待执行修改任务，超出返回 `BUSY`。
同一资源的修改串行执行；慢引擎探测不阻塞无关实例的停止，下载仓库查询在下载队列提交锁外执行。
等待超时不代表取消任务。使用 `lsm tasks cancel --args '{"id":"任务 ID"}'` 或 MCP `tasks_cancel`
取消尚处于 `queued` 的任务（包含下载入队前的网络查询）。取消结果直接返回该任务状态，不创建新的异步任务。
已进入 `running` 的操作返回 `TASK_RUNNING`，请使用对应资源的停止/暂停命令；不会强行中断持久化事务。
重复取消已结束任务只返回现有终态。客户端退出仍不会自动取消任务。

## 7. 验证范围与回退

新增接口及实际 Windows 验证结果见 [非集群交叉验证](agent-management-noncluster-validation.md)。
检查点兼容性必须先调用 `checkpoints.eligibility`；本轮 Qwen3.8 27B 返回
`hybrid_recurrent_unsupported`，不能在该模型上强行启用检查点，也不能把冷启动算作恢复成功。

自动化覆盖 CLI、MCP、认证 IPC、类型/未知字段、配置冲突、密钥脱敏、手动端口、进程生命周期、
路由转发、任务幂等、重启恢复，以及现有浏览器/Rust/运行时回归。
合成引擎用于确定性回归；真实引擎测试单独运行 `scripts/test-management-real.cjs`，需要
`LSM_REAL_ENGINE`、`LSM_REAL_MODEL` 指向现有文件。实际通过结果应以本轮交付说明和 CI 为准。

远端 SSH、跨机器 RPC、长时间大模型下载，以及 macOS/Linux 桌面显示仍需要对应环境人工验证。
当前不包含引擎包自动下载/解压升级器，也不支持无桌面会话的 Linux 管理 daemon。

回退：先 `service stop --wait`，关闭 Harness 的该 MCP 连接，再使用原 master 测试包。
探索目录可保留用于排查；确认不需要后自行删除。已有模型和引擎仍位于原目录。
