// The quick replies the popup and the phone offer (docs/specs/special-replies.md D27). The daemon applies the same
// limits again when it saves the settings (`sanitize_quick_replies` in config.rs, keep the two alike); keeping them
// here lets the card show the list exactly as it will be stored.

/** The menu holds this many replies; the digits 1–9 and 0 pick them. */
export const QUICK_REPLIES_MAX = 10;
/** One reply is one line of at most this many characters. */
export const QUICK_REPLY_MAX_CHARS = 200;

/** Trims every reply, drops blank ones, cuts long ones and keeps the first {@link QUICK_REPLIES_MAX}. */
export function sanitizeQuickReplies(list: readonly string[]): string[] {
  const out: string[] = [];
  for (const raw of list) {
    const text = Array.from(raw.trim()).slice(0, QUICK_REPLY_MAX_CHARS).join("").trimEnd();
    if (text) out.push(text);
    if (out.length === QUICK_REPLIES_MAX) break;
  }
  return out;
}
