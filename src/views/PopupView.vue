<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  popupInit,
  submitPopup,
  cancelPopup,
  openSettings,
  updateTheme,
  openPath,
  previewAttachments,
  closePreview,
  readImageDataUrl,
  fileIconDataUrl,
  showAttachmentMenu,
} from "../lib/ipc";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { renderMarkdown } from "../lib/markdown";
import { applyTheme, fileToDataUrl } from "../lib/theme";
import type {
  AskRequest,
  FileAttachment,
  ImageAttachment,
  Question,
  QuestionAnswer,
  ThemeMode,
} from "../lib/types";

const request = ref<AskRequest | null>(null);
const loadError = ref<string | null>(null);

// 当前展示的问题索引（0 始）。
const current = ref(0);

// 按题保存的作答状态（长度与问题数一致）。
const chosenByQ = ref<string[][]>([]);
const inputByQ = ref<string[]>([]);
const imagesByQ = ref<ImageAttachment[][]>([]);
const replyFilesByQ = ref<{ path: string; name: string }[][]>([]);
// 每题是否已被「查看过」。
const visited = ref<boolean[]>([]);

const submitting = ref(false);
const inputRef = ref<HTMLTextAreaElement | null>(null);
const fileRef = ref<HTMLInputElement | null>(null);
const scrolled = ref(false);
// 取消二次确认（已有部分回答时）。
const showCancelConfirm = ref(false);

function onScroll(e: Event) {
  scrolled.value = (e.target as HTMLElement).scrollTop > 0;
}

const pinned = ref(false);
const theme = ref<ThemeMode>("system");
const sourceName = ref("the Loop");

const questions = computed<Question[]>(() => request.value?.questions ?? []);
const total = computed(() => questions.value.length);
const isMulti = computed(() => total.value > 1);
const currentQuestion = computed<Question | null>(
  () => questions.value[current.value] ?? null
);
// 共享 Message（描述 + 附件）。无 -q 时 text 为空（第一个参数已提升为问题）。
const messageText = computed(() => request.value?.message.text ?? "");
const messageHtml = computed(() =>
  request.value?.isMarkdown ? renderMarkdown(messageText.value) : ""
);
const showDescription = computed(
  () => messageText.value.trim() !== "" || attachments.value.length > 0
);
// 存在 Message（描述/附件）或多题时，显示问题头部以区隔 Message 与 Question。
const showQuestionHeader = computed(() => showDescription.value || isMulti.value);
// 多题显示「Question i/n」；单题（仅在有 Message 时显示头部）只显示「Question」。
const questionHeaderLabel = computed(() =>
  isMulti.value ? `Question ${current.value + 1}/${total.value}` : "Question"
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
let unlistenIndex: UnlistenFn | null = null;
let unlistenFocus: UnlistenFn | null = null;
let unlistenDrop: UnlistenFn | null = null;

function setAttRef(el: Element | null, i: number) {
  if (el) attRefs.value[i] = el as HTMLElement;
}

function selectFile(index: number) {
  focusAttachment(index);
}

function openFile(file: FileAttachment) {
  openPath(file.path).catch(() => {});
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

const renderedHtml = computed(() =>
  request.value?.isMarkdown && currentQuestion.value
    ? renderMarkdown(currentQuestion.value.message)
    : ""
);

function toggle(option: string) {
  const arr = chosenByQ.value[current.value];
  if (!arr) return;
  const i = arr.indexOf(option);
  if (i >= 0) arr.splice(i, 1);
  else arr.push(option);
}

function pickFiles() {
  fileRef.value?.click();
}

async function addFiles(files: FileList | File[]) {
  for (const file of Array.from(files)) {
    if (!file.type.startsWith("image/")) continue;
    const data = await fileToDataUrl(file);
    imagesByQ.value[current.value]?.push({
      data,
      mediaType: file.type,
      filename: file.name,
    });
  }
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
  const attachPaths = new Set(attachments.value.map((a) => a.path));
  for (const path of paths) {
    if (attachPaths.has(path)) continue;
    const name = path.split(/[\\/]/).pop() || "file";
    if (IMAGE_EXT.test(path)) {
      try {
        const data = await readImageDataUrl(path);
        const semi = data.indexOf(";");
        const mediaType = semi > 5 ? data.slice(5, semi) : "image/png";
        imagesByQ.value[current.value]?.push({ data, mediaType, filename: name });
      } catch (err) {
        console.error("读取拖入图片失败", path, err);
      }
    } else if (!replyFiles.value.some((f) => f.path === path)) {
      replyFilesByQ.value[current.value]?.push({ path, name });
    }
  }
}

function removeReplyFile(index: number) {
  replyFilesByQ.value[current.value]?.splice(index, 1);
}

async function onPaste(e: ClipboardEvent) {
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

function goTo(index: number) {
  if (index < 0 || index >= total.value || index === current.value) return;
  stopPreview();
  selectedFile.value = null;
  current.value = index;
  markVisited(index);
  requestAnimationFrame(() => {
    inputRef.value?.focus({ preventScroll: true });
    autoGrow();
  });
}

function goPrev() {
  slideDir.value = "prev";
  goTo(current.value - 1);
}

function goNext() {
  slideDir.value = "next";
  goTo(current.value + 1);
}

// 问题切换动画完成后聚焦输入框并校正高度（out-in 下新元素挂载在此时）。
function onQuestionEntered() {
  inputRef.value?.focus({ preventScroll: true });
  autoGrow();
}

function collectAnswers(): QuestionAnswer[] {
  return questions.value.map((q, i) => ({
    selectedOptions: q.predefinedOptions.filter((o) =>
      (chosenByQ.value[i] ?? []).includes(o)
    ),
    userInput: inputByQ.value[i] ?? "",
    images: imagesByQ.value[i] ?? [],
    files: (replyFilesByQ.value[i] ?? []).map((f) => f.path),
  }));
}

async function submit() {
  if (submitting.value) return;
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

function onKeydown(e: KeyboardEvent) {
  if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
    e.preventDefault();
    if (isMulti.value) {
      if (allViewed.value) submit();
      else goNext();
    } else {
      submit();
    }
    return;
  }
  if ((e.metaKey || e.ctrlKey) && (e.key === "w" || e.key === "W")) {
    e.preventDefault();
    requestCancel();
    return;
  }
  const tgt = e.target as HTMLElement | null;
  const typing =
    tgt && (tgt.tagName === "TEXTAREA" || tgt.tagName === "INPUT");
  if (!typing) handleAttachmentKey(e);
}

onMounted(async () => {
  window.addEventListener("paste", onPaste);
  window.addEventListener("keydown", onKeydown);
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
  try {
    const init = await popupInit();
    applyTheme(init.theme);
    theme.value = init.theme;
    pinned.value = init.alwaysOnTop;
    sourceName.value = init.sourceName;
    request.value = init.request;
    const n = init.request.questions.length;
    chosenByQ.value = Array.from({ length: n }, () => []);
    inputByQ.value = Array.from({ length: n }, () => "");
    imagesByQ.value = Array.from({ length: n }, () => []);
    replyFilesByQ.value = Array.from({ length: n }, () => []);
    visited.value = Array.from({ length: n }, () => false);
    if (n > 0) visited.value[0] = true;
    loadThumbs();
    loadDragIcons();
    requestAnimationFrame(() => {
      inputRef.value?.focus({ preventScroll: true });
      autoGrow();
    });
  } catch (err) {
    console.error("popup_init 失败", err);
    loadError.value = String(err);
  }
});

onBeforeUnmount(() => {
  window.removeEventListener("paste", onPaste);
  window.removeEventListener("keydown", onKeydown);
  unlistenIndex?.();
  unlistenFocus?.();
  unlistenDrop?.();
});
</script>

<template>
  <div v-if="!request" class="popup popup-status">
    <p v-if="loadError" class="status-error">加载失败：{{ loadError }}</p>
    <p v-else class="status-loading">加载中…</p>
  </div>

  <div
    v-else
    class="popup"
    @dragover.prevent
    @drop.prevent="onDrop"
    @click="onBackgroundClick"
  >
    <header class="navbar" :class="{ scrolled }" data-tauri-drag-region>
      <span class="brand">
        <span class="brand-dot"></span>
        <span class="brand-title">Question from {{ sourceName }}</span>
      </span>
      <span class="nav-actions">
        <button
          class="nav-btn"
          :class="{ active: pinned }"
          type="button"
          title="窗口置顶"
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
          title="切换主题"
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
          title="设置"
          @click="openSettingsWindow"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.6 1.6 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.6 1.6 0 0 0-1.8-.3 1.6 1.6 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1a1.6 1.6 0 0 0-1-1.5 1.6 1.6 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.6 1.6 0 0 0 .3-1.8 1.6 1.6 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1a1.6 1.6 0 0 0 1.5-1 1.6 1.6 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.6 1.6 0 0 0 1.8.3H9a1.6 1.6 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.6 1.6 0 0 0 1 1.5 1.6 1.6 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.6 1.6 0 0 0-.3 1.8V9a1.6 1.6 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.6 1.6 0 0 0-1.5 1z" />
          </svg>
        </button>
      </span>
    </header>
    <div class="content" @scroll="onScroll">
      <!-- 共享 Message 区（描述 + 附件），仅在有内容时展示，顶部常驻 -->
      <template v-if="showDescription">
        <div
          v-if="messageText && request.isMarkdown"
          class="markdown-body"
          v-html="messageHtml"
        ></div>
        <pre v-else-if="messageText" class="plain-body">{{ messageText }}</pre>

        <div v-if="attachments.length" class="attachments">
        <div class="att-caption">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48" />
          </svg>
          <span>附件 · {{ attachments.length }}</span>
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
          ></div>
          <pre v-else-if="currentQuestion?.message" class="plain-body">{{ currentQuestion?.message }}</pre>

          <div v-if="currentQuestion && currentQuestion.predefinedOptions.length" class="options">
            <div
              v-for="(opt, i) in currentQuestion.predefinedOptions"
              :key="i"
              class="option"
              :class="{ selected: chosen.includes(opt) }"
              @click="toggle(opt)"
            >
              <span class="check">{{ chosen.includes(opt) ? "✓" : "" }}</span>
              <span class="label">{{ opt }}</span>
            </div>
          </div>

          <!-- 输入框 + 内置「添加图片」小图标（右下角） -->
          <div class="input-wrap">
            <textarea
              ref="inputRef"
              v-model="userInput"
              class="textarea"
              placeholder="输入你的回复…"
              @input="autoGrow"
            ></textarea>
            <button
              class="img-btn"
              type="button"
              title="添加图片"
              aria-label="添加图片"
              @click="pickFiles"
            >
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                <rect x="3" y="3" width="18" height="18" rx="2" />
                <circle cx="8.5" cy="8.5" r="1.6" />
                <path d="M21 15l-5-5L5 21" />
              </svg>
            </button>
          </div>

          <div v-if="images.length" class="thumbs">
            <div v-for="(img, i) in images" :key="i" class="thumb">
              <img :src="img.data" alt="" />
              <button class="remove" type="button" @click="removeImage(i)">
                ×
              </button>
            </div>
          </div>

          <div v-if="replyFiles.length" class="reply-files">
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
        取消 <kbd class="sc">⌘W</kbd>
      </button>
      <span class="spacer"></span>
      <button
        class="btn"
        type="button"
        :disabled="submitting || current === 0"
        @click="goPrev"
      >
        上一个
      </button>
      <button
        class="btn"
        :class="{ 'btn-primary': !allViewed }"
        type="button"
        :disabled="submitting || current === total - 1"
        @click="goNext"
      >
        下一个 <kbd v-if="!allViewed" class="sc">⌘↵</kbd>
      </button>
      <button
        v-if="allViewed"
        class="btn btn-primary"
        type="button"
        :disabled="submitting"
        @click="submit"
      >
        提交 <kbd class="sc">⌘↵</kbd>
      </button>
    </div>

    <!-- 单问题底部：取消(左) + 发送(右) -->
    <div v-else class="footer" data-tauri-drag-region>
      <button class="btn" type="button" :disabled="submitting" @click="requestCancel">
        取消 <kbd class="sc">⌘W</kbd>
      </button>
      <span class="spacer"></span>
      <button
        class="btn btn-primary"
        type="button"
        :disabled="submitting"
        @click="submit"
      >
        发送 <kbd class="sc">⌘↵</kbd>
      </button>
    </div>

    <!-- 取消二次确认 -->
    <div v-if="showCancelConfirm" class="confirm-overlay" @click.self="dismissCancelConfirm">
      <div class="confirm-box">
        <p class="confirm-title">确定要取消吗？</p>
        <p class="confirm-desc">已填写的回答将全部丢失。</p>
        <div class="confirm-actions">
          <button class="btn" type="button" @click="dismissCancelConfirm">
            继续作答
          </button>
          <button class="btn btn-danger" type="button" @click="doCancel">
            确定取消
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
  background: var(--card-bg, var(--bg-elevated));
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
