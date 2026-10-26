<script setup lang="ts">
// The inbox Sidebar: the pending requests grouped by project. It only presents what the page hands it; the page
// owns the state (which request is open, unread marks, drafts, the answer transition) and acts on the events.
// It doubles as the small window's list: the window is this column alone (docs/specs/popup-inbox-collapse.md §2.4).
// Its look in each window form is driven by the page's data attributes on `.inbox-root` (see the styles below).
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
  /** Requests marked "answer later". */
  secondary: ReadonlySet<string>;
  blocked: boolean;
  motion: InboxMotion | null;
  /** The project whose group is folding away because its last request just ended. */
  collapsingGroup: string | null;
  /** Files are dragged over the window: the requests that can take them, the row under the pointer, and the row that a wait opens. */
  droppable?: ReadonlySet<string>;
  dropTarget?: string | null;
  dwell?: { id: string; ms: number } | null;
}>();
const emit = defineEmits<{
  choose: [id: string];
  remember: [];
  mark: [id: string];
  unmark: [id: string];
  /** The small window's way back to the expanded one. */
  expand: [];
  /** A peek's way back to the small window. */
  shrink: [];
}>();
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
    <div class="sb-lift">
      <header class="inbox-heading" data-tauri-drag-region>
        <span data-tauri-drag-region>{{ t('popup.inbox.pending') }} <span class="inbox-count" data-tauri-drag-region>{{ total }}</span></span>
        <!-- One slot, two icons: "expand" in the small window, "shrink" in a peek (expanded with nothing left to answer). -->
        <button class="mini-expand" type="button" :aria-label="t('popup.inbox.expand')" :title="t('popup.inbox.expand')" @click="emit('expand')">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7" /></svg>
        </button>
        <button class="mini-shrink" type="button" :aria-label="t('popup.inbox.shrink')" :title="t('popup.inbox.shrink')" @click="emit('shrink')">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 14h6v6M20 10h-6V4M14 10l7-7M3 21l7-7" /></svg>
        </button>
      </header>
      <nav class="inbox-navigation" data-tauri-drag-region @pointerdown.capture="emit('remember')">
        <section v-for="group in groups" :key="group.path" class="inbox-group" :class="{ 'inbox-group-collapse': collapsingGroup === group.path }" :style="motion?.group === group.path ? { '--completion-group-height': `${motion.groupHeight}px` } : undefined">
          <h2 :title="group.path">{{ projectName(group.path) }} <span>{{ group.requests.length }}</span></h2>
          <div v-for="request in group.requests" :key="request.requestId" class="inbox-entry" :class="{ 'inbox-entry-collapse': motion?.from === request.requestId && motion.phase !== 'waiting' && !motion.reduced }" :style="motion?.from === request.requestId ? { '--completion-row-height': `${motion.rowHeight}px` } : undefined">
            <button class="inbox-row" :data-inbox-row="request.requestId" :class="{ selected: request.requestId === selectedId, secondary: secondary.has(request.requestId), flash: flashed.has(request.requestId) && motion?.from !== request.requestId, unread: !visited.has(request.requestId), 'can-drop': droppable?.has(request.requestId), 'drop-target': dropTarget === request.requestId, dwelling: dwell?.id === request.requestId }" :style="dwell?.id === request.requestId ? { '--dwell-ms': `${dwell.ms}ms` } : undefined" :aria-current="request.requestId === selectedId ? 'true' : undefined" :title="inboxTitle(request)" @click="emit('choose', request.requestId)">
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
            <!-- An archive box at the end of the title with three looks: an icon on hover (answer later), the "later"
                 label on a row that is set aside, and "undo" instead of the label while that row is hovered. -->
            <button class="row-flag flag-archive" type="button" :class="{ 'is-secondary': secondary.has(request.requestId) }"
              :aria-label="secondary.has(request.requestId) ? t('popup.inbox.undoLater') : t('popup.inbox.later')"
              :title="secondary.has(request.requestId) ? t('popup.inbox.undoLaterHint') : t('popup.inbox.laterHint')"
              @click="secondary.has(request.requestId) ? emit('unmark', request.requestId) : emit('mark', request.requestId)">
              <span class="fa-stack">
                <span class="fa-put"><svg viewBox="0 0 24 24" aria-hidden="true"><rect x="2" y="3" width="20" height="5" rx="1" /><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8" /><path d="M10 12h4" /></svg></span>
                <span class="fa-state"><svg viewBox="0 0 24 24" aria-hidden="true"><rect x="2" y="3" width="20" height="5" rx="1" /><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8" /><path d="M10 12h4" /></svg>{{ t('popup.inbox.laterLabel') }}</span>
                <span class="fa-undo"><svg viewBox="0 0 24 24" aria-hidden="true"><rect x="2" y="3" width="20" height="5" rx="1" /><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8" /><path d="m9 15 3-3 3 3" /><path d="M12 12v9" /></svg>{{ t('popup.inbox.undoLater') }}</span>
              </span>
            </button>
          </div>
        </section>
      </nav>
    </div>
  </aside>
</template>

<style>
.inbox-sidebar {
  display: flex; flex-direction: column; min-width: 0; min-height: 0; background: color-mix(in srgb, var(--bg, #f5f5f5) 72%, transparent);
  transition: opacity calc(var(--ms) * .4) var(--inbox-ease-out) 0ms, visibility 0s linear 0ms;
}
/* A single request has no Sidebar column: its list is laid out hidden, to be measured, and shows only in the small window. */
.inbox-root[data-view="expanded"][data-sidebar="off"] .inbox-sidebar {
  opacity: 0; visibility: hidden;
  transition: opacity calc(var(--ms) * .3) var(--inbox-ease-in) 0ms, visibility 0s linear calc(var(--ms) * .3);
}
/* The list fades in under the leaving body. */
.inbox-root[data-view="collapsed"] .inbox-sidebar {
  transition: opacity calc(var(--ms) * .45) var(--inbox-ease-out) calc(var(--ms) * .2), visibility 0s linear 0ms;
}
/* The small window has no traffic lights, so its content moves up into their place. */
.sb-lift { flex: 1; min-height: 0; display: flex; flex-direction: column; transition: transform var(--ms) var(--inbox-ease); }
.inbox-root[data-lights="hide"]:is([data-view="collapsed"], [data-measure]) .sb-lift { transform: translateY(calc(var(--inbox-lift) * -1)); }
.inbox-heading { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 30px 16px 14px; font-size: 13px; font-weight: 600; user-select: none; }
.inbox-heading .inbox-count, .inbox-group h2 span { font-variant-numeric: tabular-nums; opacity: .6; }
.inbox-heading .inbox-count { transition: all calc(var(--ms) * .5) var(--inbox-ease); }
.mini-expand, .mini-shrink {
  display: none; align-items: center; justify-content: center; width: 24px; height: 24px; margin: -4px -6px -4px auto;
  padding: 0; border: 0; border-radius: 7px; background: transparent; color: var(--text-secondary); cursor: pointer;
}
.mini-expand:hover, .mini-shrink:hover { background: var(--card-bg); color: var(--text-primary); }
.mini-expand svg, .mini-shrink svg, .row-flag svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
.inbox-root[data-view="collapsed"] .mini-expand { display: inline-flex; }
/* A peek: expanded and nothing is left to answer, so nothing else holds the window open. */
.inbox-root[data-view="expanded"][data-primary="no"][data-sidebar="on"] .mini-shrink { display: inline-flex; }
/* The small window: the same column, in the accent colour, with the count as a pill. */
.inbox-root[data-view="collapsed"] .inbox-heading { color: var(--accent); }
/* The pill is part of what the small window's height is measured from, so the measuring pass lays it out too. */
.inbox-root:is([data-view="collapsed"], [data-measure]) .inbox-heading .inbox-count { opacity: 1; color: #fff; background: var(--accent); padding: 1px 7px; border-radius: 9px; font-size: 12px; }
.inbox-navigation { position: relative; flex: 1; min-height: 0; overflow: auto; padding: 0 8px 16px; }
/* Settled in the small window: the list is as tall as its rows and scrolls inside the window. */
.inbox-root[data-settled="yes"] .inbox-navigation { flex: none; height: var(--mini-nav-h); }
.inbox-group h2 { display: flex; justify-content: space-between; gap: 8px; font-size: 11px; font-weight: 600; opacity: .6; padding: 16px 12px 7px; margin: 0; overflow: hidden; }
.inbox-entry { position: relative; }
.inbox-entry-collapse { overflow: hidden; animation: inbox-entry-complete 300ms cubic-bezier(.2, 0, .2, 1) both; }
.inbox-group-collapse { overflow: hidden; animation: inbox-group-complete 300ms cubic-bezier(.2, 0, .2, 1) both; }
@keyframes inbox-entry-complete { from { height: var(--completion-row-height); opacity: 1; } to { height: 0; opacity: 0; } }
@keyframes inbox-group-complete { from { height: var(--completion-group-height); opacity: 1; } to { height: 0; opacity: 0; } }
.inbox-row { --inbox-ripple-color: rgba(38, 133, 232, .18); position: relative; isolation: isolate; overflow: hidden; display: flex; align-items: flex-start; gap: 7px; width: 100%; padding: 10px 8px; border: 0; border-radius: 8px; text-align: left; background: transparent; color: inherit; cursor: pointer; }
.theme-dark .inbox-row { --inbox-ripple-color: rgba(38, 133, 232, .252); }
@media (prefers-color-scheme: dark) { :root:not(.theme-light) .inbox-row { --inbox-ripple-color: rgba(38, 133, 232, .252); } }
.inbox-row.selected { background: color-mix(in srgb, #2685e8 12%, transparent); }
.inbox-row:hover { background: color-mix(in srgb, #2685e8 8%, transparent); }
/* Files dragged over the window (docs/specs/popup-inbox-collapse.md C28): every row that takes them reads as a target, and
   the one under the pointer is lit. The row that a wait is about to open (C31) fills a bar along its bottom edge. */
.inbox-root[data-dragging="yes"] .inbox-row.can-drop { box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 30%, transparent); }
.inbox-root[data-dragging="yes"] .inbox-row.drop-target { background: color-mix(in srgb, var(--accent) 18%, transparent); box-shadow: inset 0 0 0 2px var(--accent); }
.inbox-row.dwelling::after {
  content: ""; position: absolute; z-index: 2; left: 8px; bottom: 0; height: 3px; border-radius: 3px; background: var(--accent);
  animation: inbox-dwell var(--dwell-ms) linear forwards;
}
@keyframes inbox-dwell { from { width: 0; } to { width: calc(100% - 16px); } }
.inbox-row.flash { animation: inbox-row-arrival 640ms cubic-bezier(.22, 0, .22, 1); }
.inbox-dot { position: relative; z-index: 1; flex: 0 0 6px; height: 6px; margin-top: 5px; }
.inbox-dot.unread::before { content: ''; position: absolute; left: -2px; top: -2px; width: 10px; height: 10px; border-radius: 50%; background: #2685e8; animation: inbox-unread-dot 2.8s cubic-bezier(.4, 0, .2, 1) infinite; }
.inbox-row-content { position: relative; z-index: 1; min-width: 0; flex: 1; transition: opacity 240ms ease; }
.inbox-row strong { display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; font-size: 12px; line-height: 1.45; font-weight: 500; padding-right: 18px; }
.inbox-kind { display: inline-block; vertical-align: 1px; font-size: 9px; line-height: 1.4; font-weight: 400; padding: 1px 4px; border: 1px solid var(--border); border-radius: 3px; margin-right: 5px; color: var(--text-secondary); }
.inbox-row-meta {
  display: flex; align-items: center; gap: 5px; min-width: 0; font-size: 10px; line-height: 1.5; color: var(--text-secondary); margin-top: 6px;
  max-height: 20px; overflow: hidden;
  transition: max-height calc(var(--ms) * .8) var(--inbox-ease), margin-top calc(var(--ms) * .8) var(--inbox-ease), opacity calc(var(--ms) * .5) ease;
}
/* Compact in the small window: a row is its title alone. */
.inbox-root:is([data-view="collapsed"], [data-measure]) .inbox-row-meta { max-height: 0; margin-top: 0; opacity: 0; }
.inbox-row-agent { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.inbox-row-meta.has-session .inbox-row-agent { flex: 0 0 auto; max-width: 40%; }
.inbox-source-separator { flex: none; color: var(--text-tertiary); }
.inbox-session { display: flex; align-items: center; gap: 4px; min-width: 0; flex: 1; font-size: 11px; }
.inbox-session svg { flex: none; width: 11px; height: 11px; stroke: currentColor; fill: none; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
.inbox-session-title { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.inbox-row-meta em { flex: none; font-size: 9px; font-style: normal; margin-left: auto; }
/* Set aside rows are only dimmed next to requests that are still waiting; when all are set aside they read normally. */
.inbox-root[data-primary="yes"] .inbox-row.secondary .inbox-row-content { opacity: .5; }
.inbox-root[data-primary="yes"] .inbox-row.secondary:hover .inbox-row-content { opacity: .85; }
/* The label sits over the end of the title: keep the first line clear of it. The small window has no label, and it is
   measured while the root still reads "expanded", so the measuring pass must not leave this room (it would wrap titles
   that fit in the small window and make the window too tall). */
.inbox-root[data-view="expanded"]:not([data-measure]) .inbox-row.secondary strong { padding-right: 90px; }
.row-flag {
  position: absolute; right: 6px; top: 7px; z-index: 3; width: 22px; height: 22px; padding: 0; border: 0; border-radius: 6px;
  display: grid; place-items: center; background: transparent; color: var(--text-secondary); cursor: pointer; opacity: 0; transition: opacity 120ms ease;
}
.inbox-entry:hover .row-flag, .row-flag:focus-visible { opacity: 1; }
.row-flag:hover { background: var(--card-bg); color: var(--text-primary); }
/* The three looks share one grid cell, so the button keeps its size while it swaps; a look that does not apply leaves the cell. */
.row-flag.flag-archive { width: auto; min-width: 22px; height: 22px; padding: 0 5px; }
.flag-archive .fa-stack { display: grid; }
.flag-archive .fa-stack > span {
  grid-area: 1 / 1; display: inline-flex; align-items: center; justify-content: center; gap: 4px;
  font-size: 10px; line-height: 20px; white-space: nowrap; transition: opacity 140ms ease, color 140ms ease;
}
.flag-archive svg { width: 13px; height: 13px; }
.flag-archive .fa-state, .flag-archive .fa-undo { opacity: 0; }
.row-flag.flag-archive:not(.is-secondary) .fa-state, .row-flag.flag-archive:not(.is-secondary) .fa-undo { display: none; }
.row-flag.flag-archive.is-secondary .fa-put { display: none; }
/* A set aside row keeps its label visible (a quiet pill) and turns it into the undo button while hovered or focused. */
.row-flag.flag-archive.is-secondary { opacity: 1; background: var(--card-bg); }
.flag-archive.is-secondary .fa-state { opacity: 1; }
/* The undo button's fill is opaque (tokens.css: translucent washes let what is underneath show through): the small window keeps
   no room for it, so there it is drawn over the end of the title, and the title fades into it rather than running through it. */
.inbox-entry:hover .flag-archive.is-secondary, .flag-archive.is-secondary:focus-visible { background: color-mix(in srgb, var(--accent) 14%, var(--surface-overlay)); color: var(--accent); }
.inbox-root[data-view="collapsed"] .row-flag.flag-archive.is-secondary::before {
  content: ""; position: absolute; right: 100%; top: 0; bottom: 0; width: 16px; pointer-events: none;
  background: linear-gradient(to right, transparent, color-mix(in srgb, var(--accent) 14%, var(--surface-overlay)));
}
.inbox-entry:hover .flag-archive.is-secondary .fa-state, .flag-archive.is-secondary:focus-visible .fa-state { opacity: 0; }
.inbox-entry:hover .flag-archive.is-secondary .fa-undo, .flag-archive.is-secondary:focus-visible .fa-undo { opacity: 1; }
/* The small window lists set aside rows only: no standing label there, just the undo button on hover (or keyboard focus).
   These rules are more specific than the hover look above, so the button's fill has to be restated here. */
.inbox-root[data-view="collapsed"] .row-flag.flag-archive.is-secondary { opacity: 0; background: transparent; }
.inbox-root[data-view="collapsed"] .inbox-entry:hover .row-flag.flag-archive.is-secondary,
.inbox-root[data-view="collapsed"] .row-flag.flag-archive.is-secondary:focus-visible { opacity: 1; background: color-mix(in srgb, var(--accent) 14%, var(--surface-overlay)); }
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
