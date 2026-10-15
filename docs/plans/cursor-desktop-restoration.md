# Cursor Desktop 实验功能恢复与原生预检

2026-10-16，用户选择“恢复默认关闭的 Desktop 实验功能，并加入原生行为预检”。
前置证据见 [恢复前验证](../investigations/cursor-desktop-restoration-validation.md)。
沿用干净的主 checkout，不开展并行开发；CLI 接管继续延期。

## 已确认边界

- 恢复原先全局后台完成控制，默认关闭；CLI/MCP 自动集成门控、None 还原、普通提交和 Await 不变。
- 明确标为实验功能，说明依赖 Cursor 内部接口及启动 / 重载接入间隙，不扩大平台与自动化范围。
- 原始 bridge / 配置 / 设置 / 生命周期恢复后，增加已验证的 paired / 无 paired 契约分支。
- 原生行为预检必须先于原型修改与 active 状态；失败不改变真实聊天、不发送模型请求、不留下 guard。
- 保留生产配置的关闭偏好；安装遵守现有 graceful drain，不强杀其他在途提问。

## 实施顺序

1. 按撤回补丁的功能文件恢复既有模块和回归，保留历史调查记录及其它功能改动。
2. 将 renderer 拆为同步服务发现和解除调试暂停后的异步激活。原生预检使用新建的隔离 store /
   service、虚拟 composer 与受限依赖，模型边界一律拒绝；预检成功且所有权未变才安装包装。
3. 预检覆盖原生 pull / 同步 ACK / nack、Shell / 子代理真实状态标识、Await 排除、活跃轮次排队、
   Stop 后完成、in-flight 清理；paired 分支追加游标排序与缺口保护。接口调用次数和异步等待时间有界。
4. 加入预检失败、异步 ACK、意外模型调用、超时、取消 / 重载 / 还原竞态、契约漂移和状态准确性回归。
   现有真实处理器不增加逐会话结束状态，验证元数据随窗口状态返回。
5. 更新中英文实验说明和当前规格 / overview；跑针对性与必要全量检查，运行 `./scripts/install.sh`。
6. 安装后在拥有的隔离 Cursor 进程验证编译内嵌的真实接入与预检，核对六个版本 / 界面组合；
   不因验证临时开启生产配置或对原用户 Cursor 注入。核心完成处理保持已验算法时，复用此前
   真实 Grok 4.7 / 人类 End 证据；若其行为改变或出现新疑点，再补必要真实模型场景，模型仅 Grok 4.7。
7. 审核失败清理和恢复路径，提交实现；用新安装的 AskHuman 完成任务交接。

## 审核重点

调试暂停期间不能等待预检 Promise；异步激活晚到不能重新安装已取消的 guard。预检不能借用
真实后台队列、会话或文件服务，不能以源码关键字匹配替代行为检查；未知依赖 / 异步确认必须拒绝。
重复接入和 Daemon 接管只复用相同 owner / revision / lease 的已验证包装，部分窗口失败不能报告全局 active。

本计划不要求解决启动 / 重载期间的保护间隙，不引入 Cursor 启动接管或 CLI 包装器。

## 实现与审核记录

- 恢复配置、Daemon 生命周期、CLI/MCP/None 门控、设置、诊断及还原路径。设置中英文均标为
  实验功能，说明兼容检查和重载接入过程；配置缺失仍为关闭。
- `renderer.js` 在 provider 暂停期间只发现服务并登记准备状态，`bridge.js` 解除暂停、移除
  断点并脱离调试器后才调用异步激活。成功激活前不替换真实原型。
- `preflight.js` 创建独立原生队列，使用同一个完成处理器工厂验证行为。同步队列接口调用
  有预算检查，原生调用之间检查 1000ms 截止时间，异步激活另设 1500ms 超时；这不是对未知
  原生同步死循环的强制抢占保证。拒绝 thenable ACK，超时或还原立即释放探针资源，晚到结果
  不能安装拦截，原型在检查期间被其它组件替换也会拒绝激活。
- 原生 dispatcher 继续负责 ACK、Await 排除与 nack、Stop 后结算和 paired inbox 游标。
  实际拦截仍只返回通知处理结果 `completed`，没有改写任务 success/error/aborted 状态，
  没有新增模型请求入口或逐会话结束状态。
- 状态带有 `verified` / `checks`，聚合要求每个窗口已验证；旧 revision 的状态缓存不可复用。
  适配器 revision 同时覆盖桥接、renderer 和预检代码，本轮为 `085b2d09c2dcf51e`。
- 自审覆盖：预检失败不遗留包装、取消后的晚到 Promise、未知依赖触及模型边界、异步 ACK、
  ACK 丢失和 in-flight 残留、Await 被跳过、重复重新排队的调用预算、paired 游标缺口，以及
  用户/其它组件调试器和包装的所有权。新增回归与现有生命周期回归一起通过。

## 自动检查与安装

- Rust 全量：1333 passed、4 ignored；前端 Vitest：56 files / 389 passed；Node：32 passed。
  Node 中 25 项覆盖 Cursor bridge / activation / preflight，另 7 项为已有项目回归。
- `cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`pnpm build` 和
  `git diff --check` 通过。最终原生测试同步修改后再次通过 Clippy / fmt。
- `./scripts/install.sh` 成功构建并签名安装本机 `AskHuman v0.14.4`。安装后 `doctor --json`
  返回当前适配器 revision、`phase: disabled`、`preferenceEnabled: false`、`active: false`；
  Daemon 已自然换新，无强制终止在途请求。读取诊断时 activeRequests 为 0。
- 安装后只调整 ignored 原生测试的重载同步与失败清理，不改变生产二进制行为。首次脚本在
  “已接入”日志后立刻打开 Inspector，与测试的端口关闭断言相撞；已增加明确的“可重载”
  同步点，并把测试内断言改为可进入最终还原的错误路径。该失败不计入通过矩阵。

## 安装后的真实 Cursor 原生验证

macOS 27.0.1 / arm64，使用编译后的 ignored 原生测试，仅指定自有隔离进程 PID；测试内嵌的
三个 JS 文件与安装版本 revision 一致。每个组合均验证首次接入、拒绝错误认证、关闭临时
Inspector、窗口重载后的再次预检，以及最终还原原生处理器。逐项结果见
[`cursor-desktop-restoration-product-evidence.json`](../investigations/cursor-desktop-restoration-product-evidence.json)。

| 构建 | 窗口 | 每代原生检查数 | 重载后恢复观察值 | 还原 |
|---|---|---:|---:|---|
| 3.14.27 | Agents | 6 | 3619ms | 通过 |
| 3.14.27 | IDE | 6 | 2528ms | 通过 |
| 3.18.25 | Agents | 8 | 3023ms | 通过 |
| 3.18.25 | IDE | 8 | 3021ms | 通过 |
| 3.24.12 | Agents | 10 | 3013ms | 通过 |
| 3.24.12 | IDE | 10 | 3013ms | 通过 |

3.14 走明确匹配的 direct ACK 分支；3.18 增加 paired inbox 排序和缺口检查；3.24 再增加
原生取消子代理丢弃、成功子代理仍交给拦截器两项。首次和重载后均 `verified: true`，测试消息
只进入隔离原生 store / 虚拟 composer，不进入真实聊天或模型。六个测试 profile 的模型请求
日志标记均无新增。表中重载时间是本机观察值，不是性能保证。

本轮没有重新发起 Grok 请求或伪造人类 End。实际完成处理器保持原算法，按已确认计划复用
[恢复前验证](../investigations/cursor-desktop-restoration-validation.md) 的 Grok 4.7 Stop、Shell、
子代理、Await 和主动继续证据，以及 [初版安装验收](cursor-desktop-background-followup.md) 的
真实人类 End 与 3.24 两条运行路径证据。paired cloud inbox 使用原生算法与合成文件名，
不能据此声称新增了真实云子代理端到端验收。

六个成功矩阵进程和首个时序冲突进程均已退出；每个窗口退出前确认自有 guard 不存在、原生
handler 已恢复。测试 manifest / socket、9229 监听和临时控制凭据均已清理。原用户 Cursor
PID 92826 保持原命令运行，本轮没有对它注入、重载或修改模型偏好。生产开关仍为 `false`。

恢复范围已完成。未来构建仍需通过结构和原生行为预检，启动/重载有接入间隙；终端 Cursor CLI、
其它平台和未测试的后端不在本次交付范围。
