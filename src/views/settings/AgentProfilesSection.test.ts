import { describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { i18n } from "../../i18n";
import AgentProfilesSection from "./AgentProfilesSection.vue";
import type { DiscoveredProfileInfo, ExtraProfileStatus } from "../../lib/types";

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => "/tmp/picked-dir"),
}));

describe("AgentProfilesSection", () => {
  it("renders discovered T3 profiles and emits explicit enable / ignore actions", async () => {
    i18n.global.locale.value = "en";
    const discovered: DiscoveredProfileInfo[] = [
      {
        agent: "antigravity",
        root: "/Users/test/.t3/userdata/providers/antigravity/ac0a3dfd",
        displayRoot: "~/.t3/userdata/providers/antigravity/ac0a3dfd",
        label: "T3 Code (antigravity)",
        source: "t3_discovered",
      },
    ];
    const wrapper = mount(AgentProfilesSection, {
      props: { agent: "antigravity", profiles: [], discovered, busy: false },
      global: { plugins: [i18n] },
    });

    const row = wrapper.get(`[data-testid="discovered-profile-${discovered[0].root}"]`);
    expect(row.text()).toContain("T3 Code (antigravity)");
    const pathEl = row.get(".agent-path");
    expect(pathEl.text()).toBe("~/.t3/userdata/providers/antigravity/ac0a3dfd");
    expect(pathEl.attributes("title")).toBe(discovered[0].root);

    const buttons = row.findAll("button");
    await buttons[0].trigger("click");
    expect(wrapper.emitted("add")?.[0]).toEqual([
      discovered[0].root,
      "T3 Code (antigravity)",
      "t3_discovered",
    ]);

    await buttons[1].trigger("click");
    expect(wrapper.emitted("dismiss")?.[0]).toEqual([discovered[0].root]);
  });

  it("shows conflict detach action, sync button, and manual add form", async () => {
    i18n.global.locale.value = "en";
    const profiles: ExtraProfileStatus[] = [
      {
        id: "antigravity-1234",
        agent: "antigravity",
        root: "/tmp/ag-alt",
        label: "Alt AG",
        source: "manual",
        enabled: true,
        state: "conflict",
        mode: "mcp",
        needsUpdate: false,
        error: "rules/AGENTS.md was changed outside AskHuman",
        hostPluginDisabled: false,
        mcpGrant: null,
      },
    ];
    const wrapper = mount(AgentProfilesSection, {
      props: { agent: "antigravity", profiles, discovered: [], busy: false },
      global: { plugins: [i18n] },
    });

    const item = wrapper.get('[data-testid="extra-profile-antigravity-1234"]');
    expect(item.text()).toContain("rules/AGENTS.md was changed outside AskHuman");
    expect(item.text()).toContain("Forget without deleting");

    const detachBtn = item.findAll("button").find((b) => b.text().includes("Forget without deleting"))!;
    await detachBtn.trigger("click");
    expect(wrapper.emitted("remove")?.[0]).toEqual(["antigravity-1234", true]);

    await wrapper.get('[data-testid="agent-profile-add-toggle-antigravity"]').trigger("click");
    const form = wrapper.get('[data-testid="profile-add-form-antigravity"]');
    const inputs = form.findAll("input");
    await inputs[0].setValue("/tmp/custom-ag");
    await inputs[1].setValue("Custom");
    const confirmBtn = form.findAll("button").find((b) => b.text() === "Add")!;
    await confirmBtn.trigger("click");
    expect(wrapper.emitted("add")?.[0]).toEqual(["/tmp/custom-ag", "Custom", "manual"]);
  });
});
