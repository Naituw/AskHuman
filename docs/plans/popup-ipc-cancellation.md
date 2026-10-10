# Popup IPC 分段读取的取消安全

2026-10-10。用户提供 `popup-ipc-cancellation-handoff.md`，确认修复 IPC 及同类路径，暂不做
草稿持久化。当前工作区是 v0.14.3，不是交接来源的旧 0.5.3 工作区。

## 已确认的问题

共享宿主的 `handle_popup_host` 在同一 `select!` 中等待 `read_msg` 和 100ms 终态巡检。
巡检完成会丢弃本次读取 Future；`read_msg` 的局部 String 及 Tokio `read_line` 的部分字节
随之消失。下一轮只读到 JSON 后半段，解析失败后断连，GUI 退出，Daemon 按恢复预算重建宿主。
草稿只在旧 GUI 内存中，所以无法跨进程恢复。

对当前产品 `ipc/codec.rs` 重建交接复现，旧循环返回 `InvalidData`；不中断及保留同一次
Future 的两组对照成功。锁定 Tokio 1.52.3 的源码文档明确说明 `read_line` 非取消安全。
此缺陷已经证实，但现场没有保留当时的解析错误，不能把隔离复现当作该次现场的完整错误链。

审查发现独立 Ask / Confirm 弹窗的外部终态分支、Interject 的发送分支也会取消读取后继续
使用连接。Interject 预填查询超时后复用读端，同样需要保留半帧。

## 实现范围

- 新增持久 `ipc::MessageReader`，连接拥有 Tokio `Lines`，每次 `next_line` 取消后由该对象
  保留分帧状态。完整帧同步解析 JSON；wire 格式、空行、EOF 和现有单次读取接口保持。
- 共享宿主、独立 Ask / Confirm 和 Interject 的会继续读同一连接的路径使用该对象。
  不删除巡检，不通过延长周期、调整截图大小或 socket 缓冲掩盖问题。
- 断连日志区分 EOF、JSON 类别 / 行列、读写 IO 的 kind / OS code 和正常退役；共享宿主
  带 generation 和可取得的请求 ID。不格式化可能含用户值的 JSON / IO 错误，不记录答案、
  图片、一次性 token 或凭证。
- 首个终态生效、重复提交 ACK、外部渠道抢答、取消 / 超时、预热、恢复预算和 drain 保持。
  不新增草稿文件、恢复协议或跨进程草稿产品行为。

## 验证要求

- 正式测试调用产品 `handle_popup_host`，用独立 duplex 流、测试时钟和内存 ServerState；
  不连接生产 daemon、IM 或启动真实 GUI。测试确认前缀已经被消费，再跨过巡检并发送剩余内容。
- 比较文字、选项及落盘图片字节；同连接继续下一题并拒绝重复提交；另一题的外部终态仍被巡检
  清理，不破坏正在读取的答案。历史记录关闭，测试图片仅在随机请求临时目录内，完成后清理。
- 另验 EOF、非法 JSON、写端失败、Interject 查询超时与发送分支、跨 UTF-8 分段以及日志脱敏。
- 在主工作区开发，用私有安装目录及独立 `ASKHUMAN_HOME` 做隔离验证，不更改生产凭据、
  不强制重启生产 daemon。完成安装后通过新版本 AskHuman 交付复查。

## 执行结果（2026-10-10）

- 修复前先在实际 `handle_popup_host` 路径执行两条分段回归：前缀被消费后触发巡检，
  “队列未变”和“另一请求外部终结”两种情况均因断连失败；修复后两条均通过。
- 新增 10 项回归，完整 Rust 测试 1306 passed / 3 ignored；`cargo fmt --check` 与
  `cargo clippy --all-targets --features custom-protocol -- -D warnings` 通过。前端无改动，
  安装器根据 183 个输入文件指纹复用已有构建。
- `./scripts/install.sh` 成功安装到私有 bin；再复制同一产物到临时 QA app bundle，
  以独立 `ASKHUMAN_HOME`、popup-only 配置启动真实 Daemon / WKWebView。通过原生界面
  选择 787252 字节 PNG、输入中文文字、勾选选项并发送；最终文字与选项完全一致，PNG
  逐字节一致（SHA-256 `5f2f8d6bf982f24063ba0bfbcbce2c11ba9da235fba1dc2329bb55d9734afe4c`）。
  第二题继续发送成功，两次完成前后 Popup PID 都为 76399。该实窗测试验证完整发送流程；
  确保半帧跨越巡检的证据来自前述确定性产品处理器回归。
- QA Daemon 已正常 stop。验证发现现有启动兼容迁移仍会更新全局 Agent Hook 的可执行路径；
  `ASKHUMAN_HOME` 仅隔离 AskHuman 数据，并未隔离这些 Agent 配置。测试临时写入的 QA
  可执行路径已逐项精确恢复为 `/Users/wutian/.local/bin/AskHuman`，未改动其他 Hook 内容，
  也没有复制生产凭据或连接生产 IM。今后使用不同路径的 QA bundle 时，应额外隔离 Agent
  配置目录，不能假定独立 `ASKHUMAN_HOME` 已提供该隔离。
- 随后再次运行 `./scripts/install.sh`，成功编译并签名安装到日常使用的
  `/Users/wutian/.local/bin/AskHuman`。安装按既有 graceful drain 等待在途请求结束，未强杀
  生产 Daemon 或窗口；最终交付通过新安装的 AskHuman 发出。

本轮未提供 Windows / Linux 实机证据；传输仍使用原有跨平台 NDJSON，不改变协议版本。
草稿仍只在当前 GUI 内存中，持久化不属于本轮实现。
