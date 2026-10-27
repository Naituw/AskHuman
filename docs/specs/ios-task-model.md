# 需求：iOS 新建任务时可以选择模型与强度

> 状态：**已批准，开工中**（2026-10-27 起草；同日用户经 AskHuman 批准）。M1–M6 是用户先前选定的；M7–M13 是我据代码与只读探测提出、评审时一并批准的；
> M14–M19 是评审时用户对六条待确认点的结论（见「决策记录」），其中 Grok 清单、Cursor 清单还各留一条要在动手前再问用户（末尾「仍待确认」）。
> 实施顺序（同 `ios-follow-narrowing`）：先做 Mac 端并装机，App 之后再单独开工。**范围只含 iOS。**
>
> 痛点：手机上新建任务只能选「工作区 / Agent / 权限 / 待办 / 文字」，模型与强度一律用 Agent 自己的默认；想换个模型只能回 Mac。
>
> 用户原话（2026-10-27）：「我希望 iOS 版创建任务时可以选择模型，你分析一下」；探测前强调：「注意严禁触发计费」。
>
> 目标：新建任务时，除工作区 / Agent / 权限外，还能选**模型**，并在该模型有可选档位时再选**强度**；清单由 Mac 向各 Agent 动态获取（Claude 例外）；
> 不选就是「默认」，即**不给 Agent 传任何模型 / 强度参数**；Agent 详情页与新任务进度页显示模型。
>
> 关联计划：`docs/plans/ios-task-model.md`。落地后同步修订 `docs/specs/ios-channel.md`（§6.8 新建任务、决策表）、`docs/overview-ios.md`、
> `docs/wiki/ios-setup.md` 与 `.en.md`。

## 现状（供评审对照）

- **新建任务流水线**：手机 `task.options.get` → Mac 回 `task.options`（`workspaces[]`、`agents[]`、`permissionMode`）→ 手机 `task.create`
  （`clientMsgId`、`workspaceId`、`agentKind`、`permission`、`todoId?`、`text`）→ Mac `ios_tasks::create` 校验 → 共享的 `task_launch::start_task`
  （IM `/new` 也走它）→ Codex 桌面应用：`Operation::Create`；终端：一次性私有 `LaunchRecord`（5 分钟有效、带 `payload_sha256`）→ 打开终端 →
  helper `__agent-launch` 用**固定 argv** `exec` Agent（`agent_launch::agent_args`）。**整条链没有模型 / 强度的概念**。
- **线上现状**：`AgentOption { kind, label, ready, subtitle, permissionSelectable, reason? }`；`TaskCreate` 如上。iOS 线协议**不**对未知字段报错
  （无 `deny_unknown_fields`），增量可选字段不动 iOS 线版本、也不动 Mac IPC 的 `PROTOCOL_VERSION`（先例：`sessionTitle`）。
- **各 Agent 怎么指定模型 / 强度**（本机版本：Claude Code 2.1.226、Codex 0.163.0、cursor-agent 2026.10.01、Grok 1.0.70、Pi 0.84.2）：

| Agent | 选模型 | 选强度 | 本机能拿到的清单 |
|---|---|---|---|
| Claude Code | `--model <别名或全名>` | `--effort low / medium / high / xhigh / max` | **没有列表命令**（`--help` 只举了别名例子） |
| Codex（终端） | `--model <slug>` | `-c model_reasoning_effort="<档位>"` | `codex app-server` 的 `model/list`（M16；Codex 另有 `$CODEX_HOME/models_cache.json`，本期不读） |
| Codex（桌面应用） | `thread/start` 的 `model` | `config.model_reasoning_effort`（现有 `resolve_model_settings` 已经在解析这一对） | 同上（桌面应用内置的 `codex`）；桌面路径自己解析默认值时本来就会查 `model/list` |
| Cursor | `--model <id>` | **没有单独开关**：强度 / Fast / 上下文已折进模型 id（本机约 270 个） | `cursor-agent --list-models`（等价于 `models` 子命令） |
| Grok | `--model <id>` | `--reasoning-effort <档位>`（别名 `--effort`） | `grok models`（M15；Grok 另有 `~/.grok/models_cache.json`，本期不读）；输出格式未知，见探测记录 |
| Pi | `--model <provider/id>` | `--thinking off / minimal / low / medium / high / xhigh / max` | `pi --list-models`（表格，含 `thinking yes/no` 列） |

- **「实际在用的模型」**：`agents::session_info::resolve(kind, sessionId)` 已能从各家会话记录里读出 `model` 与 `reasoning`（只读头尾各 ≤ 1 MB）。
  现在只给 Mac 端 GUI 的悬浮卡用；每帧都读太重，需要按会话缓存。

## 探测记录（2026-10-27，只读，没有产生任何计费）

用户批准探测的同时强调「严禁触发计费」，所以只跑了**不起会话、不发提示词**的命令：

- **跑过**：`cursor-agent --list-models` 与 `cursor-agent models`（拉模型清单，约 1.7–2.3 秒，联网，输出完全相同）；Codex `app-server` 的 `model/list`
  （在**空的临时 `CODEX_HOME`、没有任何凭据**下，约 0.4 秒，只返回内置清单；临时目录与进程已清理）；`pi --list-models`（约 0.8 秒，纯本地）；
  读本机 Codex / Grok 的模型缓存（只看键名与结构）；各 Agent 的 `--help`；各 Agent 配置里的「默认模型」键名。
- **没有跑**：`grok models`（`grok models --help` 只说「List available models and exit」，并提供 `--leader-socket <PATH>`——默认会用 `~/.grok/leader.sock` 的本机 leader 进程，
  我无法保证它不触发模型调用）、任何 Agent 会话 / 提示词、带真实凭据的 Codex 调用。评审时用户选了用 `grok models` 做 Grok 清单（M15），
  所以它的输出格式**还没有样本**，第一次执行要再经用户批准（见「仍待确认」）。
- **得到的事实**：
  - Cursor 输出：标题行 `Available models`，随后每行 `<id> - <显示名>`，`auto - Auto (default)` 带 `(default)` 标记，显示名里混有零宽空格，
    个别带 `(NO ZDR)` 之类后缀，末尾一行 `Tip: …`。**强度与 Fast 已体现在 id 里**（如 `…-low`、`…-high-fast`），所以 Cursor 没有「强度」这个独立维度。
    `Tip` 还提到「参数化模型接受 `--model 'id[context=1m,effort=high,fast=false]'`」——本期**不用**这种写法（见已知局限 2）。
  - Pi 输出：表头 `provider  model  context  max-out  thinking  images`，之后是空格对齐的行；`thinking` 列为 `yes / no`，只有 `yes` 的模型有强度可选。
  - Codex `model/list`：`data[]` 里每项有 `id` / `model` / `displayName` / `hidden` / `isDefault` / `defaultReasoningEffort` /
    `supportedReasoningEfforts[]`；`nextCursor` 分页；参数 `includeHidden` / `limit` / `cursor`（现有创建路径就是这样调的）。**每个模型的可选强度不同**。
    （Codex 自己维护的 `models_cache.json` 里对应的是 `slug` / `display_name` / `visibility` / `default_reasoning_level` / `supported_reasoning_levels[].effort`，本期不读。）
  - Grok 缓存文件（本期**不读**）：`models.<id>.info` 里有 `id` / `name` / `hidden` / `supports_reasoning_effort` / `reasoning_efforts[]`；
    **同一个对象里还有 `api_key` / `env_key` 字段**——这也是改用 `grok models` 输出的理由之一：连这个文件都不碰。
  - 各 Agent 配置里的默认：Claude `settings.json` 的 `model`；Codex `config.toml` 顶层 `model` / `model_reasoning_effort`；
    Cursor `cli-config.json` 的 `model.displayName`（它同时带 `selectedModel.parameters`，即参数化的默认；**同一个文件里还有 `authInfo`**，只能按键名取）；
    Grok `config.toml` 的 `[models] default` / `default_reasoning_effort`；Pi `settings.json` 的 `defaultProvider` / `defaultModel`。
- 原始样本只留在本机的 Agent 私有存储里，**不进仓库**（仓库公开，样本里有内部网关 / 供应商名）；解析夹具用**合成数据**写成，并刻意带上
  零宽字符、`(default)`、`api_key` 这类脏点。

## 决策记录

用户选定（2026-10-27，AskHuman）：

- **M1 模型 + 强度一起做**：表单里多一行「强度」，按所选模型的可选档位显示（推荐过「只做模型」，用户选了一起做）。
- **M2 清单动态发现为主**：Codex / Cursor / Grok / Pi 问各自的 CLI 或读它们自己维护的本地缓存；Claude 用固定别名；拿不到就只剩「默认」。
- **M3 每次都从「默认」开始，不记住上次的选择**：手机上随手选一次贵模型，之后不会悄悄沿用。
- **M4 显示模型**：Agent 详情页显示该会话**实际**在用的模型（和强度）；新任务进度页显示**所选**的。
- **M5 先只做 iOS，管线放共享层**：模型 / 强度走 `TaskSpec` → `LaunchRecord` / `Operation::Create`，IM `/new` 与 Mac GUI 以后各加一步入口即可，本期不改。
- **M6 先把探测抓成夹具；严禁触发计费**：全部验证都不启动真实 Agent、不发提示词；确需真实 Agent 的一轮（可选）必须由用户**当次**批准。

我据代码与探测提出、待批准：

- **M7 「默认」= 不传任何开关**：选「默认」时 argv 与今天**逐字节相同**；只有手机明确选了才加开关。「默认」实际指向什么（比如 Codex 配置里的模型）
  只用于**显示**与**决定强度行有哪些档位**，从不替用户传出去。
- **M8 Cursor 不出强度行**：它的强度 / Fast 已折在 id 里，手机上就是在一长串模型里选（带搜索）。
- **M9 Mac 在 `task.create` 时重新校验**：`model` 必须出现在该 Agent 当前清单里，`effort` 必须是该模型（或「默认」所指模型）的档位；不满足则**不启动任何东西**，
  回 `failed`，原因 `modelUnavailable` / `effortUnavailable`。手机传来的 id 只是「选了清单里的哪一项」，绝不当作命令行片段。
- **M10 配置的强度与所选模型不兼容时，显式传该模型自己的默认强度**：只对声明了每个模型档位的 Agent——Codex 一定有，Grok 取决于 `grok models` 的输出（M15）。
  例：配置里 `model_reasoning_effort = max`，手机选了不支持 `max` 的模型、强度留「默认」——不这样做，Agent 会拿着不支持的档位启动。Claude / Pi 没有声明，不处理。
- **M11 不加 Mac 端开关 / 配置项**：清单为空或拿不到时行就不出现，不需要设置页入口。
- **M12 清单缓存**：清单来源（Cursor、Pi、Grok 的命令，Codex 的 `app-server`）进程内缓存 30 分钟，过期后「先用旧的、后台刷新」；daemon 重启后重新获取，不落盘。
  只用于「默认所指」的本地配置（Claude / Codex / Cursor / Grok / Pi 的设置文件）每次按文件修改时间判断是否重读，成本是一次 `stat`。
- **M13 Claude 固定别名 + 固定五档强度**：别名 `opus` / `sonnet` / `haiku`（`fable` 见 M14）；强度用 `--help` 里写明的五档，不按模型过滤（验证需要真实调用，做不到）。

评审结论（2026-10-27，用户经 AskHuman 批准整份需求与计划，并对我给出的六条待确认点作了选择）：

- **M14 Claude 别名**：`opus` / `sonnet` / `haiku` 固定；本机 `claude --help` 里出现 `fable` 字样时自动加入（采纳我的建议）。
- **M15 Grok 清单用 `grok models`**（**没有**采纳我的「读缓存文件」建议）：官方输出更干净，也不用碰带 `api_key` 的缓存文件。
  代价：它的输出格式目前没有样本，**第一次执行要再经用户批准**；我建议在「空的临时 HOME + 独立 `--leader-socket`、没有任何凭据」下跑，让它连模型都调不了，
  跑完杀掉可能拉起的 leader 并删掉临时目录。解析器与夹具等拿到样本后才写。运行时由 daemon 调用，缓存 30 分钟（M12）。
- **M16 Codex 清单一律走 `codex app-server` 的 `model/list`**（**没有**采纳我的「优先读缓存文件」建议）：`includeHidden: true`、翻页，与现有创建路径同一套握手；
  不读 `models_cache.json`。可执行文件跟随启动目标：桌面应用路径用应用内置的 `codex`，终端路径用 `codex`（就绪探测已解析的那个）。
  这意味着 daemon 在手机打开新任务时（缓存过期后）会起一个很短命的 `app-server` 进程；它只发 `initialize` 与 `model/list`，不建线程、不发 turn。
- **M17 Pi 强度只给 `low / medium / high` 三档**（采纳建议）。
- **M18 做 M10**（采纳建议）。
- **M19 真实 Agent 一轮：实现完做，但每次执行前都要用户批准；Cursor 永不实测**：用最便宜的模型 + 一句话任务，只覆盖 Claude / Codex / Grok / Pi。
  用户原话：「但是 cursor 严禁实测」——我按最严的理解执行：**不启动任何 Cursor 任务，本机验证里也不再调用 `cursor-agent` 的任何命令（含 `--list-models`）**，
  Cursor 的解析与流程只靠合成夹具与断言；唯一例外是 daemon 在你手机打开新任务时对 Cursor 清单的运行时获取（见「仍待确认」2）。

## 术语

| 术语 | 含义 |
|---|---|
| 清单 | Mac 对某个 Agent 能给出的模型列表（`models[]`）。 |
| 默认 | 不传模型 / 强度参数，由 Agent 自己的配置与默认值决定。 |
| 默认所指 | 「默认」实际对应的模型（`defaultModel`），由 Mac 尽力从 Agent 配置里解析，**只用于显示与强度档位**。 |
| 强度 | 推理强度 / 思考深度：Claude `--effort`、Codex `model_reasoning_effort`、Grok `--reasoning-effort`、Pi `--thinking`。 |

## 规则

### 手机上看到什么

新任务表单现有顺序是 Mac → 工作区 → Agent → 权限 → 待办 → 文字。在 Agent 之后、权限之前加两行：

- **「模型」行**（所选 Agent 有 `models` 才出现）：显示当前选择，初始为「默认」，副标题是 `defaultModel.label`，没有就写「Agent 自己的设置」。
  点开进选择页：第一行「默认」，其后按 Mac 给的顺序列出；超过 12 项出现搜索框（按名字与 id 过滤）；单选，选完返回。
- **「强度」行**（当前选择能确定一组档位才出现）：行内菜单，选项是「默认」加各档位；「默认」旁在已知时写出实际默认档位。
  能确定档位的情形：选了某个模型且它带 `efforts`；或模型为「默认」且 `defaultModel.efforts` 有值（Claude 恒有）。
- **复位**：切换 Agent → 两行都回「默认」；切换模型 → 当前强度不在新模型的档位里就回「默认」，在就保留。每次打开表单从「默认」开始（M3）。
- **旧 Mac / 没拿到清单**：两行都不出现，表单与今天一样；Face ID、幂等、进度页流程不变。
- **进度页**多一行「模型：…　强度：…」（所选，不等 Agent 回报）。

### 线协议（只加可选字段）

```jsonc
// task.options 里 agents[i] 新增两个可选字段
{
  "kind": "codex", "label": "Codex", "ready": true, "subtitle": "…", "permissionSelectable": true,
  "models": [                                   // 缺省 = 这台 Mac 对该 Agent 给不出清单，手机不显示「模型」行
    { "id": "model-a", "label": "Model A", "note": "…",                       // note 可选
      "efforts": ["low", "medium", "high"], "defaultEffort": "medium" },      // 可选：该模型的档位与它自己的默认
    { "id": "model-b", "label": "Model B" }
  ],
  "defaultModel": {                             // 可选：「默认」所指，尽力而为
    "label": "model-a", "efforts": ["low", "medium", "high"], "defaultEffort": "high"
  }
}

// task.create 新增两个可选字段（不选就不出现）
{ "clientMsgId": "…", "workspaceId": "…", "agentKind": "codex", "permission": "default",
  "model": "model-b", "effort": "high", "text": "…" }
```

- 上限：每个 Agent 至多 400 项；`id` ≤ 128 字符、`label` ≤ 80、`note` ≤ 120、`efforts` ≤ 8 档（超出的在 Mac 端截断）。
- 幂等：`task.create` 的内容哈希**仅在带了 `model` / `effort` 时**才把它们算进去，旧 App 发来的请求哈希不变；同一个 `clientMsgId` 换了模型再发会被当作冲突。
- `task.status.reason` 新增 `modelUnavailable`、`effortUnavailable`（App 需有对应文案）。
- `watch.frame` 的 `frame`（自由 JSON，两端宽松解码）新增可选字符串 `model`、`reasoning`，用于详情页。
- iOS 线版本、`ios/Fixtures/wire-v1/`（新增用例，Rust 与 Swift 测试共用）、中继、Mac IPC 的 `PROTOCOL_VERSION`：**都不变**。

### Mac：清单从哪来

| Agent | 来源 | 强度档位 | 「默认所指」 |
|---|---|---|---|
| Claude Code | 固定别名 `opus` / `sonnet` / `haiku`，本机 `claude --help` 提到 `fable` 时再加入（M14）；`~/.claude/settings.json` 的 `env` 里若配了 `ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL`（网关把别名映射到别的模型），副标题写映射结果 | 固定五档（M13） | `settings.json` 的 `model` |
| Codex | `codex app-server` 的 `model/list`（M16）：`includeHidden: true`、翻页；可见项进清单，隐藏项只留着查「默认所指」的档位 | 每个模型自己的 `supportedReasoningEfforts`，默认档位 `defaultReasoningEffort` | `config.toml` 顶层 `model`（在含隐藏项的完整清单里查它的档位）与 `model_reasoning_effort`；配置里没写模型时取 `isDefault` 那一项 |
| Cursor | `cursor-agent --list-models`（已解析的可执行文件 + 固定参数，走现有的登录 shell 探测通道，超时放宽到 8 秒）；去掉零宽字符与 `(default)` 标记 | 无（M8） | `cli-config.json` 的 `model.displayName`（只作副标题） |
| Grok | `grok models`（M15，同 Cursor 的通道；格式待首次获批执行后再定） | 待样本：若输出带每个模型的档位就用，否则只给 Agent 级的固定档位或不显示强度行（拿到样本后在计划里定） | `config.toml` 的 `[models] default` 与 `default_reasoning_effort` |
| Pi | `pi --list-models`（同 Cursor 的通道）；模型 id 取 `provider/model` | 仅 `thinking = yes` 的模型有：`low / medium / high`（M17） | `settings.json` 的 `defaultProvider` + `defaultModel` |

通用规则：

- **净化**：id 必须匹配 `^[A-Za-z0-9][A-Za-z0-9._/-]{0,127}$`（不会以 `-` 开头、不含空白与控制字符、不含 `:`——Pi 把 `:` 当强度简写），强度必须匹配 `^[a-z][a-z0-9_-]{0,15}$`；
  不合格的条目直接丢弃。显示名去零宽 / 控制字符、折叠空白。去重（同 id 取第一个）。
- **顺序**：保持来源给出的顺序（Codex 的 `model/list` 本身已按其优先级排序）。
- **失败一律降级**：命令失败 / 超时 / 解析不出任何条目 → 该 Agent 没有 `models`（手机上只有「默认」），不报错、不影响新任务其他部分。
- **不阻塞**：`task.options` 现有 15 秒预算内，各来源并行、单个来源最多等 4 秒（Cursor 首次约 2 秒）；超时的来源在后台继续，填进缓存供下一次使用。

### Mac：怎么启动

只对**新任务**（`LaunchMode::New`）；续聊 / 分叉不带模型。开关放在提示词之前（Codex 在 `--` 之前）：

| Agent | 模型 | 强度 |
|---|---|---|
| Claude Code | `--model <id>` | `--effort <档位>` |
| Codex（终端） | `--model <id>` | `-c model_reasoning_effort="<档位>"` |
| Codex（桌面应用） | 所选模型**覆盖** `resolve_model_settings` 里配置的模型 | 所选强度覆盖配置的强度；不被该模型支持就用该模型自己的默认（M10） |
| Cursor | `--model <id>` | — |
| Grok | `--model <id>` | `--reasoning-effort <档位>` |
| Pi | `--model <provider/id>` | `--thinking <档位>` |

- 手机只选强度、模型留「默认」：只加强度开关（`defaultModel.efforts` 里有这一档才允许，M9）。
- M10 的触发条件：强度留「默认」、模型是明确选的、该模型声明了档位、而 Agent 配置里的默认强度不在其中 → 显式传该模型的 `defaultReasoningEffort`。
- `LaunchRecord` 新增可选的 `model` / `effort`；`payload_sha256` **仅在带了它们时**才把它们算进去，所以旧记录、旧哈希照常校验；`validate_claim` 能发现被篡改的模型 / 强度。

### 校验与失败码

`ios_tasks::create` 在现有校验（开关、Agent 就绪、工作区、权限、待办、文字）之后：

1. `model` 有值：取该 Agent 当前清单（缓存没有或不含时同步刷新一次，最多等 6 秒；刷新失败就沿用旧缓存）；仍不含 → `modelUnavailable`。
2. `effort` 有值：必须在「所选模型的档位」或（模型为默认时）`defaultModel.efforts` 里；否则 `effortUnavailable`。
3. 通过后才进入 `start_task`；任何一步失败都**不开终端、不动待办**。

### 显示实际模型（M4）

- **详情页**：`watch_frame` 在帧里加可选 `model`、`reasoning`，来自 `session_info::resolve`，按会话缓存约 10 秒；会话刚起、记录里还没有模型时不显示。
  模型变化不单独触发新帧，随下一帧带出。对任何会话都有效（不限于手机发起的）。
- **进度页**：显示手机上选的（`TaskDraft` 记下所选 id 与显示名），不等 Agent 回报。

## 对 App 的要求

- 新增「模型」「强度」两行与模型选择页（含搜索）；复位规则见上；详情页一行只读的模型显示；进度页一行所选。
- 新文案（`Localizable.xcstrings` 经 `ios/Tools/make-catalog.py` 生成，中文缺失会构建失败）：行名、「默认」「Agent 自己的设置」、搜索提示、两个失败原因。
- 解码保持宽松：`models` / `defaultModel` / `model` / `reasoning` 全是可选；缺失 = 不显示。
- 不动：Face ID、幂等重发、进度页的阶段流程、`Wire.maxFollow` 等无关项。

## 兼容

| Mac | App | 行为 |
|---|---|---|
| 旧 | 新 | 没有 `models` → 不出现两行；`model` / `reasoning` 缺失 → 详情页不显示；与今天一样 |
| 新 | 旧 | 旧 App 忽略 `models`，不发 `model` / `effort` → 全部走「默认」 |
| 新 | 新 | 完整功能 |

任何一条路径都不会「悄悄用了一个和界面上显示的不同的模型」：要么手机明确选了并被 Mac 校验后传下去，要么一律「默认」。

## 安全与计费

1. **全程不启动真实 Agent、不发提示词**（M6、M19）：自动化验证用存根 Agent（一个临时的可执行文件，只用来通过「存在且可执行」的检查，**从不被执行**）、合成夹具和网关测试台；
   argv 靠断言 helper 组装出的 `Command` 来验证，而不是真的 `exec`。
   带真实 Agent 的一轮（可选）会产生模型费用，必须由用户**每次**批准，用最便宜的模型与一句话任务；**Cursor 永远不在其中**（M19）。
2. **清单来源只读**：只读本地配置文件，或用**已解析好的可执行文件 + 固定参数**调列表命令 / 起 `app-server` 只发 `model/list`（走 `agent_launch` 现有的登录 shell 探测通道：
   stdin 关闭、有超时；本期再给它加输出上限）；绝不把手机传来的内容拼进任何命令。
3. **这些本地文件里夹着密钥 / 账号信息**：Claude `settings.json` 的 `env` 里有 `ANTHROPIC_AUTH_TOKEN`；Cursor `cli-config.json` 有 `authInfo`；
   （Grok 与 Codex 的模型缓存文件分别带 `api_key` / `identity`，本期不读。）解析一律**按键名取需要的字段**（Claude 只取 `model`、`env.ANTHROPIC_MODEL`、
   `env.ANTHROPIC_DEFAULT_*_MODEL(_NAME)`；Cursor 只取 `model.displayName`），其余在反序列化时即丢弃；**不**遍历 `env`、**不**对整个对象做 `Debug` / 日志输出。
   夹具里的密钥是合成的，并有测试断言它们不会出现在任何输出里。
4. **Cursor**：我不会在本机再调用 `cursor-agent` 的任何命令（M19）；它的列表解析器只靠合成夹具。运行时（用户手机打开新任务、缓存过期）daemon 会调一次
   `cursor-agent --list-models`：只读、约 2 秒、不起会话——这是功能本身，不是我的测试；是否保留见「仍待确认」2。
5. **argv 注入防护**：手机传来的 `model` / `effort` 必须与 Mac 清单里的某一项**完全相等**才放行，且清单条目本身已按白名单正则净化（M9）；
   `LaunchRecord` 的哈希覆盖它们，helper 认领时再校验一次。
6. **日志**只记 Agent 种类、所选模型 id 与强度档位；不记任务正文、不记任何命令输出原文。

## 已知局限

1. **Claude 没有清单命令**：别名在你的账号 / 网关下是否存在、某模型是否支持所选强度，Mac 无法在不真实调用（计费）的前提下验证。固定别名表是 `opus` / `sonnet` / `haiku`，`fable` 只在本机 `--help` 提到它时才加入（M14）。
2. **Cursor 的强度 / Fast / 上下文折在模型 id 里**，手机上是一长串（带搜索，不分组）；本期不使用 `--model 'id[context=…,effort=…]'` 的参数化写法，
   所以 `cli-config.json` 里那种参数化默认也只用于显示。
3. **「默认所指」尽力而为**：配置读不到或格式变了，副标题退回「Agent 自己的设置」，强度行在无法确定档位时不出现（Claude 除外）。它不影响启动。
4. **清单是 Agent 当时给的**：个别条目可能因账号权限在启动后才被拒绝，此时 Agent 自己报错；手机上看不到价格 / 倍率，也不提示「这个更贵」。
5. **私有格式会变**：Cursor / Pi / Grok 的列表输出与 Codex `app-server` 的接口都不是公开契约；升级后解析不出就降级成只有「默认」，靠夹具测试发现并修。
6. **Codex 的桌面应用与终端两条路径用各自的 `codex` 可执行文件**（应用内置的与终端里的），版本可能不同，清单取启动目标对应的那个；`app-server` 是实验性接口。
7. **只覆盖「新建任务」**：续聊 / 分叉不带模型；IM `/new` 与 Mac GUI 的新任务窗口本期不支持。
8. 不记住上次选择（M3）；表单里没有「常用 / 最近」置顶。

## 验收

1. 打开新任务：有清单的 Agent 出现「模型」行；带档位的出现「强度」行；旧 Mac（无 `models`）两行都不出现。
2. 选了模型与强度 → helper 要 `exec` 的 argv **精确含**对应开关（断言组装出的 `Command`，不真的启动）；什么都不选 → argv 与改动前**逐字节相同**。
3. Codex 桌面应用路径把所选模型与强度写进新对话的 `thread/start` 参数；强度被所选模型不支持时落到该模型自己的默认。
4. 切换 Agent 复位；切换模型时不被新模型支持的强度复位。
5. 清单拿不到（命令失败 / 超时 / 缓存缺失）→ 只剩「默认」，不报错，新任务其他部分照常。
6. 伪造或过期的模型 / 强度 → `failed` + `modelUnavailable` / `effortUnavailable`，不开任何终端。
7. 详情页显示该会话实际的模型（和强度）；进度页显示所选。
8. 自动化覆盖：Rust 单测（解析夹具、校验、argv、哈希兼容、桌面覆盖）与网关测试；Swift 单测与真实 daemon 端到端（不起真实 Agent）；模拟器 UI 测试（en + zh-Hans）。
   **没有任何一步启动真实 Agent，也没有任何一步调用 `cursor-agent`。**
9. （可选，M19）真实一轮：Claude / Codex / Grok / Pi 各挑最便宜的模型、一句话任务，每次执行前经用户批准；核对终端里 Agent 的启动参数、详情页显示的模型与所选一致，
   再各试一次「默认」确认与以前相同。Cursor 不在其中。

## 后续（本期不做）

- IM `/new`、Mac GUI 新任务窗口也能选模型（管线已是共享层，只差入口）。
- Cursor 的参数化写法 → 把强度 / Fast / 上下文拆成独立选项。
- 记住上次选择；常用 / 最近置顶；价格 / 倍率提示。

## 仍待确认（动手到那一步之前再问用户）

评审时的六条待确认点已全部有结论（M14–M19）。因为那些结论，又多出两件要在动到对应代码前问用户：

1. **`grok models` 的第一次执行**（M15）：我要拿一份真实输出当解析夹具的依据。建议的做法：空的临时 `HOME`（没有 Grok 配置与凭据）+ 独立的 `--leader-socket`，
   超时 10 秒，跑完杀掉可能拉起的 leader、删临时目录——没有凭据就调不了模型。需要用户批准后才执行（P2 开头）。
2. **Cursor 清单的运行时获取**：手机打开新任务（缓存过期）时 daemon 会调一次 `cursor-agent --list-models`。用户说「cursor 严禁实测」，我把它理解为不要在测试里
   启动 Cursor；这个调用是功能本身（M2 的动态发现），所以默认**保留**。如果用户连这个都不想要，Cursor 就只有「默认」，不出模型行（改动很小，P2 里让 Cursor 来源返回空即可）。
