import { mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";
import AttachmentDiffPreview from "./AttachmentDiffPreview.vue";
vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
describe("bounded diff DOM", () => {
  it("renders the viewport around the saved position while retaining prefixes and both line numbers", () => {
    const rows = Array.from({ length: 19000 }, (_, i) => ({ text: `+line ${i}`, kind: "add" as const, old: null, new: String(i + 1) }));
    const wrapper = mount(AttachmentDiffPreview, { props: { parsed: { sections: [{ title: "diff --git a/x b/x", rows }], notice: null }, viewport: null, top: 220000 } });
    expect(wrapper.findAll(".attachment-diff-line").length).toBeLessThan(70);
    expect(wrapper.text()).toContain("+line 10000"); expect(wrapper.text()).toContain("10001");
    expect(wrapper.findAll(".attachment-diff-number").length).toBeGreaterThan(0); wrapper.unmount();
  });
});
