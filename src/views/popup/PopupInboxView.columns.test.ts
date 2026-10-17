import { afterEach, describe, expect, it } from "vitest";
import view from "../PopupInboxView.vue?raw";
import { createCascade, styleBlock } from "./cssCascade.testkit";

// Windows and Linux lay the window out as a CSS grid (macOS pins a canvas and places everything absolutely). The list is a
// grid column only when it is a real Sidebar column. A single request's list (laid out hidden to be measured before the small
// window, the small window itself, and the list fading out once the window has expanded) must leave the whole row to the
// body: as a column it squeezes the body for the frames before the window changes form, and again until the list goes.
// Which grid rule wins depends on specificity, which jsdom does not sort, so the shared cascade helper resolves it.
const cascade = createCascade(styleBlock(view));

type Platform = "grid" | "native";
type List = "sidebar" | "lone" | "none";

function windowOf(platform: Platform, list: List, form: "expanded" | "collapsed") {
  const classes = ["inbox-root", platform === "native" ? "inbox-native" : "", list === "none" ? "" : "inbox-expanded"].filter(Boolean).join(" ");
  document.body.innerHTML = `
    <div class="${classes}" data-view="${form}" data-sidebar="${list === "sidebar" ? "on" : "off"}">
      ${list === "none" ? "" : '<aside class="inbox-sidebar"></aside><div class="inbox-divider"></div>'}
      <main class="inbox-body"></main>
    </div>`;
  const pick = (selector: string) => document.querySelector(selector);
  return { root: pick(".inbox-root")!, list: pick(".inbox-sidebar"), divider: pick(".inbox-divider"), body: pick(".inbox-body")! };
}

afterEach(() => { document.body.innerHTML = ""; });

describe("the list's grid column on Windows and Linux", () => {
  it("is a real column when the Sidebar is on: list, divider, then the body", () => {
    for (const form of ["expanded", "collapsed"] as const) {
      const { root, list, divider } = windowOf("grid", "sidebar", form);
      expect(cascade(root, "grid-template-columns"), form).toBe("var(--inbox-sidebar) 6px minmax(0, 1fr)");
      expect(cascade(list!, "position"), form).toBeUndefined();
      expect(cascade(divider!, "display"), form).toBeUndefined();
    }
  });

  it("is no column for a single request's list: the body keeps the whole row and the list lies at the left edge", () => {
    // Hidden for the measuring pass and fading out after the expansion (expanded), and the small window's own list (collapsed).
    for (const form of ["expanded", "collapsed"] as const) {
      const { root, list, divider } = windowOf("grid", "lone", form);
      expect(cascade(root, "grid-template-columns"), form).toBe("minmax(0, 1fr)");
      expect(cascade(list!, "position"), form).toBe("absolute");
      expect(cascade(list!, "left"), form).toBe("0");
      expect(cascade(list!, "width"), form).toBe("var(--inbox-sidebar)");
      expect(cascade(divider!, "display"), form).toBe("none");
    }
  });

  it("has just the body's column without any list", () => {
    const { root } = windowOf("grid", "none", "expanded");
    expect(cascade(root, "grid-template-columns")).toBe("minmax(0, 1fr)");
  });

  it("leaves the pinned canvas of macOS alone: the list stays where the canvas puts it", () => {
    for (const list of ["sidebar", "lone"] as const) {
      const { root, list: aside, divider } = windowOf("native", list, "collapsed");
      expect(cascade(root, "display"), list).toBe("block");
      expect(cascade(aside!, "left"), list).toBe("var(--inbox-visible-left)");
      expect(cascade(divider!, "display"), list).toBeUndefined();
    }
  });
});
