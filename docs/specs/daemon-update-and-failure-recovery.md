# Daemon 更新接班与错误卡 24 小时接续

> 状态：2026-11-05 用户已评审通过并要求实施；已实现，macOS 隔离回归通过并安装 Dev。正式转送往返待确认，尚未合并或安装日常版。
> 用户已确认的行为边界见 §2。本文补充而不替换
> [优雅排空](daemon-graceful-drain.md)、[错误收件箱](agent-failure-inbox.md)与
> [生命周期结果及接续](agent-lifecycle-results-and-recovery.md)。
> Cursor 只读调查；实施与验证均不得向真实 Cursor 发消息、触发 Agent 或人为制造失败（会计费）。

## 1. 事故与根因

2026-11-05，北京时间，日常 daemon 日志与 Cursor 结构化日志：

| 时间 | 事实 |
| --- | --- |
| 02:30:27 | `4e7c23f9`（iOS popup and response features）的 Round 6 提问被受理 |
| 02:48:39 | 盘上二进制已替换，daemon 开始排空 |
| 03:59:02 | Cursor 会话 `782998a5`（iOS task addition feature）重试 10 次后失败；daemon 03:59:04 读到 |
| 04:41:46 | Cursor 会话 `4e7c23f9` 重试 7 次后失败；daemon 04:41:47 读到 |
| 04:46:32 | Cursor 会话 `aa70cefc`（Spatial Camera data collection）重试 9 次后失败；daemon 04:46:33 读到 |
| 05:46:47 | `aa70cefc` 因一小时无活动被 registry 标为 ended；失败结果仍保留 |
| 07:26:09 | 用户在手机答完 Round 6；请求归零，daemon 正常记录 drain complete、stopped |
| 07:37:30 | 用户手动启动，daemon 恢复 |
| 07:37:32 | 只有 `4e7c23f9` 的错误卡补弹；用户 07:37:41 选要求继续 |
| 07:38:04 | `782998a5` 在 Cursor 出现一次 resume，旧 ended 时间随后被覆盖 |

两处缺口：

1. `begin_drain` 只退出，依赖外部 CLI 发起下一次启动；托盘只重连，launchd 只有 RunAtLoad。
   keepalive 的现有语义是跳过空闲退出和下次登录启动，不保证换新接班。此次停机 11 分 21 秒。
2. 排空期间错误卡仅登记为 saved，未成为在途提问，不能刷新会话活动；会话随后 TTL 结束。
   restore 的 touch_activity 只刷新 active，current 又只接受 idle，于是删除待恢复卡。
   `4e7c23f9` 因 Round 6 的在途提问每 15 秒刷新而幸存。`aa70cefc` 的 ended 时间是直接证据；
   `782998a5` 的 TTL 路径由代码与无在途提问的时序推断，旧 ended 时间已不可读，不声称精确到秒。

三次 Cursor 失败的底层原因均为 Accepted Agent WebSocket connection closed，均早于 daemon 退出；
不能证明由 AskHuman 排空造成。本文修接班与错误提醒，不宣称修复 Cursor 自身网络错误。

## 2. 已确认的产品行为

| 编号 | 决定 |
| --- | --- |
| R1 | 本次保证更新换新自动接班；意外崩溃后的常驻自愈另行设计 |
| R2 | 仍等已受理提问结束再换新，不自动 force、不迁移普通在途请求 |
| R3 | 会话可以照旧因一小时无活动进入 ended；已经登记的未处理错误卡不因此消失 |
| R4 | 错误卡与接续资格保留至首次检测到失败后 24 小时；排空、补弹、重启、重试均不续期 |
| R5 | 原生支持接续的会话，在此期间不因 TTL ended 或最近结束列表容量失去要求继续 |
| R6 | Cursor 未运行或聊天未在窗口打开：保留卡与接续资格，明确提示恢复后重试；本次不自动打开聊天 |
| R7 | 验证只用隔离 daemon、伪造日志和原生控制替身；不触发真实 Cursor Agent |

R4/R5 不是无条件运行保证：用户已知道了、已开始新回合、明确 sessionEnd、关闭确认/集成、卡到期时，
旧卡正常结束；Agent 本来不支持原会话接续时仍只有知道了。不新开 CLI/聊天代替原会话。
临时断线与一小时无活动均不代表用户处理了卡。Mac 离线时手机不能提交，重连后剩余期限继续算。

## 3. 更新必须在 daemon 自己的退出流程中完成接班

### 3.1 区分退出意图

把仅有 draining/shutdown 信号的流程补成明确的退出意图，由一个入口串行决定：

| 意图 | 触发 | 谁保证后续启动 |
| --- | --- | --- |
| ReplaceOwnBinary | 盘上自身安装入口的指纹变化，且自动换新开启 | daemon 本身 |
| YieldToClient | 只有客户端内容/协议不同，自身安装入口未变化 | 发起让位的客户端，沿用既有机制 |
| Stop | 显式 stop/force stop | 不自启 |
| Signal | SIGTERM/登出/服务管理器停止 | 不自启 |
| Idle | 活动模式自然闲退 | 不自启 |

无在途请求时的 Hello 立即退出分支也必须记录 ReplaceOwnBinary，不能只修 begin_drain。
停止/信号优先于已经挂起的换新：用户在换新排空期间明确停止，即撤销自行接班；
周期指纹检查不能重新覆盖停止意图。显式 restart 仍由 restart 调用方完成 stop→start。
尊重 ASKHUMAN_DAEMON_AUTORESTART=0、多二进制 hold、Dev Instance 与隔离 home。

只为自身盘上升级接班，避免“客户端要求另一个版本，旧 daemon 又把自己的版本拉起”的互顶。
此行为不取决于托盘是否显示、有无打开窗口、keepalive/activity 或是否有新 CLI。

### 3.2 推荐的平台实现

macOS / Linux：原进程在完成现有关停事务后，直接 exec 稳定安装入口的 `daemon run`。
macOS 沿用当前 launchd job / Aqua 上下文，不 bootout，不增加长期监督进程，也不把 daemon plist 改成无条件 KeepAlive。
Linux 用 executable::launch_path，而非可能已被替换后保留的旧镜像路径。

run_cmd 持有 daemon.lock，serve 返回“退出码 + 退出意图”；换新路径在 exec 边界才通过 close-on-exec 释放锁。
实施时须验证锁 FD 的 CLOEXEC，避免锁泄漏，也避免旧服务尚未清理就出现第二个服务。
关停阶段必须先停止/隔离旧后台写任务和监听者，持久化 registry、冻结卡记录、停止 iOS gateway、
关闭旧渠道、清理旧 socket/meta，再执行 exec；清理后的旧任务不能重新发卡、写回状态或重建连接。
任何常规清理不能延后到新服务启动后运行。

Windows：无 exec，使用一次性、有界的后台接班入口：旧进程在不可逆关停前成功创建接班进程，
接班进程等待旧实例释放单实例锁后，直接进入新 daemon 服务。
不得拿“连不上 socket”当作旧进程完全退出，也不得复用会 Replace 健康实例的抢占路径。
启动失败时不宣称交接成功；尚未完成关停时保留旧服务并明确报告错误。
接班入口绑定本实例及安装入口，不是任意命令执行接口；等待超时退出并记录原因，无无限后台重试。
仍需 Windows 原生验证，不能用 macOS 的结果冒充。

通用要求：

- 继续使用单实例锁与现有启动串行化。并发 CLI 若已启动同安装入口的健康后继，接班者接受该实例，
  不杀、不触发另一次指纹让位；已经打开新请求的后继不得被接班路径 bootout。
- exec 可能保持同一个 PID，因此成功判断看健康握手、非 draining 与目标指纹，而非仅看 PID 变化。
  当前 wait_for_drain 已接受“不再 draining”；要补同 PID 换新的回归用例。
- 同一次排空中再次安装，接班使用退出时安装入口的最新完整镜像；目标不可读/不可执行时不循环重启。
- 启动/exec 系统调用失败做短且有上限的重试并写清原因；最终失败保留 pending 与本地失败诊断，
  托盘显示“自动换新失败，可手动启动”，不能永远声称正在等待接班。
  新程序已成功接管后自身崩溃不属于本次自动恢复保证。
- 状态记录含本次交接标识、旧指纹、目标指纹、时间和失败类别；健康新服务确认后清除对应失败，
  不让旧记录覆盖后来成功的交接。
- 不新增普通在途请求的超时；最后一题几小时后才答完也必须自行接班。

## 4. 错误卡期限与 registry 活动状态分开

### 4.1 身份、期限与保留

错误处理的身份始终是 `(kind, sessionId, resultId)`；首次接受最终失败时就持久化 detectedAt/expiresAt，
不是首次投放或点击时才写。重复日志、结果重复上报、重启和失败重试都不得延后 expiresAt。
卡可以 pendingDelivery / awaitingAnswer / continuing / uncertain；只有明确处理或到期才结束。

`failure-cards.json` 继续作为未处理卡的唯一清单，不再用“registry 此刻必须 idle”代表卡是否有效。
卡匹配失败结果且尚未处理，registry 为 idle 或因无活动 ended，均保留；
新回合取代该失败、明确 sessionEnd、开关关闭/None 或到期则撤销。
关停 freeze 只保护因进程退出发生的收尾；新回合/明确撤销仍要可靠落盘。

### 4.2 失败回合不能被最近结束 10 条挤掉

仅修改 current 允许 ended 不够：结果被 registry 淘汰后仍无法安全继续。
为已有未处理错误卡，在 registry 的对应记录上保留一个有截止时间的引用，绑定 resultId。
普通 ended 历史仍保留最近 10 条；额外保留的错误记录数量受现有 MAX_CARDS=256 与 24 小时共同约束。
同一记录不复制成两份，不新增另一份可写的 lastTurnResult，也不把它复活为 working/idle。

保留引用随 registry 持久化；load/历史裁剪之前就要识别，不能先裁剪再尝试 restore。
卡清单为权威来源：启动时核对引用与卡，卡已处理/到期/结果改变时释放引用；旧引用不会无限保留。
新回合即使发生于 ended 状态，也更新同一记录并撤销旧卡，仍按回合 ID 防乱序。
首次恢复读取旧格式记录时，只迁移仍在保存清单中的卡；不扫描全部历史失败补弹。

### 4.3 不借错误卡刷新会话活动

真正来自 Agent、有人等待的普通提问继续按每 15 秒刷新，不改其有效期和活跃状态。
daemon 自持的错误卡及其接续反馈不再刷新所属会话活动，否则实际已停止的聊天会被卡伪装为活动。
restore 不靠 touch_activity 强行保活；卡的有效性检查和结果保留引用承担其寿命。
错误卡仍不计入排空 active_count，托盘待答仍计入 in_flight_count。

### 4.4 24 小时内的要求继续

区分“此 Agent 有原会话接续能力”和“此刻原生连接可发送”。支持原会话接续的卡保留要求继续；
临时离线、聊天没在窗口打开、单纯 TTL ended 不永久降级为只能知道了。
点击时才按当前原生状态、resultId、会话身份、排队操作和在途提问重新校验，不以旧快照保证发送成功。

- Cursor：复用现有允许 ended 聊天的发送服务；原聊天存在、Cursor 与桥可用、聊天已打开并且原生为空闲时提交。
  不自动打开 Cursor/聊天，不另建任务；桥的原子空闲检查保留。
- Codex Desktop / Pi：沿用原生的准确会话与代际、空闲及连接校验，不能因 registry 过期就拒绝仍可接续的原生会话；
  真实会话消失不能用新进程冒充它。其它未实现原生接续的 Agent 仍只通知。
- “知道了”结束该卡但不清除失败事实；用户主动新开一轮则旧卡失效。

当前普通 Ask 会先结束卡再调用控制服务，因此本次需要将“这次答题结束”和“错误已处理”分开：

| 发送结论 | 卡与后续动作 |
| --- | --- |
| 确认新回合已经开始 | 结束卡，registry 由新回合事件进入 working |
| 明确未发送（离线、聊天未打开等） | 保留原期限与失败身份，反馈具体原因，重新提供可操作卡；不自动发送 |
| 已提交但未确认 / outcome unknown | 保留待核实状态和原操作 ID，不自动重发；提示查看原聊天，核实前禁止新的发送 |
| 发现另一新回合 / 结果已变 | 旧卡结束，不把旧指令投进新回合 |

“可以重试”只适用于已确定没有发送的结果；不把 unknown 当作失败未发送。
每次用户明确的新尝试使用新 attemptId，网络重传复用同 attemptId；不能继续只用 resultId 作为操作幂等键，
否则第一次未发送的失败回执会永久挡住后续尝试。每个失败最多一个发送中/待核实操作，跨重启沿用持久回执。
unknown 的解除必须来自原生已开始证据或可证明未发送的终态；到期仍不重发。
反馈卡/重投放不新增错误、重置期限或重复推送“又出错”。

## 5. 可观察性和兼容

- daemon 日志区分：等待在途、开始接班、接班健康、接班失败；失败卡区分登记、排空延后、恢复、重试待核实、
  到期/用户确认/新回合/明确关闭而移除。日志只用固定类别和必要的匿名标识，不输出提问、答案或凭据。
- drain complete 的 held_count 只是已投放卡数，另记 saved/pendingDelivery 数，避免“0 张”被误解为没有待恢复错误。
- 新持久化字段有默认值。旧 saved 卡的 createdMs 作为已知首次登记时间，保持其余寿命，不在迁移时归零。
- 优先复用普通提问/反馈展示，不增加 iOS 专属操作协议。若需要新 IPC 枚举或修改旧端语义，按 overview 的规则升协议；
  仅 serde(default) 的增量字段不升。设计不预先宣称一定不升号。
- 这是含修复的 daemon 启动后才生效的保证：首次安装时仍由当前旧 daemon 控制退出，不能靠新 CLI 消除旧代码缺口。
  日常安装后明确核对运行指纹，不能仅以安装脚本退出成功作为修复已生效。
- 本次不恢复此前已经被旧版删除的错误卡，不主动触发两个故障 Cursor 聊天的接续。

## 6. 实施顺序与文档边界

1. 明确退出意图，覆盖主动指纹监听和 Hello 的有/无在途路径；补停止优先与单实例并发测试。
2. 实现平台接班与失败诊断；验证 exec 同 PID、旧资源收尾和并发 CLI 不互杀。
3. 错误卡拆分检测期限、投放与处理状态；registry 保留有期限的失败记录，TTL ended 不撤卡。
4. 接续临时失败可再次明确尝试；保留 unknown 防重放、原生代际和原子空闲检查；Cursor 全用替身。
5. 扩展真实隔离 daemon 冒烟与跨重启验证；再执行项目规定的安装和新二进制验收。

落点：daemon/runtime/mod.rs、daemon/spawn.rs、daemon/lifecycle.rs、file_lock.rs（如需要窄接口）、
daemon/runtime/failure_confirmation.rs、agents/registry.rs、agents/failure.rs、
daemon/runtime/agent_control.rs、agents/control.rs、app/gui_host.rs 及对应测试/隔离脚本。
接续能力与新 attemptId 的改动不应影响普通手机空闲消息的既有幂等契约。

实施后更新 daemon-graceful-drain、agent-failure-inbox、agent-lifecycle-results-and-recovery 的局部规则；
生命周期 spec 只补未处理错误的记录保留例外，普通一小时规则不变。
主 overview 仅在“退出接班责任/错误卡跨会话状态独立寿命”等全局不变量实际改变后做最小更新。
本草案不将现有实现文档提前改写成已实现。

## 7. 验证矩阵

| 场景 | 必须证明 |
| --- | --- |
| 无托盘、无等待 CLI，更新且无在途 | 新指纹 daemon 自行健康接班 |
| 一个长提问，更新，随后回答 | 旧提问正常完成，接班不依赖另一条新请求 |
| 同一次排空又安装一次 | 最终运行稳定入口的最新镜像 |
| 更新排空中 stop / force / SIGTERM | 不自启；不被周期监听重新覆盖 |
| activity 自然闲退 / auto-restart=0 / 仅客户端版本不同 | 仍按原语义处理，不被自身接班抢回 |
| 多个 CLI 与接班者同时启动 | 一个健康实例，无 bootout 健康后继，无新请求被切断 |
| 接班目标缺失、权限错误或接班创建失败 | 有界失败、可见原因，无无限等待/重启循环 |
| Unix exec 保持 PID | 等排空的 CLI 根据非 draining/指纹恢复，不继续死等 |
| 排空中合成三次失败，模拟经过 >1h | 记录可 ended，三张卡换新后均恢复 |
| 再制造 >10 个普通 ended 会话 | 未处理失败结果与卡仍在，可经替身接续 |
| 错误卡已投放，模拟 >1h | 不凭卡续会话活动，ended 不撤卡 |
| 初次检测后 23h59m / 24h | 前者保留接续，后者到期；重启/补弹/重试不延长 |
| 原聊天未打开，恢复后再次点击 | 第一次确定未发送且卡保留；第二次替身只提交一次 |
| 发送超时结果不明，重启再点击 | 沿用同一回执，不出现第二次原生提交 |
| 新回合、明确 sessionEnd、知道了、关开关 | 旧卡按原因移除，重启不补回 |
| 出错时挂着普通提问 | 普通题保持可回答，错误卡仍排在其后，不改答案/普通题期限 |
| iOS 重连与 Mac 错误卡恢复 | 同一错误不重复推送，未处理卡和操作回执一致 |

时间验证使用可注入时钟与持久化 fixture，不实等一小时/一天；隔离脚本用临时 HOME/ASKHUMAN_HOME、
伪造 Cursor 日志根、回环渠道与 CursorChat 替身。不得启动真实 Cursor、修改其真实 hooks 或向模型发请求。
功能实施后，先 node scripts/machine-health.mjs，再按项目规则测试、install.sh 和新安装二进制验收；
若使用并行 worktree，须先按 agent-worktree-setup.md 完成 Dev Instance 准备与 AskHuman 渠道选择。

Windows 接班、macOS launchd 与 Linux 更新入口均需对应平台验证；缺平台则明确写未验证，不缩减标准冒充通过。

## 8. 实施与验证记录（2026-11-05）

实现落点与设计一致；另补 channels/ios.rs、daemon/runtime/ios.rs、ios/gateway/requests.rs 的内部 quiet feedback 标记，
防止一次接续失败的反馈卡再次触发手机推送/前台无确认兜底。没有新增 IPC 或手机协议变体，无需 TestFlight。
持久清单的可选 attempt 和 attempted 分别保存待核实操作与“用户已尝试接续”的事实，重启后仍可安静地展示反馈。

验证均未向真实 Cursor 提交消息或触发模型。测试中的 Cursor 日志、会话 ID、bot 与 native sender 都是替身。

| 检查 | 结果 |
| --- | --- |
| 实施阶段 Rust 全量 | 2475 passed，11 ignored，0 failed；在最终 execve/静默反馈收尾前执行 |
| 最终代码相关 Rust 测试 | daemon、agents::registry、ios::gateway::tests_requests：325 passed、1 ignored、0 failed |
| CI 版本 clippy | Rust 1.99.0，all-targets + custom-protocol,development-ui + -D warnings 通过 |
| 最终 Dev install | install.sh 成功；build.json 确认程序与全部当前构建输入一致 |
| daemon-update-handoff-smoke.mjs | 五项通过：无客户端自接班、旧提问答完后接班、stop 优先、auto-restart=0、损坏镜像有界失败并留诊断 |
| 追加并发隔离场景 | 同一 drain 两次安装 + 新 CLI 同时等待，CLI 在同 PID 后继恢复，停止与失败路径也通过；基于上述 fixture 的临时扩展运行 |
| agent-failure-cursor-smoke.mjs | 十个场景通过；新增 TTL ended + 25 条新 ended 后恢复、明确未发送后的重新作答与原期限不变、跨停机满 24 小时不补卡 |
| iOS 反馈推送 | Rust 虚拟时钟测试证明反馈仍到前台收件箱，且后台/前台未确认均不产生新推送 |

损坏镜像的隔离验证发现：标准 exec 包装可能在 ENOEXEC 后退回 shell，无法给出接班失败诊断；
最终实现改用 execve，明确保留 ENOEXEC，并增加仅临时文本文件的单测（不会执行该文件）。

尚未做 Windows/Linux 原生接班验收，不以 macOS 结果代替；崩溃自动恢复与自动打开 Cursor 聊天仍不在范围内。
这次修复只会从含修复的新 daemon 起生效，当前日常 daemon 尚未替换；旧版已经丢掉的卡不自动补弹。

启动恢复补充回归：Cards 清单在 AgentRegistry::load 之前读取，其权威引用在遍历/裁剪任何记录前应用。
这样可恢复“卡已写入、registry 的 lease 字段尚未写入就退出”的旧格式/中断状态；临时文件测试让该失败记录
排在 25 条普通 ended 之前，证明冷加载仍保留它，不依赖两秒后的 restore 补救。
