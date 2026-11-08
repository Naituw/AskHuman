// The rules of an Agent card's header: which tags it wears, when it has details to fold, and when
// it opens by itself.
import { describe, expect, it } from "vitest";
import type { AgentId, AgentModeStatus } from "../../lib/types";
import {
  agentTags,
  hasDetails,
  hasUpdate,
  needsAction,
  versionProblem,
  type AgentCardInput,
} from "./agentCard";
import { modeStatus } from "./integration.testkit";

function input(
  status: Partial<AgentModeStatus> = {},
  extra: Partial<Omit<AgentCardInput, "status">> = {},
): AgentCardInput {
  return {
    id: "claude" as AgentId,
    status: modeStatus(status),
    readError: false,
    versionLoading: false,
    ...extra,
  };
}

const keys = (card: AgentCardInput) => agentTags(card).map((tag) => tag.key);

describe("hasDetails", () => {
  it("is true only for an integrated Agent whose state could be read", () => {
    expect(hasDetails(input({ mode: "cli" }))).toBe(true);
    expect(hasDetails(input({ mode: "mcp" }))).toBe(true);
    expect(hasDetails(input({ mode: "none" }))).toBe(false);
    expect(hasDetails(input({ mode: "cli" }, { readError: true }))).toBe(false);
  });
});

describe("hasUpdate", () => {
  it("is false for an Agent that needs nothing", () => {
    expect(hasUpdate(modeStatus())).toBe(false);
  });

  it.each<[string, Partial<AgentModeStatus>]>([
    ["the aggregate flag", { needsUpdate: true }],
    ["the rules artifact", { ruleNeedsUpdate: true }],
    ["the hook artifact", { hookNeedsUpdate: true }],
    ["the MCP artifact", { mcpNeedsUpdate: true }],
    ["the permission row", { permissionNeedsUpdate: true }],
    [
      "the lifecycle row",
      {
        lifecycle: {
          enabled: true,
          preferenceConfigured: true,
          installed: false,
          outdated: false,
          supported: true,
          needsUpdate: true,
          cleanupRequired: false,
        },
      },
    ],
    [
      "the stop row",
      {
        stop: {
          supported: true,
          enabled: true,
          installed: true,
          outdated: true,
          otherHandlersDetected: false,
        },
      },
    ],
    [
      "the question-takeover row",
      {
        askQuestion: { supported: true, enabled: true, installed: true, outdated: true },
      },
    ],
  ])("is true for %s", (_name, status) => {
    expect(hasUpdate(modeStatus(status))).toBe(true);
  });

  it("ignores a stale flag on a row the Agent does not have", () => {
    const stale = modeStatus({
      stop: {
        supported: false,
        enabled: false,
        installed: false,
        outdated: true,
        otherHandlersDetected: false,
      },
      askQuestion: { supported: false, enabled: false, installed: false, outdated: true },
    });
    expect(hasUpdate(stale)).toBe(false);
  });
});

describe("versionProblem", () => {
  it("only Pi and Codex judge the Agent's own version", () => {
    const old = { versionSupported: false };
    expect(versionProblem(input(old, { id: "pi" }))).toBe(true);
    expect(versionProblem(input(old, { id: "codex" }))).toBe(true);
    for (const id of ["cursor", "claude", "grok", "antigravity"] as const) {
      expect(versionProblem(input(old, { id }))).toBe(false);
    }
  });

  it("does not judge Pi while its version probe is still running", () => {
    expect(
      versionProblem(input({ versionSupported: false }, { id: "pi", versionLoading: true })),
    ).toBe(false);
  });

  it("does not judge an Agent that is not integrated", () => {
    expect(versionProblem(input({ mode: "none", versionSupported: false }, { id: "codex" }))).toBe(
      false,
    );
  });
});

describe("agentTags", () => {
  it("marks an integrated Agent with one green tag", () => {
    expect(agentTags(input())).toEqual([{ key: "integrated", tone: "ok" }]);
  });

  it("gives a not-integrated Agent no tag at all", () => {
    expect(agentTags(input({ mode: "none" }))).toEqual([]);
  });

  it("reports an unreadable state and nothing else", () => {
    expect(
      agentTags(input({ needsUpdate: true }, { readError: true })),
    ).toEqual([{ key: "unreadable", tone: "err" }]);
  });

  it("adds an orange update tag to an integrated Agent", () => {
    expect(agentTags(input({ ruleNeedsUpdate: true }))).toEqual([
      { key: "integrated", tone: "ok" },
      { key: "needsUpdate", tone: "warn" },
    ]);
  });

  it("adds a version tag for an old Codex, but not while Pi is still being probed", () => {
    expect(keys(input({ versionSupported: false }, { id: "codex" }))).toEqual([
      "integrated",
      "unsupportedVersion",
    ]);
    expect(keys(input({ versionSupported: false }, { id: "pi", versionLoading: true }))).toEqual([
      "integrated",
    ]);
  });

  it("says when the Agent's own policy keeps the hooks from running", () => {
    const blocked = {
      permission: {
        supported: true,
        unsupportedReason: null,
        enabled: true,
        configured: true,
        outdated: false,
        needsUpdate: false,
        knownBlockedReason: "disable_all_hooks",
        otherHandlersDetected: false,
      },
    };
    expect(keys(input(blocked))).toEqual(["integrated", "blocked"]);
    // The red line is part of the permission row, so an Agent without that row never shows it.
    expect(
      keys(input({ permission: { ...blocked.permission, supported: false } })),
    ).toEqual(["integrated"]);
  });

  it("does not count other handlers as a problem: that note stays in the details", () => {
    expect(
      keys(
        input({
          stop: {
            supported: true,
            enabled: true,
            installed: true,
            outdated: false,
            otherHandlersDetected: true,
          },
        }),
      ),
    ).toEqual(["integrated"]);
  });

  it("asks for a cleanup of a leftover lifecycle hook instead of calling it an update", () => {
    const leftover = {
      mode: "none" as const,
      needsUpdate: true,
      lifecycle: {
        enabled: false,
        preferenceConfigured: false,
        installed: true,
        outdated: false,
        supported: true,
        needsUpdate: true,
        cleanupRequired: true,
      },
    };
    expect(agentTags(input(leftover))).toEqual([{ key: "cleanup", tone: "warn" }]);
  });

  it("still names another pending update of a not-integrated Agent", () => {
    expect(keys(input({ mode: "none", needsUpdate: true }))).toEqual(["needsUpdate"]);
  });
});

describe("needsAction", () => {
  it("is false for an Agent that needs nothing, a blocked one included", () => {
    expect(needsAction(input())).toBe(false);
    expect(needsAction(input({ mode: "none" }))).toBe(false);
  });

  it("is true for an update or an Agent too old", () => {
    expect(needsAction(input({ hookNeedsUpdate: true }))).toBe(true);
    expect(needsAction(input({ versionSupported: false }, { id: "codex" }))).toBe(true);
  });

  it("is false when there are no details to open", () => {
    expect(needsAction(input({ mode: "none", needsUpdate: true }))).toBe(false);
    expect(needsAction(input({ ruleNeedsUpdate: true }, { readError: true }))).toBe(false);
  });
});
