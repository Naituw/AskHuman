<p align="center">
  <img src="assets/banner.jpg" alt="AskHuman" width="800">
</p>

<p align="center">简体中文 | <a href="./README.en.md">English</a></p>

# AskHuman

跨平台的「Human-in-the-loop」交互工具。当 AI Agent 准备结束对话或需要确认时，调用命令行 `AskHuman` 弹出窗口，让你继续提问、勾选选项、补充文字或附带图片，并把结果回传给 AI。

- 单一可执行文件 `AskHuman`，既是 CLI 又能按需弹出 GUI 窗口
- 基于 **Tauri 2（Rust + Vue 3）**，支持 **macOS / Windows / Linux**
- 多通信渠道：本地弹窗 + Telegram + 钉钉 + 飞书，可独立开关、多开并行「抢答」
- 内置设置界面、Cursor Hook、参考提示词；macOS 原生毛玻璃外观

## 安装

```bash
# npm（推荐）：只下载与当前平台匹配的一个二进制
npm i -g askhuman
```

也可从 [GitHub Releases](https://github.com/Naituw/HumanInLoop/releases) 下载对应平台压缩包，解压后把 `AskHuman` 放入 `PATH`。从源码构建见[开发文档](docs/development.md)。

> Linux 运行 GUI 弹窗需系统具备 WebKitGTK（如 `libwebkit2gtk-4.1`）；缺失且配置了会话型渠道时会自动改走该渠道。

## 使用

### 一、AskHuman 命令

```bash
# 提问（结果写入 stdout）。无 -q 时第一个参数即问题
AskHuman "要继续吗？" -o "继续" -o "停止"

# 多问题：第一个参数是共享描述(Message)，每个 -q 是一题，-o 归其前最近的问题
AskHuman "请确认几点：" -q "保留日志？" -o "保留" -o "清除" -q "开启缓存？" -o "开" -o "关"

# 附带文件 / 图片展示（作用于 Message，可多次；支持 绝对 / 相对 / ~ 路径）
AskHuman "看看这个？" -f ~/Documents/spec.md -f ./diagram.png

# 其它
AskHuman "纯文本" --no-markdown   # 关闭 Markdown 渲染
AskHuman --settings              # 打开设置界面
AskHuman --help                  # 帮助
AskHuman --version               # 版本
```

结果按 `[选择的选项]` / `[用户输入]` / `[图片]` / `[文件]` / `[状态]` 区块写入 stdout，日志走 stderr。完整的调用方式与输出格式见 `AskHuman --agent-help`。

### 二、与 AI Agent / Cursor 搭配

让 Agent「结束前先问人」，通常三步：

1. **提示词进 rules**：设置页「集成」Tab 提供可复制的参考提示词，把它加入你的 Agent 规则（如 Cursor 的 rules / `AGENTS.md`），引导 Agent 在结束或需要确认时调用 `AskHuman`。
2. **Cursor Hook**（仅 macOS / Linux）：设置页一键安装，向 `~/.cursor/hooks.json` 注册脚本——检测到 Shell 调用 `AskHuman` 时，自动把工具调用超时延长到 24 小时，避免等待你回应时被强制取消。
3. **程序集成**：把 `askhuman` 加入项目依赖（`npm i askhuman`），`npm install` 会自动装上当前平台二进制，运行时解析路径并调用：

```js
import { getBinaryPath, isAvailable } from "askhuman";
import { spawnSync } from "node:child_process";

if (isAvailable()) {
  const r = spawnSync(getBinaryPath(), ["要继续吗？", "-o", "继续", "-o", "停止"], { encoding: "utf8" });
  if (r.status === 3) { /* 无任何可用 channel：降级，不阻塞流程 */ }
  else if (r.status === 0) { /* 解析 r.stdout 的结果区块 */ }
}
```

> 退出码：成功 / 取消为 `0`；无任何可用 channel 为 `3`；其它异常为 `1`。
> 自定义来源名：设环境变量 `ASKHUMAN_ENV_SOURCE_NAME=Agent`，弹窗标题与渠道消息头变为 `Question from Agent`。

## 配置

配置存于 `~/.askhuman/config.json`，由设置界面读写。通用配置与环境变量见[配置文档](docs/wiki/configuration.md)；各通信渠道接入见 [Telegram](docs/wiki/telegram-setup.md) · [钉钉](docs/wiki/dingtalk-setup.md) · [飞书 / Lark](docs/wiki/feishu-setup.md)。

## 开发

本地构建、测试与发布流程见[开发文档](docs/development.md)（English）。
