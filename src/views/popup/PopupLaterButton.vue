<script setup lang="ts">
// 「稍后回答 / 取消稍后答」按钮（合并窗口）：同一个位置随状态切换文案，三种底栏排布共用。
// 独立的单请求窗口没有这个入口（laterAvailable 为 false 时不渲染）。
import { useI18n } from "vue-i18n";
import { usePopupContext } from "./context";

const { t } = useI18n();
const { laterAvailable, laterMarked, submitting, toggleLater } = usePopupContext();
</script>

<template>
  <button
    v-if="laterAvailable"
    class="btn btn-later"
    type="button"
    :disabled="submitting"
    :title="laterMarked ? t('popup.inbox.undoLaterHint') : t('popup.inbox.laterHint')"
    @click="toggleLater"
  >
    {{ laterMarked ? t("popup.inbox.undoLater") : t("popup.inbox.later") }}
  </button>
</template>
