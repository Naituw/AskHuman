// Settings → Advanced → "Agent readiness": what an Agent still lacks before an IM-created task can
// start it. Each lacking thing is an orange cross in the readiness matrix that sends the person to
// where it is fixed (`useAgentTasks.openReadinessIssue`); a met condition is a check.
import type { AgentTaskReadiness } from "../../lib/types";

export type ReadinessIssue = "binary" | "lifecycle" | "integration";

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
