# 需求：设置页「Agents」「通信渠道」两页的卡片版式

> 状态：用户已定案（2026-11-08，经 AskHuman）；实现按向导同款流程——直接设计、用真实窗口截图迭代，截图随结果交审。
> 依赖 / 复用：`docs/specs/onboarding.md`（徽标 `BrandBadge`、状态标签、渠道状态判定 `channelState`、渠道注册表 `channelDefs`）。
> 平台：macOS / Windows / Linux（纯前端，三端同一份）。
> 影响面：`src/views/settings/`（`IntegrationTab.vue`、`ChannelsTab.vue`、`ChannelCardHead.vue`、`agentCard.ts`、
> `useAgentDetails.ts`、`reveal.ts`、`useIntegration.ts`、`useChannels.ts`、`useSearch.ts`、`useAgentTasks.ts`、`settings.css`）、
> `src/views/setup/`（`BrandBadge` 的样式随组件走、`channelTone`）、`src/styles/controls.css`（共用 `.status-tag`）、i18n。
> **不改**：IPC / Rust / 配置字段；两页的信息架构（每一行、每个开关的含义、顺序与保存方式）；设置页的深链地址与搜索入口（`integration-<id>`、`lifecycle-<id>`、`channel-<id>`）。

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
| D2 | Agents 卡 | **折叠卡片**：标题行常驻 = 徽标 + 名称 + 状态标签 + 接入方式（CLI \| MCP \| 未集成）；其余各行收进「详情」。有更新 / 出错、深链跳转、搜索命中时自动展开。信息架构不变。 |
| D3 | 渠道卡 | 加徽标与状态标签（未配置 / 未启用 / 配置不完整 / 已启用 / 已连接 / 异常），**保持**现在的「开关 + 内联表单」。 |
| D4 | 流程 | 与向导相同：直接设计并实现，用真实窗口截图迭代；不另走 spec / plan 的审阅轮，spec 作为永久记录，成品带截图交审一次。 |
| D5 | 工作位置 | 主工作区（纯前端，不碰 daemon 与端口）。 |

## 3. Agents 卡

### 3.1 标题行

`徽标（36px）｜名称 + 状态标签…｜「详情」折叠入口（名称下方）｜接入方式分段控件（右侧）`。窄窗口（最小宽 480）下标签落到名称下面、
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
- 「详情」是一个带 `aria-expanded` / `aria-controls` 的按钮，键盘可达，有焦点环。
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
- 折叠入口叫「详情」，放在名称下方而不是卡片右下角，这样折叠时整张卡只有一个徽标那么高。

## 7. 验证

- 单测：`agentCard.test.ts`（标签、有无详情、需要处理的判定）、`useAgentDetails.test.ts`（自动展开的转换语义、手动折起后不再弹开、Pi 版本探测中、读不出 / 未集成永不展开）、
  `IntegrationTab.test.ts`（标题行、折叠与展开、`v-show` 下行仍在 DOM、各状态的标签、reveal 事件、消息位置）、
  `useChannels.test.ts`（状态判定、两路读取互不影响、轮询只在渠道页可见时）、`ChannelCardHead.test.ts`、`ChannelsTab.test.ts`、
  `channelDefs.test.ts`（`channelTone`）、`BrandBadge.test.ts`（`popup`）。
- 视觉：深 / 浅色 × 默认宽 560 / 最小宽 480 × 中 / 英文，Agents 页（全折叠、有更新自动展开、读不出状态、遗留待清理、未集成）与渠道页
  （未配置 / 未启用 / 配置不完整 / 已启用 / 已连接 / 异常）；深链 `tab#lifecycle-<id>` 与设置搜索命中会展开对应卡并滚动到那一行。
