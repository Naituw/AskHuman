// The wizard's Agent-page rules in isolation (docs/plans/onboarding.md §3.3).
import { describe, expect, it } from "vitest";
import {
  agentsPrimary,
  isBlocked,
  isIntegrated,
  isMainRow,
  isPending,
  modesOf,
  preselect,
  rowTags,
} from "./agentRows";
import { row } from "./setup.testkit";

describe("row classification", () => {
  it("an integrated row is any mode but none", () => {
    expect(isIntegrated(row({ mode: "none" }))).toBe(false);
    expect(isIntegrated(row({ mode: "cli" }))).toBe(true);
    expect(isIntegrated(row({ mode: "mcp" }))).toBe(true);
  });

  it("a row is blocked by an unreadable state or a too-old Agent", () => {
    expect(isBlocked(row())).toBe(false);
    expect(isBlocked(row({ statusError: "permission denied" }))).toBe(true);
    expect(isBlocked(row({ delivery: "unsupportedVersion" }))).toBe(true);
    expect(isBlocked(row({ delivery: "untrusted" }))).toBe(false);
  });

  it("the main area holds installed or integrated Agents; the rest wait under other Agents", () => {
    expect(isMainRow(row({ detected: true }))).toBe(true);
    expect(isMainRow(row({ detected: false, mode: "mcp" }))).toBe(true);
    expect(isMainRow(row({ detected: false, mode: "none" }))).toBe(false);
  });

  it("lists the modes an Agent supports, CLI first", () => {
    expect(modesOf(row({ supportsCli: true, supportsMcp: true }))).toEqual(["cli", "mcp"]);
    expect(modesOf(row({ supportsCli: false, supportsMcp: true }))).toEqual(["mcp"]);
    expect(modesOf(row({ supportsCli: true, supportsMcp: false }))).toEqual(["cli"]);
  });
});

describe("preselect", () => {
  it("keeps an integrated Agent's mode and leaves it ticked, even when it was not detected", () => {
    expect(preselect(row({ mode: "mcp", recommended: "cli", detected: false }))).toEqual({
      checked: true,
      mode: "mcp",
    });
  });

  it("ticks a detected, not yet integrated Agent with the recommended mode", () => {
    expect(preselect(row({ mode: "none", detected: true, recommended: "mcp" }))).toEqual({
      checked: true,
      mode: "mcp",
    });
  });

  it("leaves an undetected Agent unticked, with the recommended mode ready", () => {
    expect(preselect(row({ mode: "none", detected: false, recommended: "cli" }))).toEqual({
      checked: false,
      mode: "cli",
    });
  });

  it("never ticks a blocked row", () => {
    expect(preselect(row({ mode: "cli", statusError: "unreadable" })).checked).toBe(false);
    expect(preselect(row({ detected: true, delivery: "unsupportedVersion" })).checked).toBe(false);
  });
});

describe("isPending", () => {
  const cli = { checked: true, mode: "cli" as const };

  it("is false without a pick or an untick: unticking an integrated row means leave it alone", () => {
    expect(isPending(row({ mode: "none" }), undefined)).toBe(false);
    expect(isPending(row({ mode: "none" }), { checked: false, mode: "cli" })).toBe(false);
    expect(isPending(row({ mode: "cli" }), { checked: false, mode: "mcp" })).toBe(false);
  });

  it("is true for a ticked row that is not integrated yet", () => {
    expect(isPending(row({ mode: "none" }), cli)).toBe(true);
  });

  it("is true when the picked mode differs from what is on disk", () => {
    expect(isPending(row({ mode: "cli" }), { checked: true, mode: "mcp" })).toBe(true);
  });

  it("is false for an up-to-date row in the picked mode", () => {
    expect(isPending(row({ mode: "cli" }), cli)).toBe(false);
  });

  it("is true for a ticked row that needs an update, so it is applied by default", () => {
    expect(isPending(row({ mode: "cli", needsUpdate: true }), cli)).toBe(true);
  });

  it("is never true for a blocked row", () => {
    expect(isPending(row({ mode: "none", statusError: "x" }), cli)).toBe(false);
  });
});

describe("rowTags", () => {
  it("an unreadable row shows that and nothing else", () => {
    expect(rowTags(row({ mode: "cli", needsUpdate: true, statusError: "x" }))).toEqual([
      { key: "unreadable", tone: "warn" },
    ]);
  });

  it("names the integrated mode", () => {
    expect(rowTags(row({ mode: "cli" }))).toEqual([
      { key: "integrated", params: { mode: "CLI" }, tone: "ok" },
    ]);
    expect(rowTags(row({ mode: "mcp" }))[0].params).toEqual({ mode: "MCP" });
  });

  it("marks a detected, not yet integrated Agent; an undetected one gets no tag", () => {
    expect(rowTags(row({ mode: "none", detected: true }))).toEqual([{ key: "detected", tone: "muted" }]);
    expect(rowTags(row({ mode: "none", detected: false }))).toEqual([]);
  });

  it("adds the update, trust and version tags after the integration tag", () => {
    expect(
      rowTags(row({ mode: "cli", needsUpdate: true, delivery: "untrusted" })).map((t) => t.key),
    ).toEqual(["integrated", "needsUpdate", "untrusted"]);
    expect(rowTags(row({ delivery: "unsupportedVersion" })).map((t) => t.key)).toEqual([
      "detected",
      "unsupportedVersion",
    ]);
  });
});

describe("agentsPrimary", () => {
  it("applies when there is something to write, whatever happened before", () => {
    expect(agentsPrimary(true, false)).toBe("apply");
    expect(agentsPrimary(true, true)).toBe("apply");
  });

  it("continues after an apply and goes next when nothing changed", () => {
    expect(agentsPrimary(false, true)).toBe("continue");
    expect(agentsPrimary(false, false)).toBe("next");
  });
});
