// Shared helpers for the history window (list + read-only detail).

import type { HistoryEntry } from "./types";

/** Default caller source name (backend `models::DEFAULT_SOURCE_NAME`). */
export const DEFAULT_SOURCE_NAME = "the Loop";

// Known agent-family display names (as recorded in `source` by older versions,
// identical in zh/en) → family id. Lets legacy entries without `agentKind`
// still resolve to an agent family.
const SOURCE_TO_KIND: Record<string, string> = {
  "claude code": "claude",
  codex: "codex",
  cursor: "cursor",
  grok: "grok",
};

/**
 * Effective agent family of an entry: persisted `agentKind`, falling back to a
 * `source` that matches a known family display name (legacy entries).
 */
export function agentKindOf(e: HistoryEntry): string {
  if (e.agentKind) return e.agentKind;
  return SOURCE_TO_KIND[e.source.trim().toLowerCase()] ?? "";
}

/** Workspace display name (folder basename of the project root path). */
export function workspaceNameOf(e: HistoryEntry): string {
  if (!e.project) return "";
  const parts = e.project.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] || e.project;
}

/**
 * Custom caller source name worth showing next to the agent badge: non-empty,
 * not the built-in default, and not just the agent family label itself.
 */
export function customSourceOf(e: HistoryEntry, agentLabel: string): string {
  const s = e.source.trim();
  if (!s || s === DEFAULT_SOURCE_NAME) return "";
  if (agentLabel && s.toLowerCase() === agentLabel.trim().toLowerCase()) return "";
  return s;
}
