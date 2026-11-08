<script setup lang="ts">
// Wizard: the one decision about starting Agent tasks from the phone or IM, on a screen of its own
// inside the remote step (docs/specs/onboarding.md D25). Before the answer it says what the
// feature does and what it costs, with two plain buttons. After "Turn on" it shows what the
// phone's "New task" page will list, and says now — not on the phone, later — what is still
// missing. The check is advisory: it never blocks.
import { computed, onBeforeUnmount, onMounted, watch } from "vue";
import { useI18n } from "vue-i18n";
import { openSettings } from "../../lib/ipc";
import SetupHeader from "./SetupHeader.vue";
import { useSetupContext } from "./useSetup";
import { useTasksCheck } from "./useTasksCheck";

/** More projects than this are summed up ("N more"): the phone has the whole list. */
const MAX_PROJECT_CHIPS = 3;

const { t } = useI18n();
const { goto, tasksWanted, iosEnabled, hasEnabledChannel, wantTasks, declineTasks } =
  useSetupContext();
const {
  state: checkState,
  projects,
  agents,
  noProjects,
  noAgents,
  hasIssue,
  run: runCheck,
} = useTasksCheck();

/** The offer is for the person who has not turned it on; once on, the page shows the check. */
const turnedOn = computed(() => tasksWanted.value);

/** What the lead says the phone does: tap in the app, send a command in IM, or either. */
const leadKey = computed(() => {
  if (iosEnabled.value && hasEnabledChannel.value) return "leadBoth";
  return iosEnabled.value ? "leadIos" : "leadIm";
});

const checking = computed(() => checkState.value !== "done");
const unknown = computed(
  () => checkState.value === "done" && projects.value === null && agents.value === null,
);

const shownProjects = computed(() => (projects.value ?? []).slice(0, MAX_PROJECT_CHIPS));
const moreProjects = computed(() => Math.max(0, (projects.value ?? []).length - MAX_PROJECT_CHIPS));

const onTitle = computed(() =>
  hasIssue.value ? t("setup.tasks.onTitleIssue") : t("setup.tasks.onTitle"),
);
const onLead = computed(() => {
  if (checking.value) return t("setup.tasks.onLeadChecking");
  if (unknown.value) return t("setup.tasks.onLeadUnknown");
  return hasIssue.value ? t("setup.tasks.onLeadIssue") : t("setup.tasks.onLead");
});
/** The button that goes where the first missing thing is fixed. */
const fixLabel = computed(() =>
  noProjects.value ? t("setup.tasks.fixProjects") : t("setup.tasks.fixAgents"),
);

// Run the check as soon as the page shows it (on "Turn on", or when it is opened already on).
watch(
  turnedOn,
  (on) => {
    if (on) void runCheck();
  },
  { immediate: true },
);

// What the person fixes in a Settings window shows here when they come back to this one.
function onWindowFocus() {
  if (turnedOn.value && checkState.value === "done") void runCheck(true);
}
onMounted(() => window.addEventListener("focus", onWindowFocus));
onBeforeUnmount(() => window.removeEventListener("focus", onWindowFocus));
</script>

<template>
  <div class="settings-body setup-body">
    <div class="setup-page" data-testid="tasks-page">
      <template v-if="!turnedOn">
        <SetupHeader :title="t('setup.tasks.title')" :lead="t(`setup.tasks.${leadKey}`)" lead-testid="tasks-lead">
          <template #icon>
            <span class="setup-header-glyph" aria-hidden="true">
              <svg viewBox="0 0 24 24" focusable="false">
                <rect x="6.5" y="2.5" width="11" height="19" rx="2.8" />
                <path d="M10.6 18.4h2.8" />
              </svg>
            </span>
          </template>
        </SetupHeader>

        <section class="setup-panel" data-testid="tasks-offer">
          <ul class="setup-benefits">
            <li class="setup-benefit">
              <span class="setup-benefit-dot" aria-hidden="true">1</span>
              <div>
                <p class="setup-benefit-title">{{ t("setup.tasks.pick") }}</p>
                <p class="setup-benefit-note">{{ t("setup.tasks.pickNote") }}</p>
              </div>
            </li>
            <li class="setup-benefit">
              <span class="setup-benefit-dot" aria-hidden="true">2</span>
              <div>
                <p class="setup-benefit-title">{{ t("setup.tasks.terminal") }}</p>
                <p class="setup-benefit-note">{{ t("setup.tasks.terminalNote") }}</p>
              </div>
            </li>
            <li class="setup-benefit cost" data-testid="tasks-cost">
              <span class="setup-benefit-dot" aria-hidden="true">!</span>
              <div>
                <p class="setup-benefit-title">{{ t("setup.tasks.cost") }}</p>
                <p class="setup-benefit-note">{{ t("setup.tasks.costNote") }}</p>
              </div>
            </li>
            <!-- The IM bot note only applies when an IM bot is on; a phone-only setup has none. -->
            <li v-if="hasEnabledChannel" class="setup-benefit cost" data-testid="tasks-bot">
              <span class="setup-benefit-dot" aria-hidden="true">!</span>
              <div>
                <p class="setup-benefit-title">{{ t("setup.tasks.bot") }}</p>
                <p class="setup-benefit-note">{{ t("setup.tasks.botNote") }}</p>
              </div>
            </li>
          </ul>
        </section>
      </template>

      <template v-else>
        <SetupHeader :title="onTitle" :lead="onLead" lead-testid="tasks-lead">
          <template #icon>
            <span v-if="hasIssue" class="setup-header-glyph warn" aria-hidden="true">
              <svg viewBox="0 0 24 24" focusable="false">
                <path d="M12 7v6.5" />
                <circle cx="12" cy="17" r="0.7" />
              </svg>
            </span>
            <span v-else class="setup-success" aria-hidden="true">
              <svg viewBox="0 0 24 24" focusable="false"><path d="M5.5 12.8l4.2 4.2 8.8-9.4" /></svg>
            </span>
          </template>
        </SetupHeader>

        <section class="setup-panel" data-testid="tasks-check">
          <ul class="check-list">
            <li class="check-item" :class="{ warn: noProjects }">
              <span class="check-icon" aria-hidden="true"></span>
              <span class="check-label">{{ t("setup.tasks.projects") }}</span>
              <span class="check-value" data-testid="tasks-projects">
                <template v-if="checking">…</template>
                <span v-else-if="projects === null" class="check-none">{{ t("setup.tasks.unknown") }}</span>
                <span v-else-if="noProjects">{{ t("setup.tasks.projectsNone") }}</span>
                <template v-else>
                  <span v-for="project in shownProjects" :key="project.path" class="check-chip">
                    {{ project.label }}
                  </span>
                  <span v-if="moreProjects > 0" class="check-none">
                    {{ t("setup.tasks.projectsMore", { n: moreProjects }) }}
                  </span>
                </template>
              </span>
            </li>
            <li class="check-item" :class="{ warn: noAgents }">
              <span class="check-icon" aria-hidden="true"></span>
              <span class="check-label">{{ t("setup.tasks.agents") }}</span>
              <span class="check-value" data-testid="tasks-agents">
                <template v-if="checking">…</template>
                <span v-else-if="agents === null" class="check-none">{{ t("setup.tasks.unknown") }}</span>
                <span v-else-if="noAgents">{{ t("setup.tasks.agentsNone") }}</span>
                <template v-else>
                  <span v-for="agent in agents" :key="agent.kind" class="check-chip">
                    {{ agent.label }}
                  </span>
                </template>
              </span>
            </li>
            <li class="check-item">
              <span class="check-icon" aria-hidden="true"></span>
              <span class="check-label">{{ t("setup.tasks.background") }}</span>
              <span class="check-value" data-testid="tasks-background">
                {{ t("setup.tasks.backgroundValue") }}
              </span>
            </li>
          </ul>
          <p v-if="noProjects" class="check-hint" data-testid="tasks-projects-hint">
            {{ t("setup.tasks.projectsHint") }}
          </p>
          <p v-if="noAgents" class="check-hint" data-testid="tasks-agents-hint">
            {{ t("setup.tasks.agentsHint") }}
          </p>
        </section>

        <p v-if="checkState === 'done' && !hasIssue && !unknown" class="setup-fine">
          {{ t("setup.tasks.fine") }}
        </p>
        <p class="setup-fine">
          <a class="link" href="#" data-testid="tasks-undo" @click.prevent="declineTasks">
            {{ t("setup.tasks.undo") }}
          </a>
        </p>
      </template>
    </div>
  </div>

  <footer class="setup-footer">
    <span class="spacer"></span>
    <template v-if="!turnedOn">
      <button class="btn btn-lg" type="button" data-testid="tasks-later" @click="declineTasks">
        {{ t("setup.tasks.later") }}
      </button>
      <button
        class="btn btn-primary btn-lg"
        type="button"
        data-testid="tasks-enable"
        @click="wantTasks"
      >
        {{ t("setup.tasks.enable") }}
      </button>
    </template>
    <template v-else>
      <button
        v-if="hasIssue"
        class="btn btn-lg"
        type="button"
        data-testid="tasks-fix"
        @click="openSettings('advanced')"
      >
        {{ fixLabel }}
      </button>
      <button
        class="btn btn-primary btn-lg"
        type="button"
        data-testid="tasks-continue"
        @click="goto('done')"
      >
        {{ t("setup.common.continue") }}
      </button>
    </template>
  </footer>
</template>
