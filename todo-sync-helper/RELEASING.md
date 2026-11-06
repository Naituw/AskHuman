# 同步组件的构建与公证

需求见 [正式分发规格](../docs/specs/todo-sync-release.md)，当前进展见
[实施计划](../docs/plans/todo-sync-release.md)。本流程只生产组件；正式版本号、真实验收和公开发布另行安排。

## 本地构建

需要 Xcode、XcodeGen、已有 Developer ID Application 证书和本组件的 Production profile。
profile 必须属于 `DMJXDB9H6Q`，App ID / 容器与 `project.yml` 一致。

```sh
python3 todo-sync-helper/scripts/release.py build \
  --profile /absolute/path/to/helper.provisionprofile \
  --identity '<Developer ID Application certificate SHA-1>' \
  --output "$PWD/todo-sync-helper/build/candidate"
```

构建先生成 universal app，再嵌入已有 profile、按最小所需的 Production CloudKit / push 权限签名。
它不会请求 Xcode 自动创建 App ID、容器或 profile。已有 profile 中 CloudKit service 为 `*`
是允许的，但实际签名只请求 `CloudKit`。

## 配置本地公证登录

使用维护者指定的现有 `notarytool` 钥匙串 profile 时，只需其名称。
如果需要新建 profile，维护者可以在终端交互输入 app-specific password：

```sh
xcrun notarytool store-credentials AskHumanTodoSync-Notary \
  --apple-id '<your Apple ID>' --team-id DMJXDB9H6Q
```

该命令安全提示输入密码并存到钥匙串。也可以用维护者提供的 App Store Connect Team API key：

```sh
xcrun notarytool store-credentials AskHumanTodoSync-Notary \
  --key /private/path/AuthKey.p8 --key-id '<key ID>' --issuer '<issuer UUID>'
```

登录方式和存储见 [Apple TN3147](https://developer.apple.com/documentation/technotes/tn3147-migrating-to-the-latest-notarization-tool)。
公证凭据由维护者明确选择，保持既有 TestFlight 签名工具的凭据边界。

## 上传候选并生成可嵌入资源

确认候选后执行；这个步骤会上传软件给 Apple：

```sh
python3 todo-sync-helper/scripts/release.py notarize \
  --app "$PWD/todo-sync-helper/build/candidate/AskHumanTodoSync.app" \
  --output "$PWD/todo-sync-helper/build/candidate/AskHumanTodoSync.zip" \
  --keychain-profile AskHumanTodoSync-Notary --allow-apple-upload
```

只有 Apple 返回 `Accepted`、staple 成功、ticket / Developer ID / Production 权限 / 两种架构
全部检查通过，才产生可发布 zip。公证原理见
[Apple 的分发文档](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)。

提交后立即把 submission ID 写到候选目录的 `notary-<签名代码哈希>.json`，每次最多等 10 分钟。
超时或中断后，对同一份 app 重跑上述命令会继续等待原提交，不重复上传。CI 还会保留
`todo-sync-notary-recovery` artifact（receipt 和 `notary-input.zip`）：恢复时在空目录解开 input zip，
把 receipt 放在 `.app` 同级，再运行 notarize。保留同一份签名候选，避免为等待服务结果而重新构建。

```sh
ASKHUMAN_TODO_SYNC_ARCHIVE="$PWD/todo-sync-helper/build/candidate/AskHumanTodoSync.zip" \
ASKHUMAN_REQUIRE_TODO_SYNC_RESOURCE=1 cargo build --release \
  --manifest-path src-tauri/Cargo.toml --features custom-protocol

src-tauri/target/release/AskHuman todo-sync resource-info --require-bundled
src-tauri/target/release/AskHuman todo-sync resource-verify
node scripts/todo-sync-resource-smoke.mjs \
  --binary src-tauri/target/release/AskHuman \
  --archive todo-sync-helper/build/candidate/AskHumanTodoSync.zip
```

`resource-verify` 和 smoke 只在临时目录检查，既不安装、执行或注册 helper，也不访问 iCloud。
smoke 会实际 npm pack / 解包，并模拟旧版直接更新器仅复制主程序，再核对资源哈希与公证。
测试 Intel 构建时传 `--arch x64`；ARM 构建可传 `--arch arm64`。
专门测试拒绝未公证资源时传 `--expect-unnotarized`，不能把它当成正式资源验收成功。

## GitHub Actions 配置

继续使用已有 `MACOS_CERT_P12_BASE64`、`MACOS_CERT_PASSWORD`、`MACOS_KEYCHAIN_PASSWORD`，
为 release job 增加维护者选定的以下 secrets：

| 名称 | 内容 |
|---|---|
| `MACOS_TODO_SYNC_PROFILE_BASE64` | 本组件已有 Production `.provisionprofile` 的 base64 |
| `MACOS_NOTARY_KEY_BASE64` | 用于 CI 公证的 Team API `.p8` 的 base64 |
| `MACOS_NOTARY_KEY_ID` | 该 key 的 ID |
| `MACOS_NOTARY_ISSUER` | 该 key 的 issuer UUID |

也可以沿用本机的 Apple ID + 应用专用密码认证，配置 `MACOS_NOTARY_APPLE_ID`、
`MACOS_NOTARY_PASSWORD` 及相同的 `MACOS_TODO_SYNC_PROFILE_BASE64`。CI 会把凭据存进
任务的临时钥匙串，再用 profile 调用公证工具；无需另外创建 API key。完整 Team API key 配置
存在时优先使用该配置，否则使用 Apple ID 方式。密码由维护者通过 GitHub CLI 的交互输入设置：

```sh
gh secret set MACOS_NOTARY_APPLE_ID --repo Naituw/AskHuman
gh secret set MACOS_NOTARY_PASSWORD --repo Naituw/AskHuman
```

本地钥匙串 profile 名称不能代替 CI 的认证内容。CI 用临时钥匙串签名，用临时文件传给 notarytool，
任务退出时清理临时证书 / profile / API key；凭据文件和密钥不进产物或仓库。
维护者批准配置后，可通过 stdin 设置加密 secrets：

```sh
base64 < /private/path/helper.provisionprofile | gh secret set MACOS_TODO_SYNC_PROFILE_BASE64 --repo Naituw/AskHuman
base64 < /private/path/AuthKey.p8 | gh secret set MACOS_NOTARY_KEY_BASE64 --repo Naituw/AskHuman
gh secret set MACOS_NOTARY_KEY_ID --repo Naituw/AskHuman
gh secret set MACOS_NOTARY_ISSUER --repo Naituw/AskHuman
```

CI 先生成已公证 universal helper，再把同一份 zip 嵌入两个 Mac 架构的主程序。
资源缺失、签名 / 公证失败都会阻止后续发布；Windows / Linux 构建不嵌入该资源。

手动触发 release workflow 默认 `verify_only=true`：只构建和验证 artifact，跳过 npm 与
GitHub Release 发布。配置好凭据并推送已审阅的源代码后，可以先跑：

```sh
gh workflow run release.yml --repo Naituw/AskHuman --ref main -f verify_only=true
```

真正发布仍走已确认版本的 tag 流程；手动设置 `verify_only=false` 会进入现有发布阶段，
因此只在明确执行发布 / 恢复发布时使用。

## 发布前真实验收

独立账号首次启用、稳定版 macOS 14+、系统后台项批准、双机断网编辑后恢复、睡眠唤醒、
升级 / 回滚、24 小时运行和性能基线仍须记录真实结果。`resource-verify` 不代替这些验收。
