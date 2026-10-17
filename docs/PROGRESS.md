# PROGRESS

记录需要跨会话保留的未完成 / 延期事项和明确下一步。任务 / 需求完成后删除其 section
（历史留在 git）。

## 进行中：合并窗口折叠态（需求讨论 → 独立原型 → 正式实现）

2026-10-17 用户要求把已收到的提问标记为「稍后回答」（最初称「暂不回答」/ 内部称「次要」，仍可随时浏览 / 回答）；所有待答请求都是
稍后答时整窗缩成不占屏幕空间、仍显眼的小窗（跟随置顶），切换动画要优雅流畅。需求决策与待确认问题记录在
`docs/specs/popup-inbox-collapse.md`。需求语义已定（决策 C1–C32 在 spec §5），独立原生原型已做好并通过原生几何 /
像素自检（spec §6.2；41 个模型单测），用户已认可动画基线、迷你列表形态、中档显眼度、归档盒行内按钮，正式代码未改动。

下一步：用户在第 7 轮试用后确认手感合适（停留展开 500ms / 切换 250ms / 边缘滚动，C31）、直接松手落该请求的当前题（C32），并选了「可以进入实施计划」（spec §1.8）。实施计划 `docs/plans/popup-inbox-collapse.md`（P0–P9）已于第 8 轮获用户通过：当前主工作树实施、逐阶段提交、原型进仓库、不留实验开关、标题「待答 N」；**全部完成再覆盖日常二进制**，中间阶段用计划 §4.1 的隔离安装验证（C33、C34）。实施进度记在计划 §8。
**打开原型前先问用户**：自动测试已改成静默（`--auto` / `--auto-controls` 不取焦点，C26），会取走焦点的运行（`--focus`、`--burst`、
`--video`、交互原型）必须先征得同意（第 6 轮用户已同意在改完后打开交互原型）。窗口折叠展开的偶发跳动用户已说「先这样吧」（C25），不再追。正式实现要点：小窗尺寸不得写入窗口尺寸偏好；首次呈现前的等待不能依赖动画帧；保留生产画布的屏幕外预留 /
原子提交协议；加上点击切换请求时的正文动画（C17）；「切走缩回」要用应用级激活状态（NSApplication didResignActive + 无任何 AskHuman 窗口为 key）；
拖入图片要新增 Sidebar 条目作为投放目标并支持投给非当前请求（窗口级监听者按落点交给对应请求），带停留展开 / 停留切换 / 进度条 / 上下边缘自动滚动（Sidebar 列表与正文）和按题落点（顺序布局落当前题、纵向布局落指针下的卡，C31、C32，spec §6.8 ⑤），wry 的落点 y 在视口钉大于窗口时（小窗 / 动画中）偏大需要修正，原生拖放事件要串行 + 合并处理而不是逐个并发 `await`（spec §6.7 ②、⑥）。
**新一轮窗口位置不得漂移（C30，用户明确要求）**：原型一轮结束后无 Sidebar 的下一轮窗口沿用了带 Sidebar 窗口的 x（正文逐轮左移），原型不改，但正式实现必须让 `reset` 回到正文锚点（产品现有语义），并覆盖小窗拖动 / 小窗状态结束一轮等路径（spec §6.7 ⑤）；以小窗为锚展开会让正文相对缩小前挪 `left_span`（单请求缩小后来第二条等），计划用 `Geometry.drift` 抵消，不带进下一轮（plan §2.3）。
运行：`./scripts/popup-collapse-demo.sh`；显示器层面的帧测量：`--video`（见 `docs/development.md`）。

## 待观察：拖入文件失败（`daemon.log` 的 `askhuman_drop` 行）

2026-10-17 用户在日常版（v0.14.5）遇到几次「拖图片进窗口没加附件」（来源访达的下载 / 桌面 / 文稿，第二次拖就可以）。根因没有证实（最可能是 macOS 隐私控制拒绝读取，
或第一次落在预览区 / 分隔条的死区）。已加每种情形的可见提示和不含路径的诊断日志，决策与读日志的办法见 `docs/specs/popup-inbox-collapse.md` §6.9。

真实窗口验证（2026-10-17，隔离安装，用户随手拖了 4 次「下载」里的文件）：落到普通请求正文 / 多问题卡片各一次 → `added`，落到「只能选择」的请求两次 →
`not_accepting` 并确认看到橙边提示「…不接收附件」；日志的位置类别、落点、请求编号都对得上，没有读取失败。用户决定不再补测，下面这些**没在真窗口里看过**，只有自动测试覆盖：
桌面文件与系统授权弹窗 / 拒绝后的提示、网页里的图片（应提示改按粘贴）、拖到附件预览区、读取失败 / 部分成功 / 全部重复的提示、成功提示条「已添加 N 个文件到「…」」的外观
（用户没答）、独立弹窗标题栏下的一行。等日常版里真遇到时再对号。

待做：
- **日志出来再定**（目前都不做）：图片读取改成异步（现在是一次同步整文件读取，大图 / 网络盘会卡界面）；遇到 `permission_denied` 自动重试一次（若日志证实「第一次被拒、授权后可以」）；
  预览区接收文件（现在落上去只提示、不收）；拖入过程中对预览区显示「不可放」的视觉。
- 用户下次再遇到「拖不进去」时，先 `rg askhuman_drop ~/.askhuman/daemon.log` 对号入座，再决定上面哪一项（日常版要先用含这批提交的构建覆盖安装，才会有这些行和提示）。

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
