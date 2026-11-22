<script setup lang="ts">
// "Keep this Mac reachable": the global daemon lifecycle (`general.daemonLifecycle`) as one switch
// where the phone is set up, so the phone does not find this Mac offline. It is the Advanced tab's
// setting, not an iOS one, and "Allow creating Agent tasks" pins it on (docs/specs/ios-setup-experience.md §8).
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useSettingsContext } from "./context";

const { t } = useI18n();
const ctx = useSettingsContext();
const config = computed(() => ctx.config.value!);

const locked = computed(() => config.value.agentTasks.enabled);
const on = computed(() => locked.value || config.value.general.daemonLifecycle === "keepalive");

function toggle(event: Event) {
  const checked = (event.target as HTMLInputElement).checked;
  void ctx.changeDaemonLifecycle(checked ? "keepalive" : "activity");
}
</script>

<template>
  <div class="row" data-testid="ios-keep-row">
    <div class="col">
      <span class="label">{{ t("settings.channels.ios.keep.title") }}</span>
      <p class="ios-subdesc">
        {{ t(locked ? "settings.channels.ios.keep.locked" : "settings.channels.ios.keep.desc") }}
      </p>
    </div>
    <span class="spacer"></span>
    <label class="switch">
      <input
        type="checkbox"
        :checked="on"
        :disabled="locked"
        :aria-label="t('settings.channels.ios.keep.title')"
        data-testid="ios-keep-switch"
        @change="toggle"
      />
      <span class="track"></span>
    </label>
  </div>
</template>
