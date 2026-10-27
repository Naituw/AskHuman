# 计划：iOS 新建任务时选择模型与强度

> 状态：**已批准，开工中**（2026-10-27 起草；同日用户经 AskHuman 批准，并对六条待确认点作了选择，结论已回写进需求 M14–M19 与本计划）。
> 需求与决策：`docs/specs/ios-task-model.md`（M1–M19）。
> 地图：`docs/overview-ios.md`。前置：`docs/plans/ios-follow-narrowing.md`（已实现；本计划沿用它的做法——先 Mac 后 App、App 在 `/tmp` 隔离副本里开发）。
> **硬约束（用户 2026-10-27）：严禁触发计费；Cursor 严禁实测**。所有阶段的验证都**不启动真实 Agent、不发提示词、不用真实凭据调模型**，
> **也不再调用 `cursor-agent` 的任何命令**（含 `--list-models`，Cursor 只靠合成夹具）。允许的只有：本地只读（读配置、`--help`）、`pi --list-models`（纯本地）、
> 以及**经用户批准**的 `grok models` 样本抓取（P2 开头，沙盒内、无凭据）。真实 Agent 的一轮每次执行前都要用户批准，且永远不含 Cursor（P4，可选）。

## 阶段

| 阶段 | 内容 | 主要文件 | 验证 |
|---|---|---|---|
| P1 | Mac：启动管线——`ModelChoice` 贯穿 `TaskSpec` → `LaunchRecord` → argv；Codex 桌面应用覆盖 | `agents/model_catalog.rs`（只放类型与净化）、`integrations/agent_launch.rs`、`daemon/runtime/task_launch.rs`、`codex_desktop/{mod,runtime}.rs` | Rust 单测、存根 Agent、clippy、fmt |
| P2 | Mac：清单模块、线协议、网关、`ios_tasks`、详情页「实际模型」 | `agents/model_catalog.rs`、`agents/session_info.rs`、`ios/wire.rs`、`ios/gateway/{hostapi,tasks,testkit}.rs`、`daemon/runtime/{ios_tasks,ios_agents}.rs`、夹具 | Rust 单测 + 网关测试台、clippy、fmt、装机 |
| P3 | iOS App：模型 / 强度两行、选择页、进度页与详情页显示、文案 | `ios/AskHuman/Views/Agents/…`、`AHProtocol`、`AHCore`、`AHMock`、`Localizable.xcstrings`、`ios/**/Tests` | `swift test`、`xcodebuild test`（模拟器，en + zh-Hans） |
| P4 | 文档回写、装机、（可选且需批准）真实 Agent 一轮 | 需求头部列出的文档 | 用户在场 |

- P1–P2 只动 Mac（Rust）：iOS 线协议版本、`PROTOCOL_VERSION`、中继、`AppConfig` 都**不变**，**不需要**动 IPC（不碰 `ipc/mod.rs`）；
  线上只多可选字段，旧 App 连新 Mac 照旧走「默认」。
- 建议顺序：P1 → P2 → 装机 → P3 → P4。P3 开工前先 `git status ios/` / `git log ios/`：另一个 iOS 会话（App 外观打磨）当前在主树里有未提交改动
  （`AgentsScreen`、`ListChrome`、`InboxScreen`、`RequestScreen`、`AppStore`、`MockBackend*`、`ProbeUITests`），所以 P3 照 `ios-follow-narrowing` 的做法，
  在 `/tmp` 的隔离副本里开发并验证，再作为补丁合进主树。
- 提交：每个阶段单独提交，Conventional Commits，iOS 相关的 `feat` 带 `Release-Note: skip`；`docs/PROGRESS.md` 是共享文件（另一会话有未提交改动），
  只提交我自己的那几段（以 HEAD 的内容为底、只叠加我的改动）。提交只在本地，不推送。

## P1 细节（启动管线；纯 Rust，不碰网络与 Agent）

1. **类型**（`agents/model_catalog.rs`，P1 先只放这些）：`ModelChoice { model: Option<String>, effort: Option<String> }`（`Default` = 不选，`is_default()`）；
   `valid_model_id` / `valid_effort`（需求「净化」里的两个正则）。`agents/mod.rs` 注册该模块。
2. **`LaunchRecord`**（`integrations/agent_launch.rs`）：新增 `model` / `effort`，`#[serde(default, skip_serializing_if = "Option::is_none")]`。
   - 新增 `create_record_with_choice(source, cwd, kind, permission, task, files, warnings, &ModelChoice)`；`create_record_with_files` 委托它（传默认值），
     所以 IM `/new`、Mac GUI、现有测试都不用改；`create_fork_record` 传默认值。`create_record_internal` 增加一个参数。
   - 创建时再校验一次：`valid_*`；**Cursor 不接受 `effort`**（M8）、Pi 的权限限制照旧。违反 → `Err`，不写记录；校验放在现有任务边界检查旁、
     就绪探测与写文件之前（与 `create_record_validates_task_boundaries_first` 同口径，测试里用不存在的工作区目录即可证明没有副作用）。
   - `payload_sha256(task, files, warnings, launch_mode, choice)`：`choice` 为默认时，序列化的元组与现在**完全相同**（旧哈希、旧记录照常通过）；
     否则在元组末尾追加 `(model, effort)`。`validate_claim` 同步，`legacy_payload_sha256` 兜底保留。
3. **argv**：`agent_args(kind, permission, mode, choice, prompt)`；新增纯函数 `model_flags(kind, choice) -> Vec<String>`，只在 `LaunchMode::New` 使用，
   放在 YOLO 开关之后、`--`（Codex）与提示词之前：
   `claude --model X --effort Y` ／ `codex --model X -c model_reasoning_effort="Y"` ／ `cursor-agent --model X` ／ `grok --model X --reasoning-effort Y` ／ `pi --model X --thinking Y`
   （五家都用长选项，本机 `--help` 已逐一核对过拼写）。
   `choice` 为默认时输出与现在逐字节相同（表驱动测试逐个 Agent × 权限 × 选择组合断言）。
   为了能不 `exec` 就断言最终命令：把 `run_helper` 里「认领 → 校验 → 组装 `Command`」抽成 `prepare_helper(args) -> Result<Command>`，`run_helper` 只剩平台相关的 `exec` / `status`。
4. **`TaskSpec.choice`**（`daemon/runtime/task_launch.rs`）：`start_task` 把它传给 `create_record_with_choice` 与桌面分支的 `Operation::Create { model, effort }`。
   IM `/new` 与 GUI 构造 `TaskSpec` 时用默认值（本期不改它们的行为）。
5. **Codex 桌面应用**（`codex_desktop/mod.rs`、`runtime.rs`）：`Operation::Create` 与 `ControlIntent::Create` 增加可选 `model` / `effort`（`#[serde(default)]`）。
   `resolve_model_settings` 增加覆盖参数：
   - 覆盖了模型 → 不能再走「配置齐全就直接返回」的捷径，要分页查 `model/list` 找到该模型（已有的 `seen` 防环逻辑保留）；找不到 → 报错。
   - 强度：覆盖值必须在该模型的 `supportedReasoningEfforts` 里（条目有声明时），否则报错；没有覆盖值时，配置的强度被该模型支持就用，不支持用 `defaultReasoningEffort`（M10）。
   - 只覆盖强度、不覆盖模型 → 模型沿用配置 / 默认，强度同样校验。
   - 没有任何覆盖 → 行为与现在**完全一致**；现有测试与 `codex_desktop/fixtures/model-settings-contract.json` 必须原样通过。
   - `thread/start` 与首个 turn 用同一对 `(model, effort)`——具体接线以 `control_request` 里现有的 `request["model"]` / `["effort"]` 为准，实现时先读再改。
6. **测试**（`integrations/agent_launch.rs` 与 `codex_desktop/runtime.rs` 的测试模块）：

| 用例 | 期望 |
|---|---|
| 默认选择 × 每个 Agent × `AgentDefault` / `Yolo` | argv 与改动前逐字节相同；记录的 `payload_sha256` 与旧算法相同 |
| 每个 Agent 带模型 / 带强度 / 都带 | argv 精确等于需求表；开关都在提示词之前；Codex 的 `--` 仍紧贴提示词 |
| Cursor 带强度、非法 id（以 `-` 开头、含空白 / 控制字符 / 超长 / `:`） | 创建记录失败，没有写出任何文件 |
| 篡改记录里的 `model` / `effort` 后认领 | `validate_claim` 报哈希不匹配 |
| 没有 `model` / `effort` 字段的旧记录 JSON | 能反序列化、能通过校验 |
| helper 路径：临时 `ASKHUMAN_HOME`（沿用 `paths.rs` 测试里的 `ENV_LOCK` 做法）+ 一个只存在、**从不被执行**的存根可执行文件；写记录 → 认领 → 校验 → 组装 `Command` | `Command` 的程序与参数符合预期；**不 `exec`、不启动任何真实 Agent** |
| 桌面解析：覆盖模型 + 配置强度被支持 / 不被支持 / 覆盖强度非法 / 模型不在清单 / 清单分页 / 无覆盖 | 分别得到：沿用配置强度 / 模型默认强度（M10）/ 报错 / 报错 / 翻页后找到 / 与现状一致 |

   P1 完成标准：`cargo test`、clippy（本机 1.94 与最新稳定版）、`cargo fmt --check` 全绿；不装机（P1 只是铺管线，对外行为没变）。

## P2 细节

0. **前置（动手写 Grok 解析器之前）：抓 `grok models` 的样本——要先问用户**（需求 M15 / 「仍待确认」1）。做法：经 AskHuman 说明命令、环境与清理步骤，用户批准后，
   在**空的临时 `HOME`（没有 Grok 配置与凭据）+ 独立的 `--leader-socket`、超时 10 秒**下执行一次，拿到 stdout / stderr 后杀掉可能拉起的 leader、删掉临时目录；
   再据此写**合成**夹具（仓库公开）。如果没有凭据时它只给出报错而没有清单格式，就把「真实格式」留给用户手机上的那一轮，Grok 解析器先按 `--help` 与常见表格 / JSON 两种做保守实现并标记
   `PROGRESS` 待验。与此同时问「仍待确认」2（Cursor 清单的运行时获取是否保留）。
1. **清单模块**（`agents/model_catalog.rs`）：
   - 类型：`ModelEntry { id, label, note, efforts, default_effort }`；`AgentCatalog { models（可见项）, all（含隐藏，只用来查「默认所指」的档位）, default: Option<DefaultInfo> }`。
   - **纯解析器**（输入字符串 / JSON，便于夹具测试）：`parse_cursor_list`（找 `Available models` 标题，行格式 `<id> - <名>`，去零宽字符与 `(default)`，忽略不符合格式的行，
     所以登录 shell 的 rc 噪声无害）、`parse_pi_list`（按表头列位置切，`thinking` 列为 `yes` 才带 `low / medium / high`）、
     `parse_codex_model_list`（`app-server` 的 `model/list` 各页的 `data[]`）、`parse_grok_models`（样本到手后写）、`claude_catalog(settings, help)`（固定别名 + 网关映射副标题 + 按需加入 `fable`）。
   - **Codex 清单**（M16）：在 `codex_desktop/runtime.rs` 新增 `list_models(executable, home) -> Result<Vec<Value>, String>`——与创建路径同一套握手
     （`initialize` → `initialized` → 分页 `model/list { includeHidden: true, limit: 100 }`），关 stdin 后等待退出、超时 kill；**不动 `create_or_finish`**，也不发 `thread/*`。
     来源可执行文件随启动目标：`launch_status().target` 为 Desktop → `inspect()` 得到的 `Installation { executable, home }`；Terminal → 就绪探测已解析的 `codex` + `CODEX_HOME`（沿用 `data_home` 的解析）。
   - **默认所指**：`codex_default`（`toml_edit` 读 `config.toml` 顶层 `model` / `model_reasoning_effort`）、`claude_default`、`cursor_default`（`cli-config.json` 的 `model.displayName`）、
     `grok_default`、`pi_default`。每个都只按键名取，失败返回 `None`。
   - **校验与归一化**：`resolve_choice(&AgentCatalog, kind, &ModelChoice) -> Result<ModelChoice, ChoiceError>`：模型必须在 `models` 里、强度必须在对应档位里、Cursor 不接受强度；
     在这里实现 M10（Codex / Grok 的「配置强度不被所选模型支持 → 填该模型默认强度」）。`ChoiceError::{Model, Effort}` 对应 `modelUnavailable` / `effortUnavailable`。
   - **获取与缓存**：`catalog(kind, source) -> AgentCatalog`。所有来源（Cursor、Pi、Grok 的命令，Codex 的 `app-server`）30 分钟，过期先返回旧值并在后台刷新（带「刷新中」标志防重入）；
     「默认所指」用到的设置文件按 mtime 重读。命令来源走 `agent_launch` 里新增的 `pub(crate) fn run_listing(executable, args, timeout, max_bytes) -> Option<String>`——
     从 `probe_help` 抽出公共部分（登录 shell、stdin 关闭、超时），**只取 stdout**，输出上限 512 KB，命令自身最多跑 8 秒（`task.options` 只等 4 秒，见第 4 点，
     其余时间让它在后台跑完填缓存）；Windows 与 `probe_help` 一样不经 shell。
   - 路径一律用现有的 `paths` / `CODEX_HOME` 解析助手，不新写一套。
2. **线协议**（`ios/wire.rs`）：`ModelOption { id, label, note?, efforts?, defaultEffort? }`、`DefaultModel { label?, efforts?, defaultEffort? }`；
   `AgentOption.models` / `.default_model`、`TaskCreate.model` / `.effort`，全部 `#[serde(default, skip_serializing_if = "Option::is_none")]`（`AgentOption` 仍可 `Eq`）。
3. **网关**（`ios/gateway/hostapi.rs`、`tasks.rs`、`testkit.rs`）：`TaskLaunch` 增加 `model` / `effort`；`on_task_create` 的内容哈希**仅在有值时**把它们算进去；
   `TestHost` 的 `choices` 能带 `models`、`create` 能记下收到的选择，并能模拟 `modelUnavailable` / `effortUnavailable`。
4. **`ios_tasks`**（`daemon/runtime/ios_tasks.rs`）：新增纯函数 `build_agent_options`（就绪信息 + 清单 → `AgentOption`），`choices()` 在 readiness 之后并行取各 Agent 的清单
   （`tokio::task::spawn_blocking` + 单个来源 4 秒超时，超时的来源在后台继续填缓存）；`create()` 在现有校验之后取清单（缓存没有或不含时同步刷新一次，最多 6 秒，失败沿用旧缓存）、
   调 `resolve_choice`，失败回 `modelUnavailable` / `effortUnavailable`，成功写进 `TaskSpec.choice`。逻辑放进纯函数，`choices()` / `create()` 保持薄。
5. **详情页「实际模型」**（`daemon/runtime/ios_agents.rs::watch_frame`、`agents/session_info.rs`）：`session_info` 新增 `resolve_cached(kind, sid)`（10 秒 TTL、至多 128 项，
   写法仿 `commands.rs::popup_agent_info` 里的小缓存）；`watch_frame` 在 `frame_detail_json` 的结果上追加可选 `model` / `reasoning`（有值才加；**不改** GUI 共用的
   `frame_detail_json` 本身，也不进 `signature`，模型变化随下一帧带出）。
6. **夹具**（全部合成，仓库公开）：`src-tauri/src/agents/model_catalog/fixtures/`——
   Cursor 列表（零宽字符、`(default)`、`(NO ZDR)`、CRLF、开头夹一行 rc 噪声、末尾 `Tip:`；**按我早先抓的结构合成，不再调用 `cursor-agent`**）、
   Pi 表（含 `yes / no`、`provider/id`、带 `:` 的 id）、Codex `model/list` 两页（含 `hidden`、`isDefault`、分页）、`grok models`（P2-0 拿到样本后合成）、
   Claude `settings.json`（**含合成的 `ANTHROPIC_AUTH_TOKEN`**）、Cursor `cli-config.json`（**含合成的 `authInfo`**）、两份 `config.toml`。
   `ios/Fixtures/wire-v1/messages.json` 增补用例：带 `models` 的 `task.options`、带 `model` / `effort` 的 `task.create`、老格式 `task.create`、带 `model` / `reasoning` 的 `watch.frame`
   （Rust 与 Swift 测试共用）。提交前对新增文件 `rg` 一遍内部网关 / 供应商名，确认没有。
7. **测试**：

| 用例 | 期望 |
|---|---|
| 各解析器吃夹具 | 条目、顺序、档位、默认档位、`hidden` 过滤、净化（零宽 / 非法 id 被丢）与预期一致 |
| 夹具里的密钥串（Claude `ANTHROPIC_AUTH_TOKEN`、Cursor `authInfo`） | **不出现**在清单、`Debug`、日志里（断言整段输出不含合成密钥） |
| Codex `list_models`：对一个**假的 `app-server`**（测试里写的小脚本，按 JSON 行回 `initialize` 与分页 `model/list`，从不联网）跑 | 握手顺序正确；翻页收齐；关 stdin 后进程退出；超时会 kill；脚本不回应时不挂死 |
| 解析器吃空串 / 乱码 / 缺字段 / 截断 JSON | 返回空清单，不 panic |
| 默认所指：配置缺失 / 键缺失 / 模型不在清单 | `None` 或退回，不报错 |
| `resolve_choice`：合法 / 模型不在清单 / 强度不在档位 / 默认模型 + 强度 / Cursor + 强度 / M10 两个场景 | 对应成功或 `Model` / `Effort` 错误；M10 补出模型默认强度 |
| 缓存：命中 / 文件 mtime 变化重读 / CLI 来源过期后台刷新只触发一次 / 刷新失败沿用旧值 | 与预期一致（时钟可注入） |
| 网关：`task.options` 带 / 不带 `models`；`task.create` 带 `model` / `effort` 传到主机；同 `clientMsgId` 换模型 → 冲突；老格式请求哈希不变；失败码原样回给手机 | 通过 |
| `watch_frame`：有 / 无 `model`、`reasoning`；10 秒内不重复读 | 通过 |
| 本机真实数据冒烟（`#[ignore]`，由用户手动 `cargo test -- --ignored`，或经批准后由我执行） | 只读本机配置、`pi --list-models`、各 Agent 的 `--help`；**只打印条目数**，不起任何 Agent，**不含 Cursor**，不含带真实凭据的 `app-server` 与 `grok models`（后两者留给用户手机上的那一轮） |

   P2 完成标准：`cargo test` 全量、clippy（1.94 与最新稳定版）、`cargo fmt --check` 全绿；变异检查（破坏哈希兼容、M10、净化、校验后，对应测试确实失败）；
   `./scripts/install.sh` 装机，`AskHuman channel ios status` 看 daemon 正常；此时旧版 App 连新 Mac 与今天完全一样。

## P3 细节（iOS App）

| 文件 | 改动 |
|---|---|
| `AHProtocol/Messages.swift` | `ModelOption`、`DefaultModel`；`AgentOption.models` / `.defaultModel`（可选）；`TaskCreate.model` / `.effort`（可选，`nil` 不编码） |
| `AHCore/TaskComposer.swift`（`final class`） | 新增 `modelId` / `effortId`；`selectAgent` 复位两者；`selectModel(_:)`（强度不被新模型支持则复位）；`modelChoices(in:)` / `effortChoices(in:)`（选了模型 → 该模型档位，「默认」→ `defaultModel.efforts`）；`draft(options:)` 带上 id 与显示名；`applyDefaults` **不**恢复模型 / 强度（M3） |
| `AHCore/AgentModels.swift` | `TaskDraft.model` / `.effort` / `.modelLabel` / `.effortLabel`；`WatchFrameData.model` / `.reasoning`（宽松解码） |
| `AHCore/PhoneEngine+Tasks.swift` | `createTask` 把选择放进 `TaskCreate`；失败原因码照旧透传 |
| `AHMock/MockData+Agents.swift`、`MockBackend+Agents.swift` | 五个 Agent 的 mock 清单（Claude 固定、Codex 带逐模型档位、Cursor 一长串、Grok 带档位、Pi 部分带 thinking）；旧 Mac 场景（启动参数 `-AHNoModels`，不带 `models`）；创建任务按所选回显；`watch` 帧带 `model` |
| `Views/Agents/NewTaskSheet.swift` | Agent 与权限之间加「模型」行（`NavigationLink` → 选择页）与「强度」行（菜单 `Picker`）；`accessibilityIdentifier`：`task-model`、`task-effort`；Face ID 流程不变 |
| `Views/Agents/ModelPickerScreen.swift`（新） | 「默认」行 + 列表；超过 12 项 `.searchable`；选完返回 |
| `Views/Agents/TaskProgressScreen.swift` | 一行所选模型 / 强度；`modelUnavailable` / `effortUnavailable` 文案 |
| `Views/Agents/AgentDetailScreen.swift` | 状态区加一行只读「模型：x · 强度」（有 `model` 才显示） |
| `Localizable.xcstrings` | 构建后经 `ios/Tools/make-catalog.py` 重新生成并补中文（缺中文会失败）；确认无缺译、无多余键、再生成无差异 |

测试：

| 层 | 用例 |
|---|---|
| `TaskComposerTests`（AHApp） | 默认不选；切 Agent 复位；切模型保留 / 复位强度；「默认」+ 强度取 `defaultModel.efforts`；无 `models` 时两行不可用；`draft` 带显示名 |
| `AgentTests` + `TestMac+Agents`（AHCore） | `task.create` 线格式带 `model` / `effort`、老格式不带；`task.options` 无 `models` 时旧行为；失败原因透传 |
| `MessageFixtureTests` | 共享夹具的新用例解码 / 编码往返 |
| `AgentE2ETests`（真实 daemon，任务开关保持关） | 只确认 `task.options` 的 `blocked` 路径与既有一致（**不起终端、不起 Agent**）；真实清单由 P2 的 Rust 测试覆盖 |
| `AskHumanUITests`（mock，en + zh-Hans） | 选模型 + 强度并提交；搜索 Cursor 长表；切 Agent 复位；强度随模型变化；旧 Mac 场景两行隐藏；进度页显示所选；详情页显示模型 |

完成标准：`swift test`（AHCore / AHApp / AHProtocol）、真实 daemon 端到端既有用例不退化、模拟器 UI 测试 en 与 zh-Hans 全量。

## P4

- 回写文档：`docs/specs/ios-channel.md`（§6.8 新建任务、决策表加一条、局限里的「不能选模型」）、`docs/overview-ios.md`（线消息与模块说明各加一行）、
  `docs/wiki/ios-setup.md` 与 `.en.md`（「新建任务」一段）；`docs/overview.md` 只在其模块清单列了 `agents/` 下的文件时才加 `model_catalog.rs` 一行，否则不动。
- `./scripts/install.sh` 装机；更新 `docs/PROGRESS.md`，已完成项删除。
- **真机一轮（M19：实现完做，但每次执行前经用户批准；会产生模型费用；Cursor 永远不在其中）**：手机新建任务，Claude / Codex / Grok / Pi 各选最便宜的模型、一句话任务；
  核对终端里 Agent 的启动参数（`ps` 看 argv）与详情页显示的模型 / 强度一致；每家再试一次「默认」确认参数与以前相同。
  手机打开新任务表单时，daemon 会对 Cursor 调一次 `cursor-agent --list-models`（只读、不起会话；是否保留见需求「仍待确认」2），这是功能本身，不算对 Cursor 的实测。
  同时借这一轮验证 `app-server model/list` 与 `grok models` 在真实账号下的输出（用户自己的设备、自己的凭据，我不在本机替他跑）。

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| Cursor / Pi / Grok 的列表输出与 Codex `app-server` 接口是私有格式，升级会变 | 解析容错 + 合成夹具测试 + 失败降级为只有「默认」；`#[ignore]` 冒烟测试能在本机快速发现 |
| `grok models` 还没有样本，可能拉起 Grok 的 leader 进程 | P2-0 先问用户；沙盒 HOME + 独立 leader socket + 无凭据；跑完杀进程删目录；样本不够就保守实现并在 PROGRESS 标待验 |
| `app-server` 是实验性接口，起进程有成本 | 30 分钟缓存 + 后台刷新；只发 `initialize` 与 `model/list`、不建线程；超时 kill；失败降级 |
| Cursor 首次获取约 2 秒；登录 shell 的 rc 慢或有噪声 | 并行、单来源 4 秒超时、后台继续填缓存；解析器只认符合行格式的行 |
| 本地配置文件里有密钥（Claude `ANTHROPIC_AUTH_TOKEN`、Cursor `authInfo`） | 只按键名取字段、不遍历 `env`、不 `Debug` 整个对象；合成夹具带假密钥并断言不泄漏；Grok / Codex 的缓存文件不读 |
| 夹具 / 文档泄漏内部网关或供应商名（仓库公开） | 全部合成；提交前 `rg` 检查新增文件 |
| 手机选了 Agent 当前不接受的模型 / 强度 | Mac 在 `task.create` 重新校验，拒绝并回失败码；Codex（及 Grok，若 `grok models` 带逐模型档位）的强度不兼容由 M10 兜底 |
| 另一个 iOS 会话改同一批 View 文件 | P3 在 `/tmp` 隔离副本里做，验证后作为补丁合入；开工前再看一次 `git status ios/` |
| 误触发计费 | 验证只用存根 Agent、夹具与只读命令；本机不再调用 `cursor-agent`；`grok models` 与真实一轮每次执行前必须经用户批准，真实一轮永远不含 Cursor |

## 不做

- 不改 iOS 线协议版本、`PROTOCOL_VERSION`、中继、`AppConfig`、Mac IPC；不改 IM `/new` 与 Mac GUI 新任务窗口。
- 不用 Cursor 的参数化写法；不记住上次选择；不做价格 / 倍率提示；续聊 / 分叉不带模型。
