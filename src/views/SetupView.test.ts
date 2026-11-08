// The setup wizard window end to end, with only the IPC layer replaced: the shell and its entry
// points, the Agent page, the remote-answering page and the Done page
// (docs/specs/onboarding.md, docs/plans/onboarding.md §3).
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../i18n/en";
import zh from "../i18n/zh";
import { i18n } from "../i18n";
import type {
  AgentTaskReadiness,
  AgentTaskWorkspace,
  SetupAgentRow,
  SetupApplyResult,
  SetupChannelStatus,
  SetupEnvReport,
  SetupStatus,
  SettingsPayload,
} from "../lib/types";
import SetupView from "./SetupView.vue";
import {
  channelRow,
  env,
  noChannels,
  readiness,
  row,
  settings,
  status,
  workspace,
} from "./setup/setup.testkit";

const ipc = vi.hoisted(() => ({
  getSettings: vi.fn(),
  saveSettings: vi.fn(),
  setupStatus: vi.fn(),
  setupAgentsScan: vi.fn(),
  setupEnvCheck: vi.fn(),
  setupApplyAgents: vi.fn(),
  setupChannelStatus: vi.fn(),
  setupSkip: vi.fn(),
  setupFinish: vi.fn(),
  setupProbeStart: vi.fn(),
  setupProbeCancel: vi.fn(),
  collaborationStyleApplyIntegrations: vi.fn(),
  agentModeStatus: vi.fn(),
  channelHealth: vi.fn(),
  detectCancel: vi.fn(),
  openSettings: vi.fn(),
  openPath: vi.fn(),
  telegramTest: vi.fn(),
  agentTaskWorkspaces: vi.fn(),
  agentTaskReadiness: vi.fn(),
}));
const events = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
}));
const win = vi.hoisted(() => ({ close: vi.fn() }));
// The page offers the Agent-task switch only where a terminal can be driven (macOS, Windows); the
// real value comes from the test runner's user agent, so each test sets the platform it means.
const platform = vi.hoisted(() => ({ isMac: false, supportsAgentTasks: true }));

vi.mock("../lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/platform")>()),
  get isMac() {
    return platform.isMac;
  },
  get supportsAgentTasks() {
    return platform.supportsAgentTasks;
  },
}));
vi.mock("../lib/ipc", () => ipc);
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    events.handlers.set(name, handler);
    return () => events.handlers.delete(name);
  }),
}));
vi.mock("@tauri-apps/api/webviewWindow", () => ({ getCurrentWebviewWindow: () => win }));
// The settings context the wizard shares its core with imports this plugin, which jsdom cannot load.
vi.mock("tauri-plugin-liquid-glass-api", () => ({ isGlassSupported: vi.fn(async () => false) }));

const message = (path: string): string =>
  path.split(".").reduce<unknown>((node, key) => (node as Record<string, unknown>)[key], en) as string;

/** A mixed Agent list: every kind of row the first page has to treat differently. */
function mixedRows(): SetupAgentRow[] {
  return [
    row({ id: "claude", title: "Claude Code", mode: "cli" }),
    row({
      id: "cursor",
      title: "Cursor",
      recommended: "mcp",
      planned: {
        cli: [],
        mcp: [
          { path: "/u/.cursor/mcp.json", exists: true },
          { path: "/u/.cursor/rules/askhuman.mdc", exists: false },
        ],
      },
    }),
    row({ id: "grok", title: "Grok", mode: "mcp", needsUpdate: true }),
    row({ id: "pi", title: "Pi", statusError: "boom" }),
    row({ id: "codex", title: "Codex", mode: "cli", delivery: "untrusted" }),
    row({ id: "antigravity", title: "Antigravity", detected: false, recommended: "mcp" }),
  ];
}

describe("setup wizard", () => {
  const wrappers: VueWrapper[] = [];
  let payload: SettingsPayload;
  let statusValue: SetupStatus;
  let rowsValue: SetupAgentRow[];
  let envValue: SetupEnvReport;
  let channelRows: SetupChannelStatus[];
  // What the Agent-task offer's pre-flight reads: the saved projects and each Agent's readiness.
  let workspacesValue: AgentTaskWorkspace[];
  let readinessValue: AgentTaskReadiness[];
  let health: { channel: string; message: string; atMs: number }[];
  let capabilities: Record<string, { supported: boolean; enabled: boolean }>;
  // What reached the backend and in which order, for the "style first, then integrations" rules.
  let order: string[];
  let saves: { config: SettingsPayload["config"]; actions: Record<string, unknown> }[];
  // The optional checks the wizard asked the backend to start, in order.
  let probeStarts: { id: string; kind: string; channel?: string }[];

  beforeEach(() => {
    vi.resetAllMocks();
    i18n.global.locale.value = "en";
    platform.isMac = false;
    platform.supportsAgentTasks = true;
    events.handlers.clear();
    payload = settings();
    statusValue = status();
    rowsValue = [row({ id: "claude", mode: "cli" })];
    envValue = env();
    channelRows = noChannels();
    workspacesValue = [workspace("/u/dev/alpha")];
    readinessValue = [readiness("claude")];
    health = [];
    capabilities = {};
    order = [];
    saves = [];

    ipc.getSettings.mockImplementation(async () => payload);
    ipc.saveSettings.mockImplementation(async (config: unknown, actions: unknown) => {
      order.push("save");
      saves.push({
        config: JSON.parse(JSON.stringify(config)),
        actions: JSON.parse(JSON.stringify(actions)),
      });
    });
    ipc.setupStatus.mockImplementation(async () => statusValue);
    ipc.setupAgentsScan.mockImplementation(async () => rowsValue);
    ipc.setupEnvCheck.mockImplementation(async () => envValue);
    ipc.setupApplyAgents.mockImplementation(async () => []);
    ipc.setupChannelStatus.mockImplementation(async () => channelRows);
    ipc.setupSkip.mockImplementation(async () => {});
    ipc.setupFinish.mockImplementation(async () => {
      order.push("finish");
    });
    probeStarts = [];
    ipc.setupProbeStart.mockImplementation(async (kind: string, channel?: string) => {
      const id = `probe-${probeStarts.length + 1}`;
      probeStarts.push({ id, kind, channel });
      return id;
    });
    ipc.setupProbeCancel.mockImplementation(async () => {});
    ipc.collaborationStyleApplyIntegrations.mockImplementation(async () => {
      order.push("applyStyle");
    });
    ipc.agentModeStatus.mockImplementation(async () => ({
      lifecycle: { supported: false, enabled: false },
      permission: { supported: false, enabled: false },
      stop: { supported: false, enabled: false },
      askQuestion: { supported: false, enabled: false },
      ...capabilities,
    }));
    ipc.channelHealth.mockImplementation(async () => health);
    ipc.detectCancel.mockImplementation(async () => {});
    ipc.openSettings.mockImplementation(async () => {});
    ipc.openPath.mockImplementation(async () => {});
    ipc.telegramTest.mockImplementation(async () => "Message delivered");
    ipc.agentTaskWorkspaces.mockImplementation(async () => workspacesValue);
    ipc.agentTaskReadiness.mockImplementation(async () => readinessValue);
    win.close.mockImplementation(async () => {
      order.push("close");
    });
  });

  afterEach(() => {
    wrappers.splice(0).forEach((w) => w.unmount());
    document.body.innerHTML = "";
    window.history.replaceState({}, "", "/");
  });

  async function mountWizard(page?: string) {
    window.history.replaceState({}, "", page ? `/?view=setup&page=${page}` : "/?view=setup");
    const wrapper = mount(SetupView, { global: { plugins: [i18n] }, attachTo: document.body });
    wrappers.push(wrapper);
    await flushPromises();
    return wrapper;
  }

  const find = (w: VueWrapper, id: string) => w.find(`[data-testid="${id}"]`);
  const get = (w: VueWrapper, id: string) => w.get(`[data-testid="${id}"]`);
  const text = (w: VueWrapper, id: string) => get(w, id).text();
  const isChecked = (w: VueWrapper, id: string) => (get(w, id).element as HTMLInputElement).checked;
  const isDisabled = (w: VueWrapper, id: string) =>
    (get(w, id).element as HTMLInputElement | HTMLButtonElement).disabled;
  const click = async (w: VueWrapper, id: string) => {
    await get(w, id).trigger("click");
    await flushPromises();
  };
  const emit = async (name: string, payloadValue?: unknown) => {
    events.handlers.get(name)?.({ payload: payloadValue });
    await flushPromises();
  };

  describe("shell", () => {
    it("renders nothing until the settings are loaded", async () => {
      const wrapper = mount(SetupView, { global: { plugins: [i18n] } });
      wrappers.push(wrapper);
      expect(find(wrapper, "setup-window").exists()).toBe(false);
      await flushPromises();
      expect(find(wrapper, "setup-window").exists()).toBe(true);
    });

    it("starts on the first page, with the three steps in the bar", async () => {
      const w = await mountWizard();
      expect(find(w, "agents-lead").exists()).toBe(true);
      expect(get(w, "step-agents").classes()).toContain("active");
      expect(get(w, "step-remote").classes()).not.toContain("active");
      expect(text(w, "step-agents")).toContain("Connect Agents");
      expect(text(w, "step-remote")).toContain("Remote answers");
      expect(text(w, "step-done")).toContain("Done");
    });

    it.each([
      ["remote", "remote-list"],
      ["done", "done-summary"],
      ["agents", "agents-lead"],
      ["bogus", "agents-lead"],
    ])("opens the page named by ?page=%s", async (page, marker) => {
      const w = await mountWizard(page);
      expect(find(w, marker).exists()).toBe(true);
    });

    it("moves an open window to the page an entry point asks for", async () => {
      const w = await mountWizard();
      await emit("setup-goto-page", "done");
      expect(find(w, "done-summary").exists()).toBe(true);
      expect(get(w, "step-done").classes()).toContain("active");
      await emit("setup-goto-page", "nonsense");
      expect(find(w, "agents-lead").exists()).toBe(true);
    });

    it("lets the step bar jump to any page", async () => {
      const w = await mountWizard();
      await click(w, "step-done");
      expect(find(w, "done-summary").exists()).toBe(true);
      await click(w, "step-remote");
      expect(find(w, "remote-list").exists()).toBe(true);
      await click(w, "step-agents");
      expect(find(w, "agents-lead").exists()).toBe(true);
    });

    it("always lands on a page's list when the page changes, not inside a channel form", async () => {
      const w = await mountWizard("remote");
      await click(w, "channel-open-telegram");
      expect(find(w, "channel-form-telegram").exists()).toBe(true);
      await click(w, "step-done");
      await click(w, "step-remote");
      expect(find(w, "remote-list").exists()).toBe(true);
    });

    it("refreshes the status and rescans when the backend says something changed", async () => {
      await mountWizard();
      const statusCalls = ipc.setupStatus.mock.calls.length;
      const scanCalls = ipc.setupAgentsScan.mock.calls.length;
      await emit("setup-status-changed");
      expect(ipc.setupStatus.mock.calls.length).toBe(statusCalls + 1);
      expect(ipc.setupAgentsScan.mock.calls.length).toBe(scanCalls + 1);
    });

    it("gives a step the user already settled a check instead of its number", async () => {
      statusValue = status({ agentsDecided: true, remoteDecided: true });
      const w = await mountWizard("remote");
      expect(get(w, "step-agents").classes()).toContain("done");
      expect(get(w, "step-agents").find(".setup-step-check").exists()).toBe(true);
      // The page being shown keeps its number, and Done is never settled before it is finished.
      expect(get(w, "step-remote").find(".setup-step-check").exists()).toBe(false);
      expect(text(w, "step-remote")).toContain("2");
      expect(get(w, "step-done").classes()).not.toContain("done");
    });

    it("keeps the numbers while nothing is settled", async () => {
      const w = await mountWizard();
      for (const name of ["agents", "remote", "done"]) {
        expect(get(w, `step-${name}`).find(".setup-step-check").exists()).toBe(false);
      }
    });

    it("follows a language change made in another window", async () => {
      const w = await mountWizard();
      await emit("settings-updated", { language: "zh" });
      expect(text(w, "step-agents")).toContain(zh.setup.steps.agents);
    });
  });

  describe("page 1: Agents", () => {
    it("prefills the table: integrated stays ticked, detected is ticked, undetected waits unticked", async () => {
      rowsValue = mixedRows();
      const w = await mountWizard();

      expect(isChecked(w, "agent-check-claude")).toBe(true);
      expect(isChecked(w, "agent-check-cursor")).toBe(true);
      expect(isChecked(w, "agent-check-grok")).toBe(true);
      expect(isChecked(w, "agent-check-codex")).toBe(true);
      // Blocked rows are never ticked.
      expect(isChecked(w, "agent-check-pi")).toBe(false);

      // The undetected Agent is folded away until asked for.
      expect(find(w, "agent-antigravity").exists()).toBe(false);
      expect(text(w, "agents-others-toggle")).toContain("Agents not detected (1)");
      await click(w, "agents-others-toggle");
      expect(find(w, "agents-others").exists()).toBe(true);
      expect(isChecked(w, "agent-check-antigravity")).toBe(false);
    });

    it("tags each row with its state", async () => {
      rowsValue = mixedRows();
      const w = await mountWizard();
      expect(text(w, "agent-tag-claude-integrated")).toBe("Connected · CLI");
      expect(text(w, "agent-tag-grok-integrated")).toBe("Connected · MCP");
      // The section title already says "Detected", so a tile does not repeat it.
      expect(find(w, "agent-tag-cursor-detected").exists()).toBe(false);
      expect(text(w, "agent-tag-grok-needsUpdate")).toBe("Update available");
      expect(text(w, "agent-tag-codex-untrusted")).toBe("Needs trust");
      expect(text(w, "agent-tag-pi-unreadable")).toBe("Cannot read state");
    });

    it("explains the Codex trust step and shows why an unreadable row is disabled", async () => {
      rowsValue = mixedRows();
      const w = await mountWizard();
      expect(text(w, "agent-trust-codex")).toContain("trust the AskHuman hook");
      expect(find(w, "agent-trust-claude").exists()).toBe(false);
      expect(isDisabled(w, "agent-check-pi")).toBe(true);
      expect(text(w, "agent-error-pi")).toContain("boom");
      expect(isDisabled(w, "agent-check-claude")).toBe(false);
    });

    // The way to connect is a two-button control; the pressed button is the chosen mode.
    const pressed = (w: VueWrapper, id: string) => get(w, id).attributes("aria-pressed") === "true";
    const modeLabels = (w: VueWrapper, id: string) =>
      get(w, id)
        .findAll("button")
        .map((b) => b.text());

    it("preselects the integrated mode, or the recommended one, and marks the recommended mode", async () => {
      rowsValue = mixedRows();
      const w = await mountWizard();
      expect(pressed(w, "agent-mode-claude-cli")).toBe(true);
      expect(pressed(w, "agent-mode-claude-mcp")).toBe(false);
      expect(modeLabels(w, "agent-mode-claude")).toEqual(["CLI (recommended)", "MCP"]);
      expect(pressed(w, "agent-mode-cursor-mcp")).toBe(true);
      expect(modeLabels(w, "agent-mode-cursor")).toEqual(["CLI", "MCP (recommended)"]);
      // An integrated Agent keeps its own mode even where it is not the recommended one.
      expect(pressed(w, "agent-mode-grok-mcp")).toBe(true);
      expect(pressed(w, "agent-mode-grok-cli")).toBe(false);
    });

    it("shows the only mode an Agent supports as a fixed label, not a choice", async () => {
      rowsValue = [row({ id: "grok", title: "Grok", supportsCli: false, recommended: "mcp" })];
      const w = await mountWizard();
      expect(find(w, "agent-mode-grok-cli").exists()).toBe(false);
      expect(find(w, "agent-mode-grok-mcp").exists()).toBe(false);
      expect(text(w, "agent-mode-grok")).toBe("MCP (recommended)");
    });

    it("choosing how to connect an Agent also ticks it", async () => {
      const w = await mountWizard();
      await get(w, "agent-check-claude").setValue(false);
      expect(isChecked(w, "agent-check-claude")).toBe(false);
      await click(w, "agent-mode-claude-mcp");
      expect(isChecked(w, "agent-check-claude")).toBe(true);
      expect(pressed(w, "agent-mode-claude-mcp")).toBe(true);
      expect(pressed(w, "agent-mode-claude-cli")).toBe(false);
    });

    it("lists the files an apply would touch below the tiles, only for rows that would change", async () => {
      rowsValue = mixedRows();
      const w = await mountWizard();
      // Folded away at first, with only the number of files on the toggle.
      expect(find(w, "agent-files-cursor").exists()).toBe(false);
      expect(text(w, "agents-files-toggle")).toContain("(2)");
      await click(w, "agents-files-toggle");
      const items = get(w, "agent-files-cursor").findAll("li");
      expect(items.map((li) => [li.get(".setup-file-kind").text(), li.get("code").text()])).toEqual([
        ["edit", "/u/.cursor/mcp.json"],
        ["new", "/u/.cursor/rules/askhuman.mdc"],
      ]);
      // Claude is integrated and current: nothing to show.
      expect(find(w, "agent-files-claude").exists()).toBe(false);
    });

    it("leaves out the file list when nothing would be written", async () => {
      const w = await mountWizard();
      expect(find(w, "agents-files").exists()).toBe(false);
    });

    it("greets a first run with a welcome and a rerun with the compact header", async () => {
      rowsValue = [row({ id: "claude", mode: "none" })];
      const first = await mountWizard();
      expect(first.get("h1").text()).toBe("Welcome to AskHuman");
      expect(text(first, "agents-lead")).toContain("Pick the Agents to connect");
      expect(first.find(".setup-app-icon").exists()).toBe(true);
      first.unmount();
      wrappers.length = 0;

      // Claude is already connected, so this is somebody coming back.
      rowsValue = [row({ id: "claude", mode: "cli" })];
      const rerun = await mountWizard();
      expect(rerun.get("h1").text()).toBe("Connect your Agents");
      expect(text(rerun, "agents-lead")).toContain("Your existing setup is kept");
      expect(rerun.find(".setup-app-icon").exists()).toBe(false);
    });

    it("the main button follows what would be written", async () => {
      rowsValue = mixedRows();
      const w = await mountWizard();
      // Cursor is new and Grok has an update waiting.
      expect(text(w, "agents-primary")).toBe("Apply and continue");

      await get(w, "agent-check-cursor").setValue(false);
      expect(text(w, "agents-primary")).toBe("Apply and continue");
      await get(w, "agent-check-grok").setValue(false);
      expect(text(w, "agents-primary")).toBe("Next");

      await get(w, "style-autonomous").setValue(true);
      expect(text(w, "agents-primary")).toBe("Apply and continue");
      await get(w, "style-aligned").setValue(true);
      expect(text(w, "agents-primary")).toBe("Next");
    });

    it("an integrated row that is unticked is left alone, never uninstalled", async () => {
      const w = await mountWizard();
      await get(w, "agent-check-claude").setValue(false);
      expect(text(w, "agents-primary")).toBe("Next");
      await click(w, "agents-primary");
      expect(ipc.setupApplyAgents).not.toHaveBeenCalled();
      expect(find(w, "remote-list").exists()).toBe(true);
    });

    it("offers Custom only while custom is what is set", async () => {
      const aligned = await mountWizard();
      expect(find(aligned, "style-aligned").exists()).toBe(true);
      expect(find(aligned, "style-autonomous").exists()).toBe(true);
      expect(find(aligned, "style-custom").exists()).toBe(false);
      expect(isChecked(aligned, "style-aligned")).toBe(true);
      aligned.unmount();
      wrappers.length = 0;

      payload = settings({ style: "custom" });
      const custom = await mountWizard();
      expect(find(custom, "style-custom").exists()).toBe(true);
      expect(isChecked(custom, "style-custom")).toBe(true);
      expect(text(custom, "agents-style")).toContain("Custom (edit in Settings)");
    });

    it("applies the style first, then the integrations, and shows each result in place", async () => {
      rowsValue = mixedRows();
      const applied = mixedRows();
      applied[1] = row({ ...applied[1], mode: "mcp" });
      applied[2] = row({ ...applied[2], needsUpdate: false });
      ipc.setupAgentsScan.mockResolvedValueOnce(rowsValue).mockResolvedValue(applied);
      ipc.setupApplyAgents.mockImplementation(async () => {
        order.push("apply");
        return [
          { agent: "cursor", requested: "mcp", ok: true, error: null, modeAfter: "mcp", delivery: null },
          { agent: "grok", requested: "mcp", ok: true, error: null, modeAfter: "mcp", delivery: null },
        ] satisfies SetupApplyResult[];
      });
      const w = await mountWizard();

      await get(w, "style-autonomous").setValue(true);
      await click(w, "agents-primary");

      expect(order).toEqual(["save", "applyStyle", "apply"]);
      expect(saves[0].config.general).toMatchObject({ collaborationStyle: "autonomous" });
      // Only the rows that would change, in table order.
      expect(ipc.setupApplyAgents).toHaveBeenCalledWith([
        { agent: "cursor", mode: "mcp" },
        { agent: "grok", mode: "mcp" },
      ]);
      expect(text(w, "agent-result-cursor")).toBe("✓ Connected · MCP");
      expect(text(w, "agent-result-grok")).toBe("✓ Connected · MCP");
      expect(text(w, "agents-restart")).toContain("must be restarted");

      // The page does not move on by itself: the button now says Continue.
      expect(find(w, "agents-lead").exists()).toBe(true);
      expect(text(w, "agents-primary")).toBe("Continue");
      await click(w, "agents-primary");
      expect(find(w, "remote-list").exists()).toBe(true);
    });

    it("applies only the style when no row would change", async () => {
      const w = await mountWizard();
      await get(w, "style-autonomous").setValue(true);
      await click(w, "agents-primary");
      expect(order).toEqual(["save", "applyStyle"]);
      expect(ipc.setupApplyAgents).not.toHaveBeenCalled();
      expect(text(w, "agents-primary")).toBe("Continue");
    });

    it("keeps a failed row ticked so it can be retried, and says why it failed", async () => {
      rowsValue = mixedRows();
      ipc.setupApplyAgents.mockImplementation(async () => [
        { agent: "cursor", requested: "mcp", ok: false, error: "disk full", modeAfter: "none", delivery: null },
        { agent: "grok", requested: "mcp", ok: true, error: null, modeAfter: "mcp", delivery: null },
      ]);
      const w = await mountWizard();
      await click(w, "agents-primary");
      expect(text(w, "agent-result-cursor")).toBe("✗ disk full");
      expect(text(w, "agent-result-grok")).toContain("✓");
      expect(isChecked(w, "agent-check-cursor")).toBe(true);
      expect(text(w, "agents-primary")).toBe("Apply and continue");
    });

    it("shows a backend error from applying and lets the user try again", async () => {
      rowsValue = mixedRows();
      ipc.setupApplyAgents.mockRejectedValueOnce(new Error("daemon is restarting"));
      const w = await mountWizard();
      await click(w, "agents-primary");
      expect(text(w, "agents-apply-error")).toContain("daemon is restarting");
      expect(isDisabled(w, "agents-primary")).toBe(false);
      expect(text(w, "agents-primary")).toBe("Apply and continue");
    });

    it("a Dev Instance sees the Agent area read-only", async () => {
      rowsValue = mixedRows();
      payload = settings({ readOnly: true });
      const w = await mountWizard();
      expect(find(w, "agents-readonly").exists()).toBe(true);
      expect(isDisabled(w, "agent-check-claude")).toBe(true);
      expect(isDisabled(w, "agent-check-cursor")).toBe(true);
      expect(isDisabled(w, "style-autonomous")).toBe(true);
      // Nothing to apply, but the page can still be left.
      expect(text(w, "agents-primary")).toBe("Next");
      await click(w, "agents-primary");
      expect(ipc.setupApplyAgents).not.toHaveBeenCalled();
      expect(find(w, "remote-list").exists()).toBe(true);
    });

    it("also treats the status flag as read-only", async () => {
      rowsValue = mixedRows();
      statusValue = status({ everydayIntegrationsReadOnly: true });
      const w = await mountWizard();
      expect(find(w, "agents-readonly").exists()).toBe(true);
      expect(text(w, "agents-primary")).toBe("Next");
    });

    it("skips the step only while the Agent question is open, and moves on", async () => {
      const w = await mountWizard();
      await click(w, "agents-skip");
      expect(ipc.setupSkip).toHaveBeenCalledWith("agents");
      expect(find(w, "remote-list").exists()).toBe(true);

      w.unmount();
      wrappers.length = 0;
      statusValue = status({ agentsDecided: true });
      const decided = await mountWizard();
      expect(find(decided, "agents-skip").exists()).toBe(false);
    });

    it("says so when no Agent was found, and still lets the user continue", async () => {
      rowsValue = [row({ id: "claude", detected: false })];
      const w = await mountWizard();
      expect(find(w, "agents-none").exists()).toBe(true);
      expect(find(w, "agents-main").exists()).toBe(false);
      expect(text(w, "agents-primary")).toBe("Next");
    });

    it("lets the user retry a scan that failed", async () => {
      ipc.setupAgentsScan.mockRejectedValueOnce(new Error("scan failed"));
      const w = await mountWizard();
      expect(text(w, "agents-scan-error")).toContain("scan failed");
      await get(w, "agents-scan-error").get("button").trigger("click");
      await flushPromises();
      expect(find(w, "agents-scan-error").exists()).toBe(false);
      expect(find(w, "agent-claude").exists()).toBe(true);
    });

    describe("environment cards", () => {
      it("a clean machine gets no card", async () => {
        rowsValue = mixedRows();
        const w = await mountWizard();
        for (const id of ["env-path", "env-transient", "env-installs"]) {
          expect(find(w, id).exists()).toBe(false);
        }
      });

      it("warns about a missing PATH only for an Agent that runs the CLI, with the fix to copy", async () => {
        envValue = env({
          askhumanOnPath: "missing",
          pathFix: { command: "ln -s /app/AskHuman ~/.local/bin/AskHuman", dir: "~/.local/bin", needsPathEdit: true },
        });
        const w = await mountWizard();
        expect(text(w, "env-path-command")).toBe("ln -s /app/AskHuman ~/.local/bin/AskHuman");
        expect(find(w, "env-path-edit").exists()).toBe(true);
      });

      it("does not warn about PATH when no CLI-mode Agent is integrated or about to be", async () => {
        rowsValue = [row({ id: "cursor", mode: "none", detected: true, recommended: "mcp" })];
        envValue = env({
          askhumanOnPath: "missing",
          pathFix: { command: "ln -s x y", dir: "d", needsPathEdit: false },
        });
        const w = await mountWizard();
        expect(find(w, "env-path").exists()).toBe(false);

        // Switching that row to CLI makes the missing PATH matter.
        await click(w, "agent-mode-cursor-cli");
        expect(find(w, "env-path").exists()).toBe(true);
        expect(find(w, "env-path-edit").exists()).toBe(false);
      });

      it("shows the temporary-location and several-installs cards when the check found them", async () => {
        envValue = env({
          transient: { path: "/Users/me/Downloads/AskHuman", kind: "downloads" },
          installs: { message: "Found AskHuman in 2 places" },
        });
        const w = await mountWizard();
        expect(text(w, "env-transient")).toContain("/Users/me/Downloads/AskHuman");
        expect(text(w, "env-installs")).toContain("Found AskHuman in 2 places");
      });

      it("shows nothing when the check itself failed", async () => {
        ipc.setupEnvCheck.mockRejectedValue(new Error("no"));
        const w = await mountWizard();
        expect(find(w, "agents-lead").exists()).toBe(true);
        expect(find(w, "env-path").exists()).toBe(false);
      });
    });
  });

  describe("page 2: remote answering", () => {
    it("lists the channels in the settings page's order and recommends only Feishu", async () => {
      const w = await mountWizard("remote");
      const ids = w.findAll('[data-testid^="channel-row-"]').map((el) => el.attributes("data-testid"));
      expect(ids).toEqual([
        "channel-row-feishu",
        "channel-row-telegram",
        "channel-row-dingding",
        "channel-row-slack",
      ]);
      expect(get(w, "channel-row-feishu").find(".setup-recommended").exists()).toBe(true);
      for (const id of ["telegram", "dingding", "slack"]) {
        expect(get(w, `channel-row-${id}`).find(".setup-recommended").exists()).toBe(false);
      }
    });

    it("names what each channel needs, leaving out the field auto-detect fills in", async () => {
      const w = await mountWizard("remote");
      // Telegram has no auto-detect: both fields are typed.
      expect(get(w, "channel-row-telegram").text()).toContain("Needs Bot Token · Chat ID");
      // Feishu's Open ID is filled in by auto-detect, so only the credentials are named.
      expect(get(w, "channel-row-feishu").text()).toContain("Needs App ID · App Secret");
      expect(get(w, "channel-row-feishu").text()).not.toContain("Open ID");
      // The alias in parentheses stays out of the short line.
      expect(get(w, "channel-row-dingding").text()).toContain("Needs ClientId · ClientSecret");
    });

    it("stops naming what a channel needs once it is set up", async () => {
      channelRows = [
        channelRow("feishu", { enabled: true, configured: true, connected: true }),
        channelRow("telegram", { enabled: false, configured: true }),
        // Incomplete: enabled but not configured, so it still says what is missing.
        channelRow("slack", { enabled: true, configured: false }),
      ];
      const w = await mountWizard("remote");
      expect(get(w, "channel-row-feishu").text()).not.toContain("Needs");
      expect(get(w, "channel-row-telegram").text()).not.toContain("Needs");
      expect(get(w, "channel-row-slack").text()).toContain("Needs Bot Token · App-Level Token");
      expect(get(w, "channel-row-dingding").text()).toContain("Needs ClientId · ClientSecret");
    });

    it("shows each channel's state", async () => {
      channelRows = [
        channelRow("telegram"),
        channelRow("slack", { enabled: true, configured: true, connected: true }),
        channelRow("dingding", { enabled: false, configured: true }),
        channelRow("feishu", { enabled: true, configured: false }),
      ];
      const w = await mountWizard("remote");
      expect(text(w, "channel-state-telegram")).toBe("Not set up");
      expect(text(w, "channel-state-slack")).toBe("Set up · connected");
      expect(text(w, "channel-state-dingding")).toBe("Set up (off)");
      expect(text(w, "channel-state-feishu")).toBe("Incomplete");
      // A configured channel is edited, a new one is set up.
      expect(text(w, "channel-open-telegram")).toBe("Set up");
      expect(text(w, "channel-open-dingding")).toBe("Edit");
    });

    it("an enabled channel that is running without a daemon is plain set up, not connected", async () => {
      channelRows = [channelRow("telegram", { enabled: true, configured: true, connected: null })];
      const w = await mountWizard("remote");
      expect(text(w, "channel-state-telegram")).toBe("Set up");
    });

    it("shows a known channel fault the same way the settings page does", async () => {
      channelRows = [channelRow("feishu", { enabled: true, configured: true, connected: false })];
      health = [{ channel: "feishu", message: "auth failed", atMs: Date.now() }];
      const w = await mountWizard("remote");
      expect(text(w, "channel-state-feishu")).toBe("Problem");
      expect(text(w, "channel-row-feishu")).toContain("auth failed");
    });

    it("lists iOS only while the experimental features are on, and opens its Settings card", async () => {
      const off = await mountWizard("remote");
      expect(find(off, "channel-row-ios").exists()).toBe(false);
      off.unmount();
      wrappers.length = 0;

      payload = settings({ experimental: true });
      const on = await mountWizard("remote");
      expect(find(on, "channel-row-ios").exists()).toBe(true);
      await click(on, "channel-open-ios");
      expect(ipc.openSettings).toHaveBeenCalledWith("experimental");
    });

    it("offers only Skip while no channel is on, and Skip records the decision and moves on", async () => {
      // Where no Agent task can start, there is no screen to stop at: Skip goes straight to Done.
      platform.supportsAgentTasks = false;
      const w = await mountWizard("remote");
      expect(find(w, "remote-continue").exists()).toBe(false);
      await click(w, "remote-skip");
      expect(ipc.setupSkip).toHaveBeenCalledWith("remote");
      expect(find(w, "done-summary").exists()).toBe(true);
    });

    it("moves on to the Agent-task screen from Skip, which needs no channel", async () => {
      const w = await mountWizard("remote");
      await click(w, "remote-skip");
      expect(ipc.setupSkip).toHaveBeenCalledWith("remote");
      expect(find(w, "tasks-page").exists()).toBe(true);
    });

    it("offers Continue, without recording a skip, once a channel is on", async () => {
      channelRows = [channelRow("slack", { enabled: true, configured: true })];
      // Where no Agent task can start, there is no offer to stop at: Continue goes straight to Done.
      platform.supportsAgentTasks = false;
      const w = await mountWizard("remote");
      expect(find(w, "remote-skip").exists()).toBe(false);
      await click(w, "remote-continue");
      expect(ipc.setupSkip).not.toHaveBeenCalled();
      expect(find(w, "done-summary").exists()).toBe(true);
    });

    describe("a channel's detail", () => {
      async function openTelegram(w: VueWrapper) {
        await click(w, "channel-open-telegram");
        expect(find(w, "channel-form-telegram").exists()).toBe(true);
      }

      it("shows the platform-side key points for that channel", async () => {
        const w = await mountWizard("remote");
        await openTelegram(w);
        expect(get(w, "channel-points").findAll("li")).toHaveLength(3);
      });

      it("saves the credentials and turns the channel on through the shared save", async () => {
        const w = await mountWizard("remote");
        await openTelegram(w);
        await get(w, "field-telegram-botToken").setValue("123:abc");
        await get(w, "field-telegram-chatId").setValue("42");
        const refreshes = ipc.setupChannelStatus.mock.calls.length;
        await click(w, "channel-save");

        expect(saves).toHaveLength(1);
        expect(saves[0].config.channels.telegram).toMatchObject({ enabled: true, botToken: "123:abc", chatId: "42" });
        expect(saves[0].actions.telegramToken).toEqual({ kind: "set", value: "123:abc" });
        expect(find(w, "channel-saved").exists()).toBe(true);
        // The list's states are refreshed after the save.
        expect(ipc.setupChannelStatus.mock.calls.length).toBeGreaterThan(refreshes);
      });

      it("counts a secret that is already stored as filled", async () => {
        payload = settings({ secretsPresent: { telegramToken: true } });
        const w = await mountWizard("remote");
        await openTelegram(w);
        expect((get(w, "field-telegram-botToken").element as HTMLInputElement).placeholder).toBe("••••••••");
        await get(w, "field-telegram-chatId").setValue("42");
        await click(w, "channel-save");
        expect(find(w, "channel-missing").exists()).toBe(false);
        expect(saves).toHaveLength(1);
        expect(saves[0].actions.telegramToken).toEqual({ kind: "unchanged" });
        expect(saves[0].config.channels.telegram.enabled).toBe(true);
      });

      it("refuses to save with a required field empty, and does not turn the channel on", async () => {
        const w = await mountWizard("remote");
        await openTelegram(w);
        await get(w, "field-telegram-botToken").setValue("123:abc");
        await click(w, "channel-save");
        expect(text(w, "channel-missing")).toBe("Fill in every required field first.");
        expect(saves).toHaveLength(0);
        expect(find(w, "channel-saved").exists()).toBe(false);
      });

      it("takes the channel back off when the save fails, so no later save carries it along", async () => {
        ipc.saveSettings.mockRejectedValueOnce(new Error("config is read-only"));
        const w = await mountWizard("remote");
        await openTelegram(w);
        await get(w, "field-telegram-botToken").setValue("123:abc");
        await get(w, "field-telegram-chatId").setValue("42");
        await click(w, "channel-save");
        expect(find(w, "channel-saved").exists()).toBe(false);
        expect(w.text()).toContain("config is read-only");

        // Done saves the whole config again: the half-done enable must not be in it.
        await click(w, "step-done");
        await click(w, "finish");
        expect(saves).toHaveLength(1);
        expect(saves[0].config.channels.telegram.enabled).toBe(false);
      });

      it("a retry after a failed save turns the channel on", async () => {
        ipc.saveSettings.mockRejectedValueOnce(new Error("config is read-only"));
        const w = await mountWizard("remote");
        await openTelegram(w);
        await get(w, "field-telegram-botToken").setValue("123:abc");
        await get(w, "field-telegram-chatId").setValue("42");
        await click(w, "channel-save");
        await click(w, "channel-save");
        expect(saves).toHaveLength(1);
        expect(saves[0].config.channels.telegram).toMatchObject({ enabled: true, botToken: "123:abc" });
        expect(find(w, "channel-saved").exists()).toBe(true);
        expect(w.text()).not.toContain("config is read-only");
      });

      it("tests the connection with the same call the settings page makes", async () => {
        const w = await mountWizard("remote");
        await openTelegram(w);
        await get(w, "field-telegram-botToken").setValue("123:abc");
        await get(w, "field-telegram-chatId").setValue("42");
        await click(w, "channel-test");
        expect(ipc.telegramTest).toHaveBeenCalledWith({ botToken: "123:abc", chatId: "42", apiBaseUrl: "" });
        expect(text(w, "channel-message")).toBe("Message delivered");
      });

      it("only channels with an auto-detect show the button", async () => {
        const w = await mountWizard("remote");
        await openTelegram(w);
        expect(find(w, "detect-telegram").exists()).toBe(false);
        await click(w, "channel-back");
        await click(w, "channel-open-slack");
        expect(find(w, "detect-slack").exists()).toBe(true);
        expect(get(w, "channel-points").findAll("li")).toHaveLength(5);
      });

      it("opens the channel's guide and its card in Settings", async () => {
        const w = await mountWizard("remote");
        await openTelegram(w);
        await click(w, "channel-guide");
        expect(ipc.openPath).toHaveBeenCalledWith(expect.stringContaining("telegram-setup.en.md"));
        await click(w, "channel-more");
        expect(ipc.openSettings).toHaveBeenCalledWith("channel#channel-telegram");
      });

      it("goes back to the list", async () => {
        const w = await mountWizard("remote");
        await openTelegram(w);
        await click(w, "channel-back");
        expect(find(w, "remote-list").exists()).toBe(true);
        expect(find(w, "channel-form-telegram").exists()).toBe(false);
      });
    });
  });

  describe("the Agent-task screen", () => {
    function enabledSlack() {
      channelRows = [channelRow("slack", { enabled: true, configured: true })];
    }

    /** Leave the remote step the way the page lets this person: Continue with an IM channel, else Skip. */
    async function leaveRemoteStep(w: VueWrapper) {
      await click(w, find(w, "remote-continue").exists() ? "remote-continue" : "remote-skip");
    }

    /** Arrive at the screen with an IM channel on and the default Agent and project; the check has answered. */
    async function openScreen() {
      enabledSlack();
      const w = await mountWizard("remote");
      await leaveRemoteStep(w);
      return w;
    }

    const stage = (w: VueWrapper) => get(w, "tasks-page").attributes("data-stage");

    it("is the screen after the remote step when an Agent can start a task and the platform allows it", async () => {
      const w = await openScreen();
      expect(find(w, "tasks-page").exists()).toBe(true);
      expect(find(w, "done-summary").exists()).toBe(false);
      // It belongs to the remote step: the bar keeps showing that one.
      expect(get(w, "step-remote").classes()).toContain("active");
      expect(get(w, "step-done").classes()).not.toContain("active");
      expect(stage(w)).toBe("ready");
      expect(text(w, "tasks-later")).toBe(message("setup.tasks.later"));
      expect(text(w, "tasks-enable")).toBe(message("setup.tasks.enable"));
      // Arriving turns nothing on.
      expect(saves).toHaveLength(0);
    });

    it("needs no remote channel: Skip with nothing set up still lands on it, and the lead says so", async () => {
      const w = await mountWizard("remote");
      await click(w, "remote-skip");
      expect(ipc.setupSkip).toHaveBeenCalledWith("remote");
      expect(find(w, "tasks-page").exists()).toBe(true);
      expect(text(w, "tasks-lead")).toBe(message("setup.tasks.leadNone"));
      // No IM channel, so no IM bot to warn about.
      expect(find(w, "tasks-bot").exists()).toBe(false);
    });

    it("checks first and asks second: Turn on waits for the check", async () => {
      let release: (items: AgentTaskReadiness[]) => void = () => {};
      ipc.agentTaskReadiness.mockImplementation(
        () =>
          new Promise<AgentTaskReadiness[]>((resolve) => {
            release = resolve;
          }),
      );
      const w = await openScreen();
      expect(stage(w)).toBe("checking");
      expect(find(w, "tasks-checking").exists()).toBe(true);
      expect(find(w, "tasks-offer").exists()).toBe(false);
      expect(isDisabled(w, "tasks-enable")).toBe(true);

      release([readiness("claude")]);
      await flushPromises();
      expect(stage(w)).toBe("ready");
      expect(find(w, "tasks-offer").exists()).toBe(true);
      expect(isDisabled(w, "tasks-enable")).toBe(false);
      // It read what the phone's page reads: the refreshed project list and the backend's verdicts.
      expect(ipc.agentTaskWorkspaces).toHaveBeenCalledWith(true);
      expect(ipc.agentTaskReadiness).toHaveBeenCalledWith(undefined);
    });

    it("says what it costs, and names the IM bot only when an IM channel is on", async () => {
      const withBot = await openScreen();
      expect(text(withBot, "tasks-cost")).toContain(message("setup.tasks.cost"));
      expect(text(withBot, "tasks-bot")).toContain(message("setup.tasks.bot"));
      withBot.unmount();
      wrappers.length = 0;

      channelRows = noChannels();
      payload = settings({ ios: true });
      const phoneOnly = await mountWizard("remote");
      await leaveRemoteStep(phoneOnly);
      expect(find(phoneOnly, "tasks-cost").exists()).toBe(true);
      expect(find(phoneOnly, "tasks-bot").exists()).toBe(false);
    });

    it.each([
      ["only the iOS app is on", { ios: true }, false, "leadIos"],
      ["only an IM channel is on", {}, true, "leadIm"],
      ["both are on", { ios: true }, true, "leadBoth"],
      ["nothing is set up yet", {}, false, "leadNone"],
    ])("words the lead for the way the phone starts a task when %s", async (_name, over, im, key) => {
      payload = settings(over);
      if (im) enabledSlack();
      const w = await mountWizard("remote");
      await leaveRemoteStep(w);
      expect(text(w, "tasks-lead")).toBe(message(`setup.tasks.${key}`));
    });

    it("decides from what is saved when the step is left, not from what this window loaded", async () => {
      const w = await mountWizard("remote");
      // The iOS app is set up in a Settings window while the wizard stays open.
      payload = settings({ ios: true });
      await click(w, "remote-skip");
      expect(find(w, "tasks-page").exists()).toBe(true);
      expect(text(w, "tasks-lead")).toBe(message("setup.tasks.leadIos"));
    });

    it.each([
      [
        "the feature is already on",
        () => {
          payload = settings({ agentTasks: true, daemonLifecycle: "keepalive" });
        },
      ],
      [
        "the person turned it down on an earlier run",
        () => {
          statusValue = status({ phoneTasksDeclined: true });
        },
      ],
      [
        "no Agent is connected",
        () => {
          rowsValue = [row({ id: "claude", mode: "none" })];
        },
      ],
      [
        "only Antigravity is connected, which cannot start a task",
        () => {
          rowsValue = [row({ id: "antigravity", title: "Antigravity", mode: "mcp" })];
        },
      ],
      [
        "the platform cannot run Agent tasks",
        () => {
          platform.supportsAgentTasks = false;
        },
      ],
    ])("is skipped, straight to Done, when %s", async (_name, arrange) => {
      enabledSlack();
      arrange();
      const w = await mountWizard("remote");
      await click(w, "remote-continue");
      expect(find(w, "tasks-page").exists()).toBe(false);
      expect(find(w, "done-summary").exists()).toBe(true);
    });

    it("is not asked when Settings turned the feature on meanwhile, and keeps it on", async () => {
      const w = await mountWizard("remote");
      payload = settings({ ios: true, agentTasks: true, daemonLifecycle: "keepalive" });
      await click(w, "remote-skip");
      expect(find(w, "tasks-page").exists()).toBe(false);
      expect(text(w, "summary-tasks-state")).toBe(message("setup.done.tasksOn"));
      await click(w, "finish");
      expect(saves[0].config.agentTasks).toMatchObject({ enabled: true });
    });

    describe("what it checks", () => {
      it("lists the Agents a task can start on, each with where it would run", async () => {
        readinessValue = [
          readiness("claude"),
          // Neither has a CLI here, but the Codex app and Cursor's Agents window start the task: the
          // phone lists them, so the wizard must too.
          readiness("codex", { binaryReady: false, ready: false, startable: true, startRoute: "desktop" }),
          readiness("cursor", { binaryReady: false, ready: false, startable: true, startRoute: "desktop" }),
          // Not integrated, and one that cannot start tasks at all: neither is the person's to fix here.
          readiness("grok", { integrationMode: "none", ready: false }),
          readiness("antigravity", { launchSupported: false, ready: false }),
        ];
        const w = await openScreen();
        expect(stage(w)).toBe("ready");
        const listed = get(w, "tasks-startable")
          .findAll('[data-testid^="tasks-startable-"]')
          .map((chip) => chip.attributes("data-testid"));
        expect(listed).toEqual(["tasks-startable-claude", "tasks-startable-codex", "tasks-startable-cursor"]);
        expect(get(w, "tasks-startable-claude").get("em").text()).toBe(message("setup.tasks.route.terminal"));
        expect(get(w, "tasks-startable-codex").get("em").text()).toBe(message("setup.tasks.route.codexApp"));
        expect(get(w, "tasks-startable-cursor").get("em").text()).toBe(message("setup.tasks.route.cursorWindow"));
        expect(find(w, "tasks-not-yet").exists()).toBe(false);
      });

      it("takes the backend's verdict over the CLI's, falling back to the CLI only without one", async () => {
        // The CLI is ready but the backend says no route can start: that is the answer.
        readinessValue = [readiness("claude", { ready: true, startable: false })];
        const w = await openScreen();
        expect(stage(w)).toBe("blocked");
        w.unmount();
        wrappers.length = 0;

        // An older build gives no verdict: the CLI's readiness stands in for it.
        readinessValue = [readiness("claude")];
        const old = await openScreen();
        expect(stage(old)).toBe("ready");
      });

      it("says quietly which set-up Agents cannot start yet, and why", async () => {
        readinessValue = [
          readiness("claude"),
          readiness("cursor", {
            ready: false,
            binaryReady: false,
            startable: false,
            startRoute: null,
            launch: {
              preference: "terminal",
              target: null,
              desktopAvailable: false,
              terminalAvailable: false,
              desktopReason: "appNotRunning",
              terminalReason: "cliNotDetected",
              integrated: true,
            },
          }),
        ];
        const w = await openScreen();
        // One Agent can start, so this is still an offer; the other is only mentioned.
        expect(stage(w)).toBe("ready");
        const note = text(w, "tasks-not-yet");
        expect(note).toContain("Cursor");
        expect(note).toContain(message("cursorDesktop.launch.reasons.cliNotDetected"));
        expect(note).toContain(message("cursorDesktop.launch.reasons.appNotRunning"));
        expect(isDisabled(w, "tasks-enable")).toBe(false);
      });

      it("shows the first three projects the phone can pick, then a count", async () => {
        workspacesValue = [
          workspace("/u/dev/alpha"),
          workspace("/u/dev/beta"),
          workspace("/u/dev/gamma"),
          workspace("/u/dev/delta"),
          workspace("/u/dev/stale", { hidden: true }),
        ];
        const w = await openScreen();
        const projects = get(w, "tasks-projects");
        expect(projects.findAll(".check-chip").map((chip) => chip.text())).toEqual(["alpha", "beta", "gamma"]);
        expect(projects.get(".check-none").text()).toBe("1 more");
      });

      it("treats missing projects as a hint only: the offer stays", async () => {
        workspacesValue = [workspace("/u/dev/stale", { hidden: true })];
        const w = await openScreen();
        expect(stage(w)).toBe("ready");
        expect(text(w, "tasks-projects")).toBe(message("setup.tasks.projectsNone"));
        expect(isDisabled(w, "tasks-enable")).toBe(false);
      });

      it("says it could not check, and nothing is wrong, when the reads fail", async () => {
        ipc.agentTaskWorkspaces.mockRejectedValue(new Error("no index"));
        ipc.agentTaskReadiness.mockRejectedValue(new Error("no shell"));
        const w = await openScreen();
        expect(stage(w)).toBe("unknown");
        expect(text(w, "tasks-unknown")).toContain(message("setup.tasks.unknownAgents"));
        expect(text(w, "tasks-projects")).toBe(message("setup.tasks.unknown"));
        expect(find(w, "tasks-blocked").exists()).toBe(false);
        // Nothing is known to be wrong, so the person can still decide.
        expect(isDisabled(w, "tasks-enable")).toBe(false);
      });

      it("still reports the Agents when only the project read fails", async () => {
        ipc.agentTaskWorkspaces.mockRejectedValue(new Error("no index"));
        const w = await openScreen();
        expect(stage(w)).toBe("ready");
        expect(text(w, "tasks-projects")).toBe(message("setup.tasks.unknown"));
        expect(find(w, "tasks-startable-claude").exists()).toBe(true);
      });

      it("looks again when the window is focused again, so what was fixed elsewhere shows up", async () => {
        workspacesValue = [];
        const w = await openScreen();
        expect(text(w, "tasks-projects")).toBe(message("setup.tasks.projectsNone"));

        // The person added a project in a Settings window and comes back.
        workspacesValue = [workspace("/u/dev/alpha")];
        window.dispatchEvent(new Event("focus"));
        await flushPromises();
        expect(text(w, "tasks-projects")).toBe("alpha");
        // The Agents' CLIs are looked up again, not taken from the last lookup.
        expect(ipc.agentTaskReadiness).toHaveBeenLastCalledWith({ force: true });
      });

      it("stops looking on focus once the screen is left", async () => {
        const w = await openScreen();
        await click(w, "tasks-enable");
        const calls = ipc.agentTaskReadiness.mock.calls.length;
        window.dispatchEvent(new Event("focus"));
        await flushPromises();
        expect(ipc.agentTaskReadiness.mock.calls.length).toBe(calls);
      });
    });

    describe("when no Agent can start a task", () => {
      beforeEach(() => {
        readinessValue = [
          readiness("claude", { binaryReady: false, ready: false }),
          readiness("antigravity", { launchSupported: false, ready: false }),
        ];
      });

      it("says why for each Agent, and offers nothing to turn on", async () => {
        const w = await openScreen();
        expect(stage(w)).toBe("blocked");
        expect(text(w, "tasks-lead")).toBe(message("setup.tasks.blockedLead"));
        expect(text(w, "tasks-blocked-claude")).toContain(
          "The claude command was not found; install Claude Code's command-line tool first",
        );
        // An Agent that cannot start tasks at all is not a reason: there is nothing to fix.
        expect(find(w, "tasks-blocked-antigravity").exists()).toBe(false);
        expect(find(w, "tasks-enable").exists()).toBe(false);
        expect(find(w, "tasks-later").exists()).toBe(false);
        expect(find(w, "tasks-offer").exists()).toBe(false);
        expect(text(w, "tasks-blocked-hint")).toBe(message("setup.tasks.blockedHint"));
      });

      it("explains Codex and Cursor by both of their routes", async () => {
        readinessValue = [
          readiness("codex", {
            ready: false,
            binaryReady: false,
            startable: false,
            launch: {
              preference: "desktop",
              target: null,
              desktopAvailable: false,
              terminalAvailable: false,
              desktopReason: "appNotDetected",
              terminalReason: "cliNotDetected",
              integrated: true,
            },
          }),
        ];
        const w = await openScreen();
        expect(stage(w)).toBe("blocked");
        // Neither the app nor the CLI is there: said once, as the one thing to fix.
        expect(text(w, "tasks-blocked-codex")).toContain(message("desktop.noneDetectedReason"));
      });

      it("names no Agent at all when none is set up", async () => {
        readinessValue = [readiness("claude", { integrationMode: "none", ready: false })];
        const w = await openScreen();
        expect(stage(w)).toBe("blocked");
        expect(text(w, "tasks-blocked-none")).toContain(message("setup.tasks.agentsNone"));
      });

      it("Skip goes on to Done without recording a decline, so a rerun asks again", async () => {
        const w = await openScreen();
        await click(w, "tasks-skip");
        expect(find(w, "done-summary").exists()).toBe(true);
        expect(ipc.setupSkip).not.toHaveBeenCalledWith("phoneTasks");
        expect(text(w, "summary-tasks-state")).toBe(message("setup.done.tasksOff"));
        await click(w, "finish");
        expect(saves[0].config.agentTasks).toMatchObject({ enabled: false });
      });

      it("Check again looks afresh, and the offer appears once something can start", async () => {
        const w = await openScreen();
        expect(find(w, "tasks-enable").exists()).toBe(false);

        readinessValue = [readiness("claude")];
        await click(w, "tasks-recheck");
        expect(ipc.agentTaskReadiness).toHaveBeenLastCalledWith({ force: true });
        expect(stage(w)).toBe("ready");
        expect(isDisabled(w, "tasks-enable")).toBe(false);
      });

      it("checks again by itself when the window is focused again", async () => {
        const w = await openScreen();
        readinessValue = [readiness("claude")];
        window.dispatchEvent(new Event("focus"));
        await flushPromises();
        expect(stage(w)).toBe("ready");
      });
    });

    describe("Turn on", () => {
      it("goes straight on to Done, and saves only there", async () => {
        const w = await openScreen();
        await click(w, "tasks-enable");
        expect(find(w, "tasks-page").exists()).toBe(false);
        expect(find(w, "done-summary").exists()).toBe(true);
        expect(saves).toHaveLength(0);
        expect(text(w, "summary-tasks-state")).toBe(message("setup.done.tasksOn"));
        // The feature pins the keep-running switch on, and Done saves both.
        expect(isChecked(w, "switch-b")).toBe(true);
        expect(isDisabled(w, "switch-b")).toBe(true);
        expect(text(w, "done-background")).toContain(message("setup.done.switchBLocked"));
        await click(w, "finish");
        expect(saves).toHaveLength(1);
        expect(saves[0].config.agentTasks).toMatchObject({ enabled: true });
        expect(saves[0].config.general).toMatchObject({ daemonLifecycle: "keepalive" });
      });

      it("works with no remote channel yet: Done says so, and still pins the keep-running switch", async () => {
        const w = await mountWizard("remote");
        await click(w, "remote-skip");
        await click(w, "tasks-enable");
        expect(text(w, "summary-tasks-state")).toBe(message("setup.done.tasksOnNoChannel"));
        // No channel is on, but the feature needs a resident daemon all the same.
        expect(isChecked(w, "switch-b")).toBe(true);
        expect(isDisabled(w, "switch-b")).toBe(true);
        await click(w, "finish");
        expect(saves[0].config.agentTasks).toMatchObject({ enabled: true });
        expect(saves[0].config.general).toMatchObject({ daemonLifecycle: "keepalive" });
      });
    });

    describe("Not now", () => {
      it("keeps the feature off, remembers the answer and moves on to Done", async () => {
        const w = await openScreen();
        await click(w, "tasks-later");
        expect(ipc.setupSkip).toHaveBeenCalledWith("phoneTasks");
        expect(find(w, "done-summary").exists()).toBe(true);
        expect(text(w, "summary-tasks-state")).toBe(message("setup.done.tasksOff"));
        await click(w, "finish");
        expect(saves[0].config.agentTasks).toMatchObject({ enabled: false });
        expect(saves[0].config.general).toMatchObject({ daemonLifecycle: "activity" });
      });

      it("moves on even when the answer cannot be remembered", async () => {
        ipc.setupSkip.mockImplementation(async (domain: string) => {
          if (domain === "phoneTasks") throw new Error("disk full");
        });
        const w = await openScreen();
        await click(w, "tasks-later");
        expect(find(w, "done-summary").exists()).toBe(true);
        expect(text(w, "summary-tasks-state")).toBe(message("setup.done.tasksOff"));
      });

      it("is not asked again on a rerun, though Done still offers the way back", async () => {
        enabledSlack();
        statusValue = status({ phoneTasksDeclined: true });
        const w = await mountWizard("remote");
        await click(w, "remote-continue");
        expect(find(w, "tasks-page").exists()).toBe(false);
        await click(w, "summary-tasks-link");
        expect(find(w, "tasks-page").exists()).toBe(true);
      });
    });

    describe("from the step bar and the Done page", () => {
      it("jumping straight to Done leaves the question to the summary row", async () => {
        const w = await openScreen();
        await click(w, "step-done");
        expect(find(w, "tasks-page").exists()).toBe(false);
        expect(text(w, "summary-tasks-state")).toBe(message("setup.done.tasksOff"));
        // Nothing was turned on or turned down by leaving without an answer.
        expect(ipc.setupSkip).not.toHaveBeenCalledWith("phoneTasks");
        await click(w, "finish");
        expect(saves[0].config.agentTasks).toMatchObject({ enabled: false });
      });

      it("Done's row goes back to the screen, and Turn on there is saved at Done", async () => {
        enabledSlack();
        const w = await mountWizard("done");
        await click(w, "summary-tasks-link");
        expect(find(w, "tasks-page").exists()).toBe(true);
        expect(get(w, "step-remote").classes()).toContain("active");
        await click(w, "tasks-enable");
        expect(text(w, "summary-tasks-state")).toBe(message("setup.done.tasksOn"));
        await click(w, "finish");
        expect(saves[0].config.agentTasks).toMatchObject({ enabled: true });
        expect(saves[0].config.general).toMatchObject({ daemonLifecycle: "keepalive" });
      });

      it("a feature that is already on opens on the same check, and the step bar still leaves it", async () => {
        enabledSlack();
        payload = settings({ agentTasks: true, daemonLifecycle: "keepalive" });
        const w = await mountWizard("done");
        await click(w, "summary-tasks-link");
        expect(find(w, "tasks-page").exists()).toBe(true);
        expect(stage(w)).toBe("ready");
        await click(w, "step-remote");
        expect(find(w, "remote-list").exists()).toBe(true);
      });
    });
  });

  describe("page 3: done", () => {
    function enabledSlack() {
      channelRows = [channelRow("slack", { enabled: true, configured: true })];
    }

    /** The test ids of the summary chips that start with a prefix, in page order. */
    const chipIds = (w: VueWrapper, prefix: string) =>
      w.findAll(`[data-testid^="${prefix}"]`).map((chip) => chip.attributes("data-testid"));

    it("summarises the Agents, the style, the channels and what is switched on", async () => {
      rowsValue = mixedRows();
      payload = settings({ style: "autonomous" });
      channelRows = [
        channelRow("slack", { enabled: true, configured: true }),
        channelRow("feishu", { enabled: true, configured: true }),
        channelRow("telegram", { enabled: false, configured: true }),
      ];
      capabilities = {
        lifecycle: { supported: true, enabled: true },
        permission: { supported: true, enabled: false },
        stop: { supported: true, enabled: true },
      };
      const w = await mountWizard("done");
      // One chip per connected Agent, in the scan's order, each with the way it is connected.
      expect(chipIds(w, "summary-agent-")).toEqual([
        "summary-agent-claude",
        "summary-agent-grok",
        "summary-agent-codex",
      ]);
      expect(text(w, "summary-agent-claude")).toContain("Claude Code");
      expect(get(w, "summary-agent-claude").get("em").text()).toBe("CLI");
      expect(get(w, "summary-agent-grok").get("em").text()).toBe("MCP");
      expect(get(w, "summary-agent-codex").get("em").text()).toBe("CLI");
      expect(text(w, "summary-style")).toBe(message("settings.integration.collabAutonomous"));
      // Only channels that are on, in the order the remote page lists them.
      expect(chipIds(w, "summary-channel-")).toEqual(["summary-channel-feishu", "summary-channel-slack"]);
      expect(text(w, "summary-channel-slack")).toContain(message("settings.channels.slackTitle"));
      expect(text(w, "summary-channel-feishu")).toContain(message("settings.channels.feishuTitle"));
      expect(text(w, "summary-capabilities")).toContain("Lifecycle tracking · Stop confirmation");
      await click(w, "summary-adjust");
      expect(ipc.openSettings).toHaveBeenCalledWith("integration");
    });

    it("says none yet when nothing is configured, and hides the capabilities row", async () => {
      rowsValue = [row({ id: "claude", mode: "none" })];
      const w = await mountWizard("done");
      expect(text(w, "summary-agents")).toBe("None yet");
      expect(text(w, "summary-channels")).toBe(message("setup.done.channelsLater"));
      expect(chipIds(w, "summary-agent-")).toEqual([]);
      expect(chipIds(w, "summary-channel-")).toEqual([]);
      expect(find(w, "summary-capabilities").exists()).toBe(false);
    });

    it("still has the summary when the page is opened directly, before any scan", async () => {
      rowsValue = mixedRows();
      const w = await mountWizard("done");
      expect(text(w, "summary-agent-claude")).toContain("Claude Code");
      expect(get(w, "summary-agent-claude").get("em").text()).toBe("CLI");
    });

    it("offers the launch-at-login switch only where there is a tray, from the menu-bar mode", async () => {
      payload = settings({ menuBarIcon: "always" });
      const w = await mountWizard("done");
      expect(isChecked(w, "switch-a")).toBe(true);
      w.unmount();
      wrappers.length = 0;

      statusValue = status({ trayAvailable: false });
      const noTray = await mountWizard("done");
      expect(find(noTray, "switch-a").exists()).toBe(false);
      // Neither switch applies: the whole card goes.
      expect(find(noTray, "done-background").exists()).toBe(false);
    });

    it("offers the keep-running switch only once a channel is on, from the lifecycle mode", async () => {
      payload = settings({ daemonLifecycle: "keepalive" });
      const without = await mountWizard("done");
      expect(find(without, "switch-b").exists()).toBe(false);
      without.unmount();
      wrappers.length = 0;

      enabledSlack();
      const withChannel = await mountWizard("done");
      expect(isChecked(withChannel, "switch-b")).toBe(true);
    });

    it("keeps the keep-running switch on while the Agent-task feature depends on it", async () => {
      enabledSlack();
      payload = settings({ agentTasks: true });
      const w = await mountWizard("done");
      expect(isDisabled(w, "switch-b")).toBe(true);
    });

    describe("the iOS app as a remote channel", () => {
      it("lists the iOS app next to the IM channels that are on", async () => {
        enabledSlack();
        payload = settings({ ios: true });
        const w = await mountWizard("done");
        expect(chipIds(w, "summary-channel-")).toEqual(["summary-channel-slack", "summary-channel-ios"]);
        expect(text(w, "summary-channel-ios")).toContain(message("setup.remote.ios"));
      });

      it("is a remote channel by itself: the summary has it and the keep-running switch is offered", async () => {
        payload = settings({ ios: true, daemonLifecycle: "keepalive" });
        const w = await mountWizard("done");
        expect(chipIds(w, "summary-channel-")).toEqual(["summary-channel-ios"]);
        expect(text(w, "summary-channels")).not.toContain(message("setup.done.channelsLater"));
        expect(isChecked(w, "switch-b")).toBe(true);
        expect(text(w, "done-background")).toContain(message("setup.done.switchBDesc"));
      });
    });

    describe("the Agent-task row: reported here, decided on the Agent-task screen", () => {
      it("shows Off with a way to the screen where it is on offer, and no switch for it", async () => {
        enabledSlack();
        const w = await mountWizard("done");
        expect(text(w, "summary-tasks-state")).toBe(message("setup.done.tasksOff"));
        expect(get(w, "summary-tasks").classes()).toContain("none");
        expect(text(w, "summary-tasks-link")).toContain(message("setup.done.tasksEnable"));
        // The decision is not made on this page any more.
        expect(find(w, "switch-c").exists()).toBe(false);
        expect(text(w, "done-background")).not.toContain(message("setup.tasks.title"));
        // Nothing to point at later while it is on offer.
        expect(find(w, "done-tasks-tip").exists()).toBe(false);
      });

      it("follows the saved setting, with a way to look at it again", async () => {
        enabledSlack();
        payload = settings({ agentTasks: true, daemonLifecycle: "keepalive" });
        const w = await mountWizard("done");
        expect(text(w, "summary-tasks-state")).toBe(message("setup.done.tasksOn"));
        expect(get(w, "summary-tasks").classes()).not.toContain("none");
        expect(text(w, "summary-tasks-link")).toContain(message("setup.done.tasksChange"));
      });

      it("is there with only the iOS app", async () => {
        payload = settings({ ios: true });
        const w = await mountWizard("done");
        expect(find(w, "summary-tasks").exists()).toBe(true);
      });

      it("is there without any remote channel: the switch is global, and a channel may come later", async () => {
        const w = await mountWizard("done");
        expect(find(w, "summary-tasks").exists()).toBe(true);
        expect(text(w, "summary-tasks-state")).toBe(message("setup.done.tasksOff"));
        expect(find(w, "done-tasks-tip").exists()).toBe(false);
      });

      it.each([
        [
          "no Agent is connected",
          () => {
            enabledSlack();
            rowsValue = [row({ id: "claude", mode: "none" })];
          },
        ],
        [
          "only Antigravity is connected, which cannot start a task",
          () => {
            enabledSlack();
            rowsValue = [row({ id: "antigravity", title: "Antigravity", mode: "mcp" })];
          },
        ],
        [
          "the platform cannot run Agent tasks",
          () => {
            enabledSlack();
            platform.supportsAgentTasks = false;
          },
        ],
      ])("is left out when %s", async (_name, arrange) => {
        arrange();
        const w = await mountWizard("done");
        expect(find(w, "summary-tasks").exists()).toBe(false);
      });

      it("still reports a feature that is on where it could not be offered, and says when no channel can use it yet", async () => {
        rowsValue = [row({ id: "claude", mode: "none" })];
        payload = settings({ agentTasks: true, daemonLifecycle: "keepalive" });
        const without = await mountWizard("done");
        expect(text(without, "summary-tasks-state")).toBe(message("setup.done.tasksOnNoChannel"));
        without.unmount();
        wrappers.length = 0;

        enabledSlack();
        const withChannel = await mountWizard("done");
        expect(text(withChannel, "summary-tasks-state")).toBe(message("setup.done.tasksOn"));
      });

      it("keeps the keep-running switch as saved while the feature is not touched", async () => {
        enabledSlack();
        const w = await mountWizard("done");
        expect(isChecked(w, "switch-b")).toBe(false);
        expect(isDisabled(w, "switch-b")).toBe(false);
        await click(w, "finish");
        expect(saves[0].config.agentTasks).toMatchObject({ enabled: false });
        expect(saves[0].config.general).toMatchObject({ daemonLifecycle: "activity" });
      });

      it("leaves the feature alone where the offer is not available", async () => {
        payload = settings({ agentTasks: true, daemonLifecycle: "keepalive" });
        const w = await mountWizard("done");
        await click(w, "finish");
        expect(saves[0].config.agentTasks).toMatchObject({ enabled: true });
        expect(saves[0].config.general).toMatchObject({ daemonLifecycle: "keepalive" });
      });

      it("turning it off on the Agent-task screen lets the keep-running switch be chosen again", async () => {
        enabledSlack();
        payload = settings({ agentTasks: true, daemonLifecycle: "keepalive" });
        const w = await mountWizard("done");
        expect(isDisabled(w, "switch-b")).toBe(true);

        await click(w, "summary-tasks-link");
        await click(w, "tasks-later");
        expect(text(w, "summary-tasks-state")).toBe(message("setup.done.tasksOff"));
        expect(isDisabled(w, "switch-b")).toBe(false);
        // B goes back to what it was: the mode that is saved.
        expect(isChecked(w, "switch-b")).toBe(true);
        await get(w, "switch-b").setValue(false);
        await click(w, "finish");
        expect(saves[0].config.agentTasks).toMatchObject({ enabled: false });
        expect(saves[0].config.general).toMatchObject({ daemonLifecycle: "activity" });
      });

      it("turning it off is saved even where the offer could no longer be made", async () => {
        // On from an earlier run, but no Agent is connected now: the person still chooses to turn it off.
        rowsValue = [row({ id: "claude", mode: "none" })];
        payload = settings({ agentTasks: true, daemonLifecycle: "keepalive" });
        const w = await mountWizard("done");
        await click(w, "summary-tasks-link");
        await click(w, "tasks-later");
        await click(w, "finish");
        expect(saves[0].config.agentTasks).toMatchObject({ enabled: false });
      });
    });

    describe("where the feature is not on offer yet", () => {
      it("tells where to turn it on later, and opens that place", async () => {
        // No Agent is connected yet, so there is nothing to start a task on.
        rowsValue = [row({ id: "claude", mode: "none" })];
        const w = await mountWizard("done");
        expect(text(w, "done-tasks-tip")).toContain(message("setup.done.tasksTip"));
        await click(w, "done-tasks-tip-link");
        expect(ipc.openSettings).toHaveBeenCalledWith("advanced");
      });

      it("says nothing while an Agent can start a task, because the row above is the way in", async () => {
        const w = await mountWizard("done");
        expect(find(w, "done-tasks-tip").exists()).toBe(false);
        expect(find(w, "summary-tasks").exists()).toBe(true);
      });

      it("says nothing when the feature is already on, or the platform cannot run it", async () => {
        payload = settings({ agentTasks: true });
        const on = await mountWizard("done");
        expect(find(on, "done-tasks-tip").exists()).toBe(false);
        on.unmount();
        wrappers.length = 0;

        payload = settings();
        platform.supportsAgentTasks = false;
        const unsupported = await mountWizard("done");
        expect(find(unsupported, "done-tasks-tip").exists()).toBe(false);
      });
    });

    describe("what another window saved while the wizard stayed open", () => {
      /** The Settings window saves a new state of the config; the wizard learns nothing until it asks. */
      const savedMeanwhile = (over: Parameters<typeof settings>[0]) => {
        payload = settings(over);
      };

      it("offers the Agent-task row for an iOS channel that was set up in Settings meanwhile", async () => {
        const w = await mountWizard("remote");
        savedMeanwhile({ ios: true });
        await click(w, "step-done");
        expect(find(w, "summary-channel-ios").exists()).toBe(true);
        expect(find(w, "summary-tasks").exists()).toBe(true);
        expect(find(w, "done-tasks-tip").exists()).toBe(false);
      });

      it("shows the feature as on when Settings turned it on, and does not turn it off again", async () => {
        const w = await mountWizard("remote");
        savedMeanwhile({ ios: true, agentTasks: true, daemonLifecycle: "keepalive" });
        await click(w, "step-done");
        expect(text(w, "summary-tasks-state")).toBe(message("setup.done.tasksOn"));
        await click(w, "finish");
        expect(saves[0].config.agentTasks).toMatchObject({ enabled: true });
        expect(saves[0].config.general).toMatchObject({ daemonLifecycle: "keepalive" });
      });

      it("saves on top of what is saved now, so the iOS settings are not overwritten", async () => {
        const w = await mountWizard("done");
        // Settings enables the iOS channel after the wizard loaded its copy; Done is already open.
        savedMeanwhile({ ios: true });
        await click(w, "finish");
        expect(saves[0].config.channels.ios).toMatchObject({ enabled: true });
      });

      it("keeps the person's own choice over a newer saved value", async () => {
        payload = settings({ menuBarIcon: "active" });
        const w = await mountWizard("done");
        await get(w, "switch-a").setValue(true);
        savedMeanwhile({ menuBarIcon: "off" });
        await click(w, "finish");
        expect(saves[0].config.general).toMatchObject({ menuBarIcon: "always" });
      });
    });

    it.each([
      ["turns the menu bar on from the active mode", "active", true, "always"],
      ["leaves an off menu bar alone", "off", false, "off"],
      ["leaves an active menu bar alone", "active", false, "active"],
      ["relaxes an always menu bar to active when switched off", "always", false, "active"],
      ["keeps an always menu bar", "always", true, "always"],
    ])("Done %s", async (_name, initial, switchA, expected) => {
      payload = settings({ menuBarIcon: initial });
      const w = await mountWizard("done");
      await get(w, "switch-a").setValue(switchA);
      await click(w, "finish");
      expect(saves).toHaveLength(1);
      expect(saves[0].config.general).toMatchObject({ menuBarIcon: expected });
    });

    it.each([
      ["turns the keep-running mode on", "activity", true, "keepalive"],
      ["turns it back to activity when switched off", "keepalive", false, "activity"],
    ])("Done %s", async (_name, initial, switchB, expected) => {
      enabledSlack();
      payload = settings({ daemonLifecycle: initial });
      const w = await mountWizard("done");
      await get(w, "switch-b").setValue(switchB);
      await click(w, "finish");
      expect(saves[0].config.general).toMatchObject({ daemonLifecycle: expected });
    });

    it("leaves the lifecycle alone when no channel is on, and the menu bar alone without a tray", async () => {
      payload = settings({ menuBarIcon: "always", daemonLifecycle: "keepalive" });
      statusValue = status({ trayAvailable: false });
      const w = await mountWizard("done");
      await click(w, "finish");
      expect(saves[0].config.general).toMatchObject({ menuBarIcon: "always", daemonLifecycle: "keepalive" });
    });

    it("saves the choices, then records the decision, then closes the window", async () => {
      const w = await mountWizard("done");
      await click(w, "finish");
      expect(order).toEqual(["save", "finish", "close"]);
      expect(find(w, "finish-error").exists()).toBe(false);
    });

    it("stays open and says why when saving fails, without recording the decision", async () => {
      ipc.saveSettings.mockRejectedValueOnce(new Error("config is read-only"));
      const w = await mountWizard("done");
      await click(w, "finish");
      expect(text(w, "finish-error")).toContain("config is read-only");
      expect(order).toEqual([]);
      expect(isDisabled(w, "finish")).toBe(false);
      await click(w, "finish");
      expect(order).toEqual(["save", "finish", "close"]);
    });

    it("stays open and says why when recording the decision fails", async () => {
      ipc.setupFinish.mockRejectedValueOnce(new Error("could not write the login item"));
      const w = await mountWizard("done");
      await click(w, "finish");
      expect(text(w, "finish-error")).toContain("could not write the login item");
      expect(win.close).not.toHaveBeenCalled();
    });
  });

  describe("optional checks", () => {
    /** The backend reports that a check ended. */
    const ended = (probeId: string, kind: string, outcome: string, extra: Record<string, unknown> = {}) =>
      emit("setup-probe-result", { probeId, kind, outcome, ...extra });
    const slackOn = () => {
      channelRows = [channelRow("slack", { enabled: true, configured: true })];
    };

    describe("the desktop popup check", () => {
      it("starts the test question and follows it to its end", async () => {
        const w = await mountWizard("done");
        expect(find(w, "probe-popup-status").exists()).toBe(false);
        await click(w, "probe-popup-start");
        expect(probeStarts).toEqual([{ id: "probe-1", kind: "popup" }]);
        expect(text(w, "probe-popup-status")).toBe(message("setup.probe.popup.running"));
        // While it runs the button cancels it.
        expect(find(w, "probe-popup-start").exists()).toBe(false);
        expect(find(w, "probe-popup-cancel").exists()).toBe(true);

        await ended("probe-1", "popup", "ok");
        expect(text(w, "probe-popup-status")).toBe(message("setup.probe.popup.ok"));
        expect(get(w, "probe-popup-status").classes()).toContain("ok");
        // A finished check can be run again.
        expect(text(w, "probe-popup-start")).toBe(message("setup.probe.retry"));
      });

      it.each(["timeout", "cancelled", "failed"])("reports %s as a problem and still lets Done finish", async (outcome) => {
        const w = await mountWizard("done");
        await click(w, "probe-popup-start");
        await ended("probe-1", "popup", outcome);
        expect(text(w, "probe-popup-status")).toBe(message(`setup.probe.popup.${outcome}`));
        expect(get(w, "probe-popup-status").classes()).toContain("err");

        await click(w, "finish");
        expect(order).toEqual(["save", "finish", "close"]);
      });

      it("says why a check could not even start", async () => {
        ipc.setupProbeStart.mockRejectedValueOnce(new Error("daemon is restarting"));
        const w = await mountWizard("done");
        await click(w, "probe-popup-start");
        expect(text(w, "probe-popup-status")).toBe(
          message("setup.probe.startFailed").replace("{error}", "Error: daemon is restarting"),
        );
        expect(get(w, "probe-popup-status").classes()).toContain("err");
      });

      it("cancel stops the check at the backend and brings the start button back", async () => {
        const w = await mountWizard("done");
        await click(w, "probe-popup-start");
        await click(w, "probe-popup-cancel");
        expect(ipc.setupProbeCancel).toHaveBeenCalledWith("probe-1");
        expect(find(w, "probe-popup-status").exists()).toBe(false);
        expect(text(w, "probe-popup-start")).toBe(message("setup.probe.popup.button"));
      });

      it("ignores the end of a check it did not start", async () => {
        const w = await mountWizard("done");
        await click(w, "probe-popup-start");
        await ended("somebody-else", "popup", "ok");
        expect(text(w, "probe-popup-status")).toBe(message("setup.probe.popup.running"));
      });
    });

    describe("the Agent check", () => {
      it("is offered only once an Agent is connected", async () => {
        rowsValue = [row({ id: "claude", mode: "none" })];
        const none = await mountWizard("done");
        expect(find(none, "probe-popup").exists()).toBe(true);
        expect(find(none, "probe-agent").exists()).toBe(false);
        none.unmount();
        wrappers.length = 0;

        rowsValue = [row({ id: "claude", mode: "cli" })];
        const connected = await mountWizard("done");
        expect(find(connected, "probe-agent").exists()).toBe(true);
      });

      it("is not offered in a Dev Instance, whose Agents talk to the everyday daemon", async () => {
        payload = settings({ readOnly: true });
        const w = await mountWizard("done");
        expect(find(w, "probe-agent").exists()).toBe(false);
      });

      it("names the Agent the question came from", async () => {
        rowsValue = [
          row({ id: "claude", title: "Claude Code", mode: "cli" }),
          row({ id: "codex", title: "Codex", mode: "cli" }),
        ];
        const w = await mountWizard("done");
        await click(w, "probe-agent-start");
        expect(probeStarts).toEqual([{ id: "probe-1", kind: "agent" }]);
        expect(text(w, "probe-agent-status")).toBe(message("setup.probe.agent.running"));

        await ended("probe-1", "agent", "ok", { agentKind: "codex" });
        expect(text(w, "probe-agent-status")).toBe("A question arrived from Codex. The connection works.");
      });

      it("lists what to look at when no Agent asked anything, and only then", async () => {
        const w = await mountWizard("done");
        await click(w, "probe-agent-start");
        expect(find(w, "probe-agent-checklist").exists()).toBe(false);
        await ended("probe-1", "agent", "timeout");
        expect(get(w, "probe-agent-checklist").findAll("li")).toHaveLength(5);

        await click(w, "probe-agent-start");
        await ended("probe-2", "agent", "failed");
        expect(find(w, "probe-agent-checklist").exists()).toBe(false);
      });

      it("does not depend on the popup check", async () => {
        const w = await mountWizard("done");
        await click(w, "probe-popup-start");
        await click(w, "probe-agent-start");
        await ended("probe-2", "agent", "timeout");
        expect(text(w, "probe-popup-status")).toBe(message("setup.probe.popup.running"));
        expect(find(w, "probe-agent-checklist").exists()).toBe(true);
      });
    });

    describe("the channel round trip", () => {
      async function openChannel(w: VueWrapper, id: string) {
        await click(w, `channel-open-${id}`);
        expect(find(w, `channel-form-${id}`).exists()).toBe(true);
      }

      it("is offered only for a channel that is saved and on", async () => {
        channelRows = [
          channelRow("telegram", { enabled: true, configured: false }),
          channelRow("dingding", { enabled: false, configured: true }),
          channelRow("slack", { enabled: true, configured: true }),
        ];
        const w = await mountWizard("remote");
        await openChannel(w, "telegram");
        expect(find(w, "probe-channel").exists()).toBe(false);
        await click(w, "channel-back");
        await openChannel(w, "dingding");
        expect(find(w, "probe-channel").exists()).toBe(false);
        await click(w, "channel-back");
        await openChannel(w, "slack");
        expect(find(w, "probe-channel").exists()).toBe(true);
      });

      it("runs for that channel and names it in what it says", async () => {
        slackOn();
        const w = await mountWizard("remote");
        await openChannel(w, "slack");
        await click(w, "probe-channel-start");
        expect(probeStarts).toEqual([{ id: "probe-1", kind: "channel", channel: "slack" }]);
        const params = { channel: message("settings.channels.slackTitle") };
        expect(text(w, "probe-channel-status")).toBe(
          message("setup.probe.channel.running").replace("{channel}", params.channel),
        );

        await ended("probe-1", "channel", "ok", { channel: "slack" });
        expect(text(w, "probe-channel-status")).toBe(
          message("setup.probe.channel.ok").replace("{channel}", params.channel),
        );
      });

      it("shows a failed round trip as a problem with the hint for it", async () => {
        slackOn();
        const w = await mountWizard("remote");
        await openChannel(w, "slack");
        await click(w, "probe-channel-start");
        await ended("probe-1", "channel", "timeout", { channel: "slack" });
        expect(text(w, "probe-channel-status")).toBe(message("setup.probe.channel.timeout"));
        expect(get(w, "probe-channel-status").classes()).toContain("err");
      });

      it("is stopped, and forgotten, when the form is left", async () => {
        slackOn();
        const w = await mountWizard("remote");
        await openChannel(w, "slack");
        await click(w, "probe-channel-start");
        await click(w, "channel-back");
        expect(ipc.setupProbeCancel).toHaveBeenCalledWith("probe-1");
        await openChannel(w, "slack");
        expect(find(w, "probe-channel-status").exists()).toBe(false);
      });

      it("is stopped when the wizard moves to another page", async () => {
        slackOn();
        const w = await mountWizard("remote");
        await openChannel(w, "slack");
        await click(w, "probe-channel-start");
        await click(w, "step-done");
        expect(ipc.setupProbeCancel).toHaveBeenCalledWith("probe-1");
      });
    });

    describe("leaving the wizard", () => {
      it("stops a running check when Done is clicked, and still finishes", async () => {
        const w = await mountWizard("done");
        await click(w, "probe-popup-start");
        await click(w, "finish");
        expect(ipc.setupProbeCancel).toHaveBeenCalledWith("probe-1");
        expect(order).toEqual(["save", "finish", "close"]);
      });

      it("does not stop a check that already ended", async () => {
        const w = await mountWizard("done");
        await click(w, "probe-popup-start");
        await ended("probe-1", "popup", "ok");
        await click(w, "finish");
        expect(ipc.setupProbeCancel).not.toHaveBeenCalled();
      });

      it("stops running checks when the window is hidden or closed", async () => {
        const w = await mountWizard("done");
        await click(w, "probe-popup-start");
        window.dispatchEvent(new Event("pagehide"));
        expect(ipc.setupProbeCancel).toHaveBeenCalledWith("probe-1");
        await flushPromises();
        expect(find(w, "probe-popup-status").exists()).toBe(false);
      });

      it("stops running checks when the view is torn down", async () => {
        const w = await mountWizard("done");
        await click(w, "probe-popup-start");
        w.unmount();
        wrappers.length = 0;
        expect(ipc.setupProbeCancel).toHaveBeenCalledWith("probe-1");
      });
    });
  });
});
