<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  popupInit,
  submitPopup,
  submitConfirmAction,
  confirmPopupReady,
  cancelPopup,
  openSettings,
  openHistory,
  updateTheme,
  openPath,
  previewAttachments,
  closePreview,
  readImageDataUrl,
  fileIconDataUrl,
  showAttachmentMenu,
  startSpeech,
  stopSpeech,
  flushSpeech,
  speechAvailable,
  popupUpdateState,
  updateApply,
  updateGetNotes,
  focusAgentTerminal,
  popupAgentTerminal,
  popupAgentResolved,
  popupShowWindow,
  popupImTipVisible,
  popupImTipDismiss,
} from "../lib/ipc";
import { isFocusableTerminal } from "../lib/terminals";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { formatShortcut, matchShortcut } from "../lib/shortcut";
import { applyLanguage } from "../i18n";
import { renderMarkdown, handleCodeCopyClick } from "../lib/markdown";
import { applyTheme, fileToDataUrl } from "../lib/theme";
import { mark as perfMarkFe, enable as perfEnableFe } from "../lib/perf";
import type {
  AskRequest,
  ConfirmRequest,
  FileAttachment,
  ImageAttachment,
  PopupInit,
  Question,
  QuestionAnswer,
  ThemeMode,
} from "../lib/types";

const { t } = useI18n();

// Localized labels for the markdown code-block copy button. Referencing t()
// inside a computed keeps the rendered markdown reactive to language changes.
const codeCopyLabels = computed(() => ({
  copyLabel: t("common.copyCode"),
  copiedLabel: t("common.copied"),
}));

const request = ref<AskRequest | null>(null);
const confirmRequest = ref<ConfirmRequest | null>(null);
const isConfirm = computed(() => confirmRequest.value !== null);
const confirmChoiceIndex = ref<number | null>(null);
const confirmComment = ref("");
const showConfirmCloseWarning = ref(false);
const loadError = ref<string | null>(null);

// 视图态：false=Markdown 预览（默认），true=源码（原始文本）。作用于整篇（message + 所有问题）。
const viewSource = ref(false);
// 复制 message 反馈（短暂显示对勾）。
const copiedMessage = ref(false);
let copiedTimer: number | undefined;

async function copyMessage() {
  try {
    await navigator.clipboard.writeText(messageText.value);
  } catch {
    /* 剪贴板不可用：静默忽略 */
  }
  copiedMessage.value = true;
  if (copiedTimer) window.clearTimeout(copiedTimer);
  copiedTimer = window.setTimeout(() => (copiedMessage.value = false), 1500);
}

// ===== 版本自更新（弹窗入口 / 浮层 / 待生效横条） =====
const updateAvailable = ref(false);
const updatePending = ref(false);
const updateLatest = ref("");
const updatePopoverOpen = ref(false);
const updating = ref(false);
const updateStarted = ref(false);
const updateError = ref("");
const updateNotesHtml = ref("");

async function toggleUpdatePopover() {
  updatePopoverOpen.value = !updatePopoverOpen.value;
  if (updatePopoverOpen.value && !updateNotesHtml.value) {
    try {
      const notes = await updateGetNotes(false);
      updateNotesHtml.value = notes.trim()
        ? renderMarkdown(notes, codeCopyLabels.value)
        : "";
    } catch {
      updateNotesHtml.value = "";
    }
  }
}

async function applyUpdateFromPopup() {
  if (updating.value || updateStarted.value) return;
  updating.value = true;
  updateError.value = "";
  try {
    await updateApply();
    updateStarted.value = true;
  } catch (e) {
    const s = String(e);
    updateError.value = /rate-limited|\b403\b|\b429\b/i.test(s)
      ? t("popup.update.rateLimited")
      : `${t("popup.update.failed")}: ${s}`;
  } finally {
    updating.value = false;
  }
}

// 当前展示的问题索引（0 始）。
const current = ref(0);

// 按题保存的作答状态（长度与问题数一致）。
const chosenByQ = ref<string[][]>([]);
const inputByQ = ref<string[]>([]);
const imagesByQ = ref<ImageAttachment[][]>([]);
const replyFilesByQ = ref<{ path: string; name: string }[][]>([]);
// 每题是否已被「查看过」。
const visited = ref<boolean[]>([]);

// 语音输入：macOS 26 原生 SpeechAnalyzer（经 Swift 桥 + Tauri 事件）。
// 仅 macOS 26+ 可用；后端 speech_available 判定，否则隐藏麦克风按钮。
const speechSupported = ref(false);
// listening：会话已激活（含 loading 与录制中）。speechReady：真正进入实时录制（高亮）。
const listening = ref(false);
const speechReady = ref(false);
const speechError = ref<string | null>(null);
const speechStatus = ref<string | null>(null);
// 识别语言（来自设置；auto/空 → 后端按系统首选语言）。
const speechLang = ref("auto");
// 语音输入快捷键（来自设置；空串 = 关闭快捷键，仅麦克风按钮可用）。
const speechShortcut = ref("cmd+d");
const speechHotkeyLabel = computed(() =>
  speechShortcut.value ? formatShortcut(speechShortcut.value) : ""
);
// 插入模型（复刻 demo）：文本布局 = [...已提交...][实时片段]。
// interimStart 指向实时片段起点；interimLen 为其长度。committed 在 interimStart 处永久插入；
// volatile 就地替换 [interimStart, interimStart+interimLen]。用户中途移动光标→固定并 flush。
let speechTargetQ = 0;
let interimStart = 0;
let interimLen = 0;
// 待替换选区（激活时若有选区，延迟到首个识别文字到达才删除，模拟原生听写）。
let pendingSelStart = -1;
let pendingSelEnd = -1;
// 用户按下鼠标拖选期间，暂停把语音更新写进 DOM，避免冲掉正在进行的选区。
let suspendSpeechDom = false;
// 最近一次「已知」的选区（程序化设置或已处理过的用户选择）；据此区分用户的新操作。
let lastSelStart = -1;
let lastSelEnd = -1;
let speechErrorTimer: ReturnType<typeof setTimeout> | null = null;
// speech-* 事件取消订阅句柄。
let unlistenSpeech: UnlistenFn[] = [];

const submitting = ref(false);
// 每题的 textarea（函数 ref 按索引登记）；inputRef = 当前题(active) 的 textarea，
// 供语音 / autoGrow / 聚焦复用既有逻辑（current 即 active 指针）。
const inputRefs = ref<(HTMLTextAreaElement | null)[]>([]);
function setInputRef(el: HTMLTextAreaElement | null, i: number) {
  if (el) inputRefs.value[i] = el;
}
const inputRef = computed<HTMLTextAreaElement | null>(
  () => inputRefs.value[current.value] ?? null
);
const fileRef = ref<HTMLInputElement | null>(null);
// 多问题纵向列表：滚动容器（IntersectionObserver root）+ 每题卡片 + 每题底部哨兵 + 每题缩略图容器。
const contentRef = ref<HTMLElement | null>(null);
const cardRefs = ref<(HTMLElement | null)[]>([]);
function setCardRef(el: HTMLElement | null, i: number) {
  cardRefs.value[i] = el;
}
const sentinelRefs = ref<(HTMLElement | null)[]>([]);
function setSentinelRef(el: HTMLElement | null, i: number) {
  sentinelRefs.value[i] = el;
}
const thumbsRefs = ref<(HTMLElement | null)[]>([]);
function setThumbsRef(el: HTMLElement | null, i: number) {
  thumbsRefs.value[i] = el;
}
// 当前聚焦的问题索引（null = 无）；驱动折叠输入框展开。
const focusedQ = ref<number | null>(null);
// 待归属图片的目标题（「添加图片」按钮点选时设置）。
let pendingPickQ = 0;
// 弹窗刚上屏（原生「出现」动画期）短时吞掉 ⌘W：用户常按 ⌘W 关别的窗口，弹窗恰好此刻弹出会被误关。
const APPEAR_GUARD_MS = 500;
let appearGuardUntil = 0;
// 键盘/按钮 setActive 后短暂锁定，避免随即的滚动事件把 active 改回去。
let activeLockUntil = 0;
// 导航滚动锁时长：需覆盖「聚焦展开(双 rAF) + smooth 滚动动画」的整个过程，否则动画尾段的滚动事件会让
// scroll-spy 把 current 抢到别题（露出题顶=对齐顶部定位时，阅读线并不落在目标题，故必须锁到动画结束）。
const NAV_LOCK_MS = 700;
let io: IntersectionObserver | null = null;
const scrolled = ref(false);
// 内容是否已滚到最顶（scrollTop<=0）：用于纵向模式判断「首题的『上一个』能否再往上露出 message」。
const atTop = ref(true);
// 按住 ⌘/Ctrl 时高亮右侧快捷键 Badge（提示「此刻按数字即可选项」）。
const cmdHeld = ref(false);
// 取消二次确认（已有部分回答时）。
const showCancelConfirm = ref(false);

function onScroll(e: Event) {
  const st = (e.target as HTMLElement).scrollTop;
  scrolled.value = st > 0;
  atTop.value = st <= 0;
  updateActiveFromScroll();
}

// 滚动定位「当前题」：用「比例阅读线」(proportional scroll-spy)——判定线在视口内的纵向位置
// 随滚动进度 p=scrollTop/maxScroll 从顶部线性扫到底部（p=0→视口顶、p=1→视口底）。active =
// 该线当前落在的题（即最后一个 top ≤ 线的题）。如此滚动进度被均匀分配给各题：滚到最顶=第一题、
// 滚到底=末题、中间进度=中间题，**每题都有一段可达区间**（修复「内容仅略超视口时，一滑就从首题
// 跳到末题、中间题选不中」）；且因用真实卡片边界，超长题在其铺满视口期间持续保持 active（高度自适应）。
// 键盘/按钮导航后 450ms 内不被滚动回改（activeLockUntil）。
function readingLineY(root: HTMLElement): number {
  const r = root.getBoundingClientRect();
  const max = root.scrollHeight - root.clientHeight;
  const p = max > 0 ? Math.min(1, Math.max(0, root.scrollTop / max)) : 0;
  return r.top + p * root.clientHeight;
}
function activeForScroll(root: HTMLElement): number {
  const line = readingLineY(root);
  let next = 0;
  for (let i = 0; i < cardRefs.value.length; i++) {
    const el = cardRefs.value[i];
    if (!el) continue;
    if (el.getBoundingClientRect().top <= line) next = i;
  }
  return next;
}
let scrollRaf = 0;
function updateActiveFromScroll() {
  if (!verticalMode.value) return;
  if (scrollRaf) return;
  scrollRaf = requestAnimationFrame(() => {
    scrollRaf = 0;
    if (Date.now() < activeLockUntil) return;
    const root = contentRef.value;
    if (!root) return;
    const next = activeForScroll(root);
    if (next !== current.value) current.value = next;
  });
}

// 建立/重建底部哨兵观察：哨兵进视口 → 该题「已看到」（兼容超长题）。
function setupQuestionObserver() {
  io?.disconnect();
  io = null;
  if (!verticalMode.value) return;
  const root = contentRef.value;
  if (!root) return;
  io = new IntersectionObserver(
    (entries) => {
      for (const en of entries) {
        if (!en.isIntersecting) continue;
        const idx = Number((en.target as HTMLElement).dataset.qSentinel);
        if (!Number.isNaN(idx)) markVisited(idx);
      }
    },
    { root, threshold: 0 }
  );
  for (const el of sentinelRefs.value) if (el) io.observe(el);
}

const pinned = ref(false);
const theme = ref<ThemeMode>("system");
const sourceName = ref("the Loop");
// 来源 workspace：名称用于标题区展示，完整路径用于 hover 提示。空则隐藏该元素。
const projectName = ref("");
const projectPath = ref("");
// 来源 agent：家族标识 + pid + 所在终端类型（决定 badge 是否可点击激活 tab）。
const agentKind = ref("");
const agentPid = ref<number | null>(null);
const agentTerminal = ref<string | null>(null);
// agent badge 文案：本地化家族名（Claude Code / Codex / Cursor）；未知家族回退原始标识。
const agentLabel = computed(() => {
  const k = agentKind.value;
  if (!k) return "";
  const label = t(`agents.kind.${k}`);
  return label === `agents.kind.${k}` ? k : label;
});
// agent badge 是否可点击：所在终端可激活 tab 且有 pid。
const agentFocusable = computed(
  () => !!agentPid.value && isFocusableTerminal(agentTerminal.value)
);

// 点击 agent badge：聚焦该 agent 所在终端的 tab（失败静默，仅日志）。
async function onFocusAgentTerminal() {
  if (!agentFocusable.value || agentPid.value == null) return;
  try {
    await focusAgentTerminal(agentPid.value);
  } catch (err) {
    console.warn("focus agent terminal failed", err);
  }
}

// 点击 workspace badge：在文件管理器打开该目录。
async function onOpenWorkspace() {
  if (!projectPath.value) return;
  try {
    await openPath(projectPath.value);
  } catch (err) {
    console.warn("open workspace failed", err);
  }
}

const questions = computed<Question[]>(() => request.value?.questions ?? []);
const total = computed(() => questions.value.length);
const isMulti = computed(() => total.value > 1);
// 实验开关：多问题是否纵向同时显示（来自 popup_init）。verticalMode = 开关开 且 多问题。
// 关 / 单问题 → 旧版「一次一题 + 上/下一步」（sequential）。
const verticalEnabled = ref(false);
const verticalMode = computed(() => verticalEnabled.value && isMulti.value);
// 严格选择：隐藏补充输入 / 附件区，且必须选中才能提交（D11）。
const selectOnly = computed(() => request.value?.selectOnly ?? false);
// 单选：选项渲染为 radio，每题恰好一个（D11）。
const single = computed(() => request.value?.single ?? false);
const currentQuestion = computed<Question | null>(
  () => questions.value[current.value] ?? null
);
// 旧版（sequential）单题代理：作用于「当前题」，供 v-else 单题面板复用旧模板写法。
const chosen = computed(() => chosenByQ.value[current.value] ?? []);
const userInput = computed<string>({
  get: () => inputByQ.value[current.value] ?? "",
  set: (v) => {
    if (current.value < inputByQ.value.length) inputByQ.value[current.value] = v;
  },
});
const images = computed(() => imagesByQ.value[current.value] ?? []);
const replyFiles = computed(() => replyFilesByQ.value[current.value] ?? []);
const renderedHtml = computed(() =>
  currentQuestion.value ? questionHtml(currentQuestion.value) : ""
);
// 旧版切题左右滑动方向 + 过渡名；「全部看过」用于 sequential 模式显示发送按钮。
const slideDir = ref<"next" | "prev">("next");
const transitionName = computed(() =>
  slideDir.value === "next" ? "q-slide-next" : "q-slide-prev"
);
const allViewed = computed(
  () => visited.value.length > 0 && visited.value.every(Boolean)
);
// 旧版单题面板的头部 ref（切题时滚到顶）。
const qHeaderRef = ref<HTMLElement | null>(null);
// 共享 Message（描述 + 附件）。无 -q 时 text 为空（第一个参数已提升为问题）。
const messageText = computed(() => request.value?.message.text ?? "");
const messageHtml = computed(() =>
  request.value?.isMarkdown && !viewSource.value
    ? renderMarkdown(messageText.value, codeCopyLabels.value)
    : ""
);
const showDescription = computed(
  () => messageText.value.trim() !== "" || attachments.value.length > 0
);
// 存在 Message（描述/附件）或多题时，显示问题头部以区隔 Message 与 Question。
const showQuestionHeader = computed(() => showDescription.value || isMulti.value);
// 顶栏来源头部：默认来源 "the Loop"（human-in-the-loop 固定短语）强制英文；
// 自定义来源跟随界面语言（与后端 i18n::source_header 规则一致）。
const DEFAULT_SOURCE_NAME = "the Loop";
const headerTitle = computed(() => {
  const key = showQuestionHeader.value ? "popup.messageFrom" : "popup.questionFrom";
  const named = { source: sourceName.value };
  return sourceName.value === DEFAULT_SOURCE_NAME
    ? t(key, named, { locale: "en" })
    : t(key, named);
});
// 标题内联 agent 胶囊：探测到 agent 且未定制来源名时，把「the Loop」文字替换为 agent 胶囊本身
// （后端未定制时已把来源名解析为 agent 名，故 sourceName 可能等于 agentLabel；MCP 下后端回退
// "the Loop" 但前端经 AgentResolved 拿到 agentKind，故再判默认值）。
const agentInline = computed(
  () =>
    !!agentLabel.value &&
    (sourceName.value === DEFAULT_SOURCE_NAME ||
      sourceName.value === agentLabel.value)
);
// 内联模式把「Message from {source}」按 {source} 占位切成前后两段，胶囊嵌入中间。
// 文案强制英文（与默认 "the Loop" 一致）：英文 source 在句尾，故前缀「Message from」、后缀为空，
// 渲染为「Message from [Cursor] [Project]」，规避中文「来自 [胶囊] 的消息」夹在句中的观感。
const SOURCE_SLOT = "\u0000";
const headerParts = computed(() => {
  const key = showQuestionHeader.value ? "popup.messageFrom" : "popup.questionFrom";
  const full = t(key, { source: SOURCE_SLOT }, { locale: "en" });
  const idx = full.indexOf(SOURCE_SLOT);
  if (idx < 0) return { prefix: full, suffix: "" };
  // 去掉占位两侧空白，由 .brand 的 flex gap 统一提供胶囊间距，避免空格叠加。
  return {
    prefix: full.slice(0, idx).trimEnd(),
    suffix: full.slice(idx + SOURCE_SLOT.length).trimStart(),
  };
});
// 统一渲染：非内联 → 前缀=完整标题、后缀空；内联 → 拆分后的前后缀（胶囊夹在中间）。
const headerPrefix = computed(() =>
  agentInline.value ? headerParts.value.prefix : headerTitle.value
);
const headerSuffix = computed(() =>
  agentInline.value ? headerParts.value.suffix : ""
);
// 弹窗头部时间：提问创建时刻（epoch ms，daemon 上送；冷/单进程为弹窗构造时刻）。0/缺失=不显示。
const createdAtMs = ref(0);
// 每秒 tick，让相对时间随停留时长自动走字（与请求内容解耦）。
const nowMs = ref(Date.now());
let timeTicker: number | undefined;
// 相对时间：满一天改绝对时间（Q2 甲）。<5s 刚刚 / <60s N 秒前 / <60min N 分钟前 / <24h N 小时前。
const popupTimeRel = computed(() => {
  const created = createdAtMs.value;
  if (!created) return "";
  const diff = Math.max(0, Math.floor((nowMs.value - created) / 1000));
  if (diff < 5) return t("popup.time.justNow");
  if (diff < 60) return t("popup.time.secondsAgo", { n: diff });
  const min = Math.floor(diff / 60);
  if (min < 60) return t("popup.time.minutesAgo", { n: min });
  const hr = Math.floor(min / 60);
  if (hr < 24) return t("popup.time.hoursAgo", { n: hr });
  // 满一天：绝对时间（跟随系统语言/格式）。
  return new Date(created).toLocaleString();
});
// hover 精确绝对时间（title）。
const popupTimeAbs = computed(() =>
  createdAtMs.value ? new Date(createdAtMs.value).toLocaleString() : ""
);
// 多题显示「Question i/n」；单题（仅在有 Message 时显示头部）只显示「Question」。
const questionHeaderLabel = computed(() =>
  isMulti.value
    ? t("popup.question.indexed", { i: current.value + 1, n: total.value })
    : t("popup.question.single")
);
// 「已看到」：卡片底部进视口（IntersectionObserver）或曾被设为当前题（setActive）。
// 多问题发送按钮出现条件 = 最后一题已看到；单问题恒真。
const lastSeen = computed(
  () => !isMulti.value || (visited.value[total.value - 1] ?? false)
);
const hasAnyAnswer = computed(() =>
  questions.value.some((_, i) => isAnswered(i))
);
// 严格选择下「必须选中才能提交」：每个有选项的问题都需至少一个勾选。
const canSubmit = computed(() => {
  if (!selectOnly.value) return true;
  return questions.value.every(
    (_, i) => (chosenByQ.value[i]?.length ?? 0) > 0
  );
});
// 是否处于最后一题：多题时 CMD+回车 仅在最后一题提交，否则前往下一题。
const onLastQuestion = computed(() => current.value === total.value - 1);

// 「上一个」是否可用：纵向模式下即使在首题，只要还没滚到最顶（上方 message 未露全）就可用（点它=露出 message）；
// 旧版顺序模式仍是「非首题才可用」。
const canGoPrev = computed(() =>
  verticalMode.value ? !(current.value === 0 && atTop.value) : current.value > 0
);

// 纵向模式下 ⌘↵ 是否会「提交」：已看完全部 且 当前焦点之后再无未答题（含焦点在末题时恒真）。
// 与 onCmdEnter 的分支完全一致——「谁挂 ⌘↵ = ⌘↵ 就干谁」，故 ⌘↵ 角标据此挂在提交按钮上。
const cmdEnterWillSubmit = computed(
  () => verticalMode.value && lastSeen.value && nextUnansweredAfter(current.value) < 0
);
// 提交按钮挂 ⌘↵ / 为主按钮：纵向按 cmdEnterWillSubmit，旧版顺序沿用 onLastQuestion。
const submitShowsCmdEnter = computed(() =>
  verticalMode.value ? cmdEnterWillSubmit.value : onLastQuestion.value
);
const submitPrimary = computed(() => submitShowsCmdEnter.value);
// 下一个是否主按钮：末题从不主；否则在「提交尚未成为主按钮」时为主（读题引导）。
const nextPrimary = computed(
  () => !onLastQuestion.value && !submitPrimary.value
);

// CMD+数字 选项快捷键上限（1-9）；超出的选项不分配快捷键。
const OPTION_HOTKEY_MAX = 9;
function optionHotkey(i: number): string | null {
  return i < OPTION_HOTKEY_MAX ? `⌘${i + 1}` : null;
}

function isAnswered(i: number): boolean {
  return (
    (chosenByQ.value[i]?.length ?? 0) > 0 ||
    (inputByQ.value[i]?.trim().length ?? 0) > 0 ||
    (imagesByQ.value[i]?.length ?? 0) > 0 ||
    (replyFilesByQ.value[i]?.length ?? 0) > 0
  );
}

// 折叠输入仅在纵向模式生效：默认 1 行，聚焦或已有内容时展开。单题 / 旧版顺序模式恒展开。
function expandedQ(i: number): boolean {
  if (!verticalMode.value) return true;
  return focusedQ.value === i || (inputByQ.value[i]?.trim().length ?? 0) > 0;
}
// 每题题干渲染（Markdown 全局开关 + 源码视图）。
function questionHtml(q: Question): string {
  return request.value?.isMarkdown && !viewSource.value
    ? renderMarkdown(q.message, codeCopyLabels.value)
    : "";
}
// 仅「当前题」显示 ⌘1–9 角标（避免每题都冒出 ⌘1）。
function cardOptionHotkey(qIndex: number, optIndex: number): string | null {
  if (isMulti.value && qIndex !== current.value) return null;
  return optionHotkey(optIndex);
}

// 提问附带的文件附件（AI→人，仅展示）：Message 级，顶部常驻，不随题切换。
const attachments = computed<FileAttachment[]>(
  () => request.value?.message.files ?? []
);
const selectedFile = ref<number | null>(null);
const thumbs = ref<Record<string, string>>({});
const dragIcons = ref<Record<string, string>>({});
const attRefs = ref<HTMLElement[]>([]);
const previewing = ref(false);
// 托盘「待答」子菜单点击本弹窗时，边框闪烁一次（accent 蓝脉冲）。
const flashing = ref(false);
let flashTimer: number | undefined;
let unlistenIndex: UnlistenFn | null = null;
let unlistenFocus: UnlistenFn | null = null;
let unlistenDrop: UnlistenFn | null = null;
let unlistenSettings: UnlistenFn | null = null;
let unlistenUpdate: UnlistenFn | null = null;
let unlistenCloseReq: UnlistenFn | null = null;
let unlistenFlash: UnlistenFn | null = null;
let unlistenAgent: UnlistenFn | null = null;
// 方案6 预热弹窗：daemon 领用时 emit 的唤醒事件，前端据此 pull 请求并渲染。
let unlistenShow: UnlistenFn | null = null;

function triggerFlash() {
  // 重启动画：先关再于下一帧开，确保连续点击也能重新触发。
  flashing.value = false;
  if (flashTimer) window.clearTimeout(flashTimer);
  requestAnimationFrame(() => {
    flashing.value = true;
    // 两次脉冲约 0.6s 后复位。
    flashTimer = window.setTimeout(() => {
      flashing.value = false;
    }, 700);
  });
}

function setAttRef(el: Element | null, i: number) {
  if (el) attRefs.value[i] = el as HTMLElement;
}

function selectFile(index: number) {
  focusAttachment(index);
}

function openFile(file: FileAttachment) {
  openPath(file.path).catch(() => {});
}

// 渲染后的 Markdown 里的链接：用系统默认浏览器打开，避免在弹窗 webview 内跳转。
function onContentClick(e: MouseEvent) {
  // 代码块的拷贝按钮优先处理（命中即结束，不再走链接逻辑）。
  if (handleCodeCopyClick(e)) return;
  const anchor = (e.target as HTMLElement | null)?.closest?.("a") as
    | HTMLAnchorElement
    | null;
  if (!anchor) return;
  const href = anchor.href;
  if (!/^(https?:|mailto:)/i.test(href)) return;
  e.preventDefault();
  e.stopPropagation();
  openPath(href).catch(() => {});
}

function focusAttachment(index: number) {
  selectedFile.value = index;
  attRefs.value[index]?.focus();
}

function previewSelected(index: number) {
  focusAttachment(index);
  previewing.value = true;
  previewAttachments(
    attachments.value.map((f) => f.path),
    index
  ).catch(() => {});
}

function stopPreview() {
  if (!previewing.value) return;
  previewing.value = false;
  closePreview().catch(() => {});
}

function onBackgroundClick(e: MouseEvent) {
  if ((e.target as HTMLElement).closest(".attachment")) return;
  if (selectedFile.value !== null) selectedFile.value = null;
  stopPreview();
}

function handleAttachmentKey(e: KeyboardEvent): boolean {
  if (!attachments.value.length) return false;
  const i = selectedFile.value;
  if (i === null) return false;
  if (e.key === "Enter") {
    openFile(attachments.value[i]);
  } else if (e.key === " ") {
    previewSelected(i);
  } else if (e.key === "ArrowRight" || e.key === "ArrowDown") {
    if (i < attachments.value.length - 1) focusAttachment(i + 1);
  } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
    if (i > 0) focusAttachment(i - 1);
  } else {
    return false;
  }
  e.preventDefault();
  return true;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

async function loadThumbs() {
  for (const file of attachments.value) {
    if (!file.isImage || thumbs.value[file.path]) continue;
    try {
      thumbs.value[file.path] = await readImageDataUrl(file.path);
    } catch {
      /* 缩略图加载失败：退回通用图标，不阻断 */
    }
  }
}

async function loadDragIcons() {
  for (const file of attachments.value) {
    if (dragIcons.value[file.path]) continue;
    try {
      dragIcons.value[file.path] = await fileIconDataUrl(file.path);
    } catch {
      /* 取图标失败：拖出时回退用缩略图或不带预览 */
    }
  }
}

const draggingOut = ref(false);

function onAttachmentContextMenu(file: FileAttachment, i: number, e: MouseEvent) {
  e.preventDefault();
  selectFile(i);
  showAttachmentMenu(file.path).catch((err) =>
    console.error("打开右键菜单失败", err)
  );
}

function onAttachmentDragStart(file: FileAttachment, e: DragEvent) {
  e.preventDefault();
  const icon = dragIcons.value[file.path] || thumbs.value[file.path] || "";
  draggingOut.value = true;
  startDrag({ item: [file.path], icon }, () => {
    setTimeout(() => (draggingOut.value = false), 300);
  }).catch((err) => {
    draggingOut.value = false;
    console.error("拖出附件失败", err);
  });
}

async function togglePin() {
  pinned.value = !pinned.value;
  try {
    await getCurrentWindow().setAlwaysOnTop(pinned.value);
  } catch {
    pinned.value = !pinned.value;
  }
}

async function cycleTheme() {
  const order: ThemeMode[] = ["system", "light", "dark"];
  const next = order[(order.indexOf(theme.value) + 1) % order.length];
  theme.value = next;
  applyTheme(next);
  try {
    await updateTheme(next);
  } catch {
    /* 忽略：持久化失败不影响当前显示 */
  }
}

function openSettingsWindow() {
  openSettings().catch(() => {});
}

// ===== 首次运行引导（R6）：无 IM 渠道时页脚上方的一次性提示 =====
const imTipVisible = ref(false);

function imTipConfigure() {
  imTipVisible.value = false;
  popupImTipDismiss().catch(() => {});
  openSettings("channel").catch(() => {});
}

function imTipDismiss() {
  imTipVisible.value = false;
  popupImTipDismiss().catch(() => {});
}

function openHistoryWindow() {
  openHistory().catch(() => {});
}

// 切换某题的选项（带题索引，供选项点击 / CMD+数字 复用）。
function toggle(qIndex: number, option: string) {
  const arr = chosenByQ.value[qIndex];
  if (!arr) return;
  const i = arr.indexOf(option);
  // 单选：选中即替换为唯一项；再次点击当前选中项则清空（保留"可不选"，除非严格模式）。
  if (single.value) {
    if (i >= 0) arr.splice(0, arr.length);
    else arr.splice(0, arr.length, option);
    return;
  }
  if (i >= 0) arr.splice(i, 1);
  else arr.push(option);
}

// 通过序号（0 始）切换「当前题」的选项，供 CMD+数字 调用。
function toggleByIndex(i: number) {
  const opts = currentQuestion.value?.predefinedOptions;
  if (!opts || i < 0 || i >= opts.length) return;
  toggle(current.value, opts[i].text);
}

// 点「添加图片」：记录目标题后唤起文件选择。
function pickFiles(qIndex: number) {
  pendingPickQ = qIndex;
  fileRef.value?.click();
}

async function addFiles(files: FileList | File[], qIndex: number) {
  if (selectOnly.value) return;
  let added = 0;
  for (const file of Array.from(files)) {
    if (!file.type.startsWith("image/")) continue;
    const data = await fileToDataUrl(file);
    imagesByQ.value[qIndex]?.push({
      data,
      mediaType: file.type,
      filename: file.name,
    });
    added++;
  }
  if (added) scrollImagesIntoView(qIndex);
}

// 新增图片后把该题最新缩略图滚入可见区：粘贴/选择时即使内容已上滚，也能立刻确认成功。
async function scrollImagesIntoView(qIndex: number) {
  await nextTick();
  const wrap = thumbsRefs.value[qIndex];
  if (!wrap) return;
  const last = (wrap.lastElementChild as HTMLElement | null) ?? wrap;
  const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  last.scrollIntoView({ block: "nearest", behavior: reduce ? "auto" : "smooth" });
}

function onFileChange(e: Event) {
  const input = e.target as HTMLInputElement;
  if (input.files) addFiles(input.files, pendingPickQ);
  input.value = "";
}

function removeImage(qIndex: number, index: number) {
  imagesByQ.value[qIndex]?.splice(index, 1);
}

// DOM 级 drop 仅阻止默认（真正落盘走原生 onDragDropEvent，带落点坐标）。
function onDrop(_e: DragEvent) {}

// 原生拖放落点 → 命中的问题卡片索引（physical 坐标需除以 DPR 转 CSS 像素）。
function questionAtPoint(physX: number, physY: number): number {
  if (!verticalMode.value) return current.value;
  const dpr = window.devicePixelRatio || 1;
  const el = document.elementFromPoint(physX / dpr, physY / dpr) as HTMLElement | null;
  const card = el?.closest?.(".q-card") as HTMLElement | null;
  if (card?.dataset.qIndex != null) {
    const idx = Number(card.dataset.qIndex);
    if (!Number.isNaN(idx)) return idx;
  }
  return current.value;
}

const IMAGE_EXT = /\.(png|jpe?g|gif|webp|bmp|heic|heif|tiff?|svg)$/i;

async function addDroppedPaths(paths: string[], qIndex: number) {
  if (selectOnly.value) return;
  const attachPaths = new Set(attachments.value.map((a) => a.path));
  let addedImage = 0;
  for (const path of paths) {
    if (attachPaths.has(path)) continue;
    const name = path.split(/[\\/]/).pop() || "file";
    if (IMAGE_EXT.test(path)) {
      try {
        const data = await readImageDataUrl(path);
        const semi = data.indexOf(";");
        const mediaType = semi > 5 ? data.slice(5, semi) : "image/png";
        imagesByQ.value[qIndex]?.push({ data, mediaType, filename: name });
        addedImage++;
      } catch (err) {
        console.error("读取拖入图片失败", path, err);
      }
    } else if (!(replyFilesByQ.value[qIndex] ?? []).some((f) => f.path === path)) {
      replyFilesByQ.value[qIndex]?.push({ path, name });
    }
  }
  if (addedImage) scrollImagesIntoView(qIndex);
}

function removeReplyFile(qIndex: number, index: number) {
  replyFilesByQ.value[qIndex]?.splice(index, 1);
}

// 粘贴图片：归到当前聚焦的问题（无聚焦则归当前题 active）。
async function onPaste(e: ClipboardEvent) {
  if (selectOnly.value) return;
  const items = e.clipboardData?.items;
  if (!items) return;
  const files: File[] = [];
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (item.kind === "file" && item.type.startsWith("image/")) {
      const f = item.getAsFile();
      if (f) files.push(f);
    }
  }
  if (files.length) {
    e.preventDefault();
    await addFiles(files, focusedQ.value ?? current.value);
  }
}

// 输入框随内容自增高（封顶 240px，超出则框内滚动）。仅展开态生效（折叠态固定 1 行）。
const MAX_TEXTAREA_H = 240;
function autoGrow(i: number = current.value) {
  const el = inputRefs.value[i];
  if (!el) return;
  if (!expandedQ(i)) {
    // 折叠态：清除内联高度，交回 CSS 的 1 行高度。
    el.style.height = "";
    return;
  }
  el.style.height = "auto";
  el.style.height = `${Math.min(el.scrollHeight, MAX_TEXTAREA_H)}px`;
}

// textarea 聚焦/失焦：维护 focusedQ + 切当前题（聚焦即展开）；失焦且空则折叠。
function onTextareaFocus(i: number) {
  focusedQ.value = i;
  setActive(i, false);
  nextTick(() => autoGrow(i));
}
function onTextareaBlur(i: number) {
  if (focusedQ.value === i) focusedQ.value = null;
  nextTick(() => autoGrow(i));
}

// ===== 多题导航（纵向列表：当前题指针 + 滚动定位） =====
function markVisited(i: number) {
  if (i >= 0 && i < visited.value.length) visited.value[i] = true;
}

// 第 i 题是否已完整落在视口内（含上下 16px 呼吸位）；卡片比视口还高时永远返回 false（无法完整容纳）。
function isCardFullyVisible(i: number): boolean {
  const root = contentRef.value;
  const el = cardRefs.value[i];
  if (!root || !el) return true;
  const margin = 16;
  const rootRect = root.getBoundingClientRect();
  const elRect = el.getBoundingClientRect();
  return (
    elRect.top >= rootRect.top + margin &&
    elRect.bottom <= rootRect.top + root.clientHeight - margin
  );
}

// 第 i 题是否**完全在屏外**（整卡在视口上方或下方，一点都看不到）。用于 reveal-first：刚打开时长 message
// 把 Q1 顶到屏外，此时点「下一个」应先把 Q1 露出来而非跳到 Q2；而只是「底部被切一点」不算屏外，可正常推进。
function isCardOffScreen(i: number): boolean {
  const root = contentRef.value;
  const el = cardRefs.value[i];
  if (!root || !el) return false;
  const rootRect = root.getBoundingClientRect();
  const elRect = el.getBoundingClientRect();
  return (
    elRect.top >= rootRect.top + root.clientHeight || elRect.bottom <= rootRect.top
  );
}

// 把内容滚到最顶（scrollTop=0）：纵向模式在首题按「上一个」时用来完整露出上方 message。
function scrollContentToTop() {
  const root = contentRef.value;
  if (!root) return;
  const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  root.scrollTo({ top: 0, behavior: reduce ? "auto" : "smooth" });
}

// 把第 i 题滚到「顶部对齐 + 16px 呼吸位」——统一露出该题**顶部**（含末题：夹到 max 即贴底、也完整可见）。
// 不再用比例定位（会把靠后的题只露出个顶、后续再切就卡住）。对齐顶部后阅读线不落在目标题 → 必须靠 NAV_LOCK_MS
// 锁住 scroll-spy 到动画结束，避免 current 被抢；用户手动滚动时 scroll-spy 才重新接管。
function scrollQuestionIntoView(i: number) {
  const root = contentRef.value;
  const el = cardRefs.value[i];
  if (!root || !el) return;
  const max = root.scrollHeight - root.clientHeight;
  if (max <= 0) return; // 内容未超视口：无可滚动空间（active 由 setActive 直接置位）
  // 已完整可见（含上下 16px 呼吸位）就**不滚动**：避免「目标题本就在屏内还硬滚一段」（用户反馈）。
  if (isCardFullyVisible(i)) return;
  const rootRect = root.getBoundingClientRect();
  const elRect = el.getBoundingClientRect();
  const offsetTop = elRect.top - rootRect.top + root.scrollTop;
  const top = Math.max(0, Math.min(max, offsetTop - 16));
  const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  root.scrollTo({ top, behavior: reduce ? "auto" : "smooth" });
}

// 设当前题：夹边界、置 active + 标记已看到；可选滚动到可见。短锁避免滚动事件回改。
function setActive(index: number, scroll: boolean) {
  const i = Math.max(0, Math.min(index, total.value - 1));
  current.value = i;
  markVisited(i);
  if (scroll) {
    activeLockUntil = Date.now() + NAV_LOCK_MS;
    scrollQuestionIntoView(i);
  }
}

// 导航「整题闪一下渐隐」：提示焦点落点，上一个/下一个按钮与 ⌘[/⌘]/⌘↵ 都会触发。闪光区域为 `.q-card::before`
// （圆角、accent 淡底，见 CSS），一次性 opacity 1→0 渐隐。
// 用 class + 强制 reflow 重放（卡片无 :class 绑定，Vue 不会清掉）；prefers-reduced-motion 下不闪。
function flashCard(i: number) {
  const el = cardRefs.value[i];
  if (!el) return;
  el.classList.remove("flash");
  void el.offsetWidth; // 强制 reflow，令同一题也能再次触发动画
  el.classList.add("flash");
}

// 跳到第 i 题（绝对索引）：置当前题 + 标记看过 + **自动聚焦该题输入框**（用户要求「切到某题就激活它的输入框」）。
// 聚焦会触发折叠输入框展开、改变高度，故**展开后再滚动**（双 nextTick），避免用旧高度定位。全程用 NAV_LOCK_MS
// 锁住 scroll-spy 到滚动动画结束，避免 current 被滚动事件抢走。供上一个/下一个、⌘[/⌘]、⌘↵ 复用，行为一致。
function goToIdx(target: number) {
  stopPreview();
  selectedFile.value = null;
  const i = Math.max(0, Math.min(target, total.value - 1));
  setActive(i, false); // 先置当前题、不滚动（滚动放到展开之后）
  activeLockUntil = Date.now() + NAV_LOCK_MS;
  nextTick(() => {
    inputRefs.value[i]?.focus({ preventScroll: true });
    nextTick(() => {
      activeLockUntil = Date.now() + NAV_LOCK_MS; // 展开后重置锁：确保覆盖此刻才开始的 smooth 滚动动画
      scrollQuestionIntoView(i);
    });
  });
}

// 相对移动当前题（上一个/下一个 + ⌘[/⌘]）。委托 goToIdx（焦点携带 + 展开后滚动）。
function goRel(delta: number) {
  goToIdx(current.value + delta);
}

// 旧版顺序模式切题：仅一题可见，改 current 即换页（聚焦/滚动由 Transition after-enter 处理）。
function goToSeq(index: number) {
  if (index < 0 || index >= total.value || index === current.value) return;
  stopListening(); // 切题前停语音，避免回调写进旧题
  stopPreview();
  selectedFile.value = null;
  current.value = index;
  markVisited(index);
}

// 旧版单题面板头部滚到顶（Message 很长时也能露出当前题）。
function scrollHeaderIntoView() {
  const el = qHeaderRef.value;
  if (!el) return;
  const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  el.scrollIntoView({ block: "start", behavior: reduce ? "auto" : "smooth" });
}

// 旧版切题动画完成后：聚焦输入 + 校正高度 + 滚动头部到顶（新面板已挂载、高度确定）。
function onQuestionEntered() {
  if (verticalMode.value) return;
  inputRef.value?.focus({ preventScroll: true });
  autoGrow(current.value);
  scrollHeaderIntoView();
}

// 纵向导航统一在此闪一下：无论来自「上一个/下一个」按钮还是 ⌘[/⌘]，落点题都整题闪一次（用户要求按钮也闪）。
function goPrev() {
  if (verticalMode.value) {
    // 已在首题：「上一个」= 把上方 message 完整露出（滚到最顶），而非无动作（用户预期两级：Q2→Q1 露出 Q1、
    // 在 Q1 再上一个才露出 message）。
    if (current.value === 0) {
      activeLockUntil = Date.now() + NAV_LOCK_MS;
      scrollContentToTop();
      flashCard(0);
      return;
    }
    goRel(-1);
    flashCard(current.value);
    return;
  }
  slideDir.value = "prev";
  goToSeq(current.value - 1);
}

function goNext() {
  if (verticalMode.value) {
    // 当前题**完全在屏外**（如长 message 刚打开把 Q1 顶到屏外）→ 先把它露出来 + 聚焦 + 闪一下，而非直接跳下一题
    // （用户反馈：刚打开点「下一个」直接跳到 Q2、Q1 从没露出）。仅「底部被切一点」不算屏外，可正常推进到下一题。
    if (isCardOffScreen(current.value)) {
      goToIdx(current.value);
      flashCard(current.value);
      return;
    }
    goRel(1);
    flashCard(current.value);
    return;
  }
  slideDir.value = "next";
  goToSeq(current.value + 1);
}

// 从 from 之后找第一个「未答」/「未看过」的题；无则 -1（供 ⌘↵ 智能推进）。
function nextUnansweredAfter(from: number): number {
  for (let i = from + 1; i < total.value; i++) if (!isAnswered(i)) return i;
  return -1;
}
function nextUnseenAfter(from: number): number {
  for (let i = from + 1; i < total.value; i++) if (!(visited.value[i] ?? false)) return i;
  return -1;
}

// 纵向模式的 ⌘↵：从当前焦点向后找下一个未答题→跳过去（闪+滚入+置焦点）；后面没有未答了→已看完全部
// 则提交，否则去下一个没看过的题继续读（推进「读完」门槛）。永不落到已答题（修「跳回已答」bug）。
// 自由模式想留空提交：直接点底部「提交」按钮（阶段②/③ 可见）。
function onCmdEnter() {
  // 当前题**完全在屏外**（长 message 顶开首题等）→ 先把它露出来 + 聚焦 + 闪一下，而非直接跳到后面的未答题。
  if (isCardOffScreen(current.value)) {
    goToIdx(current.value);
    flashCard(current.value);
    return;
  }
  const u = nextUnansweredAfter(current.value);
  if (u >= 0) {
    goToIdx(u);
    flashCard(u);
    return;
  }
  if (lastSeen.value) {
    submit();
    return;
  }
  const s = nextUnseenAfter(current.value);
  if (s >= 0) {
    goToIdx(s);
    flashCard(s);
    return;
  }
  submit();
}

function collectAnswers(): QuestionAnswer[] {
  return questions.value.map((q, i) => ({
    selectedOptions: q.predefinedOptions
      .map((o) => o.text)
      .filter((o) => (chosenByQ.value[i] ?? []).includes(o)),
    userInput: inputByQ.value[i] ?? "",
    images: imagesByQ.value[i] ?? [],
    files: (replyFilesByQ.value[i] ?? []).map((f) => f.path),
  }));
}

async function submit() {
  if (submitting.value || !canSubmit.value) return;
  submitting.value = true;
  try {
    await submitPopup({ answers: collectAnswers() });
  } catch {
    submitting.value = false;
  }
}

const selectedConfirmChoice = computed(() =>
  confirmChoiceIndex.value === null
    ? null
    : confirmRequest.value?.choices[confirmChoiceIndex.value] ?? null
);
const confirmInput = computed(() => confirmRequest.value?.presentation.input ?? null);
const showConfirmInput = computed(
  () =>
    !!confirmInput.value &&
    selectedConfirmChoice.value?.id === confirmInput.value.visibleWhenActionId
);
const confirmCanSubmit = computed(
  () => confirmChoiceIndex.value !== null && !submitting.value
);
const confirmDetailHtml = computed(() =>
  confirmRequest.value?.detail.bodyMd
    ? renderMarkdown(confirmRequest.value.detail.bodyMd, codeCopyLabels.value)
    : ""
);
const confirmToolName = computed(
  () =>
    confirmRequest.value?.context.find((field) => field.id === "tool")?.value ??
    "Tool"
);

function selectConfirmChoice(index: number) {
  if (!submitting.value) confirmChoiceIndex.value = index;
}

async function submitConfirm() {
  if (!confirmCanSubmit.value || confirmChoiceIndex.value === null) return;
  submitting.value = true;
  const comment = showConfirmInput.value
    ? confirmComment.value.slice(0, confirmInput.value?.maxChars ?? 1000)
    : null;
  try {
    await submitConfirmAction(confirmChoiceIndex.value, comment || null);
  } catch {
    submitting.value = false;
  }
}

function requestConfirmClose() {
  if (!submitting.value) showConfirmCloseWarning.value = true;
}

function dismissConfirmCloseWarning() {
  showConfirmCloseWarning.value = false;
}

async function confirmCloseAndDeny() {
  const req = confirmRequest.value;
  if (!req || submitting.value) return;
  const index = req.choices.findIndex((choice) => choice.id === req.dismissActionId);
  if (index < 0) return;
  confirmChoiceIndex.value = index;
  showConfirmCloseWarning.value = false;
  await submitConfirm();
}

// 取消入口：有回答时二次确认，否则直接取消。
function requestCancel() {
  if (submitting.value) return;
  if (hasAnyAnswer.value) {
    showCancelConfirm.value = true;
  } else {
    doCancel();
  }
}

async function doCancel() {
  if (submitting.value) return;
  submitting.value = true;
  showCancelConfirm.value = false;
  try {
    await cancelPopup();
  } catch {
    submitting.value = false;
  }
}

function dismissCancelConfirm() {
  showCancelConfirm.value = false;
}

// ===== 语音输入（macOS 26 SpeechAnalyzer，⌘D 切换） =====
// 后端(Swift/Rust)语音事件以 "key" 或 "key|param" 上报；此处拆解并交给 i18n 翻译，
// 故 speechStatus/speechError 存「原始 payload」，模板渲染时再翻译（语言切换可即时重渲染）。
function parseSpeechPayload(payload: string): { key: string; param: string } {
  const i = payload.indexOf("|");
  return i === -1
    ? { key: payload, param: "" }
    : { key: payload.slice(0, i), param: payload.slice(i + 1) };
}

function speechStatusText(payload: string): string {
  const { key } = parseSpeechPayload(payload);
  const path = `speech.status.${key}`;
  const s = t(path);
  return s === path ? payload : s; // 未知 key → 原样展示
}

function speechErrorText(payload: string): string {
  const { key, param } = parseSpeechPayload(payload);
  const path = `speech.error.${key}`;
  const params =
    key === "unsupportedLocale"
      ? { locale: param }
      : key === "generic"
      ? { message: param }
      : {};
  const s = t(path, params);
  return s === path ? param || key : s; // 未知 key → 退回原始信息
}

// 入参为语义 key（或 key|param），翻译在模板渲染时进行。
function showSpeechError(payload: string) {
  speechError.value = payload;
  if (speechErrorTimer) clearTimeout(speechErrorTimer);
  speechErrorTimer = setTimeout(() => {
    speechError.value = null;
    speechErrorTimer = null;
  }, 4000);
}

function toggleSpeech() {
  if (listening.value) stopListening();
  else startListening();
}

function startListening() {
  if (!speechSupported.value) {
    showSpeechError("needMacos26");
    return;
  }
  if (speechErrorTimer) {
    clearTimeout(speechErrorTimer);
    speechErrorTimer = null;
  }
  speechError.value = null;
  speechStatus.value = null;
  speechTargetQ = current.value;
  // 听写起点 = 当前光标处。若存在选区：保持高亮，待首个识别文字到达时才替换（原生听写语义）。
  const el = inputRef.value;
  const fieldLen = inputByQ.value[speechTargetQ]?.length ?? 0;
  let start = fieldLen;
  let end = fieldLen;
  if (el && speechTargetQ === current.value) {
    start = el.selectionStart ?? fieldLen;
    end = el.selectionEnd ?? start;
  }
  interimStart = start;
  interimLen = 0;
  lastSelStart = start;
  lastSelEnd = end;
  // 延迟替换的待删选区（end>start 时有效）；不立刻删，保留选区高亮。
  pendingSelStart = end > start ? start : -1;
  pendingSelEnd = end > start ? end : -1;

  listening.value = true;
  speechReady.value = false; // 先进入 loading，待 speech-ready 再高亮。
  const locale =
    speechLang.value && speechLang.value !== "auto" ? speechLang.value : "";
  startSpeech(locale).catch((err) => {
    listening.value = false;
    speechReady.value = false;
    showSpeechError("startFailed");
    console.error("启动语音失败", err);
  });
}

function stopListening() {
  if (!listening.value) return;
  listening.value = false;
  speechReady.value = false;
  stopSpeech().catch(() => {});
}

// 首个识别文字到达时，删除「待替换选区」（实现：说话才替换选中文本）。
function consumePendingSelection() {
  if (pendingSelStart >= 0 && pendingSelEnd > pendingSelStart) {
    const v = inputByQ.value[speechTargetQ] ?? "";
    inputByQ.value[speechTargetQ] =
      v.slice(0, pendingSelStart) + v.slice(pendingSelEnd);
    interimStart = pendingSelStart;
    interimLen = 0;
  }
  pendingSelStart = -1;
  pendingSelEnd = -1;
}

// 「已最终化」片段：移除当前实时片段，再在 interimStart 处永久插入。
function onSpeechCommitted(delta: string) {
  if (!delta || suspendSpeechDom) return;
  consumePendingSelection();
  let v = inputByQ.value[speechTargetQ] ?? "";
  if (interimLen > 0) {
    v = v.slice(0, interimStart) + v.slice(interimStart + interimLen);
    interimLen = 0;
  }
  v = v.slice(0, interimStart) + delta + v.slice(interimStart);
  interimStart += delta.length;
  inputByQ.value[speechTargetQ] = v;
  syncCaret();
}

// 实时片段：就地替换 [interimStart, interimStart+interimLen]。
function onSpeechVolatile(text: string) {
  if (suspendSpeechDom) return;
  // 尚无任何文字、也无既有实时片段时（空回调），不触碰选区。
  if (!text && interimLen === 0) return;
  consumePendingSelection();
  let v = inputByQ.value[speechTargetQ] ?? "";
  v = v.slice(0, interimStart) + text + v.slice(interimStart + interimLen);
  interimLen = text.length;
  inputByQ.value[speechTargetQ] = v;
  syncCaret();
}

// 把光标移到实时片段末尾，并记录为「程序化」位置（避免误判为用户移动）。
function syncCaret() {
  if (speechTargetQ !== current.value || suspendSpeechDom) return;
  nextTick(() => {
    autoGrow();
    const el = inputRef.value;
    if (!el) return;
    const pos = Math.min(interimStart + interimLen, el.value.length);
    el.selectionStart = el.selectionEnd = pos;
    lastSelStart = pos;
    lastSelEnd = pos;
  });
}

// 鼠标在输入框按下即开始拖选：暂停语音写入 DOM，保护用户选区。
function onTextareaMouseDown() {
  if (listening.value && speechTargetQ === current.value) {
    suspendSpeechDom = true;
  }
}

// 鼠标松开（可能在窗口任意处）：恢复语音写入，并按最终选区处理。
function onDocMouseUp() {
  if (!suspendSpeechDom) return;
  suspendSpeechDom = false;
  onUserCaretMaybeMoved();
}

// 用户在听写中主动移动光标/编辑：固定当前内容、以新光标为起点重启识别会话。
function onUserCaretMaybeMoved() {
  if (!listening.value || speechTargetQ !== current.value) return;
  const el = inputRef.value;
  if (!el) return;
  const selStart = el.selectionStart ?? 0;
  const selEnd = el.selectionEnd ?? selStart;
  // 与上次已知选区相同 → 无新操作（含程序化设置）。
  if (selStart === lastSelStart && selEnd === lastSelEnd) return;
  // 用户改变了光标/选区：以此为新起点重启会话。
  if (selEnd > selStart) {
    // 选区 → 延迟替换（说话才删）。
    pendingSelStart = selStart;
    pendingSelEnd = selEnd;
  } else {
    pendingSelStart = -1;
    pendingSelEnd = -1;
  }
  interimStart = selStart;
  interimLen = 0;
  lastSelStart = selStart;
  lastSelEnd = selEnd;
  flushSpeech().catch(() => {});
}

// 订阅后端 speech-* 事件。
async function setupSpeechListeners() {
  unlistenSpeech.push(
    await listen<string>("speech-committed", (e) => onSpeechCommitted(e.payload))
  );
  unlistenSpeech.push(
    await listen<string>("speech-volatile", (e) => onSpeechVolatile(e.payload))
  );
  unlistenSpeech.push(
    await listen<string>("speech-status", (e) => {
      speechStatus.value = e.payload || null;
    })
  );
  unlistenSpeech.push(
    await listen("speech-ready", () => {
      if (listening.value) speechReady.value = true;
    })
  );
  unlistenSpeech.push(
    await listen<string>("speech-error", (e) => {
      listening.value = false;
      speechReady.value = false;
      showSpeechError(e.payload || "generic");
    })
  );
  unlistenSpeech.push(
    await listen("speech-stopped", () => {
      listening.value = false;
      speechReady.value = false;
    })
  );
}

// 仅「纯」⌘/Ctrl（未叠加 Shift/Option）才算命中快捷键修饰键：例如 Cmd+Shift（截屏）下
// 再按 1–9 不会命中选项快捷键，故此时不应高亮。
function onlyCmdHeld(e: KeyboardEvent): boolean {
  return (e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey;
}

// ⌘/Ctrl 按下/松开 → 切换 cmdHeld（驱动快捷键 Badge 高亮）。窗口失焦时复位，避免卡住。
function onKeyup(e: KeyboardEvent) {
  cmdHeld.value = onlyCmdHeld(e);
}
function onWindowBlur() {
  cmdHeld.value = false;
}

function onKeydown(e: KeyboardEvent) {
  const mod = e.metaKey || e.ctrlKey;
  cmdHeld.value = onlyCmdHeld(e);
  if (isConfirm.value) {
    if (e.key === "Escape") {
      e.preventDefault();
      if (showConfirmCloseWarning.value) dismissConfirmCloseWarning();
      else requestConfirmClose();
      return;
    }
    if (mod && e.key === "Enter") {
      e.preventDefault();
      void submitConfirm();
      return;
    }
    if (mod && (e.key === "w" || e.key === "W")) {
      e.preventDefault();
      if (Date.now() >= appearGuardUntil) requestConfirmClose();
      return;
    }
    if (mod && e.key >= "1" && e.key <= "9") {
      const index = Number(e.key) - 1;
      if (index < (confirmRequest.value?.choices.length ?? 0)) {
        e.preventDefault();
        selectConfirmChoice(index);
      }
      return;
    }
    return;
  }
  // 录音中按 Esc：结束本次语音输入（不关闭弹窗）。
  if (e.key === "Escape" && listening.value) {
    e.preventDefault();
    stopListening();
    return;
  }
  if (mod && e.key === "Enter") {
    e.preventDefault();
    // 纵向多题：⌘↵ 智能推进（下一未答→看完提交→否则去下一未看），永不回跳已答。
    if (verticalMode.value) {
      onCmdEnter();
    } else if (isMulti.value && !onLastQuestion.value) {
      // 旧版顺序多题：非最后一题始终前往下一题（即使提交按钮已出现），最后一题才提交。
      goNext();
    } else {
      submit();
    }
    return;
  }
  if (mod && (e.key === "w" || e.key === "W")) {
    e.preventDefault();
    // 出现动画期内吞掉 ⌘W：防止用户为关闭其它窗口按下的 ⌘W 恰好误关刚弹出的弹窗（仍 preventDefault 拦住原生关窗）。
    if (Date.now() < appearGuardUntil) return;
    requestCancel();
    return;
  }
  // 语音输入快捷键（可在设置中自定义；空串=关闭）。
  if (
    speechSupported.value &&
    speechShortcut.value &&
    matchShortcut(e, speechShortcut.value)
  ) {
    e.preventDefault();
    toggleSpeech();
    return;
  }
  // 多题：CMD+] 下一题，CMD+[ 上一题（不影响 CMD+回车）。闪一下由 goNext/goPrev 内部统一处理（按钮/键盘一致）。
  if (isMulti.value && mod && e.key === "]") {
    e.preventDefault();
    goNext();
    return;
  }
  if (isMulti.value && mod && e.key === "[") {
    e.preventDefault();
    goPrev();
    return;
  }
  // CMD+数字（1-9）：选中/取消当前题对应序号的选项。
  if (mod && e.key >= "1" && e.key <= "9") {
    const idx = Number(e.key) - 1;
    const opts = currentQuestion.value?.predefinedOptions;
    if (opts && idx < opts.length && idx < OPTION_HOTKEY_MAX) {
      e.preventDefault();
      toggleByIndex(idx);
      return;
    }
  }
  const tgt = e.target as HTMLElement | null;
  const typing =
    tgt && (tgt.tagName === "TEXTAREA" || tgt.tagName === "INPUT");
  if (!typing) handleAttachmentKey(e);
}

// 方案6 预热：领用一次性守卫——首个带 request 的 init 才渲染，避免重复领用。
let adopting = false;

// 把（含 request 的）init 渲染上屏：套主题/语言/来源 → 设 request → 双 rAF 打点 → 首帧后再做非关键初始化。
// 预热弹窗（init.warm）窗口起始隐藏，绘制完成后调 popup_show_window 让后端延后 show（杜绝空白闪现）。
function renderInit(init: PopupInit) {
  const interaction = init.interaction;
  if (!interaction) return;
  applyTheme(init.theme);
  theme.value = init.theme;
  // 精确语言来自 popup_init（零钥匙串）；main.ts 只做 auto 兜底，故此处校正。
  if (typeof init.language === "string") applyLanguage(init.language);
  pinned.value = init.alwaysOnTop;
  sourceName.value = init.sourceName;
  projectName.value = init.projectName;
  projectPath.value = init.project;
  agentKind.value = init.agentKind ?? "";
  agentPid.value = init.agentPid ?? null;
  createdAtMs.value = init.createdAtMs ?? 0;
  // 领用/渲染即刻校准 now，避免 tick 首帧前相对时间偏大。
  nowMs.value = Date.now();
  // 语音设置改取自 popup_init（不再走 get_settings / 钥匙串）。
  speechLang.value = init.speechLanguage || "auto";
  speechShortcut.value = init.speechShortcut || "cmd+d";
  verticalEnabled.value = init.verticalQuestions ?? false;
  const req = interaction.type === "ask" ? interaction.request : null;
  request.value = req;
  confirmRequest.value = interaction.type === "confirm" ? interaction.request : null;
  confirmChoiceIndex.value =
    interaction.type === "confirm" && interaction.request.presentation.defaultActionId
      ? interaction.request.choices.findIndex(
          (choice) => choice.id === interaction.request.presentation.defaultActionId
        )
      : null;
  if (confirmChoiceIndex.value === -1) confirmChoiceIndex.value = null;
  confirmComment.value = "";
  showConfirmCloseWarning.value = false;
  const n = req?.questions.length ?? 0;
  chosenByQ.value = Array.from({ length: n }, () => []);
  inputByQ.value = Array.from({ length: n }, () => "");
  imagesByQ.value = Array.from({ length: n }, () => []);
  replyFilesByQ.value = Array.from({ length: n }, () => []);
  visited.value = Array.from({ length: n }, () => false);
  if (n > 0) visited.value[0] = true;
  // 重置每题 DOM 引用数组（v-for 重渲染会按索引回填）。
  inputRefs.value = [];
  cardRefs.value = [];
  sentinelRefs.value = [];
  thumbsRefs.value = [];
  focusedQ.value = null;
  current.value = 0;
  loadThumbs();
  loadDragIcons();
  // 纵向模式（实验开关开 且 多题）：不自动聚焦、保持全部折叠、建哨兵观察。
  // 否则（单题 / 旧版顺序模式）：聚焦当前题输入框 + 校正高度。
  const vertical = verticalEnabled.value && n > 1;
  const afterPaint = () => {
    // DOM 更新后再聚焦/建观察（此时 textarea / 哨兵已挂载）。
    nextTick(() => {
      if (!vertical) {
        inputRef.value?.focus({ preventScroll: true });
        autoGrow(0);
      } else {
        setupQuestionObserver();
      }
    });
    // 双 rAF：第一帧让正文进入 DOM 并即将绘制，第二帧回调时该帧已真正合成上屏，
    // 此刻打点更贴近用户真正看到内容的时刻（比单 rAF 晚约 1 帧）。
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        perfMarkFe("fe.painted");
        if (isConfirm.value) confirmPopupReady().catch(() => {});
        // harness 模式：内容已上屏即自动取消，免人工点按。
        if (init.perfAutodismiss && !isConfirm.value) {
          cancelPopup().catch(() => {});
        }
      });
    });
  };
  if (init.warm) {
    // 预热弹窗的窗口此刻仍隐藏（ordered-out），没有 display link → rAF 不会回调，故不能「先双 rAF 再 show」。
    // 改为：nextTick 等 DOM 把正文更新完，再请后端上屏；窗口可见后 WebKit 即绘制当前 DOM（已是正文，无
    // 「加载中→正文」闪现），rAF 也随之恢复，afterPaint 在 show 之后打点 / 自动取消。
    nextTick(() => {
      popupShowWindow().catch(() => {});
      appearGuardUntil = Date.now() + APPEAR_GUARD_MS; // 窗口刚上屏：短时吞掉 ⌘W 防误关
      afterPaint();
    });
  } else {
    // 冷路径：窗口已在 setup 中显示，rAF 正常回调。
    appearGuardUntil = Date.now() + APPEAR_GUARD_MS;
    afterPaint();
  }
  // 内容已渲染：把其余初始化（事件监听 / 语音 / 自更新 / 终端探测）放到首帧之后，不阻塞首屏。
  void initAfterPaint(init);
}

// 预热弹窗领用：重新 pull popup_init，若已带 request 则渲染（一次性）。
async function adopt() {
  if (request.value || confirmRequest.value || adopting) return;
  adopting = true;
  try {
    const init = await popupInit();
    if (init.interaction) {
      // 热路径领用：丢弃预热阶段缓存的标记（fe.bootstrap/fe.mounted/待命 popup_init 不属本次请求），
      // 只上报领用后的标记（如 fe.painted），避免污染时间线（负的 page boot）。
      if (init.perf) perfEnableFe(true);
      renderInit(init);
    }
  } catch (err) {
    console.error("popup adopt 失败", err);
  } finally {
    adopting = false;
  }
}

onMounted(async () => {
  // 头部相对时间每秒走字（开销极小；无 createdAt 时 computed 返回空，不渲染）。
  timeTicker = window.setInterval(() => {
    nowMs.value = Date.now();
  }, 1000);
  // 同步窗口监听（开销极小）：放最前，保证粘贴 / 快捷键 / Esc 从首帧即可用。
  window.addEventListener("paste", onPaste);
  window.addEventListener("keydown", onKeydown);
  window.addEventListener("keyup", onKeyup);
  window.addEventListener("blur", onWindowBlur);
  document.addEventListener("mouseup", onDocMouseUp);
  // 方案6：预热弹窗领用唤醒事件——尽早注册以免漏接（冷路径不会收到，无害）。
  unlistenShow = await listen("popup-show", () => {
    void adopt();
  });
  // 关键路径：第一步即取请求内容并渲染，尽快上屏；其余初始化全部移到渲染之后（见 initAfterPaint）。
  try {
    const init = await popupInit();
    // 后端在 helper 进程收到 ASKHUMAN_PERF_ID 时置 perf=true：开启前端埋点并冲刷此前缓存的标记。
    if (init.perf) perfEnableFe();
    perfMarkFe("fe.popup_init_done");
    if (init.interaction) {
      // 冷路径 / 已领用：直接渲染。
      renderInit(init);
    } else {
      // 预热待命：先按当前主题/语言渲染（窗口隐藏），等 popup-show 领用。
      applyTheme(init.theme);
      theme.value = init.theme;
      if (typeof init.language === "string") applyLanguage(init.language);
      // 兜底竞态：领用可能发生在首个 popup_init 与监听注册之间，立即复查一次。
      void adopt();
    }
  } catch (err) {
    console.error("popup_init 失败", err);
    loadError.value = String(err);
  }
});

// 首帧渲染后再执行的非关键初始化。这些监听对应的事件都由 daemon 在 show 之后才可能发来，
// 或为用户 / 托盘触发，略晚于首帧注册无碍（自更新态另用 popupUpdateState() 拉初值兜底）。
// 放此处是为了不阻塞弹窗首屏（原先这些 await 串在 popupInit 之前，正是「加载中」停留的来源）。
async function initAfterPaint(init: PopupInit) {
  unlistenIndex = await listen<number>("preview-index", (e) => {
    const i = e.payload;
    if (i >= 0 && i < attachments.value.length) {
      selectedFile.value = i;
      nextTick(() => {
        const el = attRefs.value[i];
        el?.focus();
        el?.scrollIntoView({ block: "nearest" });
      });
    }
  });
  unlistenFocus = await listen("preview-closed", () => {
    previewing.value = false;
    const i = selectedFile.value;
    if (i !== null) nextTick(() => attRefs.value[i]?.focus());
  });
  unlistenDrop = await getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type !== "drop") return;
    if (draggingOut.value) {
      draggingOut.value = false;
      return;
    }
    const pos = event.payload.position;
    const qIndex = questionAtPoint(pos?.x ?? 0, pos?.y ?? 0);
    addDroppedPaths(event.payload.paths, qIndex);
  });
  // 设置变更实时生效（同进程内设置窗口保存后广播 general 配置）。
  unlistenSettings = await listen<{
    theme?: ThemeMode;
    language?: string;
    speechLanguage?: string;
    speechShortcut?: string;
  }>("settings-updated", (e) => {
    // daemon 架构下由 Daemon 经 IPC 下发（独立 GUI Helper 进程）；单进程下由设置窗口同进程广播。
    if (typeof e.payload.theme === "string") {
      applyTheme(e.payload.theme);
      theme.value = e.payload.theme;
    }
    if (typeof e.payload.language === "string") applyLanguage(e.payload.language);
    if (typeof e.payload.speechLanguage === "string")
      speechLang.value = e.payload.speechLanguage || "auto";
    if (typeof e.payload.speechShortcut === "string")
      speechShortcut.value = e.payload.speechShortcut;
  });
  // 探测语音是否可用（macOS 26+）+ 订阅 speech-* 事件。
  try {
    speechSupported.value = await speechAvailable();
  } catch {
    speechSupported.value = false;
  }
  if (speechSupported.value) await setupSpeechListeners();
  // 版本自更新：先拉初值（规避事件早于监听），再监听 daemon 经 GUI Helper 转发的实时变更。
  try {
    const u = await popupUpdateState();
    updateAvailable.value = u.available;
    updatePending.value = u.pending;
    updateLatest.value = u.latestVersion;
  } catch {
    /* 单进程回退 / 无 daemon：忽略 */
  }
  unlistenUpdate = await listen<{
    available: boolean;
    latestVersion: string;
    pending: boolean;
  }>("update-state", (e) => {
    updateAvailable.value = e.payload.available;
    updatePending.value = e.payload.pending;
    updateLatest.value = e.payload.latestVersion;
  });
  // 原生关闭按钮：后端阻止关闭并转发此事件 → 与 ⌘W 一致走二次确认。
  unlistenCloseReq = await listen("popup-close-requested", () => {
    if (isConfirm.value) requestConfirmClose();
    else requestCancel();
  });
  // 托盘「待答」子菜单点击本弹窗：后端已聚焦窗口，这里播放边框闪烁。
  unlistenFlash = await listen("popup-flash", () => {
    triggerFlash();
  });
  // 调用方 agent 信息（家族 + pid）由 daemon 从 caller_pid **异步** walk 得到（方案5/b），经
  // `agent-resolved` 后推：先 pull 初值（规避事件早于监听的竞态），再监听实时升级。拿到 pid 才把 badge
  // 升级成「可点 + ↗」（终端类型探测仍要跑进程链 ps，故也在此渲染后异步进行）。旧 daemon 可能随
  // popup_init 直接带 pid → 一并处理。
  if (init.agentPid != null) {
    void applyAgentResolved(init.agentKind, init.agentPid);
  }
  unlistenAgent = await listen<{ kind?: string | null; pid?: number | null }>(
    "agent-resolved",
    (e) => {
      void applyAgentResolved(e.payload.kind, e.payload.pid);
    },
  );
  try {
    const r = await popupAgentResolved();
    if (r.kind || r.pid != null) void applyAgentResolved(r.kind, r.pid);
  } catch {
    /* 无 daemon / 单进程回退：忽略 */
  }
  // R6 一次性引导：未配置任何 IM 渠道且未被关闭过时，页脚上方显示提示条。
  try {
    imTipVisible.value = await popupImTipVisible();
  } catch {
    /* 旧后端无此命令：忽略 */
  }
}

/// 应用 daemon 异步解析出的 agent 信息：补全家族 badge 文案，并据 pid 解析所在终端把 badge 升级成
/// 「可点 + ↗」。幂等：pull 初值与事件可能各触发一次，重复设值无副作用。
async function applyAgentResolved(
  kind: string | null | undefined,
  pid: number | null | undefined
) {
  if (kind && !agentKind.value) agentKind.value = kind;
  if (pid != null) {
    agentPid.value = pid;
    try {
      agentTerminal.value = (await popupAgentTerminal(pid)) ?? null;
    } catch {
      /* 探测失败：保持纯文字 badge */
    }
  }
}

onBeforeUnmount(() => {
  window.removeEventListener("paste", onPaste);
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("keyup", onKeyup);
  window.removeEventListener("blur", onWindowBlur);
  document.removeEventListener("mouseup", onDocMouseUp);
  unlistenIndex?.();
  unlistenFocus?.();
  unlistenDrop?.();
  unlistenSettings?.();
  unlistenUpdate?.();
  unlistenCloseReq?.();
  unlistenFlash?.();
  unlistenAgent?.();
  unlistenShow?.();
  if (timeTicker) window.clearInterval(timeTicker);
  if (flashTimer) window.clearTimeout(flashTimer);
  if (copiedTimer) window.clearTimeout(copiedTimer);
  io?.disconnect();
  io = null;
  if (scrollRaf) cancelAnimationFrame(scrollRaf);
  stopListening();
  unlistenSpeech.forEach((fn) => fn());
  unlistenSpeech = [];
  if (speechErrorTimer) clearTimeout(speechErrorTimer);
});
</script>

<template>
  <div v-if="!request && !confirmRequest" class="popup popup-status">
    <p v-if="loadError" class="status-error">
      {{ t("popup.loadError", { msg: loadError }) }}
    </p>
    <p v-else class="status-loading">{{ t("popup.loading") }}</p>
  </div>

  <div
    v-else
    class="popup"
    :class="{ 'cmd-held': cmdHeld }"
    @dragover.prevent
    @drop.prevent="onDrop"
    @click="onBackgroundClick"
  >
    <div v-if="flashing" class="flash-overlay" aria-hidden="true"></div>
    <header class="navbar" :class="{ scrolled }" data-tauri-drag-region>
      <span class="brand" :class="{ inline: agentInline }">
        <span class="brand-dot"></span>
        <span class="brand-title">{{ headerPrefix }}</span>
        <component
          :is="agentFocusable ? 'button' : 'span'"
          v-if="agentLabel"
          class="brand-chip brand-agent"
          :class="{ clickable: agentFocusable }"
          :type="agentFocusable ? 'button' : undefined"
          :title="agentFocusable ? t('agents.focusTerminal') : undefined"
          @click="onFocusAgentTerminal"
        >
          <span class="chip-text">{{ agentLabel }}</span>
          <svg
            v-if="agentFocusable"
            class="chip-arrow"
            viewBox="0 0 10 10"
            aria-hidden="true"
          >
            <path
              d="M3 7 L7 3 M4 3 H7 V6"
              fill="none"
              stroke="currentColor"
              stroke-width="1.2"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
        </component>
        <button
          v-if="projectName"
          type="button"
          class="brand-chip brand-workspace clickable"
          :title="projectPath"
          @click="onOpenWorkspace"
        >
          <span class="chip-text">{{ projectName }}</span>
          <svg class="chip-arrow" viewBox="0 0 10 10" aria-hidden="true">
            <path
              d="M3 7 L7 3 M4 3 H7 V6"
              fill="none"
              stroke="currentColor"
              stroke-width="1.2"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
        </button>
        <span v-if="headerSuffix" class="brand-title brand-suffix">{{
          headerSuffix
        }}</span>
        <span
          v-if="popupTimeRel"
          class="brand-time"
          :title="popupTimeAbs"
          >· {{ popupTimeRel }}</span
        >
      </span>
      <span class="nav-actions">
        <div v-if="updateAvailable" class="update-wrap">
          <button
            class="nav-btn update-btn"
            type="button"
            :title="t('popup.nav.update')"
            @click.stop="toggleUpdatePopover"
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 3v12" />
              <path d="M7 10l5 5 5-5" />
              <path d="M5 21h14" />
            </svg>
            <span class="update-dot"></span>
          </button>
          <div v-if="updatePopoverOpen" class="update-popover" @click.stop>
            <p class="up-title">
              {{ t("popup.update.title", { version: updateLatest }) }}
            </p>
            <div
              v-if="updateNotesHtml"
              class="up-notes markdown-body"
              v-html="updateNotesHtml"
              @click="onContentClick"
            ></div>
            <p v-else class="up-notes muted">{{ t("popup.update.noNotes") }}</p>
            <p class="up-hint">
              {{
                updateStarted
                  ? t("popup.update.startedHint")
                  : t("popup.update.applyHint")
              }}
            </p>
            <p v-if="updateError" class="up-error">{{ updateError }}</p>
            <div class="up-actions">
              <button
                class="btn btn-primary"
                type="button"
                :disabled="updating || updateStarted"
                @click="applyUpdateFromPopup"
              >
                {{
                  updating
                    ? t("popup.update.updating")
                    : t("popup.update.button")
                }}
              </button>
            </div>
          </div>
        </div>
        <button
          class="nav-btn"
          :class="{ active: pinned }"
          type="button"
          :title="t('popup.nav.pin')"
          @click="togglePin"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 17v5" />
            <path d="M9 10.8a2 2 0 0 1-1.1 1.8l-1.8.9A2 2 0 0 0 5 15.2V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.8a2 2 0 0 0-1.1-1.8l-1.8-.9A2 2 0 0 1 15 10.8V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z" />
          </svg>
        </button>
        <button
          class="nav-btn"
          type="button"
          :title="t('popup.nav.theme')"
          @click="cycleTheme"
        >
          <svg v-if="theme === 'light'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="4" />
            <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
          </svg>
          <svg v-else-if="theme === 'dark'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9z" />
          </svg>
          <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 3a9 9 0 0 1 0 18z" fill="currentColor" stroke="none" />
          </svg>
        </button>
        <button
          class="nav-btn"
          type="button"
          :title="t('popup.nav.history')"
          @click="openHistoryWindow"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 3v5h5" />
            <path d="M3.05 13a9 9 0 1 0 2.5-6.36L3 8" />
            <path d="M12 7v5l3 2" />
          </svg>
        </button>
        <button
          class="nav-btn"
          type="button"
          :title="t('popup.nav.settings')"
          @click="openSettingsWindow"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.6 1.6 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.6 1.6 0 0 0-1.8-.3 1.6 1.6 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1a1.6 1.6 0 0 0-1-1.5 1.6 1.6 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.6 1.6 0 0 0 .3-1.8 1.6 1.6 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1a1.6 1.6 0 0 0 1.5-1 1.6 1.6 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.6 1.6 0 0 0 1.8.3H9a1.6 1.6 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.6 1.6 0 0 0 1 1.5 1.6 1.6 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.6 1.6 0 0 0-.3 1.8V9a1.6 1.6 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.6 1.6 0 0 0-1.5 1z" />
          </svg>
        </button>
      </span>
    </header>
    <div
      v-if="updatePopoverOpen"
      class="update-backdrop"
      @click="updatePopoverOpen = false"
    ></div>
    <div v-if="updatePending" class="update-pending-banner">
      {{ t("popup.update.pendingBanner") }}
    </div>
    <div
      ref="contentRef"
      class="content"
      @scroll="onScroll"
    >
      <template v-if="isConfirm && confirmRequest">
        <section class="confirm-request">
          <h1 class="confirm-request-title">{{ confirmRequest.title }}</h1>
          <p v-if="confirmRequest.detail.summary" class="confirm-reason">
            <strong>{{ t("popup.permissionReason") }}</strong>
            {{ confirmRequest.detail.summary }}
          </p>
          <section class="confirm-tool">
            <header class="confirm-tool-header">{{ confirmToolName }}</header>
            <div
              v-if="confirmRequest.detail.bodyMd"
              class="markdown-body confirm-detail"
              v-html="confirmDetailHtml"
              @click="onContentClick"
            ></div>
          </section>
          <div class="confirm-options" role="radiogroup" :aria-label="confirmRequest.title">
            <div
              v-for="(choice, index) in confirmRequest.choices"
              :key="choice.id"
              class="option single confirm-option"
              :class="[
                `role-${choice.role}`,
                { selected: confirmChoiceIndex === index },
              ]"
              role="radio"
              tabindex="0"
              :aria-checked="confirmChoiceIndex === index"
              @click="selectConfirmChoice(index)"
              @keydown.enter.prevent="selectConfirmChoice(index)"
              @keydown.space.prevent="selectConfirmChoice(index)"
            >
              <span class="check radio" aria-hidden="true"></span>
              <span class="label confirm-option-label">
                <span>{{ choice.label }}</span>
                <small v-if="choice.description">{{ choice.description }}</small>
              </span>
              <kbd v-if="index < 9" class="opt-sc">⌘{{ index + 1 }}</kbd>
            </div>
          </div>
          <label v-if="showConfirmInput && confirmInput" class="confirm-input-block">
            <span>{{ confirmInput.label }}</span>
            <textarea
              v-model="confirmComment"
              class="textarea"
              :maxlength="confirmInput.maxChars"
              :placeholder="confirmInput.placeholder"
              rows="4"
            ></textarea>
            <small>{{ confirmComment.length }} / {{ confirmInput.maxChars }}</small>
          </label>
        </section>
      </template>
      <template v-else>
      <!-- 共享 Message 区（描述 + 附件），仅在有内容时展示，顶部常驻 -->
      <template v-if="showDescription">
        <div
          v-if="messageText && request?.isMarkdown && !viewSource"
          class="markdown-body"
          v-html="messageHtml"
          @click="onContentClick"
        ></div>
        <pre v-else-if="messageText" class="plain-body">{{ messageText }}</pre>

        <div v-if="attachments.length" class="attachments">
        <div class="att-caption">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48" />
          </svg>
          <span>{{ t("popup.attachments", { n: attachments.length }) }}</span>
        </div>
        <div class="att-list">
          <div
            v-for="(file, i) in attachments"
            :key="file.path"
            :ref="(el) => setAttRef(el as Element | null, i)"
            class="attachment"
            :class="{ selected: selectedFile === i }"
            tabindex="0"
            draggable="true"
            :title="file.path"
            @click="selectFile(i)"
            @dblclick="openFile(file)"
            @dragstart="onAttachmentDragStart(file, $event)"
            @contextmenu="onAttachmentContextMenu(file, i, $event)"
          >
            <span class="att-icon" :class="{ 'is-image': file.isImage && thumbs[file.path] }">
              <img v-if="file.isImage && thumbs[file.path]" :src="thumbs[file.path]" alt="" />
              <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
                <path d="M14 3v4a1 1 0 0 0 1 1h4" />
                <path d="M17 21H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h7l5 5v11a2 2 0 0 1-2 2z" />
              </svg>
            </span>
            <span class="att-meta">
              <span class="att-name">{{ file.name }}</span>
              <span class="att-size">{{ formatBytes(file.size) }}</span>
            </span>
          </div>
        </div>
      </div>
      </template>

      <!-- message 下方右对齐工具条：复制 Message + Markdown/源码切换（切换作用于整篇）。
           仅在有共享 Message 时显示——直接提问（无 message）不显示复制/源码按钮。 -->
      <div v-if="messageText.trim()" class="msg-tools">
        <button
          class="mt-btn"
          :class="{ done: copiedMessage }"
          type="button"
          :title="copiedMessage ? t('common.copied') : t('popup.view.copyMessage')"
          :aria-label="t('popup.view.copyMessage')"
          @click="copyMessage"
        >
          <svg class="mt-ico mt-copy" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" /></svg>
          <svg class="mt-ico mt-check" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5" /></svg>
        </button>
        <button
          class="mt-btn"
          :class="{ active: viewSource }"
          type="button"
          :title="viewSource ? t('popup.view.viewRendered') : t('popup.view.viewSource')"
          :aria-label="viewSource ? t('popup.view.viewRendered') : t('popup.view.viewSource')"
          @click="viewSource = !viewSource"
        >
          <svg class="mt-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><polyline points="18 16 22 12 18 8" /><polyline points="6 8 2 12 6 16" /><line x1="14.5" y1="4" x2="9.5" y2="20" /></svg>
        </button>
      </div>

      <!-- 纵向模式（实验开关 + 多题）：所有问题纵向平铺成卡片，当前题高亮 -->
      <template v-if="verticalMode">
      <div
        v-for="(q, qi) in questions"
        :key="qi"
        :ref="(el) => setCardRef(el as HTMLElement | null, qi)"
        class="q-card"
        :data-q-index="qi"
        @mousedown="setActive(qi, false)"
      >
        <!-- 问题头部：问号图标 + 「Question i/n」。每题上方加分割线（与 Message/上一题区隔）。 -->
        <div
          class="q-header with-divider"
        >
          <svg class="q-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="9" />
            <path d="M9.2 9.3a2.8 2.8 0 0 1 5.4 1c0 1.9-2.8 2.5-2.8 2.5" />
            <path d="M12 17.2h.01" />
          </svg>
          <span class="q-label">{{
            t("popup.question.indexed", { i: qi + 1, n: total })
          }}</span>
        </div>

        <div
          v-if="request?.isMarkdown && !viewSource && q.message"
          class="markdown-body"
          v-html="questionHtml(q)"
          @click="onContentClick"
        ></div>
        <pre v-else-if="q.message" class="plain-body">{{ q.message }}</pre>

        <div v-if="q.predefinedOptions.length" class="options">
          <div
            v-for="(opt, i) in q.predefinedOptions"
            :key="i"
            class="option"
            :class="{ selected: (chosenByQ[qi] ?? []).includes(opt.text), single }"
            @click="toggle(qi, opt.text)"
          >
            <span class="check" :class="{ radio: single }">{{ single ? "" : ((chosenByQ[qi] ?? []).includes(opt.text) ? "✓" : "") }}</span>
            <span class="label"><span v-if="opt.recommended" class="rec-badge"><span class="rec-badge-pill"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3z"></path><path d="M7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"></path></svg>{{ t("popup.recommended") }}</span></span>{{ opt.text }}</span>
            <kbd v-if="cardOptionHotkey(qi, i)" class="opt-sc">{{ cardOptionHotkey(qi, i) }}</kbd>
          </div>
        </div>

        <!-- 输入框 + 内置「添加图片」小图标（右下角）；严格选择模式隐藏 -->
        <div v-if="!selectOnly" class="input-wrap">
          <textarea
            :ref="(el) => setInputRef(el as HTMLTextAreaElement | null, qi)"
            v-model="inputByQ[qi]"
            class="textarea"
            :class="{ collapsed: !expandedQ(qi) }"
            rows="1"
            :placeholder="t('popup.inputPlaceholder')"
            @input="autoGrow(qi)"
            @focus="onTextareaFocus(qi)"
            @blur="onTextareaBlur(qi)"
            @keyup="onUserCaretMaybeMoved"
            @mousedown="onTextareaMouseDown"
          ></textarea>
          <template v-if="expandedQ(qi)">
            <button
              v-if="speechSupported"
              class="mic-btn"
              :class="{ loading: listening && current === qi && !speechReady, recording: listening && current === qi && speechReady }"
              type="button"
              :title="
                speechReady
                  ? t('popup.speech.stop') +
                    (speechHotkeyLabel ? ' ' + speechHotkeyLabel : '')
                  : listening
                  ? t('popup.speech.preparing')
                  : t('popup.speech.start') +
                    (speechHotkeyLabel ? ' ' + speechHotkeyLabel : '')
              "
              :aria-label="
                listening ? t('popup.speech.stop') : t('popup.speech.start')
              "
              @mousedown.prevent
              @click="(setActive(qi, false), toggleSpeech())"
            >
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                <rect x="9" y="2" width="6" height="12" rx="3" />
                <path d="M5 11a7 7 0 0 0 14 0" />
                <path d="M12 18v3" />
              </svg>
            </button>
            <button
              class="img-btn"
              type="button"
              :title="t('popup.addImage')"
              :aria-label="t('popup.addImage')"
              @click="pickFiles(qi)"
            >
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                <rect x="3" y="3" width="18" height="18" rx="2" />
                <circle cx="8.5" cy="8.5" r="1.6" />
                <path d="M21 15l-5-5L5 21" />
              </svg>
            </button>
          </template>
        </div>
        <p v-if="!selectOnly && current === qi && speechError" class="speech-error">
          {{ speechErrorText(speechError) }}
        </p>
        <p v-else-if="!selectOnly && current === qi && listening && speechStatus" class="speech-status">
          {{ speechStatusText(speechStatus) }}
        </p>

        <div
          v-if="!selectOnly && (imagesByQ[qi] ?? []).length"
          :ref="(el) => setThumbsRef(el as HTMLElement | null, qi)"
          class="thumbs"
        >
          <div v-for="(img, i) in imagesByQ[qi]" :key="i" class="thumb">
            <img :src="img.data" alt="" />
            <button class="remove" type="button" @click="removeImage(qi, i)">
              ×
            </button>
          </div>
        </div>

        <div v-if="!selectOnly && (replyFilesByQ[qi] ?? []).length" class="reply-files">
          <div
            v-for="(f, i) in replyFilesByQ[qi]"
            :key="f.path"
            class="reply-file"
            :title="f.path"
          >
            <span class="rf-icon">📄</span>
            <span class="rf-name">{{ f.name }}</span>
            <button class="rf-remove" type="button" @click="removeReplyFile(qi, i)">
              ×
            </button>
          </div>
        </div>

        <!-- 底部哨兵：进视口即「已看到」该题（兼容超长题） -->
        <div
          :ref="(el) => setSentinelRef(el as HTMLElement | null, qi)"
          class="q-sentinel"
          :data-q-sentinel="qi"
          aria-hidden="true"
        ></div>
      </div>
      </template>

      <!-- 旧版（顺序模式）：单题 / 实验开关关时——一次显示一个问题，上一步/下一步左右滑动切换 -->
      <template v-else>
        <!-- 问题头部：间距 + 分割线 + 问号图标 + 「Question i/n」 -->
        <div
          v-if="showQuestionHeader"
          ref="qHeaderRef"
          class="q-header"
          :class="{ 'with-divider': showDescription }"
        >
          <svg class="q-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="9" />
            <path d="M9.2 9.3a2.8 2.8 0 0 1 5.4 1c0 1.9-2.8 2.5-2.8 2.5" />
            <path d="M12 17.2h.01" />
          </svg>
          <span class="q-label">{{ questionHeaderLabel }}</span>
        </div>

        <!-- 当前问题区（上一个/下一个左右滑动） -->
        <Transition :name="transitionName" mode="out-in" @after-enter="onQuestionEntered">
          <div class="question-pane" :key="current">
            <div
              v-if="request?.isMarkdown && !viewSource && currentQuestion?.message"
              class="markdown-body"
              v-html="renderedHtml"
              @click="onContentClick"
            ></div>
            <pre v-else-if="currentQuestion?.message" class="plain-body">{{ currentQuestion?.message }}</pre>

            <div v-if="currentQuestion && currentQuestion.predefinedOptions.length" class="options">
              <div
                v-for="(opt, i) in currentQuestion.predefinedOptions"
                :key="i"
                class="option"
                :class="{ selected: chosen.includes(opt.text), single }"
                @click="toggle(current, opt.text)"
              >
                <span class="check" :class="{ radio: single }">{{ single ? "" : (chosen.includes(opt.text) ? "✓" : "") }}</span>
                <span class="label"><span v-if="opt.recommended" class="rec-badge"><span class="rec-badge-pill"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3z"></path><path d="M7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"></path></svg>{{ t("popup.recommended") }}</span></span>{{ opt.text }}</span>
                <kbd v-if="optionHotkey(i)" class="opt-sc">{{ optionHotkey(i) }}</kbd>
              </div>
            </div>

            <!-- 输入框 + 内置「添加图片」小图标（右下角）；严格选择模式隐藏 -->
            <div v-if="!selectOnly" class="input-wrap">
              <textarea
                :ref="(el) => setInputRef(el as HTMLTextAreaElement | null, current)"
                v-model="userInput"
                class="textarea"
                :placeholder="t('popup.inputPlaceholder')"
                @input="autoGrow(current)"
                @keyup="onUserCaretMaybeMoved"
                @mousedown="onTextareaMouseDown"
              ></textarea>
              <button
                v-if="speechSupported"
                class="mic-btn"
                :class="{ loading: listening && !speechReady, recording: speechReady }"
                type="button"
                :title="
                  speechReady
                    ? t('popup.speech.stop') +
                      (speechHotkeyLabel ? ' ' + speechHotkeyLabel : '')
                    : listening
                    ? t('popup.speech.preparing')
                    : t('popup.speech.start') +
                      (speechHotkeyLabel ? ' ' + speechHotkeyLabel : '')
                "
                :aria-label="
                  listening ? t('popup.speech.stop') : t('popup.speech.start')
                "
                @mousedown.prevent
                @click="toggleSpeech"
              >
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="9" y="2" width="6" height="12" rx="3" />
                  <path d="M5 11a7 7 0 0 0 14 0" />
                  <path d="M12 18v3" />
                </svg>
              </button>
              <button
                class="img-btn"
                type="button"
                :title="t('popup.addImage')"
                :aria-label="t('popup.addImage')"
                @click="pickFiles(current)"
              >
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="3" y="3" width="18" height="18" rx="2" />
                  <circle cx="8.5" cy="8.5" r="1.6" />
                  <path d="M21 15l-5-5L5 21" />
                </svg>
              </button>
            </div>
            <p v-if="!selectOnly && speechError" class="speech-error">
              {{ speechErrorText(speechError) }}
            </p>
            <p v-else-if="!selectOnly && listening && speechStatus" class="speech-status">
              {{ speechStatusText(speechStatus) }}
            </p>

            <div v-if="!selectOnly && images.length" :ref="(el) => setThumbsRef(el as HTMLElement | null, current)" class="thumbs">
              <div v-for="(img, i) in images" :key="i" class="thumb">
                <img :src="img.data" alt="" />
                <button class="remove" type="button" @click="removeImage(current, i)">
                  ×
                </button>
              </div>
            </div>

            <div v-if="!selectOnly && replyFiles.length" class="reply-files">
              <div
                v-for="(f, i) in replyFiles"
                :key="f.path"
                class="reply-file"
                :title="f.path"
              >
                <span class="rf-icon">📄</span>
                <span class="rf-name">{{ f.name }}</span>
                <button class="rf-remove" type="button" @click="removeReplyFile(current, i)">
                  ×
                </button>
              </div>
            </div>
          </div>
        </Transition>
      </template>
      </template>
    </div>

    <input
      ref="fileRef"
      type="file"
      accept="image/*"
      multiple
      hidden
      @change="onFileChange"
    />

    <!-- 首次运行引导（R6）：未配置 IM 渠道时的一次性提示条 -->
    <div v-if="imTipVisible" class="im-tip">
      <span class="im-tip-text">{{ t("popup.imTip.text") }}</span>
      <button class="im-tip-action" type="button" @click="imTipConfigure">
        {{ t("popup.imTip.action") }}
      </button>
      <button
        class="im-tip-close"
        type="button"
        :title="t('popup.imTip.dismiss')"
        @click="imTipDismiss"
      >
        <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
          <line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
          <line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
        </svg>
      </button>
    </div>

    <!-- 多问题底部：取消(左) + 上一个/下一个/提交(右) -->
    <div v-if="!isConfirm && isMulti" class="footer" data-tauri-drag-region>
      <button class="btn" type="button" :disabled="submitting" @click="requestCancel">
        {{ t("common.cancel") }} <kbd class="sc">⌘W</kbd>
      </button>
      <span class="spacer"></span>
      <button
        class="btn"
        type="button"
        :disabled="submitting || !canGoPrev"
        @click="goPrev"
      >
        {{ t("popup.prev") }} <kbd v-if="canGoPrev" class="sc">⌘[</kbd>
      </button>
      <button
        class="btn"
        :class="{ 'btn-primary': nextPrimary }"
        type="button"
        :disabled="submitting || current === total - 1"
        @click="goNext"
      >
        {{ t("popup.next") }}
        <kbd v-if="!onLastQuestion && !submitShowsCmdEnter" class="sc">⌘↵</kbd>
      </button>
      <button
        v-if="verticalMode ? lastSeen : allViewed"
        class="btn"
        :class="{ 'btn-primary': submitPrimary }"
        type="button"
        :disabled="submitting || !canSubmit"
        @click="submit"
      >
        {{ t("common.submit") }}
        <kbd v-if="submitShowsCmdEnter" class="sc">⌘↵</kbd>
      </button>
    </div>

    <!-- 单问题底部：取消(左) + 发送(右) -->
    <div v-else-if="!isConfirm" class="footer" data-tauri-drag-region>
      <button class="btn" type="button" :disabled="submitting" @click="requestCancel">
        {{ t("common.cancel") }} <kbd class="sc">⌘W</kbd>
      </button>
      <span class="spacer"></span>
      <button
        class="btn btn-primary"
        type="button"
        :disabled="submitting || !canSubmit"
        @click="submit"
      >
        {{ t("popup.send") }} <kbd class="sc">⌘↵</kbd>
      </button>
    </div>

    <div v-else class="footer" data-tauri-drag-region>
      <button class="btn" type="button" :disabled="submitting" @click="requestConfirmClose">
        {{ t("common.cancel") }} <kbd class="sc">⌘W</kbd>
      </button>
      <span class="spacer"></span>
      <button
        class="btn btn-primary"
        type="button"
        :disabled="!confirmCanSubmit"
        @click="submitConfirm"
      >
        {{ confirmRequest?.presentation.submitLabel ?? t("common.submit") }}
        <kbd class="sc">⌘↵</kbd>
      </button>
    </div>

    <!-- 取消二次确认 -->
    <div v-if="!isConfirm && showCancelConfirm" class="confirm-overlay" @click.self="dismissCancelConfirm">
      <div class="confirm-box">
        <p class="confirm-title">{{ t("popup.confirmCancel.title") }}</p>
        <p class="confirm-desc">{{ t("popup.confirmCancel.desc") }}</p>
        <div class="confirm-actions">
          <button class="btn" type="button" @click="dismissCancelConfirm">
            {{ t("popup.confirmCancel.keep") }}
          </button>
          <button class="btn btn-danger" type="button" @click="doCancel">
            {{ t("popup.confirmCancel.confirm") }}
          </button>
        </div>
      </div>
    </div>

    <div
      v-if="isConfirm && showConfirmCloseWarning"
      class="confirm-overlay"
      @click.self="dismissConfirmCloseWarning"
    >
      <div class="confirm-box">
        <p class="confirm-title">{{ t("popup.confirmClose.title") }}</p>
        <p class="confirm-desc">
          {{
            confirmComment.trim()
              ? t("popup.confirmClose.descWithReason")
              : t("popup.confirmClose.desc")
          }}
        </p>
        <div class="confirm-actions">
          <button class="btn" type="button" @click="dismissConfirmCloseWarning">
            {{ t("popup.confirmClose.keep") }}
          </button>
          <button class="btn btn-danger" type="button" @click="confirmCloseAndDeny">
            {{ t("popup.confirmClose.deny") }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.popup {
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.confirm-request {
  display: flex;
  flex-direction: column;
  gap: 18px;
  width: 100%;
  min-width: 0;
}
.confirm-request-title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  line-height: 1.35;
  color: var(--text-primary);
}
.confirm-tool {
  display: grid;
  gap: 0;
  min-width: 0;
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: color-mix(in srgb, var(--surface) 92%, transparent);
  box-shadow: 0 1px 2px color-mix(in srgb, #000 4%, transparent);
}
.confirm-tool-header {
  padding: 12px 14px;
  border-bottom: 1px solid var(--border);
  font-size: 14px;
  font-weight: 600;
  line-height: 1.25;
  color: var(--text-primary);
}
.confirm-reason {
  margin: 0;
  color: var(--text-primary);
  font-size: 14px;
  line-height: 1.5;
}
.confirm-detail {
  min-width: 0;
  padding: 13px 14px 14px;
  background: color-mix(in srgb, var(--surface) 68%, transparent);
}
.confirm-options {
  display: grid;
  gap: 8px;
}
.confirm-option {
  width: 100%;
  box-sizing: border-box;
}
.confirm-option.role-destructive.selected {
  border-color: #ff453a;
  background: color-mix(in srgb, #ff453a 10%, transparent);
}
.confirm-option.role-destructive.selected .check.radio {
  border-color: #ff453a;
}
.confirm-option.role-destructive.selected .check.radio::after {
  background: #ff453a;
}
.confirm-option.role-destructive.selected .opt-sc {
  color: #ff453a;
  border-color: color-mix(in srgb, #ff453a 40%, transparent);
}
.confirm-option-label {
  display: grid;
  gap: 2px;
  min-width: 0;
}
.confirm-option-label small {
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.4;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
.confirm-input-block {
  display: grid;
  gap: 7px;
  color: var(--text-primary);
  font-size: 13px;
}
.confirm-input-block small {
  justify-self: end;
  color: var(--text-secondary);
}
@media (max-width: 560px) {
  .confirm-request {
    gap: 14px;
  }
}

/* 托盘「待答」子菜单点击本弹窗 → 边框 accent 蓝脉冲 2 次（约 0.6s）。仅视觉，不拦截交互。 */
.flash-overlay {
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: 9999;
  border-radius: var(--radius-lg, 10px);
  box-shadow:
    inset 0 0 0 2px var(--accent),
    inset 0 0 14px 2px color-mix(in srgb, var(--accent) 55%, transparent);
  animation: popup-flash 0.3s ease-in-out 2;
}
@keyframes popup-flash {
  0% { opacity: 0; }
  50% { opacity: 1; }
  100% { opacity: 0; }
}

/* 顶部导航栏：整条可拖动；品牌区/动作区透传拖拽，仅按钮可点 */
.navbar {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 8px 12px 8px 14px;
  border-bottom: 1px solid transparent;
}
.navbar.scrolled {
  border-bottom-color: var(--border);
}
.vibrancy .navbar {
  padding-top: 30px;
}
.brand {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  pointer-events: none;
  /* 允许在窄窗内收缩，让标题/workspace 省略而非换行。 */
  min-width: 0;
  flex: 0 1 auto;
}
.brand-dot {
  position: relative;
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: #30d158;
  box-shadow: 0 0 0 3px color-mix(in srgb, #30d158 22%, transparent);
  animation: brand-dot-breathe 2.4s ease-in-out infinite;
}
.brand-dot::after {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: 50%;
  background: #30d158;
  animation: brand-dot-ping 2.4s ease-out infinite;
}
@keyframes brand-dot-breathe {
  0%,
  100% {
    opacity: 0.85;
    transform: scale(1);
  }
  50% {
    opacity: 1;
    transform: scale(1.12);
  }
}
@keyframes brand-dot-ping {
  0% {
    opacity: 0.5;
    transform: scale(1);
  }
  70%,
  100% {
    opacity: 0;
    transform: scale(2.6);
  }
}
@media (prefers-reduced-motion: reduce) {
  .brand-dot,
  .brand-dot::after {
    animation: none;
  }
  .brand-dot::after {
    display: none;
  }
}
.brand-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-primary);
  letter-spacing: 0.1px;
  /* 窄窗时省略而非换行（如「Message from the Loop」），且优先于 workspace 被截断。 */
  min-width: 0;
  flex: 1 1 auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
/* 内联模式（标题中嵌 agent/project 胶囊，如「来自 [Cursor] [Project] 的消息」）：
   文字先于胶囊截断，优先级 后缀 > 前缀 > 项目名（agent 品牌名不缩）。
   用 flex-shrink 权重近似分级：后缀 1000 ≫ 前缀 50 ≫ 项目名 1。 */
.brand.inline .brand-title {
  flex: 0 50 auto;
}
.brand.inline .brand-suffix {
  flex: 0 1000 auto;
}
.brand.inline .brand-workspace {
  flex-shrink: 1;
  min-width: 0;
}
/* 头部弹窗时间：灰色小字，紧跟头部之后。空间不足时最先让位——给远高于标题/胶囊的收缩权重
   （100000 ≫ 后缀 1000 ≫ 前缀 50 ≫ 项目 1）+ overflow 裁剪，使其先被压没再动其它元素。 */
.brand-time {
  pointer-events: auto;
  flex: 0 100000 auto;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  font-size: 12px;
  font-weight: 500;
  color: var(--text-secondary);
  opacity: 0.7;
  letter-spacing: 0.1px;
}
/* 标题旁的来源胶囊（agent / workspace）：浅灰底圆角矩形纯文字。
   需 pointer-events:auto 才能 hover 出原生 title / 接收点击（导航栏其余可拖拽）。
   标题先截断、胶囊尽量保留完整：胶囊不参与收缩（flex:0 0 auto）。 */
.brand-chip {
  pointer-events: auto;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 3px;
  max-width: 180px;
  padding: 2px 7px;
  border: none;
  border-radius: 6px;
  background: color-mix(in srgb, var(--text-primary) 8%, transparent);
  font-size: 12px;
  font-weight: 500;
  color: var(--text-secondary);
  letter-spacing: 0.1px;
  font-family: inherit;
  cursor: default;
}
.brand-chip .chip-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.brand-chip.clickable {
  cursor: pointer;
  transition: background 0.12s ease, color 0.12s ease;
}
.brand-chip.clickable:hover {
  background: color-mix(in srgb, var(--text-primary) 14%, transparent);
  color: var(--text-primary);
}
.chip-arrow {
  flex: 0 0 auto;
  width: 10px;
  height: 10px;
  opacity: 0.65;
}
.brand-counter {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}
.nav-actions {
  margin-left: auto;
  display: inline-flex;
  align-items: center;
  gap: 2px;
  pointer-events: none;
}
.nav-btn {
  pointer-events: auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--text-secondary);
  cursor: default;
  transition: background 0.12s ease, color 0.12s ease;
}
.nav-btn:hover {
  background: var(--bg-elevated);
  color: var(--text-primary);
}
.nav-btn.active {
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  color: var(--accent);
}
.nav-btn svg {
  width: 16px;
  height: 16px;
}
/* 版本自更新：入口按钮 + 圆点 + 浮层 + 待生效横条 */
.update-wrap {
  position: relative;
  display: inline-flex;
  pointer-events: auto;
  /* 与右侧「置顶」按钮拉开一点距离 */
  margin-right: 4px;
}
.update-btn {
  position: relative;
  color: var(--accent-orange);
}
.update-dot {
  position: absolute;
  top: 3px;
  right: 3px;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #30d158;
  box-shadow: 0 0 0 2px var(--bg-base, rgba(0, 0, 0, 0.25));
}
.update-backdrop {
  position: fixed;
  inset: 0;
  z-index: 40;
}
.update-popover {
  position: absolute;
  top: 34px;
  right: 0;
  z-index: 50;
  width: 280px;
  max-height: 360px;
  overflow-y: auto;
  /* 用不透明的 --bg：弹窗窗体是毛玻璃，--bg-elevated 仅 3%~6% alpha 会透出底下文字 */
  background: var(--bg);
  border: 1px solid var(--border, rgba(127, 127, 127, 0.2));
  border-radius: 10px;
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.28);
  padding: 12px;
  text-align: left;
}
.up-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0 0 8px;
}
.up-notes {
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-secondary);
  max-height: 180px;
  overflow-y: auto;
  margin: 0 0 8px;
}
.up-notes.muted {
  opacity: 0.7;
}
.up-hint {
  font-size: 11px;
  color: var(--text-secondary);
  margin: 0 0 8px;
}
.up-error {
  font-size: 11px;
  color: #ff453a;
  white-space: pre-wrap;
  margin: 0 0 8px;
}
.up-actions {
  display: flex;
  justify-content: flex-end;
}
.update-pending-banner {
  flex: 0 0 auto;
  font-size: 12px;
  color: var(--text-primary);
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  border-bottom: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
  padding: 8px var(--space-4);
  text-align: center;
}
.popup-status {
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 13px;
  padding: 24px;
  text-align: center;
}
.status-error {
  color: #ff453a;
  white-space: pre-wrap;
}
.content {
  flex: 1 1 auto;
  overflow-y: auto;
  /* 切题滑动时面板水平位移会超出宽度，裁剪掉以免出现横向滚动条 */
  overflow-x: hidden;
  padding: var(--space-4) var(--space-4) var(--space-3);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.options {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
/* message 下方右对齐的小工具条（复制 message + Markdown/源码切换）。
   独立一行、不与正文重叠；按钮矮而非方形，常态淡显、hover/激活时加亮。 */
.msg-tools {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 4px;
  margin-top: calc(-1 * var(--space-2));
  margin-bottom: calc(-1 * var(--space-2));
}
.mt-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 24px;
  padding: 0 9px;
  border: 1px solid var(--border);
  border-radius: 5px;
  background: var(--bg-elevated);
  color: var(--text-secondary);
  cursor: pointer;
  opacity: 0.6;
  transition: opacity 0.12s ease, background 0.12s ease, color 0.12s ease;
}
.mt-btn:hover {
  opacity: 1;
  color: var(--text-primary);
  background: color-mix(in srgb, var(--text-primary) 8%, var(--bg-elevated));
}
.mt-btn.active {
  opacity: 1;
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  border-color: color-mix(in srgb, var(--accent) 40%, transparent);
}
.mt-btn.done {
  opacity: 1;
  color: var(--accent);
  border-color: color-mix(in srgb, var(--accent) 40%, transparent);
}
.mt-ico {
  width: 13px;
  height: 13px;
}
.mt-check {
  display: none;
}
.mt-btn.done .mt-copy {
  display: none;
}
.mt-btn.done .mt-check {
  display: inline;
}
/* 附件区 */
.attachments {
  display: flex;
  flex-direction: column;
  gap: 7px;
}
.att-caption {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.3px;
  color: var(--text-secondary);
}
.att-caption svg {
  width: 13px;
  height: 13px;
}
.att-list {
  display: flex;
  flex-flow: row wrap;
  gap: 8px;
}
.attachment {
  display: inline-flex;
  align-items: center;
  gap: 9px;
  max-width: 100%;
  padding: 5px 12px 5px 5px;
  border: 1px solid transparent;
  border-radius: 999px;
  background: var(--bg-elevated);
  cursor: default;
  outline: none;
  transition: background 0.12s ease, box-shadow 0.12s ease;
}
.attachment:hover {
  background: color-mix(in srgb, var(--text-primary) 8%, var(--bg-elevated));
}
.attachment.selected,
.attachment:focus-visible {
  box-shadow: 0 0 0 2px var(--accent);
}
.att-icon {
  flex: 0 0 auto;
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--accent);
  border-radius: 50%;
  overflow: hidden;
  background: color-mix(in srgb, var(--accent) 16%, transparent);
}
.att-icon.is-image {
  border-radius: 7px;
  background: var(--card-bg);
}
.att-icon img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.att-icon svg {
  width: 15px;
  height: 15px;
}
.att-meta {
  display: inline-flex;
  align-items: baseline;
  gap: 6px;
  min-width: 0;
}
.att-name {
  font-size: 13px;
  color: var(--text-primary);
  max-width: 180px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.att-size {
  flex: 0 0 auto;
  font-size: 11px;
  color: var(--text-secondary);
}
/* 问题头部：问号图标 + 「Question i/n」，与 Message 同样式区隔靠分割线 */
.q-header {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
}
/* 有 Message 时：间距 + 分割线，与上方描述区隔开 */
.q-header.with-divider {
  margin-top: 6px;
  padding-top: 14px;
  border-top: 1px solid var(--border);
}
.q-header .q-icon {
  width: 17px;
  height: 17px;
  color: var(--accent);
}
/* 问题卡片：纵向平铺的单题容器（单问题时无高亮、不折叠，外观同旧版） */
.q-card {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  /* 导航滚到顶时留出呼吸位；与 scroll-spy 判定线对齐 */
  scroll-margin-top: 16px;
  scroll-margin-bottom: 12px;
}
/* 焦点无常驻高亮：当前题（⌘1–9 角标落点）由**滚动**（scroll-spy）+ 点选/键盘导航决定，**已去掉 hover 改焦点**
   （hover 会随滚轮把卡片移到光标下而乱跳）；也**不再有常驻**底色块/竖条（旧版常驻大色块 + 左侧蓝条会随焦点搬家）。
   上一个/下一个/⌘[/⌘]/⌘↵ 跳转时用 `.q-card::before` 给整题「闪一下」（圆角、accent 淡底），一次性 opacity
   1→0 渐隐（加 `.flash` 类触发，见 script flashCard）。 */
.q-card::before {
  content: "";
  position: absolute;
  /* 上/右/下维持不变（勿覆盖题间分割线）；只把**左边**外扩到 -9px：比 -6px 多一点蓝、又不贴窗口边框（留 ~7px 呼吸位）。 */
  inset: 8px -6px -6px -9px;
  border-radius: 8px;
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  pointer-events: none;
  z-index: 0;
  opacity: 0; /* 平时不可见；仅 .flash 动画期间短暂显现后渐隐 */
}
/* 内容压在闪光层之上，保证可点击/可读 */
.q-card > * {
  position: relative;
  z-index: 1;
}
.q-card.flash::before {
  animation: q-flash 0.55s ease-out;
}
@keyframes q-flash {
  from {
    opacity: 1;
  }
  to {
    opacity: 0;
  }
}
@media (prefers-reduced-motion: reduce) {
  .q-card.flash::before {
    animation: none;
  }
}
/* 底部哨兵：零高度标记，进视口即判该题「已看到」 */
.q-sentinel {
  height: 1px;
  margin-top: -1px;
  pointer-events: none;
}
/* 多问题折叠态输入框：真·单行（聚焦或有内容时由 expandedQ 还原成多行高度）。
   折叠态不显示麦克风/图片按钮，故无需底部留白；配合 rows="1" + height:auto 得 1 行。
   scoped 提升特异性以覆盖全局 .textarea。 */
.textarea.collapsed {
  min-height: 0;
  height: auto;
  padding-top: 8px;
  padding-bottom: 8px;
  overflow: hidden;
  white-space: nowrap;
}
/* 旧版（顺序模式）单题面板容器 + 上一个/下一个左右滑动（out-in） */
.question-pane {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.q-slide-next-enter-active,
.q-slide-next-leave-active,
.q-slide-prev-enter-active,
.q-slide-prev-leave-active {
  transition: transform 0.14s ease, opacity 0.14s ease;
}
.q-slide-next-enter-from {
  transform: translateX(26px);
  opacity: 0;
}
.q-slide-next-leave-to {
  transform: translateX(-26px);
  opacity: 0;
}
.q-slide-prev-enter-from {
  transform: translateX(-26px);
  opacity: 0;
}
.q-slide-prev-leave-to {
  transform: translateX(26px);
  opacity: 0;
}
@media (prefers-reduced-motion: reduce) {
  .q-slide-next-enter-active,
  .q-slide-next-leave-active,
  .q-slide-prev-enter-active,
  .q-slide-prev-leave-active {
    transition: none;
  }
}
/* 输入框容器：承载内置「添加图片」图标 */
.input-wrap {
  position: relative;
  display: flex;
}
.img-btn {
  position: absolute;
  right: 8px;
  bottom: 8px;
  width: 26px;
  height: 26px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--text-secondary);
  cursor: default;
  transition: background 0.12s ease, color 0.12s ease;
}
.img-btn:hover {
  background: var(--bg-elevated);
  color: var(--text-primary);
}
.img-btn svg {
  width: 17px;
  height: 17px;
}
/* 语音输入按钮：与图片按钮并列，置于其左侧 */
.mic-btn {
  position: absolute;
  right: 38px;
  bottom: 8px;
  width: 26px;
  height: 26px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--text-secondary);
  cursor: default;
  transition: background 0.12s ease, color 0.12s ease;
}
.mic-btn:hover {
  background: var(--bg-elevated);
  color: var(--text-primary);
}
.mic-btn svg {
  width: 17px;
  height: 17px;
}
/* 准备中（loading）：初始化/下载模型期间显示 iOS 风格转圈（渐隐拖尾环），未真正录制 */
.mic-btn.loading {
  color: var(--text-secondary);
}
.mic-btn.loading svg {
  display: none;
}
.mic-btn.loading::before {
  content: "";
  width: 15px;
  height: 15px;
  border-radius: 50%;
  background: conic-gradient(
    from 0deg,
    color-mix(in srgb, currentColor 8%, transparent),
    currentColor
  );
  -webkit-mask: radial-gradient(
    farthest-side,
    transparent calc(100% - 2.4px),
    #000 calc(100% - 2.4px)
  );
  mask: radial-gradient(
    farthest-side,
    transparent calc(100% - 2.4px),
    #000 calc(100% - 2.4px)
  );
  animation: mic-spin 0.7s linear infinite;
}
@keyframes mic-spin {
  to {
    transform: rotate(360deg);
  }
}
/* 录音中：实心蓝（同发送按钮）+ 白色图标 + 透明度呼吸（不缩放） */
.mic-btn.recording,
.mic-btn.recording:hover {
  color: #fff;
  background: var(--accent);
  animation: mic-breathe 1.6s ease-in-out infinite;
}
@keyframes mic-breathe {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.82;
  }
}
@media (prefers-reduced-motion: reduce) {
  .mic-btn.recording,
  .mic-btn.loading::before {
    animation: none;
  }
}
.speech-error {
  margin: 6px 2px 0;
  font-size: 12px;
  color: #ff453a;
}
.speech-status {
  margin: 6px 2px 0;
  font-size: 12px;
  color: var(--text-muted, #8e8e93);
}
.footer {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  border-top: 1px solid var(--border);
  background: transparent;
}
/* R6 一次性引导提示条（页脚上方） */
.im-tip {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 6px var(--space-4);
  border-top: 1px solid var(--border);
  background: color-mix(in srgb, var(--accent, #0a84ff) 8%, transparent);
  font-size: 12px;
}
.im-tip-text {
  flex: 1 1 auto;
  color: var(--text-secondary);
}
.im-tip-action {
  flex: 0 0 auto;
  border: none;
  background: transparent;
  padding: 2px 4px;
  font-size: 12px;
  font-weight: 500;
  color: var(--accent, #0a84ff);
  cursor: default;
}
.im-tip-action:hover {
  text-decoration: underline;
}
.im-tip-close {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: var(--text-muted, #8e8e93);
  cursor: default;
}
.im-tip-close:hover {
  background: rgba(128, 128, 128, 0.2);
  color: var(--text-secondary);
}
.footer .spacer {
  flex: 1 1 auto;
  pointer-events: none;
}
/* 回复文件 chip */
.reply-files {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
}
.reply-file {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 220px;
  padding: 4px 6px 4px 8px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-elevated);
  font-size: 12px;
}
.reply-file .rf-icon {
  flex: 0 0 auto;
  font-size: 13px;
  line-height: 1;
}
.reply-file .rf-name {
  flex: 1 1 auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-primary);
}
.reply-file .rf-remove {
  flex: 0 0 auto;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  border: none;
  background: rgba(128, 128, 128, 0.25);
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1;
  cursor: default;
  display: flex;
  align-items: center;
  justify-content: center;
}
.reply-file .rf-remove:hover {
  background: rgba(128, 128, 128, 0.4);
  color: var(--text-primary);
}

/* 选项末尾的快捷键标注（⌘1…⌘9） */
.option .opt-sc {
  flex: 0 0 auto;
  align-self: center;
  margin-left: 4px;
  font-family: inherit;
  font-size: 11px;
  line-height: 1;
  color: var(--text-secondary);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: 5px;
  padding: 3px 5px;
  opacity: 0.85;
}
.option.selected .opt-sc {
  color: var(--accent);
  border-color: color-mix(in srgb, var(--accent) 40%, transparent);
}

/* 按钮上的快捷键标注 */
.btn .sc {
  margin-left: 6px;
  font-size: 11px;
  line-height: 1;
  opacity: 0.75;
  font-family: inherit;
  border: none;
  background: transparent;
  padding: 0;
}
.btn-primary .sc {
  opacity: 0.85;
}
/* 按住 ⌘/Ctrl：高亮可用的快捷键 Badge（提示此刻按数字即可选项 / 按括号可切题）。
   仅当前题渲染了 .opt-sc，故天然只高亮当前题的选项角标。 */
.option .opt-sc {
  transition: color 0.1s ease, background 0.1s ease, border-color 0.1s ease,
    opacity 0.1s ease;
}
.popup.cmd-held .option .opt-sc {
  opacity: 1;
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
}
.popup.cmd-held .btn:not(:disabled) .sc {
  opacity: 1;
}

/* 取消二次确认弹层 */
.confirm-overlay {
  position: fixed;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.32);
  z-index: 50;
}
.confirm-box {
  width: min(320px, 84%);
  /* 用不透明 --bg：弹窗是毛玻璃，--card-bg 仅 3~6% alpha 会透出底下内容 */
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-md, 12px);
  padding: 18px 18px 14px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.28);
  text-align: center;
}
.confirm-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0 0 6px;
}
.confirm-desc {
  font-size: 12px;
  color: var(--text-secondary);
  margin: 0 0 16px;
}
.confirm-actions {
  display: flex;
  gap: 10px;
  justify-content: center;
}
.btn-danger {
  color: #fff;
  background: #ff453a;
  border-color: transparent;
}
.btn-danger:hover {
  background: #e23b31;
}
</style>
