# 跨设备待办同步概览

> 本文是 `docs/overview.md` 的专题补充，记录 iCloud 待办同步（实验功能，仅 macOS）当前的实现地图与不变量；需求决策见 `docs/specs/todo-sync.md`，实现步骤与实施记录见 `docs/plans/todo-sync.md`，界面稿在 `docs/assets/todo-sync-ui/`。

## 形态

```
todos.json ◄── 读写 ──► CLI / 弹窗 / 待办窗口 / IM / MCP       （本机的唯一数据源，格式不变）
    │  ▲
    │  │ 本地阶段：diff → 实体 → 物化              state/todo-sync/state.json（旁路：实体、快照、链接表、令牌）
    ▼  │
  todo_sync::tick ── Transport ──► helper_client ── unix 套接字（一行一个 JSON）──► CloudKit helper（.app）──► iCloud
                                                                                       │ 推送 / 轮询 / 唤醒 / 联网恢复 / todos.json 变化
                                                                                       └──────► 拉起 `AskHuman todo-sync tick --reason …`
```

- **`todos.json` 仍是本机唯一数据源**，所有现有入口照旧直读直写。同步核心只做「本地变化 → 实体 → 云」与「云 → 实体 → 物化回 `todos.json`」，旁路状态都在 `~/.askhuman/state/todo-sync/`。
- **整个同步在一个 tick 里完成**：本地阶段（链接目录、diff、物化，不联网）→ 拉 → 合并 → 推。tick 由 helper 的触发器、读取钩子、窗口刷新或 CLI 拉起，任何一步崩溃或断网都能由下一个 tick 接着做。
- **helper 不含同步逻辑**：它是无状态的 CloudKit 适配器（Developer ID、非沙盒的 `.app`，登录项），只提供 `hello / status / ensure / fetch / save / deleteZone`，并把「该同步了」的各种信号翻译成拉起 tick。推送只当提示，不用 `CKSyncEngine`。
- **每台 Mac 单独开启**：开关是标记文件 `state/todo-sync/enabled`，关闭时每个入口只多一次 `stat`；Dev Instance 与非 macOS 构建恒不可用（`service::supported()`），所以开发实例永远不会碰生产容器。
- **粒度**：一条待办 = 一个实体（项目身份 + `text` / `ord` / `st` / `att` 几个寄存器，各带 HLC），字段级合并，已删除的是墓碑；项目身份 = 规范化后的 git remote，同一仓库的多个目录 / worktree 是同一份待办的副本；无 remote 的目录不同步。

## Mac 端（`src-tauri/src/todo_sync/`）

| 位置 | 职责 |
|---|---|
| `hlc.rs` · `order_key.rs` | 混合逻辑时钟（字符串序即时间序）、分数索引排序键（拖拽只改一条的键） |
| `identity.rs` · `links.rs` | 直接读 `.git/config`（含 worktree 的 `gitdir` / `commondir`）得到项目身份；链接表（目录 → 身份）只 `stat` 配置文件，改动才重读 |
| `model.rs` · `store.rs` | 实体 / 云端 payload / `state.json`（未知字段保留，新版本写的文件不改写，坏文件挪开不删）；原子写与 `todo-sync.lock` |
| `diff.rs` · `merge.rs` · `apply.rs` | 本地变化检测（对快照 diff，含清空历史的全局语义）、实体的字段级合并（交换、结合、幂等）、物化（纯函数，保留 `auto`、附件与无实体的条目） |
| `cloudkeys.rs` | 虚拟键 `icloud:<身份>`：本机没有目录的项目在 `todos.json` 里的住处（见下） |
| `tick.rs` | 一个 tick（`full` / `local_only` / `reconcile` / `restart_from_local` / `resume` / `clear_cloud` / `remove_project` / `peek`）；`state.json` 先于 `todos.json` 落盘，令牌在合并并物化之后才推进，推送确认后才记已推 |
| `transport.rs` | `Transport` trait（账号、建区、增量拉取、保存、删区）、`NetError`（含 `TimedOut` / `Unavailable` 这类软结果）、`MemTransport`（测试用内存云，可注入故障） |
| `helper_client.rs` | 生产 `Transport`：helper RPC 的客户端，线格式由 `todo-sync-helper/Fixtures/rpc-v1` 夹具钉住（Rust 与 Swift 共用）；helper 缺失 / 未签名 / 太旧 / 太慢都是软结果，tick 照做本地部分 |
| `service.rs` | 生产接线：文件位置、启用 / 停用 / 清除 / 恢复 / 从本地重来、状态（设置卡与侧栏底部用）、读取钩子、helper 拉起节流、窗口用的项目列表与详情、从 iCloud 移除项目 |
| `commands.rs` | `todo_sync_*` Tauri 命令：状态 / 启用 / 停用 / 立即同步 / `touch` / 清除 / 本地重来 / 恢复 / 修复 / 打开登录项，以及 iCloud 模式窗口用的 `projects` / `project_details` / `remove_project` / `history_count`；都在阻塞线程上跑，窗口从不等网络 |
| `sim.rs` | 整体测试：多台模拟 Mac（各有 `todos.json`、旁路与时钟）+ 一个可注入故障与崩溃点的内存云；顺序模型、随机并发、崩溃点全枚举、各暂停原因、虚拟键场景 |

`src-tauri/src/cli/todo_sync_cmd.rs`：隐藏（不进 help）的 `AskHuman todo-sync status | tick | enable | disable | resume | restart-from-local | clear-cloud | repair`；`tick --reason` 就是 helper 拉起的那条命令。

旁路目录 `~/.askhuman/state/todo-sync/`：`state.json`、`todo-sync.lock`、`enabled`（开关）、`helper.json`（交给 helper 的配置）、`helper-launch`（拉起节流）、`last-tick.json`（最近一次 tick 的结果，状态展示用）、`helper.sock`（套接字，路径长度受 104 字节限制）。标记在而 `state.json` 缺失或损坏时，下一次 tick 自行重建状态（新设备 id，云端实体按 id 认领，不重复也不丢）；没有标记就什么都不建。

## 读取钩子

| 钩子 | 做什么 | 调用点 |
|---|---|---|
| `before_read(project)` | 只做 tick 的**本地阶段**（链接当前目录、把别的 Mac 已到达的内容物化进来），不联网；等不到锁就放弃（≤ 100 ms） | `cli/todo_cmd.rs`、`agents/stop.rs`、`mcp/ask.rs`、`daemon/runtime/{inbound,todo,ios_tasks}.rs` |
| `before_whats_next(project)` | `before_read` + 上次成功拉取超过 20 秒才拉，最多等 1.5 秒；超时的拉取交给脱离的 `todo-sync tick --reason late` 在后台完成，离线或超时直接用本地 | `cli/mod.rs`（whats-next 分支内、自动派发之前） |
| `refresh_if_stale` | 待办窗口打开 / 获得焦点时后台触发，不阻塞渲染；结果经 `todos.json` 的文件监听到达窗口 | `commands.rs`（`todos_init`）、`todo_sync_touch` |

`todos.rs` 的写函数没有改动：同步对它们完全透明。

## 虚拟键：本机没有目录的项目

别的 Mac 同步来、而本机没有该仓库目录的项目，在 `todos.json` 里以 `icloud:<身份>` 为键存放。核心把它当一个目录来链接（`links` 里 `cfg_mtime_ms = 0` 的链接），所以 diff / 物化 / 推送都不需要特例，窗口、IM、CLI 也都按普通项目键读写。生命周期在 tick 的本地阶段：

1. `cloudkeys::ensure`：为「有存活实体、本机没有真实目录」的身份建链接；
2. 照常 diff，所以在虚拟副本里做的编辑也会成为实体事件；
3. `cloudkeys::prune`：真实目录出现（待办改物化到那里）或没有存活实体时，丢掉虚拟键及其快照与文件切片——放在 diff 之后，所以克隆仓库前一刻在虚拟副本里的编辑不会丢。

`remove_project`（窗口的「从 iCloud 移除此项目」）给该身份的所有存活实体打墓碑，对所有 Mac 生效；这不是封禁，之后新待办会让项目重新出现。虚拟键不进设备记录的项目列表，也不进「新建任务」候选与 IM `/todo` 的项目选择；它进 `todos_projects`（有待办时）与「全部待办」。

## 前端

| 位置 | 职责 |
|---|---|
| `src/views/TodosView.vue` | 待办窗口：侧栏 + 项目头 + 列表；iCloud 模式的项目头标签、菜单、仅云端说明条、来源设备与附件提示、历史的设备标注、移除确认 |
| `src/views/todos/todoProjects.ts` | 侧栏与「全部待办」的视图模型（纯函数）：本机模式两段、iCloud 模式三段（同步 / 无待办折叠 / 仅本机），同一仓库的多个目录合并成一行（`rowKeyOf` / `labelOf`） |
| `src/views/todos/TodosSidebar.vue` · `SyncFooter.vue` · `syncState.ts` · `useSyncStatus.ts` | 侧栏、底部状态槽（需求 §9.7 的各状态与动作）、状态到展示的映射、状态轮询（首帧之后才请求，只在同步开启时轮询） |
| `src/views/settings/TodoSyncCard.vue` · `useTodoSync.ts` | 设置 → 实验 →「待办存储」卡片 |
| `src/prototype/todos-window.html` · `settings-sync.html` | 浏览器预览（`mockIPC` 驱动真实组件；`data=real|stress|empty|dups|icloud`、`theme`、`platform`、`project`、`sync=…`），看布局、深浅色、窄窗抽屉与大数据，生产构建不引用 |

## CloudKit helper（`todo-sync-helper/`）

`Kit/`（线编解码、分发、tick 调度、错误映射、套接字服务、CloudKit 后端）是纯 Swift 包，`swift test` 即可；`App/`（`AppDelegate`、`HelperRuntime`、登录项注册、触发器、拉起 tick）由 XcodeGen 的工程 `project.yml` 生成。`todo-sync-helper/scripts/build.sh unsigned|dev|devid` 构建；`dev` / `devid` 需要维护者的 Apple 账号，没有 `HELPER_ALLOW_CLOUD_CHANGES=1` 会拒绝运行。`scripts/todo-sync-install-helper.sh` 把构建好的 `.app` 装进 `~/Applications`、注册登录项并打招呼。helper 暂不随正式版发布（后置项见 `docs/PROGRESS.md`）。

**装到另一台 Mac**：`ditto -c -k --keepParent` 打成 zip，`scp` 过去后 `ditto -x -k` 解开（不带隔离属性），在那台上跑 `scripts/todo-sync-install-helper.sh <解开的 .app>`（SSH 里也能注册登录项）；两台 `hello` 回的 `accountHash` 前 8 位应一致。AskHuman 本体要用**那台 Mac 自己的** Developer ID 签名：钥匙串按指定要求（含团队 ID）信任二进制，把另一个团队签的二进制直接拷过去会让后台 daemon 读不到 IM 密钥（见 `scripts/install.sh` 的签名注释）。SSH 会话拿不到签名私钥（`codesign` 报 `errSecInternalComponent`），要像 `install.sh` 的 `sign_via_gui_launchd` 那样用 `launchctl bootstrap gui/<uid>` 起一次性任务在 GUI 会话里签；最省事的是在那台上直接 `./scripts/install.sh`。`~/.local/bin/AskHuman` 若是指向 npm 包的符号链接，要替换的是链接指向的文件。装完用 `AskHuman doctor` 看 daemon 已换成新指纹。

## 验证

- Rust：`cd src-tauri && cargo test --bin AskHuman todo_sync`（核心 + `sim.rs` 收敛模拟；`TODO_SYNC_SOAK=<n>` 加大随机种子数浸泡）；`cargo test --release --bin AskHuman todo_sync::sim::the_local_phase -- --ignored --nocapture` 量本地阶段与项目列表在压力数据（200 项目 / 4000 实体）上的耗时。
- Swift：`cd todo-sync-helper && swift test`。
- 前端：`npx vitest run src/views`（`todoProjects` / 侧栏 / `TodosView` / 设置卡）。
- 窗口打开性能：`scripts/perf-todos-window.mjs`（`--ref <旧二进制>` 做 A/B，门限 +10% 且 +30 ms）。
- **真实 iCloud（Development 环境）**：`HELPER_ALLOW_CLOUD_CHANGES=1 todo-sync-helper/scripts/build.sh dev` 做 Xcode 自动签名构建（会联系 Apple，要账号所有者同意），再 `python3 scripts/todo-sync-e2e.py setup | reset | run [用例…] | teardown`：在一台 Mac 上用两份假 HOME（A、B）模拟两台设备，一个 helper 通过 `ASKHUMAN_TODO_SYNC_DIR` 服务 A，B 的 `helper.sock` 软链到 A 的；用例覆盖增 / 改 / 排序 / 完成 / 恢复 / 删除互传、并发编辑、全局清空历史、虚拟键与接管、worktree、helper 缺席、账号变化、重建、清云与暂停恢复、轮询追平、`kill -9` 循环、空闲资源。CloudKit 只看到一台设备，所以**推送延迟与跨机睡眠唤醒要两台真 Mac**。新容器在大陆 iCloud 账号上首次 `ensure` 会被拒（CKError 15 `serverRejectedRequest`），需要账号所有者在 CloudKit Dashboard 里把该容器的 Development 环境打开一次（不用建区）。
- **两台真 Mac 对 Production**：先由账号所有者把 schema 部署到 Production，再 `HELPER_ALLOW_CLOUD_CHANGES=1 todo-sync-helper/scripts/build.sh devid` 构建（未公证也能 `open` 启动，拷贝时别带隔离属性），然后 `TODO_SYNC_E2E2_REMOTE=<B 的 ssh 目标> python3 scripts/todo-sync-e2e-twomac.py setup | reset | run [用例…] | teardown`：本机（A）与 B（SSH）各用一份假 HOME 和自己的 helper，只碰 Production 里的一个测试区；用例（`run`，默认全跑，约 16 分钟，含 10 分钟空闲）验证环境、双向互传、设备列表、并发新增、删除 / 清空、helper 被杀后自行追平、空闲后互传，并从两台 helper 日志里算出「保存 → 推送到达 → 拉取」的时间线。`teardown` 清掉测试区、停 helper、收走日志、删假 HOME；需要免密 SSH 与 B 上的图形会话（helper 用 `open` 启动，与产品一致）。推送延迟的实测数字见计划 §10。睡眠唤醒、联网恢复与登录项形态远程做不了。

## 不变量与注意点

- 未启用时 `todos.json` 的字节输出与改造前一致，且不创建旁路目录（有测试）；打开待办窗口的路径上没有网络、helper、git 子进程与 daemon 往返，同步派生的信息异步补充，`todo_sync_projects` 在同步关闭时只 `stat` 一次就返回空。
- **清空历史是全局的**：iCloud 模式下清空一个项目的执行历史会清掉所有 Mac 与 iCloud 里该项目的历史（确认框写明条数）；按上限裁剪和 `todo_history_limit = 0` 不产生同步事件。
- 附件不同步，只同步「N 个附件，在 X 上」的提示；⚡ 自动执行标记不跨机。
- `open` 会把调用者的环境变量传给被启动的 helper，helper 再传给它拉起的每个 tick：从 Dev Instance 的 shell 里启动 helper 时必须先去掉 `ASKHUMAN_HOME`，否则所有 tick 都因 `supported()` 为假而静默空转（日志里是 `tick_done exit=0 ms=50`，`last-tick.json` 不更新）。真实使用时 helper 由登录项或 `AskHuman` 拉起，不受影响。
- 改 helper RPC 先改 `todo-sync-helper/Fixtures/rpc-v1`，Rust 与 Swift 两边测试必须同时通过；`state.json` 的 schema 只增不改，旧构建读到新 schema 时不改写文件。
- 容器 `iCloud.com.naituw.humaninloop.todosync`、helper bundle id `com.naituw.humaninloop.todosync`；创建 App ID / 容器 / 描述文件、部署 CloudKit schema 到 Production、公证与签名发布都需要维护者手动完成（计划 §7）。
