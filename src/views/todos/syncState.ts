// What the todos window and the settings card say about iCloud todo sync (spec todo-sync §9.7).
// One pure function turns the backend status into a single line, so the sidebar footer and the card
// agree, and the wording rules are testable without a window.
import type { TodoSyncStatus } from "../../lib/types";

/** How loud the line is: only trouble gets a colour. */
export type SyncTone = "off" | "ok" | "busy" | "warn" | "error";

export type SyncKey =
  | "localOnly"
  | "syncing"
  | "synced"
  | "offline"
  | "notSignedIn"
  | "accountChanged"
  | "zoneMissing"
  | "quota"
  | "helperMissing"
  | "helperDown"
  | "helperProblem"
  | "newerState"
  | "error";

/** What the user can do about it, right where the line is shown. */
export type SyncAction = "enable" | "repair" | "view" | "resume";

export interface SyncLine {
  tone: SyncTone;
  key: SyncKey;
  /** `synced`: how long ago the last pull or push succeeded. */
  ageMs?: number;
  /** `offline`: changes waiting for the cloud. */
  count?: number;
  /** `helperProblem` / `error`: the reason, as the backend reports it. */
  detail?: string;
  action?: SyncAction;
}

/**
 * The one line for the sidebar footer. `null` = nothing to show (no iCloud sync on this platform, or
 * the status has not arrived yet).
 */
export function describeSync(status: TodoSyncStatus | null, now: number): SyncLine | null {
  if (!status || !status.supported) return null;
  if (!status.enabled) return { tone: "off", key: "localOnly", action: "enable" };

  // The user has to decide something: loudest first.
  switch (status.paused) {
    case "accountChanged":
      return { tone: "error", key: "accountChanged", action: "view" };
    case "zoneMissing":
      return { tone: "error", key: "zoneMissing", action: "view" };
    case "quota":
      return { tone: "warn", key: "quota", action: "resume" };
    case "notSignedIn":
      return { tone: "warn", key: "notSignedIn" };
    default:
      break;
  }
  if (status.newerState) return { tone: "warn", key: "newerState" };

  // The helper is what talks to iCloud; without it nothing leaves this Mac.
  if (!status.helper.installed && status.helper.state !== "running") {
    return { tone: "error", key: "helperMissing", action: "repair" };
  }
  if (status.helper.state === "notRunning" || status.helper.state === "incompatible") {
    return { tone: "error", key: "helperDown", action: "repair" };
  }
  if (status.helper.problem) {
    return { tone: "error", key: "helperProblem", detail: status.helper.problem, action: "repair" };
  }

  if (status.syncing) return { tone: "busy", key: "syncing" };

  const last = status.lastTick;
  if (last?.outcome === "offline") {
    return { tone: "warn", key: "offline", count: status.unsynced };
  }
  if (last?.outcome === "error") {
    return { tone: "error", key: "error", detail: last.message };
  }

  const lastOk = Math.max(status.lastPullOkMs, status.lastPushOkMs);
  // Enabled, but nothing has completed yet: the first sync is on its way.
  if (lastOk === 0) return { tone: "busy", key: "syncing" };
  return { tone: "ok", key: "synced", ageMs: Math.max(0, now - lastOk) };
}

/** Units of a relative time; the caller maps them to its own wording. */
export interface Age {
  unit: "justNow" | "seconds" | "minutes" | "hours" | "days";
  n: number;
}

export function ageOf(ageMs: number): Age {
  const sec = Math.floor(Math.max(0, ageMs) / 1000);
  if (sec < 5) return { unit: "justNow", n: 0 };
  if (sec < 60) return { unit: "seconds", n: sec };
  const min = Math.floor(sec / 60);
  if (min < 60) return { unit: "minutes", n: min };
  const hr = Math.floor(min / 60);
  if (hr < 24) return { unit: "hours", n: hr };
  return { unit: "days", n: Math.floor(hr / 24) };
}

/** How often the window asks for a fresh status. `null` = not at all (nothing can change by itself). */
export function pollDelayMs(status: TodoSyncStatus | null): number | null {
  if (!status || !status.supported || !status.enabled) return null;
  // While a tick runs the answer is about to change; otherwise a slow heartbeat keeps "3 min ago" honest.
  return status.syncing ? 1500 : 15000;
}
