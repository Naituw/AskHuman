// Design round for guiding the iOS App channel in the setup wizard (docs/specs/setup-ios-guidance.md):
// the real `SetupView` on its remote page in a plain browser with a mock backend, where the iOS tile
// is the prototype tile (`IosTileProto.vue`) and the page's lead and footer read as they would once
// iOS counts as a remote channel.
//
// `?scene=` picks a state (see setupIosScenes.ts): off | agent | incomplete | stopped | connecting |
// await | ready | ready-two | issue | im-ready, or `all` for every state of the tile in one column.
// `?theme=dark`, `?lang=en`.
import { createApp } from "vue";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import "../styles/tokens.css";
import "../styles/base.css";
import "../styles/controls.css";
import "../views/settings/settings.css";
import "../views/setup/setup.css";
import "./setupIos.css";
import { i18n, applyLanguage } from "../i18n";
import { applyTheme } from "../lib/theme";
import type { IosGatewayState } from "../lib/types";
import { channelRow, env, row, settings, status } from "../views/setup/setup.testkit";
import { SCENES } from "./setupIosScenes";

// A prototype that fails should say why on the page rather than stay blank.
function showError(message: string) {
  const box = document.createElement("pre");
  box.style.cssText =
    "position:fixed;left:8px;right:8px;bottom:8px;margin:0;padding:8px;background:#fee;color:#900;font:11px monospace;white-space:pre-wrap;z-index:99";
  box.textContent = message;
  document.body.appendChild(box);
}
window.addEventListener("error", (event) => showError(String(event.error?.stack ?? event.message)));
window.addEventListener("unhandledrejection", (event) =>
  showError(String((event.reason as Error | undefined)?.stack ?? event.reason)),
);

const query = new URLSearchParams(location.search);
const sceneId = query.get("scene") ?? "off";
const theme = query.get("theme") === "dark" ? "dark" : "light";
const lang = query.get("lang") === "en" ? "en" : "zh";
const scene = SCENES[sceneId] ?? SCENES.off;
const facts = scene.facts;

// The wording this proposal adds or changes (the keys live under `setup.remote`).
i18n.global.mergeLocaleMessage("zh", {
  setup: {
    remote: {
      lead: "离开电脑时，也能在手机上收到 Agent 的提问并回复，IM 或 iOS App 均可。选一个你常用的配置，也可以先跳过。",
      iosTile: {
        needsItems: "MQTT Broker 账号 · iPhone（iOS 17+，通过 TestFlight 安装）",
        cap: {
          broker: "注册服务商账号并填入",
          app: "通过 TestFlight 安装",
          pair: "扫码并核对确认码",
        },
        await: "待配对手机",
        paired: "已就绪 · {n} 台手机",
        continue: "继续设置",
        manage: "管理",
        view: "查看",
      },
    },
  },
});
i18n.global.mergeLocaleMessage("en", {
  setup: {
    remote: {
      lead: "When you're away, answer an Agent's question on your phone, in an IM app or the iOS app. Set up the one you use most, or skip for now.",
      iosTile: {
        needsItems: "an MQTT broker account · an iPhone (iOS 17+, installed via TestFlight)",
        cap: {
          broker: "Create an account",
          app: "Install via TestFlight",
          pair: "Scan, compare codes",
        },
        await: "Waiting for a phone",
        paired: "Ready · {n} phone | Ready · {n} phones",
        continue: "Continue setup",
        manage: "Manage",
        view: "View",
      },
    },
  },
});

document.documentElement.classList.add("macos");
applyTheme(theme);
applyLanguage(lang);
// The window can be dragged down to 560 x 600; `?w=` and `?h=` show the page at another size.
const frame = document.getElementById("app");
const width = Number(query.get("w"));
const height = Number(query.get("h"));
if (frame && width >= 480) frame.style.width = `${width}px`;
if (frame && height >= 480) frame.style.height = `${height}px`;

if (sceneId === "all") {
  document.body.classList.add("si-gallery");
  void import("./IosTileGallery.vue").then(({ default: Gallery }) => {
    createApp(Gallery).use(i18n).mount("#app");
  });
} else {
  if (query.get("page") !== "remote") {
    query.set("page", "remote");
    history.replaceState(null, "", `${location.pathname}?${query.toString()}`);
  }

  const gateway: IosGatewayState | null = facts.gateway
    ? {
        connected: facts.gateway.connected,
        lastError: facts.gateway.lastError,
        macId: "mac-1",
        macName: "MacBook Pro",
        pendingRequests: 0,
        phones: facts.phones.map((phone, index) => ({
          phoneId: `phone-${index}`,
          name: phone.name,
          state: "active",
          online: true,
          foreground: false,
          createdAt: Date.now() - 86_400_000,
          lastSeenAt: Date.now() - 30_000,
        })),
      }
    : null;

  // The proposal counts an enabled iOS channel as "remote decided" (D3), so the page behaves as it would.
  const remoteDecided = facts.enabled || scene.imReady === true;

  void mockWindows("setup");
  mockIPC(
    (cmd) => {
      switch (cmd) {
        case "get_settings": {
          const payload = settings({ ios: facts.enabled, secretsPresent: { iosBrokerPassword: facts.passwordFilled } });
          payload.config.general.theme = theme;
          payload.config.general.language = lang;
          if (scene.imReady) {
            (payload.config.channels.telegram as { enabled: boolean }).enabled = true;
          }
          return payload;
        }
        case "setup_status":
          return status({ agentsDecided: true, remoteDecided });
        case "setup_agents_scan":
          return [
            row({ id: "claude", title: "Claude Code", mode: "cli" }),
            row({ id: "cursor", title: "Cursor", mode: "cli" }),
          ];
        case "setup_env_check":
          return env();
        case "channel_health":
          return [];
        case "setup_channel_status":
          return ["telegram", "slack", "dingding", "feishu"].map((id) =>
            id === "telegram" && scene.imReady
              ? channelRow(id, { enabled: true, configured: true, connected: true })
              : channelRow(id),
          );
        case "ios_op":
          return { kind: "state", state: gateway };
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );

  void Promise.all([import("../views/SetupView.vue"), import("./IosTileProto.vue")]).then(
    ([{ default: SetupView }, { default: Tile }]) => {
      const app = createApp(SetupView).use(i18n);
      app.config.errorHandler = (error) => showError(String((error as Error)?.stack ?? error));
      app.mount("#app");
      // Put the prototype tile after the page's own iOS block once the remote page has rendered.
      const timer = window.setInterval(() => {
        const real = document.querySelector<HTMLElement>(
          '.channel-grid [data-testid="channel-row-ios"]:not(.iost)',
        );
        if (!real) return;
        window.clearInterval(timer);
        real.style.display = "none";
        const staging = document.createElement("div");
        createApp(Tile, { facts, agentOpen: scene.agentOpen === true })
          .use(i18n)
          .mount(staging);
        real.after(...(Array.from(staging.children) as HTMLElement[]));
        if (remoteDecided) {
          // The footer button and the skip note as they read once iOS counts (spec §5).
          const skip = document.querySelector<HTMLElement>('[data-testid="remote-skip"]');
          if (skip) {
            skip.textContent = lang === "en" ? "Continue" : "继续";
            skip.classList.add("btn-primary");
          }
          document.querySelector('[data-testid="remote-skip-note"]')?.remove();
        }
      }, 50);
    },
  );
}
