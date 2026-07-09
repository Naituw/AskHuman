# 开发计划：AskHuman 经 stdin 输入 Message（`--stdin`）

> 关联需求：`docs/specs/cli-stdin-input.md`
> 计划描述方案与技术 / 规则细节，具体代码以实现为准。

## 0. 方案总览

```
AskHuman -q "Q" -o "A" -o "B" --stdin <<'EOF'
…长 Markdown（反引号/$/引号原样）…
EOF
  └─ cli/mod.rs 提问分支：预扫描 argv 是否含 --stdin
       ├─ 含：IsTerminal 兜底（终端→exit 1）→ 读 stdin 到 String → 去尾换行 → Some(content)
       └─ 不含：None
            └─ args::parse_ask(&argv[1..], lang, stdin_message)   （纯函数，stdin 内容以参数注入）
                 · --stdin token → 用注入内容作 message_text；与位置参数互斥
                 · 其余（-q/-o/-f/--no-markdown/位置参数）逻辑不变
                 · 归一化：无 -q 时 message 提升为唯一问题（现有规则）
                      └─ unix：组 TaskRequest 经 client→daemon→GUI Helper
                         非 unix：组 AskRequest 走单进程 run_ask
```

stdin 仅作为 **Message 文本来源**；不改 daemon 协议、stdout 契约、退出码。

---

## 1. CLI 解析（`src-tauri/src/cli/args.rs`，纯逻辑可单测）

- `parse_ask` 签名增加参数：`stdin_message: Option<String>`（已读好的 stdin 内容；`None` 表示未给 `--stdin`）。
- 新增 token 分支 `"--stdin"`：
  - 若已出现过位置参数 / 已出现过 `--stdin`（即 `seen_positional` 为真）→ 返回错误 `tr(lang, "cli.stdinWithPositional")`。
  - 取 `stdin_message`（约定调用方在传入 `--stdin` 时必为 `Some`；若为 `None` 视为空串兜底）作为 `message_text`，并置 `seen_positional = true`。
  - 不要求位置：`--stdin` 是具名标志，可出现在 `-q` 前后任意处（与位置参数的「必须在 -q 前」不同）。
- 位置参数分支不变：若此前已 `seen_positional`（含 `--stdin` 已设）或已 `seen_question_flag` → 报「仅 Message 可作位置参数 / 重复」错误，从而实现 D4 互斥（位置参数 + `--stdin` 二者只能其一）。
- 归一化不变：无 `-q` 时把 `message_text`（可能来自 stdin）提升为唯一问题；有效性校验（至少有 message / `-q` / `-f`）不变 → 覆盖 D8 空内容。
- 现有所有调用点改为传第三参（无 stdin 时传 `None`）。

## 2. stdin 读取与分发（`src-tauri/src/cli/mod.rs`，IO 层）

- `dispatch()` 的「提问」分支（`_ =>`）在调用 `parse_ask` 前：
  - 预扫描 `argv[1..]` 是否含 `--stdin`。
  - 含 → 调用新私有函数 `read_stdin_message(lang) -> String`，把结果包成 `Some` 传入 `parse_ask`；不含 → 传 `None`。
- `read_stdin_message(lang)`：
  - 用 `std::io::IsTerminal`（`std::io::stdin().is_terminal()`）判断：若是终端（无管道）→ `eprintln!` 报 `tr(lang, "cli.stdinIsTty")` + `exit(1)`（D7，避免阻塞）。
  - 否则 `std::io::Read::read_to_string` 读全部；失败 → 报错 + `exit(1)`。
  - 去尾换行：依次去掉一个结尾 `"\n"`，若随之结尾为 `"\r"` 再去掉（处理 `\r\n`）；其余原样保留（D6）。
- 关键：dispatch 顶部 `first if first.starts_with('-') && !matches!(first, "-q"|...|"--no-markdown")` 的「未知前导选项」分支需把 **`--stdin` 加入 allowlist**，否则以 `--stdin` 打头的调用会被判为未知选项而非进入提问分支。
- 提问分支两条目标路径均沿用现状：
  - unix：`crate::ipc::TaskRequest { message, questions, … }` → `client::run_ask`。
  - 非 unix：`AskRequest::new(...)` → `app::run_ask`。
  - 二者拿到的 `parsed` 已含来自 stdin 的 Message，无需各自再处理。

## 3. i18n 文案（`src-tauri/src/i18n.rs`，zh + en）

新增词条（英文为源语言）：

- `cli.stdinWithPositional`：EN「cannot combine --stdin with a positional <Message>」/ ZH「--stdin 不能与位置参数 <Message> 同时使用」。
- `cli.stdinIsTty`：EN「--stdin was given but stdin is a terminal (no piped input)」/ ZH「指定了 --stdin，但 stdin 是终端（没有管道输入）」。

空内容复用现有 `cli.missingContent`。

## 4. 帮助与文档（D10 / D11）

`src-tauri/src/cli/help.rs` 的 `agent_help_text`（zh + en）：

- 在「参数说明 / Arguments」新增一行（紧随 `<Message>`）：
  - EN：`  --stdin               Read the <Message> from stdin (use a quoted heredoc to avoid all shell quoting)`
  - ZH：`  --stdin               从标准输入读取 <Message>（用带引号的 heredoc 规避所有 shell 转义）`
- 「使用示例 / Examples」新增一条 heredoc 示例（**`-q`/`-o` 先、`--stdin` 垫底**，不为「无 -q 提升」单列示例）：
  - EN：
    ```
    {prog} -q "Continue?" -o "Continue" -o "Stop" --stdin <<'EOF'
    # A long Markdown message with `backticks`, $vars and "quotes"
    EOF
    ```
  - ZH：同形中文示例。
- **不**修改 Invocation 主行的最小语法（保持简洁）；`--stdin` 仅在 Arguments + 一条 Example 体现。

`README.md` 使用段：在 `-f` 示例附近补一条 `--stdin` heredoc 示例 + 一句说明（「长 Markdown 含反引号/引号时，用 `--stdin` + heredoc 规避转义」）。是否同步 `README.en.md` 由 review 决定（默认同步，与现有双语一致）。

`src-tauri/src/prompts.rs`：**不改**（其刻意只指向 `--agent-help`）。

## 5. 测试（`cli/args.rs` 单测）

`parse_ask` 第三参为 `Option<String>`，便于纯逻辑单测：

- `--stdin` 提供 Message + `-q`：message 作共享 Message，questions 来自 `-q`。
- `--stdin` 无 `-q`：内容提升为唯一问题（message_text 清空）。
- `--stdin` + 位置参数 → Err（D4）。
- `--stdin` 内容为空 + 无 `-q`/`-f` → Err（缺内容）；`--stdin` 空 + 有 `-q` → Ok（Message 留空）。
- `--stdin` 出现在 `-q` 之后仍可用（D5）。
- 既有用例：所有现有 `pa(...)` 调用补第三参 `None`，断言不变。

> TTY 兜底与「去尾换行」属 IO 层（`cli/mod.rs`），以手动 / 端到端验证为主（heredoc 实际调用）。

## 6. 涉及文件清单

- `src-tauri/src/cli/args.rs`：`parse_ask` 加 `stdin_message` 参 + `--stdin` 分支 + 单测。
- `src-tauri/src/cli/mod.rs`：提问分支预扫描 + `read_stdin_message` + allowlist 加 `--stdin` + 两路径传参。
- `src-tauri/src/i18n.rs`：两条新词条（zh+en）。
- `src-tauri/src/cli/help.rs`：`agent_help_text` 加 `--stdin` 说明 + heredoc 示例（zh+en）。
- `README.md`（及 `README.en.md`）：使用示例补 `--stdin` heredoc。

## 7. 任务顺序

1. `args.rs`：`parse_ask` 加参 + `--stdin` 分支 + 补全所有调用点的第三参 + 单测。
2. `cli/mod.rs`：`read_stdin_message`（IsTerminal 兜底 + 去尾换行）+ 预扫描分发 + allowlist 加 `--stdin`。
3. `i18n.rs`：两条词条。
4. `help.rs` + README：文档与示例。
5. `./scripts/install.sh` 编译安装；用 heredoc 实测（含反引号/$/引号多行、无 -q、互斥报错、TTY 报错）。

## 8. 风险与注意

- **dispatch allowlist**：忘记把 `--stdin` 加入「未知前导选项」allowlist，会让 `AskHuman --stdin …` 误判为未知选项；务必在第 2 步处理。
- **阻塞读 stdin**：必须先 `IsTerminal` 判断再读，避免无管道时永久阻塞。
- **去尾换行边界**：仅去结尾一个换行（含 `\r\n`），不要 `trim()` 掉前导 / 内部空白，以免破坏 Markdown 缩进 / 代码块。
- **纯函数边界**：stdin 的 IO 只在 `cli/mod.rs`；`parse_ask` 仅接收已读内容，保持可单测。
