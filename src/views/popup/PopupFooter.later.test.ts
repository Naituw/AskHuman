import { mount } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import PopupFooter from "./PopupFooter.vue";
import { PopupCtxKey, type PopupContext } from "./context";

// The "Answer later" button of the shared popup page (docs/specs/popup-inbox-collapse.md C10, C21, C24): one button in
// the same place in all three footer layouts; its label follows the request's mark, and a standalone window has none.

type Layout = "single" | "multi" | "confirm";
function footer(layout: Layout, options: { available?: boolean; marked?: boolean; submitting?: boolean } = {}) {
  const toggleLater = vi.fn();
  const context = {
    isConfirm: layout === "confirm", isMulti: layout === "multi", verticalMode: false, imTipVisible: false,
    imTipConfigure: vi.fn(), imTipDismiss: vi.fn(), submitting: ref(options.submitting ?? false),
    requestCancel: vi.fn(), canGoPrev: false, goPrev: vi.fn(), goNext: vi.fn(), actionQuestionIndex: 0, total: 2,
    onLastQuestion: false, submitShowsCmdEnter: false, submitKeyLabel: "⌘↵", submitPrimary: false, nextPrimary: true,
    lastSeen: true, allViewed: true, canSubmit: true, submit: vi.fn(), confirmRequest: null, confirmCanSubmit: true,
    submitConfirm: vi.fn(), requestConfirmClose: vi.fn(),
    laterAvailable: options.available ?? true, laterMarked: options.marked ?? false, toggleLater,
  } as unknown as PopupContext;
  const wrapper = mount(PopupFooter, { global: {
    plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })],
    provide: { [PopupCtxKey as symbol]: context },
  } });
  return { wrapper, toggleLater };
}

describe("the footer's answer later button", () => {
  for (const layout of ["single", "multi", "confirm"] as const) {
    describe(`${layout} layout`, () => {
      it("sits right after Cancel and toggles the mark", async () => {
        const { wrapper, toggleLater } = footer(layout);
        const buttons = wrapper.findAll(".footer > .btn");
        const later = wrapper.get(".btn-later");
        expect(later.text()).toBe("Answer later");
        expect(buttons[0].text()).toContain("Cancel");
        expect(buttons[1].element).toBe(later.element);
        await later.trigger("click");
        expect(toggleLater).toHaveBeenCalledTimes(1);
      });

      it("offers to undo once the request is marked", () => {
        const { wrapper } = footer(layout, { marked: true });
        expect(wrapper.get(".btn-later").text()).toBe("Undo answer later");
        expect(wrapper.get(".btn-later").attributes("title")).toBe(en.popup.inbox.undoLaterHint);
      });

      it("is absent from a standalone window", () => {
        const { wrapper } = footer(layout, { available: false });
        expect(wrapper.find(".btn-later").exists()).toBe(false);
      });
    });
  }

  it("cannot be used while the answer is being sent", () => {
    const { wrapper } = footer("single", { submitting: true });
    expect(wrapper.get(".btn-later").attributes("disabled")).toBeDefined();
  });
});
