<script setup lang="ts">
// The global "create Agent tasks from phone or IM" switch, shown inside the channel cards that
// can start them (each IM card and the iOS card) so it sits where the channel is set up and is not
// missed when a channel is added later. It is the same setting as the Advanced tab's switch, so
// turning it on goes through the same confirmation (it keeps the daemon resident). It reads no
// Agent probes of its own: it only says more when the readiness list is already loaded.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useSettingsContext } from "./context";
import { isStartable } from "./readiness";

const { t } = useI18n();
const ctx = useSettingsContext();
const { supportsAgentTasks, toggleAgentTasks, taskReadiness, activeTab } = ctx;
// 父组件仅在 config 加载后渲染本组件，这里可安全断言非空。
const config = computed(() => ctx.config.value!);
const enabled = computed(() => config.value.agentTasks.enabled);

/** On, yet nothing can start: only known once the readiness list was loaded (never probed here). */
const stuck = computed(
  () =>
    enabled.value &&
    taskReadiness.value.length > 0 &&
    !taskReadiness.value.some((item) => isStartable(item)),
);
</script>

<template>
  <div v-if="supportsAgentTasks" class="agent-tasks-row" data-testid="agent-tasks-row">
    <div class="row">
      <div class="col">
        <span class="label">{{ t("settings.agentTasks.title") }}</span>
        <span class="card-desc">{{
          enabled ? t("settings.agentTasks.channelOn") : t("settings.agentTasks.channelOff")
        }}</span>
      </div>
      <span class="spacer"></span>
      <label class="switch">
        <input
          v-model="config.agentTasks.enabled"
          type="checkbox"
          :aria-label="t('settings.agentTasks.title')"
          data-testid="agent-tasks-switch"
          @change="toggleAgentTasks"
        />
        <span class="track"></span>
      </label>
    </div>
    <p v-if="stuck" class="card-desc warn" data-testid="agent-tasks-stuck">
      {{ t("settings.agentTasks.channelStuck") }}
    </p>
    <p v-if="enabled" class="card-desc">
      <a
        class="link"
        href="#"
        data-testid="agent-tasks-manage"
        @click.prevent="activeTab = 'advanced'"
      >
        {{ t("settings.agentTasks.channelManage") }} →
      </a>
    </p>
  </div>
</template>
