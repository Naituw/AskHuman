# 多请求统一作答窗口实施计划

需求见 [Spec](../specs/popup-request-inbox.md)。2026-10-06，用户通过 AskHuman 验收
无回弹的原生 demo，明确要求制定计划并开始正式实现。本计划覆盖真实请求链路，
不以 mock demo 的结果代替产品验收。

## 1. 实现边界

每个 Daemon / Dev Instance 至多一个独立 Popup Host，承载一个作答窗口。保持 Daemon
无 GUI、GUI Host 的设置/历史/托盘职责、CLI/MCP 逐请求等待及 stdout/退出码契约。
Daemon 的 RequestRegistry、Ask/Confirm Coordinator 和各 IM adapter 继续持有真实请求。

采用 Daemon 与 Popup Host 的一条用户私有 IPC 连接，按 request ID 复用 Show、提交、
取消、终态和来源更新。宿主启动由 Daemon 串行预留，使用一次性随机凭证与宿主代次；
同时到达不会 spawn 多个可见窗口。旧宿主回调不能清除新宿主的连接/请求归属。

现有冷/热 Helper 是一次请求一次进程，GuiBridge 提交后立即关闭整个窗口；这些规则不能
复用于共享宿主。新的请求容器明确区分 pending / submitting / terminal，结果以 Daemon
确认的首个终态为准。无效提交保留表单并报错；自提交成功或外部终态只移除对应 ID。

## 2. 前端请求隔离

以请求为单位建立 PopupContext，复用现有普通题、多题、权限、Stop、语音、待办、查找、
回复附件和预览组件。首次打开时创建正文；已打开的正文保持挂载，切换只隐藏/显示，
保留草稿、选项、IME、编辑器焦点/选区、题内进度、正文滚动及阅读状态。

只允许当前请求响应窗口级键盘、粘贴、拖放和原生预览事件。提交、权限 diff、附件读取、
历史/Agent 上下文等操作均携带 ID；异步结果再次核对请求和读取代次，不回退到“当前 ID”。
隐藏请求不能改变主题/语言、自动聚焦、打开原生预览或提交另一条请求。

导航复用已验收的项目分组、组内到达顺序、同项目下一条、持久蓝点和草稿标记。
窗口级关闭由统一容器处理；取消层固定 ID 快照，按各请求类型计算后果，发送中防重复。
非当前外部终态只更新列表，当前终态短暂提示后选择下一条，末条按最新队列关闭。

## 3. 几何与提醒

扩展现有 popup_preview 原生几何所有者为左队列 / 中正文 / 右预览的共同事务。
正文初始尺寸沿用配置和正常尺寸偏好；Sidebar 默认 240、最小 180，右预览默认 700、
最小 320。先缩预览、再缩 Sidebar、最后缩正文；自动受限和提醒不写入正常偏好。
保留手动移动、外缘/分隔线调整、最大化恢复、DPI 和多屏对账。

macOS 将 demo 的旧画面覆盖、绘制确认和单次原生 frame 提交纳入正式事务，避免左扩窗
的中间跳动。提醒用 SkyLight 合成变换与 CVDisplayLink，保持已验收的 2.2% / 540ms
无回弹曲线。完成、重置、终态、销毁、异常都恢复捕获的原 transform。
按减少动态效果和运行时能力降级；不永久置顶，不激活其他应用，不改变当前请求。
Windows / Linux 使用自身原生的非激活显示能力，位置/前置服从窗口管理器限制。

附件内容缓存保持有界；阅读模式、位置与缩放不依赖缓存字节是否被淘汰。共享宿主只有
一个活动原生预览，请求切换先撤销旧原生视图/读取，再显示新请求的对应内容。

## 4. 生命周期与故障

待答期间宿主常驻；空队列先结束本轮窗口，再按已有 popupPrewarm 配置决定预热或退出。
预热保留单个隐藏待命窗口；关闭时完全清理本轮请求状态及 Sidebar 展开标志。
空队列与新到达的竞态通过宿主代次和 Daemon 停止确认解决，不能用迟到“最后一条”误关新题。
宿主/预热 IPC 不独自保活 Daemon；在途请求仍由 CLI 等待连接保活。

宿主异常退出后的推荐方案：Daemon 有界重启并重放尚未终结的请求，不延长权限 deadline、
不生成用户批准/拒绝、不重放已提交结果。连续启动失败沿用渠道故障契约：保留可用 IM，
没有任何作答面时返回明确失败。首版草稿只保留内存，重启后提示其丢失，不自动落盘。
用户已通过 AskHuman 确认此故障体验；缩放不可用时保留统一窗口、前置及蓝点/高亮，
跳过整窗缩放，不阻塞作答，也不展示私有接口的实现错误。

drain / 二进制换新 / popup 配置启停遵守现有 Daemon 生命周期；先完成在途请求，
再回收宿主和预热，避免旧宿主连接或派发回调阻止换新。

## 5. 实施与验证顺序

1. 建立共享宿主协议、启动状态机和请求路由；覆盖并发启动、代次、连接失败和终态竞态。
2. 接入真实请求容器和按 ID 寻址的命令；实现 Sidebar、独立状态、统一关闭及提交确认。
3. 接入三区原生几何、已有附件组件和已验收提醒；补齐状态/资源清理。
4. 完成预热、托盘指定请求聚焦、来源更新、异常恢复与 drain 集成。
5. 运行相关 Rust / Vue 测试、类型检查和生产构建；执行 ./scripts/install.sh 到隔离 Dev Instance。
6. 用安装后的真实 AskHuman 逐项验收普通/多题/权限/Stop 并发、草稿/IME/附件切换、
   关闭快照、新到达与末条提交、真实远端终态、宿主崩溃和更新排空。
7. 更新主 overview 的进程职责、Popup UI 专题地图以及被替换的预热/焦点规格；
   PROGRESS 只记录尚未完成的外部验收和具体下一步。

跨平台实机证据独立记录；没有运行 Windows / Linux 就不声称这些平台验收完成。
本 worktree 当前 popup-only；真实 IM 验证需使用用户确认的测试 preset，不能复用生产凭据。

## 6. 当前状态

共享宿主、请求隔离、统一关闭、三区几何、原生合成提醒、预热、恢复及托盘按 ID 路由
已实现；当前进行最终实机验收和回归。实施分支为 codex/popup-request-inbox，测试配置
为独立 popup-only Dev Instance，不使用生产 bot。

## 7. 初轮正式验证（2026-10-06）

已执行两轮 `./scripts/install.sh` 到隔离 Dev Instance；前端全量 219 项 Vitest、5 项 Node
检查和共享宿主/几何 6 项 Rust 测试通过。真实 Daemon 接受两条普通提问，只产生一个
Popup Host；CUA 输入 A 草稿，切换 B 并输入独立回答，提交 B 后回到 A，A 草稿保留，
Sidebar 在剩一条时保留。提交 A 后窗口隐藏，两个 CLI 各自输出正确的 JSON 回答，
Daemon active requests 为 0，预热宿主仍存活。此结果不代表权限/IM/恢复/跨平台已验收。

CUA 对裸命令行二进制没有可唯一匹配的 App 身份，因此原生自动化使用临时测试 bundle
`.askhuman-dev/AskHumanInboxReview.app`（`com.naituw.askhuman.popup-inbox-review`），从
bundle 内的实际可执行路径启动测试 CLI/Daemon。测试 bin 临时符号链接到该副本；安装
覆盖后需要重签测试 bundle，或先还原正常 bin。测试不切换或关闭生产 Daemon/窗口。

## 8. 扩展正式验证（2026-10-06）

- 全量前端 222 项 Vitest / 39 files 与 5 项 Node 检查通过。新增真实容器回归覆盖批量终态
  跳过已结束 successor、关闭快照排除后来请求、保留挂载草稿、terminal tombstone 防迟到重放。
- 完整 Rust 首轮 1241 passed / 3 ignored；新增宿主握手顺序、逐 ID ACK/拒绝保留 pending/
  重试与 tombstone 后，宿主相关 10 项通过。最终全量结果见下面后续记录。
- 真实 Daemon + CUA：F/G 关闭层打开后 H 到达，取消全部只取消 F/G，H 自动打开并成功
  回答；旧实现的“下一条已结束但终态事件未处理”竞态已修复并有独立回归。
- 强制终止测试 Popup Host：只恢复 H，显示草稿丢失提示，原内存草稿为空；恢复后提交返回
  正确逐请求 JSON。已结束的 F/G 没有重放。
- 普通题、通过私有 Dev IPC 提交的虚构 Permission 与 Stop 共用一个宿主。权限选择 Deny，
  caller 收到 confirmFinal(actionId=deny, sourceChannelId=popup)，接着打开同项目 Stop。
  关闭层分别显示取消 Question / 允许 Stop 结束；只取消当前 Stop 后普通题仍保留。
  Fixture 不执行任何真实命令，也不结束真实 Agent。
- 在普通题等待期间重新安装：显示更新排空条；提交普通题返回正确 JSON 后 Daemon drain
  complete，宿主退出。未抢先终止在途请求。
- 新修复：先排队 PopupHostAccepted 再发布共享 sender，避免并发 Show 越过认证握手；
  空闲宿主也订阅配置/更新；到达 sound/pulse 短时合并；轮次 generation 阻止迟到提醒。
  隐藏上下文监听在异步返回后清理，语音仅作用于活动上下文，pin 在同窗口所有正文间共享。
- 原生文档与 WebKit 属于不同 sibling。曾把 PDF/Quick Look bitmap 合入 WK snapshot，
  但慢速检查发现 PDF tile 文字丢失。当前实验版保留原生视图屏幕位置并延后最终坐标；
  过渡仍没有通过视觉验收，不能视为已完成。

CUA 同时存在同 bundle 的 GUI Host 时会选择到没有正文的宿主；自动化期间临时将此 Dev
配置的 menuBarIcon 改为 off，原配置保存在 .askhuman-dev/test-config-backup.json。收尾恢复
该字段及普通安装 bin。此调整仅用于隔离测试，不修改生产配置或产品默认值。

## 9. macOS 验收与后续 gate

最终完整 Rust 回归为 1245 passed / 3 ignored，类型检查和安装通过。安装脚本会清理旧
编译缓存，因此不得在同一 target 中把 install 清理与 cargo test 并发；一次 Swift module
cache 被清理的构建失败，单独重跑后全量通过，未改变 SDK 或编译器配置。

真实 native PDF 三栏、macOS Window → Zoom/恢复、最小化后请求 K 到达并恢复窗口已验证；
当前仍是 I 的 PDF，K 保留未查看蓝点，恢复后的原生窗口处于非 key 状态。尺寸偏好在
上述过程前后均为 main 560×620 / preview 700。正式非激活前置与私有 transform 使用
已验收的原型同一路径；完整原型 native probe 记录见 Spec §13–14。

用户通过 AskHuman 接受正式体验，确认本轮完成 macOS，将真实 IM 和 Windows/Linux
实机/DPI/多屏验收留作后续，已记录项目 todo #4（a7d02da0-72d5-4191-954d-d0bcb77d322e）。
用户进一步要求一个可以反复显示/隐藏 Sidebar 的测试窗口，以检查展开时闪烁。
为此仅在 Dev Instance 且 ASKHUMAN_INBOX_LAYOUT_REVIEW=1 时追加 layoutReview URL，
显示几何测试按钮；按钮调用当时的 prepare/commit 与原生遮罩，保留同一正文和草稿。
正常窗口仍遵循 Sidebar 展开后保持到本轮全部答完的规则。

后续慢速验收被反馈 Sidebar 先在旧窗口出现、正文逐帧闪烁、Preview 最终错位。
拆分 prepare / frame commit / final paint ACK、固定靠右正文、隔离重叠绘制和确认 Preview
最终位置后，用户仍观察到正文轻微横向抖动、分割线明显抖动。正式几何 gate 未完成。
用户要求先做独立三栏基础 demo；分析见 `docs/investigations/popup-pane-stability.md`。
该 demo 不接入正式路由。此阶段的待授权状态随后被下面的用户指令取代。

## 10. 固定画布正式接入与可选旧模式

用户表示基础 demo 看起来没有问题，要求电影期间自主继续验证并正式实现，同时增加
默认合并、可切回独立窗口的设置。基础验证与机制分析见
`docs/investigations/popup-pane-stability.md`；旧截图/原生遮罩方案已从正式路径移除。

固定 WebKit viewport 与原生 PDF/Quick Look 共同置于 PopupCanvas。只改变窗口裁切与
画布原点；按 backing pixel grid 及实际 NSWindow frame 补偿正文位置。220ms 普通过渡，
减少动态效果时跳过；主区变宽、系统缩放和受限空间采用独立布局更新。修正 macOS
Tauri inner_size 读取虚拟 WebView 的差异，并仅在真实外缘拖动时记忆窗口尺寸。

实际模式切换验证：A/B 在共享队列保留草稿，切独立后 C/D 使用两个旧 Helper；切回
合并后 E 加入原共享 Host。A/B/E、C/D 分别返回正确逐请求 JSON；A/B 草稿保持，剩余
一题仍保留 Sidebar，队列清空隐藏。预热仅补充当前模式，旧模式在途窗口保留。

CUA 测试 bundle 现在为普通安装二进制的副本，不再把安装 bin 改为符号链接。测试只
作用于本 worktree 的 popup-only Dev Instance。最终安装、系统 Zoom 回归和设置界面
检查结果在收尾记录，不把旧实验日志算作固定画布通过证据。

## 11. 固定画布收尾验证（2026-10-06）

- 最终全量 Rust：1246 passed / 3 ignored；前端 227 项 / 40 files，Node 5 项通过。
  类型检查、前端 production build 和 `./scripts/install.sh` 成功。
- 正常 220ms 打开 PDF 和展开 Sidebar：19 / 18 个原生采样，最大间隔约 20.1 / 19.7ms；
  模型、WindowServer、CALayer presentation 正文 X 偏差均为 0，PDF X 保持原位，
  WebKit viewport 始终 4174×620pt。慢速 700ms 与基础 demo 的证据单独保留。
- Window → Zoom / 还原：正文、原生 PDF 和草稿正常；偏好始终 main 560×620 /
  preview 700。修复了 Zoom 中间事件误写正常尺寸，使用 AppKit inLiveResize 识别
  真实外缘拖动，系统临时分配不保存。
- 最小化后新题到达：恢复原共享窗口，当前草稿/PDF 保留，新题仅增加未查看蓝点。
  CUA 对非激活应用的合成鼠标事件不激活 NSApp；临时原生事件诊断确认落点命中正确
  WKWebView、窗口并未忽略鼠标，失败点击时 active/key 均为 false。用可访问性编辑与
  键盘提交验证恢复后的作答链路；不以合成点击代替物理鼠标激活证据。诊断 monitor
  已移除，正式代码不为自动化限制改变点击行为。
- 测试副本意外调用语音时曾因缺少 NSSpeechRecognitionUsageDescription 被 TCC
  终止；生产 Info.plist 已包含相应声明。测试 bundle 补齐同样声明。Daemon 只恢复
  未答 B，显示内存草稿丢失提示；已答 A 没有重放，B 重新输入后正确返回 JSON。
- Settings Review 实机检查中英文「作答窗口 / Answer windows」；点击独立/合并后
  config.json 分别保存 independent/merged，提示说明新请求生效、在途草稿保留。
  实际混合模式并发的逐请求 JSON 验证见 §10。

测试请求均已收尾，Dev Daemon active requests 为 0；测试 bundle/GUI Host 退出，普通
安装 bin 保持正常文件。临时语言、菜单栏、预热及尺寸字段恢复本轮备份，windowMode
保留默认 merged。生产环境、IM 凭据和主工作树未改动。本地完成不表示已发布；真实
IM 和 Windows/Linux/DPI/多屏仍按用户同意的 todo #4 后置。

## 12. 普通外缘缩放与过渡布局交接（2026-10-06）

用户实际外缘拖动反馈 Sidebar 吸收宽度和正文抖动。已确认策略：预览关闭时正文吸收
外缘宽度变化；预览打开时正文固定、预览吸收变化；Sidebar 只由分隔线调整。
固定画布只用于 pane 过渡，普通模式恢复 AppKit / WebKit 同步 autoresizing。

前端先固定实际 DOM 宽度，再扩展原生画布；过渡完成先在前端仍固定时恢复原生
响应布局，最后解除 CSS 固定。保留相同正文坐标原点，PDF / Quick Look 随父视图
同步缩放，不在每次 Resized 事件中异步重做 prepare。macOS 几何读取实际 outer_size，
用 AppKit inLiveResize 区分外缘拖动和系统 Zoom 等临时分配。

- 独立原生 harness 84 项通过（12 项 pane 交接 + 72 项正常宽高缩放）；模型、DOM、
  WebKit 和原生 PDF 交接偏差为 0，草稿、选择和滚动保留。
- 完整 Rust 1249 passed / 3 ignored；前端 228 项 / 40 files；类型检查和安装通过。
- CUA 合成外缘拖动没有改变正式窗口尺寸，因此以上程序化结果不当作物理鼠标
  持续拖动观感证据。用户复看后继续要求 UI Demo 调整，未补充新的外缘拖动结果。

## 13. 10pt 未查看蓝点正式接入与收尾（2026-10-06）

经过独立 UI Demo 迭代，用户确认固定实心蓝色动画，并指定正式实现为 10pt、
「只改这个圆点，其他部分不要动」。正式改动只在原 6pt 占位内绘制 10pt 伪元素，
保持标题、来源行和间距；2.8s 动画只改变两个不透明蓝色，无大小、透明度和光环变化。
查看后清除蓝点，减少动态效果时保持静态。Demo 中即时展开和移除附件双击等交互
没有复制进正式实现；此前已安装的 §12 缩放修复保留。

`./scripts/install.sh` 已成功安装普通签名二进制到隔离 Dev Instance。
真实正式窗口浅色和深色冷启动显示检查通过；新提问不切走当前条目，打开条目后
未查看标记清除。浅色 A/B/C 和深色 A/B/C 各自提交返回正确逐请求 JSON；浅色中
切换和自动切回后 A 的原草稿保留。最后一题提交后窗口隐藏，Daemon 为 0 active。

验证中另发现合并宿主实时切换主题没有同步原生背景：浅色→深色时前端更新但背景
仍浅色，深色冷启动正常。ConfigChanged 分支缺少旧 Helper 已有的原生主题处理。
该额外问题在本次蓝点交付时未改动；用户随后选择单独修复，结果见 §14。

测试请求全部完成，测试 Daemon / Popup Host 和独立 Demo 退出；临时主题、语言、
菜单栏、预热及 pane 尺寸恢复本轮 human-review 备份，合并模式保持。普通安装 bin
是正常签名文件，主工作树没有改动。真实 IM / Windows / Linux / DPI / 多屏仍按
用户已同意的项目 todo #4 后置；当前结果不表示发布或这些外部 gate 已完成。

## 14. 合并宿主原生主题热同步（2026-10-06）

用户在上轮交付后选择继续修复实时主题背景。共享宿主的 `ConfigChanged` 分支现在
先调用已有 `commands::apply_theme_to_windows`，再应用材质并发出前端配置事件；与旧
独立 Helper 一致。修复仅补齐该调用，不重建窗口或改变请求、蓝点及其他交互。

- 共享宿主/几何相关 Rust 7 项通过；`./scripts/install.sh` 编译、production build 和签名安装成功。
- 在真实安装副本的同一宿主进程（PID 37416）中，模糊材质浅色→深色→浅色→跟随系统
  （当时系统深色）均正确；纯色材质的浅色→深色→浅色也正确。CUA 截图确认背景和
  文字同步，A 一直保留原草稿，B 的未查看标记在打开前保留、打开后清除。
- 在 B 中另输入草稿并切回浅色后提交，自动回 A 的原草稿仍在；两个 CLI 输出分别
  精确为 `theme draft B remains independent` 和 `theme draft A survives all switches`。
  最后一题提交后窗口隐藏，Daemon 为 0 active，测试 Daemon/Host 正常退出。
- 临时 theme、menuBarIcon、windowEffect 恢复本轮备份，安装 bin 保持普通签名文件。
  未改变生产配置或主工作树；仅追加真实 macOS 热切换证据，不扩大其他平台验收结论。

已删除 PROGRESS 中该修复项；真实 IM / 其他平台 / DPI / 多屏 gate 仍按原延期安排保留。

## 15. 新条目连续高亮三次（2026-10-06）

用户在正式安装后反馈新到提问只闪一次，要求闪三次。沿用原高亮颜色与每轮 1.2s
柔和淡出，CSS iteration count 改为 3，条目高亮状态保留 3.6s，避免原 1.2s 定时器
提前终止后两轮。10pt 蓝点的独立颜色动画及其他交互保持原样。

组件回归 7 项通过；新增计时回归覆盖第一/第二轮后仍高亮、第三轮后恢复、未查看
状态与当前问题独立。安装到 Dev Instance 成功；真实新题 B 到达后 A 草稿保留，
高亮结束仍保留 B 的未查看蓝点，提交 A 后自动打开 B 并清除其蓝点，两个 CLI 返回
对应输入，最后队列归零/窗口隐藏。测试 Daemon 正常退出，临时菜单栏配置已恢复。
