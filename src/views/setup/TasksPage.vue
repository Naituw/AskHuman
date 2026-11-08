<script setup lang="ts">
// Wizard: the one question about allowing Agent tasks to be created from the phone or IM, on a
// screen of its own (docs/specs/onboarding.md D25). It only asks. It draws what "allow" means (the
// channels that are set up, and the Agents they can create tasks for) and greys out whatever is
// not there yet, because the switch is global and easy to miss when a channel or an Agent is added
// later. Whether a task could start right now is deliberately not checked here: allowing is a
// permission, not a promise, and Settings → Advanced shows what an Agent still needs.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import BrandBadge from "./BrandBadge.vue";
import SetupHeader from "./SetupHeader.vue";
import { isIntegrated } from "./agentRows";
import { CHANNEL_DEFS } from "./channelDefs";
import { useSetupContext } from "./useSetup";

/**
 * The Agents a task can be created on, in the order the README shows them. Antigravity has no way
 * to start one, so it is not on the page.
 */
const FLOW_AGENTS: readonly { id: string; label: string }[] = [
  { id: "claude", label: "Claude Code" },
  { id: "codex", label: "Codex" },
  { id: "cursor", label: "Cursor" },
  { id: "grok", label: "Grok" },
  { id: "pi", label: "Pi" },
];

const { t } = useI18n();
const { rows, iosEnabled, enabledChannelIds, wantTasks, declineTasks } = useSetupContext();

/** The phone app first (it is the one with a "New task" button), then the IM channels. */
const channels = computed(() => [
  { id: "ios", label: t("setup.remote.ios"), on: iosEnabled.value },
  ...CHANNEL_DEFS.map((def) => ({
    id: def.id,
    label: t(def.titleKey),
    on: enabledChannelIds.value.includes(def.id),
  })),
]);

const agents = computed(() =>
  FLOW_AGENTS.map((agent) => ({
    ...agent,
    on: rows.value.some((row) => row.id === agent.id && isIntegrated(row)),
  })),
);

/** Something is greyed out, so the page says what that means. */
const anyOff = computed(
  () => channels.value.some((item) => !item.on) || agents.value.some((item) => !item.on),
);
</script>

<template>
  <div class="settings-body setup-body">
    <div class="setup-page" data-testid="tasks-page">
      <SetupHeader
        :title="t('setup.tasks.title')"
        :lead="t('setup.tasks.lead')"
        lead-testid="tasks-lead"
      >
        <template #icon>
          <span class="setup-header-glyph" aria-hidden="true">
            <svg viewBox="0 0 24 24" focusable="false">
              <rect x="6.5" y="2.5" width="11" height="19" rx="2.8" />
              <path d="M10.6 18.4h2.8" />
            </svg>
          </span>
        </template>
      </SetupHeader>

      <section class="setup-panel" data-testid="tasks-flow">
        <div class="tasks-flow">
          <div class="tasks-col">
            <p class="tasks-col-title">{{ t("setup.tasks.channels") }}</p>
            <ul class="tasks-list" data-testid="tasks-channels">
              <li
                v-for="item in channels"
                :key="item.id"
                class="tasks-item"
                :class="{ off: !item.on }"
                :data-testid="`tasks-channel-${item.id}`"
                :data-on="item.on"
              >
                <BrandBadge :id="item.id" :label="item.label" />
                <span class="tasks-item-label">{{ item.label }}</span>
              </li>
            </ul>
          </div>

          <div class="tasks-arrow" aria-hidden="true">
            <span class="tasks-arrow-label">{{ t("setup.tasks.flow") }}</span>
            <svg viewBox="0 0 72 12" focusable="false">
              <path d="M1 6h68M63 1.5L69 6l-6 4.5" />
            </svg>
          </div>

          <div class="tasks-col">
            <p class="tasks-col-title">{{ t("setup.tasks.agents") }}</p>
            <ul class="tasks-list" data-testid="tasks-agents">
              <li
                v-for="item in agents"
                :key="item.id"
                class="tasks-item"
                :class="{ off: !item.on }"
                :data-testid="`tasks-agent-${item.id}`"
                :data-on="item.on"
              >
                <BrandBadge :id="item.id" :label="item.label" />
                <span class="tasks-item-label">{{ item.label }}</span>
              </li>
            </ul>
          </div>
        </div>

        <p v-if="anyOff" class="setup-fine tasks-note" data-testid="tasks-dim-note">
          {{ t("setup.tasks.dimNote") }}
        </p>
      </section>
    </div>
  </div>

  <footer class="setup-footer">
    <span class="spacer"></span>
    <button class="btn btn-lg" type="button" data-testid="tasks-later" @click="declineTasks">
      {{ t("setup.tasks.later") }}
    </button>
    <button class="btn btn-primary btn-lg" type="button" data-testid="tasks-enable" @click="wantTasks">
      {{ t("setup.tasks.enable") }}
    </button>
  </footer>
</template>
