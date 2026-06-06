# askhuman

跨平台「Human-in-the-loop」交互工具。命令行 `AskHuman` 在 AI 助手需要确认/补充时弹出窗口（或经 Telegram / 钉钉 / 飞书）收集人类回应，并把结果按固定区块写到 stdout。

底层为单一可执行文件（Tauri 2 / Rust）。本 npm 包通过「平台子包」分发：安装时只会拉取与当前平台匹配的一个二进制。

## 单独使用

```bash
npm i -g askhuman
AskHuman "要不要继续？" -o "继续" -o "停止"
```

## 作为依赖（程序集成）

```bash
npm i askhuman
```

```js
import { getBinaryPath, isAvailable } from "askhuman";
import { spawnSync } from "node:child_process";

if (!isAvailable()) {
  // 二进制未就位：跳过人工确认环节，避免阻塞流程
} else {
  const r = spawnSync(getBinaryPath(), ["要继续吗？", "-o", "继续", "-o", "停止"], {
    encoding: "utf8",
  });
  if (r.status === 3) {
    // 当前环境无任何可用 channel（GUI 打不开且未配置会话型渠道）：降级处理
  } else if (r.status === 0) {
    // 成功：解析 r.stdout 的结果区块
    console.log(r.stdout);
  }
}
```

`getBinaryPath()` 解析顺序：环境变量 `ASKHUMAN_BINARY`（兼容旧 `HUMANINLOOP_BINARY`）→ 平台子包 → 系统 `PATH`。

## 退出码契约

| 退出码 | 含义 |
|---|---|
| `0` | 成功拿到结果，或用户取消（输出 `[状态]`） |
| `3` | 无任何可用 channel（本地弹窗打不开且未配置会话型渠道）——下游应降级 |
| `1` | 其他异常 |

stdout 只含结果区块（`[选择的选项]`/`[用户输入]`/`[图片]`/`[文件]`/`[状态]`），所有日志与报错走 stderr。

## 平台与系统依赖

支持 macOS（arm64/x64）、Windows（x64）、Linux（x64）。

> Linux 运行 GUI 弹窗需系统具备 WebKitGTK（如 `libwebkit2gtk-4.1`）。若缺失且配置了会话型渠道（Telegram / 钉钉 / 飞书），会自动改走该渠道；若皆不可用，则以退出码 `3` 提示降级。

更多信息见项目仓库：<https://github.com/Naituw/HumanInLoop>
