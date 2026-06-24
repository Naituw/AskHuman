# 开发计划：多问题弹窗改「纵向列表」

> 关联需求：`docs/specs/multi-question-vertical.md`
> 仅前端 `src/views/PopupView.vue`；单问题(n=1)路径保持现状。计划描述方案，代码以实现为准。

## 0. 现状摘要（改造起点）

`PopupView.vue` 当前多问题靠单题渲染：
- 状态：`current`、`chosenByQ/inputByQ/imagesByQ/replyFilesByQ`（按题数组）、`visited[]`、`allViewed`、`canSubmit`、`onLastQuestion`。
- 「当前题视图」用计算属性 `chosen/userInput/images/replyFiles` 读写 `*[current]`。
- 单 `inputRef`（当前题 textarea）；`autoGrow` 作用其上；语音/粘贴/选图/拖拽都隐式作用「当前题」。
- 模板：`<Transition>` 包一个 `question-pane`（`:key="current"`，左右滑动）；底部 `isMulti` 与单题两套 footer。
- 键盘：`onKeydown` 内 `⌘1-9`、`⌘[`/`⌘]`、`⌘↵`（下一个/提交）、`⌘W`。

## 1. 渲染结构改造（多问题分支）

- 多问题(n>1)：把 `question-pane` 的 `<Transition>` 单题渲染替换为 **`v-for` 纵向列表**，每题一个 `.q-card`（含：`Question i/n` 头部 + 分割线 + 题干 + 选项 + 输入区 + 图片缩略 + 回复文件）。
- 单问题(n=1)：保留**现状**单题渲染（不进入列表分支，不显示当前题高亮，维持自动聚焦/现有底部）。
- 共享 Message 区 + `msg-tools`（复制/源码切换）仍在列表上方，沿用现状。
- 每题正文按全局 `request.isMarkdown && !viewSource` 渲染（沿用 `renderMarkdown` + `onContentClick` 事件委托做代码块复制）。

## 2. 状态模型调整

- 保留按题数组 `chosenByQ/inputByQ/imagesByQ/replyFilesByQ`。
- 新增 `active = ref(0)`：**当前题**指针（与 textarea 聚焦解耦）；取代 `current` 作为键盘 / 高亮 / 角标目标（单问题分支不使用）。
- 新增 `seen = ref<boolean[]>([])`：替代/并入 `visited`；置位来源：①卡片底部哨兵进视口；②`active` 曾指向过该题（`setActive` 时 `seen[i]=true`）。
- `lastSeen = computed(() => seen[total-1])`：发送出现条件（多问题）。
- `canSubmit` 沿用（select-only 时每个有选项题需选一）。
- 移除单题滑动相关：`slideDir/transitionName/onQuestionEntered`（多问题不再用；如单题分支仍需可保留最小集，倾向直接移除滑动、单题无切换无需动画）。
- 列表模式下「当前题视图计算属性」`chosen/userInput/...` 不再适用（每题在模板里直接绑定 `*[i]`）；单问题分支仍可用或改为绑定 `*[0]`。

## 3. 每题作答绑定（列表内按索引）

- 选项：`@click="toggle(i, opt.text)"`；`toggle` 增题索引参数，写 `chosenByQ[i]`。`single` 单选语义不变。
- 输入框：`v-model` 绑 `inputByQ[i]`；`@focus` → `setActive(i)` + 展开；`@blur` → 内容空则折叠；`@input` → 该题 `autoGrow`。
- 文本框 ref：用**数组 ref**（`inputRefs[i]`）替代单 `inputRef`，`autoGrow(i)` 按索引取元素。
- 图片缩略 / 回复文件：绑 `imagesByQ[i] / replyFilesByQ[i]`；删除按钮带索引。
- 每题「添加图片」按钮：`pickFiles(i)`（记录目标题，回填 `imagesByQ[i]`）。
- 每题麦克风：语音写入目标题 `inputByQ[i]`（语音会话锁定发起时的题索引；切题/失焦保持写回原题，沿用现有「锁定题」思路）。

## 4. 「已看到」与滚动（IntersectionObserver）

- 每题卡片**底部**放 1px 哨兵 `.q-sentinel`（或观测卡片用 rootMargin 命中底部）。`root = .content` 滚动容器。
- `onMounted`（多问题）建 `IntersectionObserver`：哨兵 isIntersecting → `seen[i]=true`。`onBeforeUnmount` 断开。
- `active`（当前题）= 最后一个完整可见的问题：由 IntersectionObserver 维护「完整可见集合」，取最大索引；用户滚动时更新（但**键盘/点击 setActive 优先**，滚动只在用户滚动时回写，避免与键盘打架——实现上：滚动更新 `active` 用节流，键盘 `setActive` 直接置位并滚动）。
- 下一个/上一个：`goRel(±1)` → `setActive(active±1)` + `scrollIntoViewIfNeeded(card)`（`block:"nearest"`，对应需求 N2/N3「刚好完整可见、不强制贴顶」）。
- `setActive(i)`：夹边界、置 `active=i`、`seen[i]=true`、停语音/预览（沿用 `goTo` 的收尾），不强制聚焦输入框（保持折叠）。

## 5. 折叠输入框（仅多问题）

- 输入框默认单行：CSS 基础 `min-height` = 1 行（约 1 行字高 + 内边距）；`.expanded` 时 `min-height` 回到现状多行高度。
- 展开态 `expanded(i) = focusedQ===i || (inputByQ[i]?.trim().length>0)`；模板按题 `:class="{ expanded: ... }"`。
- `focusedQ = ref<number|null>(null)`：`@focus` 设值、`@blur` 清空。
- `autoGrow(i)` 仅在展开态生效（折叠态固定 1 行，不长高）。
- 单问题分支输入框维持现状（不折叠）。

## 6. 当前题高亮（待用户确认效果）

- `.q-card.active`：淡色高亮（初版方案：左侧 2px accent 竖条 + 卡片背景极淡 accent 混色 `color-mix`）。低调、不喧宾夺主。
- 仅当前题选项显示 `⌘1-9` 角标：`optionHotkey` 仅对 `i===active` 的题返回值（或模板 `v-if="qIndex===active"`）。
- 实现后交用户确认，按反馈调整（H1）。

## 7. 键盘（`onKeydown` 改造，多问题分支）

- `⌘1-9`：toggle **active 题**的第 n 个选项（原作用 current → 改 active）。
- `⌘↵`：`active<total-1` → `goRel(+1)`；`active===total-1`（末题，已 seen）→ `submit()`。
- `⌘[` / `⌘]`：`goRel(-1)` / `goRel(+1)`。
- `⌘W`：`requestCancel()`（不变）。
- 与 textarea 输入隔离沿用现有 typing 判定；语音 `⌘D` 作用 active 题。
- 单问题分支键盘维持现状。

## 8. 图片归属

- 拖拽：`onDrop` 读 `event` 落点 → `closest('.q-card')` 得题索引 → 回填该题（无命中归 active 或忽略，按 I 决策：拖拽必有落点，命中卡片即归该题；落在列表空白则归 active）。
- 粘贴：`onPaste` 看 `document.activeElement` 是否某题 textarea → 该题；否则归 `active`（I2）。
- 现有 `pickFiles/onPaste/onDrop` 改为携带目标题索引写回对应数组。

## 9. 底部 footer（多问题）

- 左：取消（不变）。右：上一个 / 下一个；`lastSeen` 为真后出现发送（取代/并列下一个，沿用现有 `allViewed` 出现提交的布局，把判据换成 `lastSeen`，把 `current` 换成 `active`）。
- 首题禁用上一个；`active===total-1` 时下一个禁用。
- 单问题 footer 不变。

## 10. 涉及改动点清单（均在 `src/views/PopupView.vue`）

- 模板：多问题分支改 `v-for` 纵向卡片 + 每题输入区/选项/图片/哨兵；footer 判据换 `active/lastSeen`；当前题高亮 class；仅 active 题显示角标。
- 脚本：新增 `active/seen/focusedQ/inputRefs`、`setActive/goRel/autoGrow(i)/expanded(i)`、IntersectionObserver 建/拆、`toggle/pickFiles/onPaste/onDrop` 带题索引；键盘改作用 active；移除滑动 `slideDir/transitionName/onQuestionEntered`（按需保留单题最小集）。
- 样式：`.q-card`(+`.active`)、`.q-card .q-header`/分割线、折叠输入框 `min-height`/`.expanded`、哨兵。
- i18n：如需新文案（例如折叠/提示）再补；预计沿用现有 `prev/next/send/submit` 等。

## 11. 任务顺序

1. 状态与计算属性重构（`active/seen/focusedQ/inputRefs`，保留按题数组）。
2. 模板多问题分支改纵向 `v-for` 卡片 + 每题绑定（选项/输入/图片/回复文件）。
3. IntersectionObserver（哨兵）→ `seen` + 滚动回写 `active`；`goRel/setActive` + 滚动定位。
4. 折叠输入框（`expanded(i)` + CSS）。
5. 当前题高亮 + 仅 active 题角标。
6. 键盘改作用 active（`⌘1-9/⌘↵/⌘[/⌘]/⌘D`）。
7. 图片归属（拖拽落点 / 粘贴焦点）。
8. footer 判据换 `active/lastSeen`；单问题分支回归验证。
9. `pnpm build` + `./scripts/install.sh` 实测：多问题纵向、键盘全流程、折叠输入、图片归属、看到/发送时机、单问题回归。

## 12. 风险与注意

- **当前题与滚动/键盘的竞争**：滚动回写 `active` 与键盘 `setActive` 需互不打架（键盘优先 + 滚动节流）。
- **超长问题**：用底部哨兵判「看到」，避免整卡可见率永不达 100% 导致发送不出现。
- **单题回归**：单问题路径必须逐项保持现状（焦点、输入高度、底部、键盘、滑动移除后单题无副作用）。
- **数组 ref 生命周期**：`v-for` 动态 ref 需在重渲染/卸载时清理，IntersectionObserver 同步重挂。
- **语音锁定题**：录音回调写回发起题，切当前题不应串题。
