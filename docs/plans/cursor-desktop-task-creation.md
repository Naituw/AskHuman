# Cursor Desktop（Agents 窗口）新建任务实施与验证

需求与决策见 [规格](../specs/cursor-desktop-task-creation.md)；Max 逻辑见 [Max 计划](cursor-max-control.md)。
同一 worktree、同一分支实现，排在 Max 控制之后。

**硬约束**：开发与测试不启动真实 Cursor、不发送提示词；适配器只用合成对象图。Dev Instance 按设计不启用该能力，
所以真实验证只能在日常安装上做，并且只在验证计划获批后。

## 实施

1. **适配器**：消息桥的渲染器适配器（`message_renderer.js`）增加 `create(operation)`——Agents 窗口与项目选择、结构预检、
   `createAgent` 加提交前闸门（读回核对通过才允许提交）、`dryRun`、`pending` 与按操作 ID 去重；主进程分发
   （`message_dispatch.js`）只在唯一的 active Agents 窗口执行。背景完成适配器及其修订不动。
2. **Rust 运行时**：`cursor_desktop::Operation::Create`、回执（`cursor-desktop-actions/<id>.json`）、状态与不可用原因、
   桥调用与结果映射；`launchPreference` 配置与目标选择（与 Codex 同构）；整体可用性只做静态检测。
3. **IPC**：新增 `ClientMsg::CursorDesktop` / `ServerMsg::CursorDesktop`，`PROTOCOL_VERSION` 4→5；先读 overview 的
   「IPC 协议号」一节，更新历史与 `daemon-graceful-drain.md`。
4. **入口**：`start_task` 与 GUI `new_task_launch` 的 Desktop 分支泛化为按 Agent 种类；iOS 回执里的 Agent 种类不再写死
   Codex；表单加 Cursor 的目标状态行；操作 ID 复用以便重试。
5. **Max 与权限**：`modelConfig` 由 Max 核心模块构造；`yolo`→`agentApprovalModeOverride`；提交后读 `composerData`
   复查 `maxMode`，异常发告警卡。
6. 文档：规格与计划、overview 的模块地图与新任务一段、`overview-configuration.md` 的新字段、用户向 wiki
   （`settings.md` / `settings.en.md`）；协议号历史与 `daemon-graceful-drain.md` §8 记 4→5 的过渡。

## 验证

- JS：假服务与假草稿的对象图脚本测试——成功、结构预检失败、非 Agents、各核对项不符、提交抛错、重复调用、`dryRun`
  清理。Rust：桩桥与临时目录覆盖回执阶段、崩溃恢复、选择规则、错误映射、协议新旧互通。
- Dev Instance 内只验证非 Cursor 部分（界面、状态行、协议）。
- **真实验证**不自动执行：下面的「真实验证计划」经用户批准才做，失败即停，不重试。

## 真实验证计划（2026-11-04 提交审批，批准前不执行任何一步）

只验证本机看不到的两件事：Cursor 私有创建入口在真实环境里的行为（规格 §9 的未验证项），以及 Max 保护在日常安装上的
真实表现。**逐阶段推进，任一阶段与预期不符就停下、记录、报告，不重试、不绕过。** 真实模型请求只有阶段 3、4 各一次，
都用缓存里没有任何 Max 变体的小模型、不允许 Max、一句话提示、不给工具权限、不碰别的项目。

前置：日常安装换成本分支的构建（Desktop 路线默认仍关，Max 保护默认开）。`./scripts/install.sh` 会让 daemon 换新
（协议号 4→5，按排空过渡，见 `daemon-graceful-drain.md` §8）。合并与安装的先后由用户定（见 `docs/PROGRESS.md`）。

| 阶段 | 做什么 | 请求数 | 通过标准 | 不符时 |
| --- | --- | --- | --- | --- |
| 1 日常版只读检查 | 设置页 Cursor 小节、GUI 新建任务的 Cursor 模型列表与 Max 标记、默认模型被拦截、「允许 Max」解除、换小模型；手机上同样的开关与标记（用户看，中英文各一次）。**不点开始** | 0 | 计划显示「旧计划」；本机缓存 47 个模型里 27 个可不开 Max 用、20 个仅 Max（含 CLI 默认的 Grok 4.7），界面与之一致 | 记录差异，停 |
| 2 Desktop `dryRun` | Cursor 开着、恰一个 Agents 窗口、里面有验证项目；`echo '{"op":"create","cwd":…,"text":"dry run","permission":"agent-default","id":<新 UUID>,"model":<小模型>,"dryRun":true}' \| AskHuman __cursor-desktop` | 按设计 0 | 结果 `dryRun`；窗口里没有留下聊天（或已软归档）；回执 `dryRun`；状态库里该 ID 没有消息 | 任何其他结果（`nativeUnavailable`、`rejected` 等）都带原因，停，**不进阶段 3** |
| 3 一次最小真实创建 | 同一项目与小模型，`dryRun:false`，提示词「只回复 OK，不要使用任何工具」，权限默认 | 1 | 见下 | 闸门拒绝＝没有请求；已发出而模型、Max 或目标不符：请用户在 Cursor 里点停止，停 |
| 4（可选）CLI 一次 | GUI 新建任务表单（目标终端），Cursor、同一小模型、不允许 Max、同一句提示 | 1 | `cli-config.json` 的 `maxMode` 仍为 `false`；45 秒内没有 Max 告警卡；任务正常回答；表单与 helper 无异常 | 停 |

阶段 3 要核对的（对应规格 §9）：结果 `started` 且会话 ID＝操作 ID，回执 `accepted`；会话在 Agents 窗口的正确项目下、本地、
没有新建 worktree 或分支（项目的 `git worktree list` 前后一致）；模型是所选小模型、没有 Max 标记、权限为默认；只读读
`composerData:<id>` 得 `modelConfig.maxMode=false`；20 秒内没有告警卡；同一 ID 再发一遍得回执原结果、不出现第二个聊天；
阶段 2 的 `dryRun` 没有留下草稿；约 10 分钟后 Cursor 正常（适配器持有引用的副作用）；用户在 Cursor 网页用量里确认这一次按
普通请求计，不是 API 费率。

验证项目：用户在 Agents 窗口里指定一个现有的小项目，或由我建一个空的 git 仓库（零请求）交给用户加进 Agents 窗口。
小模型由用户在批准时选（缓存里没有任何 Max 变体的候选：GPT-5 Mini、Gemini 2.5 Flash、Claude Haiku 4.5、GPT-5.4 Nano 等）。

**费用上界**：阶段 3、4 各最多一个普通（非 Max）请求；阶段 2 在私有闸门的行为与读到的代码不一致时最多发出一句固定占位文字，
同样是一个普通请求，并且这种情况下**不再进入阶段 3**。

## 验证记录

2026-11-03～04，只在 worktree 与本机：没有启动真实的 Cursor 任务，没有向任何模型发送提示词；适配器只对着合成的服务与会话对象跑。

- JS：`scripts/cursor-create.node-test.mjs` 19/19 通过（成功只提交一次、以现有默认配置为底、`yolo` 覆盖与读回、每个读回不符都在提交前停下并归档、
  `dryRun` 通过与被拒、结构预检、项目歧义与别名、非法操作、提交前后出错的区别、过期不提交、同 ID 加入／冲突／拒绝后重来、`pending`、
  引用保持、Agents 窗口的唯一性与 active 窗口）；原有 `cursor-message.node-test.mjs` 不变。
- Rust：`cargo test` 2409 通过、0 失败，其中 `cursor_desktop::create` 覆盖模型与 Max 计划、操作校验、请求形状、`pending` 续问、
  拒绝与 `dryRun` 为终态、「没发出去」与「回答丢了」的区别、回执（重试按回执返回、同 ID 不同内容冲突、崩溃读作未知、`dryRun` 不盯）、
  不可用原因；clippy 与 `cargo fmt --check` 无告警。
- Dev Instance（装在 worktree 的 `.askhuman-dev/`）：`AskHuman __cursor-desktop` 的 `dryRun` 请求回
  `{"Err":"A Dev Instance does not use Cursor Desktop"}`（按设计）；daemon 握手 `version 0.16.0 (protocol 5)`。
- **未验证**（等上面的验证计划获批）：规格 §9 的全部不确定点；表单、IM、iOS 在真实 Cursor 下的状态与结果。
