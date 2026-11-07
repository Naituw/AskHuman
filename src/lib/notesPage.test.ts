import { afterEach, describe, expect, it, vi } from "vitest";
import annotationsScript from "../../ios/AskHuman/Resources/WebMarkdown/annotations.js?raw";
import notesScript from "../../ios/AskHuman/Resources/WebMarkdown/notes.js?raw";

// notes.js is what lets the iOS Markdown page use the annotation engine (docs/plans/message-annotations.md §7.2):
// it draws the notes the app hands over after every drawing of the text, and tells the app what was selected and
// tapped. Here it runs in jsdom with the real engine bundle, a stand-in for render.js, and a stand-in for the
// app's script-message channel.

interface Page {
  render(source: string): void;
  setNotes(list: unknown): number;
  setPending(anchor: unknown): number;
  locate(id: number): number;
  flash(id: number): number;
  clearSelection(): number;
}
interface Engine {
  textOf(root: HTMLElement): string;
}
interface Posted {
  name: string;
  payload: unknown;
}

const globals = window as unknown as Record<string, unknown>;
const TEXT =
  "<h2>Plan</h2><p>Migrate <strong>users</strong> first, then orders.</p><p>Second paragraph here.</p>";

let posted: Posted[] = [];
let registrations: Array<Parameters<Document["addEventListener"]>> = [];

/** The page as it is when the app has loaded it: render.js's `AH.render`, the engine, then notes.js. */
function load(options: { renderer?: boolean } = {}): Page {
  document.body.innerHTML = '<div id="content"></div>';
  posted = [];
  globals.webkit = {
    messageHandlers: new Proxy({}, {
      get: (_target, name) => ({ postMessage: (payload: unknown) => posted.push({ name: String(name), payload }) }),
    }),
  };
  // The HTML stands for the Markdown: what render.js does that matters here is replacing the text of #content.
  globals.AH = options.renderer === false ? undefined : {
    render(source: string) {
      (document.getElementById("content") as HTMLElement).innerHTML = source;
    },
  };
  globals.AHAnnotations = new Function(`${annotationsScript}\nreturn AHAnnotations;`)();
  const listen = vi.spyOn(document, "addEventListener");
  new Function(notesScript)();
  registrations = [...listen.mock.calls];
  listen.mockRestore();
  return globals.AH as Page;
}

afterEach(() => {
  for (const [name, listener, options] of registrations) document.removeEventListener(name, listener, options);
  registrations = [];
  window.getSelection()?.removeAllRanges();
  vi.restoreAllMocks();
});

const engine = (): Engine => globals.AHAnnotations as Engine;
const content = (): HTMLElement => document.getElementById("content") as HTMLElement;

/** The text of the page as the engine counts it, for working out where a quote sits. */
function textOfPage(): string {
  return engine().textOf(content());
}

function noteOf(id: number, quote: string): { id: number; start: number; end: number; quote: string } {
  const start = textOfPage().indexOf(quote);
  if (start < 0) throw new Error(`"${quote}" is not on the page`);
  return { id, start, end: start + quote.length, quote };
}

function marks(id?: number): HTMLElement[] {
  const selector = id === undefined ? "mark[data-annotation-id]" : `mark[data-annotation-id="${id}"]`;
  return Array.from(document.querySelectorAll<HTMLElement>(selector));
}

function numberOf(id: number): string | null {
  const last = marks(id).find(mark => mark.classList.contains("an-last"));
  return last ? last.getAttribute("data-annotation-n") : null;
}

function messages(name: string): unknown[] {
  return posted.filter(item => item.name === name).map(item => item.payload);
}

function select(from: string, to = from): void {
  const walker = document.createTreeWalker(content(), NodeFilter.SHOW_TEXT);
  let head: { node: Text; at: number } | null = null;
  let tail: { node: Text; at: number } | null = null;
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const data = (node as Text).data;
    if (!head && data.includes(from)) head = { node: node as Text, at: data.indexOf(from) };
    if (data.includes(to)) tail = { node: node as Text, at: data.indexOf(to) + to.length };
  }
  if (!head || !tail) throw new Error(`cannot select "${from}"…"${to}"`);
  const range = document.createRange();
  range.setStart(head.node, head.at);
  range.setEnd(tail.node, tail.at);
  const selection = window.getSelection() as Selection;
  selection.removeAllRanges();
  selection.addRange(range);
}

/** The latest `selection` message, once the page has had its frame to say it. */
async function lastSelection(): Promise<unknown> {
  await vi.waitFor(() => expect(messages("selection").length).toBeGreaterThan(0));
  const all = messages("selection");
  return all[all.length - 1];
}

describe("drawing the notes", () => {
  it("keeps the notes it is given before there is any text, and draws them with the text", () => {
    const page = load();
    const probe = document.createElement("div");
    probe.innerHTML = TEXT;
    const start = engine().textOf(probe).indexOf("users first");
    const note = { id: 1, start, end: start + "users first".length, quote: "users first" };
    page.setNotes([note]);
    expect(marks()).toHaveLength(0);
    expect(messages("notes")).toHaveLength(0);

    page.render(TEXT);
    expect(marks().map(mark => mark.textContent).join("")).toBe("users first");
    expect(numberOf(1)).toBe("1");
    expect(messages("notes")).toEqual([[{ id: 1, start: note.start, end: note.end, located: true }]]);
  });

  it("draws them again every time the text is drawn again", () => {
    const page = load();
    page.render(TEXT);
    page.setNotes([noteOf(1, "users first")]);
    expect(marks(1).length).toBeGreaterThan(0);

    page.render(TEXT);
    expect(marks(1).map(mark => mark.textContent).join("")).toBe("users first");
    expect(messages("notes")).toHaveLength(2);
  });

  it("numbers the notes in reading order, whatever order they are given in", () => {
    const page = load();
    page.render(TEXT);
    page.setNotes([noteOf(7, "orders"), noteOf(3, "Migrate"), noteOf(9, "Second paragraph")]);
    expect(numberOf(3)).toBe("1");
    expect(numberOf(7)).toBe("2");
    expect(numberOf(9)).toBe("3");
  });

  it("says that a note whose passage is gone has no place, and draws nothing for it", () => {
    const page = load();
    page.render(TEXT);
    const kept = noteOf(2, "orders");
    page.setNotes([{ id: 1, start: 0, end: 14, quote: "text that is gone" }, kept]);
    expect(marks(1)).toHaveLength(0);
    // The note without a place takes no number from the ones that have one.
    expect(numberOf(2)).toBe("1");
    expect(messages("notes").pop()).toEqual([
      { id: 1, start: 0, end: 14, located: false },
      { id: 2, start: kept.start, end: kept.end, located: true },
    ]);
  });

  it("finds a passage by its quote when the offsets no longer hold it, and says where it was", () => {
    const page = load();
    page.render(TEXT);
    const real = noteOf(1, "orders");
    page.setNotes([{ id: 1, start: 0, end: 6, quote: "orders" }]);
    expect(marks(1).map(mark => mark.textContent).join("")).toBe("orders");
    expect(messages("notes").pop()).toEqual([{ id: 1, start: real.start, end: real.end, located: true }]);
  });

  it("gives the same page and the same report when it is given the same notes again", () => {
    const page = load();
    page.render(TEXT);
    const notes = [noteOf(1, "users first"), noteOf(2, "paragraph")];
    page.setNotes(notes);
    const html = content().innerHTML;
    const report = messages("notes").pop();
    page.setNotes(notes);
    expect(content().innerHTML).toBe(html);
    expect(messages("notes").pop()).toEqual(report);
  });

  it("ignores what is not a note", () => {
    const page = load();
    page.render(TEXT);
    page.setNotes([null, { id: "1", start: 0, end: 1 }, { id: 2, start: 0 }, noteOf(3, "orders")]);
    expect(marks().every(mark => mark.getAttribute("data-annotation-id") === "3")).toBe(true);
    page.setNotes("nothing");
    expect(marks()).toHaveLength(0);
  });

  it("leaves the text it counts in as it was", () => {
    const page = load();
    page.render(TEXT);
    const before = textOfPage();
    page.setNotes([noteOf(1, "users first"), noteOf(2, "paragraph")]);
    page.setPending({ start: 0, end: 4 });
    expect(textOfPage()).toBe(before);
  });
});

describe("the passage being written", () => {
  it("is outlined until it is taken away, and survives the notes being drawn again", () => {
    const page = load();
    page.render(TEXT);
    const anchor = noteOf(0, "then orders");
    expect(page.setPending({ start: anchor.start, end: anchor.end })).toBe(1);
    const outlined = () => Array.from(document.querySelectorAll("mark[data-annotation-draft]")).map(mark => mark.textContent).join("");
    expect(outlined()).toBe("then orders");

    page.setNotes([noteOf(1, "Migrate")]);
    expect(outlined()).toBe("then orders");
    page.render(TEXT);
    expect(outlined()).toBe("then orders");

    page.setPending(null);
    expect(outlined()).toBe("");
    expect(marks(1).length).toBeGreaterThan(0);
  });
});

describe("answers to the app", () => {
  it("locate says a number: where the passage starts, or -1 when it is not there", () => {
    const page = load();
    expect(page.locate(1)).toBe(-1);
    page.render(TEXT);
    page.setNotes([noteOf(1, "orders")]);
    // jsdom has no layout: the marks sit 120 below the page's top and the text starts at 20.
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
      return { top: this.matches("mark") ? 140 : 20 } as DOMRect;
    });
    expect(page.locate(1)).toBe(120);
    expect(page.locate(99)).toBe(-1);
  });

  it("flash makes the passage pulse, and answers a number even for a note that is not there", () => {
    const page = load();
    page.render(TEXT);
    page.setNotes([noteOf(1, "orders")]);
    expect(page.flash(1)).toBe(1);
    expect(marks(1).every(mark => mark.classList.contains("an-flash"))).toBe(true);
    expect(page.flash(99)).toBe(1);
  });
});

describe("what the person selects", () => {
  it("tells the passage, where it is and which note it touches", async () => {
    const page = load();
    page.render(TEXT);
    select("users", "first");
    const anchor = noteOf(0, "users first");
    expect(await lastSelection()).toEqual({ quote: "users first", start: anchor.start, end: anchor.end, overlapId: null });

    page.setNotes([noteOf(1, "users")]);
    select("first, then");
    const touching = noteOf(0, "first, then");
    await vi.waitFor(() => expect(messages("selection").pop()).toEqual({
      quote: "first, then", start: touching.start, end: touching.end, overlapId: null,
    }));
    select("Migrate", "users");
    const over = noteOf(0, "Migrate users");
    await vi.waitFor(() => expect(messages("selection").pop()).toEqual({
      quote: "Migrate users", start: over.start, end: over.end, overlapId: 1,
    }));
  });

  it("tells that there is none any more, and says nothing when nothing changed", async () => {
    const page = load();
    page.render(TEXT);
    select("orders");
    await lastSelection();
    const count = messages("selection").length;

    select("orders");
    await new Promise(resolve => setTimeout(resolve, 80));
    expect(messages("selection")).toHaveLength(count);

    expect(page.clearSelection()).toBe(1);
    await vi.waitFor(() => expect(messages("selection").pop()).toBeNull());
  });

  it("takes a selection that ends just outside the text, as a triple tap on the last block leaves it", async () => {
    const page = load();
    page.render(TEXT);
    const after = document.createElement("div");
    after.textContent = "Option A";
    document.body.appendChild(after);
    const last = content().querySelectorAll("p")[1].firstChild as Text;
    const range = document.createRange();
    range.setStart(last, 0);
    range.setEnd(after.firstChild as Text, 0);
    (window.getSelection() as Selection).addRange(range);
    expect(await lastSelection()).toMatchObject({ quote: "Second paragraph here." });
  });

  it("says nothing of a selection that reaches outside the text", async () => {
    const page = load();
    page.render(TEXT);
    const after = document.createElement("div");
    after.textContent = "Option A";
    document.body.appendChild(after);
    const range = document.createRange();
    range.setStart(content().querySelectorAll("p")[1].firstChild as Text, 0);
    range.setEnd(after.firstChild as Text, 6);
    (window.getSelection() as Selection).addRange(range);
    await new Promise(resolve => setTimeout(resolve, 80));
    expect(messages("selection").every(payload => payload === null)).toBe(true);
  });

  it("does not take the text of a selection from before the text is drawn", async () => {
    load();
    content().innerHTML = "<p>not drawn by the renderer</p>";
    select("renderer");
    await new Promise(resolve => setTimeout(resolve, 80));
    expect(messages("selection")).toHaveLength(0);
  });
});

describe("a tap on a highlight", () => {
  function tap(target: Element): MouseEvent {
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    target.dispatchEvent(click);
    return click;
  }

  it("tells the app which note it was, and does not follow a link under it", () => {
    const page = load();
    page.render('<p>See <a href="https://example.com/">the docs</a> or orders.</p>');
    page.setNotes([noteOf(4, "the docs"), noteOf(5, "orders")]);
    const click = tap(marks(5)[0]);
    expect(messages("noteTap")).toEqual([5]);
    expect(click.defaultPrevented).toBe(true);

    const linked = tap(marks(4)[0]);
    expect(messages("noteTap")).toEqual([5, 4]);
    expect(linked.defaultPrevented).toBe(true);
  });

  it("is not a tap on the note when a selection is still there, or when it is on plain text", () => {
    const page = load();
    page.render(TEXT);
    page.setNotes([noteOf(1, "orders")]);
    select("Migrate");
    const during = tap(marks(1)[0]);
    expect(messages("noteTap")).toHaveLength(0);
    expect(during.defaultPrevented).toBe(false);

    window.getSelection()?.removeAllRanges();
    const plain = tap(content().querySelector("h2") as Element);
    expect(messages("noteTap")).toHaveLength(0);
    expect(plain.defaultPrevented).toBe(false);
  });
});

describe("a page without what it needs", () => {
  it("adds nothing when there is no renderer to add to", () => {
    expect(() => load({ renderer: false })).not.toThrow();
    expect(registrations).toHaveLength(0);
  });
});
