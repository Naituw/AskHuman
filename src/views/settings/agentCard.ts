// Pure rules of an Agent card in Settings → Agents: which status tags its header wears, whether it
// has details to fold away, and whether something inside needs the user (so the card opens by
// itself). Kept out of the component so the rules can be tested on their own
// (docs/specs/settings-agents-channels-cards.md).
import type { AgentId, AgentModeStatus } from "../../lib/types";

export type AgentTagTone = "ok" | "warn" | "err";

/** i18n key under `settings.integration.tag`. */
export type AgentTagKey =
  | "integrated"
  | "needsUpdate"
  | "cleanup"
  | "unsupportedVersion"
  | "blocked"
  | "unreadable";

export interface AgentTag {
  key: AgentTagKey;
  tone: AgentTagTone;
}

export interface AgentCardInput {
  id: AgentId;
  status: AgentModeStatus;
  /** The status could not be read at all. */
  readError: boolean;
  /** Pi's version probe is still running: its version is not judged until the probe ends. */
  versionLoading: boolean;
}

/** An integrated card has rows to fold away; a not-integrated or unreadable one is only its header. */
export function hasDetails(input: AgentCardInput): boolean {
  return !input.readError && input.status.mode !== "none";
}

/**
 * Pi and Codex check the Agent's own version (the other Agents do not): the card shows a red line
 * when it is too old, or, for Pi, cannot be found.
 */
export function versionProblem(input: AgentCardInput): boolean {
  if (!hasDetails(input)) return false;
  if (input.id === "pi") return !input.versionLoading && !input.status.versionSupported;
  if (input.id === "codex") return !input.status.versionSupported;
  return false;
}

/** Whether any "Update" button is on the card: the artifact flags plus the per-row ones. */
export function hasUpdate(status: AgentModeStatus): boolean {
  return (
    status.needsUpdate ||
    status.ruleNeedsUpdate ||
    status.hookNeedsUpdate ||
    status.mcpNeedsUpdate ||
    status.permissionNeedsUpdate ||
    status.lifecycle.needsUpdate ||
    (status.stop.supported && status.stop.outdated) ||
    (status.askQuestion.supported && status.askQuestion.outdated)
  );
}

/** Something inside the details is waiting for the user: an update to apply, or an Agent too old. */
export function needsAction(input: AgentCardInput): boolean {
  return hasDetails(input) && (hasUpdate(input.status) || versionProblem(input));
}

/** The Agent's own policy keeps the managed hooks from running (a red line in the permission row). */
function hooksBlocked(input: AgentCardInput): boolean {
  return (
    hasDetails(input) &&
    input.status.permission.supported &&
    !!input.status.permission.knownBlockedReason
  );
}

/** The status tags a card's header wears, in display order. */
export function agentTags(input: AgentCardInput): AgentTag[] {
  if (input.readError) return [{ key: "unreadable", tone: "err" }];
  const { status } = input;
  const tags: AgentTag[] = [];
  if (status.mode !== "none") {
    tags.push({ key: "integrated", tone: "ok" });
    if (hasUpdate(status)) tags.push({ key: "needsUpdate", tone: "warn" });
    if (versionProblem(input)) tags.push({ key: "unsupportedVersion", tone: "warn" });
    if (hooksBlocked(input)) tags.push({ key: "blocked", tone: "warn" });
    return tags;
  }
  // Not integrated: only leftovers of an earlier integration are worth a tag.
  if (status.lifecycle.cleanupRequired) tags.push({ key: "cleanup", tone: "warn" });
  else if (status.needsUpdate) tags.push({ key: "needsUpdate", tone: "warn" });
  return tags;
}
