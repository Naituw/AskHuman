// The wizard's optional checks as state machines (docs/specs/onboarding.md D11): idle → running →
// done, cancel at any point, results that arrive early or late, and checks that never get in the
// way of each other.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SetupProbeResult } from "../../lib/types";
import { createProbes } from "./useProbe";

const ipc = vi.hoisted(() => ({ setupProbeStart: vi.fn(), setupProbeCancel: vi.fn() }));
vi.mock("../../lib/ipc", () => ipc);

function result(over: Partial<SetupProbeResult> & Pick<SetupProbeResult, "probeId" | "kind">): SetupProbeResult {
  return { outcome: "ok", ...over };
}

/** A start the test finishes by hand, to put a result before or after the backend's answer. */
function deferredStart() {
  let resolve!: (id: string) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<string>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("the wizard's optional checks", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    ipc.setupProbeCancel.mockResolvedValue(undefined);
  });

  it("starts idle, with nothing to report", () => {
    const { states } = createProbes();
    for (const kind of ["popup", "channel", "agent"] as const) {
      expect(states[kind]).toEqual({ phase: "idle", channel: null, outcome: null, agentKind: null, error: null });
    }
  });

  describe("a check that is answered", () => {
    it("runs, then reports success", async () => {
      ipc.setupProbeStart.mockResolvedValue("p1");
      const probes = createProbes();
      await probes.start("popup");
      expect(ipc.setupProbeStart).toHaveBeenCalledWith("popup", undefined);
      expect(probes.states.popup.phase).toBe("running");

      probes.handleResult(result({ probeId: "p1", kind: "popup" }));
      expect(probes.states.popup).toMatchObject({ phase: "done", outcome: "ok", error: null });
    });

    it.each(["timeout", "cancelled", "failed"] as const)("reports %s as it is, without an error", async (outcome) => {
      ipc.setupProbeStart.mockResolvedValue("p1");
      const probes = createProbes();
      await probes.start("popup");
      probes.handleResult(result({ probeId: "p1", kind: "popup", outcome }));
      expect(probes.states.popup).toMatchObject({ phase: "done", outcome, error: null });
    });

    it("tells which Agent asked", async () => {
      ipc.setupProbeStart.mockResolvedValue("a1");
      const probes = createProbes();
      await probes.start("agent");
      probes.handleResult(result({ probeId: "a1", kind: "agent", agentKind: "claude" }));
      expect(probes.states.agent).toMatchObject({ phase: "done", outcome: "ok", agentKind: "claude" });
    });

    it("remembers which channel a round trip ran for", async () => {
      ipc.setupProbeStart.mockResolvedValue("c1");
      const probes = createProbes();
      await probes.start("channel", "slack");
      expect(ipc.setupProbeStart).toHaveBeenCalledWith("channel", "slack");
      expect(probes.states.channel).toMatchObject({ phase: "running", channel: "slack" });
      probes.handleResult(result({ probeId: "c1", kind: "channel", channel: "slack" }));
      expect(probes.states.channel).toMatchObject({ phase: "done", channel: "slack", outcome: "ok" });
    });

    it("can be run again after it ended", async () => {
      ipc.setupProbeStart.mockResolvedValueOnce("p1").mockResolvedValueOnce("p2");
      const probes = createProbes();
      await probes.start("popup");
      probes.handleResult(result({ probeId: "p1", kind: "popup", outcome: "timeout" }));
      await probes.start("popup");
      expect(probes.states.popup).toMatchObject({ phase: "running", outcome: null });
      probes.handleResult(result({ probeId: "p2", kind: "popup" }));
      expect(probes.states.popup).toMatchObject({ phase: "done", outcome: "ok" });
    });
  });

  describe("timing", () => {
    it("takes a result that arrives before the backend has returned the id", async () => {
      const start = deferredStart();
      ipc.setupProbeStart.mockReturnValue(start.promise);
      const probes = createProbes();
      const started = probes.start("popup");
      // The check failed so fast that its result got here first.
      probes.handleResult(result({ probeId: "p1", kind: "popup", outcome: "failed" }));
      expect(probes.states.popup.phase).toBe("running");
      start.resolve("p1");
      await started;
      expect(probes.states.popup).toMatchObject({ phase: "done", outcome: "failed" });
    });

    it("ignores a result nobody waits for", async () => {
      ipc.setupProbeStart.mockResolvedValue("p1");
      const probes = createProbes();
      // Nothing is running.
      probes.handleResult(result({ probeId: "ghost", kind: "popup" }));
      expect(probes.states.popup.phase).toBe("idle");
      // Something is, but under another id.
      await probes.start("popup");
      probes.handleResult(result({ probeId: "ghost", kind: "popup" }));
      expect(probes.states.popup.phase).toBe("running");
      // And a result of another kind never settles this one.
      probes.handleResult(result({ probeId: "p1", kind: "agent" }));
      expect(probes.states.popup.phase).toBe("running");
    });

    it("does not let a result that arrived early for an old check settle a newer one", async () => {
      const first = deferredStart();
      ipc.setupProbeStart.mockReturnValueOnce(first.promise).mockResolvedValueOnce("p2");
      const probes = createProbes();
      const started = probes.start("popup");
      probes.handleResult(result({ probeId: "p1", kind: "popup" }));
      probes.cancel("popup");
      first.resolve("p1");
      await started;
      await probes.start("popup");
      expect(probes.states.popup.phase).toBe("running");
    });
  });

  describe("a check that cannot start", () => {
    it("fails with the reason, and can be tried again", async () => {
      ipc.setupProbeStart.mockRejectedValueOnce(new Error("daemon is restarting")).mockResolvedValueOnce("p2");
      const probes = createProbes();
      await probes.start("popup");
      expect(probes.states.popup).toMatchObject({ phase: "done", outcome: "failed" });
      expect(probes.states.popup.error).toContain("daemon is restarting");

      await probes.start("popup");
      expect(probes.states.popup).toMatchObject({ phase: "running", error: null });
    });

    it("does not report a start that was cancelled meanwhile", async () => {
      const start = deferredStart();
      ipc.setupProbeStart.mockReturnValue(start.promise);
      const probes = createProbes();
      const started = probes.start("popup");
      probes.cancel("popup");
      start.reject(new Error("too late"));
      await started;
      expect(probes.states.popup.phase).toBe("idle");
    });
  });

  describe("cancelling", () => {
    it("stops the backend check and returns to idle, and a late result changes nothing", async () => {
      ipc.setupProbeStart.mockResolvedValue("p1");
      const probes = createProbes();
      await probes.start("popup");
      probes.cancel("popup");
      expect(ipc.setupProbeCancel).toHaveBeenCalledWith("p1");
      expect(probes.states.popup.phase).toBe("idle");
      probes.handleResult(result({ probeId: "p1", kind: "popup", outcome: "cancelled" }));
      expect(probes.states.popup.phase).toBe("idle");
    });

    it("stops a check whose id has not arrived yet as soon as it does", async () => {
      const start = deferredStart();
      ipc.setupProbeStart.mockReturnValue(start.promise);
      const probes = createProbes();
      const started = probes.start("agent");
      probes.cancel("agent");
      expect(ipc.setupProbeCancel).not.toHaveBeenCalled();
      start.resolve("a1");
      await started;
      expect(ipc.setupProbeCancel).toHaveBeenCalledWith("a1");
      expect(probes.states.agent.phase).toBe("idle");
    });

    it("does nothing for a check that is not running", async () => {
      const probes = createProbes();
      probes.cancel("popup");
      expect(ipc.setupProbeCancel).not.toHaveBeenCalled();
      expect(probes.states.popup.phase).toBe("idle");
    });

    it("survives a backend that refuses to cancel", async () => {
      ipc.setupProbeStart.mockResolvedValue("p1");
      ipc.setupProbeCancel.mockRejectedValue(new Error("gone"));
      const probes = createProbes();
      await probes.start("popup");
      probes.cancel("popup");
      await tick();
      expect(probes.states.popup.phase).toBe("idle");
    });
  });

  describe("several checks", () => {
    it("run independently of each other", async () => {
      ipc.setupProbeStart.mockResolvedValueOnce("p1").mockResolvedValueOnce("a1");
      const probes = createProbes();
      await probes.start("popup");
      await probes.start("agent");
      probes.handleResult(result({ probeId: "p1", kind: "popup", outcome: "failed" }));
      expect(probes.states.popup).toMatchObject({ phase: "done", outcome: "failed" });
      expect(probes.states.agent.phase).toBe("running");
      probes.cancel("agent");
      expect(probes.states.popup.phase).toBe("done");
    });

    it("start only once while one is running", async () => {
      ipc.setupProbeStart.mockResolvedValue("p1");
      const probes = createProbes();
      await probes.start("popup");
      await probes.start("popup");
      expect(ipc.setupProbeStart).toHaveBeenCalledTimes(1);
    });

    it("let a round trip for another channel take over from the one running", async () => {
      ipc.setupProbeStart.mockResolvedValueOnce("c1").mockResolvedValueOnce("c2");
      const probes = createProbes();
      await probes.start("channel", "slack");
      await probes.start("channel", "telegram");
      expect(ipc.setupProbeCancel).toHaveBeenCalledWith("c1");
      expect(probes.states.channel).toMatchObject({ phase: "running", channel: "telegram" });
      // The first one's end is of no interest any more.
      probes.handleResult(result({ probeId: "c1", kind: "channel", channel: "slack", outcome: "cancelled" }));
      expect(probes.states.channel).toMatchObject({ phase: "running", channel: "telegram" });
      probes.handleResult(result({ probeId: "c2", kind: "channel", channel: "telegram" }));
      expect(probes.states.channel).toMatchObject({ phase: "done", channel: "telegram", outcome: "ok" });
    });

    it("keep a round trip for the same channel running when asked again", async () => {
      ipc.setupProbeStart.mockResolvedValue("c1");
      const probes = createProbes();
      await probes.start("channel", "slack");
      await probes.start("channel", "slack");
      expect(ipc.setupProbeStart).toHaveBeenCalledTimes(1);
    });
  });

  describe("releasing", () => {
    it("stops a running check and forgets a finished one", async () => {
      ipc.setupProbeStart.mockResolvedValueOnce("c1").mockResolvedValueOnce("p1");
      const probes = createProbes();
      await probes.start("channel", "slack");
      await probes.start("popup");
      probes.handleResult(result({ probeId: "p1", kind: "popup" }));

      probes.release("channel");
      expect(ipc.setupProbeCancel).toHaveBeenCalledWith("c1");
      expect(probes.states.channel).toMatchObject({ phase: "idle", channel: null });
      probes.release("popup");
      expect(probes.states.popup.phase).toBe("idle");
      expect(ipc.setupProbeCancel).toHaveBeenCalledTimes(1);
    });

    it("stops every running check at once when the window goes away", async () => {
      ipc.setupProbeStart
        .mockResolvedValueOnce("p1")
        .mockResolvedValueOnce("a1")
        .mockResolvedValueOnce("c1");
      const probes = createProbes();
      await probes.start("popup");
      await probes.start("agent");
      await probes.start("channel", "feishu");
      probes.releaseAll();
      expect(ipc.setupProbeCancel.mock.calls.map(([id]) => id).sort()).toEqual(["a1", "c1", "p1"]);
      for (const kind of ["popup", "channel", "agent"] as const) {
        expect(probes.states[kind].phase).toBe("idle");
      }
    });
  });
});
