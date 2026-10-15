# Cursor Desktop 恢复前验证结果

2026-10-15，按已确认的[验证计划](../plans/cursor-desktop-restoration-validation.md)执行。
本轮验证已完成；生产功能仍撤回，CLI 接管继续延期。

## 结论

三个抽样构建可以共用原来的 bridge 和完成拦截器，只需给 3.14 的“无 paired inbox”
契约增加一个明确分支。六个隔离窗口的原生队列探针最终共 **60 项通过**；3.14 / 3.18
四个窗口的新一轮真实 Grok 4.7 测试均通过 Stop、Shell / 子代理结算、Await 和手动续聊。
这比此前仅检查旧版源码的证据更充分，支持继续投入 Desktop 方案。

建议下一步恢复为**默认关闭、明确说明限制的实验功能**，同时把原生行为预检接入产品：
只有结构与队列行为都匹配才报告可用。不能直接删掉 paired inbox 检查后发布，也不能把
这三个构建推成连续版本区间或未来数月的兼容承诺。功能恢复需要下一步单独决定。

本候选沿用原先策略：抑制所有后台完成引起的自动轮次，保留当前 Await 和主动消息。
它没有新增“仅在 Stop / AskHuman 结束之后抑制”的逐会话状态机。

## 环境与接入

macOS 27.0.1，ARM64。3.14.27、3.18.25 的官方完整 ZIP 已校验 CRC、包内版本及
`codesign --verify --deep --strict`；本机 3.24.12 同样通过签名检查。来源与哈希在
[机器可读证据](cursor-desktop-restoration-evidence.json)中，官方旧包来源沿用
[发行包调查](cursor-desktop-compatibility-evidence.json)。

每个构建分别使用独立用户数据、扩展目录和空工作区，以普通参数启动 IDE / Agents，
没有调试启动参数。身份核验后才对测试 PID 发送 SIGUSR1，通过 provider 闭包发现服务。
历史 bridge 来自 `a00a0cd`，本轮未改 bridge；renderer 仅增加 paired inbox 有 / 无的契约分支。
关闭临时 Inspector 后，私有控制桥仍能查询、处理重载和恢复原方法。

| 构建 / 界面 | 候选接入与原生探针 | 本轮真实模型 | 重载恢复观察值 |
|---|---|---|---|
| 3.14.27 Agents | 通过，8 项 | legacy，通过 | 8.032 秒 |
| 3.14.27 IDE | 通过，8 项 | legacy，通过 | 3.078 秒 |
| 3.18.25 Agents | 通过，10 项 | legacy，通过 | 3.839 秒 |
| 3.18.25 IDE | 通过，10 项 | legacy，通过 | 3.040 秒 |
| 3.24.12 Agents | 通过，12 项 | 复用此前真实验收 | 3.059 秒 |
| 3.24.12 IDE | 通过，12 项 | 复用此前 Agent Host / connect 验收 | 3.085 秒 |

3.14 的原始 guard 实机拒绝，错误为 `Unsupported background completion contract`。
候选要求该版本既无 paired 提交方法，又存在 completed 后直接 ACK 的分支；没有无条件跳过检查。
3.18 的原始 guard 已实机挂载成功。候选用于上述全部六个窗口。

重载时间只代表这些观察样本。3.14 Agents 曾短暂报告 provider 未就绪，随后有界重试成功。
重载到重新接入之间仍存在保护间隙。三个版本另各重启一个隔离进程，确认新进程无旧 guard、
可以重新挂载并恢复；没有把 renderer 状态跨进程持久化。

## 原生队列与语义差异

探针创建新的厂商 store / service，调用原生 dispatcher 和候选 wrapper；composer、命令服务和
文件服务为隔离夹具，模型唤醒边界被设置为一旦进入即失败。测试不发送模型请求。

共同覆盖：Shell 与子代理完成 ACK、Await 排除与原生 suppression、前台活跃时保留队列、
停止后完成结算且不进入唤醒、dispatch in-flight 清理。3.18 / 3.24 另调用原生 paired
提交方法，验证排序后的完成批次推进游标、未呈现的较早结果阻止游标越过缺口。
这些 inbox 证据包含原生分发、路径过滤及游标算法，但不等于真实云端子代理端到端验收。

3.24 的 `_hasLiveTurn` 新增 AI generation-controller 检查，探针补齐该隔离依赖；
候选 adapter 无需再改。该版本也会在用户停止状态下丢弃 `kind=subagent,status=aborted`
的通知，但 **成功的子代理和 Shell 完成仍到达完成动作入口**。两条分支均实测保留。
因此原生的这项改进不能替代对用户所述成功后台完成唤醒的控制。

## 本轮真实 Grok 4.7 结果

每个旧版窗口发送两条父消息和一个极短后台子代理，共 **8 次父请求、4 次子代理请求**，按 Cursor 的生成请求 ID 统计，不是底层推理调用或计费次数。
按聊天固定 Grok 4.7；临时请求观察器拒绝其它模型，记录实际 runAgentLoop 的模型和请求 ID。
再核对厂商请求日志、工具完成状态、Await 结果和子代理可见最终文本；取证不读取或导出思考内容。

| 构建 / 界面 | 父会话 | Await Shell | Stop 后完成 Shell | 后台子代理 |
|---|---|---|---|---|
| 3.14 Agents | `31bd7ee1-fefd-4046-92b1-9ecd0da521d7` | 66229 | 66230 | `2386f275-638b-495f-a1dd-713e2b5f56f0` |
| 3.14 IDE | `9d2948bf-8249-4269-9cca-e64e1624d04d` | 662189 | 662190 | `da03eccc-2a0c-4cff-9cec-d441038bb587` |
| 3.18 Agents | `afe11843-8c51-4a9f-b6cf-42bc8c9900e9` | 945319 | 945320 | `e0cb9951-0f90-422d-8cb1-8208b6510e71` |
| 3.18 IDE | `3f27cda1-4e96-4e39-a2e4-93554eb4eb27` | 302901 | 302902 | `e54e1264-fc66-4daa-abaf-66439cbbd542` |

- 四个 Await 都实际跨过等待阶段，返回原生 success / complete、匹配 `AH_AWAIT_DONE`、exit 0。
- 父轮次进入前台等待后，调用与 Stop 共用的原生 `abortChatAndWaitForFinish`，再释放后台 Shell。
  后台任务正常结算并 ACK；父状态保持 aborted，队列和 dispatch in-flight 均清空。
- 停止后的独立快照距 Stop 分别约 14.5、18.9、38.1、41.7 秒，均只有原父请求。
  之后手动消息返回 `AH_MANUAL_OK`，后台子代理返回 `AH_CHILD_DONE`；完成后没有额外父请求。
- 四个子代理均为 Grok 4.7；参数化模型名可能显示 `grok-4.7-low`。
  本轮四个窗口实际均走 legacy `streamFromAgentBackend`，未强制更改实验 gate 来制造 Agent Host 覆盖。

3.14 IDE 的额外 12 秒尾部采样与重载重叠，采样器因上下文消失而退出；完成结算快照、
子代理最终结果和请求日志已经保存。不能将该额外采样记为通过，也未据此重复消耗模型请求。
该窗口退出后终端文件已被厂商清理，后台 Shell 结算证据采用保留的工具状态和原生 ACK；
其它三个窗口另保留了终端成功输出 / exit 0 的副本。

本轮没有在旧版重复发出需人类作答的 AskHuman End。3.24 的真实人类 End、后台 Shell
完成后无自动请求，以及 IDE Agent Host / connect 的 Stop / Await / 子代理证据，复用
[此前安装验收](../plans/cursor-desktop-background-followup.md#3-安装后的真实-grok-47-验收)。
它们是明确的历史证据，不计入本轮 12 次请求。

## 清理与交付边界

六种窗口均在退出前成功恢复原生处理器，三个重启样本也成功恢复。9 个测试主进程已退出；
Inspector 9229 无监听，测试控制 socket 为零。原用户 Cursor PID 92826 保持运行，从未对其
附加、注入或重载。六个隔离配置中的 54 条临时认证记录已删除，保留测试聊天和取证材料。

本轮没有恢复产品模块、改写生产配置、替换 Cursor / CLI 安装或重启生产 Daemon。
仓库只提交计划收口、报告和脱敏结构化证据，所以未重新运行安装或整套应用测试。
主 overview 和已标记撤回的规格仍准确，无需改写。

原始私有材料在 `/tmp/ah-desktop-restore-qa/`；长期事实、请求标识、候选 diff / 哈希、
各窗口检查项、重载与清理结果已保存到上述 JSON。结论限于三个 macOS ARM64 抽样构建；
Windows / Linux、Intel Mac、真实 paired cloud-child、多月无人维护及升级时无保护间隙均未验收。
