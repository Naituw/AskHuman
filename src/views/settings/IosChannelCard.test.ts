import { flushPromises, mount } from "@vue/test-utils";
import { reactive, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import IosChannelCard from "./IosChannelCard.vue";

const iosOp = vi.hoisted(() => vi.fn());
const iosPairQr = vi.hoisted(() => vi.fn());
const iosTest = vi.hoisted(() => vi.fn());
const iosConnectionImport = vi.hoisted(() => vi.fn());
const iosConnectionExport = vi.hoisted(() => vi.fn());
const useSettingsContext = vi.hoisted(() => vi.fn());

vi.mock("../../lib/ipc", () => ({
  iosOp,
  iosPairQr,
  iosTest,
  iosConnectionImport,
  iosConnectionExport,
}));
vi.mock("./context", () => ({ useSettingsContext }));

const phone = {
  phoneId: "p1",
  name: "Tian's iPhone",
  state: "active",
  online: true,
  foreground: true,
  createdAt: 1,
  lastSeenAt: 2,
};

function gateway(extra: Record<string, unknown> = {}) {
  return {
    connected: true,
    via: "tls",
    macId: "mac",
    macName: "Mac",
    phones: [phone],
    pendingRequests: 0,
    ...extra,
  };
}

describe("IosChannelCard", () => {
  let config: ReturnType<typeof makeConfig>;
  const persist = vi.fn(async () => {});

  function makeConfig(enabled: boolean) {
    return ref({
      channels: {
        ios: reactive({
          enabled,
          host: "broker.example.com",
          port: 8883,
          wsPort: 8084,
          username: "abc123",
          password: "",
          relayUrl: "https://relay.example.com",
          macName: "",
        }),
      },
    });
  }

  beforeEach(() => {
    document.body.innerHTML = '<div class="settings"><div id="mount"></div></div>';
    i18n.global.locale.value = "en";
    iosOp.mockReset();
    iosPairQr.mockReset();
    persist.mockClear();
    iosPairQr.mockResolvedValue('<svg id="qr"></svg>');
    config = makeConfig(true);
    useSettingsContext.mockReturnValue({
      config,
      persist,
      secretsPresent: ref({ iosBrokerPassword: true }),
      SECRET_PLACEHOLDER: "••••",
      clearSecret: vi.fn(),
    });
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.useRealTimers();
  });

  function mountCard() {
    return mount(IosChannelCard, { attachTo: "#mount", global: { plugins: [i18n] } });
  }

  it("does not touch the daemon while the channel is off", async () => {
    config.value.channels.ios.enabled = false;
    const wrapper = mountCard();
    await flushPromises();
    expect(iosOp).not.toHaveBeenCalled();
    expect(wrapper.find(".ios-devices").exists()).toBe(false);
  });

  it("shows the broker status and the paired phones", async () => {
    iosOp.mockResolvedValue({ kind: "state", state: gateway() });
    const wrapper = mountCard();
    await flushPromises();
    expect(wrapper.text()).toContain("Broker connected (tls)");
    expect(wrapper.get(".ios-device-name").text()).toBe("Tian's iPhone");
    wrapper.unmount();
  });

  it("shows this month's traffic and warns before the free quota is used up", async () => {
    const mib = 1024 * 1024;
    iosOp.mockResolvedValue({
      kind: "state",
      state: gateway({ traffic: { month: "2026-10", bytesIn: 2 * mib, bytesOut: 3 * mib, msgsIn: 4, msgsOut: 5 } }),
    });
    const wrapper = mountCard();
    await flushPromises();
    expect(wrapper.get(".ios-traffic").text()).toContain("sent 3.0 MB, received 2.0 MB, about <1%");
    expect(wrapper.find(".ios-traffic-warning").exists()).toBe(false);
    wrapper.unmount();

    iosOp.mockResolvedValue({
      kind: "state",
      state: gateway({ traffic: { month: "2026-10", bytesIn: 220 * mib, bytesOut: 220 * mib, msgsIn: 4, msgsOut: 5 } }),
    });
    const warned = mountCard();
    await flushPromises();
    expect(warned.get(".ios-traffic-warning").classes()).toContain("warn");
    expect(warned.get(".ios-traffic-warning").text()).toContain("close to EMQX's free quota");
    warned.unmount();

    iosOp.mockResolvedValue({
      kind: "state",
      state: gateway({ traffic: { month: "2026-10", bytesIn: 300 * mib, bytesOut: 300 * mib, msgsIn: 4, msgsOut: 5 } }),
    });
    const full = mountCard();
    await flushPromises();
    expect(full.get(".ios-traffic-warning").classes()).toContain("full");
    full.unmount();
  });

  it("says nothing about traffic before any message was exchanged", async () => {
    iosOp.mockResolvedValue({
      kind: "state",
      state: gateway({ traffic: { month: "2026-10", bytesIn: 0, bytesOut: 0, msgsIn: 0, msgsOut: 0 } }),
    });
    const wrapper = mountCard();
    await flushPromises();
    expect(wrapper.find(".ios-traffic").exists()).toBe(false);
    wrapper.unmount();
  });

  it("says so when the gateway is not running", async () => {
    iosOp.mockResolvedValue({ kind: "state", state: null });
    const wrapper = mountCard();
    await flushPromises();
    expect(wrapper.text()).toContain("Not running yet");
    const pair = wrapper.findAll("button").find((b) => b.text() === "Pair a new phone");
    expect(pair?.attributes("disabled")).toBeDefined();
    wrapper.unmount();
  });

  it("walks through pairing: QR code, confirmation code, allow", async () => {
    vi.useFakeTimers();
    let live: Record<string, unknown> | undefined;
    iosOp.mockImplementation(async (op: { op: string }) => {
      if (op.op === "state") return { kind: "state", state: gateway({ pairing: live }) };
      if (op.op === "pairStart") {
        live = { ticketId: "t1", code: "ahpair1.x", expiresAtMs: Date.now() + 300_000, stage: "waiting" };
        return { kind: "pairing", ticketId: "t1", code: "ahpair1.x", expiresAtMs: Date.now() + 300_000 };
      }
      return { kind: "done", found: true };
    });
    const wrapper = mountCard();
    await vi.advanceTimersByTimeAsync(0);

    await wrapper.findAll("button").find((b) => b.text() === "Pair a new phone")!.trigger("click");
    await vi.advanceTimersByTimeAsync(0);
    expect(wrapper.find("#qr").exists()).toBe(true);
    expect(wrapper.text()).toContain("Scan the QR code");

    // The phone scans the code: the gateway moves to the confirmation stage.
    live = { ...live, stage: "pending", phoneName: "Work iPhone", sas: "123456" };
    await vi.advanceTimersByTimeAsync(1100);
    expect(wrapper.text()).toContain('"Work iPhone" wants to pair');
    expect(wrapper.get(".ios-sas").text()).toBe("123 456");

    await wrapper.findAll("button").find((b) => b.text() === "Allow")!.trigger("click");
    await vi.advanceTimersByTimeAsync(0);
    expect(iosOp).toHaveBeenCalledWith({ op: "pairDecision", ticketId: "t1", allow: true });

    // The session ends with a new phone in the list: success is reported.
    live = undefined;
    iosOp.mockImplementation(async (op: { op: string }) =>
      op.op === "state"
        ? {
            kind: "state",
            state: gateway({ phones: [phone, { ...phone, phoneId: "p2", name: "Work iPhone" }] }),
          }
        : { kind: "done", found: true },
    );
    await vi.advanceTimersByTimeAsync(3000);
    expect(wrapper.text()).toContain('Paired "Work iPhone".');
    expect(wrapper.find("#qr").exists()).toBe(false);
    wrapper.unmount();
  });

  it("asks before revoking a phone", async () => {
    iosOp.mockResolvedValue({ kind: "state", state: gateway() });
    const wrapper = mountCard();
    await flushPromises();
    await wrapper.findAll(".ios-device button").find((b) => b.text() === "Revoke")!.trigger("click");
    expect(document.body.textContent).toContain('Revoke "Tian\'s iPhone"?');
    expect(iosOp).not.toHaveBeenCalledWith(expect.objectContaining({ op: "revoke" }));

    const confirm = Array.from(document.querySelectorAll(".confirm-actions button")).find(
      (b) => b.textContent?.trim() === "Revoke pairing",
    ) as HTMLButtonElement;
    confirm.click();
    await flushPromises();
    expect(iosOp).toHaveBeenCalledWith({ op: "revoke", phoneId: "p1" });
    wrapper.unmount();
  });

  it("fills the form from a pasted connection string and saves it", async () => {
    iosOp.mockResolvedValue({ kind: "state", state: gateway() });
    iosConnectionImport.mockResolvedValue({
      host: "other.example.com",
      port: 8883,
      wsPort: 8084,
      username: "zzz999",
      password: "secret",
      relayUrl: "https://relay2.example.com",
    });
    const wrapper = mountCard();
    await flushPromises();
    await wrapper.get(".ios-import input").setValue("ahconn1.abc");
    await wrapper.findAll(".ios-import button").find((b) => b.text() === "Import")!.trigger("click");
    await flushPromises();
    expect(config.value.channels.ios.host).toBe("other.example.com");
    expect(config.value.channels.ios.password).toBe("secret");
    expect(persist).toHaveBeenCalled();
    expect(wrapper.text()).toContain("Connection settings filled in.");
    wrapper.unmount();
  });
});
