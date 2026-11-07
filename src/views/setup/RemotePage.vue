<script setup lang="ts">
// Wizard page 2: remote answering (docs/specs/onboarding.md D10, D18, D19, D22). A list of the
// four IM channels with their state; one channel's detail form at a time.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { openSettings } from "../../lib/ipc";
import {
  CHANNEL_DEFS,
  channelDef,
  channelState,
  type ChannelDef,
  type ChannelState,
} from "./channelDefs";
import SetupChannelForm from "./SetupChannelForm.vue";
import { useSetupContext } from "./useSetup";

const { t } = useI18n();
const ctx = useSetupContext();
const {
  channelRows,
  detail,
  hasEnabledChannel,
  channelIssueText,
  openChannel,
  skipRemote,
  goto,
} = ctx;

const openDef = computed<ChannelDef | undefined>(() =>
  detail.value ? channelDef(detail.value) : undefined,
);

/** iOS is not a guided channel yet: with the experimental features on, it points at its Settings card. */
const showIos = computed(() => ctx.config.value?.experimental?.enabled === true);

interface ListRow {
  def: ChannelDef;
  state: ChannelState;
  issue: string | null;
  configured: boolean;
}

const list = computed<ListRow[]>(() =>
  CHANNEL_DEFS.map((def) => {
    const row = channelRows.value.find((candidate) => candidate.id === def.id) ?? {
      enabled: false,
      configured: false,
      connected: null,
    };
    const issue = channelIssueText(def.id);
    return {
      def,
      state: channelState(row, issue !== null),
      issue,
      configured: row.configured,
    };
  }),
);

function tone(state: ChannelState): "ok" | "warn" | "muted" {
  if (state === "connected" || state === "ready") return "ok";
  if (state === "issue" || state === "incomplete") return "warn";
  return "muted";
}
</script>

<template>
  <SetupChannelForm v-if="openDef" :def="openDef" />
  <template v-else>
    <div class="settings-body setup-body">
      <h1 class="setup-title">{{ t("setup.remote.title") }}</h1>
      <p class="setup-lead">{{ t("setup.remote.lead") }}</p>

      <div class="card setup-channels" data-testid="remote-list">
        <div
          v-for="row in list"
          :key="row.def.id"
          class="setup-channel"
          :data-testid="`channel-row-${row.def.id}`"
        >
          <div class="setup-channel-head">
            <span class="setup-channel-title">{{ t(row.def.titleKey) }}</span>
            <span
              class="setup-tag"
              :class="tone(row.state)"
              :data-testid="`channel-state-${row.def.id}`"
            >
              {{ t(`setup.remote.state.${row.state}`) }}
            </span>
            <span class="spacer"></span>
            <button
              class="btn"
              type="button"
              :data-testid="`channel-open-${row.def.id}`"
              @click="openChannel(row.def.id)"
            >
              {{ row.configured ? t("setup.remote.edit") : t("setup.remote.configure") }}
            </button>
          </div>
          <p v-if="row.issue" class="card-desc warn">{{ row.issue }}</p>
        </div>

        <div v-if="showIos" class="setup-channel" data-testid="channel-row-ios">
          <div class="setup-channel-head">
            <span class="setup-channel-title">{{ t("setup.remote.ios") }}</span>
            <span class="spacer"></span>
            <button
              class="btn"
              type="button"
              data-testid="channel-open-ios"
              @click="openSettings('experimental')"
            >
              {{ t("setup.remote.openInSettings") }}
            </button>
          </div>
        </div>
      </div>
    </div>

    <footer class="setup-footer">
      <span class="spacer"></span>
      <button
        v-if="hasEnabledChannel"
        class="btn btn-primary"
        type="button"
        data-testid="remote-continue"
        @click="goto('done')"
      >
        {{ t("setup.common.continue") }}
      </button>
      <button
        v-else
        class="btn"
        type="button"
        data-testid="remote-skip"
        @click="skipRemote"
      >
        {{ t("setup.remote.skip") }}
      </button>
    </footer>
  </template>
</template>
