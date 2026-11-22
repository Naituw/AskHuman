<script setup lang="ts">
// A finished setup in two lines (docs/specs/ios-setup-experience.md §6.2, §9): what is connected,
// with the way back to the steps, and the test question that proves the whole chain on demand.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { injectIosChannel } from "./iosChannelContext";
import { useIosTestQuestion } from "./useIosTestQuestion";

defineProps<{
  /** The broker service by name. */
  name: string;
  stepsOpen: boolean;
}>();
const emit = defineEmits<{ toggleSteps: [] }>();

const { t } = useI18n();
const { activePhones } = injectIosChannel();
const test = useIosTestQuestion();

const online = computed(() => activePhones.value.filter((p) => p.online).length);

const testText = computed(() => {
  switch (test.state.value) {
    case "sent":
      return t("settings.channels.ios.testQuestion.sent");
    case "answered":
      return t("settings.channels.ios.testQuestion.answered");
    case "timeout":
      return t("settings.channels.ios.testQuestion.timeout");
    case "cancelled":
      return t("settings.channels.ios.testQuestion.cancelled");
    case "failed":
      return test.error.value
        ? t("settings.channels.ios.testQuestion.failed", { error: test.error.value })
        : t("settings.channels.ios.testQuestion.failedPlain");
    default:
      return t("settings.channels.ios.testQuestion.idle");
  }
});
</script>

<template>
  <div class="ios-ready" data-testid="ios-ready">
    <div class="ios-ready-line">
      <span class="ios-dot on"></span>
      <span data-testid="ios-ready-line">{{
        online
          ? t("settings.channels.ios.ready.online", { name, n: online }, online)
          : t("settings.channels.ios.ready.offline", { name })
      }}</span>
      <span class="spacer"></span>
      <a class="link" href="#" data-testid="ios-steps-toggle" @click.prevent="emit('toggleSteps')">
        {{ stepsOpen ? t("settings.channels.ios.stepsHide") : t("settings.channels.ios.stepsShow") }}
      </a>
    </div>
    <!-- A real question to the phone, answered there: it proves broker, relay, push and key at once. -->
    <div class="ios-ready-line ios-try">
      <span class="ios-try-text" data-testid="ios-try-text">{{ testText }}</span>
      <span class="spacer"></span>
      <button
        class="btn"
        type="button"
        :disabled="test.state.value === 'sent'"
        data-testid="ios-try"
        @click="test.send"
      >
        {{
          test.state.value === "answered"
            ? t("settings.channels.ios.testQuestion.again")
            : t("settings.channels.ios.testQuestion.send")
        }}
      </button>
    </div>
  </div>
</template>
