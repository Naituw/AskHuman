// What became of files dropped onto an answer popup, and what to tell the user about it (docs/specs/popup-inbox-collapse.md §6.9).
// A drop that added nothing used to be silent: the page logged to a console nobody reads. Now every drop ends in a result,
// the result becomes a notice the user can read and a line in the diagnostic log (src-tauri/src/popup_drop.rs), so that
// "my drop did nothing" can be told apart from "it never reached the window" afterwards. Kept free of Vue and Tauri so it
// can be tested without a window.
import type { DropLogEntry, DropLogOutcome, DropPlace, DropReadReason } from "../../lib/types";

/** A dropped picture that could not be read. `path` is for the log (the backend classes it by place and writes no path) and never shown. */
export interface DropFailure {
  path: string;
  name: string;
  reason: DropReadReason;
  place: DropPlace;
  /** The system's own words. */
  message: string;
}

/** What dropping files on a request came to. */
export interface DropResult {
  /** The request takes no files, so none were tried. */
  refused?: boolean;
  /** How many paths were dropped. */
  requested: number;
  /** Pictures and files that were new, and how many were there already. */
  images: number;
  files: number;
  duplicates: number;
  failures: DropFailure[];
}

export const fileName = (path: string) => path.split(/[\\/]/).pop() || "file";

export function emptyResult(requested: number): DropResult {
  return { requested, images: 0, files: 0, duplicates: 0, failures: [] };
}

export const addedCount = (result: DropResult) => result.images + result.files;

const REASONS: readonly DropReadReason[] = ["permission_denied", "not_found", "other"];
const PLACES: readonly DropPlace[] = ["desktop", "documents", "downloads", "icloud", "volume", "other"];

/** A failed read, whatever it rejected with: a `DropReadError` from the backend, or anything else (an older backend, a test). */
export function failureFrom(path: string, error: unknown): DropFailure {
  const e = typeof error === "object" && error !== null ? error as { reason?: unknown; where?: unknown; message?: unknown } : {};
  return {
    path,
    name: fileName(path),
    reason: REASONS.find(reason => reason === e.reason) ?? "other",
    place: PLACES.find(place => place === e.where) ?? "other",
    message: typeof e.message === "string" ? e.message : String(error),
  };
}

/** The one word for what a result came to, as the log and the notice both tell it. */
export function outcomeOf(result: DropResult): DropLogOutcome {
  if (result.refused) return "not_accepting";
  if (result.requested === 0) return "no_paths";
  if (addedCount(result) > 0) return result.failures.length ? "partial" : "added";
  return result.failures.length ? "read_failed" : "duplicates";
}

/** The log fields that say what a drop came to. */
export function resultLog(result: DropResult): Pick<DropLogEntry, "outcome" | "added" | "images" | "files" | "duplicates" | "failed"> {
  return {
    outcome: outcomeOf(result),
    added: addedCount(result),
    images: result.images,
    files: result.files,
    duplicates: result.duplicates,
    failed: result.failures.map(failure => ({ path: failure.path, reason: failure.reason })),
  };
}

export type Translate = (key: string, named?: Record<string, unknown>, plural?: number) => string;
export interface DropWords {
  t: Translate;
  /** macOS asks before an app reads the Desktop, Documents and Downloads folders: only there is the fix a setting to change. */
  mac: boolean;
  /** The platform's paste shortcut, offered for a picture that is not a file. */
  paste: string;
}
/** What to tell the user: a warning stays longer than a plain "added". */
export interface DropNotice {
  level: "info" | "warn";
  text: string;
}
/** How long a notice stays, in ms: a warning is longer, since it says what to do and has to be read. */
export const NOTICE_MS = { info: 2600, warn: 6000 } as const;

/** Drops that were turned away before anything was tried. */
export type DropIgnored = "no_paths" | "on_preview" | "not_accepting" | "busy";

export function describeIgnored(reason: DropIgnored, words: DropWords, title?: string): DropNotice {
  const { t } = words;
  switch (reason) {
    case "no_paths": return { level: "warn", text: t("popup.drop.noFiles", { paste: words.paste }) };
    case "on_preview": return { level: "warn", text: t("popup.drop.onPreview") };
    case "not_accepting": return { level: "warn", text: title ? t("popup.drop.notAccepting", { title }) : t("popup.drop.notAcceptingHere") };
    case "busy": return { level: "warn", text: t("popup.drop.busy") };
  }
}

export interface DropContext {
  /** The request's title when the window shows several requests; it names the request in the notice. A lone popup has none. */
  title?: string;
  /** The question the files went to (0-based). Given only when the request has several, so that the notice names it. */
  question?: number | null;
  /** Say that files were added. A lone popup shows them being added (the thumbnail appears), so it speaks only of trouble. */
  announce: boolean;
}

/** How to get a refused read going again: where to allow it, when the system asks the user first. */
function permissionFix(failures: DropFailure[], words: DropWords): string | null {
  if (!words.mac) return null;
  const denied = failures.find(failure => failure.reason === "permission_denied" && failure.place !== "other");
  if (!denied) return null;
  if (denied.place === "volume") return words.t("popup.drop.fixVolume");
  return words.t("popup.drop.fixFolder", { place: words.t(`popup.drop.place.${denied.place}`) });
}

/** What to tell the user about a drop that was handed to a request, or null when there is nothing worth saying. */
export function describeResult(result: DropResult, words: DropWords, context: DropContext): DropNotice | null {
  const { t } = words;
  if (result.refused) return describeIgnored("not_accepting", words, context.title);
  if (result.requested === 0) return describeIgnored("no_paths", words);
  const added = addedCount(result);
  const { failures } = result;
  if (!failures.length) {
    if (added === 0) return { level: "info", text: t("popup.drop.duplicates") };
    if (!context.announce || !context.title) return null;
    const title = context.title;
    return { level: "info", text: context.question == null
      ? t("popup.inbox.dropAdded", { n: added, title }, added)
      : t("popup.inbox.dropAddedQuestion", { n: added, title, q: context.question + 1 }, added) };
  }
  const why = [...new Set(failures.map(failure => t(`popup.drop.why.${failure.reason}`)))].join(t("popup.drop.joiner"));
  const fix = permissionFix(failures, words);
  if (added > 0) {
    const named = { n: added, m: failures.length, why };
    return { level: "warn", text: fix ? t("popup.drop.partialFix", { ...named, fix }, added) : t("popup.drop.partial", named, added) };
  }
  const what = failures.length === 1 ? t("popup.drop.whatOne", { name: failures[0].name }) : t("popup.drop.whatMany", { n: failures.length });
  if (failures.every(failure => failure.reason === "permission_denied")) {
    return { level: "warn", text: fix ? t("popup.drop.permission", { what, fix }) : t("popup.drop.permissionPlain", { what }) };
  }
  if (failures.every(failure => failure.reason === "not_found")) return { level: "warn", text: t("popup.drop.notFound", { what }, failures.length) };
  return { level: "warn", text: failures.length === 1
    ? t("popup.drop.readFailed", { what, detail: failures[0].message })
    : t("popup.drop.readFailedWhy", { what, why }) };
}

/**
 * The drag side of the log. A drag is logged once as it starts, however many native events announce it, and once more
 * if it ends by leaving the window; what became of a drop is reported by whoever knows it, through `send`. `base` gives
 * what every line carries (the window's form, the request it is about) at the moment of the line.
 */
export function createDragLog(send: (entry: DropLogEntry) => void, base: () => Partial<DropLogEntry>) {
  let started = false;
  return {
    /** A native `enter` or `over` came in. Only the first of a drag is logged; an `enter` always starts a new one. */
    seen(from: "enter" | "over", paths: string[] | undefined, more: Partial<DropLogEntry> = {}) {
      if (started && from === "over") return;
      started = true;
      send({ ...base(), action: "enter", from, paths: paths ?? [], ...more });
    },
    /** The drag left the window without dropping. A `leave` that follows no start is not a drag worth a line. */
    left() {
      if (!started) return;
      started = false;
      send({ ...base(), action: "leave" });
    },
    /** The drag ended with a drop, or the page starts over. */
    end() { started = false; },
    /** Any other line: what became of a drop, or a failure of the controller. */
    send(entry: DropLogEntry) { send({ ...base(), ...entry }); },
  };
}
