import { computed, onBeforeUnmount, ref, shallowRef } from "vue";
import type { LatestAgentText } from "../../lib/types";

export const TEXT_UPDATE_INTERVAL_MS = 10_000;

/** A fixed throttle window keeps dense updates from starving the next visible paragraph. */
export function useLatestAgentText() {
  const displayed = shallowRef<LatestAgentText | null>(null);
  const pending = shallowRef<LatestAgentText | null>(null);
  const expanded = ref(false);
  const reading = ref(false);
  let lastShown = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const same = (a: LatestAgentText | null, b: LatestAgentText | null) =>
    a?.kind === b?.kind && a?.revision === b?.revision;
  const hasNewContent = computed(() => !!pending.value && !same(displayed.value, pending.value));

  function cancelTimer() {
    clearTimeout(timer);
    timer = undefined;
  }

  function showLatest() {
    cancelTimer();
    if (!pending.value) return;
    displayed.value = pending.value;
    pending.value = null;
    lastShown = Date.now();
  }

  function schedule() {
    if (reading.value || expanded.value || timer || !hasNewContent.value) return;
    const delay = Math.max(0, TEXT_UPDATE_INTERVAL_MS - (Date.now() - lastShown));
    if (delay === 0) showLatest();
    else timer = setTimeout(() => {
      timer = undefined;
      schedule();
    }, delay);
  }

  function receive(candidate: LatestAgentText | null) {
    if (!candidate?.text) return;
    if (same(displayed.value, candidate)) {
      pending.value = null;
      cancelTimer();
      return;
    }
    pending.value = candidate;
    if (!displayed.value) showLatest();
    else schedule();
  }

  function lockReading() {
    reading.value = true;
    cancelTimer();
  }

  function toggleExpanded() {
    expanded.value = !expanded.value;
    if (expanded.value) lockReading();
    else resume();
  }

  function resume() {
    reading.value = false;
    showLatest();
  }

  function updateNow() {
    showLatest();
    if (!expanded.value) reading.value = false;
  }

  onBeforeUnmount(cancelTimer);
  return { displayed, expanded, reading, hasNewContent, receive, lockReading, toggleExpanded, resume, updateNow };
}
