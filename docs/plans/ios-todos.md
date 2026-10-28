# 计划：iOS App 管理待办

> 需求：`docs/specs/ios-todos.md`（决策编号 I1–I16 均指其 §3）。状态：2026-10-29 起草并开工；用户睡觉期间自主推进，**P0–P8 已全部完成并自测通过，等用户次日验收**（实施记录见文末）。
> 分支：`feat/ios-todos`（在 `HumanInLoop-todosync` 工作树里从 main 切出；主检出里另一方有未提交的 iOS 作答流程改动，互不碰）。下文各阶段的文件名是起草时的设想，**实际文件名以文末「实施记录」为准**。

## 阶段

| 阶段 | 内容 | 验证 |
|---|---|---|
| P0 | 需求与计划文档 | 提交 |
| P1 | Mac：线协议（`ios/wire.rs`）、夹具、`Caps.todos`；`ios/todos_view.rs`（项目合并 / 分段 / 同名区分 / id 哈希，纯函数） | `cargo test ios::` |
| P2 | Mac：`Host` 增加待办方法；网关 `gateway/todos.rs`（读、写、幂等、订阅 / 变更通知）；测试台与 `tests_todos.rs` | `cargo test ios::gateway` |
| P3 | Mac：daemon 的 `Host` 实现（`daemon/runtime/ios_todos.rs`，读写 `todos.json`、同步元数据、Agent 快照）；单测（临时目录） | `cargo test`、`cargo clippy -D warnings`、`cargo fmt --check` |
| P4 | Swift：`AHProtocol` 新消息 + 夹具测试；`PhoneEngine+Todos`；`TestMac+Todos` 与引擎测试 | `swift test --filter AHProtocolTests`、`AHCoreTests` |
| P5 | Swift：`RemoteBackend` 新方法、`MockBackend`、`TodosStore`（来源 / 接入 Mac / 乐观更新 / 同步状态）与单测 | `swift test` |
| P6 | App：「待办」标签与全部视图、本地化（en + zh-Hans）、mock 场景与 UI 测试 | `xcodebuild test`（模拟器）、截图自查 |
| P7 | 真实 daemon 端到端（本地 broker）+ 装机自查 | `AH_E2E_BIN=… swift test --filter TodoE2ETests`、`./scripts/install.sh` |
| P8 | 文档：`overview-ios.md`、`todo-sync.md` 两处措辞、`ios-channel.md` §6、wiki、`PROGRESS.md` | 提交 |

## P1 细节

**`ios/wire.rs`**
- 常量：`t::TODO_PROJECTS_GET` 等 8 个类型；`TODO_MAX_PENDING` / `TODO_MAX_HISTORY` / `TODO_MAX_PROJECTS` / `TODO_TEXT_MAX_CHARS`。
- `Caps.todos: bool`，`#[serde(default)]`（序列化总是带上，旧手机忽略）。
- 结构：`TodoProjectsGet`、`TodoProjects`、`TodoProjectRow`、`TodoSyncInfo`、`TodoSyncDevice`、`TodoListGet`、`TodoList`、`TodoItem`、`TodoDone`、`TodoAttHint`、`TodoOp`、`TodoAck`、`TodoWatch`、`TodoChanged`；全部进 `registry!`。
- 夹具：`ios/Fixtures/wire-v1/messages.json` 加每个类型的样本（含 `hello.ack` / `pair.accept` 的 `caps.todos`）。重编码必须逐字相同，所以样本只含序列化器会输出的字段。

**`ios/todos_view.rs`**（纯函数，无 IO）
- `project_id(key) -> String`。
- `label_projects(&[(key, name)], home) -> hints`：移植前端 `labelProjects`。
- `build_rows(input) -> Vec<TodoProjectRow>`：移植 `mergeProjects` + `buildSyncedSidebarModel` / 本机分段；`Input { candidates, sync_projects, home }`。
- 单测对着前端 `todoProjects.test.ts` 的用例逐条镜像（同名区分、合并两目录、仅云端、无待办折叠、无 remote 的排最后）。

## P2 细节

- `hostapi.rs`：`TodoProjectsData`、`TodoListData`（带 `list: wire::TodoList`）、`TodoOpOutcome`；`Host` 新方法（均带保守默认）：
  `async fn todo_projects(&self) -> Option<wire::TodoProjects>`、`async fn todo_list(&self, id: &str) -> wire::TodoList`、`async fn todo_apply(&self, op: &wire::TodoOp) -> TodoOpOutcome`、`fn todo_rev(&self) -> u64`、`fn todo_refresh(&self)`。`caps()` 实现里 `todos: true`。
- `gateway/todos.rs`：`on_todo_projects_get` / `on_todo_list_get` / `on_todo_op` / `on_todo_watch` / `tick_todos`；`TodoLive { watchers, rev, checked_at }` 放进 `Actor`；`dispatch` 里 4 个新分支。写操作走 `replay_or_start` + `finish_journal`（日志里存去掉 `list` 的 ack，发送时现取最新 `list`）。
- 订阅者只对**在线 + 前台 + 已 hello** 的手机发 `todo.changed`；`remove_phone`、presence 变离线时清理。
- 测试台：`TestHost` 里放一个内存待办存储（队列、历史、项目表），实现新的 `Host` 方法；`tests_todos.rs` 覆盖需求 §12.1。

## P3 细节

- `daemon/runtime/ios_todos.rs`：`TodoEnv`（`todos.json` 路径、锁路径、历史上限、同步视图 trait、workspace 列表、Agent 快照），生产实现取 `crate::todos` 默认路径与 `todo_sync::service`；测试注入临时路径与假的同步视图。`todo_project_candidates` 从 `todo.rs` 改成 `pub(super)` 复用。
- 读：`projects()` 在 `spawn_blocking` 里取数、`todos_view::build_rows`；`list(id)` 反查 key、`before_read`、读队列与历史、`project_details`；
- 写：`apply(op)` 按 §6.4 表调用 `todos` 的 `_at` 函数（便于临时目录测试）；文本校验在这里。
- `todo_rev()`：`std::fs::metadata(todos_file())` 的 mtime 纳秒与长度混合。

## P4 / P5 细节（Swift）

- `AHProtocol/Messages.swift`：镜像 P1 的类型；`Caps` 手写 `init(from:)`，`todos` 缺省 false；`Wire` 里加限额常量与类型常量。`MessageFixtureTests` 加样本解码重编码。
- `AHCore/PhoneEngine+Todos.swift`：三个 `ReplyMailbox` 字典（项目列表按 macId、列表按 `macId|id`、ack 按 `clientMsgId`）；`onTodo*` 派发；`failTodos(for:)` 并入 Mac 掉线处理；`wantedTodoWatches` 在每次 hello 后重订阅；`todo.changed` → `EngineEvent.todoChanged`。
- `RemoteBackend` 新增 `todoProjects` / `todoList` / `todoApply` / `todoWatch`；`MockBackend` 实现（演示数据：两台 Mac，一台开着 iCloud；可编程故障）。
- `AHCore/TodoModels.swift`：`TodoSource`、`TodoLoad<T>`、同步状态推导 `TodoSyncLine`；`TodosStore`（`AppStore.todos`）。
- 测试：`AHCoreTests/TodoEngineTests.swift`（`TestMac+Todos` 脚本化 Mac）、`AHAppTests/TodosStoreTests.swift`。

## P6 细节（App）

- `Views/Todos/`：`TodosScreen.swift`（`TodosScreen`、`TodoSourceMenu`、`TodosList`、`TodoSectionHeader`、`TodoProjectRowView`、`TodoSyncLineRow`、`TodoStaleNotice`、`TodosEmptyState`）、`TodoProjectScreen.swift`（待办队列、历史、`TodoAddBar`、提示胶囊）、`TodoEditSheet.swift`；文案推导放在 `Design/TodoTexts.swift`。
- `RootView`：新标签 `AppTab.todos`；`AppStore.tab`、`-AHTab todos`。
- 本地化：`ios/Tools/make-catalog.py` 流程（先 `xcodebuild build -derivedDataPath build/DerivedData`，再跑脚本；缺中文会失败）。
- UI 测试：`AskHumanUITests/AskHumanTodosUITests.swift`，跑在 `MockBackend` 上（`-AHMock YES -AHTab todos -AHScenario demo`）；en 与 zh-Hans 各跑一遍。

## P7 细节

- `AHE2ETests/TodoE2ETests.swift`：沿用 `E2ECase` 的一次性 Mac（独立 `ASKHUMAN_HOME` / `HOME`），向真实 daemon 发 `todo.*`；一次性 Mac 的 `todos.json` 在它的 `ASKHUMAN_HOME/state/` 下，测试直接读写它来验证（用 `AskHuman todo …` CLI 在一次性 Mac 的假 git 仓库里造数据与对账）。**不碰用户真实的 `~/.askhuman`。**
- 装机：`./scripts/install.sh`（工作树里装到 `.askhuman-dev/bin`，不覆盖 `~/.local/bin`）。工作树的 Dev Instance 没有用户的真实待办，所以「装机自查」改成**真实数据形状自查**：把真实 `todos.json` 的一份**拷贝**放进一次性 Mac 的 `state/`，让手机引擎对账（只打印条数、字节数，不打印名字、路径、正文），跑完删除拷贝与测试代码（结果见实施记录）。

## 风险

1. **项目列表移植漂移**：前端与 Rust 两份合并逻辑日后可能不一致。缓解：Rust 单测逐条镜像前端用例；`docs/overview-ios.md` 注明「改待办窗口侧栏模型要同步改 `ios/todos_view.rs`」。
2. **订阅的文件签名可能漏掉同一秒内的两次写**：手机重拉看到最终状态即可；不漏「最终状态」。
3. **journal 增长**：每次写入一条日志；沿用 5000 条 / 30 天上限，待办是人手操作，量级无忧。
4. **机器很忙**：Rust 全量编译与模拟器测试在这台机器上要几分钟到几十分钟；只在每个阶段结束时跑一遍完整验证，开发中跑定向测试。
5. **与主检出里的 iOS 作答流程改动合并**：我只新增文件并在 `AppStore` / `RootView` / `Backend` / `PhoneEngine` 里加少量行，冲突面小；`Localizable.xcstrings` 由脚本生成，冲突时重新生成。

## 不做

见需求 §1 非目标与 §11。

## 实施记录（2026-10-29，P0–P8 全部完成）

| 阶段 | 提交 | 结果 |
|---|---|---|
| P0 | `d29699a` 需求与计划 | — |
| P1–P3（Mac 端） | `41734c0` | 线协议与夹具、`ios/todos_view.rs`、网关 `gateway/todos.rs`、daemon 的 `daemon/runtime/ios_todos.rs`；`cargo test ios::` 252 条、`ios_todos` 25 条全绿，clippy `-D warnings`、`fmt --check` 无告警 |
| P4–P5（Swift 引擎与状态） | `85e929d` | `PhoneEngine+Todos.swift`、`TodoModels.swift`（纯函数）、`TodosStore.swift`、`MockBackend+Todos.swift` / `MockData+Todos.swift`、`TestMac+Todos.swift`；对应测试 `TodoEngineTests`、`TodoModelsTests`、`TodosStoreTests` |
| P6–P7（界面与端到端） | `71c58a5` 与其后的修复 | `Views/Todos/`、`Design/TodoTexts.swift`、`AskHumanTodosUITests.swift`（7 条，en + zh-Hans 都跑过）、`TodoE2ETests.swift`（5 条，真实 daemon + 本地 broker）；真实数据形状自查见需求 §12.5 |
| P8（文档） | 随上两项 | `overview-ios.md`、`todo-sync.md` 两处、`ios-channel.md` §6.1、`wiki/ios-setup*.md`、`PROGRESS.md` |

与计划不同的地方：
- 视图文件名与计划不同（见 P6 细节里已改正的名字）；UI 测试文件是 `AskHumanTodosUITests.swift`。
- `MockBackend` 多了一个场景 `outdated`（两台 Mac 都不认识待办），用来在 UI 与 `TodosStoreTests` 里验证「需要更新」的说明。
- 「装机自查」改成真实数据形状自查（P7 细节）。
- 验证中找到并修了一个缺陷（需求 §13 第 7 条）：待办页先于 Mac 列表上屏时，旧 Mac 会让页面一直转圈。
- 需求的偏差汇总见 `docs/specs/ios-todos.md` §13。
