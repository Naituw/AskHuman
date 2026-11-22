import { describe, expect, it } from "vitest";
import { deriveIosSetup, stepStatus, type IosSetupInput } from "./useIosSetup";

const filled: IosSetupInput = {
  enabled: true,
  host: "abc.ala.cn-hangzhou.emqxsl.cn",
  username: "ah1",
  passwordFilled: true,
  gateway: { connected: true },
  phones: 0,
  appInstalled: false,
};

function setup(over: Partial<IosSetupInput> = {}) {
  return deriveIosSetup({ ...filled, ...over });
}

describe("the status tag", () => {
  it("is unconfigured while nothing is filled in and the channel is off", () => {
    const s = setup({ enabled: false, host: "", username: "", passwordFilled: false, gateway: null });
    expect(s.tag).toBe("unconfigured");
    expect(s.tone).toBe("muted");
  });

  it("is off, not unconfigured, once the settings were filled in", () => {
    expect(setup({ enabled: false, gateway: null }).tag).toBe("configuredOff");
  });

  it("is incomplete when the channel is on but host, user name or password is missing", () => {
    for (const over of [{ host: "  " }, { username: "" }, { passwordFilled: false }]) {
      const s = setup({ gateway: null, ...over });
      expect(s.tag, JSON.stringify(over)).toBe("incomplete");
      expect(s.tone).toBe("warn");
    }
  });

  it("counts a stored password as filled in", () => {
    expect(setup({ gateway: null, passwordFilled: true }).configured).toBe(true);
  });

  it("is enabled, not an issue, while the broker is not connected yet and nothing went wrong", () => {
    expect(setup({ gateway: null }).tag).toBe("ready");
    expect(setup({ gateway: { connected: false } }).tag).toBe("ready");
    expect(setup({ gateway: { connected: false, lastError: "" } }).tag).toBe("ready");
  });

  it("is connected once the gateway is", () => {
    const s = setup();
    expect(s.tag).toBe("connected");
    expect(s.tone).toBe("ok");
  });

  it("is an issue when the gateway reports an error while not connected", () => {
    const s = setup({ gateway: { connected: false, lastError: "authentication failed" } });
    expect(s.tag).toBe("issue");
    expect(s.tone).toBe("warn");
  });

  it("does not call an error an issue while the settings are incomplete", () => {
    expect(setup({ username: "", gateway: { connected: false, lastError: "x" } }).tag).toBe("incomplete");
  });
});

describe("the steps", () => {
  it("starts at the connection while the broker is not connected", () => {
    const s = setup({ gateway: null });
    expect(s.done).toEqual([false, false, false]);
    expect(s.current).toBe(1);
    expect(s.ready).toBe(false);
  });

  it("moves on to the app once the broker is connected", () => {
    const s = setup();
    expect(s.done).toEqual([true, false, false]);
    expect(s.current).toBe(2);
  });

  it("takes the person's word that the app is installed, and moves on to pairing", () => {
    const s = setup({ appInstalled: true });
    expect(s.done).toEqual([true, true, false]);
    expect(s.current).toBe(3);
    expect(s.ready).toBe(false);
  });

  it("takes a paired phone as proof that the app is installed", () => {
    const s = setup({ phones: 1 });
    expect(s.done).toEqual([true, true, true]);
    expect(s.current).toBe(4);
    expect(s.ready).toBe(true);
  });

  it("is ready only with both a connection and a phone", () => {
    expect(setup({ phones: 2, gateway: { connected: false } }).ready).toBe(false);
    expect(setup({ phones: 0 }).ready).toBe(false);
    expect(setup({ phones: 1 }).ready).toBe(true);
  });

  it("keeps the earlier open step current while a later one is already done", () => {
    // A phone is paired, but the broker is down: the connection is the step that needs a look.
    const s = setup({ phones: 1, gateway: { connected: false } });
    expect(s.done).toEqual([false, true, true]);
    expect(s.current).toBe(1);
  });

  it("draws finished steps as done, the first open one as current and the rest as upcoming", () => {
    const s = setup({ appInstalled: true, gateway: null });
    expect([1, 2, 3].map((n) => stepStatus(s, n as 1 | 2 | 3))).toEqual(["current", "done", "upcoming"]);
    const ready = setup({ phones: 1 });
    expect([1, 2, 3].map((n) => stepStatus(ready, n as 1 | 2 | 3))).toEqual(["done", "done", "done"]);
  });
});
