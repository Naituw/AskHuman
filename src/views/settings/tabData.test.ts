import { ref } from "vue";
import { describe, expect, it } from "vitest";
import { guardedTab, mountLoads, tabAllowed, tabLoads } from "./tabData";

describe("settings tab loads", () => {
  it("starts local integration status on any settings mount, without five-agent readiness", () => {
    expect(mountLoads("general", true)).toEqual([
      { type: "integration-status" },
      { type: "general" },
      { type: "about" },
    ]);
    expect(mountLoads("channel", true)).toEqual([{ type: "integration-status" }]);
    expect(mountLoads("general", true).some((load) => load.type === "agent-tasks")).toBe(
      false,
    );
    expect(mountLoads("general", true).some((load) => load.type === "pi-version")).toBe(
      false,
    );
    expect(
      mountLoads("general", true).some((load) => load.type === "codex-version"),
    ).toBe(false);
  });

  it("probes only the Pi and Codex versions when the integration tab is shown", () => {
    expect(tabLoads("integration", true)).toEqual([
      { type: "integration-status" },
      { type: "pi-version" },
      { type: "codex-version" },
    ]);
  });

  it("loads five-agent readiness only on the advanced tab", () => {
    expect(tabLoads("advanced", true)).toEqual([{ type: "agent-tasks", force: false }]);
    expect(tabLoads("advanced", false)).toEqual([]);
    expect(mountLoads("advanced", true)).toEqual([
      { type: "integration-status" },
      { type: "agent-tasks", force: false },
    ]);
  });
});

describe("everyday integration restriction (Dev instance)", () => {
  it("removes only the Agents tab", () => {
    for (const tab of ["general", "channel", "advanced", "experimental"] as const) {
      expect(tabAllowed(tab, true)).toBe(true);
      expect(tabAllowed(tab, false)).toBe(true);
    }
    expect(tabAllowed("integration", false)).toBe(true);
    expect(tabAllowed("integration", true)).toBe(false);
  });

  it("sends every route into the Agents tab to the first tab while restricted", () => {
    const readOnly = ref(false);
    // A deep link (?tab=integration) is read before `get_settings` answers.
    const tab = guardedTab("integration", readOnly);
    expect(tab.value).toBe("integration");

    readOnly.value = true;
    expect(tab.value).toBe("general");

    // A search hit or a readiness shortcut assigning the Agents tab must not get through either.
    tab.value = "integration";
    expect(tab.value).toBe("general");

    tab.value = "channel";
    expect(tab.value).toBe("channel");
    tab.value = "integration";
    expect(tab.value).toBe("general");
  });

  it("keeps the Agents tab reachable when nothing is restricted", () => {
    const tab = guardedTab("general", ref(false));
    tab.value = "integration";
    expect(tab.value).toBe("integration");
  });
});
