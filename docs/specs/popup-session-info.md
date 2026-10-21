# 弹窗 Agent / 项目悬浮信息与稳定工作区

## 当前状态（2026-10-15）

五家原生标题读取与新提问刷新已实现；标题快照与补齐事件已传至合并 / 独立作答区。
只按可信 `(agent_kind, agent_session_id)` 读取，不要求生命周期追踪开启，不按项目或进程猜测会话。
首屏不等待读取；当前原生名称优先于旧注册表缓存，读取失败保留缓存，再使用原有首条用户消息回退。
已打开请求不订阅重命名或轮询；新请求 / 宿主恢复时读取一次。补齐按 request ID 隔离，不重建表单。

用户试用顶栏下方的灰色会话标题行与临时原生调参工具后，认为常驻一行导致界面拥挤，
明确取消直接显示。标题行、预留高度、专用样式与临时调参工具的源码已撤下，并重新安装。
原来的单行来源栏与 Sidebar 会话标题保留。

2026-10-17 用户重新选定直接显示的方式：不加任何常驻行，把会话标题放进 macOS 标题栏（红绿灯旁本来就
空着的一条），Windows / Linux 写进原生窗口标题。已实现并安装到本机，见下方
[标题栏显示](#标题栏显示2026-10-17)；它不占布局高度，不属于上面被取消的旧设计。

Agent 与项目两张悬浮卡及稳定工作区修正已实现，并安装到本机。常驻来源栏仍为原有单行胶囊；
Agent / 项目胶囊统一为 20px 高、16px 行高，不因 span / button 默认样式产生高度差。
实施与验收记录见 [`../plans/popup-session-info.md`](../plans/popup-session-info.md)。

## 已确认需求（2026-10-15）

| 决策 | 结论 |
| --- | --- |
| 常驻布局 | 不在顶栏下方增加会话标题行或预留高度；保留原有单行 Agent / 项目胶囊（直接显示改走标题栏，见[标题栏显示](#标题栏显示2026-10-17)） |
| 信息归属 | 两个胶囊分别提供悬浮信息，不把目录信息全部放进 Agent 卡片 |
| Agent 核心信息 | 完整会话标题、可复制的 Session ID |
| Agent 首版扩展 | 当前模型 / 推理强度、上下文占用与剩余；采用有可靠来源的原生数据 |
| 项目核心信息 | 项目完整路径；调用目录与项目不同时额外显示调用目录 |
| 项目首版扩展 | Git 分支、Worktree 身份、未提交变更摘要 |
| 路径动作 | 项目路径与不同调用目录支持复制、快速在终端打开；macOS 使用 Terminal.app |
| 缺失指标 | 省略没有可靠来源的指标行，不显示占位的“暂无数据”，不伪造 0 值 |
| 项目身份 | 跟随可信会话所属工作区，不被 Agent 临时 cd 到 Temp 等目录改变 |
| 跨仓库行为 | 会话属于 A、CLI 切到 Git 仓库 B 后提问，项目归属仍为 A；调用目录 B 在项目卡片中展示 |
| 归属修正范围 | 统一覆盖提问所属项目、Sidebar 分组、回复历史、待办与 IM，而非只替换顶栏文案 |
| 缺失来源 | 无可信会话工作区时回退现有 cwd 规则；不得猜测最近打开的项目 |
| 文件路径 | 用户要求保持现有规则；-f 附件仍按真实调用 cwd，正文继续使用现有请求项目基准，不另加路径猜测 |
| 数据采集 | 只读原生记录并复用现有 Hook / Extension；不新增或接管 Claude Status line 配置 |
| 历史 | 只修正新请求的项目归属，不迁移 / 重写旧历史 |
| 悬浮打开 | 悬停 100ms 打开，覆盖此前提议的 300ms |
| 指标刷新 | 每次打开卡片按需读取一次并使用短缓存；展开期间不轮询 |
| 文档 | 持续在本文件记录已确认决策、数据来源、待定项和实现状态 |

键盘聚焦（Tab / Shift+Tab 到达胶囊）可打开、鼠标可移入卡片操作、离开 / Esc 关闭，并保留现有胶囊单击行为；
程序性聚焦与 WebKit 的初始焦点不算用户意图，不打开卡片（见“弹出时卡片已展开”一节）；
用户在该提议上将悬浮延迟改为 100ms。复制与终端打开在卡片内，不占常驻顶栏。
“常驻标题行 + 固定预留高度”的旧设计及临时调参工具已被用户否决，不得在后续实现中恢复；
标题栏显示不属于该旧设计（不占流、不预留高度），不能被当作恢复它的理由。

## 标题栏显示（2026-10-17）

用户要求把 Agent Session 标题直接显示在作答弹窗里（此前只在 Sidebar 行和悬浮卡里）。先做了可交互原型
（`src/prototype/popup-session-title.html`，用真实 `PopupView` + mock 后端，见
[`../development.md`](../development.md)），比较标题栏、内容顶部、页脚“回复给”、塞进 Agent 胶囊四种位置后，
用户选定标题栏方案。

| 决策 | 结论 |
| --- | --- |
| 位置（macOS） | 红绿灯所在的标题栏：导航栏原有的 30px 顶部留白（`.macos .navbar` 的 `padding-top`），标题居中，左右各留 88px 避开红绿灯 |
| 位置（Windows / Linux） | 没有空标题栏，写进原生窗口标题：`<会话标题> — AskHuman` |
| 字重 | 轻：12px / 500 / 次要色（`--text-secondary`），带与 Sidebar 相同的会话图标，超长省略 |
| 布局 | 绝对定位、不占流、不增加高度；标题延迟到达只做 180ms 淡入，不位移任何元素；没有标题时窗口与此前完全一致 |
| 交互 | 纯展示：`pointer-events: none`，鼠标事件穿透到可拖动的导航栏；完整标题仍在 Agent 胶囊悬浮卡内 |
| 合并窗口 | 只显示当前选中请求的标题，居中于主面板；各请求仍由 Sidebar 行区分 |
| 数据 | 复用 `usePopupCore.agentSessionTitle`（随 `PopupInit` 与 `popup-session-title` 事件更新），不新增读取、不阻塞首屏 |
| 被否决的备选 | 内容顶部（标题延迟到达会把正文顶下去）、页脚“回复给”（可选，未采用）、塞进 Agent 胶囊（表头本来就满，标题一长就挤出省略号）、标题栏 + 页脚叠加（同一行字出现两次） |

Windows / Linux 的原生标题由 Rust 侧合成并写入，而不是让前端直接调用窗口 API：页面只通过
`popup_set_session_title` 上报会话标题，`app/popup_title.rs` 负责规整（单行、去控制字符与双向覆盖字符、
最长 120 字符）、拼上应用名并 `set_title`。这样不必对所有窗口开放 `core:window:allow-set-title`
权限，命令只接受 `popup` 窗口；`save_settings` 切换界面语言时通过同一模块重建标题，不会把会话标题
重置回 “AskHuman”。窗口重建后的下一次上报总会重新写入，不做“未变化则跳过”的去重。
共享窗口里只有当前选中的请求会上报；没有标题时上报空值，标题恢复为 “AskHuman”。
macOS 的原生标题保持隐藏，不上报。

## 弹出时卡片已展开的根因与修复（2026-10-22）

现象：窗口一弹出，Agent（或项目）胶囊的悬浮卡就已展开，用户没有悬停也没有按键，窗口激活后胶囊还带一圈焦点环。

**根因**（在 Dev Instance 里用带事件镜像的临时构建复现并定位，构建已移除）：WebKit 在页面刚载入进窗口时，会把焦点
交给页面里第一个可聚焦控件，也就是导航栏里的 Agent 胶囊。此刻窗口还隐藏、未激活，`document.hasFocus()` 为 false，
事件没有任何脚本调用栈，是引擎自己发起的。`SourcePopover` 把任何 `focusin` 都当作“键盘聚焦”打开卡片。
作答区输入框随后会被脚本聚焦（`focusComposerIfInitiallyVisible`），焦点离开胶囊，卡片按离开规则很快关闭，所以通常看不到；
但消息很长、作答区不在首屏时输入框不会被聚焦，焦点就一直停在胶囊上，卡片在窗口显示时已经开着。
同一个页面复用的后续请求没有新的初始焦点（实测第二个请求弹出时没有新的聚焦事件），所以它出现在页面刚载入后的第一个请求。

| 决策 | 结论 |
| --- | --- |
| 聚焦何时打开卡片 | 只在用户刚发出意图之后：Tab / Shift+Tab 到达胶囊（`keydown` 后 400ms 内），或在胶囊上按下指针。初始焦点、`restoreFocus` 等程序性聚焦不打开 |
| 初始焦点 | 页面没有焦点时落在胶囊上的焦点不是用户选的：立即丢弃，否则窗口变为 key 时胶囊带焦点环，终端 Agent 的按钮胶囊还会响应空格 / 回车 |
| 窗口隐藏 | `visibilitychange` 变 hidden 时关闭卡片，避免隐藏窗口里遗留展开的卡片成为下次弹出看到的第一样东西 |
| 不变 | 悬停 100ms 打开；ArrowDown 打开并进入卡片；Esc 关闭并把焦点还给胶囊（不重新打开）；窗口失焦关闭；胶囊单击行为 |

丢弃初始焦点后，页面与短消息 / 热页面一样处于没有聚焦元素的状态，没有把焦点转给输入框或别的控件；此后第一次 Tab 从导航栏开头走起
（先到 Agent 胶囊并按键盘规则打开卡片）。键盘滚动不受影响：实测焦点在胶囊或没有焦点时，空格 / PageDown 对长消息都不滚动正文。

验证（macOS，Dev Instance）：长消息弹出时与窗口激活后均无卡片、胶囊无焦点环；Tab 打开 Agent 卡、再按 Tab 切到项目卡、
Esc 关闭且焦点留在胶囊且不重开、Shift+Tab 回到 Agent 卡；悬停与按下胶囊仍打开卡片。`SourcePopover.test.ts` 新增 6 项，
其中 5 项在旧实现上失败。日常环境（2026-10-22，含修复的构建覆盖安装，重启 Daemon 后由预热的共享宿主第一次上屏）：
窗口缩小到作答区不在首屏，长消息弹出时卡片没有展开、Agent 名称没有焦点环（用户肉眼确认）。
Windows / Linux 的 WebView 初始焦点行为没有实机证据；这套判断只依赖标准的焦点 / 按键 / 可见性事件，
在没有初始焦点的平台上不改变任何行为。

## 修正前的目录问题与当前归属规则

- `project.rs::detect()` 从真实 CLI cwd 向上查 `.git`，无仓库则直接把 cwd 当项目。
- `cli/mod.rs` 在解析调用方会话之前计算项目，并用同一值读取 whats-next 待办及构造请求。
- Daemon `handle_submit` 自动激活路径把 `task.project` 当作 Agent cwd 写入 / 更新注册表；
  因此注册表现有 cwd 可能已经被 Temp 请求污染，不能无条件当作稳定工作区。
- `agents/report.rs::resolve_cwd` 当前接受 Hook cwd / 项目环境变量，不单独存储稳定工作区，
  也未处理 Cursor Hook 的 `workspace_roots`。
- Cursor Shell 子进程与 Hook 环境不同：已有本机证据见 `demo/agent-lifecycle/FINDINGS.md` §7.3。
  Shell 有 session ID，但不能假设一定继承 Hook 的 `CURSOR_PROJECT_DIR`。

当前由 `agents/session_info.rs` 按可信会话解析原生工作区，在 CLI 待办读取前归一化项目；
Daemon 再统一归一化新 Ask / Confirm 的项目，以同一值驱动 Sidebar、历史、待办和 IM。
实际调用目录由 `caller_cwd` 独立传递，不用注册表里可能被临时目录污染的 cwd 当权威来源。

- Codex：精确线程数据库 cwd，或 rollout 首条 session_meta.cwd；支持配置的 codexHome 与 CODEX_HOME。
- Claude：精确 transcript 起始 cwd；既有 Hook 的 CLAUDE_PROJECT_DIR 可报告明确工作区。
- Cursor：Hook 唯一 workspace_roots / CURSOR_PROJECT_DIR，或 composer ID 精确匹配 workspaceStorage；
  再回退既有 transcript 对应项目目录的唯一可恢复路径。不同目录匹配同一会话时不猜。
- Grok：精确 summary.info.cwd；既有 Hook 的 GROK_WORKSPACE_ROOT 可报告明确工作区。
- Pi：session header.cwd 与现有 Extension 的 ctx.cwd。

工作区以私有缓存按家族 + 会话 ID 保存，普通调用 cwd 不覆盖它；明确 Hook 工作区可更新它。
恢复会话时优先已有可信归属，Fork 使用新会话的原生工作区；无法取得唯一工作区则回退原 cwd 规则。
MCP 六个工具共用每调用绑定：Codex metadata、Claude / Cursor 隐藏一次性 token、Grok 参数指纹旁路；
无可信绑定时使用 MCP server 原项目，不能采用长期继承的旧 session 环境猜测。
旧历史不迁移；附件相对路径仍从真实调用目录解析；Dev Instance 仍按原 cwd 标记路由。

## Status line 调研（2026-10-15）

### 一手资料与定位

- [ccstatusline 使用文档（固定 commit）](https://github.com/sirmalloc/ccstatusline/blob/bed850187783904a9affcaba1dd80e967622e70b/docs/USAGE.md)：
  会话身份 / 模型 / 推理强度，Token / 上下文 / 压缩，Git / Worktree / 变更，以及账号额度等独立分类。
  调研固定在 commit `bed850187783904a9affcaba1dd80e967622e70b`，只读检出已移至临时缓存，
  避免被 AskHuman 的测试发现器误收集；未安装该项目或接管用户 Status line。
- [Claude 官方 Status line 数据协议](https://code.claude.com/docs/en/statusline)：
  分开提供 `workspace.project_dir` 与 `workspace.current_dir`，并提供模型、推理强度、窗口容量与
  已用 / 剩余比例。数据经状态栏命令的 stdin 传递，并不自动成为 AskHuman Hook 的入参。
- [Codex 原生状态栏源码](https://github.com/openai/codex/blob/main/codex-rs/tui/src/chatwidget/status_surfaces.rs)：
  区分模型 / 推理强度、上下文剩余 / 容量、项目根 / 当前目录 / 分支、Session ID、账号限额等。
- [Pi 原生 Footer](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/modes/interactive/components/footer.ts)
  与 [AgentSession](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/agent-session.ts)：
  提供模型、上下文、Token / 费用和 Git 等信息。本机安装为 0.84.2；上游 main 是比较资料，不能
  将其所有细节视为本机已支持。

### 字段归属（首版已实现；可选字段未纳入）

| Agent 卡片 | 项目卡片 |
| --- | --- |
| 会话完整标题、可复制 Session ID（已确认） | 会话所属项目完整路径（可复制 / 终端打开，已确认） |
| 当前模型 / Provider，推理强度 | 不同的真实调用目录（可复制 / 终端打开，已确认） |
| 剩余上下文比例，已用 / 总容量，更新时间 | Git 分支，Worktree 身份 |
| 未纳入首版：原生权限 / 沙箱模式、压缩次数 | 未提交变更摘要（已确认），计数粒度由实现细化 |
| 可选：累计输入 / 输出 Token、原生报告的费用估算 | 可选：远端仓库，PR / CI（需要额外网络读取） |

状态栏的账号额度、每周重置、平台服务健康、系统内存、Vim 模式等可用指标，不自然属于
本次会话身份；首版不建议全部搬入悬浮卡。这是建议，不是已确认的延期任务。

### 模型 / 上下文来源可行性

| Agent | 目前证据 | 接入限制 |
| --- | --- | --- |
| Claude Code | 官方 Status line JSON 有 model、effort、context_window_size、used / remaining percentage | AskHuman 未采集此流；本机无配置的 statusLine。用户已确认首版不新增该配置，只按 transcript / 现有 Hook 提供可靠子集，缺少容量则省略剩余比例 |
| Codex | 原生状态栏与 token_count 事件提供上下文 / 容量；turn_context / 原生会话状态可提供模型配置 | 现有 activity 解析忽略 token_count；按 turn_context / token_count 读取；只用 last_token_usage，不用累计 usage；模型切换 / 压缩会清除旧窗口数据 |
| Cursor IDE | 本机 state.vscdb 的精确 composer 记录含 modelConfig.modelName / maxMode、contextUsagePercent、contextTokensUsed、contextTokenLimit | 只读观察到非空与缺失两种记录；为内部 schema，尚未验证 CLI 同源。Auto 的选中模式与最近实际服务模型须分开，缺失不能显示 0 |
| Pi | 本机 getContextUsage() 返回 tokens / contextWindow / percent，Extension 可访问；模型切换有原生记录 | 数值包含估算；压缩后尚无新响应时 tokens / percent 可为 null。现有 Extension 已上报原生 usage；压缩后 null 会清除上下文，不用全会话累计 Token |
| Grok | 本机 summary.json 含 current_model_id；updates.jsonl 中有 usage.inputTokens / outputTokens / modelUsage 等累计值 | 当前窗口容量及剩余比例尚无足够证据，累计 usage 不能充当上下文占用 |

本机 Cursor 样本中 `contextUsagePercent` 按 0–100 表示，样本值与
`contextTokensUsed / contextTokenLimit * 100` 一致；这只是所观察 schema 的证据，不是全部版本的保证。

### 指标口径与刷新约束

1. 分开“当前上下文占用”“整场会话累计 Token”和“账号剩余额度”。不得互相替代。
2. 只使用本会话的原生报告值或明确标注的估算，并附最近报告时刻；通常反映最近一次模型响应，
   不应表述为对正在组装的新请求的实时精确计算。
3. 上下文总容量未知时不计算剩余比例；缺失 / 压缩后失效不显示为 0%，也不自动套用 200k。
4. ccstatusline 的 [model-context.ts](https://github.com/sirmalloc/ccstatusline/blob/bed850187783904a9affcaba1dd80e967622e70b/src/utils/model-context.ts)
   有模型名推断 / 固定容量回退，以及 80% usable 比例。这些是该项目的策略，不等于各家 Agent
   的原生自动压缩阈值，AskHuman 不应照搬为“真实剩余上下文”。
5. 模型切换、上下文压缩、resume 与 Fork 需防止把旧模型 / 旧上下文的占用量套到新窗口容量上。
6. 项目卡片的 Git 查询针对稳定项目 A；调用目录 B 不改变其分支 / 变更的归属。
7. 首屏不阻塞；模型 / 上下文 / Git 信息在悬浮时按需读取，有界缓存，不因填充卡片改变原窗口布局。

## 原生名称来源

| Agent | 来源与优先级 |
| --- | --- |
| Codex | 配置数据目录内精确线程 ID 的 SQLite `name`，旧 schema 回退 `title` |
| Cursor | IDE 数据库名称，其次 CLI chat metadata 的 `title` |
| Claude Code | transcript 最近的手动 `custom-title.customTitle`，其次 `ai-title.aiTitle`、旧 `summary.summary` |
| Grok | `summary.json.generated_title`（包括手动 `/rename`），其次旧 `session_summary` |
| Pi | transcript 最新 `session_info.name`；明确清空名称时丢弃旧名称缓存，回退首条用户消息 |

Claude 与 Pi 的后置名称记录按有界文件尾读取，避免旧的前 4000 行限制挡住长会话末尾的改名。
名称文件不可读、缺失或超出读取边界仍为 best-effort；不得阻塞提问首屏或作答。
本机 Claude 2.1.226 的安装产物包含两种标题记录字段；Pi 安装源码明确以最新
`session_info` 为准，空名称表示清空；Grok 1.0.50 内置格式说明区分 `generated_title` 与
`session_summary`，并记录手动名称标记 `title_is_manual`。

## 已实现的元数据链路

- Daemon：两种窗口共用 `runtime/inbox.rs::show_entry` 与异步标题读取；家族延迟解析完成时补读已有可信会话。
- IPC：沿用 `ShowPayload.agent_session_title` 和按 request ID 的 `PopupSessionTitle`。
- Helper：合并宿主和独立冷热 Helper 都保存标题快照，再发 `popup-session-title` 事件。
- 初始化：`PopupInit` 包含 `agentSessionId` / `agentSessionTitle`；前端先订阅再读快照，防止早到事件被旧快照覆盖。
- Sidebar：会话图标由 `SessionIcon.vue` 提供。
- 标题栏：`PopupNavbar.vue` 在导航栏首个子元素渲染 `.session-strip`（只在 `.macos` 下由 CSS 显示）；
  非 macOS 由 `usePopupWindowTitle.ts` 上报给 `popup_set_session_title`，`app/popup_title.rs` 写入原生标题。

`SourcePopover.vue` 负责两个胶囊的 100ms 延迟、焦点、移入操作、复制、Esc、跨请求隔离和浮层定位。
聚焦只在用户意图（Tab / 按下）之后打开卡片，窗口隐藏时关闭卡片。
只在打开时读取，前后端各有 2 秒短缓存；读取失效时不继续显示旧指标。布局变化只重定位卡片，不刷新指标。
`popup_agent_info` / `popup_project_info` / `popup_open_terminal` 校验本 Popup 的请求身份；终端只接受
本请求的 project / caller 目标，不能传任意路径或 Shell 命令。

`project_info.rs` 对稳定项目只读执行有界 Git 查询：分支 / detached commit、linked Worktree 与未提交项。
计数按 porcelain 路径条目，重命名计一次；同一项可同时计入暂存与未暂存，因此两者之和不一定等于总数。
单命令超时或超过输出预算时省略相应事实；不是 Git 仓库时不伪造干净状态。

不新增模型可见的会话参数或 Agent 提示词；MCP Todo 工具只扩展既有隐藏 Hook token 的使用面。
历史格式与 IM 模板不变，新请求的项目取值统一修正。

## 已实现的可靠指标子集

| Agent | 展示能力 |
| --- | --- |
| Codex | 原生名称、模型 / 推理强度、最近记录的上下文 Token / 容量与原生剩余比例 |
| Cursor IDE | 原生名称、选中模型（包括 Auto 原值）、原生存在的 Token / 容量 / 百分比；CLI 不假定有 IDE 指标 |
| Claude | 原生名称与最新真实 assistant.model；不读取新增 Status line，不猜容量或剩余比例 |
| Grok | 原生名称与 current_model_id；累计费用 / Token 不当作当前上下文 |
| Pi | 原生名称、模型 / 推理强度、现有 Extension 上报的上下文估算；缺失时退回 transcript 模型信息 |

Codex 剩余比例沿用原生排除 12k 基线的计算，与“已用 / 总容量”的简单除法可能不同；上下文标注估算。
Pi 数据也标注估算；原生文件比运行时报告更新时不复用较旧的报告。更新时间表示最近记录时间，不表示实时采样。

## 验证

本机自动测试、安装和原生交互证据统一记录在实施计划。macOS 已覆盖合并 / 独立 Popup、
完整标题、模型 / 上下文、缺失省略、键盘、复制、Esc、请求切换、草稿、项目 A / 调用 B、
Terminal.app 两个目录、新历史与跨目录待办。Windows / Linux 终端动作已实现，但本轮没有实机证据；
各家原生元数据为 best-effort，不将某家缺失的指标解释成 0，也不声称所有厂商版本均有同一 schema。

标题栏显示（2026-10-17）在 macOS 实机上覆盖独立窗口与带 Sidebar 的合并窗口：标题与红绿灯在同一水平线上，
合并窗口里居中于主面板；真实 WebKit 里验证了在标题文字 / 图标上按下鼠标命中的是带
`data-tauri-drag-region` 的导航栏（窗口仍可从标题处拖动）、有无标题时导航栏 / 表头 / 内容区位置逐像素一致。
Windows / Linux 的原生窗口标题已实现并有 Rust 单元测试，但本轮没有实机证据。
