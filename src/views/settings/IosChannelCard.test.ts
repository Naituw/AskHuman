import { flushPromises, mount } from "@vue/test-utils";
import { reactive, ref, type Ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import IosChannelCard from "./IosChannelCard.vue";
import { IOS_JOIN_URL } from "./iosLinks";
import { SETTINGS_ACTION_EVENT } from "./reveal";

const iosOp = vi.hoisted(() => vi.fn());
const iosPairQr = vi.hoisted(() => vi.fn());
const iosTest = vi.hoisted(() => vi.fn());
const iosConnectionImport = vi.hoisted(() => vi.fn());
const iosConnectionExport = vi.hoisted(() => vi.fn());
const iosDefaultRelayUrl = vi.hoisted(() => vi.fn());
const openPath = vi.hoisted(() => vi.fn());
const setupProbeStart = vi.hoisted(() => vi.fn());
const setupProbeCancel = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() => vi.fn());
const useSettingsContext = vi.hoisted(() => vi.fn());

vi.mock("../../lib/ipc", () => ({
  iosOp,
  iosPairQr,
  iosTest,
  iosConnectionImport,
  iosConnectionExport,
  iosDefaultRelayUrl,
  openPath,
  setupProbeStart,
  setupProbeCancel,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen }));
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
  let secretsPresent: Ref<{ iosBrokerPassword: boolean }>;
  let supportsAgentTasks: boolean;
  let activeTab: ReturnType<typeof ref<string>>;
  let taskReadiness: ReturnType<typeof ref<unknown[]>>;
  let probeResult: ((result: Record<string, unknown>) => void) | null;
  const persist = vi.fn(async () => {});
  const toggleAgentTasks = vi.fn();
  const changeDaemonLifecycle = vi.fn(async () => {});
  const writeText = vi.fn(async () => {});

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
      general: reactive({ daemonLifecycle: "activity" }),
    });
  }

  function provideContext() {
    useSettingsContext.mockReturnValue({
      config,
      persist,
      secretsPresent,
      SECRET_PLACEHOLDER: "••••",
      clearSecret: vi.fn(),
      supportsAgentTasks,
      toggleAgentTasks,
      changeDaemonLifecycle,
      taskReadiness,
      activeTab,
      settingsTargetHighlight: ref(""),
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
    setupProbeStart.mockReset();
    setupProbeCancel.mockReset();
    setupProbeCancel.mockResolvedValue(undefined);
    probeResult = null;
    listen.mockReset();
    listen.mockImplementation(async (_event: string, handler: (e: { payload: Record<string, unknown> }) => void) => {
      probeResult = (payload) => handler({ payload });
      return () => {
        probeResult = null;
      };
    });
    writeText.mockClear();
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    persist.mockClear();
    changeDaemonLifecycle.mockClear();
    iosPairQr.mockResolvedValue('<svg id="qr"></svg>');
    config = makeConfig(true);
    secretsPresent = ref({ iosBrokerPassword: true });
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

  type Card = ReturnType<typeof mountCard>;
  /** What a jump to the card (`channel#channel-ios#<action>`) tells it, as the Settings window does. */
  const announce = (wrapper: Card, action: string) =>
    wrapper
      .get("#channel-ios")
      .element.dispatchEvent(new CustomEvent(SETTINGS_ACTION_EVENT, { bubbles: true, detail: { action } }));
  const button = (wrapper: Card, text: string) => wrapper.findAll("button").find((b) => b.text() === text);
  const byId = (wrapper: Card, id: string) => wrapper.get(`[data-testid="${id}"]`);
  const tag = (wrapper: Card) => byId(wrapper, "channel-state-ios").text();
  const stepClasses = (wrapper: Card, n: number) => byId(wrapper, `ios-step-${n}`).classes();

  // ——— the three forms of the card ———

  describe("off", () => {
    beforeEach(() => {
      config.value.channels.ios.enabled = false;
    });

    it("does not touch the daemon, and offers the two ways to begin", async () => {
      const wrapper = mountCard();
      await flushPromises();
      expect(iosOp).not.toHaveBeenCalled();
      expect(wrapper.find('[data-testid="ios-steps"]').exists()).toBe(false);
      expect(wrapper.find('[data-testid="ios-ready"]').exists()).toBe(false);
      expect(byId(wrapper, "ios-start").text()).toBe("Start setup");
      expect(byId(wrapper, "ios-agent").text()).toBe("Set up with an Agent");
      expect(wrapper.get(".card-title").text()).toBe("iOS App");
      wrapper.unmount();
    });

    it("says whether anything was filled in yet", async () => {
      const filled = mountCard();
      await flushPromises();
      expect(tag(filled)).toBe("Off");
      filled.unmount();

      config.value.channels.ios.host = "";
      secretsPresent.value.iosBrokerPassword = false;
      const blank = mountCard();
      await flushPromises();
      expect(tag(blank)).toBe("Not set up");
      blank.unmount();
    });

    it("turns the channel on with 'Start setup' and shows the steps", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: null });
      const wrapper = mountCard();
      await flushPromises();
      await byId(wrapper, "ios-start").trigger("click");
      await flushPromises();
      expect(config.value.channels.ios.enabled).toBe(true);
      expect(persist).toHaveBeenCalled();
      expect(wrapper.find('[data-testid="ios-steps"]').exists()).toBe(true);
      expect(wrapper.find('[data-testid="ios-start"]').exists()).toBe(false);
      wrapper.unmount();
    });

    it("starts the setup when a link says so, as the Start button does, and only then", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: null });
      const wrapper = mountCard();
      await flushPromises();

      // Any other action is not this card's to take.
      announce(wrapper, "somewhere-else");
      await flushPromises();
      expect(config.value.channels.ios.enabled).toBe(false);
      expect(persist).not.toHaveBeenCalled();

      announce(wrapper, "start");
      await flushPromises();
      expect(config.value.channels.ios.enabled).toBe(true);
      expect(persist).toHaveBeenCalledTimes(1);
      expect(wrapper.find('[data-testid="ios-steps"]').exists()).toBe(true);
      wrapper.unmount();
    });

    it("shows the prompt for an Agent in full and copies it only on a click", async () => {
      const wrapper = mountCard();
      await flushPromises();
      expect(wrapper.find('[data-testid="ios-agent-box"]').exists()).toBe(false);
      await byId(wrapper, "ios-agent").trigger("click");
      const prompt =
        "Please set up AskHuman's iOS App channel. First run:\nAskHuman channel ios --agent-guide\nThen follow the steps it prints.";
      expect(byId(wrapper, "ios-agent-prompt").text()).toBe(prompt);
      expect(writeText).not.toHaveBeenCalled();

      await byId(wrapper, "ios-agent-copy").trigger("click");
      await flushPromises();
      expect(writeText).toHaveBeenCalledWith(prompt);
      expect(byId(wrapper, "ios-agent-copy").text()).toBe("Copied");

      await byId(wrapper, "ios-agent").trigger("click");
      expect(wrapper.find('[data-testid="ios-agent-box"]').exists()).toBe(false);
      wrapper.unmount();
    });

    it("hands the prompt over in Chinese when the interface is Chinese", async () => {
      i18n.global.locale.value = "zh";
      const wrapper = mountCard();
      await flushPromises();
      expect(byId(wrapper, "ios-start").text()).toBe("开始设置");
      await byId(wrapper, "ios-agent").trigger("click");
      expect(byId(wrapper, "ios-agent-prompt").text()).toBe(
        "请帮我配置 AskHuman 的 iOS App 渠道。先运行：\nAskHuman channel ios --agent-guide\n然后按输出的步骤做。",
      );
      wrapper.unmount();
    });
  });

  describe("setup", () => {
    it("is only brought on screen by a link that says start when it is on already", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: null });
      const wrapper = mountCard();
      await flushPromises();
      expect(config.value.channels.ios.enabled).toBe(true);

      announce(wrapper, "start");
      await flushPromises();
      // Nothing is turned off and nothing is saved: "start" is for a card that is still off.
      expect(config.value.channels.ios.enabled).toBe(true);
      expect(persist).not.toHaveBeenCalled();
      wrapper.unmount();
    });

    it("starts at the connection while the gateway is not running", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: null });
      const wrapper = mountCard();
      await flushPromises();
      expect(byId(wrapper, "ios-progress").text()).toBe("1 / 3");
      expect(tag(wrapper)).toBe("Enabled");
      expect(stepClasses(wrapper, 1)).toEqual(expect.arrayContaining(["current", "open"]));
      expect(stepClasses(wrapper, 2)).toContain("upcoming");
      expect(stepClasses(wrapper, 2)).not.toContain("open");
      expect(stepClasses(wrapper, 3)).toContain("upcoming");
      expect(wrapper.find('[data-testid="ios-ready"]').exists()).toBe(false);
      // The Agent button sits at the top right of the steps, not in the off state's place.
      expect(button(wrapper, "Set up with an Agent")).toBeDefined();
      wrapper.unmount();
    });

    it("moves on to the app once the broker is connected, and folds the finished step to one line", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway({ phones: [] }) });
      const wrapper = mountCard();
      await flushPromises();
      expect(byId(wrapper, "ios-progress").text()).toBe("2 / 3");
      expect(tag(wrapper)).toBe("Connected");
      expect(stepClasses(wrapper, 1)).toContain("done");
      expect(stepClasses(wrapper, 1)).not.toContain("open");
      expect(byId(wrapper, "ios-step-1").text()).toContain("broker.example.com · Connected");
      expect(stepClasses(wrapper, 2)).toEqual(expect.arrayContaining(["current", "open"]));
      wrapper.unmount();
    });

    it("names a hosted broker by its service, and tucks its instructions away once it works", async () => {
      config.value.channels.ios.host = "abc123.ala.cn-hangzhou.emqxsl.cn";
      iosOp.mockResolvedValue({ kind: "state", state: gateway({ phones: [] }) });
      const wrapper = mountCard();
      await flushPromises();
      expect(byId(wrapper, "ios-step-1").text()).toContain("EMQX Cloud · Connected");
      await byId(wrapper, "ios-step-1").get(".ios-step-head").trigger("click");
      expect(stepClasses(wrapper, 1)).toContain("open");
      expect(stepClasses(wrapper, 2)).not.toContain("open");
      const fold = byId(wrapper, "ios-provider-guide");
      expect(fold.element.tagName).toBe("DETAILS");
      expect(fold.get("summary").text()).toBe("Setup instructions (EMQX Cloud)");
      wrapper.unmount();
    });

    it("keeps the person in step 1 when the connection comes up, and moves on with 'Next'", async () => {
      vi.useFakeTimers();
      let connected = false;
      iosOp.mockImplementation(async (op: { op: string }) =>
        op.op === "state"
          ? { kind: "state", state: connected ? gateway({ phones: [] }) : null }
          : { kind: "done", found: true },
      );
      const wrapper = mountCard();
      await vi.advanceTimersByTimeAsync(0);
      expect(stepClasses(wrapper, 1)).toContain("open");
      expect(wrapper.find('[data-testid="ios-next-1"]').exists()).toBe(false);

      connected = true;
      await vi.advanceTimersByTimeAsync(3100);
      // Done, but still open, with the way to go on: the page does not move under the person's hands.
      expect(stepClasses(wrapper, 1)).toEqual(expect.arrayContaining(["done", "open"]));
      expect(stepClasses(wrapper, 2)).not.toContain("open");
      expect(byId(wrapper, "ios-progress").text()).toBe("2 / 3");

      await byId(wrapper, "ios-next-1").trigger("click");
      expect(stepClasses(wrapper, 1)).not.toContain("open");
      expect(stepClasses(wrapper, 2)).toContain("open");
      wrapper.unmount();
    });

    it("lets any step be opened, one at a time", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway({ phones: [] }) });
      const wrapper = mountCard();
      await flushPromises();
      await byId(wrapper, "ios-step-3").get(".ios-step-head").trigger("click");
      expect(stepClasses(wrapper, 3)).toContain("open");
      expect(stepClasses(wrapper, 2)).not.toContain("open");
      await byId(wrapper, "ios-step-3").get(".ios-step-head").trigger("click");
      expect(wrapper.findAll(".ios-step.open")).toHaveLength(0);
      wrapper.unmount();
    });

    it("shows the QR code of the landing page in step 2 and takes the person's word that the app is installed", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway({ phones: [] }) });
      const wrapper = mountCard();
      await flushPromises();
      expect(iosPairQr).toHaveBeenCalledWith(IOS_JOIN_URL);
      expect(byId(wrapper, "ios-join-qr").find("#qr").exists()).toBe(true);
      expect(wrapper.get(".ios-side").text()).toContain("Install TestFlight on your iPhone.");
      expect(wrapper.get(".ios-side").text()).toContain("Requires iOS 17 or later.");

      await byId(wrapper, "ios-join-copy").trigger("click");
      await flushPromises();
      expect(writeText).toHaveBeenCalledWith(IOS_JOIN_URL);
      expect(byId(wrapper, "ios-join-copy").text()).toBe("Copied");
      await byId(wrapper, "ios-join-open").trigger("click");
      expect(openPath).toHaveBeenCalledWith(IOS_JOIN_URL);

      await byId(wrapper, "ios-join-done").trigger("click");
      expect(byId(wrapper, "ios-progress").text()).toBe("3 / 3");
      expect(byId(wrapper, "ios-step-2").text()).toContain("Installed");
      expect(stepClasses(wrapper, 2)).toContain("done");
      expect(stepClasses(wrapper, 3)).toEqual(expect.arrayContaining(["current", "open"]));
      expect(wrapper.get('[data-testid="ios-pair-help"]').text()).toContain("Pair a new phone");
      wrapper.unmount();
    });

    it("still gets the person to the link when the QR code cannot be drawn", async () => {
      iosPairQr.mockRejectedValue("no code");
      iosOp.mockResolvedValue({ kind: "state", state: gateway({ phones: [] }) });
      const wrapper = mountCard();
      await flushPromises();
      expect(wrapper.find('[data-testid="ios-join-qr"]').exists()).toBe(false);
      expect(wrapper.find('[data-testid="ios-join-open"]').exists()).toBe(true);
      wrapper.unmount();
    });

    it("says what is wrong when the gateway reports an error", async () => {
      iosOp.mockResolvedValue({
        kind: "state",
        state: gateway({ connected: false, lastError: "authentication failed", phones: [] }),
      });
      const wrapper = mountCard();
      await flushPromises();
      expect(tag(wrapper)).toBe("Problem");
      expect(byId(wrapper, "ios-issue").text()).toBe("Broker not connected: authentication failed");
      expect(stepClasses(wrapper, 1)).toEqual(expect.arrayContaining(["current", "open"]));
      wrapper.unmount();
    });

    it("says the settings are incomplete while a field is missing, and counts a stored password", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: null });
      config.value.channels.ios.username = "";
      const wrapper = mountCard();
      await flushPromises();
      expect(tag(wrapper)).toBe("Incomplete");
      wrapper.unmount();

      config.value.channels.ios.username = "abc123";
      secretsPresent.value.iosBrokerPassword = false;
      const noPassword = mountCard();
      await flushPromises();
      expect(tag(noPassword)).toBe("Incomplete");
      noPassword.unmount();

      secretsPresent.value.iosBrokerPassword = false;
      config.value.channels.ios.password = "typed";
      const typed = mountCard();
      await flushPromises();
      expect(tag(typed)).toBe("Enabled");
      typed.unmount();
    });

    it("speaks Chinese when the interface is Chinese", async () => {
      i18n.global.locale.value = "zh";
      iosOp.mockResolvedValue({ kind: "state", state: gateway({ phones: [] }) });
      const wrapper = mountCard();
      await flushPromises();
      expect(wrapper.get(".ios-guide-title").text()).toBe("设置步骤");
      expect(byId(wrapper, "ios-step-1").text()).toContain("连接 Broker");
      expect(byId(wrapper, "ios-step-1").text()).toContain("broker.example.com · 已连接");
      expect(byId(wrapper, "ios-step-2").text()).toContain("安装 iPhone App");
      expect(byId(wrapper, "ios-step-3").text()).toContain("配对手机");
      wrapper.unmount();
    });
  });

  describe("ready", () => {
    beforeEach(() => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
    });

    it("folds the steps into one box and shows the usual sections", async () => {
      const wrapper = mountCard();
      await flushPromises();
      expect(tag(wrapper)).toBe("Connected");
      expect(wrapper.find('[data-testid="ios-steps"]').exists()).toBe(false);
      expect(byId(wrapper, "ios-ready-line").text()).toBe("Ready · broker.example.com · 1 phone online");
      expect(wrapper.get(".ios-device-name").text()).toBe("Tian's iPhone");
      // No prompt for an Agent any more: a finished setup does not need one.
      expect(button(wrapper, "Set up with an Agent")).toBeUndefined();
      wrapper.unmount();
    });

    it("says when no phone is online", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway({ phones: [{ ...phone, online: false }] }) });
      const wrapper = mountCard();
      await flushPromises();
      expect(byId(wrapper, "ios-ready-line").text()).toBe("Ready · broker.example.com · No phone is online");
      wrapper.unmount();
    });

    it("counts several phones", async () => {
      iosOp.mockResolvedValue({
        kind: "state",
        state: gateway({ phones: [phone, { ...phone, phoneId: "p2" }, { ...phone, phoneId: "p3", online: false }] }),
      });
      const wrapper = mountCard();
      await flushPromises();
      expect(byId(wrapper, "ios-ready-line").text()).toBe("Ready · broker.example.com · 2 phones online");
      wrapper.unmount();
    });

    it("does not count a revoked phone", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway({ phones: [{ ...phone, state: "revoked" }] }) });
      const wrapper = mountCard();
      await flushPromises();
      expect(wrapper.find('[data-testid="ios-ready"]').exists()).toBe(false);
      expect(byId(wrapper, "ios-progress").text()).toBe("2 / 3");
      wrapper.unmount();
    });

    it("brings the steps back above the box, all of them done, and takes them away again", async () => {
      const wrapper = mountCard();
      await flushPromises();
      await byId(wrapper, "ios-steps-toggle").trigger("click");
      expect(byId(wrapper, "ios-steps-toggle").text()).toBe("Hide setup steps");
      expect(byId(wrapper, "ios-progress").text()).toBe("Done");
      expect([1, 2, 3].map((n) => stepClasses(wrapper, n).includes("done"))).toEqual([true, true, true]);
      expect(wrapper.findAll(".ios-step.open")).toHaveLength(0);
      const html = wrapper.html();
      expect(html.indexOf('data-testid="ios-steps"')).toBeLessThan(html.indexOf('data-testid="ios-ready"'));
      expect(byId(wrapper, "ios-step-3").text()).toContain("1 phone");

      // Any finished step can be read again, say the TestFlight link for a second phone.
      await byId(wrapper, "ios-step-2").get(".ios-step-head").trigger("click");
      expect(wrapper.find('[data-testid="ios-join"]').exists()).toBe(true);

      await byId(wrapper, "ios-steps-toggle").trigger("click");
      expect(wrapper.find('[data-testid="ios-steps"]').exists()).toBe(false);
      expect(byId(wrapper, "ios-steps-toggle").text()).toBe("Show setup steps");
      wrapper.unmount();
    });

    it("keeps the box and the sections when the broker goes away, and says so there", async () => {
      vi.useFakeTimers();
      let connected = true;
      iosOp.mockImplementation(async (op: { op: string }) =>
        op.op === "state"
          ? { kind: "state", state: gateway({ connected, lastError: connected ? undefined : "connection lost" }) }
          : { kind: "done", found: true },
      );
      const wrapper = mountCard();
      await vi.advanceTimersByTimeAsync(0);
      expect(byId(wrapper, "ios-ready-line").text()).toBe("Ready · broker.example.com · 1 phone online");

      // The phone stays paired, so the card stays what it was; the connection shows in the tag, the
      // line under the title and the box's own line, and a question could not reach the phone now.
      connected = false;
      await vi.advanceTimersByTimeAsync(3100);
      expect(wrapper.find('[data-testid="ios-ready"]').exists()).toBe(true);
      expect(wrapper.find('[data-testid="ios-steps"]').exists()).toBe(false);
      expect(wrapper.get(".ios-device-name").text()).toBe("Tian's iPhone");
      expect(tag(wrapper)).toBe("Problem");
      expect(byId(wrapper, "ios-issue").text()).toBe("Broker not connected: connection lost");
      expect(byId(wrapper, "ios-ready-line").text()).toBe("Broker not connected · broker.example.com");
      // Not the green "all good" box while the broker is down.
      expect(byId(wrapper, "ios-ready").classes()).toContain("down");
      expect(wrapper.get(".ios-ready .ios-dot").classes()).not.toContain("on");
      expect(byId(wrapper, "ios-try").attributes("disabled")).toBeDefined();

      // The way back to the steps opens the connection: the step that needs a look.
      await byId(wrapper, "ios-steps-toggle").trigger("click");
      expect(byId(wrapper, "ios-progress").text()).toBe("1 / 3");
      expect(stepClasses(wrapper, 1)).toEqual(expect.arrayContaining(["current", "open"]));
      expect(stepClasses(wrapper, 3)).toContain("done");

      connected = true;
      await vi.advanceTimersByTimeAsync(3100);
      expect(byId(wrapper, "ios-ready-line").text()).toBe("Ready · broker.example.com · 1 phone online");
      expect(byId(wrapper, "ios-ready").classes()).not.toContain("down");
      expect(wrapper.get(".ios-ready .ios-dot").classes()).toContain("on");
      expect(byId(wrapper, "ios-try").attributes("disabled")).toBeUndefined();
      wrapper.unmount();
    });

    it("says in Chinese that the broker is not connected", async () => {
      i18n.global.locale.value = "zh";
      iosOp.mockResolvedValue({
        kind: "state",
        state: gateway({ connected: false, lastError: "connection lost" }),
      });
      const wrapper = mountCard();
      await flushPromises();
      expect(byId(wrapper, "ios-ready-line").text()).toBe("Broker 未连接 · broker.example.com");
      wrapper.unmount();
    });

    it("keeps the EMQX note at the bottom for that service only", async () => {
      const note = "if the free allowance (1 million connection minutes and 1 GB of traffic per month) runs out";
      const other = mountCard();
      await flushPromises();
      expect(other.find('[data-testid="ios-emqx-notice"]').exists()).toBe(false);
      other.unmount();

      config.value.channels.ios.host = "abc123.ala.cn-hangzhou.emqxsl.cn";
      const emqx = mountCard();
      await flushPromises();
      expect(byId(emqx, "ios-emqx-notice").text()).toContain(note);
      expect(byId(emqx, "ios-ready-line").text()).toContain("EMQX Cloud");
      emqx.unmount();
    });

    it("speaks Chinese when the interface is Chinese", async () => {
      i18n.global.locale.value = "zh";
      const wrapper = mountCard();
      await flushPromises();
      expect(byId(wrapper, "ios-ready-line").text()).toBe("已就绪 · broker.example.com · 1 台手机在线");
      expect(byId(wrapper, "ios-steps-toggle").text()).toBe("查看设置步骤");
      expect(byId(wrapper, "ios-try-text").text()).toBe("发一条测试问题，确认手机能收到并回答。");
      wrapper.unmount();
    });
  });

  // ——— the test question ———

  describe("the test question", () => {
    const answer = (outcome: string) =>
      probeResult!({ probeId: "probe-1", kind: "channel", channel: "ios", outcome });

    beforeEach(() => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
      setupProbeStart.mockResolvedValue("probe-1");
    });

    it("sends a real question to the phone and reports the answer", async () => {
      const wrapper = mountCard();
      await flushPromises();
      expect(byId(wrapper, "ios-try-text").text()).toBe(
        "Send a test question to check that your phone receives it and can answer.",
      );
      expect(byId(wrapper, "ios-try").text()).toBe("Send test question");

      await byId(wrapper, "ios-try").trigger("click");
      await flushPromises();
      expect(setupProbeStart).toHaveBeenCalledWith("channel", "ios");
      expect(byId(wrapper, "ios-try-text").text()).toBe("Sent. Answer it on your phone…");
      expect(byId(wrapper, "ios-try").attributes("disabled")).toBeDefined();

      answer("ok");
      await flushPromises();
      expect(byId(wrapper, "ios-try-text").text()).toBe("Answer received. Everything works.");
      expect(byId(wrapper, "ios-try").text()).toBe("Send another");
      expect(byId(wrapper, "ios-try").attributes("disabled")).toBeUndefined();
      wrapper.unmount();
    });

    it("says when nobody answered in time, and lets the person try again", async () => {
      const wrapper = mountCard();
      await flushPromises();
      await byId(wrapper, "ios-try").trigger("click");
      await flushPromises();
      answer("timeout");
      await flushPromises();
      expect(byId(wrapper, "ios-try-text").text()).toBe(
        "No answer within 120 seconds. Make sure the phone is online and notifications are allowed.",
      );
      expect(byId(wrapper, "ios-try").text()).toBe("Send test question");
      wrapper.unmount();
    });

    it("says when the question was closed on the phone", async () => {
      const wrapper = mountCard();
      await flushPromises();
      await byId(wrapper, "ios-try").trigger("click");
      await flushPromises();
      answer("cancelled");
      await flushPromises();
      expect(byId(wrapper, "ios-try-text").text()).toBe("Test cancelled.");
      wrapper.unmount();
    });

    it("says why the question could not be sent", async () => {
      setupProbeStart.mockRejectedValue("the iOS channel is not connected");
      const wrapper = mountCard();
      await flushPromises();
      await byId(wrapper, "ios-try").trigger("click");
      await flushPromises();
      expect(byId(wrapper, "ios-try-text").text()).toBe(
        "The test question could not be sent: the iOS channel is not connected",
      );
      wrapper.unmount();
    });

    it("does not blame a missing reason on the person", async () => {
      const wrapper = mountCard();
      await flushPromises();
      await byId(wrapper, "ios-try").trigger("click");
      await flushPromises();
      answer("failed");
      await flushPromises();
      expect(byId(wrapper, "ios-try-text").text()).toBe("The test question could not be sent.");
      wrapper.unmount();
    });

    it("ignores the answer to somebody else's check", async () => {
      const wrapper = mountCard();
      await flushPromises();
      await byId(wrapper, "ios-try").trigger("click");
      await flushPromises();
      probeResult!({ probeId: "other", kind: "channel", channel: "ios", outcome: "ok" });
      await flushPromises();
      expect(byId(wrapper, "ios-try-text").text()).toBe("Sent. Answer it on your phone…");
      wrapper.unmount();
    });

    it("withdraws a question that is still waiting when the card goes away", async () => {
      const wrapper = mountCard();
      await flushPromises();
      await byId(wrapper, "ios-try").trigger("click");
      await flushPromises();
      wrapper.unmount();
      expect(setupProbeCancel).toHaveBeenCalledWith("probe-1");
    });
  });

  // ——— keeping the Mac reachable ———

  describe("keep this Mac reachable", () => {
    beforeEach(() => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
    });

    const keep = (wrapper: Card) => byId(wrapper, "ios-keep-switch");

    it("is the daemon lifecycle, switched from the card", async () => {
      const wrapper = mountCard();
      await flushPromises();
      expect((keep(wrapper).element as HTMLInputElement).checked).toBe(false);
      expect(byId(wrapper, "ios-keep-row").text()).toContain("When on, the background service stays running");
      await keep(wrapper).setValue(true);
      expect(changeDaemonLifecycle).toHaveBeenLastCalledWith("keepalive");
      await keep(wrapper).setValue(false);
      expect(changeDaemonLifecycle).toHaveBeenLastCalledWith("activity");
      wrapper.unmount();
    });

    it("shows a resident daemon as on", async () => {
      config.value.general.daemonLifecycle = "keepalive";
      const wrapper = mountCard();
      await flushPromises();
      expect((keep(wrapper).element as HTMLInputElement).checked).toBe(true);
      wrapper.unmount();
    });

    it("is pinned on, with the reason, while Agent tasks are allowed", async () => {
      config.value.agentTasks.enabled = true;
      const wrapper = mountCard();
      await flushPromises();
      expect((keep(wrapper).element as HTMLInputElement).checked).toBe(true);
      expect(keep(wrapper).attributes("disabled")).toBeDefined();
      expect(byId(wrapper, "ios-keep-row").text()).toContain(
        'Kept running by "Allow creating Agent tasks from phone or IM".',
      );
      wrapper.unmount();
    });

    it("is also in step 1, once a service is chosen", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: null });
      const wrapper = mountCard();
      await flushPromises();
      expect(byId(wrapper, "ios-step-1").find('[data-testid="ios-keep-switch"]').exists()).toBe(true);
      config.value.channels.ios.host = "";
      await flushPromises();
      expect(wrapper.find('[data-testid="ios-keep-switch"]').exists()).toBe(false);
      wrapper.unmount();
    });
  });

  // ——— traffic ———

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
    // against a plan: the usage is the broker's to show.
    expect(wrapper.get(".ios-traffic-details").text()).toContain("broker's console");
    expect(wrapper.find(".ios-traffic-details a").exists()).toBe(false);
    const trafficArea = [".ios-traffic", ".ios-traffic-details"].map((s) => wrapper.get(s).text()).join(" ");
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

  // ——— push timing ———

  describe("push timing", () => {
    const optionTexts = (wrapper: Card) => wrapper.findAll(".ios-alert-after option").map((o) => o.text());

    it("offers the waits, with the configured one selected", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
      const wrapper = mountCard();
      await flushPromises();
      expect(optionTexts(wrapper)).toEqual(["Immediately", "5 min", "15 min", "30 min", "1 h"]);
      expect((wrapper.get(".ios-alert-after select").element as HTMLSelectElement).value).toBe("900");
      wrapper.unmount();
    });

    it("explains in one short paragraph when the phone is notified", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
      const wrapper = mountCard();
      await flushPromises();
      expect(wrapper.text()).toContain(
        "Questions reach the phone app as soon as they are asked. The phone is only notified after the Mac's keyboard and mouse have been idle for this long, at most once per question; \"Immediately\" notifies right away.",
      );
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
      expect(optionTexts(wrapper)).toEqual(["Immediately", "5 min", "10 min (custom)", "15 min", "30 min", "1 h"]);
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

  // ——— pairing and phones ———

  describe("pairing", () => {
    let live: Record<string, unknown> | undefined;

    function pairingBackend(phones: unknown[]) {
      iosOp.mockImplementation(async (op: { op: string }) => {
        if (op.op === "state") return { kind: "state", state: gateway({ phones, pairing: live }) };
        if (op.op === "pairStart") {
          live = { ticketId: "t1", code: "ahpair1.x", expiresAtMs: Date.now() + 300_000, stage: "waiting" };
          return { kind: "pairing", ticketId: "t1", code: "ahpair1.x", expiresAtMs: Date.now() + 300_000 };
        }
        return { kind: "done", found: true };
      });
    }

    beforeEach(() => {
      live = undefined;
    });

    it("walks through pairing in step 3: QR code, confirmation code, allow, and the new phone makes it ready", async () => {
      vi.useFakeTimers();
      pairingBackend([]);
      const wrapper = mountCard();
      await vi.advanceTimersByTimeAsync(0);
      await byId(wrapper, "ios-step-3").get(".ios-step-head").trigger("click");
      // The first thing in step 3 is the way to the phone's side of it, and the primary button.
      expect(byId(wrapper, "ios-pair-help").text()).toContain("AskHuman → Macs → Add a Mac");
      expect(byId(wrapper, "ios-pair").classes()).toContain("btn-primary");

      await byId(wrapper, "ios-pair").trigger("click");
      await vi.advanceTimersByTimeAsync(0);
      expect(wrapper.find("#qr").exists()).toBe(true);
      expect(wrapper.text()).toContain("Scan the QR code");
      // The button gives way to the code while it runs.
      expect(wrapper.find('[data-testid="ios-pair"]').exists()).toBe(false);

      // The phone scans the code: the gateway moves to the confirmation stage, which has no use for the code.
      live = { ...live, stage: "pending", phoneName: "Work iPhone", sas: "123456" };
      await vi.advanceTimersByTimeAsync(1100);
      expect(wrapper.text()).toContain('"Work iPhone" wants to pair');
      expect(wrapper.get(".ios-sas").text()).toBe("123 456");
      expect(wrapper.find("#qr").exists()).toBe(false);
      expect(wrapper.find('[data-testid="ios-pair-help"]').exists()).toBe(false);

      await button(wrapper, "Allow")!.trigger("click");
      await vi.advanceTimersByTimeAsync(0);
      expect(iosOp).toHaveBeenCalledWith({ op: "pairDecision", ticketId: "t1", allow: true });

      // The session ends with a new phone in the list: success is reported, and the card is ready.
      live = undefined;
      pairingBackend([phone]);
      live = undefined;
      await vi.advanceTimersByTimeAsync(3000);
      expect(wrapper.find('[data-testid="ios-ready"]').exists()).toBe(true);
      expect(wrapper.text()).toContain('Paired "Tian\'s iPhone".');
      expect(wrapper.find("#qr").exists()).toBe(false);
      wrapper.unmount();
    });

    it("pairs another phone from the section of a finished setup", async () => {
      vi.useFakeTimers();
      pairingBackend([phone]);
      const wrapper = mountCard();
      await vi.advanceTimersByTimeAsync(0);
      await byId(wrapper, "ios-pair-ready").trigger("click");
      await vi.advanceTimersByTimeAsync(0);
      expect(wrapper.find("#qr").exists()).toBe(true);
      expect(byId(wrapper, "ios-pair-ready").attributes("disabled")).toBeDefined();
      await button(wrapper, "Cancel pairing")!.trigger("click");
      await vi.advanceTimersByTimeAsync(0);
      expect(iosOp).toHaveBeenCalledWith({ op: "pairCancel" });
      expect(wrapper.find("#qr").exists()).toBe(false);
      wrapper.unmount();
    });

    it("cannot pair while the gateway is not running", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: null });
      const wrapper = mountCard();
      await flushPromises();
      await byId(wrapper, "ios-step-3").get(".ios-step-head").trigger("click");
      expect(byId(wrapper, "ios-pair").attributes("disabled")).toBeDefined();
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
  });

  // ——— the form: test, import, services, relay ———

  describe("test connection: warnings that never block, and a hint that fits the service", () => {
    const testButton = (wrapper: Card) => byId(wrapper, "ios-test");
    const hint = (wrapper: Card) => wrapper.find(".ios-test-hint");

    beforeEach(() => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
    });

    it("tests the broker and the relay address of the form", async () => {
      iosTest.mockResolvedValue(testOk());
      const wrapper = mountCard();
      await flushPromises();
      await testButton(wrapper).trigger("click");
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
      await testButton(wrapper).trigger("click");
      await flushPromises();
      expect(wrapper.get(".result").classes()).toContain("err");
      expect(wrapper.get(".result").text()).toContain("the relay check failed");
      wrapper.unmount();
    });

    it("asks the gateway again right after a good test instead of waiting for the next poll", async () => {
      iosTest.mockResolvedValue(testOk());
      const wrapper = mountCard();
      await flushPromises();
      const before = iosOp.mock.calls.filter(([op]) => op.op === "state").length;
      await testButton(wrapper).trigger("click");
      await flushPromises();
      expect(iosOp.mock.calls.filter(([op]) => op.op === "state").length).toBeGreaterThan(before);
      wrapper.unmount();
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

  describe("the connection string of another Mac", () => {
    const fields = {
      host: "other.example.com",
      port: 8883,
      wsPort: 8084,
      username: "zzz999",
      password: "secret",
      relayUrl: "https://relay2.example.com",
    };

    beforeEach(() => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
    });

    it("is offered behind a button, and fills the form and saves it", async () => {
      iosConnectionImport.mockResolvedValue(fields);
      const wrapper = mountCard();
      await flushPromises();
      expect(wrapper.find(".ios-import").exists()).toBe(false);
      await byId(wrapper, "ios-import-open").trigger("click");
      await wrapper.get(".ios-import input").setValue("ahconn1.abc");
      await byId(wrapper, "ios-import").trigger("click");
      await flushPromises();
      expect(config.value.channels.ios.host).toBe("other.example.com");
      expect(config.value.channels.ios.password).toBe("secret");
      expect(persist).toHaveBeenCalled();
      expect(wrapper.text()).toContain("Connection settings filled in.");
      // Done: the field is put away again.
      expect(wrapper.find(".ios-import").exists()).toBe(false);
      wrapper.unmount();
    });

    it("keeps the field and says why when the string is not good", async () => {
      iosConnectionImport.mockRejectedValue("not a connection string");
      const wrapper = mountCard();
      await flushPromises();
      await byId(wrapper, "ios-import-open").trigger("click");
      await wrapper.get(".ios-import input").setValue("nonsense");
      await byId(wrapper, "ios-import").trigger("click");
      await flushPromises();
      expect(wrapper.find(".ios-import").exists()).toBe(true);
      expect(wrapper.text()).toContain("not a connection string");
      wrapper.unmount();
    });

    it("copies this Mac's connection string once the setup is finished", async () => {
      iosConnectionExport.mockResolvedValue("ahconn1.xyz");
      const wrapper = mountCard();
      await flushPromises();
      await byId(wrapper, "ios-export").trigger("click");
      await flushPromises();
      expect(writeText).toHaveBeenCalledWith("ahconn1.xyz");
      expect(wrapper.text()).toContain("Connection string copied");
      wrapper.unmount();
    });

    it("does not offer the export while the connection is still being set up", async () => {
      iosOp.mockResolvedValue({ kind: "state", state: null });
      const wrapper = mountCard();
      await flushPromises();
      expect(wrapper.find('[data-testid="ios-export"]').exists()).toBe(false);
      expect(byId(wrapper, "ios-import-open").text()).toBe("Import from another Mac…");
      wrapper.unmount();
    });
  });

  describe("the broker service: three neutral choices, then the steps of the one picked", () => {
    const tile = (wrapper: Card, id: string) => byId(wrapper, `ios-broker-${id}`);
    const active = (wrapper: Card) => wrapper.findAll(".ios-tile.active").map((t) => t.attributes("data-testid"));

    beforeEach(() => {
      // Step 1 stays open while the gateway is not running.
      iosOp.mockResolvedValue({ kind: "state", state: null });
    });

    it("lists the three services without recommending one", async () => {
      const wrapper = mountCard();
      await flushPromises();
      expect(wrapper.findAll(".ios-tile").map((t) => t.text())).toEqual([
        "EMQX CloudChina region · free allowance",
        "flespiEU region · free plan",
        "Self-hosted or othere.g. Mosquitto",
      ]);
      expect(wrapper.get(".ios-lead").text()).toBe(
        "The broker relays messages between your Mac and phone. Choose a provider:",
      );
      wrapper.unmount();
    });

    it("selects nothing and asks for no connection details until there is a host or a pick", async () => {
      config.value.channels.ios.host = "";
      const wrapper = mountCard();
      await flushPromises();
      expect(active(wrapper)).toEqual([]);
      expect(wrapper.find('[data-testid="ios-host"]').exists()).toBe(false);
      expect(wrapper.find('[data-testid="ios-guide-steps"]').exists()).toBe(false);
      wrapper.unmount();
    });

    it("treats a host it does not know as self-hosted or another broker", async () => {
      const wrapper = mountCard();
      await flushPromises();
      expect(active(wrapper)).toEqual(["ios-broker-custom"]);
      expect(wrapper.findAll('[data-testid="ios-guide-steps"] > li')).toHaveLength(3);
      expect(wrapper.find('[data-testid="ios-guide-steps"]').element.tagName).toBe("UL");
      wrapper.unmount();
    });

    it("recognizes a service by its host name, with the note and the steps that matter for it", async () => {
      config.value.channels.ios.host = "abc123.ala.cn-hangzhou.emqxsl.cn";
      const wrapper = mountCard();
      await flushPromises();
      expect(active(wrapper)).toEqual(["ios-broker-emqx"]);
      expect(byId(wrapper, "ios-notice-emqx").text()).toBe(
        "Note: if the free allowance (1 million connection minutes and 1 GB of traffic per month) runs out, the minimum top-up may be ¥100. Keep an eye on traffic and try to stay within the free allowance.",
      );
      const steps = byId(wrapper, "ios-guide-steps");
      expect(steps.element.tagName).toBe("OL");
      expect(steps.findAll(":scope > li")).toHaveLength(4);
      // The step that leaks other people's topics when it is skipped says so, with a chip for each rule.
      expect(steps.findAll(":scope > li")[3]!.text()).toContain("you must set it up as follows");
      expect(steps.findAll(":scope > li > ul > li")).toHaveLength(2);
      expect(steps.get('[data-chip="all"]').text()).toBe("#");
      wrapper.unmount();
    });

    it("fills the host and ports of a service on a pick, and keeps what was typed otherwise", async () => {
      const wrapper = mountCard();
      await flushPromises();
      const ios = config.value.channels.ios;

      await tile(wrapper, "flespi").trigger("click");
      await flushPromises();
      expect([ios.host, ios.port, ios.wsPort]).toEqual(["mqtt.flespi.io", 8883, 443]);
      expect(persist).toHaveBeenCalled();
      expect(active(wrapper)).toEqual(["ios-broker-flespi"]);
      expect(byId(wrapper, "ios-guide-steps").text()).toContain("Open flespi");

      // EMQX gives every account its own host: only the ports are filled in.
      await tile(wrapper, "emqx").trigger("click");
      await flushPromises();
      expect([ios.host, ios.port, ios.wsPort]).toEqual(["mqtt.flespi.io", 8883, 8084]);

      // "Self-hosted" fills nothing and does not save anything.
      persist.mockClear();
      ios.port = 1884;
      await tile(wrapper, "custom").trigger("click");
      await flushPromises();
      expect([ios.host, ios.port, ios.wsPort]).toEqual(["mqtt.flespi.io", 1884, 8084]);
      expect(persist).not.toHaveBeenCalled();
      expect(active(wrapper)).toEqual(["ios-broker-custom"]);
      wrapper.unmount();
    });

    it("lets a pick stand while the host is still being typed, and a pasted connection string decide again", async () => {
      const wrapper = mountCard();
      await flushPromises();
      await tile(wrapper, "emqx").trigger("click");
      await flushPromises();
      expect(active(wrapper)).toEqual(["ios-broker-emqx"]);

      iosConnectionImport.mockResolvedValue({
        host: "mqtt.flespi.io",
        port: 8883,
        wsPort: 443,
        username: "123456",
        password: "token",
        relayUrl: "https://relay.example.com",
      });
      await byId(wrapper, "ios-import-open").trigger("click");
      await wrapper.get(".ios-import input").setValue("ahconn1.abc");
      await byId(wrapper, "ios-import").trigger("click");
      await flushPromises();
      expect(active(wrapper)).toEqual(["ios-broker-flespi"]);
      wrapper.unmount();
    });

    it("hands the person the strings to type at the service, and only copies on a click", async () => {
      config.value.channels.ios.host = "abc123.ala.cn-hangzhou.emqxsl.cn";
      config.value.channels.ios.username = "";
      const wrapper = mountCard();
      await flushPromises();
      const acl = () => byId(wrapper, "ios-guide-steps").get('[data-chip="acl"]');

      // Nothing to copy for the account's topic until its user name is typed.
      expect(acl().text()).toBe("ah/<username>/#");
      expect(acl().attributes("disabled")).toBeDefined();

      await wrapper.get('[data-testid="ios-username"]').setValue("ah7");
      await flushPromises();
      expect(acl().text()).toBe("ah/ah7/#");
      expect(acl().attributes("disabled")).toBeUndefined();
      expect(writeText).not.toHaveBeenCalled();
      await acl().trigger("click");
      await flushPromises();
      expect(writeText).toHaveBeenCalledWith("ah/ah7/#");
      expect(acl().text()).toContain("Copied");

      await byId(wrapper, "ios-guide-steps").get('[data-chip="all"]').trigger("click");
      await flushPromises();
      expect(writeText).toHaveBeenLastCalledWith("#");
      wrapper.unmount();
    });

    it("names the account's topic after what flespi calls the account", async () => {
      config.value.channels.ios.host = "mqtt.flespi.io";
      config.value.channels.ios.username = "";
      const wrapper = mountCard();
      await flushPromises();
      expect(byId(wrapper, "ios-notice-flespi").classes()).toContain("info");
      expect(byId(wrapper, "ios-guide-steps").get('[data-chip="wild"]').text()).toBe("ah/#");
      expect(byId(wrapper, "ios-guide-steps").get('[data-chip="acl"]').text()).toBe("ah/<number>/#");
      wrapper.unmount();
    });

    it("opens the service's console and the wiki sections in the reader's language", async () => {
      config.value.channels.ios.host = "abc123.ala.cn-hangzhou.emqxsl.cn";
      const wrapper = mountCard();
      await flushPromises();
      await byId(wrapper, "ios-guide-steps").get('[data-link="console"]').trigger("click");
      expect(openPath).toHaveBeenLastCalledWith("https://cloud.emqx.com/");

      await tile(wrapper, "custom").trigger("click");
      await flushPromises();
      await wrapper.get('.ios-lead [data-link="suite"]').trigger("click");
      expect(openPath).toHaveBeenLastCalledWith(
        "https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-broker.en.md#verify-a-broker-yourself",
      );

      i18n.global.locale.value = "zh";
      await flushPromises();
      await wrapper.get('.ios-lead [data-link="mosquitto"]').trigger("click");
      expect(openPath).toHaveBeenLastCalledWith(
        "https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-broker.md#路径三自建-mosquitto",
      );
      wrapper.unmount();
    });
  });

  describe("the header", () => {
    beforeEach(() => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
    });

    it("turns the channel off and on, and saves it", async () => {
      const wrapper = mountCard();
      await flushPromises();
      await byId(wrapper, "channel-toggle-ios").setValue(false);
      await flushPromises();
      expect(config.value.channels.ios.enabled).toBe(false);
      expect(persist).toHaveBeenCalled();
      expect(wrapper.find('[data-testid="ios-ready"]').exists()).toBe(false);
      expect(wrapper.find('[data-testid="ios-start"]').exists()).toBe(true);
      wrapper.unmount();
    });

    it("opens the setup guide in the reader's language", async () => {
      const wrapper = mountCard();
      await flushPromises();
      await wrapper.get(".guide-link").trigger("click");
      expect(openPath).toHaveBeenLastCalledWith(
        "https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-setup.en.md",
      );
      i18n.global.locale.value = "zh";
      await flushPromises();
      await wrapper.get(".guide-link").trigger("click");
      expect(openPath).toHaveBeenLastCalledWith("https://github.com/Naituw/AskHuman/blob/main/docs/wiki/ios-setup.md");
      wrapper.unmount();
    });

    it("can be the target of a link from elsewhere in the settings", async () => {
      const wrapper = mountCard();
      await flushPromises();
      expect(wrapper.find("#channel-ios").exists()).toBe(true);
      wrapper.unmount();
    });
  });

  describe("the push relay: the public one built into the app, or one of your own", () => {
    const PUBLIC = "https://public-relay.example";
    const relaySelect = (wrapper: Card) => wrapper.find<HTMLSelectElement>(".ios-relay-mode");
    const relayInput = (wrapper: Card) => wrapper.find<HTMLInputElement>(".ios-relay input");

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
      expect(note).toContain("only forwards pushes");
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
      // An empty address still means the public relay: the card says so rather than staying silent.
      expect(wrapper.get(".ios-relay-pending").text()).toContain("the public relay is used until you enter one");

      await relayInput(wrapper).setValue("https://mine.example.com");
      await relayInput(wrapper).trigger("change");
      expect(config.value.channels.ios.relayUrl).toBe("https://mine.example.com");
      expect(persist).toHaveBeenCalled();
      expect(wrapper.find(".ios-relay-pending").exists()).toBe(false);

      // Clearing the field while editing does not yank the field away, and the hint is back.
      await relayInput(wrapper).setValue("");
      expect(relaySelect(wrapper).element.value).toBe("own");
      expect(wrapper.find(".ios-relay-pending").exists()).toBe(true);
      wrapper.unmount();
    });

    it("says nothing about a missing address when the public relay is chosen or this build has none", async () => {
      iosDefaultRelayUrl.mockResolvedValue(PUBLIC);
      config.value.channels.ios.relayUrl = "";
      const withPublic = mountCard();
      await flushPromises();
      expect(relaySelect(withPublic).element.value).toBe("public");
      expect(withPublic.find(".ios-relay-pending").exists()).toBe(false);
      withPublic.unmount();

      iosDefaultRelayUrl.mockResolvedValue("");
      config.value.channels.ios.relayUrl = "";
      const withoutPublic = mountCard();
      await flushPromises();
      expect(withoutPublic.find(".ios-relay-pending").exists()).toBe(false);
      withoutPublic.unmount();
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
      await byId(wrapper, "ios-import-open").trigger("click");
      await wrapper.get(".ios-import input").setValue("ahconn1.abc");
      await byId(wrapper, "ios-import").trigger("click");
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
      await byId(wrapper, "ios-import-open").trigger("click");
      await wrapper.get(".ios-import input").setValue("ahconn1.def");
      await byId(wrapper, "ios-import").trigger("click");
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
      await byId(wrapper, "ios-test").trigger("click");
      await flushPromises();
      expect(iosTest).toHaveBeenCalledWith(expect.objectContaining({ relayUrl: "" }));
      wrapper.unmount();
    });
  });

  describe("the app's New task needs the Agent-task switch, which is in the card", () => {
    beforeEach(() => {
      iosOp.mockResolvedValue({ kind: "state", state: gateway() });
    });

    it("has the switch among the sections of a finished setup, between the timing and the connection", async () => {
      const wrapper = mountCard();
      await flushPromises();
      const row = wrapper.get('[data-testid="agent-tasks-row"]');
      expect(row.text()).toContain("Allow creating Agent tasks from phone or IM");
      expect(row.text()).toContain("One switch for all channels");
      const html = wrapper.html();
      expect(html.indexOf('data-testid="agent-tasks-row"')).toBeGreaterThan(html.indexOf("ios-alert-after"));
      expect(html.indexOf('data-testid="agent-tasks-row"')).toBeLessThan(html.indexOf('data-testid="ios-conn"'));
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
