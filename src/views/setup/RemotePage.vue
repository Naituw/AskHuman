<script setup lang="ts">
// Wizard page 2: remote answering (docs/specs/onboarding.md D10, D18, D19, D22). One tile per IM
// channel with its state; one channel's detail form at a time.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { openSettings } from "../../lib/ipc";
import BrandBadge from "./BrandBadge.vue";
import {
  CHANNEL_DEFS,
  channelDef,
  channelState,
  type ChannelDef,
  type ChannelState,
} from "./channelDefs";
import SetupChannelForm from "./SetupChannelForm.vue";
import SetupHeader from "./SetupHeader.vue";
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

/** A field's label without its parenthetical alias ("ClientId（AppKey）" reads "ClientId"). */
function shortLabel(key: string): string {
  return t(key).replace(/\s*[（(][^）)]*[）)]/g, "");
}

/** What the user has to type for the channel; a field the auto-detect fills in is not named. */
function needs(def: ChannelDef): string {
  const items = def.fields
    .filter((field) => !field.detected)
    .map((field) => shortLabel(field.labelKey));
  return t("setup.remote.needs", { items: items.join(" · ") });
}
</script>

<template>
  <SetupChannelForm v-if="openDef" :def="openDef" />
  <template v-else>
    <div class="settings-body setup-body">
      <div class="setup-page">
        <SetupHeader :title="t('setup.remote.title')" :lead="t('setup.remote.lead')">
          <template #icon>
            <span class="setup-header-glyph" aria-hidden="true">
              <svg viewBox="0 0 24 24" focusable="false">
                <path
                  d="M21 11.5a8.4 8.4 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.4 8.4 0 0 1-3.8-.9L3 21l1.9-5.7a8.4 8.4 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.4 8.4 0 0 1 3.8-.9h.5a8.5 8.5 0 0 1 8 8v.5z"
                />
              </svg>
            </span>
          </template>
          <template #tag>
            <span class="setup-optional">{{ t("setup.remote.optional") }}</span>
          </template>
        </SetupHeader>

        <div class="channel-grid" data-testid="remote-list">
          <div
            v-for="row in list"
            :key="row.def.id"
            class="channel-tile"
            :class="tone(row.state)"
            :data-testid="`channel-row-${row.def.id}`"
          >
            <div class="tile-head">
              <BrandBadge :id="row.def.id" :label="t(row.def.titleKey)" />
              <span class="tile-id">
                <span class="tile-name">
                  {{ t(row.def.titleKey) }}
                  <span v-if="row.def.recommended" class="setup-recommended">{{
                    t("settings.channels.recommendedBadge")
                  }}</span>
                </span>
                <!-- What to prepare is only useful until the channel is set up. -->
                <span v-if="!row.configured" class="tile-sub">{{ needs(row.def) }}</span>
              </span>
            </div>
            <p v-if="row.issue" class="tile-note warn">{{ row.issue }}</p>
            <div class="tile-foot">
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
          </div>

          <div v-if="showIos" class="channel-tile" data-testid="channel-row-ios">
            <div class="tile-head">
              <BrandBadge id="ios" :label="t('setup.remote.ios')" />
              <span class="tile-id">
                <span class="tile-name">{{ t("setup.remote.ios") }}</span>
              </span>
            </div>
            <div class="tile-foot">
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

        <p v-if="!hasEnabledChannel" class="setup-note center" data-testid="remote-skip-note">
          {{ t("setup.remote.skipNote") }}
        </p>
      </div>
    </div>

    <footer class="setup-footer">
      <span class="spacer"></span>
      <button
        v-if="hasEnabledChannel"
        class="btn btn-primary btn-lg"
        type="button"
        data-testid="remote-continue"
        @click="goto('done')"
      >
        {{ t("setup.common.continue") }}
      </button>
      <button
        v-else
        class="btn btn-lg"
        type="button"
        data-testid="remote-skip"
        @click="skipRemote"
      >
        {{ t("setup.remote.skip") }}
      </button>
    </footer>
  </template>
</template>
