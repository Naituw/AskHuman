// Size and motion of the small "pending" window (docs/specs/popup-inbox-collapse.md §2.4).
// The small window is the Sidebar column alone: its width is the Sidebar width and its height is measured from the
// list in its final small layout, so the window fits what it shows.

/** Request rows the small window shows before its list scrolls. */
export const MINI_ROWS = 3;
/** Space under the last visible row. */
const MINI_BOTTOM = 16;

/** How a window form change is animated (milliseconds); the curve is the Material "emphasized" one. */
export const miniMotion = { collapse: 440, expand: 480, resize: 240, curve: [0.2, 0, 0, 1] } as const;
export type FormChange = "collapse" | "expand" | "resize";
export interface FormAnimation { durationMs: number; curve: number[] }

/** The animation of a form change, or none (an instant change) for people who asked for reduced motion. */
export function formAnimation(kind: FormChange, reduced: boolean): FormAnimation | undefined {
  return reduced ? undefined : { durationMs: miniMotion[kind], curve: [...miniMotion.curve] };
}

export interface MiniHeight {
  /** Window height. */
  height: number;
  /** Height of the list itself once it is settled, so that it scrolls inside the window. */
  navHeight: number;
}

/**
 * Measure the small window's height from the real list. The root is put into the small layout (`data-measure`) with
 * transitions disabled (`data-still`) for the measurement; the restoring style recalculation also runs with them
 * disabled, so nothing animates because of it. All of this happens synchronously and never paints.
 */
export function measureMini(root: HTMLElement, rows = MINI_ROWS): MiniHeight | null {
  const nav = root.querySelector<HTMLElement>(".inbox-navigation");
  if (!nav) return null;
  root.setAttribute("data-still", "");
  root.setAttribute("data-measure", "");
  try {
    const entries = [...nav.querySelectorAll<HTMLElement>(".inbox-entry")];
    const navBox = nav.getBoundingClientRect();
    const navTop = navBox.top - root.getBoundingClientRect().top;
    const last = entries[Math.min(rows, entries.length) - 1];
    // Row rectangles are in content coordinates whether or not the list is already height-limited.
    const navHeight = Math.ceil((last ? last.getBoundingClientRect().bottom - navBox.top + nav.scrollTop : 0) + MINI_BOTTOM);
    return { height: Math.ceil(navTop + navHeight), navHeight };
  } finally {
    root.removeAttribute("data-measure");
    void root.offsetHeight;
    root.removeAttribute("data-still");
  }
}
