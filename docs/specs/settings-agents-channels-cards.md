# 需求：设置页「Agents」「通信渠道」两页的卡片版式（及「高级」页的两处延伸）

> 状态：用户已定案（2026-11-08，经 AskHuman）；实现按向导同款流程——直接设计、用真实窗口截图迭代，截图随结果交审。
> 同日追加：复核「通用」「高级」两页后，只把「高级」页的两处搬过来（§8）；「通用」页不动（D6）。
> 同日修订：用户看过成品后反馈「改完比之前更乱、更不直观；Agent 卡里的『详情』太小、容易找不到，像只剩一个开关」，据此重做
> 「详情」入口（§3.2，D7）、就绪列表（§8.1，D8）与两张 Agent 专属卡的徽标位置（§8.2，D9），三处均经用户确认。
> 随后用户选了一个跟进：不支持从 IM 启动的 Agent（Antigravity）在矩阵里显示灰色「暂不支持」（§8.1，D10）。
> 2026-11-08 又一轮（用户在新设备上被向导先邀请「开启」、再告知「没有可以开启的 Agent」，见 `docs/specs/onboarding.md` D25 第 11 轮）：就绪矩阵改读全局统一的「能不能启动」判定（§8.5，D11），渠道卡片（Telegram / 钉钉 / 飞书 / Slack / iOS App）底部多一个「从手机 / IM 创建 Agent 任务」开关（§4.4，D12）。
> 依赖 / 复用：`docs/specs/onboarding.md`（徽标 `BrandBadge`、状态标签、渠道状态判定 `channelState`、渠道注册表 `channelDefs`）。
> 平台：macOS / Windows / Linux（纯前端，三端同一份）。
> 影响面：`src/views/settings/`（`IntegrationTab.vue`、`ChannelsTab.vue`、`ChannelCardHead.vue`、`agentCard.ts`、
> `useAgentDetails.ts`、`reveal.ts`、`useIntegration.ts`、`useChannels.ts`、`useSearch.ts`、`useAgentTasks.ts`、`AgentTasksSwitchRow.vue`、`settings.css`；
> 「高级」页延伸：`AdvancedTab.vue`、`CursorMaxCard.vue`、`PermissionRulesCard.vue`、`readiness.ts`）、
> `src/views/setup/`（`BrandBadge` 的样式随组件走、`channelTone`）、`src/styles/controls.css`（共用 `.status-tag`）、i18n。
> **不改**：IPC / 配置字段；各页的信息架构（每一行、每个开关的含义、顺序与保存方式）；设置页的深链地址与搜索入口（`integration-<id>`、`lifecycle-<id>`、`channel-<id>`）。
> 最初也不改 Rust；2026-11-08 的一轮是例外：后端 `agent_task_readiness` 多带 `startable` / `startRoute` / `launch`（additive，§8.5），其余仍是纯前端。

## 1. 背景与目标

安装向导（`docs/specs/onboarding.md` §8）把 Agent 与渠道做成「徽标 + 名称 + 状态标签」的磁贴后，设置页相比之下有两处问题：

- **Agents 页太长**：6 张卡，每张集成后有 8–12 行（规则、超时 Hook、MCP 配置、停止确认、权限、生命周期、Desktop 设置…），
  要找一家的状态得滚好几屏；卡上看不出「这家是否正常」，要逐行读。
- **通信渠道页看不出状态**：每张卡只有一个名称和开关，是否配好、是否连上、是否出错要展开表单才知道，也没有品牌标志。

目标：把向导里已经验证过的版式搬到这两页——一眼看出每家的状态，需要处理的才展开——而不改变任何设置项本身。

## 2. 已确认决策（用户经 AskHuman 定案）

| 编号 | 决策项 | 结论 |
|---|---|---|
| D1 | 范围 | 只做 Agents 与通信渠道两页；其余页以开关行为主、版式差异不大，做完再看要不要往外推。 |
| D2 | Agents 卡 | **折叠卡片**：标题行常驻 = 徽标 + 名称 + 状态标签 + 接入方式（CLI \| MCP \| 未集成）；其余各行收进「详情」（入口见 D7）。有更新 / 出错、深链跳转、搜索命中时自动展开。信息架构不变。 |
| D3 | 渠道卡 | 加徽标与状态标签（未配置 / 未启用 / 配置不完整 / 已启用 / 已连接 / 异常），**保持**现在的「开关 + 内联表单」。 |
| D4 | 流程 | 与向导相同：直接设计并实现，用真实窗口截图迭代；不另走 spec / plan 的审阅轮，spec 作为永久记录，成品带截图交审一次。 |
| D5 | 工作位置 | 主工作区（纯前端，不碰 daemon 与端口）。 |
| D6 | 「通用」「高级」复核（2026-11-08） | 「通用」页 11 张卡都是「标题 + 几行彼此独立的偏好」，没有「一家一个对象 + 一堆状态行」的结构，**不动**。「高级」页只搬两处：Agent 就绪列表（§8.1）、两张 Agent 专属卡的徽标（§8.2）。**不做**：把这两页的卡折叠（行都是要直接改的偏好，折叠只会把设置藏起来）、给每张卡加图标、页内再分组、「关于」卡的更新状态标签（价值低，该信息已在「最新版本」一行里）。 |
| D7 | 「详情」入口（2026-11-08 反馈） | 入口从名称下方的小号灰字「› 详情」改成**标题行下方的整行条**：按钮外观、整条可点，粗体「详情」+ 一句里面有什么 + 右侧箭头。折叠后整张卡不再像「只剩一个模式开关」。**否决**「已集成的 Agent 默认展开」：页面又回到要滚好几屏的老问题（§1），D2 的折叠不变。 |
| D8 | 就绪列表（2026-11-08 反馈） | 上一版（徽标 + 名称 + 标签，缺的条件以蓝色下划线链接挂在名称下）行高不一、参差，被判「更乱」。改成**对齐的矩阵**：一家一行，固定三列「CLI / 生命周期 / 集成」，满足 ✓、缺失 ×（可点）。**否决**回到原来的一行小字（`CLI ✓ · Lifecycle ✓ …`）。 |
| D9 | 两张 Agent 专属卡的徽标（2026-11-08 反馈） | 徽标缩成 24px，放进**标题那一行**；说明文字回到卡片整宽。**否决**上一版的「徽标在左、标题与说明缩进在右」（说明比其它卡窄，显得不齐）。 |
| D10 | 不支持的 Agent 一行（2026-11-08，用户选的跟进） | `launchSupported = false` 的 Agent（目前只有 Antigravity：没有新任务 / Fork 路径，也没有生命周期追踪）在矩阵里不是「未就绪」加三个修不好的橙色 ×，而是**灰色「暂不支持」标签 + 三格灰色短横**，没有任何可点的东西；表格下提示一句说明。**否决**从矩阵里隐藏它：后端注释写明 `all_readiness` 有意把这类 family 也报出来，让设置页与 doctor 去解释它们（选择器才过滤）。 |
| D11 | 矩阵读统一判定（2026-11-08，用户在新设备上反馈） | 总状态标签不再等于 `binary ∧ lifecycle ∧ integration`，而是后端 `agent_task_readiness` 返回的 `startable`（`integrations/agent_start.rs`；手机、IM、向导、文字版用同一个）：Codex 的桌面 App 或终端任一可用、Cursor 的 Agents 窗口或终端任一可用（偏好只决定谁优先）、其余 Agent 的 CLI 就绪。标签随路线变化（§8.5）。**否决**让矩阵继续用 CLI 就绪：那会在 Codex / Cursor 能用桌面路线时仍写「未就绪」，正是用户看到的前后矛盾。 |
| D12 | 开关进渠道卡片（2026-11-08，用户让我设计） | 「从手机 / IM 创建 Agent 任务」是全局开关，不属于任何一个渠道，配渠道时又没人提醒它，容易漏。在每个**已启用**的远程渠道卡片展开内容的最底部放同一个开关（同一个设置、同一个确认弹窗，§4.4）。**否决**：在向导或设置里到处加提醒条；把它放进卡片标题行（标题行已有状态标签与启用开关，且渠道没启用时谈不上从它新建任务）。 |

## 3. Agents 卡

### 3.1 标题行

`徽标（36px）｜名称 + 状态标签…｜接入方式分段控件（右侧）`，已集成的卡在其正下方还有一整行「详情」条（§3.2）。窄窗口（最小宽 480）下标签落到名称下面、
分段控件始终留在右侧；`.agent-id` 与标签行允许换行。标签规则（纯函数 `agentCard.ts: agentTags`，有单测）：

| 状态 | 标签（语气） |
|---|---|
| 状态读不出来（`modeReadError`） | 无法读取状态（红）；此时没有分段控件，只有「重新读取状态」 |
| 已集成（`mode ≠ none`） | 已集成（绿） |
| 已集成 + 任一「更新」按钮可点 | 有更新（橙）——含 `needsUpdate` / 规则 / Hook / MCP / 权限 / 生命周期各 `needsUpdate`，以及 `stop`、`askQuestion` 的 `outdated` |
| 已集成 + Pi / Codex 版本问题 | 版本不受支持（橙）——Pi 还包括「找不到 Pi / 读不出版本」；Pi 的版本探测还在跑时不判 |
| 已集成 + Agent 自己的策略让托管 Hook 不运行（`permission.knownBlockedReason`） | Hook 未生效（橙） |
| 未集成 + 有旧版生命周期遗留 | 待清理（橙），清理提示与「更新」按钮仍在标题行下方 |
| 未集成 + `needsUpdate` | 有更新（橙） |
| 未集成且无遗留 | 无标签 |

**信息不丢的原则**：折叠后，任何需要用户动手的东西要么在标题行有标签，要么让卡片自动展开。仅属于「提示」的内容
（如别家 Hook 与托管 Hook 并存的警告）留在详情里，不上提。

### 3.2 详情与折叠

- 详情 = 原来标题行以下的所有行（规则 / Skill / 协议、委托运行、CLI 的 Hook、MCP 配置、停止确认、AskUserQuestion、权限、生命周期，
  以及 Codex / Cursor Desktop 子块），逐字保留。未集成或读不出状态的卡没有详情，也没有「详情」入口。
- 折叠用 `v-show`，**不**用 `v-if`：深链与设置搜索都靠 DOM 里能找到目标行；折叠的行仍在 DOM 里，跳转时先展开再滚动。
- **入口是整行条**（D7，`.agent-disclosure`）：标题行（`.agent-head`）下方、卡片整宽、34px 高，用与按钮相同的底色 / 描边 / 悬停色；
  左边粗体「详情」，中间一句说明里面有什么（`settings.integration.detailsHint`：规则、Hook、权限、生命周期等设置，过窄时省略号截断），
  右边箭头（折叠朝下、展开翻成朝上）。整条都是按钮，带 `aria-expanded` / `aria-controls`，键盘可达，有焦点环。条在展开前后位置不变，收起总在同一个地方。
  上一版是名称下方 12px 的灰色小字链接，太容易漏看，折叠后整张卡像只有一个模式开关，「以前的设置都不见了」。
- 动作结果消息（`modeMessage`）：折叠时放在标题行下方（否则看不见），展开时放在详情底部（紧挨触发它的行），任何时刻只出现一处。

### 3.3 自动展开

- **何时**：卡片从「不需要处理」变成「需要处理」的那一刻自动展开。需要处理 = 已集成且（有「更新」按钮可点，或 Pi / Codex 版本有问题）
  （`agentCard.ts: needsAction`）。「Hook 未生效」只出标签、不触发展开（那是 Agent 自身策略，用户在卡里也改不了）。
- **之后归用户**：自动展开是**转换触发**的，用户手动折起后，同一个需求仍在时不会再被弹开；需求消失再出现才会再次自动展开。
- **状态位置**：展开状态存在设置上下文（`useAgentDetails`，由 `useIntegration` 持有），切换标签页再回来保持原样；
  不跨设置窗口的两次打开持久化，每次打开都由上面的规则重新判定。

### 3.4 深链与搜索命中

跳转目标可能在折叠的详情里（`integration-<id>`、`lifecycle-<id>`、搜索命中的某一行）。契约：

1. 跳转代码（`gotoSettingsTarget`、`gotoSearchResult`）在滚动前，对目标元素调用 `revealSettingsTarget(el)`（`reveal.ts`）：
   在该元素上派发一个会冒泡的 `settings-reveal` 事件，并等一个 `nextTick`；
2. 会折叠它的卡片在根上监听该事件并展开自己；没有任何东西折叠它的目标直接忽略事件；
3. 展开后再 `scrollIntoView` 与高亮。

好处：跳转代码不需要知道卡片的状态，卡片也不需要知道谁在跳转。

## 4. 通信渠道卡

### 4.1 标题行

`徽标（36px）｜名称（+ 飞书的「推荐」）+ 状态标签｜配置指南 ↗｜开关`（`ChannelCardHead.vue`，五张卡共用）。名称区允许换行，窄窗口下
标签落到名称下面而不挤开开关。保持原有的元素 id `channel-<id>`、跳转高亮、`channelIssueText` 的错误行（在标题行正下方）与各自的内联表单。

### 4.2 状态判定

沿用向导「远程回答」页的判定函数 `channelState`（`src/views/setup/channelDefs.ts`；向导侧的语义见 `docs/specs/onboarding.md` D19 的页 2），输入四项：

| 输入 | 来源 |
|---|---|
| 开关是否打开 | 当前（未必已落盘的）配置，所以关掉开关立刻变「未启用」，不等下一次轮询 |
| 必填项是否都有值 | `requiredFieldsFilled`：输入框有值，或密钥已存在（`secretsPresent`） |
| 是否已连上 | `setup_channel_status` 的 `connected`；没有 daemon 时是 `null`（未知，不是「没连上」），读作「已启用」 |
| 是否有未恢复的错误 | `channel_health`；只有开关打开的渠道才会因它变成「异常」（关着的渠道留着旧错误也不报异常）。卡片里错误行本身的显示规则不变 |

| 状态 | 标签（语气） |
|---|---|
| 关、没配 | 未配置（灰） |
| 关、已配 | 未启用（灰） |
| 开、缺必填项 | 配置不完整（橙） |
| 开、已配、没连上或未知 | 已启用（绿） |
| 开、已配、daemon 已连上 | 已连接（绿） |
| 开 + 有未恢复错误 | 异常（橙，压过其它状态） |

「渠道」页可见期间，原有的 10 秒轮询同时取 `channel_health` 与 `setup_channel_status`；两路互不依赖，一路失败只丢那一半信息。
轮询不启动 daemon（`setup_channel_status` 的既有语义）。

### 4.3 本地弹窗卡

加蓝底白色窗口的徽标（`popup`），**没有状态标签**——本地弹窗没有连接可报告；其余（开关、记住窗口尺寸、默认宽高）不变。

### 4.4 渠道卡片里的 Agent 任务开关（D12，2026-11-08）

`AgentTasksSwitchRow.vue`：同一个组件放在 Telegram / 钉钉 / 飞书 / Slack 四张卡**已启用**时展开内容的最底部（前面一道分隔线），`IosChannelCard.vue` 里也放一份（取代原来的顶部提示条）。卡片未启用或折叠时没有它。

- **内容**：标题「从手机 / IM 创建 Agent 任务」+ 一个开关 + 一句说明（关：「开启后，在手机 App 或 IM 里就能选项目和 Agent，让这台电脑开一个新的 Agent 会话。所有渠道共用这一个开关。」；开：「已开启，所有渠道共用。」）。开关写 `agentTasks.enabled`，走 `toggleAgentTasks`——开启前仍是「强制保活并添加登录项」的确认弹窗，所以各处入口效果一致。
- **开着时**：多一个「在「高级」里管理权限、工作目录和 Agent 就绪状态」链接（切到「高级」Tab）；若「高级」页已读出的就绪结果里没有任何 Agent 能启动，再多一行橙色警示（`agent-tasks-stuck`）。这行警示只读已加载的结果，**不**为它在渠道页里探测五家 Agent（`tabData.ts` 的懒加载约定不变）。
- **平台不支持**（`supportsAgentTasks` 为假）时整行不渲染。
- **测试**：`ChannelsTab.test.ts`（只在已启用的 IM 卡末尾出现、全局共用一个值、平台不支持时没有、中文）、`IosChannelCard.test.ts`。

## 5. 共享件

- `BrandBadge` 的样式（`brandBadge.css`）随组件走，设置页引用即得；`brandBadges.ts` 仍是唯一的徽标注册表，新增 `popup`（`logos/window.svg`，来源见 `logos/NOTICE.md`）。
- 状态标签 `.status-tag`（`.ok` / `.warn` / `.err`，深浅色各一套）提到 `controls.css`，向导与设置页共用；向导改用同一个类。
- `channelTone(state)`（`setup/channelDefs.ts`）给出渠道标签的语气，向导与设置页共用。

## 6. 我自行决定（可否决）

- 本地弹窗的徽标用蓝底白窗口，并且不带状态标签。
- 「异常」用橙色而不是红色，和向导、渠道卡里原有的错误行一致。
- 别家 Hook 与托管 Hook 并存的警告留在详情里，不上提到标题行（那是提示，不是待办）。
- 标题行不放「更新」按钮：更新入口仍是各行的按钮和页面顶部的「全部更新」。
- 「版本不受支持」同时覆盖「太旧」与「Pi 找不到 / 读不出版本」，不再分两个标签。
- 展开状态不跨窗口会话保存。
- 折叠入口叫「详情」。最初放在名称下方的小字，折叠时整张卡只有一个徽标那么高，但用户反馈太小、容易找不到，改为标题行下方的整行条（D7），
  代价是每张已集成的折叠卡多约 46px 高。

## 7. 验证

- 单测：`agentCard.test.ts`（标签、有无详情、需要处理的判定）、`useAgentDetails.test.ts`（自动展开的转换语义、手动折起后不再弹开、Pi 版本探测中、读不出 / 未集成永不展开）、
  `IntegrationTab.test.ts`（标题行、整行「详情」条、折叠与展开、`v-show` 下行仍在 DOM、各状态的标签、reveal 事件、消息位置）、
  `useChannels.test.ts`（状态判定、两路读取互不影响、轮询只在渠道页可见时）、`ChannelCardHead.test.ts`、`ChannelsTab.test.ts`、
  `channelDefs.test.ts`（`channelTone`）、`BrandBadge.test.ts`（`popup`）。
- 视觉：深 / 浅色 × 默认宽 560 / 最小宽 480 × 中 / 英文，Agents 页（全折叠、有更新自动展开、读不出状态、遗留待清理、未集成）与渠道页
  （未配置 / 未启用 / 配置不完整 / 已启用 / 已连接 / 异常）；深链 `tab#lifecycle-<id>` 与设置搜索命中会展开对应卡并滚动到那一行。

## 8. 「高级」页的延伸（D6）

### 8.1 Agent 就绪矩阵（D8）

位置：「高级 → 从手机 / IM 创建 Agent 任务」（2026-11-08 前叫「从 IM 创建 Agent 任务」）开启后的「Agent 就绪状态」（`AdvancedTab.vue`；规则在 `readiness.ts`）。
这是「高级」页里唯一和 Agents 页同构的列表。原来每家一行 `名称 · 圆点 + 已就绪 / 未就绪 · CLI ✓ · Lifecycle ✓ · Integration ✓`：
没有徽标，三个条件不管满没满足都打出来（写死的英文、不走 i18n）。第一次改版（徽标 + 名称 + 标签，缺的条件作为蓝色下划线链接挂在名称下）
被用户判为「更乱、不直观」：每行高度不一，下划线文字参差，看不出缺的是「哪一项」。现版式：

```
                                  CLI    生命周期    集成        ← 列头（小号灰字，本地化，aria-hidden）
[徽标28px] Claude Code [已就绪]    ✓        ✓         ✓
[徽标28px] Codex       [未就绪]    ✓        ×         ✓
```

- **行**：`徽标（28px）｜名称｜总状态标签`，标签 = 已就绪（绿）/ 未就绪（橙）。最初它是后端的 `ready`（`binary ∧ lifecycle ∧ integration`）；2026-11-08 起读统一的「能不能启动」（D11，§8.5）。
  行高固定（42px），行间一条细线。
- **列**：固定三列（各 64px，列头与每行的格子同宽，所以必然对齐）：CLI、生命周期、集成。顺序即修复顺序（`readinessIssues` 的顺序，纯函数，有单测）。
- **格子**：满足 = 绿色 ✓（画出来的 SVG，不是文字；`role="img"`，`aria-label` = `名称: 列名`）；
  缺失 = 橙色圆底 ×，是**按钮**（24px），`aria-label` = `名称: 条件文案`，`title` = `条件文案 · 点击后做什么`。目标与之前一致：

  | 条件 | 列 | 条件文案（i18n） | 点击 |
  |---|---|---|---|
  | `binaryReady = false` | CLI | CLI 不可用（`issueBinary`） | 打开该 Agent 的官方安装文档 |
  | `lifecycleReady = false` | 生命周期 | 生命周期追踪未就绪（`issueLifecycle`） | 去「Agents」页该 Agent 卡的生命周期行（折叠的详情先自动展开，§3.4）；**该 Agent 还没集成、没有生命周期行时，退到它的卡片并高亮**（见下） |
  | `integrationReady = false` | 集成 | 集成不可用（`issueIntegration`） | 去「Agents」页该 Agent 的卡 |

- **不支持的 Agent（D10）**：`launchSupported = false` 时，行状态是 `unsupported`（纯函数 `readinessState`，有单测），标签为灰色「暂不支持」
  （`.status-tag.muted`，与渠道页的「未配置」同灰），三格各是一个灰色短横 `–`（`aria-hidden`，不可点），没有 ✓ 也没有 ×；
  无论它的三个条件实际是什么都这样，因为用户改什么都改变不了它。列表里只要有这样一家，提示下面就另起一行多一句
  「标「暂不支持」的 Agent 目前不能从手机 / IM 创建任务，无需处理。」（`unsupportedHint`；两句各占一行，免得中文句号后面夹一个多余的空格）。
  后端的就绪字段不动，其它 Agent 的行完全不受影响。
- 表格下方一行提示「点击橙色 × 前往修复；CLI 不可用时会打开官方安装文档」（`readinessHint`）。列头 `colBinary` / `colLifecycle` / `colIntegration`。
- 窄窗口（480）：三列固定 64px 宽，英文「Integration」列头放得下；列与列之间不留间隙（64px 的单元格自己够开），省下的宽度全给名称列，
  使最长的一对（Antigravity + Unsupported）在最窄时仍在一行里。名称与标签放不下时仍允许标签换行（兜底）。
- 不变：刷新时机、加载中的转圈（加载中且无数据时只有转圈，没有矩阵与提示）、`/new` 与手机端的门控——这里只是显示方式。就绪判定本身在 2026-11-08 改为全局统一的「能不能启动」（D11，§8.5）。

**死链接修复**（上一版就存在的缺陷）：生命周期未就绪、而该 Agent 未集成时，目标 `lifecycle-<id>` 不在 DOM 里（它只存在于已集成卡的详情里，
或「未集成但有遗留」的清理块里），点击没有任何反应。`gotoSettingsTarget(target, fallback?)` 现在在目标元素不存在时改去 `fallback`
（`openReadinessIssue` 传 `integration-<id>`，即卡片根，始终存在），照常 reveal + 滚动 + 高亮，人落在卡片上、先把集成打开。没有 `fallback` 的调用保持原样。

### 8.2 Agent 专属卡的徽标（D9）

「Cursor 任务的 Max 模式」（`CursorMaxCard.vue`）与「Codex 会话授权」（`PermissionRulesCard.vue`）的标题那一行前面加对应 Agent 的徽标
（24px，`.agent-card-title`：徽标 + `.card-title` 同一行垂直居中），说明文字在标题行**下面**、占卡片整宽，结构与加徽标之前相同，只是标题行多一个徽标。
其余的卡（守护进程生命周期、IM 渠道按需发送、从手机 / IM 创建 Agent 任务）不是某一家 Agent 的，不加。标题文字、各行与保存方式不变；
设置搜索靠 `.card-title` 文字定位，该元素与文案原样保留。第一次改版把徽标放在左侧、标题与说明缩进在右，说明变窄，被判不齐。

### 8.3 我自行决定（可否决）

- 条件文案用「CLI 不可用」而不是「未找到 CLI」：后端的 `binaryReady` 还包含「Pi 版本过低」与「login shell 查找没查完」，
  后者按 `agent_launch.rs` 的既有约定不能说成「没找到」（会把人送去重装已有的东西）。同理「集成不可用」覆盖未集成、规则缺失、Hook / MCP 缺失或过期。
- Antigravity 没有生命周期追踪、也不支持从 IM 启动（`launchSupported = false`）。矩阵仍列出全部六家，但它这一行按 D10 显示「暂不支持」，
  不再是三个修不好的 ×（后端的 `ready` 对它仍是 false，没改）。「暂不支持」英文用 “Unsupported”（最初的 “Not supported” 在 480px 宽、英文下放不进一行，标签会掉到名称下面，所以换成更短的词）。
- 矩阵里的 × 用橙色而不是红色：它们是「待办」，不是「故障」，与「未就绪」标签同色。
- 总状态标签保留在矩阵里（没有因为有了三列就去掉）：它是「这家能不能用」的答案，三列是原因。

### 8.4 验证

- 单测：`readiness.test.ts`（缺哪项、顺序、已就绪为空；`readinessState` 的就绪 / 未就绪 / 不支持）、`AdvancedTab.test.ts`（每家的徽标 / 名称 / 标签与语气、三列与本地化列头、✓ 与 × 的位置与是否可点、不支持的 Agent 是灰标签 + 三个短横且无按钮、其它行不受影响、说明句只在有不支持的 Agent 时出现、
  `aria-label` / `title`、点击把 `(kind, issue)` 交给 `openReadinessIssue`、提示行、中英文、加载中无矩阵、两张 Agent 专属卡的徽标在标题行内而说明在行外、其它卡不带徽标）、
  `useAgentTasks.test.ts`（缺 CLI 打开安装文档且不切标签；生命周期行存在则去该行；不存在则退到卡片；集成去卡片；无 fallback 保持原样；高亮 2.2 秒后清除）。
- 视觉：深 / 浅色 × 560 / 480 × 中 / 英文；全部就绪、部分就绪、全部缺的 Pi、「暂不支持」的 Antigravity；Max 卡与 Codex 授权卡的标题行；点击 × 的三类跳转。

### 8.5 矩阵读统一判定（D11，2026-11-08）

后端 `agent_task_readiness` 现在在每家的就绪字段之外多带 `startable`（布尔）、`startRoute`（`terminal` / `desktop` / 空）与 `launch`（Codex / Cursor 两条路各自的状态），矩阵每一行只读这几项（`readiness.ts`：`isStartable`、`readinessState`、`readinessIssues`、`startsOutsideTheCli`、`startBlockers`）。

- **标签**：能启动且走 CLI →「已就绪」；Codex 走桌面 App →「就绪 · Desktop App」；Cursor 走 Agents 窗口 →「就绪 · Agents 窗口」（都是绿色）；不能启动 →「未就绪」（橙）；`launchSupported = false` →「暂不支持」（灰，D10 不变）。读不到 `startable` 的旧后端回落到 `ready`。
- **格子**：走桌面路线的 Agent，CLI 与生命周期两列画灰色短横（不需要，不是缺项；`title` 写「不需要：这条路径不经过 CLI」，`aria-label` 同），**集成**一列照常（桌面路线同样需要 AskHuman 集成）；列表里只要有这样一家，提示下面多一句「标「Desktop App」或「Agents 窗口」的 Agent 不用 CLI 启动，CLI 那一栏用不上（–），无需处理。」（`outsideHint`）。
- **原因**：Codex / Cursor 两条路都不通时，该行下方多一行小字（`readiness-blockers-<kind>`），按偏好的先后写明各路缺什么（读 `launch` 里的 `desktopReason` / `terminalReason`，与 Agent 卡里的状态行是同一套文案）；其余 Agent 缺什么已经由 × 说明，不加；没有 `launch`（旧后端）也不加。行与原因包在同一个 `.readiness-item` 里，分隔线画在外层。
- **不改**：列、顺序、× 的点击目标、刷新时机、加载态。
