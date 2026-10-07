# 提问气泡到达提醒：实施与验收

2026-10-07，用户已批准将 Web Demo 柔光瓷面方案正式接入并安装验收，明确在主工作树开发。
规格见 `../specs/popup-arrival-envelope.md`。

## 实现

- `inboxArrival.ts` 提供时序、弧线及可取消的并发动画控制器；各请求保持独立时间轴。
- `InboxArrivalNotice.vue` 负责透过输入的 SVG / 瓷面叠层，沿用已选图标轮廓、mask 和变形参数。
  Lucide ISC 声明保留在源码并通过 SVG desc 编入产物，不增加远程图标依赖。
- `PopupInboxView.vue` 在原生布局提交 / 非激活前置后启动提醒，按 Sidebar + 正文实际边界定位，
  只在启动时调整 Sidebar 滚动；查看、终态、离屏、隐藏及卸载有清理路径。
  关闭确认层在提醒上方；换题中的新请求等换题结束后一起播放。
- Rust Inbox Snapshot 补充 presented；保留非激活前置、最小化恢复和突发提示音，
  移除生产整窗缩放及其取消 / 交互注册。旧缩放文件仍供回归样例独立引用。

## 验证

- 新增 11 项单元 / Vue 集成检查：独立并发时序、10px 精确落点、目标失效与释放、
  当前 textarea 草稿 / 焦点 / 选区及组合态不被操作、固定画布与右侧预览排除、
  首次呈现门槛、冷启动突发与恢复不重播、终态取消、点开后的未读状态、导航滚动中止、
  减少动态效果与隐藏 / 卸载清理。
- 更新已有五次闪烁检查，验证落位的一次 640ms 高亮后接未读 ripple。
- `pnpm build` 通过；Rust `app::popup_inbox` 13 项通过。
- 完整前端 276 项和 Node 5 项通过；补充冷启动 / 恢复检查后，弹窗定向回归 35 项通过。
- `./scripts/install.sh` 通过，安装到主环境 `/Users/wutian/.local/bin/AskHuman`，正式签名成功。
- 通过已安装的 AskHuman MCP 发起真实验收；另用同一安装的 MCP ask 生成临时 A / B / C 请求，
  A / B 在 6.043s / 6.467s 连续到达，C 在 18.038s 到达，42s 后按请求撤回。
  用户选择「效果符合 Demo，输入保持正常，可以保留」。结束后 daemon 0 active，未强制重启或取消他人请求。

这次用户反馈确认原生动效与输入体验；关闭确认下层、预览排除、滚动失效及减少动态效果
另有实现和自动回归覆盖，未逐项取得单独的人工反馈。Windows/Linux、真实 IM 终态及 DPI/多屏
实机矩阵仍归统一窗口既有外部 gate，不将本次 macOS 反馈扩展为其他平台证据。

## 原生出现动画恢复（已完成）

用户反馈原先的 macOS 弹出动画消失，并批准恢复每轮首条的原生出现动画、保留非激活前置。
代码追踪发现：合并窗口初始提交 `799334f` 的 `PresentPopup` 绕过独立窗口的
`finalize_popup_show`，未应用 `general.appearAnimation`；`f5ab320` 移除首条也会播放的旧缩放
后，首条没有中央气泡，因此漏接原生出现动画的问题更明显。当前配置仍为 `alert`。

`popup_transition::appear` 在隐藏且未最小化时应用配置，调用不变 key 的 `orderFront:`
启动 AppKit 出现效果，再调用 `orderFrontRegardless` 非激活前置。普通可见到达仍走 `front`，
最小化恢复仍走系统 `deminiaturize`。Apple 对 animationBehavior 的说明只明确描述
orderFront / orderOut，因此单独设置属性不能替代原生体验确认。

新增 `popup_appearance_regression` 原生回归样例，使用生产显示路径，检查 Alert / None /
Document 配置、复用同一窗口下一轮、可见前置不重播、前台应用 PID、key 状态及 native frame。
样例只读回配置与焦点 / 几何，不将这些读数当作“动画已视觉播放”的证明。
最初样例在 WebView 创建后仍收到 Wry 的 NSApplication.activate 启动副作用，原有 front
对照组也在 400ms 后成为 key。样例改为创建阶段禁止自身激活、Ready 后转 Accessory，
排除启动激活后，原有 front 对照及 4 轮新显示路径均保持前台 PID / key / frame。
该探针验证的是已运行宿主的原生显示路径，不将其扩展为冷启动焦点矩阵的证据。
`cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 均通过；`./scripts/install.sh`
编译、签名并安装成功。一次 Clippy 与安装后的缓存回收并行导致 fingerprint 目录消失，
待安装结束后单独重跑 Clippy 通过，没有将该构建竞争记为代码错误。
实际清空待答列表后通过新安装的 AskHuman 再弹一轮，用户确认
「原生弹出动画已恢复，焦点正常」。真实体验与探针分别覆盖视觉出现和非激活显示属性，
Windows / Linux 的显示路径未调整。
