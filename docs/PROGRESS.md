# PROGRESS

按具体任务 / 需求记录待办与当前进展。任务 / 需求完成后删除其 section（历史留在 git）。

## 进行中：核对并推进 Agent 权限审批

计划 `docs/plans/agent-permission-approval.md`。M-1 通用双动作确认卡 view/builder/transport 已完成并验证，
`/stage` 行为与钉钉固定模板保持兼容；权限 Confirm、Hook 共存、设置/CLI 整包语义已完成核对。
M0 通用确认模型/IPC/registry/coordinator 与 M1 本地确认弹窗已实现并通过全量回归、前端构建及安装验证。
M2 Daemon 抢答、四 IM 卡片、投递状态、按需补推与终态 tombstone 已实现，钉钉专用权限模板已生成并发布；
M3 Claude Code / Codex PermissionRequest 协议适配、M4 Hook 共存/Codex trust/mode 整包 reconcile、M5
设置页/CLI/doctor 已实现。重复设置 mode 会完整更新磁盘，但不会自动改变 Permission 启用偏好。
当前步骤：M6 全量验证、安装、文档核对与人工验收。

## 待办：优化钉钉 Permission 卡的元数据展示

权限审批专用模板 `docs/assets/dingtalk-permission-confirm-card-template.json` 已完成功能真机验收：动态单选、
仅 deny 显示原因、Submit、原因传输与终态均正确；但 Agent / 项目 / workspace / tool / permission mode /
请求时间等元数据目前整体塞在 Markdown 正文中，视觉层级和弹窗相比明显较差。后续单独优化模板布局与字段
样式，保持已发布变量/回调协议和其它 Ask、`/stage` 模板不变。


## 待办：Watch 卡片「重新关注」按钮 — 全渠道

计划 `docs/plans/watch-rewatch.md`，需求 `docs/specs/watch-rewatch.md`。
AutoStopped / Cancelled 终态的 watch 卡提供可点击「重新关注」按钮。
飞书已上线验收通过；钉钉模板已更新（`docs/assets/dingtalk-watch-card-template.json`）、
代码已就绪；Telegram / Slack 代码已就绪。待真机验收钉钉 → Telegram → Slack。

## 待办：分析 src-tauri/target 编译产物过大（40+ GB）

## Hook 性能优化 —— 进程树遍历移至 Daemon

计划 `docs/plans/hook-perf-walk-optimization.md`。PreToolUse hook 耗时从 ~300ms 降至 ~33ms。
代码已完成（IPC hint_pid + daemon 缓存 + interject 优先响应），待用户确认后清除。

## 待办：Codex 生命周期 hook 信任哈希加固（「hook 新、哈希旧」窗口）

用户曾遇一次 Codex 弹「不信任 AskHuman hook」（时值 M2 迁移逻辑经其它任务的 install.sh 生效、
`migrate_outdated()` 给 PreToolUse 补 timeout=86400 的窗口期）。代码分析确认三个真实窗口
（当前盘上状态已核对一致，哈希与独立复算逐字节相同）：
1. `codex_install` 两步写（hooks.json → config.toml 信任）非原子且第二步失败**不回滚**；
2. config.toml 无锁「读-改-写」，与 Codex CLI 自身写入 / `mcp_config` 并发时后写者覆盖 `[hooks.state]`；
3. 新旧双二进制交错重装（GUI 宿主滞留旧版，见下方既有待办）可致 file/hash 版本错配。
另：信任键含数组下标（外部增删同事件条目即失效）；自愈仅在 daemon 启动时跑。
**候选修法**（用户定案：暂不做）：方案1 第二步失败回滚 hooks.json；方案2（推荐）daemon 周期 tick
顺带核对 Codex trust 一致性、不一致即幂等重装（秒级自愈，兼作竞态事后修复）。

## 待办：install.sh 换新后 daemon 与 GUI 宿主「换新不同步」→ 旧 GUI 重建旧路径产物

现象（本轮 grok skill 改名 `askhuman`→`interaction-protocol` 时踩到）：`install.sh` 换二进制后 daemon 会自动
drain+重启到新版（`ASKHUMAN_DAEMON_AUTORESTART`），但 **GUI 宿主（`--gui-host` 菜单栏 app）有独立的二进制
换新监视（`gui_host.rs::start_binary_watch`/`maybe_refresh_binary`，每 15s，且仅在「无打开窗口」时才换）**，
可长时间滞留旧二进制（实测滞留 6h+）。分裂期内 **旧 GUI 按旧代码的产物路径反复重建托管产物**：删掉
`~/.grok/skills/askhuman` 后，每逢 daemon 重连/配置事件它又按旧路径补回（内容为旧版 `name: askhuman`），
即便 daemon 已是新版。手动退出并重开 app（GUI 切到新二进制）后复现消失，重启 daemon 回归验证通过。

风险点：任何「产物落点/命名变更」的发布，在用户 GUI 未及时换新前都可能被旧 GUI 以旧路径重建，产生「新旧两份
并存」。待评估修法：install.sh/daemon 换新时主动通知 GUI 宿主换新（而非仅靠其自身 15s+无窗口门控）；或让 GUI
换新不被「有窗口」长期阻塞；或产物 reconcile 统一由单一新二进制来源执行。

## 待办：daemon 二进制变化检测 —— 轮询 vs filewatch（后续评估，优先级低）

二进制变化检测目前是 **15s 轮询** `current_exe()` 指纹（稳态≈1 次 `stat`，靠 `binhash.json` 内容哈希缓存避免重哈希）。
是否改 **filewatch** 待权衡——难点：二进制走原子替换（rename 换 inode，需盯父目录 + 按文件名过滤 + 每次替换后重挂，
参考 `config_watch.rs`）、装在任意目录（`~/.local/bin`/brew/npm 前缀/`.app` bundle…）、且 watcher 仍要 stat/hash 才能确认
内容**真**变（指纹是内容哈希而非 mtime）。延迟要求松（~15s 够）+ Hello 路径兜底，故暂保持轮询。
