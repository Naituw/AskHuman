我想优化 AskHuman 提问多个问题时的表现：
- 首先，统一单个和多个问题时的界面布局：
  - 取消按钮都在左下角
  - 添加图片按钮，放在补充内容输入框内部右下角一个小的图片图标
- 增加 message 和 question 的区分，例如：
  - AskHuman "Message" -q "Question1" -o "Option1" -q "Question2" -o "Option2"
    - 其中 Message 是所有问题的描述，question 才是实际的每个问题
    - 如果只有一个问题，可以省略 question，用 message 作为 question
    - -f 文件只能添加在 Message 上，不过不限制 -f 的位置，可以放在 -q 之后
    - 存在 -q 参数时，-o 不能出现在 -q 之前
  - 多个问题切换时，顶部都显示 Message，上一个下一个切换的是 Question + 选项 + 输入框
    - Question 的索引显示不再在标题上了，而是在 Message 下面，Question 上面，增加类似 Question 1/3
  - Telegram 通道先把 Message 发出，然后发送第一个问题，收到回答发送下一个

## 反馈意见

- 2026-06-04：`--agent-help` 文案需要随本次 Message/Question 模型变更一并更新（调用方式、参数说明、示例、多题输出说明等都要反映新模型）。
- 2026-06-04：澄清 message 与 -q 的关系——第一个位置参数始终是 **Message**；**完全没有 `-q` 时，第一个参数等价于 `-q`**（`AskHuman "X"` ≡ `AskHuman -q "X"`），即作为唯一问题，`-o` 归这个问题、`-f` 仍归 Message。因此内部模型中 **Message 只含 `text` + `files`（不持有 options）**，`questions` 恒 ≥ 1（无 `-q` 时由第一个参数“提升”而来）。
- 2026-06-04：单题且带附件（如 `AskHuman "X" -f f.png`，无 `-q`）——X 作为唯一问题，`f.png` 作为 Message 附件显示在问题上方（附件在上、问题在下）。**（2026-11-16 起，弹窗里这一形态改为题干当作 Message、排在附件上面且没有「Question」小标题，手机上同样题干在上、文件在下；用户的决定见 `docs/specs/message-annotations.md` D20 与 §11。IM 渠道不受影响。）**
- 2026-06-04：输入框内的「添加图片」小图标需加 tooltip / aria-label「添加图片」以提升可发现性。
- 2026-06-04：参数出错时（缺少提问内容 / 未知选项 / 解析失败）应显示 `--agent-help` 的内容，而非 `--help`；显式 `--help`/`-h` 仍显示完整帮助。
