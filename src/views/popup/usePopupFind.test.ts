// Regression coverage for popup find: only user navigation may move the viewport or reveal a
// question; passive repaints (scroll-spy driven `currentQ`, Markdown updates) must not.
import { nextTick, ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import type { Question } from "../../lib/types";
import { usePopupFind } from "./usePopupFind";

// jsdom has no CSS.escape; segment ids here are plain identifiers, so a minimal escape suffices.
if (typeof globalThis.CSS === "undefined") {
  (globalThis as unknown as { CSS: { escape: (s: string) => string } }).CSS = {
    escape: (s: string) => s.replace(/[^a-zA-Z0-9_-]/g, (c) => `\\${c}`),
  };
}

function question(message: string): Question {
  return { message, predefinedOptions: [] } as unknown as Question;
}

/** Fake layout: `.content` is a 0–600px viewport; each segment reports the given top. */
function mountContent(tops: Record<string, number>): HTMLElement {
  const content = document.createElement("div");
  content.getBoundingClientRect = () =>
    ({ top: 0, bottom: 600, height: 600 }) as DOMRect;
  for (const [segId, top] of Object.entries(tops)) {
    const seg = document.createElement("p");
    seg.setAttribute("data-find-seg", segId);
    seg.textContent = `alpha ${segId} alpha`;
    content.appendChild(seg);
    // Marks inherit their segment's position for the purpose of these tests.
    const rect = { top, bottom: top + 16, height: 16 } as DOMRect;
    seg.getBoundingClientRect = () => rect;
  }
  document.body.appendChild(content);
  return content;
}

function scrollSpy(content: HTMLElement): { calls: number } {
  const state = { calls: 0 };
  let value = 0;
  Object.defineProperty(content, "scrollTop", {
    configurable: true,
    get: () => value,
    set: (v: number) => {
      state.calls += 1;
      value = v;
    },
  });
  return state;
}

function setup(opts: { vertical: boolean; tops: Record<string, number> }) {
  const content = mountContent(opts.tops);
  const scroll = scrollSpy(content);
  // Marks are created by applyFindMarks; give them the segment's rect.
  const origQuerySelectorAll = content.querySelectorAll.bind(content);
  content.querySelectorAll = ((sel: string) => {
    const list = origQuerySelectorAll(sel);
    list.forEach((el) => {
      const seg = (el as HTMLElement).closest("[data-find-seg]") as HTMLElement | null;
      if (seg) (el as HTMLElement).getBoundingClientRect = seg.getBoundingClientRect;
    });
    return list;
  }) as typeof content.querySelectorAll;

  const currentQ = ref(0);
  const revealQuestion = vi.fn(async (i: number) => {
    currentQ.value = i;
  });
  const find = usePopupFind({
    contentRef: ref(content),
    isConfirm: ref(false),
    confirmRequest: ref(null),
    messageText: ref("alpha message alpha"),
    viewSource: ref(false),
    attachments: ref([]),
    questions: ref([question("alpha q-0-msg alpha"), question("alpha q-1-msg alpha")]),
    whatsNext: ref(false),
    todoPrefix: ref(""),
    confirmChoiceTexts: ref([]),
    confirmTitle: ref(""),
    confirmSummary: ref(""),
    confirmToolName: ref(""),
    confirmBodyText: ref(""),
    currentQ,
    verticalMode: ref(opts.vertical),
    revealQuestion,
  });
  return { content, scroll, currentQ, revealQuestion, find };
}

async function settle(): Promise<void> {
  for (let i = 0; i < 4; i++) await nextTick();
}

describe("usePopupFind viewport ownership", () => {
  it("vertical mode: scroll-spy changing currentQ neither reveals nor scrolls", async () => {
    const { content, scroll, currentQ, revealQuestion, find } = setup({
      vertical: true,
      // Current match (first alpha in message) is far above the viewport.
      tops: { message: -2000, "q-0-msg": -1500, "q-1-msg": 3000 },
    });
    find.openFind(false);
    find.onFindQueryInput("alpha");
    await settle();
    expect(find.findTotal.value).toBe(6);
    expect(find.findCurrent.value).toBe(0);
    const navigationScrolls = scroll.calls;
    const navigationReveals = revealQuestion.mock.calls.length;
    expect(navigationScrolls).toBeGreaterThan(0);

    // User scrolls; scroll-spy moves the current question pointer.
    currentQ.value = 1;
    await settle();
    expect(revealQuestion.mock.calls.length).toBe(navigationReveals);
    expect(scroll.calls).toBe(navigationScrolls);
    expect(find.findCurrent.value).toBe(0);
    expect(content.querySelectorAll("[data-popup-find]").length).toBe(6);
  });

  it("passive refreshFind repaints without scrolling; Enter navigates again", async () => {
    const { scroll, revealQuestion, find } = setup({
      vertical: true,
      tops: { message: -2000, "q-0-msg": -1500, "q-1-msg": 3000 },
    });
    find.openFind(false);
    find.onFindQueryInput("alpha");
    await settle();
    const afterOpen = scroll.calls;

    find.refreshFind();
    await settle();
    expect(scroll.calls).toBe(afterOpen);

    await find.goFind(1);
    await settle();
    expect(find.findCurrent.value).toBe(1);
    expect(scroll.calls).toBeGreaterThan(afterOpen);
    expect(revealQuestion).not.toHaveBeenCalled(); // message matches carry no question
  });

  it("sequential mode: a manual question switch stays put, next match reveals again", async () => {
    const { currentQ, revealQuestion, find } = setup({
      vertical: false,
      tops: { message: 100, "q-0-msg": 200 },
    });
    find.openFind(false);
    find.onFindQueryInput("q-0-msg");
    await settle();
    expect(find.findTotal.value).toBe(1);
    // Already on question 0: nothing to reveal.
    expect(revealQuestion).not.toHaveBeenCalled();

    // User presses "next question" while the only match lives on question 0.
    currentQ.value = 1;
    await settle();
    expect(revealQuestion).not.toHaveBeenCalled();
    expect(currentQ.value).toBe(1);
    expect(find.findCountLabel.value).toBe("1/1");

    // Enter / ⌘G: explicit navigation is allowed to bring question 0 back.
    await find.goFind(1);
    await settle();
    expect(revealQuestion).toHaveBeenLastCalledWith(0);
    expect(currentQ.value).toBe(0);
  });
});
