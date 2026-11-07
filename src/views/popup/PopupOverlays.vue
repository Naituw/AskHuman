<script setup lang="ts">
// 根级弹层（多根组件）：取消二次确认 + 确认弹窗的关闭警告。
import { useI18n } from "vue-i18n";
import { usePopupContext } from "./context";

const { t } = useI18n();
const {
  isConfirm,
  showCancelConfirm,
  hasAnnotations,
  dismissCancelConfirm,
  doCancel,
  showConfirmCloseWarning,
  dismissConfirmCloseWarning,
  confirmCloseAndDeny,
  confirmComment,
} = usePopupContext();
</script>

<template>
  <!-- 取消二次确认 -->
  <div v-if="!isConfirm && showCancelConfirm" class="confirm-overlay" @click.self="dismissCancelConfirm">
    <div class="confirm-box">
      <p class="confirm-title">{{ t("popup.confirmCancel.title") }}</p>
      <p class="confirm-desc">{{ t(hasAnnotations ? "popup.confirmCancel.descWithNotes" : "popup.confirmCancel.desc") }}</p>
      <div class="confirm-actions">
        <button class="btn" type="button" @click="dismissCancelConfirm">
          {{ t("popup.confirmCancel.keep") }}
        </button>
        <button class="btn btn-danger" type="button" @click="doCancel">
          {{ t("popup.confirmCancel.confirm") }}
        </button>
      </div>
    </div>
  </div>

  <div
    v-if="isConfirm && showConfirmCloseWarning"
    class="confirm-overlay"
    @click.self="dismissConfirmCloseWarning"
  >
    <div class="confirm-box">
      <p class="confirm-title">{{ t("popup.confirmClose.title") }}</p>
      <p class="confirm-desc">
        {{
          confirmComment.trim()
            ? t("popup.confirmClose.descWithReason")
            : t("popup.confirmClose.desc")
        }}
      </p>
      <div class="confirm-actions">
        <button class="btn" type="button" @click="dismissConfirmCloseWarning">
          {{ t("popup.confirmClose.keep") }}
        </button>
        <button class="btn btn-danger" type="button" @click="confirmCloseAndDeny">
          {{ t("popup.confirmClose.deny") }}
        </button>
      </div>
    </div>
  </div>
</template>
