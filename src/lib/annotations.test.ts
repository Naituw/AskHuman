import { beforeEach, describe, expect, it } from "vitest";
import { applyFindMarks, clearFindMarks } from "./findInDom";
import {
  MAX_QUOTE_CHARS,
  anchorFromRange,
  clearHighlights,
  codePointLength,
  currentAnchor,
  cutChars,
  draftMarks,
  flash,
  foldText,
  highlight,
  markDraft,
  marksOf,
  middleTruncate,
  overlapping,
  relocate,
  textOf,
  type PlacedNote,
} from "./annotations";

// The DOM half of docs/specs/message-annotations.md: the text coordinate system, selections turned into anchors,
// the highlight marks, and finding a passage again after the Markdown was redrawn.

function mount(html: string): HTMLElement {
  document.body.innerHTML = `<div id="root">${html}</div>`;
  return document.getElementById("root") as HTMLElement;
}

function textNodeWith(root: HTMLElement, needle: string): { node: Text; index: number } {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const index = (node as Text).data.indexOf(needle);
    if (index >= 0) return { node: node as Text, index };
  }
  throw new Error(`no text node holds "${needle}"`);
}

/** A range from the start of `from` to the end of `to` (the first occurrence of each). */
function rangeOf(root: HTMLElement, from: string, to = from): Range {
  const head = textNodeWith(root, from);
  const tail = textNodeWith(root, to);
  const range = document.createRange();
  range.setStart(head.node, head.index);
  range.setEnd(tail.node, tail.index + to.length);
  return range;
}

function anchorOf(root: HTMLElement, from: string, to = from) {
  const anchor = anchorFromRange(root, rangeOf(root, from, to));
  if (!anchor) throw new Error(`no anchor for "${from}"`);
  return anchor;
}

function note(id: number, anchor: { start: number; end: number; quote: string }): PlacedNote {
  return { id, ...anchor };
}

const ARTICLE =
  "<p>Alpha <strong>beta</strong> gamma</p><p>delta epsilon</p>";

beforeEach(() => {
  document.body.innerHTML = "";
  window.getSelection()?.removeAllRanges();
});

describe("textOf", () => {
  it("puts one newline between blocks and keeps inline text together", () => {
    expect(textOf(mount(ARTICLE))).toBe("Alpha beta gamma\ndelta epsilon");
  });

  it("treats a <br> as a line break", () => {
    expect(textOf(mount("<p>line one<br>line two</p>"))).toBe("line one\nline two");
  });

  it("numbers list items and nested blocks as separate lines", () => {
    expect(textOf(mount("<ul><li>one</li><li>two <em>x</em></li></ul>"))).toBe("one\ntwo x");
  });

  it("leaves out buttons, svg, Mermaid blocks and find-skip subtrees", () => {
    const root = mount(
      '<div class="code-block"><button class="code-copy">Copy</button><pre><code>let x = 1;</code></pre></div>' +
        '<div class="code-block mermaid-block"><pre>graph TD</pre><button>Show source</button></div>' +
        '<p>tail<svg><text>icon</text></svg><span data-find-skip>status</span></p>',
    );
    expect(textOf(root)).toBe("let x = 1;\ntail");
  });

  it("is not changed by highlights or find marks", () => {
    const root = mount(ARTICLE);
    const before = textOf(root);
    highlight(root, [{ id: 1, ...anchorOf(root, "beta", "delta"), n: 1 }]);
    expect(textOf(root)).toBe(before);
    applyFindMarks(root, "ta", false);
    expect(textOf(root)).toBe(before);
    clearFindMarks(root);
    clearHighlights(root);
    expect(textOf(root)).toBe(before);
  });
});

describe("anchorFromRange", () => {
  it("returns offsets into the text of the root and the folded quote", () => {
    const root = mount(ARTICLE);
    const anchor = anchorOf(root, "beta", "gamma");
    expect(anchor).toEqual({ start: 6, end: 16, quote: "beta gamma" });
    expect(textOf(root).slice(anchor.start, anchor.end)).toBe("beta gamma");
  });

  it("drops the whitespace at both ends of the selection", () => {
    const root = mount(ARTICLE);
    const first = textNodeWith(root, "Alpha ");
    const last = textNodeWith(root, " gamma");
    const range = document.createRange();
    range.setStart(first.node, 5); // the space after "Alpha"
    range.setEnd(last.node, 1); // the space before "gamma"
    expect(anchorFromRange(root, range)).toEqual({ start: 6, end: 10, quote: "beta" });
  });

  it("folds the break between blocks into one space in the quote", () => {
    const root = mount(ARTICLE);
    const anchor = anchorOf(root, "gamma", "delta");
    expect(anchor.quote).toBe("gamma delta");
    expect(textOf(root).slice(anchor.start, anchor.end)).toBe("gamma\ndelta");
  });

  it("handles a selection that starts and ends on elements (triple click)", () => {
    const root = mount(ARTICLE);
    const paragraphs = root.querySelectorAll("p");
    const range = document.createRange();
    range.setStart(paragraphs[0], 0);
    range.setEnd(paragraphs[1], 0);
    expect(anchorFromRange(root, range)?.quote).toBe("Alpha beta gamma");
  });

  it("trims a triple click on the last block, whose range ends at the start of the next element outside the root", () => {
    const root = mount(ARTICLE);
    const options = document.createElement("div");
    options.innerHTML = "<label>Option A</label>";
    document.body.appendChild(options);
    const last = root.querySelectorAll("p")[1];

    const byText = document.createRange();
    byText.setStart(last.firstChild as Text, 0);
    byText.setEnd(options.querySelector("label")?.firstChild as Text, 0);
    expect(anchorFromRange(root, byText)).toEqual({ start: 17, end: 30, quote: "delta epsilon" });

    const byElements = document.createRange();
    byElements.setStart(last, 0);
    byElements.setEnd(options, 0);
    expect(anchorFromRange(root, byElements)?.quote).toBe("delta epsilon");

    const afterRoot = document.createRange();
    afterRoot.setStart(last, 0);
    afterRoot.setEndAfter(root);
    expect(anchorFromRange(root, afterRoot)?.quote).toBe("delta epsilon");

    // The live selection takes the same way in.
    window.getSelection()?.addRange(byText);
    expect(currentAnchor(root)?.quote).toBe("delta epsilon");
  });

  it("trims a start that sits at the end of the element just before the root", () => {
    document.body.innerHTML = '<p id="head">Header</p><div id="root"><p>Alpha beta</p><p>gamma</p></div>';
    const root = document.getElementById("root") as HTMLElement;
    const head = document.getElementById("head")?.firstChild as Text;
    const range = document.createRange();
    range.setStart(head, head.data.length);
    range.setEnd(textNodeWith(root, "gamma").node, 5);
    expect(anchorFromRange(root, range)?.quote).toBe("Alpha beta gamma");
  });

  it("still refuses a range that covers text outside the root, at either end or both", () => {
    document.body.innerHTML = '<p id="head">Header</p><div id="root"><p>Alpha beta</p></div><p id="tail">Footer</p>';
    const root = document.getElementById("root") as HTMLElement;
    const head = document.getElementById("head")?.firstChild as Text;
    const tail = document.getElementById("tail")?.firstChild as Text;
    const inside = textNodeWith(root, "Alpha").node;
    const rangeBetween = (from: Node, fromOffset: number, to: Node, toOffset: number) => {
      const range = document.createRange();
      range.setStart(from, fromOffset);
      range.setEnd(to, toOffset);
      return range;
    };

    expect(anchorFromRange(root, rangeBetween(inside, 0, tail, 2))).toBeNull();
    expect(anchorFromRange(root, rangeBetween(head, 3, inside, inside.data.length))).toBeNull();
    expect(anchorFromRange(root, rangeBetween(head, 0, tail, tail.data.length))).toBeNull();
    const page = document.createRange();
    page.selectNodeContents(document.body);
    expect(anchorFromRange(root, page)).toBeNull();
    // Nothing of the root at all.
    expect(anchorFromRange(root, rangeBetween(tail, 0, tail, tail.data.length))).toBeNull();
    expect(anchorFromRange(root, rangeBetween(head, 0, head, head.data.length))).toBeNull();
  });

  it("answers null instead of throwing when the range lives in another tree than the root", () => {
    const root = mount(ARTICLE);
    const detached = document.createElement("div");
    detached.innerHTML = "<p>elsewhere</p>";
    const text = detached.firstChild?.firstChild as Text;
    const range = document.createRange();
    range.setStart(text, 0);
    range.setEnd(text, 4);
    expect(anchorFromRange(root, range)).toBeNull();
  });

  it("refuses a selection that reaches outside the root, a collapsed one and whitespace", () => {
    const root = mount(`${ARTICLE}<p id="gap"> </p>`);
    const outside = document.createElement("p");
    outside.textContent = "outside";
    document.body.appendChild(outside);
    const wide = document.createRange();
    wide.setStart(textNodeWith(root, "beta").node, 0);
    wide.setEnd(outside.firstChild as Text, 3);
    expect(anchorFromRange(root, wide)).toBeNull();

    const collapsed = document.createRange();
    collapsed.setStart(textNodeWith(root, "beta").node, 2);
    collapsed.collapse(true);
    expect(anchorFromRange(root, collapsed)).toBeNull();

    const blank = document.createRange();
    blank.selectNodeContents(root.querySelector("#gap") as HTMLElement);
    expect(anchorFromRange(root, blank)).toBeNull();
  });

  it("ignores a selection that only covers skipped content", () => {
    const root = mount('<p>text</p><div class="code-block mermaid-block"><pre>graph TD</pre></div>');
    const range = document.createRange();
    range.selectNodeContents(root.querySelector("pre") as HTMLElement);
    expect(anchorFromRange(root, range)).toBeNull();
  });

  it("middle-truncates a very long quote to 2000 characters but keeps the real offsets", () => {
    const long = "word ".repeat(1200).trim();
    const root = mount(`<p>${long}</p>`);
    const anchor = anchorOf(root, "word", "word word word word word word word word word word word word");
    const whole = anchorFromRange(root, (() => {
      const range = document.createRange();
      range.selectNodeContents(root.querySelector("p") as HTMLElement);
      return range;
    })()) as { start: number; end: number; quote: string };
    expect(anchor.quote.startsWith("word word")).toBe(true);
    expect(whole.end - whole.start).toBe(long.length);
    expect(codePointLength(whole.quote)).toBe(MAX_QUOTE_CHARS);
    expect(whole.quote).toContain("…");
  });

  it("reads the live selection through currentAnchor", () => {
    const root = mount(ARTICLE);
    expect(currentAnchor(root)).toBeNull();
    window.getSelection()?.addRange(rangeOf(root, "delta"));
    expect(currentAnchor(root)?.quote).toBe("delta");
  });

  it("still works across the marks of earlier notes", () => {
    const root = mount(ARTICLE);
    highlight(root, [{ id: 1, ...anchorOf(root, "beta"), n: 1 }]);
    const anchor = anchorOf(root, "beta", "delta");
    expect(anchor.quote).toBe("beta gamma delta");
    expect(overlapping([{ id: 1, ...anchorOf(root, "beta") }], anchor)).toBe(1);
  });
});

describe("highlight", () => {
  it("wraps the passage in one mark and numbers the last fragment", () => {
    const root = mount(ARTICLE);
    highlight(root, [{ id: 7, ...anchorOf(root, "beta"), n: 2 }]);
    const marks = marksOf(root, 7);
    expect(marks).toHaveLength(1);
    expect(marks[0].textContent).toBe("beta");
    expect(marks[0].className).toContain("an-last");
    expect(marks[0].getAttribute("data-annotation-n")).toBe("2");
  });

  it("marks several passages that share one text node", () => {
    const root = mount("<p>one two three four five</p>");
    const second = anchorOf(root, "two");
    const fourth = anchorOf(root, "four");
    highlight(root, [
      { id: 1, ...second, n: 1 },
      { id: 2, ...fourth, n: 2 },
    ]);
    expect(marksOf(root, 1).map(mark => mark.textContent)).toEqual(["two"]);
    expect(marksOf(root, 2).map(mark => mark.textContent)).toEqual(["four"]);
    expect(root.textContent).toBe("one two three four five");
  });

  it("marks neighbouring passages in the same text node without touching their text", () => {
    const root = mount("<p>aaa bbb ccc</p>");
    const first = anchorOf(root, "aaa");
    const last = anchorOf(root, "ccc");
    const middle = anchorOf(root, "bbb");
    highlight(root, [
      { id: 3, ...last, n: 3 },
      { id: 1, ...first, n: 1 },
      { id: 2, ...middle, n: 2 },
    ]);
    expect(root.querySelector("p")?.innerHTML).toBe(
      '<mark class="an-mark an-last" data-annotation-id="1" data-annotation-n="1">aaa</mark> ' +
        '<mark class="an-mark an-last" data-annotation-id="2" data-annotation-n="2">bbb</mark> ' +
        '<mark class="an-mark an-last" data-annotation-id="3" data-annotation-n="3">ccc</mark>',
    );
  });

  it("splits a passage across blocks into one mark per block and numbers only the last", () => {
    const root = mount(ARTICLE);
    highlight(root, [{ id: 1, ...anchorOf(root, "gamma", "delta"), n: 1 }]);
    const marks = marksOf(root, 1);
    expect(marks.map(mark => mark.textContent)).toEqual(["gamma", "delta"]);
    expect(marks.map(mark => mark.hasAttribute("data-annotation-n"))).toEqual([false, true]);
  });

  it("skips fragments that are only whitespace between blocks", () => {
    const root = mount("<p>first</p>\n<p>second</p>");
    highlight(root, [{ id: 1, ...anchorOf(root, "first", "second"), n: 1 }]);
    expect(marksOf(root, 1).map(mark => mark.textContent)).toEqual(["first", "second"]);
    expect(root.textContent).toBe("first\nsecond");
  });

  it("is idempotent and clearing restores the original markup", () => {
    const root = mount(ARTICLE);
    const original = root.innerHTML;
    const specs = [
      { id: 1, ...anchorOf(root, "Alpha"), n: 1 },
      { id: 2, ...anchorOf(root, "gamma", "delta"), n: 2 },
    ];
    highlight(root, specs);
    const once = root.innerHTML;
    highlight(root, specs);
    expect(root.innerHTML).toBe(once);
    clearHighlights(root);
    expect(root.innerHTML).toBe(original);
    expect(root.querySelector("p")?.childNodes).toHaveLength(3);
  });

  it("redraws with fewer notes and numbers by the given position", () => {
    const root = mount("<p>aaa bbb ccc</p>");
    const first = anchorOf(root, "aaa");
    const last = anchorOf(root, "ccc");
    highlight(root, [
      { id: 1, ...first, n: 1 },
      { id: 3, ...last, n: 2 },
    ]);
    highlight(root, [{ id: 3, ...last, n: 1 }]);
    expect(marksOf(root, 1)).toHaveLength(0);
    expect(marksOf(root, 3)[0].getAttribute("data-annotation-n")).toBe("1");
  });

  it("draws a draft outline that never disturbs the notes and goes away with null", () => {
    const root = mount("<p>aaa bbb ccc</p>");
    const first = anchorOf(root, "aaa");
    const draft = anchorOf(root, "ccc");
    highlight(root, [{ id: 1, ...first, n: 1 }]);
    markDraft(root, draft);
    expect(draftMarks(root).map(mark => mark.textContent)).toEqual(["ccc"]);
    expect(marksOf(root, 1)).toHaveLength(1);
    markDraft(root, null);
    expect(draftMarks(root)).toHaveLength(0);
    expect(marksOf(root, 1)).toHaveLength(1);
    markDraft(root, draft);
    clearHighlights(root);
    expect(root.innerHTML).toBe("<p>aaa bbb ccc</p>");
  });

  it("lives next to find marks in either order", () => {
    const root = mount("<p>one two three</p>");
    const two = anchorOf(root, "two");
    highlight(root, [{ id: 1, ...two, n: 1 }]);
    applyFindMarks(root, "wo", false);
    expect(root.querySelectorAll("mark[data-popup-find]")).toHaveLength(1);
    // A find mark that is still on screen only splits the passage into more fragments; it stays the same passage.
    highlight(root, [{ id: 1, ...two, n: 1 }]);
    expect(marksOf(root, 1).map(mark => mark.textContent).join("")).toBe("two");
    expect(marksOf(root, 1).filter(mark => mark.hasAttribute("data-annotation-n"))).toHaveLength(1);
    applyFindMarks(root, "three", false);
    expect(root.textContent).toBe("one two three");
    expect(textOf(root)).toBe("one two three");
    clearFindMarks(root);
    clearHighlights(root);
    expect(root.innerHTML).toBe("<p>one two three</p>");
  });

  it("flashes the marks of a note and clears the flash again", async () => {
    const root = mount(ARTICLE);
    highlight(root, [{ id: 1, ...anchorOf(root, "beta"), n: 1 }]);
    flash(root, 1, 5);
    expect(marksOf(root, 1)[0].classList.contains("an-flash")).toBe(true);
    await new Promise(resolve => setTimeout(resolve, 20));
    expect(marksOf(root, 1)[0].classList.contains("an-flash")).toBe(false);
  });
});

describe("relocate", () => {
  it("keeps the stored offsets while they still hold the quote", () => {
    const root = mount(ARTICLE);
    const stored = note(1, anchorOf(root, "beta"));
    expect(relocate(root, [stored])).toEqual([{ id: 1, start: stored.start, end: stored.end, located: true }]);
  });

  it("finds the quote again when text was added before it", () => {
    const root = mount("<p>one two three</p>");
    const stored = note(1, anchorOf(root, "two"));
    root.innerHTML = "<p>zero one two three</p>";
    const [result] = relocate(root, [stored]);
    expect(result.located).toBe(true);
    expect(textOf(root).slice(result.start, result.end)).toBe("two");
  });

  it("matches across a changed line break because quotes are folded", () => {
    const root = mount("<p>alpha</p><p>beta</p>");
    const stored = note(1, anchorOf(root, "alpha", "beta"));
    root.innerHTML = "<p>alpha beta</p>";
    const [result] = relocate(root, [stored]);
    expect(result.located).toBe(true);
    expect(textOf(root).slice(result.start, result.end)).toBe("alpha beta");
  });

  it("reports a note whose passage is gone and leaves its offsets alone", () => {
    const root = mount("<p>one two three</p>");
    const stored = note(1, anchorOf(root, "two"));
    root.innerHTML = "<p>one three</p>";
    expect(relocate(root, [stored])).toEqual([{ id: 1, start: stored.start, end: stored.end, located: false }]);
  });

  it("picks the occurrence closest to where the note was", () => {
    const root = mount("<p>foo bar foo baz foo</p>");
    const stored = note(1, { start: 8, end: 11, quote: "foo" });
    root.innerHTML = "<p>x foo bar foo baz foo</p>";
    const [result] = relocate(root, [stored]);
    expect(result).toMatchObject({ located: true, start: 10 });
  });

  it("never gives two notes the same passage", () => {
    const root = mount("<p>foo and foo</p>");
    const first = note(1, { start: 0, end: 3, quote: "foo" });
    const second = note(2, { start: 0, end: 3, quote: "foo" });
    const results = relocate(root, [first, second]);
    expect(results.map(item => item.located)).toEqual([true, true]);
    expect(results[0].start).not.toBe(results[1].start);
    const third = note(3, { start: 0, end: 3, quote: "foo" });
    expect(relocate(root, [first, second, third]).map(item => item.located)).toEqual([true, true, false]);
  });

  it("does not search for a middle-truncated quote", () => {
    const long = "word ".repeat(1200).trim();
    const root = mount(`<p>${long}</p>`);
    const whole = anchorFromRange(root, (() => {
      const range = document.createRange();
      range.selectNodeContents(root.querySelector("p") as HTMLElement);
      return range;
    })()) as { start: number; end: number; quote: string };
    const stored = note(1, whole);
    expect(relocate(root, [stored])[0].located).toBe(true);
    root.innerHTML = `<p>intro ${long}</p>`;
    expect(relocate(root, [stored])[0].located).toBe(false);
  });

  it("returns one result per note in the order given", () => {
    const root = mount("<p>aaa bbb ccc</p>");
    const notes = [note(9, anchorOf(root, "ccc")), note(4, anchorOf(root, "aaa"))];
    expect(relocate(root, notes).map(item => item.id)).toEqual([9, 4]);
  });
});

describe("overlapping", () => {
  it("answers the id of the first note that shares text", () => {
    const notes = [
      { id: 1, start: 0, end: 5 },
      { id: 2, start: 10, end: 20 },
    ];
    expect(overlapping(notes, { start: 4, end: 6 })).toBe(1);
    expect(overlapping(notes, { start: 15, end: 30 })).toBe(2);
    expect(overlapping(notes, { start: 5, end: 10 })).toBeNull();
  });
});

describe("text helpers", () => {
  it("counts and cuts by code point, not by UTF-16 unit", () => {
    expect(codePointLength("a😀b")).toBe(3);
    expect(cutChars("a😀bc", 2)).toBe("a😀");
    expect(cutChars("abc", 5)).toBe("abc");
  });

  it("folds whitespace into single spaces", () => {
    expect(foldText("  a \n\t b\u00a0c  ")).toBe("a b c");
  });

  it("truncates like the Rust side: head 999, an ellipsis, tail 1000", () => {
    const text = "a".repeat(1500) + "b".repeat(1500);
    const cut = middleTruncate(text, 2000);
    expect(codePointLength(cut)).toBe(2000);
    expect(cut.slice(0, 999)).toBe("a".repeat(999));
    expect(cut.charAt(999)).toBe("…");
    expect(cut.slice(1000)).toBe("b".repeat(1000));
    expect(middleTruncate("short", 2000)).toBe("short");
  });
});
