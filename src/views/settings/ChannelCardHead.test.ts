// The header row of a card in Settings → Channels: badge, name, status tag, guide link and switch
// (docs/specs/settings-agents-channels-cards.md).
import { mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it } from "vitest";
import { i18n } from "../../i18n";
import type { ChannelState } from "../setup/channelDefs";
import ChannelCardHead from "./ChannelCardHead.vue";

type Props = InstanceType<typeof ChannelCardHead>["$props"];

function mountHead(props: Partial<Props> = {}) {
  return mount(ChannelCardHead, {
    props: { id: "feishu", title: "Feishu", enabled: false, ...props },
    global: { plugins: [i18n] },
  });
}

describe("ChannelCardHead", () => {
  beforeEach(() => {
    i18n.global.locale.value = "en";
  });

  it("shows the channel's badge and name", () => {
    const head = mountHead({ id: "telegram", title: "Telegram" });
    expect(head.find('.brand-badge[data-badge="telegram"]').exists()).toBe(true);
    expect(head.get(".card-title").text()).toBe("Telegram");
  });

  it.each<[ChannelState, string, string]>([
    ["unconfigured", "Not set up", "muted"],
    ["configuredOff", "Off", "muted"],
    ["incomplete", "Incomplete", "warn"],
    ["ready", "Enabled", "ok"],
    ["connected", "Connected", "ok"],
    ["issue", "Problem", "warn"],
  ])("tags the %s state as %s in the %s tone", (state, label, tone) => {
    const head = mountHead({ state });
    const tag = head.get('[data-testid="channel-state-feishu"]');
    expect(tag.text()).toBe(label);
    expect(tag.classes()).toContain("status-tag");
    expect(tag.classes()).toContain(tone);
  });

  it("speaks the interface language in its tag", () => {
    i18n.global.locale.value = "zh";
    const head = mountHead({ state: "connected" });
    expect(head.get('[data-testid="channel-state-feishu"]').text()).toBe("已连接");
  });

  it("has no tag for the local popup, which has no connection to report", () => {
    const head = mountHead({ id: "popup", title: "Local popup" });
    expect(head.find(".status-tag").exists()).toBe(false);
    expect(head.find('.brand-badge[data-badge="popup"]').exists()).toBe(true);
  });

  it("marks only the recommended channel", () => {
    expect(mountHead().find(".channel-recommended-badge").exists()).toBe(false);
    expect(mountHead({ recommended: true }).get(".channel-recommended-badge").text()).toBe("Recommended");
  });

  it("offers the setup guide only when asked to, and says so when it is clicked", async () => {
    expect(mountHead().find(".guide-link").exists()).toBe(false);
    const head = mountHead({ guide: true });
    expect(head.get(".guide-link").text()).toContain("Setup guide");
    await head.get(".guide-link").trigger("click");
    expect(head.emitted("guide")).toHaveLength(1);
  });

  it("reflects the switch and reports a toggle with the new value", async () => {
    const off = mountHead({ enabled: false });
    const input = off.get<HTMLInputElement>('[data-testid="channel-toggle-feishu"]');
    expect(input.element.checked).toBe(false);
    await input.setValue(true);
    expect(off.emitted("toggle")).toEqual([[true]]);

    const on = mountHead({ enabled: true });
    const checked = on.get<HTMLInputElement>('[data-testid="channel-toggle-feishu"]');
    expect(checked.element.checked).toBe(true);
    await checked.setValue(false);
    expect(on.emitted("toggle")).toEqual([[false]]);
  });

  it("names the switch after the channel for assistive tech", () => {
    const head = mountHead({ title: "Slack", id: "slack" });
    expect(head.get('[data-testid="channel-toggle-slack"]').attributes("aria-label")).toBe("Slack");
  });
});
