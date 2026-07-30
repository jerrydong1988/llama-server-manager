# 贡献与合并流程

本项目的默认分支 `master` 受 GitHub Ruleset 保护。所有代码、文档和版本变更都必须通过 Pull Request 合并。

## 开发流程

1. 同步最新的 `master`：

   ```bash
   git switch master
   git pull --ff-only origin master
   ```

2. 创建独立分支，Codex 任务统一使用 `codex/<简短说明>`：

   ```bash
   git switch -c codex/example-change
   ```

3. 完成修改并运行与变更相匹配的本地检查。涉及发布行为的代码至少运行：

   ```bash
   npm run check:release
   npm run build
   ```

4. 推送分支并创建目标为 `master` 的 Pull Request。
5. 保持分支基于最新 `master`，解决全部审查对话，并等待以下检查通过：

   - `quality`
   - `build-windows`
   - `build-macos`
   - `build-linux`
   - `build-linux-arm64`

6. 通过 GitHub 合并 Pull Request。

## 禁止事项

- 禁止直接向 `master` 提交或推送。
- 禁止强制推送或删除 `master`。
- 禁止为日常开发绕过或停用分支 Ruleset。
- 禁止在必需检查失败或仍在运行时合并。

## 正式发布

版本号和发布说明同样通过 Pull Request 合并。合并且必需 CI 全部成功后，才可基于 `master` 的合并提交创建版本标签和 GitHub Release。上游兼容性检查失败时不得发布。
