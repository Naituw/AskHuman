<script setup lang="ts">
// Prototype frame: a cropped macOS desktop (menu bar + wallpaper) hosting the away-mode surfaces.
// Everything inside the capsule / card uses the real design tokens and `.btn` controls.
import { computed } from "vue";
import { TRAY_AWAY, TRAY_IDLE } from "./awayTrayIcons";
import {
  AWAY,
  agentOverview,
  agentSubmenu,
  trayOff,
  trayOn,
  type MacAwayView,
  type MenuNode,
} from "./macAwayMock";

const props = defineProps<{ view: MacAwayView; w: number; h: number }>();

// The status item shows the away glyph whenever away mode is on.
const glyph = computed(() => (props.view === "tray-off" ? TRAY_IDLE : TRAY_AWAY));

const mainNodes = computed<MenuNode[]>(() => (props.view === "tray-off" ? trayOff() : trayOn()));
const isCapsule = computed(() =>
  ["capsule", "capsule-pending", "banner-over", "capsule-center"].includes(props.view),
);
const isCard = computed(() => ["card", "welcome", "menubar-card"].includes(props.view));
// Decided placement: top centre, right under the notch. The two rejected alternatives stay drawable for the
// placement board: top right under the menu bar (`banner-over`) and no floating window at all (`menubar-card`).
const centered = computed(
  () => (isCapsule.value || isCard.value) && props.view !== "banner-over" && props.view !== "menubar-card",
);
const hasLead = (nodes: MenuNode[]) => nodes.some((n) => n.t === "item" && (n.check || n.glyph));
</script>

<template>
  <div
    class="ma-desk"
    :class="{ 'ma-compact': centered }"
    :style="{ width: w + 'px', height: h + 'px' }"
  >
    <!-- Menu bar (right-hand part only) -->
    <div class="ma-menubar">
      <div v-if="centered" class="ma-notch"></div>
      <div class="ma-bar-right">
        <span
          class="ma-status"
          :class="{ open: view === 'tray-on' || view === 'tray-off' || view === 'menubar-card' }"
        >
          <img class="ma-glyph" :src="glyph" alt="" />
          <span v-if="view === 'menubar-card'" class="ma-status-text">1:23</span>
        </span>
        <svg class="ma-sys" viewBox="0 0 24 18" width="17" height="13" fill="none" stroke="currentColor" stroke-width="2.1" stroke-linecap="round">
          <path d="M2.2 6.2a14 14 0 0 1 19.6 0" />
          <path d="M5.8 10a8.8 8.8 0 0 1 12.4 0" />
          <path d="M9.3 13.7a3.8 3.8 0 0 1 5.4 0" />
          <circle cx="12" cy="16.4" r="0.9" fill="currentColor" stroke="none" />
        </svg>
        <svg class="ma-sys" viewBox="0 0 28 14" width="25" height="12" fill="none">
          <rect x="1" y="1" width="23" height="12" rx="3.6" stroke="currentColor" stroke-opacity=".5" />
          <rect x="2.9" y="2.9" width="14.5" height="8.2" rx="2" fill="currentColor" />
          <path d="M26 5v4" stroke="currentColor" stroke-opacity=".55" stroke-width="1.6" stroke-linecap="round" />
        </svg>
        <svg class="ma-sys" viewBox="0 0 20 16" width="16" height="13" fill="none" stroke="currentColor" stroke-width="1.3">
          <rect x="1" y="1.4" width="18" height="5" rx="2.5" />
          <circle cx="5" cy="3.9" r="1.5" fill="currentColor" stroke="none" />
          <rect x="1" y="9.6" width="18" height="5" rx="2.5" />
          <circle cx="15" cy="12.1" r="1.5" fill="currentColor" stroke="none" />
        </svg>
        <span class="ma-clock">周三 14:32</span>
      </div>
    </div>

    <!-- Collapsed capsule -->
    <div
      v-if="isCapsule"
      class="ma-hud ma-capsule"
      :class="{ center: centered }"
    >
      <svg class="ma-moon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
        <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
      </svg>
      <span class="ma-cap-title">离开模式</span>
      <span class="ma-cap-time">{{ AWAY.elapsed }}</span>
      <span v-if="view === 'capsule-pending'" class="ma-pend">{{ AWAY.pending }}</span>
    </div>

    <!-- Expanded card (away running) / welcome-back card -->
    <div v-else-if="isCard" class="ma-hud ma-card" :class="{ popover: view === 'menubar-card', center: centered }">
      <span v-if="view === 'menubar-card'" class="ma-arrow"></span>
      <div class="ma-card-head">
        <span class="ma-card-icon">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
          </svg>
        </span>
        <div class="ma-card-titles">
          <div class="ma-card-title">{{ view === "welcome" ? "欢迎回来" : "离开模式" }}</div>
          <div class="ma-card-sub">
            <template v-if="view === 'welcome'">你离开了 {{ AWAY.elapsed }}，离开模式仍在运行</template>
            <template v-else>{{ AWAY.scope }} · {{ AWAY.since }} 开始</template>
          </div>
        </div>
        <span v-if="view === 'card'" class="ma-card-elapsed">已离开 {{ AWAY.elapsed }}</span>
      </div>

      <div class="ma-sec-label">期间自动回复 {{ AWAY.total }} 次</div>
      <ul class="ma-rows">
        <li v-for="a in AWAY.agents" :key="a.title" class="ma-row">
          <span class="ma-row-kind">{{ a.kind }}</span>
          <span class="ma-row-title">{{ a.title }}</span>
          <span class="ma-row-count">{{ a.count }} 次</span>
        </li>
      </ul>

      <div class="ma-pending">
        <span class="ma-pending-dot"></span>
        <span class="ma-pending-text">{{ AWAY.pending }} 个提问在等你处理</span>
        <button class="ma-link" type="button">查看</button>
      </div>

      <div class="ma-actions">
        <button v-if="view === 'welcome'" class="btn" type="button">保持离开</button>
        <button v-else class="btn" type="button">查看记录</button>
        <span class="ma-spacer"></span>
        <button class="btn btn-primary" type="button">结束离开模式</button>
      </div>
    </div>

    <!-- Notification banner (other app) -->
    <div v-if="view === 'banner-over'" class="ma-banner">
      <span class="ma-banner-icon">
        <svg viewBox="0 0 24 24" fill="currentColor"><path d="M12 3C6.5 3 2 6.6 2 11.1c0 2.5 1.4 4.7 3.6 6.2-.2 1-.7 2.3-1.6 3.3-.2.2 0 .6.3.5 1.7-.2 3.3-.9 4.4-1.7.9.2 1.8.3 2.7.3 5.5 0 10-3.6 10-8.1S17.5 3 12 3Z" /></svg>
      </span>
      <div class="ma-banner-body">
        <div class="ma-banner-row">
          <span class="ma-banner-title">小明</span>
          <span class="ma-banner-time">现在</span>
        </div>
        <div class="ma-banner-text">今晚七点老地方见？</div>
      </div>
    </div>

    <!-- Native-looking tray menu -->
    <div
      v-if="view === 'tray-on' || view === 'tray-off'"
      class="ma-menu"
      :class="{ lead: hasLead(mainNodes) }"
      style="top: 27px; right: 6px; width: 292px"
    >
      <template v-for="(n, i) in mainNodes" :key="i">
        <div v-if="n.t === 'sep'" class="ma-sep"></div>
        <div v-else class="ma-mi" :class="{ off: n.enabled === false, hover: n.hover }">
          <span class="ma-mi-lead">
            <svg v-if="n.glyph === 'moon'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" /></svg>
          </span>
          <span class="ma-mi-label">{{ n.label }}</span>
          <svg v-if="n.chev" class="ma-mi-chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" /></svg>
        </div>
      </template>
    </div>

    <!-- Per-agent cascade -->
    <template v-if="view === 'tray-agent'">
      <div class="ma-menu" style="top: 44px; left: 12px; width: 488px">
        <template v-for="(n, i) in agentOverview()" :key="i">
          <div v-if="n.t === 'item'" class="ma-mi" :class="{ hover: n.hover }">
            <span class="ma-mi-label">{{ n.label }}</span>
            <svg v-if="n.chev" class="ma-mi-chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" /></svg>
          </div>
        </template>
      </div>
      <div class="ma-menu lead" style="top: 44px; left: 496px; width: 236px">
        <template v-for="(n, i) in agentSubmenu()" :key="i">
          <div v-if="n.t === 'sep'" class="ma-sep"></div>
          <div v-else class="ma-mi" :class="{ hover: n.hover }">
            <span class="ma-mi-lead">
              <svg v-if="n.check" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5" /></svg>
            </span>
            <span class="ma-mi-label">{{ n.label }}</span>
          </div>
        </template>
      </div>
    </template>
  </div>
</template>
