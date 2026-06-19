# 计划：弹窗启动延迟 —— 低风险优化首轮（方案7 + 方案2 + 方案1）

> 状态：**已实现并量化（2026-06）**。同机 compare（mock IM，冷热双跑）相对基线两闸均 OK 无回归：
> WARM 端到端 p90 −5.9%（`frontend boot→painted` −37%、`popup_init` −51%）；COLD 端到端 −0.3%
> （冷启动被 ~463ms IM 建连主导，属方案3/6），但 `frontend boot→painted` −16%、`popup_init` −85%。
> 附带：`HistoryView` 改由 `history_init.lang` 应用语言（计划外补丁，见下「影响文件」），`main.ts` 自此零 IPC。

需求与方法论见 `docs/specs/popup-launch-performance.md`。本计划只覆盖**首轮低风险组合**，目标是压缩「页面加载 + 前端 boot→painted」两段，不动架构。

## 目标与范围

- **做**：方案7（前端 bundle 代码分割）、方案2（`onMounted` 先取内容渲染）、方案1（`main.ts` 不阻塞挂载）。三者配套一个支撑改动：把 `popup_init` 扩成弹窗路径的**唯一非钥匙串配置源**，从而**彻底去掉弹窗路径的两次 `get_settings()`（钥匙串）**。
- **不做（本轮）**：方案6 预热复用、方案5 detect 移 daemon、方案4 attach 省钥匙串、方案8 延后 show/骨架屏。

## 已确认决策（2026-06）

1. 配置来源：`popup_init` 统一供 语言/主题/语音设置（全部取自 helper 的 `load_without_secrets` 配置，零钥匙串）；`main.ts` 立即挂载（`auto` 兜底语言），`PopupView` 拿到 `popup_init` 后再 `applyLanguage`。接受极少数情况下 `popup_init` 返回前文本短暂为系统(auto)语言。
2. `onMounted` 调度：`popupInit()` 作为第一步并设 `request` 先渲染；listeners / speech / update 改为渲染后后台启动，不 `await` 阻塞首帧。
3. 代码分割范围：仅把 `Settings/History/Agents` 三个 view 改异步（`defineAsyncComponent`），`PopupView` 保持静态；`markdown-it` 本轮不动。
4. 验证与基线：改完用隔离 harness 对比现有 `docs/perf/baseline.json` 看收益 + 防回归；**三项全部落地并确认后**再刷新 baseline 为优化后新基线。

## 改动详解

### 支撑改动 S：`popup_init` 作为弹窗唯一非钥匙串配置源
- 后端 `commands.rs` 的 `PopupInit` 增字段：`language`（取 `state.config.general.language` 原始值，如 `auto/en/zh`）、`speech_language`、`speech_shortcut`（均来自 `state.config`，`load_without_secrets`，无钥匙串）。`theme` 已有。
- 前端 `src/lib/types.ts` 的 `PopupInit` 同步加 `language` / `speechLanguage` / `speechShortcut`。
- 作用：让弹窗的语言/主题/语音都来自这一个内存态命令，**不再需要 `get_settings()`**。

### 方案1：`main.ts` 不阻塞挂载
- 去掉挂载前的 `await getSettings()`：保留 `applyLanguage("auto")` 兜底后**立即** `createApp().mount()`；精确语言交由 `PopupView` 从 `popup_init` 应用。
- 既有 perf 埋点（`fe.bootstrap` / `fe.mounted`）保留。

### 方案2：`PopupView.onMounted` 重排
- 第一步即 `await popupInit()`；resolve 后立刻：`applyTheme(init.theme)` → `applyLanguage(init.language)` → 设置语音字段（来自 `init`）→ 设 `request.value` 与各题数组 → `loadThumbs()` / `loadDragIcons()` → 双 `rAF` 打 `fe.painted`（harness 下 autodismiss）。
- 其余初始化移到内容渲染**之后**、不阻塞首帧：注册各 `listen(...)`（preview-index/closed、drag-drop、settings-updated、update-state、popup-close-requested、popup-flash）、`popupUpdateState()`、`speechAvailable()` + `setupSpeechListeners()`、`popupAgentTerminal()`。可并行（`Promise.all`）或 fire-and-forget。
- 删除 `onMounted` 内原有的 `await getSettings()`（语音设置改取自 `popup_init`）。
- 兼容性说明：监听器注册比首帧略晚——这些事件均由 daemon 在 show 之后才可能发来（更新状态用 `popupUpdateState()` 拉初值兜底，其余为用户/托盘触发），首帧竞态可忽略。

### 方案7：前端 bundle 代码分割
- `src/App.vue`：把 `SettingsView` / `HistoryView` / `AgentsView` 由静态 `import` 改为 `defineAsyncComponent(() => import("./views/XxxView.vue"))`；`PopupView` **保持静态**（关键路径不引入额外动态 import 往返）。
- 效果：Vite 自动分块，弹窗入口 chunk 不再含另三个 view 及其依赖，减少解析/执行（落在 `page boot` 与 `frontend boot` 段）。

## 影响文件
- `src-tauri/src/commands.rs`（`PopupInit` + `popup_init`；并补 `HistoryInit` + `history_init` 的 `lang`）
- `src/lib/types.ts`（`PopupInit` + `HistoryInit` 类型）
- `src/main.ts`（方案1）
- `src/views/PopupView.vue`（方案2 + 消费 `init` 新字段，新增 `initAfterPaint`）
- `src/views/HistoryView.vue`（计划外补丁：消费 `history_init.lang` 应用语言，详见下）
- `src/App.vue`（方案7）

### 计划外补丁：HistoryView 语言来源
`main.ts` 被所有窗口共用；去掉其挂载前的 `get_settings()` 后，`SettingsView`/`AgentsView` 各自 init
里仍 `applyLanguage`（不受影响），唯独 `HistoryView` 只读 `history_init`（原只返回 theme/project），
会回退系统语言 → 对设置了非 `auto` 语言的用户算轻微回归。补法（已与用户确认）：`history_init` 增 `lang`
（后端 `Lang::resolve` 为 `en`/`zh`，与 `agents_init` 同模式），`HistoryView` 拿到后 `applyLanguage(init.lang)`。
自此 `main.ts` 彻底零 IPC（仅 `auto` 兜底 + 立即挂载），各窗口语言均由各自 init 命令承担。

## 风险与兼容
- 语言短暂为 `auto`：仅在 `popup_init` 返回前的极短窗口；`auto` = 系统语言，通常与配置一致，肉眼基本无感。
- 监听器延后注册：事件均后到，影响可忽略（见上）。
- 异步组件首次加载：仅影响 `settings/history/agents` 窗口（非关键路径）的极小首次延迟。
- 协议/IPC：`PopupInit` 仅增字段，旧前端忽略未知字段；`popup_init` 为进程内命令，无跨版本兼容问题。

## 验证
1. `./scripts/install.sh` 编译安装（`vue-tsc` 已通过；Vite 已把 Settings/History/Agents 拆为独立 chunk，弹窗入口包不再含此三者约 69 kB JS + 各自 CSS）。
2. `node scripts/perf-popup.mjs`（无脑 compare，隔离 daemon + mock IM + 冷热双跑，对比 `docs/perf/baseline.json`）。实测 `frontend boot→painted` / `popup_init` / `page boot` 均下降，端到端 p90 不回归（两闸 OK）。
3. 人工 sanity：正常 `AskHuman` 弹窗内容/语言/主题/语音/附件交互正常；设置/历史/Agents 窗口可正常打开（异步组件首次加载）。
4. 确认有效后 `node scripts/perf-popup.mjs --update-baseline` 把基线刷新为优化后新数（如此后续方案才以此为新起点防回归）。

## 不在本轮（留待后续，见 spec §4/§5/§6）
方案6 预热复用（大头、架构级）、方案5 detect 移 daemon、方案4 attach 省钥匙串、方案8 延后 show/骨架屏。

> 更新（2026-06）：**方案3/4/5 已在次轮实现**，见 `docs/plans/popup-launch-daemon-optimization.md`。
> 仅余方案6（架构级、远期）、方案8、markdown-it 按需懒加载未做。
