<script setup lang="ts">
// "Set up with an Agent": the prompt to hand to one, shown in full and copied only on a click, so
// nothing reaches the clipboard that the person has not seen. The rules the Agent follows (which
// steps only the person may run, that credentials never pass through it) are in the output of the
// command the prompt names, not here (docs/specs/ios-setup-experience.md §7).
import { onBeforeUnmount, ref } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
const copied = ref(false);
let timer: ReturnType<typeof setTimeout> | undefined;

async function copyPrompt() {
  try {
    await navigator.clipboard.writeText(t("settings.channels.ios.agentPrompt"));
  } catch {
    /* clipboard unavailable: the prompt is on screen and can be selected */
  }
  copied.value = true;
  clearTimeout(timer);
  timer = setTimeout(() => (copied.value = false), 1500);
}
onBeforeUnmount(() => clearTimeout(timer));
</script>

<template>
  <div class="ios-agent" data-testid="ios-agent-box">
    <div class="ios-agent-head">
      <span class="ios-agent-text">{{ t("settings.channels.ios.agentCaption") }}</span>
      <span class="spacer"></span>
      <button class="btn" type="button" data-testid="ios-agent-copy" @click="copyPrompt">
        {{ copied ? t("settings.channels.ios.agentCopied") : t("settings.channels.ios.agentCopy") }}
      </button>
    </div>
    <pre class="ios-prompt" data-testid="ios-agent-prompt">{{ t("settings.channels.ios.agentPrompt") }}</pre>
  </div>
</template>
