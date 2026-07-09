# 开发计划：Dev Instance —— 多 WorkTree 并行开发隔离

> 关联需求：`docs/specs/dev-instance-parallel.md`
> 计划描述方案与技术/规则细节；具体代码以实现为准。
> 状态：设计已对齐；**P0–P2 已在 `feat/dev-instance-parallel` worktree 落地**（待 review / commit）

## 0. 方案总览

为每个**已 enable 的 Git 工作树**提供一套互不共享的 Dev Instance（bin + home/daemon + popup-only 默认渠道）。主环境（`~/.local/bin` + `~/.askhuman`）保持今日语义。

运行时靠 **cwd 向上找 `.askhuman-dev/enabled`** 自动改道，使 agent 提示词仍只需 `install.sh` + `AskHuman`，无需按 worktree 写分支逻辑。

```
cwd 命中 <root>/.askhuman-dev/enabled
  ├─ install.sh → INSTALL_DIR=<root>/.askhuman-dev/bin
  │                seed home + popup-only config（若缺）
  └─ AskHuman   → re-exec <root>/.askhuman-dev/bin/AskHuman
                   env: ASKHUMAN_HOME=<root>/.askhuman-dev/home
                        实例模式禁主钥匙串
                   paths::config_dir() → ASKHUMAN_HOME
```

多 WorkTree = 多个并列 `<root>/.askhuman-dev/`，每套独立 flock/sock/config；**没有**「全局一份 dev 配置」。

---

## 1. 路径与环境（`paths.rs` 等）

### 1.1 `ASKHUMAN_HOME`

- 新增：若 `std::env::var("ASKHUMAN_HOME")` 非空，则 `config_dir()` = 该路径（绝对路径；相对则相对 cwd 规范化一次）。
- 未设置 → 现状 `home()/.askhuman`。
- 所有今日依赖 `config_dir()` 的路径（daemon sock/lock/meta/log、history、agents、state、gui-host、binhash、update 状态等）自动落入实例 home，**不必逐个改**。
- `home()` / 集成路径（`~/.cursor` 等）**不**随 `ASKHUMAN_HOME` 改变（避免 perf 改 `HOME` 那类误伤）；实例隔离只圈 AskHuman 自有数据。

### 1.2 实例模式判定

- `is_dev_instance()`：`ASKHUMAN_HOME` 已设置，或（实现期）内部 flag；用于密钥与 config 回退策略。
- 实例模式下：
  - 不读 `legacy_config_dir()` / 用户主 `~/.askhuman` 作为回退；
  - 等效 `ASKHUMAN_NO_KEYCHAIN`：不读不写系统钥匙串生产条目，密钥仅来自实例 `config.json`（0600）。

### 1.3 标记发现

- 公共函数（Rust + install.sh 各一份等价逻辑）：
  - `find_dev_root(start: Path) -> Option<PathBuf>`：从 `start` 向上，若 `ancestor/.askhuman-dev/enabled` 为文件则返回 `ancestor`。
  - 工作树根（仅 `dev enable` 用）：向上找含 `.git` 的祖先。

---

## 2. CLI 入口 dispatcher（`cli::dispatch` 极早路径）

在解析业务 argv 之前统一跑 `maybe_enter_dev_instance()`。

### 2.1 跳过改道的角色（信任父进程 env）

以下由 daemon/宿主 **spawn** 的角色**不做** cwd 找标记 / re-exec（避免 cwd 不在 worktree 时误伤或环）：

- `--popup`、`--popup --warm`
- `--gui-host`
- 其它明确「子角色」且已带 endpoint/token 的隐藏参数

这些进程应已继承父进程写入的 `ASKHUMAN_HOME`（daemon spawn 已透传 `ASKHUMAN_*`）。

### 2.2 命令分级

命中 `.askhuman-dev/enabled` 时：

| 类别 | 示例 | 实例 bin 缺失 | 实例 bin 存在且 ≠ current_exe |
|---|---|---|---|
| **A. 元命令** | `dev …`；`--help`/`-h`；`--version`/`-v`；`--agent-help`；`--scripting-help` | 用 **current exe**，仅 export `ASKHUMAN_HOME`+no-keychain（`dev` 必须能在未 install 时跑） | **可选 re-exec**（`version` 建议 re-exec 以显示本树版本；`dev` 用 current 或 re-exec 皆可，实现取：有 bin 则 re-exec，保证 dev 逻辑与本树代码一致） |
| **B. 配置/窗口** | `--settings`；`--history`；`config …`；`channel …` | 用 current exe + `ASKHUMAN_HOME`（**允许未 install 就开设置配 bot**，支撑 preset save --from-instance） | re-exec |
| **C. 运行时** | 提问；`daemon …`；`mcp`；`doctor`；`agents …`（除纯信息外） | **exit 1**，提示先 `./scripts/install.sh` | re-exec |

### 2.3 re-exec 细节

- env：`ASKHUMAN_HOME=<root>/.askhuman-dev/home`、`ASKHUMAN_NO_KEYCHAIN=1`。
- Unix：`exec` 替换；Windows：spawn 同参并 `exit` 子进程码。
- 已是目标 bin：只保证 env 后 continue。
- 死循环防护：目标 bin 路径与 current 相同则不再 exec。

### 2.4 `--settings` 与 GUI 宿主

- Unix 设置窗走 `gui_host::host_open`；host 的 sock/lock 在 `config_dir()` 下 → 实例模式下自动用**实例** gui-host，与主 tray **分离**。
- 故在已 enable 树内 `--settings` 写入实例 `config.json`，这正是 `--from-instance` 所依赖的路径。
- 未 enable 时 `--settings` 仍进主环境（现状）。
- **正确性约束**：`maybe_enter_dev_instance()` 必须在任何 `AppConfig::load*` / `host_open` / `run_settings` 之前设置好 `ASKHUMAN_HOME` 与 no-keychain；验收：未 install 改设置后主 `~/.askhuman/config.json` 与主钥匙串不变。

### 2.5 引导

主 `~/.local/bin/AskHuman` 需至少升级一次到含 dispatcher 的版本，PATH 入口才能改道到各 worktree bin。

---

## 3. Daemon / spawn / 指纹

- Daemon 进程继承 CLI 的 `ASKHUMAN_HOME`（`daemon/spawn.rs` 已透传 `HOME`/`TMPDIR`/`PATH`/`ASKHUMAN_*`，确认 `ASKHUMAN_HOME` 在透传集合内即可）。
- 单实例 flock 在实例 home 内 → 多实例天然并行。
- 二进制指纹仍按 `current_exe()` 内容哈希；各实例 bin 路径不同，**不会**因另一 worktree 安装而误判 stale。
- 实例内 install 换本树 bin → 本树 daemon 走既有 graceful drain；与其它树、主环境无关。

---

## 4. 配置 seed、渠道与预设

### 4.1 Seed

`dev enable` 或 install 发现 home 无 `config.json` 时写入最小配置：

- `general` 可与默认一致（主题/语言等本地偏好允许用默认值）。
- 所有 IM 渠道 `enabled: false`，密钥空。
- 不从主 config 拷贝任何字段。

### 4.2 实例内改渠道

- re-exec + `ASKHUMAN_HOME` 后，设置 GUI（`--settings`）、`channel …`、`config …` 均写当前实例 home。
- **`--from-instance` 日常路径**：`dev enable` → 本树打开 `--settings` 配测试 bot → `dev preset save <name> --from-instance`。

### 4.3 密钥

- 实例模式强制 no-keychain；测试 token 0600 明文在实例 `config.json`。
- 禁止实例模式把密钥迁入主钥匙串。
- 预设文件同样 0600 明文（测试 bot）；主 daemon 从不读 `dev-presets/`。

### 4.4 预设注册表与租约（`~/.askhuman/dev-presets/`）

- `paths`：`dev_presets_dir()` = **主** `home()/.askhuman/dev-presets`（注意：预设目录固定在用户主 ashuman 树下，**不**随实例 `ASKHUMAN_HOME` 变）。
- `index.json`：`presets.<name>.{ file, lease?: { worktreeRoot, claimedAt } }`；读写加文件锁（与 history.lock 同级 flock），防两树同时 enable 竞态。
- `<name>.json`：可并入 `AppConfig` 的渠道子集（仅 channel 段 + enabled）；`save --from-instance` 从当前实例 config 抽取「已配置」渠道（enabled 或关键 id/token 非空的段）。
- **物化**：enable 将预设渠道 deep-merge 进实例 `config.json`（general 等保留 seed/本地偏好）；写入实例侧元数据 `appliedPresets: string[]`（可放 `home/dev-meta.json` 或 config 内扩展字段，实现选简单且不进生产 schema 污染的方案——推荐旁路 `home/dev-meta.json`）。
- **占租约**：同名 lease 存在且 holder 的 `worktreeRoot/.askhuman-dev/enabled` 仍在且 root 规范化路径 ≠ 本树 → 失败；僵死（无目录或无 enabled）→ 回收；`--force` → 覆盖 lease。
- **释放**：`dev disable` 读本树 `appliedPresets`/`lease` 反查 index，holder 匹配本树则清空 lease；`preset release` 直接清。
- 一期 `--force` **不** stop 对方 daemon、**不**改对方 home。

---

## 5. `dev` 子命令（`cli`）

| 子命令 | 要点 |
|---|---|
| `dev enable [--preset name…] [--force]` | 解析 git 工作树根；创建标记与目录；seed config；处理 preset 租约与物化 |
| `dev disable [--purge]` | stop 本实例 daemon；去 `enabled`；释 lease；可选删整树 `.askhuman-dev` |
| `dev status` | root/home/bin/daemon/渠道/占用的 preset |
| `dev preset save <name> [--from-instance]` | 快照或交互写入预设文件 |
| `dev preset list\|show\|rm\|release` | 列表（脱敏）、详情脱敏、删除、只释租约 |

Dispatcher 特例：

- `dev …` 全组：实例 bin 缺失时仍可用**当前 exe**（否则无法第一次 enable / preset save）。
- 其它命令：要实例 bin。

---

## 6. `install.sh`

- 解析：从 cwd 向上找 `.askhuman-dev/enabled`。
- 命中且未传 `--global`：`INSTALL_DIR=<root>/.askhuman-dev/bin`；install 结束后可打印「dev instance: …」。
- `--global`：强制 `INSTALL_DIR=${INSTALL_DIR:-$HOME/.local/bin}`，忽略 dev 标记。
- Windows：`install-windows.ps1` 同等逻辑（若一期只做 Unix，在计划备注；推荐同期最小支持标记检测 + INSTALL_DIR）。
- 不在此脚本里 `dev enable`（enable 必须显式）。

---

## 7. GUI host 与弹窗

- GUI host 的 sock/lock 已在 `config_dir()` 下 → 随 `ASKHUMAN_HOME` 隔离，主环境 tray 与 dev 实例不抢同一 lock。
- 一期：dev 实例需要弹窗时仍 spawn `--popup`；是否显示 dev 专用 tray 不强制，避免干扰主菜单栏即可（无 lock 冲突即达标）。
- 预热池、helper token 均在实例 daemon 内，无跨实例问题。

---

## 8. 文档与仓库卫生

- 根 `.gitignore` 增加 `.askhuman-dev/`。
- `docs/development.md` 增加「并行 / WorkTree 开发」小节（链到 agent 向文档）。
- **Agent 向：WorkTree 准备文档**（新建，建议路径 `docs/agent-worktree-setup.md`）：
  - 何时读：创建/使用 git worktree 做本项目功能开发时（含主工作树若要并行隔离）。
  - 步骤：`dev enable` →（可选渠道）→ `install.sh` → 用 AskHuman 验证。
  - **必须用 AskHuman 询问人**：本 worktree 是否需要 IM 测试渠道；若需要，列出 `dev preset list` 已有预设供选择，或引导「先空 enable → settings 配置 → preset save」；popup-only 则不带 `--preset`。
  - 说明租约冲突时如何处理（对方 disable / `--force` 含义）。
  - 明确 **不要**改 agent 日常提示词分支：enable 完成后仍只 `install.sh` + `AskHuman`。
  - 说明主环境与 dev 实例差异、禁止把生产 bot 当预设（无 from-main）。
- **`Agents.md`**：在「Before a complex task」附近增加一节——**若任务需要新建或进入 git worktree 做并行开发，先读 `docs/agent-worktree-setup.md` 并按其准备**；验证步骤仍写 `install.sh`（dev 标记下会自动装到实例 bin）。
- `docs/overview.md`：实现落地后补 Dev Instance 一句。

---

## 9. 测试

- 单元：`find_dev_root` 向上查找 / 最近命中；`config_dir` 尊重 `ASKHUMAN_HOME`；实例模式不碰 keychain（可继续用 `ASKHUMAN_NO_KEYCHAIN` 单测路径）。
- 集成/脚本级（可选）：临时目录搭两套 fake root+enabled+bin stub，断言 re-exec env 或「bin 缺失报错」。
- 手工验收：对照需求 §10 清单（双 worktree 并行提问 + 主环境不被 drain）。

---

## 10. 实现分期

| 阶段 | 内容 | 交付 |
|---|---|---|
| **P0** | `ASKHUMAN_HOME` + 实例禁钥匙串/禁主 config 回退；`find_dev_root`；入口 re-exec 与 `dev` 特例；gitignore | 手造目录即可双实例并行 |
| **P1** | `dev enable\|disable\|status`；`install.sh` 识别标记与 `--global`；seed config | 一次 enable 后提示词零改 |
| **P1b** | `dev-presets` 目录 + 租约锁 + `preset save/list/show/rm/release` + `enable --preset/--force` 物化 | 新 WorkTree 点名 bot，免重复手填 |
| **P2** | `docs/agent-worktree-setup.md` + `Agents.md` 引用；development.md；doctor 摘要；overview | Agent 可按文档准备 worktree |
| **P3**（可后置） | Windows install 对齐；MCP 错误 cwd 的辅助 env；dev tray 策略收紧；force 时可选 stop 对方 | 打磨 |

---

## 11. 风险与边界（写入实现注释/文档即可）

- 主 bin 过旧无 dispatcher → 无法自动改道；需先全局安装一版含本功能的 AskHuman。
- MCP/hook 若 cwd 不是 worktree → 走主环境；依赖各 agent 以 workspace 为 cwd（当前主流如此）。
- 全局 lifecycle hook 仍安装在用户级；事件进入「实际 exec 到的」daemon，文档说明即可。
- 用户若把同一测试 bot 配进两个运行中实例，平台层冲突——产品默认 popup-only 降低误触，文档警告。

---

## 12. 明确不改

- 主环境单 Daemon + 生产 bot 模型。
- stdout 洁净、退出码、抢答、graceful drain 协议（仅作用域变为「每实例」）。
- Agent 提示词内容（由目录标记驱动，不要求提示词分支）。
