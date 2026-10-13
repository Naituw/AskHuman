import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { i18n } from "../i18n";
import ComposerAttachments from "./ComposerAttachments.vue";

describe("ComposerAttachments", () => {
  it("offers an accessible preview button only when enabled and keeps removal separate", async () => {
    const wrapper = mount(ComposerAttachments, {
      props: { images: [{ data: "data:image/png;base64,cG5n", filename: "capture.png" }], files: [] },
      global: { plugins: [i18n] },
    });
    expect(wrapper.find(".thumb-preview").exists()).toBe(false);
    await wrapper.setProps({ previewable: true, selectedImage: 0, questionIndex: 2 });
    const button = wrapper.get(".thumb-preview");
    expect(button.attributes("aria-label")).toContain("capture.png");
    expect(button.attributes("aria-pressed")).toBe("true");
    expect(button.attributes("data-question-index")).toBe("2");
    expect(wrapper.get(".thumb").classes()).toContain("selected");
    await button.trigger("click");
    expect(wrapper.emitted("previewImage")?.[0][0]).toBe(0);
    await wrapper.get(".remove").trigger("click");
    expect(wrapper.emitted("removeImage")).toEqual([[0]]);
    expect(wrapper.emitted("previewImage")).toHaveLength(1);
    wrapper.unmount();
  });
  it("shares thumbnail and file-chip removal behavior across composers", async () => {
    const wrapper = mount(ComposerAttachments, {
      props: {
        images: [
          {
            key: "image-1",
            data: "data:image/png;base64,cG5n",
            filename: "capture.png",
          },
        ],
        files: [{ key: "file-1", path: "/tmp/notes.txt", name: "notes.txt" }],
      },
      global: { plugins: [i18n] },
    });

    expect(wrapper.get(".thumb img").attributes("src")).toContain("data:image/png");
    expect(wrapper.get(".reply-file").text()).toContain("notes.txt");

    await wrapper.get(".thumb .remove").trigger("click");
    await wrapper.get(".reply-file .rf-remove").trigger("click");
    expect(wrapper.emitted("removeImage")).toEqual([[0]]);
    expect(wrapper.emitted("removeFile")).toEqual([[0]]);
  });

  it("marks a missing source-file reference as unavailable", () => {
    const wrapper = mount(ComposerAttachments, {
      props: {
        images: [],
        files: [
          {
            key: "missing",
            path: "/tmp/missing.pdf",
            name: "missing.pdf",
            available: false,
          },
        ],
      },
      global: { plugins: [i18n] },
    });

    const chip = wrapper.get(".reply-file");
    expect(chip.classes()).toContain("unavailable");
    expect(chip.attributes("title")).toContain("/tmp/missing.pdf");
    expect(chip.get(".rf-warning svg").element.tagName.toLowerCase()).toBe("svg");
    expect(chip.text()).not.toContain("⚠️");
  });
});
