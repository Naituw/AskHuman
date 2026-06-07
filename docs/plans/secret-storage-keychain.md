# 计划：渠道密钥安全存储（系统钥匙串 + 构建签名）

> 关联需求：`docs/specs/secret-storage-keychain.md`

把改造拆成可独立编译/验证的阶段。核心思路：**先在配置层加一层「密钥解析/落盘」抽象，让上层读取代码零改动；再做设置页与 daemon 衔接；最后补构建签名（发布前置）。**

## 0. 模块改动总览

```
src-tauri/src/
  secrets.rs          [新] 密钥库封装：service/account 常量、get/set/delete、可用性探测
  config.rs           [改] load 解析密钥（库优先, config 兜底）+ 自动迁移；save 剥离密钥写库、兜底写明文(0600)
  commands.rs         [改] 设置「读配置」返回密钥置空 + 各项「是否已配置」标志；「存配置」按 不变/覆盖/清除 处理密钥
  daemon/mod.rs       [改] 无需改比对逻辑（内存携带解析后真值）；仅确认 on_config_changed/invalidate 走解析值
src/
  views/SettingsView.vue  [改] 三项密钥改「已保存」占位 + 清除按钮；提交时带每项意图
  lib/ipc.ts / types.ts   [改] 配置读写类型：密钥占位 + secretsPresent + 每项 secretAction
scripts/install.sh    [改] 用证书签名(自动探测 + CODESIGN_IDENTITY 覆盖 + ad-hoc 回退) + 固定 identifier
.github/workflows/release.yml  [改] macOS target 用 Developer ID(从 secrets) 显式签名 + 固定 identifier
src-tauri/Cargo.toml  [改] 新增 keyring 依赖（按平台 feature）
```

> 复用而非重写：`config.rs` 的容错解码、`save_to` 原子写 + 0600 收紧（阶段 A 已加）保留；渠道连接、`daemon` 的 `invalidate_changed_routers` 比对逻辑**不改**——它比对的是内存中解析后的密钥值，迁库后自然仍能识别变化。

## 阶段 A（已完成）：权限收紧 + 输入遮挡

- `config.rs`：`save_to` 设文件 0600、目录 0700；`load` 对历史文件自愈；新增单测。
- `SettingsView.vue`：`botToken` 改密码框。
- 状态：已实现、`cargo test` 通过、已安装并实测权限为 0600/0700。

## 1. 阶段 B1：密钥库封装 `secrets.rs`（已完成）

目标：提供与上层解耦的密钥库读写，跨平台 + 可用性判定。

- `Cargo.toml` 增加：`keyring`，按 `cfg` 启用 `apple-native`(macOS) / `windows-native`(Windows) / Secret Service(Linux)。
- 常量：`SERVICE = "com.naituw.humaninloop"`；三个 account 字符串（见 spec §5.2）。
- API（同步、尽力而为）：
  - `get(account) -> Result<Option<String>, Unavailable>`：命中 `Some`；`NoEntry` → `None`；其它错误 → `Unavailable`。
  - `set(account, value) -> Result<(), Unavailable>`；`delete(account) -> Result<(), Unavailable>`。
- 不在本模块做迁移/优先级，仅薄封装；上层（config.rs）编排策略。

验证：`cargo build`；可加一个被 `#[ignore]` 的本机往返测试（CI 无密钥库，默认不跑）。

## 2. 阶段 B2：配置层接入解析/落盘/迁移 `config.rs`（已完成）

目标：上层读取零改动，密钥真值来自密钥库（兜底 config），并完成自动迁移。

- 定义「三项密钥」的统一描述：account ↔ `&mut String` 字段访问器（钉钉/飞书/Telegram 各一）。
- `AppConfig::load()`：
  1. 读 `config.json`（含历史明文）。
  2. 对每项密钥按 spec §5.3 解析，把**解析后的真值**填入内存结构。
  3. 自动迁移（§5.5）：字段非空且库可用 → `set` → 清空字段 → 标记需重写；循环后若有迁移且非回退 → 原子重写 `config.json`。
- `AppConfig::save_to()`：
  - 构造「写盘副本」：对每项密钥，若成功 `set` 入库 → 副本该字段置空；若 `Unavailable` → 副本保留明文（回退）。
  - 序列化写盘副本（沿用临时文件 + rename + 0600）。内存 `self` 不变（仍持解析值）。
  - 注意：`save_to` 接收 `&self` 需要可写副本——改为内部克隆出可改副本再剥离，签名保持 `&self`。
- 兼容：`load_from`(供测试) 维持「纯文件」语义不接触密钥库；密钥库交互只在 `load`/`save_to` 真实路径（或抽出可注入开关，避免单测触碰系统库）。为保单测纯净：把密钥库步骤包在「仅 `save`/`load` 默认位置」路径，`*_to(path)` 不碰库。

验证：`cargo test`（现有 config 单测 + 迁移/回退的纯逻辑测试，用可注入的假密钥库或仅测「字段剥离/优先级」纯函数）。

## 3. 阶段 B3：设置页交互（已完成）

目标：密钥不流出库，UI 用「已保存」占位 + 清除。

- 后端「读配置」命令（commands.rs）：返回的配置三项密钥置空，并附 `secretsPresent { dingding, feishu, telegram }`（由「字段非空 或 库中存在」推断）。
- 后端「存配置」命令：入参对每项密钥带 `secretAction`：`unchanged` / `set(value)` / `clear`。
  - `unchanged`：保持现状（不写库、字段保持空）。
  - `set`：按 B2 落盘逻辑写入。
  - `clear`：`delete(account)` + 字段空。
- 前端 `SettingsView.vue`：三项密钥输入框初始为空 + 占位「已保存/未设置」；用户输入即 `set`；提供「清除」按钮置 `clear`；未触碰为 `unchanged`。`lib/types.ts`/`ipc.ts` 增加对应字段。

验证：装好后在设置页：未配置→输入→保存→`config.json` 字段空 + 库有条目；重开设置显示「已保存」；清除→条目删除；留空保存→不变。

## 4. 阶段 B4：daemon 衔接确认（已完成）

目标：确认热重载/惰性失效在迁库后仍正确，必要时仅做最小适配。

- `on_config_changed` 调 `AppConfig::load()` → 已自动从库解析，`invalidate_changed_routers` 比对解析后的 `client_secret`/`app_secret`/`bot_token` → 改密钥即失效对应 Router。无需改比对代码。
- 确认 daemon 各处读密钥都经 `AppConfig`（已是）；不新增 IPC。
- 边角：设置改密钥触发原子写 → 监听脉冲 → 重载 → 失效。验证此链路。

验证：daemon 运行中改某渠道密钥 → 下一个请求用新密钥连接（旧请求保留旧连接至结束）。

## 5. 阶段 C：构建签名

目标：本地与发布二进制具稳定签名身份，使密钥库读取免弹框且安全不打折。

### C1 本地 `scripts/install.sh`（已完成）
- 替换现「ad-hoc 重签」为：
  - identity = `${CODESIGN_IDENTITY:-<自动探测首个有效 codesigning 证书>}`；探测不到则 `-`（ad-hoc）。
  - `codesign -i com.naituw.humaninloop --force --sign "$identity" "$INSTALL_DIR/AskHuman"`。
- 打印实际所用 identity，便于排查。

验证：装两次（改动源码使 cdhash 变）后，daemon/设置读密钥库不再弹框（对照阶段实测）。

### C2 发布 `.github/workflows/release.yml`（仅 macOS 两 target）（已完成，v0.4.0 发布实测通过）
- 新增「导入证书 + 签名」步骤：从 Secrets（`.p12` base64 + 密码 + 临时钥匙串密码）建临时钥匙串、导入、解锁、设搜索域，再
  `codesign -i com.naituw.humaninloop --force --options runtime --sign "Developer ID Application: …" <bin>`（`--options runtime` 可留作未来公证用，不公证亦可保留）。
- 显式 identity，不自动探测；构建后校验 `codesign -dvv` 含预期 TeamIdentifier。
- 不做 notarization。

前置（用户操作，非代码）：在仓库配置上述 Secrets；准备 Developer ID Application 证书并导出 `.p12`。

验证：v0.4.0 release CI 两 macOS target 均通过，签名步骤输出 `Identifier=com.naituw.humaninloop`、`Authority=Developer ID Application: Wu Tian (DMJXDB9H6Q)`、`TeamIdentifier=DMJXDB9H6Q`，`codesign --verify --strict` 通过且「satisfies its Designated Requirement」。

> 注：macOS 构建 runner 必须为 `macos-26`（提供 macOS 26 SDK）——Swift 语音桥用到 `SpeechAnalyzer` 等 26 专有 API，`macos-14`(Sonoma SDK) 无法编译。`build.yml` 同步改 `macos-26`。

## 6. 实施顺序与验证节奏

B1 → B2 →（`cargo test`）→ B3 →（装机实测设置页）→ B4（daemon 实测改密钥）→ C1（本机实测免弹框）→ C2（CI，发布前）。每步保持编译通过；按 AGENTS.md 装机后用新 `AskHuman` 验证。
