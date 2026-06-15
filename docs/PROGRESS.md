# PROGRESS

按具体任务 / 需求记录待办与当前进展。任务 / 需求完成后删除其 section（历史留在 git）。

## 进行中：MCP 支持（实现阶段）

需求 `docs/specs/mcp.md` + 计划 `docs/plans/mcp.md` 已定稿（用户已审过）。为 Codex/Claude/Cursor 增加
MCP 模式（与 CLI 模式互斥）。关键设计：
- MCP server 是 daemon 的**薄壳**——`AskHuman mcp` 走 STDIO，每次 `ask` 工具调用就 spawn 现有
  `AskHuman --output json …` 子进程复用全部 ask 流程（弹窗/IM/抢答/历史/落盘/排空重连全复用）。
  输出走**结构化 JSON + output schema**（剔除脚本专用的 `selectedIndices`），图片读回转 `ImageContent`
  放进 content 直返。全平台同一套；daemon 重启自动重连（每次新调用）。
- 自动集成改为每家「CLI | MCP | 未集成」三态互斥；MCP 绑定 Rule(MCP 版) + MCP 配置（用户级全局），
  CLI 绑定 Rule(CLI 版) + 超时 Hook。一键切换（自动卸旧装新）。MCP 不需要超时 Hook。
- turn 追踪保持正交（仍只靠实验性 lifecycle hook）。工具名 `ask`，配置 server 名 `askhuman`。

实现按计划 §11 任务顺序推进（rmcp 依赖 + `mcp` 子命令 → ask 工具 → 提示词 → MCP 配置集成 →
三态编排 → 设置 UI → headless/doctor）。**实现完成前不自动跑 install.sh**，待用户要求再实测。

## 待办：daemon 二进制变化检测 —— 轮询 vs filewatch（后续评估，优先级低）

二进制变化检测目前是 **15s 轮询** `current_exe()` 指纹（稳态≈1 次 `stat`，靠 `binhash.json` 内容哈希缓存避免重哈希）。
是否改 **filewatch** 待权衡——难点：二进制走原子替换（rename 换 inode，需盯父目录 + 按文件名过滤 + 每次替换后重挂，
参考 `config_watch.rs`）、装在任意目录（`~/.local/bin`/brew/npm 前缀/`.app` bundle…）、且 watcher 仍要 stat/hash 才能确认
内容**真**变（指纹是内容哈希而非 mtime）。延迟要求松（~15s 够）+ Hello 路径兜底，故暂保持轮询。
