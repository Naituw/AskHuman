import { mount } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { nextTick, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";

const mocks = vi.hoisted(() => ({ setTitle: vi.fn() }));
vi.mock("../../lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/ipc")>()),
  popupSetSessionTitle: mocks.setTitle,
}));
vi.mock("../../lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/platform")>()),
  isMac: false,
}));
import PopupNavbar from "./PopupNavbar.vue";
import { PopupCtxKey, type PopupContext } from "./context";

// Where the Agent session title shows inside the popup (docs/specs/popup-session-info.md): a strip at the top of the
// navbar, which CSS only displays on macOS, plus the native window title that elsewhere takes its place.

function navbar(title: string, active = true) {
  const agentSessionTitle = ref(title);
  const popupActive = ref(active);
  const context = { agentSessionTitle, popupActive, agentLabel: "", projectName: "" } as unknown as PopupContext;
  const wrapper = mount(PopupNavbar, {
    global: {
      plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })],
      provide: { [PopupCtxKey as symbol]: context },
      // The real Transition renders no wrapper element, which is the DOM shape the page has in production.
      stubs: { transition: false },
    },
  });
  return { wrapper, agentSessionTitle, popupActive };
}

describe("the session title in the popup's navbar", () => {
  beforeEach(() => {
    mocks.setTitle.mockReset();
    mocks.setTitle.mockResolvedValue(undefined);
  });

  it("puts the trimmed title and the session icon in a strip that comes first in the navbar", () => {
    const { wrapper } = navbar("  Fix the popup title ");
    const strip = wrapper.get(".navbar > .session-strip");
    expect(strip.element).toBe(wrapper.get(".navbar").element.firstElementChild);
    expect(strip.get(".session-strip-title").text()).toBe("Fix the popup title");
    expect(strip.find("svg").exists()).toBe(true);
  });

  it("adds nothing without a title, so the navbar is exactly what it was", () => {
    for (const title of ["", "   "]) {
      const { wrapper } = navbar(title);
      expect(wrapper.find(".session-strip").exists()).toBe(false);
      expect(wrapper.get(".navbar").element.firstElementChild?.classList.contains("brand")).toBe(true);
    }
  });

  it("shows a title that arrives after the first paint without touching the rest of the bar", async () => {
    const { wrapper, agentSessionTitle } = navbar("");
    const brand = wrapper.get(".brand").element;
    agentSessionTitle.value = " Late title ";
    await nextTick();
    expect(wrapper.get(".session-strip-title").text()).toBe("Late title");
    expect(wrapper.get(".brand").element).toBe(brand);
  });

  it("is not a drag handle or a click target of its own", () => {
    const { wrapper } = navbar("Fix the popup title");
    const strip = wrapper.get(".session-strip");
    // The navbar owns dragging; the strip is display only and lets pointer events through in CSS.
    expect(strip.attributes("data-tauri-drag-region")).toBeUndefined();
    expect(strip.find("button, a, [role=button]").exists()).toBe(false);
  });

  it("also reports the title for the native window title where the page has no strip", async () => {
    const { agentSessionTitle, popupActive } = navbar("Fix the popup title");
    expect(mocks.setTitle).toHaveBeenLastCalledWith("Fix the popup title");
    agentSessionTitle.value = "Renamed";
    await nextTick();
    expect(mocks.setTitle).toHaveBeenLastCalledWith("Renamed");
    popupActive.value = false;
    agentSessionTitle.value = "Not shown";
    await nextTick();
    expect(mocks.setTitle).toHaveBeenLastCalledWith("Renamed");
  });
});
