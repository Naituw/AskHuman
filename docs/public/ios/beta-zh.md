---
title: AskHuman Mobile 内测
lang: zh-Hans
---
# AskHuman Mobile 内测

[English](../beta/) · [隐私政策](../privacy-zh/)

在 iPhone 上回答自己 Mac 上 Agent 的提问。按下面五步配置；可选自部署方案在文末。

**开始前**：准备一台 Mac 和 iOS 17 或以上的 iPhone。

## 1. 安装 Mac 端

Mac 上未安装 Node.js 时，先安装 [Node.js LTS](https://nodejs.org/zh-cn/download)。然后打开终端执行：

```bash
npm install -g askhuman@latest
AskHuman setup
```

在向导里接入你使用的 Agent；其它聊天工具可以先跳过。完成后重启已打开的 Agent，让接入生效。

## 2. 连接 Broker

Broker 是 Mac 和手机之间的中转站，由你自备，AskHuman 不绑定某个服务。

终端执行 `AskHuman --settings` 打开设置，进入「通信渠道」→「iOS App」，点「开始设置」。卡片的第 1 步包含 EMQX Cloud Serverless（国内托管）、flespi（欧洲托管）和自建 Broker（如 Mosquitto），每种都有对应的操作步骤。相同的事实，以及各服务的免费额度和缺点，见 **[Broker 的要求、选择与验证](https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-broker.md)**。

填写主机、用户名和密码，点「测试连接」。这个账号必须只能读写 `ah/<用户名>/#`，读不到别人的话题。可以开启「保持这台 Mac 随时可连」，手机上就不会显示这台 Mac 离线；使用期间保持 Mac 在线、唤醒。

想让 Agent 帮你做？点卡片上的「让 Agent 帮我配置」，把复制的文字发给它。

## 3. 安装手机 App

卡片的第 2 步有一个二维码，对应[加入页面](https://naituw.github.io/AskHuman/)。在 iPhone 上安装 [TestFlight](https://apps.apple.com/app/testflight/id899247664)，打开这个页面，加入测试，安装 AskHuman，并允许通知。

## 4. 配对手机

1. 在卡片的第 3 步点「配对新手机」。手机打开「Mac」页 →「添加 Mac」→ 扫描二维码。
2. 核对两边的 **6 位确认码** 一致，在 Mac 点「允许」。二维码过期就重新生成，有效期 5 分钟。

## 5. 测试收答和通知

设置完成后，卡片显示「已就绪」。点「发一条测试问题」，在手机上回答；这是不调用模型的收答测试。

想测通知，在手机上关闭 AskHuman 后再发一次。测试问题会立即推送；其它问题默认要等 Mac 空闲 15 分钟才推送（卡片上的「推送时机」可改），手机收件箱不受这个等待限制。

遇到问题，把失败页面截图和两端版本号发给维护者；遮住密码、连接串和配对码。

## 可选：自己部署推送中继

适合有 **Apple 开发者账号和 Xcode**、希望使用自己服务的人：

1. 用自己的 Team 和 Bundle ID 编译、安装 iOS App，并创建匹配的 APNs 推送密钥。
2. 将仓库的 `relay/` 部署到自己的腾讯云云函数，配置上述密钥，确认 `/v1/health` 检查通过。
3. 在卡片的「更多设置」里选「使用自己的中继」，填入中继地址，测试连接后，与自己签名的 App 重新配对。

具体命令和配置见 **[完整自部署指南](https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-setup.md)**。
这条路线使用自己签名的 iOS App；前面的 TestFlight 方案使用维护者提供的 App 和公共中继。
