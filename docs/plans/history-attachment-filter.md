# 开发计划：回复历史附件筛选

> 关联需求：`docs/specs/reply-history.md` §3.7（A1–A14）
> 状态：已实现并通过实机验证（2026-10-22）。纯前端增量，不改 Rust、IPC、CLI、`history.jsonl`。

## 0. 目标与现状

历史窗口的搜索只有关键词（附件文件名虽能被关键词匹配，但看不到「哪些记录带附件」，也不能按后缀成组筛）。
本增量在搜索框旁加「附件」筛选：有无附件、提问 / 回复来源、后缀多选；条件以标签展示，与
`项目 → session → 关键词` 叠加，并让清理动作只作用于可见记录。

```text
entries ─ projectEntries ─ sessionEntries ─ searchedEntries(关键词) ─ filteredEntries(附件条件) ─ 列表 / 清理
                                                   │                          ▲
                                                   └─ attachmentFacet(后缀条数，面板打开时才算)
entries ── toRaw ── attachmentIndex(id → facts，每次加载一次) ──┘
```

## 1. 实现

| 层 | 文件 | 职责 |
|---|---|---|
| 纯函数 | `src/lib/historyAttachments.ts` | 后缀规范化（A6）、每条记录的附件事实（提问 / 回复数量与后缀集合）、`matchesAttachmentFilter`、后缀选项统计 `attachmentFacet`（A5）、筛选状态的不可变更新、条件标签 |
| 状态 | `src/views/history/useAttachmentFilter.ts` | 筛选状态与面板开关、事实索引、过滤结果、标签与确认框摘要、列表角标提示 |
| 面板 | `src/views/history/AttachmentFilter.vue` | 「附件」按钮 + 弹层（来源分段、开关、后缀列表）；键盘、Esc、点外部 / 失焦关闭 |
| 标签 | `src/views/history/AttachmentChips.vue` | 搜索框下方的可删除标签与「清除筛选」 |
| 编排 | `src/views/HistoryView.vue` | 把原 `filteredEntries` 拆成 `searchedEntries`（关键词）与附件过滤两步；清理范围、确认框「附件」行、空结果、列表角标、新打开请求时重置 |
| 文案 | `src/i18n/zh.ts` · `en.ts` | `history.attach.*`、`searchEmptyHintAttach`、`deleteScopeAttachments` |

要点（均由测试覆盖）：

- **关键词过滤与附件过滤是两个 computed**：改后缀不会重跑关键词匹配（真实 2000 条上约 30 ms / 次，见 §3）。
- **事实索引读原始数组**（`toRaw`）：`entries` 只会被整体替换，经 Vue 深层代理读取会为每条记录的每个字段建立依赖，慢约 10 倍。
- **后缀条数不含已选后缀**：点选时选项顺序与条数不变；已选但范围内为 0 的后缀仍显示。
- **「只看有附件」被隐含时显示为开启且锁定**：来源非「全部」或已选后缀；显式开关的值保留，取消后缀 / 来源后回到用户自己的选择。
- **Esc 只关面板**：面板内的 `keydown` 先 `stopPropagation`，历史窗口的附件预览面板 Esc 处理收不到；WebKit 点击按钮不会聚焦，所以面板打开后主动聚焦面板自身。
- **清理联动**：`isSearching = 关键词 || 附件条件`，清理范围类型与实体都据此，不会出现「清空所选会话」删到不可见记录。

## 2. 测试

| 文件 | 覆盖 |
|---|---|
| `src/lib/historyAttachments.test.ts`（30） | 后缀规范化边界、提问 / 回复事实、过滤组合（来源 × 后缀 × 无后缀）、选项统计与排序、状态更新与标签 |
| `src/views/HistoryView.attachFilter.test.ts`（27） | 列表角标与提示、面板选项与计数、后缀 / 来源 / 开关筛选与锁定、与关键词叠加、已选为 0 的后缀、标签与清除、空结果、清理标签与确认框「附件」行及冻结 ID、新打开请求重置、重载保持条件、Esc / 外部点击 / 失焦、方向键漫游与单一 tab 停靠、中文文案 |

变异检查：把 `matchesAttachmentFilter` 改成恒真，上述两份测试共 15 项失败（已还原）。

## 3. 性能（真实历史 2000 条：530 条带附件、28 种后缀；Vue 响应式环境，中位数）

| 步骤 | 耗时 | 何时发生 |
|---|---|---|
| 附件事实索引 | 1.4 ms（经响应式代理读取则 17 ms） | 每次加载一次 |
| 附件过滤 | 0.5 ms | 改条件 / 关键词结果变化时 |
| 后缀条数统计 | 0.7 ms | 仅面板打开时 |
| （既有）关键词过滤 | 约 30 ms | 每次输入关键词；改附件条件不触发 |

列表角标是索引的 O(1) 查表。附带观察：既有关键词过滤每次按键为每条记录重建 haystack，是窗口里最贵的一步，本次不动。

## 4. 验证记录

- 自动化：`vue-tsc` 通过；`pnpm test` 全部通过（vitest 80 个文件 / 858 项，node 32 项）；`pnpm build` 通过。
- 视觉：用一次性的静态 mock（无 Tauri，Chromium 无头截图，不进仓库）渲染真实 `HistoryView`，检查了浅色 / 深色、中英文、窄窗口、选中 / 悬停 / 键盘焦点、空结果各状态。
- 实机：用户在日常环境（macOS）逐项验证通过（2026-10-22，需求 §5 验收 10–13）；Windows / Linux 未单独实机，WebView 差异风险与历史窗口其余 UI 相同。
