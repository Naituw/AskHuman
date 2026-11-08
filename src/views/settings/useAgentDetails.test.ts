// The open / folded state of the Agent cards: they open by themselves when something needs the
// user, and after that the user decides.
import { nextTick, ref } from "vue";
import { describe, expect, it } from "vitest";
import type { AgentId, AgentModeStatus } from "../../lib/types";
import { modeStatus } from "./integration.testkit";
import { useAgentDetails } from "./useAgentDetails";

const IDS: AgentId[] = ["cursor", "claude", "codex", "grok", "pi", "antigravity"];

function setup(initial: Partial<Record<AgentId, Partial<AgentModeStatus>>> = {}) {
  const modes = ref(
    Object.fromEntries(
      IDS.map((id) => [id, modeStatus({ mode: "none", ...initial[id] })]),
    ) as Record<AgentId, AgentModeStatus>,
  );
  const readError = ref<Partial<Record<AgentId, boolean>>>({});
  const piVersionLoading = ref(false);
  const details = useAgentDetails(
    IDS.map((id) => ({ id })),
    modes,
    readError,
    piVersionLoading,
  );
  return { modes, readError, piVersionLoading, ...details };
}

describe("useAgentDetails", () => {
  it("starts with every card folded", () => {
    const { agentExpanded } = setup({ claude: { mode: "cli" }, codex: { mode: "mcp" } });
    expect(Object.values(agentExpanded.value).every((open) => !open)).toBe(true);
  });

  it("opens a card that already needs an update when the state is first there", () => {
    const { agentExpanded } = setup({ claude: { mode: "cli", ruleNeedsUpdate: true } });
    expect(agentExpanded.value.claude).toBe(true);
    expect(agentExpanded.value.cursor).toBe(false);
  });

  it("opens a card when an update turns up later, and leaves the others alone", async () => {
    const { agentExpanded, modes } = setup({ claude: { mode: "cli" }, cursor: { mode: "cli" } });
    modes.value.claude = modeStatus({ mode: "cli", hookNeedsUpdate: true });
    await nextTick();
    expect(agentExpanded.value.claude).toBe(true);
    expect(agentExpanded.value.cursor).toBe(false);
  });

  it("opens Codex that is too old, and Pi only once its version probe is over", async () => {
    const { agentExpanded, modes, piVersionLoading } = setup({
      codex: { mode: "mcp" },
      pi: { mode: "cli" },
    });
    piVersionLoading.value = true;
    modes.value.codex = modeStatus({ mode: "mcp", versionSupported: false });
    modes.value.pi = modeStatus({ mode: "cli", versionSupported: false });
    await nextTick();
    expect(agentExpanded.value.codex).toBe(true);
    expect(agentExpanded.value.pi).toBe(false);
    piVersionLoading.value = false;
    await nextTick();
    expect(agentExpanded.value.pi).toBe(true);
  });

  it("keeps a card the user folded folded, until the need goes away and comes back", async () => {
    const { agentExpanded, modes, toggleAgentDetails } = setup({
      claude: { mode: "cli", ruleNeedsUpdate: true },
    });
    expect(agentExpanded.value.claude).toBe(true);
    toggleAgentDetails("claude");
    expect(agentExpanded.value.claude).toBe(false);
    // Another refresh with the same need does not re-open it.
    modes.value.claude = modeStatus({ mode: "cli", ruleNeedsUpdate: true });
    await nextTick();
    expect(agentExpanded.value.claude).toBe(false);
    // The need goes away, then a new one appears.
    modes.value.claude = modeStatus({ mode: "cli" });
    await nextTick();
    modes.value.claude = modeStatus({ mode: "cli", mcpNeedsUpdate: true });
    await nextTick();
    expect(agentExpanded.value.claude).toBe(true);
  });

  it("does not fold a card by itself once the update is done", async () => {
    const { agentExpanded, modes } = setup({ claude: { mode: "cli", ruleNeedsUpdate: true } });
    modes.value.claude = modeStatus({ mode: "cli" });
    await nextTick();
    expect(agentExpanded.value.claude).toBe(true);
  });

  it("never opens a card that is not integrated: it has no details", () => {
    const { agentExpanded } = setup({ grok: { mode: "none", needsUpdate: true } });
    expect(agentExpanded.value.grok).toBe(false);
  });

  it("does not open a card whose state cannot be read", async () => {
    const { agentExpanded, modes, readError } = setup({ claude: { mode: "cli" } });
    readError.value.claude = true;
    modes.value.claude = modeStatus({ mode: "cli", hookNeedsUpdate: true });
    await nextTick();
    expect(agentExpanded.value.claude).toBe(false);
  });

  it("toggles and opens on request", () => {
    const { agentExpanded, toggleAgentDetails, openAgentDetails } = setup();
    toggleAgentDetails("grok");
    expect(agentExpanded.value.grok).toBe(true);
    toggleAgentDetails("grok");
    expect(agentExpanded.value.grok).toBe(false);
    openAgentDetails("grok");
    openAgentDetails("grok");
    expect(agentExpanded.value.grok).toBe(true);
  });
});
