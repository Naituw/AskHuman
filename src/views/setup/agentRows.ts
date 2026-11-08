// Pure rules of the wizard's Agent page: what a row starts as, which rows still need applying,
// and which tags it wears. Kept out of the components so the rules can be tested on their own
// (docs/plans/onboarding.md §3.3).
import type { SetupAgentMode, SetupAgentRow } from "../../lib/types";

/** The user's choice for one row: whether it is part of this round, and with which mode. */
export interface RowPick {
  checked: boolean;
  mode: SetupAgentMode;
}

/** Something is already on disk for this Agent, in whichever mode. */
export function isIntegrated(row: SetupAgentRow): boolean {
  return row.mode !== "none";
}

/** A row that cannot be changed: its state is unreadable, or the Agent is too old to integrate. */
export function isBlocked(row: SetupAgentRow): boolean {
  return !!row.statusError || row.delivery === "unsupportedVersion";
}

/**
 * Rows worth the main area: installed here or already integrated. The rest are listed below it as
 * small tiles ("other supported Agents"): they show what else AskHuman supports and can still be
 * ticked by hand when detection missed one.
 */
export function isMainRow(row: SetupAgentRow): boolean {
  return row.detected || isIntegrated(row);
}

/** The modes a row offers, CLI first. */
export function modesOf(row: SetupAgentRow): SetupAgentMode[] {
  const modes: SetupAgentMode[] = [];
  if (row.supportsCli) modes.push("cli");
  if (row.supportsMcp) modes.push("mcp");
  return modes;
}

/**
 * What a row starts as. An integrated Agent keeps its mode and stays ticked; a detected one
 * starts ticked with the recommended mode (that is what a rerun is there to add); an undetected
 * one starts unticked but can still be picked.
 */
export function preselect(row: SetupAgentRow): RowPick {
  const integrated = isIntegrated(row);
  const mode: SetupAgentMode = integrated
    ? (row.mode as SetupAgentMode)
    : row.recommended;
  return {
    mode,
    checked: !isBlocked(row) && (integrated || row.detected),
  };
}

/**
 * Whether applying would change something for this row. Unticking an integrated row means "leave
 * it alone this time": the wizard never uninstalls.
 */
export function isPending(row: SetupAgentRow, pick: RowPick | undefined): boolean {
  if (!pick || !pick.checked || isBlocked(row)) return false;
  return pick.mode !== row.mode || row.needsUpdate;
}

export type TagTone = "ok" | "warn" | "muted";

export interface RowTag {
  /** i18n key under `setup.agents.tags`. */
  key: string;
  /** Interpolation values for the label. */
  params?: Record<string, string>;
  tone: TagTone;
}

/** The status tags a row wears, in display order. */
export function rowTags(row: SetupAgentRow): RowTag[] {
  const tags: RowTag[] = [];
  if (row.statusError) {
    tags.push({ key: "unreadable", tone: "warn" });
    return tags;
  }
  if (isIntegrated(row)) {
    tags.push({
      key: "integrated",
      params: { mode: row.mode === "cli" ? "CLI" : "MCP" },
      tone: "ok",
    });
  } else if (row.detected) {
    tags.push({ key: "detected", tone: "muted" });
  } else {
    tags.push({ key: "notDetected", tone: "muted" });
  }
  if (row.needsUpdate) tags.push({ key: "needsUpdate", tone: "warn" });
  if (row.delivery === "untrusted") tags.push({ key: "untrusted", tone: "warn" });
  if (row.delivery === "unsupportedVersion") {
    tags.push({ key: "unsupportedVersion", tone: "warn" });
  }
  return tags;
}

/** What the page's main button says (`apply` = there is something to write). */
export type AgentsPrimary = "apply" | "next" | "continue";

export function agentsPrimary(hasChanges: boolean, applied: boolean): AgentsPrimary {
  if (hasChanges) return "apply";
  return applied ? "continue" : "next";
}
