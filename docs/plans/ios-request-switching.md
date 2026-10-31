# iOS 手动切换实施

2026-10-31 用户在设计交接后明确选择正式接入；继续当前共享主工作树，不改变 Dev／全局路由。已确认规格见 `../specs/ios-request-switching.md`。保留其他 agent 的输入光标修复及并行改动。

1. AppStore 统一请求入口，捕获手动浏览范围与稳定顺序；新请求追加，结束请求跳过，当前结束项保留位置。手动替换唯一详情页。
2. 按 G 安装右侧原生左右箭头组与最右更多菜单，旧系统回退；单条隐藏、边界禁用，切换前提交输入并收键盘。
3. 按请求保存／恢复阅读锚点，保留草稿与选择；附件准备状态归请求拥有，换页继续，不写入其他请求。
4. 给发送记录导航代次，已切走的完成回调不抢页；发送后的 `nextPending` 与自动前进开关保持原有全收件箱规则。
5. Swift 测试覆盖队列、筛选、状态变化、草稿与后台发送；中英文原生 UI 覆盖按钮／返回、阅读恢复、键盘／组合文字、确认与附件；检查 iOS 18.5、Release 与到达动效共存。
6. 更新局部实现地图与适当规格，运行安装脚本，使用新安装 AskHuman 验收和交接。此任务不包含 TestFlight 分发或真实付费 Agent 调用。

## 验证

2026-10-31 正式接入、自动检查与本机安装完成。设计样机不计入以下功能证据。

- Swift 全套：488 项通过，43 项真实服务测试跳过。`RequestNavigationTests` 新增 8 项，覆盖范围／稳定队列、跨 Mac 身份、结束锚点、草稿附件和阅读位置隔离、后台上传切走又回来不触发旧自动前进，以及原全收件箱自动前进规则。
- iOS 26.2 原生快切：英文、中文各 8 个独立用例通过（分批运行）；覆盖 G 的实际按钮位置和一次点按、单页返回、单条隐藏、草稿／确认输入往返、Web Markdown 长表单阅读恢复、真实 UITextInput marked text 提交与键盘收起、到达动效飞行期间快切，以及共用更多菜单忽略确认。
- 原流程另外 4 项中文 UI 回归通过：取消请求、权限确认选择闸门、发送回执与自动换页、新请求从顶部开始且不继承已读闸门。
- iOS 18.5 回退：按钮次序与单页返回、长表单阅读恢复、键盘与草稿 3 项通过。
- 实际模拟器 Dark 外观＋最大辅助字号：按钮与返回、更多菜单忽略确认 2 项通过。首轮测试已回列表但用屏外懒加载行作断言，修正为首个可见行后重跑通过；未因此改产品逻辑。测试后恢复模拟器 Light。
- Debug 与 Release（模拟器双架构）构建通过；本轮没有改 wire／IPC，未重复无关 Rust 逻辑测试。
- `./scripts/install.sh` 成功安装到 `/Users/wutian/.local/bin/AskHuman`。共享主工作树保持未启用 Dev，没有改全局路由或用户 Agent 集成。
- `git diff --check` 通过。更新局部实现地图、功能规格与中英文用户说明；总览的全局架构与协议不变量未改变。

本机日志：`/tmp/ah-switch-swift-all-first.log`、`ah-switch-ui-en-first.log`、`ah-switch-ui-en-followup.log`、`ah-switch-ui-zh.log`、`ah-switch-ui-zh-followup.log`、`ah-switch-ui-ios18.log`、`ah-switch-ui-dark-large-final.log`、`ah-switch-release.log`、`ah-switch-install.log`。原生关键帧在 `/tmp/ah-switch-native-shots/`；对外审阅副本在 `demo/ios-request-switch/output/ios-request-switch-*.png`。

边界：以上以 mock 后端验证，不调用真实 Agent，也不当作完整 iOS 渠道验收。系统媒体选择器里挑大视频并在转换中换页、真实网络后台上传、iOS 17 实机与实际 iPhone 体验尚未单独验收；附件转换及取消原有自动检查已随 Swift 全套通过，后台上传／结果归属有本轮 Core 集成检查。本任务未上传 TestFlight，手机 App 尚未更新。
