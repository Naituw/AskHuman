# HumanInLoop

跨平台的「Human-in-the-loop」交互工具。当 AI 助手准备结束对话或需要确认时，调用命令行 `AskHuman` 弹出窗口，让你继续提问、勾选选项、补充文字或附带图片，并把结果回传给 AI。

- 单一可执行文件 `AskHuman`，既是 CLI 又能按需弹出 GUI 窗口
- 基于 **Tauri 2（Rust 后端 + Vue 3 前端）**，支持 **macOS / Windows / Linux**
- 多「通信 Channel」：本地弹窗 + Telegram（可独立开关，多开时并行「抢答」）
- 内置设置界面、Cursor Hook 安装、参考提示词
- macOS 原生毛玻璃外观；纯手写 macOS 风 CSS

## 安装

### 方式一：npm（推荐）

```bash
npm i -g askhuman      # 全局安装，得到 AskHuman 命令
```

只会下载与当前平台匹配的一个二进制（mac arm64/x64、win x64、linux x64）。

### 方式二：GitHub Release

从 [Releases](https://github.com/Naituw/HumanInLoop/releases) 下载对应平台的 `AskHuman-<平台>-vX.Y.Z.(tar.gz|zip)`，解压后把 `AskHuman` 放入 PATH。

### 方式三：从源码构建

需要 [Rust 工具链](https://rustup.rs)、[pnpm](https://pnpm.io)（Node 20+）。

```bash
# macOS / Linux
./install.sh            # 构建并安装到 ~/.local/bin/AskHuman
```

```powershell
# Windows
./install-windows.ps1   # 构建并安装到 %LOCALAPPDATA%\Programs\AskHuman
```

> Linux 运行 GUI 弹窗需系统具备 WebKitGTK（如 `libwebkit2gtk-4.1`）。缺失且配置了 Telegram 时会自动改走 Telegram；两者皆不可用则以退出码 3 提示降级。

## 作为依赖（程序集成）

把 `askhuman` 加入项目依赖（`npm i askhuman`），在代码里解析二进制路径并调用——`npm install` 时会自动装上当前平台的二进制：

```js
import { getBinaryPath, isAvailable } from "askhuman";
import { spawnSync } from "node:child_process";

if (isAvailable()) {
  const r = spawnSync(getBinaryPath(), ["要继续吗？", "-o", "继续", "-o", "停止"], { encoding: "utf8" });
  if (r.status === 3) { /* 无任何可用 channel：降级，不阻塞流程 */ }
  else if (r.status === 0) { /* 解析 r.stdout 的结果区块 */ }
}
```

`getBinaryPath()` 解析顺序：环境变量 `HUMANINLOOP_BINARY` → 平台子包 → 系统 `PATH`。

## 使用

```bash
# 提问（结果写入 stdout）
AskHuman "要不要继续？" -o "继续" -o "停止"

# 附带文件展示（可多次；支持绝对路径 / 相对路径 / ~）
AskHuman "看看这个文档？" -f ~/Documents/spec.md -f ./diagram.png

# 关闭 Markdown 渲染（按纯文本显示）
AskHuman "纯文本内容" --no-markdown

# 打开设置界面
AskHuman --settings

# 帮助 / 版本
AskHuman --help
AskHuman --version
```

### 输出格式

成功时按区块输出（仅在有内容时出现，区块间空行分隔）：

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

退出码：成功 / 取消为 0；无任何可用 channel（本地弹窗打不开且未配置 Telegram）为 3；其他异常为 1。所有日志走 stderr，stdout 仅含结果区块。

## 设置界面

`AskHuman --settings`（或弹窗右上角齿轮）打开，含三个 Tab：

- **通用**：主题（跟随系统 / 浅色 / 深色）、窗口置顶
- **集成**：参考提示词（可复制）、Cursor Hook（安装 / 移除 / 打开 hooks.json）
- **通信渠道**：本地弹窗设置、Telegram（Bot Token / Chat ID / API Base URL / 测试连接）

## 通信 Channel

- **本地弹窗**：默认启用。支持预定义选项、自由文本、图片（粘贴 / 拖拽 / 选择文件）。顶部导航栏可切换置顶、主题、打开设置。提问附带的文件（`-f`）展示在提问下方：单击选中、双击打开、空格预览（macOS 走 QuickLook，其它平台回退为打开）。
- **Telegram**：填写 Bot Token 与数字 Chat ID 后启用。发送提问（选项为 inline 按钮）+ 接收文字回复与「发送」操作；不接收图片。提问附带的文件（`-f`）会一并发送（图片用 sendPhoto、其它用 sendDocument）。

多个 Channel 同时启用时，哪一端先「发送 / 取消」就采用哪一端的结果，其余自动收尾。当本地弹窗所在环境无法显示 GUI（如 Linux 缺 WebKitGTK、无显示环境）时：若已配置 Telegram，会自动改走 Telegram 并在 stderr 说明原因；若无其他可用 Channel，则以退出码 3 告知调用方降级。

## Cursor Hook

在设置「集成」Tab 一键安装（仅 macOS / Linux）。安装后向 `~/.cursor/hooks.json` 的 `preToolUse` 注册脚本（`~/.cursor/hooks/humaninloop-timeout.sh`）：检测到 Shell 调用 `AskHuman` 时，自动把工具调用 timeout 延长到 24 小时，避免等待用户回应时被强制取消。移除时仅删除本应用注入的条目。

## 配置文件

`~/.humaninloop/config.json`，由设置界面读写（原子写入、容错解码）。

## 开发

```bash
pnpm install
pnpm tauri dev          # 启动 Vite + Tauri（调试窗口）
cargo test --manifest-path src-tauri/Cargo.toml   # Rust 单元测试
```

项目概览见 `docs/overview.md`。

## 发布

版本号以 `scripts/bump-version.mjs` 统一同步（`Cargo.toml` / `tauri.conf.json` / 根 `package.json` / npm 主包与平台子包）：

```bash
# 1. 更新版本号（如有变更，统一写入各处）
node scripts/bump-version.mjs 0.2.0
git commit -am "release: v0.2.0"

# 2. 一键发布：校验版本一致/是否已发布 → 打 tag → 推送触发 CI（加 -y 跳过确认）
./publish.sh
```

`publish.sh` 会校验各处版本一致、检查该版本是否已发布（已发布则报错并提示更新版本号），通过后打 tag 并推送；`release.yml` 随即编译 4 平台二进制 → 发布 npm（主包 + 平台子包）→ 创建 GitHub Release。

> 前置条件：在仓库 Settings → Secrets 配置 `NPM_TOKEN`（npmjs automation token）。预发布版本（如 `0.2.0-rc.1`）会以 npm dist-tag `next` 发布并标记为 GitHub pre-release。

发布架构与 channel 降级设计见 `docs/plans/release-and-channel-degradation.md`。
