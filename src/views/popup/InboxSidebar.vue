<script setup lang="ts">
// The inbox Sidebar: the pending requests grouped by project. It only presents what the page hands it; the page
// owns the state (which request is open, unread marks, drafts, the answer transition) and acts on the events.
import SessionIcon from "../../components/SessionIcon.vue";
import { useI18n } from "vue-i18n";
import UnreadRipple from "./UnreadRipple.vue";
import type { PopupInboxRequest } from "../../lib/types";
import { inboxKind, inboxTitle } from "./inboxQueue";
import type { InboxMotion } from "./inboxTransition";

defineProps<{
  groups: { path: string; requests: PopupInboxRequest[] }[];
  total: number;
  selectedId: string | null;
  visited: ReadonlySet<string>;
  arriving: ReadonlySet<string>;
  flashed: ReadonlySet<string>;
  drafts: ReadonlySet<string>;
  blocked: boolean;
  motion: InboxMotion | null;
  /** The project whose group is folding away because its last request just ended. */
  collapsingGroup: string | null;
}>();
const emit = defineEmits<{ choose: [id: string]; remember: [] }>();
const { t } = useI18n();

function agentLabel(request: PopupInboxRequest) {
  const kind = request.agentKind;
  if (!kind) return request.source;
  const key = `agents.kind.${kind}`;
  const label = t(key);
  return label === key ? kind : label;
}
function projectName(path: string) { return path.split(/[\\/]/).filter(Boolean).slice(-1)[0] || t("popup.inbox.unknownProject"); }
</script>

<template>
  <aside class="inbox-sidebar" :inert="blocked" :aria-label="t('popup.inbox.pending')">
    <header class="inbox-heading" data-tauri-drag-region>{{ t('popup.inbox.pending') }} <span>{{ total }}</span></header>
    <nav class="inbox-navigation" @pointerdown.capture="emit('remember')">
      <section v-for="group in groups" :key="group.path" class="inbox-group" :class="{ 'inbox-group-collapse': collapsingGroup === group.path }" :style="motion?.group === group.path ? { '--completion-group-height': `${motion.groupHeight}px` } : undefined">
        <h2 :title="group.path">{{ projectName(group.path) }} <span>{{ group.requests.length }}</span></h2>
        <div v-for="request in group.requests" :key="request.requestId" class="inbox-entry" :class="{ 'inbox-entry-collapse': motion?.from === request.requestId && motion.phase !== 'waiting' && !motion.reduced }" :style="motion?.from === request.requestId ? { '--completion-row-height': `${motion.rowHeight}px` } : undefined">
          <button class="inbox-row" :data-inbox-row="request.requestId" :class="{ selected: request.requestId === selectedId, flash: flashed.has(request.requestId) && motion?.from !== request.requestId, unread: !visited.has(request.requestId) }" :aria-current="request.requestId === selectedId ? 'true' : undefined" :title="inboxTitle(request)" @click="emit('choose', request.requestId)">
            <UnreadRipple :active="!visited.has(request.requestId) && !arriving.has(request.requestId) && !flashed.has(request.requestId)" />
            <span class="inbox-dot" :class="{ unread: !visited.has(request.requestId) && !arriving.has(request.requestId) }" :aria-label="!visited.has(request.requestId) ? t('popup.inbox.unread') : undefined"></span>
            <span class="inbox-row-content">
              <strong><span v-if="inboxKind(request) !== 'ask'" class="inbox-kind">{{ t(`popup.inbox.kind.${inboxKind(request)}`) }}</span>{{ inboxTitle(request) || t('popup.inbox.untitled') }}</strong>
              <span class="inbox-row-meta" :class="{ 'has-session': !!request.agentSessionTitle?.trim() }">
                <span class="inbox-row-agent" :title="agentLabel(request)">{{ agentLabel(request) }}</span>
                <template v-if="request.agentSessionTitle?.trim()">
                  <span class="inbox-source-separator" aria-hidden="true">/</span>
                  <span class="inbox-session" :title="request.agentSessionTitle">
                    <SessionIcon />
                    <span class="inbox-session-title" :title="request.agentSessionTitle">{{ request.agentSessionTitle }}</span>
                  </span>
                </template>
                <em v-if="drafts.has(request.requestId)">{{ t('popup.inbox.draft') }}</em>
              </span>
            </span>
          </button>
        </div>
      </section>
    </nav>
  </aside>
</template>

<style>
.inbox-sidebar { display: flex; flex-direction: column; min-width: 0; min-height: 0; background: color-mix(in srgb, var(--bg, #f5f5f5) 72%, transparent); }
.inbox-heading { display: flex; align-items: center; justify-content: space-between; padding: 30px 16px 14px; font-size: 13px; font-weight: 600; user-select: none; }
.inbox-heading span, .inbox-group h2 span { font-variant-numeric: tabular-nums; opacity: .6; }
.inbox-navigation { flex: 1; overflow: auto; padding: 0 8px 16px; }
.inbox-group h2 { display: flex; justify-content: space-between; gap: 8px; font-size: 11px; font-weight: 600; opacity: .6; padding: 16px 12px 7px; margin: 0; overflow: hidden; }
.inbox-entry-collapse { overflow: hidden; animation: inbox-entry-complete 300ms cubic-bezier(.2, 0, .2, 1) both; }
.inbox-group-collapse { overflow: hidden; animation: inbox-group-complete 300ms cubic-bezier(.2, 0, .2, 1) both; }
@keyframes inbox-entry-complete { from { height: var(--completion-row-height); opacity: 1; } to { height: 0; opacity: 0; } }
@keyframes inbox-group-complete { from { height: var(--completion-group-height); opacity: 1; } to { height: 0; opacity: 0; } }
.inbox-row { --inbox-ripple-color: rgba(38, 133, 232, .18); position: relative; isolation: isolate; overflow: hidden; display: flex; align-items: flex-start; gap: 7px; width: 100%; padding: 10px 8px; border: 0; border-radius: 8px; text-align: left; background: transparent; color: inherit; cursor: pointer; }
.theme-dark .inbox-row { --inbox-ripple-color: rgba(38, 133, 232, .252); }
@media (prefers-color-scheme: dark) { :root:not(.theme-light) .inbox-row { --inbox-ripple-color: rgba(38, 133, 232, .252); } }
.inbox-row.selected { background: color-mix(in srgb, #2685e8 12%, transparent); }
.inbox-row:hover { background: color-mix(in srgb, #2685e8 8%, transparent); }
.inbox-row.flash { animation: inbox-row-arrival 640ms cubic-bezier(.22, 0, .22, 1); }
.inbox-dot { position: relative; z-index: 1; flex: 0 0 6px; height: 6px; margin-top: 5px; }
.inbox-dot.unread::before { content: ''; position: absolute; left: -2px; top: -2px; width: 10px; height: 10px; border-radius: 50%; background: #2685e8; animation: inbox-unread-dot 2.8s cubic-bezier(.4, 0, .2, 1) infinite; }
.inbox-row-content { position: relative; z-index: 1; min-width: 0; flex: 1; }
.inbox-row strong { display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; font-size: 12px; line-height: 1.45; font-weight: 500; }
.inbox-kind { display: inline-block; vertical-align: 1px; font-size: 9px; line-height: 1.4; font-weight: 400; padding: 1px 4px; border: 1px solid var(--border); border-radius: 3px; margin-right: 5px; color: var(--text-secondary); }
.inbox-row-meta { display: flex; align-items: center; gap: 5px; min-width: 0; font-size: 10px; line-height: 1.5; color: var(--text-secondary); margin-top: 6px; }
.inbox-row-agent { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.inbox-row-meta.has-session .inbox-row-agent { flex: 0 0 auto; max-width: 40%; }
.inbox-source-separator { flex: none; color: var(--text-tertiary); }
.inbox-session { display: flex; align-items: center; gap: 4px; min-width: 0; flex: 1; font-size: 11px; }
.inbox-session svg { flex: none; width: 11px; height: 11px; stroke: currentColor; fill: none; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
.inbox-session-title { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.inbox-row-meta em { flex: none; font-size: 9px; font-style: normal; margin-left: auto; }
.inbox-sidebar[inert] { pointer-events: none; }
@keyframes inbox-row-arrival {
  0%, 100% { box-shadow: inset 0 0 0 100px #2685e800; }
  28% { box-shadow: inset 0 0 0 100px rgba(38, 133, 232, .17); }
  58% { box-shadow: inset 0 0 0 100px rgba(38, 133, 232, .09); }
}
@keyframes inbox-unread-dot { 0%, 100% { background: #2685e8; } 45% { background: #006dff; } 70% { background: #2685e8; } }
@media (prefers-reduced-motion: reduce) {
  .inbox-row.flash, .inbox-dot.unread::before { animation: none; }
  .inbox-row.unread { box-shadow: inset 0 0 0 100px rgba(38, 133, 232, .08); }
}
</style>
