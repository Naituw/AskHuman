// The popup footer's pointer to the setup wizard, which replaced the one-time "configure an IM
// channel" tip (docs/specs/onboarding.md D18): when it shows, what each of its two buttons does.
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import zh from "../../i18n/zh";
import type { ConfirmRequest, PopupInit } from "../../lib/types";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke, convertFileSrc: (path: string) => path }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setAlwaysOnTop: vi.fn() }) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn() }));
import PopupView from "../PopupView.vue";

// jsdom has no CSS.escape, which the in-page find uses to address its segments.
if (typeof globalThis.CSS === "undefined") {
  (globalThis as unknown as { CSS: { escape: (s: string) => string } }).CSS = {
    escape: (s: string) => s.replace(/[^a-zA-Z0-9_-]/g, (c) => `\\${c}`),
  };
}

const base = {
  theme: "light", alwaysOnTop: false, sourceName: "Codex", project: "/project", projectName: "project",
  language: "en", warm: true,
};

function ask(id: string, questions = 1, setupProbe = false): PopupInit {
  return { ...base,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      setupProbe,
      message: { text: "", files: [] },
      questions: Array.from({ length: questions }, (_, i) => ({
        message: `Question ${i + 1} of ${id}`,
        predefinedOptions: [{ text: "Yes", recommended: false }, { text: "No", recommended: false }],
      })) } },
  } as PopupInit;
}

function permission(id: string): PopupInit {
  const request: ConfirmRequest = {
    id, title: "Run command?", context: [], detail: { summary: "Needed for verification", bodyMd: "`cargo test`" },
    choices: [
      { id: "allow", label: "Allow once", description: "", role: "primary" },
      { id: "deny", label: "Deny", description: "", role: "destructive" },
    ],
    presentation: { type: "singleSelectSubmit", submitLabel: "Submit" },
    dismissActionId: "deny", createdAtMs: 1, expiresAtMs: 2,
  };
  return { ...base, interaction: { type: "confirm", request } } as PopupInit;
}

describe("the popup's setup hint", () => {
  const wrappers: VueWrapper[] = [];
  let interaction: PopupInit;
  let hint: unknown;
  let hintFails: boolean;

  beforeEach(() => {
    vi.clearAllMocks();
    interaction = ask("a");
    hint = { id: "remote", page: "remote" };
    hintFails = false;
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.stubGlobal("requestAnimationFrame", (f: FrameRequestCallback) => { queueMicrotask(() => f(0)); return 1; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    vi.stubGlobal("IntersectionObserver", class {
      observe() {}
      unobserve() {}
      disconnect() {}
      takeRecords() { return []; }
    });
    mocks.invoke.mockImplementation(async (command: string) => {
      switch (command) {
        case "popup_init": return interaction;
        case "popup_agent_resolved": return {};
        case "popup_update_state": return { available: false, pending: false };
        case "todos_list": return [];
        case "popup_setup_hint":
          if (hintFails) throw new Error("unknown command");
          return hint;
        default: return false;
      }
    });
  });

  afterEach(() => {
    wrappers.splice(0).forEach((w) => { try { w.unmount(); } catch { /* already unmounted */ } });
    document.body.innerHTML = "";
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  async function mountPopup(locale: "en" | "zh" = "en") {
    const wrapper = mount(PopupView, {
      attachTo: document.body,
      global: { plugins: [createI18n({ legacy: false, locale, messages: { en, zh } })] },
    });
    wrappers.push(wrapper);
    await flushPromises();
    return wrapper;
  }

  const calls = (command: string) => mocks.invoke.mock.calls.filter(([name]) => name === command).map(([, args]) => args);
  const strip = (w: VueWrapper) => w.find('[data-testid="setup-hint"]');

  it("shows the pointer above the footer when the backend names an open page", async () => {
    const w = await mountPopup();
    expect(strip(w).exists()).toBe(true);
    expect(strip(w).text()).toContain(en.popup.setupHint.remote.text);
    expect(w.get('[data-testid="setup-hint-action"]').text()).toBe(en.popup.setupHint.remote.action);
    expect(calls("popup_setup_hint")).toHaveLength(1);
  });

  it("speaks the interface language", async () => {
    const w = await mountPopup("zh");
    expect(strip(w).text()).toContain(zh.popup.setupHint.remote.text);
  });

  it("shows it on a permission request as well", async () => {
    interaction = permission("p");
    const w = await mountPopup();
    expect(strip(w).exists()).toBe(true);
  });

  it("stays out of the wizard's own test question, which is already part of the wizard", async () => {
    interaction = ask("probe", 1, true);
    const w = await mountPopup();
    expect(w.text()).toContain("Question 1 of probe");
    expect(strip(w).exists()).toBe(false);
  });

  it.each([
    ["every question is settled", null],
    ["an older backend answers something else", false],
  ])("is hidden when %s", async (_name, answer) => {
    hint = answer;
    const w = await mountPopup();
    expect(strip(w).exists()).toBe(false);
  });

  it("is hidden, without breaking the popup, when the backend does not know the command", async () => {
    hintFails = true;
    const w = await mountPopup();
    expect(strip(w).exists()).toBe(false);
    expect(w.text()).toContain("Question 1 of a");
  });

  it("the action button opens the wizard on that page, counts as decided, and hides the strip", async () => {
    const w = await mountPopup();
    await w.get('[data-testid="setup-hint-action"]').trigger("click");
    await flushPromises();
    expect(calls("open_setup")).toEqual([{ page: "remote" }]);
    expect(calls("popup_setup_hint_dismiss")).toEqual([{ id: "remote" }]);
    expect(strip(w).exists()).toBe(false);
  });

  it("the close button counts as decided too, but does not open anything", async () => {
    const w = await mountPopup();
    await w.get('[data-testid="setup-hint-dismiss"]').trigger("click");
    await flushPromises();
    expect(calls("popup_setup_hint_dismiss")).toEqual([{ id: "remote" }]);
    expect(calls("open_setup")).toEqual([]);
    expect(strip(w).exists()).toBe(false);
  });
});
