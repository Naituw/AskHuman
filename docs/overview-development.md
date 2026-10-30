# 开发验收实现地图

需求与用户体验见 [日常沟通与开发验收隔离](specs/dev-verification-routing.md)，操作见
[工作树准备流程](agent-worktree-setup.md)，实施与验证记录见 [计划](plans/dev-verification-routing.md)。

## 目标与进程

`development/routing.rs` 在业务配置、密钥与 GUI 初始化前解析目标。目标同时包含程序、home、工作树、
选择原因和密钥策略，通过 `ASKHUMAN_RESOLVED_TARGET_V1` 固定内部子进程；外部 Agent 启动链路清除此
内部上下文，独立测试启动器再设置自己的完整目标。显式选择与外部 home 或继承目标冲突时失败。

路由状态固定在账户的 `~/.askhuman/development/routing.json`，不随 `ASKHUMAN_HOME` 移动。
已有状态缺失时采用 `legacy`；首次全新安装采用 `explicit`，既有安装通过明确迁移进入新流程。
正式程序注册保留稳定安装入口，Linux 运行映像与后续启动入口仍按 `executable` 模块区分。

`development/build.rs` 与 `scripts/dev-build-record.mjs` 使用同一源码快照算法；安装记录位于工作树
`.askhuman-dev/build.json`。快照包含构建输入中的未提交变化。验证同时检查源码、实际程序、能力版本，
客户端提交前还检查 daemon 的能力和启动时指纹。源码在构建中变化时不签发记录。

## 一次验收与远程转送

`dev ask`／MCP `dev_ask` 由 `verification.rs` 建立唯一验收记录，执行本树程序。`MessagePrompt.development`
是可选、带默认值的载荷；旧端会忽略它，因此验收客户端必须先通过能力与指纹检查，不能据此直接复用旧端。
IPC 协议号不变。构建信息单独返回，不写进人类回答正文。

一次验收清除原生 Agent 身份、祖先探测和日常待办交接权限，答案按 ID／构建保存于 Dev 的
`home/state/verifications/`。重取不重发，拒绝错构建；取消或发起执行器退出不算人类答复。
新建请求时按 1000 条／30 天清理非活跃记录；并发未完成请求上限 256。

`relay.rs` 为远程投递签发本次请求的有界授权；正式实例只用自己的渠道连接收取答案并回传，不记录普通
历史或执行待办。Dev 保存测试历史与附件。授权最长 24 小时、最多 256 个；题目指纹、工作树、验收 ID 和
构建必须匹配。取消／超时关闭转送子进程，沿用协调器首个终态生效的语义。

附件先归入 Dev 的验收目录，图片恢复为图片附件；单文件 100 MiB、单请求合计 200 MiB／128 个。
借用正式渠道只证明转送；验证渠道实现仍使用 Dev 专用 bot 或 iOS 配对。

## 独立客户端配置

`session.rs` 为每工作树、每 Agent 保存独立 profile，位于 `.askhuman-dev/clients/<agent>`。
原生 session 需经该 profile 的生命周期 Hook 建立绑定，转录文件查找也受该绑定约束。
完整测试的子任务与 Fork 使用原 profile 与 Dev 目标；无法确认归属的原生启动会失败。
迁移后，直接 Dev 提问必须走 `dev ask` 或测试 profile；未绑定 profile 的原生事件、权限确认和会话控制会被拒绝，
旧记录不会凭一个原生 session ID 重新取得控制能力。

| 客户端 | 配置隔离与启动 |
| --- | --- |
| Claude Code | `CLAUDE_CONFIG_DIR`，仅 user settings，严格独立 MCP 配置 |
| Codex CLI | `CODEX_HOME`，独立 git 空工作区启动，`--add-dir` 允许目标工作树 |
| Grok | `GROK_HOME`，同样使用独立工作区，使用已有 lifecycle，遵循其 Stop 能力限制 |
| Pi | `PI_CODING_AGENT_DIR`，关闭自动扩展发现，仅装载测试扩展 |
| Cursor | 当前版本仍读取全局 MCP，完整测试会话不开放；保留基础 Dev 问答 |
| Antigravity | 无已验证的独立原生启动适配器，完整会话不开放；基础问答遵循平台原有支持 |

测试 profile 不复制日常认证文件。首次启动按客户端要求登录，后续复用该工作树的 profile。
临时空工作区有所有权记录；`dev disable --purge` 在没有活动测试会话时删除自己拥有的空工作区。
普通退出保留 profile、认证与测试数据，供下一次验收；开发窗口和确认使用 Dev 标识。

## 设置、迁移与回退

`workbench.rs` 提供「开发测试」页：工作树注册、构建诊断、投递偏好、独立配置准备／启动及明确的
Dev 窗口。GUI 进程不临时切换 home；Dev 设置窗口拒绝写日常 Agent 集成或用户级登录项。

`migration.rs` 检查受管产物、自定义 MCP、现存 Dev 会话和旧 MCP 连接。检查阶段不切换模式；执行前
再次检查在途会话与文件指纹。正式集成生成明确的 `--instance system`，包括 Rules、MCP、生命周期、
权限、Stop、恢复、子代理 guard、Pi 扩展与 Codex 信任配置。

受管文件和原权限先私有备份。每次发布前持久化允许的内容指纹，进程中断后仍可识别已完成的写入。
各文件原子发布，整体不是跨文件原子事务；部分失败显示 `partial`，允许重试或回退。原始错误保存在
私有诊断文件，公开结果不带配置正文。只有最新检查记录可执行／回退；外部改动必须先处理，不能被回退覆盖。
回退全部恢复后才恢复原路由模式。迁移或回退后需重启受影响客户端，在途请求不会迁移。
