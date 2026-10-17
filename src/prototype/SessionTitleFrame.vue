<script setup lang="ts">
// One popup window for the "Agent session title in the popup" prototype. The real PopupView runs inside a fake
// window (traffic lights on macOS, a title bar on Windows). Variant A (the title strip) was chosen and now ships in
// PopupNavbar, so it is the product's own DOM here; the rejected variants B-D are still teleported into the real DOM
// for comparison. Which variants are on comes from the `?variant=` list; a cell without `strip` hides the product's
// strip (sessionTitle.css) so it shows its alternative on its own.
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import PopupView from "../views/PopupView.vue";
import InboxSidebar from "../views/popup/InboxSidebar.vue";
import SessionIcon from "../components/SessionIcon.vue";
import type { PopupInboxRequest } from "../lib/types";
import { readParams, sidebarRequests, type Variant } from "./sessionTitleMock";
import "./sessionTitle.css";

const params = readParams();
const mac = params.platform === "mac";
const popup = ref<InstanceType<typeof PopupView> | null>(null);
// The title as the product's popup core holds it: it follows the late `popup-session-title` event too.
const title = computed(() => popup.value?.ctx.agentSessionTitle.value.trim() ?? "");
const has = (variant: Variant) => params.variants.includes(variant);
// On Windows and Linux the page has no title strip, so the product puts the title into the native window title instead
// (`<session title> — AskHuman`, composed by app/popup_title.rs). A browser cannot do that, so this cell fakes the bar.
const nativeTitle = computed(() => (has("strip") && title.value ? `${title.value} — AskHuman` : "AskHuman"));

const sidebarExtra = params.sidebar ? 240 + 6 : 0;
const windowStyle = {
  "--pt-win-w": `${params.width + sidebarExtra}px`,
  "--pt-win-h": `${params.height + (mac ? 0 : 32)}px`,
};
const requests = sidebarRequests(params);
const groups = computed(() => {
  const byProject = new Map<string, PopupInboxRequest[]>();
  for (const request of requests) {
    const list = byProject.get(request.project) ?? [];
    list.push(request);
    byProject.set(request.project, list);
  }
  return [...byProject].map(([path, list]) => ({ path, requests: list }));
});
// The third row has not been looked at yet, so the Sidebar also shows its unread dot.
const visited = new Set(requests.filter((r) => r.sequence !== 3).map((r) => r.requestId));
const none = new Set<string>();

// The popup renders after `popup_init` resolves, and some of its elements are swapped later (the Agent chip turns
// from a span into a button once the terminal is resolved). So the landmarks are tracked as live element refs and the
// variants teleport into whatever element is current.
const contentEl = shallowRef<HTMLElement | null>(null);
const footerEl = shallowRef<HTMLElement | null>(null);
const chipEl = shallowRef<HTMLElement | null>(null);
let observer: MutationObserver | undefined;
function probe() {
  const root = document.querySelector<HTMLElement>(".pt-main .popup-main");
  contentEl.value = root?.querySelector<HTMLElement>(".content") ?? null;
  footerEl.value = root?.querySelector<HTMLElement>(".footer") ?? null;
  chipEl.value = root?.querySelector<HTMLElement>(".brand-agent") ?? null;
}
// `?hover=agent` opens the product's own hover card on the Agent chip, as if the pointer rested on it.
if (params.hover) {
  watch(chipEl, (chip) => {
    const anchor = chip?.closest<HTMLElement>(".source-popover-anchor");
    if (anchor) window.setTimeout(() => anchor.dispatchEvent(new MouseEvent("mouseenter")), 400);
  });
}
onMounted(() => {
  observer = new MutationObserver(probe);
  observer.observe(document.body, { childList: true, subtree: true });
  probe();
});
onBeforeUnmount(() => observer?.disconnect());
</script>

<template>
  <div class="pt-stage" :data-variant="params.variants.join(' ')" :data-platform="params.platform">
    <div class="pt-window" :data-platform="params.platform" :style="windowStyle">
      <div v-if="!mac" class="pt-winbar">
        <span class="pt-win-icon"></span>
        <span class="pt-win-title">{{ nativeTitle }}</span>
        <span class="pt-winctl" aria-hidden="true">
          <span><svg viewBox="0 0 10 10"><path d="M0 5h10" /></svg></span>
          <span><svg viewBox="0 0 10 10"><rect x=".5" y=".5" width="9" height="9" /></svg></span>
          <span><svg viewBox="0 0 10 10"><path d="M0 0l10 10M10 0L0 10" /></svg></span>
        </span>
      </div>
      <div class="pt-body">
        <template v-if="params.sidebar">
          <div class="pt-sidebar inbox-root" data-view="expanded" data-sidebar="on" data-primary="yes" data-lights="keep" data-settled="no">
            <InboxSidebar :groups="groups" :total="requests.length" :selected-id="requests[0].requestId" :visited="visited"
              :arriving="none" :flashed="none" :drafts="none" :secondary="none" :blocked="false" :motion="null" :collapsing-group="null" />
          </div>
          <div class="pt-divider"></div>
        </template>
        <div class="pt-main"><PopupView ref="popup" /></div>
      </div>
      <div v-if="mac" class="pt-lights" aria-hidden="true"><i></i><i></i><i></i></div>
    </div>

    <!-- A (chosen, shipped): the title strip next to the traffic lights comes from PopupNavbar itself. -->
    <!-- B: a quiet line at the top of the scrolling answer area. -->
    <Teleport v-if="has('caption') && contentEl" :to="contentEl">
      <Transition name="pt-fade">
        <div v-if="title" class="pt-caption" :title="title"><SessionIcon /><span>{{ title }}</span></div>
      </Transition>
    </Teleport>
    <!-- C: who the answer goes to, in the free room of the footer. -->
    <Teleport v-if="has('reply') && footerEl" :to="footerEl">
      <Transition name="pt-fade">
        <div v-if="title" class="pt-reply" :title="title">
          <span class="pt-reply-label">回复给</span><SessionIcon /><span class="pt-reply-title">{{ title }}</span>
        </div>
      </Transition>
    </Teleport>
    <!-- D: the title inside the Agent chip itself. -->
    <Teleport v-if="has('inline') && chipEl" :to="chipEl">
      <template v-if="title"><span class="pt-chip-sep">·</span><span class="pt-chip-title">{{ title }}</span></template>
    </Teleport>
  </div>
</template>
