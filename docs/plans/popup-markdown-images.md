# 提问正文的本地 Markdown 图片

2026-10-09，用户反馈动效原型提问中的图片没有显示，要求正式接入动效后排查。

## 初始 Base64 修复（已验收，后续由下述 URL 路径替代）

原消息使用 `![750ms 原型](/Users/.../preview-750.png)`。Markdown-it 直接生成绝对路径
`img src`，WebView 把它解析到 `tauri://localhost` 或 `http://tauri.localhost` 的应用资源下，
没有读取系统文件。普通文件链接已有委托打开路径，但图片没有对应的读取步骤。

- `MarkdownContent.vue` 启用本地图片渲染：绝对路径、file URL、项目相对路径经已有路径解析器
  处理，先输出无 src 的占位 img，再调用现有 `read_image_data_url` 接口。
- 每轮 DOM 渲染按路径合并重复读取；主题、源码或项目变化后的旧结果不得写入新 DOM。
  图片加载后发送既有 `markdown-content-updated` 事件，供布局与搜索刷新。
- 正文与各题、历史记录共用该组件；相对路径使用调用方传入的项目目录，不使用应用资源地址。
- HTTPS / data 图片沿用原渲染方式；HTML 转义、危险链接协议限制和外部链接委托仍生效。
  文件缺失、相对路径无项目或图片无法解码时显示“图片无法显示”和原 alt 标签。
- 不增加 IPC、文件读取权限或图片弹出交互；复用现有图片读取接口及浏览器支持的格式。

## 验证

Markdown 及组件定向回归 54 项通过，覆盖两个应用资源 origin、本地绝对路径 / 中文 / 空格 /
file URL / 项目相对路径、重复读取、远程图不经本地读取、缺失与解码失败、过期结果丢弃，
以及已有 Mermaid、链接委托和 HTML 转义检查。`vue-tsc --noEmit` 通过。

完整前端 348 项及 Node 5 项通过，`./scripts/install.sh` 编译、签名并安装成功。
经 graceful drain 自动换新后，以新安装的 AskHuman 在正文直接嵌入原先失败的
`preview-750.png`，用户确认“两项都正常，可以保留”（动效与正文图片）。
Windows / Linux 的图片解码与路径实机证据仍需随现有跨平台 Popup gate 补齐。

## 大图性能与原文件 URL 可行性调查

用户进一步要求补充大图性能保护，并提出原文件 URL 方案；由于记得早期本地文件加载存在问题，
要求先确认技术可行性。初步验证时正式路径仍为上述 Base64 读取；之后用户确认接入原文件 URL、懒加载和超限回退。

历史 `file-attachments.md` 的 D10 / D12 明确采用 Base64 + CSS 缩放，实施计划接受 v1 全量读取。
首次实现提交 `bf93899` 与当前配置均未开启 asset protocol；仓库可检索历史中未找到
`convertFileSrc` / asset protocol 试验及失败记录，不能据此断定用户记忆中的试验未发生。

独立探针 `src-tauri/examples/local_image_url_probe.rs` 复用当前 Tauri 2.11.5、custom-protocol
构建和真实 WKWebView，在 `tauri://localhost` 页面验证同一张 1280×825 PNG：

| 图片地址 | 本机结果 |
|---|---|
| 原始 `/Users/.../preview-750.png` | 加载失败 |
| `file:///Users/.../preview-750.png` | 加载失败 |
| 当前未启用的 `asset://` | 加载失败 |
| 注册的 `image-probe://localhost/known-image` | 成功，naturalWidth=1280 / naturalHeight=825 |
| 未分配的协议地址 | 拒绝，不显示图片 |

运行命令：`cargo run --manifest-path src-tauri/Cargo.toml --example local_image_url_probe --features custom-protocol -- <绝对 PNG 路径>`。
最初探针不接触 Daemon、其他在途提问或 IM，只提供一个固定允许文件的二进制图片响应；不使用
Base64，也未更改产品配置。结果证明注册图片协议可行，不能直接将裸路径 / file URL 当作 src。
单张小图成功不构成大图内存或跨平台性能证据。

官方依据：[Tauri convertFileSrc](https://tauri.app/reference/javascript/api/namespacecore/)
明确要求 asset protocol 启用和路径 scope；[异步协议接口](https://docs.rs/tauri/latest/tauri/struct.Builder.html#method.register_asynchronous_uri_scheme_protocol)
可从后台返回二进制图片。该 API 仍返回完整响应体，不能将其称作零拷贝文件映射或无需内存的
流式加载。用户确认原图 URL 方向后，已按下述范围接入体积与像素限制、懒加载及生命周期。


## 已确认的原图 URL 实现

- 使用 `askhuman-image` 异步协议，JSON IPC 只返回随机 token 和尺寸；`convertFileSrc` 生成
  操作系统对应地址。协议只服务当前窗口、当前文档 scope 内的 token，不接受磁盘路径 URL。
- `local_image_create_scope` / `local_image_prepare` / `local_image_release_scope` 管理文档拥有期；
  在 IntersectionObserver 的 240px 可见区域缓冲内才准备图片，同一路径合并请求。
  缺少 observer 时回退为有界读取。源码、项目、主题或语言变化以及组件卸载都撤销 scope；
  窗口销毁清掉该窗口全部 scope，后台过期结果不再启动新图解码。
- `image_resource.rs` 从附件读取器提取共用预算与 header / SVG / 动画检查，避免另建不一致限制。
  单图 20 MiB、40M 画布像素，SVG 2 MiB、5,000 节点 / 65 层；动画不超过 500 帧及 80M
  累计像素。每份 Markdown 的注册图片合计 80M 显示像素，按路径去重，最多 128 个资源。
- 后台检查与协议读取共用单 worker；仅检查压缩文件信息，不生成全量 RGBA。
  协议返回原字节，GIF / APNG / WebP 保留动画；发送前重新核对文件指纹及图片元数据。
  响应设置 `no-store`，没有 Rust 原图字节缓存。WebView 仍需要解码内存；2026-10-10 探针确认
  WKWebView 可能复用已解码的同 URL 像素，不能把 token 撤销视为立即清除浏览器缓存。
- 点击图片或键盘 Enter / Space 用系统默认应用打开原图；超限显示“图片过大”及原图入口。
  缺失 / 不支持 / 解码失败也保留图片说明和原图入口。没有项目目录的相对图片不能推断路径。
- 用户要求平台兼容性分析，并说明没有 Linux 环境。官方资料及当前依赖源码确认 Windows
  WebView2 与 Linux WebKitGTK 的机制支持，不能标记实机验收通过。完整审计、依据和可复跑
  平台脚本见 `docs/investigations/base64-usage-audit.md`。

## URL 路径验证

- 完整前端 351 项、Node 5 项及 TypeScript 检查通过；新增组件测试覆盖可见区域懒加载、
  重复路径、晚到 scope 清理、旧消息结果丢弃、加载错误和点击 / 键盘原图操作。
- 新增 Rust registry 5 项通过，涵盖真实二进制、窗口隔离、撤销、任意路径拒绝、文件变更、
  字节 / 像素 / 动画 / 文档总预算、路径复用、窗口清理、HEAD 与错误 method。
  原附件相关 10 项通过 / 1 个手动 benchmark 忽略；全 targets Clippy `-D warnings` 和格式检查通过。完整 Rust 1289 项通过 / 3 项忽略。
- 独立探针改为直接编译生产模块。本机真实 packaged WKWebView 成功加载 1280×825 PNG、
  原字节 GIF 与 SVG；原路径 / file URL / 未注册与已撤销 URL 均失败，20 MiB 和像素超限返回
  `limit`。探针不经过 Daemon，也不影响其他 Agent 的提问。
- `./scripts/install.sh` 编译、签名与安装成功，Daemon 在无活跃请求时正常换新。
  2026-10-10 用户在新安装的 AskHuman 实窗确认普通 PNG、10000×10000 SVG 的超限入口、
  点击打开原图符合预期，选择“符合，保留这版”。Windows / Linux 保持上述资料核查边界。
