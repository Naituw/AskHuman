# 开发计划：AskHuman 提问附带文件（`-f` 文件附件）

> 关联需求：`docs/specs/file-attachments.md`
> 计划描述方案与技术/规则细节，具体代码以实现为准。

## 0. 方案总览

```
AskHuman "问题" -f a.md -f b.png
  └─ CLI 解析 -f → 解析/校验路径（绝对/相对CWD/~ 展开；不存在→exit 1）
       └─ 构造 AskRequest{ ..., files: [FileAttachment{path,name,size,isImage}] }
            ├─ GUI 弹窗：提问下方渲染附件区
            │    · 非图片：扩展名图标 + 名称 + 大小（路径 tooltip）
            │    · 图片：缩略图（前端 invoke 读取 data URL，CSS 缩放）
            │    · 单击选中 / 双击·回车=打开 / 空格=预览(QuickLook)
            │      → invoke open_path / preview_path（Rust 调系统命令）
            └─ Telegram（若启用）：发完提问后逐个发送
                 · 图片→sendPhoto，其它→sendDocument（带文件名）
                 · 单个失败→stderr 警告 + 发一条含路径的失败消息
```

文件附件**只**用于展示，不写入 stdout、不参与抢答结果。

---

## 1. 数据模型（Rust + 前端类型对齐）

`src-tauri/src/models.rs`

- 新增结构 `FileAttachment`（serde camelCase）字段：
  - `path: String`（绝对路径）
  - `name: String`（文件名，用于显示与 Telegram 文件名）
  - `size: u64`（字节数，用于显示「大小」）
  - `is_image: bool`（按扩展名判定，前端据此决定图标 or 缩略图）
- `AskRequest` 增加 `files: Vec<FileAttachment>`（`#[serde(default)]` 容错）。
- `AskRequest::new(...)` 签名增加 `files` 参数。

`src/lib/types.ts`

- 新增 `FileAttachment { path: string; name: string; size: number; isImage: boolean }`。
- `AskRequest` 增加 `files: FileAttachment[]`。

> 说明：`ChannelResult` / `PopupSubmission` **不**增加文件字段（文件不回传、不进结果）。

## 2. CLI 解析与路径校验

`src-tauri/src/cli/args.rs`（纯逻辑，可单测）

- `AskArgs` 增加 `files: Vec<String>`（原始路径串，按出现顺序）。
- 解析新增分支 `"-f" | "--file"`：缺参数值时报中文错误（与 `-o` 一致）。
- 单测补充：单个/多个 `-f`、缺值报错、与 `-o`/`--no-markdown` 混用。

新增模块 `src-tauri/src/cli/file_attachment.rs`（承载「副作用」解析，便于 dispatch 调用，纯逻辑部分加单测）

- `resolve(raw_paths: &[String]) -> Result<Vec<FileAttachment>, String>`：
  - 逐个路径：
    1. `~` / `~/` 展开为家目录（`paths::home()`）。
    2. 相对路径 → 相对「当前工作目录」（`std::env::current_dir()`）解析为绝对路径。
    3. 校验存在且为文件；不存在/不可访问 → 返回中文错误（触发 exit 1）。
    4. 读取大小（`fs::metadata().len()`）、文件名（`file_name`）、`is_image`（扩展名小写匹配 D11 集合）。
  - 返回 `FileAttachment` 列表。
- 抽出可单测的纯函数：`expand_tilde(raw, home)`、`is_image_ext(name)`。

`src-tauri/src/cli/mod.rs`

- 提问分支：解析得到 `parsed.files` 后调用 `file_attachment::resolve`：
  - `Ok(files)` → `AskRequest::new(message, options, is_markdown, files)`。
  - `Err(e)` → `eprintln!("错误: {}", e)` + `exit(1)`（与现有错误风格一致）。

## 3. GUI 弹窗：附件区渲染与交互

`src/lib/ipc.ts` 新增三个命令封装：

- `openPath(path)` → `invoke("open_path", { path })`
- `previewPath(path)` → `invoke("preview_path", { path })`
- `readImageDataUrl(path)` → `invoke<string>("read_image_data_url", { path })`

`src/views/PopupView.vue`

- 从 `request.files` 渲染「附件」区，位置在提问正文（`markdown-body`/`plain-body`）之后、`options` 之前。
- 列表项结构（每个 `FileAttachment` 一项）：
  - 图片类（`isImage`）：左侧 `<img>` 缩略图；挂载后对每个图片 `readImageDataUrl(path)` 取 data URL 填充 `src`（与现有 `.thumb` 样式一致的缩略尺寸）。
  - 非图片：左侧通用「文档图标」（按扩展名可细分，至少有一个通用图标；实现可用内联 SVG，按扩展名映射少量常见类型）。
  - 右侧：文件名（`name`）+ 次要文字显示大小（人类可读，如 `12.3 KB`）。
  - 整项 `title=path`（hover tooltip 显示完整路径）。
- 选中态：本地 `selectedFileIndex: number | null`，单选；点击项设为选中并高亮（复用 `.option.selected` 风格的高亮 token）。
- 交互绑定（项元素设 `tabindex` 使其可聚焦/接收键盘）：
  - `@click` → 选中。
  - `@dblclick` → `openPath(path)`。
  - 键盘：当某项选中且聚焦时，`Enter` → 打开；`Space` → `previewPath(path)`（`preventDefault` 避免页面滚动）。
  - 注意：全局 `onKeydown` 已处理 `⌘/Ctrl+Enter` 发送与 `Esc` 取消；附件项的 `Enter`/`Space` 处理需在项级监听并 `stopPropagation`，避免与全局逻辑及 textarea 冲突。
- 大小格式化：前端小工具 `formatBytes(n)`（B/KB/MB）。

> 失败兜底：`openPath`/`previewPath`/`readImageDataUrl` 调用失败仅 `console` 记录，不阻断弹窗。

## 4. 后端命令：打开 / 预览 / 读取缩略图

`src-tauri/src/commands.rs` 新增 3 个 `#[tauri::command]`，并在 `app/mod.rs` 的 `invoke_handler!` 注册：

- `open_path(path: String) -> Result<(), String>`：按平台启动默认程序
  - macOS：`open <path>`
  - Windows：`cmd /C start "" <path>`（注意空标题参数）
  - Linux：`xdg-open <path>`
  - 用 `std::process::Command` spawn；失败返回中文错误。
- `preview_path(path: String) -> Result<(), String>`：
  - macOS：`qlmanage -p <path>`（其 stdout/stderr 重定向到 null，避免污染）
  - 其它平台：直接复用 `open_path` 的逻辑（D7 回退为打开）。
- `read_image_data_url(path: String) -> Result<String, String>`：
  - 读取文件字节 → base64 编码（已有 `base64` 依赖）→ 拼 `data:<mime>;base64,...`。
  - `mime` 按扩展名映射（与 `cli/image_writer.rs` 的扩展名↔media_type 思路一致，可复用/共置一个小工具）。
  - 仅供图片缩略图使用；大文件读入内存在 v1 可接受。

> capability：以上均为自定义 `invoke` 命令，使用 `core:default` 已允许的 IPC，无需修改 `capabilities/default.json`，也不引入新插件。

## 5. Telegram：发送文件

依赖：`src-tauri/Cargo.toml` 给 `reqwest` 增加 `multipart` feature（现有 `["json","rustls-tls"]` → 追加 `"multipart"`）。

`src-tauri/src/telegram/mod.rs` `TelegramClient` 新增：

- `send_document(path, filename) -> Result<i64, TelegramError>`
- `send_photo(path, filename) -> Result<i64, TelegramError>`
- 实现：构造 `reqwest::multipart::Form`，含 `chat_id` 文本字段 + 文件 part（从磁盘读入字节，设 `file_name`），POST 到 `{base}/bot{token}/{sendDocument|sendPhoto}`；按现有 `call` 的 `ok/result` 解析风格判定成功，返回 `message_id`。
- 错误沿用 `TelegramError`（网络/Api/BadResponse）。

`src-tauri/src/channels/telegram.rs` `run_session`：

- `request.files` 通过 `request` 已携带，无需改签名。
- 在「发送选项消息」「发送操作消息」之后、进入长轮询之前，遍历 `request.files`：
  - `is_image` → `send_photo`，否则 `send_document`，传 `name`。
  - 单个失败：`eprintln!("警告: 文件发送失败: <path>: <err>")`，并 `send_message`（纯文本）通知，如：`⚠️ 文件发送失败：<绝对路径>`（远端不可访问，仅告知）。
  - 失败不 return、不影响后续文件与长轮询。

> 注意：GUI+Telegram 抢答路径下，文件由 `TelegramChannel::start` → `run_session` 发送；仅 Telegram 的 headless 路径复用同一 `run_session`，两条路径自动一致。

## 6. 输出与帮助文档

- `cli/output.rs`：**不改**（无 `[文件]` 区块）。
- `cli/help.rs`：在「选项」段新增 `-f, --file <path>  附带文件（可多次），在弹窗中展示`；可在说明里点明「文件仅用于展示，不出现在结果输出」。
- `prompts.rs` `CLI_REFERENCE`：在调用方式与说明中补充 `-f` 用法及「文件仅展示、不进结果」。
- `README.md`：使用示例补充 `-f`；说明交互（双击打开/单击选中/空格预览）、平台差异（QuickLook 仅 macOS，其它回退打开）、Telegram 会把文件发过去。

## 7. 涉及文件清单

- `src-tauri/src/models.rs`：`FileAttachment` + `AskRequest.files` + `new` 签名。
- `src-tauri/src/cli/args.rs`：`-f/--file` 解析 + 单测。
- `src-tauri/src/cli/file_attachment.rs`（新增）：路径解析/校验、`~` 展开、`is_image` + 单测。
- `src-tauri/src/cli/mod.rs`：接 `file_attachment::resolve`，错误退出 1。
- `src-tauri/src/commands.rs`：`open_path` / `preview_path` / `read_image_data_url`。
- `src-tauri/src/app/mod.rs`：`invoke_handler!` 注册新命令。
- `src-tauri/src/telegram/mod.rs`：`send_photo` / `send_document`。
- `src-tauri/src/channels/telegram.rs`：发完提问后发送文件 + 失败提示。
- `src-tauri/Cargo.toml`：`reqwest` 加 `multipart`。
- `src-tauri/src/cli/help.rs`、`src-tauri/src/prompts.rs`、`README.md`：文档。
- 前端：`src/lib/types.ts`、`src/lib/ipc.ts`、`src/views/PopupView.vue`。

## 8. 任务顺序

1. 数据模型（Rust `FileAttachment`/`AskRequest`，TS 类型）。
2. CLI：`args.rs` 的 `-f` 解析 + `file_attachment.rs` 路径解析/校验（含单测）+ `cli/mod.rs` 接线（exit 1）。
3. 后端命令：`open_path`/`preview_path`/`read_image_data_url` + 注册。
4. 前端：`PopupView.vue` 附件区渲染（图标/缩略图/大小/tooltip）+ 选中/双击/回车/空格交互 + `ipc.ts`。
5. Telegram：`reqwest` multipart + `send_photo`/`send_document` + `run_session` 发送与失败提示。
6. 文档：`help.rs` / `prompts.rs` / `README`。
7. 构建（`--features custom-protocol`）+ 安装 + 弹窗实测（含 macOS 预览、Telegram 实测如配置可用）。

## 9. 测试策略

- Rust 单测：`args.rs` 的 `-f` 解析；`file_attachment.rs` 的 `expand_tilde` / `is_image_ext`。
- 手动/端到端：
  - `-f` 单个/多个、图片缩略图、非图片图标+大小、tooltip 路径。
  - 双击打开、回车打开、空格预览（macOS QuickLook）/其它平台回退打开。
  - `-f` 指向不存在文件 → exit 1、不弹窗。
  - 文件不出现在 stdout；回答区图片/选项/文字回归正常。
  - Telegram（如配置）：图片 sendPhoto、其它 sendDocument、失败提示消息。

## 10. 风险与注意

- **键盘冲突**：附件项的 `Space`/`Enter` 必须与全局 `Esc`/`⌘Enter` 及 textarea 输入隔离（项级监听 + `stopPropagation` + 失焦时不拦截 Space）。
- **`qlmanage` 可用性**：属 macOS 自带；若异常仅预览失败，不影响其它功能。
- **大图内存**：缩略图读全量字节为 data URL，超大图占内存；v1 接受，后续可加后端缩放或大小上限。
- **Telegram 文件大小限制**：Bot API 上传有大小上限（约 50MB）；超限由 Api 错误走失败提示路径，不崩溃。
