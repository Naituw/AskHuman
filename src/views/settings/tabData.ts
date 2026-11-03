import { computed, ref, type Ref, type WritableComputedRef } from "vue";
import type { Tab } from "./context";

export type SettingsLoad =
  | { type: "integration-status" }
  | { type: "pi-version" }
  | { type: "codex-version" }
  | { type: "general" }
  | { type: "about" }
  | { type: "agent-tasks"; force: boolean };

/**
 * Whether a settings window may show `tab`. A window that must not change the everyday Agent
 * integration (a Dev instance) has no Agents tab: its controls would write to configuration the
 * everyday install shares, and the backend refuses those writes anyway.
 */
export function tabAllowed(tab: Tab, everydayIntegrationsReadOnly: boolean): boolean {
  return !(everydayIntegrationsReadOnly && tab === "integration");
}

/**
 * The tab the window shows. Every way into a tab (tab button, search hit, deep link, "fix it"
 * shortcut) assigns to this one ref, so the guard lives in one place: a tab the window may not
 * show resolves to the first one, and becomes reachable again if the restriction lifts.
 */
export function guardedTab(
  initial: Tab,
  everydayIntegrationsReadOnly: Ref<boolean>,
): WritableComputedRef<Tab> {
  const requested = ref<Tab>(initial);
  return computed<Tab>({
    get: () =>
      tabAllowed(requested.value, everydayIntegrationsReadOnly.value)
        ? requested.value
        : "general",
    set: (tab) => {
      requested.value = tab;
    },
  });
}

/** Loads started when a settings tab becomes visible. */
export function tabLoads(tab: Tab, supportsAgentTasks: boolean): SettingsLoad[] {
  switch (tab) {
    case "general":
      return [{ type: "general" }, { type: "about" }];
    case "integration":
      return [
        { type: "integration-status" },
        { type: "pi-version" },
        { type: "codex-version" },
      ];
    case "advanced":
      return supportsAgentTasks ? [{ type: "agent-tasks", force: false }] : [];
    default:
      return [];
  }
}

/**
 * Loads started after `get_settings`: local integration status always starts in
 * the background (tab-bar update badge), plus the visible tab's own work.
 */
export function mountLoads(activeTab: Tab, supportsAgentTasks: boolean): SettingsLoad[] {
  const tab = tabLoads(activeTab, supportsAgentTasks);
  if (tab.some((load) => load.type === "integration-status")) return tab;
  return [{ type: "integration-status" }, ...tab];
}
