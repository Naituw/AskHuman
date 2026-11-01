<script setup lang="ts">
// 顶部导航栏：来源头部（agent/workspace 胶囊 + 相对时间）+ 右侧动作（自更新入口/置顶/待办/
// 历史/设置）。多根组件：navbar 之外还含自更新浮层背板与「更新待生效」横条（保持 .popup 的
// flex 子级顺序）。
import { computed } from "vue";
import SessionIcon from "../../components/SessionIcon.vue";
import SourcePopover from "./SourcePopover.vue";
import { useI18n } from "vue-i18n";
import { usePopupContext } from "./context";
import { usePopupWindowTitle } from "./usePopupWindowTitle";

const { t } = useI18n();
const {
  request,
  scrolled,
  agentInline,
  headerPrefix,
  headerSuffix,
  agentLabel,
  agentSessionId,
  agentSessionTitle,
  popupRequestId,
  popupActive,
  callerCwd,
  agentFocusable,
  onFocusAgentTerminal,
  projectName,
  projectPath,
  onOpenWorkspace,
  popupTimeRel,
  popupTimeAbs,
  updateAvailable,
  updatePending,
  updateLatest,
  updateApplyMode,
  updatePopoverOpen,
  updating,
  updateStarted,
  updateError,
  updateNotesHtml,
  toggleUpdatePopover,
  applyUpdateFromPopup,
  onContentClick,
  pinned,
  togglePin,
  shrinkAvailable,
  shrinkWindow,
  agentConsoleAvailable,
  openAgentConsoleWindow,
  openTodosWindow,
  openHistoryWindow,
  openSettingsWindow,
} = usePopupContext();

// The Agent session title: drawn in the title strip on macOS, written into the native window title elsewhere.
const sessionTitle = computed(() => {
  const development = request.value?.message.development;
  return development
    ? `${t(development.testProfile ? "development.testProfile" : "development.verification")} · ${development.worktree.replace(/\\/g, "/").split("/").filter(Boolean).pop()}`
    : (agentSessionTitle.value || "").trim();
});
usePopupWindowTitle(sessionTitle, popupActive);
</script>

<template>
  <header class="navbar" :class="{ scrolled }" data-tauri-drag-region>
    <!-- macOS only (CSS): the Agent session title in the title strip beside the traffic lights. It is out of flow, so it adds no height and a late title shifts nothing. -->
    <Transition name="session-strip">
      <div v-if="sessionTitle" class="session-strip">
        <SessionIcon />
        <span class="session-strip-title">{{ sessionTitle }}</span>
      </div>
    </Transition>
    <span class="brand" :class="{ inline: agentInline }">
      <span class="brand-dot"></span>
      <span class="brand-title">{{ headerPrefix }}</span>
      <SourcePopover v-if="agentLabel" kind="agent" :request-id="popupRequestId" :active="popupActive" :enabled="!!agentSessionId" :label="agentLabel" :session-id="agentSessionId" :session-title="agentSessionTitle">
      <component
        :is="agentFocusable ? 'button' : 'span'"
        v-if="agentLabel"
        class="brand-chip brand-agent"
        :class="{ clickable: agentFocusable }"
        :type="agentFocusable ? 'button' : undefined"
        :tabindex="agentFocusable ? undefined : 0"
        :title="!agentSessionId && agentFocusable ? t('agents.focusTerminal') : undefined"
        @click="onFocusAgentTerminal"
      >
        <span class="chip-text">{{ agentLabel }}</span>
        <svg
          v-if="agentFocusable"
          class="chip-arrow"
          viewBox="0 0 10 10"
          aria-hidden="true"
        >
          <path
            d="M3 7 L7 3 M4 3 H7 V6"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
        </svg>
      </component>
      </SourcePopover>
      <SourcePopover v-if="projectName" kind="project" :request-id="popupRequestId" :active="popupActive" :label="projectName" :project="projectPath" :caller-cwd="callerCwd">
      <button
        v-if="projectName"
        type="button"
        class="brand-chip brand-workspace clickable"
        @click="onOpenWorkspace"
      >
        <span class="chip-text">{{ projectName }}</span>
        <svg class="chip-arrow" viewBox="0 0 10 10" aria-hidden="true">
          <path
            d="M3 7 L7 3 M4 3 H7 V6"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
        </svg>
      </button>
      </SourcePopover>
      <span v-if="headerSuffix" class="brand-title brand-suffix">{{
        headerSuffix
      }}</span>
      <span
        v-if="popupTimeRel"
        class="brand-time"
        :title="popupTimeAbs"
        >· {{ popupTimeRel }}</span
      >
    </span>
    <span class="nav-actions">
      <span class="nav-actions-btns">
        <div v-if="updateAvailable" class="update-wrap">
          <button
            class="nav-btn update-btn"
            type="button"
            :title="t('popup.nav.update')"
            @click.stop="toggleUpdatePopover"
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 3v12" />
              <path d="M7 10l5 5 5-5" />
              <path d="M5 21h14" />
            </svg>
            <span class="update-dot"></span>
          </button>
          <div v-if="updatePopoverOpen" class="update-popover" @click.stop>
            <p class="up-title">
              {{ t("popup.update.title", { version: updateLatest }) }}
            </p>
            <div
              v-if="updateNotesHtml"
              class="up-notes markdown-body"
              v-html="updateNotesHtml"
              @click="onContentClick"
            ></div>
            <p v-else class="up-notes muted">{{ t("popup.update.noNotes") }}</p>
            <p class="up-hint">
              {{
                updateApplyMode !== "automatic"
                  ? t("popup.update.manualHint")
                  : updateStarted
                  ? t("popup.update.startedHint")
                  : t("popup.update.applyHint")
              }}
            </p>
            <p v-if="updateError" class="up-error">{{ updateError }}</p>
            <div class="up-actions">
              <button
                class="btn btn-primary"
                type="button"
                :disabled="updating || updateStarted"
                @click="applyUpdateFromPopup"
              >
                {{
                  updateApplyMode !== "automatic"
                    ? t("popup.update.manualButton")
                    : updating
                    ? t("popup.update.updating")
                    : t("popup.update.button")
                }}
              </button>
            </div>
          </div>
        </div>
        <!-- 合并窗口的 peek（只剩「稍后答」）且没有 Sidebar 时，缩回小窗的入口放在图钉旁；显隐由 CSS 按窗口状态决定 -->
        <button
          v-if="shrinkAvailable"
          class="nav-btn nav-shrink"
          type="button"
          :title="t('popup.inbox.shrink')"
          :aria-label="t('popup.inbox.shrink')"
          @click="shrinkWindow"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <path d="M4 14h6v6M20 10h-6V4M14 10l7-7M3 21l7-7" />
          </svg>
        </button>
        <button
          class="nav-btn"
          :class="{ active: pinned }"
          type="button"
          :title="t('popup.nav.pin')"
          @click="togglePin"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 17v5" />
            <path d="M9 10.8a2 2 0 0 1-1.1 1.8l-1.8.9A2 2 0 0 0 5 15.2V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.8a2 2 0 0 0-1.1-1.8l-1.8-.9A2 2 0 0 1 15 10.8V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z" />
          </svg>
        </button>
        <button
          v-if="agentConsoleAvailable"
          class="nav-btn"
          type="button"
          :title="t('popup.nav.agentConsole')"
          :aria-label="t('popup.nav.agentConsole')"
          @click="openAgentConsoleWindow"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="4" width="18" height="16" rx="3" />
            <path d="M7 13h2l2-4 3 7 2-4h2" />
          </svg>
        </button>
        <button
          class="nav-btn"
          type="button"
          :title="t('popup.nav.todos')"
          @click="openTodosWindow"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="4" width="18" height="16" rx="3" />
            <path d="M7 10l2 2 3-3" />
            <path d="M14.5 10.5H18M7 16h11" />
          </svg>
        </button>
        <button
          class="nav-btn"
          type="button"
          :title="t('popup.nav.history')"
          @click="openHistoryWindow"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 3v5h5" />
            <path d="M3.05 13a9 9 0 1 0 2.5-6.36L3 8" />
            <path d="M12 7v5l3 2" />
          </svg>
        </button>
        <button
          class="nav-btn"
          type="button"
          :title="t('popup.nav.settings')"
          @click="openSettingsWindow"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.6 1.6 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.6 1.6 0 0 0-1.8-.3 1.6 1.6 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1a1.6 1.6 0 0 0-1-1.5 1.6 1.6 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.6 1.6 0 0 0 .3-1.8 1.6 1.6 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1a1.6 1.6 0 0 0 1.5-1 1.6 1.6 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.6 1.6 0 0 0 1.8.3H9a1.6 1.6 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.6 1.6 0 0 0 1 1.5 1.6 1.6 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.6 1.6 0 0 0-.3 1.8V9a1.6 1.6 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.6 1.6 0 0 0-1.5 1z" />
          </svg>
        </button>
      </span>
    </span>
  </header>
  <div
    v-if="updatePopoverOpen"
    class="update-backdrop"
    @click="updatePopoverOpen = false"
  ></div>
  <div v-if="updatePending" class="update-pending-banner">
    {{ t("popup.update.pendingBanner") }}
  </div>
</template>

<style>
/*
 * The Agent session title (docs/specs/popup-session-info.md). macOS only: Windows / Linux show it in the
 * native window title instead.
 *
 * It sits in the title strip the traffic lights leave empty (the navbar's 30px top padding in popup.css),
 * centered between the lights and the right edge. It is out of flow, so it adds no height and a title that
 * arrives late moves nothing, and with no title the window is exactly as it was. It is display only: pointer
 * events pass through to the draggable navbar, and the full title stays in the Agent chip's hover card.
 */
.session-strip {
  display: none;
}
.macos .navbar {
  position: relative;
}
.macos .session-strip {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 30px;
  /* Clear the traffic lights on the left and mirror it on the right so the title stays centered. */
  padding: 0 88px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  pointer-events: none;
  font-size: 12px;
  line-height: 16px;
  font-weight: 500;
  letter-spacing: 0.1px;
  color: var(--text-secondary);
}
.session-strip svg {
  flex: none;
  width: 12px;
  height: 12px;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.session-strip-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.session-strip-enter-active {
  transition: opacity 180ms ease;
}
.session-strip-enter-from {
  opacity: 0;
}
@media (prefers-reduced-motion: reduce) {
  .session-strip-enter-active {
    transition: none;
  }
}
</style>
