<script setup lang="ts">
import { onMounted, ref } from "vue";
import {
  applyWindowEffect,
  cursorHookInstall,
  cursorHookReveal,
  cursorHookStatus,
  cursorHookUninstall,
  dingtalkDetectPrepare,
  dingtalkDetectWait,
  dingtalkTest,
  getPrompt,
  getSettings,
  openTestPopup,
  saveSettings,
  setTheme,
  telegramTest,
} from "../lib/ipc";
import { applyTheme } from "../lib/theme";
import { isGlassSupported } from "tauri-plugin-liquid-glass-api";
import type {
  AppConfig,
  HookStatus,
  PopupAnimation,
  ThemeMode,
  WindowEffect,
} from "../lib/types";

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

async function changeAnimation(anim: PopupAnimation) {
  if (!config.value) return;
  config.value.general.appearAnimation = anim;
  await persist();
}

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
    dingtalkMessage.value = `已识别并填入 UserId：${userId}`;
  } catch (e) {
    dingtalkMessage.value = String(e);
    dingtalkError.value = true;
  } finally {
    dingtalkDetecting.value = false;
    dingtalkDetectCode.value = null;
  }
}

onMounted(async () => {
  config.value = await getSettings();
  applyTheme(config.value.general.theme);
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
        通用
      </button>
      <button
        data-tauri-drag-region
        :class="{ active: activeTab === 'integration' }"
        @mousedown="onTabDown"
        @click="onTabClick('integration', $event)"
      >
        集成
      </button>
      <button
        data-tauri-drag-region
        :class="{ active: activeTab === 'channel' }"
        @mousedown="onTabDown"
        @click="onTabClick('channel', $event)"
      >
        通信渠道
      </button>
    </nav>

    <div class="settings-body">
      <!-- 通用 -->
      <template v-if="activeTab === 'general'">
        <div class="card">
          <p class="card-title">外观</p>
          <div class="row">
            <span class="label">主题</span>
            <span class="spacer"></span>
            <div class="segmented">
              <button
                :class="{ active: config.general.theme === 'system' }"
                @click="changeTheme('system')"
              >
                跟随系统
              </button>
              <button
                :class="{ active: config.general.theme === 'light' }"
                @click="changeTheme('light')"
              >
                浅色
              </button>
              <button
                :class="{ active: config.general.theme === 'dark' }"
                @click="changeTheme('dark')"
              >
                深色
              </button>
            </div>
          </div>
        </div>

        <div class="card">
          <p class="card-title">弹窗行为</p>
          <div class="row">
            <span class="label">窗口置顶</span>
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
              <span class="label">窗口效果</span>
              <span class="spacer"></span>
              <div class="segmented">
                <button
                  :class="{ active: config.general.windowEffect === 'glass' }"
                  @click="changeWindowEffect('glass')"
                >
                  玻璃
                </button>
                <button
                  :class="{ active: config.general.windowEffect === 'blur' }"
                  @click="changeWindowEffect('blur')"
                >
                  模糊
                </button>
              </div>
            </div>
          </template>
          <template v-if="isMac">
            <hr class="divider" />
            <div class="row">
              <span class="label">弹出动画</span>
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
            <span class="label">弹出测试窗口</span>
            <span class="spacer"></span>
            <button class="btn" type="button" @click="openTestPopup">
              测试
            </button>
          </div>
        </div>
      </template>

      <!-- 集成 -->
      <template v-else-if="activeTab === 'integration'">
        <div class="card">
          <div class="row">
            <p class="card-title">参考提示词</p>
            <span class="spacer"></span>
            <button class="btn" type="button" @click="copyPrompt">
              {{ promptCopied ? "已复制" : "复制" }}
            </button>
          </div>
          <p class="card-desc">
            把以下提示词加入你的 AI 助手，引导它通过 AskHuman 与你交互。
          </p>
          <pre class="code-area">{{ prompt }}</pre>
        </div>

        <div class="card">
          <div class="row">
            <p class="card-title">Cursor Hook</p>
            <span class="spacer"></span>
            <span class="badge">
              <span class="dot" :class="hook.installed ? 'on' : 'off'"></span>
              {{ hook.installed ? "已安装" : "未安装" }}
            </span>
          </div>
          <p class="card-desc">
            安装后会在 ~/.cursor/hooks.json 注册 preToolUse 钩子：检测到 Shell 调用
            AskHuman 时自动把超时延长到 24 小时，避免长时间等待被强制取消。移除时仅删除本应用注入的条目。
          </p>
          <div class="row">
            <button
              v-if="hook.installed"
              class="btn"
              type="button"
              :disabled="!hook.supported"
              @click="uninstallHook"
            >
              移除
            </button>
            <button
              v-else
              class="btn"
              type="button"
              :disabled="!hook.supported"
              @click="installHook"
            >
              安装
            </button>
            <button
              class="btn"
              type="button"
              :disabled="!hook.hooksJsonExists"
              @click="cursorHookReveal"
            >
              打开 hooks.json
            </button>
            <span class="spacer"></span>
          </div>
          <p v-if="!hook.supported" class="result err">
            Windows 暂不支持 Cursor Hook
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
            <p class="card-title">本地弹窗</p>
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
              <span class="label">记住窗口尺寸</span>
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
              <span class="label">默认宽度</span>
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
              <span class="label">默认高度</span>
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
            <p class="card-title">Telegram</p>
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
              <label>Bot Token</label>
              <input
                class="input"
                v-model="config.channels.telegram.botToken"
                @change="persist"
              />
            </div>
            <div class="field">
              <label>Chat ID</label>
              <input
                class="input"
                v-model="config.channels.telegram.chatId"
                @change="persist"
              />
            </div>
            <div class="field">
              <label>API Base URL</label>
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
                {{ telegramTesting ? "测试中…" : "测试连接" }}
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
            <p class="card-title">钉钉</p>
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
              <label>ClientId（AppKey）</label>
              <input
                class="input"
                v-model="config.channels.dingding.clientId"
                @change="persist"
              />
            </div>
            <div class="field">
              <label>ClientSecret（AppSecret）</label>
              <input
                class="input"
                type="password"
                v-model="config.channels.dingding.clientSecret"
                @change="persist"
              />
            </div>
            <div class="field">
              <label>UserId</label>
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
                  {{ dingtalkDetecting ? "识别中…" : "自动识别" }}
                </button>
              </div>
            </div>
            <p v-if="dingtalkDetectCode" class="result ok">
              请用目标钉钉账号私聊机器人发送：<b>{{ dingtalkDetectCode }}</b
              >（120 秒内有效）
            </p>
            <div class="row">
              <button
                class="btn"
                type="button"
                :disabled="dingtalkTesting"
                @click="runDingtalkTest"
              >
                {{ dingtalkTesting ? "测试中…" : "测试连接" }}
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
          </template>
        </div>

        <div class="card">
          <p class="card-desc" style="margin: 0">更多通信 Channel 敬请期待</p>
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
</style>
