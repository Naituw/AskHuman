# 实现计划：活跃槽切走时自动结束该渠道的 watch

需求与决策见 `docs/specs/im-auto-end-watch.md`。核心只有一个挂钩点（`set_active_channel`）+ 一个新终态 +
一个子开关。分 4 步，均在既有代码上小改，无新模块。

## P1 — 配置字段（`config.rs` + 前端类型/UI/i18n）

- `ChannelsConfig` 增 `auto_end_watch: bool`（camelCase `autoEndWatch`），**默认 `true`**：
  - 字段加 `#[serde(default = "…true…")]`（缺字段→`true`）；
  - `ChannelsConfig` 现为 `#[derive(Default)]`，会给新字段填 `false` → 改为**手写 `impl Default`**
    （各渠道字段 `Default::default()`、`auto_activation: false`、`auto_end_watch: true`），保证「文件全缺省」时也为开。
  - 单测：缺 `autoEndWatch` 的 JSON 反序列化后为 `true`；`ChannelsConfig::default().auto_end_watch == true`。
- 前端：
  - `src/lib/types.ts`：`ChannelsConfig` 增 `autoEndWatch: boolean`。
  - `src/views/SettingsView.vue`：在实验区「IM 渠道按需发送」卡片内、说明文字之后加一枚子开关
    （缩进/次级视觉），`v-model="config.channels.autoEndWatch"` + `@change="persist"`，
    `:disabled="!config.channels.autoActivation"`（关时置灰）。
  - i18n `zh.ts` / `en.ts`：`settings.channels.autoEndWatchTitle` / `autoEndWatchDesc`。

## P2 — 新终态 `FinalKind::AutoStopped`（`watch.rs` + 各渠道渲染器）

- `watch.rs`：
  - `FinalKind` 增 `AutoStopped(String)`（携带**切换目标渠道展示名** `{to}`）。因引入 `String`，
    `FinalKind` 与 `CardMode` 去掉 `Copy`、改 `Clone`（决定采用此法：把动态文案逻辑收敛在 `final_label_text`，
    渲染器几乎不动）。
  - `final_label_text` 增 arm：`AutoStopped(to) => i18n "watch.btnAutoStopped".replace("{to}", to)`。
- 修正因去 `Copy` 而报错的少数「`mode` 按值重用」处（如 `dingtalk/watch.rs` 同时 `match mode` 与
  `matches!(mode, …)`）：改用 `match &mode` + 必要处 `.clone()`；`telegram/watch.rs`、`slack/watch.rs`、
  `feishu`(`card_view`) 的 `CardMode::Final(kind)` 解构本就各用一次，按需微调。
- i18n `watch.btnAutoStopped`（zh/en，含 `{to}` 占位）。
- 单测：`final_label_text(FinalKind::AutoStopped("本地弹窗"), Zh)` == 「已切换到 本地弹窗 · 自动结束关注」。

## P3 — 抽共享收尾 helper + 挂钩 `set_active_channel`（`daemon/mod.rs`）

- 抽一个 `async fn finalize_and_drop_watches(state, channel_id, final_kind, config, lang)`（或等价签名）：
  对某渠道选中的一批订阅，逐个 `WatchClient::edit` 定格 + 从 `subs` 移除 + `persist_watch_subs` +
  `notify` + `ensure_watch_routes`。`handle_unwatch_cmd` 的收尾段改为复用它（行为不变）。
- `set_active_channel`：在既有「旧渠道反激活提示」块（`if let Some(old) { if old != "popup" && old != new_id { … } }`）
  内，追加：
  ```
  if cfg.channels.auto_activation && cfg.channels.auto_end_watch {
      finalize_and_drop_watches(state, &old, FinalKind::AutoStopped(channel_label(new_id, lang)), &cfg, lang).await;
  }
  ```
  （D4：不发额外文字；反激活提示照旧。注意锁作用域：读取 `old` 渠道订阅时用临时 guard，勿跨 await 持锁。）

## P4 — 文档 + 验证

- 更新 `docs/overview.md`「IM 会话期自动激活」节：补「按需发送子开关：切走活跃槽自动结束该渠道 watch」。
- 更新 `docs/PROGRESS.md`：加本任务的待验收 section。
- `cargo test` 全绿 + `./scripts/install.sh`（用户确认后再装，沿用本会话约定）。

## 影响面小结

- 改动文件：`config.rs`、`watch.rs`、`telegram/watch.rs`、`slack/watch.rs`、`dingtalk/watch.rs`、
  `daemon/mod.rs`、`i18n.rs`、`src/lib/types.ts`、`src/views/SettingsView.vue`、`src/i18n/{zh,en}.ts`、docs。
- 无新增模块、无新增 IPC、无协议变更；纯行为在 daemon 内闭环。
