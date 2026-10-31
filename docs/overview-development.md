# 项目级开发验收实现地图

需求见 [项目级开发验收](specs/dev-verification-routing.md)，操作见
[工作树准备流程](agent-worktree-setup.md)，验证记录见 [实施计划](plans/dev-verification-routing.md)。

## 项目准备与入口

`development/project.rs` 将准备并入幂等 `dev enable`。主规则是入库的 `AGENTS.md`，
Claude、Cursor、Antigravity 有最小项目适配，在首次 enable 前即可读取。
是否启用 Dev 沿用原有任务约定，项目规则不要求每次功能开发初始化 Dev。
程序只编辑本树的 `askhuman-dev` 原生项目 MCP 条目，JSONC/TOML 保留其它内容。
同名自定义条目拒绝覆盖；私有 ledger 在发布前记意图，外部编辑阻止清理。
新生成的本机配置精确加入 Git 仓库本地 exclude，不忽略整个客户端目录。
写入与加载分开：当前控制器版本的真实工具调用留下存活观察记录，文件生成不宣称客户端已信任或重载。
enable 报告准备结果；enable／status 分别检查安装构建和所选投递方式，旧正式端明确显示未就绪，
不能靠重复重建 Dev 或刷新项目连接补齐正式转送能力。

`workspace.rs` 的 `.askhuman-dev/workspace.json` 绑定物理树和正式入口。
普通调用只保留旧 `enabled` 的 cwd 路由；新 manifest 不改变日常去向。
显式 enable 仅升级本树旧标记，先检查旧 Dev Agent/MCP，备份后移除，不强杀连接。
没有账户级路由模式、程序注册表或全局集成迁移。
`routing.rs` 固定内部子进程的程序、home、工作树和密钥策略；Linux 保留稳定安装入口。

## 开发控制器与构建

`scripts/dev.mjs` 是仓库入口；其 Node SDK MCP 只提供 `dev_ask`。
普通 Rust MCP 仍为六个工具，不含开发例外。连接启动即绑定绝对树，列目录不依赖 Dev 已构建。
每次验收委托该树实际程序，不在共享 server 中切换全局 home。
controller 本地处理 Codex 后台／委派 guard、MCP 取消与精确 turn_aborted 兜底；
日常身份和 provider 环境不传给 Dev，调用元数据只在 controller 中读取。

`build.rs` 与 `dev-build-record.mjs` 使用相同源码快照算法，包含未提交构建输入与嵌入 macOS 程序的 `Info.plist`。
安装只在构建前后快照一致时签发 `build.json`，客户端检查实际程序、能力和 daemon 指纹。
能力探测使用旧版也会拒绝的选项形式，避免旧程序把未知内部命令当成提问。
普通／Dev MCP 生命周期内目标固定；缺失、旧版和 stale build 均明确失败。

## 请求、取消和远程连续性

`verification.rs` 为单次验收和私有原生交互保存独立 ID、构建与终态。
结果在 Dev 的 `home/state/verifications/`，重取不重发并校验构建。
取消先记 interrupted 再关闭 socket；原 controller 或 Windows driver 退出也取消。
已中断记录拒绝迟到回答。非活跃留存 1000 条／30 天，活跃请求最多 256。
一次验收不取得日常原生控制权限；附件留存于该 Dev，单文件 100 MiB、合计 200 MiB／128 个。

新项目默认 remote，只有明确 local 才用本机弹窗；channels 测试真实 Dev 渠道且不回退 relay。
`relay.rs` 为问答、私有权限和 Stop 逐请求授权，最长 24 小时、最多 256 个。
授权匹配 ID、树、构建、有效期和题目指纹。正式端仅运输，使用唯一的正式连接；
不写普通历史，不执行待办或权限记忆，不查找或控制原生测试会话。
Stop 的隐藏待办元数据在正式载荷中清除，选项文本返回后由 Dev 处理；
权限记忆 finalizer 只在原 Dev 的协调器执行。

正式转送先用无 Hello 的 Status 查询能力、实际 daemon 指纹和远程出口。
不兼容、未运行、排空或无出口时明确失败，不触发换新，不静默等待本机。
新增载荷为带默认值的可选字段，IPC 仍为 4；语义能力要求 developmentVerification=2，
相同协议号不能替代能力检查。
显式 relay 绕过日常活跃槽但不改槽；iOS 仅该请求立即推送，日常 idle 门槛保持原样。
断线、取消、到期和发送失败通过原协调器收尾，不产生成功答案。

`channel_safety.rs` 在项目 daemon 初始连接和配置重载前比较正式端提供的不透明渠道身份摘要，
拒绝复用正式 bot／配对；正式 helper 只读配置，不迁移凭据。
预设独占租约同时识别新 manifest 与旧 marker。测试转送代码本身使用私有测试端和测试渠道。

## 私有 Agent profile

`session.rs` 在 `.askhuman-dev/clients/<agent>` 管理 profile；原生 session 由私有 lifecycle 绑定。
转录、控制、新建和 Fork 沿用该绑定，未绑定或错构建请求拒绝。
主开发项目配置不会被测试启动器再次载入。

| 客户端 | 完整测试启动边界 |
| --- | --- |
| Claude Code | CLAUDE_CONFIG_DIR、仅 user settings、严格私有 MCP |
| Codex CLI | CODEX_HOME、拥有的中性 git 工作区、add-dir 目标树 |
| Grok | GROK_HOME、中性工作区、按现有 Stop 能力开放 |
| Pi | PI_CODING_AGENT_DIR、关闭自动扩展发现、明确私有扩展 |
| Cursor / Antigravity | 项目 MCP 与完整隔离启动分别判定，完整启动尚未开放 |

不复制日常认证；登录和真实模型调用仍按用户授权。
profile 普通退出保留，purge 仅清理本工具拥有且无活动会话的中性工作区。
原生 profile 选项在 prompt 的 `--` 分隔符前组装；仓库 CLI 的 `session --stdio` 继承调用终端的输入输出。

## Dev 界面与普通产品

`DevelopmentView.vue` 只通过 Dev 的 `dev ui` 打开，后端限制为窗口绑定的唯一树。
普通设置没有开发 tab、迁移或注册。
Dev 安装同时启用 Cargo `development-ui` 与 Vite `VITE_ASKHUMAN_DEV_WORKBENCH=1`；
普通构建不嵌入工作台入口。前端缓存已有 VITE 环境维度，两个变体不复用错误资产。
普通用户级模板、规则与 MCP 注册不加入本方案的开发指令。
