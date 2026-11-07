// Message annotations: the part that touches the DOM (docs/specs/message-annotations.md E4,
// docs/plans/message-annotations.md §6.1).
//
// A passage the user selects becomes {start, end, quote}. `start`/`end` are offsets into the "text of the root"
// and `quote` is the passage itself, which is what the Agent receives and what finds the passage again after the
// Markdown was redrawn (image loads, Mermaid, source view, ...).
//
// The text of the root is the concatenation of its text nodes in document order, with one "\n" wherever the
// reading flow breaks: between two block elements and at a <br>. Skipped subtrees (buttons, SVG, Mermaid blocks,
// ...) contribute nothing, so their toolbars never leak into a quote. Highlight marks and find marks only split
// text nodes, so they never change this text, and offsets stay valid while they come and go.
//
// This file is self-contained on purpose: no Vue, no imports and nothing newer than Safari 13, so the iOS web
// bundle can ship the very same engine.

export const MAX_ANNOTATIONS = 20;
export const MAX_NOTE_CHARS = 2000;
export const MAX_QUOTE_CHARS = 2000;

/** A selected passage in the coordinates of one root. */
export interface Anchor {
  start: number;
  end: number;
  /** Visible text folded to one line, middle-truncated at MAX_QUOTE_CHARS. */
  quote: string;
}

export interface PlacedNote extends Anchor {
  id: number;
}

export interface Relocation {
  id: number;
  start: number;
  end: number;
  /** False when neither the stored offsets nor a search for the quote found the passage. */
  located: boolean;
}

export interface MarkSpec {
  id: number;
  start: number;
  end: number;
  /** 1-based number drawn on the last fragment of the passage. */
  n: number;
}

export const MARK_CLASS = "an-mark";
export const DRAFT_CLASS = "an-draft";
export const LAST_CLASS = "an-last";
export const FLASH_CLASS = "an-flash";
export const ID_ATTR = "data-annotation-id";
export const NUMBER_ATTR = "data-annotation-n";
export const DRAFT_ATTR = "data-annotation-draft";

const SKIPPED =
  "script, style, textarea, input, button, svg, iframe, [data-find-skip], .mermaid-block";
const BLOCKS = new Set([
  "address", "article", "aside", "blockquote", "details", "dd", "div", "dl", "dt", "fieldset", "figcaption",
  "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hr", "li", "main", "nav", "ol", "p",
  "pre", "section", "summary", "table", "tbody", "td", "tfoot", "th", "thead", "tr", "ul",
]);

// ===== Plain text helpers (shared with the Rust side's sanitizing rules) =====

/** Number of Unicode code points, which is how the limits are counted (not UTF-16 units). */
export function codePointLength(text: string): number {
  return Array.from(text).length;
}

/** The first `max` code points. */
export function cutChars(text: string, max: number): string {
  const chars = Array.from(text);
  return chars.length <= max ? text : chars.slice(0, max).join("");
}

/** Collapse every whitespace run to one space and trim: the shape a quote is stored and compared in. */
export function foldText(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

/** Keep the head and the tail of a long text with "…" in between, `max` code points in total. */
export function middleTruncate(text: string, max: number): string {
  const chars = Array.from(text);
  if (chars.length <= max) return text;
  const head = Math.floor((max - 1) / 2);
  const tail = max - 1 - head;
  return `${chars.slice(0, head).join("")}…${chars.slice(chars.length - tail).join("")}`;
}

function quoteOf(text: string): string {
  return middleTruncate(foldText(text), MAX_QUOTE_CHARS);
}

function isTruncatedQuote(quote: string): boolean {
  if (codePointLength(quote) !== MAX_QUOTE_CHARS) return false;
  const head = Math.floor((MAX_QUOTE_CHARS - 1) / 2);
  return Array.from(quote)[head] === "…";
}

// ===== The text of a root =====

interface Segment {
  node: Text;
  start: number;
  end: number;
}

interface TextModel {
  text: string;
  segments: Segment[];
}

function blockOf(from: Element | null, root: Element): Element {
  for (let el = from; el && el !== root; el = el.parentElement) {
    if (BLOCKS.has(el.localName)) return el;
  }
  return root;
}

function buildModel(root: HTMLElement): TextModel {
  const segments: Segment[] = [];
  let text = "";
  let previousBlock: Element | null = null;
  let hasPrevious = false;
  let pendingBreak = false;
  const visit = (node: Node): void => {
    if (node.nodeType === 3) {
      const data = (node as Text).data;
      if (!data) return;
      const block = blockOf(node.parentElement, root);
      if (hasPrevious && (pendingBreak || block !== previousBlock)) text += "\n";
      pendingBreak = false;
      segments.push({ node: node as Text, start: text.length, end: text.length + data.length });
      text += data;
      previousBlock = block;
      hasPrevious = true;
      return;
    }
    if (node.nodeType !== 1) return;
    const element = node as Element;
    if (element !== root && element.matches(SKIPPED)) return;
    if (element.localName === "br") pendingBreak = true;
    for (let child = element.firstChild; child; child = child.nextSibling) visit(child);
  };
  visit(root);
  return { text, segments };
}

/** The text of `root` in annotation coordinates (see the file comment). */
export function textOf(root: HTMLElement): string {
  return buildModel(root).text;
}

// ===== Selection → anchor =====

/** The text a range from (`fromNode`, `fromOffset`) to (`toNode`, `toOffset`) covers, whitespace-trimmed. */
function textCovered(fromNode: Node, fromOffset: number, toNode: Node, toOffset: number): string {
  const covered = document.createRange();
  covered.setStart(fromNode, fromOffset);
  covered.setEnd(toNode, toOffset);
  return covered.toString().trim();
}

/**
 * `range` limited to `root`, or null when it covers text outside it (N7). A triple click on the last block of the
 * root, or a drag that ends just past its last line, leaves one end of the range at the very edge of the next (or
 * previous) element, outside the root, although nothing outside is highlighted; such an overreach holds no text
 * and is trimmed to the root's edge.
 */
function withinRoot(root: HTMLElement, range: Range): Range | null {
  const startsInside = root.contains(range.startContainer);
  const endsInside = root.contains(range.endContainer);
  if (startsInside && endsInside) return range;
  try {
    const edge = document.createRange();
    edge.selectNodeContents(root);
    const trimmed = range.cloneRange();
    if (!endsInside) {
      // The range must run on past the end of the root, and nothing but empty space may lie beyond it.
      if (range.comparePoint(edge.endContainer, edge.endOffset) !== 0) return null;
      if (textCovered(edge.endContainer, edge.endOffset, range.endContainer, range.endOffset)) return null;
      trimmed.setEnd(edge.endContainer, edge.endOffset);
    }
    if (!startsInside) {
      if (range.comparePoint(edge.startContainer, edge.startOffset) !== 0) return null;
      if (textCovered(range.startContainer, range.startOffset, edge.startContainer, edge.startOffset)) return null;
      trimmed.setStart(edge.startContainer, edge.startOffset);
    }
    return trimmed;
  } catch {
    // Comparing points throws when the range and the root are in different trees (the page was redrawn under the
    // selection): such a range has nothing to do with this root.
    return null;
  }
}

/** The anchor of `range`, or null when it is empty, only whitespace, or covers text outside `root`. */
export function anchorFromRange(root: HTMLElement, selected: Range): Anchor | null {
  const range = withinRoot(root, selected);
  if (!range) return null;
  const model = buildModel(root);
  let start = -1;
  let end = -1;
  for (const segment of model.segments) {
    if (!range.intersectsNode(segment.node)) continue;
    const from = segment.node === range.startContainer ? range.startOffset : 0;
    const to = segment.node === range.endContainer ? range.endOffset : segment.node.data.length;
    if (to <= from) continue;
    if (start < 0 || segment.start + from < start) start = segment.start + from;
    if (segment.start + to > end) end = segment.start + to;
  }
  if (start < 0 || end <= start) return null;
  const raw = model.text.slice(start, end);
  start += raw.length - raw.replace(/^\s+/, "").length;
  end -= raw.length - raw.replace(/\s+$/, "").length;
  if (end <= start) return null;
  return { start, end, quote: quoteOf(model.text.slice(start, end)) };
}

/** The anchor of the current selection when it lies inside `root`. */
export function currentAnchor(root: HTMLElement): Anchor | null {
  const selection = window.getSelection();
  if (!selection || selection.rangeCount === 0 || selection.isCollapsed) return null;
  return anchorFromRange(root, selection.getRangeAt(0));
}

/** The id of the first note whose passage shares text with `anchor`, if any. */
export function overlapping(
  notes: ReadonlyArray<{ id: number; start: number; end: number }>,
  anchor: { start: number; end: number },
): number | null {
  for (const note of notes) {
    if (note.start < anchor.end && anchor.start < note.end) return note.id;
  }
  return null;
}

// ===== Finding the passages again =====

function foldWithMap(text: string): { folded: string; map: number[] } {
  let folded = "";
  const map: number[] = [];
  let inSpace = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text.charAt(i);
    const code = ch.charCodeAt(0);
    const space = code <= 32 || (code > 126 && /\s/.test(ch));
    if (space) {
      if (!inSpace) {
        folded += " ";
        map.push(i);
        inSpace = true;
      }
    } else {
      folded += ch;
      map.push(i);
      inSpace = false;
    }
  }
  return { folded, map };
}

function overlapsAny(taken: Array<{ start: number; end: number }>, start: number, end: number): boolean {
  return taken.some(range => range.start < end && start < range.end);
}

/**
 * Place every note in the current text of `root`: the stored offsets when they still hold the quote, otherwise the
 * closest occurrence of the quote that no other note uses. A note whose passage is gone is reported unlocated.
 */
export function relocate(root: HTMLElement, notes: ReadonlyArray<PlacedNote>): Relocation[] {
  const { text } = buildModel(root);
  const result = new Map<number, Relocation>();
  const taken: Array<{ start: number; end: number }> = [];
  const ordered = [...notes].sort((a, b) => a.start - b.start);

  for (const note of ordered) {
    const inRange = note.start >= 0 && note.end <= text.length && note.start < note.end;
    if (inRange && quoteOf(text.slice(note.start, note.end)) === note.quote && !overlapsAny(taken, note.start, note.end)) {
      taken.push({ start: note.start, end: note.end });
      result.set(note.id, { id: note.id, start: note.start, end: note.end, located: true });
    }
  }

  const lost = ordered.filter(note => !result.has(note.id));
  const { folded, map } = lost.length > 0 ? foldWithMap(text) : { folded: "", map: [] as number[] };
  for (const note of lost) {
    let found: { start: number; end: number } | null = null;
    if (note.quote && !isTruncatedQuote(note.quote)) {
      const spots: Array<{ start: number; end: number }> = [];
      for (let at = folded.indexOf(note.quote); at !== -1; at = folded.indexOf(note.quote, at + 1)) {
        spots.push({ start: map[at], end: map[at + note.quote.length - 1] + 1 });
      }
      spots.sort((a, b) => Math.abs(a.start - note.start) - Math.abs(b.start - note.start));
      found = spots.find(spot => !overlapsAny(taken, spot.start, spot.end)) ?? null;
    }
    if (found) {
      taken.push(found);
      result.set(note.id, { id: note.id, start: found.start, end: found.end, located: true });
    } else {
      result.set(note.id, { id: note.id, start: note.start, end: note.end, located: false });
    }
  }
  return notes.map(note => result.get(note.id) as Relocation);
}

// ===== Highlights =====

function unwrap(root: HTMLElement, selector: string): void {
  const marks = Array.from(root.querySelectorAll<HTMLElement>(selector));
  const parents = new Set<Node>();
  for (const mark of marks) {
    const parent = mark.parentNode;
    if (!parent) continue;
    while (mark.firstChild) parent.insertBefore(mark.firstChild, mark);
    parent.removeChild(mark);
    parents.add(parent);
  }
  parents.forEach(parent => parent.normalize());
}

/** Wrap [start, end) in marks, one per text node it touches, and return them in document order. */
function wrapRange(model: TextModel, start: number, end: number, create: () => HTMLElement): HTMLElement[] {
  const parts: Array<{ node: Text; from: number; to: number }> = [];
  for (const segment of model.segments) {
    if (segment.end <= start) continue;
    if (segment.start >= end) break;
    const from = Math.max(start - segment.start, 0);
    const to = Math.min(end - segment.start, segment.node.data.length);
    if (from >= to || !/\S/.test(segment.node.data.slice(from, to))) continue;
    parts.push({ node: segment.node, from, to });
  }
  const marks: HTMLElement[] = [];
  for (let i = parts.length - 1; i >= 0; i--) {
    const { node, from, to } = parts[i];
    const middle = from > 0 ? node.splitText(from) : node;
    if (to - from < middle.data.length) middle.splitText(to - from);
    const mark = create();
    const parent = middle.parentNode;
    if (!parent) continue;
    parent.replaceChild(mark, middle);
    mark.appendChild(middle);
    marks.unshift(mark);
  }
  return marks;
}

/** Remove every highlight (notes and the draft) and put the text nodes back together. */
export function clearHighlights(root: HTMLElement): void {
  unwrap(root, `mark[${ID_ATTR}], mark[${DRAFT_ATTR}]`);
}

/** Draw the notes: purple marks, with the number on the last fragment of each passage. */
export function highlight(root: HTMLElement, marks: ReadonlyArray<MarkSpec>): void {
  clearHighlights(root);
  if (marks.length === 0) return;
  const model = buildModel(root);
  // Back to front, so splitting a text node never moves a passage that is still to be wrapped.
  const ordered = [...marks].sort((a, b) => b.start - a.start);
  for (const spec of ordered) {
    const fragments = wrapRange(model, spec.start, spec.end, () => {
      const mark = document.createElement("mark");
      mark.className = MARK_CLASS;
      mark.setAttribute(ID_ATTR, String(spec.id));
      return mark;
    });
    const last = fragments[fragments.length - 1];
    if (last) {
      last.classList.add(LAST_CLASS);
      last.setAttribute(NUMBER_ATTR, String(spec.n));
    }
  }
}

/** Outline the passage of a note being written; null removes the outline. */
export function markDraft(root: HTMLElement, anchor: Anchor | null): void {
  unwrap(root, `mark[${DRAFT_ATTR}]`);
  if (!anchor) return;
  const model = buildModel(root);
  wrapRange(model, anchor.start, anchor.end, () => {
    const mark = document.createElement("mark");
    mark.className = DRAFT_CLASS;
    mark.setAttribute(DRAFT_ATTR, "");
    return mark;
  });
}

/** The marks drawn for a note (one per text node), in document order. */
export function marksOf(root: HTMLElement, id: number): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(`mark[${ID_ATTR}="${id}"]`));
}

/** The marks of the passage being written. */
export function draftMarks(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(`mark[${DRAFT_ATTR}]`));
}

/** Scroll a note's passage into the middle of the view; false when it has no marks. */
export function scrollToNote(root: HTMLElement, id: number, smooth: boolean): boolean {
  const first = marksOf(root, id)[0];
  if (!first) return false;
  if (typeof first.scrollIntoView === "function") {
    first.scrollIntoView({ block: "center", behavior: smooth ? "smooth" : "auto" });
  }
  return true;
}

/** Pulse a note's passage once, so the eye finds it after a scroll. */
export function flash(root: HTMLElement, id: number, duration = 1200): void {
  const marks = marksOf(root, id);
  for (const mark of marks) {
    mark.classList.remove(FLASH_CLASS);
    void mark.offsetWidth;
    mark.classList.add(FLASH_CLASS);
  }
  setTimeout(() => marks.forEach(mark => mark.classList.remove(FLASH_CLASS)), duration);
}
