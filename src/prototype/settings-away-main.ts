// Settings preview for the away-mode / quick-reply cards: the real `SettingsView` in a plain browser
// with a mock backend, plus two prototype cards injected right after the "Reply history" card.
// `?theme=dark` switches the look. Everything but the new cards can be hidden with `?focus=1`.
import { createApp } from "vue";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import "../styles/tokens.css";
import "../styles/base.css";
import "../styles/controls.css";
import "./settingsAway.css";
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
  return root;
}

const query = new URLSearchParams(location.search);
const theme = query.get("theme") === "dark" ? "dark" : "light";
if (!query.get("tab")) {
  history.replaceState(null, "", `${location.pathname}?${query.toString()}${query.toString() ? "&" : ""}tab=general`);
}
if (query.get("focus")) document.documentElement.classList.add("sa-focus");

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

void Promise.all([import("../views/SettingsView.vue"), import("./SettingsAwayCards.vue")]).then(
  ([{ default: SettingsView }, { default: Cards }]) => {
    createApp(SettingsView).use(i18n).mount("#app");
    // Insert the prototype cards after the "回复历史" card once the tab has rendered.
    const timer = window.setInterval(() => {
      const cards = Array.from(document.querySelectorAll<HTMLElement>(".settings-body > .card"));
      const anchor = cards.find((c) => c.querySelector(".card-title")?.textContent?.trim() === "回复历史");
      if (!anchor) return;
      window.clearInterval(timer);
      anchor.classList.add("sa-ctx");
      // Mount off-screen, then move the cards in so `.card + .card` spacing keeps working.
      const staging = document.createElement("div");
      createApp(Cards).use(i18n).mount(staging);
      anchor.after(...Array.from(staging.children));
    }, 50);
  },
);
