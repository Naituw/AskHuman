# Preview 分割线拖动响应修复

> 2026-10-07：问题已由用户在安装后二进制的隔离 Popup 中复现；修复范围经 AskHuman 确认。
> 修复、安装与正式 Popup 用户验收已完成。当前工作树实施，不创建 Worktree，不修改其他 Agent 的功能。

## 1. 问题与证据

- Markdown Preview 的分割线会落后于鼠标；快速往返后，停住时仍依次回放旧位置。
- 5 KB 普通 Markdown 同样复现；同一窗口拖右侧外缘流畅，排除文档大小作为主要原因。
- 私有配置写入观察到数十次旧位置更新，一个连续批次约 2.4 秒，主区先到约 731 再回约 452。
- 合并模式每帧经共享队列执行 `popup_inbox_layout → commit → finish`；prepare / commit
  等待鼠标主键松开。独立模式也逐帧排队且每次写配置。
- 自动化未可靠执行物理拖动，以上观感以用户现场复现为证；不得将失败的合成拖动算作通过。

## 2. 已确认方案

- 分割线仅重分配正文与 Preview 的内部宽度，保持外窗、Sidebar 和响应式原生 viewport。
  专用路径不冻结大画布，不执行窗口 setter，不等待鼠标释放。
- 合并与独立窗口共享最新位置合并：一个执行中的请求，待处理位置被新位置覆盖。
  松手发送最终位置，只在结束时持久化有效尺寸；键盘调整同样作为一次完成的操作。
- 展开 / 关闭仍使用原窗口事务；请求切换、预览关闭、组件销毁丢弃未执行的旧拖动及迟到回执。
- 普通状态主区至少 420、Preview 至少 320；受限 / 最大化分区使用既有临时下限，极窄时平分。
  临时分配不覆盖正常偏好，保留草稿、阅读状态、图片与原生 PDF / Quick Look。
- 行为规格沿用 `docs/specs/popup-attachment-preview-panel.md`；不修改全局 overview 地图。

## 3. 验证

- 前端相关回归 **49 passed / 4 files**：阻塞 IPC 下快速往返只执行首个及最终位置；两种模式
  使用同一合并队列，最终请求携带 finished；关闭、切请求不回放旧拖动或应用迟到回执。
  合并模式仅调用内部 resize，保留草稿，不调用 prepare / commit / finish。
- Popup Rust 回归 **31 passed**：普通 / 受限 / 最大化及极窄分区、无 / 有 Sidebar、无效
  extent；分隔线重分配不改变窗口 frame、Sidebar 与原生 viewport，临时分配不作为持久偏好。
- 类型检查、production build、Clippy custom-protocol all-targets（`-D warnings`）与 diff 检查通过。
- `./scripts/install.sh` 编译、签名、安装通过；验证时正式 Popup 进程在本次安装后启动。
  没有停止其他 Agent 的 Popup / daemon；自建临时 GUI 与假 IPC 服务已退出。
- 首次新版本复查未附文件，用户要求通过 AskHuman 直接附测试文件；随后正式提问附上同一
  Markdown、仓库图片与既有三页 PDF。用户实际拖动、切附件并输入草稿，确认
  **「顺畅跟随，图片、PDF 和草稿也正常」**。正式合并窗口的实机观感以此为证。
- 独立模式的队列与结束记忆变更有自动回归覆盖，本轮未宣称独立模式另做物理拖动验收。
  Windows / Linux 外部实机 gate 仍沿用既有附件预览事项，本次 macOS 修复不替代它们。

临时测试记录位于本机 `askhuman-divider-qa-*` 目录；构建日志位于
`/tmp/askhuman-divider-{rust-tests,clippy,install}.log`，不是发布资源或长期测试依赖。
