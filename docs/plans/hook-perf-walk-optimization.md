# 计划：Hook 性能优化 —— 进程树遍历移至 Daemon

> 状态：待确认

## 背景与问题

每次 Cursor 工具调用前（PreToolUse hook）阻塞 **~300ms**。实测分析：

| 阶段 | 耗时 | 占比 |
|------|------|------|
| `walk_agent_pid_from_self` (进程树遍历) | 253-297ms | 94% |
| interject poll (等 daemon 回复) | 12-20ms | 5% |
| 其余（env/stdin/tokio/socket） | <5ms | 1% |

根因：`process_chain()` 从 hook 进程向上走 6 层（每层 spawn 2 次 `ps` 命令），macOS 上每次 fork+exec 约 20-25ms，12 次 = ~280ms。此遍历在**每个 hook 事件**（含每次工具调用的 Pre/PostToolUse）都无条件执行，结果不变却重复计算。

## 方案

将进程树遍历从 hook 进程移到 daemon 进程，daemon 按 session_id 缓存解析结果。

- Hook 侧：不再调用 `walk_agent_pid_from_self`，改为发送 `ppid`（`libc::getppid()`，一次 syscall，0ms）作为提示，供 daemon 解析。
- Daemon 侧：首次收到某 session 的 hint_pid 时从其向上 walk 找到 agent PID，缓存在内存；后续同 session 事件直接用缓存。
- PreToolUse interject poll：先回复 interject 裁决（不依赖 PID），再异步解析 PID 做 apply_event。

预期效果：PreToolUse hook 从 ~300ms 降至 ~20ms。

## 已确认决策

1. IPC 协议：新增 `hint_pid: Option<u32>` 字段（`#[serde(default, skip_serializing_if)]`），不改 `pid` 字段。旧 daemon 忽略新字段、收到 `pid: None` 安全退化（仅丢失存活轮询）。
2. 所有 hook 事件统一改：session-start/turn-start/activity/turn-end/session-end 全部跳过 walk、发 hint_pid。
3. Daemon PID 缓存 key 为 `(session_id, hint_pid)`：agent 可 resume 同一 session（session_id 不变但 PID 变），hint_pid 变化是进程上下文变化的信号，触发重解析。
4. 缓存生命周期：跟随 `AgentRegistry` 的 session 生命周期，session 结束或 TTL 清除时一并清除。

## 改动详解

### 1. IPC 协议 (`src-tauri/src/ipc/mod.rs`)

`ClientMsg::AgentEvent` 新增字段：

```rust
/// hook 进程的 parent PID（ppid），供 daemon 从其向上 walk 解析 agent PID。
/// 旧 daemon 忽略；旧 hook 不带此字段 → None。
#[serde(default, skip_serializing_if = "Option::is_none")]
hint_pid: Option<u32>,
```

现有 `pid` 字段保留，hook 发 `pid: None`；daemon 侧 `pid: Some(x)` 视为已解析的 agent PID（兼容旧 hook）。

### 2. Hook 侧 (`src-tauri/src/agents/report.rs`)

替换：
```rust
let pid = detect::walk_agent_pid_from_self(intended);
```

为：
```rust
let pid = None;
let hint_pid = Some(unsafe { libc::getppid() } as u32);
```

`ClientMsg::AgentEvent` 构造时传入 `hint_pid`。

### 3. Daemon PID 解析与缓存 (`src-tauri/src/agents/registry.rs`)

`AgentRegistry` 新增内部字段 `pid_cache: HashMap<(String, u32), Option<u32>>`（`(session_id, hint_pid)` → 已解析 agent PID）。

缓存 key 为 `(session_id, hint_pid)` 而非单纯 session_id，原因：agent 可以 resume 之前的 session（session_id 相同但进程不同），hint_pid 变化是进程上下文变化的信号，应触发重解析。

新增方法：
```rust
pub fn resolve_pid(&self, session_id: &str, kind: AgentKind, hint_pid: Option<u32>) -> Option<u32>
```

逻辑：
1. `hint_pid` 为 None → 返回 None（无法解析）
2. 查 `pid_cache[(session_id, hint_pid)]` → 有值直接返回
3. 无缓存 → 调 `detect::walk_agent_pid(kind, hint_pid)` 解析
4. 缓存结果（含 None = 解析失败）并返回
5. Session 从 `active` 移除时清除所有匹配 session_id 的缓存条目

### 4. Daemon AgentEvent 处理重排 (`src-tauri/src/daemon/mod.rs`)

对 `interject_poll=true` 的 PreToolUse 事件，调整处理顺序：

**改前**（串行）：
1. resolve PID
2. apply_event（需 PID）
3. set_current_tool
4. interject poll → 回帧

**改后**：
1. interject poll → **立即回帧**（仅用 session_id，不依赖 PID）
2. resolve PID（可能命中缓存 0ms，或首次 walk ~280ms）
3. apply_event
4. set_current_tool

对非 interject_poll 事件（PostToolUse 及其它火后即忘事件）：顺序不影响延迟（hook 已退出），正常串行处理。

### 5. 测试单元

- `ipc/mod.rs`：新增反序列化测试确认 `hint_pid` 缺失时默认 None、有值时正确解析。
- `registry.rs`：`resolve_pid` 缓存命中/未命中逻辑。
- 集成验证：`ASKHUMAN_HOOK_PERF=1`（本次调试已加的临时计时宏，实现后移除）对比改前后 PreToolUse 耗时。

## 影响文件

| 文件 | 改动 |
|------|------|
| `src-tauri/src/ipc/mod.rs` | AgentEvent 增 `hint_pid` 字段 |
| `src-tauri/src/agents/report.rs` | 删 `walk_agent_pid_from_self`，加 `getppid` |
| `src-tauri/src/agents/registry.rs` | 新增 `pid_cache` + `resolve_pid` |
| `src-tauri/src/daemon/mod.rs` | AgentEvent 处理重排（interject 优先） |

## 向后兼容

| 场景 | 行为 |
|------|------|
| 新 hook → 旧 daemon | 旧 daemon 忽略 `hint_pid`，`pid: None` → 不存 PID（安全退化，丢失存活轮询直到 daemon 更新） |
| 旧 hook → 新 daemon | 旧 hook 发 `pid: Some(resolved)`，新 daemon 直接用（命中已有路径） |
| graceful drain 交叉期 | 同「新 hook → 旧 daemon」，持续数秒 |
