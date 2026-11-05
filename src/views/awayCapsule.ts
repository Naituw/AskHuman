// The away capsule's behaviour apart from its markup (docs/specs/special-replies.md D22, D23): when the
// card is open, how the user can move the window, and what "end away mode" asks for.
import { computed, onBeforeUnmount, ref, type Ref } from "vue";
import type { CapsuleView } from "../lib/types";

/** Hover has to rest on the capsule this long before it opens, so passing the mouse over it does not flicker. */
export const HOVER_OPEN_MS = 140;
/** The card stays open this long after the mouse has left it. */
export const HOVER_CLOSE_MS = 380;
/** The welcome-back card goes back to being a capsule after this long without an answer. */
export const WELCOME_TIMEOUT_MS = 120_000;
/** A press that moves farther than this is a drag of the window, not a click. */
export const DRAG_THRESHOLD_PX = 4;

/**
 * Whether the card is open: always on "welcome back", while pinned by a click, or while the mouse is
 * on it. Pointer handlers go on the whole window content so moving from the capsule onto the card
 * does not close it.
 */
export function useCapsuleExpansion(welcome: Ref<boolean>) {
  const hovering = ref(false);
  const pinned = ref(false);
  let openTimer: ReturnType<typeof setTimeout> | null = null;
  let closeTimer: ReturnType<typeof setTimeout> | null = null;

  function clear() {
    if (openTimer) clearTimeout(openTimer);
    if (closeTimer) clearTimeout(closeTimer);
    openTimer = null;
    closeTimer = null;
  }

  function enter() {
    if (closeTimer) clearTimeout(closeTimer);
    closeTimer = null;
    if (hovering.value || openTimer) return;
    openTimer = setTimeout(() => {
      openTimer = null;
      hovering.value = true;
    }, HOVER_OPEN_MS);
  }

  function leave() {
    if (openTimer) clearTimeout(openTimer);
    openTimer = null;
    if (!hovering.value || closeTimer) return;
    closeTimer = setTimeout(() => {
      closeTimer = null;
      hovering.value = false;
    }, HOVER_CLOSE_MS);
  }

  /** A click on the capsule or the card's head pins the card open, or lets it go. */
  function toggle() {
    clear();
    pinned.value = !pinned.value;
    // Letting go while the mouse is still there keeps it open until the mouse leaves.
    hovering.value = true;
  }

  function collapse() {
    clear();
    pinned.value = false;
    hovering.value = false;
  }

  const expanded = computed(() => welcome.value || pinned.value || hovering.value);
  onBeforeUnmount(clear);
  return { expanded, pinned, hovering, enter, leave, toggle, collapse };
}

/**
 * Press-and-move on the head of the window drags it; a press that does not move is a click. The
 * native drag only starts once the pointer has gone past the threshold, so clicks are never eaten.
 */
export function useWindowDrag(startDragging: () => Promise<void> | void, onClick: () => void) {
  let origin: { x: number; y: number } | null = null;
  let dragged = false;

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    origin = { x: e.screenX, y: e.screenY };
    dragged = false;
  }

  function move(e: PointerEvent) {
    if (!origin || dragged || (e.buttons & 1) === 0) return;
    if (Math.hypot(e.screenX - origin.x, e.screenY - origin.y) < DRAG_THRESHOLD_PX) return;
    dragged = true;
    void startDragging();
  }

  function up() {
    const wasClick = origin !== null && !dragged;
    origin = null;
    dragged = false;
    if (wasClick) onClick();
  }

  return { down, move, up };
}

/** What "end away mode" on the card has to do: the global switch, or each session that is away on its own. */
export type EndStep = { op: "end" } | { op: "session"; sessionKey: string; setting: "follow" };

export function endSteps(view: CapsuleView): EndStep[] {
  if (view.scope === "all") return [{ op: "end" }];
  return view.sessionKeys.map((sessionKey) => ({ op: "session", sessionKey, setting: "follow" }));
}
