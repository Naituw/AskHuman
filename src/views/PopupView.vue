<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { popupInit, submitPopup, cancelPopup } from "../lib/ipc";
import { renderMarkdown } from "../lib/markdown";
import { applyTheme, fileToDataUrl } from "../lib/theme";
import type { AskRequest, ImageAttachment } from "../lib/types";

const request = ref<AskRequest | null>(null);
const loadError = ref<string | null>(null);
const chosen = ref<string[]>([]);
const userInput = ref("");
const images = ref<ImageAttachment[]>([]);
const submitting = ref(false);
const inputRef = ref<HTMLTextAreaElement | null>(null);
const fileRef = ref<HTMLInputElement | null>(null);

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
  try {
    const init = await popupInit();
    applyTheme(init.theme);
    request.value = init.request;
    requestAnimationFrame(() => inputRef.value?.focus());
  } catch (err) {
    console.error("popup_init 失败", err);
    loadError.value = String(err);
  }
});

onBeforeUnmount(() => {
  window.removeEventListener("paste", onPaste);
  window.removeEventListener("keydown", onKeydown);
});
</script>

<template>
  <div v-if="!request" class="popup popup-status">
    <p v-if="loadError" class="status-error">加载失败：{{ loadError }}</p>
    <p v-else class="status-loading">加载中…</p>
  </div>

  <div v-else class="popup" @dragover.prevent @drop.prevent="onDrop">
    <div class="content">
      <div
        v-if="request.isMarkdown"
        class="markdown-body"
        v-html="renderedHtml"
      ></div>
      <pre v-else class="plain-body">{{ request.message }}</pre>

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

    <div class="footer">
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
.footer {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  border-top: 1px solid var(--border);
  background: var(--bg);
}
.footer .spacer {
  flex: 1 1 auto;
}
</style>
