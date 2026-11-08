// The pre-flight behind the Agent-task offer (docs/specs/onboarding.md D25): which Agents a task can
// be started on right now, and which projects the phone's "New task" page would list. It reads the
// sources the phone reads — the backend's own verdict on each Agent (`startable`, the one rule the
// phone, IM and Settings share) and the saved projects — and it runs BEFORE the question is asked,
// so the screen never invites someone to turn on what it already knows cannot work. The projects
// are only a hint (a new device has none until an Agent has been used in a folder); a read that
// fails is shown as "could not check", never as a problem.
import { computed, ref } from "vue";
import { agentTaskReadiness, agentTaskWorkspaces } from "../../lib/ipc";
import type { AgentTaskReadiness, AgentTaskWorkspace } from "../../lib/types";
import { isStartable } from "../settings/readiness";

export type TasksCheckState = "idle" | "checking" | "done";

/** What the screen is about: still checking, an Agent can start, none can, or the check failed. */
export type TasksStage = "checking" | "ready" | "blocked" | "unknown";

/** The projects the phone may pick from: the saved ones that are not hidden. */
export function visibleProjects(items: AgentTaskWorkspace[]): AgentTaskWorkspace[] {
  return items.filter((item) => !item.hidden);
}

/**
 * The Agents the person has set up and a task could ever start on. An Agent that is not
 * integrated is not theirs to fix here, and one with no launch path at all (Antigravity) never
 * will start, so neither is listed.
 */
export function setUpAgents(items: AgentTaskReadiness[]): AgentTaskReadiness[] {
  return items.filter((item) => item.launchSupported && item.integrationMode !== "none");
}

export function useTasksCheck() {
  const state = ref<TasksCheckState>("idle");
  /** A check is under way (the first one, or a "check again"). */
  const busy = ref(false);
  /** `null` = the read failed (or has not finished). */
  const projects = ref<AgentTaskWorkspace[] | null>(null);
  /** The Agents the person has set up, each with the backend's verdict; `null` = the read failed. */
  const agents = ref<AgentTaskReadiness[] | null>(null);

  const startable = computed(() => (agents.value ?? []).filter(isStartable));
  const blocked = computed(() => (agents.value ?? []).filter((item) => !isStartable(item)));
  const stage = computed<TasksStage>(() => {
    if (state.value !== "done") return "checking";
    if (agents.value === null) return "unknown";
    return startable.value.length > 0 ? "ready" : "blocked";
  });
  /** Only a finished check says there are no projects. */
  const noProjects = computed(
    () => state.value === "done" && projects.value !== null && projects.value.length === 0,
  );

  let inflight = false;

  /**
   * Read both again; a run already under way is not started twice. Only the first run puts the
   * screen in the "checking" stage: a later one (`fresh`: the person fixed something elsewhere, so
   * the Agents' CLIs are looked up again instead of trusting the last lookup) swaps the answer in
   * quietly and only marks `busy`.
   */
  async function run(fresh = false) {
    if (inflight) return;
    inflight = true;
    busy.value = true;
    if (state.value === "idle") state.value = "checking";
    try {
      const [workspaces, readiness] = await Promise.allSettled([
        // `true` also merges what the Agents' recent history shows, as the phone's list does.
        agentTaskWorkspaces(true),
        agentTaskReadiness(fresh ? { force: true } : undefined),
      ]);
      projects.value =
        workspaces.status === "fulfilled" ? visibleProjects(workspaces.value) : null;
      agents.value = readiness.status === "fulfilled" ? setUpAgents(readiness.value) : null;
      state.value = "done";
    } finally {
      inflight = false;
      busy.value = false;
    }
  }

  return { state, stage, busy, projects, agents, startable, blocked, noProjects, run };
}
