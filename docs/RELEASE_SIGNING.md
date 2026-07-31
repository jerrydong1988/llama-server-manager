# 发布签名配置

正式 `v*` 标签会同时构建 Windows、macOS、Linux x64 和 Linux ARM64 安装包。操作系统证书不是发布的硬性条件：没有配置签名服务时，CI 会说明 fallback 并继续发布可测试的安装包；凭据齐全时自动启用对应平台的正式签名。Tauri Updater 签名是独立且强制的发布门禁，不能用 Windows 或 Apple 证书替代。

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

## Tauri Updater 与 Cloudflare R2

仓库使用 GitHub Environment `release-r2` 隔离正式 Updater 私钥和 R2 写入凭据。普通 push 与 pull request 不进入该环境：各平台只使用临时密钥让 Tauri 生成正确的 Updater 包装格式；四个平台构建完成后，`publish-updater` 才从 SignPath 结果或明确标记的 fallback 中选择最终文件，并使用正式私钥重新签名。

`release-r2` 需要以下 Environment Secrets：

| Secret | 内容 |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri Updater 私钥完整内容 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 私钥密码 |
| `R2_ACCESS_KEY_ID` | 仅限更新存储桶的 R2 S3 Access Key ID |
| `R2_SECRET_ACCESS_KEY` | 对应的 R2 S3 Secret Access Key |
| `R2_ENDPOINT` | 账户级 R2 S3 API 地址 |
| `R2_BUCKET` | `llama-server-manager-updates` |
| `R2_PUBLIC_BASE_URL` | R2 自定义域 HTTPS Origin，不带结尾 `/` |

发布顺序固定为：

1. 构建并完成可选的 Windows SignPath、macOS Developer ID 与公证流程。
2. 收集 Windows NSIS、Windows MSI、macOS `.app.tar.gz`、Linux x64 AppImage 和 Linux ARM64 AppImage。
3. 对最终字节重新生成 Tauri `.sig`，再构造包含全部平台条目的 `latest.json`。
4. 先将版本化产物上传到 R2 的 `releases/v<version>/`，使用长期不可变缓存。
5. 将 `latest.json` 最后上传并设置为必须重新验证；随后从公开自定义域下载并逐字节核对。
6. 相同 Updater 产物和清单同时附加到 GitHub Release 作为备份。

Tauri 私钥或密码一旦丢失，已安装版本将无法验证后续更新。私钥不得提交到仓库；维护机的恢复副本应放在仓库外，并另做离线备份。更换 R2 写入凭据不影响客户端，因为客户端只访问公开下载域名，不持有任何 R2 密钥。

`v2.9.35` 及更早版本没有 Updater，首个启用版本必须手动安装。Linux DEB 不执行应用内更新，AppImage 才进入自动更新路径。

## 发布前核对

1. 先在普通提交上确认四个平台的测试、Clippy 和安装包构建通过。
2. 创建 `v*` 标签后检查 GitHub Actions 的 Windows 与 macOS 签名摘要。
3. Windows 已接入 SignPath 时，确认签名请求获批且 Release 中上传的是签名结果。
4. 未配置 Apple 凭据时，确认 macOS 日志明确显示 ad-hoc fallback，而不是静默伪装成已公证版本。
5. 确认 `publish-updater` 使用 `release-r2` 环境成功，并从自定义域返回与 CI 生成内容一致的 `latest.json`。
6. 下载 Release 资产，在干净设备上检查安装、首次启动、Updater 检查与签名状态。

证书、私钥、密码、API Token 和 Apple 凭据不得提交到仓库。
