<script setup lang="ts">
// Max mode for Cursor tasks (spec cursor-max-control §6): the policy, the plan type, and what
// Cursor's own data says about the plan (read-only).
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { agentTaskCursorPlan } from "../../lib/ipc";
import { useSettingsContext } from "./context";

const { t } = useI18n();
const ctx = useSettingsContext();
const { persist } = ctx;
// 父组件仅在 config 加载后渲染本卡片，这里可安全断言非空。
const config = computed(() => ctx.config.value!);

/** `null` while the local data is being read. */
const detected = ref<"legacy" | "token-based" | "unknown" | null>(null);

const detectedLabel = computed(() => {
  switch (detected.value) {
    case "legacy":
      return t("settings.cursorMax.detectedLegacy");
    case "token-based":
      return t("settings.cursorMax.detectedTokenBased");
    case "unknown":
      return t("settings.cursorMax.detectedUnknown");
    default:
      return t("settings.cursorMax.detectedLoading");
  }
});

onMounted(async () => {
  try {
    detected.value = await agentTaskCursorPlan();
  } catch (err) {
    console.warn("cursor plan detection failed", err);
    detected.value = "unknown";
  }
});
</script>

<template>
  <div class="card" data-testid="cursor-max-card">
    <div class="row">
      <div class="col">
        <p class="card-title">{{ t("settings.cursorMax.title") }}</p>
        <p class="card-desc">{{ t("settings.cursorMax.description") }}</p>
      </div>
    </div>
    <hr class="divider" />
    <div class="row">
      <span class="label">{{ t("settings.cursorMax.policy") }}</span>
      <span class="spacer"></span>
      <select
        class="select"
        data-testid="cursor-max-policy"
        v-model="config.agentTasks.cursorMax.policy"
        @change="persist"
      >
        <option value="enforce-off">{{ t("settings.cursorMax.policyEnforceOff") }}</option>
        <option value="inherit">{{ t("settings.cursorMax.policyInherit") }}</option>
      </select>
    </div>
    <p class="card-desc">
      {{
        config.agentTasks.cursorMax.policy === "inherit"
          ? t("settings.cursorMax.policyInheritDesc")
          : t("settings.cursorMax.policyEnforceOffDesc")
      }}
    </p>
    <hr class="divider" />
    <div class="row">
      <span class="label">{{ t("settings.cursorMax.plan") }}</span>
      <span class="spacer"></span>
      <select
        class="select"
        data-testid="cursor-max-plan"
        v-model="config.agentTasks.cursorMax.plan"
        @change="persist"
      >
        <option value="auto">{{ t("settings.cursorMax.planAuto") }}</option>
        <option value="legacy">{{ t("settings.cursorMax.planLegacy") }}</option>
        <option value="token-based">{{ t("settings.cursorMax.planTokenBased") }}</option>
      </select>
    </div>
    <p class="card-desc" data-testid="cursor-max-detected">
      {{ t("settings.cursorMax.detected", { plan: detectedLabel }) }}
    </p>
    <p class="card-desc hint">{{ t("settings.cursorMax.planHint") }}</p>
  </div>
</template>
