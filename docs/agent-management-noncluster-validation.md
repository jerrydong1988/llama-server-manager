# 非集群 GUI / CLI / MCP 交叉验证

范围：探索分支的 Windows 独立测试程序，2026-10-19。此处保留当时的验证证据；后续 master 测试包复验应核对各自的 `BUILD.json`，测试包不作为正式版本发布。
集群不在本轮新增验收范围。真实网络、真实模型与合成回归分别记录，不能互相替代。

## 接口与页面对照

| GUI 位置 | CLI 操作（MCP 将点换成下划线） | 重点验收 |
| --- | --- | --- |
| 模型仓库 → 扫描/资产详情 | `models.refresh/metadata/delete_preview/delete/open_folder` | 元信息、分片组、配置/运行时引用；未授权路径拒绝；删除预览后再次校验 |
| 引擎管理 → 引擎卡片 | `engines.*`、`directories.*` | 重命名、默认选择、单个/批量能力探测；默认或被引用引擎拒绝移除；库存移除不删二进制 |
| 下载管理 → 远程仓库/任务/设置 | `downloads.*` | 两家提供方、限速/并发、暂停/恢复/重试/取消、重启续传、记录与文件分别清理 |
| 实例管理 → 创建/更多/快捷操作 | `instances.*` | 创建/克隆/参数修改/启停；真实连接测试；进程启动不等于模型 ready；运行与路由引用阻止删除 |
| 实例管理 → 更多 → 配置参数 | `instances.update/validate`、`presets.*` | 模式切换、手动端口、浮点值、固定/继承意图、六个共享模板；外部修改与 GUI 草稿冲突 |
| 参数配置 → KV / Prefill 缓存检查点；实例管理 → 检查点状态 | `checkpoints.*` | 资格、停机保存、换进程恢复、首请求实际复用；损坏与指纹失配；运行时不能清除 |
| 实例路由 → 服务/路由规则/API Key | `routes.*` | 优先故障切换、权重、leastBusy、并发队列、限流/超时、密钥范围和配额 |
| 实例路由 → 用量统计/请求明细 | `usage.*` | 四种生成协议、向量/重排；流式和非流式；CSV、游标查询、未知计数、历史清理不重置额度 |
| 性能监控 / 大屏模式 | `instances.health/metrics/slots/resources`、`monitoring.series` | 实时 Prefill/Decode 与阶段平均分开，过期/不支持显示不可用；向量不显示伪造 Decode |
| 性能监控 → 历史会话；服务器日志 | `telemetry.*`、`instances.logs/clear_log` | 样本/请求/分析/诊断一致；日志清除及外部维护刷新 |
| 存储维护 | `storage.*` | 清理前看 blockers；仅已知可清理组；模型、引擎及外部产物保持保护 |
| 托盘/后台运行；侧栏开机自启动 | `runtime.*`、`service.show/hide`、`system.autostart/set_autostart` | 宿主退出后继续实际推理；重新接管同一 PID；独立目录自启动；GUI 状态刷新 |

参数小数位置：**推理与生成**的温度/Top-P；**高级设置 → 高级采样**的 Mirostat 学习率、
XTC 阈值、自适应衰减；**采样参数扩展**的 Min-P。可在参数搜索输入 flag 定位。

## 本轮真实验证

使用维护中的 llama.cpp b11716 Windows ROCm 引擎。网络测试使用 Hugging Face 和 ModelScope。

- 下载：双文件并发/限速、单文件与全局暂停、宿主重启保留偏移、实际续传完成；已有外来文件不覆盖，
  错误转终态后可重试；取消只清理拥有的部分文件；真实三分片模型被首分片引用时，第二分片也拒绝删除。
  正在下载和等待排队的文件均拒绝直接删除；解除引用后整组删除；清除任务记录保留已完成文件。GUI 发起 BGE 下载并暂停，MCP 恢复；CLI 任务同步到 GUI。
- 生成和路由：Qwen3.8 27B 与 TinyLlama 真实双实例，四种生成协议的流式/非流式响应逐条对账；
  主实例停止后的回退、恢复后的重新选择、权重分配、leastBusy、队列等待、超时与取消；
  未授权/范围/模型/上下文/额度错误均有相应回执。
- 向量与重排：Qwen3 0.6B、BGE-small、Jina reranker，29 条真实客户端响应与请求账本核对；
  共 35 条账本记录覆盖拒绝、取消及计数缺失。含额度并发预留、重启保留和清理历史独立性。
- 检查点：TinyLlama 和 Qwen3 0.6B 实际 slot 文件换进程恢复。Qwen3 0.6B 保存 56,893,908 字节，
  481 个提示 token 的首请求复用 480 个，16 token 输出与冷启动一致。
  最新代损坏回退旧代，全部损坏回退冷启动，配置改变拒绝旧指纹。
- GUI：下载进度/暂停/完成、引用模型删除拒绝、GUI 启动 BGE 后经接口健康检查并实际生成 384 维向量、
  GUI 停止后 MCP 确认退出。GUI 保存温度 0.83 后 CLI/MCP 一致，MCP 改为 0.91/0.93 后已打开页面正确回显；CLI 切换登录自启动立即同步开关，GUI 关闭后 CLI 确认为 false。其余界面状态、表单边界和冲突由原生交叉检查与浏览器回归共同覆盖。
- 后台：MCP 交接退出宿主，真实路由仍可推理；CLI 重连新宿主后引擎 PID 不变，历史用量保留；
  显式停止清除后台启动设置并释放工作负载。

真实响应缓存计数是首请求复用的证据。快于监控采样间隔的请求可能没有监控观察值，必须显示不可用，
不能用 idle 状态已重置的零计数代替，也不能仅凭文件 restore 成功宣称命中。

## 本轮修复

1. 下载早期失败未正确进入 error，导致界面看似仍在下载。
2. Windows NTFS 重命名继承旧文件创建时间，导致合法重试产物被误判为无所有权；保留打开的源文件句柄恢复身份。
3. 分片模型的单文件删除/覆盖遗漏首分片引用；GUI 引擎移除与接口共用默认/运行/配置引用保护，并明确显示拒绝原因。
4. 仓库同名文件必须用完整相对路径；根目录精确路径优先，重复选择拒绝。
5. 检查点恢复后的 idle 零计数误报未命中；只记录实际测得的请求计数。
6. CLI 外部清理后，下载卡片、用量/存储页面和自启动开关及时刷新。
7. 登录启动项保留独立 data-dir；裸可执行测试包不请求不匹配的正式安装器更新。

## 可重复执行

常规门禁：`npm run check:release`、`npm run build`、`cargo test`、
`cargo clippy --all-targets -- -D warnings`、`npm run test:management`、`npm run test:runtime-service`。
Cargo 命令在 `src-tauri` 下运行。合成管理测试覆盖接口契约、鉴权、脱敏、配置冲突及确定性生命周期。

真实测试是显式启用的脚本，会启动服务、写入指定的专用目录，并修改其中的测试配置：

```text
node scripts/test-management-downloads-real.cjs --bin <绝对二进制目录> --root <独立验证目录> --engine <绝对引擎路径>
node scripts/test-management-checkpoint-real.cjs --bin <绝对二进制目录> --root <独立验证目录> --engine <绝对引擎路径> --model <绝对模型路径> --id checkpoint-real
node scripts/test-management-routing-real.cjs --bin <绝对二进制目录> --root <独立验证目录> --engine <绝对引擎路径> --model <Qwen绝对路径> --fallback <TinyLlama绝对路径>
node scripts/test-router-real-models.cjs --engine <绝对引擎路径> --generation <生成模型> --embedding <向量模型> --reranking <重排模型> --report <JSON路径>
```

运行前核对脚本参数与固定端口是否空闲；不同套件用不同 root。检查点脚本默认 CPU，路由主实例默认 GPU 99 层；
两者均可显式传 `--gpu-layers`，按硬件选择，不能把参数默认值当作性能建议。
检查点脚本会故意损坏**其测试实例**的缓存并保留载荷回退副本，不能指向日常数据目录。
`runtime.detach {"enable":true}` 会注册当前测试目录的后台登录项；验证后 `runtime.stop` 撤销。

## 明确未承诺的范围

- Qwen3.8 27B 混合架构当前返回 `hybrid_recurrent_unsupported`；本轮确认拒绝有效，没有强开或伪报缓存恢复。
- 没有实际 Windows 注销/重启、断电/磁盘满、长达数小时的大模型下载，也没有在本轮操作 macOS/Linux 桌面。
  跨平台编译与自动回归不能替代对应机器上的验证。
- GUI 安装正式更新未执行；探索包不更改正式发布通道。升级安装器、引擎分发器和无图形 Linux daemon 不在本次接口补齐中。
- 集群未新增实测；既有集群自动化结果不能据此声称远端分布式推理已经验证。

验证记录与回退副本保存在独立部署目录；测试完成时停止实际推理与路由，保留配置用于人工复核。
