# 依赖安全审计说明

> 最近复核：2026-07-14，适用于 v2.9.25

发布前使用 RustSec `cargo-audit 0.22.2` 扫描 `src-tauri/Cargo.lock`，结果为 0 个已知安全漏洞。CI 的质量检查也会运行 `rustsec/audit-check@v2.0.0`；以后新增的 RustSec 漏洞会阻止构建。

本次已将直接依赖 `reqwest` 从 0.11 升级到 0.12，并移除不再维护的 `rustls-pemfile 1.x` 依赖链；`mdns-sd` 也从 0.11 升级到当前兼容的 0.20。

RustSec 仍会报告若干不阻止构建的信息性告警：

- Linux 桌面端由 Tauri/WebKitGTK 间接使用不再维护的 GTK3 Rust 绑定，并包含 `glib 0.18` 的特定迭代器 API 告警；本程序没有直接调用该 API。移除这些依赖需要 Tauri 的 Linux 运行时迁移，当前没有可兼容的应用内替代方案。
- `mdns-sd 0.20` 经 `flume 0.12` 间接使用已从 crates.io 撤回的 `spin 0.9.8`。这是当前最新版依赖链，RustSec 没有为它报告安全漏洞；后续上游发布替代版本时再升级。
- `proc-macro-error` 与 `unic-*` 为 Tauri 宏和 URL 处理链的间接依赖，当前仅被标记为不再维护，没有已知安全漏洞。

这些信息性告警不会被静默忽略：每次 CI 都会重新扫描并在日志中列出；一旦升级为漏洞或出现可兼容替代版本，应优先升级依赖。
