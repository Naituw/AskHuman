import { describe, expect, it } from "vitest";
import type { HistoryEntry } from "./types";
import {
  ALL_HISTORY_SESSIONS,
  DEFAULT_SOURCE_NAME,
  agentKindOf,
  customSourceOf,
  draftOf,
  groupHistorySessions,
  historySessionOf,
  historySessionToken,
  matchesHistorySession,
  shortSessionId,
  shownReplies,
  workspaceNameOf,
} from "./history";

function entry(overrides: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: "e",
    timestampMs: 1,
    project: "/p",
    source: "Codex",
    channel: "popup",
    action: "send",
    isMarkdown: true,
    message: { text: "", files: [] },
    questions: [],
    answers: [],
    ...overrides,
  };
}

describe("agentKindOf", () => {
  it("prefers the persisted agentKind", () => {
    expect(agentKindOf(entry({ agentKind: "codex", source: "Claude Code" }))).toBe(
      "codex"
    );
  });

  it("falls back to legacy source display names, case-insensitively", () => {
    expect(agentKindOf(entry({ source: "Claude Code" }))).toBe("claude");
    expect(agentKindOf(entry({ source: "  CURSOR  " }))).toBe("cursor");
  });

  it("returns empty for unknown sources", () => {
    expect(agentKindOf(entry({ source: "my-script" }))).toBe("");
    expect(agentKindOf(entry({ source: "" }))).toBe("");
  });
});

describe("workspaceNameOf", () => {
  it("returns the basename of the project root", () => {
    expect(workspaceNameOf(entry({ project: "/home/me/proj" }))).toBe("proj");
  });

  it("ignores trailing separators and handles Windows paths", () => {
    expect(workspaceNameOf(entry({ project: "/home/me/proj//" }))).toBe("proj");
    expect(workspaceNameOf(entry({ project: "C:\\work\\proj\\" }))).toBe("proj");
  });

  it("returns empty when there is no project", () => {
    expect(workspaceNameOf(entry({ project: "" }))).toBe("");
  });
});

describe("customSourceOf", () => {
  it("hides the built-in default source", () => {
    expect(customSourceOf(entry({ source: DEFAULT_SOURCE_NAME }), "")).toBe("");
    expect(customSourceOf(entry({ source: "  " }), "")).toBe("");
  });

  it("hides a source that just repeats the agent label", () => {
    expect(customSourceOf(entry({ source: "Claude Code" }), "claude code")).toBe("");
  });

  it("keeps a genuinely custom source", () => {
    expect(customSourceOf(entry({ source: "release-bot" }), "Codex")).toBe(
      "release-bot"
    );
  });
});

describe("history session identity", () => {
  it("prefers an exact native session over an MCP fallback", () => {
    expect(
      historySessionOf(
        entry({
          agentKind: "codex",
          agentSessionId: "session-1",
          mcpInstanceId: "instance-1",
        })
      )
    ).toEqual({ type: "agent", agentKind: "codex", sessionId: "session-1" });
  });

  it("does not infer a native session kind from the legacy source", () => {
    expect(
      historySessionOf(
        entry({
          agentKind: null,
          agentSessionId: "session-1",
          mcpInstanceId: "instance-1",
        })
      )
    ).toEqual({ type: "mcp", project: "/p", instanceId: "instance-1" });

    expect(
      historySessionOf(
        entry({ agentKind: null, agentSessionId: "session-1", mcpInstanceId: null })
      )
    ).toEqual({ type: "unbound" });
  });

  it("includes the project in MCP fallback identity", () => {
    const a = historySessionToken(
      historySessionOf(entry({ project: "/a", mcpInstanceId: "same" }))
    );
    const b = historySessionToken(
      historySessionOf(entry({ project: "/b", mcpInstanceId: "same" }))
    );
    expect(a).not.toBe(b);
  });

  it("uses structured tokens without delimiter collisions", () => {
    const a = historySessionToken({
      type: "agent",
      agentKind: "a:b",
      sessionId: "c",
    });
    const b = historySessionToken({
      type: "agent",
      agentKind: "a",
      sessionId: "b:c",
    });
    expect(a).not.toBe(b);
  });
});

describe("history session grouping", () => {
  it("aggregates counts and sorts by most recent entry", () => {
    const groups = groupHistorySessions([
      entry({ id: "a1", timestampMs: 1, agentKind: "codex", agentSessionId: "a" }),
      entry({ id: "b", timestampMs: 3, agentKind: "cursor", agentSessionId: "b" }),
      entry({ id: "a2", timestampMs: 4, agentKind: "codex", agentSessionId: "a" }),
      entry({ id: "u", timestampMs: 2, source: "script", project: "" }),
    ]);

    expect(groups.map((group) => [group.ref.type, group.count, group.lastMs])).toEqual([
      ["agent", 2, 4],
      ["agent", 1, 3],
      ["unbound", 1, 2],
    ]);
  });

  it("matches a selected session without changing the all-session behavior", () => {
    const a = entry({ agentKind: "codex", agentSessionId: "a" });
    const b = entry({ agentKind: "codex", agentSessionId: "b" });
    const token = historySessionToken(historySessionOf(a));
    expect(matchesHistorySession(a, token)).toBe(true);
    expect(matchesHistorySession(b, token)).toBe(false);
    expect(matchesHistorySession(a, ALL_HISTORY_SESSIONS)).toBe(true);
    expect(matchesHistorySession(b, ALL_HISTORY_SESSIONS)).toBe(true);
  });
});

it("shortens long ids but preserves short ids", () => {
  expect(shortSessionId("short")).toBe("short");
  expect(shortSessionId("1234567890")).toBe("12345678…");
});

describe("the unsent content of a cancelled ask (docs/specs/popup-draft-on-cancel.md D8)", () => {
  const typed = { answers: [{ selectedOptions: ["No"], userInput: "text", images: [], files: [] }] };

  it("belongs to a cancelled entry only", () => {
    expect(draftOf(entry({ action: "cancel", draft: typed }))).toBe(typed);
    expect(draftOf(entry({ action: "cancel" }))).toBeNull();
    expect(draftOf(entry({ action: "cancel", draft: null }))).toBeNull();
    // Whatever rides on a sent entry is not a draft.
    expect(draftOf(entry({ action: "send", draft: typed }))).toBeNull();
  });

  it("is what the window shows as the reply of a cancelled entry, and never takes a sent answer's place", () => {
    const sent = [{ selectedOptions: [], userInput: "answer", images: [], files: [] }];
    expect(shownReplies(entry({ action: "cancel", draft: typed }))).toBe(typed.answers);
    expect(shownReplies(entry({ action: "cancel" }))).toEqual([]);
    expect(shownReplies(entry({ action: "send", answers: sent, draft: typed }))).toBe(sent);
  });
});
