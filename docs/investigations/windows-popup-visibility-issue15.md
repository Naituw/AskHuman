# Windows 合并窗口空白残留：#15

2026-10-08，按用户要求先复现、再修复，并验证最近新增的合并窗口功能。

## 复现与根因

[Issue #15](https://github.com/Naituw/AskHuman/issues/15) 报告 Windows 11 build 26300、
AskHuman 0.14.0、默认 merged、popupPrewarm / alwaysOnTop 开启时，最后一题答完后留下空白窗口。
本次在 Windows 11 Home x64 24H2（10.0.26100）的已登录 Session 1 使用官方 0.14.0；exe SHA256
为 `00021CED65E8DB391CF207D5FEDFB49A8F4EF9F1192444BFFCC5A83547EC277F`，与报告一致。

三轮真实 UI Automation 填写并提交都返回正确 CLI 答案，daemon 活跃请求为零，但同一个
HWND 656446 / Popup Host PID 11920 仍可见且空白。独立模式对照答完销毁窗口。

`popup_transition::front` 直接调用 Win32 `ShowWindow(SW_SHOWNOACTIVATE)`，没有更新 Tao 0.35.3
内部 visibility flag。窗口由 `visible(false)` 创建，Tao 因而仍认为它隐藏；随后 Tauri `hide()`
走 hidden → hidden 的无变化分支，返回成功但不执行原生隐藏。该路径在 0.14.1 中也存在。

修复在主线程上先调用 Tauri `show()` 同步状态，再保留原生非激活显示 / 前置。窗口创建时
`focused(false)`，显示保持非激活；原生探针验证另一窗口仍持有前台焦点。空队列 `hide()`
返回错误时增加 `popup_host / idle_hide_failed` 持久日志，日志不包含题目与回复。

## 实机结果

| 场景 | 结果 |
| --- | --- |
| 官方 0.14.0，merged + prewarm，连续三轮 | 3/3 可见空白残留，答案正常返回 |
| 修复版，merged + prewarm，连续三轮 | 3/3 隐藏，复用 HWND 918590 / PID 1324，CLI exit 0 |
| 修复版，merged + 关闭 prewarm，两轮 | 2/2 销毁窗口，CLI exit 0 |
| 修复版，independent | 答完销毁，CLI exit 0 |
| 原生探针，alwaysOnTop 开启 / 关闭各三轮 | 显示、可见前置、隐藏通过；另一窗口前台及 frame 不变 |
| 原生探针，两种置顶设置分别最小化再前置 | 恢复、隐藏通过，另一窗口前台不变 |
| 两个项目，六条普通请求 | 分组、独立草稿、答案路由、提交后自动换题通过 |
| 新到达动效 | 实际截图序列显示中央气泡、飞入对应未读点、高亮和持续 ripple |
| Markdown 预览展开后两条突发到达 | 三栏保持，动画中心排除右侧预览，当前草稿不变 |
| Sidebar 内部分隔线拖动 | 外窗 frame 与预览边界不变，正文分配相应调整 |
| 主动最小化后新题到达 | 恢复同一窗口，当前草稿保持 |
| 关闭确认打开后再到达新题 | 只取消快照中的两题，新题保留并可提交，最终隐藏 |
| 普通提问 / Claude Permission / Codex Stop 混合 | 同窗三项；取消后普通请求取消、Permission deny、Stop `{}` 放行，CLI exit 0，最终隐藏 |
| 中文草稿及文本选区 | `Unicode draft 中文 12345` 和完整选区在混合到达前后相同，编辑器仍聚焦 |

原生回归直接引用生产 `popup_transition.rs`，以 `IsWindowVisible` / `IsIconic` /
`GetForegroundWindow` / `GetWindowRect` 断言 HWND；共 8 个场景通过。运行：

```powershell
.\scripts\popup-visibility-regression.ps1
```

需交互式 Windows 桌面。脚本编译 local-install example，再使用 Windows SDK `mt.exe` 嵌入
Common Controls v6 manifest；Cargo example 不自动取得 Tauri 正式 binary 的资源，省略 manifest
会在进入测试前因 `TaskDialogIndirect` 导入失败而退出。已验证完整脚本 exit 0。

macOS / Windows 的全 targets Clippy（`-D warnings`）、格式检查与安装通过，20 个相关 Rust
测试通过。前端没有改动；Windows 安装包含类型检查和 production Vite build。

## 环境与证据保留

VM 测试根目录 `C:\dev\AskHuman-issue15-20261008`，包含 `baseline-merged`、`fixed-merged`、
`fixed-cold`、`fixed-independent`、`features`、`mixed` 的结果 JSON、CLI 输出和截图序列；
原生完整脚本记录为 `native-runner.stdout.txt` / `native-runner.exit.txt`。

生产修复源码在当前主工作树，Windows 只接收不含 Git 元数据的导出副本。未覆盖
`C:\dev\AskHuman` 的旧分支 / 未提交修改，也未覆盖旧 0.12.2 安装；原有 GUI PID 9328 / 9484 /
9568 保留。移除新增测试 PATH 与单次任务，恢复测试 daemon 自动迁移的用户 Codex Stop Hook
到原安装路径，保留 `track` 语义及其他 Hook 字段。SSH 连接按用户要求长期保留在交接文档旁。

本次仅证明该 Win11 VM、popup-only、本地实际 WebView2 场景。没有据此声称 build 26300、
Windows 10、Linux、真实 IM 抢答终态、多 DPI / 多屏、真实 IME 组合态和完整附件格式矩阵通过；
对应延期 gate 仍在 `docs/PROGRESS.md`。
