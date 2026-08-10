# 依赖安全审计说明

> 最近复核：2026-08-11，基线为 v2.9.42

## npm / GitHub Actions 供应链

CI 在每次 `npm ci` **之前**运行 `scripts/check-npm-supply-chain.cjs`：

- 从 `package-lock.json` 提取所有精确的 npm 包名和版本，查询 OSV 的 npm 恶意软件通告；任何 `MAL-*` 命中都会阻止构建，OSV 不可用或响应异常时也会失败关闭。
- 内置 [Aikido ChainDrop 披露](https://www.aikido.dev/blog/keyv-and-friends-compromised-in-npm-supply-chain-attack)中首批恶意版本的本地拒绝列表，避免仅依赖在线通告。
- 审核所有 `hasInstallScript` 锁文件条目。CI 使用 `npm ci --ignore-scripts`，随后只显式重建已复核的 `esbuild`；可选的 `fsevents` 安装脚本保持禁用。
- 安装完成后、构建开始前扫描 `node_modules`，阻断 ChainDrop 已披露的恶意脚本文件名和下载域名 IOC。
- Pull Request 通过固定提交 SHA 的 GitHub Dependency Review 检查新增依赖风险；所有工作流 Action 同样固定到完整提交 SHA，checkout 不持久化凭据。

平台编译任务只拥有 `contents: read` 权限，不能直接修改 GitHub Release。受保护的更新发布任务不再安装完整 npm 依赖树；它只从 npm 官方注册表下载锁文件指定的 Tauri CLI 包和当前平台二进制包，校验锁文件中的 SHA-512 完整性后用于签名。

本地发布检查 `npm run check:release` 也包含同一 npm 供应链门禁。更新任何带生命周期脚本的依赖时，必须单独复核并同步更新门禁白名单，不能用宽泛的脚本放行替代。

## Rust / Tauri 依赖

CI 的质量检查使用固定提交 SHA 的 `RustSec/audit-check` 扫描 `src-tauri/Cargo.lock`。允许项必须与 `.github/rustsec-allowlist.json` 中带到期日期的精确清单一致；新增 RustSec 漏洞会阻止构建。

本次已将直接依赖 `reqwest` 从 0.11 升级到 0.12，并移除不再维护的 `rustls-pemfile 1.x` 依赖链；`mdns-sd` 也从 0.11 升级到当前兼容的 0.20。

RustSec 仍会报告若干不阻止构建的信息性告警：

- Linux 桌面端由 Tauri/WebKitGTK 间接使用不再维护的 GTK3 Rust 绑定，并包含 `glib 0.18` 的特定迭代器 API 告警；本程序没有直接调用该 API。移除这些依赖需要 Tauri 的 Linux 运行时迁移，当前没有可兼容的应用内替代方案。
- `mdns-sd 0.20` 经 `flume 0.12` 间接使用已从 crates.io 撤回的 `spin 0.9.8`。这是当前最新版依赖链，RustSec 没有为它报告安全漏洞；后续上游发布替代版本时再升级。
- `proc-macro-error` 与 `unic-*` 为 Tauri 宏和 URL 处理链的间接依赖，当前仅被标记为不再维护，没有已知安全漏洞。

这些信息性告警不会被静默忽略：每次 CI 都会重新扫描并在日志中列出；一旦升级为漏洞或出现可兼容替代版本，应优先升级依赖。
