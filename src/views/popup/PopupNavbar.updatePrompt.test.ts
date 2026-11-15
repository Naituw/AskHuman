import { mount } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import zh from "../../i18n/zh";

vi.mock("../../lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/ipc")>()),
  popupSetSessionTitle: vi.fn(async () => {}),
}));
vi.mock("../../lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/platform")>()),
  isMac: false,
}));
import PopupNavbar from "./PopupNavbar.vue";
import { PopupCtxKey, type PopupContext } from "./context";

// The update prompt in the popup's navbar (docs/specs/self-update.md D9): besides updating, the user
// can stop being prompted for the announced version.

function navbar(overrides: Record<string, unknown> = {}, locale: "en" | "zh" = "en") {
  const dismissUpdateVersion = vi.fn();
  const applyUpdateFromPopup = vi.fn();
  const context = {
    request: ref(null),
    agentSessionTitle: ref(""),
    popupActive: ref(true),
    agentLabel: "",
    projectName: "",
    updateAvailable: ref(true),
    updatePending: ref(false),
    updateLatest: ref("1.2.0"),
    updateApplyMode: ref("automatic"),
    updatePopoverOpen: ref(true),
    updating: ref(false),
    updateStarted: ref(false),
    updateError: ref(""),
    updateNotesHtml: ref(""),
    toggleUpdatePopover: vi.fn(),
    applyUpdateFromPopup,
    dismissUpdateVersion,
    ...overrides,
  } as unknown as PopupContext;
  const wrapper = mount(PopupNavbar, {
    global: {
      plugins: [createI18n({ legacy: false, locale, messages: { en, zh } })],
      provide: { [PopupCtxKey as symbol]: context },
      stubs: { transition: false },
    },
  });
  return { wrapper, dismissUpdateVersion, applyUpdateFromPopup };
}

describe("the update prompt in the popup's navbar", () => {
  it("offers to skip the announced version next to the update button", async () => {
    const { wrapper, dismissUpdateVersion, applyUpdateFromPopup } = navbar();
    const skip = wrapper.get('[data-testid="update-dismiss"]');
    expect(skip.text()).toBe("Skip this version");
    expect(skip.attributes("title")).toContain("Stop prompting for this version");
    expect(skip.element.nextElementSibling?.classList.contains("btn-primary")).toBe(true);

    await skip.trigger("click");

    expect(dismissUpdateVersion).toHaveBeenCalledOnce();
    expect(applyUpdateFromPopup).not.toHaveBeenCalled();
  });

  it("is worded for Chinese users too", () => {
    const { wrapper } = navbar({}, "zh");
    expect(wrapper.get('[data-testid="update-dismiss"]').text()).toBe("忽略此版本");
  });

  it("cannot skip a version whose update is already being applied", () => {
    for (const busy of [{ updating: ref(true) }, { updateStarted: ref(true) }]) {
      const { wrapper } = navbar(busy);
      expect(wrapper.get('[data-testid="update-dismiss"]').attributes("disabled")).toBeDefined();
    }
  });

  it("shows nothing when no update is announced", () => {
    const { wrapper } = navbar({ updateAvailable: ref(false) });
    expect(wrapper.find('[data-testid="update-dismiss"]').exists()).toBe(false);
  });
});
