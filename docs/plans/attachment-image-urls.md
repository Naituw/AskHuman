# 附件大图的有界图片 URL

2026-10-10。用户确认右侧附件大图也移除 Base64，转码 PNG 使用有界临时文件加图片 URL。
历史继续保存原文件路径，不保存预览 token 或临时 PNG，不新增永久原图副本。

## 数据与拥有期

- `popup_preview_read` 仍从活跃请求的附件索引取得路径；后台两个读取 worker 和读取代次验证保留。
  `attachment_preview::load_url` 给每个图片结果建立窗口所有的 scope，只返回 token、尺寸、
  二进制字节数与总显示像素。前端通过 `convertFileSrc(token, "askhuman-image")` 显示。
- PNG / JPEG / GIF / WebP / BMP / SVG 由共享 `local_image.rs` 协议读取原文件二进制。
  GIF / WebP 保留原字节动画；SVG 仍为隔离 img。准备阶段不生成整幅 RGBA，也不编码 Base64。
- ICO / TGA / PNM 与 macOS Image I/O 的单幅系统图片沿用有界解码，转成 PNG 写入私有临时目录。
  协议响应的 MIME、尺寸、Content-Length 与预算来自转码 PNG；原文件的大小 / 修改时间指纹继续
  约束访问。多图 TIFF / HEIC、PDF 和其他系统文档保留原生视图流程。
- 后端读取完成后再次检查请求、代次和窗口；失效结果立即撤销 scope。前端收到过期结果同样
  撤销。每个图片由内容缓存与当前显示各持一个逻辑引用；缓存淘汰不撤销另一个 reader 仍在显示
  的图片。最后引用释放、请求结束或窗口销毁时撤销，转码临时 PNG 随资源删除。
- 收起预览保留有预算的缓存以支持快速重开；切附件不清掉独立的缩放 / 平移和阅读状态。
  原文件打开、拖出、菜单和历史记录始终针对原路径；原文件移动 / 删除沿用不可用提示。

## 资源边界与清理

单图仍限 20 MiB、4,000 万画布像素；动画最多 500 帧 / 8,000 万总显示像素；SVG 限
2 MiB / 5,000 nodes / 65 层。转码 PNG 也限 20 MiB。注册表保留最多 256 个窗口所有的
scope、每 scope 最多 128 个资源 / 8,000 万显示像素；后台协议读取串行执行。

当前 WebView 的内容 LRU 缓存共享 64 MiB 成本预算、8,000 万总显示像素和 128 项上限。
成本为 JSON 的 UTF-16 字符估算加图片二进制大小；不以短 URL 长度冒充图片成本。像素预算独立
约束解码显示量，不宣称为 WebView 实际内存测量。缓存外仍显示的图片单独持有引用，受单图与
注册表上限约束。原文件不长期缓存 Rust 字节；协议响应仍是完整有界 Vec，不是零拷贝。

转码目录通过 `tempfile` 创建，使用 `askhuman-preview-images-` 前缀和进程独占锁。普通释放删除
对应 PNG，正常退出删除目录；下次首次转码清理超过 24 小时且能取得独占锁的遗留目录。
Unix 同时校验目录所有者，跳过符号链接；其他仍运行的进程即使存活超过一天也不被清理。
进程崩溃的残留不会写入历史或永久缓存。WebView 可能保留已经解码的像素，撤销 URL 只保证
后端后续读取被拒绝；前端移除旧 img，新拥有期使用新 token。

列表小缩略图、回复 / 粘贴运输、MCP / IM 与已有离线快照的 Base64 契约保持。此次只改变附件
大图的显示路径；其他纯显示入口见 [display-image-urls.md](display-image-urls.md)。

## 验证记录

- 前端 368 Vitest + 5 Node tests、类型检查通过。新增覆盖 URL 转换、关闭重开与缩放状态、
  过期结果、字节 / 像素预算淘汰、多 reader 显示引用及请求切换释放。
- Rust 1296 passed / 3 ignored。新增覆盖原 GIF 二进制、TGA 转 PNG、短 JSON 响应、临时文件
  删除、原文件变化 / 取消拒绝、有效进程锁及崩溃目录清理。
- macOS 系统格式回归切到实际 `load_url` 路径，覆盖 ICNS / ICO 转码 URL、多页 TIFF 保留原生、
  系统文档与损坏 / 不支持图片回退。
- `./scripts/local-image-url-probe.sh` 在真实 packaged WKWebView 通过，包含临时生成 PNG
  的 32×18 显示、撤销后 404、原 PNG / GIF / SVG 与字节 / 像素超限；跨窗口拒绝由 Rust 回归覆盖。
  类型检查、Rustfmt 与 custom-protocol all-targets Clippy 通过。
- `./scripts/install.sh` 完成类型检查、production 前端、local-install 二进制、稳定身份签名与
  `/Users/wutian/.local/bin/AskHuman` 安装。安装后版本命令正常，使用新的 AskHuman 附图交付；
  自动验证与探针通过不等同于用户已完成本次视觉验收。
- 安装后通过 AskHuman 交付 JPEG、动画 WebP 与 ICNS，用户确认「显示正常，保留当前实现」，
  并授权推送和发布新的 patch 版本。
- Windows / Linux 沿用既有实机 gate，
  不以 macOS 结果替代；协议依据与复跑脚本见 [Base64 审计](../investigations/base64-usage-audit.md)。
