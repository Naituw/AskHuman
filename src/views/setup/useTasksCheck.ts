// The pre-flight behind the Agent-task offer (docs/specs/onboarding.md D25): what the phone's
// "New task" page would list once the feature is on, read from the same sources the phone reads —
// the saved projects and the Agents that can be started. It is advisory: whatever it finds, the
// person can go on, and a read that fails is shown as "could not check", never as a problem.
import { computed, ref } from "vue";
import { agentTaskReadiness, agentTaskWorkspaces } from "../../lib/ipc";
import type { AgentTaskReadiness, AgentTaskWorkspace } from "../../lib/types";
import { readinessState } from "../settings/readiness";

export type TasksCheckState = "idle" | "checking" | "done";

/** The projects the phone may pick from: the saved ones that are not hidden. */
export function visibleProjects(items: AgentTaskWorkspace[]): AgentTaskWorkspace[] {
  return items.filter((item) => !item.hidden);
}

/** The Agents a task can be started in right now. */
export function startableAgents(items: AgentTaskReadiness[]): AgentTaskReadiness[] {
  return items.filter((item) => readinessState(item) === "ready");
}

export function useTasksCheck() {
  const state = ref<TasksCheckState>("idle");
  /** `null` = the read failed (or has not finished). */
  const projects = ref<AgentTaskWorkspace[] | null>(null);
  const agents = ref<AgentTaskReadiness[] | null>(null);

  /** Whether a prerequisite is known to be missing. Only a finished check says so. */
  const noProjects = computed(
    () => state.value === "done" && projects.value !== null && projects.value.length === 0,
  );
  const noAgents = computed(
    () => state.value === "done" && agents.value !== null && agents.value.length === 0,
  );
  const hasIssue = computed(() => noProjects.value || noAgents.value);

  let inflight = false;

  /**
   * Read both again; a run already under way is not started twice. Only the first run shows the
   * "checking" state: a later one (`fresh`: the person fixed something elsewhere, so the Agents'
   * CLIs are looked up again instead of trusting the last lookup) swaps the answer in quietly.
   */
  async function run(fresh = false) {
    if (inflight) return;
    inflight = true;
    if (state.value === "idle") state.value = "checking";
    try {
      const [workspaces, readiness] = await Promise.allSettled([
        // `true` also merges what the Agents' recent history shows, as the phone's list does.
        agentTaskWorkspaces(true),
        agentTaskReadiness(fresh ? { force: true } : undefined),
      ]);
      projects.value =
        workspaces.status === "fulfilled" ? visibleProjects(workspaces.value) : null;
      agents.value = readiness.status === "fulfilled" ? startableAgents(readiness.value) : null;
      state.value = "done";
    } finally {
      inflight = false;
    }
  }

  return { state, projects, agents, noProjects, noAgents, hasIssue, run };
}
