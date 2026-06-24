<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  popupInit,
  submitPopup,
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
const loadError = ref<string | null>(null);

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
const inputRef = ref<HTMLTextAreaElement | null>(null);
const fileRef = ref<HTMLInputElement | null>(null);
const qHeaderRef = ref<HTMLElement | null>(null);
const thumbsRef = ref<HTMLElement | null>(null);
const scrolled = ref(false);
// 取消二次确认（已有部分回答时）。
const showCancelConfirm = ref(false);

function onScroll(e: Event) {
  scrolled.value = (e.target as HTMLElement).scrollTop > 0;
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
// 严格选择：隐藏补充输入 / 附件区，且必须选中才能提交（D11）。
const selectOnly = computed(() => request.value?.selectOnly ?? false);
// 单选：选项渲染为 radio，每题恰好一个（D11）。
const single = computed(() => request.value?.single ?? false);
const currentQuestion = computed<Question | null>(
  () => questions.value[current.value] ?? null
);
// 共享 Message（描述 + 附件）。无 -q 时 text 为空（第一个参数已提升为问题）。
const messageText = computed(() => request.value?.message.text ?? "");
const messageHtml = computed(() =>
  request.value?.isMarkdown
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
// 多题显示「Question i/n」；单题（仅在有 Message 时显示头部）只显示「Question」。
const questionHeaderLabel = computed(() =>
  isMulti.value
    ? t("popup.question.indexed", { i: current.value + 1, n: total.value })
    : t("popup.question.single")
);
// 上一个/下一个的切换方向，驱动左右滑动动画。
const slideDir = ref<"next" | "prev">("next");
const transitionName = computed(() =>
  slideDir.value === "next" ? "q-slide-next" : "q-slide-prev"
);
const allViewed = computed(
  () => visited.value.length > 0 && visited.value.every(Boolean)
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

// ===== 当前题的作答视图（读写当前索引） =====
const chosen = computed(() => chosenByQ.value[current.value] ?? []);
const userInput = computed({
  get: () => inputByQ.value[current.value] ?? "",
  set: (v: string) => {
    inputByQ.value[current.value] = v;
  },
});
const images = computed(() => imagesByQ.value[current.value] ?? []);
const replyFiles = computed(() => replyFilesByQ.value[current.value] ?? []);

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

function openHistoryWindow() {
  openHistory().catch(() => {});
}

const renderedHtml = computed(() =>
  request.value?.isMarkdown && currentQuestion.value
    ? renderMarkdown(currentQuestion.value.message, codeCopyLabels.value)
    : ""
);

function toggle(option: string) {
  const arr = chosenByQ.value[current.value];
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

// 通过序号（0 始）切换当前题的选项，供 CMD+数字 调用。
function toggleByIndex(i: number) {
  const opts = currentQuestion.value?.predefinedOptions;
  if (!opts || i < 0 || i >= opts.length) return;
  toggle(opts[i].text);
}

function pickFiles() {
  fileRef.value?.click();
}

async function addFiles(files: FileList | File[]) {
  if (selectOnly.value) return;
  let added = 0;
  for (const file of Array.from(files)) {
    if (!file.type.startsWith("image/")) continue;
    const data = await fileToDataUrl(file);
    imagesByQ.value[current.value]?.push({
      data,
      mediaType: file.type,
      filename: file.name,
    });
    added++;
  }
  if (added) scrollImagesIntoView();
}

// 新增图片后把最新缩略图滚入可见区：粘贴/选择时即使内容已上滚，也能立刻确认成功。
async function scrollImagesIntoView() {
  await nextTick();
  const wrap = thumbsRef.value;
  if (!wrap) return;
  const last = (wrap.lastElementChild as HTMLElement | null) ?? wrap;
  const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  last.scrollIntoView({ block: "nearest", behavior: reduce ? "auto" : "smooth" });
}

function onFileChange(e: Event) {
  const input = e.target as HTMLInputElement;
  if (input.files) addFiles(input.files);
  input.value = "";
}

function removeImage(index: number) {
  imagesByQ.value[current.value]?.splice(index, 1);
}

function onDrop(_e: DragEvent) {}

const IMAGE_EXT = /\.(png|jpe?g|gif|webp|bmp|heic|heif|tiff?|svg)$/i;

async function addDroppedPaths(paths: string[]) {
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
        imagesByQ.value[current.value]?.push({ data, mediaType, filename: name });
        addedImage++;
      } catch (err) {
        console.error("读取拖入图片失败", path, err);
      }
    } else if (!replyFiles.value.some((f) => f.path === path)) {
      replyFilesByQ.value[current.value]?.push({ path, name });
    }
  }
  if (addedImage) scrollImagesIntoView();
}

function removeReplyFile(index: number) {
  replyFilesByQ.value[current.value]?.splice(index, 1);
}

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
    await addFiles(files);
  }
}

// 输入框随内容自增高（封顶 240px，超出则框内滚动；底部留白由 CSS padding 提供）。
const MAX_TEXTAREA_H = 240;
function autoGrow() {
  const el = inputRef.value;
  if (!el) return;
  el.style.height = "auto";
  el.style.height = `${Math.min(el.scrollHeight, MAX_TEXTAREA_H)}px`;
}

// ===== 多题导航 =====
function markVisited(i: number) {
  if (i >= 0 && i < visited.value.length) visited.value[i] = true;
}

// 切题时把问题头部滚到可见区顶部：Message 很长时也能露出当前问题。
function scrollQuestionIntoView() {
  const el = qHeaderRef.value;
  if (!el) return;
  const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  el.scrollIntoView({ block: "start", behavior: reduce ? "auto" : "smooth" });
}

function goTo(index: number) {
  if (index < 0 || index >= total.value || index === current.value) return;
  // 切题前先停掉语音会话：否则录音回调仍写入开始录音时锁定的旧题。
  stopListening();
  stopPreview();
  selectedFile.value = null;
  current.value = index;
  markVisited(index);
}

function goPrev() {
  slideDir.value = "prev";
  goTo(current.value - 1);
}

function goNext() {
  slideDir.value = "next";
  goTo(current.value + 1);
}

// 问题切换动画完成后再聚焦/校正高度/滚动：此时新面板已挂载、高度确定，
// 避免新旧面板高度不同导致的上下跳动。
function onQuestionEntered() {
  inputRef.value?.focus({ preventScroll: true });
  autoGrow();
  scrollQuestionIntoView();
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

function onKeydown(e: KeyboardEvent) {
  const mod = e.metaKey || e.ctrlKey;
  // 录音中按 Esc：结束本次语音输入（不关闭弹窗）。
  if (e.key === "Escape" && listening.value) {
    e.preventDefault();
    stopListening();
    return;
  }
  if (mod && e.key === "Enter") {
    e.preventDefault();
    // 多题：非最后一题始终前往下一题（即使提交按钮已出现），最后一题才提交。
    if (isMulti.value && !onLastQuestion.value) goNext();
    else submit();
    return;
  }
  if (mod && (e.key === "w" || e.key === "W")) {
    e.preventDefault();
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
  // 多题：CMD+] 下一题，CMD+[ 上一题（不影响 CMD+回车）。
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
  const req = init.request;
  if (!req) return;
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
  // 语音设置改取自 popup_init（不再走 get_settings / 钥匙串）。
  speechLang.value = init.speechLanguage || "auto";
  speechShortcut.value = init.speechShortcut || "cmd+d";
  request.value = req;
  const n = req.questions.length;
  chosenByQ.value = Array.from({ length: n }, () => []);
  inputByQ.value = Array.from({ length: n }, () => "");
  imagesByQ.value = Array.from({ length: n }, () => []);
  replyFilesByQ.value = Array.from({ length: n }, () => []);
  visited.value = Array.from({ length: n }, () => false);
  if (n > 0) visited.value[0] = true;
  loadThumbs();
  loadDragIcons();
  // 首帧之后：聚焦 + 打点 + （harness）自动取消。冷热路径共用。
  const afterPaint = () => {
    inputRef.value?.focus({ preventScroll: true });
    autoGrow();
    // 双 rAF：第一帧让正文进入 DOM 并即将绘制，第二帧回调时该帧已真正合成上屏，
    // 此刻打点更贴近用户真正看到内容的时刻（比单 rAF 晚约 1 帧）。
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        perfMarkFe("fe.painted");
        // harness 模式：内容已上屏即自动取消，免人工点按。
        if (init.perfAutodismiss) {
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
      afterPaint();
    });
  } else {
    // 冷路径：窗口已在 setup 中显示，rAF 正常回调。
    afterPaint();
  }
  // 内容已渲染：把其余初始化（事件监听 / 语音 / 自更新 / 终端探测）放到首帧之后，不阻塞首屏。
  void initAfterPaint(init);
}

// 预热弹窗领用：重新 pull popup_init，若已带 request 则渲染（一次性）。
async function adopt() {
  if (request.value || adopting) return;
  adopting = true;
  try {
    const init = await popupInit();
    if (init.request) {
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
  // 同步窗口监听（开销极小）：放最前，保证粘贴 / 快捷键 / Esc 从首帧即可用。
  window.addEventListener("paste", onPaste);
  window.addEventListener("keydown", onKeydown);
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
    if (init.request) {
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
    addDroppedPaths(event.payload.paths);
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
    requestCancel();
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
  if (flashTimer) window.clearTimeout(flashTimer);
  stopListening();
  unlistenSpeech.forEach((fn) => fn());
  unlistenSpeech = [];
  if (speechErrorTimer) clearTimeout(speechErrorTimer);
});
</script>

<template>
  <div v-if="!request" class="popup popup-status">
    <p v-if="loadError" class="status-error">
      {{ t("popup.loadError", { msg: loadError }) }}
    </p>
    <p v-else class="status-loading">{{ t("popup.loading") }}</p>
  </div>

  <div
    v-else
    class="popup"
    @dragover.prevent
    @drop.prevent="onDrop"
    @click="onBackgroundClick"
  >
    <div v-if="flashing" class="flash-overlay" aria-hidden="true"></div>
    <header class="navbar" :class="{ scrolled }" data-tauri-drag-region>
      <span class="brand">
        <span class="brand-dot"></span>
        <span class="brand-title">{{ headerTitle }}</span>
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
    <div class="content" @scroll="onScroll">
      <!-- 共享 Message 区（描述 + 附件），仅在有内容时展示，顶部常驻 -->
      <template v-if="showDescription">
        <div
          v-if="messageText && request.isMarkdown"
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
            v-if="request.isMarkdown && currentQuestion?.message"
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
              @click="toggle(opt.text)"
            >
              <span class="check" :class="{ radio: single }">{{ single ? "" : (chosen.includes(opt.text) ? "✓" : "") }}</span>
              <span class="label"><span v-if="opt.recommended" class="rec-badge"><span class="rec-badge-pill"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3z"></path><path d="M7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"></path></svg>{{ t("popup.recommended") }}</span></span>{{ opt.text }}</span>
              <kbd v-if="optionHotkey(i)" class="opt-sc">{{ optionHotkey(i) }}</kbd>
            </div>
          </div>

          <!-- 输入框 + 内置「添加图片」小图标（右下角）；严格选择模式隐藏 -->
          <div v-if="!selectOnly" class="input-wrap">
            <textarea
              ref="inputRef"
              v-model="userInput"
              class="textarea"
              :placeholder="t('popup.inputPlaceholder')"
              @input="autoGrow"
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
              @click="pickFiles"
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

          <div v-if="!selectOnly && images.length" ref="thumbsRef" class="thumbs">
            <div v-for="(img, i) in images" :key="i" class="thumb">
              <img :src="img.data" alt="" />
              <button class="remove" type="button" @click="removeImage(i)">
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
              <button class="rf-remove" type="button" @click="removeReplyFile(i)">
                ×
              </button>
            </div>
          </div>
        </div>
      </Transition>
    </div>

    <input
      ref="fileRef"
      type="file"
      accept="image/*"
      multiple
      hidden
      @change="onFileChange"
    />

    <!-- 多问题底部：取消(左) + 上一个/下一个/提交(右) -->
    <div v-if="isMulti" class="footer" data-tauri-drag-region>
      <button class="btn" type="button" :disabled="submitting" @click="requestCancel">
        {{ t("common.cancel") }} <kbd class="sc">⌘W</kbd>
      </button>
      <span class="spacer"></span>
      <button
        class="btn"
        type="button"
        :disabled="submitting || current === 0"
        @click="goPrev"
      >
        {{ t("popup.prev") }} <kbd v-if="current > 0" class="sc">⌘[</kbd>
      </button>
      <button
        class="btn"
        :class="{ 'btn-primary': !onLastQuestion }"
        type="button"
        :disabled="submitting || current === total - 1"
        @click="goNext"
      >
        {{ t("popup.next") }} <kbd v-if="!onLastQuestion" class="sc">⌘↵</kbd>
      </button>
      <button
        v-if="allViewed"
        class="btn"
        :class="{ 'btn-primary': onLastQuestion }"
        type="button"
        :disabled="submitting || !canSubmit"
        @click="submit"
      >
        {{ t("common.submit") }}
        <kbd v-if="onLastQuestion" class="sc">⌘↵</kbd>
      </button>
    </div>

    <!-- 单问题底部：取消(左) + 发送(右) -->
    <div v-else class="footer" data-tauri-drag-region>
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

    <!-- 取消二次确认 -->
    <div v-if="showCancelConfirm" class="confirm-overlay" @click.self="dismissCancelConfirm">
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
  </div>
</template>

<style scoped>
.popup {
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
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
/* 问题区滑动容器 */
.question-pane {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
/* 上一个/下一个：左右滑动（out-in） */
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
