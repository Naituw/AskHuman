# PROGRESS

记录需要跨会话保留的未完成 / 延期事项和明确下一步。任务 / 需求完成后删除其 section
（历史留在 git）。

## 待办：实现 Codex 权限弹窗“本会话 / 始终允许”（规格已定案）

规格见 `docs/specs/codex-permission-remember.md`（2026-07-18 定案，D1–D48，全部开放问题已关闭）。
北极星：用户作答次数 ≤ 原生 Codex TUI。实现范围：会话级 shadow rules（文件 / Shell / network / MCP，
对话树共享 + 30 天滚动清理）、永久级原生写入（Shell prefix_rule、network_rule、MCP approval_mode）+
会话桥接、插件 / codex_apps 的跨会话 shadow 兜底（D41）、Shell 判断混合复刻（execpolicy check CLI +
脚本拆分 / 层叠 / heuristics 复刻，版本上限门控）、guardian / strict_auto_review fail-closed（D36/D43）、
设置页授权管理面板（D16/D17/D48）。开发完成后按规格 §6.4 在本文件登记“定期同步 Codex Shell 判定复刻”。

## 待办：Cursor 全局 Rules 迁移为用户级 always-on Skill

调查与候选设计见 `docs/investigations/cursor-global-rule-user-skill.md`。无 workspace folder 的 Cursor IDE
不创建项目 Rules 加载器，因此不会读取 `~/.cursor/rules/askhuman.mdc`。未来改为用户级
`~/.cursor/skills/askhuman/SKILL.md`，旧安装显示“需更新”，迁移时先写新 Skill、再清理旧托管 MDC。
Grok 默认会扫描 Cursor Skills，候选 frontmatter 已设计为对 Cursor 常驻、对 Grok 不可调用。

## 待办：daemon 二进制变化检测 —— 轮询 vs filewatch（后续评估，优先级低）

二进制变化检测目前是 **15s 轮询** `current_exe()` 指纹（稳态≈1 次 `stat`，靠 `binhash.json` 内容哈希缓存避免重哈希）。
是否改 **filewatch** 待权衡——难点：二进制走原子替换（rename 换 inode，需盯父目录 + 按文件名过滤 + 每次替换后重挂，
参考 `config_watch.rs`）、装在任意目录（`~/.local/bin`/brew/npm 前缀/`.app` bundle…）、且 watcher 仍要 stat/hash 才能确认
内容**真**变（指纹是内容哈希而非 mtime）。延迟要求松（~15s 够）+ Hello 路径兜底，故暂保持轮询。
