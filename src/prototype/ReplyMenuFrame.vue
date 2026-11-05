<script setup lang="ts">
// One popup window for the "quick replies and away / mute directives" prototype. The real PopupView runs inside a
// fake window; what the prototype adds is teleported into the real DOM so it sits among the product's own controls:
//  - the "Quick reply" menu button in the footer (a request-level control, not one per question),
//  - the armed bar above the footer (a directive picked from the menu waits there until Send),
//  - the away banner under the navbar (an escalated request while away mode is on).
// Which state is drawn comes from the URL (`?scene=menu|armed-away|armed-mute|escalated&q=1|3`); the controls also work.
import { computed, onBeforeUnmount, onMounted, ref, shallowRef } from "vue";
import PopupView from "../views/PopupView.vue";
import { readParams } from "./replyMenuMock";
import "./replyMenu.css";

const params = readParams();
const mac = params.platform === "mac";
const windowStyle = {
  "--rp-win-w": `${params.width}px`,
  "--rp-win-h": `${params.height + (mac ? 0 : 32)}px`,
};

const menuOpen = ref(params.scene === "menu");
const armed = ref<"away" | "mute" | null>(params.scene === "armed-away" ? "away" : params.scene === "armed-mute" ? "mute" : null);
const scope = ref<"all" | "session">("all");
const note = ref(params.note);
const awayOn = params.away;

const presets = ["继续", "按你的判断来，不用等我", "先跳过这个，做别的", "再详细解释一下", "都不合适，换个思路"];
const unanswered = computed(() => (params.questions === 1 ? (params.answered ? 0 : 1) : params.answered ? 2 : 3));
const sendHint = computed(() =>
  params.questions === 1 || unanswered.value === 0 ? "点击即发送" : `点击即发送 · 未答的 ${unanswered.value} 题同样回复`,
);
const armedLabel = computed(() => (armed.value === "mute" ? "发送并停用 AskHuman" : "发送并开启离开模式"));

function arm(kind: "away" | "mute") {
  armed.value = kind;
  menuOpen.value = false;
}

// The popup renders after `popup_init` resolves, so the landmarks are tracked as live element refs.
const mainEl = shallowRef<HTMLElement | null>(null);
const footerEl = shallowRef<HTMLElement | null>(null);
let observer: MutationObserver | undefined;
function probe() {
  const main = document.querySelector<HTMLElement>(".rp-main .popup-main");
  mainEl.value = main;
  footerEl.value = main?.querySelector<HTMLElement>(":scope > .footer") ?? null;
}
onMounted(() => {
  observer = new MutationObserver(probe);
  observer.observe(document.body, { childList: true, subtree: true });
  probe();
  if (params.answered) {
    // The first option of the first question, as if the user had already picked it. The click bubbles to the stage
    // (which closes the menu on an outside click), so the scene's own state is restored right after.
    window.setTimeout(() => {
      document.querySelector<HTMLElement>(".rp-main .option")?.click();
      menuOpen.value = params.scene === "menu";
    }, 350);
  }
});
onBeforeUnmount(() => observer?.disconnect());
</script>

<template>
  <div class="rp-stage" :data-scene="params.scene" @click="menuOpen = false">
    <div class="rp-window" :class="{ 'rp-armed': !!armed, 'rp-away': awayOn }" :data-platform="params.platform" :style="windowStyle">
      <div v-if="!mac" class="rp-winbar">
        <span class="rp-win-icon"></span>
        <span class="rp-win-title">AskHuman</span>
      </div>
      <div class="rp-body">
        <div class="rp-main"><PopupView ref="popup" /></div>
      </div>
      <div v-if="mac" class="rp-lights" aria-hidden="true"><i></i><i></i><i></i></div>
    </div>

    <!-- Footer: "answer later" (what the merged window has), then the request-level reply menu, then the product's own buttons. -->
    <Teleport v-if="footerEl" :to="footerEl">
      <button v-if="params.later" class="btn btn-later rp-later" type="button">稍后回答</button>
      <div class="rp-anchor" @click.stop>
        <button class="btn rp-trigger" :class="{ open: menuOpen }" type="button" aria-haspopup="menu" :aria-expanded="menuOpen" @click="menuOpen = !menuOpen">
          快捷回复
          <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="m7 15 5 5 5-5" />
            <path d="m7 9 5-5 5 5" />
          </svg>
        </button>

        <div v-if="menuOpen" class="rp-menu" role="menu">
          <div class="rp-head">
            <span>快捷回复</span>
            <span class="rp-head-hint">{{ sendHint }}</span>
          </div>
          <button v-for="(text, i) in presets" :key="text" class="rp-item" :class="{ hover: params.hover === i }" type="button" role="menuitem">
            <span class="rp-item-text">{{ text }}</span>
            <kbd class="rp-key">{{ i + 1 }}</kbd>
          </button>

          <div class="rp-sep"></div>

          <button v-if="!awayOn" class="rp-item rp-item-two" :class="{ hover: params.hover === presets.length }" type="button" role="menuitem" @click="arm('away')">
            <svg class="rp-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
            </svg>
            <span class="rp-item-body">
              <span class="rp-item-text">离开模式…</span>
              <span class="rp-item-sub">我暂时不在，Agent 的提问不再弹出，让它自己推进</span>
            </span>
          </button>
          <button v-else class="rp-item rp-item-two" type="button" role="menuitem">
            <svg class="rp-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
            </svg>
            <span class="rp-item-body">
              <span class="rp-item-text">结束离开模式</span>
              <span class="rp-item-sub">离开模式开启中，结束后 Agent 的提问照常弹出</span>
            </span>
          </button>
          <button class="rp-item rp-item-two" :class="{ hover: params.hover === presets.length + 1 }" type="button" role="menuitem" @click="arm('mute')">
            <svg class="rp-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M8.7 3A6 6 0 0 1 18 8a21.3 21.3 0 0 0 .6 5" />
              <path d="M17 17H3s3-2 3-9a4.67 4.67 0 0 1 .3-1.7" />
              <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" />
              <path d="m2 2 20 20" />
            </svg>
            <span class="rp-item-body">
              <span class="rp-item-text">本会话不再使用 AskHuman…</span>
              <span class="rp-item-sub">这个会话里 Agent 不再向我提问</span>
            </span>
          </button>

          <div class="rp-sep"></div>
          <button class="rp-manage" type="button" role="menuitem">
            管理快捷回复…
          </button>
        </div>
      </div>
      <!-- While a directive is armed the send button says what sending will do. -->
      <button v-if="armed" class="btn btn-primary rp-send" type="button">
        {{ armedLabel }} <kbd class="sc">⌘↵</kbd>
      </button>
    </Teleport>

    <!-- Armed bar: the directive waits here until Send. -->
    <Teleport v-if="mainEl && armed" :to="mainEl">
      <div class="rp-bar" role="group">
        <template v-if="armed === 'away'">
          <svg class="rp-bar-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
          </svg>
          <span class="rp-bar-label">离开模式</span>
          <select v-model="scope" class="select rp-scope" aria-label="范围">
            <option value="all">所有 Agent</option>
            <option value="session">仅此会话</option>
          </select>
          <input v-model="note" class="rp-note" type="text" placeholder="附加指示（可选），例如：别动生产环境" aria-label="附加指示" />
        </template>
        <template v-else>
          <svg class="rp-bar-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M8.7 3A6 6 0 0 1 18 8a21.3 21.3 0 0 0 .6 5" />
            <path d="M17 17H3s3-2 3-9a4.67 4.67 0 0 1 .3-1.7" />
            <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" />
            <path d="m2 2 20 20" />
          </svg>
          <span class="rp-bar-label">本会话不再使用 AskHuman</span>
          <span class="rp-bar-text">Agent 不再向你提问，权限审批与出错通知除外</span>
        </template>
        <button class="rp-bar-close" type="button" title="取消" aria-label="取消" @click="armed = null">
          <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
            <line x1="2" y1="2" x2="10" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
            <line x1="10" y1="2" x2="2" y2="10" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
          </svg>
        </button>
      </div>
    </Teleport>

    <!-- Away banner: this request was escalated by the Agent while away mode is on. -->
    <Teleport v-if="mainEl && awayOn" :to="mainEl">
      <div class="rp-banner" role="status">
        <svg class="rp-bar-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
        </svg>
        <span class="rp-banner-text"><b>离开模式开启中</b>，Agent 请求你处理（无法继续，或已全部完成）</span>
        <button class="rp-banner-action" type="button">结束离开模式</button>
      </div>
    </Teleport>
  </div>
</template>
