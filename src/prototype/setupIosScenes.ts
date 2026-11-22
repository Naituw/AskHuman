// Prototype-only scenes (`?scene=`) for the wizard's iOS App tile (docs/specs/setup-ios-guidance.md).
// A scene is the set of facts the real tile would read: the saved iOS settings, the gateway's answer
// and the paired phones. Everything else on the page is the real wizard.

export interface TileFacts {
  enabled: boolean;
  host: string;
  username: string;
  /** Only whether a password is stored; the prototype never keeps one. */
  passwordFilled: boolean;
  /** `null`: the gateway is not running (or not asked). */
  gateway: { connected: boolean; lastError?: string } | null;
  phones: { name: string }[];
}

export interface Scene {
  facts: TileFacts;
  /** The prompt for an Agent is already open. */
  agentOpen?: boolean;
  /** Telegram is set up as well, so the page's footer reads "Continue" for an IM reason. */
  imReady?: boolean;
  /** Caption of the scene in the gallery. */
  caption: string;
}

const HOST = "x1a2b3c4.ala.cn-hangzhou.emqxsl.cn";

const OFF: TileFacts = {
  enabled: false,
  host: "",
  username: "",
  passwordFilled: false,
  gateway: null,
  phones: [],
};

const CONNECTED: TileFacts = {
  enabled: true,
  host: HOST,
  username: "alice",
  passwordFilled: true,
  gateway: { connected: true },
  phones: [],
};

export const SCENES: Record<string, Scene> = {
  off: { facts: OFF, caption: "未配置" },
  agent: { facts: OFF, agentOpen: true, caption: "未配置 · 已展开「让 Agent 帮我配置」" },
  incomplete: {
    facts: { ...OFF, enabled: true },
    caption: "配置不完整（启用了但缺 Broker 信息）",
  },
  stopped: {
    facts: { ...CONNECTED, enabled: false, gateway: null },
    caption: "已配置（未启用）",
  },
  connecting: {
    facts: { ...CONNECTED, gateway: { connected: false } },
    caption: "已配置，Broker 还没连上（无报错）",
  },
  await: { facts: CONNECTED, caption: "Broker 已连接，待配对手机" },
  ready: {
    facts: { ...CONNECTED, phones: [{ name: "张三的 iPhone" }] },
    caption: "已就绪 · 1 台手机",
  },
  "ready-two": {
    facts: { ...CONNECTED, phones: [{ name: "张三的 iPhone" }, { name: "张三的 iPad" }] },
    caption: "已就绪 · 2 台手机",
  },
  issue: {
    facts: {
      ...CONNECTED,
      gateway: { connected: false, lastError: "authentication failed" },
    },
    caption: "异常（Broker 拒绝了用户名或密码）",
  },
  "im-ready": {
    facts: { ...CONNECTED, phones: [{ name: "张三的 iPhone" }] },
    imReady: true,
    caption: "IM 和 iOS App 都已就绪",
  },
};

/** The scenes the gallery shows, in the order of a person's journey. */
export const GALLERY = ["off", "await", "ready", "issue", "stopped", "incomplete"] as const;
