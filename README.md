<p align="center">
  <img src="assets/banner.jpg" alt="AskHuman" width="800">
</p>

# HumanInLoop

跨平台的「Human-in-the-loop」交互工具。当 AI 助手准备结束对话或需要确认时，调用命令行 `AskHuman` 弹出窗口，让你继续提问、勾选选项、补充文字或附带图片，并把结果回传给 AI。

- 单一可执行文件 `AskHuman`，既是 CLI 又能按需弹出 GUI 窗口
- 基于 **Tauri 2（Rust 后端 + Vue 3 前端）**，支持 **macOS / Windows / Linux**
- 多「通信 Channel」：本地弹窗 + Telegram + 钉钉 + 飞书（可独立开关，多开时并行「抢答」）
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

> Linux 运行 GUI 弹窗需系统具备 WebKitGTK（如 `libwebkit2gtk-4.1`）。缺失且配置了会话型渠道（Telegram / 钉钉 / 飞书）时会自动改走该渠道；皆不可用则以退出码 3 提示降级。

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
# 提问（结果写入 stdout）。无 -q 时第一个参数就是问题：AskHuman "X" 等价于 AskHuman -q "X"
AskHuman "要不要继续？" -o "继续" -o "停止"

# 一次提多个问题：第一个参数是所有问题的共享描述（Message），每个 -q 是一个实际问题；
# -o 归其前最近的问题，-f 始终附加在 Message 上（位置不限）
AskHuman "下面是本次改动，请确认几点：" -f ./diff.patch \
  -q "需要保留日志吗？" -o "保留" -o "清除" \
  -q "要开启缓存吗？" -o "开" -o "关"

# 附带文件展示（仅作用于 Message，可多次；支持绝对路径 / 相对路径 / ~）
AskHuman "看看这个文档？" -f ~/Documents/spec.md -f ./diagram.png

# 关闭 Markdown 渲染（按纯文本显示）
AskHuman "纯文本内容" --no-markdown

# 自定义来源名：弹窗标题与 Telegram 消息头由「the Loop」变为指定名称
ASKHUMAN_ENV_SOURCE_NAME=Agent AskHuman "要继续吗？"   # 标题显示 “Question from Agent”

# 打开设置界面
AskHuman --settings

# 面向 AI 的精简用法（仅提问相关：调用方式/参数/用户回应/示例）
AskHuman --agent-help

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

[文件]
/Users/me/Downloads/report.pdf
```

> `[图片]` 为粘贴 / 拖入的图片（落盘后给出路径）；`[文件]` 为拖入回复的非图片文件（直接透传其绝对路径，不复制）。

多个问题时，每题以 `# Qn` 分组、题间用 `---` 分隔；某题未作答输出 `[状态]\n用户未回答此问题`，若所有题都未作答则只输出一次取消提示。单问题不加 `# Qn` 头（与上方格式一致）。Message 仅作描述/附件展示，不进入输出：

```
# Q1
[选择的选项]
继续

---

# Q2
[状态]
用户未回答此问题
```

取消时：

```
[状态]
用户取消了操作，你必须重新询问用户是否确定要取消，直到用户给出明确答复
```

退出码：成功 / 取消为 0；无任何可用 channel（本地弹窗打不开且未配置 Telegram / 钉钉 / 飞书）为 3；其他异常为 1。所有日志走 stderr，stdout 仅含结果区块。

## 设置界面

`AskHuman --settings`（或弹窗右上角齿轮）打开，含三个 Tab：

- **通用**：主题（跟随系统 / 浅色 / 深色）、窗口置顶
- **集成**：参考提示词（可复制）、Cursor Hook（安装 / 移除 / 打开 hooks.json）
- **通信渠道**：本地弹窗设置、Telegram（Bot Token / Chat ID / API Base URL / 测试连接）、钉钉（ClientId / ClientSecret / UserId / 卡片模板 ID / 自动识别 / 测试连接）、飞书（App ID / App Secret / Open ID / 服务域名 / 自动识别 / 测试连接）

## 通信 Channel

- **本地弹窗**：默认启用。支持预定义选项、自由文本、图片（粘贴 / 拖拽 / 选择文件；「添加图片」为输入框内右下角小图标，输入框随内容自增高）。拖入文件时，图片作为图片附件、非图片作为回复文件附件（以胶囊展示、可移除，提交后进入 `[文件]` 区块）。顶部导航栏可切换置顶、主题、打开设置；底部左下角为「取消」。Message 的附件（`-f`）展示在顶部描述区：单击选中、双击打开、空格预览（macOS 走 QuickLook，其它平台回退为打开）；在 macOS 上还可**拖出**到其它应用，以及**右键**弹出 Finder 风格菜单（打开 / 打开方式 / 快速查看 / 在访达中显示 / 拷贝 / 拷贝路径）。
- **Telegram**：填写 Bot Token 与数字 Chat ID 后启用。发送提问（选项为 inline 按钮）+ 接收文字回复与「发送」操作；不接收图片。来源名「Question from {名称}」（见 `ASKHUMAN_ENV_SOURCE_NAME`）。Message 的附件（`-f`）会随 Message 一并发送（图片用 sendPhoto、其它用 sendDocument）。
- **钉钉**：企业内部应用 + 机器人 + Stream 模式（无需公网）。填写 ClientId（AppKey）/ ClientSecret（AppSecret）/ UserId 后启用（机器人 robotCode 即 ClientId，无需单独配置）。UserId 旁的「自动识别」按钮会先校验 ClientId/ClientSecret，再提示你用目标账号私聊机器人发送一个 4 位数字以精确回填。提问以**互动卡片高级版**逐题下发：卡片内勾选预定义选项（多选）、可补充文字，点「提交」完成该题（回调走 Stream，零公网）；作答期间在聊天里发的**图片、文件**会被累积进答案（图片/文件经 Stream 接收并回传给 AI；纯文字请用卡片输入框）。卡片模板需在钉钉卡片平台**搭建并发布**（同一应用），其模板 ID 填入「卡片模板 ID」；**留空则用内置默认模板**。若卡片投放失败会自动**回退**为「纯文本 + 编号选项」（回复编号/文字/图片/文件作答）。Message 的附件（`-f`）会经媒体上传后随 Message 发送。
- **飞书 / Lark**：企业自建应用 + 机器人 + 长连接（WebSocket）模式（无需公网）。填写 App ID / App Secret / Open ID 后启用；「服务域名」留空走飞书国内 `open.feishu.cn`，Lark 国际版填 `open.larksuite.com`。Open ID 旁的「自动识别」按钮会先校验 App ID/Secret，再提示你用目标账号私聊机器人发送一个 4 位数字以精确回填。提问以**互动卡片**（卡片 JSON 2.0，直接下发无需搭建模板）逐题下发：卡片表单内勾选预定义选项（多选）、可补充文字，点「提交」完成该题（回调走长连接，零公网）；作答期间在聊天里发的**图片、文件**会被累积进答案（纯文字请用卡片输入框）。若卡片投放失败会自动**回退**为「纯文本 + 编号选项」。Message 的附件（`-f`）会经媒体上传后随 Message 原生发送（图片用 image、其它用 file）。开放平台应用创建、权限导入与事件/回调配置见 [`docs/feishu-setup.md`](docs/feishu-setup.md)。

> 多问题：弹窗顶部常驻 Message（描述 + 附件），下方以 `Question i/n` 计数逐题切换（底部「上一个/下一个」，全部查看过后右下角出现「提交」一次性回传）；Telegram / 钉钉 / 飞书先发 Message，再逐题串行发送（题首带 `Question i/n`），答完一题再发下一题，全部答完才回传。各端同启时以「整个会话」为粒度抢答——哪端先答完全部即采用该端结果。

多个 Channel 同时启用时，哪一端先「发送 / 取消」就采用哪一端的结果，其余自动收尾。当本地弹窗所在环境无法显示 GUI（如 Linux 缺 WebKitGTK、无显示环境）时：若已配置任一会话型渠道（Telegram / 钉钉 / 飞书），会自动改走该渠道（headless 并行）并在 stderr 说明原因；若无其他可用 Channel，则以退出码 3 告知调用方降级。

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
