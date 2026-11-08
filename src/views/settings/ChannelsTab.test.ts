// Settings → Channels: every card opens with the same header (badge, name, status tag, guide link,
// switch), and the switch still writes through the shared config and saves
// (docs/specs/settings-agents-channels-cards.md).
import { mount } from "@vue/test-utils";
import { ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import type { AppConfig } from "../../lib/types";
import type { ChannelState } from "../setup/channelDefs";
import { config as baseConfig, secrets } from "../setup/setup.testkit";
import ChannelsTab from "./ChannelsTab.vue";

const useSettingsContext = vi.hoisted(() => vi.fn());

vi.mock("./context", () => ({ useSettingsContext }));

const IM_CHANNELS = ["feishu", "telegram", "dingding", "slack"] as const;

function channelsConfig(): AppConfig {
  const cfg = baseConfig();
  Object.assign(cfg.channels, {
    popup: { enabled: true, rememberSize: true, width: 600, height: 700 },
    autoActivation: true,
  });
  return cfg;
}

function mountTab(
  options: {
    states?: Partial<Record<string, ChannelState>>;
    issues?: Record<string, string>;
    highlight?: string | null;
  } = {},
) {
  const config = ref<AppConfig | null>(channelsConfig());
  const api = {
    persist: vi.fn(async () => {}),
    openChannelGuide: vi.fn(),
    channelCardState: vi.fn((id: string): ChannelState => options.states?.[id] ?? "unconfigured"),
    channelIssueText: vi.fn((id: string) => options.issues?.[id] ?? null),
    secretsPresent: ref(secrets()),
    SECRET_PLACEHOLDER: "••••••••",
    clearSecret: vi.fn(),
    settingsTargetHighlight: ref(options.highlight ?? null),
    stepWidth: vi.fn(),
    stepHeight: vi.fn(),
  };
  // Everything else the tab reads (test / detect state of each channel) is only touched inside a
  // card's open form: a quiet ref is enough there.
  const context = new Proxy({ config, ...api } as Record<string, unknown>, {
    get: (target, key) => (key in target ? target[key as string] : ref(false)),
  });
  useSettingsContext.mockReturnValue(context);
  const wrapper = mount(ChannelsTab, { global: { plugins: [i18n] } });
  return { wrapper, config, api };
}

describe("ChannelsTab", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
    useSettingsContext.mockReset();
  });

  it("opens the popup card and each IM card with a badge and the channel's name", () => {
    const { wrapper } = mountTab();
    const titles: Record<string, string> = {
      popup: "Local popup",
      feishu: "Feishu",
      telegram: "Telegram",
      dingding: "DingTalk",
      slack: "Slack",
    };
    for (const [id, title] of Object.entries(titles)) {
      const head = wrapper
        .findAll(".channel-head")
        .find((candidate) => candidate.find(`.brand-badge[data-badge="${id}"]`).exists());
      expect(head, id).toBeDefined();
      expect(head!.get(".card-title").text(), id).toBe(title);
    }
  });

  it("tags each IM card with the state the settings context derives for it, and not the popup", () => {
    const { wrapper, api } = mountTab({
      states: { feishu: "connected", telegram: "configuredOff", slack: "issue" },
    });
    expect(wrapper.get('[data-testid="channel-state-feishu"]').text()).toBe("Connected");
    expect(wrapper.get('[data-testid="channel-state-telegram"]').text()).toBe("Off");
    expect(wrapper.get('[data-testid="channel-state-dingding"]').text()).toBe("Not set up");
    expect(wrapper.get('[data-testid="channel-state-slack"]').text()).toBe("Problem");
    expect(wrapper.find('[data-testid="channel-state-popup"]').exists()).toBe(false);
    for (const id of IM_CHANNELS) expect(api.channelCardState).toHaveBeenCalledWith(id);
  });

  it("keeps Feishu's recommended pill and gives only IM cards a guide link", () => {
    const { wrapper } = mountTab();
    expect(wrapper.findAll(".channel-recommended-badge")).toHaveLength(1);
    expect(wrapper.get("#channel-feishu .channel-recommended-badge").text()).toBe("Recommended");
    expect(wrapper.findAll(".guide-link")).toHaveLength(4);
    expect(wrapper.find(".card:not(.channel-card) .guide-link").exists()).toBe(false);
  });

  it("opens the guide of the channel whose link was clicked", async () => {
    const { wrapper, api } = mountTab();
    await wrapper.get("#channel-telegram .guide-link").trigger("click");
    expect(api.openChannelGuide).toHaveBeenCalledWith("telegram");
  });

  it("writes a switch through the shared config and saves, opening the card's form", async () => {
    const { wrapper, config, api } = mountTab();
    expect(wrapper.find("#channel-telegram .field").exists()).toBe(false);
    await wrapper.get('[data-testid="channel-toggle-telegram"]').setValue(true);
    expect(config.value!.channels.telegram.enabled).toBe(true);
    expect(api.persist).toHaveBeenCalledTimes(1);
    expect(wrapper.find("#channel-telegram .field").exists()).toBe(true);

    await wrapper.get('[data-testid="channel-toggle-telegram"]').setValue(false);
    expect(config.value!.channels.telegram.enabled).toBe(false);
    expect(api.persist).toHaveBeenCalledTimes(2);
    expect(wrapper.find("#channel-telegram .field").exists()).toBe(false);
  });

  it("switches the popup the same way, showing its size rows only while it is on", async () => {
    const { wrapper, config, api } = mountTab();
    expect(wrapper.findAll(".stepper")).toHaveLength(2);
    await wrapper.get('[data-testid="channel-toggle-popup"]').setValue(false);
    expect(config.value!.channels.popup.enabled).toBe(false);
    expect(api.persist).toHaveBeenCalledTimes(1);
    expect(wrapper.findAll(".stepper")).toHaveLength(0);
  });

  it("keeps each card's anchor and its jump highlight", () => {
    const { wrapper } = mountTab({ highlight: "channel-feishu" });
    for (const id of IM_CHANNELS) expect(wrapper.find(`#channel-${id}`).exists(), id).toBe(true);
    expect(wrapper.get("#channel-feishu").classes()).toContain("settings-target-highlight");
    expect(wrapper.get("#channel-slack").classes()).not.toContain("settings-target-highlight");
  });

  it("shows a channel's last error right under its header", () => {
    const { wrapper } = mountTab({ issues: { slack: "⚠ Channel issue (just now): bad token" } });
    const card = wrapper.get("#channel-slack");
    expect(card.get(".channel-head + .card-desc.warn").text()).toContain("bad token");
    expect(wrapper.find("#channel-feishu .card-desc.warn").exists()).toBe(false);
  });
});
