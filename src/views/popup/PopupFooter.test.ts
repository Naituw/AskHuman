import { mount } from "@vue/test-utils";
import { ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import { PopupCtxKey, type PopupContext } from "./context";
import PopupFooter from "./PopupFooter.vue";

describe("confirmed submission feedback", () => {
  it.each(["single", "multi", "confirm"])("keeps %s feedback on the disabled submit button", async mode => {
    i18n.global.locale.value = "zh";
    const completion = ref<"sent" | "submitted" | null>(null);
    const context = {
      isConfirm: mode === "confirm", isMulti: mode === "multi", verticalMode: false,
      imTipVisible: false, submitting: ref(true), completionFeedback: completion,
      canGoPrev: false, actionQuestionIndex: 0, total: 1, onLastQuestion: true,
      submitShowsCmdEnter: true, submitKeyLabel: "⌘↵", submitPrimary: true,
      nextPrimary: false, lastSeen: true, allViewed: true, canSubmit: true,
      confirmRequest: { presentation: { submitLabel: "允许执行" } }, confirmCanSubmit: false,
      submit: vi.fn(), submitConfirm: vi.fn(),
    } as unknown as PopupContext;
    const wrapper = mount(PopupFooter, { global: { plugins: [i18n], provide: { [PopupCtxKey as symbol]: context } } });
    const buttons = wrapper.findAll(".btn");
    const button = buttons[buttons.length - 1];
    expect(button.text()).toContain(mode === "confirm" ? "提交中" : "发送中");
    completion.value = mode === "confirm" ? "submitted" : "sent";
    await wrapper.vm.$nextTick();
    expect(button.text()).toBe(mode === "confirm" ? "已提交" : "已发送");
    expect(button.classes()).toContain("completion-success");
    expect(button.find("svg").exists()).toBe(true);
    expect(button.find("kbd").exists()).toBe(false);
    expect(button.attributes("disabled")).toBeDefined();
    wrapper.unmount(); i18n.global.locale.value = "en";
  });
});
