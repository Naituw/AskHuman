<script setup lang="ts">
// 弹窗编排层：状态/逻辑在 ./popup/*（usePopupCore 组装、createPopupContext provide，
// 各区块子组件 inject）。此处仅负责根布局：导航栏 / 内容区（确认面板 或 Message+问题区）/
// 页脚 / 根级弹层。样式统一在 ./popup/popup.css（弹窗为独立窗口，天然隔离）。
import { useI18n } from "vue-i18n";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { type PopupScope } from "./popup/usePopupCore";
import { createPopupContext } from "./popup/context";
import { frozenMainStyle, usePreviewDivider } from "./popup/usePreviewDivider";
import PopupNavbar from "./popup/PopupNavbar.vue";
import DevVerificationBanner from "./popup/DevVerificationBanner.vue";
import FindBar from "./popup/FindBar.vue";
import ConfirmPane from "./popup/ConfirmPane.vue";
import MessageSection from "./popup/MessageSection.vue";
import QuestionCards from "./popup/QuestionCards.vue";
import SequentialPane from "./popup/SequentialPane.vue";
import TodoSection from "./popup/TodoSection.vue";
import ComposerDock from "./popup/ComposerDock.vue";
import ArmedDirectiveBar from "./popup/ArmedDirectiveBar.vue";
import EscalatedNotice from "./popup/EscalatedNotice.vue";
import PopupFooter from "./popup/PopupFooter.vue";
import PopupOverlays from "./popup/PopupOverlays.vue";
import AttachmentPreviewPanel from "./popup/AttachmentPreviewPanel.vue";
import AnnotationLayer from "./popup/AnnotationLayer.vue";
import "./popup/popup.css";

const props = defineProps<{ scope?: PopupScope }>();
const ctx = createPopupContext(props.scope);
defineExpose({ ctx });
const { t } = useI18n();
const updating = ref(false);
let connectionListener: UnlistenFn | undefined;
let disposed = false;
onMounted(async () => {
  if (props.scope) return;
  const unlisten = await listen<boolean>("popup-connection", event => {
    updating.value = event.payload;
  });
  if (disposed) unlisten();
  else connectionListener = unlisten;
});
onBeforeUnmount(() => {
  disposed = true;
  connectionListener?.();
});

const {
  request,
  confirmRequest,
  isConfirm,
  loadError,
  submissionError,
  cmdHeld,
  flashing,
  verticalMode,
  dropHint,
  dropNotice,
  contentRef,
  fileRef,
  onScroll,
  onContentWheel,
  onMarkdownContentUpdated,
  onDrop,
  onBackgroundClick,
  onFileChange,
  previewLayout,
  previewOpen,
  previewTransition,
  resizePreview,
  findActive,
  findScope,
  noteFindInteraction,
} = ctx;

const shellStyle = computed(() => {
  const allocation = props.scope?.layout?.value;
  const fixed = !!allocation?.canvas && allocation.canvas.frozen !== false;
  const open = previewLayout.value.side !== "closed";
  return { gridTemplateColumns: open
    ? `${allocation?.mainWidth ?? previewLayout.value.mainWidth}px 6px ${fixed ? `${allocation!.previewWidth}px` : "minmax(0, 1fr)"}`
    : fixed ? `${allocation!.mainWidth}px` : "minmax(0, 1fr)",
    gridTemplateRows: "minmax(0, 1fr)", gridTemplateAreas: open ? "'main divider preview'" : "'main'" };
});
const mainTransitionStyle = computed(() => frozenMainStyle(previewTransition.value));
const { beginDivider } = usePreviewDivider({ mainWidth: () => previewLayout.value.mainWidth, resize: resizePreview });
</script>

<template>
  <div v-if="!request && !confirmRequest" class="popup popup-status">
    <p v-if="loadError" class="status-error">
      {{ t("popup.loadError", { msg: loadError }) }}
    </p>
    <p v-else class="status-loading">{{ t("popup.loading") }}</p>
  </div>

  <div
    v-else
    class="popup-shell"
    @pointerdown.capture="noteFindInteraction"
    @wheel.capture.passive="noteFindInteraction"
    @keydown.capture="noteFindInteraction"
    @focusin.capture="noteFindInteraction"
    :style="shellStyle"
    @dragover.prevent
    @drop.prevent="onDrop"
    @click="onBackgroundClick"
  >
    <div class="popup popup-main" :class="{ 'cmd-held': cmdHeld }" :style="mainTransitionStyle">
    <div v-if="flashing" class="flash-overlay" aria-hidden="true"></div>
    <!-- Files over the answer area (inbox): a dashed frame and a pill that says where they will go. -->
    <Transition name="drop-hint">
      <div v-if="dropHint" class="drop-hint" aria-hidden="true"><span>{{ dropHint }}</span></div>
    </Transition>
    <PopupNavbar />
    <!-- A question that came through away mode says why, with a one-click way out (spec D21). -->
    <EscalatedNotice v-if="!isConfirm" />
    <FindBar v-if="findActive && findScope === 'question'" />
    <p v-if="updating" class="popup-connection-notice" role="status">{{ t("popup.inbox.updating") }}</p>
    <p v-if="submissionError" class="status-error popup-submission-error" role="alert">{{ t("popup.inbox.submitError", { message: submissionError }) }}</p>
    <!-- What became of files dropped on a standalone popup (an inbox page shows its own notice). -->
    <p v-if="dropNotice" class="drop-notice" :data-level="dropNotice.level" :role="dropNotice.level === 'warn' ? 'alert' : 'status'">{{ dropNotice.text }}</p>
    <div
      :ref="(el) => (contentRef = el as HTMLElement | null)"
      class="content"
      @scroll="onScroll"
      @wheel.passive="onContentWheel"
      @markdown-content-updated="onMarkdownContentUpdated"
    >
      <DevVerificationBanner v-if="request?.message.development" :info="request.message.development" />
      <ConfirmPane v-if="isConfirm" />
      <template v-else>
        <MessageSection />
        <QuestionCards v-if="verticalMode" />
        <SequentialPane v-else />
        <!-- 待办下拉区：跟在最后一个问题后面（sequential 多题时仅最后一题面板显示） -->
        <TodoSection v-if="!request?.message.development || !!request?.message.development?.testProfile" />
      </template>
    </div>

    <ComposerDock v-if="!isConfirm" />
    <!-- A quick-reply directive waits here until Send (spec D19). -->
    <ArmedDirectiveBar v-if="!isConfirm" />

    <input
      :ref="(el) => (fileRef = el as HTMLInputElement | null)"
      type="file"
      accept="image/*"
      multiple
      hidden
      @change="onFileChange"
    />

    <PopupFooter />
    </div>
    <div v-if="previewLayout.side !== 'closed'" class="attachment-preview-divider horizontal" role="separator" tabindex="0"
      :aria-label="t('popup.preview.resize')" aria-orientation="vertical"
      @pointerdown.prevent="beginDivider"
      @keydown.left.prevent="resizePreview(previewLayout.mainWidth - 20)"
      @keydown.right.prevent="resizePreview(previewLayout.mainWidth + 20)"
 />
    <AttachmentPreviewPanel v-if="previewOpen && previewLayout.side !== 'closed' && !isConfirm" />
    <!-- Notes on the Agent's text: the button beside a selection and the editor float over the whole window. -->
    <AnnotationLayer v-if="!isConfirm" />
    <PopupOverlays />
  </div>
</template>

<style scoped>
.popup-connection-notice { margin: 0; padding: 8px 14px; color: var(--text-secondary); font-size: 12px; }
</style>
