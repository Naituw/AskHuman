# 需求：AskHuman 经 stdin 输入 Message（`--stdin`，规避 shell 引号转义）

> 状态：已实现
> 关联计划：`docs/plans/cli-stdin-input.md`
> 影响面：仅 CLI 入参（`cli/args.rs`、`cli/mod.rs`、`cli/help.rs`、i18n、README）。**不改** daemon 协议、stdout 结果契约、退出码语义、各渠道行为。

## 1. 背景

`AskHuman` 的 Message / 问题 / 选项当前**全部来自 argv 字符串**。当 AI Agent 生成调用命令、而 Message 是含**反引号、`$`、单/双引号**的长 Markdown 时，要把它塞进一个 shell 引号参数极易出错：

- 双引号包裹：内部 `` ` `` 反引号与 `$` 会被 shell 求值 / 破坏；
- 单引号包裹：内容里只要出现一个单引号就提前闭合。

这导致 Agent 频繁踩「引号转义冲突」。本需求新增一条**绕过 shell 引号**的 Message 入参通道：从 **stdin** 读取 Message，配合**带引号定界符的 heredoc**（`<<'EOF' … EOF`），正文里的反引号 / `$` / 引号全部按字面传入，零转义。

## 2. 目标

新增 `--stdin`：Message 文本改从标准输入读取（替代位置参数 `<Message>`）。问题与选项仍用 `-q` / `-o`。典型调用：

```bash
AskHuman --stdin -q "要继续吗？" -o "继续" -o "停止" <<'EOF'
# 标题

这里是含 `反引号`、$VAR、'单引号'、"双引号" 的长 Markdown，
多行也没问题，全部按字面传入。
EOF
```

只有一个问题时仍可省略 `-q`（与现有规则一致）：

```bash
AskHuman --stdin <<'EOF'
单个问题：要继续吗？（这段长文本即作为该问题正文）
EOF
```

## 3. 已确认决策

| 编号 | 决策项 | 结论 |
|---|---|---|
| D1 | 范围 | **仅** 为 Message 增加 stdin 入参通道；问题（`-q`）、选项（`-o`）、`-f`、`--no-markdown` **保持不变**（不引入 JSON / 自定义分隔格式 / 问题选项的 stdin 通道） |
| D2 | 参数形式 | 新增布尔标志 `--stdin`；出现时 Message 文本取自标准输入到 EOF |
| D3 | 语义等价 | `--stdin` 提供的内容**等价于位置参数 `<Message>`**：有 `-q` 时作为所有问题的共享 Message；无任何 `-q` 时按现有规则**提升为唯一问题**的正文 |
| D4 | 互斥 | `--stdin` 与位置参数 `<Message>` **不可同时给出**；同时给出 → stderr 报错 + 退出码 `1` |
| D5 | 位置自由 | `--stdin` 是具名标志，可出现在参数任意位置（不要求在 `-q` 之前），仅与位置参数 `<Message>` 互斥 |
| D6 | 读取与去尾换行 | 读取 stdin 全部内容；**去除结尾的一个换行**（`\n` 或 `\r\n`，即 heredoc 末尾的固有换行），其余（含前导/内部空白）原样保留 |
| D7 | TTY 兜底 | 给了 `--stdin` 但 stdin 是终端（无管道输入）→ 不阻塞等待，直接 stderr 报错 + 退出码 `1` |
| D8 | 空内容 | stdin 去尾换行后为空：若另有 `-q` 或 `-f` 则视为「Message 可选、留空」（与现状一致）；若既无 `-q` 也无 `-f` → 报「缺少内容」错误（复用现有校验）+ 退出码 `1` |
| D9 | 纯逻辑可测 | stdin 内容由 CLI（IO 层）读出后**作为参数传入** `parse_ask`，保持 `parse_ask` 为无副作用纯函数、可单测 |
| D10 | 帮助文案（重点约束） | `--agent-help` 中把 `--stdin` 描述为「Message 的输入替代」即可，**不**特别解释「无 `-q` 时提升为问题」的逻辑；示例采用 **Message + `-q`** 形式（沿用「只有一个问题可省略 `-q`」的既有说法，不为 stdin 单列特殊行为示例） |
| D11 | 文档同步 | 同步更新 `--agent-help`（含一条 heredoc 示例）与 `README` 使用示例；`prompts.rs` 保持不变（其刻意不内嵌用法、只指向 `--agent-help`） |

## 4. 约束与既有规则（不可破坏）

- **stdout 洁净**：stdout 仍只输出现有结果区块；`--stdin` 仅影响入参。
- **daemon 协议不变**：unix 下 CLI 读完入参才组 `TaskRequest`，stdin 读取在此之前完成；daemon / GUI Helper 契约零改动。非 unix 单进程路径同样在 CLI 读 stdin 后组 `AskRequest`。
- **既有入参语义不变**：`<Message>` 位置参数、`-q` / `-o` / `-f` / `--no-markdown`、`--help` / `--version` / `--settings` / `--history` / `daemon` 子命令、退出码（0/1/3）全部保持。
- 解析逻辑保持纯函数可单测（stdin 内容以参数注入）。

## 5. 验收标准

1. `AskHuman --stdin -q "Q" <<'EOF' …含反引号/$/引号的多行 Markdown… EOF` 能把正文**原样**作为共享 Message 送达（弹窗 / 各渠道），无转义破坏。
2. `AskHuman --stdin <<'EOF' …文本… EOF`（无 `-q`）把该文本作为唯一问题正文。
3. 同时给 `--stdin` 与位置参数 → stderr 报错、退出码 `1`。
4. 给 `--stdin` 但 stdin 为终端（无管道）→ stderr 报错、退出码 `1`，不卡住。
5. stdin 为空且无 `-q`/`-f` → 「缺少内容」错误、退出码 `1`；有 `-q` 时允许 Message 留空。
6. 结尾多余的一个换行被去除；内部 / 前导空白保留。
7. `--agent-help` 出现 `--stdin` 说明与一条 heredoc 示例（Message + `-q` 形式）；README 使用段含 `--stdin` heredoc 示例。
8. 既有用法（位置参数 Message、`-q`/`-o`/`-f`/`--no-markdown`）回归正常；daemon / 渠道 / stdout / 退出码不变。

## 6. 反馈意见

（后续 review 中产生的调整意见追加于此，标注日期。）
