# Agent：为本项目准备 Git WorkTree（Dev Instance）

> 供 agent 在**新建或进入** git worktree 做本仓库功能开发时阅读。  
> 规格：`docs/specs/dev-instance-parallel.md`。

## 何时读本文

任务需要：

- `git worktree add …` 新建并行工作树，或  
- 在已有 worktree 里开发，且希望 **install / AskHuman 不打断主环境、不与其它 agent 互抢 daemon**  

则**先完成本文准备**，再改代码。

主环境日常使用（`~/.local/bin` + `~/.askhuman` + 生产 bot）与本文无关；未 `dev enable` 的树行为与过去一致。

## 目标体验

准备完成后，agent **提示词仍只需**：

```bash
./scripts/install.sh
AskHuman "…"   # 或 MCP ask
```

不必在提示词里分支 env / PATH；cwd 在已 enable 的树内会自动改道到该树的 bin + daemon。

Windows 对应安装命令是 `.\scripts\install-windows.cmd`；其余 `AskHuman dev …` 命令相同。

## 准备步骤

### 1. 进入 worktree 根目录

```bash
cd /path/to/your-worktree
```

### 2. 用 AskHuman 询问操作者（必须）

在 `dev enable` 前，通过 AskHuman **向人确认**：

1. 本 worktree 是否需要 **IM 测试渠道**（飞书 / 钉钉 / Telegram / Slack）？  
   - **不需要** → popup-only，`dev enable` 不带 `--preset`。  
   - **需要** → 继续下面 2a / 2b。

2a. 已有机器级预设时：

```bash
AskHuman dev preset list
```

把列表给操作者选择一个或多个名字，然后：

```bash
AskHuman dev enable --preset <name>
# 多个：--preset a --preset b
```

若报「已被其它 worktree 占用」：告知人可在对方树 `dev disable`，或确认后使用 `--force`（会抢租约；对方若仍跑着 daemon 可能双连 bot）。

2b. 还没有合适预设时：

```bash
AskHuman dev enable
AskHuman --settings          # 在 GUI 里只配【测试】bot，勿用生产凭据
# 或：AskHuman channel set …
AskHuman dev preset save <name> --from-instance
```

之后其它新树可直接 `dev enable --preset <name>`。

### 3. 安装本树二进制

```bash
./scripts/install.sh
```

在已 enable 的树内会装到 `.askhuman-dev/bin/`，**不会**覆盖 `~/.local/bin`（除非 `./scripts/install.sh --global`）。

Linux/Nix 构建应使用提供 GTK/WebKit 依赖及 `GSETTINGS_SCHEMAS_PATH` 的 Nix 环境。
构建脚本会把这份 schema 数据根目录编入 ELF，进程启动时补充自身的 `XDG_DATA_DIRS`，
所以安装后退出构建 shell 仍可打开附件选择器；不需要改登录环境或给实例 bin 套包装器。
未提供元数据时保持普通构建行为；显式提供的路径无效则构建失败。Nix store 依赖仍须保留，
不要把本机编译产物视为跨机器可移植包。旧 daemon 的环境不会原地改变，验证时应在无活动
请求后正常切换到新 daemon，不能沿用手工补过路径的旧进程作为通过证据。

Windows PowerShell / cmd：

```powershell
.\scripts\install-windows.cmd
# 显式更新生产安装（逃生口）：.\scripts\install-windows.cmd -Global
```

Windows 默认安装同样只写 `.askhuman-dev\bin\AskHuman.exe`，不会修改生产 EXE、用户 PATH 或受管
`WindowsApps\AskHuman.cmd` launcher。

Linux 安装使用同目录暂存与原子交换，允许旧 ELF 仍在运行。旧文件保留在安装目录下
`.askhuman-install-<pid>-<nonce>/AskHuman`，供存活 MCP、daemon 及其子进程继续使用；
安装器不会自动清理这些目录。仅在确认没有相关进程或后续自启动依赖后另行清理。
最终 `AskHuman` 若为符号链接或目录则拒绝安装；文件系统不支持原子交换时保留原安装并报错。
新实现启动时记住安装入口，后续 MCP 提问与独立 daemon/GUI Host 启动使用该入口，daemon
也在该入口检测更新；运行中版本的身份与其 Popup 子进程仍使用原映像。
已在修复前启动的进程无法补获入口缓存，需在适当时机普通重启客户端才能获得这部分行为。
macOS 沿用原有复制和签名流程，本条 Linux 替换保证不适用于 macOS/Windows。

### 4. 自检（可选）

```bash
AskHuman dev status
AskHuman daemon status    # endpoint 应按本树 .askhuman-dev/home/ 独立派生
```

## 禁止事项

- **不要**把生产 bot 凭据写进 dev 实例，也不要指望从主配置一键导入（产品不提供）。  
- **不要**在两棵树上同时 `enable --preset` 同一预设（平台长连接互斥）；需要并行真 IM 时准备**两套**测试 bot / 两个预设。  
- **不要**为了「走 dev」去改 agent 全局提示词分支；靠目录标记自动改道。
- **不要**在实例里运行会改 Agent 集成的显式命令（`AskHuman agents …`、设置页的 Agent 集成开关）来「试一试」：
  `ASKHUMAN_HOME` 不隔离 `~/.cursor/hooks.json`、`~/.codex/…`、`~/.claude/settings.json` 等，这些写入会落到**真实**用户配置。
  实例 daemon 启动时的自动迁移已被跳过，但显式命令不受限（见 spec §7.1）。
- **不要**为了验证只换 `HOME` 起 daemon / GUI Host：沙箱必须同时设**非空 `ASKHUMAN_HOME`**（实例上下文）。
  launchd label（GUI Host 登录项）按用户全局唯一，与 `HOME` 无关，生产上下文的进程会 `bootout` 并替换真实登录项。
  需要验证 Agent 集成写入时，用一次性假 `HOME` + 非空 `ASKHUMAN_HOME`，收尾核对真实 hook 文件与登录项未变。

## 结束 worktree 时（可选）

```bash
AskHuman dev disable           # 停实例 daemon、释 preset 租约、去 enabled；保留 bin/home
AskHuman dev disable --purge   # 连 .askhuman-dev 目录一起删
```

## 故障简表

| 现象 | 处理 |
|---|---|
| `binary is missing` | 在本树跑 `./scripts/install.sh`；Windows 跑 `.\scripts\install-windows.cmd` |
| 提问仍进主 daemon | 确认 cwd 在 enable 的树内；PATH 上的 `AskHuman` 是否已含 dispatcher（至少全局装过一版本功能） |
| preset 占用冲突 | 对方 `dev disable` 或本树 `--force` |
| 设置改到了生产 config | 未 enable；先 `dev enable` 再 `--settings` |
