// The state a channel card's status tag shows in Settings → Channels, and the poll that feeds it
// (docs/specs/settings-agents-channels-cards.md).
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { defineComponent, h, ref, type Ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import type { AppConfig, ChannelIssue, SecretsPresent, SetupChannelStatus } from "../../lib/types";
import { channelRow, config, secrets } from "../setup/setup.testkit";
import type { SettingsCore, Tab } from "./context";
import { useChannels } from "./useChannels";

const ipc = vi.hoisted(() => ({
  channelHealth: vi.fn(),
  setupChannelStatus: vi.fn(),
}));

vi.mock("../../lib/ipc", () => ipc);

describe("channelCardState", () => {
  let wrapper: VueWrapper | undefined;
  let api!: ReturnType<typeof useChannels>;
  let cfg: Ref<AppConfig | null>;
  let present: Ref<SecretsPresent>;
  let tab: Ref<Tab>;

  async function mountIt() {
    const core: SettingsCore = {
      config: cfg,
      activeTab: tab,
      secretsPresent: present,
      persist: vi.fn(async () => {}),
    };
    wrapper = mount(
      defineComponent({
        setup() {
          api = useChannels(core);
          return () => h("div");
        },
      }),
      { global: { plugins: [i18n] } },
    );
    await flushPromises();
  }

  /** A Feishu card with every required field typed in. */
  function fillFeishu(enabled: boolean) {
    const feishu = cfg.value!.channels.feishu;
    feishu.enabled = enabled;
    feishu.appId = "cli_x";
    feishu.appSecret = "secret";
    feishu.openId = "ou_x";
  }

  function live(rows: SetupChannelStatus[]) {
    ipc.setupChannelStatus.mockResolvedValue(rows);
  }

  function issues(list: ChannelIssue[]) {
    ipc.channelHealth.mockResolvedValue(list);
  }

  beforeEach(() => {
    vi.resetAllMocks();
    i18n.global.locale.value = "en";
    cfg = ref<AppConfig | null>(config());
    present = ref(secrets());
    tab = ref<Tab>("channel");
    issues([]);
    live([]);
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
    vi.useRealTimers();
  });

  it("reads Not set up while the switch is off and nothing is filled in", async () => {
    await mountIt();
    for (const id of ["feishu", "telegram", "dingding", "slack"] as const) {
      expect(api.channelCardState(id), id).toBe("unconfigured");
    }
  });

  it("reads Off for a channel that is filled in but switched off", async () => {
    fillFeishu(false);
    await mountIt();
    expect(api.channelCardState("feishu")).toBe("configuredOff");
  });

  it("reads Incomplete for a switched-on channel with a required field missing", async () => {
    cfg.value!.channels.feishu.enabled = true;
    cfg.value!.channels.feishu.appId = "cli_x";
    await mountIt();
    expect(api.channelCardState("feishu")).toBe("incomplete");
  });

  it("counts a stored secret as a filled field", async () => {
    cfg.value!.channels.telegram.enabled = true;
    cfg.value!.channels.telegram.chatId = "42";
    present.value = secrets({ telegramToken: true });
    await mountIt();
    expect(api.channelCardState("telegram")).toBe("ready");
  });

  it("reads Enabled while no daemon answers, not Connected", async () => {
    fillFeishu(true);
    live([channelRow("feishu", { enabled: true, configured: true, connected: null })]);
    await mountIt();
    expect(api.channelCardState("feishu")).toBe("ready");
  });

  it("reads Connected once the daemon holds the connection", async () => {
    fillFeishu(true);
    live([channelRow("feishu", { enabled: true, configured: true, connected: true })]);
    await mountIt();
    expect(api.channelCardState("feishu")).toBe("connected");
    // The connection of one channel says nothing about another.
    expect(api.channelCardState("slack")).toBe("unconfigured");
  });

  it("drops back to Off the moment the switch goes off, without waiting for the next poll", async () => {
    fillFeishu(true);
    live([channelRow("feishu", { enabled: true, configured: true, connected: true })]);
    await mountIt();
    expect(api.channelCardState("feishu")).toBe("connected");
    cfg.value!.channels.feishu.enabled = false;
    expect(api.channelCardState("feishu")).toBe("configuredOff");
  });

  it("reads Problem when the daemon reports an unrecovered error", async () => {
    fillFeishu(true);
    live([channelRow("feishu", { enabled: true, configured: true, connected: true })]);
    issues([{ channel: "feishu", message: "invalid app secret", atMs: Date.now() }]);
    await mountIt();
    expect(api.channelCardState("feishu")).toBe("issue");
    expect(api.channelIssueText("feishu")).toContain("invalid app secret");
  });

  it("does not show an old error on a channel that is switched off", async () => {
    fillFeishu(false);
    issues([{ channel: "feishu", message: "invalid app secret", atMs: Date.now() }]);
    await mountIt();
    expect(api.channelCardState("feishu")).toBe("configuredOff");
  });

  it("keeps the other half of the picture when one read fails", async () => {
    fillFeishu(true);
    live([channelRow("feishu", { enabled: true, configured: true, connected: true })]);
    ipc.channelHealth.mockRejectedValue(new Error("daemon unreachable"));
    await mountIt();
    expect(api.channelCardState("feishu")).toBe("connected");

    wrapper?.unmount();
    ipc.channelHealth.mockResolvedValue([
      { channel: "feishu", message: "bad token", atMs: Date.now() },
    ]);
    ipc.setupChannelStatus.mockRejectedValue(new Error("no status"));
    await mountIt();
    expect(api.channelCardState("feishu")).toBe("issue");
  });

  it("is Not set up before the settings have loaded", async () => {
    cfg.value = null;
    await mountIt();
    expect(api.channelCardState("feishu")).toBe("unconfigured");
  });

  it("polls both reads while the Channels tab is open, and only then", async () => {
    vi.useFakeTimers();
    tab.value = "general";
    await mountIt();
    expect(ipc.channelHealth).not.toHaveBeenCalled();
    expect(ipc.setupChannelStatus).not.toHaveBeenCalled();

    tab.value = "channel";
    await flushPromises();
    expect(ipc.channelHealth).toHaveBeenCalledTimes(1);
    expect(ipc.setupChannelStatus).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(10_000);
    expect(ipc.channelHealth).toHaveBeenCalledTimes(2);
    expect(ipc.setupChannelStatus).toHaveBeenCalledTimes(2);

    tab.value = "integration";
    await flushPromises();
    await vi.advanceTimersByTimeAsync(30_000);
    expect(ipc.channelHealth).toHaveBeenCalledTimes(2);
    expect(ipc.setupChannelStatus).toHaveBeenCalledTimes(2);
  });

  it("picks up a connection that comes up between two polls", async () => {
    vi.useFakeTimers();
    fillFeishu(true);
    live([channelRow("feishu", { enabled: true, configured: true, connected: null })]);
    await mountIt();
    expect(api.channelCardState("feishu")).toBe("ready");

    live([channelRow("feishu", { enabled: true, configured: true, connected: true })]);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(api.channelCardState("feishu")).toBe("connected");
  });
});
