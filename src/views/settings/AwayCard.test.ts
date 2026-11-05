import { mount } from "@vue/test-utils";
import { reactive, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import AwayCard from "./AwayCard.vue";

const useSettingsContext = vi.hoisted(() => vi.fn());
const platform = vi.hoisted(() => ({ isMac: true }));

vi.mock("./context", () => ({ useSettingsContext }));
vi.mock("../../lib/platform", () => ({
  get isMac() {
    return platform.isMac;
  },
}));

describe("AwayCard", () => {
  const persist = vi.fn(async () => {});

  function setup(opts: { mac: boolean; experimental: boolean; pushAfterSecs?: number }) {
    platform.isMac = opts.mac;
    const config = ref({
      general: reactive({
        away: { capsule: true, welcomeBack: true, pushAfterSecs: opts.pushAfterSecs ?? 60 },
      }),
      experimental: reactive({ enabled: opts.experimental }),
    });
    useSettingsContext.mockReturnValue({ config, persist });
    return { config, wrapper: mount(AwayCard, { global: { plugins: [i18n] } }) };
  }

  beforeEach(() => {
    i18n.global.locale.value = "en";
    persist.mockClear();
  });

  it("on a Mac without the iOS channel shows the capsule and welcome-back switches only", () => {
    const { wrapper } = setup({ mac: true, experimental: false });
    expect(wrapper.find('[data-testid="away-card"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="away-capsule"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="away-welcome-back"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="away-push-after"]').exists()).toBe(false);
  });

  it("on a Mac with the iOS channel also offers the push wait", () => {
    const { wrapper } = setup({ mac: true, experimental: true });
    const options = wrapper.findAll('[data-testid="away-push-after"] option').map((o) => o.text());
    expect(options).toEqual([
      "Immediately",
      "After the Mac is idle for 1 min",
      "After the Mac is idle for 5 min",
    ]);
    expect((wrapper.get('[data-testid="away-push-after"]').element as HTMLSelectElement).value).toBe("60");
  });

  it("lists a hand-edited wait in its place, marked as custom", () => {
    const { wrapper } = setup({ mac: true, experimental: true, pushAfterSecs: 120 });
    const options = wrapper.findAll('[data-testid="away-push-after"] option').map((o) => o.text());
    expect(options).toHaveLength(4);
    expect(options[2]).toBe("After the Mac is idle for 2 min (custom)");
  });

  it("off a Mac it only exists while the iOS channel is visible, and only has the push wait", () => {
    expect(setup({ mac: false, experimental: false }).wrapper.find('[data-testid="away-card"]').exists()).toBe(false);
    const { wrapper } = setup({ mac: false, experimental: true });
    expect(wrapper.find('[data-testid="away-card"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="away-capsule"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="away-welcome-back"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="away-push-after"]').exists()).toBe(true);
  });

  it("saves when a switch or the wait changes", async () => {
    const { wrapper, config } = setup({ mac: true, experimental: true });
    await wrapper.get('[data-testid="away-capsule"]').setValue(false);
    expect(config.value.general.away.capsule).toBe(false);
    expect(persist).toHaveBeenCalledTimes(1);
    await wrapper.get('[data-testid="away-push-after"]').setValue("300");
    expect(config.value.general.away.pushAfterSecs).toBe(300);
    expect(persist).toHaveBeenCalledTimes(2);
  });
});
