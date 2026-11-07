// The Settings banner for an unanswered Agent question, and the always-present wizard card
// (docs/specs/onboarding.md D2, D15, D17).
import { mount } from "@vue/test-utils";
import { ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import type { SetupStatus } from "../../lib/types";
import { status } from "../setup/setup.testkit";
import SetupBanner from "./SetupBanner.vue";
import SetupCard from "./SetupCard.vue";

const useSettingsContext = vi.hoisted(() => vi.fn());
const openSetup = vi.hoisted(() => vi.fn(async () => {}));

vi.mock("./context", () => ({ useSettingsContext }));
vi.mock("../../lib/ipc", () => ({ openSetup }));

describe("SetupBanner", () => {
  const dismissSetupBanner = vi.fn(async () => {});

  function banner(state: SetupStatus | null) {
    useSettingsContext.mockReturnValue({ setupState: ref(state), dismissSetupBanner });
    return mount(SetupBanner, { global: { plugins: [i18n] } });
  }

  beforeEach(() => {
    i18n.global.locale.value = "en";
    vi.clearAllMocks();
  });

  it("shows while the Agent question is open", () => {
    const w = banner(status({ agentsDecided: false }));
    expect(w.find('[data-testid="setup-banner"]').exists()).toBe(true);
    expect(w.text()).toContain("No Agent is connected yet");
  });

  it.each([
    ["the Agent question is answered", status({ agentsDecided: true })],
    ["the status is not known yet", null],
  ])("is hidden when %s", (_name, state) => {
    expect(banner(state).find('[data-testid="setup-banner"]').exists()).toBe(false);
  });

  it("is not driven by the other questions", () => {
    const w = banner(status({ agentsDecided: false, remoteDecided: true, backgroundDecided: true }));
    expect(w.find('[data-testid="setup-banner"]').exists()).toBe(true);
  });

  it("opens the wizard on the Agent page", async () => {
    const w = banner(status());
    await w.get('[data-testid="setup-banner-open"]').trigger("click");
    expect(openSetup).toHaveBeenCalledWith("agents");
  });

  it("counts closing it as skipping the Agent step", async () => {
    const w = banner(status());
    await w.get('[data-testid="setup-banner-dismiss"]').trigger("click");
    expect(dismissSetupBanner).toHaveBeenCalledTimes(1);
    expect(openSetup).not.toHaveBeenCalled();
  });
});

describe("SetupCard", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
    vi.clearAllMocks();
  });

  it("is always there, with no status behind it, and opens the wizard from its first page", async () => {
    const w = mount(SetupCard, { global: { plugins: [i18n] } });
    expect(w.text()).toContain("Setup wizard");
    await w.get('[data-testid="setup-card-open"]').trigger("click");
    expect(openSetup).toHaveBeenCalledTimes(1);
    expect(openSetup).toHaveBeenCalledWith();
  });
});
