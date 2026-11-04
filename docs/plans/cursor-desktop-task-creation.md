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
- **真实验证**不自动执行：下面的「真实验证计划」里只有阶段 1–2 由 Agent 在用户批准装机后做，阶段 3–4 由用户自己做；失败即停，不重试。

## 真实验证计划（2026-11-04 用户裁定：阶段 1–2 由 Agent 在日常版装好后执行，阶段 3–4 由用户自行执行）

只验证本机看不到的两件事：Cursor 私有创建入口在真实环境里的行为（规格 §9 的未验证项），以及 Max 保护在日常安装上的
真实表现。**逐阶段推进，任一阶段与预期不符就停下、记录、报告，不重试、不绕过。** 真实模型请求只有阶段 3、4 各一次，
都用缓存里没有任何 Max 变体的小模型、不允许 Max、一句话提示、不给工具权限、不碰别的项目。

前置：日常安装换成本分支的构建（Desktop 路线默认仍关，Max 保护默认开）。`./scripts/install.sh` 会让 daemon 换新
（协议号 4→5，按排空过渡，见 `daemon-graceful-drain.md` §8）。合并与安装的先后由用户定（见 `docs/PROGRESS.md`）。

用户 2026-11-04 的裁定：Agent 只执行阶段 1–2（按设计 0 请求），**不执行阶段 3、4**；阶段 3、4 由用户按下面的「阶段 3、4：用户自行执行的步骤」
自己做，并把现象原样告诉 Agent。计划写好不等于获准：装日常版之前和每个阶段开始之前，Agent 都先在日常渠道里问用户。

| 阶段 | 做什么 | 请求数 | 通过标准 | 不符时 |
| --- | --- | --- | --- | --- |
| 1 日常版只读检查（Agent） | 设置页 Cursor 小节、GUI 新建任务的 Cursor 模型列表与 Max 标记、默认模型被拦截、「允许 Max」解除、换小模型；手机上同样的开关与标记（用户看，中英文各一次）。**不点开始** | 0 | 计划显示「旧计划」；本机缓存 47 个模型里 27 个可不开 Max 用、20 个仅 Max（含 CLI 默认的 Grok 4.7），界面与之一致 | 记录差异，停 |
| 2 Desktop `dryRun`（Agent） | Cursor 开着、恰一个 Agents 窗口、里面有验证项目；`echo '{"op":"create","cwd":…,"text":"dry run","permission":"agent-default","id":<新 UUID>,"model":<小模型>,"dryRun":true}' \| AskHuman __cursor-desktop` | 按设计 0 | 结果 `dryRun`；窗口里没有留下聊天（或已软归档）；回执 `dryRun`；状态库里该 ID 没有消息 | 任何其他结果（`nativeUnavailable`、`rejected` 等）都带原因，停，**不进阶段 3** |
| 3 一次最小真实创建（用户） | 同一项目与小模型，`dryRun:false`，提示词「只回复 OK，不要使用任何工具」，权限默认 | 1 | 见下 | 闸门拒绝＝没有请求；已发出而模型、Max 或目标不符：请用户在 Cursor 里点停止，停 |
| 4（可选）CLI 一次（用户） | GUI 新建任务表单（目标终端），Cursor、同一小模型、不允许 Max、同一句提示 | 1 | `cli-config.json` 的 `maxMode` 仍为 `false`；45 秒内没有 Max 告警卡；任务正常回答；表单与 helper 无异常 | 停 |

阶段 3 要核对的（对应规格 §9）：结果 `started` 且会话 ID＝操作 ID，回执 `accepted`；会话在 Agents 窗口的正确项目下、本地、
没有新建 worktree 或分支（项目的 `git worktree list` 前后一致）；模型是所选小模型、没有 Max 标记、权限为默认；只读读
`composerData:<id>` 得 `modelConfig.maxMode=false`；20 秒内没有告警卡；同一 ID 再发一遍得回执原结果、不出现第二个聊天；
阶段 2 的 `dryRun` 没有留下草稿；约 10 分钟后 Cursor 正常（适配器持有引用的副作用）；用户在 Cursor 网页用量里确认这一次按
普通请求计，不是 API 费率。

### 阶段 3、4：用户自行执行的步骤

前提：阶段 1、2 都按预期通过，日常版已装上本分支。**每一步与预期不符就停，把现象和完整输出原样告诉 Agent，不要重试，也不要换参数再试。**
下面的 `<P>`（验证项目的绝对路径）和 `<M>`（小模型的 `--model` 标识）由 Agent 在阶段 2 通过后填好，连命令一起发给用户。

**阶段 3：在 Agents 窗口里新建（1 个普通请求）**

1. Cursor 开着，只留**一个** Agents 窗口，验证项目 `<P>` 已在里面。先记下 `git -C <P> worktree list` 的输出。
2. 在终端里运行（`ID` 是这次操作的编号，也是 Cursor 里会话的 ID；保留这个终端，后面要再用）：

   ```bash
   ID=$(uuidgen); echo "$ID"
   echo '{"op":"create","cwd":"<P>","text":"只回复 OK，不要使用任何工具","permission":"agent-default","id":"'"$ID"'","model":"<M>","dryRun":false}' | AskHuman __cursor-desktop
   ```

   成功时输出形如 `{"Ok":{"sessionId":"<ID>","status":"started","dryRun":false,…,"maxMode":false}}`。输出 `{"Err":"…"}` 时先读原因：
   被拒绝的原因（没有 Agents 窗口、多个窗口、读回不符等）都表示**没有向模型发送任何内容**。
3. 核对：
   - Agents 窗口里 `<P>` 项目下多了一个本地会话，模型是 `<M>`，没有 Max 标记，权限是默认，回答「OK」；
   - 状态库里该会话没开 Max（只读查询）：

     ```bash
     sqlite3 -readonly "$HOME/Library/Application Support/Cursor/User/globalStorage/state.vscdb" \
       "select json_extract(value,'\$.modelConfig.maxMode'), json_extract(value,'\$.modelConfig.modelName') from cursorDiskKV where key='composerData:$ID'"
     ```

     应得 `0|<M 对应的模型名>`；
   - **把第 2 步的同一条命令原样再运行一遍**（同一个 `$ID`）：应当原样返回第一次的结果，窗口里**不出现第二个聊天**；
   - `git -C <P> worktree list` 与第 1 步一致（没有新建 worktree 或分支）；
   - 20 秒内 AskHuman 没有弹「Max 被打开」的告警卡；
   - 约 10 分钟后 Cursor 仍正常（适配器持有引用的副作用）；
   - 在 Cursor 网页用量里确认这一次按普通请求计，不是 API 费率。
4. 回复 Agent：每一项通过／不通过，加上第 2 步的完整输出。

**阶段 4（可选）：经 CLI 创建一次（1 个普通请求）**

1. 记下 CLI 配置里 `maxMode` 的当前值，应为 `False`（Python 的写法；配置在 `~/.cursor/cli-config.json`，设了 `CURSOR_CONFIG_DIR` 或
   `XDG_CONFIG_HOME` 时在那里）：

   ```bash
   python3 -c "import json,os; print(json.load(open(os.path.expanduser('~/.cursor/cli-config.json'))).get('maxMode'))"
   ```

2. 在 AskHuman 的新建任务表单里：Agent 选 Cursor、项目 `<P>`、模型 `<M>`、「允许 Max」保持关着、权限默认，提示词「只回复 OK，不要使用任何工具」，
   运行位置选终端（CLI），点开始。
3. 核对：任务在终端里正常回答；再运行第 1 步的命令，`maxMode` 仍是 `False`；45 秒内 AskHuman 没有弹「Max 被打开」的告警卡。
4. 回复 Agent：每一项通过／不通过。

验证项目：用户在 Agents 窗口里指定一个现有的小项目，或由 Agent 建一个空的 git 仓库（零请求）交给用户加进 Agents 窗口。
小模型由用户在阶段 2 通过之后选（缓存里没有任何 Max 变体的候选：GPT-5 Mini、Gemini 2.5 Flash、Claude Haiku 4.5、GPT-5.4 Nano 等）。

**费用上界**：阶段 3、4 各最多一个普通（非 Max）请求；阶段 2 在私有闸门的行为与读到的代码不一致时最多发出一句固定占位文字，
同样是一个普通请求，并且这种情况下**不再进入阶段 3**。

## 验证记录

2026-11-03～04，只在 worktree 与本机：没有启动真实的 Cursor 任务，没有向任何模型发送提示词；适配器只对着合成的服务与会话对象跑。

- **阶段 1（2026-11-04，Agent，日常版 `7e07a728`，0 请求）通过。** 做法：AskHuman 的窗口属于后台应用，Computer Use 的应用列表里没有它，按进程号解析成应用目标，
  只做「按下控件／滚动」，不发键盘事件（⌘↵ 会提交），不操作 Cursor，从不按「启动任务」。结果：设置页「Cursor 任务的 Max 模式」检测结果＝旧计划（按次计费）；
  新建表单（待办的「创建 Agent 任务」进入，Agent 选 Cursor）里，沿用默认＝「Grok 4.7 256K High Fast · 需要 Max」被拦（选了权限模式「Agent 默认」后「启动任务」仍 disabled）、
  勾「允许 Max」后拦截消失、换 Composer 2.5 后读作「不需要 Max」且拦截消失；模型下拉 24 个（Cursor CLI 暴露的部分，缓存共 47 个）：12 个不需要、12 个需要，
  与 Cursor 本地缓存逐个核对（14 个按显示名精确匹配、10 个按模型族匹配）0 个不符；关窗后没有启动记录与回执、没有新的 `cursor-agent` 进程；
  `agentTasks.cursorMax` 与 `launchPreference` 都没写（取默认：强制关闭＋自动检测、终端）。手机端的开关与标记留给 TestFlight 包。
- **阶段 2 第一次尝试（2026-11-04，Agent，日常版 `7e07a728`，0 请求）：前置条件不满足，在查项目这一步被拒，闸门本身还没验到。** 验证项目
  `~/Developer/AskHumanCursorVerify`（空 git 仓库，一个 README 提交）已建；`open -g -a Cursor <P>` 在 Cursor 里打开一次，Cursor 把它写进工作区记录
  （`workspaceMetadata.entries` 9→10，不论窗口是否还开着都留着），随后用 System Events 按窗口名和 `AXCloseButton` 只关了那个新窗口（没碰 Agents 窗口；
  状态库里多出一条没有消息、没有标题的空聊天，时间与开窗吻合，应是 Cursor 为新窗口自动建的，没有删）。接着跑 `dryRun`（`gpt-5.4-nano`、不允许 Max）：
  `{"Err":"Agents 窗口里没有这个项目。请先在 Cursor 的 Agents 窗口添加该项目，或把运行位置改为 CLI。（9 known projects）"}`，回执 `rejected`／`projectNotFound`，
  状态库里没有该 ID 的聊天，没有任何模型请求。看起来 Agents 窗口的项目列表是它自己装入内存的，别的窗口新开的文件夹不会实时进来；
  要让它认识验证项目，得在 Agents 窗口自己的界面里添加，或让它重载（重启 Cursor）。这一次之后没有重试。

- JS：`scripts/cursor-create.node-test.mjs` 19/19 通过（成功只提交一次、以现有默认配置为底、`yolo` 覆盖与读回、每个读回不符都在提交前停下并归档、
  `dryRun` 通过与被拒、结构预检、项目歧义与别名、非法操作、提交前后出错的区别、过期不提交、同 ID 加入／冲突／拒绝后重来、`pending`、
  引用保持、Agents 窗口的唯一性与 active 窗口）；原有 `cursor-message.node-test.mjs` 不变。
- Rust：`cargo test` 2409 通过、0 失败，其中 `cursor_desktop::create` 覆盖模型与 Max 计划、操作校验、请求形状、`pending` 续问、
  拒绝与 `dryRun` 为终态、「没发出去」与「回答丢了」的区别、回执（重试按回执返回、同 ID 不同内容冲突、崩溃读作未知、`dryRun` 不盯）、
  不可用原因；clippy 与 `cargo fmt --check` 无告警。
- Dev Instance（装在 worktree 的 `.askhuman-dev/`）：`AskHuman __cursor-desktop` 的 `dryRun` 请求回
  `{"Err":"A Dev Instance does not use Cursor Desktop"}`（按设计）；daemon 握手 `version 0.16.0 (protocol 5)`。
- **未验证**（阶段 2 起；阶段 2 由 Agent、阶段 3–4 由用户做）：规格 §9 的全部不确定点；IM、iOS 在真实 Cursor 下的状态与结果（GUI 表单与设置页的 Max 部分已由阶段 1 验过）。
