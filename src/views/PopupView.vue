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
  ThemeMode,
} from "../lib/types";

const request = ref<AskRequest | null>(null);
const loadError = ref<string | null>(null);
const chosen = ref<string[]>([]);
const userInput = ref("");
const images = ref<ImageAttachment[]>([]);
// 用户拖入回复的非图片文件（仅展示路径，提交时透传绝对路径）。
const replyFiles = ref<{ path: string; name: string }[]>([]);
const submitting = ref(false);
const inputRef = ref<HTMLTextAreaElement | null>(null);
const fileRef = ref<HTMLInputElement | null>(null);
const scrolled = ref(false);

function onScroll(e: Event) {
  scrolled.value = (e.target as HTMLElement).scrollTop > 0;
}

const pinned = ref(false);
const theme = ref<ThemeMode>("system");
const sourceName = ref("the Loop");

// 提问附带的文件附件（AI→人，仅展示）。
const attachments = computed<FileAttachment[]>(() => request.value?.files ?? []);
const selectedFile = ref<number | null>(null);
const thumbs = ref<Record<string, string>>({});
// 附件的系统图标 PNG data URL，挂载时预取，拖出时同步作为原生拖拽预览图标。
const dragIcons = ref<Record<string, string>>({});
const attRefs = ref<HTMLElement[]>([]);
// 预览是否打开。打开后，面板保持 key，方向键经原生委托回传 preview-index 事件联动切换。
const previewing = ref(false);
let unlistenIndex: UnlistenFn | null = null;
let unlistenFocus: UnlistenFn | null = null;
let unlistenDrop: UnlistenFn | null = null;

function setAttRef(el: Element | null, i: number) {
  if (el) attRefs.value[i] = el as HTMLElement;
}

function selectFile(index: number) {
  // WebKit 单击 div(tabindex) 默认不赋键盘焦点，需手动 focus，方向键才生效。
  focusAttachment(index);
}

function openFile(file: FileAttachment) {
  openPath(file.path).catch(() => {});
}

function focusAttachment(index: number) {
  selectedFile.value = index;
  attRefs.value[index]?.focus();
}

// 打开预览：预览当前选中项。面板保持 key，后续方向键由原生委托处理并回传索引。
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

// 点击附件以外区域：取消选中并关闭预览。
function onBackgroundClick(e: MouseEvent) {
  if ((e.target as HTMLElement).closest(".attachment")) return;
  if (selectedFile.value !== null) selectedFile.value = null;
  stopPreview();
}

// 附件列表的键盘操作（在全局 keydown 中处理，不依赖具体 div 的 DOM 焦点；
// 只要 WKWebView 是 first responder，事件即会冒泡到 window）。返回是否已处理。
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

// 预取每个附件的系统图标，供拖出到其它应用时作为原生拖拽预览。
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

// 标记正在「拖出」附件：用于让本窗口的原生 drop 监听忽略这次拖拽，
// 避免把拖出的 -f 附件又当作回复输入加回来（只能拖到窗口外）。
const draggingOut = ref(false);

// -f 附件胶囊右键：弹出原生 Finder 风格菜单（打开/打开方式/预览/在访达中显示/拷贝等）。
function onAttachmentContextMenu(file: FileAttachment, i: number, e: MouseEvent) {
  e.preventDefault();
  selectFile(i);
  showAttachmentMenu(file.path).catch((err) =>
    console.error("打开右键菜单失败", err)
  );
}

// -f 附件胶囊拖出到其它应用（如拖到 Dock 应用图标用其打开）。
function onAttachmentDragStart(file: FileAttachment, e: DragEvent) {
  // 阻止 WebView 自带的 HTML5 拖拽，改用 Tauri 原生拖拽会话。
  e.preventDefault();
  const icon = dragIcons.value[file.path] || thumbs.value[file.path] || "";
  draggingOut.value = true;
  startDrag({ item: [file.path], icon }, () => {
    // 结束回调可能早于窗口 drop 事件，延迟复位兜底（路径匹配为主判据）。
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
  request.value?.isMarkdown ? renderMarkdown(request.value.message) : ""
);

function toggle(option: string) {
  const i = chosen.value.indexOf(option);
  if (i >= 0) chosen.value.splice(i, 1);
  else chosen.value.push(option);
}

function pickFiles() {
  fileRef.value?.click();
}

async function addFiles(files: FileList | File[]) {
  for (const file of Array.from(files)) {
    if (!file.type.startsWith("image/")) continue;
    const data = await fileToDataUrl(file);
    images.value.push({ data, mediaType: file.type, filename: file.name });
  }
}

function onFileChange(e: Event) {
  const input = e.target as HTMLInputElement;
  if (input.files) addFiles(input.files);
  input.value = "";
}

function removeImage(index: number) {
  images.value.splice(index, 1);
}

// 浏览器 drop 事件在 Tauri 下拿不到 files（原生层接管），仅保留以阻止默认行为；
// 实际文件拖入经 Tauri 的 onDragDropEvent 处理（见 onMounted）。
function onDrop(_e: DragEvent) {}

const IMAGE_EXT = /\.(png|jpe?g|gif|webp|bmp|heic|heif|tiff?|svg)$/i;

// 拖入的本地文件：图片读为 data URL 作为图片附件；非图片以绝对路径作为回复文件附件。
async function addDroppedPaths(paths: string[]) {
  // -f 附件路径集合：拖出附件若落回窗口内，路径会与之相同，直接跳过（确定性判断，不依赖时序）。
  const attachPaths = new Set(attachments.value.map((a) => a.path));
  for (const path of paths) {
    if (attachPaths.has(path)) continue;
    const name = path.split(/[\\/]/).pop() || "file";
    if (IMAGE_EXT.test(path)) {
      try {
        const data = await readImageDataUrl(path);
        const semi = data.indexOf(";");
        const mediaType = semi > 5 ? data.slice(5, semi) : "image/png";
        images.value.push({ data, mediaType, filename: name });
      } catch (err) {
        console.error("读取拖入图片失败", path, err);
      }
    } else if (!replyFiles.value.some((f) => f.path === path)) {
      replyFiles.value.push({ path, name });
    }
  }
}

function removeReplyFile(index: number) {
  replyFiles.value.splice(index, 1);
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

async function send() {
  if (submitting.value) return;
  submitting.value = true;
  const opts = request.value?.predefinedOptions ?? [];
  const selectedOptions = opts.filter((o) => chosen.value.includes(o));
  try {
    await submitPopup({
      selectedOptions,
      userInput: userInput.value,
      images: images.value,
      files: replyFiles.value.map((f) => f.path),
    });
  } catch {
    submitting.value = false;
  }
}

async function cancel() {
  if (submitting.value) return;
  submitting.value = true;
  try {
    await cancelPopup();
  } catch {
    submitting.value = false;
  }
}

function onKeydown(e: KeyboardEvent) {
  if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
    e.preventDefault();
    send();
    return;
  }
  // 取消改用 ⌘/Ctrl+W（不再用 Esc，避免误触关闭弹窗）。
  if ((e.metaKey || e.ctrlKey) && (e.key === "w" || e.key === "W")) {
    e.preventDefault();
    cancel();
    return;
  }
  // 在文本输入框内不拦截方向键/空格（让光标正常移动、输入空格）。
  const tgt = e.target as HTMLElement | null;
  const typing =
    tgt && (tgt.tagName === "TEXTAREA" || tgt.tagName === "INPUT");
  if (!typing) handleAttachmentKey(e);
}

onMounted(async () => {
  window.addEventListener("paste", onPaste);
  window.addEventListener("keydown", onKeydown);
  // 面板内方向键切换时，原生委托回传新索引 → 同步高亮 + 同步把 DOM 焦点移到该项，
  // 避免「最初点击项仍持有 :focus 焦点环、当前项又有 .selected」的双焦点。
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
  // 面板关闭（Esc/点击外部）经 endPreviewPanelControl 回传 → 复位预览状态，
  // 并把 DOM 焦点落在当前选中项，保证只有单一焦点。
  unlistenFocus = await listen("preview-closed", () => {
    previewing.value = false;
    const i = selectedFile.value;
    if (i !== null) nextTick(() => attRefs.value[i]?.focus());
  });
  // 原生文件拖放：Tauri 接管 OS 拖放，drop 时回传文件路径，逐个读为图片附件。
  unlistenDrop = await getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type !== "drop") return;
    // 忽略由「拖出附件」自身产生、又落回窗口内的 drop，避免把附件当回复输入。
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
    loadThumbs();
    loadDragIcons();
    requestAnimationFrame(() => inputRef.value?.focus({ preventScroll: true }));
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
      <div
        v-if="request.isMarkdown"
        class="markdown-body"
        v-html="renderedHtml"
      ></div>
      <pre v-else class="plain-body">{{ request.message }}</pre>

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

      <div v-if="request.predefinedOptions.length" class="options">
        <div
          v-for="(opt, i) in request.predefinedOptions"
          :key="i"
          class="option"
          :class="{ selected: chosen.includes(opt) }"
          @click="toggle(opt)"
        >
          <span class="check">{{ chosen.includes(opt) ? "✓" : "" }}</span>
          <span class="label">{{ opt }}</span>
        </div>
      </div>

      <textarea
        ref="inputRef"
        v-model="userInput"
        class="textarea"
        placeholder="输入你的回复…"
      ></textarea>

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

    <div class="footer" data-tauri-drag-region>
      <button class="btn btn-icon" type="button" @click="pickFiles">
        添加图片
      </button>
      <input
        ref="fileRef"
        type="file"
        accept="image/*"
        multiple
        hidden
        @change="onFileChange"
      />
      <span class="spacer"></span>
      <button class="btn" type="button" :disabled="submitting" @click="cancel">
        取消 <kbd class="sc">⌘W</kbd>
      </button>
      <button
        class="btn btn-primary"
        type="button"
        :disabled="submitting"
        @click="send"
      >
        发送 <kbd class="sc">⌘↵</kbd>
      </button>
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
  /* 始终占 1px、仅切换颜色：避免布局跳动；不加 transition/mask，
     以免在透明窗口上促成不透明合成层（会破坏毛玻璃）。 */
  border-bottom: 1px solid transparent;
}
.navbar.scrolled {
  border-bottom-color: var(--border);
}
/* macOS Overlay 标题栏：下压让出红绿灯空间 */
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
/* 扩散的光环：用伪元素做 ping 式涟漪，避免影响 dot 自身阴影 */
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
/* 附件区：与「选项」明显区分——填充式胶囊 + 彩色文件瓦片 + 选中外环 */
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
/* 回复文件 chip（拖入的非图片文件，自适应换行） */
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
</style>
