# 计划：调用方断开时保住弹窗里没提交的内容

规格：[popup-draft-on-cancel.md](../specs/popup-draft-on-cancel.md)（设计定案以规格为准；本计划只写怎么做、怎么验）。
状态：已实施并验收（2026-11-16，实施记录与验收结果见 §3 和规格 §5）。执行位置：用户选了主工作区。主工作区里有别的会话的未提交改动
（`src-tauri/src/codex_desktop/mod.rs`、`docs/specs/codex-desktop-session-integration.md`、`ios/AskHuman/Views/Attachments/*`、
`ios/AskHumanUITests/*`）：提交只按明确路径；它们会被一起编进我的构建，测试失败要先分清是不是它们的；
安装前先问用户怎么处理它们（用户在选位置时已知晓这一点）。

## 0. 前置核对（开工前做完，结果写进实施记录）

- `daemon/request.rs` 里 `RequestEntry` 的 `session_key` / `coalesce_fingerprint` 是在哪一步算出来的、`ShowPayload`
  是否在此之后才构造（要用它们填 `draftKey`）。
- 弹窗渠道的 `Cancel`（`GuiHelperPopupChannel::interrupt`）与后来 `prune_inbox` 补发的 `Cancel` 是否走同一条宿主连接、
  先后顺序是否保证（决定「`Cancel` 先到、`Show` 后到」这句话能不能成立；不成立也只是这次没载入，见规格 D6）。
- `ShowPayload`、`ServerMsg::Cancel` 是否进入恢复存储或其序列化哈希（AGENTS.md「Changing recovery storage」）。
  新增字段无值时不序列化，哈希应不变，要用现有夹具测试证明，而不是推断。
- `popup_init`（`commands.rs` 62 行起）同时服务合并宿主、预热宿主、独立窗口，并被 `hover_init` 同步调用；它是同步函数，
  **不改它的签名与行为**——载入走新命令 `popup_draft_claim`，只在合并宿主的前端调用。
- 历史条目的结构体没有 `deny_unknown_fields`（已查）：旧版本读到带 `draft` 的行不会失败，只在重写时丢掉这个字段。

## 1. 分阶段

### P1 历史存储（Rust，无界面）

- `history.rs`：`DraftKey { session_key: Option<String>, fingerprint: String }`；`HistoryDraft { key: Option<DraftKey>,
  answers: Vec<HistoryAnswer>, annotations: Vec<UserAnnotation>, claimed_by: Option<String> }`；
  `HistoryEntry.draft: Option<HistoryDraft>`（`#[serde(default, skip_serializing_if = "Option::is_none")]`）。
- `attach_draft(id, draft) -> bool`：持跨进程锁；只在该 id 的记录存在、`action == Cancel`、还没有草稿时写入。
- `claim_draft(session_key, fingerprint, request_id) -> Option<HistoryDraft>`：持锁；取最新的「有草稿、`key` 的会话键与指纹都
  相同、未被认领或已被本请求认领」的记录，写 `claimed_by`，返回草稿；会话键为空不匹配。
- 测试（`history.rs` 的 `mod tests`）：无 `draft` 的旧记录照常解码；附加到取消记录；对不存在 / `send` / 已有草稿的记录不动；
  认领取最新、一次性、对同一请求幂等；换指纹 / 换会话不认领；`trim` / `delete_ids` / `clear_all` 带走草稿；
  `recent_sends_*`、`entry_by_id` 对带草稿的取消记录行为不变。

### P2 取消路径（Rust，daemon 与渠道）

- `ipc/mod.rs`：`ServerMsg::Cancel` 加 `#[serde(default)] keep_draft: bool`；`ShowPayload` 加
  `#[serde(default, skip_serializing_if = "Option::is_none")] draft_key: Option<DraftKey>`。所有构造与解构处由编译器列出，
  已知的有：`Cancel`——`channels/popup.rs`、`daemon/runtime/inbox.rs`（`prune_inbox` 等两处）、`daemon/runtime/mod.rs`
  （`serve_gui` 的早到取消）、`app/popup_inbox.rs`（解构）、`daemon/runtime/inbox/tests.rs`（测试里的解构补 `..`）；
  `ShowPayload`——`daemon/request.rs` 三处（328 与 815 行是 Permission 确认卡，`draft_key` 填 `None`；614 行是 Ask 类，见下）、
  `app/popup_inbox.rs` 的测试夹具。默认值一律 `false` / `None`，不改变它们现有行为。
  按 AGENTS.md「Changing IPC messages」读过 `docs/overview.md` 的「IPC 协议号」再改；**不升** `PROTOCOL_VERSION`。
- `channels/mod.rs`：`Channel` 加带默认实现的 `interrupt_abandoned(&self, reason)`（默认转调 `interrupt`）；
  `GuiHelperPopupChannel` 覆盖它，对 `Cancelled` 发 `keep_draft: true`。IM / iOS 渠道不改。`cancel_request` 的调用方只有
  调用方断开、daemon 关停（`cancel_all_requests`）、开发中继，全是「被放弃」，没有人的取消走它（已查全部调用点）。
  `prune_inbox` 与 `serve_gui` 对已终结请求补发的 `Cancel` 是兜底清理，`keep_draft` 保持 `false`。
- `coordinator.rs::cancel_request`：先 `record_history`，再对每个渠道 `interrupt_abandoned`。锁的持有范围不变
  （原来 `record_history` 就在 `submit_lock` 与 `inner` 锁内）。`submit` 里「人」的取消仍走 `interrupt`。
- `daemon/request.rs`：Ask 类请求（`InteractionRequest::Ask`，含 whats-next、Stop）构造 `ShowPayload` 时填 `draft_key`
  （`ask_dedup::session_key` + `coalesce_fingerprint`）；Confirm 不填。
- 测试：`cancel_request` 在 interrupt 时历史里已有这条记录（用测试渠道在 `interrupt` 里读历史）；弹窗渠道对调用方 / 系统取消发
  `keep_draft: true`，对 `AnsweredBy`、人的取消、`Handoff` 不带；`Cancel` 新旧 JSON 互读（缺字段时为 `false`）。

### P3 保存：弹窗宿主与前端

- `app/popup_inbox.rs`：`Inner` 加「待存草稿」集合（请求 id → `draft_key`）与 `Notify`；`finish` 在 `keep_draft` 时登记并让
  `Terminal` 带 `keepDraft`；提供 `take_pending(id)`、`wait_pending(key, 3s)`；两条退出路径（`PopupHostIdleAck keep_warm=false`、
  `PopupHostShutdown`）改为：集合非空时在后台任务里最多等 3 秒再 `app.exit(0)`，空则立即退出，行为与今天一致。
- `commands.rs`：新命令 `popup_inbox_draft(request_id, draft: Option<DraftSave>)`，注册到 `app/invoke.rs`：
  图片按题用 `cli::image_writer::save` 落盘（沿用请求 id 目录），拼 `HistoryDraft`（`key` 取该请求的 `draft_key`），
  `history::attach_draft`；无论成败都清掉待存登记并唤醒等待者；失败只写日志。
- 前端：`src/lib/types.ts` 加 `DraftSave` / `DraftLoad`；`src/lib/ipc.ts` 加 `popupInboxDraft`；
  `usePopupCore.ts` 加 `collectDraft()`（选项文本、文字、图片、回复文件路径、批注；不含待办 / 指令；没有内容返回 `null`）并通过
  `PopupScope` 新增的 `draftReader` 注册；`PopupInboxView.vue` 的 `popup-inbox-terminal` 监听里，遇到 `keepDraft` 同步读取并
  `popupInboxDraft(...)`（不 await，卡片照常移除）。
- 测试：Rust——待存集合的登记 / 清除 / 超时、退出路径的有界等待、`popup_inbox_draft` 的图片落盘与记录合并；
  vitest——`collectDraft` 的各部分与空内容、提交进行中也能读、监听里只在 `keepDraft` 时回传、卡片未挂载时回传 `null`。

### P4 载入：重发的新卡

- `commands.rs` 新增异步命令 `popup_draft_claim(request_id) -> Option<DraftLoad>`（注册到 `app/invoke.rs`）：请求带 `draft_key`
  且有会话键时，先 `wait_pending(同键, 3s)`，再 `history::claim_draft(...)`，返回 `DraftLoad`（选项、文字、图片路径、
  已确认仍存在的文件、批注）；其余情况返回 `None`。`popup_init` 不动，预热与独立窗口路径不会调用它。
- 新命令 `popup_draft_images(paths)`：只读取系统临时目录下 `askhuman/` 里存在的图片，返回 `ImageAttachment[]` 与缺失张数
  （路径先规范化再校验前缀，限制单文件大小，防止被当成任意文件读取口）。
- 前端：`src/lib/ipc.ts` 加 `popupDraftClaim` / `popupDraftImages`；`renderInit` 之后（仅合并宿主的卡片）异步取草稿，
  取到后 `applyDraft`——选项按文本匹配、文字、图片（调命令）、回复文件、批注（设为未定位后 `reapply()`，由现有的
  `engine.relocate` 按引文定位）；置 `draftNotice`，卡片顶部一行可关闭的提示「已载入上次未提交的内容」（有图片缺失时附张数）。
  `hasDraft` 随内容自然为真。若用户在草稿到达前已经开始输入，**不覆盖**已有内容（只补空的部分，提示照常显示）。
- i18n：`zh.ts` / `en.ts` 加 `popup.draftRestored`、`popup.draftRestoredMissing`。
- 测试：Rust——`popup_draft_claim` 里「等待 → 认领」的纯逻辑（抽成函数）、无会话键 / 无 `draft_key` 返回 `None`、
  `popup_draft_images` 的路径校验；vitest——`applyDraft` 各部分、新卡没有的选项丢弃、用户已输入时不覆盖、提示的显示与关闭、
  批注找不到时排在最后、独立窗口 / 预热卡不调用认领。

### P5 历史窗口

- `HistoryView.vue`：带草稿的取消记录加「草稿」标记；详情里每题下只读显示草稿（选项、文字、图片、文件）与批注，复制按钮，
  状态行写明「取消时未提交的内容」（实施时写的是「调用方断开时…」，验收后改了，见规格 D8）与是否已载入；`haystackOf` 把草稿文字纳入搜索。
- i18n：`history.draft*`。
- 测试：vitest——标记与详情、搜索命中草稿文字、没有草稿的记录渲染不变。

### P6 文档、验证、安装

- 文档回写：`overview-popup-ui.md`（第 37 行那段「内存草稿」补一句：调用方断开时草稿写进历史、重发时载入；宿主现在也会写历史文件）、
  `specs/reply-history.md`（「当前实现补充」加一条 `draft` 字段）、`specs/duplicate-ask-coalescing.md`（D4 末尾加一句指向本规格）、
  本规格状态改「已实现」并补验收结果；检查 `docs/wiki/` 里历史 / 弹窗相关页面是否需要同步；`docs/PROGRESS.md` 更新。
- 验证：`node scripts/machine-health.mjs` → `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test`、`vitest`、`vue-tsc`；
  按 AGENTS.md 用 `./scripts/install.sh` 安装后验证（独立 Dev Worktree 时按 `docs/agent-worktree-setup.md`）。
- 真实验证（规格 §5 的 1–10）：用真实弹窗输入内容、杀掉 CLI 子进程（等同 MCP 被关闭时的结果）、看历史、重发、载入、
  再取消、用户取消 / 手机抢答的不保存；daemon `stop` 一项单独做。「调用方断开」用命令行杀掉 `AskHuman` CLI 子进程来模拟，
  不依赖任何 Agent 会话。真人点击与输入的部分在同一轮提问里请你配合，做不到的如实写在实施记录里。

## 2. 风险

| 风险 | 处理 |
| --- | --- |
| `cancel_request` 调换顺序后，渠道 interrupt 晚于历史写入一次文件 IO | 原来 `record_history` 就在同一把锁内、同一函数里，只是换了先后；IM 卡定格延迟是一次小文件写 |
| 宿主退出被「待存草稿」拖住 | 有界 3 秒，超时记日志并退出；集合为空时与今天一样立即退出 |
| 前端读草稿时卡片已被别的流程移除 | 监听里同步读；读不到回传 `null`，宿主清登记，不会卡住退出 |
| 草稿图片在系统临时目录，被清理后载入缺图 | 载入时跳过并在提示里写明张数；规格 §6 第 3 条已列为待确认 |
| 载入影响预热 / 独立窗口路径 | `popup_init` 不动；认领是独立命令，只有合并宿主的前端调用，有测试覆盖「其余路径不调用」 |
| 草稿到达时用户已开始输入 | 不覆盖已有内容，只补空的部分 |
| 历史文件被宿主与 daemon 两个进程写 | 全部走既有跨进程锁与原子重写；`attach_draft` / `claim_draft` 各自一次读-改-写 |
| 新旧版本混用丢草稿字段 | 规格 D10 已说明：只丢草稿本身；不崩溃 |

## 3. 实施记录

### 提交（主工作区；提交只按明确路径）

| 阶段 | 提交 |
| --- | --- |
| 规格与计划、评审记录 | `963cbbcf`、`dd745613` |
| P1 历史存储 | `4e76e67c` |
| P2 取消路径 | `1deef0f1` |
| P3 保存 | `85a1bd44` |
| P4 载入 | `cb89ce16` |
| P5 历史窗口 | `0b3525fb` |
| P6 文档回写（含 wiki 里「系统触发的取消不记录」的更正） | `619f8758` |
| 验收结果与本记录 | 本记录所在的提交（标题 `docs(popup): record the draft-on-cancel acceptance run …`） |

### 前置核对的结论

- 会话键与指纹在 `daemon/request.rs` 构造 `ShowPayload` 之前就已算好（收敛键本来就存在 `RequestEntry` 里），`draft_key` 直接取它们；
  只有 Ask 类请求填，Confirm 填 `None`。
- 弹窗渠道的 `Cancel` 与 daemon 请求表后来补发的 `Cancel` 走同一条宿主连接，但**先后原来不保证**：宿主在听到的第一条 `Cancel`
  上结束卡片，补发的那条（不带 `keepDraft`）若先到，草稿就读不到。另一个会话在 `229f9e7e` 里把 `finalizing` 的设置挪到所有渠道
  都 `interrupt_abandoned` 之后，才让这个先后成立（他们修的是幽灵卡问题，提交说明里点名了本篇的 D4 / D5）。
- 恢复存储：`ShowPayload`、`ServerMsg::Cancel` 新增字段都是 `#[serde(default)]`，无值时不序列化；完整 `cargo test` 里的恢复存储哈希夹具通过，
  没有升 `PROTOCOL_VERSION`。
- `popup_init` 是同步函数，没有改它；载入走新命令 `popup_draft_claim`，只有合并宿主的前端调用。
- 历史条目结构体没有 `deny_unknown_fields`，旧版本读到带 `draft` 的行不会失败。

### 与计划不同的地方

- **没有 `popup_draft_images` 命令，也就没有路径前缀校验。** 图片由宿主在 `popup_draft_claim` 里读，连同缺失张数一起放进
  `DraftLoad` 返回，少一次往返。计划里的「只读 `askhuman/` 前缀内的文件」是为「页面传路径给宿主读」那个命令设计的；现在命令只收请求 id，
  路径只来自宿主自己写进历史草稿的那一份，页面碰不到，所以没做前缀校验，只保留单文件 32 MiB 的上限、缺失 / 过大都按「缺图」处理。
  代价：历史文件若被别的进程改过，草稿里的图片路径可以指向别处的文件（会被读进这张卡的表单，仍要用户自己点发送才会出去）；
  能改历史文件的进程本来就能直接读那些文件。
- **宿主退出路径多了一条。** 计划写的是两条（`PopupHostIdleAck keep_warm=false`、`PopupHostShutdown`）；实现里与 daemon 的连接断开
  （EOF）导致的退出也会等待待存草稿（同样有界 3 s），否则 daemon 先停时草稿会在宿主被带走的路上丢掉。
- 规格里两处前提在实施时更正（已写回规格）：§1 的「26 次调用方取消」其实是 15 条旧协调器单元测试写进真实历史的污染
  （项目 `/project`）、8 条 Antigravity 实验、3 条真实的；§6 第 3 条草稿图片的保留期是 daemon 自己的临时目录清理（24 小时），
  不是系统的。

### 验证

- 最终 HEAD `53f234c2`：`cargo fmt --check`、`cargo clippy --all-targets -D warnings` 通过；`cargo test` 3433 通过 / 0 失败 / 12 忽略；
  vitest 139 个文件 / 1960 项；`vue-tsc` 通过。Node 脚本测试里只有 `codex-native-title.node-test.mjs` 失败（读本机 Codex App 的包，
  与本篇无关）。
- 安装：用户先确认另一个会话的未提交改动已提交（`229f9e7e`、`53f234c2`），再从 `53f234c2` 全局安装；daemon 热更新到指纹 `f69c35f2`。
  `53f234c2` 让被热更新接管的空闲宿主退休并换成新镜像；安装当时的日常弹窗宿主还被另一个会话的一条长时间未答的提问占着，
  要等那条结束才会换成新二进制。
- 真实验收：用户选了「不做真人验收」，改在隔离的私有实例里无人值守地跑，界面用浏览器页看，逐项结果与没验证到的部分见规格 §5「验收结果」。
- 清理：私有实例已停，临时目录和浏览器页用的一次性脚本都已删；验收期间真实历史和 daemon 没动过。

### 验收后用户定的几件事（2026-11-16）

- **措辞**：历史窗口的状态行和「复制草稿」的标题原来写「调用方断开时未提交的内容」，对 daemon 停止造成的取消（渠道「系统」）不准；
  改成「取消时未提交的内容」（英文 `Not submitted when it was cancelled`）。载入到新卡的提示本来就不带来源，没有动。
- **历史里的旧测试污染**：用户让我删。先把整份历史备份到 `~/.askhuman/history.backup-before-test-pollution-cleanup.jsonl`（权限 0600，
  用户不需要时可自行删掉），再按历史文件的锁，只删了 15 条项目为 `/project`、取消、无回答、无草稿的记录（来源 Codex、会话 `session-1`、
  内容 `context` / `question`，是旧协调器单元测试的夹具）；删完核对过：其余 129 行与原文件逐字节相同、顺序不变。
- **图片路径前缀校验**：用户定了不补（见上面「与计划不同的地方」）。
- **推送**：用户同意推送；到我检查时 `origin/main` 已经在 `46d327ef`，无需再推。

### 发布（2026-11-17）

用户要求发布新的补丁版本，草稿功能随桌面 [v0.17.2](https://github.com/Naituw/AskHuman/releases/tag/v0.17.2) 公开。
同一版还带上了 v0.17.1 之后其余的面向用户的提交（共 90 条），其中包括仍在等用户验收的 Antigravity 深度集成；
发布说明如实写了「只在 macOS 验过」和升级后要执行一次的迁移命令。

- **发布说明**：用户在手写版与 git-cliff 自动版之间选了手写版，随版本提交为 `docs/release-notes/v0.17.2.md`
  （英文、按功能与修复分组、不含 iPhone 专属条目）。Release 页的正文与该文件逐字一致。
- **流程**：干净的临时 worktree 里 `bump-version` 到 0.17.2（含 `Cargo.lock`），提交 `1d7c51fd chore(release): prepare v0.17.2`，
  按 SHA 单独推送。该提交的 [CI 43917249624](https://github.com/Naituw/AskHuman/actions/runs/43917249624) 七项全绿
  （上一个提交 `1565ef0f` 也全绿，Windows `cargo test` 在 `7592c4b3` 的修复后恢复通过）。
  注解 tag `v0.17.2` 指向 `1d7c51fd`，只推了这一个 tag：没有用 `--tags`（本地的 `ios-testflight-*` 标签没有出去），
  也没有用 `scripts/publish.sh`（它是裸 `git push`）。
- **发布结果**：[Release 43921656141](https://github.com/Naituw/AskHuman/actions/runs/43921656141) 全部成功，
  从打 tag 到结束约 24 分钟（含排队约 13 分钟），2026-11-17 02:08:54（北京时间）发布。
  Release 为非草稿、非预发布，四个二进制归档和 `SHA256SUMS` 都在。
- **npm**：`askhuman` 与四个 `@humaninloop/*` 平台包的 0.17.2 都能查到，latest 一致，主包依赖固定为 0.17.2。
  注册表有几分钟的传播延迟：工作流结束后约 3 分钟，`darwin-arm64` 与 `linux-x64` 才能被直接查到；
  期间我做的第一次隔离安装只装上了主包，用 `--prefer-online` 重装才装全。刚发布的几分钟里，用户可能遇到同样的情况。
- **核验**：下载的 Mac ARM64 归档通过 `SHA256SUMS`；解出的程序通过严格签名验证（Developer ID Application，
  TeamID `DMJXDB9H6Q` 与既有常量一致），`--version` 为 v0.17.2；`todo-sync resource-info --require-bundled`
  给出内嵌 production 组件版本 0.17.2、协议 1。隔离目录里全新的 npm 安装（`--prefer-online`）能运行 `AskHuman --version`，
  其 native binary 与 GitHub 下载包的 SHA-256 一致。
- **没验证**：Windows、Linux、Intel Mac 的发布二进制没有在本机运行（CI 只覆盖编译与测试）；
  没有实际走一遍应用内自更新到 0.17.2；本机日常版仍是 0.17.1，重装后才对齐。

### 没做、留给后续的

- 只杀 MCP server（SIGTERM / SIGKILL）会留下孤儿 CLI 子进程和一张不会关的卡，是另一类问题（规格 §4），没有处理。
