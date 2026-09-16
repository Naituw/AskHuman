# 开发计划：Daemon 优雅排空换新（graceful drain）

> 关联需求：`docs/specs/daemon-graceful-drain.md`
> 计划描述方案与技术 / 规则细节，具体代码以实现为准。

## 0. 方案总览

```
（任一客户端 Hello 时发现 stale，或收到 graceful Stop）
  └─ Daemon：draining = true
       ├─ 在途请求（registry 中的 Submit）继续服务到完结；GuiHello / Answer 照常
       ├─ 新 Hello → HelloAck{ status: Draining }；新 Submit → ServerMsg::Draining{active} 后断开
       ├─ Detect → Error 拒绝（客户端侧已被 Hello 挡住，此为兜底）
       └─ 排空看门狗：registry.active_count() == 0 → shutdown → 退出、释放 socket/flock
            └─ 等待中的 CLI 轮询发现下线 → spawn 新 Daemon（flock 去重）→ 重新握手提交
```

排空只新增「**何时退出**」与「**拒新收新**」逻辑；在途请求路径（handle_submit / handle_gui / 协调器 / IM Router）一行不动。

---

## 1. IPC 协议（`src-tauri/src/ipc/mod.rs`）

`PROTOCOL_VERSION` 保持 1，全部增量演进：

- `HelloStatus` 增 `Draining`：Daemon 正在排空，将在在途请求完结后退出；客户端应等其下线后用新二进制拉起再提交。
- `ServerMsg` 增 `Draining { active: usize }`：Submit 撞上排空期（Hello 后、Submit 前才开始 drain 的竞态窗口）时的拒绝回复，回完即断开。
- `ClientMsg::Stop` 由单元变体改为 `Stop { #[serde(default)] force: bool }`。serde 内部标签枚举两向兼容：旧 Daemon 解析 `{"type":"stop","force":…}` 忽略多余字段；新 Daemon 解析 `{"type":"stop"}` 取默认 false。附编解码兼容单测。
- `StatusInfo` 增 `pub draining: bool`（`#[serde(default)]`，旧 Daemon 回包缺字段 → false）。

过渡期边界（记录，不处理）：旧二进制 CLI 收到 `Draining` 枚举值会解码失败 → 现有代码按连接异常处理（重试后退 3）。正常流程不出现（盘上已是新二进制，新发起的 CLI 都是新代码）。

## 2. Daemon 端（`src-tauri/src/daemon/mod.rs`）

### 2.1 状态与排空看门狗

- `ServerState` 增 `draining: AtomicBool`。
- 进入排空（幂等，`swap(true)` 防重复触发）时 spawn 看门狗任务：每 500ms 检查 `state.registry.active_count() == 0` → log + `shutdown.notify_one()` 退出循环（进入前先即时检查一次，覆盖「请求恰好刚完结」）。
- 退出路径复用现有 serve() 收尾（此时 registry 已空，`cancel_all_requests` 为 0，不等 2 秒）。
- 空闲退出计时器与排空互不干扰（排空期若 active 连接为 0 被空闲计时杀掉也是正确结果）。

### 2.2 Hello 处理（改造现有 stale 分支）

- 排空中（无论 stale 与否，含 graceful stop 触发的排空）：回 `HelloAck{ status: Draining, reason: "draining: waiting for active requests" }`，**保持连接继续 control_loop**（客户端自己断开）。
- 非排空且 stale && auto_restart：
  - `registry.active_count() == 0` → 现状不变：回 `Restarting`，立即 shutdown。
  - `> 0` → 置 draining、spawn 看门狗，回 `Draining`。
- `ASKHUMAN_DAEMON_AUTORESTART=0` 时照旧不触发（逃生口语义不变）。

### 2.3 Submit / Detect 闸门

- `handle_submit` 入口处检查 `draining`：是 → 回 `ServerMsg::Draining { active: registry.active_count() }`，直接 return（不入 registry、不弹窗）。
- `ClientMsg::Detect`：`draining` 时回 `ServerMsg::Error { "daemon is draining" }`（兜底；正常被客户端 Hello 闸门挡住）。
- `ClientMsg::GuiHello` / `Answer` / `Status` 排空期**照常**（在途请求的弹窗与作答必须不受影响）。

### 2.4 Stop 处理

- `ClientMsg::Stop { force }`：
  - `force == true` 或 `registry.active_count() == 0` → 现状：回 `Stopping`，立即 shutdown。
  - 否则 → 置 draining + spawn 看门狗，回 `Stopping`（CLI 端靠轮询 status 获知进度）。

### 2.5 status 与子命令

- `ClientMsg::Status` 回包填 `draining: state.draining.load()`。
- `print_status`：`requests` 行追加排空标注，如 `requests   2 active (draining)`；或单独一行 `state      draining`——实现取前者（少一行，信息同样完整）。
- `dispatch`：`stop` / `restart` 解析可选 `--force`（其余参数照旧报 unknown）；usage 行更新为 `daemon <run|start|stop [--force]|restart [--force]|status|logs>`。
- `stop_cmd(force)`：
  - `request_stop(force)` 返回 true 后：force → `wait_until_down(5s)` 即完成；graceful → 循环每秒 `request_status()`，有 `draining` 即打印 `waiting for N active request(s)… (use --force to terminate now)`（首条立即、之后每 30s 一条），直至 status 连不上（已下线）→ 打印 stopped。无限等待。
- `restart_cmd(force)`：同上等到下线后 `ensure_running()` 拉新。
- `start_cmd` 不变（其 `ensure_running` 对 Draining 的行为见 §3）。

## 3. CLI 客户端（`src-tauri/src/client/mod.rs`）

- `request_stop(force: bool)`：发 `Stop { force }`（两处调用方传参）。
- `hello_status()` 自然返回新枚举；各调用点处理 `Draining`：
  - `ensure_running()`：遇 `Draining` → 返回 `Err("daemon is draining")`（**不等待**——该函数还服务于设置进程 `request_detect`，不能无限阻塞；`daemon start` 遇之打印错误退 1 即可接受）。
  - `request_detect()`：`ensure_running` 出错 → 返回 None → 设置窗口回退进程内识别（与现状「不可达即回退」一致，零额外改动）。
- `run_ask_async()` 重构等待逻辑（核心改动）：
  - 现有「3 次重试」语义保留，仅覆盖瞬时失败（连接 / 读写错误）。
  - Hello 得 `Draining`，或 Submit 后收到 `ServerMsg::Draining`：进入**排空等待**（不消耗重试预算，等完后重置重试计数重来）：
    - 立即打印首条 stderr 提示，此后每 30s 一条；活动数 N 通过 `request_status()` 即时获取（Status 不带 Hello，不会误触发 stale 判定），取不到则省略数字。
    - 文案（英文，与现有 `askhuman:` 系 stderr 一致）：`askhuman: daemon is draining (N active request(s) left); waiting to submit… (run 'AskHuman daemon restart --force' to switch now, interrupting them)`。
    - 等待方式：轮询 `transport::connect()` 直至连不上（下线），无超时上限；下线后回到主循环 `ensure_running()` 拉新提交。
  - `ServerMsg::Draining` 只会出现在 `Accepted` 之前；`Accepted` 之后流程不变。
- `wait_until_down(max)` 保持；排空等待用独立的无上限轮询（带提示回调），不复用改签名。
- **2026-09-16 修正（spec D7–D9）**：上述「轮询 `connect()` 直至连不上」在实测中永远等不到——旧 daemon 退出后 ≤1s 新 daemon 就被别的 `ensure_running()` 调用方拉起。现改为记住排空 daemon 的 pid，每 500ms 查 `Status`，无应答 / pid 变化 / 不再 draining 即结束；`wait_until_down` 与 `wait_stopped` 同样按 pid 判定。文案补「this question will pop up automatically once they are answered」。配套：`ensure_running` 的拉起段持 `spawn.lock` 串行化并在拿锁后重新握手；macOS 对 launchd 仍报存活的任务不 `bootout`（`SpawnPolicy::ReuseAlive`，等就绪超时后才 `Replace`）；daemon 安装 SIGTERM 处理，语义等同 graceful `Stop`。

## 4. install.sh / install-windows.ps1

- 脚本开头（构建前）：若 `command -v AskHuman` 存在，跑 `AskHuman daemon status`，从输出 grep `requests\s+N active` 提取 N；N>0 时打印提示（不中断安装）：
  - `提示: daemon 当前有 N 个在途请求；安装后将在它们完结后自动换新（期间新提问会等待）。立即换新: AskHuman daemon restart --force（会打断在途请求）`。
- Windows 版做对应的 PowerShell 文本匹配，文案一致。
- 旧二进制的 status 输出已含 `requests N active` 行，提取逻辑对新旧二进制均有效。

## 5. 帮助与文档

- `cli/help.rs` 用户帮助中 daemon 子命令一节（若有）补 `--force` 说明；**agent-help 不动**（等待提示文案本身已包含强制命令，无需扩充 agent 上下文）。
- `docs/overview.md`：daemon 一节补「stale/stop 默认 drain 排空、--force 立即、排空期拒新提问」。
- `docs/PROGRESS.md`：实现完成后清理本任务标记。

## 6. 测试

- 单测：
  - `ipc`：`Stop{force}` 新旧两向编解码（`{"type":"stop"}` → force=false；带 force 字段可解）；`StatusInfo` 缺 `draining` 可解为 false；`HelloStatus::Draining` / `ServerMsg::Draining` 序列化往返。
  - daemon 内部纯逻辑（如有抽出的判定函数）随实现补充；drain 状态机以手测为主（依赖 socket + 多进程，不强行单测）。
- 手测脚本（install 后，两终端）：
  1. 终端 1 `AskHuman "Q1" -o A -o B`（不作答）；
  2. 改任意源码注释跑 `install.sh`（产生新指纹）；
  3. 终端 2 `AskHuman "Q2"` → 应打印排空等待提示且不弹窗；`AskHuman daemon status` 显示 draining + 1 active；
  4. 作答终端 1 弹窗 → 终端 1 正常输出；数秒内旧 Daemon 退出、终端 2 弹窗出现并可正常完成；
  5. 重复 1–3 后改用 `AskHuman daemon restart --force` → 终端 1 立即收尾（连接断 / 卡片 Cancelled）、终端 2 在新 Daemon 上弹出；
  6. `daemon stop`（有在途）→ 等待提示 → 作答后自动退出；`daemon stop --force` 立即退出。

## 7. 涉及文件清单

- `src-tauri/src/ipc/mod.rs`：协议增量 + 兼容单测。
- `src-tauri/src/daemon/mod.rs`：`ServerState.draining` + 看门狗 + Hello/Submit/Detect/Stop 闸门 + status + `--force` 子命令解析 + `print_status`。
- `src-tauri/src/client/mod.rs`：`request_stop(force)`、`Draining` 处理、`run_ask_async` 排空等待 + 周期提示。
- `scripts/install.sh`、`scripts/install-windows.ps1`：在途请求提示。
- `src-tauri/src/cli/help.rs`（如含 daemon 用法）、`docs/overview.md`、`docs/PROGRESS.md`。

## 8. 任务顺序

1. `ipc/mod.rs` 协议增量 + 编解码兼容单测。
2. `daemon/mod.rs`：draining 状态 + 看门狗 + 各消息闸门 + Stop force + status。
3. `client/mod.rs`：Draining 处理 + run_ask 排空等待 + stop/restart 子命令改造。
4. install 脚本提示 + 帮助/文档。
5. `cargo test`；待用户同意后 `install.sh` 实测手测脚本（注意：首次升级仍是旧 Daemon 的立即重启行为，需先让新 Daemon 跑起来再验证 drain）。

## 9. 风险与注意

- **首次升级例外**（spec D6）：验证 drain 前必须确保当前运行的已是含本特性的 Daemon。
- **竞态窗口**：Hello Ok → Submit 间隔内开始排空 → 由 `handle_submit` 入口闸门兜底回 `ServerMsg::Draining`，客户端走同一等待路径。
- **永不完结的在途请求**：用户一直不作答则 drain 永等——这正是 D2 的「无限等待 + --force 提示」设计意图，不加超时。
- **`ensure_running` 不能无限等**：它被设置进程 `request_detect` 复用，遇 Draining 必须快速返回错误；只有 `run_ask` 与 `stop/restart` 子命令实现无限等待。
- **Status 不带 Hello**：等待期轮询 N 用 `request_status()`，不会误触发 stale 判定，也兼容向旧 Daemon 查询。
- **serde 兼容**：`Stop` 变体改形依赖内部标签枚举「单元变体忽略多余字段」行为，必须有双向单测兜底。
