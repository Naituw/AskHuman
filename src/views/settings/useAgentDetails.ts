// Which Agent cards of Settings → Agents have their details open. The state lives in the settings
// context, not in the tab, so a card stays as the user left it when they switch tabs and come back.
import { ref, watch, type Ref } from "vue";
import type { AgentId, AgentModeStatus } from "../../lib/types";
import { needsAction } from "./agentCard";

export function useAgentDetails(
  agents: readonly { id: AgentId }[],
  modes: Ref<Record<AgentId, AgentModeStatus>>,
  readError: Ref<Partial<Record<AgentId, boolean>>>,
  piVersionLoading: Ref<boolean>,
) {
  const agentExpanded = ref(
    Object.fromEntries(agents.map((agent) => [agent.id, false])) as Record<AgentId, boolean>,
  );

  // A card opens by itself the moment something in it needs the user (an update to apply, an
  // Agent too old), and from then on the user decides: folding it again keeps it folded until the
  // need goes away and comes back.
  for (const { id } of agents) {
    watch(
      () =>
        needsAction({
          id,
          status: modes.value[id],
          readError: readError.value[id] === true,
          versionLoading: id === "pi" && piVersionLoading.value,
        }),
      (needed) => {
        if (needed) agentExpanded.value[id] = true;
      },
      { immediate: true },
    );
  }

  function toggleAgentDetails(id: AgentId) {
    agentExpanded.value[id] = !agentExpanded.value[id];
  }

  function openAgentDetails(id: AgentId) {
    agentExpanded.value[id] = true;
  }

  return { agentExpanded, toggleAgentDetails, openAgentDetails };
}
