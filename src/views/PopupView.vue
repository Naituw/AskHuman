<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  popupInit,
  submitPopup,
  cancelPopup,
  openSettings,
  updateTheme,
  openPath,
  previewPath,
  readImageDataUrl,
} from "../lib/ipc";
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
const submitting = ref(false);
const inputRef = ref<HTMLTextAreaElement | null>(null);
const fileRef = ref<HTMLInputElement | null>(null);
const scrolled = ref(false);

function onScroll(e: Event) {
  scrolled.value = (e.target as HTMLElement).scrollTop > 0;
}

const pinned = ref(false);
const theme = ref<ThemeMode>("system");

// 提问附带的文件附件（AI→人，仅展示）。
const attachments = computed<FileAttachment[]>(() => request.value?.files ?? []);
const selectedFile = ref<number | null>(null);
const thumbs = ref<Record<string, string>>({});
const previewActive = ref(false);
const previewUnlisten: UnlistenFn[] = [];

function selectFile(index: number) {
  const changed = selectedFile.value !== index;
  selectedFile.value = index;
  // 预览开着时，选其它附件即切换预览（与原生 QuickLook 类似）。
  if (previewActive.value && changed) {
    previewFile(attachments.value[index]);
  }
}

function openFile(file: FileAttachment) {
  openPath(file.path).catch(() => {});
}

function previewFile(file: FileAttachment) {
  previewPath(file.path, pinned.value).catch(() => {});
}

function onFileKeydown(e: KeyboardEvent, file: FileAttachment) {
  if (e.key === "Enter") {
    e.preventDefault();
    e.stopPropagation();
    openFile(file);
  } else if (e.key === " ") {
    e.preventDefault();
    e.stopPropagation();
    previewFile(file);
  }
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

function onDrop(e: DragEvent) {
  if (e.dataTransfer?.files?.length) addFiles(e.dataTransfer.files);
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
  } else if (e.key === "Escape") {
    e.preventDefault();
    cancel();
  }
}

onMounted(async () => {
  window.addEventListener("paste", onPaste);
  window.addEventListener("keydown", onKeydown);
  previewUnlisten.push(
    await listen("preview-opened", () => {
      previewActive.value = true;
    }),
    await listen("preview-closed", () => {
      previewActive.value = false;
    })
  );
  try {
    const init = await popupInit();
    applyTheme(init.theme);
    theme.value = init.theme;
    pinned.value = init.alwaysOnTop;
    request.value = init.request;
    loadThumbs();
    requestAnimationFrame(() => inputRef.value?.focus({ preventScroll: true }));
  } catch (err) {
    console.error("popup_init 失败", err);
    loadError.value = String(err);
  }
});

onBeforeUnmount(() => {
  window.removeEventListener("paste", onPaste);
  window.removeEventListener("keydown", onKeydown);
  previewUnlisten.forEach((un) => un());
});
</script>

<template>
  <div v-if="!request" class="popup popup-status">
    <p v-if="loadError" class="status-error">加载失败：{{ loadError }}</p>
    <p v-else class="status-loading">加载中…</p>
  </div>

  <div v-else class="popup" @dragover.prevent @drop.prevent="onDrop">
    <header class="navbar" :class="{ scrolled }" data-tauri-drag-region>
      <span class="brand">
        <span class="brand-dot"></span>
        <span class="brand-title">Question from the Loop</span>
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
        <div
          v-for="(file, i) in attachments"
          :key="file.path"
          class="attachment"
          :class="{ selected: selectedFile === i }"
          tabindex="0"
          :title="file.path"
          @click="selectFile(i)"
          @dblclick="openFile(file)"
          @keydown="onFileKeydown($event, file)"
        >
          <span class="att-icon">
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
        placeholder="输入你的回复…（⌘/Ctrl+Enter 发送，Esc 取消）"
      ></textarea>

      <div v-if="images.length" class="thumbs">
        <div v-for="(img, i) in images" :key="i" class="thumb">
          <img :src="img.data" alt="" />
          <button class="remove" type="button" @click="removeImage(i)">
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
        取消
      </button>
      <button
        class="btn btn-primary"
        type="button"
        :disabled="submitting"
        @click="send"
      >
        发送
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
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: #30d158;
  box-shadow: 0 0 0 3px color-mix(in srgb, #30d158 22%, transparent);
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
.attachments {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.attachment {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: 8px;
  cursor: default;
  outline: none;
  transition: background 0.12s ease, border-color 0.12s ease;
}
.attachment:hover {
  background: var(--bg-elevated);
}
.attachment.selected,
.attachment:focus-visible {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
}
.att-icon {
  flex: 0 0 auto;
  width: 36px;
  height: 36px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  border-radius: 6px;
  overflow: hidden;
  background: var(--bg-elevated);
}
.att-icon img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.att-icon svg {
  width: 20px;
  height: 20px;
}
.att-meta {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.att-name {
  font-size: 13px;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.att-size {
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
</style>
