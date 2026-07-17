<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { applyTheme } from "../lib/theme";
import { applyLanguage } from "../i18n";
import { interjectCancel, interjectInit, interjectSubmit } from "../lib/ipc";
import type { AgentKind, ThemeMode } from "../lib/types";

const { t } = useI18n();

// 目标 agent 信息由 Rust 侧经窗口 URL 注入：?view=interject&session=...&kind=...&project=...
const params = new URLSearchParams(window.location.search);
const session = params.get("session") ?? "";
const kind = params.get("kind") ?? "";
const project = params.get("project") ?? "";

const text = ref("");
// 打开时已有待送达条数（>0 时提示「提交将整体覆盖」）。
const pendingEntries = ref(0);
const loaded = ref(false);
const sending = ref(false);
const textarea = ref<HTMLTextAreaElement | null>(null);

function kindLabel(k: string): string {
  const known: AgentKind[] = ["claude", "codex", "cursor", "grok"];
  return known.includes(k as AgentKind) ? t(`agents.kind.${k}`) : k;
}

// 可提交：有内容，或「清空已有待送达」（预填被删空也算一次有效提交 = 撤回）。
const canSend = computed(
  () => !sending.value && (text.value.trim().length > 0 || pendingEntries.value > 0),
);

async function send(): Promise<void> {
  if (!canSend.value) return;
  sending.value = true;
  try {
    await interjectSubmit(session, text.value.trim());
    // 后端提交后即关窗；此处无需善后。
  } catch (err) {
    console.warn("interject submit failed", err);
    sending.value = false;
  }
}

async function cancel(): Promise<void> {
  try {
    await interjectCancel(session);
  } catch (err) {
    console.warn("interject cancel failed", err);
  }
}

function onKeydown(e: KeyboardEvent): void {
  if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
    e.preventDefault();
    void send();
  } else if (e.key === "Escape") {
    e.preventDefault();
    void cancel();
  }
}

let unlistenSettings: UnlistenFn | null = null;

onMounted(async () => {
  try {
    const init = await interjectInit(session);
    applyTheme(init.theme);
    applyLanguage(init.lang);
    text.value = init.text;
    pendingEntries.value = init.entries;
  } catch {
    /* daemon 不可达：保持空预填，提交时后端兜底重试 */
  }
  // 设置变更实时生效（主题/语言与设置窗口同宿主进程广播）。
  unlistenSettings = await listen<{ theme?: ThemeMode; language?: string }>(
    "settings-updated",
    (e) => {
      if (typeof e.payload.theme === "string") applyTheme(e.payload.theme);
      if (typeof e.payload.language === "string") applyLanguage(e.payload.language);
    }
  );
  loaded.value = true;
  // 聚焦输入框、光标移到末尾（预填内容之后继续输入）。
  requestAnimationFrame(() => {
    const el = textarea.value;
    if (el) {
      el.focus();
      el.setSelectionRange(el.value.length, el.value.length);
    }
  });
});

onBeforeUnmount(() => {
  unlistenSettings?.();
});
</script>

<template>
  <div class="interject" @keydown="onKeydown">
    <header class="ij-header" data-tauri-drag-region>
      <span class="ij-title" data-tauri-drag-region>{{ t("interject.title") }}</span>
      <span v-if="kind" class="kind-badge">{{ kindLabel(kind) }}</span>
      <span v-if="project" class="ij-project" :title="project">{{ project }}</span>
    </header>

    <div class="ij-body">
      <p class="ij-hint">{{ t("interject.hint") }}</p>
      <textarea
        ref="textarea"
        v-model="text"
        class="ij-input"
        :placeholder="t('interject.placeholder')"
        :disabled="!loaded || sending"
        spellcheck="false"
      />
    </div>

    <footer class="ij-footer">
      <span v-if="pendingEntries > 0" class="ij-pending">
        {{ t("interject.overwriteNote", { n: pendingEntries }) }}
      </span>
      <span class="ij-actions">
        <button type="button" class="btn" @click="cancel">
          {{ t("interject.cancel") }}
        </button>
        <button type="button" class="btn primary" :disabled="!canSend" @click="send">
          {{ t("interject.send") }}
        </button>
      </span>
    </footer>
  </div>
</template>

<style scoped>
.interject {
  display: flex;
  flex-direction: column;
  height: 100%;
  color: var(--text-primary);
}
.ij-header {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  border-bottom: var(--hairline) solid var(--border);
}
.macos .ij-header {
  padding-top: 30px;
}
.ij-title {
  font-size: 14px;
  font-weight: 600;
  white-space: nowrap;
}
.kind-badge {
  flex: 0 0 auto;
  padding: 1px 7px;
  border-radius: 5px;
  font-size: 10px;
  font-weight: 600;
  background: color-mix(in srgb, var(--text-primary) 9%, transparent);
  color: var(--text-secondary);
  white-space: nowrap;
}
.ij-project {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 12px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  text-align: right;
}
.ij-body {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 14px 0;
}
.ij-hint {
  flex: 0 0 auto;
  margin: 0;
  font-size: 11px;
  color: var(--text-secondary);
}
.ij-input {
  flex: 1 1 auto;
  min-height: 0;
  resize: none;
  padding: 8px 10px;
  border: var(--hairline) solid var(--control-border);
  border-radius: var(--radius-sm, 8px);
  background: var(--control-bg);
  color: var(--text-primary);
  font-size: 13px;
  line-height: 1.5;
  font-family: inherit;
  outline: none;
  box-shadow: var(--clickable-shadow);
}
.ij-input:focus {
  border-color: var(--accent, #0a84ff);
}
.ij-footer {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
  padding: 10px 14px 12px;
}
.ij-pending {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 11px;
  color: #c77700;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.ij-actions {
  flex: 0 0 auto;
  display: inline-flex;
  gap: 8px;
}
.btn {
  appearance: none;
  border: var(--hairline) solid var(--control-border);
  background: var(--control-bg);
  color: var(--text-primary);
  font-size: 12px;
  font-weight: 600;
  padding: 5px 14px;
  border-radius: 7px;
  cursor: pointer;
  box-shadow: var(--clickable-shadow);
}
.btn:hover {
  background: var(--control-hover-bg);
}
.btn.primary {
  border-color: transparent;
  background: var(--accent, #0a84ff);
  color: #fff;
}
.btn.primary:hover {
  background: color-mix(in srgb, var(--accent, #0a84ff) 88%, #000);
}
.btn:disabled {
  opacity: 0.45;
  cursor: default;
}
</style>
