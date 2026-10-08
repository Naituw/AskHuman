import type { Ref } from "vue";
import type { DiffRow, ParsedDiff } from "./useAttachmentContent";
import { findAllRanges, type TextRange } from "../../lib/findInDom";

export type FindScope = "question" | "attachment";
export type AttachmentFindStatus = "ready" | "loading" | "searching" | "unsupported" | "noText" | "readFailed" | "limit" | "error";
export interface AttachmentFindState { total: number; current: number; status: AttachmentFindStatus }
export interface AttachmentFindAdapter {
  state: Ref<AttachmentFindState>;
  search(query: string, caseSensitive: boolean, navigate: boolean, reset?: boolean): void;
  go(delta: number): void | Promise<void>;
  refresh(): void;
  clear(): void;
  selection(): string;
  restoreFocus(): void;
}
export const DIFF_ROW_HEIGHT = 22;
export interface DiffFindLine { title: string | null; row: DiffRow | null }
export interface DiffFindMatch extends TextRange { line: number; occurrence: number }
export function diffFindLines(parsed: ParsedDiff): DiffFindLine[] {
  return parsed.sections.flatMap(section => [
    ...(section.title ? [{ title: section.title, row: null }] : []),
    ...section.rows.map(row => ({ title: null, row })),
  ]);
}
export function diffFindMatches(parsed: ParsedDiff, query: string, caseSensitive: boolean): DiffFindMatch[] {
  return diffFindLines(parsed).flatMap((line, index) =>
    findAllRanges(line.row?.text ?? line.title ?? "", query, caseSensitive)
      .map((range, occurrence) => ({ ...range, line: index, occurrence })),
  );
}
