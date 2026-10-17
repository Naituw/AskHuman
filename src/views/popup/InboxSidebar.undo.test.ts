import { afterEach, describe, expect, it } from "vitest";
import sidebar from "./InboxSidebar.vue?raw";
import { createCascade, styleBlock } from "./cssCascade.testkit";

// The "undo later" button on a set aside row is drawn over the end of the title in the small window, which keeps no room for
// it. It once had no fill there (the small window's rules out-rank the hover look, so the hover fill never applied) and its
// label ran through the title text. jsdom neither tracks :hover nor sorts rules by specificity, so the cascade is resolved by
// the shared helper, with real selector matching, against the component's own stylesheet. The component's :hover and
// :focus-visible become classes the fixture sets.
const cascade = createCascade(styleBlock(sidebar), { ":hover": ".is-hovered", ":focus-visible": ".is-focused" });

function undoButton(view: "collapsed" | "expanded", state: "idle" | "hover" | "focus") {
  document.body.innerHTML = `
    <div class="inbox-root" data-view="${view}" data-primary="${view === "expanded" ? "yes" : "no"}" data-sidebar="on">
      <div class="inbox-entry${state === "hover" ? " is-hovered" : ""}">
        <button class="inbox-row secondary"></button>
        <button class="row-flag flag-archive is-secondary${state === "focus" ? " is-focused" : ""}"></button>
      </div>
    </div>`;
  return document.querySelector(".row-flag")!;
}

afterEach(() => { document.body.innerHTML = ""; });

describe("the undo button on a set aside row", () => {
  it("is hidden in the small window until the row is hovered or the button has keyboard focus", () => {
    expect(cascade(undoButton("collapsed", "idle"), "opacity")).toBe("0");
    expect(cascade(undoButton("collapsed", "hover"), "opacity")).toBe("1");
    expect(cascade(undoButton("collapsed", "focus"), "opacity")).toBe("1");
  });

  it("has a fill wherever it is drawn over the title: hovered in the small window, or focused there", () => {
    for (const state of ["hover", "focus"] as const) {
      const fill = cascade(undoButton("collapsed", state), "background");
      expect(fill, state).toBeDefined();
      expect(fill, state).not.toBe("transparent");
      // Opaque, not one of the translucent washes that let the title show through (tokens.css).
      expect(fill, state).toContain("--surface-overlay");
    }
  });

  it("keeps its standing label and its hover look in the expanded Sidebar, where the title leaves room for it", () => {
    expect(cascade(undoButton("expanded", "idle"), "opacity")).toBe("1");
    expect(cascade(undoButton("expanded", "idle"), "background")).toBe("var(--card-bg)");
    expect(cascade(undoButton("expanded", "hover"), "background")).toContain("--surface-overlay");
  });

  it("is not just a fixture that matches everything: the small window's idle button has no fill", () => {
    // Guards the checks above, which only mean something while the cascade really tells these states apart.
    expect(cascade(undoButton("collapsed", "idle"), "background")).toBe("transparent");
  });
});
