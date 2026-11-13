import { computed, onBeforeUnmount, ref, shallowRef } from "vue";
import type { LatestAgentText } from "../../lib/types";

/** New text replaces the shown text at most this often. */
export const TEXT_UPDATE_INTERVAL_MS = 10_000;
/** How long the text holds still after the pointer that was on it has left. */
export const TEXT_READING_LEASE_MS = 3_000;

/**
 * A fixed throttle window keeps dense updates from starving the next visible paragraph. Reading holds the text as a
 * lease (docs/specs/agent-observation-latest-text.md §4): a pointer on the text holds it, and so does the lease
 * after the pointer left, which a pointer that comes back renews. An opened text holds until it is closed or the
 * newest is asked for.
 */
export function useLatestAgentText() {
  const displayed = shallowRef<LatestAgentText | null>(null);
  const pending = shallowRef<LatestAgentText | null>(null);
  const expanded = ref(false);
  /** A pointer is down on the text. */
  const touching = ref(false);
  /** The pointer has left the text, but only moments ago. */
  const leased = ref(false);
  /** The text is held for somebody who is reading it (an opened text holds by itself: see `expanded`). */
  const reading = computed(() => touching.value || leased.value);
  let lastShown = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let leaseTimer: ReturnType<typeof setTimeout> | undefined;

  const same = (a: LatestAgentText | null, b: LatestAgentText | null) =>
    a?.kind === b?.kind && a?.revision === b?.revision;
  const hasNewContent = computed(() => !!pending.value && !same(displayed.value, pending.value));

  function cancelTimer() {
    clearTimeout(timer);
    timer = undefined;
  }

  function endLease() {
    clearTimeout(leaseTimer);
    leaseTimer = undefined;
    leased.value = false;
  }

  function startLease() {
    clearTimeout(leaseTimer);
    leased.value = true;
    cancelTimer();
    leaseTimer = setTimeout(() => {
      leaseTimer = undefined;
      leased.value = false;
      schedule();
    }, TEXT_READING_LEASE_MS);
  }

  function showLatest() {
    cancelTimer();
    if (!pending.value) return;
    displayed.value = pending.value;
    pending.value = null;
    lastShown = Date.now();
  }

  /** The pending text waits for the replacement interval, counted from the last replacement, and for the lease. */
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

  /** A pointer came down on the text. */
  function holdBegin() {
    touching.value = true;
    endLease();
    cancelTimer();
  }

  /** The pointer that was down on the text is up, or has left the window. */
  function holdEnd() {
    if (!touching.value) return;
    touching.value = false;
    startLease();
  }

  /** Something that happens on the text and is over at once (a turn of the wheel, a change of the selection). */
  function holdBriefly() {
    if (!touching.value) startLease();
  }

  function toggleExpanded() {
    expanded.value = !expanded.value;
    endLease();
    if (expanded.value) cancelTimer();
    else showLatest();
  }

  function updateNow() {
    showLatest();
    endLease();
  }

  onBeforeUnmount(() => {
    cancelTimer();
    clearTimeout(leaseTimer);
  });
  return {
    displayed, expanded, reading, hasNewContent, receive, holdBegin, holdEnd, holdBriefly, toggleExpanded, updateNow,
  };
}
