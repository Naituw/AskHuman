# Diff 附件预览

> 本文记录共享 diff 格式与 macOS Quick Look 渲染。Popup 同窗面板与原文切换见
> [popup-attachment-preview-panel.md](popup-attachment-preview-panel.md)，其 diff 格式沿用本文。

## 已确认范围

- 扩展名 `.diff` / `.patch` 大小写不敏感；Popup 使用同窗面板，macOS 其他入口使用原生 Quick Look。
- 单栏 unified diff；显示新增绿色底、删除红色底、原始 `+` / `-` 前缀、旧 / 新行号、文件头与变更块。
- `attachment_diff::parse` 返回同一份文件段与行模型；Quick Look 生成静态 HTML，Popup 通过 Vue 文本插值虚拟显示行，保留原文切换和每附件阅读状态。
- Popup 的激活、键盘和跨平台文件操作由关联面板规格定义；其他入口保留原生预览流程。
- 不增加代码语法高亮或双栏对照；普通文本识别属于 Popup 面板，不属于 diff 解析器。

## 格式和显示

支持 Git unified diff、普通带 `---` / `+++` 文件头的 unified diff，以及 `git format-patch` 中的 unified diff；邮件头、提交说明、重命名、文件模式、二进制段与其他元数据保留原文。文件路径保留 Git 原始引号 / 转义表示，不访问路径所指向的文件，也不应用 patch。

解析器按 `@@ -old,count +new,count @@` 检查完整变更块，再展示行号；省略 count 时为 1，零长度范围用于新增 / 删除文件。上下文递增两侧，删除只递增旧侧，新增只递增新侧；缺少末尾换行标记不占行号。变更块内部以 `---` / `+++` 开头的代码按增删内容处理。各变更块独立重置行号。

不完整或无法解析的变更块、combined diff、无文件头的片段保持原文，不推测行号；出现不能解析的变更块或整份附件没有支持的变更块时显示说明。此预览不是 patch 合法性校验器，未识别的其他内容也仍完整显示。

使用等宽字体，保留缩进、空白、空行和显式无末尾换行标记，兼容 UTF-8 BOM / CRLF；行号不可选中，代码保留原始前缀。Quick Look 长行在文件块内横向滚动、文件头可换行，跟随系统深浅色；Popup 长行在独立正文横向滚动、文件头保持单行，跟随应用有效主题。行号 DTO 序列化为十进制字符串，避免 JavaScript 大整数精度丢失。

## 安全与资源边界

- 静态 HTML + 内联 CSS，文本和标题全量转义；CSP 禁止脚本、外部资源、表单与 base URL。
- 文件限 2 MiB、20,000 行、单行 16 KiB；读取最多 2 MiB + 1 字节。超限显示明确提示，不展示部分内容冒充完整变更。
- 非 UTF-8、NUL 二进制数据、读取失败分别展示提示；用户可从附件打开原文件。
- 临时 HTML 写入失败回退原始路径，沿用原生预览。
- 转换在后台执行，原生面板操作在主线程执行。每次请求持有代次，替换 / 关闭 / 原生面板结束控制会使旧结果失效；展示前检查原调用窗口仍存在。
- 临时 HTML 沿用 `temp/askhuman/preview/<uuid>/<原名>.html` 和现有临时目录清理机制。附件原文件不修改，默认打开、右键操作、拖出仍使用原始路径。

## 验证

Rust 测试覆盖范围解析、两侧行号、多个文件 / 变更块、增删文件、重命名、邮件头、二进制段、无末尾换行、BOM / CRLF、扩展名、转义和资源上限。验收样例为 `src-tauri/tests/fixtures/attachment-preview.patch`。

macOS 已用安装后的真实 Popup 验证 diff 高亮、两侧行号、原文与 Markdown 混合切换。Popup 面板的安装验收及 Windows / Linux 实机 gate 见关联实现计划；原生 Quick Look 转换与其他入口行为保留。
