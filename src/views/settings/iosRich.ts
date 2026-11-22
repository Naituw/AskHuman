// The few bits of inline markup the iOS App card's instructions use (docs/specs/ios-setup-experience.md
// §14). A message stays one translatable sentence instead of being cut into fragments around its
// links and copyable strings:
//   **bold**   `code`   [label](linkId)   [[chipId]]
// Nothing here produces HTML: the pieces are rendered as text by the component.

export type RichPiece =
  | { kind: "text"; text: string }
  | { kind: "bold"; text: string }
  | { kind: "code"; text: string }
  | { kind: "link"; text: string; id: string }
  | { kind: "chip"; id: string };

/** A string that copies on click: what it shows, and what a click copies. */
export interface RichChip {
  label: string;
  /** `null` while there is nothing real to copy yet (the label is a placeholder). */
  value: string | null;
}

// One alternation, so the markup is read left to right and cannot nest.
const TOKEN = /\*\*(.+?)\*\*|`([^`]+)`|\[\[([\w-]+)\]\]|\[([^\]]+)\]\(([\w-]+)\)/g;

export function parseRich(source: string): RichPiece[] {
  const pieces: RichPiece[] = [];
  let last = 0;
  for (const match of source.matchAll(TOKEN)) {
    const at = match.index ?? 0;
    if (at > last) pieces.push({ kind: "text", text: source.slice(last, at) });
    if (match[1] !== undefined) pieces.push({ kind: "bold", text: match[1] });
    else if (match[2] !== undefined) pieces.push({ kind: "code", text: match[2] });
    else if (match[3] !== undefined) pieces.push({ kind: "chip", id: match[3] });
    else pieces.push({ kind: "link", text: match[4]!, id: match[5]! });
    last = at + match[0].length;
  }
  if (last < source.length) pieces.push({ kind: "text", text: source.slice(last) });
  return pieces;
}
