# Base64 用途审计与图片协议兼容性

2026-10-09。用户要求审计全项目的 Base64 用法，并以官方资料核查 macOS 以外兼容性。
范围为跟踪的源码、脚本、工作流和 demo；依赖、生成产物与测试 fixture 不计入生产用途。
核对关键词：`base64`、`B64`、`data_url` / `dataUrl`、`readAsDataURL`、`toDataURL`、
`btoa` / `atob`、`data:image` 及 Buffer / digest 编码接口。没有读取签名证书或渠道密钥内容。

## 结论

不能把所有 Base64 换成图片 URL。纯显示用途可以优化；内部图片运输可减少重复编码，但需要
保持提交、CLI、Daemon 与 IM 的数据契约；协议指定的编码及离线文档的自包含约束应保留。

正文本地图片 URL、懒加载和超限原图入口已实现；2026-10-10 安装后用户确认正常图片、
超限提示和点击打开原图符合预期并保留。随后用户确认继续优化历史、待办草稿与 Interject 的
纯显示图片，已复用同一协议并补齐单资源释放，见 [实施记录](../plans/display-image-urls.md)。
其他项仍为审计建议。

## 完整用途清单

| 用途 / 实际入口 | 当前路径与开销 | 优化判断 |
| --- | --- | --- |
| Markdown 正文本地图片；`MarkdownContent.vue` | 最初调用 `read_image_data_url`，全文件 → Base64 → IPC JSON → img | 本轮改为注册 token URL、原文件二进制响应、按可见区域加载；可移除 Base64 |
| 历史附件 / 回复图片；`HistoryDetail.vue` → `localImageUrls.ts` | 已改有界 token URL；可见区域加载，记录切换 / 卸载撤销资源 | 2026-10-10 已实现；原图打开和 Quick Look 保留，原生拖拽用系统 / 应用小 PNG 图标 |
| 待办新附件预览；`TodosView.vue` → `localImageUrls.ts` | 已改有界 token URL，移除 / 清空 / 项目切换释放；提交路径保持 | 2026-10-10 已实现；已保存的 128px 缩略图与粘贴运输不变 |
| Interject 已选文件预览；`useInterjectAttachments.ts` → `localImageUrls.ts` | 已改有界 token URL，移除 / reset / 卸载释放；提交仍是 `filePaths` | 2026-10-10 已实现；不可读 / 超限 / 解码失败保留文件胶囊 |
| Popup 拖入回复图片；`usePopupCore.ts::addDroppedPaths` | 同一接口返回的 Base64 同时作为预览和 `ImageAttachment.data` 提交 | 不能只替换为 URL；要把显示与传输分开，可把本地路径保留到后端提交阶段 |
| Popup 选图 / 粘贴、Interject 粘贴、Todo 粘贴；`theme.ts::fileToDataUrl` | FileReader 读 Blob 为 data URL，既显示又经 JSON 提交；由 `image_writer.rs` 解码落盘 | 可先用 `URL.createObjectURL` 显示，移除 / 换题时 revoke；运输可采用 Tauri 二进制上传后返回文件引用，需要另行设计共同提交契约 |
| 同窗图片预览与列表缩略图；`attachment_preview.rs::image_content` / `png_content`、`useAttachmentContent.ts` | 普通图原文件编码；特殊格式转 PNG 后编码；20 MiB、40M 像素、动画 80M 像素等限制已存在，前端缓存计入字符串成本 | 第二优先级：普通图可用请求范围 token URL；转换后的 PNG 可用有界二进制资源或磁盘缓存。需保留 request ID、取消代次、缓存淘汰和原图动画语义 |
| Todo 保存的 128px PNG 缩略图；`todo_attachments.rs::read_thumbnail_data_url`、`commands.rs::todo_attachment_thumbnail` | 通常读取已有小 PNG；引用原图变更后会重新生成缩略图 | 仅改 URL 收益较低；重新生成目前先完整解码原图（上限 100M 像素），该解码成本要单独优化，缩略图读取大小保护也值得补齐 |
| macOS 拖出文件的 64×64 系统图标；`macos_quicklook.rs::file_icon_png_base64`、`file_icon_data_url` | History、Popup、Todo 的 native drag icon；像素很小，缓存使用 | 低收益：drag 插件也接受 PNG 文件路径，可用缓存文件代替；需管理清理与拖拽完成时机，不能直接传 WebView 专用 URL |
| Mermaid 渲染沙箱；`mermaid.ts::base64ToUtf8` / `utf8ToBase64` | 解码上游 strict iframe，验证 SVG，重新构造 CSP / sandbox 的自包含 HTML data URL；离线浏览器导出也使用同一文档 | 不适合全局图片协议。临时视图可评估 Blob URL，但沙箱、跨引擎、生命周期和离线导出都要保持；数据 URL 不是必须使用 Base64，但 URL 编码也需评估净收益 |
| 飞书 / 钉钉 / Slack 入站回复图片；`channels/{feishu,dingding,slack}.rs` | 先下载落盘，再读取文件并转 `ImageAttachment.data`；后续结果运输 / CLI 再解码落盘 | 有重复工作：可把内部数据改为已落盘文件引用，直到真正需要 Base64 的边界再编码；涉及首答协调、文件拥有期和旧结果契约，应单独实现。不是 IM 图片下载协议强制要求 Base64 |
| CLI 回复图片保存；`cli/image_writer.rs::decode_image_data`、`models.rs::ImageAttachment` | 接受 raw Base64 或 data URL，清理前缀并解码到本地文件 | 当前兼容接口必须保留；内部引入文件引用后可绕过重复解码，而非删除兼容读取 |
| MCP 返回附件图片；`mcp/ask.rs` 的 ask 工具结果中 `ContentBlock::image` | 读取可返回图片，编码为 MCP ImageContent 的 `data`；远程客户端不能依赖本机图片 URL | 保留协议边界的 Base64；当前 `std::fs::read` 无大小保护，应评估总返回字节 / 数量限制，但不能直接换本机 token URL |
| 飞书 WebSocket 卡片 ACK；`feishu/ws.rs::respond_card` | 业务 JSON 按飞书协议嵌入 Base64 `data` | 保留；体积小，属于协议 |
| Windows hook 命令；`integrations/hook_edit.rs::windows_command` | UTF-16LE → Base64 → PowerShell `-EncodedCommand`，稳定携带路径及参数 | 保留；编码是当前调用方式的要求，改为图片 URL 无关 |
| 发布证书；`.github/workflows/release.yml` | GitHub text secret 保存 P12 的 Base64，构建时解码为证书文件 | 保留；不处于应用运行热路径 |
| IM 性能 mock WebSocket 握手；`scripts/perf-mock-im.mjs` | SHA-1 摘要 Base64 形成 `Sec-WebSocket-Accept` | 保留；WebSocket 协议要求，仅用于测试工具 |

搜索结果中其余命中：`shell_safety.rs` 判断外部 `base64` 命令的安全性、`agents/report.rs` 跳过
大型响应数据、history / types 的“只保存路径、不保存 Base64”注释、i18n 错误文案、依赖声明和测试
数据，都不是另一个 Base64 生产者。Telegram 当前不接收入站图片，没有这条入站编码链路。
CSS 和 demo 的小 SVG `data:image/svg+xml,%3C...` 是百分号编码，并非 Base64；无需随着本次修改。

## 收益与边界

Base64 长度约为原文件的 4/3。20 MiB 文件会得到约 26.7 MiB 的字符串，再经过 IPC JSON 和
前端缓存；现有缓存按 UTF-16 字符估算约 53.3 MiB，实际 JS 引擎字符串表示可能更紧凑。
协议 URL 让前端只保存 token 与尺寸，省去编码、字符串运输和这些长期缓存。

当前 Tauri / Wry 的协议响应仍是完整有界 `Vec<u8>`，不是零拷贝文件映射，也不是文件到显示器
的无内存路径。准备阶段只检查头、尺寸、SVG 结构和动画预算，不生成全量 RGBA；响应阶段重新
读取有界原文件，验证文件指纹与元数据。没有二进制内容的长期 Rust 缓存；后台检查 / 读取串行。
WebView 解码显示仍消耗像素内存，因此保留单图 20 MiB、40M 画布像素、最多 500 帧，以及每份
Markdown 或显示视图 scope 的注册图片总显示预算 80M 像素。SVG 额外保留 2 MiB、5,000 节点
和 65 层限制。正文超限保留“打开原图”，历史 / 草稿沿用已有占位或文件胶囊，不向 WebView 送大图。

## macOS / Windows / Linux 兼容性核查

依据项目锁定的 Tauri 2.11.5 / Wry 0.55.1 源码，并核对官方资料：

| 平台 | 前端地址 | 实际实现 | 证据等级 |
| --- | --- | --- | --- |
| macOS / WKWebView | `askhuman-image://localhost/<token>` | 注册异步 URI 协议并返回二进制响应 | 本机真实 packaged WebView 已验证 PNG / GIF / SVG、拒绝未注册 / 已撤销 URL、字节与像素超限 |
| Windows / WebView2 | 默认 `http://askhuman-image.localhost/<token>`；若应用启用 https scheme，API 自动返回 https | Wry 使用 WebResourceRequested filter / deferral，在 UI 线程交付响应 | 官方支持、当前源码核查通过；本轮没有 Windows 实机运行，不标记验收通过 |
| Linux / WebKitGTK | `askhuman-image://localhost/<token>` | WebContext 注册 URI scheme；异步响应回主线程形成 MemoryInputStream / URISchemeResponse | 官方支持、当前源码核查通过；用户没有 Linux 环境，保留实机 gate |

[Wry 官方协议文档](https://docs.rs/wry/0.55.1/wry/struct.WebViewBuilder.html#method.with_asynchronous_custom_protocol)
明确说明两种平台地址。[Tauri convertFileSrc](https://tauri.app/reference/javascript/api/namespacecore/)
的第二参数支持自定义协议；实现使用该 API，不手写操作系统分支。默认 asset protocol 的 enable /
scope 要求针对 `asset`；本轮单独注册协议，由 token、窗口和文档 scope 限定访问，未启用整个磁盘范围。
项目当前 CSP 为 null；若将来收紧 CSP，需要同时允许自定义 scheme 与 Windows 的对应 origin。

Windows 能力由 [Microsoft WebView2 的 WebResourceRequested 文档](https://learn.microsoft.com/en-us/microsoft-edge/webview2/how-to/webresourcerequested)
支持。Wry 的 `webview2/mod.rs` 对已注册 scheme 的虚拟 HTTP URL 建立 filter，并将请求转换回
scheme URI；响应通过 deferral 交回 UI 线程。本轮没有启动 localhost TCP 服务。

Linux 能力由 [WebKitGTK register_uri_scheme 文档](https://webkitgtk.org/reference/webkit2gtk/stable/method.WebContext.register_uri_scheme.html)
支持。当前 Wry 注册为 secure scheme，在 GTK 主线程处理响应；Tauri runtime 会避免 shared
WebContext 重复注册，并从实际请求的 WebView ID 构造 `UriSchemeContext`。所以多窗口 token 校验
不依赖“第一个注册窗口”的标签。图像 GET 不需要 `linux-body` 的 POST 请求体 feature。

普通 img 不设置 `crossorigin`、不通过 fetch 读取像素，不引入依赖 CORS 的 canvas 功能。
懒加载使用 IntersectionObserver；缺少该 API 时退为有界即时读取。浏览器不支持某个图片编码时，
已有 `error` 回退保留原图打开入口；不承诺旧 WebKit 对所有新图片格式的解码支持。
专用 URL 的运行机制在三平台可行；以上资料不能证明具体 Linux 发行版 / WebKitGTK 版本的实测性能。

## 可复跑验证

`src-tauri/examples/local_image_url_probe.rs` 直接编译生产 `image_resource.rs` / `local_image.rs`，
使用真实 Tauri 页面和真实 `convertFileSrc`，不接触 daemon、用户配置或其他 agent 请求。自建 PNG、
GIF、SVG 和超限 fixture，验证原路径 / file URL、授权与撤销、像素和字节拒绝，结果输出 JSON。
2026-10-10 追加单资源释放：共享引用首次释放仍可加载，最后引用释放后生产注册表返回 404。
WKWebView 在旧 img 仍持有解码资源时可能复用相同 URL；探针单独记录缓存观察，不把撤销等同于
立即清除浏览器像素缓存。前端移除旧 DOM / URL，新拥有期使用新 token。

先安装仓库构建依赖并运行 `pnpm build`；然后在有桌面的系统执行：

```bash
# macOS / Linux
./scripts/local-image-url-probe.sh
# Optional: an existing PNG, including spaces or Unicode in its path.
./scripts/local-image-url-probe.sh '/absolute/path/image.png'
```

```powershell
# Windows: Rust / Windows SDK / WebView2; run in the logged-in desktop session.
.\scripts\local-image-url-probe.ps1
```

Windows 脚本为 Cargo example 嵌入 Common Controls v6 manifest（复用已有 native regression 的
manifest），避免 Tauri example 在测试前因 TaskDialogIndirect 导入失败。Exit 0 代表本次全部
断言符合预期；1 为断言 / 运行错误，2 为 20 秒未完成。不要以无桌面 SSH 构建替代实际 WebView 证据。
