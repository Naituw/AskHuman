import { mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it } from "vitest";
import { i18n } from "../../i18n";
import { settings } from "../setup/setup.testkit";
import CollaborationStyleEditor from "./CollaborationStyleEditor.vue";

const defaults = { aligned: "aligned body", autonomous: "autonomous body" };

describe("CollaborationStyleEditor", () => {
  beforeEach(() => { i18n.global.locale.value = "en"; });

  function editor() {
    const general = settings().config.general;
    general.collaborationStyle = "custom";
    general.collaborationStyleCustomText = "shared custom body";
    const wrapper = mount(CollaborationStyleEditor, {
      props: { agent: "codex", general, defaults },
      global: { plugins: [i18n] },
    });
    return { wrapper, general };
  }

  it("copies the effective default on first custom selection and keeps edits out of saved config", async () => {
    const { wrapper, general } = editor();
    expect(wrapper.get("select").element.value).toBe("inherit");
    await wrapper.get("select").setValue("custom");
    expect(wrapper.get("textarea").element.value).toBe("shared custom body");
    await wrapper.get("textarea").setValue("codex-specific body");
    expect(general.agentCollaborationStyles?.codex).toBeUndefined();
    await wrapper.get("button").trigger("click");
    expect(wrapper.emitted("save")).toEqual([["custom", "codex-specific body"]]);
  });

  it("restores the independent draft after changing styles and the default", async () => {
    const { wrapper, general } = editor();
    await wrapper.setProps({ general: { ...general, agentCollaborationStyles: { codex: { style: "autonomous", customText: "own draft" } } } });
    await wrapper.get("select").setValue("custom");
    expect(wrapper.get("textarea").element.value).toBe("own draft");
    await wrapper.get("select").setValue("inherit");
    await wrapper.setProps({ general: { ...wrapper.props("general"), collaborationStyleCustomText: "changed shared text" } });
    await wrapper.get("select").setValue("custom");
    expect(wrapper.get("textarea").element.value).toBe("own draft");
  });

  it("preserves an intentionally empty saved draft and reports its fallback", async () => {
    const { wrapper, general } = editor();
    await wrapper.setProps({ general: { ...general, agentCollaborationStyles: { codex: { style: null, customText: "" } } } });
    await wrapper.get("select").setValue("custom");
    expect(wrapper.get("textarea").element.value).toBe("");
    expect(wrapper.text()).toContain("An empty paragraph uses the built-in aligned style.");
  });

  it("saves inheritance while retaining the edited draft", async () => {
    const { wrapper } = editor();
    await wrapper.get("select").setValue("custom");
    await wrapper.get("textarea").setValue("retained draft");
    await wrapper.get("select").setValue("inherit");
    await wrapper.get("button").trigger("click");
    expect(wrapper.emitted("save")).toEqual([[null, "retained draft"]]);
  });

  it("reports saved application errors and retries the specific agent", async () => {
    const { wrapper } = editor();
    await wrapper.setProps({ results: [{ agent: "cursor", applied: false, error: "write failed" }] });
    expect(wrapper.text()).toContain("cursor preferences saved. Applying failed — write failed");
    const buttons = wrapper.findAll("button");
    await buttons[buttons.length - 1].trigger("click");
    expect(wrapper.emitted("retry")).toEqual([["cursor"]]);
  });

  it("applies default-style selection immediately and explicitly saves subsequent custom edits", async () => {
    const general = settings().config.general;
    general.collaborationStyleCustomText = "";
    const wrapper = mount(CollaborationStyleEditor, {
      props: { general, defaults }, global: { plugins: [i18n] },
    });
    await wrapper.get('button:nth-child(2)').trigger("click");
    expect(wrapper.emitted("save")?.[0]).toEqual(["autonomous", ""]);
    await wrapper.get('button:nth-child(3)').trigger("click");
    expect(wrapper.get("textarea").element.value).toBe(defaults.aligned);
    await wrapper.get("textarea").setValue("edited default body");
    expect(wrapper.emitted("save")).toHaveLength(2);
    const buttons = wrapper.findAll("button");
    await buttons[buttons.length - 1].trigger("click");
    expect(wrapper.emitted("save")?.[2]).toEqual(["custom", "edited default body"]);
  });
});
