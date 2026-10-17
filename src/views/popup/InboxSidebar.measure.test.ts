import { afterEach, describe, expect, it } from "vitest";
import sidebar from "./InboxSidebar.vue?raw";

// The small window is as tall as the list measured under `data-measure` (inboxMini.ts), while the root still reads
// "expanded". Whatever changes the size of the list's rows or heading in the small window must therefore apply to that
// measuring pass too, or the window is cut to the wrong height. jsdom does no layout, so this compares the computed
// style of the real stylesheet instead.
const css = /<style>([\s\S]*?)<\/style>/.exec(sidebar)![1];

const SIZED = ["padding-top", "padding-right", "padding-bottom", "padding-left", "margin-top", "max-height", "font-size"];

function layout(attributes: Record<string, string>) {
  document.head.innerHTML = `<style>${css}</style>`;
  document.body.innerHTML = `
    <div class="inbox-root">
      <div class="inbox-heading"><span class="inbox-count">2</span></div>
      <div class="inbox-entry"><button class="inbox-row secondary">
        <div class="inbox-row-content"><strong>Title</strong><div class="inbox-row-meta">Agent</div></div>
      </button></div>
    </div>`;
  const root = document.querySelector<HTMLElement>(".inbox-root")!;
  for (const [name, value] of Object.entries(attributes)) root.setAttribute(name, value);
  const result: Record<string, Record<string, string>> = {};
  for (const selector of [".inbox-count", ".inbox-row strong", ".inbox-row-meta"]) {
    const style = getComputedStyle(root.querySelector(selector)!);
    result[selector] = Object.fromEntries(SIZED.map(property => [property, style.getPropertyValue(property)]));
  }
  return result;
}

afterEach(() => {
  document.head.innerHTML = "";
  document.body.innerHTML = "";
});

describe("the Sidebar's measuring pass", () => {
  const small = { "data-view": "collapsed" };
  const measuring = { "data-view": "expanded", "data-measure": "" };

  it("lays the rows and the heading out like the small window", () => {
    expect(layout(measuring)).toEqual(layout(small));
  });

  it("is not just the expanded layout", () => {
    // Guards the comparison above: it only means something while the two forms really differ.
    const expanded = layout({ "data-view": "expanded" });
    expect(expanded[".inbox-row strong"]["padding-right"]).toBe("90px");
    expect(expanded[".inbox-count"]).not.toEqual(layout(small)[".inbox-count"]);
    expect(expanded[".inbox-row-meta"]).not.toEqual(layout(small)[".inbox-row-meta"]);
  });
});
