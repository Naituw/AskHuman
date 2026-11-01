<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { DevVerificationInfo } from "../../lib/types";

const props = defineProps<{ info: DevVerificationInfo }>();
const { t } = useI18n();
const worktree = computed(() => props.info.worktree.replace(/\\/g, "/").split("/").filter(Boolean).pop());
const built = computed(() => new Date(props.info.build.createdAtMs).toLocaleString());
</script>

<template>
  <details class="dev-verification" data-testid="dev-verification-banner">
    <summary>
      <strong>{{ t(info.testProfile ? "development.testProfile" : "development.verification") }}</strong>
      <span class="dev-worktree">{{ worktree }}</span>
      <span class="dev-built">{{ t("development.built", { time: built }) }}</span>
    </summary>
    <dl>
      <dt>{{ t("development.worktree") }}</dt><dd>{{ info.worktree }}</dd>
      <dt>{{ t("development.binary") }}</dt><dd>{{ info.build.binary }}</dd>
      <template v-if="info.diagnostics"><dt>{{ t("development.home") }}</dt><dd>{{ info.diagnostics.home }}</dd><dt>IPC</dt><dd>{{ info.diagnostics.endpoint }}</dd><dt>{{ t("development.selectionReason") }}</dt><dd>{{ info.diagnostics.reason }}</dd></template>
      <dt>{{ t("development.build") }}</dt><dd>{{ info.build.binaryFingerprint }}</dd>
      <dt>{{ t("development.verificationId") }}</dt><dd>{{ info.id }}</dd>
    </dl>
  </details>
</template>

<style scoped>
.dev-verification { margin: 0 0 14px; padding: 10px 12px; border: 1px solid var(--border); border-inline-start: 3px solid #bf861f; border-radius: 8px; background: color-mix(in srgb, #bf861f 8%, transparent); font-size: 12px; }
summary { cursor: pointer; line-height: 1.6; }
strong { color: var(--text-primary); }
.dev-worktree { margin-inline-start: 8px; overflow-wrap: anywhere; }
.dev-built { display: block; color: var(--text-secondary); margin-inline-start: 14px; }
dl { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 6px 10px; margin: 10px 0 0; }
dt { color: var(--text-secondary); }
dd { margin: 0; user-select: text; overflow-wrap: anywhere; }
</style>
