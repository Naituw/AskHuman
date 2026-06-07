# 待办 / 已知问题

记录暂不处理但需跟踪的问题与后续增强。

> 历史「同一 client-id 多开 Stream 相互干扰」已由 daemon 架构（Phase 2：IM 渠道迁入 Daemon、每种全局单条长连接、Router 按键路由）根治，并经真机并发实测确认（见下「人工实测」），条目已移除。

## 已修复

### 钉钉卡片「提交」误报『请求失败』toast（实际已成功）✅

> 已修复并真机验证（点提交不再弹『请求失败』、卡片正常置灰、答案正确送达）。

- **根因**：daemon 架构里钉钉长连接由 `dingtalk/router.rs` 的 Reader 独占，Reader 收到卡片回调后**对所有回调一律回空包**。但钉钉**互动卡片「提交」按钮**要求那条 3 秒内的同步回包必须是**非空的成功/更新回包**，否则客户端判定提交失败弹红条（答案其实已送达）。
- **实现方案**（ACK 由「真正接受提交的会话」产出 = 确认而非预测，且读循环不被慢活拖住）：
  1. `dingtalk/card.rs`：新增纯函数 `is_submit(data)`、`submit_ack_success()`（置灰点击者私有 `submitted=true` 的成功回包）。
  2. `dingtalk/router.rs`：Reader 判 `is_submit` —— 非提交回调（选项切换）直接空 ACK、不转发；提交回调转发给对应会话并带 `oneshot` 回执，**带超时(2.5s)等会话裁决**后回包；孤儿/超时回空包（诚实地不显示成功）。
  3. `channels/dingding.rs`：会话认出本卡片提交即**立刻**经 oneshot 回 `submit_ack_success()`（不在 3 秒关键路径上等任何慢活），随后再经 OpenAPI 写公有终态文案；并把作答期间的图片/文件改为**并发下载**（spawn），保证提交一到就能被立刻处理。
- **影响范围与超时取舍**：Reader 等待裁决期间只暂停「当下并发的钉钉」（不影响飞书/Popup/Telegram，各自独立连接），且因会话即时回包＋下载并发化，等待几乎为毫秒级、极少触顶。
### 飞书卡片「提交」按钮置灰有可见闪烁（Loading→弹回 Submit→才变已提交）✅（已大幅改善）

> 同源问题：飞书 Reader 也是收到回调即空 ACK、置灰靠之后的 OpenAPI `patch_card`，导致按钮先弹回再异步变终态。

- **实现**（与钉钉同构，复用飞书已有但未用的 `respond_card` 同步回包）：
  1. `feishu/router.rs`：卡片回调改为**带 oneshot 回执转发给会话**；超时(2.5s)等会话裁决——`Some(body)` → `respond_card` 同步更新卡片、`None`/孤儿/超时 → 空 ACK。
  2. `channels/feishu.rs`：会话认出本卡片提交即**立刻**经 oneshot 回**终态卡片**（`card::callback_update_card` 包装 `build_finalized_card`），按钮 Loading 直接变终态；并把附件下载改为并发。
  3. 去掉了提交路径上的 `patch_card` 兜底（那次二次渲染是残留快速回弹的来源）；被抢答/断连路径仍用 `patch_card`（无回调可同步回包）。
- **现状**：置灰快了很多、不再二次渲染；**仍有一下极快的回弹**——经排查为**飞书客户端自身渲染行为**（收到回调先复位按钮再套用新卡片），非本端可控，保持现状。

### Telegram ✅
- 用 `answerCallbackQuery`，机制不同；已人工回归确认：点按钮无报错、卡片正常更新、答案送达，无需改动。

### 消息 / 卡片正文的 Markdown 渲染（飞书、Telegram）✅
- **飞书**：消息提示无原生 Markdown 文本类型 → `send_message_prompt` 在 `is_markdown` 时改发 **Markdown 卡片**（`card::build_message_card`）。
- **Telegram**：原 `MarkdownV2` 转义挑剔、缺斜体/删除线/表格/列表，且任一特殊字符不配对会整条回退纯文本。改为 **`parse_mode=HTML`**（`telegram/markdown.rs::to_html`）：仅转义 `< > &`，`<b>/<i>/<s>/<code>/<pre>/<blockquote>/<a>` 标签天然配对；标题统一加粗（HTML 无字号）、表格转等宽代码块、无序列表 `•`、`_` 带词边界判断避免吃 snake_case；卡片活动态/终态同走 HTML（终态解析失败回退纯文本编辑）。已真机预览确认。

## daemon 架构：人工实测（已逐项跑通）

> 真机实测（install 后经新 daemon→GUI Helper 链路）全部通过：① 单题弹窗作答（退出 0）；② **并发两请求弹窗不串台**（A→A、B→B）；③ 取消返回 `[Status]` 再问指引（退出 0）；④ `daemon status` 显示 running 且常热 IM 连接。下列逐项已验证：

- [x] **真实 IM 并发（真 TODO#1）**：并发两请求均在钉钉作答（同 client_id）→ A 仅得 A 选项、B 仅得 B 选项，**无串台**（`outTrackId` 路由正确）；`daemon status` 始终单实例长连接。
- [x] **飞书 / Telegram 提交回包**：飞书已改同步回包（残留回弹为飞书自身渲染）；Telegram 实测正常无需改。
- [x] **被抢答 / 跨渠道抢答**：一条多渠道请求，在钉钉作答 → 飞书 / Telegram 卡片即时置灰「已在 钉钉 回答」、弹窗关闭（窗口形态无置灰态）。
- [x] **Phase 3 实时配置（验收 #7）**：弹窗开着时改 `config.json` 主题→浅色 / 语言→`zh` → 整窗（含毛玻璃）实时切浅色 + 界面实时切中文。（**期间修复**：`ConfigChanged` 之前只切前端 CSS、未切原生窗口外观，导致毛玻璃下「网页浅、窗体深」；已补 `apply_theme_to_windows` 同步原生 `set_theme`。语言合法值仅 `auto/en/zh`，`zh-CN` 会被当未知回退系统。）
- [x] **Phase 3 凭据热重载（惰性失效）**：禁用 Telegram → `daemon.log` 出现 `config reloaded`、`im conns` 即去掉 telegram、下个请求不再发 Telegram；重新启用 → 下个请求恢复。
- [x] **临时目录清理（A10）**：构造 mtime>24h 的 `temp/askhuman/<id>/` 与一个新目录 → `daemon restart` 触发启动清理 → 过期目录被删、新目录与刚产出的图片目录保留。
- [x] **生命周期**：`daemon stop` → `status` not running；`daemon start/restart` → running（`im conns: none`，惰性按需建连）；二进制指纹换新（重装后旧 daemon 自动让位、新 daemon 接管）。
- [x] **空闲自动回收**：用 `ASKHUMAN_DAEMON_IDLE_SECS` 把超时调短（15s）实测三类——(A) 无请求约 30s（受 30s 自查档限制）自动退出、清理 socket/meta；(B) 弹窗打开期间连接常驻 → `active≥1` → 全程不回收（实测 120s 远超阈值），作答完无活动后约一个自查档即回收；(C) 任意连接（含 `daemon status` 轮询）会重置空闲计时「续命」，停止轮询后下一档回收。检测只看 `daemon.log` + socket 文件存废，避免连接重置计时。
- [x] **自动识别 userId/open_id（Q6）**：设置窗口点「自动识别」→ 经 daemon `Detect`（无现有连接时临时开连）→ 私聊机器人发验证码 → 钉钉 UserId、飞书 Open ID 均自动回填成功。

## 后续增强 / 性能优化

### A. 钉钉卡片「变灰」延迟（daemon 架构引入）

- **背景**：daemon 架构下钉钉长连接由 Router 独占共享，卡片回调由 Router 即时空 ACK（满足 3 秒约束），卡片置灰（「已提交」/「已在 X 回答」）改走 OpenAPI `updateCard`（见 `docs/specs/daemon-architecture.md` §11）。
- **影响**：相比单进程时代「stream 同步回包即时变灰」，现在变灰是一次独立 HTTPS 调用，慢约 100–300ms（仅视觉延迟，功能一致）。
- **可选优化**：如需即时变灰，可让会话经 oneshot 把回包回传给 Router 的 Reader，由其在 3 秒内用长连接写回（代价：Reader↔会话耦合、Reader 读循环需短暂等待会话算回包）。当前判断收益有限，暂不做。
