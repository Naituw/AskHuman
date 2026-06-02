# HumanInLoop (Swift Native)

`humaninloop` 的 macOS 原生重写版本：一个用于「Human-in-the-loop」交互的 CLI 工具。当 AI 助手想结束对话时，调用 `AskHuman` 弹出原生窗口，让你继续提问、选择选项、补充文字或附带图片，并把结果回传给 AI。

- 单一二进制 `AskHuman`
- AppKit 管理生命周期 + SwiftUI 实现界面
- 可扩展的「通信 Channel」：本地弹窗 + Telegram（可独立开关，多开时并行抢答）
- 内置设置界面、Cursor Hook 安装、参考提示词
- 仅支持 macOS 13+

## 构建与安装

```bash
# 构建
swift build -c release

# 安装到 ~/.local/bin/AskHuman
./install.sh
```

## 使用

```bash
# 提问（结果写入 stdout）
AskHuman "要不要继续？" -o "继续" -o "停止"

# 关闭 Markdown 渲染
AskHuman "纯文本内容" --no-markdown

# 打开设置界面
AskHuman --settings

# 帮助 / 版本
AskHuman --help
AskHuman --version
```

### 输出格式

成功时按区块输出（仅在有内容时出现）：

```
[选择的选项]
继续

[用户输入]
记得保留日志

[图片]
/var/folders/.../humaninloop/<id>/img-1.png
```

取消时：

```
[状态]
用户取消了操作，你必须重新询问用户是否确定要取消，直到用户给出明确答复
```

退出码：成功 / 取消为 0，异常为 1。

## 设置界面

`AskHuman --settings` 打开，包含三个 Tab：

- **General**：主题（跟随系统 / 浅色 / 深色）、窗口置顶
- **集成**：参考提示词（可复制）、Cursor Hook（安装 / 移除 / 打开 hooks.json）
- **Channel**：本地弹窗设置、Telegram 设置（Bot Token / Chat ID / API Base URL / 测试连接）

## Cursor Hook

在设置「集成」Tab 中一键安装。安装后会向 `~/.cursor/hooks.json` 的 `preToolUse` 注册一个钩子脚本（`~/.cursor/hooks/humaninloop-timeout.sh`）：当检测到 Shell 工具调用 `AskHuman` 时，自动把工具调用 timeout 延长到 24 小时，避免等待用户回应时被强制取消。移除时仅删除本应用注入的条目，不影响其他钩子。

## 通信 Channel

- **本地弹窗**：默认启用，支持预定义选项、自由文本、图片（粘贴 / 拖拽 / 选择文件）。
- **Telegram**：在设置中填写 Bot Token 与数字 Chat ID 后启用。发送提问（选项为 inline 按钮）+ 接收文字回复与「发送」操作；与原版一致，不接收图片。

多个 Channel 同时启用时，哪一端先点「发送 / 取消」就采用哪一端的结果，其余自动关闭。

## 配置文件

配置存储于 `~/.humaninloop/config.json`，由设置界面读写。

## 开发

```bash
swift build      # debug 构建
swift test       # 运行单元测试
```

文档：

- 需求：`docs/specs/swift-native.md`
- 开发计划：`docs/plans/swift-native.md`
