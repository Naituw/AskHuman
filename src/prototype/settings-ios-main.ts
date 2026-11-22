// Design round for the iOS App setup experience (docs/specs/ios-setup-experience.md): the real
// `SettingsView` on the Channels tab in a plain browser with a mock backend, and the redesigned iOS
// card (prototype-only, `IosSetupProto.vue`) inserted where it would live once it graduates out of
// the Experimental tab.
//
// `?scene=` picks a state (see IosSetupProto.vue): off | agent | broker | emqx | emqx-filled | flespi |
// custom | app | pair-start | pair | confirm | try | try-sent | steps | ready. `?theme=dark`, `?focus=1` (hide the
// other cards), `?fold=0` (never fold the provider instructions), `?w=` (window width in px).
// The card sits right below the local popup card and above the "按需发送" tip (the owner's call).
import { createApp } from "vue";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import "../styles/tokens.css";
import "../styles/base.css";
import "../styles/controls.css";
import "./settingsIos.css";
import { i18n, applyLanguage } from "../i18n";
import { applyTheme } from "../lib/theme";

const DEFAULTS = `
agentTasks.enabled = false
agentTasks.permissionPrompt = ask
channels.autoActivation = false
channels.autoEndWatch = true
channels.dingding.cardTemplateId =
channels.dingding.clientId =
channels.dingding.clientSecret =
channels.dingding.confirmCardTemplateId =
channels.dingding.convertTextToDocx = true
channels.dingding.enabled = false
channels.dingding.inlineSmallText = true
channels.dingding.permissionConfirmCardTemplateId =
channels.dingding.userId =
channels.feishu.appId =
channels.feishu.appSecret =
channels.feishu.baseUrl = https://open.feishu.cn
channels.feishu.enabled = false
channels.feishu.openId =
channels.ios.alertAfterSecs = 900
channels.ios.enabled = false
channels.ios.host =
channels.ios.macName =
channels.ios.password =
channels.ios.port = 8883
channels.ios.relayUrl =
channels.ios.username =
channels.ios.wsPort = 8084
channels.popup.enabled = true
channels.popup.height = 620
channels.popup.previewWidth = 700
channels.popup.rememberSize = true
channels.popup.sidebarWidth = 240
channels.popup.width = 560
channels.popup.windowMode = merged
channels.slack.appToken =
channels.slack.botToken =
channels.slack.enabled = false
channels.slack.userId =
channels.telegram.apiBaseUrl = https://api.telegram.org
channels.telegram.botToken =
channels.telegram.chatId =
channels.telegram.enabled = false
codexDesktop.appPath =
codexDesktop.codexHome =
codexDesktop.launchPreference = desktop
cursorDesktop.suppressBackgroundCompletionFollowUp = false
experimental.enabled = false
experimental.verticalQuestions = true
general.alwaysOnTop = true
general.appearAnimation = alert
general.collaborationStyle = aligned
general.collaborationStyleCustomText =
general.daemonLifecycle = activity
general.historyLimit = 200
general.language = zh
general.menuBarIcon = always
general.popupPrewarm = true
general.popupSound =
general.popupSubmitKey = cmdEnter
general.speechLanguage = auto
general.speechShortcut = cmd+d
general.theme = light
general.todoHistoryLimit = 100
general.windowEffect = blur
permissions.codexRelaxedShell = false
todoSync.deviceName =
todoSync.pollMinutes = 10
`;

function defaultConfig(): Record<string, unknown> {
  const root: Record<string, unknown> = {};
  // The settings window applies the saved theme itself, so `?theme=` has to be set in the config.
  const wanted = new URLSearchParams(location.search).get("theme") === "dark" ? "dark" : "light";
  for (const line of DEFAULTS.split("\n")) {
    const match = /^(\S+) = ?(.*)$/.exec(line.trim());
    if (!match) continue;
    const [, path, raw] = match;
    const value: unknown =
      raw === "true" ? true : raw === "false" ? false : raw !== "" && !isNaN(Number(raw)) ? Number(raw) : raw;
    const keys = path.split(".");
    let node = root;
    for (const key of keys.slice(0, -1)) node = (node[key] ??= {}) as Record<string, unknown>;
    node[keys[keys.length - 1]] = value;
  }
  (root.general as Record<string, unknown>).theme = wanted;
  return root;
}

const query = new URLSearchParams(location.search);
const theme = query.get("theme") === "dark" ? "dark" : "light";
if (!query.get("tab")) {
  history.replaceState(null, "", `${location.pathname}?${query.toString()}${query.toString() ? "&" : ""}tab=channel`);
}
if (query.get("focus")) document.documentElement.classList.add("si-focus");
const width = Number(query.get("w"));
if (width >= 480) document.body.style.width = `${width}px`;

void mockWindows("settings");
mockIPC(
  (cmd) => {
    switch (cmd) {
      case "get_settings":
        return {
          config: defaultConfig(),
          secretsPresent: {
            dingdingSecret: false, feishuSecret: false, telegramToken: false,
            slackBotToken: false, slackAppToken: false, iosBrokerPassword: false,
          },
        };
      case "channel_health":
      case "setup_channel_status":
        return [];
      case "ios_op":
        return { kind: "state", state: null };
      default:
        return null;
    }
  },
  { shouldMockEvents: true },
);

document.documentElement.classList.add("macos");
applyTheme(theme);
applyLanguage("zh");

void Promise.all([import("../views/SettingsView.vue"), import("./IosSetupProto.vue")]).then(
  ([{ default: SettingsView }, { default: Card }]) => {
    createApp(SettingsView).use(i18n).mount("#app");
    // Insert the prototype card once the Channels tab has rendered its IM cards.
    const timer = window.setInterval(() => {
      const cards = Array.from(document.querySelectorAll<HTMLElement>(".settings-body > .card"));
      const popup = cards[0];
      const telegram = cards.find((c) => c.id === "channel-telegram");
      if (!popup || !telegram) return;
      window.clearInterval(timer);
      // Mount off-screen, then move the card in so `.card + .card` spacing keeps working. In the DOM it
      // follows the popup card, ahead of the tip; the IM cards come later through CSS `order` (10+).
      const staging = document.createElement("div");
      createApp(Card).use(i18n).mount(staging);
      popup.after(...(Array.from(staging.children) as HTMLElement[]));
    }, 50);
  },
);
