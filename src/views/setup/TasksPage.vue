<script setup lang="ts">
// Wizard: the one decision about starting Agent tasks from the phone or IM, on a screen of its own
// (docs/specs/onboarding.md D25). It checks first and asks second: the page reads what the phone's
// "New task" page would list — the Agents a task can start on, by the same rule the phone uses, and
// the saved projects — and only invites when at least one Agent can start. When none can, it says
// why for each Agent and offers no "Turn on" at all. It never needs a remote channel: the switch is
// global and easy to miss when a channel is set up later.
import { computed, onBeforeUnmount, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import type { AgentTaskReadiness } from "../../lib/types";
import { startBlockers } from "../settings/readiness";
import BrandBadge from "./BrandBadge.vue";
import SetupHeader from "./SetupHeader.vue";
import { useSetupContext } from "./useSetup";
import { useTasksCheck } from "./useTasksCheck";

/** More projects than this are summed up ("N more"): the phone has the whole list. */
const MAX_PROJECT_CHIPS = 3;

const { t } = useI18n();
const { goto, iosEnabled, hasEnabledChannel, wantTasks, declineTasks } = useSetupContext();
const { stage, busy, projects, startable, blocked, noProjects, run: runCheck } = useTasksCheck();

/** What the lead says the phone does: tap in the app, send a command in IM, either, or not yet. */
const leadKey = computed(() => {
  if (iosEnabled.value && hasEnabledChannel.value) return "leadBoth";
  if (iosEnabled.value) return "leadIos";
  return hasEnabledChannel.value ? "leadIm" : "leadNone";
});

const blockedStage = computed(() => stage.value === "blocked");
const title = computed(() =>
  blockedStage.value ? t("setup.tasks.blockedTitle") : t("setup.tasks.title"),
);
const lead = computed(() =>
  blockedStage.value ? t("setup.tasks.blockedLead") : t(`setup.tasks.${leadKey.value}`),
);

const shownProjects = computed(() => (projects.value ?? []).slice(0, MAX_PROJECT_CHIPS));
const moreProjects = computed(() => Math.max(0, (projects.value ?? []).length - MAX_PROJECT_CHIPS));

/** Where a task would run, as a short tag on the Agent's chip. */
function routeLabel(agent: AgentTaskReadiness): string {
  if (agent.startRoute === "desktop") {
    return t(agent.kind === "codex" ? "setup.tasks.route.codexApp" : "setup.tasks.route.cursorWindow");
  }
  return t("setup.tasks.route.terminal");
}

/** Why a task cannot start on this Agent, in one line. */
function why(agent: AgentTaskReadiness): string {
  return startBlockers(agent, t).join(t("desktop.reasonSeparator"));
}

// The check runs as soon as the page shows, before the question is on screen long enough to answer.
void runCheck();

// What the person fixes elsewhere (installs an Agent, opens Cursor) shows when they come back.
function onWindowFocus() {
  if (stage.value !== "checking") void runCheck(true);
}
onMounted(() => window.addEventListener("focus", onWindowFocus));
onBeforeUnmount(() => window.removeEventListener("focus", onWindowFocus));
</script>

<template>
  <div class="settings-body setup-body">
    <div class="setup-page" data-testid="tasks-page" :data-stage="stage">
      <SetupHeader :title="title" :lead="lead" lead-testid="tasks-lead">
        <template #icon>
          <span v-if="blockedStage" class="setup-header-glyph warn" aria-hidden="true">
            <svg viewBox="0 0 24 24" focusable="false">
              <path d="M12 7v6.5" />
              <circle cx="12" cy="17" r="0.7" />
            </svg>
          </span>
          <span v-else class="setup-header-glyph" aria-hidden="true">
            <svg viewBox="0 0 24 24" focusable="false">
              <rect x="6.5" y="2.5" width="11" height="19" rx="2.8" />
              <path d="M10.6 18.4h2.8" />
            </svg>
          </span>
        </template>
      </SetupHeader>

      <!-- Reading what this computer can start. -->
      <section v-if="stage === 'checking'" class="setup-panel" data-testid="tasks-checking">
        <ul class="check-list">
          <li class="check-item none">
            <span class="check-icon" aria-hidden="true"></span>
            <span class="check-label">{{ t("setup.tasks.checking") }}</span>
            <span class="check-value">…</span>
          </li>
        </ul>
      </section>

      <!-- No Agent can start: say why for each, and offer nothing to turn on. -->
      <section v-else-if="stage === 'blocked'" class="setup-panel" data-testid="tasks-blocked">
        <ul class="check-list">
          <li
            v-for="agent in blocked"
            :key="agent.kind"
            class="check-item warn"
            :data-testid="`tasks-blocked-${agent.kind}`"
          >
            <span class="check-icon" aria-hidden="true"></span>
            <span class="check-label">{{ agent.label }}</span>
            <span class="check-value">{{ why(agent) }}</span>
          </li>
          <li v-if="blocked.length === 0" class="check-item warn" data-testid="tasks-blocked-none">
            <span class="check-icon" aria-hidden="true"></span>
            <span class="check-label">{{ t("setup.tasks.agents") }}</span>
            <span class="check-value">{{ t("setup.tasks.agentsNone") }}</span>
          </li>
        </ul>
        <p class="check-hint" data-testid="tasks-blocked-hint">{{ t("setup.tasks.blockedHint") }}</p>
      </section>

      <!-- At least one Agent can start (or the check could not run): what the phone will have. -->
      <template v-else>
        <section class="setup-panel" data-testid="tasks-now">
          <ul class="check-list">
            <li v-if="stage === 'ready'" class="check-item">
              <span class="check-icon" aria-hidden="true"></span>
              <span class="check-label">{{ t("setup.tasks.now") }}</span>
              <span class="check-value" data-testid="tasks-startable">
                <span
                  v-for="agent in startable"
                  :key="agent.kind"
                  class="check-chip"
                  :data-testid="`tasks-startable-${agent.kind}`"
                >
                  <BrandBadge :id="agent.kind" :label="agent.label" size="sm" />
                  {{ agent.label }}
                  <em>{{ routeLabel(agent) }}</em>
                </span>
              </span>
            </li>
            <li v-else class="check-item none" data-testid="tasks-unknown">
              <span class="check-icon" aria-hidden="true"></span>
              <span class="check-label">{{ t("setup.tasks.agents") }}</span>
              <span class="check-value check-none">{{ t("setup.tasks.unknownAgents") }}</span>
            </li>
            <!-- Agents that are set up but cannot start yet: said once, quietly, not as a blocker. -->
            <li
              v-if="stage === 'ready' && blocked.length > 0"
              class="check-item none"
              data-testid="tasks-not-yet"
            >
              <span class="check-icon" aria-hidden="true"></span>
              <span class="check-label">{{ t("setup.tasks.notNow") }}</span>
              <span class="check-value check-none">
                <span v-for="agent in blocked" :key="agent.kind" class="check-line">
                  {{ agent.label }} — {{ why(agent) }}
                </span>
              </span>
            </li>
            <li class="check-item" :class="{ none: noProjects || projects === null }">
              <span class="check-icon" aria-hidden="true"></span>
              <span class="check-label">{{ t("setup.tasks.projects") }}</span>
              <span class="check-value" data-testid="tasks-projects">
                <span v-if="projects === null" class="check-none">{{ t("setup.tasks.unknown") }}</span>
                <span v-else-if="noProjects" class="check-none">{{ t("setup.tasks.projectsNone") }}</span>
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
            <li class="check-item">
              <span class="check-icon" aria-hidden="true"></span>
              <span class="check-label">{{ t("setup.tasks.background") }}</span>
              <span class="check-value" data-testid="tasks-background">
                {{ t("setup.tasks.backgroundValue") }}
              </span>
            </li>
          </ul>
        </section>

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
    </div>
  </div>

  <footer class="setup-footer">
    <span class="spacer"></span>
    <template v-if="stage === 'blocked'">
      <button class="btn btn-lg" type="button" data-testid="tasks-skip" @click="goto('done')">
        {{ t("setup.tasks.skip") }}
      </button>
      <button
        class="btn btn-primary btn-lg"
        type="button"
        data-testid="tasks-recheck"
        :disabled="busy"
        @click="runCheck(true)"
      >
        {{ busy ? t("setup.tasks.rechecking") : t("setup.tasks.recheck") }}
      </button>
    </template>
    <template v-else>
      <button class="btn btn-lg" type="button" data-testid="tasks-later" @click="declineTasks">
        {{ t("setup.tasks.later") }}
      </button>
      <button
        class="btn btn-primary btn-lg"
        type="button"
        data-testid="tasks-enable"
        :disabled="stage === 'checking'"
        @click="wantTasks"
      >
        {{ t("setup.tasks.enable") }}
      </button>
    </template>
  </footer>
</template>
