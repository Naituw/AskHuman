# Markdown 浏览器打开：实现与验证

2026-10-05；行为决策见 [规格](../specs/markdown-browser-open.md)。已实现，macOS 本地安装和实际交互验证通过。

## 实现

- `useAttachments.ts` 根据已加载的 Markdown 内容及阅读模式选择主动作；原文、列表回车保持原文件打开。双击于 2026-10-07 改为一次预览切换，不打开外部应用；下方 2026-10-05 的双击验收记录为当时行为。主按钮固定 112 px，按钮与标题栏拖拽区域独立。
- 更多与右键菜单复用原生菜单，Markdown 提供固定的浏览器打开 / 打开原文件；Open With、复制和文件管理器定位继续针对原文件。非 Popup 入口不增加浏览器动作。
- `popup_preview_open_browser` 只接收请求 ID / 附件索引，读取前及真正启动前校验活跃提问；后台生成最多两个并发快照，前端同请求阻止重复点击并丢弃过期结果。
- `attachment_browser.rs` 每次复用 `attachment_preview::load` 重新有界读取，沿用字节、行数、Markdown 事件和 HTML 输出预算。生成独立文档，转义原始 HTML，保持受限链接 / 图片策略，使用 CSP 阻止脚本、对象、表单和 base URL。
- `attachment_html.rs` 提取已有 Quick Look 样式与临时文件写入。快照位于既有 `askhuman/preview/<uuid>/<stem>.html`，沿用 daemon 的 24h 清理。同名附件和连续点击不覆盖旧快照。
- 失败提示保留明确的原文件按钮；当前附件失败显示在预览内，其他附件或预览关闭时显示在附件列表下。失败不修改当前阅读模式、激活、几何或回答草稿，也不静默打开原文件。

## 默认浏览器适配

所有平台解析 HTTPS 的用户默认关联，不查询 HTML 文件关联，不硬编码 Safari。

- macOS 主线程用 [NSWorkspace 的默认应用查询](https://developer.apple.com/documentation/appkit/nsworkspace/urlforapplication%28toopen%3A%29-7qkzf) 获取浏览器，再显式指定应用打开本地文件；启动返回 false 时报告失败。
- Windows 用 [AssocQueryStringW](https://learn.microsoft.com/en-us/windows/win32/api/shlwapi/nf-shlwapi-assocquerystringw) 和 [ASSOCF_IS_PROTOCOL](https://learn.microsoft.com/en-us/windows/win32/shell/assocf_str) 查询用户 HTTPS 关联的可执行文件，通过独立参数传递 file URI，不解释注册表命令字符串。
- Linux 复用 Tauri 已有的 GIO 0.18 运行时，按 [默认 URI scheme 应用](https://docs.gtk.org/gio/type_func.AppInfo.get_default_for_uri_scheme.html) 查询并调用 [launch_uris](https://docs.gtk.org/gio/method.AppInfo.launch_uris.html)。Cargo.lock 仅增加本包对已有 GIO 的直接依赖，没有新增 crate。

## 自动验证

- 完整 Rust tests：1231 passed，3 ignored；Clippy all-targets / `-D warnings` 通过。
- 前端：213 Vitest / 36 文件，5 Node tests；类型检查及 production build 通过。
- `./scripts/install.sh` 编译、安装并签名通过。最终安装二进制为 42,585,456 bytes。
- 新 Rust 回归覆盖新内容读取、源文件字节不变、Unicode / 空格 / 标题转义、同名独立快照和资源 / 编码 / 缺失文件限制。
- 新前端回归覆盖模式路由、双击 / Enter 的原文件动作、固定菜单行为、请求归属、重复点击、资源及启动失败、失败不改变激活，以及过期结果丢弃。
- Windows 浏览器适配函数已在独立最小工程以 `x86_64-pc-windows-msvc` 类型检查通过；这不代表完整 Windows 构建或桌面运行通过。Linux API 签名与实际 GIO 0.18.4 源码核对，未作 Linux 编译或实机运行声明。

## macOS 安装后实机验证

使用独立 QA app / ASKHUMAN_HOME / IPC 服务，未改生产配置及系统默认应用。

- 默认浏览器 Safari 实际打开带中文、空格和大写 `.MD` 文件名的渲染快照；表格、代码块、任务列表及系统深色阅读样式正常，原始 script 标记显示为文本。
- 原文模式切换主按钮为 Open；普通 `.md` 原文件在 MWeb Pro 中显示。双击及附件列表 Enter 同样打开原文件，没有推进题目或提交草稿。
- 原文模式的菜单浏览器入口仍渲染；右键 / 更多都有固定的 Open in browser、Open original file、原文件 Open With。
- 切附件和收起重开保持该附件模式；新测试请求默认渲染。标题切换截图中计数、导航、更多和关闭位置相同。
- 原文在 Popup 缓存后改为 gamma，重新点击浏览器入口实际显示 gamma；原来的 alpha 快照仍独立存在，浏览器操作前后源文件 SHA-256 相同。同名另一附件独立显示 beta。
- 超过 2 MiB 的 Markdown 拒绝生成；文件移走后重新点击报告读取失败并保留原文件入口。预览关闭及其他附件的菜单失败都可见，不激活失败附件、不替换正在阅读的内容。
- Safari 原生 Find 查询 gamma 返回 1 match。两轮 Popup 的回答草稿保持原值。
- 测试 Popup / IPC 服务与生成的 Safari 标签已关闭；仅关闭 MWeb 的测试文档，保留原有文档和浏览器标签。

原文件大写 `.MD` 路径已正确交给系统默认应用，MWeb 未为该扩展创建文档；默认应用的内容验收使用普通 `.md`。浏览器的大写扩展识别已经实测。

## 剩余实机边界

Windows / Linux 的完整构建、默认浏览器选择、原生菜单和启动失败仍沿用 Popup 已确认的外部 gate。
本机 HTML 与 HTTPS 的默认应用均为 Safari，未通过改动用户系统关联制造两者不同；该场景的实际平台矩阵仍待补验，代码明确使用 HTTPS 关联和指定应用启动。

本地证据位于 `/tmp/askhuman-popup-qa/browser-*`，包括 install、Rust、Clippy、前端及 Windows API check 日志、源文件哈希、原文 / 渲染 / Safari / 菜单 / 失败提示截图。
