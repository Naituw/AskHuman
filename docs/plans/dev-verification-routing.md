# 日常沟通与开发验收隔离实施计划

> 需求：[日常沟通与开发验收隔离](../specs/dev-verification-routing.md)。
> 2026-10-30 用户确认设计后要求全部实现，范围包括两个阶段。
> 实施地点：当前主工作树；保留原有 `PROGRESS.md` 与 Cursor 调查文档的未提交修改。

## 实施顺序

| 步骤 | 工作 | 状态 |
| --- | --- | --- |
| P1 | 共享目标解析、兼容状态、固定内部子进程和新旧程序能力检查 | 已实现 |
| P2 | 安装构建记录、CLI `dev ask`、MCP `dev_ask`、验收结果及恢复 | 已实现 |
| P3 | Dev 卡片与窗口标识、屏蔽日常原生会话控制、取消和版本校验 | 已实现 |
| P4 | 正式渠道转送、请求授权和有界状态、附件与取消回传 | 已实现 |
| P5 | 独立 Agent 配置／测试会话、平台能力检测、子任务继承和清理 | 已实现；按客户端能力开放 |
| P6 | 开发测试设置页、工作树注册、迁移诊断／执行／回退和受管集成更新 | 已实现 |
| P7 | 跨进程回归、前端验证、安装、通过新 AskHuman 验收并更新文档 | 自动化与安装已完成；本机界面按用户要求延期 |

## 验证记录

macOS 本机已运行安装脚本，新安装程序能力探针返回 developmentVerification=1。当前本地验证如下。跨平台自动验证以 [PR #27 检查](https://github.com/Naituw/AskHuman/pull/27/checks) 的最新结果为准；用户已明确将本机界面验收延期并登记待办。

- Rust 全量回归：2272 条通过、11 条原有忽略；开发验收模块 18 条通过，包含附件持久化与授权边界。
- Clippy `--all-targets --features custom-protocol -- -D warnings` 通过。
- 前端：1078 条通过；Node 维护脚本 61 条通过、19 条平台跳过；生产构建通过。
- `scripts/dev-routing-smoke.mjs` 用实际新程序和临时 HOME 验证两工作树、正式／Dev／正式、稳定注册入口、
  冲突 home、继承目标冲突、MCP 生命周期内 marker 变化、Dev 重取与错构建拒绝、缺失／不可执行／旧程序、源码未重建。
  独立 profile 从中性工作区调用实际 MCP 时仍使用 Dev 工作树的待办；内部临时 daemon 关闭预热和 GUI，结束时清理。
- 同一跨进程用例准备 Claude／Codex／Grok／Pi 私有配置，无模型调用，日常文件逐字不变；再迁移六种真实受管配置，
  检查新正式入口与 Codex 信任；MCP／CLI 两组模式均通过。注入目录不可写后呈现部分完成并可重试；
  外部编辑会阻止回退，消除冲突后按原字节与权限恢复。
- Relay 单元覆盖稀疏多题、图片、无决策／非法选项、重复索引、授权工作树／构建／题目／过期与迟到结果。
- 原生界面：Mac 锁定，用户选择本次先完成代码和 CI，已登记解锁后的界面验收待办。真实远程渠道、各客户端登录后的完整原生会话，以及 Linux／Windows
  桌面交互需在对应环境实测，不能以配置准备或 CI 编译替代。

Cursor 2026.10.01 的 `CURSOR_CONFIG_DIR` 只改部分配置，实际 MCP 仍读 `~/.cursor/mcp.json`，因此完整独立会话不开放。
Antigravity 无可靠独立原生启动适配器，同样只保留基础验收。Linux GUI 启动需检测到受支持终端，否则可用 `--stdio`；
Windows GUI 启动需 Windows Terminal。Codex App 的生产原生桥在 Dev 内禁用，完整测试使用 Codex CLI 私有 profile。

客户端依据：[Claude settings](https://code.claude.com/docs/en/settings)、[Claude CLI](https://code.claude.com/docs/en/cli-reference)、
[Codex environment](https://learn.chatgpt.com/docs/config-file/environment-variables)、[Grok settings](https://docs.x.ai/build/settings)、
[Pi environment](https://pi.dev/docs/latest/environment-variables)，并核实本机客户端帮助与 Cursor 已安装包的配置读取代码。

## 实现约束

- 优先复用既有问答、渠道、回答格式、首答生效、进程断开取消和安装发布链路。
- 只用加默认值的可选载荷字段保持同协议兼容；确需新增可被旧端收到的消息变体时，按 overview 更新协议号。
- 普通 MCP 的进程内工具和子进程固定目标，显式 Dev 工具独立委托，不能临时改全局环境。
- 验收请求使用独立关联标识，不把日常原生 session 身份交给 Dev，daemon 也不能从祖先进程补回控制能力。
- 远程转送只借用正式渠道投递，不复制生产凭据；测试历史仍归 Dev。
- 独立会话仅使用经过核实的客户端隔离接口；不支持或缺少前提的组合报告实际原因。
- 迁移默认保护已有自动行为，记录各受管产物及部分失败，不覆盖自定义配置。
- 只对新行为增补有意义的测试；文档状态与实际完成、平台验证结果保持一致。

## 验收重点

以需求 §9 为验收清单，重点验证正式／Dev／正式连续调用、两工作树并发、旧 binary、
构建与 daemon 不匹配、MCP 生命周期内 marker 改变、取消及重取、远程迟到回答、
独立测试会话的全套集成归属，以及迁移失败和回退。

macOS 原生环境在本机验证；Linux／Windows 的可用编译、测试和 CI 分别记录。
不能以跨平台代码存在替代原生实测结论。
