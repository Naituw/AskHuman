# 回复输入框循环增高调查与修复

日期：2026-10-29。

## 现象与根因

在本机 Linux / Wayland、WebKitGTK 2.54.1 环境中，普通带边框回复输入框即使没有换行，也会随输入逐次增高；退格也可能增高。达到 240px 后，部分输入会使其突然缩回。原生窗口尺寸不变。

`textareaAutosize.ts` 原有增长路径使用 `scrollHeight + computed borders` 判断所需高度。此环境中，实际外高为 70px 时，scrollHeight 为 69px，而 computed 上下边框各为 1px，计算结果为 71px。写入之后，下一次得到 72px，形成几何误差的正反馈。

增长分支提前 return，因此删除输入即使允许收缩，也先被误报的增长拦住。达到 240px 上限之后，增长分支不再成立，下一次允许收缩的输入进入隐藏镜像测量，使高度突然恢复到实际内容需要的大小。复位取决于输入类型与调用时机，并非随机计时器。

## 修复

保留普通输入无需增长时的快速返回；疑似增长也先用已有隐藏镜像确认真实内容高度，然后只写一次最终高度。允许收缩的输入直接完成镜像测量，不再被虚假增长分支截断。普通插入仍不收缩；live textarea 不经过临时 `height:auto`，避免已有的 WebKit 滚动锚定问题。

没有平台特判、新依赖、协议或数据迁移。修改限于高度 helper、相关测试与说明；提交内容、原生窗口几何、daemon、渠道和历史不受改动。

## 复现方法

在真实 WebKitGTK 页面加载项目 `tokens.css`、`base.css`、`controls.css`、`popup.css` 和转译后的 `textareaAutosize.ts`。建立固定宽度的普通 `.textarea`，填入不换行的短文本，反复调用 `resizeTextareaToContent(textarea, true, allowShrink)`，记录 `getBoundingClientRect().height`、scrollHeight 与 computed border。

分别以 `GDK_BACKEND=wayland` 和 `GDK_BACKEND=x11` 启动相同版本的 WebKitWebDriver / MiniBrowser。固定内容也可复现，说明不依赖字符数或换行。补充 WebDriver `insertText` / `deleteContentBackward` 验证真实 input 事件路径。

## 本机实测

| 环境 | 修复前普通带边框输入框 | 修复后 |
| --- | --- | --- |
| 原生 WebKitGTK 2.54.1，Wayland | 约68.333 → 70 → 71 → … → 109px（40次）；固定短文400次调用可见240 → 70px周期 | 400次混合允许/禁止收缩调用无累积漂移 |
| 同版本、同机，强制X11（XWayland） | 69px稳定 | 稳定 |
| Chromium 155 headless | 69px稳定 | 稳定 |

无边框 collapsible 输入框在最小页面中没有累积增长。修复后，带边框/无边框 × 普通/停靠的四组布局都保持短文稳定，长文上限分别为240px/120px，删除可恢复短文高度。

真实 WebDriver 输入中，修复前五次 deleteContentBackward 可令高度从114px逐次增加至119px；修复后无此逐次增加。运行安装脚本并使用新安装构建后，用户确认真实回复窗口「输入和退格正常，未再出现异常增高/复位」。

回归单测模拟 live scrollHeight 与边框合计比实际外高大1px，覆盖重复调用、允许/禁止收缩，以及误报增长时删除仍能收缩。原有尺寸stub未包含这种不一致。

## 验证边界与成本

- 这是本机 Wayland/WebKitGTK 2.54.1 的确定复现，不代表所有 Linux/Wayland 环境必现，也未定位上游引擎首次引入差异的提交。
- X11对照运行于XWayland，未验证独立Xorg桌面。macOS/Windows本轮未实机回归；macOS无异常来自用户既有人工体验，Chromium不能替代Windows WebView2验收。
- 中文输入法组合态、多屏/DPI、完整焦点/滚动矩阵没有完成实机验收。测试机环境包含1.5倍缩放相关设置，未做合成器真实100%输出缩放对照。
- 镜像确认增加疑似增长时的布局开销。本机最小页面300次调用的单次微测约2.24ms，无边框快速路径约0.007ms；这不是完整输入延迟或低性能机器保证。
- 无需为此改造composer或改变全应用显示后端。应关注聚焦、切题、语音更新及停靠/回位时也会调用此helper的事实。
