# 开发计划：跨设备待办同步（多台 Mac，iCloud）

> 状态：2026-10-27 起草，同日**评审通过并开工**（第 8 轮：用户把整个实施交给 agent 独立完成；与评审意见相关的改动见需求 §14，实施中的差异见本文 §10）。
> 对应需求：`docs/specs/todo-sync.md`（决策编号 T1–T16 指该文 §3；章节号指该文）
> 开工条件：本计划与需求评审通过 → 读 `docs/agent-worktree-setup.md` → 经 AskHuman 问清是否给 worktree 挂渠道预设 → 再动手。
> 范围：自用阶段（T1 第一步）。「随正式版发布」另行评审，不在本计划内。

## 0. 方案总览

三块可独立推进、最后拼起来的东西：

1. **Rust 同步核心**（`src-tauri/src/todo_sync/`）：本地变更检测、字段级合并、物化、顺序、元数据、tick。与传输无关，全部逻辑可用内存传输做属性测试，**不需要 iCloud 就能测完大部分**。
2. **CloudKit helper**（`todo-sync-helper/`，Swift，独立 `.app`）：无状态 RPC（`hello / status / ensure / fetch / save / deleteZone`）＋触发器（推送、轮询、唤醒、联网恢复、监听 `todos.json`）。
3. **待办窗口侧栏**（`src/views/todos/`）：先做**纯重构、不含同步**的统一侧栏（P1，可单独先合并），再叠加 iCloud 模式的内容（P5）。

```
P0 基线与预备 ─┬─ P1 侧栏重构（本机模式，可单独合并）──────────────┐
               └─ P2 同步核心（Rust，内存传输）─ P3 CloudKit helper ─ P4 集成 ─ P5 iCloud 界面 ─ P6 双机验收与加固
```

为什么这样切：
- P1 是风险最高的「用户可见回归」（既有窗口的拖拽、附件拖入、实时刷新），而且与同步无关，所以单独先做、先合并、先过性能门（T9）。
- P2 全是纯逻辑，属性测试能把「收敛 / 崩溃安全 / 离线」这些最难的正确性问题在没有网络的情况下钉死（验收 3–6、8、12）。
- P3 才需要真实 iCloud 与用户的手动步骤（§7），所以排在后面；它的接口（RPC）在 P2 就由 `MemTransport` 与夹具固定，两边并行也不会漂移。

## 1. 交付物与代码落点

| 路径 | 内容 |
|---|---|
| `src-tauri/src/todo_sync/mod.rs` | 对外 API：`before_read(project)`、`before_whats_next(project)`、`tick(reason, budget)`、`status()`、`enable()`、`disable()`；未启用时恒为空操作 |
| `…/hlc.rs`、`…/order_key.rs` | HLC 与分数索引顺序键（纯函数，属性测试） |
| `…/identity.rs` | 项目身份：远程地址解析 / 规范化 / 选择；读取 `.git/config`（含 worktree 的 `gitdir:` / `commondir`，不 fork `git`） |
| `…/model.rs` | 实体、字段、`payload` 的 serde；未知字段保留（前向兼容） |
| `…/store.rs` | `state/todo-sync/state.json` 的读写（原子写、`todo-sync.lock`） |
| `…/diff.rs` | 本地变更检测（需求 §5.5 的表） |
| `…/merge.rs` | 字段级合并（取 HLC 较大者的纯函数；交换律 / 结合律 / 幂等） |
| `…/apply.rs` | 物化到 `todos.json`：多目录副本、保留 `auto` 与附件、历史上限、`todo_history_limit = 0` |
| `…/links.rs` | 本机项目 key → 项目身份的链接表与缓存 |
| `…/transport.rs` | `trait Transport`（`fetch` / `save` / `ensure`）与测试用 `MemTransport`（延迟、丢包、重复、乱序、冲突、令牌过期） |
| `…/helper_client.rs` | 对 helper 的 unix 套接字 JSON 行 RPC 客户端（仅 macOS，其他平台编译为空桩） |
| `…/tick.rs` | 一轮 tick 的编排（§4.1） |
| `src-tauri/src/cli/todo_sync_cmd.rs` | 隐藏子命令 `AskHuman todo-sync`：`status`、`tick`、`enable`、`disable` |
| `src-tauri/src/config.rs` | `todoSync { enabled, deviceName, pollMinutes }` |
| `src-tauri/src/commands.rs` | `todo_sync_status`、`todo_sync_enable`、`todo_sync_disable`、`todo_sync_now`、`todo_sync_projects`（云端感知的项目行）、`todos_all_pending`（全部待办聚合）；`create_todos_window` 的默认尺寸 |
| `todo-sync-helper/` | XcodeGen 工程：`project.yml`、`Sources/`（`App.swift`、`Rpc.swift`、`Cloud.swift`、`Triggers.swift`、`Config.swift`、`Log.swift`）、`Entitlements`、`Fixtures/rpc-v1/`、`scripts/{build,install-self,launch}.sh`。**不放进 `src-tauri/swift/`**（那里的源文件会被 `build.rs` 编进主二进制） |
| `src/views/todos/todoProjects.ts` | 项目视图模型：分组、排序、同名区分、云端 / 本机行合并（纯函数，vitest） |
| `src/views/todos/TodosSidebar.vue` | 侧栏组件（搜索、分组、键盘导航、抽屉形态、底部状态槽） |
| `src/views/TodosView.vue` | 去掉头部 `<select>`，接入侧栏；iCloud 模式的项目头 / 行内标记（P5） |
| `src/views/settings/TodoSyncCard.vue`、`useTodoSync.ts` | 设置 → 实验 里的「iCloud 待办同步」卡片 |
| `src/i18n/{zh,en}.ts`、`src/lib/types.ts` | 文案与类型 |
| `scripts/perf-todos-window.mjs`、`docs/perf/todos-window-baseline.json` | 待办窗口打开耗时的度量脚本与基线（沿用 `docs/perf/baseline.json` 的做法与阈值思路） |
| `scripts/todo-sync-install-helper.sh` | 自用阶段：把本机构建好的 helper 装到 `~/Applications` 并拉起（另一台手动拷贝后同一脚本安装） |
| 文档 | 落地后更新：`docs/overview.md`（模块地图 + 「todos.json 仍是事实来源、同步走旁路」这条不变量）、`docs/specs/todo-whats-next.md` D9 加一行指向需求、`docs/wiki/todo-sync(.en).md`（自用安装与排障）；`docs/PROGRESS.md` |

## 2. 标识与常量

| 项 | 值 |
|---|---|
| CloudKit 容器 | `iCloud.com.naituw.humaninloop.todosync`（建议；待用户确认，需求 §14 第 1 点） |
| helper bundle id | `com.naituw.humaninloop.todosync` |
| 区 / 记录类型 / 字段 | 区 `Todos`；记录类型 `Todo`、`Device`；唯一字段 `payload`（`encryptedValues`） |
| 订阅 | 区订阅（静默推送），id `todos-zone` |
| 旁路目录 | `~/.askhuman/state/todo-sync/`：`state.json`、`helper.json`（核心 → helper 的握手：`AskHuman` 路径、state 目录、轮询分钟数、协议号）、`helper.sock`（0600） |
| 锁 | `todo-sync.lock`（tick 互斥）；`todos.lock`（沿用，只在本地阶段与物化时短暂持有） |
| RPC 协议号 | `1`（`hello` 握手；不一致则本机仅做本地阶段并提示升级 helper） |
| 时间常量 | `STALE_MS = 20_000`、`PULL_BUDGET_MS = 1_500`、`POLL_MINUTES = 10`、`LOCAL_DEBOUNCE_MS = 1_000`、`WAKE_DEBOUNCE_MS = 2_000`、`HLC_CLAMP_MS = 24h`、`BACKOFF_CAP = 30min`、`SAVE_RETRIES = 5` |
| 大小常量 | `ORDER_KEY_MAX = 32`、`TEXT_MAX = 256 KiB`、「全部待办」折叠阈值 50 行 |

## 3. 数据与协议

### 3.1 旁路状态 `state.json`

```jsonc
{
  "schema": 1,
  "deviceId": "uuid", "deviceName": "Mac Studio",
  "accountHash": "…", "enabledAtMs": 0,
  "zoneToken": "base64（helper 给的不透明值）",
  "lastPullOkMs": 0, "lastPushOkMs": 0,
  "paused": null,                       // 或 {"reason": "accountChanged|zoneMissing|notSignedIn|quota", "sinceMs": 0}
  "hlc": {"wallMs": 0, "counter": 0},
  "entities": { "<todoId>": {
      "p": "github.com/naituw/humaninloop",
      "text": {"v": "…", "h": "<hlc>"},
      "ord":  {"k": "a5", "h": "<hlc>"},
      "st":   {"v": "pending|done|deleted", "h": "<hlc>"},
      "c":    {"ms": 0, "dev": "<deviceId>", "ak": "cursor"},
      "dn":   {"ms": 0, "dev": "<deviceId>"},          // state=done 时
      "att":  {"n": 2, "names": ["a.png"], "dev": "<deviceId>", "h": "<hlc>"},
      "dirty": true, "tag": "<服务端 changeTag>" } },
  "snapshot": { "<localKey>": {"pending": ["id"], "history": ["id"], "hash": {"<id>": "<文本与顺序的哈希>"}} },
  "links":    { "<localKey>": {"p": "<项目身份>|null", "cfgMtimeMs": 0, "checkedMs": 0} },
  "devices":  { "<deviceId>": {"name": "…", "lastSyncMs": 0, "projects": ["…"]} }
}
```

全部放一个文件：体量只有几十 KB，一次原子写比多文件更不容易不一致。`todos.json` 完全不动。

### 3.2 云端 `payload`

等于上面 `entities[id]` 去掉本机专属字段（`dirty`、`tag`）再加 `"v": 1`。`Device` 记录的 `payload`：`{v, name, lastSyncMs, projects[]}`。解析时**忽略未知字段**，保留并原样回写（前向兼容）。

### 3.3 helper RPC（本机 unix 套接字，每行一个 JSON）

```jsonc
→ {"id":1,"op":"hello"}
← {"id":1,"ok":true,"protocol":1,"helperVersion":"0.1.0","environment":"production",
   "container":"iCloud.com.naituw.humaninloop.todosync","accountStatus":"available","accountHash":"…"}

→ {"id":2,"op":"fetch","token":"base64?"}
← {"id":2,"ok":true,"changes":[{"recordName":"…","recordType":"Todo","payload":"base64","changeTag":"…"}],
   "deletions":[{"recordName":"…","recordType":"Todo"}],"token":"base64","moreComing":false}

→ {"id":3,"op":"save","records":[{"recordName":"…","recordType":"Todo","payload":"base64","changeTag":"…?"}]}
← {"id":3,"ok":true,"results":[{"recordName":"…","status":"saved","changeTag":"…"},
   {"recordName":"…","status":"conflict","serverPayload":"base64","serverChangeTag":"…"},
   {"recordName":"…","status":"error","code":"…"}]}

→ {"id":4,"op":"ensure"}      // 建区 + 订阅，幂等
→ {"id":5,"op":"deleteZone"}  // 「清除 iCloud 数据」
→ {"id":6,"op":"status"}
// 失败： {"id":n,"ok":false,"error":{"code":"offline|rateLimited|notAuthenticated|zoneNotFound|quota|…","retryAfterMs":0,"message":"…"}}
```

- 令牌、`changeTag` 对核心都是不透明字符串。
- `fetch` 的 `deletions`：云端记录被真删（只会发生在「清除 iCloud 数据」之后重建、或手工在 Dashboard 删）时才出现；核心把它们当作「实体丢失」，保留本机副本并标记为脏重新上传，不据此删本机待办。
- 夹具：`todo-sync-helper/Fixtures/rpc-v1/*.json` 由 Rust 与 Swift 两边的测试共用；改协议先改夹具（与 `ios/Fixtures/wire-v1` 同一做法）。

### 3.4 HLC

字符串编码 `{wallMs 13 位十进制}-{counter 4 位十六进制}-{设备 id 前 8 位}`，**字典序即时间序**。本机事件：`wall = max(now, last.wall)`；`counter` 在同毫秒内递增。收到远端 HLC：`wall = max(now, local.wall, min(remote.wall, now + HLC_CLAMP_MS))`。

### 3.5 顺序键

分数索引字符串（base-62，字典序），`between(a, b)` 在两键之间生成新键，`a`/`b` 可为空表示队首 / 队尾；排序键为 `(顺序键, HLC, id)`。键长超过 `ORDER_KEY_MAX` 时由发起方对该项目全部条目一次性重排并各自给新 HLC。

## 4. 核心算法

### 4.1 tick（`tick.rs`）

```
tick(reason, budget):
  lock(todo-sync.lock)            # 无预算的触发源（helper / 窗口）：拿不到就合并为一次，直接返回「已在进行」
                                  # whats-next 内联调用：最多等持锁者 budget；等到后若它刚拉成功（陈旧度 ≤ STALE_MS）则不再重复拉
  state = store.load()
  # —— 本地阶段（不碰网络）——
  with lock(todos.lock):
      todos = read(todos.json)
      links.refresh(known_keys(todos, workspaces, agents, current_key))     # 只读 .git/config，按 mtime 缓存
      events = diff(todos, state.snapshot, state.links)                     # §5.5
      state.apply_local(events)                                             # 分配 HLC / 顺序键，标脏
      apply.materialize(todos, state)                                       # 链接目录间趋同；保留 auto 与附件
      write(todos.json) if changed; state.snapshot = snapshot_of(todos)
  store.save(state)
  if disabled or paused or helper unreachable or budget==local_only: return
  # —— 网络阶段（不持 todos.lock）——
  loop:
      r = helper.fetch(state.zoneToken)                                     # 失败 → 退避并返回
      state.merge_remote(r.changes)                                         # 纯函数：取每个字段 HLC 较大者
      with lock(todos.lock): materialize & write; snapshot 更新
      state.zoneToken = r.token; store.save(state)                          # 先写文件、后推进令牌（幂等）
      if not r.moreComing: break
  dirty = state.dirty_entities()
  results = helper.save(dirty)                                              # 冲突 → merge(serverPayload) → 重试（≤ SAVE_RETRIES）
  state.mark_pushed(results); store.save(state)
  state.lastPullOkMs = now
```

不变量：
1. `todos.lock` 只在本地阶段与物化时持有，每次毫秒级；网络调用期间不持有。
2. 合并是纯函数且幂等：同一批变更应用两次结果不变；崩溃后重跑即可。
3. 物化后立即更新 `snapshot`，使「远端写进来的变化」不会被下一轮 diff 当成本机编辑。
4. 令牌只在对应变更已写进 `todos.json` 与 `state.json` 之后推进。
5. tick 记录自己最后一次写 `todos.json` 的修改时间 / 哈希；helper 的文件监听事件若匹配则忽略（防反馈环）。

### 4.2 合并（`merge.rs`）

- `join(local: Entity, remote: Entity) -> Entity`：`text`、`ord`、`st`、`att` 各取 HLC 较大者；`c`（创建信息）取非空者；`dn` 随 `st` 一起走。
- `st == deleted` 的实体忽略后续对 `text`/`ord` 的更新（保留其 HLC 以便审计）。
- 性质（属性测试）：交换律、结合律、幂等；对任意操作序列与任意投递顺序，各副本最终相等。

### 4.3 物化（`apply.rs`）

- 对每个链接到项目身份 P 的本机目录 K：按 `(顺序键, HLC, id)` 排出 P 的待办，替换 K 的队列里同 id 条目的 `text`，缺的插入（`auto = false`，无附件），K 里已不是待办状态的移出；已执行的实体进 K 的历史（按 `doneAtMs` 正序，裁到 `todo_history_limit`；上限为 0 则不物化）。
- K 里本机独有的条目（还没有实体）不动，由 diff 在下一轮建实体。
- 只有项目身份为 null 的目录（仅本机）完全不碰。

### 4.4 读取钩子

- `before_read(project)`：未启用 → 立即返回；否则只做 tick 的**本地阶段**（链接当前目录并物化）。被 CLI `todo`、Stop 卡、弹窗待办区、IM `/todo` 调用。
- `before_whats_next(project)`：`before_read` + 陈旧度判断 + 带 `PULL_BUDGET_MS` 的网络阶段（需求 §6.3）；超时的拉取通过拉起脱离的 `AskHuman todo-sync tick --reason late` 在后台继续。调用点：`cli/mod.rs` 进入 whats-next 分支之后、`try_whats_next_auto` **之前**。

## 5. 阶段与验收

### P0 基线与预备（不写产品功能）

- 读 `docs/agent-worktree-setup.md`，经 AskHuman 问是否挂渠道预设；建 worktree 与分支 `feat/todo-sync`。
- 度量基线：加 `ASKHUMAN_PERF` 门控的待办窗口里程碑（`todos.win.created` / `todos.win.firstPaint`），写 `scripts/perf-todos-window.mjs`（冷 / 热各 N 次，真实数据与压力数据两档：200 项目 / 2000 待办 / 2000 历史），把现状基线落到 `docs/perf/todos-window-baseline.json`。
- 用户手动步骤（§7）启动：标识命名确认、开发者后台新建（可与 P1、P2 并行，P3 之前完成即可）。
- **验收**：基线数字入库；需求 §9.9 的度量目标按基线校准并回填需求。

### P1 侧栏重构（纯 UI，不含同步；T9）

- `todoProjects.ts`：从现有 `todos_projects` / `todos_projects_enriched` 的行生成分组（全部待办 / 有待办 / 最近项目），同名目录用路径后缀区分；纯函数 + vitest。
- `TodosSidebar.vue`：搜索、分组、选中态、↑/↓ 键盘导航、`listbox` 语义、窄窗抽屉形态、底部状态槽（P1 为空）。
- `TodosView.vue`：删掉头部 `<select>` 与相关逻辑，接入侧栏；新增「全部待办」聚合视图（新命令 `todos_all_pending`，只读一次 `todos::all()`）；预选项目（`todos-goto-project`、URL 参数、弹窗 / 托盘 / Agent 卡入口）行为不变；`create_todos_window` 默认尺寸约 760×580、最小约 560×360。
- 沿用「两段式加载」：首屏只用本地快路径，活跃 Agent 等补充在后台到达，侧栏行高固定不重排。
- **验收**：①本机模式下所有现有能力不回归（增删改、勾选完成 / 撤回、恢复、拖拽排序、附件拖入 / 粘贴 / 预览、⚡、复制、新建任务、清空、实时刷新、预选项目、两种语言、深浅色）；②性能门：真实数据 p95 ≤ 基线 +10% 且绝对增量 ≤ 30 ms，压力数据首屏 ≤ 300 ms；③既有 `TodosView.test.ts` 通过，新增 `todoProjects` 与侧栏组件测试；④`./scripts/install.sh` 装机后手动走一遍。
- **提交**：`feat(todos): replace the project dropdown with a sidebar`（用户可见）；`refactor` / `test` 按需拆开。可单独合并，不等同步。

### P2 同步核心（Rust，内存传输）

按顺序，每步都带测试：
1. `hlc`、`order_key`：属性测试（单调、全序、`between` 严格夹在中间、键长上限与重排）。
2. `identity`：夹具表（需求 §5.1 的全部例子 + scp 式 / ssh 端口 / `file://` / 本地路径 / 多个远程的选择顺序 / worktree 的 `.git` 文件与 `commondir` / 裸仓库 / 缺失配置）；不依赖 `git` 可执行文件。
3. `model` / `store`：serde 往返、未知字段保留、原子写、损坏文件降级（备份并重建）、`todo-sync.lock` 互斥。
4. `diff`：需求 §5.5 表里的每一行一个用例；多目录副本矛盾变化的采纳顺序；`auto` 与附件变化不产生事件；**清空历史（T16）**：历史由非空变空且消失的 id 不在队列 → 该项目云端全部 `done` 实体标墓碑（含本机早已裁掉的），而按上限裁剪（历史仍非空）、`todo_history_limit = 0` 下既有历史保留都不产生事件；部分恢复后清空（恢复的 id 在队列里）也算清空。
5. `merge`：交换律 / 结合律 / 幂等；墓碑与并发编辑；HLC 钳制。
6. `apply`：多目录趋同、保留 `auto` 与附件、历史上限与 `0`、新目录物化、仅本机目录不碰。
7. `transport` + `MemTransport` + `tick`：N 个副本 × 随机操作 × 随机投递（乱序、重复、丢失、延迟、`serverRecordChanged`、令牌过期）的收敛模拟；在 tick 的每个步骤注入「崩溃」后重跑，验证无丢失无重复。
8. 零影响：未启用时 `todos.json` 的字节输出与改造前一致，且不创建旁路目录。
- **验收**：需求 §16 的 3、4、5、6、8、12（逻辑层）；`cargo test` 全绿；覆盖率关键路径（diff / merge / apply / tick）无未测分支。
- **提交**：`feat(todo-sync): add the transport-agnostic sync core`，带页脚 `Release-Note: skip`（此阶段对用户不可见；用户可见的说明由 P4、P5 的提交承担）。

### P3 CloudKit helper（Swift）

- 工程：XcodeGen；Developer ID + 描述文件签名；非沙盒；Hardened Runtime；`LSUIElement`；登录项注册（`SMAppService`）。沿用 S0 的 `scripts/build.sh devid` 做法与 spike 里已验证的 CloudKit 调用（账号状态、建区、`recordZoneChanges`、保存与冲突）。
- RPC 服务（§3.3）+ 夹具；触发器：区订阅推送、轮询定时器（只在系统清醒时）、`NSWorkspace` 唤醒与解锁、`NWPathMonitor` 联网恢复、`todos.json` 的 FSEvents（去抖 1 秒，忽略 tick 自身写入）；事件统一翻译成拉起 `AskHuman todo-sync tick --reason …`。
- 单实例、崩溃自恢复、`hello` 版本握手、日志（轮转，不写待办内容，只写条数与耗时）。
- **验收**（手动 + 脚本，在用户两台 Mac 上）：RPC 往返、`fetch` 增量与全量、`save` 冲突返回、订阅推送触发 tick、唤醒 / 联网 / 轮询各触发一次、helper 被杀后自动重启、空闲资源（需求 §16 第 11 项）。
- 依赖用户手动步骤 §7.1–§7.3。**提交**：`feat(todo-sync): add the CloudKit helper app`，带页脚 `Release-Note: skip`。

### P4 集成

- `helper_client.rs`；读取钩子接入各入口：`cli/mod.rs`（whats-next，含 ⚡ 路径）、`cli/todo_cmd.rs`、`agents/stop.rs`、`mcp/ask.rs`、`daemon/runtime/{inbound,todo,ios_tasks}.rs`；**`todos.rs` 的写函数不动**。
- 配置 `todoSync`；Tauri 命令；设置 → 实验 →「iCloud 待办同步」卡片（需求 §8，含 Mac 列表与最后同步）；启用 / 停止 / 清除流程；账号与区被清除的暂停与提示；Dev Instance 恒关闭；helper 缺失时的「修复」入口。
- 窗口打开 / 获得焦点异步触发 tick（不阻塞渲染）；侧栏底部状态槽接入（本机模式显示「仅本机 · 启用 iCloud 同步…」，仅 macOS）。
- **验收**：需求 §16 的 1、2、9（Dev Instance）、10、12，在两台真机上用 CLI 级场景跑通（A 上 `AskHuman todo add` → B 上 whats-next 卡片出现）；Windows / Linux 构建与既有测试不受影响。
- **提交**：`feat(todo-sync): sync project todos across Macs via iCloud (experimental)`。

### P5 iCloud 模式界面

- 视图模型：云端项目行（`todo_sync_projects`：来自旁路缓存，不读 git、不联网）与本机行按链接合并；仅云端、仅本机、无待办的项目分组；全部待办聚合含云端项目。
- 项目头（仓库地址 / 目录数 / Mac 数 / 菜单）、行内设备名与附件提示、历史的设备标注与「所有 Mac 共享」、仅云端说明条、「从 iCloud 移除此项目」、侧栏底部状态（需求 §9.7 的全部状态）。
- 「清空历史」的确认框（T16）：iCloud 模式下写明会清空所有 Mac 与 iCloud 中该项目的执行历史及条数；本机模式文案不变。
- 仅云端项目的读写走旁路实体（核心 API），合并项目的写落最近使用目录的副本。
- **验收**：与 `docs/assets/todo-sync-ui/` 的界面稿一致；§9.9 性能门（iCloud 模式数据同样适用）；zh / en；深浅色；键盘与读屏。
- **提交**：`feat(todos): show every project's todos in iCloud mode`。

### P6 双机验收与加固

- 真机场景 E1–E12（§6）；至少一次 24 小时浸泡（脚本在两台 Mac 上随机增删改，周期性比较两边 `todos.json` 的摘要）；`kill -9` 循环（helper 与 tick 各数百次）；断网 / 恢复；睡眠唤醒；账号 / 区被清除（用调试开关模拟账号哈希变化，因为没有第二个账号）。
- 文档：`docs/overview.md`、`docs/specs/todo-whats-next.md` D9、`docs/wiki/todo-sync.md`（含英文版）；`docs/PROGRESS.md` 删除本条并登记「随正式版发布评审」「大陆账号复测」「附件同步」等后置项。
- **验收**：需求 §16 全部。

### 后置（不在本计划，另行评审）

随正式版发布（公证、helper 打进 `AskHuman.app` 的 `Contents/Library/LoginItems`、自更新下发、第二个大陆 iCloud 账号复测）；附件同步（含同机多目录）；iOS App 读写待办；其他传输（MQTT / 同步文件夹）；手动指定同步名与项目合并 / 改名工具。

## 6. 测试矩阵

| 层 | 内容 | 方式 |
|---|---|---|
| Rust 单测 | hlc / order_key / identity / model / store / diff / merge / apply | `cargo test`，含 `proptest` 属性测试 |
| Rust 收敛模拟 | N 副本 × 随机操作 × 随机投递 × 崩溃注入 | `MemTransport`，固定种子可复现，失败时打印种子 |
| 零影响 | 未启用时 `todos.json` 字节一致、无旁路目录、无网络 | 单测 |
| 协议夹具 | `Fixtures/rpc-v1`、`payload` 样例 | Rust 与 Swift 两边测试共用 |
| Vue | `todoProjects` 视图模型、侧栏组件、`TodosView` 回归 | vitest |
| 性能 | 窗口打开基线 / 压力 / 读取钩子本地阶段耗时 | `scripts/perf-todos-window.mjs` + 钩子的基准测试 |
| 真机 E1 | A 增 → B 的 `todos.json`（两边活跃） | 手动 + 脚本计时 |
| 真机 E2 | A 增 → B 空闲 → B 上 whats-next | 手动：卡片含新待办且额外等待 ≤ 1.5 秒 |
| 真机 E3 | 编辑 / 排序 / 完成 / 恢复 / 删除的双向传播 | 手动 + 摘要比较 |
| 真机 E4 | 并发编辑同一条的不同字段 → 字段级合并 | 手动（两机同时改） |
| 真机 E5 | 两机离线各自编辑后恢复 | 手动（断网） |
| 真机 E6 | 同机多 worktree 趋同；新建 worktree 首次 whats-next | 手动 |
| 真机 E7 | 仅云端项目：B 没有仓库时的查看 / 编辑 | 手动 |
| 真机 E8 | 睡眠唤醒、联网恢复后的追平 | 手动 |
| 真机 E9 | helper 被杀 / 升级 / 缺失 | 手动 |
| 真机 E10 | 清除 iCloud 数据后另一台的暂停与恢复 | 手动 |
| 真机 E11 | 账号变化（调试开关模拟） | 手动 |
| 真机 E12 | 删除旁路元数据后重建 | 手动 |
| 平台 | Windows / Linux 构建、既有测试、侧栏界面 | CI / 手动 |

## 7. 用户手动步骤（我不接触任何密钥、密码与 `.p8`）

1. **标识**（P0 并行，P3 前完成）：在 Apple Developer 后台新建 App ID `com.naituw.humaninloop.todosync`（开启 iCloud / CloudKit 与 Push Notifications）、容器 `iCloud.com.naituw.humaninloop.todosync`、Developer ID 描述文件（同 S0 的做法，见 spike 的 `RUNBOOK.txt`）。**容器与 bundle id 一经创建不能改名**，命名先在评审里确认。
2. **schema 部署**（P3 首次真机跑通后）：helper 先在 Development 环境建出记录类型 `Todo`、`Device`（字段 `payload`），你在 CloudKit Dashboard 里把 schema **部署到 Production**（只增不减）。
3. **构建与安装**：在 Mac A 上用 Xcode 构建并签名 helper（`scripts/todo-sync-install-helper.sh` 辅助）；把 `.app` 拷贝到 Mac B 的 `~/Applications` 后运行同一安装脚本；首次启动按系统提示批准「登录项」。
4. 两台 Mac 登录**同一个 iCloud 账号**；不要求开启 iCloud Drive。
5. **授权边界**：公证上传、注册设备、其他 Apple 账号变更都需要你另行授权；SSH 到 Mac B 仅在你当次同意后用于本功能的验收。

## 8. 风险与开放验证项

| 编号 | 风险 / 未验证 | 处理 |
|---|---|---|
| V1 | 稳定版 macOS 上普通 CloudKit 调用与推送行为 | P6 在可用时复测；不依赖 `CKSyncEngine` 已降低风险 |
| V2 | 另一个大陆 iCloud 账号（容器 / 用户层初始化） | 发布前必测（后置） |
| V3 | 断网离线编辑后的重试与合并 | P2 模拟 + P6 真机 E5 |
| V4 | 关闭 Surge 后空闲 Mac 的推送延迟 | 不影响设计（推送只是提示）；有机会再量 |
| V5 | 登录项在系统升级 / 用户手动关闭后的表现 | P6 E9；状态栏「同步程序未运行 · 修复」 |
| V6 | 侧栏重构的回归面（拖拽、附件拖入、实时刷新） | P1 单独先合并 + 既有测试 + 手动清单 |
| V7 | tick 写 `todos.json` 与文件监听的反馈环 | §4.1 不变量 5；P3 里专门测空转 |
| V8 | 读取钩子对 CLI 延迟的影响 | 本地阶段目标 ≤ 20 ms（压力 ≤ 100 ms），P2 基准测试 |
| V9 | helper 常驻资源 | 验收项（需求 §16 第 11 项）；超标则改成按需拉起 + 登录项仅负责推送 |
| V10 | 时钟错乱 | HLC 钳制；P2 属性测试覆盖 |

## 9. 提交与文档规范

- 提交遵循 Conventional Commits（见 `AGENTS.md`）：用户可见的变化用 `feat` / `fix`，其余用 `refactor` / `test` / `docs` / `build` / `chore`；scope 用 `todos`（窗口）、`todo-sync`（同步）、`settings`。release notes 里只应出现：侧栏、iCloud 待办同步（实验）。
- 分支 `feat/todo-sync`（worktree，Dev Instance，按 `docs/agent-worktree-setup.md`）；P1 可从主线单独分支先合并。
- 每个阶段结束：`./scripts/install.sh` 装机验证；`docs/PROGRESS.md` 更新当前阶段与下一步；完成的条目删除，历史留在 git。
- 改 `ipc/mod.rs` 的消息类型（若 P4 需要新增 daemon IPC）前先读 `docs/overview.md`「IPC 协议号」：本计划目前**不需要**新增 daemon IPC（tick 与 helper 通信走独立套接字）。
- 代码注释用英文。

## 10. 实施记录（与计划的差异）

- 2026-10-27：评审通过，用户上班不在线，原话「你自己全部搞定」。渠道预设问题没有再问，取最保守的 popup-only（不挂任何 IM 预设）。分支 `feat/todo-sync`，worktree `../HumanInLoop-todosync`。
- 授权边界不变：开发者后台新建 App ID / 容器 / 描述文件、Dashboard 部署 schema、公证上传、注册设备都只有用户能做（§7）；这些步骤之前的阶段能做多少做多少，被挡住的验收项会写在 `docs/PROGRESS.md`。SSH 到 Mac B 需要用户当次同意。
- **P2 同步核心（`src-tauri/src/todo_sync/`）完成**，与计划的差异：
  - 旁路状态里实体是嵌套的 `Rec{e, dirty, tag}`（§3.1 画成平铺），另有 `deviceRecTag`（本机 `Device` 记录的服务器标签）。
  - 首次同步**先拉后做本地阶段**：`lastPullOkMs == 0` 时 `local_only` 是空操作，`full` 先 fetch 再 diff，首次合并的排序键才能参照云端已有实体（需求 §5.6）。
  - 项目身份变了（remote 被改）：该目录的快照丢弃（等于「首次见到」），属于旧项目的 id 重新生成（带附件的条目留在本机、不同步，因为附件目录以 id 命名）。
  - 附件提示只接管缺失 / 空 / 自己写的提示（`diff.rs`）。
  - 新增 `tick::resume`（配额暂停后用户点「重试」）；未启用时 `full` / `restart_from_local` / `clear_cloud` / `resume` 也不创建旁路目录（零影响，有测试）。
  - 一个执行完又被「历史上限」裁掉、而 diff 没有机会先看到它的条目（一次性执行超过上限条）会被当作删除，而不是「已完成但本机不显示」：无害，不处理。
  - 测试在 `todo_sync/sim.rs`：顺序模型（每步操作后全体同步，逐步与「一份共享列表」模型比对，含顺序与历史）、随机并发（离线 / 丢确认 / 重复投递 / 令牌过期 / 时钟偏差 / 随机崩溃注入）、崩溃点全枚举（含二次崩溃）、各暂停原因。`TODO_SYNC_SOAK=<n>` 把种子数调大做浸泡。
- **P1 侧栏重构完成**（`feat(todos): replace the project dropdown with a sidebar`），与计划的差异：
  - 「全部待办」聚合视图直接内联在 `TodosView.vue`（共用现有行样式），没有单独拆组件；分组排序、标签、折叠（每次 50 行，「显示更多」再加 50 行）都在纯函数 `todoProjects.ts` 里并有 vitest。
  - `todos_all_pending` 只返回「按 key 排序的非空分组」，顺序由前端按侧栏顺序排（后端不知道活跃 Agent 的排名）；聚合行可直接勾选完成（沿用 1 秒撤回），点行 / 「打开项目 ›」切到该项目并选中该待办。
  - `todos_init` 多返回 `home`，用来把路径显示成 `~/...`；项目头 `⋯` 菜单（复制路径、在访达中显示）是本机模式下的最小版本，iCloud 模式的项（复制仓库地址、从 iCloud 移除）留给 P5。
  - ⌘/Ctrl+F 聚焦侧栏搜索（窄窗同时展开抽屉），Esc 收起抽屉 / 菜单；侧栏里的 ↑/↓/Home/End 自己处理，窗口级的待办行 ↑/↓ 导航对侧栏事件让路。
  - 新增浏览器预览页 `src/prototype/todos-window.html`（`mockIPC` 驱动真实 `TodosView`；`?data=real|stress|empty|dups&theme=&platform=`），看布局、深浅色、窄窗抽屉、大数据时用，P5 的 iCloud 视图也复用它。
  - 本机 shell 的 `HOME` 被指到 `/tmp/ph` 时，`pnpm install` 会因 store 路径不同要求清空重装 `node_modules`（交互提示会让 `install.sh` 卡住）：需要 `export npm_config_store_dir=<真实 HOME>/Library/pnpm/store/v10`。
- **P3 CloudKit helper（`todo-sync-helper/`）完成（未上真机）**，与计划的差异：
  - 纯 Swift 的 `Kit/`（线编解码、分发、tick 调度、错误映射、套接字服务、CloudKit 后端）与 XcodeGen 生成的 `App/` 分开，`swift test` 不需要签名即可跑（43 个测试，含与 Rust 共用的 `Fixtures/rpc-v1`）。
  - 未签名的构建能启动、开套接字，并对 `hello` 回 `problem=missingEntitlement`；`build.sh dev|devid` 会联系 Apple，没有 `HELPER_ALLOW_CLOUD_CHANGES=1` 拒绝运行。账号状态、建区、增量拉取、保存与冲突的调用沿用 S0 spike 已验证的写法，但**没有对真实 iCloud 跑过**，P3 的验收项（RPC 往返、订阅推送、唤醒 / 联网 / 轮询触发、被杀自恢复、空闲资源）都被 §7 的手动步骤挡住，登记在 `docs/PROGRESS.md`。
- **P4 集成完成（Rust 与窗口 / 设置卡，未上真机）**，与计划的差异：
  - 开关是标记文件 `state/todo-sync/enabled`，不是配置里的 `enabled`：读路径关闭时只多一次 `stat`，Dev Instance 与非 macOS 由 `service::supported()` 恒关闭；配置 `todoSync` 只放设备名与轮询间隔。
  - 新增 `service.rs`（文件位置、启用 / 停用 / 清除 / 恢复 / 从本地重来、状态、`last-tick.json`、给 helper 的 `helper.json`、helper 拉起节流）、`helper_client.rs`（生产 `Transport`）、`commands.rs`（`todo_sync_*` 命令组）与隐藏的 `cli/todo_sync_cmd.rs`；`NetError` 增加 `TimedOut` / `Unavailable`，helper 缺失 / 未签名 / 太旧 / 太慢都是软结果（tick 仍做完本地阶段）；`Opts.fresh_within_ms` 用来在近期拉过时跳过网络。
  - 读取钩子的位置：`before_read` 接入 CLI `todo`、Stop 卡、MCP 待办列表、IM `/todo` 与 `/new`（含管理卡的 `send_todo_manage`）；`todos_list` 不调用它，窗口只在 `todos_init` / `todo_sync_touch` 里后台触发 `refresh_if_stale`，靠 `todos.json` 的文件监听拿结果，打开路径零网络。
  - 设置 → 实验 →「待办存储」卡片；侧栏底部的同步状态在首帧之后才请求，只在同步开启时轮询；从侧栏跳到卡片时即使实验功能关着也显示实验页。
  - 清空「某个已同步项目的待办」时也会提示其他 Mac 同样会丢失（计划只要求清空历史的全局提示）。
- **P5 iCloud 模式界面完成（未上真机）**，与计划 / 需求的差异：
  - **仅云端项目不读写「旁路实体」，而是以虚拟键 `icloud:<身份>` 住在 `todos.json` 里**（`cloudkeys.rs`；需求 §9.4 / §9.8 已改写）。这样窗口、IM、CLI 的读写路径都不用为它写特例，diff / 物化 / 推送也不用；代价是虚拟键要有生命周期（`ensure` → diff → `prune`，真实目录出现时接管且不丢未同步的编辑）并被排除在「新建任务」候选与 IM `/todo` 的项目选择之外。「从 iCloud 移除」给该身份的所有存活实体打墓碑，不是封禁。
  - 窗口新增三个命令：`todo_sync_projects`（读旁路缓存，不读 git、不联网，同步关闭时 `stat` 一次返回空）、`todo_sync_project_details`（来源 Mac / 附件提示 / 完成所在 Mac，异步补充）、`todo_sync_remove_project`。`todo_sync_projects` 与 `todos_projects` 并行取，首屏分组一次到位。
  - 同一仓库的多个目录合并成一行（行键取最近使用的目录）；同步行以仓库身份命名（最后一段，重名时加 owner 提示），**仅本机行以目录名为准**（界面稿里写的是整条路径，侧栏不放路径）；搜索同时匹配目录。「无待办的项目」折叠区收纳所有零待办项目（含仅本机的），搜索或选中项在其中时自动展开。「全部待办」里同一仓库的副本去重，顺序同侧栏（有 remote 的在前，仅本机的在后）。
  - 性能（优化构建，压力数据 200 项目 / 4000 实体，`todos.json` 797 KiB、`state.json` 1.95 MiB）：本地阶段（空闲）中位数 21.5 ms（目标 ≤ 100 ms），`todo_sync_projects` 5.8 ms，单独读 `todos.json` 0.8 ms；前端视图模型在 640 个项目行上不到 0.5 ms。真实 Dev Instance 上无法开启同步（`supported()`），iCloud 模式的首屏数字待真机补。
  - 与界面稿 `docs/assets/todo-sync-ui/` 的对照是用浏览器预览（`data=icloud`，浅色 / 深色）目测完成的，不是像素级。
- **真实 iCloud（Development 环境）验证完成**（用户 2026-10-27 22:05 授权：Xcode 自动签名创建 App ID / 容器 / 开发描述文件，只在 Development 环境、在本机用两份独立状态模拟两台设备）。与计划的差异与发现：
  - **一台 Mac 只能模拟「两个副本」，不能模拟「两台设备」**：两份假 HOME（A、B）各有自己的 `todos.json` 与旁路状态，共用一个 helper（B 的 `helper.sock` 软链到 A 的），所以 CloudKit 只看到一台设备，B 的保存不会给 A 的 helper 发推送（`apns=registered` 但没收到）。因此推送路径与延迟（E1 / E2）、睡眠唤醒与联网触发（E8）、登录项安装形态仍要两台真 Mac；T7 改成「轮询追平 + 报告」（1 分钟轮询加 30 秒 leeway，实测 14–98 秒追平，中位约 70 秒）。
  - 验收脚本是 `scripts/todo-sync-e2e.py`（`setup | reset | run [用例…] | teardown`，用法见 `docs/overview-todo-sync.md`）。13 组用例共 59 项断言全过（最终版脚本 + 最终版二进制，`reset` 后从空白开始一次跑完）：互传（增 / 改 / 排序 / 完成 / 恢复 / 删除）、并发编辑、历史与全局清空、虚拟键与接管、worktree、helper 缺席、账号变化、重建、清云 / 暂停 / 恢复、轮询追平、`kill -9` 循环、空闲资源。
  - **新容器在大陆 iCloud 账号上，首次访问私有库会被拒**（CKError 15 `serverRejectedRequest`，helper 映射成 `rpc_failed code=other`）：与 S0 一样，要账号所有者在 CloudKit Dashboard 里把该容器的 Development 环境打开一次（不用建区）后才正常。这是开发者侧的一次性初始化，不是产品缺陷；Production 同理要在 Dashboard 里部署 schema（§7）。
  - **验收中发现并修了一个真实缺陷**：旁路目录里只剩 `enabled` 标记、`state.json` 丢了（手动清理、半损坏）时，tick 以为「同步关闭」直接空转，设置里却显示已开启。`service::run_tick_with` 现在在「有标记、无状态」时重建状态（新设备 id，实体按 id 从云端认领，不重复）；`state.json` 内容损坏的情形本来就由 `load_for_tick` 重建。有单元测试，E12 也加了这一步（先只删 `state.json`，再删整个旁路目录两种路径）。
  - **真实延迟（本机负载很高时测的）**：`fetch` 中位 0.48 s（p90 0.83 s）、`save` 0.70 s、`ensure`（建区 + 订阅）中位约 2 s（1.3–4.8 s）、首次全量同步约 13 s；无改动的 `AskHuman todo-sync tick` 中位 0.45 s（冷连接的第一次 2.45 s）。whats-next 的 1.5 s 拉取预算够用，冷连接时会超时并走「用本地 + 后台补拉」的软路径（有测试）。
  - helper 空闲 5 分钟（触发器开着）：0.0 % CPU、常驻内存 9 MiB、累计 CPU 0.02 s；第二个 helper 对同一状态目录会记 `already_running` 并在 1.5 s 内退出。`kill -9` 循环（B 的 tick 在随机时刻被杀，helper 每 7 轮被杀并重启）：默认 14 轮之外又各跑了 60 轮（种子 11 与 23，各杀 8 次 helper），两副本都收敛，无重复、无丢失。
  - 清理已做：用 `todo-sync clear-cloud --yes` 删掉 Development 区（之后 `fetch` 返回 `zoneNotFound`），helper 已停，假 HOME 与临时文件已删；容器、App ID 与开发描述文件按计划保留。
- **两台真 Mac 对 Production 容器的验收完成**（用户 2026-10-28 授权：Developer ID 构建 + SSH 到 Mac B 做双机验收，只用假 HOME 与测试数据，验收后清理；Production 的 schema 由用户在 Dashboard 部署，Todo / Device 两个记录类型）。不含公证。与计划的差异与发现：
  - Developer ID helper 用 `HELPER_ALLOW_CLOUD_CHANGES=1 todo-sync-helper/scripts/build.sh devid` 构建（`xcodebuild archive` + `export`，自动签名）；已核对签名与权限（`aps-environment=production`、`icloud-container-environment=Production`、容器 id、CloudKit）。未公证的 `.app` 用 `scp` / `ditto` 拷到另一台（不带隔离属性）后，`open` 可以直接启动。两台的 `hello` 都回 `environment=production`，账号哈希一致。
  - 验收脚本是 `scripts/todo-sync-e2e-twomac.py`（`setup | reset | run [用例…] | teardown`，用法见 `docs/overview-todo-sync.md`）：A（本机）与 B（SSH）各有一份假 HOME 与自己的 helper（`open -g -n -a … --env HOME=…`），CloudKit 看到的是两台设备，推送路径是真的。8 组用例共 22 项断言全过：环境、A→B、B→A、互相列为设备、双方同时新增（4.7 秒一致、无重复）、一台删除 / 另一台清空互传、B 的 helper 被杀后重启自行追平（启动后 3.4 秒）、两台空闲 10 分钟后再传。
  - **推送路径通了，延迟与 S0 同量级**（取自两台 helper 日志里 `rpc_save` 到 `push` 的间隔）。两台 Mac 都无人在用（B 是 Mac Studio，A 是笔记本，没人操作）时，一台保存后另一台收到推送要 **142 秒**（首次）、**107 秒**、**273 秒**（两台空闲 10 分钟之后；这次数据其实在 25 秒后由 B 的 10 分钟轮询先带到）；刚有过往来的「热」状态下 **12.1 秒与 8.5 秒**（删除、清空各一次；并发新增那组靠两台各自的 tick 互相拉到，4.7 秒一致，不算推送）。推送到达后 helper 拉起 `todo-sync tick --reason push`，拉取 + 合并 + 物化 0.5–2.7 秒。空闲时最长 4.6 分钟，被 10 分钟轮询封顶，在需求 §16 第 1 条的「轮询周期 + 2 分钟」之内；但该条的「两台活跃时 5 秒内」要有人在用的机器才能判定，这次是无人值守，没有得出结论。
  - 每台 Mac 也会收到自己那次保存引发的推送（`push ours=1`），拉回自己的记录再合并，幂等，只多一次不到 1 秒的拉取，没有处理。
  - helper 空闲资源（Production、Developer ID 构建，带十几次 tick 的 12–17 分钟里）：A 累计 CPU 0.75 秒、常驻 5.5 MiB，B 0.15 秒、34 MiB，空闲时 0.0 %。
  - **测试脚本里的一个坑，不是产品缺陷**：`open` 会把调用者的环境变量传给被启动的 App，helper 又传给它拉起的每个 tick。我的 shell 带着 `ASKHUMAN_HOME`（Dev Instance 的变量），于是 A 的 helper 拉起的所有 tick 都被 `supported()` 判为不可用而静默空转（`tick_done exit=0 ms=50`，`last-tick.json` 不更新）。脚本现在启动 helper 前剔除该变量；真实使用时 helper 由登录项或 `AskHuman` 拉起，不带它。
  - 没有做（超出授权，或远程做不了）：睡眠唤醒与联网恢复触发（E8）、登录项安装形态（`scripts/todo-sync-install-helper.sh` 会改真实的登录项）、helper 缺失 / 过期时的安装体验、24 小时浸泡、公证与分发、另一个大陆账号、有人在用时的推送延迟。
  - 清理：`clear-cloud` 删掉了 Production 里的测试区（两台都随即报 `zoneMissing`），两台的 helper 已停，A 的 `/tmp` 与 B 的 `~/AskHumanTodoSync-E2E`、两台 `~/Library/Logs/AskHumanTodoSync` 里的 helper 日志都已删，没有碰任何一台的真实 `~/.askhuman`，也没动 B 的网络与代理。Production 的 schema、App ID、容器与描述文件保留。`~/Library/Caches/CloudKit/com.naituw.humaninloop.todosync`：B 上已删，A 上被系统保护删不掉（空缓存目录）。
- **两台 Mac 装机并对真实数据启用同步**（2026-10-28 夜）：A（MacBook Pro）在用户授权后装 main 的构建，B（Mac Studio）在用户第二次授权 SSH 后装；只做了装机与启用，睡眠唤醒、联网恢复用户选了「稍后再说」。
  - **A**：`./scripts/install.sh` 装 main（v0.16.0，daemon 自动换新、0 个在途请求）→ `scripts/todo-sync-install-helper.sh` 装 helper，登录项直接是 `enabled`（没有弹批准）→ `AskHuman todo-sync enable`，首次同步 93 条记录（21 待办 + 72 历史），`unsynced` 0。本机目录 8 → 10、待办 21 → 34、历史 72 → 127：同一仓库的多个目录互为副本（`HumanInLoop-ios` 收到 `HumanInLoop` 的 11 条待办和 54 条历史，`WeiboMainSandbox` 收到 `WeiboMain` 的 2 条和 1 条），是设计行为。启用前把 `todos.json` 备份成 `todos.json.before-todo-sync-2026-10-28`。
  - **B**：B 的 `~/.local/bin/AskHuman` 是指向 npm 包文件的符号链接（`install.sh` 本来就替换链接指向的文件）。**不能把 A 的二进制直接拷过去**：A 的由 Wu Tian（DMJXDB9H6Q）的 Developer ID 签名，B 上原有的由 Sina（MPXZ3AAD6Q）的 Developer ID 签名，两者的指定要求（含团队 ID）不同；B 的钥匙串按旧要求信任二进制，换了签名后台 daemon 读 IM 密钥会失败或弹框。所以在 B 上用 B 自己的 Developer ID 重签：SSH 会话里 `codesign` 报 `errSecInternalComponent`（拿不到私钥），改用 `launchctl bootstrap gui/<uid>` 起一次性任务在 GUI 会话里签（与 `install.sh` 的 `sign_via_gui_launchd` 同法）；先在副本上试通，再核对新旧指定要求完全一致，旧二进制备份到 B 的 `~/.askhuman/backups/pre-todo-sync-install-<时间>/`，最后原子替换。B 的 daemon 随即让位（`daemon-restarts.json` 记 `trigger: disk`，数秒内），下一次调用拉起新版（`doctor`：安装无冲突、飞书连接正常）。helper 用 zip + `ditto` 拷过去，`todo-sync-install-helper.sh` 在 SSH 里就能注册登录项（直接 `enabled`），两台 `hello` 的账号哈希前 8 位一致、环境都是 production。装前备份了 B 的 `todos.json`，只数了数量没读内容。
  - **首次合并**：B 启用后第一次 tick 一次拉下 A 的全部 94 条、上传自己的 33 条；两台随后各显示 40 待办 / 86 历史、0 未上传，设备列表有两台。B 本机项目 8 → 14、待办 23 → 44、历史 14 → 51（同样是同仓库目录互为副本，B 上 `~/Developer/AskHuman` 与 `~/Developer/Sina/HumanInLoop` 是同一个仓库的两份检出）。A 在约 3.5 分钟后才由 10 分钟轮询追平（期间没有收到 B 那次保存的推送，与上面 107–273 秒的无人值守数字同量级）；A 之后的一次上传 0.4 秒内推到了 B。
  - **对真实数据第一次跑，发现并修了一个真实缺陷**：A 的每个 tick（轮询、推送、账号通知……）都会上传一条记录，下一个 tick 拉回自己的那条再上传一条，永不安静（B 没有这个现象）。查 `state.json` 发现只有一条待办的附件提示（`att`）每个 tick 都被 A 重新盖章，值始终是 `n = 0`。原因：同一条待办出现在 A 的两个目录（同一仓库的副本）里，只有一处有附件文件（2 个），另一处没有；`diff.rs` 逐目录判断「我的提示与本目录的数量不一致」，先写 `n = 2`、再被后一个目录改回 `n = 0`，每个 tick 都重新盖章并标脏，最后提交的是 `n = 0`——真实的附件提示从来没发出去，还白推一条记录。修法：按本目录的数量判断「要改」时，改为查这台 Mac 所有目录里同 id 待办附件最多的那份（并列时按名字取，与遍历顺序无关），提示跟着它走（`richest_attachments`），有单元测试（两目录、提示稳定、清掉后再安静）。开发阶段的模拟与脚本里副本的附件数都一致，所以没碰到；这份真实数据里 19 条待办出现在多个目录，其中 1 条附件数不同。装上修复后的构建（A，main）：手动 tick 一次上传了真实的提示（`n = 2`，只盖一次章），之后每个 tick 都是 `pushed = 0`，轮询 tick 只有 `rpc_fetch changes=0`、不再带 `rpc_save`（修之前每个 tick 都带）；B 约 10 秒后经推送拿到了这条提示（B 当时装的是修复前的构建，不受这个缺陷影响；之后已更新，见下一条）。
  - `AskHuman doctor` 在 B 上显示 cursor / codex / grok 的若干集成「需更新」（A 上全部正常）：新旧两个构建之间没有改集成文件的提交，应是 B 原有的状态；没有动 B 的集成配置。
  - **把修复后的构建也装到 B**（用户第二次授权 SSH）：第一次在 B 的 npm 包目录里新建暂存文件就报 `No space left on device`——B 的数据卷只剩 300–540 MB 且在波动，写盘的是 B 上别的项目的 WebDriverAgent 测试（一个约 600 MB 的 `.xcresult` 诊断文件、约 620 MB 的 `CoreSimulator.log`），与同步无关；什么都没发布，暂存目录已删，向用户报告后由用户清理磁盘，随后重来：上传 → 在 GUI 会话里用 B 自己的 Developer ID 重签（约 2 秒）→ 指定要求与已装的完全一致 → 旧二进制备份到 `~/.askhuman/backups/pre-todo-sync-fix-<时间>/` → 原子替换。B 的 daemon 随即让位（装时 0 个在途请求），`daemon start` 后换成新指纹，`todo-sync tick` 为 `fetched=0 pushed=0`，`doctor install --warn-only` 静默，暂存与一次性 launchd 任务都已清掉。装机脚本是一次性的，没有入库，流程同上一条。
  - 两次重签后 B 上二进制的指定要求都与装前逐字相同（团队 ID 取自 B 自己的证书）；第二次换新后 daemon 启动即连上飞书（`im conns feishu`）。
- **提交与发布说明**：P3、P4、P5 的提交都带 `Release-Note: skip`，因为 helper 暂不随正式版发布（计划后置项）。合并并决定随版本发布时，需要补一条带用户可见说明的 `feat(todo-sync)` 提交（或在合并提交里写）。
- **文档**：除计划里列的 `overview.md` 之外，新增专题概览 `docs/overview-todo-sync.md`（模块地图、钩子、虚拟键、验证命令）；`docs/specs/todo-whats-next.md` D9 已补一句 iCloud 模式；面向用户的 `docs/wiki/todo-sync.md`（含英文版）等功能能让普通用户启用（helper 随版本发布）时再写，避免文档先于功能。
