import { afterEach, describe, expect, it } from "vitest";
import sidebar from "./InboxSidebar.vue?raw";

// The "undo later" button on a set aside row is drawn over the end of the title in the small window, which keeps no room for
// it. It once had no fill there (the small window's rules out-rank the hover look, so the hover fill never applied) and its
// label ran through the title text. jsdom neither tracks :hover nor sorts rules by specificity, so this does the cascade for
// the two properties that matter itself, with real selector matching, against the component's own stylesheet.
const css = /<style>([\s\S]*?)<\/style>/.exec(sidebar)![1];

interface Rule { selector: string; order: number; declarations: Record<string, string> }

function splitSelectors(list: string): string[] {
  const parts: string[] = [];
  let depth = 0, start = 0;
  for (let i = 0; i < list.length; i++) {
    if (list[i] === "(" || list[i] === "[") depth++;
    else if (list[i] === ")" || list[i] === "]") depth--;
    else if (list[i] === "," && depth === 0) { parts.push(list.slice(start, i)); start = i + 1; }
  }
  parts.push(list.slice(start));
  return parts.map(part => part.trim()).filter(Boolean);
}

// Flat rules only: the at-rules in this file hold nothing the button's look depends on.
function parse(text: string): Rule[] {
  const stripped = text.replace(/\/\*[\s\S]*?\*\//g, "");
  const pattern = /([^{}]+)\{([^{}]*)\}/g;
  const rules: Rule[] = [];
  let order = 0;
  for (let match = pattern.exec(stripped); match; match = pattern.exec(stripped)) {
    const declarations: Record<string, string> = {};
    for (const declaration of match[2].split(";")) {
      const colon = declaration.indexOf(":");
      if (colon > 0) declarations[declaration.slice(0, colon).trim()] = declaration.slice(colon + 1).trim();
    }
    for (const selector of splitSelectors(match[1])) rules.push({ selector, order: order++, declarations });
  }
  return rules;
}

function specificity(selector: string): number {
  const flat = selector.replace(/:not\(([^)]*)\)/g, " $1 ");
  const ids = (flat.match(/#[\w-]+/g) ?? []).length;
  const classes = (flat.match(/\.[\w-]+|\[[^\]]*\]|:(?!:)[\w-]+/g) ?? []).length;
  const types = (flat.replace(/\[[^\]]*\]|[.#:][\w-]+/g, " ").match(/(?:^|[\s>+~])[a-z][\w-]*/gi) ?? []).length;
  return ids * 10000 + classes * 100 + types;
}

// The component's :hover / :focus-visible become classes the fixture sets.
const states = (selector: string) => selector.replaceAll(":hover", ".is-hovered").replaceAll(":focus-visible", ".is-focused");
const rules = parse(css);

function cascade(element: Element, property: string): string | undefined {
  let winner: { specificity: number; order: number; value: string } | undefined;
  for (const rule of rules) {
    const value = rule.declarations[property];
    if (value === undefined) continue;
    const selector = states(rule.selector);
    let matches = false;
    try { matches = element.matches(selector); } catch { /* a pseudo-element or something jsdom cannot match */ }
    if (!matches) continue;
    const candidate = { specificity: specificity(selector), order: rule.order, value };
    if (!winner || candidate.specificity > winner.specificity || (candidate.specificity === winner.specificity && candidate.order > winner.order)) winner = candidate;
  }
  return winner?.value;
}

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
