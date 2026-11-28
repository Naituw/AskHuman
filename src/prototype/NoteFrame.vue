<script setup lang="ts">
// One popup window for the "Note" (留言) prototype (docs/specs/agent-notes.md). The real PopupView runs inside a fake
// window and shows the Agent's message as the product does; the real InboxSidebar lists the notes among the questions.
// What this prototype adds is drawn over that DOM: the note tag above the message, the reply field's states, the
// single primary button that names what will happen (Received / Interject / Wake), and the wake confirmation.
// The scene comes from the URL (see `Scene` in noteMock.ts).
import { computed, onBeforeUnmount, onMounted, shallowRef } from "vue";
import PopupView from "../views/PopupView.vue";
import InboxSidebar from "../views/popup/InboxSidebar.vue";
import type { PopupInboxRequest } from "../lib/types";
import { i18n } from "../i18n";
import { readParams, sidebarRequests, REPLY_TEXT } from "./noteMock";
import "./replyMenu.css";
import "./note.css";

const params = readParams();
const scene = params.scene;
const mini = scene === "mini" || scene === "mini-notes";
const listed = scene === "list" || mini;
const SIDEBAR = 252;

// What each scene shows of the reply field and the button.
type Mode = "ack" | "interject" | "wake";
const typed = scene === "typed" || scene === "idle" || scene === "confirm";
const mode: Mode = scene === "typed" ? "interject" : scene === "idle" || scene === "confirm" ? "wake" : "ack";
const locked = scene === "locked";
const label: Record<Mode, string> = { ack: "已收到", interject: "插话", wake: "唤醒" };

const windowStyle = {
  "--rp-win-w": `${mini ? SIDEBAR : params.width + (listed ? SIDEBAR + 6 : 0)}px`,
  "--rp-win-h": mini ? "auto" : `${params.height}px`,
};

const requests = sidebarRequests(scene === "mini-notes" ? "note" : undefined);
// The mock marks a note with the kind the product will derive from the request's `note` flag (see noteMock.ts).
const isNote = (r: PopupInboxRequest) => (r.kind as string | undefined) === "note";
const asks = requests.filter((r) => !isNote(r));
const notes = requests.filter(isNote);
// With only notes left the heading names them instead of "pending": nothing is waiting for an answer.
if (!asks.length) i18n.global.mergeLocaleMessage("zh", { popup: { inbox: { pending: "留言" } } });
const groups = computed(() => {
  const byProject = new Map<string, PopupInboxRequest[]>();
  for (const request of requests) {
    const list = byProject.get(request.project) ?? [];
    list.push(request);
    byProject.set(request.project, list);
  }
  return [...byProject].map(([path, list]) => ({ path, requests: list }));
});
// The third row (a note) has not been looked at yet: its unread dot shows.
const visited = new Set(requests.filter((r) => r.sequence !== 3).map((r) => r.requestId));
const none = new Set<string>();
const selected = requests[0]?.requestId ?? null;

const contentEl = shallowRef<HTMLElement | null>(null);
const footerEl = shallowRef<HTMLElement | null>(null);
const headingEl = shallowRef<HTMLElement | null>(null);
let observer: MutationObserver | undefined;
let filled = false;

function probe() {
  const root = document.querySelector<HTMLElement>(".nt-main .popup-main");
  contentEl.value = root?.querySelector<HTMLElement>(".content") ?? null;
  footerEl.value = root?.querySelector<HTMLElement>(".footer") ?? null;
  headingEl.value = document.querySelector<HTMLElement>(".nt-sidebar .inbox-heading > span:first-child");
  const area = root?.querySelector<HTMLTextAreaElement>("textarea");
  if (area) {
    area.placeholder = "回复这条留言";
    area.disabled = locked;
    if (typed && !filled) {
      filled = true;
      // Through the field's own event, so the popup core holds the text as if it had been typed.
      area.value = REPLY_TEXT;
      area.dispatchEvent(new Event("input", { bubbles: true }));
    }
  }
  // The Sidebar's own rows: a note's row has no "later" flag (a note is not waiting for an answer).
  for (const entry of document.querySelectorAll<HTMLElement>(".nt-sidebar .inbox-entry")) {
    const tag = entry.querySelector(".inbox-kind");
    if (tag?.textContent?.trim() === "留言") entry.setAttribute("data-note", "1");
  }
}
onMounted(() => {
  observer = new MutationObserver(probe);
  observer.observe(document.body, { childList: true, subtree: true });
  probe();
});
onBeforeUnmount(() => observer?.disconnect());
</script>

<template>
  <div class="rp-stage nt-stage" :data-scene="scene" :data-locked="locked ? 'yes' : 'no'" :data-mode="mode">
    <div class="rp-window nt-window" data-platform="mac" :style="windowStyle">
      <div class="rp-body">
        <template v-if="listed">
          <div
            class="nt-sidebar inbox-root"
            :data-view="mini ? 'collapsed' : 'expanded'"
            data-sidebar="on"
            data-primary="yes"
            data-lights="keep"
            :data-settled="mini ? 'yes' : 'no'"
          >
            <InboxSidebar
              :groups="groups"
              :total="asks.length || notes.length"
              :selected-id="mini ? null : selected"
              :visited="visited"
              :arriving="none"
              :flashed="none"
              :drafts="none"
              :secondary="none"
              :blocked="false"
              :motion="null"
              :collapsing-group="null"
            />
          </div>
          <div v-if="!mini" class="nt-divider"></div>
        </template>
        <div v-if="!mini" class="rp-main nt-main"><PopupView /></div>
      </div>
      <div class="rp-lights" aria-hidden="true"><i></i><i></i><i></i></div>
      <div v-if="mini" class="nt-mini-ring" aria-hidden="true"></div>

      <!-- The note is not asking anything, so the heading counts the questions and names the notes beside them. -->
      <Teleport v-if="headingEl && notes.length" :to="headingEl">
        <span class="nt-heading-notes">{{ asks.length ? `· 留言 ${notes.length}` : "" }}</span>
      </Teleport>

      <!-- The confirmation of a wake: a new turn costs. -->
      <div v-if="scene === 'confirm'" class="nt-confirm" role="alertdialog" aria-labelledby="nt-confirm-title">
        <h3 id="nt-confirm-title">重新唤醒 Agent？</h3>
        <p>它这一轮已经结束了。发送后会开启新的一轮，会产生新的用量。</p>
        <div class="nt-confirm-actions">
          <button class="btn" type="button">取消</button>
          <button class="btn btn-primary" type="button">唤醒并发送</button>
        </div>
      </div>
    </div>

    <!-- The note tag: what kind of message this is, and that nothing is waiting for the user. -->
    <Teleport v-if="!mini && contentEl" :to="contentEl">
      <div class="nt-tag"><svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="5" width="18" height="14" rx="3" /><path d="m4 8 8 5.5L20 8" /></svg>留言<i>·</i>无需回复</div>
    </Teleport>

    <!-- The footer has one button, named after what it will do. -->
    <Teleport v-if="!mini && footerEl" :to="footerEl">
      <button class="btn nt-send" :class="{ 'btn-primary': mode !== 'ack' }" type="button">
        <svg v-if="mode === 'ack'" class="nt-check" viewBox="0 0 24 24" aria-hidden="true"><path d="m5 12.5 4.5 4.5L19 7" /></svg>{{ label[mode] }} <kbd class="sc">⌘↵</kbd>
      </button>
    </Teleport>
  </div>
</template>
