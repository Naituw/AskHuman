// Attachment facts and filtering for the history window (docs/specs/reply-history.md §3.7).
//
// An entry's attachments are the files sent with the question (`message.files`) plus the images and files
// the user replied with. Facts are computed once per loaded entry; the filter, the suffix counts and the
// list badge all read those, so typing in the search box never re-parses paths.

import type { HistoryEntry } from "./types";

/** Which attachments a condition looks at. */
export type AttachmentSource = "all" | "question" | "reply";

/** Suffix key of attachments without one (directories, `Makefile`, …). A real suffix is never empty. */
export const NO_EXTENSION = "";

export interface AttachmentFilter {
  /** Narrows "has attachments" and the suffix match to one kind of attachment. */
  source: AttachmentSource;
  /** The user's explicit "only entries with attachments"; implied by a non-`all` source or any suffix. */
  withAttachments: boolean;
  /** Selected suffixes (lowercase, no dot, `NO_EXTENSION` for none) in selection order; any one matches. */
  exts: string[];
}

export interface AttachmentFacts {
  /** Attachments sent with the question. */
  question: number;
  /** Images and files the user replied with. */
  reply: number;
  /** Distinct suffixes among each kind (and both). */
  questionExts: readonly string[];
  replyExts: readonly string[];
  allExts: readonly string[];
}

/** Shared by every entry without attachments, so the index stays small. */
export const NO_ATTACHMENTS: AttachmentFacts = Object.freeze({
  question: 0,
  reply: 0,
  questionExts: Object.freeze([]) as readonly string[],
  replyExts: Object.freeze([]) as readonly string[],
  allExts: Object.freeze([]) as readonly string[],
});

export function emptyAttachmentFilter(): AttachmentFilter {
  return { source: "all", withAttachments: false, exts: [] };
}

export function isAttachmentFilterActive(filter: AttachmentFilter): boolean {
  return filter.source !== "all" || filter.withAttachments || filter.exts.length > 0;
}

/** A source other than `all` or any suffix already means "has attachments"; the explicit switch is then moot. */
export function isAttachmentRequirementImplied(filter: AttachmentFilter): boolean {
  return filter.source !== "all" || filter.exts.length > 0;
}

/**
 * Lowercase suffix of a file name or path: what follows the last dot of the last path segment, without the
 * dot. `.gitignore`, `name.` and names without a dot have none; `a.tar.gz` is `gz`.
 */
export function extensionOf(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? "";
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return NO_EXTENSION;
  return name.slice(dot + 1).trim().toLowerCase();
}

function distinctExts(names: readonly string[]): string[] {
  return [...new Set(names.map(extensionOf))];
}

export function attachmentFactsOf(entry: HistoryEntry): AttachmentFacts {
  const questionNames = (entry.message?.files ?? []).map((file) => file.name || file.path);
  const replyNames: string[] = [];
  for (const answer of entry.answers ?? []) {
    replyNames.push(...(answer.images ?? []), ...(answer.files ?? []));
  }
  if (!questionNames.length && !replyNames.length) return NO_ATTACHMENTS;
  const questionExts = distinctExts(questionNames);
  const replyExts = distinctExts(replyNames);
  return {
    question: questionNames.length,
    reply: replyNames.length,
    questionExts,
    replyExts,
    allExts: [...new Set([...questionExts, ...replyExts])],
  };
}

/**
 * Facts by entry id. Pass the raw array rather than Vue's reactive proxy of it: reading two thousand entries
 * through proxies costs about ten times as much and the facts never need to track the entries' fields.
 */
export function buildAttachmentIndex(entries: readonly HistoryEntry[]): Map<string, AttachmentFacts> {
  const index = new Map<string, AttachmentFacts>();
  for (const entry of entries) index.set(entry.id, attachmentFactsOf(entry));
  return index;
}

export function attachmentCountOf(facts: AttachmentFacts, source: AttachmentSource): number {
  if (source === "question") return facts.question;
  if (source === "reply") return facts.reply;
  return facts.question + facts.reply;
}

export function attachmentExtsOf(facts: AttachmentFacts, source: AttachmentSource): readonly string[] {
  if (source === "question") return facts.questionExts;
  if (source === "reply") return facts.replyExts;
  return facts.allExts;
}

/** Whether an entry passes the attachment conditions (an inactive filter passes everything). */
export function matchesAttachmentFilter(facts: AttachmentFacts, filter: AttachmentFilter): boolean {
  if (!isAttachmentFilterActive(filter)) return true;
  if (attachmentCountOf(facts, filter.source) === 0) return false;
  if (!filter.exts.length) return true;
  const present = attachmentExtsOf(facts, filter.source);
  return filter.exts.some((ext) => present.includes(ext));
}

export interface AttachmentExtOption {
  ext: string;
  /** Entries in scope that carry this suffix; an entry counts once however many files share it. */
  count: number;
}

export interface AttachmentFacet {
  /** Entries in scope with at least one attachment of the chosen source. */
  entries: number;
  /** Suffixes by count (descending), then name; "no suffix" always last. */
  options: readonly AttachmentExtOption[];
}

export const EMPTY_ATTACHMENT_FACET: AttachmentFacet = Object.freeze({
  entries: 0,
  options: Object.freeze([]) as readonly AttachmentExtOption[],
});

function compareOptions(a: AttachmentExtOption, b: AttachmentExtOption): number {
  if ((a.ext === NO_EXTENSION) !== (b.ext === NO_EXTENSION)) return a.ext === NO_EXTENSION ? 1 : -1;
  if (a.count !== b.count) return b.count - a.count;
  return a.ext < b.ext ? -1 : a.ext > b.ext ? 1 : 0;
}

/**
 * The suffixes offered for the entries in scope. Counts ignore the suffixes already selected, so selecting
 * one never changes the others (and the list keeps its order while the user clicks); a selected suffix that
 * no entry in scope carries stays listed at zero so it can still be unselected.
 */
export function attachmentFacet(
  entries: readonly HistoryEntry[],
  index: ReadonlyMap<string, AttachmentFacts>,
  source: AttachmentSource,
  selected: readonly string[]
): AttachmentFacet {
  const counts = new Map<string, number>();
  let withAttachments = 0;
  for (const entry of entries) {
    const facts = index.get(entry.id);
    if (!facts || attachmentCountOf(facts, source) === 0) continue;
    withAttachments += 1;
    for (const ext of attachmentExtsOf(facts, source)) counts.set(ext, (counts.get(ext) ?? 0) + 1);
  }
  for (const ext of selected) if (!counts.has(ext)) counts.set(ext, 0);
  const options = [...counts].map(([ext, count]) => ({ ext, count })).sort(compareOptions);
  return { entries: withAttachments, options };
}

export function setAttachmentSource(filter: AttachmentFilter, source: AttachmentSource): AttachmentFilter {
  return { ...filter, source };
}

export function setAttachmentRequired(filter: AttachmentFilter, withAttachments: boolean): AttachmentFilter {
  return { ...filter, withAttachments };
}

export function toggleAttachmentExt(filter: AttachmentFilter, ext: string): AttachmentFilter {
  const exts = filter.exts.includes(ext) ? filter.exts.filter((item) => item !== ext) : [...filter.exts, ext];
  return { ...filter, exts };
}

/** One removable condition shown under the search row. */
export type AttachmentChip =
  | { kind: "has" }
  | { kind: "source"; source: "question" | "reply" }
  | { kind: "ext"; ext: string };

/** Conditions as the user sees them: the source, else the explicit "has attachments", then each suffix. */
export function attachmentChipsOf(filter: AttachmentFilter): AttachmentChip[] {
  const chips: AttachmentChip[] = [];
  if (filter.source !== "all") chips.push({ kind: "source", source: filter.source });
  else if (filter.withAttachments && !filter.exts.length) chips.push({ kind: "has" });
  for (const ext of filter.exts) chips.push({ kind: "ext", ext });
  return chips;
}

export function attachmentChipKey(chip: AttachmentChip): string {
  switch (chip.kind) {
    case "has":
      return "has";
    case "source":
      return `source:${chip.source}`;
    case "ext":
      return `ext:${chip.ext}`;
  }
}

export function removeAttachmentChip(filter: AttachmentFilter, chip: AttachmentChip): AttachmentFilter {
  switch (chip.kind) {
    case "has":
      return { ...filter, withAttachments: false };
    case "source":
      return { ...filter, source: "all" };
    case "ext":
      return { ...filter, exts: filter.exts.filter((ext) => ext !== chip.ext) };
  }
}
