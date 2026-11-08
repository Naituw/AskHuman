// A folded card hides its rows, so a jump to one of them (a deep link such as `integration#lifecycle-claude`,
// a settings-search hit) has to open the card first. The jump announces itself on the target
// element with a bubbling event; a card that folds its rows listens for it on its root and opens
// itself. A target that nothing folds away just ignores the event.
import { nextTick } from "vue";

export const SETTINGS_REVEAL_EVENT = "settings-reveal";

/** Ask whatever folds `el` away to open, and wait until the DOM shows it. */
export async function revealSettingsTarget(el: Element | null | undefined): Promise<void> {
  if (!el) return;
  el.dispatchEvent(new CustomEvent(SETTINGS_REVEAL_EVENT, { bubbles: true }));
  await nextTick();
}
