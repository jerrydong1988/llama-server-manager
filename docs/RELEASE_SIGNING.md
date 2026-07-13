# 发布签名配置

正式 `v*` 标签会同时构建 Windows、macOS、Linux x64 和 Linux ARM64 安装包。证书不是发布的硬性条件：没有配置签名服务时，CI 会显示警告并继续发布可测试的安装包；凭据齐全时自动启用对应平台的正式签名。

## Windows：SignPath 开源签名

本项目不再使用可导出 PFX 的旧式证书流程。公开仓库优先申请 SignPath Foundation 提供的免费开源代码签名。

### 申请步骤

1. 确保 GitHub 账号已启用双重验证。
2. 阅读本项目的[代码签名政策](../CODE_SIGNING_POLICY.md)和[隐私政策](../PRIVACY.md)。
3. 打开 [SignPath Foundation 申请页面](https://signpath.org/apply.html)，填写仓库、Release、许可证和维护者信息。
4. 审核通过后，在 SignPath 中连接本 GitHub 仓库，创建项目、Artifact Configuration 和 Signing Policy。
5. Artifact Configuration 应接受 GitHub Actions 上传的 ZIP，至少覆盖其中的 NSIS `.exe` 和 MSI `.msi` 安装包。
6. 在仓库 `Settings > Secrets and variables > Actions` 中配置下表 Secrets。

| Secret | 内容 |
|---|---|
| `SIGNPATH_API_TOKEN` | SignPath API Token |
| `SIGNPATH_ORGANIZATION_ID` | SignPath Organization ID |
| `SIGNPATH_PROJECT_SLUG` | 项目 Slug |
| `SIGNPATH_SIGNING_POLICY_SLUG` | Signing Policy Slug |
| `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG` | Artifact Configuration Slug |

凭据齐全时，CI 会先上传由 GitHub Actions 生成的未签名安装包，再向 SignPath 提交签名请求并等待审批。签名完成后，GitHub Release 只上传 SignPath 返回的 Windows 安装包。

如果任一 Secret 缺失，CI 会在日志和任务摘要中显示警告，并发布文件名带 `-unsigned` 的未签名 Windows 安装包，不会中断其他平台。

## macOS：可选 Developer ID 签名

当前没有 Apple Developer Program 会员也可以发布。未配置下列全部 Secrets 时，Tauri 使用 `signingIdentity: "-"` 生成 ad-hoc 签名的 DMG，Release 文件名带 `-adhoc`；该产物没有 Apple 公证，用户首次打开时可能看到 Gatekeeper 提示。

将来具备 Apple Developer Program 条件后，可在仓库 Actions Secrets 中配置：

| Secret | 内容 |
|---|---|
| `APPLE_CERTIFICATE` | `Developer ID Application` `.p12` 文件的纯 Base64 内容 |
| `APPLE_CERTIFICATE_PASSWORD` | P12 导出密码 |
| `APPLE_SIGNING_IDENTITY` | 完整签名身份，例如 `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` | Apple Developer 账号邮箱 |
| `APPLE_PASSWORD` | 该账号的 app-specific password |
| `APPLE_TEAM_ID` | Apple Developer Team ID |

macOS 生成 P12 Base64：

```bash
openssl base64 -A -in certificate.p12
```

只有六项凭据全部存在时，CI 才会导入临时钥匙串并启用 Developer ID 签名、公证和 stapling。缺失或只配置一部分时会退回 ad-hoc 签名，临时证书和钥匙串不会写入仓库。

## 发布前核对

1. 先在普通提交上确认四个平台的测试、Clippy 和安装包构建通过。
2. 创建 `v*` 标签后检查 GitHub Actions 的 Windows 与 macOS 签名摘要。
3. Windows 已接入 SignPath 时，确认签名请求获批且 Release 中上传的是签名结果。
4. 未配置 Apple 凭据时，确认 macOS 日志明确显示 ad-hoc fallback，而不是静默伪装成已公证版本。
5. 下载 Release 资产，在干净设备上检查安装、首次启动和签名状态。

证书、私钥、密码、API Token 和 Apple 凭据不得提交到仓库。
