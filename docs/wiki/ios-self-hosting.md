# 进阶：自己编译 App 与自建推送中继

简体中文 | [English](./ios-self-hosting.en.md)

默认路径——从 TestFlight 安装作者发布的 App，推送走公共中继——不需要本文，见 [iOS App 渠道配置](./ios-setup.md)。

只有你要**用自己的 Apple 开发者账号编译并安装 App**（用自己的 Bundle ID）时才需要这些。公共中继只服务作者发布的 App，会拒绝其它 Bundle ID 的登记，所以自己编译的 App 要配上你自己的推送中继和 APNs 密钥。

**需要准备**：Apple 开发者账号、Xcode，以及一个腾讯云账号（用来部署中继的云函数）。

## 1. 准备 Apple 开发者资源

1. 在 Apple Developer → Identifiers 注册两个 App ID：主 App（勾选 **Push Notifications**）和通知扩展（不需要任何能力）。作者发布的 App 用 `com.naituw.humaninloop.ios` 与 `com.naituw.humaninloop.ios.notification`；你要换成自己的，并把 `ios/project.yml` 里的 Team 与 Bundle ID 一并改成自己的。
2. Keys → 新建一个启用 **Apple Push Notifications service (APNs)** 的密钥，团队和环境要与你的 App 匹配：开发构建用 Sandbox，TestFlight / 正式版用 Production。下载 `.p8` 文件（**只能下载一次**），记下 **Key ID** 和你的 **Team ID**。新建的密钥可以按团队或指定 App 授权、按环境区分；已有同时支持两个环境的旧密钥仍可使用。一个中继实例只配一把密钥，两种环境都要用时分别部署。以 [Apple 当前的推送认证说明](https://developer.apple.com/documentation/UserNotifications/establishing-a-token-based-connection-to-apns)为准。

## 2. 部署推送中继

1. 在仓库里运行 `relay/scripts/build-zip.sh`，得到 `relay/dist/askhuman-relay.zip`。
2. 腾讯云控制台 → 云函数 → 新建 **Web 函数**：运行环境 Node.js 20，地域「中国香港」（无需备案、能直连 APNs），内存 128 MB，超时 15 秒，上传 zip。
3. 在函数配置里填环境变量：

   | 变量 | 内容 |
   |---|---|
   | `APNS_KEY_P8` | `.p8` 文件的全部内容（直接粘贴，换行可以写成字面的 `\n`） |
   | `APNS_KEY_ID` | 密钥的 Key ID |
   | `APNS_TEAM_ID` | 你的 Team ID |
   | `APNS_TOPIC` | 主 App 的 Bundle ID（要和 `ios/project.yml` 里的一致） |
   | `RELAY_HANDLE_KEY` | 32 字节随机数的 base64（例如 `openssl rand -base64 32`） |
   | `REVOKED_RIDS` | （可选）要手动作废的句柄编号，逗号分隔 |

4. 创建**函数 URL**（公网访问，授权类型「开放」）。访问 `<函数URL>/v1/health`，应返回 `{"ok":true,"v":1,"apns":true}`。

> `.p8` 和 `RELAY_HANDLE_KEY` 只放在云函数的环境变量里，不要放进仓库、聊天记录或截图。

## 3. 编译并安装 App

1. 用 Xcode 打开 `ios/AskHuman.xcodeproj`（先 `cd ios && xcodegen generate`），确认 Team 与 Bundle ID 是你自己的。
2. 选你的设备，Run。首次启动时允许通知。

## 4. 让 Mac 使用你的中继

在 AskHuman 设置 → **通信渠道** → **iOS App** 卡片的「更多设置」里选「使用自己的中继」，填函数 URL，点 **测试连接**，再把自己签名的 App 重新配对到这台 Mac（见[配置指南的第 3 步](./ios-setup.md#3-配对手机)）。命令行：在 `AskHuman channel set ios …` 后面加 `--relay-url <函数URL>`。

## 费用

使用公共中继不产生你的费用。自己部署时，腾讯云云函数的调用量很小（每月约 1 千到 2 万次）；部署前在费用中心核对一下你账号的计费规则，避免意外的固定月费。
