// Settings → Advanced → "Agent readiness" and the setup wizard's Agent-task offer: whether a task can
// be started on an Agent, and what it still lacks when it cannot. The verdict is the backend's — the
// same one the phone and IM act on — so no screen can call an Agent unready that the phone starts
// (`integrations::agent_start`). Each lacking thing is an orange cross in the readiness matrix that
// sends the person to where it is fixed (`useAgentTasks.openReadinessIssue`); a met condition is a
// check. An Agent that cannot start tasks at all has nothing to fix, so it is shown as unsupported.
import { codexLaunchReasons } from "../../lib/codexLaunch";
import { cursorLaunchReasons } from "../../lib/cursorLaunch";
import type { CodexLaunchStatus, CursorLaunchStatus } from "../../lib/ipc";
import type { AgentTaskReadiness } from "../../lib/types";

type Translate = (key: string, values?: Record<string, string>) => string;

export type ReadinessIssue = "binary" | "lifecycle" | "integration";

/** The overall state of one Agent's row, and so the tag it carries. */
export type ReadinessState = "ready" | "notReady" | "unsupported";

/**
 * Whether a task can be started on this Agent right now. `startable` is the backend's verdict and
 * also counts Codex's Desktop App and Cursor's Agents window; `ready` (the CLI route alone) is the
 * answer only where no verdict is given (older builds, test fixtures). An Agent that cannot start
 * tasks at all is never startable.
 */
export function isStartable(
  item: Pick<AgentTaskReadiness, "ready" | "launchSupported" | "startable">,
): boolean {
  return item.launchSupported && (item.startable ?? item.ready);
}

/**
 * Whether the Agent can start a task, is still missing something, or cannot start tasks whatever
 * is done (`launchSupported`, e.g. Antigravity): there is nothing the person can fix for the last.
 */
export function readinessState(
  item: Pick<AgentTaskReadiness, "ready" | "launchSupported" | "startable">,
): ReadinessState {
  if (!item.launchSupported) return "unsupported";
  return isStartable(item) ? "ready" : "notReady";
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

/**
 * Whether the Agent can start a task although its CLI route is not ready: it runs in its Desktop
 * App or Agents window instead. Its CLI conditions are then not needed, so a screen shows them as
 * such rather than as something to fix.
 */
export function startsOutsideTheCli(
  item: Pick<AgentTaskReadiness, "ready" | "launchSupported" | "startable" | "startRoute">,
): boolean {
  return isStartable(item) && !item.ready && item.startRoute === "desktop";
}

/**
 * Why a task cannot be started on this Agent, in plain words. Codex and Cursor have a route besides
 * the CLI, so they are explained by both routes (the backend's launch status); every other Agent
 * has the CLI alone and is explained by the conditions it misses.
 */
export function startBlockers(item: AgentTaskReadiness, t: Translate): string[] {
  if (item.kind === "codex" && item.launch) {
    return codexLaunchReasons(item.launch as CodexLaunchStatus, t);
  }
  if (item.kind === "cursor" && item.launch) {
    return cursorLaunchReasons(item.launch as CursorLaunchStatus, t);
  }
  return readinessIssues(item).map((issue) =>
    t(`setup.tasks.reasons.${issue}`, { label: item.label, command: item.command }),
  );
}
