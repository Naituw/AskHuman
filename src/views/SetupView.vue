<script setup lang="ts">
// Setup wizard window shell (docs/specs/onboarding.md): a clickable step bar over three pages (the
// remote step also has a screen of its own for the Agent-task offer, D25).
// The window is a global singleton; a second entry point focuses it and moves it to the page it
// asks for (`?page=` on a fresh window, `setup-goto-page` on an open one).
import { computed, onBeforeUnmount, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { applyLanguage } from "../i18n";
import { applyTheme } from "../lib/theme";
import type { SetupProbeResult, ThemeMode, UiLanguage } from "../lib/types";
import AgentsPage from "./setup/AgentsPage.vue";
import DonePage from "./setup/DonePage.vue";
import RemotePage from "./setup/RemotePage.vue";
import TasksPage from "./setup/TasksPage.vue";
import { SETUP_PAGES, createSetup, parsePage, stepOf, type SetupStep } from "./setup/useSetup";
import "./settings/settings.css";
import "./setup/setup.css";

const { t } = useI18n();

const setup = createSetup(parsePage(new URLSearchParams(window.location.search).get("page")));
const { config, page, status, goto, load, refreshStatus, scanAgents, probes } = setup;

/** The step the current page belongs to: the Agent-task offer sits inside the remote step. */
const activeStep = computed(() => stepOf(page.value));
const currentIndex = computed(() => SETUP_PAGES.indexOf(activeStep.value));

/** A step the user already settled (set up or skipped) gets a check instead of its number. */
function stepDone(name: SetupStep): boolean {
  if (name === "agents") return status.value?.agentsDecided === true;
  if (name === "remote") return status.value?.remoteDecided === true;
  return false;
}

const unlisteners: UnlistenFn[] = [];
// A check still running when the window goes away is stopped (its own timeout is the fallback).
// Closing the window never unmounts the page, so the page being hidden counts too.
const stopChecks = () => probes.releaseAll();
window.addEventListener("pagehide", stopChecks);
onBeforeUnmount(() => {
  unlisteners.forEach((off) => off());
  window.removeEventListener("pagehide", stopChecks);
  stopChecks();
});

onMounted(async () => {
  const payload = await load();
  applyTheme(payload.config.general.theme);
  applyLanguage(payload.config.general.language);

  // Another window changed the language or the theme.
  unlisteners.push(
    await listen<{ theme?: ThemeMode; language?: UiLanguage }>("settings-updated", (e) => {
      if (e.payload.language) applyLanguage(e.payload.language);
      if (typeof e.payload.theme === "string") {
        applyTheme(e.payload.theme);
        if (config.value) config.value.general.theme = e.payload.theme;
      }
    }),
  );
  // An entry point asked an open window for a page.
  unlisteners.push(
    await listen<string>("setup-goto-page", (e) => goto(parsePage(e.payload))),
  );
  // Something outside the wizard changed what is integrated or decided.
  unlisteners.push(
    await listen("setup-status-changed", () => {
      void refreshStatus();
      void scanAgents(true);
    }),
  );
  // One of the optional checks ended.
  unlisteners.push(
    await listen<SetupProbeResult>("setup-probe-result", (e) => probes.handleResult(e.payload)),
  );
});
</script>

<template>
  <div v-if="config" class="settings setup-win" data-testid="setup-window">
    <header class="setup-top" data-tauri-drag-region>
      <ol class="setup-steps" data-tauri-drag-region>
        <template v-for="(name, index) in SETUP_PAGES" :key="name">
          <li
            v-if="index > 0"
            class="setup-step-line"
            :class="{ filled: index <= currentIndex }"
            aria-hidden="true"
            data-tauri-drag-region
          ></li>
          <li class="setup-step-item">
            <button
              type="button"
              class="setup-step"
              :class="{ active: activeStep === name, done: stepDone(name) }"
              :aria-current="activeStep === name ? 'step' : undefined"
              :data-testid="`step-${name}`"
              @click="goto(name)"
            >
              <span class="setup-step-dot">
                <svg
                  v-if="stepDone(name) && activeStep !== name"
                  class="setup-step-check"
                  viewBox="0 0 12 12"
                  aria-hidden="true"
                >
                  <path d="M2.6 6.5l2.2 2.2 4.6-5" />
                </svg>
                <template v-else>{{ index + 1 }}</template>
              </span>
              <span class="setup-step-name">{{ t(`setup.steps.${name}`) }}</span>
            </button>
          </li>
        </template>
      </ol>
    </header>
    <AgentsPage v-if="page === 'agents'" />
    <RemotePage v-else-if="page === 'remote'" />
    <TasksPage v-else-if="page === 'tasks'" />
    <DonePage v-else />
  </div>
</template>
