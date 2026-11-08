// 「通信渠道」域：渠道健康横幅（R7）、配置指南外链、各渠道连接测试与自动识别、
// 弹窗尺寸步进。
import { onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import {
  channelHealth,
  detectCancel,
  dingtalkDetectPrepare,
  dingtalkDetectWait,
  dingtalkTest,
  feishuDetectPrepare,
  feishuDetectWait,
  feishuTest,
  openPath,
  setupChannelStatus,
  slackDetectPrepare,
  slackDetectWait,
  slackTest,
  telegramTest,
} from "../../lib/ipc";
import type { ChannelIssue, SetupChannelStatus } from "../../lib/types";
import {
  channelDef,
  channelSection,
  channelState,
  requiredFieldsFilled,
  type ChannelId,
  type ChannelState,
} from "../setup/channelDefs";
import type { SettingsCore } from "./context";

export function useChannels(core: SettingsCore) {
  const { t, locale } = useI18n();
  const { config, activeTab, persist } = core;

  // ===== 渠道健康（R7 渠道故障可见化）=====
  // daemon 侧渠道故障快照：渠道 tab 可见期间拉取并轻量轮询（修好后横幅自动消失——
  // daemon 在配置变更 / 下一次成功操作后即清除记录）。同一轮询也取各渠道的实时连接状态，
  // 供渠道卡标题旁的状态标签使用。
  const channelIssues = ref<ChannelIssue[]>([]);
  const channelLive = ref<SetupChannelStatus[]>([]);
  let channelHealthTimer: number | undefined;

  async function refreshChannelHealth() {
    // The two reads are independent: a failing one only loses its own half of the picture.
    const [issues, live] = await Promise.all([
      channelHealth().catch(() => [] as ChannelIssue[]),
      setupChannelStatus().catch(() => [] as SetupChannelStatus[]),
    ]);
    channelIssues.value = issues;
    channelLive.value = live;
  }

  /**
   * What a channel card's status tag says: the saved settings (switch on, every required field
   * filled), whether the daemon holds a live connection for it, and its last unrecovered error.
   * `connected` is unknown (not "false") while no daemon runs, which reads as "enabled".
   */
  function channelCardState(id: ChannelId): ChannelState {
    const def = channelDef(id);
    const current = config.value;
    if (!def || !current) return "unconfigured";
    const enabled = channelSection(current, id).enabled === true;
    const configured = requiredFieldsFilled(def, current, core.secretsPresent.value);
    const connected = channelLive.value.find((row) => row.id === id)?.connected ?? null;
    return channelState({ enabled, configured, connected }, channelIssueText(id) !== null);
  }

  /** 渠道卡错误横幅文案；无故障返回 null（不渲染）。 */
  function channelIssueText(id: string): string | null {
    const issue = channelIssues.value.find((i) => i.channel === id);
    if (!issue) return null;
    return t("settings.channels.issueBanner", {
      time: issueAgo(issue.atMs),
      msg: issue.message,
    });
  }

  // 渠道配置指南（仓库内 wiki 文档；中文默认 .md，英文 .en.md）。
  const CHANNEL_SETUP_DOCS: Record<string, string> = {
    telegram: "telegram-setup",
    dingding: "dingtalk-setup",
    feishu: "feishu-setup",
    slack: "slack-setup",
  };

  function openChannelGuide(id: string) {
    const slug = CHANNEL_SETUP_DOCS[id];
    if (!slug) return;
    const suffix = String(locale.value).startsWith("zh") ? ".md" : ".en.md";
    void openPath(
      `https://github.com/Naituw/AskHuman/blob/main/docs/wiki/${slug}${suffix}`,
    );
  }

  function issueAgo(atMs: number): string {
    const diff = Math.max(0, Math.floor((Date.now() - atMs) / 1000));
    if (diff < 60) return t("agents.time.justNow");
    const min = Math.floor(diff / 60);
    if (min < 60) return t("agents.time.minutesAgo", { n: min });
    const hr = Math.floor(min / 60);
    if (hr < 24) return t("agents.time.hoursAgo", { n: hr });
    return t("agents.time.daysAgo", { n: Math.floor(hr / 24) });
  }

  watch(
    activeTab,
    (tab) => {
      if (channelHealthTimer) {
        window.clearInterval(channelHealthTimer);
        channelHealthTimer = undefined;
      }
      if (tab === "channel") {
        void refreshChannelHealth();
        channelHealthTimer = window.setInterval(
          () => void refreshChannelHealth(),
          10_000,
        );
      }
    },
    { immediate: true },
  );
  onBeforeUnmount(() => {
    if (channelHealthTimer) window.clearInterval(channelHealthTimer);
  });

  function clamp(v: number, min: number, max: number) {
    return Math.min(max, Math.max(min, v));
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

  const telegramTesting = ref(false);
  const telegramMessage = ref<string | null>(null);
  const telegramError = ref(false);

  // 「自动识别」取消标记（三家共用）：点了取消后，等待会被中止并 reject，catch 据此走中性
  // 收尾而非报错。
  const detectCancelled = ref(false);

  // 取消正在进行的自动识别（三家共用）：置标记 + 通知后端中止等待（连带释放临时长连接）。
  async function cancelDetect() {
    detectCancelled.value = true;
    try {
      await detectCancel();
    } catch {
      /* 取消本身失败可忽略：等待仍会按既有路径超时收尾 */
    }
  }

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

  const slackTesting = ref(false);
  const slackDetecting = ref(false);
  const slackDetectCode = ref<string | null>(null);
  const slackMessage = ref<string | null>(null);
  const slackError = ref(false);

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
    detectCancelled.value = false;
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
      if (detectCancelled.value) {
        dingtalkMessage.value = null;
        dingtalkError.value = false;
      } else {
        dingtalkMessage.value = String(e);
        dingtalkError.value = true;
      }
    } finally {
      dingtalkDetecting.value = false;
      dingtalkDetectCode.value = null;
      detectCancelled.value = false;
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
    detectCancelled.value = false;
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
      if (detectCancelled.value) {
        feishuMessage.value = null;
        feishuError.value = false;
      } else {
        feishuMessage.value = String(e);
        feishuError.value = true;
      }
    } finally {
      feishuDetecting.value = false;
      feishuDetectCode.value = null;
      detectCancelled.value = false;
    }
  }

  async function runSlackTest() {
    if (!config.value) return;
    slackTesting.value = true;
    slackMessage.value = null;
    const sl = config.value.channels.slack;
    try {
      slackMessage.value = await slackTest({
        botToken: sl.botToken,
        appToken: sl.appToken,
        userId: sl.userId,
      });
      slackError.value = false;
    } catch (e) {
      slackMessage.value = String(e);
      slackError.value = true;
    } finally {
      slackTesting.value = false;
    }
  }

  // 自动识别：先校验并取识别码 → 展示提示 → 等用户私聊发送该码 → 回填 userId。
  async function runSlackDetect() {
    if (!config.value) return;
    const sl = config.value.channels.slack;
    slackDetecting.value = true;
    detectCancelled.value = false;
    slackMessage.value = null;
    slackDetectCode.value = null;
    try {
      const code = await slackDetectPrepare({
        botToken: sl.botToken,
        appToken: sl.appToken,
      });
      slackDetectCode.value = code;
      const userId = await slackDetectWait({
        botToken: sl.botToken,
        appToken: sl.appToken,
        code,
      });
      sl.userId = userId;
      await persist();
      slackError.value = false;
      slackMessage.value = t("settings.channels.slackDetected", { userId });
    } catch (e) {
      if (detectCancelled.value) {
        slackMessage.value = null;
        slackError.value = false;
      } else {
        slackMessage.value = String(e);
        slackError.value = true;
      }
    } finally {
      slackDetecting.value = false;
      slackDetectCode.value = null;
      detectCancelled.value = false;
    }
  }

  return {
    channelIssueText,
    channelCardState,
    openChannelGuide,
    stepWidth,
    stepHeight,
    cancelDetect,
    telegramTesting,
    telegramMessage,
    telegramError,
    runTelegramTest,
    dingtalkTesting,
    dingtalkDetecting,
    dingtalkDetectCode,
    dingtalkMessage,
    dingtalkError,
    runDingtalkTest,
    runDingtalkDetect,
    feishuTesting,
    feishuDetecting,
    feishuDetectCode,
    feishuMessage,
    feishuError,
    runFeishuTest,
    runFeishuDetect,
    slackTesting,
    slackDetecting,
    slackDetectCode,
    slackMessage,
    slackError,
    runSlackTest,
    runSlackDetect,
  };
}
