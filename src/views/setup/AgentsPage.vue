<script setup lang="ts">
// Wizard page 1: connect the Agents and pick the collaboration style (docs/specs/onboarding.md D5-D9).
// A first run opens with a welcome header; a rerun gets the compact one that says what is kept.
// Agents found here are full tiles; the other supported ones follow as small tiles, and when
// nothing was found at all a notice card leads the page.
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import appIcon from "../../../src-tauri/icons/128x128@2x.png";
import type { CollaborationStyle } from "../../lib/types";
import AgentTile from "./AgentTile.vue";
import BrandBadge from "./BrandBadge.vue";
import EnvCards from "./EnvCards.vue";
import SetupHeader from "./SetupHeader.vue";
import { isBlocked } from "./agentRows";
import { useSetupContext } from "./useSetup";

const { t } = useI18n();
const {
  status,
  readOnly,
  rows,
  rowsLoaded,
  rowsError,
  picks,
  mainRows,
  otherRows,
  pendingRows,
  isRerun,
  style,
  styleBaseline,
  applying,
  applyError,
  appliedOnce,
  agentsPrimaryKind,
  applyAgents,
  skipAgents,
  scanAgents,
  rescanAgents,
  goto,
} = useSetupContext();

const filesOpen = ref(false);

/** Nothing is installed here and nothing is integrated: the page says so before anything else. */
const noneFound = computed(
  () => rowsLoaded.value && !rowsError.value && mainRows.value.length === 0,
);
const rescanning = ref(false);
/** A "check again" has finished at least once, so "still nothing" is worth saying. */
const rescanned = ref(false);

async function rescan() {
  if (rescanning.value) return;
  rescanning.value = true;
  try {
    await rescanAgents();
    rescanned.value = true;
  } finally {
    rescanning.value = false;
  }
}

/** "Custom" is only an option while it is what is set: it is edited in Settings, never here. */
const styleOptions = computed<CollaborationStyle[]>(() =>
  styleBaseline.value === "custom" ? ["aligned", "autonomous", "custom"] : ["aligned", "autonomous"],
);

function styleLabel(option: CollaborationStyle): string {
  if (option === "aligned") return t("settings.integration.collabAligned");
  if (option === "autonomous") return t("settings.integration.collabAutonomous");
  return t("setup.agents.styleCustom");
}

function styleDesc(option: CollaborationStyle): string {
  if (option === "aligned") return t("settings.integration.collabAlignedDesc");
  if (option === "autonomous") return t("settings.integration.collabAutonomousDesc");
  return t("settings.integration.collabCustomDesc");
}

const primaryLabel = computed(() => {
  switch (agentsPrimaryKind.value) {
    case "apply":
      return t("setup.agents.applyContinue");
    case "continue":
      return t("setup.common.continue");
    default:
      return t("setup.common.next");
  }
});

/** Agents ticked for this round; a row that cannot be changed never counts. */
const selectedCount = computed(
  () => rows.value.filter((row) => !isBlocked(row) && picks[row.id]?.checked === true).length,
);

/** The files applying would write, grouped by Agent; empty while nothing is pending. */
const plannedGroups = computed(() =>
  pendingRows.value
    .map((row) => ({ row, files: row.planned[picks[row.id].mode] }))
    .filter((group) => group.files.length > 0),
);
const plannedCount = computed(() =>
  plannedGroups.value.reduce((total, group) => total + group.files.length, 0),
);

/** Per-Agent hints that do not belong on a tile: they are long and only matter once ticked. */
const cursorRow = computed(() => {
  const row = rows.value.find((candidate) => candidate.id === "cursor");
  return row && !isBlocked(row) && picks.cursor?.checked === true ? row : undefined;
});
const codexRow = computed(() =>
  rows.value.find((row) => row.id === "codex" && row.delivery === "untrusted"),
);

async function onPrimary() {
  if (agentsPrimaryKind.value === "apply") await applyAgents();
  else goto("remote");
}
</script>

<template>
  <div class="settings-body setup-body">
    <div class="setup-page">
      <SetupHeader
        :variant="isRerun ? 'page' : 'hero'"
        :title="isRerun ? t('setup.agents.title') : t('setup.agents.welcomeTitle')"
        :lead="
          isRerun
            ? t('setup.agents.leadRerun')
            : noneFound
              ? t('setup.agents.leadNone')
              : t('setup.agents.lead')
        "
        lead-testid="agents-lead"
      >
        <template v-if="!isRerun" #icon>
          <img class="setup-app-icon" :src="appIcon" alt="" width="84" height="84" />
        </template>
      </SetupHeader>

      <EnvCards />

      <p v-if="readOnly" class="setup-note" data-testid="agents-readonly">
        {{ t("setup.agents.readOnly") }}
      </p>

      <div v-if="!rowsLoaded" class="settings-section-loading" data-testid="agents-loading">
        <span class="permission-state-spinner"></span>
        <p>{{ t("setup.agents.scanning") }}</p>
      </div>
      <div v-else-if="rowsError" class="card" data-testid="agents-scan-error">
        <p class="error-text">{{ rowsError }}</p>
        <button class="btn" type="button" @click="scanAgents()">
          {{ t("settings.integration.retryStatus") }}
        </button>
      </div>
      <template v-else>
        <div v-if="noneFound" class="setup-none" role="status" data-testid="agents-none">
          <span class="setup-none-icon" aria-hidden="true">
            <svg viewBox="0 0 20 20">
              <circle cx="10" cy="10" r="8" />
              <path d="M10 5.8v5" />
              <path d="M10 13.9v.1" />
            </svg>
          </span>
          <div class="setup-none-body">
            <p class="setup-none-title">{{ t("setup.agents.noneTitle") }}</p>
            <p class="setup-none-desc">{{ t("setup.agents.noneDesc") }}</p>
            <div class="setup-none-actions">
              <button
                class="btn"
                type="button"
                :disabled="rescanning"
                data-testid="agents-rescan"
                @click="rescan"
              >
                {{ rescanning ? t("setup.agents.rescanning") : t("setup.agents.rescan") }}
              </button>
              <span
                v-if="rescanned && !rescanning"
                class="setup-none-result"
                data-testid="agents-rescanned"
              >
                {{ t("setup.agents.rescanned") }}
              </span>
            </div>
          </div>
        </div>

        <section v-if="mainRows.length > 0" class="setup-section">
          <h2 class="setup-section-title">
            {{ t("setup.agents.detectedTitle") }}
            <span class="setup-count">{{ mainRows.length }}</span>
          </h2>
          <div class="agent-grid" data-testid="agents-main">
            <AgentTile v-for="row in mainRows" :key="row.id" :row="row" />
          </div>
        </section>

        <section v-if="otherRows.length > 0" class="setup-section" data-testid="agents-others">
          <h2 class="setup-section-title">
            {{ mainRows.length > 0 ? t("setup.agents.othersTitle") : t("setup.agents.supportedTitle") }}
            <span class="setup-count">{{ otherRows.length }}</span>
          </h2>
          <p v-if="mainRows.length > 0" class="setup-section-lead">
            {{ t("setup.agents.othersLead") }}
          </p>
          <div class="agent-grid compact">
            <AgentTile v-for="row in otherRows" :key="row.id" :row="row" compact />
          </div>
        </section>

        <ul v-if="cursorRow || codexRow" class="setup-notes" data-testid="agents-notes">
          <li v-if="cursorRow" data-testid="agent-note-cursor">
            <b>{{ cursorRow.title }}</b> {{ t("settings.integration.cursorRulesHint") }}
          </li>
          <li v-if="codexRow" data-testid="agent-trust-codex">
            <b>{{ codexRow.title }}</b> {{ t("setup.agents.codexTrustHint") }}
          </li>
        </ul>

        <section v-if="plannedCount > 0" class="setup-files-card" data-testid="agents-files">
          <button
            class="setup-disclosure"
            type="button"
            data-testid="agents-files-toggle"
            :aria-expanded="filesOpen"
            @click="filesOpen = !filesOpen"
          >
            <span class="disclosure-caret" :class="{ open: filesOpen }">▸</span>
            {{ t("setup.agents.files", { n: plannedCount }) }}
          </button>
          <div v-if="filesOpen" class="setup-files-groups">
            <div
              v-for="group in plannedGroups"
              :key="group.row.id"
              class="setup-files-group"
              :data-testid="`agent-files-${group.row.id}`"
            >
              <p class="setup-files-agent">
                <BrandBadge :id="group.row.id" :label="group.row.title" size="sm" />
                {{ group.row.title }}
              </p>
              <ul class="setup-files">
                <li v-for="file in group.files" :key="file.path">
                  <span class="setup-file-kind">{{
                    file.exists ? t("setup.agents.fileModify") : t("setup.agents.fileCreate")
                  }}</span>
                  <code class="agent-path">{{ file.path }}</code>
                </li>
              </ul>
            </div>
          </div>
        </section>

        <section class="setup-section" data-testid="agents-style">
          <h2 class="setup-section-title">{{ t("setup.agents.styleTitle") }}</h2>
          <p class="setup-section-lead">{{ t("setup.agents.styleLead") }}</p>
          <div class="style-grid">
            <label
              v-for="option in styleOptions"
              :key="option"
              class="style-card"
              :class="{ on: style === option }"
            >
              <input
                v-model="style"
                class="tile-input"
                type="radio"
                name="collaboration-style"
                :value="option"
                :disabled="readOnly || applying"
                :data-testid="`style-${option}`"
              />
              <span class="style-card-head">
                <b>{{ styleLabel(option) }}</b>
                <span class="style-radio" aria-hidden="true"></span>
              </span>
              <span class="style-card-desc">{{ styleDesc(option) }}</span>
            </label>
          </div>
        </section>
      </template>

      <p v-if="applyError" class="error-text" data-testid="agents-apply-error">{{ applyError }}</p>
      <p v-if="appliedOnce" class="setup-note" data-testid="agents-restart">
        {{ t("setup.agents.restartNote") }}
      </p>
    </div>
  </div>

  <footer class="setup-footer">
    <button
      v-if="status && !status.agentsDecided"
      class="btn setup-skip"
      type="button"
      data-testid="agents-skip"
      :disabled="applying"
      @click="skipAgents"
    >
      {{ t("setup.agents.skip") }}
    </button>
    <span class="spacer"></span>
    <span v-if="rowsLoaded && !rowsError" class="setup-footer-hint" data-testid="agents-selected">
      {{ t("setup.agents.selected", { n: selectedCount }) }}
    </span>
    <button
      class="btn btn-primary btn-lg"
      type="button"
      data-testid="agents-primary"
      :disabled="applying || !rowsLoaded"
      @click="onPrimary"
    >
      {{ applying ? t("setup.agents.applying") : primaryLabel }}
    </button>
  </footer>
</template>
