// Popup "Quick reply" menu (docs/specs/special-replies.md D17–D21, D27): the footer button's state, the
// user's preset replies (one click sends them), the directive that waits for Send ("away mode" /
// "stop using AskHuman" for this session) and the escalation banner's "end away mode".
//
// The menu is KEYBOARD-MODAL like the todo popover: while it is open `handleQuickReplyKeydown`
// swallows every shortcut the popup owns except ⌘/Ctrl+W, so a key meant for the menu can never
// submit an answer or close the window.
import { computed, nextTick, ref, watch, type Ref } from "vue";
import { awayEnd } from "../../lib/ipc";
import { primaryModifierPressed } from "../../lib/platform";
import type { AskRequest, EscalationReason, PopupDirective } from "../../lib/types";

/** The longest note the daemon keeps (`away::NOTE_MAX_CHARS`). */
export const QUICK_REPLY_NOTE_MAX = 500;

/** A directive picked from the menu that waits for Send (spec D19). */
export interface ArmedDirective {
  kind: "away" | "mute";
  /** Mute is always about the asking session; the scope selector only exists for away mode. */
  scope: "global" | "session";
  note: string;
}

export type QuickReplyItemId = "away" | "endAway" | "mute";

/** One row of the menu, in the order it is shown: presets, then the directives, then "manage". */
export type QuickReplyItem =
  | {
      kind: "preset";
      /** Stable key and `data-item` of the row. */
      id: string;
      /** Index in the user's list. */
      index: number;
      text: string;
      /** The digit that picks it (1–9, then 0), if it has one. */
      key: string | null;
      /** Every question already has an answer, so there is nothing a preset could fill in. */
      disabled: boolean;
    }
  | { kind: "directive"; id: QuickReplyItemId; disabled: false }
  | { kind: "manage"; id: "manage"; disabled: false };

/** The digit key of the n-th preset: 1…9, then 0 for the tenth. */
export function presetKey(index: number): string | null {
  if (index < 0 || index > 9) return null;
  return String((index + 1) % 10);
}

export interface QuickReplyDeps {
  request: Ref<AskRequest | null>;
  isConfirm: Ref<boolean>;
  /** The directive waiting for Send; owned by the core because `canSubmit` depends on it. */
  armed: Ref<ArmedDirective | null>;
  /** The in-page find bar is open; it covers the footer area, so ⌘K stays out of the way. */
  findActive: () => boolean;
  /** The key press is the user's own speech shortcut: it wins over ⌘K. */
  speechTaken: (event: KeyboardEvent) => boolean;
  /** The popup cannot take input right now (hidden, behind a dialog, another overlay is open). */
  suspended: () => boolean;
  /** How many questions a preset would answer: those with nothing in them yet (0 where presets make no sense). */
  presetTargets: () => number;
  /** Sends the popup now with this text as the reply of every question that has no answer yet (spec D18). */
  sendPreset: (text: string) => void | Promise<void>;
  /** Opens the settings window at the quick-reply card ("Manage quick replies…"). */
  openManage: () => void;
}

export function useQuickReply(deps: QuickReplyDeps) {
  const open = ref(false);
  /** The highlighted menu row. */
  const index = ref(0);
  /** The banner's "end away mode" succeeded: the request is no longer an escalation. */
  const awayEnded = ref(false);
  const ending = ref(false);
  const awayError = ref("");

  /** Only an Agent's own question has the menu (Stop cards, permission cards and relays do not, N10). */
  const available = computed(
    () => !deps.isConfirm.value && !!deps.request.value && deps.request.value.quickReply !== false,
  );

  /** Why this question came through away mode; null once the user ended it. */
  const escalated = computed<EscalationReason | null>(() =>
    awayEnded.value ? null : deps.request.value?.escalated ?? null,
  );

  /**
   * The user's presets this question carries. The daemon sends none with What's next or a strict choice (spec D20,
   * D27), and a free-text reply makes no sense there, so this hides them even if some arrive.
   */
  const presets = computed(() => {
    const request = deps.request.value;
    if (!available.value || !request || request.whatsNext || request.selectOnly) return [];
    return (request.quickReplies ?? []).filter((text) => text.trim());
  });
  /** How many questions a click on a preset would fill in; the menu says so and greys the presets out at 0. */
  const presetTargets = computed(() => (presets.value.length ? deps.presetTargets() : 0));
  const presetsEnabled = computed(() => presetTargets.value > 0);

  /**
   * What the menu offers: the presets, then the directives (while away mode is on, which makes this an
   * escalation, the first one is "end", else "start"), then the link to the settings.
   */
  const items = computed<QuickReplyItem[]>(() => [
    ...presets.value.map<QuickReplyItem>((text, index) => ({
      kind: "preset",
      id: `preset-${index}`,
      index,
      text,
      key: presetKey(index),
      disabled: !presetsEnabled.value,
    })),
    { kind: "directive", id: escalated.value ? "endAway" : "away", disabled: false },
    { kind: "directive", id: "mute", disabled: false },
    { kind: "manage", id: "manage", disabled: false },
  ]);

  /** The directive to send along with the answer; undefined when nothing is armed. */
  const directive = computed<PopupDirective | undefined>(() => {
    const armed = deps.armed.value;
    if (!armed) return undefined;
    const note = armed.note.trim().slice(0, QUICK_REPLY_NOTE_MAX);
    return {
      kind: armed.kind,
      scope: armed.kind === "mute" ? "session" : armed.scope,
      ...(armed.kind === "away" && note ? { note } : {}),
    };
  });

  let restoreFocusEl: HTMLElement | null = null;

  function openMenu() {
    if (open.value || !available.value || deps.suspended()) return;
    const active = document.activeElement;
    restoreFocusEl = active instanceof HTMLElement && active !== document.body ? active : null;
    index.value = Math.max(0, items.value.findIndex((item) => !item.disabled));
    open.value = true;
  }

  /** `restoreFocus` puts the focus back where it was; skipped when the focus already moved on. */
  function closeMenu(restoreFocus = true) {
    if (!open.value) return;
    open.value = false;
    const target = restoreFocusEl;
    restoreFocusEl = null;
    if (restoreFocus && target?.isConnected) {
      void nextTick(() => target.focus({ preventScroll: true }));
    }
  }

  function toggleMenu() {
    if (open.value) closeMenu();
    else openMenu();
  }

  /** Moves the highlight, skipping rows that cannot be picked (greyed-out presets). */
  function move(delta: number) {
    const count = items.value.length;
    let next = index.value;
    for (let step = 0; step < count; step++) {
      next = (next + delta + count) % count;
      if (!items.value[next]?.disabled) {
        index.value = next;
        return;
      }
    }
  }

  /** Arms a directive: it waits above the footer until Send (spec D19), × takes it back. */
  function arm(kind: "away" | "mute") {
    const previous = deps.armed.value;
    deps.armed.value = {
      kind,
      // Away defaults to every Agent; a note typed for the other kind is not carried over.
      scope: kind === "away" ? previous?.kind === "away" ? previous.scope : "global" : "session",
      note: kind === "away" && previous?.kind === "away" ? previous.note : "",
    };
  }

  function disarm() {
    deps.armed.value = null;
  }

  /** The banner's button and the menu's "end away mode" row. */
  async function endAwayMode() {
    if (ending.value || !escalated.value) return;
    ending.value = true;
    awayError.value = "";
    try {
      await awayEnd();
      awayEnded.value = true;
    } catch (error) {
      awayError.value = String(error);
    } finally {
      ending.value = false;
    }
  }

  function choose(id: QuickReplyItemId) {
    const target = restoreFocusEl;
    closeMenu(false);
    if (id === "endAway") {
      void endAwayMode();
    } else {
      arm(id);
    }
    // Picking a row is not a reason to lose the place in the answer: the focus goes back where it was.
    if (target?.isConnected) void nextTick(() => target.focus({ preventScroll: true }));
  }

  /** A preset sends the popup at once: answered questions keep their answers, the others get this text. */
  function choosePreset(index: number) {
    const text = presets.value[index];
    if (text === undefined || !presetsEnabled.value) return;
    closeMenu(false);
    void deps.sendPreset(text);
  }

  function chooseManage() {
    closeMenu(false);
    deps.openManage();
  }

  /** Whatever row it is (a click, Enter on the highlighted row, a digit). */
  function chooseItem(item: QuickReplyItem) {
    if (item.disabled) return;
    if (item.kind === "preset") choosePreset(item.index);
    else if (item.kind === "manage") chooseManage();
    else choose(item.id);
  }

  /** Forget everything about the previous request (the core is initialised once per request). */
  function reset() {
    open.value = false;
    index.value = 0;
    awayEnded.value = false;
    ending.value = false;
    awayError.value = "";
    restoreFocusEl = null;
    deps.armed.value = null;
  }

  // A window-level dialog, a confirmation modal, another overlay or another request taking the window
  // over closes the menu; the focus already belongs to whatever took over.
  watch(
    () => deps.suspended(),
    (suspended) => {
      if (suspended) closeMenu(false);
    },
  );

  // The menu has nothing to offer once the request lost it (a request is replaced in place).
  watch(available, (value) => {
    if (!value) {
      closeMenu(false);
      disarm();
    }
  });

  /**
   * Returns true when the event was consumed (the caller must stop handling it). Call it before any
   * other popup shortcut.
   */
  function handleQuickReplyKeydown(event: KeyboardEvent): boolean {
    const composing = event.isComposing || event.keyCode === 229;
    const mod = primaryModifierPressed(event);
    const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
    const toggles = mod && !event.altKey && !event.shiftKey && key === "k" && !deps.speechTaken(event);

    if (!open.value) {
      if (!toggles || composing || deps.findActive() || !available.value) return false;
      event.preventDefault();
      openMenu();
      return true;
    }

    // Open: nothing below may reach the rest of the popup, whatever the focus is on.
    if (composing) return true;
    // Closing the window stays available; the popup's own ⌘W handler takes it from here.
    if (mod && key === "w") return false;
    if (toggles || event.key === "Escape") {
      event.preventDefault();
      closeMenu();
      return true;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      move(event.key === "ArrowDown" ? 1 : -1);
      return true;
    }
    if (event.key === "Enter") {
      // Enter on a row keeps its native click; anywhere else it picks the highlighted row.
      const target = event.target;
      if (target instanceof HTMLElement && target.closest(".qr-item")) return true;
      event.preventDefault();
      const item = items.value[index.value];
      if (item) chooseItem(item);
      return true;
    }
    // A digit picks the preset that carries it; the key is the menu's own, so a modified digit (⌘1 picks an
    // option of the question) stays swallowed above.
    if (/^[0-9]$/.test(event.key) && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) {
      event.preventDefault();
      const preset = items.value.find((item) => item.kind === "preset" && item.key === event.key);
      if (preset) chooseItem(preset);
      return true;
    }
    return true;
  }

  return {
    quickReplyAvailable: available,
    quickReplyOpen: open,
    quickReplyIndex: index,
    quickReplyItems: items,
    quickReplyPresetTargets: presetTargets,
    quickReplyDirective: directive,
    quickReplyEscalated: escalated,
    quickReplyEnding: ending,
    quickReplyAwayError: awayError,
    openQuickReply: openMenu,
    closeQuickReply: closeMenu,
    toggleQuickReply: toggleMenu,
    chooseQuickReply: choose,
    chooseQuickReplyItem: chooseItem,
    armDirective: arm,
    disarmDirective: disarm,
    endAwayMode,
    resetQuickReply: reset,
    handleQuickReplyKeydown,
  };
}
