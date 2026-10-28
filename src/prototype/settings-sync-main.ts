// Settings preview focused on the todo storage card: the real `SettingsView` in a plain browser
// with a mock backend. `?sync=` takes the scenarios of the todos-window preview (todosWindowMock.ts);
// `?theme=dark` and `?lang=en` switch the look. The tab is `experimental` unless `?tab=` says otherwise.
import { createApp } from "vue";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import "../styles/tokens.css";
import "../styles/base.css";
import "../styles/controls.css";
import { i18n, applyLanguage } from "../i18n";
import { applyTheme } from "../lib/theme";
import { readParams, syncStatus } from "./todosWindowMock";

// The default config as `AskHuman config show` prints it (flat), nested here.
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
general.language = auto
general.menuBarIcon = always
general.popupPrewarm = true
general.popupSound =
general.popupSubmitKey = cmdEnter
general.speechLanguage = auto
general.speechShortcut = cmd+d
general.theme = system
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

const params = readParams();
const query = new URLSearchParams(location.search);
const lang = query.get("lang") === "en" ? "en" : "zh";
if (!query.get("tab")) {
  history.replaceState(null, "", `${location.pathname}?${query.toString()}${query.toString() ? "&" : ""}tab=experimental`);
}

// Enabling and disabling flip a local flag so the card can be clicked through.
let enabled = params.sync !== "off" && params.sync !== "none";
function status() {
  return { ...syncStatus(params.sync === "none" ? "none" : enabled ? (params.sync === "off" ? "ok" : params.sync) : "off") };
}

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
      case "todo_sync_status":
      case "todo_sync_now":
      case "todo_sync_resume":
      case "todo_sync_restart_from_local":
      case "todo_sync_clear_cloud":
        return status();
      case "todo_sync_enable":
        enabled = true;
        return status();
      case "todo_sync_disable":
        enabled = false;
        return status();
      case "todo_sync_repair":
        return { ok: false, needsApproval: true, message: "requires approval" };
      case "ios_op":
        return { kind: "state", state: null };
      default:
        return null;
    }
  },
  { shouldMockEvents: true },
);

document.documentElement.classList.add("macos");
applyTheme(params.theme);
applyLanguage(lang);

void import("../views/SettingsView.vue").then(({ default: SettingsView }) => {
  createApp(SettingsView).use(i18n).mount("#app");
});
