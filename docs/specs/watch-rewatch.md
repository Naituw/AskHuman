# Watch 卡片「重新关注」按钮

> AutoStopped 终态的 watch 卡片提供可点击按钮，让用户一键重新关注同一 agent。

## 背景

「按需发送」模式下活跃槽切走时，watch 卡会被自动结束（`FinalKind::AutoStopped`）。典型场景：
用户在飞书 watch 了一个 agent，然后在弹窗中回复了问题，导致活跃槽切到 popup，飞书上的 watch
卡被自动结束。此时 agent 大概率仍在工作中，用户回答完问题后往往想继续关注。

## 决策记录

- **D1 适用范围**：仅 `AutoStopped` 支持重新关注。其他终态不适用：
  - `Ended`：agent 已结束，无内容可关注
  - `Idle`：对本来就 Idle 的 agent 发一次性终态卡；已有订阅转 Idle 时先保留 5 分钟宽限，期满后的
    Idle 终态仍不提供重新关注
  - `Cancelled`：用户主动取消，不需要在同一张卡上提供重入口
  - `Replaced`/`Moved`：新卡已存在，旧卡无意义
- **D2 交互方式**：点击后发新卡（非就地恢复旧卡为活动态），旧卡按钮变为 disabled「已重新关注」
- **D3 按钮文案**：「已切换到 {to} · 重新关注」/ "Switched to {to} · Re-watch"
  - 按钮可点击（非 disabled），样式为 `default` 类型（非 primary/danger，视觉偏淡，保持终态感）
  - 点击后按钮变为 disabled「已重新关注」/ "Rewatched"（防止重复点击）
