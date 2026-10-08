# Markdown 附件 Mermaid：实现与验证

2026-10-09。用户确认补齐同窗附件预览与浏览器打开，并要求在主工作树开发、保留其他 agent 修改。
行为规格见 `docs/specs/markdown-browser-open.md` §4.1。

## 原因与范围

原有 Mermaid 只接入本地提问 / 历史 / Agent 正文。附件由 Rust pulldown-cmark 生成静态 HTML，
预览直接展示，浏览器快照禁止全部脚本；即使正确标为 `mermaid`，也缺少后续图表渲染。
报告中的 SceneDelegate / SkinManager 流程图在真实 Mermaid adapter 测试中解析和清洗均成功。

本次保持显式 fence 识别、原文件行为及后端 URL 限制。未扩展语言标签猜测、IM 或旧 Quick Look。

## 实现

- Rust renderer 标记首个 info token 忽略大小写后等于 `mermaid` 的 fence；原始 HTML 仍转义，
  生成的内部 wrapper 计入 Markdown 事件预算。
- 独立 `AttachmentMarkdownContent.vue` 消费受限 HTML；`attachmentMermaid.ts` 复用 Mermaid adapter，
  处理异步失效、复制 / 源码、资源上限、宽度变化和 Find 原子元数据。未修改既有正文组件的链接逻辑。
- 浏览器专用 Vite IIFE 包自包含所有图型依赖，按 Safari 13 target 构建；与主包共享 Marked
  lookbehind feature probe 的兼容处理。普通 Popup 入口不加载此包。
- 浏览器后台仅在生成 HTML 有内部 Mermaid 标记时从 Tauri embedded assets 取固定产品脚本，
  写到快照自己的 UUID 目录。CSP nonce 仅允许该经典脚本，图表继续清洗并封装到无权限 iframe。
  普通 Markdown 不取脚本；asset 缺失则保留源文和本地化提示。

## 验证

- Rust 附件相关测试：37 passed / 1 ignored，覆盖 fence 识别、转义、事件预算、快照 nonce、
  普通 Markdown 不加载脚本、缺失 renderer 回退、源文件不变和既有资源边界。
- 前端完整基线及新增 helper：48 文件 / 321 测试通过，另新增组件和报告原文回归的 focused
  3 文件 / 28 测试通过；后续 Find 元数据修改随安装前 focused 检查复核。
- production build 通过。最终浏览器专用包 3,529.81 kB raw / 970.82 kB gzip，独立于普通 Popup 入口。
- 本地 Chromium `file://` harness：flowchart（报告原文）、sequence、state、class、mindmap 渲染；
  非法语法与外链图回退；五个最终 iframe 均空 sandbox；源码切换与系统明暗重绘成功；无 JS / CSP
  error、无 HTTP 请求。已生成打印 PDF并核对可提取的图中文字。页面主 frame 的 `window.find`
  不跨子 frame，图表子 frame 的原生文字查找成功。
- 当前 macOS Safari 实际打开同一离线文件，五种图型的 label 均可见、两张错误图保留源码。
  Safari 原生 Find 查询 `SceneDelegate` 显示 `1 of 2 matches`；图 / 源码切换正常。验证标签页已关闭，
  原有标签页保留。
- Rust Clippy all-targets / `-D warnings` 通过；最后的 helper / 组件 focused 5 项通过。
  `./scripts/install.sh` 编译、安装与正式签名通过；背景修复后再次安装，最终二进制为 44,455,840 bytes。

## Safari 深色背景复核

首轮实际浏览器入口验收指出图表背景仍为白色。已在真实生成的快照复现：主页面声明
`color-scheme: light dark`，但图表子文档未声明自己的 scheme；Safari 在明暗模式不一致的 iframe
中为透明 canvas 加了白底。早期 harness 没有主页面的 scheme 声明，未触发此差异。

已让 Mermaid adapter 的自有 sandbox 文档声明此次渲染的明确 light / dark scheme，并在附件
iframe 上同步 scheme。最终 focused 29 项通过；复制真实快照样式并使用新 renderer 在 Safari
再次验证，白底消失，节点、label、连线均与深色页面一致。测试标签已关闭，用户原快照保留。

最终安装后用户再次检查原例子，确认“两个入口都正常，背景也正常”；本次附件预览与浏览器
支持已完成本机验收。Windows / Linux / Catalina 实机沿用既有外部 gate；本机证据不代替其他平台
验收。仓库级架构与不变量未改变，因此仅更新 Popup 专题 overview 与对应规格。
