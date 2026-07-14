# Rust 依赖审计基线与 v2.9.26 发布设计

日期：2026-07-14

状态：方案已确认，待实施

## 1. 目标

消除当前 CI 中可由项目修复的依赖和 GitHub Actions 警告，对当前 Tauri 2 无法移除的传递依赖公告建立精确、可追踪的受控例外，并在完整跨平台验证后发布 v2.9.26。

## 2. 根因

- `spin 0.9.8` 已被撤回，由 `mdns-sd -> flume` 引入；更新锁文件可升级到 `spin 0.9.9`。
- `rustsec/audit-check@v2.0.0` 使用 Node 20；官方提交 `858dc40f52ca2b8570b7a997c1c4e35c6fc9a432` 已迁移到 Node 24。
- 17 个公告来自当前 Tauri 2 的 Linux GTK3 依赖和 `tauri-utils -> urlpattern 0.3`。当前 Tauri 2.11.5 仍依赖 GTK 0.18，无法在本项目中独立升级到 GTK4。

## 3. 受控例外

仅允许以下公告：

```text
RUSTSEC-2024-0370
RUSTSEC-2024-0411
RUSTSEC-2024-0412
RUSTSEC-2024-0413
RUSTSEC-2024-0414
RUSTSEC-2024-0415
RUSTSEC-2024-0416
RUSTSEC-2024-0417
RUSTSEC-2024-0418
RUSTSEC-2024-0419
RUSTSEC-2024-0420
RUSTSEC-2024-0429
RUSTSEC-2025-0075
RUSTSEC-2025-0080
RUSTSEC-2025-0081
RUSTSEC-2025-0098
RUSTSEC-2025-0100
```

白名单必须直接写在工作流中并附上来源说明。不得关闭 RustSec 信息类检查，不得使用通配规则。任何新增漏洞、撤回依赖、未维护公告或 soundness 公告都必须使 CI 产生新的可见结果并阻止发布评审。

## 4. 实施

1. 更新 Rust 锁文件到当前兼容版本，至少包含 Tauri 2.11.5、Tauri Build 2.6.3、Tauri Utils 2.9.3 和 spin 0.9.9。
2. 将 RustSec Action 固定到 Node 24 官方提交，并传入 17 个精确公告编号。
3. 增加发布回归检查，验证 Action 固定提交、精确白名单、禁止宽泛忽略及版本一致性。
4. 将应用版本更新为 2.9.26，并同步 GUIDE、Cargo、Tauri 和 npm 元数据。
5. 在功能分支完成本地验证和 PR CI，再合并 master、创建 `v2.9.26` 标签并监控正式发布流水线。
6. 正式发布说明必须披露受控例外、零已知漏洞结果和未来随 Tauri 上游迁移清理白名单的要求。

## 5. 验收条件

- `spin 0.9.8` 不再存在于锁文件。
- GitHub Actions 不再产生 Node 20 或“18 warnings found”注解。
- npm 审计零漏洞，RustSec 零未忽略漏洞和零新增警告。
- 前端发布检查、生产构建、Rust 测试、格式检查和严格 Clippy 全部通过。
- Windows、macOS、Linux x64 和 Linux ARM64 安装包全部构建成功。
- GitHub Release v2.9.26 包含完整中文发布说明和各平台安装包。

## 6. 非目标

- 不迁移到尚未发布稳定版的 Tauri v3。
- 不放弃 Linux 构建以换取空依赖树。
- 不修改或维护 GTK、glib、urlpattern 的私有分叉。
- 不把受控例外描述为上游依赖已经修复。
