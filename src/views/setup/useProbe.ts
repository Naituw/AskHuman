// The setup wizard's three optional checks, one small state machine each
// (docs/specs/onboarding.md D11, docs/plans/onboarding.md §4.4): idle → running (it can be
// cancelled) → done (ok / timeout / cancelled / failed). None of them is a step of the wizard: a
// check that fails or never finishes changes nothing about finishing it, and the three do not
// affect each other. The backend runs them; this only follows what it reports.
import { reactive } from "vue";
import { setupProbeCancel, setupProbeStart } from "../../lib/ipc";
import type { SetupProbeKind, SetupProbeOutcome, SetupProbeResult } from "../../lib/types";

export interface ProbeState {
  phase: "idle" | "running" | "done";
  /** The channel a `channel` check was started for. */
  channel: string | null;
  outcome: SetupProbeOutcome | null;
  /** The Agent family whose question the `agent` check saw. */
  agentKind: string | null;
  /** Why the check could not even start. */
  error: string | null;
}

/** How many results are kept for a start whose answer has not come back yet. */
const EARLY_LIMIT = 8;

/** A check that is not running and has nothing to report. */
export function idleProbe(): ProbeState {
  return { phase: "idle", channel: null, outcome: null, agentKind: null, error: null };
}

const blank = idleProbe;

export function createProbes() {
  const states = reactive<Record<SetupProbeKind, ProbeState>>({
    popup: blank(),
    channel: blank(),
    agent: blank(),
  });
  /** The id the backend gave each running check; `null` between the click and its answer. */
  const ids: Record<SetupProbeKind, string | null> = { popup: null, channel: null, agent: null };
  /** Bumped whenever a kind starts or is released, so a late answer knows it is out of date. */
  const runs: Record<SetupProbeKind, number> = { popup: 0, channel: 0, agent: 0 };
  /** Results that arrived before `setupProbeStart` returned the id they belong to. */
  const early = new Map<string, SetupProbeResult>();

  function settle(kind: SetupProbeKind, result: SetupProbeResult) {
    ids[kind] = null;
    states[kind] = {
      phase: "done",
      channel: result.channel ?? states[kind].channel,
      outcome: result.outcome,
      agentKind: result.agentKind ?? null,
      error: null,
    };
  }

  function stopBackend(id: string) {
    setupProbeCancel(id).catch(() => {
      /* the check ends by itself at its own timeout */
    });
  }

  /** Start a check; one that is already running for the same target is left as it is. */
  async function start(kind: SetupProbeKind, channel?: string) {
    const current = states[kind];
    if (current.phase === "running") {
      // Another channel's check gives way; anything else is already underway.
      if (kind !== "channel" || current.channel === (channel ?? null)) return;
      cancel(kind);
    }
    const run = ++runs[kind];
    ids[kind] = null;
    states[kind] = { ...blank(), phase: "running", channel: channel ?? null };
    let id: string;
    try {
      id = await setupProbeStart(kind, channel);
    } catch (e) {
      if (runs[kind] === run) {
        states[kind] = { ...blank(), phase: "done", outcome: "failed", channel: channel ?? null, error: String(e) };
      }
      return;
    }
    if (runs[kind] !== run) {
      // Cancelled while the backend was still answering.
      stopBackend(id);
      return;
    }
    const result = early.get(id);
    if (result) {
      early.delete(id);
      settle(kind, result);
      return;
    }
    ids[kind] = id;
  }

  /** A check ended. A result nobody waits for any more is dropped. */
  function handleResult(result: SetupProbeResult) {
    const kind = result.kind;
    if (ids[kind] === result.probeId) {
      settle(kind, result);
      return;
    }
    // The check may have ended before its id reached us.
    if (states[kind].phase === "running" && ids[kind] === null) {
      early.set(result.probeId, result);
      while (early.size > EARLY_LIMIT) early.delete(early.keys().next().value as string);
    }
  }

  /** Stop a running check and go back to idle. */
  function cancel(kind: SetupProbeKind) {
    if (states[kind].phase !== "running") return;
    const id = ids[kind];
    runs[kind]++;
    ids[kind] = null;
    states[kind] = blank();
    if (id) stopBackend(id);
  }

  /** Back to idle whatever the state: a running check is stopped, a result is forgotten. */
  function release(kind: SetupProbeKind) {
    cancel(kind);
    states[kind] = blank();
  }

  function releaseAll() {
    (Object.keys(states) as SetupProbeKind[]).forEach(release);
  }

  return { states, start, cancel, release, releaseAll, handleResult };
}

export type Probes = ReturnType<typeof createProbes>;
