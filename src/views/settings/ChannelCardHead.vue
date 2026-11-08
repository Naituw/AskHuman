<script setup lang="ts">
// Header row of a card in Settings → Channels: the channel's badge and name, an optional
// "recommended" pill, a status tag and the setup-guide link, then the on/off switch
// (docs/specs/settings-agents-channels-cards.md). The card's form below it stays the caller's.
import { useI18n } from "vue-i18n";
import BrandBadge from "../setup/BrandBadge.vue";
import { channelTone, type ChannelState } from "../setup/channelDefs";

defineProps<{
  /** Badge id and test-id suffix: a channel id, or "popup" for the local window. */
  id: string;
  title: string;
  enabled: boolean;
  /** Absent for the local popup, which has no connection to report. */
  state?: ChannelState;
  recommended?: boolean;
  /** Show the setup-guide link. */
  guide?: boolean;
}>();

const emit = defineEmits<{
  toggle: [enabled: boolean];
  guide: [];
}>();

const { t } = useI18n();
</script>

<template>
  <div class="row channel-head">
    <BrandBadge :id="id" :label="title" />
    <div class="channel-id">
      <p class="card-title">{{ title }}</p>
      <span v-if="recommended" class="channel-recommended-badge">{{
        t("settings.channels.recommendedBadge")
      }}</span>
      <span
        v-if="state"
        class="status-tag"
        :class="channelTone(state)"
        :data-testid="`channel-state-${id}`"
      >
        {{ t(`settings.channels.state.${state}`) }}
      </span>
    </div>
    <a v-if="guide" class="link guide-link" href="#" @click.prevent="emit('guide')">
      {{ t("settings.channels.setupGuide") }} ↗
    </a>
    <label class="switch">
      <input
        type="checkbox"
        :checked="enabled"
        :aria-label="title"
        :data-testid="`channel-toggle-${id}`"
        @change="emit('toggle', ($event.target as HTMLInputElement).checked)"
      />
      <span class="track"></span>
    </label>
  </div>
</template>
