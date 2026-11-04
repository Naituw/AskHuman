# 在 Cursor Desktop（Agents 窗口）新建任务

> 状态：2026-11-03 用户确认规格并要求实施。实施计划：[cursor-desktop-task-creation](../plans/cursor-desktop-task-creation.md)。
> 关联：[Max 模式控制](cursor-max-control.md)（计划检测、模型目录、「允许 Max」均复用）；
> 参照 [Codex Desktop 会话集成](codex-desktop-session-integration.md) 的创建流程，
> 复用 [Cursor Desktop 桥](cursor-desktop-background-followup.md) 的挂接与安全约束。

## 1. 目标与边界

新建 Cursor 任务时，除现有终端 CLI 外，可选在 **Cursor Desktop 的 Agents 窗口**里创建会话并自动发送，
会话带显式的模型配置，遵守「允许 Max」。

- **实验性**：依赖 Cursor 的私有内部接口，经 AskHuman 已有的 Desktop 桥访问；默认关闭，仅 macOS，Dev Instance 不启用。
- 只做 Agents 窗口、只做**本地**项目；不创建云端 Agent；不改 Cursor 的全局默认模型；经典 IDE 窗口本版不支持。
- 研发硬约束同 Max 规格：不得为研发或测试驱动真实 Cursor 运行 Agent。适配器只用合成对象图测试；
  真实验证只在实现完成、你批准验证计划后进行。

## 2. 已确认决策

| 编号 | 决策 |
| --- | --- |
| D1 | 与 Max 控制一并实施，含创建后自动发送 |
| D2 | 面向 Agents 窗口 |
| D3 | 实现完成后先给你验证计划，批准才做真实验证；若一次能覆盖，接受一次完成 |
| D4 | GUI 进程经 IPC 请求 daemon：新增 `ClientMsg::CursorDesktop`，`PROTOCOL_VERSION` 4→5（与 Codex 同构） |
| D5 | `cursorDesktop.launchPreference` 缺省为 `terminal`，真实验证通过后由你在设置里改为 `desktop` |

## 3. 可用性与目标选择

- 设置 `cursorDesktop.launchPreference`：`terminal`（默认）／`desktop`。与 Codex 的目标选择同构：偏好 Desktop 且可用→
  Desktop；偏好 Desktop 但整体不可用→终端，状态文案说明原因（「新任务将使用 CLI（未检测到运行中的 Cursor）」）；
  偏好终端→终端；终端本身不可用时，状态行如实说明，不会改用 Agents 窗口（与 Codex 不同：Agents 窗口是实验路线，只在被选中时使用）。
  **任务一旦开始，不得静默改走另一条路径。**
- **整体可用**只做廉价的静态检测（表单状态行用它，不挂接 Cursor）：macOS；Cursor 集成模式不是 None 且生命周期追踪
  已安装；恰有一个原始 Cursor 进程；其 Inspector 开关可用（沿用既有检测，不新增对 Cursor 的写入）。
- **逐任务条件**要挂接桥才能知道，所以到创建时才检查，不满足一律直接报错、不自动改走终端：恰有一个已打开的
  Agents 窗口（「请先打开 Cursor 的 Agents 窗口」）；所选工作区的规范路径等于该窗口里某个未归档本地项目的目录
  （「Agents 窗口里没有这个项目，请先在 Cursor 里添加，或把偏好改为 CLI」）。
- GUI 表单、IM、iOS 都按同一目标选择；表单用与 Codex 相同的状态行。同一次任务重试复用操作 ID，
  不会重复创建。

## 4. 创建流程

走 Agents 窗口自己的创建入口 `agentRepositoryService.createAgent`——与用户在窗口里点「发送」是同一条路，项目归属、
会话落库、与 hook 的衔接都由 Cursor 自己完成。这个入口带一个**提交前闸门**（`beforeInitialSubmit`）：闸门抛错时，
Cursor 在发出任何模型请求之前清理该会话。AskHuman 把全部核对放进闸门，所以**只有每项核对都通过，才会产生模型请求**；
任何一步失败即停止。

> 与最初规格的差异：原案是「创建草稿 → 读回 → 再走聊天提交入口」两步；实施改为 `createAgent` 加闸门，因为它把创建与
> 提交收在同一个入口里，核对必然发生在模型请求之前，也省去「草稿转正式会话」这一未验证的二次入口。

1. **窗口与项目**：桥的 `create` 操作先在 active 窗口里找 Agents 窗口（`_environmentService.isGlass`），必须恰好一个
   （`noAgentsWindow`／`multipleAgentsWindows`）；所选工作区的规范路径必须对应窗口里恰好一个未归档的本地项目
   （`projectNotFound`／`projectAmbiguous`）。
2. **结构预检**：适配器对所需服务与方法逐项做类型检查（沿用桥按服务 ID 在注入链上查找的方式）；缺失或类型不符 →
   `nativeUnavailable`，不创建任何东西。
3. **创建**：`createAgent(任务文本, {existing, environment}, options)`。`options` 含预分配的会话 ID（`composerId` ＝ 操作 ID）、
   显式 `modelConfig`（§5）、`unifiedMode: "agent"`、`destination: local`、`trustPolicy: "require-trusted"`、
   `skipFocusAfterSubmission`、中止信号、`promptPreview`（取自标题）；`yolo` 时再加 `agentApprovalModeOverride: "unrestricted"`（§6）。
4. **提交前闸门**：读回该会话——ID、目标为本地且指向所选项目、模式、模型名与参数、`maxMode`、权限覆盖——逐项比较，
   任一不符即抛错，Cursor 丢弃会话、不向模型发送任何内容，结果为 `rejected`（带原因，如 `modelMismatch`、`maxModeMismatch`）。
   `dryRun` 在闸门通过之后同样抛错（哨兵），结果为 `dryRun`，**不产生模型请求**，用于不计费的验证。
   `dryRun` 的提示词由 daemon 换成固定的占位文字，任务内容不进入 Cursor；即使 Cursor 将来忽略闸门，发出的也只是这句占位。
5. **清理**：被丢弃的会话尽力做一次软归档（`archiveAgent`，约 1.5 秒内）；归档不是删除，失败只在结果里记 `tidied: false`。
6. **等待与续问**：适配器约 2 秒后若尚无结果就答 `pending`；daemon 以同一操作 ID 每 250 毫秒再问（同一 ID 加入同一次创建，
   不会再建一个），直到最终结果或 45 秒窗口结束。适配器自己带 40 秒的 `deadline`，逾期不再提交，所以 daemon 放弃等待后
   不会有迟到的提交。创建成功后适配器持有会话引用 10 分钟，窗口重载或桥恢复时释放。
7. **回执**：状态目录 `cursor-desktop-actions/<id>.json` 记录阶段 `prepared`（碰桥之前）→ `creating`（请求已写出）→
   `accepted`／`dryRun`／`rejected`／`unknown`。只有 `prepared` 与 `rejected` 允许同一操作 ID 重来；`creating` 与 `unknown`
   表示 Cursor 可能已经创建，一律不自动重试，提示用户去 Cursor 里查看。同一 ID 的重试按回执原路返回，
   不重新提交；表单、IM、iOS 重试复用操作 ID。成功后返回会话 ID，IM／iOS 以它关注新会话。
8. **提交后复查**：之后 20 秒内每秒只读读取该会话 `composerData` 的 `modelConfig.maxMode`；为真而本次要求关闭 Max 时（初始即为真
   则给 4 秒缓冲），经 `client::internal_ask` 发一张只有「知道了」的告警卡（机制同 [Max 规格](cursor-max-control.md) §4.5，
   文案指向该会话的模型选择器）。闸门通过后这应当不会发生，它是读回核对之外的独立兜底。

## 5. 模型与 Max

主进程用 [Max 规格](cursor-max-control.md) 的目录读取构造 `modelConfig`
（`modelName`、`maxMode`、`selectedModels[{modelId, parameters[]}]`）：

- 模型：iOS 显式选择的 id 经目录映射为（模型名，参数）；映射不到→拒绝。未选择时，以 IDE 当前 composer 模型配置为底。
- 允许 Max **关闭**：`maxMode:false`，且目录判定不需要 Max；Required→拦截（`maxRequired`，同 CLI 规则）。
  Unknown→仍以 `maxMode:false` 创建，由第 3 步读回确认 `maxMode===false`，否则拒绝；入口展示「未能判断」提示。
- 允许 Max **开启**：沿用 Cursor 自身的推导，AskHuman 不额外打开 Max；唯一例外是所选模型按目录**必须**用 Max 时，
  配置里写 `maxMode:true`，与 Cursor 自己的模型选择器对这类变体的写法一致，闸门同样读回核对。
- 因为每个会话显式携带自己的配置，**不会修改 Cursor 的全局默认模型**，也不依赖之前会话遗留的配置。

## 6. 权限

`agent-default`→不设覆盖；`yolo`→会话字段 `agentApprovalModeOverride: "unrestricted"`。读回不符
（例如被组织策略禁止「全部放行」）→ 拒绝并说明原因。纠正此前说法：Desktop 会话**可以**按任务设置权限。

## 7. 附件

与终端路径一致：按现有规则把附件路径与警告拼进任务文本，不走 Cursor 的原生附件。

## 8. 与既有桥的关系

- 扩展「消息桥」：独立状态键 `__ASKHUMAN_CURSOR_MESSAGES_V1__`、独立 manifest／socket／token，新增 `create` 操作；
  背景完成适配器及其修订保持不变。仅在显式创建时按需挂接，适配器版本变化时重新引导。
- 窗口选择：只在状态为 active 的窗口里取 `surface === "agents"` 者，必须恰好一个，否则拒绝。
- 桥与适配器仍由 daemon 持有；GUI 进程经 IPC 请求。新增 `ClientMsg::CursorDesktop` /
  `ServerMsg::CursorDesktop`（与 Codex 同构），按 overview 规则属于「新增枚举变体」，**`PROTOCOL_VERSION` 4→5**，
  并在 overview 历史与 `daemon-graceful-drain.md` 补记（D4）。不采用 GUI 进程直连桥，因为桥只由 daemon 持有。
- 会话库、hook 与 Cursor 现有会话信息读取无需改动；创建后的会话与普通 Cursor 会话一视同仁。
  `dryRun` 的会话在闸门里就被丢弃，没有模型请求，也就不会有 hook 事件；万一产生，按操作 ID 对应的回执状态忽略。

## 9. 实现依据与未验证项（如实记录）

调用序列来自对 Cursor 3.27.22 安装包 Agents 窗口代码（`workbench.glass.main.js`）的静态阅读，**尚未在真实 Cursor 里运行**：
`agentRepositoryService.createAgent(prompt, {type:"existing", environment:{id, uri}}, options)`，`options` 支持
`composerId`、`modelConfig`、`agentApprovalModeOverride`、`unifiedMode`、`destination`、`trustPolicy`、
`promptPreview`、`skipFocusAfterSubmission`、`startupAbortSignal`、`onCreated(ref)`（先于闸门触发）、
`beforeInitialSubmit(ref)`（抛错则 Cursor 清理会话、不发模型请求）。引用 `ref` 含 `header.id`、
`composerDataHandle.data`、`workspaceIdentifier`、`dispose()`。以下是已知不确定点，均由闸门而不是假设处理：

| 不确定点 | 处理 |
| --- | --- |
| `agentRepositoryService` 能否沿注入链取到、方法形状是否一致 | 结构预检；`dryRun` 验证 |
| `workspaceMetadataService.getAllMetadata()` 记录里项目的 id／目录字段 | 取自 Agents 窗口自己保存的项目记录，不自造；找不到或有歧义即拒绝 |
| 权限覆盖字段是否被原样保留 | 闸门读回核对，不符拒绝 |
| 闸门抛错后，没有会话头的 `dryRun` 会话是否被 Cursor 清干净、软归档能否收尾 | 归档尽力而为并如实记 `tidied`；真实验证里确认有无残留 |
| 提交是否会改变目标（例如建 worktree） | 闸门核对目标为本地项目；真实验证里确认提交之后目标不变 |
| 创建后适配器持有引用 10 分钟是否有副作用 | 仅持有不操作；窗口重载或桥恢复时释放；真实验证里观察 |

任何一项在运行时核对失败，结果都是**拒绝并说明**，不会降级成另一种创建方式。

## 10. 验收

- 默认不启用；未启用或不满足 §3 时，行为与现在完全一致（终端 CLI）。
- 结构预检：缺服务、类型不符、非 Agents 窗口、多个 Agents 窗口、项目不在 Agents 中，均在创建前拒绝，且无副作用。
- 闸门任一核对不符：不向模型发送任何内容、会话被 Cursor 丢弃；`dryRun` 不产生模型请求，并尽力软归档。
- 同一操作 ID 重试不重复提交；回执能区分「碰桥之前」「请求已写出」「已接受」「被拒绝」「结果未知」，后两种之外只有
  `prepared` 与 `rejected` 允许重来。
- 允许 Max 关闭时 `maxMode:false` 写入并被读回确认；Required 拦截、Unknown 提示与 CLI 规则一致。
- 适配器用合成对象图（假服务、假会话引用）覆盖：成功、`dryRun`、各核对项不符、窗口与项目选择、`pending` 与同一 ID 加入、
  重复调用；Rust 用桩桥与临时目录覆盖回执、状态、模型映射、选择与错误映射。**测试不启动真实 Cursor。**
- 真实验证按批准的验证计划执行：先 `dryRun`（不计费），通过后再做一次最小真实创建。
