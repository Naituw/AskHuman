<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { popupAgentInfo, popupProjectInfo, popupOpenTerminal } from "../../lib/ipc";
import type { PopupAgentInfo, PopupProjectInfo } from "../../lib/types";
const props = withDefaults(defineProps<{ kind: "agent" | "project"; requestId: string; active: boolean;
  enabled?: boolean; label: string; sessionId?: string; sessionTitle?: string; project?: string; callerCwd?: string }>(), { enabled: true });
const { t } = useI18n();
const anchor = ref<HTMLElement | null>(null), card = ref<HTMLElement | null>(null);
const open = ref(false), agent = ref<PopupAgentInfo | null>(null), git = ref<PopupProjectInfo | null>(null);
const cardStyle = ref<Record<string,string>>({}), error = ref(""), copied = ref("");
let opening: ReturnType<typeof setTimeout> | undefined, closing: ReturnType<typeof setTimeout> | undefined;
let copiedTimer: ReturnType<typeof setTimeout> | undefined;
const owner = Symbol("source-card");
let layoutObserver: ResizeObserver | undefined;
let generation = 0, loadedAt = 0, disposed = false, suppressFocus = false;
function trigger() { return anchor.value?.querySelector<HTMLElement>(".brand-chip") ?? null; }
function position() {
  const element = trigger(); if (!element || !open.value) return;
  const rect = element.getBoundingClientRect();
  const main = element.closest(".popup-main")?.getBoundingClientRect();
  const leftLimit = Math.max(12, (main?.left ?? 0) + 12);
  const rightLimit = Math.min(window.innerWidth - 12, (main?.right ?? window.innerWidth) - 12);
  const width = Math.max(120, Math.min(360, rightLimit - leftLimit));
  const left = Math.max(leftLimit, Math.min(rect.left, rightLimit - width));
  const top = rect.bottom + 7;
  cardStyle.value = { left: `${left}px`, top: `${top}px`, width: `${width}px`, maxHeight: `${Math.max(80,window.innerHeight-top-12)}px` };
}
async function show() {
  clearTimeout(opening); clearTimeout(closing);
  if (disposed || !props.active || !props.enabled || !props.requestId) return;
  window.dispatchEvent(new CustomEvent("popup-source-open", { detail: owner }));
  open.value = true; error.value = ""; await nextTick(); position();
  if (Date.now() - loadedAt < 2000) return;
  agent.value = null; git.value = null;
  const token = ++generation, requestId = props.requestId;
  try {
    if (props.kind === "agent") {
      const value = await popupAgentInfo(requestId);
      if (!disposed && token === generation && props.requestId === requestId) agent.value = value;
    } else {
      const value = await popupProjectInfo(requestId);
      if (!disposed && token === generation && props.requestId === requestId) git.value = value;
    }
    if (token === generation) loadedAt = Date.now();
  } catch { /* Optional native facts remain absent when their source is unavailable. */ }
  await nextTick(); if (!disposed) position();
}
function stay() { clearTimeout(closing); }
function enter() { clearTimeout(closing); clearTimeout(opening); opening = setTimeout(() => { void show(); },100); }
function leave() { clearTimeout(opening); clearTimeout(closing); closing = setTimeout(() => {
  if (!card.value?.contains(document.activeElement)) close();
},160); }
function close(restore = false) {
  clearTimeout(opening); clearTimeout(closing); generation++;
  const focused = card.value?.contains(document.activeElement);
  open.value = false;
  if (restore && focused) { suppressFocus = true; trigger()?.focus(); queueMicrotask(() => { suppressFocus = false; }); }
}
function focus() { if (!suppressFocus) void show(); }
function key(event: KeyboardEvent) {
  if (event.key === "Escape" && open.value) { event.preventDefault(); event.stopImmediatePropagation(); close(true); }
  if (event.key === "ArrowDown" && anchor.value?.contains(document.activeElement) && props.enabled && props.active) {
    event.preventDefault(); event.stopPropagation(); void show().then(() => card.value?.querySelector<HTMLButtonElement>("button")?.focus());
  }
}
function outside(event: PointerEvent) {
  if (!anchor.value?.contains(event.target as Node) && !card.value?.contains(event.target as Node)) close();
}
function blur() { close(); }
function otherOpen(event: Event) { if ((event as CustomEvent).detail !== owner) close(); }
async function copy(value: string, id: string) {
  try {
    if (navigator.clipboard?.writeText) await navigator.clipboard.writeText(value);
    else {
      const previous = document.activeElement as HTMLElement | null;
      const textarea = document.createElement("textarea"); textarea.value = value;
      textarea.style.cssText = "position:fixed;left:-10000px;top:0"; document.body.append(textarea);
      textarea.select(); const ok = document.execCommand("copy"); textarea.remove(); previous?.focus();
      if (!ok) throw new Error("copy failed");
    }
    copied.value = id; clearTimeout(copiedTimer); copiedTimer = setTimeout(() => { copied.value = ""; },1600);
  } catch { error.value = t("sourceInfo.copyFailed"); }
}
async function terminal(target: "project" | "caller") {
  try { error.value = ""; await popupOpenTerminal(props.requestId,target); }
  catch { error.value = t("sourceInfo.terminalFailed"); }
}
const differentCaller = computed(() => {
  const normalize = (path: string) => {
    let value = path.replace(/\\/g,"/").replace(/\/$/,"");
    if (/^[a-z]:\//i.test(value) || value.startsWith("//")) value = value.toLowerCase();
    return value;
  };
  return props.callerCwd && normalize(props.callerCwd) !== normalize(props.project ?? "") ? props.callerCwd : "";
});
const paths = computed(() => [
  ...(props.project ? [{ key: "project" as const, label: t("sourceInfo.projectPath"), value: props.project }] : []),
  ...(differentCaller.value ? [{ key: "caller" as const, label: t("sourceInfo.callerCwd"), value: differentCaller.value }] : []),
]);
function tokens(value: number) { return value >= 1e6 ? `${(value/1e6).toFixed(1)}M` : value >= 1000 ? `${(value/1000).toFixed(1)}k` : String(value); }
const contextText = computed(() => {
  const ctx = agent.value?.context;
  return ctx ? t("sourceInfo.contextValue", { n: Math.round(ctx.remainingPercent), used: tokens(ctx.usedTokens), total: tokens(ctx.windowTokens) }) : "";
});
const reported = computed(() => agent.value?.reportedAt ? new Date(agent.value.reportedAt*1000).toLocaleString() : "");
watch(() => props.active, active => { if (!active) close(); });
watch(() => props.requestId, () => { close(); agent.value=null;git.value=null;loadedAt=0; });
onMounted(() => {
  if (typeof ResizeObserver !== "undefined") {
    layoutObserver = new ResizeObserver(position);
    const main = trigger()?.closest(".popup-main");
    if (main) layoutObserver.observe(main);
  }
  window.addEventListener("popup-source-open",otherOpen);
  window.addEventListener("keydown",key,true); window.addEventListener("pointerdown",outside,true);
  window.addEventListener("resize",position); window.addEventListener("blur",blur);
});
onBeforeUnmount(() => {
  disposed=true; close(); clearTimeout(copiedTimer); layoutObserver?.disconnect();
  window.removeEventListener("popup-source-open",otherOpen);
  window.removeEventListener("keydown",key,true); window.removeEventListener("pointerdown",outside,true);
  window.removeEventListener("resize",position); window.removeEventListener("blur",blur);
});
</script>

<template>
  <span ref="anchor" class="source-popover-anchor" @mouseenter="enter" @mouseleave="leave" @focusin="focus" @focusout="leave">
    <slot />
  </span>
  <Teleport to="body">
    <section v-if="open" ref="card" class="source-info-card" :style="cardStyle" role="dialog" :aria-label="t(kind === 'agent' ? 'sourceInfo.agentDetails' : 'sourceInfo.projectDetails')"
      @mouseenter="stay" @mouseleave="leave" @focusin="stay" @focusout="leave" @click.stop @pointerdown.stop>
      <template v-if="kind === 'agent'">
        <h2>{{ sessionTitle || label }}</h2>
        <div v-if="sessionId" class="source-info-row source-session-id">
          <span class="source-info-key">{{ t('sourceInfo.sessionId') }}</span>
          <code :title="sessionId">{{ sessionId }}</code>
          <button type="button" class="source-info-action" :title="copied === 'session' ? t('common.copied') : t('sourceInfo.copySession')" :aria-label="t('sourceInfo.copySession')" @click="copy(sessionId,'session')">
            <svg viewBox="0 0 20 20" aria-hidden="true"><path v-if="copied === 'session'" d="m4 10 4 4 8-9"/><path v-else d="M7 3h9v11H7z M4 6H3v11h9v-1"/></svg>
          </button>
        </div>
        <div v-if="agent?.model" class="source-info-row"><span class="source-info-key">{{ t('sourceInfo.model') }}</span><span class="source-info-value">{{ agent.model }}</span></div>
        <div v-if="agent?.reasoning" class="source-info-row"><span class="source-info-key">{{ t('sourceInfo.reasoning') }}</span><span>{{ agent.reasoning }}</span></div>
        <div v-if="contextText" class="source-context"><span class="source-info-key">{{ t('sourceInfo.context') }}</span><span>{{ contextText }}</span><small v-if="agent?.context?.estimated">{{ t('sourceInfo.estimated') }}</small></div>
        <p v-if="reported && (agent?.model || agent?.context)" class="source-info-time">{{ t('sourceInfo.reportedAt', { time: reported }) }}</p>
      </template>
      <template v-else>
        <h2>{{ label }}</h2>
        <div v-for="path in paths" :key="path.key" class="source-directory">
          <span class="source-info-key">{{ path.label }}</span>
          <div class="source-directory-value"><span>{{ path.value }}</span><span class="source-directory-actions">
            <button type="button" class="source-info-action" :title="copied === path.key ? t('common.copied') : t('sourceInfo.copyPath')" :aria-label="t('sourceInfo.copyPath')" @click="copy(path.value,path.key)"><svg viewBox="0 0 20 20" aria-hidden="true"><path v-if="copied === path.key" d="m4 10 4 4 8-9"/><path v-else d="M7 3h9v11H7z M4 6H3v11h9v-1"/></svg></button>
            <button type="button" class="source-info-action" :title="t('sourceInfo.openTerminal')" :aria-label="t('sourceInfo.openTerminal')" @click="terminal(path.key)"><svg viewBox="0 0 20 20" aria-hidden="true"><rect x="2" y="3" width="16" height="14" rx="2"/><path d="m5 7 3 3-3 3 M10 13h5"/></svg></button>
          </span></div>
        </div>
        <div v-if="git?.branch" class="source-info-row"><span class="source-info-key">{{ t(git.detached ? 'sourceInfo.commit' : 'sourceInfo.branch') }}</span><code class="source-info-value">{{ git.branch }}</code></div>
        <div v-if="git?.worktree" class="source-info-row"><span class="source-info-key">{{ t('sourceInfo.worktree') }}</span><span class="source-info-value">{{ git.worktree }}</span></div>
        <div v-if="git?.changes" class="source-git-summary"><span>{{ t('sourceInfo.changes', { n: git.changes.total }) }}</span><small>{{ t('sourceInfo.changeDetails', { staged: git.changes.staged, unstaged: git.changes.unstaged, untracked: git.changes.untracked }) }}</small></div>
      </template>
      <p v-if="error" class="source-info-error" role="alert">{{ error }}</p>
    </section>
  </Teleport>
</template>

<style scoped>
.source-popover-anchor { display: contents; }
.source-info-card { position: fixed; z-index: 250; box-sizing: border-box; padding: 14px; overflow: auto; border: var(--hairline) solid var(--border); border-radius: 10px; background: var(--surface-overlay); color: var(--text-primary); box-shadow: 0 8px 26px #0002, 0 2px 5px #0001; font-family: var(--font-sans); font-size: 12px; line-height: 1.5; user-select: text; -webkit-user-select: text; }
.source-info-card h2 { font-size: 13px; line-height: 1.5; font-weight: 600; margin: 0 0 10px; overflow-wrap: anywhere; }
.source-info-row { display: flex; align-items: center; gap: 10px; margin-top: 6px; min-width: 0; }
.source-info-key { flex: 0 0 auto; color: var(--text-secondary); }
.source-info-value { min-width: 0; overflow-wrap: anywhere; }
.source-session-id code { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-secondary); font-size: 11px; }
.source-info-action { display: inline-flex; align-items: center; justify-content: center; flex: 0 0 auto; width: 25px; height: 25px; padding: 4px; background: transparent; color: var(--text-secondary); border: none; border-radius: 5px; cursor: pointer; }
.source-info-action:hover { background: var(--bg-elevated); color: var(--text-primary); }
.source-info-action:focus-visible { outline: 2px solid var(--accent); }
.source-info-action svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
.source-directory { margin-top: 10px; }
.source-directory-value { display: flex; gap: 8px; align-items: flex-start; margin-top: 3px; }
.source-directory-value > span:first-child { flex: 1; min-width: 0; overflow-wrap: anywhere; }
.source-directory-actions { display: flex; flex: 0 0 auto; }
.source-context { display: flex; flex-wrap: wrap; gap: 5px 10px; margin-top: 8px; }
.source-context small, .source-git-summary small { color: var(--text-secondary); font-size: 11px; }
.source-info-time { color: var(--text-tertiary); font-size: 11px; margin: 10px 0 0; }
.source-git-summary { display: flex; flex-direction: column; gap: 3px; margin-top: 10px; }
.source-info-error { margin: 8px 0 0; color: var(--accent-orange); }
</style>
