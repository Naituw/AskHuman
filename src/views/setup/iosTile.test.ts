// What the wizard's iOS tile shows follows the card in Settings (docs/specs/setup-ios-guidance.md §3.1):
// the same facts, the same `deriveIosSetup`, the card's tags cut a little finer.
import { describe, expect, it } from "vitest";
import { deriveIosSetup, type IosSetupInput } from "../settings/useIosSetup";
import { iosTileKind, iosTileStartsSetup, iosTileTone, type IosTileKind } from "./iosTile";

/** A broker that is fully filled in, on, and not asked yet; each case says what differs. */
function setup(over: Partial<IosSetupInput> = {}) {
  return deriveIosSetup({
    enabled: true,
    host: "broker.example.com",
    username: "abc123",
    passwordFilled: true,
    gateway: null,
    phones: 0,
    appInstalled: false,
    ...over,
  });
}

describe("iosTileKind", () => {
  it.each<[string, Partial<IosSetupInput>, IosTileKind]>([
    ["nothing set up", { enabled: false, host: "", username: "", passwordFilled: false }, "off"],
    ["on without the broker settings", { host: "", username: "", passwordFilled: false }, "incomplete"],
    ["on with a password missing", { passwordFilled: false }, "incomplete"],
    ["set up and turned off", { enabled: false }, "stopped"],
    ["set up, off, and a phone known", { enabled: false, phones: 1 }, "stopped"],
    ["on and complete, the daemon silent", {}, "configured"],
    ["on and complete, the broker not reached and no error", { gateway: { connected: false } }, "configured"],
    ["connected, no phone", { gateway: { connected: true } }, "await"],
    ["connected, one phone", { gateway: { connected: true }, phones: 1 }, "paired"],
    ["a phone, the broker down without an error", { gateway: { connected: false }, phones: 1 }, "paired"],
    ["a phone, the daemon silent", { phones: 2 }, "paired"],
    ["the broker refuses", { gateway: { connected: false, lastError: "refused" } }, "issue"],
    ["the broker refuses, with a phone paired", { gateway: { connected: false, lastError: "refused" }, phones: 1 }, "issue"],
  ])("%s → %s", (_name, over, kind) => {
    expect(iosTileKind(setup(over))).toBe(kind);
  });

  it("does not take the card's own 'I installed it' for a phone", () => {
    // The tile never knows it (the card's window does): the app counts only through a paired phone.
    expect(iosTileKind(setup({ gateway: { connected: true }, appInstalled: true }))).toBe("await");
  });
});

describe("iosTileTone", () => {
  it("is green for a working setup, orange where the person has to look, and plain otherwise", () => {
    const tones: Record<IosTileKind, "ok" | "warn" | "muted"> = {
      off: "muted",
      incomplete: "warn",
      stopped: "muted",
      issue: "warn",
      paired: "ok",
      await: "warn",
      configured: "muted",
    };
    for (const [kind, tone] of Object.entries(tones)) {
      expect(iosTileTone(kind as IosTileKind), kind).toBe(tone);
    }
  });
});

describe("iosTileStartsSetup", () => {
  it("starts the card only while it is off", () => {
    const starting = (["off", "incomplete", "stopped", "issue", "paired", "await", "configured"] as const).filter(
      iosTileStartsSetup,
    );
    expect(starting).toEqual(["off", "stopped"]);
  });
});
