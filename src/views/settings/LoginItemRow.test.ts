// The menu-bar card's "launch at login is not enabled yet" row (docs/specs/onboarding.md D13).
import { mount } from "@vue/test-utils";
import { ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import type { SetupStatus } from "../../lib/types";
import { status } from "../setup/setup.testkit";
import LoginItemRow from "./LoginItemRow.vue";

const useSettingsContext = vi.hoisted(() => vi.fn());
vi.mock("./context", () => ({ useSettingsContext }));

describe("LoginItemRow", () => {
  const enableLoginItem = vi.fn(async () => {});

  function row(opts: {
    icon?: string;
    state?: SetupStatus | null;
    busy?: boolean;
    error?: string | null;
  }) {
    useSettingsContext.mockReturnValue({
      config: ref({ general: { menuBarIcon: opts.icon ?? "always" } }),
      setupState: ref(opts.state === undefined ? status() : opts.state),
      enableLoginItem,
      loginItemBusy: ref(opts.busy ?? false),
      loginItemError: ref(opts.error ?? null),
    });
    return mount(LoginItemRow, { global: { plugins: [i18n] } });
  }

  beforeEach(() => {
    i18n.global.locale.value = "en";
    vi.clearAllMocks();
  });

  it("shows when the mode promises a login item that this instance can write and has not", () => {
    const w = row({});
    expect(w.find('[data-testid="login-item-row"]').exists()).toBe(true);
    expect(w.text()).toContain("Launch at login is not enabled yet");
  });

  it.each([
    ["it is already installed", { state: status({ loginItemInstalled: true }) }],
    ["this is a Dev Instance, which never writes one", { state: status({ loginItemAvailable: false }) }],
    ["the menu bar is not set to always", { icon: "active" }],
    ["the menu bar is off", { icon: "off" }],
    ["the status is not known yet", { state: null }],
  ])("is hidden when %s", (_name, opts) => {
    expect(row(opts).find('[data-testid="login-item-row"]').exists()).toBe(false);
  });

  it("enables the login item on click", async () => {
    const w = row({});
    await w.get('[data-testid="login-item-enable"]').trigger("click");
    expect(enableLoginItem).toHaveBeenCalledTimes(1);
  });

  it("disables the button while it is working and shows a failure", () => {
    const w = row({ busy: true, error: "Operation not permitted" });
    expect((w.get('[data-testid="login-item-enable"]').element as HTMLButtonElement).disabled).toBe(true);
    expect(w.get('[data-testid="login-item-error"]').text()).toBe("Operation not permitted");
  });
});
