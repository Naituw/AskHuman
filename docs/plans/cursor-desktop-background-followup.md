# Cursor Desktop 后台完成控制实施

> **历史实施记录**：此版于 2026-10-15 撤回；2026-10-16 用户授权以默认关闭的实验功能恢复，
> 并加入原生行为预检。新的实施与验证见 [恢复计划](cursor-desktop-restoration.md)，本页保留初版证据。


用户已授权实现；行为真值见
[`../specs/cursor-desktop-background-followup.md`](../specs/cursor-desktop-background-followup.md)。
沿用当前 checkout；未创建并行 worktree。Cursor CLI 没有实测或升级，所有真实 Desktop
模型请求均使用 Grok 4.7。实现已安装并完成本机验收；人类确认设置页报错已消失，选择保持默认关闭。

## 1. 实现

- `cursor_desktop/mod.rs` 定义默认关闭的配置和只读运行状态；有效条件为 macOS、偏好开启、
  Cursor AskHuman CLI/MCP 集成有效。None 停止接入并保留偏好。
- `runtime.rs` 由 Daemon 发现当前用户的 Cursor 主进程，核对 PID、启动时间、安装路径、
  bundle ID 和 Electron Inspector fuse，再临时附加。已存在的用户 Inspector 不由本功能关闭；
  本功能打开的 Inspector 在 bootstrap 后关闭。端点冲突和未知契约不会误连。
- `bridge.js` 建立权限为 0700/0600 的私有 Unix 控制桥，只开放带随机认证的 status/restore。
  持久 manifest 用于 Daemon 换新后的接管与失败恢复；JS revision 与每次安装的 lease 防止
  旧回调误还原新拦截。还原失败时保留可重试端点，不能把失败当成已清理。
- `renderer.js` 在已检查的真实类原型上拦截后台完成动作，返回原生“通知已处理”结果，保留
  任务状态更新、ACK 和配对子代理 inbox 台账；不进入自动模型提交。后创建的服务实例共用原型。
  不改普通提交、Await、Stop 或 AskHuman 输出协议，不维护逐会话结束状态。
- 设置卡、搜索、中英文文案、配置和 doctor 已接入。状态轮询不自行重启被手动停止的 Daemon。
  有效能力和待还原桥独立保活，不改写通用 Daemon 生命周期偏好。

## 2. 自动检查与安装

- Rust 全量：1318 passed / 4 ignored。4 个 ignored 为需要外部运行环境的既有/原生契约测试；
  本轮另外实际执行 Cursor 原生 bridge ignored 测试，IDE 与 Agents 均通过，不调用模型。
- Clippy `--all-targets -- -D warnings` 通过；生产前端 build 和 Vue 类型检查通过。
- Node 控制桥测试覆盖真实结果与 ACK、后创建服务实例、幂等、契约漂移、外来包装、重载、
  用户调试器占用、认证、还原失败重试、丢失凭据恢复和过期回调。
- 前端覆盖默认关闭、保存/回滚、状态刷新、卸载后不写状态、还原期间轮询和集成卡门控。
  最终全量 `pnpm test` 通过：382 个 Vitest 用例和 15 个 Node 用例。
- 2026-10-14 最后一次 `./scripts/install.sh` 成功，生产安装路径 `~/.local/bin/AskHuman`，
  version 0.14.4，adapter revision `2f61116e366f8caa`，已签名。后续提问使用新安装程序。
- 人工截图发现状态接口报 `Command cursor_desktop_status not found`。原因是新增 handler 只注册
  未加入 IPC 分组选择；修复为 Integration 组，并补齐真实命令名的路由回归，2 项通过。
  重新安装、重启设置宿主；人类随后确认报错已消失，并选择本机保持关闭。

## 3. 安装后的真实 Grok 4.7 验收

环境：macOS，Cursor 3.24.12；原用户主进程 92826 保持运行。测试在两个空临时工作区进行，
仅按聊天设置模型，不调用全局模型配置 setter。共 4 条父聊天消息、2 个极短真实子代理请求；
两种 Grok 参数组合仍属于 Grok 4.7。没有 Cursor CLI 模型请求，也没有额外对照组重现。

### 普通 IDE / Agent Host

聊天 `734897fe-2068-4461-b278-985224933350`，临时窗口 3。
Agent Host 日志明确选择 `runtime=connect`，reason `managed-local-unavailable`；本证据覆盖
普通 IDE 的 Agent Host 路径及其当前 connect 后端，未据此宣称其它后端均已单独验收。

- 父请求 `44136c0d-0fec-430c-ae3c-96e377ad5e6e`，模型 Grok 4.7。
- Task 启动子代理 `fa8296ab-c0e3-45c1-9861-7b3c28fbeeb0`，模型日志为
  `grok-4.7-high-fast`，可见最终文本仅 `AH_CHILD_DONE`。
- Shell `940830` 返回真实 `AH_AWAIT_DONE` / exit 0，Await 工具状态 completed。
- Shell `940831` 在前台轮次运行期间保持后台等待；启动前台 `sleep 120` 后，通过原生
  `abortChatAndWaitForFinish`（与 Stop 共用控制器）停止轮次，再释放后台 Shell。
- Stop 09:16:21；后台 Shell 09:16:23 成功，输出 `AH_IDE_BG_DONE` / exit 0。
  原生已确认 Shell 台账包含 `940831`；待处理和在途队列为 0，当前轮次保持 aborted。
- 安装 guard 处理 2 个完成批次，覆盖 Shell 和真实子代理；没有模拟完成消息或自动请求。
  请求日志从停止后到下一条主动消息仅保留原用户请求，等待期间约 14 小时仍未续跑。
  这是人工回答等待造成的自然观察窗口，不是计划中的长时间耗时测试。
- 手动请求 `ed3930dd-6605-4b7c-beff-7cb575f81e5d` 于 23:34:42 开始，仍为 Grok 4.7，
  正常返回 `AH_MANUAL_OK`。

### Cursor Agents / legacy

聊天 `d668a1d2-5f6a-4dd9-bad0-8c5c647a37e8`，临时 Agents 工作区。
此路径走 `streamFromAgentBackend` 本地窗口运行链，没有进入普通 IDE 的 Agent Host 会话。

- 父请求 `e0df6865-5938-4547-a9e1-47e3f89ba019`，Grok 4.7。
- Shell `789205` 与明确的 Await 在实际等待状态相交；Await 返回原生 success/complete、
  regexMatch `AH_AGENTS_AWAIT_DONE` / exit 0，无挂死或丢结果。
- Shell `789206` 保持后台等待。父聊天通过新安装的 AskHuman `--whats-next` 发出独立测试
  请求，人类真实选择“结束本轮”；工具收到 `[selected_options] 结束本轮` 后父轮次自然结束。
  没有伪造用户回答，也没有把测试聊天的结束当作实现任务结束。
- 父轮次 23:36:54 结束；随后释放后台 Shell，23:36:55 成功，输出
  `AH_AGENTS_BG_DONE` / exit 0。完成通知被确认，原生台账包含 `789206`，队列为 0。
- legacy Task 不支持自定义类型名 `askhuman-qa`，首次调用在参数校验处拒绝，未发出子代理
  请求；普通 IDE 支持该类型。后续将补验合并入既定的手动续聊消息，使用该路径支持的
  generalPurpose + inherit，按用量账号实际继承 Grok 4.7。
- 手动父请求 `84fe8c47-dc79-4bdb-af75-5d0b529bdb90` 正常返回 `AH_MANUAL_OK`。
  子代理 `0fa2f55f-700c-4734-aac3-82bde3a0e4c6`，请求
  `7a8cd237-a116-4952-ac44-f8af3a1b043e`，模型元数据 `grok-4.7-low`，实际模型日志
  `grok-4.7`，可见最终文本 `AH_CHILD_DONE`。
- guard 处理 Shell 和子代理两次完成批次；待处理、在途、Await 集合均为 0。
  到手动续聊及子代理结束后的最终检查，没有额外的后台自动模型请求。

## 4. 产品生命周期与清理

- 开关通过已安装的公开 config 命令开启，Daemon 自动接入原已运行 Cursor，再接入后来
  新开的 IDE 窗口；两个新增的隔离 Cursor 主进程也无需调试启动参数即可自动发现并接入。
- 二进制安装后 Daemon 从 PID 13009 换为 2439，复用已存在的 bridge；revision 相同，
  guard 计数与行为保持，没有主动拆掉有效保护。
- 使用安装后的产品挂载链对普通 IDE 和隔离 Agents 窗口重载：IDE 约 4538ms
  （attaching → active），Agents 约 4019ms（短暂 error 后有界重试至 active）。
  这两个样本是观察值，不是性能承诺；重载期间存在接入间隙，与已确认规格一致。
- 开关恢复 false 后，原主进程两个窗口均不存在自有 guard；私有 manifest/socket 已移除，
  status 为 disabled，无错误。测试窗口关闭、两个隔离 QA 进程退出，原 Cursor 继续运行。
  测试聊天保留，没有归档、隐藏或删除。
- 9229 在所有验证结束后确认无监听。测试用原生 debugger/probe 已移除。
- None 门控、平台限制及失败恢复由配置/控制桥契约覆盖。本轮没有为补验临时改写用户的全局
  Cursor CLI/MCP 集成模式。CLI 与 MCP 共用此能力门控，真实安装链采用当前有效的 CLI 模式。

私有临时原始证据：`/tmp/ah-installed-Qm0Wia/` 下的请求日志摘要、两个 settlement、终端结果、
子代理可见输出、reload-installed 和 cleanup JSON；本节已保留关键事实，避免依赖 /tmp 长期存在。

## 5. 适配限制

这是基于已检查 Cursor 内部契约的适配。当前实机版本为 3.24.12；升级后服务或行为契约不匹配时
报告不可用并有界重试，不能把挂载失败显示为已生效。Windows/Linux、终端 Cursor CLI 和其它
自动调度不在此版范围。手动停止 Daemon 后已有窗口拦截可能仍在，明确关闭本开关或切至 None
才是还原原生后台续跑行为的入口；Daemon 重新运行会接管存量桥。
