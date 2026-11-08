// Settings → Advanced → "Agent readiness": what an Agent still lacks before an IM-created task can
// start it. Each lacking thing is an orange cross in the readiness matrix that sends the person to
// where it is fixed (`useAgentTasks.openReadinessIssue`); a met condition is a check. An Agent that
// cannot start tasks at all has nothing to fix, so it is shown as unsupported instead.
import type { AgentTaskReadiness } from "../../lib/types";

export type ReadinessIssue = "binary" | "lifecycle" | "integration";

/** The overall state of one Agent's row, and so the tag it carries. */
export type ReadinessState = "ready" | "notReady" | "unsupported";

/**
 * Whether the Agent can start a task, is still missing something, or cannot start tasks whatever
 * is done (`launchSupported`, e.g. Antigravity): there is nothing the person can fix for the last.
 */
export function readinessState(
  item: Pick<AgentTaskReadiness, "ready" | "launchSupported">,
): ReadinessState {
  if (!item.launchSupported) return "unsupported";
  return item.ready ? "ready" : "notReady";
}

/** The unmet conditions, in the order they are fixed: the Agent's CLI first, then AskHuman's side. */
export function readinessIssues(
  item: Pick<AgentTaskReadiness, "binaryReady" | "lifecycleReady" | "integrationReady">,
): ReadinessIssue[] {
  const issues: ReadinessIssue[] = [];
  if (!item.binaryReady) issues.push("binary");
  if (!item.lifecycleReady) issues.push("lifecycle");
  if (!item.integrationReady) issues.push("integration");
  return issues;
}
