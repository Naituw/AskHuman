# Agent 最终失败确认实施计划

> 状态：2026-11-02 用户批准按顺序全部实现；P0 能力结论已落实，P1–P3 实现完成，P4 Dev 安装、隔离链路与手机预选项验收通过；
> 同日合并进 `main` 并装为日常版，合并后的验证见 §10。真实容量错误与失败后的原生接续尚未实测。
> 2026-11-08：同一回合的失败被重放成多张卡，修复见 §11。
> 需求与已确认决策：[agent-failure-confirmation](../specs/agent-failure-confirmation.md)。

## 1. 实施前准备

- 若使用新／已有并行 worktree，先按 `docs/agent-worktree-setup.md` 经 AskHuman 确认渠道预设，
  完成 Dev Instance 准备。实际工作树 `HumanInLoop-failure-confirmation`（`codex/agent-failure-confirmation`），无预设、使用正式远程转送；初次安装及手机收到请求已确认。
- 保留现有确认偏好，先核对 lifecycle、Stop、集成 mode 与平台组合。
- 真实模型会话启动、发送指令、真实推送验收分别按既有约束取得授权；不测试真实 Cursor。

## 2. P0：Codex CLI 能力门槛

目标是在可用版本与运行形态下，可靠观察最终失败并仅向准确的原会话接续一次。

1. 对 0.163.0 或已核对版本做 schema／能力探测，定位原共享服务，不启动替代 app-server。
2. 区分 `willRetry=true` 中间错误与最终 failed；核对断线重连后的最新回合读取，避免漏报或历史补弹。
3. 核对订阅所需操作是否会保活、加载或改写原会话配置；只接入符合原会话所有权的路径。
4. 用隔离协议替身验证操作 ID、消息 ID、错误终态和真正开始的关联。
5. 核对队列自动派发、`queue/start` 与普通 `turn/start` 的差异，特别是检查空闲之后电脑抢先发起新一轮。
6. 必须证明旧「继续」不会变成新回合的插话，也不会排队留到后面执行；无法证明则保留仅知道了降级。
7. 断线／重试保持同一个操作身份，使用已有回执保护；不能重复执行会生成新 ID 的 `codex queue`。
8. 明确能力矩阵：共享服务、嵌入／`--no-daemon`、自定义 remote、服务版本不匹配和连接不可用。

P0 通过只表示某条路线可以实现，不代替真实 capacity 失败、手机或 CLI 端到端验收。
若需真实测试，先说明模型／会话／调用预算与验证步骤，再经用户批准。

## 3. P1：统一失败处理服务

- 从现有统一 `TurnResult` 接入最终失败事件；共享开关启用、主会话、可关联新结果时建卡。
- Daemon 统一去重与卡片生命周期；被动 Hook 只上报，避免阻塞事件回调或伪造 continuation 输出。
- 普通请求承载正文、单选与可选输入；新失败卡不挪用正常 Stop 的完成语义。
- 定义稳定结果／操作绑定；实现收到实际新回合开始、结束、过期与旧回合迟到时撤销。
- 错误卡被接纳后协调既有一次性结果通知，避免同手机双提醒，同时保留建卡失败时原有通知机会。
- 两个接续入口共用现有 agent control 回执；明确接受、开始、拒绝和结果不明的反馈。

## 4. P2：事件与开关接入

- `integrations/agent_stop.rs`：沿用已有偏好，增加 Grok 错误能力，区分可确认正常结束与仅可报错。
- `integrations/agent_lifecycle.rs` 与共享安装器：lifecycle × confirm × mode 完整 reconcile，
  不启用全量 lifecycle 也能安装必要的错误观察；保留他人 Hook，更新 Codex 信任身份。
- Claude/Grok `StopFailure`、Cursor `status=error`、Pi settled error、Codex Desktop 原生 failed 接入统一服务。
- Codex CLI 按 P0 结论实现观察／控制能力；不支持形态准确反馈，不用超时猜测失败。
- 普通完成确认继续走现有机制；用户取消与失败卡不会误走自然 Stop gate。

## 5. P3：作答与跨渠道接续

- 有能力：「要求继续」＋补充指令／纯文字，或「知道了」。
- 无能力：「知道了（该 Agent 暂不支持接续）」；临时失联显示具体原因。
- 复用普通 Ask 的 Popup、iOS 收件箱、IM 路由、抢答、历史和取消契约。
- iOS 沿用普通问题的提醒门槛；不要求手机发起资格。
- 设置及 CLI 帮助更新为「结束／出错时确认」；Grok 注明仅支持出错提醒，默认关闭。
- 新版 Mac 连接旧手机仍应能以普通问题完成作答；若必须升级 App，明确列出实际原因与兼容边界。

## 6. P4：验证与安装

重点测试行为边界，不只测试字段透传：

- 最终错误／重试中／工具错误／用户取消／未知状态矩阵。
- 不同来源重复终态、终态先到、旧轮迟到、新轮取消旧卡与多个会话相同错误。
- 原生操作竞争、双击、两个入口、回执丢失、不明结果保护及继续后再次失败。
- 共享开关关闭／开启、lifecycle 关闭、None 卸载、重启不补历史错误与 Grok 设置。
- iOS 普通 Ask 收件箱、空闲门槛、结果提醒去重及跨渠道首答胜出。
- Cursor 仅静态契约与隔离替身；不能把通过替身写成实机支持已验收。

实现完成后运行相关 Rust／前端检查；若改 Swift 再运行相关包与 UI 测试。
按项目规则 `./scripts/install.sh` 安装；Dev Instance 使用 `askhuman-dev` 的显式验收入口，
普通讨论仍经正常 AskHuman。真实渠道与模型验证只做获准的一轮清晰矩阵，不制造反复的手工验收。

## 7. 文档收口

- 功能实现后才更新旧 Stop 规格中「错误不发卡」的范围，以及生命周期结果／iOS 文档中的通知边界。
- 若改变仓库级能力或不变量，再更新主 overview；局部实现地图留在相应专题。
- 将尚未完成的实现或真实验收记入 `docs/PROGRESS.md`，完成后删除对应 section。
- 不把本轮 schema／源码调查写成实际 capacity 错误已被捕获或手机接续已验收。

## 8. 实施前调查证据（2026-11-02）

- 调查阶段已检查当时的 Hook、Stop、原生控制、iOS 投递和实际配置；这一阶段没有修改功能或配置。
- 当前实现相关回归：原生结果／接续判定 2 项、Hook 失败分类 1 项、iOS Agents／通知 42 项通过。
  这些验证描述的是现有能力，不是本计划新功能的测试。
- CLI 0.163.0 帮助、本机生成的协议 schema、官方 `rust-v0.163.0` tag 源码一致具有原生队列入口。
- 对本机既有 CLI 服务的只读连接返回 connection refused；没有为探测启动服务或发起模型请求。

## 9. 实施记录（2026-11-02）

- 用户明确要求「按顺序全部实现」，并选择无预设／默认正式远程转送；用户不在电脑前，人工验收需预选项，不依赖本机弹窗。
- P0：确认现有 queue 会自动派发、普通 turn/start 可能 steer，无法保证原子预期失败结果；按已批准降级仅开放 CLI 失败观察，不发送队列请求。
  原服务 socket 不可用不启动替代服务；隔离 proxy 测试覆盖握手、只读读取及拒绝 start/queue。
- P1–P3：统一错误卡、共享开关、Grok 错误能力、最小事件与信任、Pi 原因上报、待答优先、跨入口接续保护及 iOS 提醒去重已接入。
- 自动验证：Rust 2,336 通过、10 忽略、2 个真实 Cursor 测试显式排除；前端 1,079 通过；Pi 隔离脚本 6 通过；vue-tsc 与 Clippy 1.99 全目标零新增警告。
- `scripts/agent-failure-smoke.mjs` 已通过：私有 HOME + 非空 ASKHUMAN_HOME、真实 daemon 和 loopback Telegram，核对最终失败到发卡、确认保留失败、重复回调、新回合撤销、共享开关、关闭 lifecycle 与 None 卸载／恢复。没有调用模型或真实 bot。
- `scripts/dev-relay-smoke.mjs` 已通过：在已有实际双 daemon／loopback 出口测试中补充 daemon 自建错误卡、从 session binding 恢复私有 profile 授权、正式出口转送与重复终态去重；不读取日常 Codex 集成来判定 Dev profile 权限。
- Dev 失败卡经正式远程出口送达 iOS，用户选择预选「知道了（该 Agent 暂不支持接续）」；验证记录
  `b899a626-0f9e-48de-a950-501221342aec` 状态 completed、exitCode=0，回答内容已核对。
  此卡对应 source `3110877e152b…`、binary `aa9313849a90…`、测试 profile `6bc9b486-d4ba-473b-993d-fe51b1808d7a`；
  用户另经正常 AskHuman 确认确实在 iOS 收到并提交。随后仅补齐临时失联／接续拒绝的中文和英文说明，重新安装并跑相关检查。
  没有调用模型；这证明模拟终态→开发 daemon→正式远程出口→手机回答回传，不表示真实容量错误或原生接续已实测。
- 本次没有改 Swift、推送中继或 IPC 类型；现有 iOS App 可回答普通错误卡，无需为本功能另发 TestFlight。

## 10. 合并与日常版安装（2026-11-02）

- 实现会话在交接时被额度限制中止；用户在交接里选了「合并到 main 并安装日常版」，由另一会话接手执行。
  `codex/agent-failure-confirmation`（`13102995`）与 `main` 无分叉，快进合并，保持线性历史；未推送。
- 主工作树（非 Dev）`./scripts/install.sh` 装为日常版，daemon 无在途请求，随即换新。新 daemon 启动只迁移了 Pi 运行时扩展
  （上报 `errorDetails`；只开「确认」也上报生命周期事件；已在运行的 Pi 会话需重启才加载）。
  Claude／Codex／Cursor／Grok 的真实 hook 配置前后逐字节一致：它们都开着生命周期追踪，Claude 的 `StopFailure → turn-end`、
  Cursor 的 `stop`（新二进制对非自然结束也上报）已经把失败送到 daemon；Codex 无失败 Hook，靠 Desktop 原生终态。
  Codex 的 Hook 仍为「已信任」；Grok 出现新的「结束／出错确认」开关，默认关。
- 对装好的日常二进制重跑 `scripts/agent-failure-smoke.mjs` 通过（私有 HOME + 非空 `ASKHUMAN_HOME`），前后真实配置哈希不变，
  无残留进程。`vue-tsc`、前端全量与 Pi／脚本 node 测试通过；Rust 分两批在合并后的主工作树上跑：
  `agents::*`／`integrations::agent_*`／`ios::gateway`／`daemon::request` 共 274 项，`daemon::`（含
  `daemon::server_impl::failure_confirmation`）共 189 项，均 0 失败（后一批另有 1 项原本即 ignore）。
  该测试目标共 2,348 项，等于实现会话报告的 2,336 通过＋10 忽略＋2 项排除的真实 Cursor 用例，未重跑全量。
- 未实测：真实 capacity 失败、失败后的原生接续；有「工作中」的非 Desktop Codex 会话时对共享服务的真实观察。
  审阅代码时的一个推断（未触发）：`Observer::failed_turn` 连接失败不退避，共享服务 socket 残留而没人监听时，
  daemon 会每 5 秒为每个这样的会话起一次 `codex app-server proxy` 再退出。本机目前没有非 Desktop 的 Codex 会话，
  采样 20 秒未见子进程。
- 合并后按用户选择清理了 `HumanInLoop-failure-confirmation` 工作树（`dev disable --purge`、`git worktree remove`）与
  `codex/agent-failure-confirmation` 分支（已完全并入 `main`）。Dev 实例里的验证记录原始文件随之删除，
  文档中保留记录 ID 与指纹。

## 11. 同一回合的失败不再重复出卡（2026-11-08）

- 触发：用户在 iPad 收件箱里反复收到同一张「排查 520 TestFlight 崩溃」的 Codex 失败卡（`flagged for possible cybersecurity risk`），
  而任务早已结束。
- 只读调查（没有发起任何模型请求）：当晚 9 张 Codex 失败卡里，3 张对应真失败，另 6 张是重放。
  三个桌面会话各只有一个回合，失败时刻取自各自的 Codex 会话文件（11:49:48、12:53:28、19:15:47），之后文件再没被写过；
  每张卡的回合 ID 都是那一个；9 张用户都点了「知道了」，不是「要求继续」造成的循环。
  registry 里这三个会话现在的创建时间，恰好等于各自最近一张重放卡的时刻，说明记录是被重建的；已结束记录的寿命约一小时。
- 根因：重放的前提是 registry 忘了这个会话。Desktop 来源是电平触发的（每 2 秒读最新回合，failed 就当作刚失败），
  防重复只靠 registry 记录里的 `closed_turn_ids`，而记录空闲一小时转入「最近结束」，全局只留 10 条；被挤出后同一回合重新登记，
  `apply_observed` 又为每次接纳现生成结果 ID，于是 `Cards` 里按结果 ID 的去重也认不出它。与规格 §5「不因读到历史 failed
  就批量补弹旧错误」相悖。
- 做法（用户经 AskHuman 选定「同一回合只出一次卡」，在主工作树实现）：
  - `Cards` 增加已处理失败回合的账本：（Agent 种类、会话、原生回合 ID），最近 512 条，写入与卡片记录并列的
    `~/.askhuman/state/failure-turns.json`（原子私有写，换新保留）；它与卡片记录分开，卡片清空时不删。
  - `observe` 取到需要处理的结果后先查账本（`owes_card`），再看开关、绑定、静默与「当前」判断：
    没出卡的失败（开关关着）也算见过，之后打开开关不会补弹。无原生回合 ID 的结果无从去重，按新失败处理。
  - `restore()` 把 registry 里已有的失败回合补记进账本，覆盖账本出现之前的失败。
  - 新回合（含「要求继续」之后的新回合）回合 ID 不同，照常出卡；不改 registry、IPC、iOS，协议号不变。
- 没做：registry 把已结束的桌面会话挤出、再重新登记这件事本身（Agents 列表里这些会话的创建／最近活动时间会被刷新）。
  它不再出卡，但展示上仍有影响，需要另行设计，不在这次范围。
- 验证（2026-11-09，主工作树）：`cargo test failure_confirmation` 19 项通过，其中新增 8 项：同一回合换了结果 ID 仍只出一次卡
  （另一个回合、会话、Agent 照常出卡，无回合 ID 不去重）；用两个独立的 registry 复现「registry 忘了会话，同一回合带新结果 ID
  再被接纳」；账本跨 daemon 换新、不随卡片清空而消失；上限只丢最旧；损坏或字段缺失的记录被忽略；从 registry 快照补记；
  快照里的字段名；无文件时不落盘。`observe` 本身依赖本机的集成配置，没有单测，接线靠代码审阅与日常版上的观察（见下）。
