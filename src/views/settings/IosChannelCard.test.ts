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
const iosDefaultRelayUrl = vi.hoisted(() => vi.fn());
const openPath = vi.hoisted(() => vi.fn());
const useSettingsContext = vi.hoisted(() => vi.fn());

vi.mock("../../lib/ipc", () => ({
  iosOp,
  iosPairQr,
  iosTest,
  iosConnectionImport,
  iosConnectionExport,
  iosDefaultRelayUrl,
  openPath,
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

/** What `ios_test` resolves with when the broker and the relay are fine. */
function testOk(warnings: string[] = []) {
  return {
    message: "Connected via tls (160 ms). The relay answered in 850 ms and can send pushes.",
    warnings,
  };
}

/** What `ios_test` rejects with (an `IosTestError`). */
function testFailed(kind: string, message: string, warnings: string[] = []) {
  return { kind, message, warnings };
}

describe("IosChannelCard", () => {
  let config: ReturnType<typeof makeConfig>;
  let supportsAgentTasks: boolean;
  let activeTab: ReturnType<typeof ref<string>>;
  let taskReadiness: ReturnType<typeof ref<unknown[]>>;
  const persist = vi.fn(async () => {});
  const toggleAgentTasks = vi.fn();

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
          alertAfterSecs: 900,
        }),
      },
      agentTasks: reactive({ enabled: false }),
    });
  }

  function provideContext() {
    useSettingsContext.mockReturnValue({
      config,
      persist,
      secretsPresent: ref({ iosBrokerPassword: true }),
      SECRET_PLACEHOLDER: "••••",
      clearSecret: vi.fn(),
      supportsAgentTasks,
      toggleAgentTasks,
      taskReadiness,
      activeTab,
    });
  }

  beforeEach(() => {
    document.body.innerHTML = '<div class="settings"><div id="mount"></div></div>';
    i18n.global.locale.value = "en";
    iosOp.mockReset();
    iosPairQr.mockReset();
    iosTest.mockReset();
    iosDefaultRelayUrl.mockReset();
    // A build without a public relay unless a test says otherwise.
    iosDefaultRelayUrl.mockResolvedValue("");
    openPath.mockReset();
    openPath.mockResolvedValue(undefined);
    persist.mockClear();
    iosPairQr.mockResolvedValue('<svg id="qr"></svg>');
    config = makeConfig(true);
    activeTab = ref("channels");
    taskReadiness = ref([]);
    toggleAgentTasks.mockClear();
    supportsAgentTasks = true;
    provideContext();
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

  it("shows this month's traffic and says plainly when it is close to a GB or past it", async () => {
    const mib = 1024 * 1024;
    async function noteFor(each: number) {
      iosOp.mockResolvedValue({
        kind: "state",
        state: gateway({ traffic: { month: "2026-10", bytesIn: each * mib, bytesOut: each * mib, msgsIn: 4, msgsOut: 5 } }),
      });
      const wrapper = mountCard();
      await flushPromises();
      return wrapper;
    }

    iosOp.mockResolvedValue({
      kind: "state",
      state: gateway({ traffic: { month: "2026-10", bytesIn: 2 * mib, bytesOut: 3 * mib, msgsIn: 4, msgsOut: 5 } }),
    });
    const quiet = mountCard();
    await flushPromises();
    expect(quiet.get(".ios-traffic").text()).toContain("5.0 MB");
    expect(quiet.find(".ios-traffic-note").exists()).toBe(false);
    quiet.unmount();

    const near = await noteFor(440);
    expect(near.get(".ios-traffic-note").text()).toBe("This month's data use is close to 1 GB.");
    near.unmount();

    const past = await noteFor(600);
    expect(past.get(".ios-traffic-note").text()).toBe("This month's data use is over 1 GB.");
    past.unmount();

    const next = await noteFor(975);
    expect(next.get(".ios-traffic-note").text()).toBe("This month's data use is close to 2 GB.");
    // It only counts: nothing about a free quota or what the service does when it runs out, in either colour.
    expect(next.text()).not.toContain("pauses");
    expect(next.find(".ios-traffic-note").classes()).not.toContain("warn");
    next.unmount();
  });

  it("shows the known Mac total, source details and storage failures", async () => {
    const mib = 1024 ** 2;
    const own = { month: "2026-11", bytesIn: 10 * mib, bytesOut: 20 * mib, msgsIn: 1, msgsOut: 1 };
    iosOp.mockResolvedValue({
      kind: "state",
      state: gateway({
        traffic: own,
        usageRecords: [
          { ...own, broker: "broker.example.com", macId: "a", macName: "Home Mac", updatedAt: 1000 },
          { ...own, broker: "broker.example.com", macId: "b", macName: "Office Mac", bytesOut: 10 * mib, updatedAt: 2000 },
        ],
        trafficStorageIssue: true,
      }),
    });
    const wrapper = mountCard();
    await flushPromises();
    expect(wrapper.get(".ios-traffic").text()).toContain("2 devices");
    expect(wrapper.get(".ios-traffic").text()).toContain("50.0 MB");
    expect(wrapper.get(".ios-traffic").text()).toContain("History is incomplete");
    expect(wrapper.get(".ios-traffic-details").text()).toContain("Office Mac");
    expect(wrapper.text()).toContain("Traffic could not be saved");
    // The traffic area does not name a service or link to its console, and it does not measure the use
    // against a plan: the usage is the broker's to show. (The connection form below does name the
    // services it can help with; that is a different part of the card.)
    expect(wrapper.get(".ios-traffic-details").text()).toContain("broker's console");
    expect(wrapper.find(".ios-traffic-details a").exists()).toBe(false);
    const trafficArea = [".ios-status", ".ios-traffic", ".ios-traffic-details"].map((s) => wrapper.get(s).text()).join(" ");
    expect(trafficArea).not.toMatch(/EMQX|%|allowance|quota/i);
    wrapper.unmount();
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

  describe("push timing", () => {
    const optionTexts = (wrapper: ReturnType<typeof mountCard>) =>
      wrapper.findAll(".ios-alert-after option").map((o) => o.text());

    it("offers the waits, with the configured one selected", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
      const wrapper = mountCard();
      await flushPromises();
      expect(optionTexts(wrapper)).toEqual(["Immediately", "5 min", "15 min", "30 min", "1 h"]);
      expect((wrapper.get(".ios-alert-after select").element as HTMLSelectElement).value).toBe("900");
      wrapper.unmount();
    });

    it("saves the wait that was chosen", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
      const wrapper = mountCard();
      await flushPromises();
      await wrapper.get(".ios-alert-after select").setValue("1800");
      expect(config.value.channels.ios.alertAfterSecs).toBe(1800);
      expect(persist).toHaveBeenCalled();

      await wrapper.get(".ios-alert-after select").setValue("0");
      expect(config.value.channels.ios.alertAfterSecs).toBe(0);
      wrapper.unmount();
    });

    it("keeps a hand-edited wait in the list instead of silently replacing it", async () => {
      config.value.channels.ios.alertAfterSecs = 600;
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
      const wrapper = mountCard();
      await flushPromises();
      expect(optionTexts(wrapper)).toEqual([
        "Immediately",
        "5 min",
        "10 min (custom)",
        "15 min",
        "30 min",
        "1 h",
      ]);
      expect((wrapper.get(".ios-alert-after select").element as HTMLSelectElement).value).toBe("600");
      expect(persist).not.toHaveBeenCalled();
      wrapper.unmount();
    });

    it("shows how long the Mac has been idle, when it can tell", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway({ macIdleSecs: 125 }) });
      const wrapper = mountCard();
      await flushPromises();
      expect(wrapper.get(".ios-idle").text()).toBe("The Mac has been idle for 2 min");
      wrapper.unmount();

      iosOp.mockResolvedValue({ kind: "state", state: gateway({ macIdleSecs: 7 }) });
      const fresh = mountCard();
      await flushPromises();
      expect(fresh.get(".ios-idle").text()).toBe("The Mac has been idle for 7 s");
      fresh.unmount();

      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
      const unknown = mountCard();
      await flushPromises();
      expect(unknown.find(".ios-idle").exists()).toBe(false);
      unknown.unmount();
    });

    it("speaks Chinese when the interface is Chinese", async () => {
      i18n.global.locale.value = "zh";
      iosOp.mockResolvedValue({ kind: "state", state: gateway({ macIdleSecs: 600 }) });
      const wrapper = mountCard();
      await flushPromises();
      expect(optionTexts(wrapper)).toEqual(["立即", "5 分钟", "15 分钟", "30 分钟", "1 小时"]);
      expect(wrapper.get(".ios-idle").text()).toBe("Mac 已空闲 10 分钟");
      wrapper.unmount();
    });
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

  it("tests the broker and the relay address of the form", async () => {
    iosOp.mockResolvedValue({ kind: "state", state: gateway() });
    iosTest.mockResolvedValue(testOk());
    const wrapper = mountCard();
    await flushPromises();
    const testButton = () => wrapper.findAll("button").find((b) => b.text() === "Test connection")!;

    await testButton().trigger("click");
    await flushPromises();
    expect(iosTest).toHaveBeenCalledWith({
      host: "broker.example.com",
      port: 8883,
      wsPort: 8084,
      username: "abc123",
      password: "",
      relayUrl: "https://relay.example.com",
    });
    expect(wrapper.get(".result").classes()).toContain("ok");
    expect(wrapper.get(".result").text()).toContain("The relay answered in 850 ms");

    iosTest.mockRejectedValue(
      testFailed("relay", "The broker works, but the relay check failed: the relay did not answer in time"),
    );
    await testButton().trigger("click");
    await flushPromises();
    expect(wrapper.get(".result").classes()).toContain("err");
    expect(wrapper.get(".result").text()).toContain("the relay check failed");
    wrapper.unmount();
  });

  describe("test connection: warnings that never block, and a hint that fits the service", () => {
    const testButton = (wrapper: ReturnType<typeof mountCard>) =>
      wrapper.findAll("button").find((b) => b.text() === "Test connection")!;
    const hint = (wrapper: ReturnType<typeof mountCard>) => wrapper.find(".ios-test-hint");

    beforeEach(() => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
    });

    it("shows what the check found beside a result that is fine, in the warning colour", async () => {
      iosTest.mockResolvedValue(testOk(["The broker let this account publish outside its own topics."]));
      const wrapper = mountCard();
      await flushPromises();
      await testButton(wrapper).trigger("click");
      await flushPromises();
      const results = wrapper.findAll(".result");
      expect(results.map((r) => r.classes().filter((c) => c !== "result"))).toEqual([["ok"], ["warn"]]);
      expect(results[1]!.text()).toContain("publish outside its own topics");
      // The warning is not an error: nothing about the setting is held back.
      expect(hint(wrapper).exists()).toBe(false);
      wrapper.unmount();
    });

    it("keeps the warning when the relay is what failed", async () => {
      iosTest.mockRejectedValue(testFailed("relay", "The broker works, but the relay check failed: x", ["outside"]));
      const wrapper = mountCard();
      await flushPromises();
      await testButton(wrapper).trigger("click");
      await flushPromises();
      expect(wrapper.findAll(".result").map((r) => r.classes().filter((c) => c !== "result"))).toEqual([["err"], ["warn"]]);
      expect(hint(wrapper).exists()).toBe(false);
      wrapper.unmount();
    });

    it("gives each service its own hint for a refused login, no echo and a timeout", async () => {
      const wrapper = mountCard();
      await flushPromises();
      const ios = config.value.channels.ios;

      ios.host = "mqtt.flespi.io";
      iosTest.mockRejectedValue(testFailed("rejected", "The broker rejected the credentials (bad user name or password)."));
      await testButton(wrapper).trigger("click");
      await flushPromises();
      expect(wrapper.get(".result").text()).toContain("The broker rejected the credentials");
      expect(hint(wrapper).text()).toContain("token's ID");

      ios.host = "abc123.ala.cn-hangzhou.emqxsl.cn";
      iosTest.mockRejectedValue(testFailed("noEcho", "Connected via tls, but a test message did not come back."));
      await testButton(wrapper).trigger("click");
      await flushPromises();
      expect(hint(wrapper).text()).toContain("Access Control → Authorization");

      ios.host = "broker.example.com";
      iosTest.mockRejectedValue(testFailed("timeout", "Timed out waiting for the broker."));
      await testButton(wrapper).trigger("click");
      await flushPromises();
      expect(hint(wrapper).text()).toContain("public CA");
      wrapper.unmount();
    });

    it("has nothing to add for a failure that is not about the service's settings", async () => {
      iosTest.mockRejectedValue(testFailed("input", "Fill in the broker host and username first"));
      const wrapper = mountCard();
      await flushPromises();
      await testButton(wrapper).trigger("click");
      await flushPromises();
      expect(wrapper.get(".result").classes()).toContain("err");
      expect(hint(wrapper).exists()).toBe(false);

      iosTest.mockRejectedValue(testFailed("other", "The test could not run: not connected."));
      await testButton(wrapper).trigger("click");
      await flushPromises();
      expect(hint(wrapper).exists()).toBe(false);
      wrapper.unmount();
    });

    it("shows a failure that arrives as plain text, and clears the last result when it tests again", async () => {
      iosTest.mockRejectedValue("the command failed");
      const wrapper = mountCard();
      await flushPromises();
      await testButton(wrapper).trigger("click");
      await flushPromises();
      expect(wrapper.get(".result").text()).toBe("the command failed");
      expect(wrapper.findAll(".result")).toHaveLength(1);

      let finish: (value: ReturnType<typeof testOk>) => void = () => {};
      iosTest.mockReturnValue(new Promise((resolve) => (finish = resolve)));
      await testButton(wrapper).trigger("click");
      await flushPromises();
      expect(wrapper.find(".result").exists()).toBe(false);
      finish(testOk());
      await flushPromises();
      expect(wrapper.get(".result").classes()).toContain("ok");
      wrapper.unmount();
    });

    it("keeps the hint of the service the test ran against when the form changes afterwards", async () => {
      config.value.channels.ios.host = "mqtt.flespi.io";
      iosTest.mockRejectedValue(testFailed("rejected", "The broker rejected the credentials."));
      const wrapper = mountCard();
      await flushPromises();
      await testButton(wrapper).trigger("click");
      await flushPromises();
      const before = hint(wrapper).text();
      config.value.channels.ios.host = "broker.example.com";
      await flushPromises();
      expect(hint(wrapper).text()).toBe(before);
      wrapper.unmount();
    });
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

  describe("the broker service: key points per service, ports filled in on a pick", () => {
    const select = (wrapper: ReturnType<typeof mountCard>) => wrapper.get<HTMLSelectElement>(".ios-broker-select");
    const points = (wrapper: ReturnType<typeof mountCard>) => wrapper.findAll(".ios-broker-points li").map((li) => li.text());

    beforeEach(() => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
    });

    it("lists the services and treats a host it does not know as self-hosted or another broker", async () => {
      const wrapper = mountCard();
      await flushPromises();
      expect(select(wrapper).findAll("option").map((o) => o.text())).toEqual([
        "EMQX Cloud Serverless",
        "flespi",
        "Self-hosted or another broker",
      ]);
      expect(select(wrapper).element.value).toBe("custom");
      expect(points(wrapper)).toHaveLength(4);
      expect(points(wrapper)[0]).toContain("MQTT 3.1.1");
      wrapper.unmount();
    });

    it("recognizes a service by its host name, with the points that matter for it", async () => {
      config.value.channels.ios.host = "abc123.ala.cn-hangzhou.emqxsl.cn";
      const wrapper = mountCard();
      await flushPromises();
      expect(select(wrapper).element.value).toBe("emqx");
      // The ACL step is the one that leaks other people's topics when it is skipped.
      expect(points(wrapper).find((p) => p.includes("ACL"))).toContain("catch-all deny");
      wrapper.unmount();
    });

    it("fills the host and ports of a service on a pick, and keeps what was typed otherwise", async () => {
      const wrapper = mountCard();
      await flushPromises();
      const ios = config.value.channels.ios;

      await select(wrapper).setValue("flespi");
      await flushPromises();
      expect([ios.host, ios.port, ios.wsPort]).toEqual(["mqtt.flespi.io", 8883, 443]);
      expect(persist).toHaveBeenCalled();
      expect(points(wrapper)[0]).toContain("mqtt.flespi.io");

      // EMQX gives every account its own host: only the ports are filled in.
      await select(wrapper).setValue("emqx");
      await flushPromises();
      expect([ios.host, ios.port, ios.wsPort]).toEqual(["mqtt.flespi.io", 8883, 8084]);

      // "Self-hosted" fills nothing and does not save anything.
      persist.mockClear();
      ios.port = 1884;
      await select(wrapper).setValue("custom");
      await flushPromises();
      expect([ios.host, ios.port, ios.wsPort]).toEqual(["mqtt.flespi.io", 1884, 8084]);
      expect(persist).not.toHaveBeenCalled();
      expect(select(wrapper).element.value).toBe("custom");
      wrapper.unmount();
    });

    it("lets a pick stand while the host is still being typed, and a pasted connection string decide again", async () => {
      const wrapper = mountCard();
      await flushPromises();
      await select(wrapper).setValue("emqx");
      await flushPromises();
      expect(select(wrapper).element.value).toBe("emqx");

      iosConnectionImport.mockResolvedValue({
        host: "mqtt.flespi.io",
        port: 8883,
        wsPort: 443,
        username: "123456",
        password: "token",
        relayUrl: "https://relay.example.com",
      });
      await wrapper.get(".ios-import input").setValue("ahconn1.abc");
      await wrapper.findAll(".ios-import button").find((b) => b.text() === "Import")!.trigger("click");
      await flushPromises();
      expect(select(wrapper).element.value).toBe("flespi");
      wrapper.unmount();
    });

    it("opens the step-by-step page of the service in the reader's language", async () => {
      config.value.channels.ios.host = "mqtt.flespi.io";
      const wrapper = mountCard();
      await flushPromises();
      await wrapper.get(".ios-broker-guide").trigger("click");
      expect(openPath).toHaveBeenLastCalledWith(
        "https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-broker.en.md#path-2-flespi-hosted-europe",
      );

      i18n.global.locale.value = "zh";
      await flushPromises();
      await wrapper.get(".ios-broker-guide").trigger("click");
      expect(openPath).toHaveBeenLastCalledWith(
        "https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-broker.md#路径二flespi托管欧洲",
      );
      wrapper.unmount();
    });
  });

  describe("the push relay: the public one built into the app, or one of your own", () => {
    const PUBLIC = "https://public-relay.example";
    const relaySelect = (wrapper: ReturnType<typeof mountCard>) => wrapper.find<HTMLSelectElement>(".ios-relay-mode");
    const relayInput = (wrapper: ReturnType<typeof mountCard>) => wrapper.find<HTMLInputElement>(".ios-relay input");

    beforeEach(() => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
    });

    it("is only the address field when this build has no public relay", async () => {
      const wrapper = mountCard();
      await flushPromises();
      expect(relaySelect(wrapper).exists()).toBe(false);
      expect(wrapper.find(".ios-relay-note").exists()).toBe(false);
      expect(wrapper.get(".ios-relay label").text()).toBe("Push relay URL");
      expect(relayInput(wrapper).element.value).toBe("https://relay.example.com");
      // The card's introduction agrees: with no public relay the user brings one.
      expect(wrapper.get(".card-desc").text()).toContain("You bring your own MQTT broker and a push relay.");
      wrapper.unmount();
    });

    it("shows an empty address as the public relay, with what it does and does not do", async () => {
      iosDefaultRelayUrl.mockResolvedValue(PUBLIC);
      config.value.channels.ios.relayUrl = "";
      const wrapper = mountCard();
      await flushPromises();
      expect(wrapper.get(".ios-relay label").text()).toBe("Push relay");
      expect(relaySelect(wrapper).element.value).toBe("public");
      expect(relayInput(wrapper).exists()).toBe(false);
      const note = wrapper.get(".ios-relay-note").text();
      expect(note).toContain("free, best effort, no availability guarantee");
      expect(note).toContain("stores nothing");
      expect(note).toContain("cannot read questions or answers");
      // The card's introduction agrees: only the broker is the user's to bring.
      const intro = wrapper.get(".card-desc").text();
      expect(intro).toContain("pushes go through the public relay unless you choose your own");
      expect(intro).not.toContain("and a push relay");
      wrapper.unmount();
    });

    it("shows an address that is set as the user's own relay, and keeps it", async () => {
      iosDefaultRelayUrl.mockResolvedValue(PUBLIC);
      const wrapper = mountCard();
      await flushPromises();
      expect(relaySelect(wrapper).element.value).toBe("own");
      expect(relayInput(wrapper).element.value).toBe("https://relay.example.com");
      expect(wrapper.find(".ios-relay-note").exists()).toBe(false);
      expect(config.value.channels.ios.relayUrl).toBe("https://relay.example.com");
      wrapper.unmount();
    });

    it("switches to the public relay by emptying the address, and back to an own one", async () => {
      iosDefaultRelayUrl.mockResolvedValue(PUBLIC);
      const wrapper = mountCard();
      await flushPromises();

      await relaySelect(wrapper).setValue("public");
      await flushPromises();
      expect(config.value.channels.ios.relayUrl).toBe("");
      expect(persist).toHaveBeenCalled();
      expect(relayInput(wrapper).exists()).toBe(false);
      expect(wrapper.find(".ios-relay-note").exists()).toBe(true);

      // Choosing "own" shows an empty field and stays on "own" until an address is typed or "public" is chosen again.
      persist.mockClear();
      await relaySelect(wrapper).setValue("own");
      await flushPromises();
      expect(relaySelect(wrapper).element.value).toBe("own");
      expect(relayInput(wrapper).element.value).toBe("");
      expect(persist).not.toHaveBeenCalled();

      await relayInput(wrapper).setValue("https://mine.example.com");
      await relayInput(wrapper).trigger("change");
      expect(config.value.channels.ios.relayUrl).toBe("https://mine.example.com");
      expect(persist).toHaveBeenCalled();

      // Clearing the field while editing does not yank the field away.
      await relayInput(wrapper).setValue("");
      expect(relaySelect(wrapper).element.value).toBe("own");
      wrapper.unmount();
    });

    it("follows an imported connection string: an empty relay in it means the public one", async () => {
      iosDefaultRelayUrl.mockResolvedValue(PUBLIC);
      iosConnectionImport.mockResolvedValue({
        host: "other.example.com",
        port: 8883,
        wsPort: 8084,
        username: "zzz999",
        password: "secret",
        relayUrl: "",
      });
      const wrapper = mountCard();
      await flushPromises();
      expect(relaySelect(wrapper).element.value).toBe("own");
      await wrapper.get(".ios-import input").setValue("ahconn1.abc");
      await wrapper.findAll(".ios-import button").find((b) => b.text() === "Import")!.trigger("click");
      await flushPromises();
      expect(config.value.channels.ios.relayUrl).toBe("");
      expect(relaySelect(wrapper).element.value).toBe("public");

      iosConnectionImport.mockResolvedValue({
        host: "other.example.com",
        port: 8883,
        wsPort: 8084,
        username: "zzz999",
        password: "secret",
        relayUrl: "https://relay2.example.com",
      });
      await wrapper.get(".ios-import input").setValue("ahconn1.def");
      await wrapper.findAll(".ios-import button").find((b) => b.text() === "Import")!.trigger("click");
      await flushPromises();
      expect(relaySelect(wrapper).element.value).toBe("own");
      expect(relayInput(wrapper).element.value).toBe("https://relay2.example.com");
      wrapper.unmount();
    });

    it("tests with the address as it is, so an empty one is resolved to the public relay by the app", async () => {
      iosDefaultRelayUrl.mockResolvedValue(PUBLIC);
      config.value.channels.ios.relayUrl = "";
      iosTest.mockResolvedValue(testOk());
      const wrapper = mountCard();
      await flushPromises();
      await wrapper.findAll("button").find((b) => b.text() === "Test connection")!.trigger("click");
      await flushPromises();
      expect(iosTest).toHaveBeenCalledWith(expect.objectContaining({ relayUrl: "" }));
      wrapper.unmount();
    });
  });

  describe("the app's New task needs the Agent-task switch, which is in the card", () => {
    beforeEach(() => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
    });

    it("has the switch right under the status, above the connection form", async () => {
      const wrapper = mountCard();
      await flushPromises();
      const row = wrapper.get('[data-testid="agent-tasks-row"]');
      expect(row.text()).toContain("Allow creating Agent tasks from phone or IM");
      expect(row.text()).toContain("One switch for all channels");
      const html = wrapper.html();
      expect(html.indexOf('data-testid="agent-tasks-row"')).toBeGreaterThan(html.indexOf("ios-status"));
      expect(html.indexOf('data-testid="agent-tasks-row"')).toBeLessThan(html.indexOf("ios-section-title"));
      // The old callout that only pointed at the switch is gone.
      expect(wrapper.find('[data-testid="ios-tasks-callout"]').exists()).toBe(false);
      wrapper.unmount();
    });

    it("hands the change to the shared switch handler, which asks for the confirmation", async () => {
      const wrapper = mountCard();
      await flushPromises();
      const input = wrapper.get<HTMLInputElement>('[data-testid="agent-tasks-switch"]');
      expect(input.element.checked).toBe(false);
      await input.setValue(true);
      expect(toggleAgentTasks).toHaveBeenCalledTimes(1);
      wrapper.unmount();
    });

    it("shows the switch on, says it is shared, and leads to the Advanced tab", async () => {
      config.value.agentTasks.enabled = true;
      const wrapper = mountCard();
      await flushPromises();
      expect(wrapper.get<HTMLInputElement>('[data-testid="agent-tasks-switch"]').element.checked).toBe(true);
      expect(wrapper.get('[data-testid="agent-tasks-row"]').text()).toContain("Allowed, for all channels.");
      await wrapper.get('[data-testid="agent-tasks-manage"]').trigger("click");
      expect(activeTab.value).toBe("advanced");
      wrapper.unmount();
    });

    it("warns only when the loaded readiness shows no Agent that can start a task", async () => {
      config.value.agentTasks.enabled = true;
      const wrapper = mountCard();
      await flushPromises();
      // Nothing loaded yet: it never probes by itself, so it says nothing.
      expect(wrapper.find('[data-testid="agent-tasks-stuck"]').exists()).toBe(false);

      taskReadiness.value = [
        { kind: "claude", label: "Claude Code", ready: false, launchSupported: true, startable: false },
      ];
      await flushPromises();
      expect(wrapper.get('[data-testid="agent-tasks-stuck"]').text()).toContain("No Agent can be started");

      // One that starts by its app or window is enough, whatever its CLI says.
      taskReadiness.value = [
        { kind: "claude", label: "Claude Code", ready: false, launchSupported: true, startable: false },
        { kind: "codex", label: "Codex", ready: false, launchSupported: true, startable: true, startRoute: "desktop" },
      ];
      await flushPromises();
      expect(wrapper.find('[data-testid="agent-tasks-stuck"]').exists()).toBe(false);

      // And a switch that is off has nothing to warn about.
      config.value.agentTasks.enabled = false;
      taskReadiness.value = [
        { kind: "claude", label: "Claude Code", ready: false, launchSupported: true, startable: false },
      ];
      await flushPromises();
      expect(wrapper.find('[data-testid="agent-tasks-stuck"]').exists()).toBe(false);
      wrapper.unmount();
    });

    it("is not there where Agent tasks are not supported, or while the channel is off", async () => {
      supportsAgentTasks = false;
      provideContext();
      const unsupported = mountCard();
      await flushPromises();
      expect(unsupported.find('[data-testid="agent-tasks-row"]').exists()).toBe(false);
      unsupported.unmount();

      supportsAgentTasks = true;
      provideContext();
      config.value.channels.ios.enabled = false;
      const off = mountCard();
      await flushPromises();
      expect(off.find('[data-testid="agent-tasks-row"]').exists()).toBe(false);
      off.unmount();
    });

    it("speaks Chinese when the interface is Chinese", async () => {
      i18n.global.locale.value = "zh";
      const wrapper = mountCard();
      await flushPromises();
      expect(wrapper.get('[data-testid="agent-tasks-row"]').text()).toContain("允许从手机 / IM 创建 Agent 任务");
      expect(wrapper.get('[data-testid="agent-tasks-row"]').text()).toContain("所有渠道共用这一个开关");
      wrapper.unmount();
    });
  });
});
