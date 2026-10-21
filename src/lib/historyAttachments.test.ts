import { describe, expect, it } from "vitest";
import type { HistoryAnswer, HistoryEntry } from "./types";
import {
  NO_ATTACHMENTS,
  NO_EXTENSION,
  attachmentChipKey,
  attachmentChipsOf,
  attachmentCountOf,
  attachmentFacet,
  attachmentFactsOf,
  buildAttachmentIndex,
  emptyAttachmentFilter,
  extensionOf,
  isAttachmentFilterActive,
  isAttachmentRequirementImplied,
  matchesAttachmentFilter,
  removeAttachmentChip,
  setAttachmentRequired,
  setAttachmentSource,
  toggleAttachmentExt,
  type AttachmentFilter,
} from "./historyAttachments";

function answer(images: string[] = [], files: string[] = []): HistoryAnswer {
  return { selectedOptions: [], userInput: null, images, files };
}

function entry(id: string, question: string[] = [], answers: HistoryAnswer[] = []): HistoryEntry {
  return {
    id,
    timestampMs: 1,
    project: "/p",
    source: "Codex",
    channel: "popup",
    action: "send",
    isMarkdown: true,
    message: {
      text: "",
      files: question.map((path) => ({ path, name: path.split(/[\\/]/).pop() ?? path, size: 1, isImage: false })),
    },
    questions: [],
    answers,
  };
}

function filter(overrides: Partial<AttachmentFilter> = {}): AttachmentFilter {
  return { ...emptyAttachmentFilter(), ...overrides };
}

describe("extensionOf", () => {
  it("takes what follows the last dot, lowercase and without the dot", () => {
    expect(extensionOf("report.md")).toBe("md");
    expect(extensionOf("SHOT.PNG")).toBe("png");
    expect(extensionOf("backup.tar.gz")).toBe("gz");
    expect(extensionOf(".config.json")).toBe("json");
  });

  it("reads the last path segment with either separator", () => {
    expect(extensionOf("/Users/me/dir.d/file.txt")).toBe("txt");
    expect(extensionOf("C:\\work\\Docs\\Plan.DOCX")).toBe("docx");
    expect(extensionOf("/Users/me/dir.d/Makefile")).toBe(NO_EXTENSION);
  });

  it("treats dot files, trailing dots, bare names and directories as having no suffix", () => {
    expect(extensionOf(".gitignore")).toBe(NO_EXTENSION);
    expect(extensionOf("name.")).toBe(NO_EXTENSION);
    expect(extensionOf("name. ")).toBe(NO_EXTENSION);
    expect(extensionOf("Makefile")).toBe(NO_EXTENSION);
    expect(extensionOf("/Users/me/folder/")).toBe(NO_EXTENSION);
    expect(extensionOf("")).toBe(NO_EXTENSION);
  });
});

describe("attachmentFactsOf", () => {
  it("shares one frozen value between entries without attachments", () => {
    expect(attachmentFactsOf(entry("a"))).toBe(NO_ATTACHMENTS);
    expect(attachmentFactsOf(entry("b", [], [answer(), answer()]))).toBe(NO_ATTACHMENTS);
  });

  it("counts question attachments by the name shown to the user", () => {
    const e = entry("a", ["/x/one.md", "/x/two.md", "/x/pic.PNG"]);
    const facts = attachmentFactsOf(e);
    expect(facts.question).toBe(3);
    expect(facts.reply).toBe(0);
    expect([...facts.questionExts].sort()).toEqual(["md", "png"]);
    expect(facts.replyExts).toEqual([]);

    const renamed = entry("b");
    renamed.message.files = [{ path: "/x/blob", name: "notes.txt", size: 1, isImage: false }];
    expect(attachmentFactsOf(renamed).questionExts).toEqual(["txt"]);
  });

  it("counts the images and files of every answer as reply attachments", () => {
    const e = entry("a", [], [answer(["/i/a.png"], ["/f/b.pdf"]), answer(["/i/c.JPG"], [])]);
    const facts = attachmentFactsOf(e);
    expect(facts.question).toBe(0);
    expect(facts.reply).toBe(3);
    expect([...facts.replyExts].sort()).toEqual(["jpg", "pdf", "png"]);
  });

  it("keeps the two kinds apart but unions them for the all-source view", () => {
    const e = entry("a", ["/q/one.md"], [answer(["/i/a.png"], ["/f/notes.md"])]);
    const facts = attachmentFactsOf(e);
    expect(facts.questionExts).toEqual(["md"]);
    expect([...facts.replyExts].sort()).toEqual(["md", "png"]);
    expect([...facts.allExts].sort()).toEqual(["md", "png"]);
    expect(attachmentCountOf(facts, "all")).toBe(3);
    expect(attachmentCountOf(facts, "question")).toBe(1);
    expect(attachmentCountOf(facts, "reply")).toBe(2);
  });

  it("tolerates rows that lack the optional collections", () => {
    const sparse = entry("a") as unknown as { message: unknown; answers: unknown };
    sparse.message = { text: "" };
    sparse.answers = [{ selectedOptions: [] }];
    expect(attachmentFactsOf(sparse as HistoryEntry)).toBe(NO_ATTACHMENTS);
  });
});

describe("buildAttachmentIndex", () => {
  it("maps every entry id to its facts", () => {
    const a = entry("a", ["/q/one.md"]);
    const b = entry("b");
    const index = buildAttachmentIndex([a, b]);
    expect(index.size).toBe(2);
    expect(index.get("a")?.question).toBe(1);
    expect(index.get("b")).toBe(NO_ATTACHMENTS);
  });
});

describe("matchesAttachmentFilter", () => {
  const none = attachmentFactsOf(entry("none"));
  const question = attachmentFactsOf(entry("q", ["/q/spec.md", "/q/shot.png"]));
  const reply = attachmentFactsOf(entry("r", [], [answer(["/i/a.png"], ["/f/data"])]));
  const both = attachmentFactsOf(entry("b", ["/q/spec.md"], [answer(["/i/a.png"])]));

  it("lets everything through while nothing is selected", () => {
    for (const facts of [none, question, reply, both]) {
      expect(matchesAttachmentFilter(facts, filter())).toBe(true);
    }
  });

  it("keeps entries with any attachment for 'has attachments'", () => {
    const f = filter({ withAttachments: true });
    expect(matchesAttachmentFilter(none, f)).toBe(false);
    expect(matchesAttachmentFilter(question, f)).toBe(true);
    expect(matchesAttachmentFilter(reply, f)).toBe(true);
  });

  it("implies 'has attachments' for a source on its own", () => {
    const q = filter({ source: "question" });
    expect(matchesAttachmentFilter(question, q)).toBe(true);
    expect(matchesAttachmentFilter(reply, q)).toBe(false);
    expect(matchesAttachmentFilter(both, q)).toBe(true);
    const r = filter({ source: "reply" });
    expect(matchesAttachmentFilter(question, r)).toBe(false);
    expect(matchesAttachmentFilter(reply, r)).toBe(true);
    expect(matchesAttachmentFilter(none, r)).toBe(false);
  });

  it("matches any of the selected suffixes and implies 'has attachments'", () => {
    expect(matchesAttachmentFilter(question, filter({ exts: ["md"] }))).toBe(true);
    expect(matchesAttachmentFilter(question, filter({ exts: ["pdf", "png"] }))).toBe(true);
    expect(matchesAttachmentFilter(question, filter({ exts: ["pdf"] }))).toBe(false);
    expect(matchesAttachmentFilter(none, filter({ exts: ["md"] }))).toBe(false);
  });

  it("restricts the suffix match to the chosen source", () => {
    // Both carry a .png reply image, but only `both` has a .md in the question.
    expect(matchesAttachmentFilter(both, filter({ source: "question", exts: ["png"] }))).toBe(false);
    expect(matchesAttachmentFilter(both, filter({ source: "reply", exts: ["png"] }))).toBe(true);
    expect(matchesAttachmentFilter(both, filter({ source: "question", exts: ["md"] }))).toBe(true);
    expect(matchesAttachmentFilter(question, filter({ source: "reply", exts: ["md"] }))).toBe(false);
  });

  it("matches files without a suffix through the dedicated key", () => {
    expect(matchesAttachmentFilter(reply, filter({ exts: [NO_EXTENSION] }))).toBe(true);
    expect(matchesAttachmentFilter(question, filter({ exts: [NO_EXTENSION] }))).toBe(false);
  });
});

describe("attachmentFacet", () => {
  const entries = [
    entry("1", ["/q/a.md", "/q/b.md"]),
    entry("2", ["/q/c.md", "/q/d.png"]),
    entry("3", [], [answer(["/i/e.png"], ["/f/notes"])]),
    entry("4", [], [answer(["/i/f.png", "/i/g.PNG"])]),
    entry("5"),
  ];
  const index = buildAttachmentIndex(entries);

  it("counts entries rather than files and orders by count, then name, with no-suffix last", () => {
    const facet = attachmentFacet(entries, index, "all", []);
    expect(facet.entries).toBe(4);
    // Entry 4 carries two .png files but counts once; entry 1 has two .md files and counts once.
    expect(facet.options).toEqual([
      { ext: "png", count: 3 },
      { ext: "md", count: 2 },
      { ext: NO_EXTENSION, count: 1 },
    ]);
  });

  it("keeps 'no suffix' last even when it is the most common", () => {
    const folders = [entry("a", ["/x/dir1/"]), entry("b", ["/x/dir2/"]), entry("c", ["/x/one.md"])];
    const facet = attachmentFacet(folders, buildAttachmentIndex(folders), "all", []);
    expect(facet.options).toEqual([
      { ext: "md", count: 1 },
      { ext: NO_EXTENSION, count: 2 },
    ]);
  });

  it("breaks count ties alphabetically", () => {
    const tied = [entry("a", ["/x/1.zip"]), entry("b", ["/x/2.csv"]), entry("c", ["/x/3.json"])];
    const facet = attachmentFacet(tied, buildAttachmentIndex(tied), "all", []);
    expect(facet.options.map((o) => o.ext)).toEqual(["csv", "json", "zip"]);
  });

  it("follows the chosen source", () => {
    expect(attachmentFacet(entries, index, "question", []).options).toEqual([
      { ext: "md", count: 2 },
      { ext: "png", count: 1 },
    ]);
    const reply = attachmentFacet(entries, index, "reply", []);
    expect(reply.entries).toBe(2);
    expect(reply.options).toEqual([
      { ext: "png", count: 2 },
      { ext: NO_EXTENSION, count: 1 },
    ]);
  });

  it("ignores the entries it was not given, so scope and keywords narrow the choices", () => {
    const facet = attachmentFacet([entries[2]], index, "all", []);
    expect(facet.entries).toBe(1);
    expect(facet.options).toEqual([
      { ext: "png", count: 1 },
      { ext: NO_EXTENSION, count: 1 },
    ]);
  });

  it("does not let selected suffixes change any count, and keeps a selected suffix that no entry carries", () => {
    const plain = attachmentFacet(entries, index, "all", []);
    const selected = attachmentFacet(entries, index, "all", ["md", "pdf"]);
    expect(selected.options.filter((o) => o.ext !== "pdf")).toEqual(plain.options);
    expect(selected.options.find((o) => o.ext === "pdf")).toEqual({ ext: "pdf", count: 0 });
    expect(selected.options.map((o) => o.ext)).toEqual(["png", "md", "pdf", NO_EXTENSION]);
  });

  it("is empty when nothing in scope has attachments", () => {
    const facet = attachmentFacet([entries[4]], index, "all", []);
    expect(facet).toEqual({ entries: 0, options: [] });
  });
});

describe("filter state", () => {
  it("is inactive until a source, the explicit switch or a suffix is chosen", () => {
    expect(isAttachmentFilterActive(filter())).toBe(false);
    expect(isAttachmentFilterActive(filter({ withAttachments: true }))).toBe(true);
    expect(isAttachmentFilterActive(filter({ source: "reply" }))).toBe(true);
    expect(isAttachmentFilterActive(filter({ exts: ["md"] }))).toBe(true);
  });

  it("treats the explicit switch as implied by a source or a suffix", () => {
    expect(isAttachmentRequirementImplied(filter({ withAttachments: true }))).toBe(false);
    expect(isAttachmentRequirementImplied(filter({ source: "question" }))).toBe(true);
    expect(isAttachmentRequirementImplied(filter({ exts: [NO_EXTENSION] }))).toBe(true);
  });

  it("toggles suffixes without mutating the previous filter and keeps selection order", () => {
    const start = filter();
    const one = toggleAttachmentExt(start, "png");
    const two = toggleAttachmentExt(one, "md");
    expect(start.exts).toEqual([]);
    expect(one.exts).toEqual(["png"]);
    expect(two.exts).toEqual(["png", "md"]);
    expect(toggleAttachmentExt(two, "png").exts).toEqual(["md"]);
  });

  it("keeps the explicit switch while a suffix comes and goes", () => {
    let current = setAttachmentRequired(filter(), true);
    current = toggleAttachmentExt(current, "md");
    current = toggleAttachmentExt(current, "md");
    expect(current.withAttachments).toBe(true);
    expect(isAttachmentFilterActive(current)).toBe(true);
    expect(attachmentChipsOf(current)).toEqual([{ kind: "has" }]);
  });

  it("changes the source without touching the other conditions", () => {
    const current = setAttachmentSource(filter({ exts: ["md"], withAttachments: true }), "reply");
    expect(current).toEqual({ source: "reply", withAttachments: true, exts: ["md"] });
  });
});

describe("chips", () => {
  it("lists the source, else the explicit switch, then one chip per suffix", () => {
    expect(attachmentChipsOf(filter())).toEqual([]);
    expect(attachmentChipsOf(filter({ withAttachments: true }))).toEqual([{ kind: "has" }]);
    expect(attachmentChipsOf(filter({ source: "question" }))).toEqual([{ kind: "source", source: "question" }]);
    // The explicit switch is implied by a source or a suffix, so it does not get its own chip.
    expect(attachmentChipsOf(filter({ source: "reply", withAttachments: true, exts: ["md", NO_EXTENSION] }))).toEqual([
      { kind: "source", source: "reply" },
      { kind: "ext", ext: "md" },
      { kind: "ext", ext: NO_EXTENSION },
    ]);
    expect(attachmentChipsOf(filter({ withAttachments: true, exts: ["png"] }))).toEqual([{ kind: "ext", ext: "png" }]);
  });

  it("gives every chip a distinct stable key", () => {
    const keys = attachmentChipsOf(filter({ source: "question", exts: ["md", NO_EXTENSION] })).map(attachmentChipKey);
    expect(keys).toEqual(["source:question", "ext:md", "ext:"]);
    expect(attachmentChipKey({ kind: "has" })).toBe("has");
  });

  it("removes only the condition a chip stands for", () => {
    const full = filter({ source: "question", withAttachments: true, exts: ["md", "png"] });
    expect(removeAttachmentChip(full, { kind: "ext", ext: "md" })).toEqual({ ...full, exts: ["png"] });
    expect(removeAttachmentChip(full, { kind: "source", source: "question" })).toEqual({ ...full, source: "all" });
    expect(removeAttachmentChip(filter({ withAttachments: true }), { kind: "has" })).toEqual(filter());
  });
});
