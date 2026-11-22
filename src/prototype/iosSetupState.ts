// Prototype-only state of the redesigned iOS App card and the scenes (`?scene=`) that preset it.
// The real card reads the same facts from the daemon's gateway state and the saved configuration.
import { reactive } from "vue";

export type Provider = "emqx" | "flespi" | "custom";

export interface Phone {
  name: string;
  online: boolean;
  meta: string;
}

export interface SetupState {
  enabled: boolean;
  provider: Provider | null;
  host: string;
  port: number;
  wsPort: number;
  username: string;
  /** Only whether a password is typed or saved; the prototype never keeps one. */
  password: boolean;
  testing: boolean;
  /** The last "Test connection": idle until run. A connected gateway counts as `ok` too. */
  tested: "idle" | "ok";
  keepAlive: boolean;
  /** "I installed the app" clicked (the Mac cannot see the phone before pairing). */
  appAck: boolean;
  pairing: null | "waiting" | "pending";
  phones: Phone[];
  tryState: "idle" | "sent" | "answered";
  agentOpen: boolean;
  /** The finished guide, opened again by hand. */
  stepsOpen: boolean;
  /** A step the person opened or closed by hand: 0 closes all, null follows the current step. */
  open: number | null;
  copied: string;
  alertAfter: number;
  agentTasks: boolean;
}

/** Shown before the EMQX steps (so it is read before signing up) and again under a finished setup. */
export const EMQX_NOTICE =
  "需要注意：如果免费额度（每月 100 万连接分钟、1 GB 流量）用完，最低充值金额可能需要 ¥100。建议关注流量使用，尽量保持在免费额度内。";

export const PROVIDERS: Record<Provider, { name: string; sub: string }> = {
  emqx: { name: "EMQX Cloud", sub: "国内节点 · 免费额度" },
  flespi: { name: "flespi", sub: "欧洲节点 · 免费套餐" },
  custom: { name: "自建或其它", sub: "如 Mosquitto" },
};

const EMQX_HOST = "x1a2b3c4.ala.cn-hangzhou.emqxsl.cn";

export function createState(scene: string): SetupState {
  const s = reactive<SetupState>({
    enabled: false,
    provider: null,
    host: "",
    port: 8883,
    wsPort: 8084,
    username: "",
    password: false,
    testing: false,
    tested: "idle",
    keepAlive: false,
    appAck: false,
    pairing: null,
    phones: [],
    tryState: "idle",
    agentOpen: false,
    stepsOpen: false,
    open: null,
    copied: "",
    alertAfter: 900,
    agentTasks: false,
  });
  const connected = () => {
    s.provider = "emqx";
    s.host = EMQX_HOST;
    s.username = "alice";
    s.password = true;
    s.tested = "ok";
  };
  const phone: Phone = { name: "Tian 的 iPhone", online: true, meta: "在线" };
  switch (scene) {
    case "agent":
      s.agentOpen = true;
      break;
    case "broker":
      s.enabled = true;
      break;
    case "emqx":
      s.enabled = true;
      s.provider = "emqx";
      break;
    case "emqx-filled":
      s.enabled = true;
      connected();
      s.open = 1;
      break;
    case "flespi":
      s.enabled = true;
      s.provider = "flespi";
      s.host = "mqtt.flespi.io";
      s.wsPort = 443;
      break;
    case "custom":
      s.enabled = true;
      s.provider = "custom";
      break;
    case "app":
      s.enabled = true;
      connected();
      s.keepAlive = true;
      break;
    case "pair-start":
      s.enabled = true;
      connected();
      s.keepAlive = true;
      s.appAck = true;
      break;
    case "pair":
      s.enabled = true;
      connected();
      s.keepAlive = true;
      s.appAck = true;
      s.pairing = "waiting";
      break;
    case "confirm":
      s.enabled = true;
      connected();
      s.keepAlive = true;
      s.appAck = true;
      s.pairing = "pending";
      break;
    case "try":
    case "try-sent":
      s.enabled = true;
      connected();
      s.keepAlive = true;
      s.appAck = true;
      s.phones = [phone];
      s.tryState = scene === "try-sent" ? "sent" : "idle";
      break;
    case "steps":
      // A finished setup with the guide opened again by hand.
      s.enabled = true;
      connected();
      s.keepAlive = true;
      s.appAck = true;
      s.phones = [phone];
      s.stepsOpen = true;
      break;
    case "ready":
      s.enabled = true;
      connected();
      s.keepAlive = true;
      s.appAck = true;
      s.phones = [phone, { name: "Tian 的 iPad", online: false, meta: "最近在线 2026/11/20 21:14" }];
      s.tryState = "answered";
      break;
    default:
      break;
  }
  return s;
}
