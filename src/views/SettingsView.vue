<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { applyLanguage } from "../i18n";
import {
  applyWindowEffect,
  cursorHookInstall,
  cursorHookReveal,
  cursorHookStatus,
  cursorHookUninstall,
  dingtalkDetectPrepare,
  dingtalkDetectWait,
  dingtalkTest,
  feishuDetectPrepare,
  feishuDetectWait,
  feishuTest,
  getPrompt,
  getSettings,
  openTestPopup,
  saveSettings,
  setTheme,
  telegramTest,
} from "../lib/ipc";
import { applyTheme } from "../lib/theme";
import {
  eventToSpec,
  formatShortcut,
  isModifierOnly,
  shortcutConflict,
  specToString,
  type ConflictReason,
} from "../lib/shortcut";
import { isGlassSupported } from "tauri-plugin-liquid-glass-api";
import type {
  AppConfig,
  HookStatus,
  PopupAnimation,
  ThemeMode,
  UiLanguage,
  WindowEffect,
} from "../lib/types";

const { t } = useI18n();

// 出现动画为 macOS 原生窗口能力，其它平台不展示选择器。
const isMac = navigator.userAgent.toLowerCase().includes("mac");

type Tab = "general" | "integration" | "channel";

const config = ref<AppConfig | null>(null);
const activeTab = ref<Tab>("general");

// tab 按钮同时是窗口拖拽区：用屏幕坐标区分「点击切换」与「拖动移窗」。
// 原生拖窗时窗口跟随光标，clientX/Y 几乎不变，故必须用 screenX/Y。
const tabDown = ref<{ x: number; y: number } | null>(null);
function onTabDown(e: MouseEvent) {
  tabDown.value = { x: e.screenX, y: e.screenY };
}
function onTabClick(tab: Tab, e: MouseEvent) {
  const d = tabDown.value;
  tabDown.value = null;
  if (d && Math.hypot(e.screenX - d.x, e.screenY - d.y) > 4) return;
  activeTab.value = tab;
}
const prompt = ref("");
const promptCopied = ref(false);

const hook = ref<HookStatus>({
  installed: false,
  hooksJsonExists: false,
  supported: true,
});
const hookMessage = ref<string | null>(null);
const hookError = ref(false);

const telegramTesting = ref(false);
const telegramMessage = ref<string | null>(null);
const telegramError = ref(false);

const dingtalkTesting = ref(false);
const dingtalkDetecting = ref(false);
const dingtalkDetectCode = ref<string | null>(null);
const dingtalkMessage = ref<string | null>(null);
const dingtalkError = ref(false);

const feishuTesting = ref(false);
const feishuDetecting = ref(false);
const feishuDetectCode = ref<string | null>(null);
const feishuMessage = ref<string | null>(null);
const feishuError = ref(false);

function clamp(v: number, min: number, max: number) {
  return Math.min(max, Math.max(min, v));
}

// 是否支持 Liquid Glass（macOS 26+）：决定「玻璃/模糊」开关是否显示。
const glassSupported = ref(true);

async function persist() {
  if (config.value) await saveSettings(config.value);
}

async function changeTheme(theme: ThemeMode) {
  if (!config.value) return;
  config.value.general.theme = theme;
  applyTheme(theme);
  await setTheme(theme);
  await persist();
}

// 切换界面语言：本窗口立即生效；persist 广播 settings-updated 令其它窗口同步。
async function changeLanguage(lang: UiLanguage) {
  if (!config.value) return;
  config.value.general.language = lang;
  applyLanguage(lang);
  await persist();
}

// 其它窗口改了语言时，本窗口也同步切换。
let unlistenSettings: UnlistenFn | null = null;
onBeforeUnmount(() => unlistenSettings?.());

async function changeAnimation(anim: PopupAnimation) {
  if (!config.value) return;
  config.value.general.appearAnimation = anim;
  await persist();
}

// 语音识别语言下拉项：第一项「跟随系统」(auto) + 常用语言（BCP-47）。
const SPEECH_LANGUAGES: { value: string; label: string }[] = [
  // auto 的显示文案在模板里走 i18n（settings.speech.languageSystem）。
  { value: "auto", label: "" },
  { value: "zh-CN", label: "简体中文" },
  { value: "zh-TW", label: "繁体中文" },
  { value: "en-US", label: "English (US)" },
  { value: "ja-JP", label: "日本語" },
  { value: "ko-KR", label: "한국어" },
];

async function changeSpeechLanguage(lang: string) {
  if (!config.value) return;
  config.value.general.speechLanguage = lang;
  await persist();
}

// 语音快捷键录制：点一下进入录制，直接按组合键录入；Esc 取消。
const recordingShortcut = ref(false);
const shortcutPreview = ref("");
const shortcutError = ref<ConflictReason | null>(null);
let shortcutHandler: ((e: KeyboardEvent) => void) | null = null;

function previewModifiers(e: KeyboardEvent): string {
  let o = "";
  if (e.ctrlKey) o += "⌃";
  if (e.altKey) o += "⌥";
  if (e.shiftKey) o += "⇧";
  if (e.metaKey) o += "⌘";
  return o ? o + "…" : "";
}

function stopRecordShortcut() {
  recordingShortcut.value = false;
  shortcutPreview.value = "";
  if (shortcutHandler) {
    window.removeEventListener("keydown", shortcutHandler, true);
    shortcutHandler = null;
  }
}

function startRecordShortcut() {
  if (recordingShortcut.value) return;
  recordingShortcut.value = true;
  shortcutError.value = null;
  shortcutPreview.value = "";
  shortcutHandler = (e: KeyboardEvent) => {
    // 捕获阶段拦截，避免触发浏览器/窗口默认行为（如 ⌘W 关窗）。
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      stopRecordShortcut();
      return;
    }
    if (isModifierOnly(e)) {
      shortcutPreview.value = previewModifiers(e);
      return;
    }
    const spec = eventToSpec(e);
    if (!spec) return;
    const reason = shortcutConflict(spec);
    if (reason) {
      shortcutError.value = reason;
      return;
    }
    if (config.value) {
      config.value.general.speechShortcut = specToString(spec);
      persist();
    }
    stopRecordShortcut();
  };
  window.addEventListener("keydown", shortcutHandler, true);
}

function clearShortcut() {
  if (!config.value) return;
  config.value.general.speechShortcut = "";
  shortcutError.value = null;
  persist();
  stopRecordShortcut();
}

onBeforeUnmount(stopRecordShortcut);

// 切换弹窗背景效果（仅 macOS 26+ 显示）。持久化后实时作用于已打开的 popup + 设置窗口。
async function changeWindowEffect(effect: WindowEffect) {
  if (!config.value) return;
  config.value.general.windowEffect = effect;
  await persist();
  try {
    // 后端同时切换 popup 与 settings 两个窗口（玻璃用插件、模糊用 set_effects）。
    await applyWindowEffect(effect);
  } catch (e) {
    console.error("切换窗口效果失败", e);
  }
}

function stepWidth(delta: number) {
  if (!config.value) return;
  config.value.channels.popup.width = clamp(
    config.value.channels.popup.width + delta,
    360,
    1200
  );
  persist();
}

function stepHeight(delta: number) {
  if (!config.value) return;
  config.value.channels.popup.height = clamp(
    config.value.channels.popup.height + delta,
    360,
    1400
  );
  persist();
}

async function refreshHook() {
  hook.value = await cursorHookStatus();
}

async function installHook() {
  try {
    hookMessage.value = await cursorHookInstall();
    hookError.value = false;
  } catch (e) {
    hookMessage.value = String(e);
    hookError.value = true;
  }
  await refreshHook();
}

async function uninstallHook() {
  try {
    hookMessage.value = await cursorHookUninstall();
    hookError.value = false;
  } catch (e) {
    hookMessage.value = String(e);
    hookError.value = true;
  }
  await refreshHook();
}

async function copyPrompt() {
  try {
    await navigator.clipboard.writeText(prompt.value);
  } catch {
    /* 忽略：剪贴板不可用时静默 */
  }
  promptCopied.value = true;
  setTimeout(() => (promptCopied.value = false), 1500);
}

async function runTelegramTest() {
  if (!config.value) return;
  telegramTesting.value = true;
  telegramMessage.value = null;
  const tg = config.value.channels.telegram;
  try {
    telegramMessage.value = await telegramTest({
      botToken: tg.botToken,
      chatId: tg.chatId,
      apiBaseUrl: tg.apiBaseUrl,
    });
    telegramError.value = false;
  } catch (e) {
    telegramMessage.value = String(e);
    telegramError.value = true;
  } finally {
    telegramTesting.value = false;
  }
}

async function runDingtalkTest() {
  if (!config.value) return;
  dingtalkTesting.value = true;
  dingtalkMessage.value = null;
  const dd = config.value.channels.dingding;
  try {
    dingtalkMessage.value = await dingtalkTest({
      clientId: dd.clientId,
      clientSecret: dd.clientSecret,
      userId: dd.userId,
    });
    dingtalkError.value = false;
  } catch (e) {
    dingtalkMessage.value = String(e);
    dingtalkError.value = true;
  } finally {
    dingtalkTesting.value = false;
  }
}

// 自动识别：先校验并取识别码 → 展示提示 → 等用户私聊发送该码 → 回填 userId。
async function runDingtalkDetect() {
  if (!config.value) return;
  const dd = config.value.channels.dingding;
  dingtalkDetecting.value = true;
  dingtalkMessage.value = null;
  dingtalkDetectCode.value = null;
  try {
    const code = await dingtalkDetectPrepare({
      clientId: dd.clientId,
      clientSecret: dd.clientSecret,
    });
    dingtalkDetectCode.value = code;
    const userId = await dingtalkDetectWait({
      clientId: dd.clientId,
      clientSecret: dd.clientSecret,
      code,
    });
    dd.userId = userId;
    await persist();
    dingtalkError.value = false;
    dingtalkMessage.value = t("settings.channels.detected", { userId });
  } catch (e) {
    dingtalkMessage.value = String(e);
    dingtalkError.value = true;
  } finally {
    dingtalkDetecting.value = false;
    dingtalkDetectCode.value = null;
  }
}

async function runFeishuTest() {
  if (!config.value) return;
  feishuTesting.value = true;
  feishuMessage.value = null;
  const fs = config.value.channels.feishu;
  try {
    feishuMessage.value = await feishuTest({
      appId: fs.appId,
      appSecret: fs.appSecret,
      openId: fs.openId,
      baseUrl: fs.baseUrl,
    });
    feishuError.value = false;
  } catch (e) {
    feishuMessage.value = String(e);
    feishuError.value = true;
  } finally {
    feishuTesting.value = false;
  }
}

// 自动识别：先校验并取识别码 → 展示提示 → 等用户私聊发送该码 → 回填 openId。
async function runFeishuDetect() {
  if (!config.value) return;
  const fs = config.value.channels.feishu;
  feishuDetecting.value = true;
  feishuMessage.value = null;
  feishuDetectCode.value = null;
  try {
    const code = await feishuDetectPrepare({
      appId: fs.appId,
      appSecret: fs.appSecret,
      baseUrl: fs.baseUrl,
    });
    feishuDetectCode.value = code;
    const openId = await feishuDetectWait({
      appId: fs.appId,
      appSecret: fs.appSecret,
      baseUrl: fs.baseUrl,
      code,
    });
    fs.openId = openId;
    await persist();
    feishuError.value = false;
    feishuMessage.value = t("settings.channels.feishuDetected", { openId });
  } catch (e) {
    feishuMessage.value = String(e);
    feishuError.value = true;
  } finally {
    feishuDetecting.value = false;
    feishuDetectCode.value = null;
  }
}

onMounted(async () => {
  config.value = await getSettings();
  applyTheme(config.value.general.theme);
  applyLanguage(config.value.general.language);
  unlistenSettings = await listen<{ language?: UiLanguage }>(
    "settings-updated",
    (e) => {
      if (e.payload.language) applyLanguage(e.payload.language);
    }
  );
  prompt.value = await getPrompt();
  await refreshHook();
  if (isMac) {
    try {
      glassSupported.value = await isGlassSupported();
    } catch {
      glassSupported.value = false;
    }
  }
});
</script>

<template>
  <div v-if="config" class="settings">
    <nav class="tabbar" data-tauri-drag-region>
      <button
        data-tauri-drag-region
        :class="{ active: activeTab === 'general' }"
        @mousedown="onTabDown"
        @click="onTabClick('general', $event)"
      >
        {{ t("settings.tabs.general") }}
      </button>
      <button
        data-tauri-drag-region
        :class="{ active: activeTab === 'integration' }"
        @mousedown="onTabDown"
        @click="onTabClick('integration', $event)"
      >
        {{ t("settings.tabs.integration") }}
      </button>
      <button
        data-tauri-drag-region
        :class="{ active: activeTab === 'channel' }"
        @mousedown="onTabDown"
        @click="onTabClick('channel', $event)"
      >
        {{ t("settings.tabs.channel") }}
      </button>
    </nav>

    <div class="settings-body">
      <!-- 通用 -->
      <template v-if="activeTab === 'general'">
        <div class="card">
          <p class="card-title">{{ t("settings.appearance.title") }}</p>
          <div class="row">
            <span class="label">{{ t("settings.appearance.theme") }}</span>
            <span class="spacer"></span>
            <div class="segmented">
              <button
                :class="{ active: config.general.theme === 'system' }"
                @click="changeTheme('system')"
              >
                {{ t("settings.appearance.themeSystem") }}
              </button>
              <button
                :class="{ active: config.general.theme === 'light' }"
                @click="changeTheme('light')"
              >
                {{ t("settings.appearance.themeLight") }}
              </button>
              <button
                :class="{ active: config.general.theme === 'dark' }"
                @click="changeTheme('dark')"
              >
                {{ t("settings.appearance.themeDark") }}
              </button>
            </div>
          </div>
          <hr class="divider" />
          <div class="row">
            <span class="label">{{ t("settings.appearance.language") }}</span>
            <span class="spacer"></span>
            <select
              class="select"
              :value="config.general.language"
              @change="changeLanguage(($event.target as HTMLSelectElement).value as UiLanguage)"
            >
              <option value="auto">
                {{ t("settings.appearance.languageSystem") }}
              </option>
              <option value="en">English</option>
              <option value="zh">简体中文</option>
            </select>
          </div>
        </div>

        <div class="card">
          <p class="card-title">{{ t("settings.popupBehavior.title") }}</p>
          <div class="row">
            <span class="label">{{ t("settings.popupBehavior.alwaysOnTop") }}</span>
            <span class="spacer"></span>
            <label class="switch">
              <input
                type="checkbox"
                v-model="config.general.alwaysOnTop"
                @change="persist"
              />
              <span class="track"></span>
            </label>
          </div>
          <template v-if="isMac && glassSupported">
            <hr class="divider" />
            <div class="row">
              <span class="label">{{
                t("settings.popupBehavior.windowEffect")
              }}</span>
              <span class="spacer"></span>
              <div class="segmented">
                <button
                  :class="{ active: config.general.windowEffect === 'glass' }"
                  @click="changeWindowEffect('glass')"
                >
                  {{ t("settings.popupBehavior.effectGlass") }}
                </button>
                <button
                  :class="{ active: config.general.windowEffect === 'blur' }"
                  @click="changeWindowEffect('blur')"
                >
                  {{ t("settings.popupBehavior.effectBlur") }}
                </button>
              </div>
            </div>
          </template>
          <template v-if="isMac">
            <hr class="divider" />
            <div class="row">
              <span class="label">{{
                t("settings.popupBehavior.appearAnimation")
              }}</span>
              <span class="spacer"></span>
              <div class="segmented">
                <button
                  :class="{ active: config.general.appearAnimation === 'none' }"
                  @click="changeAnimation('none')"
                >
                  None
                </button>
                <button
                  :class="{
                    active: config.general.appearAnimation === 'document',
                  }"
                  @click="changeAnimation('document')"
                >
                  Document
                </button>
                <button
                  :class="{ active: config.general.appearAnimation === 'alert' }"
                  @click="changeAnimation('alert')"
                >
                  Alert
                </button>
              </div>
            </div>
          </template>
          <hr class="divider" />
          <div class="row">
            <span class="label">{{ t("settings.popupBehavior.testPopup") }}</span>
            <span class="spacer"></span>
            <button class="btn" type="button" @click="openTestPopup">
              {{ t("common.test") }}
            </button>
          </div>
        </div>

        <!-- 语音输入（仅 macOS） -->
        <div v-if="isMac" class="card">
          <p class="card-title">{{ t("settings.speech.title") }}</p>
          <div class="row">
            <span class="label">{{ t("settings.speech.language") }}</span>
            <span class="spacer"></span>
            <select
              class="select"
              :value="config.general.speechLanguage"
              @change="changeSpeechLanguage(($event.target as HTMLSelectElement).value)"
            >
              <option
                v-for="lang in SPEECH_LANGUAGES"
                :key="lang.value"
                :value="lang.value"
              >
                {{
                  lang.value === "auto"
                    ? t("settings.speech.languageSystem")
                    : lang.label
                }}
              </option>
            </select>
          </div>
          <hr class="divider" />
          <div class="row">
            <span class="label">{{ t("settings.speech.shortcut") }}</span>
            <span class="spacer"></span>
            <button
              class="btn shortcut-rec"
              :class="{ recording: recordingShortcut }"
              type="button"
              @click="startRecordShortcut"
            >
              {{
                recordingShortcut
                  ? shortcutPreview || t("settings.speech.recording")
                  : config.general.speechShortcut
                  ? formatShortcut(config.general.speechShortcut)
                  : t("shortcut.none")
              }}
            </button>
            <button
              class="btn"
              type="button"
              style="margin-left: 6px"
              :disabled="!config.general.speechShortcut && !recordingShortcut"
              @click="clearShortcut"
            >
              {{ t("settings.speech.clear") }}
            </button>
          </div>
          <p v-if="shortcutError" class="result err">
            {{ t("shortcut.conflict." + shortcutError.key, shortcutError.params || {}) }}
          </p>
          <p
            v-else-if="recordingShortcut"
            class="card-desc"
            style="margin-top: 6px"
          >
            {{ t("settings.speech.recordHint") }}
          </p>
        </div>
      </template>

      <!-- 集成 -->
      <template v-else-if="activeTab === 'integration'">
        <div class="card">
          <div class="row">
            <p class="card-title">{{ t("settings.integration.promptTitle") }}</p>
            <span class="spacer"></span>
            <button class="btn" type="button" @click="copyPrompt">
              {{
                promptCopied
                  ? t("settings.integration.copied")
                  : t("settings.integration.copy")
              }}
            </button>
          </div>
          <p class="card-desc">
            {{ t("settings.integration.promptDesc") }}
          </p>
          <pre class="code-area">{{ prompt }}</pre>
        </div>

        <div class="card">
          <div class="row">
            <p class="card-title">{{ t("settings.integration.hookTitle") }}</p>
            <span class="spacer"></span>
            <span class="badge">
              <span class="dot" :class="hook.installed ? 'on' : 'off'"></span>
              {{
                hook.installed
                  ? t("settings.integration.installed")
                  : t("settings.integration.notInstalled")
              }}
            </span>
          </div>
          <p class="card-desc">
            {{ t("settings.integration.hookDesc") }}
          </p>
          <div class="row">
            <button
              v-if="hook.installed"
              class="btn"
              type="button"
              :disabled="!hook.supported"
              @click="uninstallHook"
            >
              {{ t("settings.integration.uninstall") }}
            </button>
            <button
              v-else
              class="btn"
              type="button"
              :disabled="!hook.supported"
              @click="installHook"
            >
              {{ t("settings.integration.install") }}
            </button>
            <button
              class="btn"
              type="button"
              :disabled="!hook.hooksJsonExists"
              @click="cursorHookReveal"
            >
              {{ t("settings.integration.openHooks") }}
            </button>
            <span class="spacer"></span>
          </div>
          <p v-if="!hook.supported" class="result err">
            {{ t("settings.integration.windowsUnsupported") }}
          </p>
          <p
            v-else-if="hookMessage"
            class="result"
            :class="hookError ? 'err' : 'ok'"
          >
            {{ hookMessage }}
          </p>
        </div>
      </template>

      <!-- 通信渠道 -->
      <template v-else>
        <div class="card">
          <div class="row">
            <p class="card-title">{{ t("settings.channels.popupTitle") }}</p>
            <span class="spacer"></span>
            <label class="switch">
              <input
                type="checkbox"
                v-model="config.channels.popup.enabled"
                @change="persist"
              />
              <span class="track"></span>
            </label>
          </div>

          <template v-if="config.channels.popup.enabled">
            <hr class="divider" />
            <div class="row">
              <span class="label">{{ t("settings.channels.rememberSize") }}</span>
              <span class="spacer"></span>
              <label class="switch">
                <input
                  type="checkbox"
                  v-model="config.channels.popup.rememberSize"
                  @change="persist"
                />
                <span class="track"></span>
              </label>
            </div>
            <div class="row">
              <span class="label">{{ t("settings.channels.defaultWidth") }}</span>
              <span class="spacer"></span>
              <div class="stepper">
                <button
                  type="button"
                  :disabled="config.channels.popup.width <= 360"
                  @click="stepWidth(-20)"
                >
                  −
                </button>
                <span class="val">{{ Math.round(config.channels.popup.width) }}</span>
                <button
                  type="button"
                  :disabled="config.channels.popup.width >= 1200"
                  @click="stepWidth(20)"
                >
                  +
                </button>
              </div>
            </div>
            <div class="row">
              <span class="label">{{ t("settings.channels.defaultHeight") }}</span>
              <span class="spacer"></span>
              <div class="stepper">
                <button
                  type="button"
                  :disabled="config.channels.popup.height <= 360"
                  @click="stepHeight(-20)"
                >
                  −
                </button>
                <span class="val">{{
                  Math.round(config.channels.popup.height)
                }}</span>
                <button
                  type="button"
                  :disabled="config.channels.popup.height >= 1400"
                  @click="stepHeight(20)"
                >
                  +
                </button>
              </div>
            </div>
          </template>
        </div>

        <div class="card">
          <div class="row">
            <p class="card-title">{{ t("settings.channels.telegramTitle") }}</p>
            <span class="spacer"></span>
            <label class="switch">
              <input
                type="checkbox"
                v-model="config.channels.telegram.enabled"
                @change="persist"
              />
              <span class="track"></span>
            </label>
          </div>

          <template v-if="config.channels.telegram.enabled">
            <hr class="divider" />
            <div class="field">
              <label>{{ t("settings.channels.botToken") }}</label>
              <input
                class="input"
                v-model="config.channels.telegram.botToken"
                @change="persist"
              />
            </div>
            <div class="field">
              <label>{{ t("settings.channels.chatId") }}</label>
              <input
                class="input"
                v-model="config.channels.telegram.chatId"
                @change="persist"
              />
            </div>
            <div class="field">
              <label>{{ t("settings.channels.apiBaseUrl") }}</label>
              <input
                class="input"
                v-model="config.channels.telegram.apiBaseUrl"
                @change="persist"
              />
            </div>
            <div class="row">
              <button
                class="btn"
                type="button"
                :disabled="telegramTesting"
                @click="runTelegramTest"
              >
                {{
                  telegramTesting
                    ? t("settings.channels.testing")
                    : t("settings.channels.testConnection")
                }}
              </button>
              <span class="spacer"></span>
            </div>
            <p
              v-if="telegramMessage"
              class="result"
              :class="telegramError ? 'err' : 'ok'"
            >
              {{ telegramMessage }}
            </p>
          </template>
        </div>

        <div class="card">
          <div class="row">
            <p class="card-title">{{ t("settings.channels.dingtalkTitle") }}</p>
            <span class="spacer"></span>
            <label class="switch">
              <input
                type="checkbox"
                v-model="config.channels.dingding.enabled"
                @change="persist"
              />
              <span class="track"></span>
            </label>
          </div>

          <template v-if="config.channels.dingding.enabled">
            <hr class="divider" />
            <div class="field">
              <label>{{ t("settings.channels.clientId") }}</label>
              <input
                class="input"
                v-model="config.channels.dingding.clientId"
                @change="persist"
              />
            </div>
            <div class="field">
              <label>{{ t("settings.channels.clientSecret") }}</label>
              <input
                class="input"
                type="password"
                v-model="config.channels.dingding.clientSecret"
                @change="persist"
              />
            </div>
            <div class="field">
              <label>{{ t("settings.channels.userId") }}</label>
              <div class="row">
                <input
                  class="input"
                  style="flex: 1"
                  v-model="config.channels.dingding.userId"
                  @change="persist"
                />
                <button
                  class="btn"
                  type="button"
                  :disabled="dingtalkDetecting"
                  @click="runDingtalkDetect"
                >
                  {{
                    dingtalkDetecting
                      ? t("settings.channels.detecting")
                      : t("settings.channels.autoDetect")
                  }}
                </button>
              </div>
            </div>
            <i18n-t
              v-if="dingtalkDetectCode"
              keypath="settings.channels.detectHint"
              tag="p"
              class="result ok"
            >
              <template #code><b>{{ dingtalkDetectCode }}</b></template>
            </i18n-t>
            <div class="row">
              <button
                class="btn"
                type="button"
                :disabled="dingtalkTesting"
                @click="runDingtalkTest"
              >
                {{
                  dingtalkTesting
                    ? t("settings.channels.testing")
                    : t("settings.channels.testConnection")
                }}
              </button>
              <span class="spacer"></span>
            </div>
            <p
              v-if="dingtalkMessage"
              class="result"
              :class="dingtalkError ? 'err' : 'ok'"
            >
              {{ dingtalkMessage }}
            </p>
            <hr class="divider" />
            <div class="field">
              <label>{{ t("settings.channels.cardTemplateId") }}</label>
              <input
                class="input"
                v-model="config.channels.dingding.cardTemplateId"
                :placeholder="t('settings.channels.cardTemplateIdPlaceholder')"
                @change="persist"
              />
            </div>
            <hr class="divider" />
            <div class="row">
              <label>{{ t("settings.channels.inlineSmallText") }}</label>
              <span class="spacer"></span>
              <label class="switch">
                <input
                  type="checkbox"
                  v-model="config.channels.dingding.inlineSmallText"
                  @change="persist"
                />
                <span class="track"></span>
              </label>
            </div>
            <p class="card-desc" style="margin-top: 0">
              {{ t("settings.channels.inlineSmallTextHint") }}
            </p>
            <div class="row">
              <label>{{ t("settings.channels.convertTextToDocx") }}</label>
              <span class="spacer"></span>
              <label class="switch">
                <input
                  type="checkbox"
                  v-model="config.channels.dingding.convertTextToDocx"
                  @change="persist"
                />
                <span class="track"></span>
              </label>
            </div>
            <p class="card-desc" style="margin-top: 0">
              {{ t("settings.channels.convertTextToDocxHint") }}
            </p>
          </template>
        </div>

        <div class="card">
          <div class="row">
            <p class="card-title">{{ t("settings.channels.feishuTitle") }}</p>
            <span class="spacer"></span>
            <label class="switch">
              <input
                type="checkbox"
                v-model="config.channels.feishu.enabled"
                @change="persist"
              />
              <span class="track"></span>
            </label>
          </div>

          <template v-if="config.channels.feishu.enabled">
            <hr class="divider" />
            <div class="field">
              <label>{{ t("settings.channels.appId") }}</label>
              <input
                class="input"
                v-model="config.channels.feishu.appId"
                @change="persist"
              />
            </div>
            <div class="field">
              <label>{{ t("settings.channels.appSecret") }}</label>
              <input
                class="input"
                type="password"
                v-model="config.channels.feishu.appSecret"
                @change="persist"
              />
            </div>
            <div class="field">
              <label>{{ t("settings.channels.openId") }}</label>
              <div class="row">
                <input
                  class="input"
                  style="flex: 1"
                  v-model="config.channels.feishu.openId"
                  @change="persist"
                />
                <button
                  class="btn"
                  type="button"
                  :disabled="feishuDetecting"
                  @click="runFeishuDetect"
                >
                  {{
                    feishuDetecting
                      ? t("settings.channels.detecting")
                      : t("settings.channels.autoDetect")
                  }}
                </button>
              </div>
            </div>
            <i18n-t
              v-if="feishuDetectCode"
              keypath="settings.channels.feishuDetectHint"
              tag="p"
              class="result ok"
            >
              <template #code><b>{{ feishuDetectCode }}</b></template>
            </i18n-t>
            <div class="field">
              <label>{{ t("settings.channels.feishuBaseUrl") }}</label>
              <input
                class="input"
                v-model="config.channels.feishu.baseUrl"
                :placeholder="t('settings.channels.feishuBaseUrlPlaceholder')"
                @change="persist"
              />
            </div>
            <div class="row">
              <button
                class="btn"
                type="button"
                :disabled="feishuTesting"
                @click="runFeishuTest"
              >
                {{
                  feishuTesting
                    ? t("settings.channels.testing")
                    : t("settings.channels.testConnection")
                }}
              </button>
              <span class="spacer"></span>
            </div>
            <p
              v-if="feishuMessage"
              class="result"
              :class="feishuError ? 'err' : 'ok'"
            >
              {{ feishuMessage }}
            </p>
          </template>
        </div>

        <div class="card">
          <p class="card-desc" style="margin: 0">
            {{ t("settings.channels.moreSoon") }}
          </p>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.settings {
  position: relative;
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.settings-body {
  flex: 1 1 auto;
  overflow-y: auto;
  padding: var(--space-4);
}
/* 快捷键录制按钮：等宽、最小宽度，录制态用强调色描边 */
.shortcut-rec {
  min-width: 76px;
  font-variant-numeric: tabular-nums;
}
.shortcut-rec.recording {
  border-color: var(--accent);
  color: var(--accent);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 30%, transparent);
}
</style>
