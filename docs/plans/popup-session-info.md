# Agent / 项目悬浮信息实施

需求与决定：[`../specs/popup-session-info.md`](../specs/popup-session-info.md)。
2026-10-15 用户授权全部实现并明确休息期间不能回答问题；常规实现选择由 Agent 自行完成。
继续使用主工作树；保留其他任务的修改，不新建 worktree 或启动子代理。

## 实施顺序

1. 建立按可信原生会话读取的工作区 / 模型 / 推理强度 / 上下文元数据层与私有缓存。
   Cursor Hook 工作区与 Pi Extension 报告复用既有集成；不新增 Status line 配置。
2. 分离真实调用 cwd 与稳定项目根，统一 CLI / MCP ask / Hook 的新请求项目归属及待办入口。
   原生来源不可靠时回退旧规则；不使用可能已污染的 registry cwd 猜测工作区。
3. 将调用 cwd 随请求送至两种 Popup；增加按请求身份读取元数据、读取 Git 摘要与在终端打开目录的命令。
4. Agent / 项目胶囊增加 100ms 悬浮卡，支持复制、Terminal.app 打开、键盘、Esc 与跨请求隔离。
   悬浮按需读取并短缓存，不轮询，不改变正文布局；保留原胶囊单击语义。
5. 运行有意义的解析 / 归属 / UI 回归、类型检查、Clippy 与安装；使用安装产物验证原生界面。
   用户休息期间无需作答的验证使用合成请求 / 本地隔离运行，避免向生产 IM 投放测试卡。
6. 同步 overview / spec 与实际能力边界，记录结果，从 PROGRESS 删除已完成实施条目。

## 实现守卫

- 不改旧历史、不自动改变 Dev Instance 的 cwd 路由、不接管用户 Status line。
- 正文与附件沿用现有路径解析器；显式文件仍在调用端转为绝对路径。
- 多根工作区不能确定唯一来源时不猜；模型 / 上下文不能取到可靠值时省略。
- 项目卡 Git 查询对应稳定项目，终端动作只接受本请求已知路径。
- 已安装的五家标题读取与异步补齐链路保留，不恢复已否决的常驻标题行。

## 实施结果（2026-10-15）

- `agents/session_info.rs` 建立精确会话工作区 / 原生模型 / 上下文读取及 0600 私有缓存。
  Codex、Claude、Cursor、Grok、Pi 各使用可验证原生来源；Pi 复用单个既有 Extension，
  空上下文清除旧值，文件比报告新时不复用旧快照；没有新增 Claude Status line。
- `project.rs` 在 CLI 读取 whats-next 待办前统一项目身份；Daemon 对 Ask / Confirm 与新 Agent
  事件统一归属；MCP 六个工具复用每调用绑定，Todo 不再仅依赖 server cwd。
  旧历史 / 待办不迁移；实际调用 cwd 独立传递；正文和附件的现有解析器、Dev Instance 路由不变。
- 两种 Popup 共享按请求身份校验的原生信息 / Git / 终端命令。`SourcePopover.vue` 仅浮层显示，
  100ms 悬停、焦点立即打开、ArrowDown 进入操作区、Esc / 离开关闭；移动到卡内可复制或打开终端。
  2 秒缓存、无指标轮询、跨请求丢弃过期结果；布局变化由 ResizeObserver 重定位。
- 标题快照和补齐事件统一为 `popup-session-title`，独立冷热 Helper 与合并宿主均可补齐。
  不恢复常驻标题行 / 调参工具；Sidebar 原会话图标改为共用组件。

## 自动验证

| 检查 | 结果 |
| --- | --- |
| Rust 全量 tests | 1332 passed，4 ignored |
| Vitest 全量 | 389 passed（56 files） |
| Node 全量 | 17 passed，包括新增 Pi Extension 原生上报 / null 清除 / Bash timeout 兼容测试 |
| TypeScript / production build | 通过，包含 vue-tsc 与两份 Vite bundle |
| 全 targets Clippy | `-D warnings` 通过 |
| 格式 / diff | cargo fmt 与 git diff --check 通过 |
| 安装 | 多次 `./scripts/install.sh` 成功，最终产物签名安装至 `~/.local/bin/AskHuman` |

Rust 新回归覆盖 latest-vs-cumulative context、模型切换 / 压缩失效、有界尾读、非法 ID、
Cursor 精确且唯一的 workspaceStorage 映射，以及真实 Git branch / linked Worktree / detached
HEAD / 路径计数 / 不改 index。前端覆盖 100ms 边界、完整身份复制、Esc 不取消提问、终端目标、
单一卡片、无轮询、请求隐藏后的迟到结果和缺失省略。

调研 clone 最初位于仓库忽略目录，Vitest 默认发现器误收集其测试，导致一次测试失败；
已移出仓库至临时缓存并重跑本项目全量测试通过，没有把第三方测试计入上述结果。

## macOS 原生验收

使用本次安装的二进制，合成精确 Codex session / SQLite / rollout 与 A、B 两个真实 Git 仓库。
本地 IPC 仅使用隔离配置，所有 IM 关闭；没有发送真实模型请求或生产 IM 卡片。
证据保存在忽略目录 `.askhuman-dev/session-info-qa/`（脚本、消息记录、历史、截图）。

- 合并模式：实际 WKWebView 展示完整原生名称、模型、推理强度、106k / 200k 及原生 50% 剩余；
  第二个会话缺失模型 / 上下文时只展示标题和 ID，切换不串卡；两请求均按 A 分组。
- 拷贝 Session ID 后在回复框粘贴，得到完整 UUID；拷贝调用路径后粘贴，得到完整 B 路径。
  鼠标移入卡内能操作；Esc 后原提问 / 草稿仍存在，切回首请求草稿保留，提交正常。
- 独立模式：采用仅模拟本地 daemon 协议的服务器启动安装产物，覆盖同一张 Agent / 项目卡、
  Tab 切卡、ArrowDown 聚焦复制按钮、Esc 和发送；仅 CODEX_HOME 环境也能读取原生模型和上下文。
- 两个 Terminal.app 按钮分别生成 cwd 为 A 与 B 的新 shell；通过进程 cwd 验证，未操作用户已有终端。
  Computer Use 不允许控制 Terminal.app，因此没有将其窗口 AX 读取声称为验收证据；已清理测试 shell。
- 首个请求的新历史 project 为 A；从 B 通过已安装 CLI 创建 / 列出 / 自动出队的待办仍属于 A，
  相对附件读取 B 的 tracked.txt（5 bytes），没有被改为 A 的文件内容。
- 浮层打开前后正文和窗口布局保持原状，没有常驻标题行或额外高度。

隔离 daemon 的自动集成迁移曾把四家受管 Hook / Extension 的 executable 切为测试副本；
发现后立即用已安装全局产物执行 agents update 恢复，并核对没有残留 QA 路径。
后续独立模式验收改用纯 IPC 模拟服务器，避免重复启动迁移；没有改变用户所选集成模式。

## 交付边界

本轮没有 Windows / Linux 实机证据，不把 macOS 或纯解析测试代替它们；终端失败会在卡内提示。
厂商未持久化或当前文件边界中缺失的模型 / 推理强度 / 上下文一律省略。
受管 Pi Extension 已更新且保留现有 cli / lifecycle / stop 选择；已运行 Pi 会话需要按其原生方式
重新加载 Extension 或新开会话后才会执行新采集逻辑，不中断用户既有 Agent 会话。

## 交付后修正：来源胶囊高度（2026-10-15）

用户要求提交，并指出 Message from 后 Agent / 项目胶囊高度不一致。
Agent 不可定位终端时为 span，而项目始终为 button，两者默认行高不同；
在共用 `.brand-chip` 中固定 height 20px / line-height 16px，保持相同的垂直 padding。
再次运行 `./scripts/install.sh` 成功；使用安装产物的原生 WKWebView 验证两者顶底对齐，
截图为 `.askhuman-dev/session-info-qa/chip-heights.png`。低影响 CSS 修正未新增镜像样式的单元测试。

## 标题栏显示实施（2026-10-17）

需求与决定见 [`../specs/popup-session-info.md`](../specs/popup-session-info.md#标题栏显示2026-10-17)。
用户在原型上选定标题栏方案、轻字重、Windows / Linux 写进原生窗口标题，并要求直接在产品里实现。

### 实施结果

- `PopupNavbar.vue`：导航栏首个子元素 `.session-strip`（macOS 由 CSS 显示：绝对定位、30px、
  `pointer-events: none`、12px / 500 / 次要色、180ms 淡入）。样式放在该组件自己的 `<style>`，没有改 `popup.css`。
- `usePopupWindowTitle.ts` 与 `ipc.ts::popupSetSessionTitle`：非 macOS 且为当前选中请求时上报会话标题。
- `app/popup_title.rs`：`popup_set_session_title` 命令（只接受 `popup` 窗口）、`normalize` / `compose` / `refresh`；
  `save_settings` 改为调用 `refresh`，切换语言不再把标题重置为 “AskHuman”。没有对所有窗口开放
  `core:window:allow-set-title`。
- 原型同步：A 现在就是产品组件本身（其余格子是被否决的备选，对照时隐藏产品的标题栏），去掉 A 的字重开关。

### 自动验证

| 检查 | 结果 |
| --- | --- |
| Vitest 全量 | 690 passed（72 files），其中新增 12 个：上报逻辑 7 个（macOS / 非 macOS、迟到、非选中请求、卸载后、失败）、导航栏 5 个 |
| Node 全量 | 32 passed |
| TypeScript | `vue-tsc --noEmit` 通过（含原型） |
| Rust | `popup_title` 7 个单测通过；`cargo fmt --check` 通过；clippy `-D warnings` 通过 |
| 安装 | `./scripts/install.sh` 成功（local-install） |

先写的单测抓到一个真问题：先去控制字符再按空白拆词，会把 `\t` / `\n` 两侧的词粘在一起，已改为先拆词再过滤。
本机较新的 clippy 对 `build.rs` 里既有的 `uninlined_format_args` 报错，与本改动无关；屏蔽该条后全 targets 通过。

### macOS 原生验收

共享 daemon 另有 2 个在途请求，换新会让提问排队，所以用安装产物在一次性实例里验证
（`ASKHUMAN_HOME=/tmp/askhuman-title-verify/home`，独立 launchd 标签，用 `ASKHUMAN_FROM_MCP` +
`ASKHUMAN_MCP_AGENT_KIND/SESSION_ID` 绑定到本次 Cursor 会话以解析真实标题）。脚本与截图在忽略目录
`.askhuman-dev/session-title/`（`native-verify.sh`、`shots/native-*.png`）。

- 独立窗口：标题和红绿灯在同一水平带上、窗口正中；原生窗口标题保持 “AskHuman”（macOS 不上报）。
- 合并窗口（两个请求，出现 Sidebar）：标题居中于右侧主面板，Sidebar 行里的标题照旧。
- 拖动：App 级合成拖拽触发不了原生窗口拖动（对已知可拖动区域的对照实验同样没有移动窗口），不能作为证据；
  改在真实 WebKit 里复现 Tauri 的判定：在标题文字 / 图标上按下鼠标，命中的是带 `data-tauri-drag-region` 的导航栏，
  而对照组（点在胶囊上）不命中。
- 无布局位移：有无标题时导航栏、表头、动作区、内容区的位置逐像素一致；无标题时不渲染该元素。
- Windows / Linux 的原生窗口标题没有实机证据，只有 Rust 单元测试覆盖标题的规整与拼接。

### 隔离实例的副作用与恢复

一次性实例用的是安装产物，它启动时会执行 `agent_stop::migrate_outdated()`。新建的 home 里没有
`stop-preferences.json`，Cursor / Codex 的结束确认偏好按默认值视为关闭，于是用户全局的 Cursor、Codex Stop hook
被改写成指向安装产物、且不再带 `confirm`（偏好文件本身未变）。验收后用 `AskHuman agents stop cursor` /
`codex` 发现 “on（需更新）”，再用全局 npm 产物执行 `agents stop cursor on` / `codex on` 恢复：
Stop hook 重新指向全局产物并带回 `confirm`，三家状态均为 on，Agent 配置里没有残留安装产物或临时目录路径；
Claude 配置未被触碰；LaunchAgents 登录项未变。恢复时偏好文件被重写，多了一个语义等同缺省的 `"pi": null`。
以后用隔离实例验收，结束后都应当做同样的检查与恢复。
