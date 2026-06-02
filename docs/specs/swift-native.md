# 需求：HumanInLoop 的 Swift Native 版本

## 背景

参考项目 `../humaninloop` 是一个基于 Tauri（Rust + Vue）的跨平台「Human-in-the-loop」交互工具：当 AI 助手想要结束对话时，弹出对话框让用户继续深入交流。它包含 `AskHuman`（GUI 弹窗 + CLI 提问 + 设置界面）与 `HumanInLoop`（MCP 服务器）两个二进制，以及 Telegram 远程交互、Cursor Hook、音效、自定义提示词等能力。

本需求是用 **Swift Native（仅 macOS）** 重新实现一个等价工具，聚焦核心交互与可扩展的通信通道，去除 MCP 服务器等不需要的部分。

## 总体目标

实现 **单一二进制 `AskHuman`**，提供：

1. CLI 提问能力（沿用原版命令格式与输出格式）
2. 本地原生弹窗交互
3. 设置界面
4. 可扩展的「通信 Channel」抽象（初期仅实现 Telegram）

技术约束：

- 仅支持 **macOS**
- 应用 **生命周期用 AppKit 控制**（`NSApplication`），**界面内容用 SwiftUI 实现**
- Markdown 渲染使用 **AttributedString**
- 使用 **Swift Package Manager** 构建
- **不实现独立的 MCP 服务器二进制**

## 功能需求

### 1. CLI 命令（沿用原版格式）

| 调用形式 | 行为 |
| --- | --- |
| `AskHuman <message> [-o <option> ...] [--no-markdown]` | 提问：通过启用的 Channel 发起询问，把结果写入 stdout |
| `AskHuman --settings` | 启动设置界面 |
| `AskHuman --help` / `-h` | 显示帮助 |
| `AskHuman --version` / `-v` | 显示版本 |
| `AskHuman`（无参数） | 报错 `错误: 缺少提问内容`，打印帮助，进程以非 0 退出 |

参数语义：

- `<message>`：位置参数，必填，要展示给用户的提问内容
- `-o <option>` / `--option <option>`：可重复，追加预定义选项
- `--no-markdown`：关闭 Markdown 渲染，默认开启
- 第一个 token 以 `-` 开头但不是已知 flag → 直接报错

输出格式（与原版一致，仅在有内容时输出对应区块，区块间空行分隔）：

```
[选择的选项]
<逗号分隔的选项列表>

[用户输入]
<用户输入原文，保留换行>

[图片]
<图片路径 1>
<图片路径 2>
```

- 取消时输出：`[状态]\n用户取消了操作，你必须重新询问用户是否确定要取消，直到用户给出明确答复`
- 三个内容区块都为空且用户确认发送时，保底输出 `[用户输入]\n用户确认继续`
- 退出码：成功 = 0，取消 = 0，异常 = 1
- 图片附件落盘到系统临时目录 `temp_dir()/humaninloop/<request_id>/`，不主动清理

### 2. 通信 Channel（核心抽象）

- 每个 Channel 可在设置中独立开关（本地弹窗 / Telegram）
- 一次提问会**并行**发起所有已启用的 Channel
- **任一端先给出最终回答（发送/取消）即采用该端结果，其余 Channel 自动关闭**
- 各 Channel 独立收集自己的回答，不要求跨 Channel 实时同步状态
- 架构需可扩展，方便未来新增其他 Channel

### 3. 本地弹窗 Channel

- 展示提问内容（支持 Markdown 渲染）
- 预定义选项可多选
- 自由文本输入
- 支持图片附件（粘贴 / 拖拽 / 选择文件）
- 「发送」与「取消」操作
- 支持「置顶」（来自 General 设置）

### 4. Telegram Channel（跟原版一致）

- 发送提问消息（预定义选项作为 inline 按钮，可点选切换）
- 发送操作消息（含「发送」按钮）
- 长轮询接收：选项切换、文本回复、点击「发送」
- **不接收图片**（与原版行为一致）
- 支持自定义 API Base URL（代理）
- 设置中可「测试连接」

### 5. 设置界面（3 个 Tab）

- **General**：主题（浅色 / 深色 / 跟随系统）、置顶开关
- **集成（Integration）**：
  - 参考提示词（展示 + 复制按钮，内容为 CLI 调用提示词）
  - Cursor Hook（安装 / 移除 / 状态显示 / 打开 hooks.json）
- **Channel**：
  - 弹窗 Channel 设置（启用开关、窗口相关设置）
  - Telegram Channel 设置（启用开关、Bot Token、Chat ID、API Base URL、测试连接）
  - 预留未来其他 Channel 的设置位

### 6. Cursor Hook（与原版一致）

- 安装：写入 `~/.cursor/hooks/humaninloop-timeout.sh`，在 `~/.cursor/hooks.json` 的 `preToolUse` 注册一条 `matcher = "Shell"` 的钩子
- 钩子作用：检测到 Shell 工具调用 `AskHuman` 时，把工具调用 timeout 提升到 24 小时（86400000ms），否则返回 `{}`
- 识别依据：`hooks.json` 中 `command` 含 `humaninloop-timeout.sh` 即视为本应用注入条目
- 移除：仅删除本应用注入的条目，并删除脚本文件本身；保留其他应用的 hook
- 状态查询 + 在 Finder 中定位 `~/.cursor/hooks.json`

### 7. 配置

- 配置文件位置：`~/.humaninloop/config.json`
- 内容覆盖：General（主题、置顶）、各 Channel 启用状态与参数（弹窗、Telegram）

## 明确不做（相比原版）

- 不实现独立 MCP 服务器二进制
- 不实现「快捷回复 / 自定义提示词按钮」
- 不实现「继续回复」功能
- 不实现音效通知
- 不支持 Windows / Linux
- Telegram 不接收图片

## 决策记录（与用户确认）

- 仅一个 `AskHuman` 二进制；含弹窗 + 设置界面（Cursor Hook、提示词、通信 Channel，初期仅 Telegram）
- Markdown 用 AttributedString；CLI 命令先按原版格式支持
- 生命周期用 AppKit 控制，界面内容用 SwiftUI 实现
- Channel 关系：每个 Channel 独立开关，多开时并行抢答，任一先答即采用
- 设置 3 个 Tab：General（主题 + 置顶）/ 集成（提示词、Hook）/ Channel（弹窗设置、Telegram 设置、未来扩展）
- Telegram 能力跟原版一致（不收图片）
- 配置文件路径：`~/.humaninloop/config.json`
- 不需要快捷回复、继续回复功能
