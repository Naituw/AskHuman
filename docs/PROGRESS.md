# PROGRESS

按具体任务 / 需求记录待办与当前进展。任务 / 需求完成后删除其 section（历史留在 git）。

## 进行中：Agent 生命周期追踪 + 状态窗口（实验性功能）—— 编码完成，待实测

需求 `docs/specs/agent-lifecycle-tracking.md` + 计划 `docs/plans/agent-lifecycle-tracking.md`（基于 `demo/agent-lifecycle/FINDINGS.md` 实测）。
**独立于** IM 渠道激活需求（不含 attach/激活逻辑）。要点：①设置「通用」底部隐蔽开关「实验性功能」→ 出现「实验」Tab，含 Claude/Codex/Cursor 三家「生命周期追踪」开关（开/关＝安装/卸载用户级 lifecycle hook）；②`AskHuman agents status` 开动态 GUI 窗口，按类型分组显示 工作中/空闲/已结束 agent（类型/标题/sessionID/项目/启动/最近活动/状态/pid）。
架构：daemon 中枢（agent 注册表 + 存活轮询 + TTL 兜底 + agents.json 持久化 + 闲退守卫 + 订阅推送）；hook 走二进制子命令 `AskHuman __agent-hook`（detectRunningAgent 去重 + walk 找 pid）；身份＝session_id、pid 仅判存活。决策见 spec 的 D1–D24。

已落地（cargo check + 8 个 agent_lifecycle 单测 + vue-tsc 全过）：
- 后端 `agents/`：`mod`(AgentKind/LifecycleEvent)、`detect`(运行家族判定/session_id/walk pid/kill0)、`title`(三家标题解析)、`registry`(apply_event/poll_liveness/ttl_sweep/最多留 10 条 ended/persist+load `~/.askhuman/agents.json`/snapshot)、`report`(`__agent-hook` 上报器 + 去重)。
- IPC：`TaskRequest` 加 `agent_kind/agent_session_id/agent_pid`；`ClientMsg::AgentEvent`/`AgentsSubscribe`；`ServerMsg::AgentsState`。
- daemon：注册表接线（事件落库+广播、1s 轮询+TTL、闲退守卫＝仅「工作中」agent 或有窗口连接才不退、drain 不受影响）；`handle_submit` 顺带 `touch_activity`（仅刷新对应 session）。
- 安装/卸载/状态 `integrations/agent_lifecycle.rs`：用户级 hook，jsonc CST（Claude `~/.claude/settings.json` Nested / Cursor `~/.cursor/hooks.json` Flat）+ toml_edit（Codex `~/.codex/hooks.json` + `config.toml [hooks.state]` 信任哈希，复刻 `codex-trust.cjs` 算法）；仅增删含 `__agent-hook` 标记条目，保留其它 hook/格式；与 timeout hook 共存。
- 窗口：`app::run_agents`/`create_agents_window` + 订阅推送；CLI `AskHuman agents status`（`agents` 子命令组预留扩展）；前端 `AgentsView.vue`（按类型分组、状态优先排序、相对时间动态刷新）。
- 设置 UI：`config.experimental.enabled`（serde 默认兼容旧配置）；「通用」底部隐蔽开关 + 「实验」Tab 三家开关（Windows 隐藏）；i18n zh/en。

**下一步**：①`./scripts/install.sh` 装好后用户实测 Claude / Codex（开关→`/hooks` 或 `~/.codex/config.toml` 核对→`agents status` 窗口看 工作中/空闲/已结束 流转→关窗/`kill -9` 看轮询判 GONE）；②Cursor 在日常开发中验证（双触发去重）；③实测通过后提交 + 清理本 section。

## 进行中：IM 渠道激活 —— Agent 信号 Demo（Claude/Codex/Cursor 三家均实测通过）

需求 `docs/todos/im-channel-activation.md`；Demo 已升级为**共享核心** `demo/agent-lifecycle/`
（`harness/`(common+hooklog+envprobe+poller, profile 驱动) + `harness/profiles/{claude,codex,cursor}.cjs`
+ `agents/{claude/.claude, codex/.codex, cursor/.cursor}` 各家启动目录 + `logs/<agent>/`）。
调研+实测结论记于 `demo/agent-lifecycle/FINDINGS.md`。目标：验证设计 doc 三层信号模型对各家 CLI 的可行性。

约束：**未经用户许可，绝不实际调用任何 Agent（claude/cursor-agent/codex）做实测**（消耗 token）。

**Claude Code 实测全部通过**（2026-06-13，2.1.176 / macOS）：不用 Hook 读 `CLAUDE_CODE_SESSION_ID` 即可拿会话 ID；
进程存活轮询是唯一不漏的电平信号（`kill -9` 丢 `SessionEnd`，poller 抓到 DEAD；`/exit`/关窗都触发 SessionEnd）；
turn-start↔turn-end 成对；`/clear` 轮换 session_id 但 pid 不变 → 会话身份应绑进程 pid；低轮次法：生命周期信号可 0 prompt 验证。

**Codex：实测通过**（2026-06-13，codex npm 包 / macOS；源码 `/Users/wutian/Developer/codex`）：
- 不用 Hook 拿会话 ID：shell 工具子进程 env `CODEX_THREAD_ID` == hook `session_id`（实测一致）；**hook 子进程无此 env**（靠 stdin）。
- 无 `SessionEnd`/`Notification`：正常退出/`kill -9` 都**零事件**，唯一靠 poller 抓 `DEAD`（实测均抓到，~1s）。
- turn-start(`UserPromptSubmit`)↔turn-end(`Stop`) 成对、带 `turn_id`（每轮轮换，session_id 跨轮稳定）；`Stop` 不依赖工具。
- 信任**程序化写入并实测正确**：`harness/codex-trust.cjs` 复刻 Codex 哈希算法（`"sha256:"+sha256(紧凑·键排序 JSON(归一 hook identity))`，状态键 `<hooks.json 绝对路径>:<event_snake>:<g>:<h>`），写进**用户级** `~/.codex/config.toml [hooks.state]`（项目信任沿用仓库根已有 trusted）；启动后 `/hooks` 9 条全 Active/Trusted、事件确实触发。
- hooks 默认开启（`Feature::CodexHooks` Stable）；项目根按 `.git` 向上找，但 `.codex` 沿 cwd→根逐级扫描 → 在 `agents/codex/` 启动即加载，**无需软链**。
- 进程定位：walk 命中原生 `codex` 二进制 pid（链路有 node(npm 启动器) 父进程，二者同生共死）；poller 仅启动即 arm（0 turn）、跨会话自动 re-arm。
- `/new`（干净复测）：再触发 `SessionStart`(source=startup)、**轮换 session_id、pid 不变** → 与 Claude `/clear` 一致，**身份绑 pid**。

**Cursor：实测通过**（2026-06-13，cursor-agent 2026.06.12 / macOS；先 bundle 静态核对再实测）：
- 静态：Hook 多源合并（企业/团队/用户 `~/.cursor/hooks.json`／项目 `.cursor/hooks.json`，`loadProjectHooks` 默认 true）+ **还读 `.claude/settings*.json`**；无信任哈希；21 个 camelCase 原生事件 + Claude 事件/工具名兼容映射（`Notification` 无对应）；payload 走 stdin（`argv_heredoc`/`CURSOR_HOOK_EOF`），`exit 0`+空 stdout=no-op、`exit 2`=阻塞。
- **生效的是用户级 hook**：项目级 `agents/cursor/.cursor`+`.claude` 在 CLI 下**全程未触发**（实测两轮，无 `scope=project` 事件）；改挂**用户级** `~/.cursor/hooks.json`+`~/.claude/settings.json` 后全部触发（与生产 `cursor_hook.rs`/`claude_hook.rs` 装用户级一致）。
- **0-turn arm**：`sessionStart` 用户级**启动即触发** → poller `arm→LIVE`（无需发 prompt）。
- 免 Hook 拿会话 ID（实测）：shell 工具子进程 `CURSOR_AGENT=1`+`CURSOR_CONVERSATION_ID`(==hook stdin `session_id`)+`AGENT_TRANSCRIPTS`；**hook 子进程**用 `CURSOR_PROJECT_DIR`/`CURSOR_VERSION`/`CURSOR_USER_EMAIL`/`CLAUDE_PROJECT_DIR`、会话 ID 走 stdin。
- **双触发 + 去重实锤**：因恒兼容加载 `~/.claude`，每个生命周期事件在 cursor-agent 下从 `~/.cursor`+`~/.claude` **各触发一次**（同 sid、同毫秒）；`detectRunningAgent`（env 有 `CURSOR_*`→running=cursor）让 `~/.claude` 那批 `dedupe_skip=true`、净一次。
- turn `beforeSubmitPrompt`↔`stop` 成对；关闭矩阵：正常退出有 `sessionEnd`、`kill -9` 必丢 → 唯一不漏靠 poller（~1-2s 抓 DEAD）；新会话＝新 pid（身份绑 pid）。详见 FINDINGS §7.7。

待定下一步：① 三家结论是否回写设计 doc（`docs/todos/im-channel-activation.md` §6/§10，已在 FINDINGS §9 列出建议）；
② 是否开始改生产 daemon（attach 门控 / 进程存活轮询 / turn 事件上报 / 跨家族运行时去重）；
③ 用户级临时 hook 改动已读完即还原（备份 `~/.cursor/hooks.json.bak.*`、`~/.claude/settings.json.bak.*`）。

## 进行中：严格选择模式 + 结构化输出（实测通过 → 仅剩收尾）

需求 `docs/specs/strict-choice-and-structured-output.md` + 计划 `docs/plans/strict-choice-and-structured-output.md`（已评审通过）。
阶段 0（卡片样式）全部定稿；阶段 1/2 编码**已完成**，`cargo test`(232) / `npm run build` 全绿；钉钉模板已由用户发布。
真机实测**已通过**：严格单选在 钉钉/Telegram/飞书/Slack/弹窗 五端提交链路均正确（JSON 返回 `selected_options`+`selected_indices`）；
多选（多值 indices）、非严格单选（radio+补充输入）亦通过。
实测中修复：飞书严格单选点提交后 loading 回弹——单选严格态表单内只剩提交按钮，飞书不下发 `form_value`，
`parse_card_submit` 误判为非提交；改为同时按按钮回调 `value.action=="submit"` 识别提交（`fix(feishu)` 已提交）。
已知限制（飞书）：非严格单选若「先打字、后点选 radio」会因整卡重渲染丢失已输入文字（表单外勾选器回调不带 form_value，无法回填）；
按「先点选、后打字」正常。

已落地：
- 数据/IPC：`models.rs`/`ipc/mod.rs` 新增 `select_only`/`single`/`output_format`（serde 默认，向后兼容）；TS `types.ts` 同步。
- CLI：`cli/args.rs` 解析 `--select-only`/`--single`/`--output <text|json>` + 「严格需每题有选项」校验 + 单测；`cli/mod.rs` allowlist/透传/`--scripting-help` 分发。
- 渲染：`cli/output.rs` 字段标记改恒英文常量（`[selected_options]`/`[user_input]`/`[files]`/`[status]`）、`[图片]`+`[文件]` 合并为 `[files]`、新增 `render_json`（D7：snake_case/省空字段/`answers` 仅含已答题/取消仅 `{action,channel}`）；`app/mod.rs::render_result` 改签名接 `&AskRequest` 按 `output_format` 分支。
- help：`cli/help.rs` 重组 `--help`（提问/管理/帮助三块）+ `--agent-help`（字段英文）+ 新增 `--scripting-help`，共享片段 `ask_arg_lines`/`script_flag_lines`/`result_field_lines`/`exit_code_lines` 组装。
- 渠道公共层：`conversation.rs::QuestionCtx` 透传 `select_only`/`single`。
- 弹窗：单选 radio（互斥）+ 严格隐藏补充输入/附件区 + 必须选中才可提交。
- Telegram：单选按钮互斥、严格忽略聊天自由文字、严格空提交弹 alert；推荐沿用文字前缀。
- Slack：单选 `radio_buttons`、严格去 `plain_text_input`、推荐用原生 `description`「👍 推荐」+ 文本加粗；文本回退遵守严格/单选。
- 飞书：单选勾选器移出表单 + 各挂 toggle 回调（会话自管互斥重渲染）、严格去 `input`、推荐左侧绿色 lark_md 前缀；文本回退遵守严格/单选。
- 钉钉：`card.rs` 新契约（`options=[{id,md}]`、`single`/`allow_input` 字符串布尔、h5 字号、绿色含括号推荐前缀、提交回传 id→按下标还原）；`DEFAULT_CARD_TEMPLATE_ID` 升级为 `d5dc7ac5-…schema`；文本回退遵守严格/单选。

收尾（待办）：
- 删除隐藏 demo 子命令 `AskHuman __demo-cards`（`src-tauri/src/cli/demo_cards.rs` + `cli/mod.rs` 分发）——用户暂选保留，确认无需后再删。
- 本地提交（`feat(cli,channels)` + `fix(feishu)` + 本 docs）由用户自行 push。

## 进行中：版本自更新机制（实现阶段）

需求/方案：`docs/specs/self-update.md`、`docs/plans/self-update.md`；提交规范见 `AGENTS.md`。

已完成：
- ① `update/` 模块（`mod`/`direct`/`npm`/`notes`/`state`）+ 单测 8 过。
- ② `paths::update_state_file` + `update.json` 状态读写 + 命令
  （`get_app_version`/`update_check`/`update_get_notes`/`update_apply`/`update_dismiss`/`restart_settings`）
  + 注册到 invoke handler。
- ④a 设置「关于」区：当前/最新版本、检查更新、更新（进度）、更新日志（聚合 markdown）、
  「查看全部发布」、更新后「重启设置页面」；i18n(zh/en) + `lib/ipc.ts` 封装 + `UpdateInfo` 类型。
  cargo 编译、`npm run build`、`cargo test update::` 均过。

- ③ `ipc ServerMsg::UpdateState`（snake_case 字段，同二进制两端）+ daemon：启动+24h 后台检查→落
  `update.json`→变化广播；15s 指纹监听→外部/应用内更新置 `pending` 并广播；GuiHello 握手携带当前态。
  `commands` 增进程内缓存 + `popup_update_state` 拉初值命令；GUI Helper 读 `UpdateState`→缓存+emit `update-state`。
- ④b 弹窗：右上角更新入口（圆点）+ 浮层（版本/日志/「答完生效」/更新按钮）+ 待生效横条；
  挂载先 `popup_update_state` 取初值再监听事件；zh/en i18n。
  cargo 编译、`npm run build`、`cargo test`(update::/ipc:: 共 16 过) 均通过。

- ⑤ 发布流程：仓库根 `cliff.toml`（按 D15/D16/D20：仅 feat/fix/perf/security/revert；breaking 置顶；
  scope 粗体前缀；`Release-Note:`/`Release-Note: skip` 单条覆盖——skip 改由 body 模板按 footer 过滤，
  避免无 body 提交触发 field error 误伤 feat/fix）；`release.yml` 接 git-cliff（`fetch-depth:0` +
  `taiki-e/install-action`），按 `docs/release-notes/v<版本>.md` 覆盖否则 `--latest` 生成、去前导空行、
  `body_path` 替换 `generate_release_notes`；新增 `docs/release-notes/README.md`。本地 git-cliff 2.13.1
  跑通 v0.4.x→v0.5.x 多版本，分组/跳过/中英文/Full Changelog 均正确。

⑥ 完整链路 install 实测：**已通过**（用 `GITHUB_TOKEN=$(gh auth token)` 注入认证额度绕过 60/时限流）。
- 降级 0.5.0（Cargo.toml/tauri.conf）重装 → 带 token 重启 daemon → 后台检查**无 403**、测到 0.5.3。
- 弹窗更新入口/浮层/「答完才生效」提示、关于区当前 0.5.0/最新 0.5.3 均正常。
- 点更新 → 下载官方 0.5.3 资产 → `codesign` 验签 TeamID `DMJXDB9H6Q` 通过 → 备份 `AskHuman.0.5.0.bak`
  → 原子替换；置 `pending` + 顶部「待生效」横条。答完在途请求后**下一次提问握手触发 drain→重拉**，
  daemon 换新到 0.5.3（status 确认 pid 变更、version 0.5.3）。
- 实测中修的问题：
  1. 浮层背景透明（`--bg-elevated` 仅 3~6% alpha 透出底字）→ 改用不透明 `--bg`。
  2. 更新入口图标改橙色（`--accent-orange`）+ 与「置顶」按钮加 4px 间距。
  3. **更新日志里的链接在 webview 内跳转把窗口顶掉** → 弹窗 `.up-notes`、设置 `.release-notes` 均接
     外链处理（`onContentClick`/`onNotesClick`：`openPath` 走系统浏览器）。
  4. **`init_update_snapshot` 启动残留 `pending`** → 刚启动的 daemon 即盘上二进制，pending 一律清零
     （否则换新后下个 daemon 常驻「待生效」横条）。
- 已恢复版本号到 0.5.3 并重装回开发态（dev-0.5.3，带自更新功能 + 本地签名）。
  注意：本地 dev 签名（自签证书）与官方 Developer ID 不同，跨签名换新时钥匙串会就各 secret 重新授权
  （点「始终允许」即可）；官方→官方升级签名一致则**不会**弹。
- 追加（用户要求，已随 install 落盘）：设置「关于」区「查看当前版本更新日志」折叠项
  （`update_get_version_notes` → `notes::notes_for_tag`，懒加载）。
- 追加（用户要求，已随 install 落盘）：限流处理——`github_client()`（带可选
  `ASKHUMAN_GITHUB_TOKEN`/`GITHUB_TOKEN` → `Authorization: Bearer`，token 头 sensitive）与 `http_client()`
  （npm/资产下载，不带鉴权头防泄露）分离；`github_status_error()` 把 403/429 归一为 `rate-limited`；
  前端映射友好文案 `settings.about.rateLimited`/`popup.update.rateLimited`。
- 全套 `cargo test` 通过；`npm run build` 通过；git-cliff 本地验证多版本。

自更新主体已提交：`feat(update)` + `ci(release)` + `docs`（3 条，未 push）。

追加修复（实测后用户反馈，未提交）：
- 原生关闭按钮也走二次确认：后端 `CloseRequested` 收尾态放行、否则 `prevent_close()` + emit
  `popup-close-requested` → 前端走与 ⌘W 相同的 `requestCancel()`（`GuiBridge::is_done` /
  `Coordinator::is_finalizing` 判收尾，避免拦截死循环）。
- 取消确认面板 `.confirm-box` 背景透明 → 改不透明 `--bg`（同更新浮层修复）。
  改动文件：`src-tauri/src/app/mod.rs`、`src/views/PopupView.vue`；install 实测两项均通过。
