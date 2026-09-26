# AppImage 恢复验证候选包

本包仅用于验证，尚未恢复正式 AppImage 发布或 Linux 自动更新。它使用 Tauri 已合并的 [#16062](https://github.com/tauri-apps/tauri/pull/16062)，固定打包工具源码为 `8e7028331ad37ac2db74d4ec20e66be5cacf2c40`。应用源码提交、工具哈希和产物依赖检查见 `build-info.json`。

## Ubuntu 上运行

1. `uname -m` 为 `x86_64` 时下载 x86_64 候选包；为 `aarch64` 时下载 aarch64 候选包。解压 GitHub Actions artifact ZIP。
2. 先退出已安装的 LlamaServerManager，并停止其后台运行时及引擎。候选包使用同一配置目录；如已有配置，先备份 `${XDG_DATA_HOME:-$HOME/.local/share}/LlamaServerManager`。验证期间不要开启登录自启动。
3. 在解压目录打开终端，运行：

```bash
sha256sum -c SHA256SUMS
chmod +x ./*.AppImage
bash ./run-with-diagnostics.sh
```

首次测试使用桌面默认环境，不额外设置 `GDK_BACKEND`、`WEBKIT_DISABLE_DMABUF_RENDERER` 或软件渲染参数。脚本会记录已有设置，并在当前目录保存 `appimage-test-*.log`，不会上传日志。窗口关闭后若只是缩到托盘，请从托盘退出。

如果提示缺少 FUSE，只影响 AppImage 挂载；可以使用 `bash ./run-with-diagnostics.sh --extract` 重新测试，并在反馈里注明。此模式不能替代普通挂载模式的验证。

## 请验证这些操作

- 窗口出现实际内容，文字、图标、菜单正常；切换页面、调整窗口大小、明暗主题都能刷新。
- 文件/目录选择对话框正常；扫描现有引擎与模型，读取引擎参数，启动一次已有模型并完成一次请求，然后正常停止。
- 验证后台运行时模式：启动引擎、退出界面保留后台、重新打开并恢复状态，再停止引擎与后台运行时。确认没有 GIO、动态库或路径失效错误。
- 下载列表及联网功能正常。如需跨发行版发布，还需另行验证 Fedora/openSUSE 的 WebView HTTPS 与相关桌面图像加载。
- 如桌面提供两种会话，分别验证 Wayland 和 X11/XWayland。没有第二种会话时注明即可，无须改变系统设置。amd64 的结果不代表 ARM64 已验证。

反馈请包含：Ubuntu 版本、`uname -m`、显卡/驱动、桌面会话类型、是否使用 `--extract`、通过或失败的操作，以及日志。若出现空白窗口，请附窗口截图。日志可能包含本机路径，分享前可自行遮盖。

## 验证边界

CI 会从最终 AppImage 解包，检查架构、Wayland 库污染、GIO 模块、WebKit 辅助进程及依赖解析，并测试外部进程的环境隔离。这些检查不等同于 Ubuntu 桌面的实际渲染成功。必须收到实机反馈后才能决定恢复正式发布；本候选流程不创建 Release、更新器清单或网站下载入口。

外部引擎和系统工具会过滤指向当前 AppImage 挂载目录的库/模块路径，保留宿主机的 CUDA/ROCm 路径和桌面会话变量。应用自身及 WebKit 仍使用打包时配套的环境。
