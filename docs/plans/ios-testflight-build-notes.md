# iOS 出包前变更清单与 TestFlight 测试内容 实施计划

需求与决策见 [规格](../specs/ios-testflight-build-notes.md)。状态：2026-11-04 起草，同日用户批准（含换 helper、补标签 3–6、对 build 6 做 read 与 probe，三处默认做法不改），同日已实现并验到 build 6；build 7（同日）首次真用，整条链走通；出 build 7 时用户把确认门改为默认不经（规格 T12，下文凡写「确认门」「出包前确认」处都按此理解，见 §10 末尾）。

本计划自成一体：先复述要做什么与全部规则，再给改动清单、各部分的设计、验证与提交。**任何与本计划不一致的做法，先经 AskHuman 问用户。**

## 0. 要做什么

给项目 skill `ios-testflight` 的出包流程增加两处，并给签名 helper 增加一个窄操作：

1. **出包前**：列出自上个构建以来 `ios/` 下的全部提交，连同起草好的中英文「测试内容」经 AskHuman 给用户确认；确认之前不归档、不上传。
2. **上传后、构建 `VALID` 时**：用 helper 把确认过的中英文测试内容写进该构建的 TestFlight「测试内容」，读回核对；然后在出包提交上打本地标签 `ios-testflight-N`。
3. **helper 新增固定操作 `notes`**（`read`／`write`／`probe` 三种模式）：这是 App Store Connect 凭据所在的程序，所以边界很窄，见 §3。

起因：2026-11-04 出 `0.1.0 (6)` 时只确认了 Max 一项，包里还带上了用户此前选过「先不出」的新版项目选择与 Agents 页改动。

### 全部规则

- 清单＝`git log <基线>..<出包提交> -- ios`，列出全部非合并提交（含 test／refactor／chore），并附 `git diff --stat` 的汇总行（防止合并提交本身改了 `ios/` 而清单里看不到）。`ios/project.yml` 的所有源码都在 `ios/` 下，路径过滤是完整的。
- 基线＝本地附注标签 `ios-testflight-N`（N＝构建号）所指的提交；取构建号最大的标签。本机缺标签、标签不是出包提交的祖先、或 Apple 上最大的构建号（按数字比较，取 `status.mjs` 返回的构建）与最大标签号不一致，一律停下问用户，不猜。标签只在本地，不推送。
- 出包必须来自一个确定的提交：默认当前 HEAD 且工作树干净；有未提交改动就停下问用户。
- 测试内容：`zh-Hans`＋`en-US` 两份纯文本，第一行 `构建 N · 源码 <7 位提交>`／`Build N · source <7 位提交>`，正文是用户视角的人话摘要，再加 1–3 条「请重点看」；总长 ≤ 4000 字（目标 ≤ 1500）；不写密钥、本机路径、未公开信息；若包里带了用户此前选择「先不出」的改动，必须明写。
- 确认门选项：按这样出包／修改文字／换出包提交点／取消。选「按这样出包」后，归档、上传、等 `VALID`、写测试内容、打标签全部自动完成。
- 排除某些提交只能换出包提交点（main 历史里更早的提交）；换点后被丢掉的提交集合必须恰好等于用户想排除的集合，否则（排除项夹在中间）停下问用户；Agent 不 cherry-pick、不拼包。换点出包从该提交的干净临时 worktree 归档。
- 写测试内容失败不重传（包已送达），只重试 `notes`。标签已存在时不覆盖，停下问用户。
- build 6 的测试内容不回填；只用它做一次 `probe` 实测。

## 1. 改动清单

| 文件 | 改动 |
| --- | --- |
| `.agents/skills/ios-testflight/scripts/changes.mjs`（新） | 取基线、校验、列清单、校验「被丢掉的集合」（§2） |
| `.agents/skills/ios-testflight/scripts/TestFlightHelper.swift` | 新增 `notes` 操作（§3），沿用现有的 JWT、会话与互斥锁 |
| `.agents/skills/ios-testflight/scripts/NotesRules.swift`（新） | 纯逻辑：请求解析、规范化、限额、接口白名单；与 helper 一起编译，也单独编译做测试 |
| `.agents/skills/ios-testflight/scripts/NotesRulesTests.swift`（新） | 上述纯逻辑的测试可执行文件（不依赖 XCTest），读取共享用例文件 |
| `.agents/skills/ios-testflight/scripts/notes-rules-cases.json`（新） | 共享用例：输入与期望的规范化结果或拒绝原因；Swift 与 Python 两套校验都对着它测，防止两边规则漂移 |
| `.agents/skills/ios-testflight/scripts/helper.py` | `notes` 子命令（§4）；`install` 编译两个源文件，并在替换前备份旧 helper |
| `.agents/skills/ios-testflight/SKILL.md` | 流程改写（§5） |
| `.agents/skills/ios-testflight/references/signing-and-recovery.md` | 凭据边界一节补上 `notes`、备份与回滚 |
| `scripts/ios-testflight-changes.node-test.mjs`（新） | `changes.mjs`、`helper.py notes` 校验与 Swift 纯逻辑测试的入口（`npm test` 已会运行 `scripts/*.node-test.mjs`；非 macOS 或没有 `swiftc` 时跳过 Swift 部分） |
| 本地标签 `ios-testflight-3/4/5/6` | 历史补标（§6） |
| `docs/plans/ios-channel.md`、`docs/PROGRESS.md` | 实施记录；build 7 首次真用的待验收项 |

在本 worktree 基于当前 main 新开分支 `chore/ios-testflight-notes` 实施；完成后问用户再并回 main。

## 2. `changes.mjs`

输入：`--tip <提交>`（默认 HEAD，＝默认会出的那个提交）、`--point <提交>`（出包提交，默认等于 tip）、`--expect-dropped <哈希,…>`（可选）、`--json`。不连 Apple、不读凭据。

流程：

1. 解析提交为全长哈希。`point` 就是 HEAD 时，要求 `git status --porcelain` 为空，否则以 `dirtyTree` 失败。
2. 列出 `ios-testflight-*` 标签并解析构建号，取最大者为基线；一个都没有就以 `noBaseline` 失败。基线提交必须是 `point` 的祖先，否则 `notAncestor`。
3. 清单：基线之后到 `point`、限于 `ios/` 的非合并提交，按从旧到新，每条含全长哈希、短哈希、日期、标题、约定式提交类型（不是约定式格式的记为 `other`）；另附 `git diff --stat <基线> <point> -- ios` 的汇总。清单为空时标 `empty: true`。
4. 建议的下一个构建号＝基线构建号＋1（与 Apple 上的最新构建号核对由 skill 用 `status.mjs` 完成，脚本不联网）。
5. `point` 与 `tip` 不同时：`point` 必须是 `tip` 的祖先；「被丢掉的集合」＝`point` 之后到 `tip`、限于 `ios/` 的非合并提交。给了 `--expect-dropped` 时，该集合必须与给定哈希（允许前缀）集合完全相等，多出或缺少都以 `droppedMismatch` 失败并列出差异。
6. 输出文本（给人看）或 JSON（给 Agent 用），含 `reason` 字段表示失败原因；失败时退出码非 0。

## 3. helper 的 `notes` 操作

**调用方式**：helper 现在接收「配置、结果文件、操作名、可选参数」。新增操作名 `notes`，参数是请求文件路径。请求文件是 JSON：`mode`（`read`／`write`／`probe`）、`build`（构建号字符串）、`texts`（仅 `write`：语言到文本的映射）。

**请求校验（`NotesRules.swift`，启动器那边再做一遍）**：

- `mode` 只能是上述三个；`build` 必须匹配 `^[1-9][0-9]{0,8}$`；`write` 必须带 `texts` 且至少一种语言，`read`／`probe` 不得带。
- 语言只能是 `zh-Hans`、`en-US`。文本规范化：`\r\n`、`\r` 统一为 `\n`；去掉整段末尾连续的空格（U+0020）与换行，其他空白字符（如不换行空格）原样保留，这样 Swift 与 Python 两边不会因为各自的「空白」定义不同而漂移；拒绝除换行外的控制字符（U+0000–U+001F、U+007F）。检查顺序固定为：控制字符、是否为空、长度。规范化后长度 1–4000，**按 UTF-16 单元计**（是字符数、标量数、字形簇数里最严的一个），超长直接拒绝，不截断。
- 请求文件必须是普通文件、属于当前用户、权限 `0600`、不超过 64 KiB。

**接口白名单**（同一个函数集中检查方法与路径形状，其他一律拒绝）：

- `GET /v1/builds`，过滤为本 App（`6828712684`）与构建号，取 `version`、`processingState`、`expired`；必须恰好一条，且 `processingState=VALID`、`expired=false`。
- `GET /v1/betaBuildLocalizations`，按该构建过滤，取 `locale`、`whatsNew`。
- `POST /v1/betaBuildLocalizations`（创建，关联该构建）、`PATCH /v1/betaBuildLocalizations/<id>`（只改 `whatsNew`）。
- `DELETE /v1/betaBuildLocalizations/<id>`：**只在 `probe` 模式下、且 `<id>` 必须是本次运行自己刚创建的那一条**。

**各模式**：

- `read`：列出现有语言与文本，不写。
- `write`：对 `texts` 里的每种语言：线上已有且文本相同→`unchanged`；已有但不同→`PATCH`（`updated`）；没有→`POST`（`created`）。全部处理完后重新读取，逐字比对规范化文本，不一致即失败。
- `probe`：先读。对该构建上**当前没有文本**的允许语言，各做一遍：`POST` 标记文本（固定常量，两行：第一行是「AskHuman helper probe, safe to ignore」，第二行以 `•` 开头并含中文——这样读回比对同时覆盖换行、项目符号与非 ASCII 字符的处理）→ 读回比对 → `DELETE` → 再读确认已不存在。两种语言都已有文本时拒绝，从不碰已有文本。中途任何一步失败：尽力 `DELETE` 自己创建的条目，并在输出里报告残留的资源 ID。
- `write` 与 `probe` 取 helper 现有的互斥锁（与导出同一把），同一时刻只有一个修改操作；`read` 不取锁。

**网络与错误**：沿用现有的 5 分钟内存签名 JWT 与临时会话，单次请求超时 30 秒，不自动重试。非 2xx：失败并输出状态码与 Apple 返回的 `errors[].code／title／detail`（各截 300 字；这些不是机密）；401／403 单独提示「授权问题，交给用户」。**永不输出令牌、私钥或请求头。**

**输出**（JSON，沿用 helper 现有的 `emit`）：`mode`、`build`、`appleBuildId`、各语言的动作（`created`／`updated`／`unchanged`／`probed`／`absent`）与字数、`verified`；`read` 另含现有文本；`probe` 失败时含 `leftover` 资源 ID 列表。退出码沿用现有约定写入结果文件。

**不变量**：不新增 Keychain 条目、不新增任何能读出凭据的命令；helper 标识、Developer ID 签名、`migrate`／`status`／`upload`／`export` 的行为都不变。

## 4. `helper.py`

- `notes --build N --mode read|write|probe [--zh 文件] [--en 文件] [--dry-run]`：文本文件按 UTF-8 读取。先在 Python 里做与 §3 相同的校验，把规范化后的请求写成 `0600` 的临时 JSON（放在启动器现有的临时目录里，结束后随目录删除），作为参数交给 helper。超时沿用非上传操作的 90 秒。
- `--dry-run`：只做校验并打印将要执行的操作（模式、构建号、各语言字数、将调用的接口列表），**不启动 helper、不连网、不碰凭据**。
- `install`：编译 `TestFlightHelper.swift` 与 `NotesRules.swift`；替换前把现有的 `testflight-helper` 复制为 `testflight-helper.previous`（`0700`，覆盖旧备份）。回滚就是把它改回原名。

## 5. SKILL.md 的流程改写

保持现有步骤与边界，改写与新增如下（具体措辞实施时写）：

1. **确定出包提交**：默认当前 HEAD 且工作树干净，否则停下问用户。先用 `status.mjs` 读 Apple 上最大的构建号，再运行 `changes.mjs`，核对「Apple 最大构建号＝最大标签号」。基线缺失、不是祖先、号不一致就停下问用户。清单为空，告诉用户「没有 iOS 改动」，由用户决定是否仍出包。
2. **起草并确认**：按规格写中英文测试内容；经 AskHuman 把「构建号与版本、基线标签与出包提交、全部提交清单、两份草稿全文」给用户，选项为按这样出包／修改文字／换出包提交点／取消。选「修改文字」就改后再确认；选「换出包提交点」按排除规则处理（用 `--point` 与 `--expect-dropped` 校验，被丢掉的集合不符就停下问用户），并在该提交的干净临时 worktree 里归档；其余情况在当前 worktree 归档。
3. **归档、预检、上传**：与现有步骤相同（构建号取基线构建号＋1）；归档前后各校验一次 HEAD 与工作树状态未变，变了就停下问用户后再打标签。
4. **等 `VALID`**：沿用现有检查（在内部组里、测试员状态）。
5. **写测试内容**：`helper.py notes --mode write`，读回核对；失败不重传，按错误码处理后只重试这一步。
6. **打标签**：`git tag -a ios-testflight-N <出包提交>`，信息含版本、构建号、Apple 构建 ID、日期；已存在就停下问用户。
7. **记录与告知**：更新 `docs/plans/ios-channel.md` 的出包记录（源码提交、基线标签、清单要点、中文测试内容全文、Apple 构建 ID 与状态）；把确切的版本与构建号告诉用户。

「边界与恢复」一节补充：`notes` 的范围；失败处理表（规格 §5）；备份与回滚；这类改动属于「经审阅的 helper 更新」。

## 6. 标签

历史补标签（附注，信息写明版本、构建号、Apple 构建 ID，并注明「按 `docs/plans/ios-channel.md` 的出包记录于 2026-11-04 补标」）：

| 标签 | 提交 | Apple 构建 ID |
| --- | --- | --- |
| `ios-testflight-3` | `5ef38ead3527b66ed64c898ab4869db9e430db6f` | `6b125b24-1856-404f-bcdc-c7d109dd4274` |
| `ios-testflight-4` | `4381628c617cc61d1c0f2d12d7cddb31797b7e93` | `b29e802b-ff20-44c3-9f0f-889c093646f8` |
| `ios-testflight-5` | `2295390a19e04a0b250d4aacd41ce5543d32742d` | `56574ec7-6cda-4ded-904f-94485ec9f41e` |
| `ios-testflight-6` | `9f00abcb70058bd5da4144dc65819d05324efbe8` | `cf44dca8-03dc-4c31-9d3c-f4acb6424ef2` |

build 2 的出包记录没有写源码提交，不补。这四个提交都已核对存在、且都在 main 的历史里。标签只在本地；标签名不匹配发布流程的 `v*` 触发条件。

## 7. 验证计划（逐步进行，任一步与预期不符就停下、记录、报告，不换参数重试）

1. **离线**：节点测试用合成 git 仓库覆盖 `changes.mjs`（无标签、非祖先、脏工作树、空清单、正常清单、非约定式标题、换点后丢掉的集合相等／多出／缺少）；`NotesRules.swift` 单独编译成测试可执行文件，覆盖构建号、语言、规范化、长度边界（4000／4001）、控制字符、请求文件权限、接口白名单（允许与拒绝的方法路径组合、`DELETE` 只认自己创建的 ID）；`helper.py notes --dry-run` 的校验。Swift 与 Python 两套校验读同一份共享用例文件，结论必须一致。helper 整体用 `swiftc` 通过类型检查。
2. **换 helper（需用户批准本计划）**：先留一份换前的 `status` 输出；`helper.py install`（自动备份旧 helper）；再跑 `status`，与换前结构一致（构建列表、内部组不变）；再用 build 6 的归档（保留到此步完成）跑 `upload.py <归档> --check-signing`（本地签名、**不上传**）。授权失效或任何不符：回滚到备份、报告。
3. **只读实测**：`notes --mode read --build 6`，预期没有任何语言有文本；若发现已有文本，原样报告，`probe` 只对没有文本的语言做。
4. **写入实测**（本计划已含用户对此的选择）：`notes --mode probe --build 6`，预期每种语言依次 `created`→读回一致→`DELETE`→确认不存在；随后再 `read`，确认 build 6 没有残留。把结果报告给用户。**不对 build 6 写任何真实文字。**
5. **标签**：创建 §6 的四个标签；运行 `changes.mjs`，基线应为 `ios-testflight-6`，清单是自 `9f00abcb` 之后 `ios/` 下的提交（若没有就是空）。
6. **首次真用在 build 7**：用户下一次要求出包时，完整走 §5；这是对整条链的最终验证，结果写进该次出包记录与 `docs/PROGRESS.md`。

## 8. 提交与文档

- 分支 `chore/ios-testflight-notes`；提交类型用 `docs`／`chore`（scope `skills`），都不进面向用户的发行说明：
  1. `docs: add spec and plan for ios testflight build notes`；
  2. `chore(skills): add the ios-testflight changes script and its tests`；
  3. `chore(skills): add the notes operation to the testflight helper`；
  4. `docs(skills): list changes and write testflight notes in the ios-testflight flow`（SKILL.md、reference、`docs/plans/ios-channel.md` 实施记录、`docs/PROGRESS.md`）。
- `docs/PROGRESS.md` 新增一节「待验收：iOS 出包清单与测试内容」，只记未完成的：build 7 首次真用。验证全部通过、build 7 走完后删除该节。
- 完成后核对：是否有 overview 或其他 spec 因此失真。`docs/overview.md` 不描述出包流程，预计无需改动。

## 9. 风险与对策

| 风险 | 对策 |
| --- | --- |
| 换 helper 后 Keychain 授权失效 | 稳定签名保证已存条目可继续使用（2026-10-30 已验证重编译后可静默使用）；换后立刻 `status` 与 `--check-signing`；保留 `testflight-helper.previous` 以便回滚；失效则停下交给用户恢复 |
| App Store Connect 对 `zh-Hans`／`filter[build]`／换行与项目符号的实际行为与文档不一致 | `read` 与 `probe`（标记文本含换行、`•` 与中文）在 build 6 上先暴露，build 7 之前就知道；不符就停下报告，不换参数重试 |
| API 密钥的角色不够写测试内容（`probe` 返回 403） | skill 的边界禁止自行提升 API 权限；停下报告，由用户决定是否调整，计划其余部分（清单、确认门、标签）仍可单独使用 |
| 测试内容里写进不该公开的信息 | 起草规则里明确禁止；确认门让用户看到全文 |
| 标签打错提交 | 归档前后各校验 HEAD 与工作树；变了就停下问用户；标签用附注并写明 Apple 构建 ID |
| 换点出包的临时 worktree 占盘 | 出包前沿用机器健康检查；用完立即删除临时 worktree |
| 误以为「写了测试内容」＝「用户已同意发布」 | 上传授权仍来自用户对具体出包的请求；测试内容只是该次授权的一部分，且已在出包前经用户确认 |

## 10. 实施记录（2026-11-04）

与计划的差异：

1. 提交：计划的第 2、3 个提交合成一个。`changes.mjs` 与 `notes` 共用同一个测试文件（`scripts/*.node-test.mjs` 是项目的测试入口），拆开会让第一个提交的测试不是全绿。
2. 空白规则细化为只认 U+0020 与 U+000A，检查顺序固定为「控制字符、是否为空、长度」（见 §3），避免 Swift 与 Python 各自的「空白」定义不同。
3. 闸门比计划更窄：`PATCH` 只允许改「本次为目标构建列出的」或「本次创建的」条目，并且只在 `write` 模式。
4. 真实运行发现一个启动器的错误：`read` 与 `probe` 发出的请求里带了空的 `texts`，helper 按设计拒绝（发生在任何网络请求之前，没有副作用）。启动器改为只在 `write` 时带 `texts`（`notes_wire`），并增加「启动器写出的请求必须能被 helper 的校验原样接受」的往返测试；故意把它改坏后，该测试会变红。

测试：`scripts/ios-testflight-changes.node-test.mjs` 共 19 条（合成 git 仓库测 `changes.mjs`；Python 与 Swift 两边读同一份 `notes-rules-cases.json`；`NotesRulesTests.swift` 128 项检查；helper 全部源码的类型检查）。另做了 12 项变异检查（故意改坏长度上限、闸门、空白规则、合并提交处理、基线排序、脏树检查、「被丢掉的集合」校验等），测试全部变红。

在线验证（均按计划顺序，逐步进行）：

- 换 helper 前先留一份 `status` 输出；`helper.py install`（新 helper 305 KB；换掉的旧 helper 保存为 `testflight-helper.previous`，与换前字节一致）；换后的 `status` 与换前逐字相同，Keychain 静默可用，没有弹授权。
- `upload.py ios/build/AskHuman-TestFlight.xcarchive --check-signing`：`EXPORT SUCCEEDED`（只在本地签名，没有上传；没有临时密钥残留）。
- build 6 `notes --mode read`：没有任何语言有文本。Apple 接受了按 App 与构建号查构建、按构建过滤测试内容两种查询。
- build 6 `notes --mode probe`：`en-US` 与 `zh-Hans` 都 `probed` 且读回一致（标记文本两行共 48 个 UTF-16 单元，换行、`•`、中文都原样读回），全程 4.6 秒；随后再 `read`，build 6 没有残留。没有向 build 6 写过任何真实文字。
- 本地附注标签 `ios-testflight-3/4/5/6` 已建（仅本地，远端没有）。`changes.mjs --point main`：基线 `ios-testflight-6`，列出 3 个 iOS 提交（见 `docs/plans/ios-channel.md`）。

没有验证的：整条链的首次真用要等 build 7；多行中英文测试内容在手机 TestFlight 里的实际显示只验到 API 读回，没有在手机上看过（`probe` 的标记文本已被删除）。

发现：`changes.mjs` 在 main 上列出的 3 个提交里有「iPad 设备族」（`build(ios): ship the app to iPad as well as iPhone`，设备族改为 `1,2`）。`upload.py` 与 helper 的出包预检仍要求 `UIDeviceFamily == [1]`，所以 build 7 会被预检拦下。这是 skill 既有的 iPhone-only 约束，与本功能无关，这里没有改，记在 `docs/PROGRESS.md`，出 build 7 前先问用户。
后续：该约束已由 [ios-testflight-ipad-preflight](ios-testflight-ipad-preflight.md) 放宽为只认 iPhone＋iPad。

### build 7 首次真用（2026-11-04）

完整走了 §5 的七步：

- 清单：`changes.mjs` 基线 `ios-testflight-6` → `9f00abc`，出包点 `71e7fb5`（main 的合并提交，工作树干净），5 个 iOS 提交、27 个文件 +1692 −183（iPad 设备族与两栏布局 3 个、答题「下一题」2 个）；Apple 最大构建号 6＝最大标签号 6。
- 确认门：这一次走了（用户看过清单与两份文案后选「按清单出包」，文案未改），随后用户改了规则，见规格 T12；SKILL.md 的第 3 步、第 8 步、第 10 步已按此改写。
- 归档、预检、上传：`ARCHIVE SUCCEEDED`（`-jobs 3`、低优先级）；预检通过（`0.1.0 (7)`、设备族 1,2、仅内部）；`EXPORT SUCCEEDED`、`Upload succeeded`。约 2.5 分钟后 Apple 状态 `VALID`，进入 `AskHuman Internal`（组内共 6 个构建）。这是 Apple 第一次校验 iPad 构建：没有新的校验错误，也没有换过任何参数。
- 测试内容：中文 339、英文 988 个字符，两种语言都是 `created`，读回逐字一致（`helper.py notes … --mode write`，随后又 `--mode read` 比对一次）；私有临时文件已删除。
- 标签：`ios-testflight-7` → `71e7fb58`；事后 `changes.mjs` 以它为基线，清单为空，建议构建号 8。
- 没有验证的：多行中英文测试内容在手机 TestFlight 里的实际显示，等用户看。

起草文案时的取舍：「请重点看」第 2 条（iPad／折叠屏）不来自 `docs/PROGRESS.md`——大屏适配之前没有待验收项，出包时补了一节。
