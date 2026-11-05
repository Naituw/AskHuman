// Prototype-only fixtures + URL params for the Mac-side away-mode surfaces
// (menu-bar glyph, floating capsule / card / welcome-back, tray menus).

export type MacAwayView =
  | "capsule"
  | "capsule-pending"
  | "card"
  | "welcome"
  | "tray-on"
  | "tray-off"
  | "tray-agent"
  | "banner-over"
  | "capsule-center"
  | "menubar-card";

export type Theme = "light" | "dark";

export interface MacAwayParams {
  view: MacAwayView;
  theme: Theme;
  w: number;
  h: number;
}

export function readParams(): MacAwayParams {
  const q = new URLSearchParams(location.search);
  const view = (q.get("view") ?? "capsule") as MacAwayView;
  const theme = (q.get("theme") === "dark" ? "dark" : "light") as Theme;
  const num = (key: string, fallback: number) => {
    const n = Number(q.get(key));
    return Number.isFinite(n) && n > 0 ? n : fallback;
  };
  return { view, theme, w: num("w", 420), h: num("h", 300) };
}

export interface AwayAgentRow {
  kind: string;
  title: string;
  /** Auto-replies sent to this session since away mode started. */
  count: number;
}

export const AWAY = {
  scope: "所有 Agent",
  since: "13:09",
  elapsed: "1 小时 23 分",
  total: 6,
  pending: 1,
  agents: [
    { kind: "Codex", title: "修复登录页 Safari 样式错位", count: 4 },
    { kind: "Claude Code", title: "重构登录会话存储", count: 2 },
  ] as AwayAgentRow[],
};

export type MenuNode =
  | {
      t: "item";
      label: string;
      enabled?: boolean;
      hover?: boolean;
      chev?: boolean;
      check?: boolean;
      glyph?: "moon";
    }
  | { t: "sep" };

const item = (label: string, extra: Partial<Extract<MenuNode, { t: "item" }>> = {}): MenuNode => ({
  t: "item",
  label,
  ...extra,
});
const sep = (): MenuNode => ({ t: "sep" });

/** Status block shared by every tray state (mirrors the real read-only rows). */
const statusBlock = (): MenuNode[] => [
  item("AskHuman — 运行中", { enabled: false }),
  item("版本 0.16.0", { enabled: false }),
  item("已运行 3 小时 12 分", { enabled: false }),
  item("渠道：飞书", { enabled: false }),
];

/** Everything below the away group; identical in both states except the pending row. */
const restBlock = (withPending: boolean): MenuNode[] => [
  ...(withPending ? [item("1 个待答", { chev: true }), sep()] : []),
  item("设置"),
  item("历史记录"),
  item("待办"),
  sep(),
  item("打开 Agent 状态窗口"),
  item("工作中 2 · 空闲 1", { chev: true }),
  item("新建 Agent 任务"),
  sep(),
  item("检查更新"),
  sep(),
  item("重启 daemon"),
  item("有 Agent 工作中，Daemon 将保持激活", { enabled: false }),
  item("退出菜单栏图标"),
];

export const trayOn = (): MenuNode[] => [
  ...statusBlock(),
  sep(),
  item("离开模式开启中 · 已 1 小时 23 分", { enabled: false, glyph: "moon" }),
  item("结束离开模式", { hover: true }),
  sep(),
  ...restBlock(true),
];

export const trayOff = (): MenuNode[] => [
  ...statusBlock(),
  sep(),
  item("开启离开模式", { hover: true }),
  sep(),
  ...restBlock(false),
];

export const agentOverview = (): MenuNode[] => [
  item("工作中 · [3] Codex — 修复登录页 Safari 样式错位（HumanInLoop）", { chev: true, hover: true }),
  item("工作中 · [5] Claude Code — 重构登录会话存储（HumanInLoop）", { chev: true }),
  item("空闲 · [2] Cursor — 补充 watch 跟底重发单测（HumanInLoop）", { chev: true }),
];

export const agentSubmenu = (): MenuNode[] => [
  item("在控制台查看"),
  item("Fork 会话"),
  item("要求提问"),
  item("发送消息…"),
  item("添加待办…"),
  item("聚焦终端"),
  sep(),
  item("离开模式（此会话）", { hover: true }),
  item("不再使用 AskHuman（此会话）"),
];
