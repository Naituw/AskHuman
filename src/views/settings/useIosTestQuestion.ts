// The "test question" of a ready iOS App channel (docs/specs/ios-setup-experience.md §9): a real
// question sent to the phone, answered there, which proves the whole chain (broker, relay, push,
// the phone's key) at once. It is the setup wizard's channel check, run for `ios`: the backend owns
// the question and its 120 s wait; this only starts it and follows how it ended.
import { computed, onBeforeUnmount, onMounted } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { SetupProbeResult } from "../../lib/types";
import { createProbes } from "../setup/useProbe";

export type IosTestQuestionState = "idle" | "sent" | "answered" | "timeout" | "cancelled" | "failed";

export function useIosTestQuestion() {
  const probes = createProbes();

  let unlisten: UnlistenFn | null = null;
  let disposed = false;
  onMounted(async () => {
    const stop = await listen<SetupProbeResult>("setup-probe-result", (e) => probes.handleResult(e.payload));
    if (disposed) stop();
    else unlisten = stop;
  });
  onBeforeUnmount(() => {
    disposed = true;
    unlisten?.();
    // A question still waiting for an answer would outlive the card that asked it.
    probes.releaseAll();
  });

  const state = computed<IosTestQuestionState>(() => {
    const probe = probes.states.channel;
    if (probe.phase === "idle") return "idle";
    if (probe.phase === "running") return "sent";
    switch (probe.outcome) {
      case "ok":
        return "answered";
      case "timeout":
        return "timeout";
      case "cancelled":
        return "cancelled";
      default:
        return "failed";
    }
  });

  /** Why the question could not be sent at all. */
  const error = computed(() => probes.states.channel.error ?? "");

  function send() {
    return probes.start("channel", "ios");
  }

  return { state, error, send };
}
