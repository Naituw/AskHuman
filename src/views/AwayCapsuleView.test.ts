import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent, nextTick, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../i18n";
import type { CapsuleState, CapsuleView } from "../lib/types";
import AwayCapsuleView from "./AwayCapsuleView.vue";
import {
  DRAG_THRESHOLD_PX,
  HOVER_CLOSE_MS,
  HOVER_OPEN_MS,
  WELCOME_TIMEOUT_MS,
  endSteps,
  useCapsuleExpansion,
  useWindowDrag,
} from "./awayCapsule";

const ipc = vi.hoisted(() => ({
  awayApply: vi.fn(async () => ({ sessions: [], escalatedWaiting: 0 })),
  capsuleDismissWelcome: vi.fn(async () => {}),
  capsuleFocusWaiting: vi.fn(async () => {}),
  capsuleInit: vi.fn(),
  capsuleResize: vi.fn(async () => {}),
  openHistoryChannel: vi.fn(async () => {}),
}));
const events = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
}));
const startDragging = vi.hoisted(() => vi.fn(async () => {}));

vi.mock("../lib/ipc", () => ipc);
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    events.handlers.set(name, handler);
    return () => events.handlers.delete(name);
  }),
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ startDragging }) }));

const SINCE = Date.UTC(2026, 10, 5, 4, 0, 0);

function capsuleView(overrides: Partial<CapsuleView> = {}): CapsuleView {
  return {
    scope: "all",
    sinceMs: SINCE,
    sessions: 0,
    sessionKeys: [],
    replies: 6,
    waiting: 0,
    rows: [
      { kind: "codex", title: "Fix login page", replies: 4 },
      { kind: "claude", title: "Refactor session storage", replies: 2 },
    ],
    more: 0,
    note: null,
    ...overrides,
  };
}

function push(state: CapsuleState) {
  events.handlers.get("capsule-state")?.({ payload: state });
}

describe("capsule expansion", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  function setup() {
    const welcome = ref(false);
    let api!: ReturnType<typeof useCapsuleExpansion>;
    mount(
      defineComponent({
        setup() {
          api = useCapsuleExpansion(welcome);
          return () => null;
        },
      }),
    );
    return { welcome, api };
  }

  it("opens after the mouse has rested on it and closes a moment after it left", () => {
    const { api } = setup();
    api.enter();
    vi.advanceTimersByTime(HOVER_OPEN_MS - 1);
    expect(api.expanded.value).toBe(false);
    vi.advanceTimersByTime(1);
    expect(api.expanded.value).toBe(true);
    api.leave();
    vi.advanceTimersByTime(HOVER_CLOSE_MS - 1);
    expect(api.expanded.value).toBe(true);
    vi.advanceTimersByTime(1);
    expect(api.expanded.value).toBe(false);
  });

  it("does not open for a mouse that only passes over", () => {
    const { api } = setup();
    api.enter();
    vi.advanceTimersByTime(HOVER_OPEN_MS - 20);
    api.leave();
    vi.advanceTimersByTime(1000);
    expect(api.expanded.value).toBe(false);
  });

  it("stays open when the mouse comes back within the closing delay", () => {
    const { api } = setup();
    api.enter();
    vi.advanceTimersByTime(HOVER_OPEN_MS);
    api.leave();
    vi.advanceTimersByTime(HOVER_CLOSE_MS - 50);
    api.enter();
    vi.advanceTimersByTime(1000);
    expect(api.expanded.value).toBe(true);
  });

  it("a click pins the card until it is clicked again and the mouse has left", () => {
    const { api } = setup();
    api.toggle();
    expect(api.expanded.value).toBe(true);
    api.leave();
    vi.advanceTimersByTime(HOVER_CLOSE_MS * 2);
    expect(api.expanded.value).toBe(true);
    api.toggle();
    api.leave();
    vi.advanceTimersByTime(HOVER_CLOSE_MS);
    expect(api.expanded.value).toBe(false);
  });

  it("welcome back keeps the card open whatever the mouse does", () => {
    const { api, welcome } = setup();
    welcome.value = true;
    expect(api.expanded.value).toBe(true);
    api.leave();
    vi.advanceTimersByTime(5000);
    expect(api.expanded.value).toBe(true);
    welcome.value = false;
    expect(api.expanded.value).toBe(false);
  });
});

describe("window drag", () => {
  function pointer(x: number, y: number, buttons = 1): PointerEvent {
    return { button: 0, buttons, screenX: x, screenY: y } as PointerEvent;
  }

  it("a press that does not move is a click", () => {
    const drag = vi.fn();
    const click = vi.fn();
    const handlers = useWindowDrag(drag, click);
    handlers.down(pointer(10, 10));
    handlers.move(pointer(10 + DRAG_THRESHOLD_PX - 1, 10));
    handlers.up();
    expect(click).toHaveBeenCalledTimes(1);
    expect(drag).not.toHaveBeenCalled();
  });

  it("moving past the threshold starts one native drag and is not a click", () => {
    const drag = vi.fn();
    const click = vi.fn();
    const handlers = useWindowDrag(drag, click);
    handlers.down(pointer(10, 10));
    handlers.move(pointer(10 + DRAG_THRESHOLD_PX, 10));
    handlers.move(pointer(40, 30));
    handlers.up();
    expect(drag).toHaveBeenCalledTimes(1);
    expect(click).not.toHaveBeenCalled();
  });

  it("ignores moves with no button held and other buttons", () => {
    const drag = vi.fn();
    const click = vi.fn();
    const handlers = useWindowDrag(drag, click);
    handlers.move(pointer(100, 100, 0));
    handlers.down({ ...pointer(0, 0), button: 2 } as PointerEvent);
    handlers.up();
    expect(drag).not.toHaveBeenCalled();
    expect(click).not.toHaveBeenCalled();
  });
});

describe("ending away mode from the card", () => {
  it("ends the global switch, or puts each away session back to following it", () => {
    expect(endSteps(capsuleView())).toEqual([{ op: "end" }]);
    expect(
      endSteps(capsuleView({ scope: "sessions", sessionKeys: ["sid:a", "mcp:b"], sessions: 2 })),
    ).toEqual([
      { op: "session", sessionKey: "sid:a", setting: "follow" },
      { op: "session", sessionKey: "mcp:b", setting: "follow" },
    ]);
  });
});

describe("AwayCapsuleView", () => {
  beforeEach(() => {
    i18n.global.locale.value = "zh";
    events.handlers.clear();
    Object.values(ipc).forEach((fn) => "mockClear" in fn && fn.mockClear());
    startDragging.mockClear();
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe() {}
        disconnect() {}
        unobserve() {}
      },
    );
    vi.spyOn(Date, "now").mockReturnValue(SINCE + 83 * 60_000);
    ipc.capsuleInit.mockResolvedValue({
      state: { view: capsuleView(), welcome: false },
      theme: "light",
      lang: "zh",
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.documentElement.classList.remove("capsule-window", "theme-light", "theme-dark");
  });

  async function open(state?: CapsuleState) {
    if (state) ipc.capsuleInit.mockResolvedValue({ state, theme: "light", lang: "zh" });
    const wrapper = mount(AwayCapsuleView, { global: { plugins: [i18n] } });
    await flushPromises();
    return wrapper;
  }

  it("shows the capsule with how long the user has been away", async () => {
    const wrapper = await open();
    expect(wrapper.find('[data-testid="capsule-pill"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("离开模式");
    expect(wrapper.get('[data-testid="capsule-elapsed"]').text()).toBe("1 小时 23 分");
    expect(wrapper.find('[data-testid="capsule-badge"]').exists()).toBe(false);
    expect(document.documentElement.classList.contains("capsule-window")).toBe(true);
    expect(document.documentElement.classList.contains("theme-light")).toBe(true);
  });

  it("counts the questions that wait for an answer", async () => {
    const wrapper = await open({ view: capsuleView({ waiting: 2 }), welcome: false });
    expect(wrapper.get('[data-testid="capsule-badge"]').text()).toBe("2");
  });

  it("says how many sessions when only single sessions are away", async () => {
    const wrapper = await open({
      view: capsuleView({ scope: "sessions", sessions: 1, sessionKeys: ["sid:a"] }),
      welcome: false,
    });
    expect(wrapper.get('[data-testid="capsule-elapsed"]').text()).toBe("1 个会话");
  });

  it("grows into the card on a click and lists the Agents with their replies", async () => {
    const wrapper = await open({
      view: capsuleView({ waiting: 1, note: "别动生产环境" }),
      welcome: false,
    });
    const pill = wrapper.get('[data-testid="capsule-pill"]');
    await pill.trigger("pointerdown");
    await pill.trigger("pointerup");
    expect(wrapper.find('[data-testid="capsule-card"]').exists()).toBe(true);
    expect(wrapper.get('[data-testid="capsule-title"]').text()).toBe("离开模式");
    expect(wrapper.get('[data-testid="capsule-sub"]').text()).toMatch(/^所有 Agent · \d\d:\d\d 开始$/);
    expect(wrapper.get('[data-testid="capsule-replies"]').text()).toBe("期间自动回复 6 次");
    expect(wrapper.get('[data-testid="capsule-note"]').text()).toBe("备注：别动生产环境");
    const rows = wrapper
      .findAll(".cap-row")
      .map((r) => r.findAll("span").map((c) => c.text()));
    expect(rows).toEqual([
      ["Codex", "Fix login page", "4 次"],
      ["Claude Code", "Refactor session storage", "2 次"],
    ]);
    expect(wrapper.get('[data-testid="capsule-waiting"]').text()).toContain("1 个提问在等你处理");
  });

  it("the card's buttons end away mode, open the record and bring the waiting question forward", async () => {
    const wrapper = await open({ view: capsuleView({ waiting: 1 }), welcome: false });
    const pill = wrapper.get('[data-testid="capsule-pill"]');
    await pill.trigger("pointerdown");
    await pill.trigger("pointerup");
    await wrapper.get('[data-testid="capsule-view-waiting"]').trigger("click");
    expect(ipc.capsuleFocusWaiting).toHaveBeenCalledTimes(1);
    await wrapper.get('[data-testid="capsule-record"]').trigger("click");
    expect(ipc.openHistoryChannel).toHaveBeenCalledWith("away");
    await wrapper.get('[data-testid="capsule-end"]').trigger("click");
    await flushPromises();
    expect(ipc.awayApply).toHaveBeenCalledWith({ op: "end" });
  });

  it("ending a session-scoped capsule sets each away session back to following the global switch", async () => {
    const wrapper = await open({
      view: capsuleView({ scope: "sessions", sessions: 2, sessionKeys: ["sid:a", "sid:b"] }),
      welcome: false,
    });
    const pill = wrapper.get('[data-testid="capsule-pill"]');
    await pill.trigger("pointerdown");
    await pill.trigger("pointerup");
    await wrapper.get('[data-testid="capsule-end"]').trigger("click");
    await flushPromises();
    expect(ipc.awayApply.mock.calls).toEqual([
      [{ op: "session", sessionKey: "sid:a", setting: "follow" }],
      [{ op: "session", sessionKey: "sid:b", setting: "follow" }],
    ]);
  });

  it("welcome back opens the card with its own title and a keep-away button", async () => {
    const wrapper = await open({ view: capsuleView(), welcome: true });
    expect(wrapper.get('[data-testid="capsule-title"]').text()).toBe("欢迎回来");
    expect(wrapper.get('[data-testid="capsule-sub"]').text()).toBe(
      "你离开了 1 小时 23 分，离开模式仍在运行",
    );
    expect(wrapper.find('[data-testid="capsule-record"]').exists()).toBe(false);
    await wrapper.get('[data-testid="capsule-keep"]').trigger("click");
    expect(ipc.capsuleDismissWelcome).toHaveBeenCalledTimes(1);
    // The Host clears the flag and says so; the card goes back to being a capsule.
    push({ view: capsuleView(), welcome: false });
    await nextTick();
    expect(wrapper.find('[data-testid="capsule-pill"]').exists()).toBe(true);
  });

  it("an ignored welcome-back card hands itself back after a while", async () => {
    vi.useFakeTimers();
    try {
      const wrapper = await open();
      push({ view: capsuleView(), welcome: true });
      await nextTick();
      expect(wrapper.find('[data-testid="capsule-keep"]').exists()).toBe(true);
      vi.advanceTimersByTime(WELCOME_TIMEOUT_MS);
      expect(ipc.capsuleDismissWelcome).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("follows the state the Host pushes, and shows nothing once away mode is over", async () => {
    const wrapper = await open();
    push({ view: capsuleView({ replies: 9, waiting: 3 }), welcome: false });
    await nextTick();
    expect(wrapper.get('[data-testid="capsule-badge"]').text()).toBe("3");
    push({ view: null, welcome: false });
    await nextTick();
    expect(wrapper.find('[data-testid="capsule"]').exists()).toBe(false);
  });

  it("speaks English", async () => {
    i18n.global.locale.value = "en";
    ipc.capsuleInit.mockResolvedValue({
      state: { view: capsuleView({ waiting: 1 }), welcome: false },
      theme: "dark",
      lang: "en",
    });
    const wrapper = await open();
    expect(wrapper.text()).toContain("Away mode");
    const pill = wrapper.get('[data-testid="capsule-pill"]');
    await pill.trigger("pointerdown");
    await pill.trigger("pointerup");
    expect(wrapper.get('[data-testid="capsule-replies"]').text()).toBe("Auto-replies while away: 6");
    expect(wrapper.get('[data-testid="capsule-waiting"]').text()).toContain("1 question(s) waiting for you");
    expect(wrapper.get('[data-testid="capsule-end"]').text()).toBe("End away mode");
  });
});
