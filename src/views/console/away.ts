// Away mode in the console (docs/specs/special-replies.md D25). Pure helpers: what the sidebar glyphs
// show, what the per-session "AskHuman" menu offers, and how long away mode has been on.
import type { AwaySessionMode, AwaySessionSetting, AwaySnapshot } from "../../lib/types";

/** Nothing set: no banner, no glyphs. */
export const emptyAway = (): AwaySnapshot => ({ global: null, sessions: [], escalatedWaiting: 0 });

/** The key the daemon uses for a registered Agent session. */
export const sidKey = (sessionId: string): string => `sid:${sessionId}`;

/** A session's own setting (`null` = it follows the global switch). */
export function ownMode(away: AwaySnapshot, sessionId: string): AwaySessionMode | null {
  const key = sidKey(sessionId);
  return away.sessions.find((s) => s.sessionKey === key)?.mode ?? null;
}

/**
 * Sessions that carry a glyph in the sidebar, by Agent session id: only those that were set on their
 * own (a moon for away, a crossed bell for "stop using AskHuman"). An "always ask normally" override
 * has no glyph, and neither does a session that merely follows the global switch.
 */
export function glyphModes(away: AwaySnapshot): Record<string, "away" | "mute"> {
  const out: Record<string, "away" | "mute"> = {};
  for (const s of away.sessions) {
    if (!s.sessionKey.startsWith("sid:")) continue;
    if (s.mode === "away" || s.mode === "mute") out[s.sessionKey.slice(4)] = s.mode;
  }
  return out;
}

export interface AwayChoice {
  /** What picking it asks the daemon for. */
  setting: AwaySessionSetting;
  /** Label key under `console.away.`. */
  label: string;
  checked: boolean;
}

/**
 * The per-session menu. While nobody is away it is normal / away / stop using AskHuman; while away
 * mode is on for everyone it is follow the global switch / always ask normally / stop using AskHuman.
 * A session of its own that is away while the global switch is on is shown as following it.
 */
export function sessionChoices(own: AwaySessionMode | null, globalOn: boolean): AwayChoice[] {
  if (globalOn) {
    return [
      { setting: "follow", label: "optFollow", checked: own === null || own === "away" },
      { setting: "normal", label: "optAlways", checked: own === "normal" },
      { setting: "mute", label: "optMute", checked: own === "mute" },
    ];
  }
  return [
    { setting: "follow", label: "optNormal", checked: own === null || own === "normal" },
    { setting: "away", label: "optAway", checked: own === "away" },
    { setting: "mute", label: "optMute", checked: own === "mute" },
  ];
}

/** The value shown on the pill next to "AskHuman" (key under `console.away.`). */
export function sessionValue(own: AwaySessionMode | null, globalOn: boolean): string {
  if (own === "mute") return "valueMute";
  if (own === "away" && !globalOn) return "valueAway";
  if (own === "normal" && globalOn) return "valueAlways";
  return globalOn ? "valueFollow" : "valueNormal";
}

/** `away` highlights the pill; everything else is neutral. */
export function sessionTone(own: AwaySessionMode | null, globalOn: boolean): "away" | "mute" | "plain" {
  if (own === "mute") return "mute";
  if (own === "away" && !globalOn) return "away";
  return "plain";
}

/** How long away mode has been on, as the pieces the i18n templates need. */
export function awayDuration(
  sinceMs: number,
  nowMs: number,
): { kind: "now" } | { kind: "m"; m: number } | { kind: "hm"; h: number; m: number } {
  const mins = Math.max(0, Math.floor((nowMs - sinceMs) / 60_000));
  if (mins < 1) return { kind: "now" };
  if (mins < 60) return { kind: "m", m: mins };
  return { kind: "hm", h: Math.floor(mins / 60), m: mins % 60 };
}
