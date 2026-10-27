// Push timing of the iOS channel (docs/specs/ios-alert-timing.md): the waits offered in the
// settings and how a duration is split into a number and a unit for display.

/** Seconds offered in the settings; `0` pushes as soon as a question arrives. */
export const ALERT_AFTER_PRESETS: readonly number[] = [0, 300, 900, 1800, 3600];

export type DurationUnit = "seconds" | "minutes" | "hours";

export interface SplitDuration {
  n: number;
  unit: DurationUnit;
}

/** A configured wait as the largest unit that divides it evenly: 900 → 15 minutes, 7200 → 2 hours. */
export function splitDuration(secs: number): SplitDuration {
  if (secs >= 3600 && secs % 3600 === 0) return { n: secs / 3600, unit: "hours" };
  if (secs >= 60 && secs % 60 === 0) return { n: secs / 60, unit: "minutes" };
  return { n: secs, unit: "seconds" };
}

/** How long the Mac has been idle, rounded down: seconds below a minute, minutes below an hour. */
export function idleDuration(secs: number): SplitDuration {
  if (secs < 60) return { n: Math.max(0, Math.floor(secs)), unit: "seconds" };
  if (secs < 3600) return { n: Math.floor(secs / 60), unit: "minutes" };
  return { n: Math.floor(secs / 3600), unit: "hours" };
}

/**
 * The waits to list: the presets plus the configured value when it is none of them (a hand-edited
 * config file), in ascending order so a custom value never hides among the presets.
 */
export function alertAfterChoices(current: number): number[] {
  const choices = [...ALERT_AFTER_PRESETS];
  if (Number.isFinite(current) && current >= 0 && !choices.includes(current)) choices.push(current);
  return choices.sort((a, b) => a - b);
}
