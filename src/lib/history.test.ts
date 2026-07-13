import { describe, expect, it } from "vitest";
import { agentKindOf, customSourceOf, workspaceNameOf, DEFAULT_SOURCE_NAME } from "./history";
import type { HistoryEntry } from "./types";

// Only the fields these helpers read; the rest of HistoryEntry is irrelevant here.
function entry(partial: Partial<HistoryEntry>): HistoryEntry {
  return { project: "", source: "", ...partial } as HistoryEntry;
}

describe("agentKindOf", () => {
  it("prefers the persisted agentKind", () => {
    expect(agentKindOf(entry({ agentKind: "codex", source: "Claude Code" }))).toBe("codex");
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
    expect(customSourceOf(entry({ source: "release-bot" }), "Codex")).toBe("release-bot");
  });
});
