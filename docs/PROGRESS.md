# PROGRESS

记录需要跨会话保留的未完成 / 延期事项和明确下一步。任务 / 需求完成后删除其 section
（历史留在 git）。

## 待观察：拖入文件失败（`daemon.log` 的 `askhuman_drop` 行）

2026-10-17 用户在日常版（v0.14.5）遇到几次「拖图片进窗口没加附件」（来源访达的下载 / 桌面 / 文稿，第二次拖就可以）。已加每种情形的可见提示和不含路径的诊断日志，
决策与读日志的办法见 `docs/specs/popup-inbox-collapse.md` §6.9。
2026-10-22 查到并修复了主因：拖拽开始时 AskHuman 不是前台，wry 在 `draggingEntered:` 里读文件被系统拒绝（`permission_denied`）；macOS 上现在带文件的拖拽进入窗口时先取前台、
拖走不松手再还回（`popup_drag_front.rs` + `swift/PopupDragFront.swift`，根因与验证见 spec §6.10）。Dev Instance 里用自动化的 Dock 堆栈拖拽验证过；同日日常版覆盖安装含修复的构建后，用户亲自把 Dock「下载」堆栈里的图片拖进弹窗成功，`askhuman_drop` 记 `added`。

真实窗口验证（2026-10-17，隔离安装，用户随手拖了 4 次「下载」里的文件）：落到普通请求正文 / 多问题卡片各一次 → `added`，落到「只能选择」的请求两次 →
`not_accepting` 并确认看到橙边提示「…不接收附件」；日志的位置类别、落点、请求编号都对得上，没有读取失败。用户决定不再补测，下面这些**没在真窗口里看过**，只有自动测试覆盖：
桌面文件与系统授权弹窗 / 拒绝后的提示、网页里的图片（应提示改按粘贴）、拖到附件预览区、读取失败 / 部分成功 / 全部重复的提示、成功提示条「已添加 N 个文件到「…」」的外观
（用户没答）、独立弹窗标题栏下的一行。等日常版里真遇到时再对号。

待做：
- 日常版已装含修复的构建，继续观察还有没有「拖不进去」，有就先 `rg askhuman_drop ~/.askhuman/daemon.log` 对号入座（这时再出现 `permission_denied` 才是真的权限问题）。
- 修复没在真机上验证过的部分：Finder 窗口 / 桌面作为拖拽来源（只测了 Dock「下载」堆栈）、其他 macOS 版本。钩子靠 wry 的类名与方法名，升级 wry 后要确认它还装得上
  （`daemon.log` 里出现 `askhuman_drag_front` 行、`installed` 为 false 就是没装上）。
- **日志出来再定**（目前都不做）：图片读取改成异步（现在是一次同步整文件读取，大图 / 网络盘会卡界面）；预览区接收文件（现在落上去只提示、不收）；拖入过程中对预览区显示「不可放」的视觉。

## 延期：Cursor CLI 后台完成控制持久性补强

2026-10-15 用户在入口取舍讨论后决定先不做 CLI 接管。计划与审核见
`docs/plans/cursor-cli-durable-control.md`；本轮未恢复功能代码，独立入口也未获确认。
恢复时先重新确认接入方式，再补构建校验、原生队列预检与跨构建验收。Desktop 已恢复为默认
关闭、带原生行为预检的实验功能，见 `docs/plans/cursor-desktop-restoration.md`；不覆盖终端 CLI。

## 待外部验收：统一作答窗口的真实渠道与其他平台

macOS 合并/可选独立模式、原生画布过渡、普通外缘缩放修复、10pt 未查看蓝点和持续圆形扩散已接入并安装。
行为规格见 `docs/specs/popup-request-inbox.md`；实施与验证记录见
`docs/plans/popup-request-inbox.md` §10–24。最近 Rust 1256 passed / 3 ignored、
最新前端 252 项通过；原生几何 harness 84 项通过，真实 Sidebar 拖动及宽度恢复已验证。
自动化外缘拖动未成功改变窗口尺寸，
程序化几何证据不代替物理拖动观感验收。

2026-10-08 已在 Windows 11 24H2 VM 的交互式桌面验证合并窗口核心流程、到达气泡、草稿与选区、
最小化恢复、Sidebar 拖动、附件三栏和混合类型取消，并复现修复 #15 的空白窗口残留；见计划 §27。
用户已同意延期的真实 IM 终态、Linux 实机、DPI/多屏矩阵仍待补齐（AskHuman 项目
todo #4：a7d02da0-72d5-4191-954d-d0bcb77d322e）。上述 Windows popup-only VM 证据
不能代替这些外部 gate；补齐后删除本节。

2026-10-09 macOS 又完成气泡两次弹跳 / 750ms 勾线飞行和正文本地 Markdown 图片的安装与实窗
验收，见 `docs/plans/popup-arrival-notice.md` 和 `docs/plans/popup-markdown-images.md`。
2026-10-10 正文本地图片又改为有界原文件 URL / 懒加载 / 超限回退，并经安装和用户实窗验收。
其他平台补验应使用最新动效参数，覆盖正文图片的本地 / 项目相对路径、专用协议 URL、
超限 / 解码失败提示和点击打开原图。Windows / Linux 已按用户要求核对官方资料及依赖源码；
本轮没有对应实机证据，详见 `docs/investigations/base64-usage-audit.md`。
后续平台补验还应覆盖历史、待办草稿与 Interject 的纯显示图片 URL、移除 / 切换 / 卸载清理、
超限文件胶囊与提交原路径；本机自动验证记录见 `docs/plans/display-image-urls.md`。
右侧附件大图也应补验原文件 URL、特殊格式临时 PNG、缓存淘汰 / 过期读取释放和 GIF / WebP 动画；
历史继续保存原路径，记录见 `docs/plans/attachment-image-urls.md`。

## 待外部验收：Windows / Linux Popup 附件预览面板

实现、macOS 安装及用户体验验收已完成，自动回归和真实 WKWebView / AppKit 验证记录见
`docs/plans/popup-attachment-preview-panel.md` §9；行为规格见
`docs/specs/popup-attachment-preview-panel.md`。已验证固定右侧、分隔线、外缘缩窄后的主区恢复、
最大化后恢复、阅读状态、图片动画 / 平移、原文件打开及多题草稿提交。展开总尺寸未污染主区偏好。

2026-10-08 已补齐 Windows 11 VM 的安装与 Markdown 三栏展开、到达动效排除预览区、Sidebar
分隔线和草稿保留，见 `docs/plans/popup-request-inbox.md` §27；未据此验收全部附件格式。
剩余为 Windows 完整附件 / 浏览器 / 拖放矩阵、Linux 实机安装、DPI / 多屏、X11 / Wayland：保留已确认的外部
gate（AskHuman 项目 todo #3），完成后删除本节。Markdown 浏览器打开也应在这两平台补验系统默认浏览器、HTML 文件关联不同、启动失败和真实菜单；本地记录见 `docs/plans/markdown-browser-open.md`。不得以 macOS 或纯计算测试代替这些平台实机证据。

2026-10-09 分区搜索已实现并通过 macOS 安装/交互验收，见 `docs/specs/popup-find-scopes.md` §9。
这两平台补验附件列表打开后 Ctrl+F 的区域路由、文本/Markdown/长 diff 的计数与定位、
窄区布局及输入法；PDF 沿用明确的不支持提示。当前自动测试不能代替这些平台实机证据。

## 待外部验收：Windows / Linux 合并窗口的稍后回答与小窗

macOS 的「稍后回答」标记、小窗「待答 N」、拖入文件的停留展开 / 切换、切走缩回已实现，并由用户在真实窗口验收；需求决策见
`docs/specs/popup-inbox-collapse.md`，实施与验证记录见 `docs/plans/popup-inbox-collapse.md` §8。Windows / Linux 的代码路径
（窗口即时变形 `popup_canvas::instant`、以窗口 `Focused` 事件作「用户在 AskHuman」信号、CSS grid 布局、拖放落点单位候选）已写好，
在 macOS 上通过类型检查和自动测试，并在 headless Chromium 里量过：单请求的列表不再挤占正文的网格列。但**没有在任何一台 Windows / Linux
机器上运行过**；用户已同意延期（计划 §1、§3 P7，AskHuman 项目 todo #1）。实机补验清单：

- 窗口变形：缩小 / 展开 / 小窗里来新请求时，小窗内容区是否恰好是页面量出的宽高（Windows：`set_resizable(false)` 去掉粗边框后客户区有无变化；
  GTK：先改尺寸再锁定的顺序，窗管会不会把尺寸钉住或还原）；展开后位置是否仍以小窗左上角为锚、最小尺寸是否恢复；拖动小窗后再展开的位置。
  Wayland 合成器不接受应用设定的位置（代码忽略被拒的位置），看窗口落在哪里是否可接受。
- DPI 与多屏：125% / 150% / 200% 缩放；在缩放不同的显示器之间拖动小窗再展开，尺寸有无取整偏差。
- 页面布局（WebView2 / WebKitGTK）：小窗的列表宽度、3 行高度与滚动，「待答 N」标题和按钮；这两个平台没有红绿灯，顶部留白与展开的 Sidebar 相同；
  系统标题栏下的小窗是否被截断或难看；单请求缩小 / 展开的瞬间正文不闪、不被挤。
- 切走缩回（窗口 `Focused` 事件）：peek 时切到别的应用 → 200ms 后缩回，回来则取消；系统文件选择框 / 确认层打开时是否被误判为离开
  （Windows 的文件对话框会让 popup 失去焦点）；弹窗不抢焦点地弹出时起始状态是否为「不在场」。
- 拖入文件：Windows（物理像素）与 GTK（CSS 像素）的落点单位是否都选对（`dropPoints` 先试一种、失败再退另一种）；停留展开 / 切换 / 边缘滚动的手感。

不得以 macOS 或纯计算测试代替以上实机证据；完成后删除本节。

## 待外部验收：Windows 发布候选

功能与架构实现已在 `codex/windows-platform-parity` 完成；设计、实施记录和 Win11 证据见
`docs/specs/windows-platform-parity.md`、`docs/plans/windows-platform-parity.md` §17.9 与
`docs/plans/windows-unsigned-update-policy.md` §12。当前 Win11 24H2 VM 已通过 PS5/PS7 install、
named-pipe daemon、完整 Rust tests（1127 passed / 2 ignored）、Windows update 专属测试（19/19）、
Clippy、165 Vitest + 5 Node tests、production/release build、真实 authenticated Codex 0.147 E2E、
卸载维护链和未签名 binary fail-closed / `update prepare` 文件锁闭环。交互式桌面已覆盖统一图标、Ctrl
快捷键、Advanced、设置稳定性、真实飞书取消、Windows Terminal 精确 focus、Dev Instance 以及无闪窗
login/logout。外部平台逻辑 Review 的 correctness 项也已收口：daemon metadata watcher、共享 Windows
path identity、launchId 平台标记和 rename 覆盖语义均有 Windows 原生回归证据。

发布认证仍依赖仓库外状态：

- 准备干净 Windows 10 22H2 x64 VM 并复跑核心矩阵；
- 生产 Authenticode、timestamp、publisher identity pinning 与 SmartScreen 认证已决定暂缓，后续作为
  独立签名项目恢复；当前 unsigned Windows 已固定关闭自动 apply，并提供 `update prepare` 手动闭环；
- 补齐真实交互式 Windows 桌面的 DPI/多屏/输入法/文件选择/声音完整矩阵；
- 如发布认证要求覆盖每个 IM provider，以真实凭据补跑飞书以外渠道；当前飞书真实取消链路已通过，
  其余渠道为 deterministic mock 覆盖。

以上 gate 完成后删除本节。Windows ARM64、原生 installer、Windows Server/RDS 多会话是已确认后置
项目，不属于当前 release candidate blocker。

## 待验收：本地 Markdown Mermaid 图表的跨平台实机运行

完整实现、自动测试、本地浏览器 sandbox / 布局验证与 bundle spike 已完成，详见
`docs/plans/mermaid-rendering.md` 的实施记录。仍需在 Catalina 级 WKWebView、Windows WebView2 与
Linux WebKitGTK 分别跑一次计划 §8.3 的图型、错误、主题、Find 和回答流程矩阵；当前 macOS Tauri
Popup 已完成本机验收。2026-10-09 又补齐 Markdown 附件预览与离线浏览器快照，用户确认两入口与
深色背景正常；实现、Safari Find / 打印和背景修复记录见 `docs/plans/attachment-mermaid-rendering.md`。
跨平台补验应包含新增附件入口。实机 gate 未齐前不降低安全等级或提高系统要求。

## 定期同步：Codex Shell 判定复刻（codex-permission-remember §6.4）

权限记忆功能复刻了 Codex 的 Shell 判定逻辑（`src-tauri/src/shell_safety.rs` +
`permission_shell.rs`）。版本门控只设下限（`VERIFIED_CODEX_VERSION_FLOOR` = **0.122**，
hook 引入版）；`VERIFIED_CODEX_VERSION_CEILING`（当前 **0.146**，对拍来源 Codex upstream
main `1a817bb95d`，2026-07-24；同批吸收 0.145.0 的 #34271 禁选前缀扩容与 #32232
hook-before-guardian 语义，见 spec D50-D53）是最近一次逐行对拍的版本，用户装机超出它时
功能**保持启用**、worker stderr 记一条日志。因此同步不再是紧急事项，但仍需**定期**
（Codex 新 minor 发布后）对拍以下上游文件并抬升已审计版本（相对 codex-rs/）：

- `shell-command/src/bash.rs`（`bash -lc` 脚本拆分）
- `shell-command/src/command_safety/is_safe_command.rs`、`is_dangerous_command.rs`（heuristics）
- `core/src/exec_policy.rs`（fallback 判定 / amendment 派生 / `BANNED_PREFIX_SUGGESTIONS`）
- `config/src/loader/`（配置层叠与项目信任，影响 rules 文件发现与 managed 检测）
- `codex execpolicy check` 的 CLI 契约（参数与 JSON 输出；有 ignored 集成测试
  `permission_shell::tests::real_codex_cli_contract_when_available` 可拿真机验证）

无差异则只改常量 + 记录新 commit；有差异先改 port 再抬已审计版本。若上游出现我方未携带的
**放宽**类变更（新增 safe 命令等），只影响覆盖率；出现语义级破坏（拆分格式、hook 契约）时
fail-closed 机制会自动降级为基础弹窗，届时按 D35 修订的证据链重新评估。

## 待办：Cursor 全局 Rules 迁移为用户级 always-on Skill

调查与候选设计见 `docs/investigations/cursor-global-rule-user-skill.md`。无 workspace folder 的 Cursor IDE
不创建项目 Rules 加载器，因此不会读取 `~/.cursor/rules/askhuman.mdc`。未来改为用户级
`~/.cursor/skills/askhuman/SKILL.md`，旧安装显示“需更新”，迁移时先写新 Skill、再清理旧托管 MDC。
Grok 默认会扫描 Cursor Skills，候选 frontmatter 已设计为对 Cursor 常驻、对 Grok 不可调用。

## 待办：daemon 二进制变化检测 —— 轮询 vs filewatch（后续评估，优先级低）

二进制变化检测目前是 **15s 轮询** `current_exe()` 指纹（稳态≈1 次 `stat`，靠 `binhash.json` 内容哈希缓存避免重哈希）。
是否改 **filewatch** 待权衡——难点：二进制走原子替换（rename 换 inode，需盯父目录 + 按文件名过滤 + 每次替换后重挂，
参考 `config_watch.rs`）、装在任意目录（`~/.local/bin`/brew/npm 前缀/`.app` bundle…）、且 watcher 仍要 stat/hash 才能确认
内容**真**变（指纹是内容哈希而非 mtime）。延迟要求松（~15s 够）+ Hello 路径兜底，故暂保持轮询。

## 待办：daemon 即时换新与在途提问恢复（延期）

用户希望评估：本地安装或版本升级时，无需等现有提问答完即可更新 daemon，换新后继续原提问。
2026-10-07 已完成初步可行性调查；用户决定暂不处理，记录为 AskHuman 项目 todo #7
（`416b9760-cf6e-4cf1-bed1-22a04fab1df5`）。尚未修改功能，也未确认具体恢复体验或实施方案。

当前请求/抢答状态只在 daemon 内存中，关停会取消在途请求；已受理请求的 CLI 在连接断开时返回
退出码 3，Popup Host 在 EOF 时退出，前端作答草稿也未持久化。可行方向是增加可靠的请求/终态存储、
稳定请求身份与恢复凭证、CLI/Popup 重连，以及答案确认和副作用去重。CLI 进程可在 daemon 重启期间
保持存活和原 stdout 管道，但现有旧客户端没有恢复能力，首次上线及不兼容协议仍需安全回退。

下一步先确认保留旧 Popup Host 还是重建新版弹窗并恢复草稿，再设计跨版本兼容、多等待者、IM 卡片
接管和 Windows exe 文件锁处理；以上均为待评估事项，现有 graceful drain 策略继续有效。

## 待办：多份安装互顶刹车的收尾（Windows 与配置扫描）

2026-10-22 落地了 Daemon 的 hold（同一对二进制 10 分钟内互顶 ≥2 次后不再让位）、换新原因记录和
`AskHuman doctor install`，决策与验收见 `docs/specs/daemon-graceful-drain.md` D10–D12、§6。macOS 上做过
隔离 e2e 与真机 PATH 扫描。下面两项用户决定之后再做（AskHuman 项目 todo #2、#3）：

- `scripts/install-windows.ps1` 装完后调用 `AskHuman doctor install --warn-only`。Rust 侧已跨平台编译，
  但 Windows 上的 PATH 扫描（只查 `AskHuman.exe`，不解析 npm 的 `.cmd` shim；`canonicalize` 会带
  `\\?\` 前缀）与 `--version` 探测没在 Windows 实机验过。
- 扫描各 Agent 的 hooks / MCP 配置里写死的 AskHuman 路径，与 Daemon 实际运行的二进制对比。
  现有补救是装完用 `AskHuman agents update` 重写。
