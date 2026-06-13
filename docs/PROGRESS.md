# PROGRESS

按具体任务 / 需求记录待办与当前进展。任务 / 需求完成后删除其 section（历史留在 git）。

## 待办：IM 渠道激活 —— Agent 信号 Demo（Claude/Codex/Cursor 三家均实测通过）

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

## 待办：daemon 二进制变化检测 —— 轮询 vs filewatch（后续评估，优先级低）

二进制变化检测目前是 **15s 轮询** `current_exe()` 指纹（稳态≈1 次 `stat`，靠 `binhash.json` 内容哈希缓存避免重哈希）。
是否改 **filewatch** 待权衡——难点：二进制走原子替换（rename 换 inode，需盯父目录 + 按文件名过滤 + 每次替换后重挂，
参考 `config_watch.rs`）、装在任意目录（`~/.local/bin`/brew/npm 前缀/`.app` bundle…）、且 watcher 仍要 stat/hash 才能确认
内容**真**变（指纹是内容哈希而非 mtime）。延迟要求松（~15s 够）+ Hello 路径兜底，故暂保持轮询。
