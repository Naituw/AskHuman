# PROGRESS

按具体任务 / 需求记录待办与当前进展。任务 / 需求完成后删除其 section（历史留在 git）。

## 待办：项目 review 的 P2 项（择机）

报告见 `docs/investigations/project-review-2026-07.md`。P0+P1 已在分支
`chore/project-review-2026-07` 完成；P2 剩余：daemon.log 轮转、SettingsView/PopupView
组件拆分、types.ts 改为从 Rust 派生（ts-rs/specta）、secrets.rs 注释修正、
gitutil 测试噪音、TS 7 升级（等 vue-tsc 支持）。

## 待办：Cursor 全局 Rules 迁移为用户级 always-on Skill

调查与候选设计见 `docs/investigations/cursor-global-rule-user-skill.md`。无 workspace folder 的 Cursor IDE
不创建项目 Rules 加载器，因此不会读取 `~/.cursor/rules/askhuman.mdc`。未来改为用户级
`~/.cursor/skills/askhuman/SKILL.md`，旧安装显示“需更新”，迁移时先写新 Skill、再清理旧托管 MDC。
Grok 默认会扫描 Cursor Skills，候选 frontmatter 已设计为对 Cursor 常驻、对 Grok 不可调用。

## 待办：Watch 卡片「重新关注」按钮 — 全渠道

计划 `docs/plans/watch-rewatch.md`，需求 `docs/specs/watch-rewatch.md`。
AutoStopped / Cancelled 终态的 watch 卡提供可点击「重新关注」按钮。
飞书已上线验收通过；钉钉模板已更新（`docs/assets/dingtalk-watch-card-template.json`）、
代码已就绪；Telegram / Slack 代码已就绪。待真机验收钉钉 → Telegram → Slack。

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
