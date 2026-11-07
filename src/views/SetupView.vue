<script setup lang="ts">
// Setup wizard window shell (docs/specs/onboarding.md): a clickable step bar over three pages.
// The window is a global singleton; a second entry point focuses it and moves it to the page it
// asks for (`?page=` on a fresh window, `setup-goto-page` on an open one).
import { onBeforeUnmount, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { applyLanguage } from "../i18n";
import { applyTheme } from "../lib/theme";
import type { SetupProbeResult, ThemeMode, UiLanguage } from "../lib/types";
import AgentsPage from "./setup/AgentsPage.vue";
import DonePage from "./setup/DonePage.vue";
import RemotePage from "./setup/RemotePage.vue";
import { SETUP_PAGES, createSetup, parsePage } from "./setup/useSetup";
import "./settings/settings.css";
import "./setup/setup.css";

const { t } = useI18n();

const setup = createSetup(parsePage(new URLSearchParams(window.location.search).get("page")));
const { config, page, goto, load, refreshStatus, scanAgents, probes } = setup;

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
    <nav class="tabbar" data-tauri-drag-region>
      <button
        v-for="(name, index) in SETUP_PAGES"
        :key="name"
        type="button"
        :class="{ active: page === name }"
        :data-testid="`step-${name}`"
        @click="goto(name)"
      >
        <span class="setup-step-no">{{ index + 1 }}</span
        >{{ t(`setup.steps.${name}`) }}
      </button>
    </nav>
    <AgentsPage v-if="page === 'agents'" />
    <RemotePage v-else-if="page === 'remote'" />
    <DonePage v-else />
  </div>
</template>
