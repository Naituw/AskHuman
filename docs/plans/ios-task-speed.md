# iOS Codex Fast 模式实施与验证

需求与决策见 [规格](../specs/ios-task-speed.md)。实现与本机验证已完成（2026-10-31）；用户要求直接在主工作树实现，不创建 Dev worktree。

## 实施

1. Mac：独立速度枚举、动态清单解析、共享启动参数、CLI 配置覆盖、桌面创建与首轮接管、记录哈希及幂等。
2. iOS：能力字段、每次关闭的开关、不可用提示、进度摘要、可靠会话速度配置显示；速度不持久化。
3. 验证：Rust 单测与网关、跨语言夹具、Swift 包、中英文模拟器；执行 `scripts/install.sh` 后用新构建问答验收。
4. 更新局部实现地图与原有模型规格，不改变主 overview 的全局架构。

不启动真实 Agent、不发送提示词、不产生模型费用；真实调用与 TestFlight 上传须获得相应授权。

## 验证记录

- Clippy 全目标 `-D warnings` 通过（第三方 vendored tao 的既有编译警告不属本项目告警）；`cargo fmt --check` 与 `git diff --check` 通过。
- Rust 全量：2285 passed、11 ignored；新增速度专属 13 项全部通过，覆盖配置覆盖、清单新旧格式、工作区预检、网关重试与冲突、CLI argv、桌面首轮与记录哈希。记录被清空完整性哈希但携带速度时也拒绝。
- Swift 包：459 passed、43 skipped（真实服务端到端环境未启用）；包含新增 5 项速度状态测试，以及原有断线重试用例新增的速度不变断言。Rust／Swift 共同读取新增速度线协议夹具。
- 模拟器构建通过；iOS 26.2 英文与中文各 3 项 UI 全通过（共 6 项），已检查 Fast 开关与标准／Fast 进度页截图。
- UI 测试点击原生开关右侧控件；SwiftUI 的整行无障碍框中心可能落在文字上，不能据此判断切换失败。iOS 27 初次运行停在 XCTest 等待崩溃日志，未作为通过证据，最终使用 26.2。
- 480 条中英文字符串目录已由 `ios/Tools/make-catalog.py` 从编译提取结果重建并检查。
- 最终主工作树 `scripts/install.sh` 已成功安装并签名至 `~/.local/bin/AskHuman`；后续正常 AskHuman 沟通使用该安装入口。
- 只读检查当前桌面 App 的原生 turn 参数构造：显式 `request.serviceTier` 优先于继承值；线程及首轮请求均已带同一覆盖值。

本轮没有启动真实 Agent 或调用模型，也未上传 TestFlight；自动测试不替代带真实凭据的桌面接管／CLI 推理验收。
