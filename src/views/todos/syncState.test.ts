import { describe, expect, it } from "vitest";
import type { TodoSyncStatus } from "../../lib/types";
import { ageOf, describeSync, pollDelayMs } from "./syncState";

const NOW = 1_800_000_000_000;

function status(patch: Partial<TodoSyncStatus> = {}): TodoSyncStatus {
  return {
    supported: true,
    enabled: true,
    syncing: false,
    deviceName: "Studio",
    paused: null,
    lastPullOkMs: NOW - 30_000,
    lastPushOkMs: NOW - 90_000,
    unsynced: 0,
    todos: 8,
    history: 20,
    devices: [{ name: "Studio", lastSyncMs: NOW - 30_000, thisMac: true }],
    helper: { state: "running", version: "1", environment: "production", problem: null, installed: true },
    lastTick: null,
    newerState: false,
    ...patch,
  };
}

describe("describeSync", () => {
  it("says nothing where iCloud sync does not exist, and before the status arrived", () => {
    expect(describeSync(null, NOW)).toBeNull();
    expect(describeSync(status({ supported: false, enabled: false }), NOW)).toBeNull();
  });

  it("offers to turn sync on while it is off", () => {
    expect(describeSync(status({ enabled: false }), NOW)).toEqual({
      tone: "off",
      key: "localOnly",
      action: "enable",
    });
  });

  it("reports the age of the latest success when all is well", () => {
    expect(describeSync(status(), NOW)).toEqual({ tone: "ok", key: "synced", ageMs: 30_000 });
    // A push newer than the pull counts too.
    expect(describeSync(status({ lastPushOkMs: NOW - 2_000 }), NOW)).toMatchObject({ ageMs: 2_000 });
  });

  it("shows the first sync as in progress before anything completed", () => {
    expect(describeSync(status({ lastPullOkMs: 0, lastPushOkMs: 0 }), NOW)).toEqual({
      tone: "busy",
      key: "syncing",
    });
  });

  it("shows a running tick as syncing", () => {
    expect(describeSync(status({ syncing: true }), NOW)).toEqual({ tone: "busy", key: "syncing" });
  });

  it("counts the changes waiting while offline", () => {
    const line = describeSync(
      status({
        unsynced: 3,
        lastTick: { atMs: NOW, reason: "poll", outcome: "offline", fetched: 0, merged: 0, pushed: 0 },
      }),
      NOW,
    );
    expect(line).toEqual({ tone: "warn", key: "offline", count: 3 });
  });

  it("puts the decisions the user has to make first, loudest tone for the destructive ones", () => {
    expect(describeSync(status({ paused: "accountChanged" }), NOW)).toEqual({
      tone: "error",
      key: "accountChanged",
      action: "view",
    });
    expect(describeSync(status({ paused: "zoneMissing" }), NOW)).toMatchObject({
      tone: "error",
      key: "zoneMissing",
    });
    expect(describeSync(status({ paused: "quota" }), NOW)).toEqual({
      tone: "warn",
      key: "quota",
      action: "resume",
    });
    expect(describeSync(status({ paused: "notSignedIn" }), NOW)).toEqual({
      tone: "warn",
      key: "notSignedIn",
    });
    // Paused wins over a helper that is also down.
    const both = status({
      paused: "zoneMissing",
      helper: { state: "notRunning", version: null, environment: null, problem: null, installed: true },
    });
    expect(describeSync(both, NOW)?.key).toBe("zoneMissing");
  });

  it("flags a helper that is not installed, not running, or unable to reach iCloud", () => {
    const base = { version: null, environment: null, problem: null };
    expect(describeSync(status({ helper: { ...base, state: "notRunning", installed: false } }), NOW)).toEqual({
      tone: "error",
      key: "helperMissing",
      action: "repair",
    });
    expect(describeSync(status({ helper: { ...base, state: "notRunning", installed: true } }), NOW)).toEqual({
      tone: "error",
      key: "helperDown",
      action: "repair",
    });
    expect(describeSync(status({ helper: { ...base, state: "incompatible", installed: true } }), NOW)?.key).toBe(
      "helperDown",
    );
    expect(
      describeSync(
        status({
          helper: { state: "running", version: "1", environment: "production", problem: "missingEntitlement", installed: true },
        }),
        NOW,
      ),
    ).toEqual({ tone: "error", key: "helperProblem", detail: "missingEntitlement", action: "repair" });
  });

  it("warns about a sidecar from a newer build and shows hard tick errors", () => {
    expect(describeSync(status({ newerState: true }), NOW)).toEqual({ tone: "warn", key: "newerState" });
    expect(
      describeSync(
        status({
          lastTick: { atMs: NOW, reason: "poll", outcome: "error", message: "boom", fetched: 0, merged: 0, pushed: 0 },
        }),
        NOW,
      ),
    ).toEqual({ tone: "error", key: "error", detail: "boom" });
  });
});

describe("ageOf", () => {
  it("rounds down into the unit a person would say", () => {
    expect(ageOf(0)).toEqual({ unit: "justNow", n: 0 });
    expect(ageOf(4_999)).toEqual({ unit: "justNow", n: 0 });
    expect(ageOf(5_000)).toEqual({ unit: "seconds", n: 5 });
    expect(ageOf(59_000)).toEqual({ unit: "seconds", n: 59 });
    expect(ageOf(60_000)).toEqual({ unit: "minutes", n: 1 });
    expect(ageOf(3_599_000)).toEqual({ unit: "minutes", n: 59 });
    expect(ageOf(3_600_000)).toEqual({ unit: "hours", n: 1 });
    expect(ageOf(86_400_000 * 3)).toEqual({ unit: "days", n: 3 });
    expect(ageOf(-5)).toEqual({ unit: "justNow", n: 0 });
  });
});

describe("pollDelayMs", () => {
  it("does not poll when nothing can change by itself", () => {
    expect(pollDelayMs(null)).toBeNull();
    expect(pollDelayMs(status({ supported: false }))).toBeNull();
    expect(pollDelayMs(status({ enabled: false }))).toBeNull();
  });

  it("polls quickly only while a tick runs", () => {
    expect(pollDelayMs(status({ syncing: true }))).toBe(1500);
    expect(pollDelayMs(status())).toBe(15000);
  });
});
