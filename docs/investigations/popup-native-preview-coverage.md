# Popup 原生预览覆盖分析

> 2026-10-05；基于 `c08ffce` 的同窗预览。分析、开发、macOS 安装、自动实机验证与用户样例验收已完成。
> 本轮按用户「独立完成分析、开发和验证」的授权推进；用户随后查看实际附件，确认「效果正常，提交本次修复」。

## 结论

ICNS 的退化不是系统失去了解码能力，而是新面板只接受有限的图片类型。常见图片可以通过
macOS Image I/O 解码成 PNG，继续使用现有同窗图片组件，不需要随应用打包大型解码库。
PDF 使用系统 PDFKit 保留多页阅读与阅读位置；Office、富文本和媒体等格式复用 macOS
Quick Look 的公开 `QLPreviewView`。系统视图直接嵌入同窗正文，不启动 Preview.app。

这两条通道已完成。预览区的宽度、标题栏、原文件动作和主回答区继续由现有实现管理；
系统视图仅占预览正文。跨平台另外启用 ICO / TGA / PNM 的轻量 feature，未新增 codec crate，
`Cargo.lock` 不变。本地安装配置的二进制增量约 426 KiB（约 1%），不是零体积或 release 包体积承诺。

## 1. 退化来源与影响

旧 `preview_attachments` 把原附件路径交给系统 Quick Look，系统决定能否预览；新
`attachment_preview::load` 仅把 PNG、JPEG、GIF、WebP、BMP、SVG 送入图片路径，其余先按
2 MiB 的普通文本规则处理。ICNS 的二进制内容因此被拒绝，而不是尝试系统解码。
只增加扩展名还不够：现有 Rust 图片 feature 与 WebView 的显示格式也需要匹配。

以下第二列描述修复前的 `c08ffce` 基线，最终实现与实测范围见 §5。

| 类型 | 修复前同窗面板 | 可利用的能力与边界 |
|---|---|---|
| ICNS、ICO | 不支持 | 本机 Image I/O 实际解码成功；选择图标容器中合适的高分辨率表示，再生成 PNG |
| TIFF/TIF、HEIC/HEIF、JPEG 2000 | 不支持 | Image I/O；本机 TIFF、HEIC、JPEG 2000 编码后再解码成功。多页/多图不能静默丢成第一页 |
| PSD、相机 RAW、OpenEXR 等 | 不支持 | 本机 Image I/O 支持类型列表包含这些类别；尚未逐个真实文件验证。PSD 是合成图预览，RAW 支持随系统与机型变化 |
| PDF | 不支持 | Quick Look 或 PDFKit，可保留多页阅读；单张 PNG 或缩略图不能替代完整 PDF |
| Office、iWork | 不支持 | Quick Look 支持这类文档；实际格式能力受系统和预览扩展影响 |
| RTF、RTFD | RTF 可能直接显示格式控制文本，RTFD 包不支持 | 需要先识别富文本，再交系统预览；不能因可解码为 UTF-8 就当普通正文 |
| 音频、视频 | 不支持 | Quick Look 提供媒体预览；应避免自动播放声音 |
| Markdown、diff、普通代码/文本、已有图片 | 已支持 | 保持当前自绘渲染；GIF/WebP 保留动画，SVG 保留图片隔离 |

Quick Look 的官方范围包括图片、PDF、iWork/Microsoft Office、音频和视频。
这不是任意二进制文件的兼容承诺，未知类型仍可能只有文件图标或文件信息。
[Apple Quick Look UI](https://developer.apple.com/documentation/QuickLookUI)

## 2. 不引入大型依赖的路线

### 2.1 原生图片解码 → 现有图片组件

在 macOS 的后台读取路径中使用 Image I/O，从已授权且有界读取的附件数据创建 image source，
检查类型、图像数量与像素尺寸，然后生成 WebView 可显示的 PNG。这样 ICNS、TIFF、HEIC 等
仍能使用现有 fit、100%、缩放、平移、每附件阅读状态；打开与拖出仍指向原文件。

Image I/O 的支持类型应从运行系统查询，不能把本机支持列表硬编码成所有 macOS 版本的保证。
本机已对项目实际 `icon.icns` 解码得到最大 1024×1024 表示，对实际 ICO 解码成功；另对
TIFF、HEIC、JPEG 2000、TGA、ICO、AVIF 做了小图编码/解码往返验证。
[CGImageSourceCopyTypeIdentifiers](https://developer.apple.com/documentation/imageio/cgimagesourcecopytypeidentifiers())

图标的多种尺寸是同一图像的表示；多页 TIFF、图像序列与动画则需要各自语义。不能把所有
多图文件都取第一幅后宣称完整支持。沿用现有文件/像素上限，并检查转码输出与缓存预算；
转码本身仍消耗 CPU 和内存，系统解码器不等于没有资源成本。

系统框架随 macOS 提供，应用无需打包其解码器。新增体积来自桥接代码和轻量 codec，实际
测量见 §5。项目已经使用 objc2/AppKit，并已有 NSImage 到 PNG 的
系统图标转换代码，可复用部分编码/数据桥接思路；大图解码不应复用其主线程绘制路径。

### 2.2 少量跨平台 codec

现有 `image 0.25.10` 的 ICO feature 只复用已经启用的 BMP、PNG，TGA、PNM feature 不增加
外部 codec crate；可作为跨平台补齐。TIFF 需要增加 tiff crate，应另外测量。不要为补几个
格式直接开启全部默认 features；AVIF/EXR 等会引入各自额外依赖。
[image 0.25.10 Cargo features](https://github.com/image-rs/image/blob/v0.25.10/Cargo.toml)

macOS 特有能力不应变成 Windows/Linux 的虚假支持承诺；这些平台可逐步使用轻量 codec 或
对应系统能力，无法解码时保留原文件操作。

## 3. 利用系统预览的几种方式

| 方式 | 依赖体积 | 能力 | 接入成本 |
|---|---|---|---|
| Image I/O → PNG | 系统框架，少量桥代码 | 常见图片；沿用现有图片交互 | 较低 |
| QLThumbnailGenerator → PNG | 系统框架 | 缩略图/文档封面，适合列表；不能替代多页阅读或媒体播放 | 较低，但正文能力有限 |
| 同窗 PDFView / QLPreviewView | 系统 PDFKit / QuickLookUI 框架 | PDF 多页阅读与系统文档、媒体预览 | 需要原生视图与 WebView 协作 |
| 独立 QLPreviewPanel | 复用现有实现 | 原有系统预览范围 | 较低，但会重新出现独立窗口 |

Preview.app 是独立应用；适合本项目同窗接入的是系统提供的图片框架和 Quick Look 视图。
`QLPreviewView` 可加入 macOS 的 NSView 层级，加载是异步的；其 `displayState` 表示当前预览项
的显示状态，具体保存 / 恢复能力由系统预览扩展决定。实测不能依赖它保证所有文件的阅读位置。
不能把原生 NSView 当作普通 DOM 元素放进 WebView。
[QLPreviewView](https://developer.apple.com/documentation/quicklookui/qlpreviewview),
[displayState](https://developer.apple.com/documentation/quicklookui/qlpreviewview/displaystate)

同窗接入覆盖正文区域坐标/裁剪、分隔线与缩放、原生焦点、Esc/发送/取消快捷键、Web 弹层
覆盖、附件切换异步代次和终态 close；这些交互已通过 macOS 自动实机验证。
系统不支持或加载失败时仍展示文件信息与原文件操作。独立系统预览可作为另一个产品选择，
但它改变此前「Popup 统一同窗、不保留 Quick Look 模式」的约定，不能未经确认自动改回。

## 4. 已实施范围

1. 保留现有 Markdown/diff/可靠文本与已支持图片的自绘流程。
2. macOS 补 Image I/O 图片通道；优先验收 ICNS、ICO、TIFF、HEIC、JPEG 2000、PSD。
3. PDFKit 在右侧正文提供 PDF 多页阅读；Quick Look 作为 Office、富文本、媒体、加密 / 无法
   加载的 PDF 及其他系统可预览格式的通道。Image I/O 无法识别的扩展图片也交系统扩展尝试。
4. 预览能力与 CLI/IM 的 `isImage` 传输分类分开；不能为了预览 PSD/ICNS 就把它们全局当作
   IM 图片发送。缩略图候选也要独立扩展，目前前端仅为 `isImage` 请求缩略图。
5. 不以系统缩略图代替完整正文；多页/多图/动画明确处理。保持请求内附件授权、后台预算、
   原文件操作与过期结果清理。

## 5. 实现与验证记录

- `macos_attachment_preview.rs` 与 `swift/AttachmentPreview.swift` 接入系统图片与正文。
  Image I/O 解码在有界后台读取中执行；图标取最大表示并处理方向。已识别的损坏图片说明
  无法显示，超限文件说明限制；均保留原文件操作。
- 多图 TIFF / HEIC 等保持原始文件交给 Quick Look，并显示识别出的图像数量及「打开原文件
  查看全部」提示。系统扩展可能只提供代表图像，不能声称完整多图浏览。
- PDF 后台创建文档，主线程挂载；保存页面、页面坐标与缩放。切换 / 收起后恢复阅读位置，
  过期任务不能重新挂载。最多保留两个静态 Quick Look 文档视图，DOCX / RTF 切换保留滚动；
  被淘汰的文档及媒体按系统扩展能力尽力恢复。新请求清除上一条的原生视图与阅读状态。
- 前端只传请求 ID / 附件索引和正文坐标，后端检查活跃请求、更新代次、已授予读取许可、
  文件大小 / 修改时间及布局裁剪。原生内容不覆盖主回答区或标题；取消确认遮罩出现时隐藏。
  原生焦点的 Esc / 发送 / 取消回到既有处理，组合输入优先；媒体不自动播放，切换后卸载。
- 系统图片沿用 20 MiB / 4,000 万像素；多图最多 500 幅 / 8,000 万总像素；转码 PNG 最多
  20 MiB。文档 / 媒体输入最多 100 MiB。缩略图读取最多 2 MiB、最长边 192 px；前端列表缓存
  8 MiB。原生文档最多两个缓存视图，系统扩展内存不计入前端 64 MiB JSON 缓存。
  未识别二进制只在 2 MiB 有界文本读取内兜底，不因此放宽普通文本上限。
- 原 `isImage` 传输分类不变；HTML / 代码仍为惰性文本，Markdown / diff 使用原渲染，
  GIF / WebP 原字节动画及 SVG 图片隔离保留。RTFD 目录包仍不符合普通文件附件契约。

| 验证层 | 证据与结果 |
|---|---|
| 系统图片实际文件 | ICNS、ICO、TIFF、HEIC、JP2、PSD、EXR、AVIF、TGA 在真实 WKWebView 中显示；ICNS 列表缩略图、缩放与损坏文件提示通过 |
| 系统文档实际文件 | 三页 PDF 连续滚动；滚到第三页 → RTF → PDF / 收起重开恢复同一位置；长 DOCX 滚到底 → RTF → DOCX 保持位置；RTF 排版通过 |
| 多图与媒体 | 两图 TIFF 的数量与原文件提示通过；WAV 显示系统播放控件，停留后仍在 0 秒且未自动播放 |
| 原生 / Web 交互 | 深浅主题、分隔线、最大化 / 恢复、原生焦点 Esc、Cmd+W 取消确认遮罩、Cmd+Enter 与按设置的裸回车、多题草稿提交通过；IPC 结果保留两题草稿 |
| 自动回归 | Rust 全量 1229 passed / 3 ignored；最终附件定向回归 10 passed / 1 ignored；Vitest 208 / 36 文件、Node 5；前端生产构建、Clippy、格式检查通过 |
| 安装 | `./scripts/install.sh` 编译、签名并安装成功，最后使用其安装二进制的独立 QA bundle 验证；生产偏好未改动 |
| 用户样例验收 | 提供 ICNS、HEIC、PSD、两图 TIFF、三页 PDF、长 DOCX、RTF 与 WAV 实际附件；用户确认效果正常并授权提交 |

原生探针、自造夹具、IPC 结果、截图与日志位于 `/tmp/askhuman-popup-qa/`；完整交付报告为
`native-preview-verification.md`。二进制比较使用同一个 `local-install` 编译配置，基线
42,083,072 bytes；未引入新外部 codec crate，不把这一测量外推为生产 release 体积。

本机未用实际相机 RAW、加密 PDF、所有 Office / iWork 版本或所有视频 codec 逐一验收；
这些能力取决于 macOS 版本、相机型号和已安装系统预览扩展。未知文件可能只有系统文件信息。
Windows / Linux 的轻量图片 feature 已实现，但其 GUI 实机验收继续属于既有外部 gate，
不把 macOS 实测当作这些平台的证据。
