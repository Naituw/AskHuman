# Cursor 任务的 Max 模式控制

> 状态：2026-11-03 用户确认规格并要求实施。实施计划：[cursor-max-control](../plans/cursor-max-control.md)。
> 关联：[Desktop（Agents 窗口）新建任务](cursor-desktop-task-creation.md)；交互参照 [Fast 模式](ios-task-speed.md)，
> 模型清单见 [模型与强度](ios-task-model.md)。

## 1. 背景与目标

旧「按次计费」计划里，Cursor 的 Max 模式按 API 费率扣额度；新计划没有 Max。AskHuman 新建 Cursor 任务时没有任何
Max 控制，旧计划用户因此不敢用。目标：

- 旧计划用户新建任务时，默认**不会在不知情下以 Max 启动**，需要时可明确放行。
- 新计划用户完全无感：不出现任何 Max 界面，也不碰 Cursor 的配置。
- 其他 Agent 不受影响。

**研发与验证的硬约束**：任何会触发计费的操作都不得执行——不运行 `cursor-agent` 任务，不驱动 Cursor 运行 Agent。
开发与测试只用桩可执行文件、临时配置与合成的模型目录。唯一例外是实现完成后由用户批准的「验证计划」
（见 §9）。

## 2. 已确认决策

| 编号 | 决策 |
| --- | --- |
| M1 | 每个任务有「允许 Max」开关，默认关、不保存（同 Fast 的交互）。开启＝沿用 Cursor 默认行为，AskHuman 不显式打开 Max |
| M2 | 开关关闭时强制关闭 Max；若将使用的模型按目录需要 Max，**拦截启动**并说明原因，不静默放行 |
| M3 | 无法判断（无 Desktop 缓存、格式变化、目录里找不到该模型）：照常启动并明确提示「未能判断是否需要 Max」，仍强制关闭全局 Max，启动后复查 |
| M4 | 设置可把默认策略改为「沿用 Cursor 当前设置」：不干预、不拦截、不提示，开关隐藏 |
| M5 | 计划类型：旧计划＝完整功能；新计划＝隐藏全部 Max 界面且不干预；检测不出按旧计划处理；设置可手动覆盖 |
| M6 | 提示：GUI 新建任务表单显示「将使用：模型 · 需要 Max／不需要／未知」；iOS 模型选择器显示 Max 标记；Desktop 会话信息只读显示 Max（开启时红色） |
| M7 | GUI 新建任务表单为 Cursor 增加模型选择器（带 Max 标记，默认「沿用 Cursor 默认」），复用 iOS 已有的模型清单与 `--model` 通路 |
| M8 | 启动后复查与告警覆盖 GUI、IM、iOS 三个入口，告警走 AskHuman 自己的提问卡，不另造通道 |

## 3. 判定

**计划类型** `Legacy / TokenBased / Unknown`：读 Cursor Desktop 本地缓存里的 `hasTokenBasedPricing`
（`true`＝新计划，`false`＝旧计划，缺失＝Unknown，按旧计划处理）。设置「计划类型」可选
自动检测（默认）／旧计划／新计划。每次启动任务、打开表单时现读，AskHuman 自己不缓存结论。

**是否需要 Max** `Required / NotRequired / Unknown`：与 Cursor 自身一致，
`model.supportsNonMaxMode === false || variant.isMaxMode === true`。Max 是**变体级**属性（主要随上下文长度），
同一模型的不同变体结论可以不同。

- 数据源：同一份缓存里的 `availableDefaultModels2[]`（模型名、`supportsNonMaxMode`、`variants[]` 的
  `isMaxMode`／`legacySlug`／`variantStringRepresentation`／`parameterValues`）。只读打开，不写。
- 将使用的模型：iOS 显式选择的模型 id；否则 CLI 取 `cli-config.json` 的 `selectedModel`＋`modelParameters`，
  Desktop 取 IDE 当前 composer 的模型配置（见 Desktop 规格）。
- 变体解析与 Cursor 一致：id 先按 `legacySlug`／`variantStringRepresentation` 命中（第一个），已有参数时按
  参数值匹配，多个候选取非 Max 默认配置；解析不出任何唯一变体＝Unknown。
- 含义已对照 Cursor 自身代码确认：IDE 的模型选择器选中 `isMaxMode` 变体时会自动打开 Max，求解器在非 Max 下对这类
  变体判为无效选择；CLI 同理，且 Max 只在内存里推导。所以全部变体都标 `isMaxMode` 的模型就是「仅 Max」，即使它的
  `supportsNonMaxMode` 为真、也带 `isDefaultNonMaxConfig`（那只是选中该模型时的默认变体）。
- 缓存只是**建议性证据**：它不是 CLI 的运行时状态，可能比服务端目录旧。结论为 Required 时的拦截说明里写明依据，
  用户可用「允许 Max」放行。

## 4. 执行（CLI 路径）

三个入口（GUI 新建任务、IM `/new`、iOS `task.create`）汇到同一个创建点
`agent_launch::create_record_internal`，Max 控制在此统一执行，入口只负责传入「允许 Max」。

1. **创建时判定**：Cursor、计划≠新计划、策略为强制关闭、允许 Max 关闭时，对将使用的模型求值。Required →
   返回 `maxRequired`（GUI／IM 显示文字，iOS 用原因码）。Unknown → 放行，入口展示 M3 提示。Fork 等延续既有会话的
   启动沿用会话自己的模型，这里无从得知：仍强制关闭 Max，但**从不**因「没有选过的模型」而拦截。
2. **一次性记录**新增可选字段 `allowMax`（缺省＝旧记录，不干预；`false`＝强制关闭）。仅在设置时纳入
   `payload_sha256`，其余记录哈希不变。
3. **Helper 启动前**（`run_helper`，`exec` 之前）：先用新鲜数据再判一次（Required 则拒绝启动并在终端说明，作为
   最后一道闸）；再强制关闭 CLI 配置——对 `<配置目录>/cli-config.json` 做**最小文本替换**：仅当顶层 `maxMode`
   或 `model.maxMode` 为 `true` 时改为 `false`，其余字节（键序、缩进）保持不变；原子写入、保留权限；文件不存在或
   无法解析时不创建、不修改。配置目录与 CLI 一致：`CURSOR_CONFIG_DIR` → `XDG_CONFIG_HOME/cursor` → `~/.cursor`。
   这是**全局粘性**配置：之后你手动运行 CLI 也保持关闭，直到你再用 `/max-mode` 打开。
4. **能力边界（必须说清）**：默认模型若本身需要 Max，CLI 只在内存里按所选变体推导，配置里看不到，改配置关不掉；
   这类情况只能靠第 1 步的预判拦截，而预判依赖 Desktop 缓存——无缓存时只能是 Unknown。
5. **启动后复查与告警**：启动方进程（GUI Host 或 daemon，谁打开的终端就是谁）在 `open_terminal` 成功后起一个有界任务，
   最多 45 秒、每秒重读一次 `cli-config.json`；发现 `maxMode:true` 而本次不允许 Max → 写运行日志，并经 `client::internal_ask`
   发一张只有「知道了」的普通提问卡（与失败确认卡同一机制，按现有渠道配置出现在弹窗／IM／iOS 收件箱）。卡片说明项目、
   检测到 Max 已开启，以及处理办法（在该会话里 `/max-mode` 关闭或结束会话）；它只告警，不终止任务。复查只覆盖
   「未能判断」的放行路径与竞态，拦截与强制关闭才是主要保护。

### 手动恢复 CLI 会话

macOS 交互终端通过 `AskHuman cursor` 或由它生成的快捷命令启动时，`--resume`、`--continue` 和 `/resume`
都走恢复保护。Cursor 会话保存上次使用的模型 ID，恢复时读取全局保存的该模型参数，可能把已经关闭的 Max 重新打开。
全局 Max 关闭时，保护在 Cursor 写配置前检查它选出的变体。若该变体需要 Max，先用 Cursor 的模型目录为同一模型选非 Max 变体，
无可用变体就提示并保留当前的非 Max 模型。如果当前模型也不能确认可在非 Max 下运行，则退出并说明原因。
已开启 Max 时沿用 Cursor 的恢复行为。直接指定模型和手动切换模型仍走原生选择逻辑。

启动时从已安装的发行文件识别启动恢复、交互恢复和模型选择接口。接口兼容即可启用，文件摘要用于核对检查与加载期间的内容。
接口缺失或核对失败时在受保护入口停止启动。后台唤醒和停滞保护各自沿用原有的验证和启用规则。

## 5. 各入口

- **GUI 新建任务表单**：选择 Cursor 且计划≠新计划且策略为强制关闭时，显示模型选择器、一行「将使用：…」和「允许 Max」
  开关（默认关，每次打开重置）。模型选择器列出 Cursor 的模型（与 iOS 同一份清单），每项带「需要 Max／不需要」标记，默认
  「沿用 Cursor 默认」，其「将使用」取 Cursor 的当前默认模型；选择后经 `--model` 传入。若所用模型需要 Max 且开关关闭，行内红字
  说明并禁止启动，出路是换模型或打开开关。Unknown 显示灰字提示但可启动。2026-11-06 桌面与 iOS 对齐后，新计划与「沿用」策略下只隐藏 Max 开关与标记，
  模型选择仍保留，见 [GUI §8](gui-agent-task-launch.md#8-桌面新建任务设置与-ios-对齐2026-11-06)。
- **IM `/new`**：没有开关，按默认策略执行。被拦截时回复原因与出路（改用 GUI／手机、或设置里改为「沿用」）。
- **iOS**：`task.options.agents[]` 增加 `maxControl: true`（Mac 具备控制能力且计划≠新计划且策略为强制关闭时才给）；
  `models[]` 增加 `maxRequired: Bool?`（缺省＝未知）；`task.create` 增加 `allowMax: Bool`。缺省沿用旧行为：旧版手机不带
  该字段时 Mac 按「不允许」处理。新手机遇到没有 `maxControl` 的 Mac 隐藏开关与标记。仅增加可选字段，不新增消息变体，
  iOS 线协议版本和本机 IPC 协议号不变。

## 6. 设置

设置页 Agent 任务区增加 Cursor 小节：Max 策略（强制关闭／沿用 Cursor 当前设置）、计划类型（自动检测／旧计划／新计划）
并只读显示当前检测结果。配置：`agentTasks.cursorMax.{policy, plan}`，取值为 kebab-case：`policy` 为 `enforce-off`（缺省）
或 `inherit`，`plan` 为 `auto`（缺省）、`legacy` 或 `token-based`；旧配置缺字段时取缺省，不改写已有项。没有 Desktop 数据的
纯 CLI 用户若是新计划，应在此选「新计划」以隐藏提示。

## 7. Desktop 会话 Max 只读显示

会话信息读取 `composerData:<id>` 的 `modelConfig.maxMode`；为真时在模型名旁显示「Max」并标红，否则不显示。
这只描述已存在的会话，不改变任何设置。CLI 会话的元数据保存上次使用的模型 ID，未单独保存可供该界面显示的 Max 状态。

## 8. 不做

- 不读取任何凭据：`cli-config.json` 里的 `authInfo` 与 IDE 存储里的 `cursorAuth/*` 不读、不记录、不写日志。
- 不改 Cursor 的计划、套餐或 Desktop 的全局默认模型配置。
- 不为 Max 另建常驻任务；复查只是启动方进程里一个最长 45 秒的有界任务。

## 9. 验收

- 默认关闭；关闭时强制关闭配置；需要 Max 的模型被拦截且给出原因；Unknown 放行并带提示；「沿用」策略无任何干预。
- 新计划与「新计划」覆盖下：无 Max 表单行与标记、无 `maxControl`、无 Max 配置改动；模型选择仍可用。
- GUI 选择器：Max 标记与清单一致；选择具体模型经 `--model` 启动；需要 Max 且开关关闭时禁止启动；清单取不到时退回「沿用默认」。
- 复查：配置在 45 秒内变为 `maxMode:true` 且不允许 Max 时，GUI、IM、iOS 各入口都发出告警卡；不变则无卡；超时即停。
- `cli-config.json` 最小替换：键序与缩进不变；只有当前为 `true` 才写；无法解析或不存在时不动；并发 helper 互斥。
- 合成目录夹具覆盖：全 Max 模型、混合模型（非 Max 小上下文＋Max 大上下文）、无 Max 变体模型、同名歧义 id、参数不全、
  缓存缺失／损坏。Rust 全部使用桩可执行文件、临时配置目录与合成 JSON，**不运行真实 Cursor／cursor-agent**。
- 旧 `LaunchRecord`、旧哈希、旧 iOS 消息保持兼容；Rust／Swift 共用夹具覆盖新字段。
- 修改后安装并在 Dev Instance 验证非计费部分；真实调用只在 §1 所述的验证计划获批后执行一次。
