// Regression: the shared window mounts one real PopupView per request in the same document. Each form must pin its
// answer editor into its OWN dock. A document-wide Teleport id used to send a later form's editor into an earlier
// form's hidden dock (or into a node that was already gone), which left an empty bar and no editor to type into.
import { computed, nextTick, ref, type Ref } from "vue";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import type { PopupInit } from "../../lib/types";
import type { PopupContext } from "./context";
import type { PopupScope } from "./usePopupCore";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke, convertFileSrc: (path: string) => path }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setAlwaysOnTop: vi.fn() }) }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn() }));
import PopupView from "../PopupView.vue";

function init(id: string): PopupInit {
  return {
    theme: "light", alwaysOnTop: false, sourceName: "Codex", project: "/project", projectName: "project",
    language: "en", warm: true,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "", files: [] }, questions: [{ message: `Question of ${id}`, predefinedOptions: [] }] } },
  } as PopupInit;
}

interface Form {
  wrapper: VueWrapper;
  host: HTMLElement;
  content: HTMLElement;
  ctx: PopupContext;
  active: Ref<boolean>;
  textarea: HTMLTextAreaElement;
  /** Scroll the answer area to an offset, as the user's wheel / trackpad would. */
  scrollBy: (delta: number) => void;
}

describe("a form's pinned answer editor in a window that holds several forms", () => {
  const wrappers: VueWrapper[] = [];
  const hosts: HTMLElement[] = [];

  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("requestAnimationFrame", (f: FrameRequestCallback) => { queueMicrotask(() => f(0)); return 1; });
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    mocks.invoke.mockImplementation(async (command: string, args: { requestId?: string }) => {
      if (command === "popup_init") return init(args.requestId!);
      if (command === "popup_agent_resolved") return {};
      if (command === "todos_list") return [];
      if (command === "popup_update_state") return { available: false, pending: false };
      return false;
    });
    // The editor sits just below the bottom edge of a 300px answer area, so looking back up pins it.
    const box = (top: number, height: number) =>
      ({ top, bottom: top + height, left: 0, right: 400, width: 400, height, x: 0, y: top, toJSON() {} }) as DOMRect;
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
      if (this.classList.contains("content")) return box(0, 300);
      if (this.classList.contains("composer-anchor") || this.classList.contains("input-wrap")) return box(290, 80);
      if (this.tagName === "TEXTAREA") return box(290, 60);
      if (this.classList.contains("composer-dock")) return box(240, 60);
      return box(0, 0);
    });
  });

  afterEach(() => {
    wrappers.splice(0).forEach((w) => { try { w.unmount(); } catch { /* already unmounted by the test */ } });
    hosts.splice(0).forEach((h) => h.remove());
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  // Mirrors the inbox: `<div class="inbox-request"><PopupView/></div>`, one per mounted request, hidden unless shown.
  async function mountForm(id: string, visible = true): Promise<Form> {
    const host = document.createElement("div");
    host.dataset.inboxRequest = id;
    host.style.display = visible ? "" : "none";
    document.body.appendChild(host);
    hosts.push(host);
    const active = ref(visible);
    const scope = { requestId: id, active, blocked: computed(() => false), ready: vi.fn(), close: vi.fn(), draft: vi.fn(),
      preview: async () => ({ revision: 1, side: "closed", mainWidth: 560, mainHeight: 620 }) } as unknown as PopupScope;
    const wrapper = mount(PopupView, {
      props: { scope },
      attachTo: host,
      global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] },
    });
    wrappers.push(wrapper);
    await flushPromises();
    await nextTick();
    const content = host.querySelector(".content") as HTMLElement;
    let top = 120;
    Object.defineProperty(content, "scrollTop", { configurable: true, get: () => top, set: (value: number) => { top = value; } });
    return {
      wrapper, host, content, active,
      ctx: (wrapper.vm as unknown as { ctx: PopupContext }).ctx,
      textarea: host.querySelector("textarea") as HTMLTextAreaElement,
      scrollBy: (delta) => {
        if (delta < 0) content.dispatchEvent(new WheelEvent("wheel", { deltaY: delta, bubbles: true }));
        top += delta;
        content.dispatchEvent(new Event("scroll"));
      },
    };
  }

  async function settle() {
    await flushPromises();
    await nextTick();
    await nextTick();
  }

  /** The user clicks into the editor, then scrolls back up to read the message above it. */
  async function lookBackUp(form: Form, distance = 60) {
    form.textarea.focus();
    form.textarea.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    form.textarea.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await nextTick();
    form.scrollBy(-distance);
    await settle();
  }

  const dockOf = (form: Form) => form.host.querySelector(".composer-dock") as HTMLElement;
  const dockTargetOf = (form: Form) => form.host.querySelector(".composer-dock-target") as HTMLElement;

  it("pins into its own dock when it is the only form", async () => {
    const form = await mountForm("a");
    await lookBackUp(form);
    expect(form.ctx.dockedComposerQ.value).toBe(0);
    expect(dockTargetOf(form).contains(form.textarea)).toBe(true);
    expect(dockOf(form).style.display).not.toBe("none");
  });

  it("gives every form its own dock target id", async () => {
    const a = await mountForm("a", false);
    const b = await mountForm("b");
    const ids = [...document.querySelectorAll(".composer-dock-target")].map((el) => el.id);
    expect(ids).toHaveLength(2);
    expect(new Set(ids).size).toBe(2);
    expect(ids).toEqual([dockTargetOf(a).id, dockTargetOf(b).id]);
  });

  it("pins a later form into its own dock while an earlier form is still mounted but hidden", async () => {
    const earlier = await mountForm("a", false);
    const later = await mountForm("b");
    await lookBackUp(later);
    expect(later.ctx.dockedComposerQ.value).toBe(0);
    expect(dockTargetOf(later).contains(later.textarea)).toBe(true);
    expect(dockTargetOf(earlier).childElementCount).toBe(0);
    expect(document.body.contains(later.textarea)).toBe(true);
  });

  it("still pins into its own dock after the form that was mounted before it has been answered and removed", async () => {
    const earlier = await mountForm("a");
    const later = await mountForm("b", false); // prepared while the earlier form is on screen
    earlier.wrapper.unmount();
    earlier.host.remove();
    later.host.style.display = "";
    later.active.value = true;
    await settle();
    await lookBackUp(later);
    expect(later.ctx.dockedComposerQ.value).toBe(0);
    expect(dockTargetOf(later).contains(later.textarea)).toBe(true);
    expect(document.body.contains(later.textarea)).toBe(true);
  });

  it("falls back to the inline editor, and stops pinning, when its dock cannot take the editor", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const form = await mountForm("a");
    dockTargetOf(form).remove(); // the dock's target is gone from the document: the editor would land nowhere
    await lookBackUp(form);
    expect(form.ctx.dockedComposerQ.value).toBeNull();
    expect(form.host.querySelector(".composer-anchor")!.contains(form.textarea)).toBe(true);
    expect(warn).toHaveBeenCalledWith(expect.stringContaining("dock unavailable"), expect.any(String));

    // Looking back again must not try (and fail) to pin again.
    warn.mockClear();
    form.scrollBy(-30);
    await settle();
    expect(form.ctx.dockedComposerQ.value).toBeNull();
    expect(form.host.querySelector(".composer-anchor")!.contains(form.textarea)).toBe(true);
    expect(warn).not.toHaveBeenCalled();
  });
});
