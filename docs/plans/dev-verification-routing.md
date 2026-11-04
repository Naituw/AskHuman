# AskHuman 项目级开发验收实施计划

> 需求：[项目级开发验收](../specs/dev-verification-routing.md)。
> 状态：[PR #27](https://github.com/Naituw/AskHuman/pull/27) 已于 2026-11-01 合并。项目级重做已实现；2026-10-31 复核修正见 §6，真实 Luna 与 iOS 转送证据见 §7；
> 2026-11-03 补做了本机界面与正式渠道转送验收并修复了验收发现的问题，见 §8。Dev 专用测试渠道的真实收发与登录后的完整隔离会话仍待验收（见 `PROGRESS.md`）。
> §1–§7 是合并前的记录（当时在主工作树继续、不另建 worktree、保留未提交修改），不代表现在的工作方式。

## 1 合并前的基线

[PR #27](https://github.com/Naituw/AskHuman/pull/27) 当时保持草稿（2026-11-01 合并）。原实现及旧 CI 留在 git 历史，
不能按旧的 7 项 CI 全绿宣布新版完成。本机没有执行全局集成迁移。

新方案只准备本仓库的项目规则、开发 MCP 与工作树设施；不再执行前次 12 个全局文件的迁移计划。
方案评审已完成，按下列步骤修改实现及当前项目配置。

## 2 重做顺序

| 步骤 | 工作 | 验证 |
| --- | --- | --- |
| P1 | 撤销普通 prompts、MCP 工具目录、集成前缀、设置页和全局迁移／注册改动；保留转送所需的内部协议 | 普通工具目录、规则、设置与任务前基线比较；日常路径回归 |
| P2 | 将项目准备并入幂等 dev enable；建立本树 manifest，分开旧自动标记和新设施 | 每树首次准备、重复调用不写入、缺项修复、旧树升级、连接门槛及两树隔离 |
| P3 | 增加项目 `askhuman-dev` MCP 和客户端项目规则适配；不覆盖普通 `askhuman` | 原生项目配置加载与冲突保护、普通项目无开发工具、工具生命周期固定本树 |
| P4 | 连接构建校验、显式 CLI／MCP 验收、恢复、附件、取消和 daemon 身份校验 | 实际构建跨进程回归；普通／Dev／普通与错误构建拒绝 |
| P5 | 逐请求正式转送覆盖问答、权限与 Stop；默认无需 Dev 专用 channel | 远程不可用不静默降级、本机活跃槽与通知延迟、身份与动作归属、取消及迟到结果 |
| P6 | 调整独立测试 profile；主开发项目 MCP 与测试会话配置分开 | Hook／权限／Stop／恢复／Fork 归属，未绑定原生请求拒绝，无全局写入 |
| P7 | 开发工作台只从本树 Dev 入口打开；调整安装与发布构建 | 普通发布无本方案新增的开发 UI／MCP 工具／规则；工作树安装不覆盖正式版 |
| P8 | 安装、CI、原生项目配置与本机 Dev 验收；同步文档并重写 PR 描述 | 按新方案报告各平台证据及仍待验收项目 |

不重写已经公开的分支历史，按最终实现追加提交。实现已更新主 overview、专题实现地图、
Dev Instance 规格、MCP 规格、AGENTS 和工作树操作文档；普通产品入口与项目开发入口已按新边界分别说明。

## 3 工程边界

项目准备不增加独立 setup：任务使用 Dev Instance 时执行幂等 dev enable，准备与验收就绪分别报告；
不要求每个功能任务启用 Dev，不自动改变共享主工作树的安装去向。
规则引导必须在首次 enable 前可读，客户端未重载与配置已生成分开记录。
仅维护当前工作树中自己的配置；生成文件保留模板／实际配置的区分，机器路径和凭据不入库。
不调用用户级 Agent 集成安装器来准备开发主会话，不直接修改全局 trust 或权限。
只有完整测试 profile 使用私有集成生成器，其写入范围必须验证为该 profile。

普通 MCP 和项目开发 MCP 使用不同 server 名称。常规请求沿用正式连接，开发请求只委托已绑定本树的实际构建。
不能在多线程 server 内临时切换全局 home，也不能用开发 server 重新实现普通待办或会话控制。

旧 `.askhuman-dev/enabled` 的转换局限于当前工作树，先检查旧连接；新 manifest 不触发旧 cwd 自动路由。
正式入口保存在工作树私有配置，不新建账户级路由模式。

IPC 若需调整，先遵守 overview 的协议号规则；仅增加默认值字段保持兼容不等于旧程序支持新开发能力。
转送需按问答／权限／Stop 分别做能力检查；错误正式版不能导致升级互顶或隐式降级。
完整远程连续性覆盖问答、权限、Stop；仅 Dev 请求即时提醒，不改变正式日常通知策略。
正式连接唯一持有者始终是正式 daemon。真实渠道测试使用专用身份，失败不回退代理当作验收成功；
修改转送本身时使用隔离测试端，不替换正在沟通的正式 daemon。

## 4 验证与交付

复用已有的有效构建、留存、授权与原生隔离测试，删除围绕被撤销全局迁移设计的测试。
新增的测试围绕项目边界、普通用户不受影响和新旧标记转换，不镜像实现细节。

功能改动完成后按仓库要求运行安装脚本，再通过新安装程序验收。CI 同时覆盖普通产品边界与 Dev 实际程序；
不为通过测试而修改用户的真实配置。原生 Agent 启动／模型调用仍须获得用户授权。

此前用户延期的本机界面待办保留，但应验收新的 Dev 工作台和 Dev 卡片，不再验收被撤销的正式设置页。
Mac 图形、真实渠道和各客户端登录后的完整会话分别记录证据；本次方案设计没有执行这些测试。

## 5 复核前的验证记录（2026-10-31）

- 已执行 `./scripts/install.sh`，安装位置为本树 `.askhuman-dev/bin/AskHuman`，签名完成；正式程序未替换。
- 新安装程序校验实际构建；面对当前不兼容的正式端明确拒绝远程验收，未换新、未产生本机等待。
- 新安装程序的两树进程回归通过：普通六工具／项目单工具、普通／Dev／普通、resume、stale build、私有 profile、旧 marker 升级及全局文件保持。
- 独立私有 daemon 与回环 Telegram fixture 通过实际转送：问答、权限记忆、Stop 待办、取消、发送失败；正式待办、权限记忆和日常活跃槽未被测试动作修改。此为模拟渠道证据。
- 前端 91 文件／1078 测试通过；Node 总计 88 测试，69 通过、19 平台条件跳过；普通前端构建不产出 DevelopmentView 工作台 chunk，Dev 安装包含该 chunk。
- Codex 0.147 的 `mcp get askhuman-dev --json` 已正确解析本树配置与超时；不将配置解析等同于当前 App 已加载连接。
- 16 处本机全局集成／配置内容与权限快照保持不变。项目生成配置只加入仓库本地精确 exclude，未修改用户级信任。

当时本机完整 Rust：2295 passed／11 ignored；严格 Clippy 通过。CI 状态以 [PR 当前提交的检查](https://github.com/Naituw/AskHuman/pull/27/checks) 为准，不引用旧 head 的结果代替。
用户已选择先完成 CI／草稿 PR；真实渠道验收登记 AskHuman 待办 #5，本机界面沿用 #4。
未发起真实模型调用，不能把私有 profile 的配置准备或手工协议夹具算作登录后 Agent 全流程通过。

## 6 复核修正（2026-10-31）

- 撤销“每次功能任务 enable”的扩展。AGENTS 最终差异经用户逐项 review：保留原有任务／工作树条件和 preset 询问，只更新已启用 Dev 的显式验收调用；client 继续自动识别，不强制传参。
- 解除共享主目录的 Dev manifest 和拥有的项目 MCP 连接，保留 bin／home 与其他会话的代码。用户明确要求继续在主工作树开发，不创建新 worktree。
- enable 的准备结果与验收就绪分开；status 显示实际 delivery、构建和正式转送前提。复核开始时的正式端能力为 1，不能提供新转送能力，刷新 MCP 或反复编译 Dev 均不能解决；本轮不为验证转送替换日常正式端。
- 原生 profile 参数移到 prompt 分隔符前；前台 CLI 继承终端输入输出；源码指纹补入实际嵌入的 Info.plist，并覆盖回归。
- 复核前的“两树普通／Dev／普通”测试中，Dev 部分是恢复预先写入的结果，只证明恢复和隔离。新增项目 MCP 实际新发问与真实 worker 完成留存的回归；转送渠道仍为回环 fixture，不能称为真实渠道验收。
- 本轮本机 Rust 2296 passed／11 ignored，严格 Clippy 通过；Node 90 项，71 通过／19 平台跳过。
- 已在主工作树运行安装脚本，显式安装到临时目录并签名；随后使用该程序跑两树进程与转送回归，均通过，包含实际新发问、终态留存、旧正式端拒绝和未替换检查。新发问首次与另一套进程回归并行时超过夹具原定的 15 秒；夹具等待改为 30 秒，单独重跑通过，没有放宽功能断言。
- 16 处全局配置内容／权限保持快照一致。期间另一个桌面消息修复会话按其任务正常执行主目录安装，将正式程序更新为能力 2；本聊天的安装目标始终是临时目录，也未使用日常正式端作候选转送验收。
- 新 head CI 以 PR checks 为准，尚未获得的结果不按旧 head 的通过状态替代。本节复核结束时尚未补做真实渠道、原生图形及登录后 Agent 会话；后续证据见 §7。

## 7 真实 Codex 与 iOS 转送验证（2026-10-31）

用户明确授权启动真实 Codex，要求使用便宜模型；本轮两个原生交互客户端均使用 Codex CLI 0.163.0、
`gpt-6-luna`、低推理，启动参数固定 `service_tier=default`。不是 `codex exec`、模拟模型或预填答案。
用户说明只能接收 iOS 通道提问；以下成功均有真人回答，未把其它 IM 的发送当成有效收题。

- **A：非 Dev 客户端**。临时普通项目没有 Dev manifest 或项目开发 MCP；原生会话
  `01a1917e-36ed-7390-80a3-3bd6dda4530c` 通过正式 `askhuman.ask` 收到「普通入口已收到」，
  随后的 `whats_next` 获得结束批准。使用日常 Codex 登录，没有复制认证。
- **B：主开发会话中的普通 → Dev → 普通**。临时测试仓库取自干净提交 `4e36b41`，没有创建 Git worktree，
  没有启用共享主目录的 Dev。实际运行安装脚本、独立 Swift 缓存及私有目标目录，安装并签名该仓库 Dev 程序。
  原生会话 `01a1918f-216a-7811-b3c5-0aebf87a30c8` 实际加载本项目 `askhuman-dev`，同一会话依次得到
  「普通 B1 已收到」「Dev B2 已收到」「普通 B3 已收到」。普通调用的进程与正式 daemon 日志也已核对。
- **B2 构建与结果**。项目 `dev_ask` 的真实新发问由 Dev 执行，通过现有兼容正式端的远程出口转送；
  验收 ID `bf0eb280-cd3d-4454-aaef-297aec3ab0b1` 的私有记录为 `completed`、退出码 0，
  `delivery=remote`、来源 `relay=false`，没有独立测试 profile 身份。提交、源码指纹、二进制指纹与实际安装记录逐项一致。
  源码指纹为 `5d9495b2dd490cc73f2620eb4d671320b0d9db917b6739a70dda8fe462e9de15`；
  二进制指纹为 `0e85d7d5675f48d8e5eeed658280ff7c56a4d971feea457af0d49fd8bfa9ca5a`。
- **B 的收尾限制**。模型在最后一张结束卡误用了 `ask`，该卡被取消；三次问答和 Dev 终态确已完成，
  但不把 B 算作完整交互协议验收通过。测试客户端随后退出，没有继续执行模型任务。
- **独立原生启动器，仅到登录前**。私有 `CODEX_HOME` 没有认证。用户明确表示现在不想且不方便登录后，
  已停止设备登录等待，没有复制日常认证。实际执行 `node scripts/dev.mjs session codex --stdio`，
  到达原生 Codex 登录菜单；进程参数确认 `--add-dir` 位于 `--` 前，方向键经 Node／worker 透传后改变菜单选中项，
  随后退出，未选择登录。该项只证明实际启动参数及终端输入输出，不证明登录后模型、权限或 Stop 流程。
- **收尾与全局边界**。本轮临时 Dev daemon 在 0 个活动请求时停止，临时仓库的 Dev 已解除启用，
  保留构建与验收记录。共享主目录保持未启用，AGENTS 与用户批准的版本逐字一致。
  相对较早的全局快照，15 处文件内容／权限不变。后续专门排查已恢复用户 `~/.codex/config.toml` 的旧原文：
  从现文件移除两处新增 TUI 状态后，SHA-256 与旧快照 `a05530a38900a37efb03d804a28a1008b32e7106ca9a07b88313d4fafad96358` 完全一致。
  文本仅新增 `tui.screen_reader_detection_done=true` 和 `tui.model_availability_nux."gpt-6.1-sol"=2`；权限从 `0644` 变为 `0600`。
  独立空白、未登录、无 MCP／Hook 的 Codex 0.163.0 仅启动到登录菜单，便复现前一标记与权限收紧，未提交 prompt 或调用模型。
  原生二进制同时具有 `startup_prompts::select_model_availability_nux` 与 `ConfigEditsBuilder::set_model_availability_nux_count` 符号；
  该计数属于原生 TUI 的新模型提示状态，不是日常模型选择。其余配置全文逐字一致，包括模型、MCP、Hook、项目与权限设置。
  没有覆盖恢复用户配置；调查前后其内容／权限均未再变。当前日常默认模型仍为 `gpt-6-astra`，普通 MCP 仍指向正式程序，
  没有用户级 `askhuman-dev` 或本次临时项目信任项；独立 profile 测试前后该用户配置字节相同。

本轮证明项目开发工具与正式 iOS 出口的真实转送；没有替换正式 daemon，也不替代专用 bot／配对的渠道实现验收。
GUI 待办 #4、独立测试渠道待办 #5 继续保留；完整隔离 profile 的登录后会话另行验收，不要求用户现在登录。

## 8 合并后的本机验收与修复（2026-11-03）

PR #27 合并后补做 §7 留下的本机界面与正式渠道转送验收（这也是对 [Issue #23](https://github.com/Naituw/AskHuman/issues/23)
所提路由模式与 Agent 策略的实际回答：日常 AskHuman 永远是正式版，验收显式进入 Dev）。验收在临时 worktree
`HumanInLoop-devacc` 里进行，自 `95551a23` 起，安装到该树的 `.askhuman-dev/bin`，用毕移除；
主工作树当时有另一会话的未提交改动，没有触碰。正式程序与正式 daemon 全程未替换。
界面由本会话用 macOS 窗口自动化逐张截图核对，不是 CI 的替代也不是 CI 的结果。

**修复前的构建**

- Dev 卡片身份：标题、来源、工作树标签、横幅，展开详情与 `build.json`／验收 ID 一致。
- 本机四条终态路径共五张卡：作答；卡片「取消」；`kill -9` CLI；`kill -9` 驱动进程（记录转 `interrupted`，迟到回答不被接受）。
  `--resume` 返回已存答案且不发新请求。
- 工作台：三张卡片、实例详情、投递选项、480 pt 最小宽度布局无裁切；正式设置没有开发页；
  Dev 历史与正式历史互不串（Dev 5 张，正式 0 张）。
- 全局边界：18 个全局集成文件（哈希与权限）与正式 `config.json` 前后一致。
- 正式渠道转送（默认 `remote`）：R1 的卡片到达正式 daemon 与飞书，取消得到 `card_finalize_succeeded`；
  R2 由用户在手机回复「ok」，Dev 记录 `cafb30ad-d32b-4021-8f4c-252b8d3dabc2` 为 `completed`、退出码 0、`[user_input]\nok`，
  正式请求 `17bbc9d6-23d7-44bf-9bd4-b20b0786220b` 约 11 分钟后结束。

**发现并修复**

| # | 现象 | 修复 |
| --- | --- | --- |
| 1 | Dev 设置窗口仍显示 Agents 页。页上的操作会被后端守卫拒绝，页面却照样摆出来，与规格 §8「不提供修改日常集成的按钮」不符。原来的只读标记只对工作台打开的设置窗口成立，Dev 卡片齿轮打开的没有 | 页面与守卫共用 `integrations/mutation_lock.rs::everyday_changes_blocked`；Dev 窗口隐藏 Agents 页，标签、搜索与深链接都落回「通用」。真值表与标签守卫有单测 |
| 2 | 取消的转送请求要等满 5 秒才结束，Dev daemon 记 `cancellation finalize timed out`：`RelayChannel` 被打断后从不通知收尾 | 被打断的转送在子进程结束后 `notify_finalized`（与 iOS 渠道同一做法）；单测覆盖「被计入、通知后立即放行」 |
| 3 | `dev ask --resume` 对被 `kill -9` 的验收显示 `pending`，实际没人在等 | 执行者已不存在的 `pending` 记录显示为 `interrupted`，重复 ID 的提示同；单测覆盖 |
| 4 | 登录后的 Grok 私有会话（用户在私有 profile 里自己登录后）起不来：启动器给 Grok 传 `--add-dir <工作树>`，Grok 1.0.70 报 `Session creation failed: Invalid params: "additionalDirectories is not supported: a session works only in its cwd"`，会话建不出来，只在重试循环里卡着。原单测只断言参数里有 `--add-dir`，没有真启动过 | Grok 不再传 `--add-dir`，仍用中性目录做 cwd，任务文本写明源码树路径；Codex 不变。单测改为断言 Grok 没有该参数 |
| 5 | `dev session`（非 `--stdio`）成功弹出终端后仍报 `invalid development launch receipt`：`osascript` 把新标签引用（`tab 1 of window id N`）打印到 stdout，混进 JSON 回执 | `open_terminal` 丢弃 `osascript` 的 stdout |

**修复后的构建（同一临时 worktree 重新安装）**

- 设置窗口：从工作台「Configure Dev channels」与 Dev 卡片右上齿轮两条入口打开，都只有「General／Channels／Advanced」；
  搜索「antigravity」得到「No matching settings」。
- 转送取消：再发一张转送卡后强杀调用方，Dev daemon 的 `cli disconnected; cancelling` 与 `cancellation finalized` 在同一秒，
  正式端卡片收尾 `card_finalize_succeeded`，不再出现 5 秒超时。
- `--resume`：被强杀的记录显示 `(interrupted)`；旧构建的记录仍按「属于不同构建」拒绝。
- `--delivery channels` 且 Dev 没有专用测试渠道：退出码 3，明确报告没有渠道可投递，不弹本机卡片，也没有请求到达正式 daemon。

**登录后的 Grok 私有会话（修复后的构建；用户在私有 profile 里自己登录，没有读取或复制任何认证文件）**

- 启动：`dev session grok` 弹出终端标签，回执是干净的 JSON；Grok 1.0.70 在中性目录建出会话，任务文本自动提交，不再需要手动回车。
- 能力边界：Grok 在 AskHuman 里只有 MCP 提问与生命周期追踪，没有权限卡和 Stop 卡（`overview.md`：Grok 仅 None／MCP，
  不支持 stop／interject）。模型经工具搜索调用 `ask` 时，Grok 对 `use_tool` 弹出的是它自己的终端权限提示，
  要用户在 Grok 终端批准（这次等了约 29 分钟）；这不是 AskHuman 的卡片。
- 问答：批准后 Grok 日志记 `mcp_tool_call_started`（server `askhuman`、`ask`、超时 86400 s）；Dev daemon 受理请求
  `ade157e6-7eb4-4daf-b0da-4ebf30800c07`，以 `dev-relay` 渠道转送正式 daemon（正式请求 `ea43fb66-9712-4a08-88b9-c004335b1661`），
  用户回答 `ok`，两边在同一秒收尾（约 81 秒），`mcp_tool_call_completed` 成功返回；模型随后执行 `echo askhuman-dev-ok`
  （Grok 原生权限 25 ms 放行）并以 `turn_ended completed` 结束。Dev 历史行 `channel: dev-relay`、`source: Dev · HumanInLoop-devacc`、答案 `["ok"]`。
- 生命周期：Dev 的 `agents.json` 记下这条 Grok 会话（`kind: grok`、`apple-terminal`、`idle`，`lastTurnResult` 为 `completed`、来源 `hook`），
  私有 profile 的生命周期 hook 确实到达 Dev daemon。
- 全局边界：18 个全局集成文件（含 `~/.grok/config.toml`、`hooks.json`、`AGENTS.md`）的哈希与权限前后一致。
- 两处修复后的补充检查：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 通过；
  `integrations::agent_launch` 与 `development::session` 的 47 个单测通过（含改过的
  `development_profile_options_precede_new_and_fork_prompt_delimiters`，现在断言 Grok 没有 `--add-dir`、Codex 仍有）。
  完整 `cargo test` 以上一轮的 2343 通过为准，修复后没有整体重跑，新 head 以 CI 为准。

**自动检查（本机，同一临时 worktree）**

- `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 通过；完整 `cargo test`：2343 通过、0 失败、11 忽略，
  其中包含本轮新增的 6 个单测（转送取消 1、`--resume` 状态 1、集成写入守卫 4）。
  转送取消的单测先断言收尾仍被占住，再断言通知后立即放行，不会只靠时序通过。
- `vue-tsc --noEmit` 通过。完整 vitest 在机器负载 500–900（另有会话在跑 iOS 模拟器与 Xcode 测试）时有 4 个
  `PopupInboxView` 用例因 5 秒超时失败，与本轮改动无关；单独重跑这两个文件（加长超时）122 个用例全部通过。
  这不是 CI 的结果，新 head 的 CI 以 GitHub 为准。

**仍未验证**

- Dev 专用测试渠道的真实收发：用户决定先不配置专用 bot／配对。只做了代码走查和上面的无渠道失败检查。
  走查结论：`channels` 既不走转送分支也不开本机弹窗（`daemon/runtime/mod.rs` 的 `development_popup_enabled` 与转送分支都排除它），
  只用 Dev 自己的渠道配置；Dev 渠道身份与正式相同时 `development/channel_safety.rs` 关闭 Dev 的全部 IM 渠道
  （daemon 启动、配置重载、`channels` 请求与权限校验都检查，任何校验失败都按关闭处理），不会连接正式 bot。
- 其他 Agent 的登录后私有会话：Codex 暂无额度，Claude Code／Pi 未测，Cursor 永不实测。
  Grok 只验了问答与生命周期；Fork／转录控制与最终失败提醒（Grok 仅支持错误提醒）没有在登录后实测。见 `PROGRESS.md`。
