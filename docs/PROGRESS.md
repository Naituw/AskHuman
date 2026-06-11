# PROGRESS

按具体任务 / 需求记录待办与当前进展。任务 / 需求完成后删除其 section（历史留在 git）。

## 进行中：版本自更新机制（实现阶段）

需求/方案：`docs/specs/self-update.md`、`docs/plans/self-update.md`；提交规范见 `AGENTS.md`。

已完成：
- ① `update/` 模块（`mod`/`direct`/`npm`/`notes`/`state`）+ 单测 8 过。
- ② `paths::update_state_file` + `update.json` 状态读写 + 命令
  （`get_app_version`/`update_check`/`update_get_notes`/`update_apply`/`update_dismiss`/`restart_settings`）
  + 注册到 invoke handler。
- ④a 设置「关于」区：当前/最新版本、检查更新、更新（进度）、更新日志（聚合 markdown）、
  「查看全部发布」、更新后「重启设置页面」；i18n(zh/en) + `lib/ipc.ts` 封装 + `UpdateInfo` 类型。
  cargo 编译、`npm run build`、`cargo test update::` 均过。

- ③ `ipc ServerMsg::UpdateState`（snake_case 字段，同二进制两端）+ daemon：启动+24h 后台检查→落
  `update.json`→变化广播；15s 指纹监听→外部/应用内更新置 `pending` 并广播；GuiHello 握手携带当前态。
  `commands` 增进程内缓存 + `popup_update_state` 拉初值命令；GUI Helper 读 `UpdateState`→缓存+emit `update-state`。
- ④b 弹窗：右上角更新入口（圆点）+ 浮层（版本/日志/「答完生效」/更新按钮）+ 待生效横条；
  挂载先 `popup_update_state` 取初值再监听事件；zh/en i18n。
  cargo 编译、`npm run build`、`cargo test`(update::/ipc:: 共 16 过) 均通过。

- ⑤ 发布流程：仓库根 `cliff.toml`（按 D15/D16/D20：仅 feat/fix/perf/security/revert；breaking 置顶；
  scope 粗体前缀；`Release-Note:`/`Release-Note: skip` 单条覆盖——skip 改由 body 模板按 footer 过滤，
  避免无 body 提交触发 field error 误伤 feat/fix）；`release.yml` 接 git-cliff（`fetch-depth:0` +
  `taiki-e/install-action`），按 `docs/release-notes/v<版本>.md` 覆盖否则 `--latest` 生成、去前导空行、
  `body_path` 替换 `generate_release_notes`；新增 `docs/release-notes/README.md`。本地 git-cliff 2.13.1
  跑通 v0.4.x→v0.5.x 多版本，分组/跳过/中英文/Full Changelog 均正确。

⑥ 完整链路 install 实测：**已通过**（用 `GITHUB_TOKEN=$(gh auth token)` 注入认证额度绕过 60/时限流）。
- 降级 0.5.0（Cargo.toml/tauri.conf）重装 → 带 token 重启 daemon → 后台检查**无 403**、测到 0.5.3。
- 弹窗更新入口/浮层/「答完才生效」提示、关于区当前 0.5.0/最新 0.5.3 均正常。
- 点更新 → 下载官方 0.5.3 资产 → `codesign` 验签 TeamID `DMJXDB9H6Q` 通过 → 备份 `AskHuman.0.5.0.bak`
  → 原子替换；置 `pending` + 顶部「待生效」横条。答完在途请求后**下一次提问握手触发 drain→重拉**，
  daemon 换新到 0.5.3（status 确认 pid 变更、version 0.5.3）。
- 实测中修的问题：
  1. 浮层背景透明（`--bg-elevated` 仅 3~6% alpha 透出底字）→ 改用不透明 `--bg`。
  2. 更新入口图标改橙色（`--accent-orange`）+ 与「置顶」按钮加 4px 间距。
  3. **更新日志里的链接在 webview 内跳转把窗口顶掉** → 弹窗 `.up-notes`、设置 `.release-notes` 均接
     外链处理（`onContentClick`/`onNotesClick`：`openPath` 走系统浏览器）。
  4. **`init_update_snapshot` 启动残留 `pending`** → 刚启动的 daemon 即盘上二进制，pending 一律清零
     （否则换新后下个 daemon 常驻「待生效」横条）。
- 已恢复版本号到 0.5.3 并重装回开发态（dev-0.5.3，带自更新功能 + 本地签名）。
  注意：本地 dev 签名（自签证书）与官方 Developer ID 不同，跨签名换新时钥匙串会就各 secret 重新授权
  （点「始终允许」即可）；官方→官方升级签名一致则**不会**弹。
- 追加（用户要求，已随 install 落盘）：设置「关于」区「查看当前版本更新日志」折叠项
  （`update_get_version_notes` → `notes::notes_for_tag`，懒加载）。
- 追加（用户要求，已随 install 落盘）：限流处理——`github_client()`（带可选
  `ASKHUMAN_GITHUB_TOKEN`/`GITHUB_TOKEN` → `Authorization: Bearer`，token 头 sensitive）与 `http_client()`
  （npm/资产下载，不带鉴权头防泄露）分离；`github_status_error()` 把 403/429 归一为 `rate-limited`；
  前端映射友好文案 `settings.about.rateLimited`/`popup.update.rateLimited`。
- 全套 `cargo test` 通过；`npm run build` 通过；git-cliff 本地验证多版本。

自更新主体已提交：`feat(update)` + `ci(release)` + `docs`（3 条，未 push）。

追加修复（实测后用户反馈，未提交）：
- 原生关闭按钮也走二次确认：后端 `CloseRequested` 收尾态放行、否则 `prevent_close()` + emit
  `popup-close-requested` → 前端走与 ⌘W 相同的 `requestCancel()`（`GuiBridge::is_done` /
  `Coordinator::is_finalizing` 判收尾，避免拦截死循环）。
- 取消确认面板 `.confirm-box` 背景透明 → 改不透明 `--bg`（同更新浮层修复）。
  改动文件：`src-tauri/src/app/mod.rs`、`src/views/PopupView.vue`；install 实测两项均通过。
